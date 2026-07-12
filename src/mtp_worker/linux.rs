use super::{
    MtpDevice, MtpEvent, MtpRequest, RemoteAsset, RemoteSession, RemoteShot, SourceFolder,
    planning::{classify_media_name, plan_import},
};
use crate::safe_copy::{CopyOutcome, CopyWriter};
use color_eyre::eyre::{Result, WrapErr, eyre};
use directories::ProjectDirs;
use futures::executor::block_on;
use log::warn;
use mtp_rs::{MtpDevice as UsbMtpDevice, ObjectHandle, Storage, StorageId};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

/// Handle for the dedicated MTP-over-USB worker thread.
///
/// `mtp-rs` devices, storages, and downloads are created and used only by `WorkerState` on this
/// thread. Requests and events contain only owned data that is safe to pass to the UI thread.
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
            .name("linux-mtp-worker".to_owned())
            .spawn(move || WorkerState::default().run(request_receiver, event_sender))
            .wrap_err("failed to start the Linux MTP worker thread")?;
        Ok(Self {
            requests: request_sender,
            events: event_receiver,
            thread: Some(thread),
        })
    }

    pub fn send(&self, request: MtpRequest) -> Result<()> {
        self.requests
            .send(request)
            .map_err(|_| eyre!("the Linux MTP worker has stopped"))
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
    source_folders: HashMap<String, HashMap<String, FolderReference>>,
    session: Option<PrivateSession>,
}

#[derive(Clone)]
struct FolderReference {
    storage_id: StorageId,
    components: Vec<String>,
}

struct PrivateSession {
    storage: Storage,
    public: RemoteSession,
    objects: HashMap<String, ObjectHandle>,
    cache_directory: PathBuf,
}

impl WorkerState {
    fn run(&mut self, requests: Receiver<MtpRequest>, events: Sender<MtpEvent>) {
        while let Ok(request) = requests.recv() {
            if matches!(request, MtpRequest::Shutdown) {
                return;
            }
            let operation = request.operation();
            let event = match self.handle(request) {
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
                if let Some(previous) = self.session.replace(session) {
                    remove_preview_cache(&previous);
                }
                Ok(MtpEvent::SessionScanned(public))
            }
            MtpRequest::ImportKept {
                shot_ids,
                destinations,
            } => self.import_kept(shot_ids, destinations),
            MtpRequest::Shutdown => unreachable!("shutdown is handled before dispatch"),
        }
    }

    fn list_devices(&self) -> Result<Vec<MtpDevice>> {
        let mut devices = UsbMtpDevice::list_devices()
            .wrap_err("failed to enumerate MTP-mode USB devices")?
            .into_iter()
            .map(|device| MtpDevice {
                id: device_id(device.location_id),
                name: device.display(),
            })
            .collect::<Vec<_>>();
        devices.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
        Ok(devices)
    }

    fn list_source_folders(&mut self, device_id: &str) -> Result<Vec<SourceFolder>> {
        let device = open_device(device_id)?;
        let storages = block_on(device.storages()).wrap_err("failed to enumerate MTP storages")?;
        let mut folders = Vec::new();
        let mut references = HashMap::new();

        for storage in &storages {
            let storage_id = storage.id();
            let name = storage_name(storage);
            let id = folder_id(device_id, storage_id, None);
            folders.push(SourceFolder {
                id: id.clone(),
                name: name.clone(),
                path: name.clone(),
            });
            references.insert(
                id,
                FolderReference {
                    storage_id,
                    components: Vec::new(),
                },
            );
            block_on(collect_source_folders(
                storage,
                device_id,
                storage_id,
                &name,
                &mut folders,
                &mut references,
            ))?;
        }
        folders.sort_by(|left, right| left.path.cmp(&right.path).then(left.id.cmp(&right.id)));
        self.source_folders.insert(device_id.to_owned(), references);
        Ok(folders)
    }

    fn scan_session(
        &mut self,
        device_id: String,
        source_folder_id: String,
    ) -> Result<PrivateSession> {
        let source = self
            .source_folders
            .get(&device_id)
            .and_then(|folders| folders.get(&source_folder_id))
            .cloned()
            .ok_or_else(|| {
                eyre!(
                    "MTP source folder {source_folder_id:?} is unknown; list source folders again"
                )
            })?;
        let device = open_device(&device_id)?;
        let storage = block_on(device.storage(source.storage_id))
            .wrap_err("failed to open the selected MTP storage")?;
        let source_handle = block_on(resolve_folder(&storage, &source.components))?;
        let cache_directory = create_preview_cache_directory()?;
        let mut grouped = BTreeMap::<String, Vec<RemoteAsset>>::new();
        let mut objects = HashMap::new();
        block_on(collect_media(
            &storage,
            &device_id,
            source.storage_id,
            source_handle,
            &cache_directory,
            &mut grouped,
            &mut objects,
        ))?;

        let mut shots = Vec::with_capacity(grouped.len());
        for (stem, mut assets) in grouped {
            assets.sort_by(|left, right| {
                left.name
                    .cmp(&right.name)
                    .then(left.object_id.cmp(&right.object_id))
            });
            shots.push(RemoteShot {
                id: shot_id(&device_id, &source_folder_id, &stem),
                stem,
                assets,
            });
        }
        Ok(PrivateSession {
            storage,
            public: RemoteSession {
                device_id,
                source_folder_id,
                shots,
            },
            objects,
            cache_directory,
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
            let handle = *session
                .objects
                .get(&asset.object_id)
                .ok_or_else(|| eyre!("MTP object {:?} is no longer retained", asset.object_id))?;
            match block_on(copy_windowed_no_clobber(
                &session.storage,
                handle,
                asset.size,
                &destination,
                &asset.name,
            ))? {
                CopyOutcome::Copied => copied += 1,
                CopyOutcome::SkippedExisting => skipped_existing += 1,
            }
        }

        // A failed or partial import deliberately leaves previews available for retry.
        remove_preview_cache(session);
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

fn open_device(id: &str) -> Result<UsbMtpDevice> {
    let location_id = parse_device_id(id)?;
    block_on(UsbMtpDevice::open_by_location(location_id))
        .wrap_err_with(|| format!("failed to open MTP device {id:?}"))
}

async fn collect_source_folders(
    storage: &Storage,
    device_id: &str,
    storage_id: StorageId,
    root_path: &str,
    folders: &mut Vec<SourceFolder>,
    references: &mut HashMap<String, FolderReference>,
) -> Result<()> {
    let mut pending = vec![(None, root_path.to_owned(), Vec::new())];
    while let Some((parent, parent_path, parent_components)) = pending.pop() {
        for object in storage
            .list_objects(parent)
            .await
            .wrap_err("failed to enumerate MTP source folders")?
        {
            if !object.is_folder() {
                continue;
            }
            let name = object.filename;
            let path = join_remote_path(&parent_path, &name);
            let mut components = parent_components.clone();
            components.push(name.clone());
            let id = folder_id(device_id, storage_id, Some(object.handle));
            folders.push(SourceFolder {
                id: id.clone(),
                name,
                path: path.clone(),
            });
            references.insert(
                id,
                FolderReference {
                    storage_id,
                    components: components.clone(),
                },
            );
            pending.push((Some(object.handle), path, components));
        }
    }
    Ok(())
}

async fn resolve_folder(storage: &Storage, components: &[String]) -> Result<Option<ObjectHandle>> {
    let mut parent = None;
    for component in components {
        let folder = storage
            .list_objects(parent)
            .await
            .wrap_err("failed to locate the selected MTP source folder")?
            .into_iter()
            .find(|object| object.is_folder() && object.filename == *component)
            .ok_or_else(|| eyre!("MTP source folder {component:?} no longer exists"))?;
        parent = Some(folder.handle);
    }
    Ok(parent)
}

async fn collect_media(
    storage: &Storage,
    device_id: &str,
    storage_id: StorageId,
    source: Option<ObjectHandle>,
    cache_directory: &Path,
    grouped: &mut BTreeMap<String, Vec<RemoteAsset>>,
    objects: &mut HashMap<String, ObjectHandle>,
) -> Result<()> {
    let mut pending = vec![source];
    while let Some(parent) = pending.pop() {
        for object in storage
            .list_objects(parent)
            .await
            .wrap_err("failed to enumerate MTP media")?
        {
            if object.is_folder() {
                pending.push(Some(object.handle));
                continue;
            }
            let Some((stem, kind)) = classify_media_name(&object.filename) else {
                continue;
            };
            let object_id = object_id(device_id, storage_id, object.handle);
            let preview_path = if kind.is_jpeg() {
                let path = cache_path(cache_directory, device_id, &object_id);
                copy_windowed_no_clobber(
                    storage,
                    object.handle,
                    object.size,
                    &path,
                    &object.filename,
                )
                .await?;
                Some(path)
            } else {
                None
            };
            objects.insert(object_id.clone(), object.handle);
            grouped.entry(stem).or_default().push(RemoteAsset {
                object_id,
                name: object.filename,
                kind,
                size: object.size,
                preview_path,
            });
        }
    }
    Ok(())
}

async fn copy_windowed_no_clobber(
    storage: &Storage,
    handle: ObjectHandle,
    expected_size: u64,
    target_path: &Path,
    source_label: &str,
) -> Result<CopyOutcome> {
    let mut download = storage
        .download_windowed_default(handle)
        .await
        .wrap_err_with(|| format!("failed to read {source_label}"))?;
    let mut writer = CopyWriter::new(expected_size, target_path, source_label)?;
    while let Some(window) = download.next_window().await {
        writer.write(&window.wrap_err_with(|| format!("failed to read {source_label}"))?)?;
    }
    writer.finish()
}

fn storage_name(storage: &Storage) -> String {
    let description = storage.info().description.trim();
    if description.is_empty() {
        format!("Storage {:016x}", storage.id().0)
    } else {
        description.to_owned()
    }
}

fn create_preview_cache_directory() -> Result<PathBuf> {
    let directories = ProjectDirs::from("com", "fruit", "mtp-cull")
        .ok_or_else(|| eyre!("could not determine the local application data directory"))?;
    let root = directories.data_local_dir().join("mtp-preview-cache");
    std::fs::create_dir_all(&root)
        .wrap_err_with(|| format!("failed to create {}", root.display()))?;
    Ok(tempfile::Builder::new()
        .prefix("session-")
        .tempdir_in(root)
        .wrap_err("failed to create an MTP preview session cache")?
        .keep())
}

fn remove_preview_cache(session: &PrivateSession) {
    if let Err(error) = std::fs::remove_dir_all(&session.cache_directory)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        warn!(
            "failed to remove MTP preview cache {}: {error}",
            session.cache_directory.display()
        );
    }
}

fn device_id(location_id: u64) -> String {
    format!("mtp-usb-{location_id:016x}")
}

fn parse_device_id(id: &str) -> Result<u64> {
    let location_id = id
        .strip_prefix("mtp-usb-")
        .ok_or_else(|| eyre!("unknown MTP device identifier {id:?}"))?;
    u64::from_str_radix(location_id, 16).map_err(|_| eyre!("invalid MTP device identifier {id:?}"))
}

fn folder_id(device_id: &str, storage_id: StorageId, handle: Option<ObjectHandle>) -> String {
    let handle = handle.map_or(0, |handle| handle.0);
    blake3::hash(format!("{device_id}\0{}\0{handle}", storage_id.0).as_bytes())
        .to_hex()
        .to_string()
}

fn object_id(device_id: &str, storage_id: StorageId, handle: ObjectHandle) -> String {
    blake3::hash(format!("{device_id}\0{}\0{}", storage_id.0, handle.0).as_bytes())
        .to_hex()
        .to_string()
}

fn cache_path(directory: &Path, device_id: &str, object_id: &str) -> PathBuf {
    let key = blake3::hash(format!("{device_id}\0{object_id}").as_bytes());
    directory.join(format!("{key}.jpg"))
}

fn shot_id(device_id: &str, source_folder_id: &str, stem: &str) -> String {
    blake3::hash(format!("{device_id}\0{source_folder_id}\0{stem}").as_bytes())
        .to_hex()
        .to_string()
}

fn join_remote_path(parent: &str, name: &str) -> String {
    format!("{parent}/{name}")
}

#[cfg(test)]
mod tests {
    use super::{device_id, folder_id, join_remote_path, parse_device_id};
    use mtp_rs::{ObjectHandle, StorageId};

    #[test]
    fn device_identifier_round_trips_a_usb_location() {
        let id = device_id(0x0123_4567_89ab_cdef);
        assert_eq!(parse_device_id(&id).unwrap(), 0x0123_4567_89ab_cdef);
    }

    #[test]
    fn folder_identifiers_include_storage_and_handle() {
        let device = device_id(1);
        let root = folder_id(&device, StorageId(1), None);
        assert_eq!(root, folder_id(&device, StorageId(1), None));
        assert_ne!(root, folder_id(&device, StorageId(2), None));
        assert_ne!(
            root,
            folder_id(&device, StorageId(1), Some(ObjectHandle(1)))
        );
    }

    #[test]
    fn remote_paths_include_the_storage_root_name() {
        assert_eq!(
            join_remote_path("Internal storage", "DCIM"),
            "Internal storage/DCIM"
        );
    }
}
