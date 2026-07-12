use super::{
    MtpCopyError, MtpCopyItem, MtpCopyResult, MtpDevice, MtpEvent, MtpFileInfo, MtpRequest,
    RemoteAsset, RemoteSession, RemoteShot, SourceFolder,
    cache::{
        PreviewCache, create_preview_cache_directory, reconcile_preview_cache, remove_preview_cache,
    },
    check_cancelled, is_cancelled,
    planning::{classify_media_name, normalize_source_path, plan_import},
    send_progress,
};
use crate::mtp_file::MtpFileType;
use crate::safe_copy::{CopyOutcome, copy_reader_no_clobber_with_progress};
use color_eyre::eyre::{Result, WrapErr, eyre};
use log::{info, warn};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use winmtp::PortableDevices::WPD_OBJECT_SIZE;
use winmtp::Provider;
use winmtp::device::{BasicDevice, Device};
use winmtp::object::{Object, ObjectType};

const PROGRESS_INTERVAL_BYTES: u64 = 4 * 1024 * 1024;

/// Handle for the dedicated Windows Portable Devices worker thread.
///
/// Requests and events deliberately contain only owned Rust data. All COM objects stay in
/// `WorkerState`, which is created and destroyed on the worker thread.
pub struct MtpWorker {
    requests: Sender<MtpRequest>,
    events: Receiver<MtpEvent>,
    cancel: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl MtpWorker {
    pub fn spawn() -> Result<Self> {
        let (request_sender, request_receiver) = mpsc::channel();
        let (event_sender, event_receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let thread = thread::Builder::new()
            .name("windows-mtp-worker".to_owned())
            .spawn(move || WorkerState::new(worker_cancel).run(request_receiver, event_sender))
            .wrap_err("failed to start the Windows MTP worker thread")?;
        Ok(Self {
            requests: request_sender,
            events: event_receiver,
            cancel,
            thread: Some(thread),
        })
    }

    pub fn send(&self, request: MtpRequest) -> Result<()> {
        if matches!(&request, MtpRequest::Cancel) {
            self.cancel.store(true, Ordering::Release);
            return Ok(());
        }
        self.cancel.store(false, Ordering::Release);
        self.requests
            .send(request)
            .map_err(|_| eyre!("the Windows MTP worker has stopped"))
    }

    pub fn try_recv(&self) -> Option<MtpEvent> {
        self.events.try_recv().ok()
    }

    pub fn recv(&self) -> Result<MtpEvent> {
        self.events
            .recv()
            .map_err(|_| eyre!("the Windows MTP worker has stopped"))
    }
}

impl Drop for MtpWorker {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        let _ = self.requests.send(MtpRequest::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct WorkerState {
    provider: Option<Provider>,
    session: Option<PrivateSession>,
    legacy_session: Option<LegacySession>,
    cancel: Arc<AtomicBool>,
}

impl WorkerState {
    fn new(cancel: Arc<AtomicBool>) -> Self {
        Self {
            provider: None,
            session: None,
            legacy_session: None,
            cancel,
        }
    }
}

struct PrivateSession {
    _device: Device,
    public: RemoteSession,
    objects: HashMap<String, Object>,
    cache: PreviewCache,
}

struct LegacySession {
    _device: Device,
    objects: HashMap<String, Object>,
}

impl WorkerState {
    fn run(&mut self, requests: Receiver<MtpRequest>, events: Sender<MtpEvent>) {
        match reconcile_preview_cache() {
            Ok(removed) if removed > 0 => {
                info!("reconciled {removed} stale MTP preview cache(s)")
            }
            Ok(_) => {}
            Err(error) => warn!("could not reconcile stale MTP preview caches: {error:#}"),
        }
        while let Ok(request) = requests.recv() {
            if matches!(request, MtpRequest::Shutdown) {
                self.release_sessions();
                return;
            }
            if matches!(request, MtpRequest::Cancel) {
                self.cancel.store(true, Ordering::Release);
                continue;
            }
            let operation = request.operation();
            let result = self.handle(request, &events);
            let event = match result {
                Ok(event) => event,
                Err(error) if is_cancelled(&error) => MtpEvent::Cancelled { operation },
                Err(error) => MtpEvent::Error {
                    operation,
                    message: format!("{error:#}"),
                },
            };
            if events.send(event).is_err() {
                self.release_sessions();
                return;
            }
        }
        self.release_sessions();
    }

    fn handle(&mut self, request: MtpRequest, events: &Sender<MtpEvent>) -> Result<MtpEvent> {
        match request {
            MtpRequest::ListDevices => Ok(MtpEvent::Devices(self.list_devices()?)),
            MtpRequest::ListSourceFolders { device_id } => {
                let folders = self.list_source_folders(&device_id, events)?;
                Ok(MtpEvent::SourceFolders { device_id, folders })
            }
            MtpRequest::StartSession {
                device_id,
                source_folder_id,
            } => {
                let session = self.scan_session(device_id, source_folder_id, events)?;
                let public = session.public.clone();
                if let Some(previous) = self.session.replace(session) {
                    remove_preview_cache(previous.cache);
                }
                self.legacy_session = None;
                Ok(MtpEvent::SessionScanned(public))
            }
            MtpRequest::ImportKept {
                shot_ids,
                destinations,
            } => self.import_kept(shot_ids, destinations, events),
            MtpRequest::ListFiles { device_id, path } => {
                let files = self.list_files(&device_id, path.as_deref(), events)?;
                Ok(MtpEvent::FilesListed(files))
            }
            MtpRequest::CopyFiles { files, keep_going } => {
                self.copy_files(files, keep_going, events)
            }
            MtpRequest::CloseSession => {
                self.close_sessions();
                Ok(MtpEvent::SessionClosed)
            }
            MtpRequest::Cancel | MtpRequest::Shutdown => {
                unreachable!("control request is handled before dispatch")
            }
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
        self.open_device_with_mode(device_id, false)
    }

    fn open_legacy_device(&mut self, device_id: &str) -> Result<Device> {
        self.open_device_with_mode(device_id, true)
    }

    fn open_device_with_mode(&mut self, device_id: &str, read_only: bool) -> Result<Device> {
        let device = self.basic_device(device_id)?;
        device
            .open(&winmtp::make_current_app_identifiers!(), read_only)
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

    fn list_source_folders(
        &mut self,
        device_id: &str,
        events: &Sender<MtpEvent>,
    ) -> Result<Vec<SourceFolder>> {
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
        let mut progress = 0;
        collect_source_folders(
            root,
            String::new(),
            &mut folders,
            &self.cancel,
            events,
            &mut progress,
        )?;
        folders.sort_by(|left, right| left.path.cmp(&right.path).then(left.id.cmp(&right.id)));
        Ok(folders)
    }

    fn scan_session(
        &mut self,
        device_id: String,
        source_folder_id: String,
        events: &Sender<MtpEvent>,
    ) -> Result<PrivateSession> {
        let device = self.open_device(&device_id)?;
        let root = device
            .content()
            .wrap_err("failed to access MTP device content")?
            .root()
            .wrap_err("failed to access MTP device root")?;
        let source = find_container(root, &source_folder_id, &self.cancel)?
            .ok_or_else(|| eyre!("MTP source folder {source_folder_id:?} no longer exists"))?;
        let cache = create_preview_cache_directory()?;
        let mut grouped = BTreeMap::<String, Vec<(RemoteAsset, Object)>>::new();
        let mut progress = 0;
        collect_media(
            source,
            &device_id,
            cache.path(),
            &mut grouped,
            &self.cancel,
            events,
            &mut progress,
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
            cache,
        })
    }

    fn import_kept(
        &mut self,
        shot_ids: Vec<String>,
        destinations: super::planning::ImportPaths,
        events: &Sender<MtpEvent>,
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
        let total_bytes = plan.iter().map(|(asset, _)| asset.size).sum();
        let mut copied = 0;
        let mut skipped_existing = 0;
        let mut completed_bytes = 0;
        for (asset, destination) in plan {
            check_cancelled(&self.cancel)?;
            let object = session
                .objects
                .get(&asset.object_id)
                .ok_or_else(|| eyre!("MTP object {:?} is no longer retained", asset.object_id))?;
            let mut input = object
                .open_read_stream()
                .wrap_err_with(|| format!("failed to read {}", asset.name))?;
            send_progress(
                events,
                &self.cancel,
                "import kept shots",
                completed_bytes,
                Some(total_bytes),
                format!("Importing {}", asset.name),
            )?;
            let mut last_reported = 0;
            match copy_reader_no_clobber_with_progress(
                &mut input,
                asset.size,
                &destination,
                &asset.name,
                |bytes| {
                    check_cancelled(&self.cancel)?;
                    if bytes != asset.size
                        && bytes.saturating_sub(last_reported) < PROGRESS_INTERVAL_BYTES
                    {
                        return Ok(());
                    }
                    last_reported = bytes;
                    send_progress(
                        events,
                        &self.cancel,
                        "import kept shots",
                        completed_bytes + bytes,
                        Some(total_bytes),
                        asset.name.clone(),
                    )
                },
            )? {
                CopyOutcome::Copied => copied += 1,
                CopyOutcome::SkippedExisting => skipped_existing += 1,
            }
            completed_bytes += asset.size;
        }

        // A failed or partial import deliberately leaves previews available for retry.
        let completed = self
            .session
            .take()
            .expect("the imported MTP session still exists");
        remove_preview_cache(completed.cache);
        Ok(MtpEvent::ImportFinished {
            copied,
            skipped_existing,
        })
    }

    fn list_files(
        &mut self,
        device_id: &str,
        path: Option<&str>,
        events: &Sender<MtpEvent>,
    ) -> Result<Vec<MtpFileInfo>> {
        self.close_sessions();
        let device = self.open_legacy_device(device_id)?;
        let root = device
            .content()
            .wrap_err("failed to access MTP device content")?
            .root()
            .wrap_err("failed to access MTP device root")?;
        let source = match path {
            Some(path) if !path.trim_matches(['/', '\\']).is_empty() => root
                .object_by_path(std::path::Path::new(&normalize_source_path(path)?))
                .wrap_err_with(|| format!("failed to find MTP path {path:?}"))?,
            _ => root,
        };
        let root_path = path.unwrap_or_default().trim_matches(['/', '\\']);
        let mut files = Vec::new();
        let mut objects = HashMap::new();
        let mut progress = 0;
        collect_legacy_files(
            source,
            root_path,
            &mut files,
            &mut objects,
            &self.cancel,
            events,
            &mut progress,
        )?;
        files.sort_by(|left, right| {
            left.file_type
                .copy_order()
                .cmp(&right.file_type.copy_order())
                .then(left.name.cmp(&right.name))
        });
        self.legacy_session = Some(LegacySession {
            _device: device,
            objects,
        });
        Ok(files)
    }

    fn copy_files(
        &mut self,
        files: Vec<MtpCopyItem>,
        keep_going: bool,
        events: &Sender<MtpEvent>,
    ) -> Result<MtpEvent> {
        let session = self
            .legacy_session
            .as_ref()
            .ok_or_else(|| eyre!("list MTP files before copying"))?;
        let total_bytes = files.iter().map(|file| file.size).sum();
        let mut completed_bytes = 0;
        let mut copied_files = 0;
        let mut skipped_files = 0;
        let mut copied_bytes = 0;
        let mut errors = Vec::new();

        for file in files {
            check_cancelled(&self.cancel)?;
            send_progress(
                events,
                &self.cancel,
                "copy files",
                completed_bytes,
                Some(total_bytes),
                format!(
                    "Copying {} to {}",
                    file.source_path,
                    file.destination.display()
                ),
            )?;
            let result = (|| {
                let object = session.objects.get(&file.object_id).ok_or_else(|| {
                    eyre!("MTP object {:?} is no longer retained", file.object_id)
                })?;
                let mut input = object
                    .open_read_stream()
                    .wrap_err_with(|| format!("failed to read {}", file.source_path))?;
                let mut last_reported = 0;
                copy_reader_no_clobber_with_progress(
                    &mut input,
                    file.size,
                    &file.destination,
                    &file.source_path,
                    |bytes| {
                        check_cancelled(&self.cancel)?;
                        if bytes != file.size
                            && bytes.saturating_sub(last_reported) < PROGRESS_INTERVAL_BYTES
                        {
                            return Ok(());
                        }
                        last_reported = bytes;
                        send_progress(
                            events,
                            &self.cancel,
                            "copy files",
                            completed_bytes + bytes,
                            Some(total_bytes),
                            file.source_path.clone(),
                        )
                    },
                )
            })();
            match result {
                Ok(CopyOutcome::Copied) => {
                    copied_files += 1;
                    copied_bytes += file.size;
                }
                Ok(CopyOutcome::SkippedExisting) => skipped_files += 1,
                Err(error) if is_cancelled(&error) => return Err(error),
                Err(error) if keep_going => errors.push(MtpCopyError {
                    source_path: file.source_path,
                    message: format!("{error:#}"),
                }),
                Err(error) => return Err(error),
            }
            completed_bytes += file.size;
        }

        Ok(MtpEvent::CopyFinished(MtpCopyResult {
            copied_files,
            skipped_files,
            copied_bytes,
            total_bytes,
            errors,
        }))
    }

    fn close_sessions(&mut self) {
        if let Some(session) = self.session.take() {
            remove_preview_cache(session.cache);
        }
        self.legacy_session = None;
    }

    fn release_sessions(&mut self) {
        self.session = None;
        self.legacy_session = None;
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
    cancel: &AtomicBool,
    events: &Sender<MtpEvent>,
    progress: &mut u64,
) -> Result<()> {
    check_cancelled(cancel)?;
    for child in container
        .children()
        .wrap_err("failed to enumerate MTP source folders")?
    {
        check_cancelled(cancel)?;
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
        *progress += 1;
        send_progress(
            events,
            cancel,
            "list source folders",
            *progress,
            None,
            path.clone(),
        )?;
        collect_source_folders(child, path, folders, cancel, events, progress)?;
    }
    Ok(())
}

fn find_container(
    container: Object,
    target_id: &str,
    cancel: &AtomicBool,
) -> Result<Option<Object>> {
    check_cancelled(cancel)?;
    if container.id().to_string_lossy() == target_id {
        return Ok(Some(container));
    }
    for child in container
        .children()
        .wrap_err("failed to enumerate MTP source folders")?
    {
        check_cancelled(cancel)?;
        if is_container(&child)
            && let Some(found) = find_container(child, target_id, cancel)?
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
    cancel: &AtomicBool,
    events: &Sender<MtpEvent>,
    progress: &mut u64,
) -> Result<()> {
    check_cancelled(cancel)?;
    for child in container
        .children()
        .wrap_err("failed to enumerate MTP media")?
    {
        check_cancelled(cancel)?;
        if is_container(&child) {
            collect_media(
                child,
                device_id,
                cache_directory,
                grouped,
                cancel,
                events,
                progress,
            )?;
            continue;
        }
        if !child.object_type().is_file_like() {
            continue;
        }
        let name = child.name().to_string_lossy().into_owned();
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
            copy_reader_no_clobber_with_progress(&mut input, size, &path, &name, |_bytes| {
                check_cancelled(cancel)
            })?;
            Some(path)
        } else {
            None
        };
        grouped.entry(stem).or_default().push((
            RemoteAsset {
                object_id,
                name: name.clone(),
                kind,
                size,
                preview_path,
            },
            child,
        ));
        *progress += 1;
        send_progress(events, cancel, "scan session", *progress, None, name)?;
    }
    Ok(())
}

fn collect_legacy_files(
    container: Object,
    parent_path: &str,
    files: &mut Vec<MtpFileInfo>,
    objects: &mut HashMap<String, Object>,
    cancel: &AtomicBool,
    events: &Sender<MtpEvent>,
    progress: &mut u64,
) -> Result<()> {
    check_cancelled(cancel)?;
    for child in container
        .children()
        .wrap_err("failed to enumerate MTP objects")?
    {
        check_cancelled(cancel)?;
        let name = child.name().to_string_lossy().into_owned();
        let path = join_remote_path(parent_path, &name);
        if child.object_type() == ObjectType::Folder {
            collect_legacy_files(child, &path, files, objects, cancel, events, progress)?;
            continue;
        }
        if !child.object_type().is_file_like() {
            continue;
        }
        let Ok(file_type) = MtpFileType::try_from_file_name(&name) else {
            continue;
        };
        let size = child
            .properties(&[WPD_OBJECT_SIZE])
            .wrap_err_with(|| format!("failed to read metadata for {path}"))?
            .get_u64(&WPD_OBJECT_SIZE)
            .wrap_err_with(|| format!("failed to read 64-bit file size for {path}"))?;
        let object_id = child.id().to_string_lossy();
        objects.insert(object_id.clone(), child);
        files.push(MtpFileInfo {
            object_id,
            name,
            path: path.clone(),
            file_type,
            size,
        });
        *progress += 1;
        send_progress(events, cancel, "list files", *progress, None, path)?;
    }
    Ok(())
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
