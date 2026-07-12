use super::{
    MtpCopyError, MtpCopyItem, MtpCopyResult, MtpDevice, MtpEvent, MtpFileInfo, MtpRequest,
    ProgressReporter, RemoteAsset, RemoteSession, RemoteShot, SourceFolder,
    cache::{
        PreviewCache, create_preview_cache_directory, reconcile_preview_cache, remove_preview_cache,
    },
    check_cancelled, is_cancelled,
    planning::{classify_media_name, normalize_source_path, plan_import},
    send_progress,
};
use crate::mtp_file::MtpFileType;
use crate::safe_copy::{CopyOutcome, CopyWriter};
use color_eyre::eyre::{Report, Result, WrapErr, eyre};
use futures::executor::block_on;
use log::{info, warn};
use mtp_rs::{CancelToken, MtpDevice as UsbMtpDevice, ObjectHandle, Storage, StorageId};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

/// Handle for the dedicated MTP-over-USB worker thread.
///
/// `mtp-rs` devices, storages, and downloads are created and used only by `WorkerState` on this
/// thread. Requests and events contain only owned data that is safe to pass to the UI thread.
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
            .name("linux-mtp-worker".to_owned())
            .spawn(move || WorkerState::new(worker_cancel).run(request_receiver, event_sender))
            .wrap_err("failed to start the Linux MTP worker thread")?;
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
            .map_err(|_| eyre!("the Linux MTP worker has stopped"))
    }

    pub fn try_recv(&self) -> Option<MtpEvent> {
        self.events.try_recv().ok()
    }

    pub fn recv(&self) -> Result<MtpEvent> {
        self.events
            .recv()
            .map_err(|_| eyre!("the Linux MTP worker has stopped"))
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
    source_folders: HashMap<String, HashMap<String, FolderReference>>,
    session: Option<PrivateSession>,
    legacy_session: Option<LegacySession>,
    cancel: Arc<AtomicBool>,
    cancel_token: CancelToken,
}

impl WorkerState {
    fn new(cancel: Arc<AtomicBool>) -> Self {
        Self {
            source_folders: HashMap::new(),
            session: None,
            legacy_session: None,
            cancel_token: CancelToken::from_arc(Arc::clone(&cancel)),
            cancel,
        }
    }
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
    cache: PreviewCache,
}

struct LegacySession {
    storages: HashMap<StorageId, Storage>,
    objects: HashMap<String, (StorageId, ObjectHandle)>,
}

struct OperationContext<'cancel, 'progress> {
    cancel_token: &'cancel CancelToken,
    progress: &'progress mut ProgressReporter<'cancel>,
}

struct MediaCollection<'a, 'b> {
    device_id: &'a str,
    storage_id: StorageId,
    cache_directory: &'a Path,
    grouped: &'a mut BTreeMap<String, Vec<RemoteAsset>>,
    objects: &'a mut HashMap<String, ObjectHandle>,
    progress: &'a mut ProgressReporter<'b>,
    cancel_token: &'b CancelToken,
}

struct LegacyCollection<'a, 'b> {
    device_id: &'a str,
    storage_id: StorageId,
    root_path: &'a str,
    files: &'a mut Vec<MtpFileInfo>,
    objects: &'a mut HashMap<String, (StorageId, ObjectHandle)>,
    progress: &'a mut ProgressReporter<'b>,
    cancel_token: &'b CancelToken,
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
            let event = match self.handle(request, &events) {
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

    fn list_source_folders(
        &mut self,
        device_id: &str,
        events: &Sender<MtpEvent>,
    ) -> Result<Vec<SourceFolder>> {
        let device = open_device(device_id)?;
        let storages = block_on(device.storages()).wrap_err("failed to enumerate MTP storages")?;
        let mut folders = Vec::new();
        let mut references = HashMap::new();
        let mut progress = ProgressReporter::new(events, &self.cancel, "list source folders");
        let mut operation = OperationContext {
            cancel_token: &self.cancel_token,
            progress: &mut progress,
        };

        for storage in &storages {
            check_cancelled(&self.cancel)?;
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
                &mut operation,
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
        events: &Sender<MtpEvent>,
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
        let source_handle = block_on(resolve_folder(
            &storage,
            &source.components,
            &self.cancel,
            &self.cancel_token,
        ))?;
        let cache = create_preview_cache_directory()?;
        let mut grouped = BTreeMap::<String, Vec<RemoteAsset>>::new();
        let mut objects = HashMap::new();
        let mut progress = ProgressReporter::new(events, &self.cancel, "scan session");
        let mut collection = MediaCollection {
            device_id: &device_id,
            storage_id: source.storage_id,
            cache_directory: cache.path(),
            grouped: &mut grouped,
            objects: &mut objects,
            progress: &mut progress,
            cancel_token: &self.cancel_token,
        };
        let result = block_on(collect_media(&storage, source_handle, &mut collection));
        result?;

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
            let handle = *session
                .objects
                .get(&asset.object_id)
                .ok_or_else(|| eyre!("MTP object {:?} is no longer retained", asset.object_id))?;
            send_progress(
                events,
                &self.cancel,
                "import kept shots",
                completed_bytes,
                Some(total_bytes),
                format!("Importing {}", asset.name),
            )?;
            match block_on(copy_windowed_no_clobber(
                &session.storage,
                handle,
                asset.size,
                &destination,
                &asset.name,
                |bytes| {
                    send_progress(
                        events,
                        &self.cancel,
                        "import kept shots",
                        completed_bytes + bytes,
                        Some(total_bytes),
                        asset.name.clone(),
                    )
                },
            ))? {
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
        let device = open_device(device_id)?;
        let storages = block_on(device.storages()).wrap_err("failed to enumerate MTP storages")?;
        let requested = path.map(normalize_source_path).transpose()?;
        let mut files = Vec::new();
        let mut storage_objects = HashMap::new();
        let mut legacy_storages = HashMap::new();
        let mut matched_storage = false;
        let mut progress = ProgressReporter::new(events, &self.cancel, "list files");

        for storage in storages {
            check_cancelled(&self.cancel)?;
            let storage_id = storage.id();
            let storage_name = storage_name(&storage);
            let (source, root_path) = match &requested {
                None => (None, storage_name.clone()),
                Some(path) => {
                    let components = path.components().collect::<Vec<_>>();
                    if components
                        .first()
                        .and_then(|component| component.as_os_str().to_str())
                        != Some(storage_name.as_str())
                    {
                        continue;
                    }
                    matched_storage = true;
                    let folder_components = components[1..]
                        .iter()
                        .map(|component| component.as_os_str().to_string_lossy().into_owned())
                        .collect::<Vec<_>>();
                    let source = block_on(resolve_folder(
                        &storage,
                        &folder_components,
                        &self.cancel,
                        &self.cancel_token,
                    ))?;
                    (source, path.to_string_lossy().into_owned())
                }
            };
            let mut collection = LegacyCollection {
                device_id,
                storage_id,
                root_path: &root_path,
                files: &mut files,
                objects: &mut storage_objects,
                progress: &mut progress,
                cancel_token: &self.cancel_token,
            };
            block_on(collect_legacy_media(&storage, source, &mut collection))?;
            legacy_storages.insert(storage_id, storage);
        }

        if requested.is_some() && !matched_storage {
            return Err(eyre!(
                "failed to find MTP path {:?}",
                path.unwrap_or_default()
            ));
        }

        files.sort_by(|left, right| {
            left.file_type
                .copy_order()
                .cmp(&right.file_type.copy_order())
                .then(left.name.cmp(&right.name))
        });
        self.legacy_session = Some(LegacySession {
            storages: legacy_storages,
            objects: storage_objects,
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
                let (storage_id, handle) =
                    *session.objects.get(&file.object_id).ok_or_else(|| {
                        eyre!("MTP object {:?} is no longer retained", file.object_id)
                    })?;
                let storage = session
                    .storages
                    .get(&storage_id)
                    .ok_or_else(|| eyre!("MTP storage {:?} is no longer retained", storage_id.0))?;
                block_on(copy_windowed_no_clobber(
                    storage,
                    handle,
                    file.size,
                    &file.destination,
                    &file.source_path,
                    |bytes| {
                        send_progress(
                            events,
                            &self.cancel,
                            "copy files",
                            completed_bytes + bytes,
                            Some(total_bytes),
                            file.source_path.clone(),
                        )
                    },
                ))
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

fn open_device(id: &str) -> Result<UsbMtpDevice> {
    let location_id = parse_device_id(id)?;
    block_on(UsbMtpDevice::open_by_location(location_id))
        .wrap_err_with(|| format!("failed to open MTP device {id:?}"))
}

fn map_mtp_error(error: mtp_rs::Error, context: &str) -> Report {
    if matches!(error, mtp_rs::Error::Cancelled) {
        Report::new(super::OperationCancelled)
    } else {
        eyre!("{context}: {error}")
    }
}

async fn collect_source_folders(
    storage: &Storage,
    device_id: &str,
    storage_id: StorageId,
    root_path: &str,
    folders: &mut Vec<SourceFolder>,
    references: &mut HashMap<String, FolderReference>,
    operation: &mut OperationContext<'_, '_>,
) -> Result<()> {
    let mut pending = vec![(None, root_path.to_owned(), Vec::new())];
    while let Some((parent, parent_path, parent_components)) = pending.pop() {
        operation.progress.check()?;
        for object in storage
            .list_objects_with_cancel(parent, Some(operation.cancel_token))
            .await
            .map_err(|error| map_mtp_error(error, "failed to enumerate MTP source folders"))?
        {
            operation.progress.check()?;
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
            operation.progress.step(
                folders
                    .last()
                    .map_or("folder", |folder| folder.path.as_str()),
            )?;
        }
    }
    Ok(())
}

async fn resolve_folder(
    storage: &Storage,
    components: &[String],
    cancel: &AtomicBool,
    cancel_token: &CancelToken,
) -> Result<Option<ObjectHandle>> {
    let mut parent = None;
    for component in components {
        check_cancelled(cancel)?;
        let folder = storage
            .list_objects_with_cancel(parent, Some(cancel_token))
            .await
            .map_err(|error| {
                map_mtp_error(error, "failed to locate the selected MTP source folder")
            })?
            .into_iter()
            .find(|object| object.is_folder() && object.filename == *component)
            .ok_or_else(|| eyre!("MTP source folder {component:?} no longer exists"))?;
        parent = Some(folder.handle);
    }
    Ok(parent)
}

async fn collect_media(
    storage: &Storage,
    source: Option<ObjectHandle>,
    collection: &mut MediaCollection<'_, '_>,
) -> Result<()> {
    let mut pending = vec![source];
    while let Some(parent) = pending.pop() {
        collection.progress.check()?;
        for object in storage
            .list_objects_with_cancel(parent, Some(collection.cancel_token))
            .await
            .map_err(|error| map_mtp_error(error, "failed to enumerate MTP media"))?
        {
            collection.progress.check()?;
            if object.is_folder() {
                pending.push(Some(object.handle));
                continue;
            }
            let Some((stem, kind)) = classify_media_name(&object.filename) else {
                continue;
            };
            let object_id = object_id(collection.device_id, collection.storage_id, object.handle);
            let preview_path = if kind.is_jpeg() {
                let path = cache_path(collection.cache_directory, collection.device_id, &object_id);
                copy_windowed_no_clobber(
                    storage,
                    object.handle,
                    object.size,
                    &path,
                    &object.filename,
                    |bytes| {
                        let _ = bytes;
                        collection.progress.check()
                    },
                )
                .await?;
                Some(path)
            } else {
                None
            };
            collection.objects.insert(object_id.clone(), object.handle);
            let filename = object.filename;
            collection
                .grouped
                .entry(stem)
                .or_default()
                .push(RemoteAsset {
                    object_id,
                    name: filename.clone(),
                    kind,
                    size: object.size,
                    preview_path,
                });
            collection.progress.step(filename)?;
        }
    }
    Ok(())
}

async fn collect_legacy_media(
    storage: &Storage,
    source: Option<ObjectHandle>,
    collection: &mut LegacyCollection<'_, '_>,
) -> Result<()> {
    let mut pending = vec![(source, collection.root_path.to_owned())];
    while let Some((parent, parent_path)) = pending.pop() {
        collection.progress.check()?;
        for object in storage
            .list_objects_with_cancel(parent, Some(collection.cancel_token))
            .await
            .map_err(|error| map_mtp_error(error, "failed to enumerate MTP objects"))?
        {
            collection.progress.check()?;
            let path = join_remote_path(&parent_path, &object.filename);
            if object.is_folder() {
                pending.push((Some(object.handle), path));
                continue;
            }
            let Ok(file_type) = MtpFileType::try_from_file_name(&object.filename) else {
                continue;
            };
            let object_id = object_id(collection.device_id, collection.storage_id, object.handle);
            collection
                .objects
                .insert(object_id.clone(), (collection.storage_id, object.handle));
            collection.files.push(MtpFileInfo {
                object_id,
                name: object.filename,
                path: path.clone(),
                file_type,
                size: object.size,
            });
            collection.progress.step(path)?;
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
    mut on_progress: impl FnMut(u64) -> Result<()>,
) -> Result<CopyOutcome> {
    let mut download = storage
        .download_windowed_default(handle)
        .await
        .wrap_err_with(|| format!("failed to read {source_label}"))?;
    let mut writer = CopyWriter::new(expected_size, target_path, source_label)?;
    let mut copied = 0;
    while let Some(window) = download.next_window().await {
        let window = window.wrap_err_with(|| format!("failed to read {source_label}"))?;
        copied += u64::try_from(window.len()).expect("window length exceeds u64");
        writer.write(&window)?;
        on_progress(copied)?;
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
