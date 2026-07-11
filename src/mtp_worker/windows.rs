use super::{
    MtpDevice, MtpEvent, MtpRequest, RemoteAsset, RemoteSession, RemoteShot, SourceFolder,
    planning::{classify_media_name, plan_import},
};
use crate::safe_copy::{CopyOutcome, copy_reader_no_clobber};
use color_eyre::eyre::{Result, WrapErr, eyre};
use directories::ProjectDirs;
use log::warn;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use winmtp::PortableDevices::WPD_OBJECT_SIZE;
use winmtp::Provider;
use winmtp::device::{BasicDevice, Device};
use winmtp::object::{Object, ObjectType};

/// Handle for the dedicated Windows Portable Devices worker thread.
///
/// Requests and events deliberately contain only owned Rust data. All COM objects stay in
/// `WorkerState`, which is created and destroyed on the worker thread.
pub struct MtpWorker {
    requests: Sender<MtpRequest>,
    events: Receiver<MtpEvent>,
    thread: Option<JoinHandle<()>>,
}

impl MtpWorker {
    pub fn spawn() -> Result<Self> {
        let (request_sender, request_receiver) = mpsc::channel();
        let (event_sender, event_receiver) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("windows-mtp-worker".to_owned())
            .spawn(move || WorkerState::default().run(request_receiver, event_sender))
            .wrap_err("failed to start the Windows MTP worker thread")?;
        Ok(Self {
            requests: request_sender,
            events: event_receiver,
            thread: Some(thread),
        })
    }

    pub fn send(&self, request: MtpRequest) -> Result<()> {
        self.requests
            .send(request)
            .map_err(|_| eyre!("the Windows MTP worker has stopped"))
    }

    pub fn try_recv(&self) -> Option<MtpEvent> {
        self.events.try_recv().ok()
    }
}

impl Drop for MtpWorker {
    fn drop(&mut self) {
        let _ = self.requests.send(MtpRequest::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Default)]
struct WorkerState {
    provider: Option<Provider>,
    session: Option<PrivateSession>,
}

struct PrivateSession {
    _device: Device,
    public: RemoteSession,
    objects: HashMap<String, Object>,
    preview_paths: Vec<PathBuf>,
}

impl WorkerState {
    fn run(&mut self, requests: Receiver<MtpRequest>, events: Sender<MtpEvent>) {
        while let Ok(request) = requests.recv() {
            if matches!(request, MtpRequest::Shutdown) {
                return;
            }
            let operation = request.operation();
            let result = self.handle(request);
            let event = match result {
                Ok(event) => event,
                Err(error) => MtpEvent::Error {
                    operation,
                    message: format!("{error:#}"),
                },
            };
            if events.send(event).is_err() {
                return;
            }
        }
    }

    fn handle(&mut self, request: MtpRequest) -> Result<MtpEvent> {
        match request {
            MtpRequest::ListDevices => Ok(MtpEvent::Devices(self.list_devices()?)),
            MtpRequest::ListSourceFolders { device_id } => {
                let folders = self.list_source_folders(&device_id)?;
                Ok(MtpEvent::SourceFolders { device_id, folders })
            }
            MtpRequest::StartSession {
                device_id,
                source_folder_id,
            } => {
                let session = self.scan_session(device_id, source_folder_id)?;
                let public = session.public.clone();
                self.session = Some(session);
                Ok(MtpEvent::SessionScanned(public))
            }
            MtpRequest::ImportKept {
                shot_ids,
                destinations,
            } => self.import_kept(shot_ids, destinations),
            MtpRequest::Shutdown => unreachable!("shutdown is handled before dispatch"),
        }
    }

    fn provider(&mut self) -> Result<&Provider> {
        if self.provider.is_none() {
            self.provider =
                Some(Provider::new().wrap_err("failed to initialize Windows Portable Devices")?);
        }
        Ok(self.provider.as_ref().expect("provider was initialized"))
    }

    fn basic_device(&mut self, device_id: &str) -> Result<BasicDevice> {
        self.provider()?
            .enumerate_devices()
            .wrap_err("failed to enumerate MTP devices")?
            .into_iter()
            .find(|device| device.device_id() == device_id)
            .ok_or_else(|| eyre!("MTP device {device_id:?} is not connected"))
    }

    fn open_device(&mut self, device_id: &str) -> Result<Device> {
        let device = self.basic_device(device_id)?;
        device
            .open(&winmtp::make_current_app_identifiers!(), false)
            .wrap_err_with(|| format!("failed to open MTP device {:?}", device.friendly_name()))
    }

    fn list_devices(&mut self) -> Result<Vec<MtpDevice>> {
        let mut devices = self
            .provider()?
            .enumerate_devices()
            .wrap_err("failed to enumerate MTP devices")?
            .into_iter()
            .map(|device| MtpDevice {
                id: device.device_id(),
                name: device.friendly_name().to_owned(),
            })
            .collect::<Vec<_>>();
        devices.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
        Ok(devices)
    }

    fn list_source_folders(&mut self, device_id: &str) -> Result<Vec<SourceFolder>> {
        let device = self.open_device(device_id)?;
        let root = device
            .content()
            .wrap_err("failed to access MTP device content")?
            .root()
            .wrap_err("failed to access MTP device root")?;
        let mut folders = vec![SourceFolder {
            id: root.id().to_string_lossy(),
            name: "Device root".to_owned(),
            path: String::new(),
        }];
        collect_source_folders(root, String::new(), &mut folders)?;
        folders.sort_by(|left, right| left.path.cmp(&right.path).then(left.id.cmp(&right.id)));
        Ok(folders)
    }

    fn scan_session(
        &mut self,
        device_id: String,
        source_folder_id: String,
    ) -> Result<PrivateSession> {
        let device = self.open_device(&device_id)?;
        let root = device
            .content()
            .wrap_err("failed to access MTP device content")?
            .root()
            .wrap_err("failed to access MTP device root")?;
        let source = find_container(root, &source_folder_id)?
            .ok_or_else(|| eyre!("MTP source folder {source_folder_id:?} no longer exists"))?;
        let cache_directory = preview_cache_directory()?;
        let mut grouped = BTreeMap::<String, Vec<(RemoteAsset, Object)>>::new();
        let mut preview_paths = Vec::new();
        collect_media(
            source,
            &device_id,
            &cache_directory,
            &mut grouped,
            &mut preview_paths,
        )?;

        let mut objects = HashMap::new();
        let mut shots = Vec::with_capacity(grouped.len());
        for (stem, mut assets) in grouped {
            assets.sort_by(|left, right| {
                left.0
                    .name
                    .cmp(&right.0.name)
                    .then(left.0.object_id.cmp(&right.0.object_id))
            });
            let id = shot_id(&device_id, &source_folder_id, &stem);
            let mut public_assets = Vec::with_capacity(assets.len());
            for (asset, object) in assets {
                objects.insert(asset.object_id.clone(), object);
                public_assets.push(asset);
            }
            shots.push(RemoteShot {
                id,
                stem,
                assets: public_assets,
            });
        }
        Ok(PrivateSession {
            _device: device,
            public: RemoteSession {
                device_id,
                source_folder_id,
                shots,
            },
            objects,
            preview_paths,
        })
    }

    fn import_kept(
        &mut self,
        shot_ids: Vec<String>,
        destinations: super::planning::ImportPaths,
    ) -> Result<MtpEvent> {
        let session = self
            .session
            .as_mut()
            .ok_or_else(|| eyre!("start an MTP session before importing"))?;
        let mut wanted = shot_ids.into_iter().collect::<HashSet<_>>();
        let selected = session
            .public
            .shots
            .iter()
            .filter(|shot| wanted.remove(&shot.id))
            .cloned()
            .collect::<Vec<_>>();
        if let Some(unknown_id) = wanted.into_iter().next() {
            return Err(eyre!(
                "remote shot {unknown_id:?} is not in the current session"
            ));
        }
        let plan = plan_import(selected, &destinations).map_err(|error| eyre!(error))?;
        let mut copied = 0;
        let mut skipped_existing = 0;
        for (asset, destination) in plan {
            let object = session
                .objects
                .get(&asset.object_id)
                .ok_or_else(|| eyre!("MTP object {:?} is no longer retained", asset.object_id))?;
            let mut input = object
                .open_read_stream()
                .wrap_err_with(|| format!("failed to read {}", asset.name))?;
            match copy_reader_no_clobber(&mut input, asset.size, &destination, &asset.name)? {
                CopyOutcome::Copied => copied += 1,
                CopyOutcome::SkippedExisting => skipped_existing += 1,
            }
        }

        // A failed or partial import deliberately leaves previews available for retry.
        for path in &session.preview_paths {
            if let Err(error) = std::fs::remove_file(path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                warn!(
                    "failed to remove MTP preview cache {}: {error}",
                    path.display()
                );
            }
        }
        self.session = None;
        Ok(MtpEvent::ImportFinished {
            copied,
            skipped_existing,
        })
    }
}

impl MtpRequest {
    fn operation(&self) -> &'static str {
        match self {
            Self::ListDevices => "list devices",
            Self::ListSourceFolders { .. } => "list source folders",
            Self::StartSession { .. } => "scan session",
            Self::ImportKept { .. } => "import kept shots",
            Self::Shutdown => "shutdown",
        }
    }
}

fn is_container(object: &Object) -> bool {
    matches!(
        object.object_type(),
        ObjectType::Folder | ObjectType::FunctionalObject
    )
}

fn collect_source_folders(
    container: Object,
    parent_path: String,
    folders: &mut Vec<SourceFolder>,
) -> Result<()> {
    for child in container
        .children()
        .wrap_err("failed to enumerate MTP source folders")?
    {
        if !is_container(&child) {
            continue;
        }
        let name = child.name().to_string_lossy();
        let path = join_remote_path(&parent_path, &name);
        folders.push(SourceFolder {
            id: child.id().to_string_lossy(),
            name,
            path: path.clone(),
        });
        collect_source_folders(child, path, folders)?;
    }
    Ok(())
}

fn find_container(container: Object, target_id: &str) -> Result<Option<Object>> {
    if container.id().to_string_lossy() == target_id {
        return Ok(Some(container));
    }
    for child in container
        .children()
        .wrap_err("failed to enumerate MTP source folders")?
    {
        if is_container(&child)
            && let Some(found) = find_container(child, target_id)?
        {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

fn collect_media(
    container: Object,
    device_id: &str,
    cache_directory: &std::path::Path,
    grouped: &mut BTreeMap<String, Vec<(RemoteAsset, Object)>>,
    preview_paths: &mut Vec<PathBuf>,
) -> Result<()> {
    for child in container
        .children()
        .wrap_err("failed to enumerate MTP media")?
    {
        if is_container(&child) {
            collect_media(child, device_id, cache_directory, grouped, preview_paths)?;
            continue;
        }
        if !child.object_type().is_file_like() {
            continue;
        }
        let name = child.name().to_string_lossy();
        let Some((stem, kind)) = classify_media_name(&name) else {
            continue;
        };
        let object_id = child.id().to_string_lossy();
        let size = child
            .properties(&[WPD_OBJECT_SIZE])
            .wrap_err_with(|| format!("failed to read metadata for {name}"))?
            .get_u64(&WPD_OBJECT_SIZE)
            .wrap_err_with(|| format!("failed to read file size for {name}"))?;
        let preview_path = if kind.is_jpeg() {
            let path = cache_path(cache_directory, device_id, &object_id);
            let mut input = child
                .open_read_stream()
                .wrap_err_with(|| format!("failed to read JPEG companion {name}"))?;
            copy_reader_no_clobber(&mut input, size, &path, &name)?;
            preview_paths.push(path.clone());
            Some(path)
        } else {
            None
        };
        grouped.entry(stem).or_default().push((
            RemoteAsset {
                object_id,
                name,
                kind,
                size,
                preview_path,
            },
            child,
        ));
    }
    Ok(())
}

fn preview_cache_directory() -> Result<PathBuf> {
    let directories = ProjectDirs::from("com", "fruit", "mtp-cull")
        .ok_or_else(|| eyre!("could not determine the local application data directory"))?;
    let directory = directories.data_local_dir().join("mtp-preview-cache");
    std::fs::create_dir_all(&directory)
        .wrap_err_with(|| format!("failed to create {}", directory.display()))?;
    Ok(directory)
}

fn cache_path(directory: &std::path::Path, device_id: &str, object_id: &str) -> PathBuf {
    let key = blake3::hash(format!("{device_id}\0{object_id}").as_bytes());
    directory.join(format!("{key}.jpg"))
}

fn shot_id(device_id: &str, source_folder_id: &str, stem: &str) -> String {
    blake3::hash(format!("{device_id}\0{source_folder_id}\0{stem}").as_bytes())
        .to_hex()
        .to_string()
}

fn join_remote_path(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{parent}/{name}")
    }
}
