use super::ProgressReporter;
use super::{
    MtpCopyError, MtpCopyItem, MtpCopyResult, MtpDevice, MtpEvent, MtpFileInfo, RemoteAsset,
    RemoteSession, RemoteShot, SourceFolder, check_cancelled, is_cancelled,
    planning::{classify_media_name, normalize_source_path, plan_import},
    send_progress,
};
use crate::mtp_file::MtpFileType;
use crate::safe_copy::{CopyOutcome, CopyWriter, copy_reader_verified};
use color_eyre::eyre::{Result, WrapErr, bail, eyre};
use futures::executor::block_on;
use mtp_rs::{
    MtpDevice as UsbMtpDevice, ObjectCollection, ObjectHandle, ObjectInfo, Storage, StorageId,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Device handles remain exclusively owned by the runtime's device thread.
pub struct MtpWorker(super::runtime::Worker);
impl MtpWorker {
    pub fn spawn() -> Result<Self> {
        Ok(Self(super::runtime::Worker::spawn(WorkerState::default)?))
    }
    pub fn send(&self, request: super::MtpRequest) -> Result<()> {
        self.0.send(request)
    }
    pub fn recv(&self) -> Result<MtpEvent> {
        self.0.recv()
    }
    pub fn try_recv(&self) -> Option<MtpEvent> {
        self.0.try_recv()
    }
}

#[derive(Default)]
struct WorkerState {
    source_folders: HashMap<String, HashMap<String, FolderReference>>,
    session: Option<PrivateSession>,
    legacy_session: Option<LegacySession>,
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

struct LegacySession {
    storages: HashMap<StorageId, Storage>,
    objects: HashMap<String, (StorageId, ObjectHandle)>,
}

impl WorkerState {
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
        let cache_directory = super::staging::directory(&device_id, &source_folder_id)?;
        let nonce = super::staging::nonce();
        let mut grouped = BTreeMap::<String, Vec<RemoteAsset>>::new();
        let mut objects = HashMap::new();
        block_on(collect_media(
            &storage,
            &device_id,
            source.storage_id,
            source_handle,
            &mut grouped,
            &mut objects,
        ))?;

        let mut shots = Vec::with_capacity(grouped.len());
        for (stem, mut assets) in grouped {
            for asset in &mut assets {
                asset.cache_key =
                    super::staging::asset_key(&device_id, &source_folder_id, asset, &nonce);
            }
            assets.sort_by(|left, right| {
                left.name
                    .cmp(&right.name)
                    .then(left.object_id.cmp(&right.object_id))
            });
            shots.push(RemoteShot {
                id: shot_id(&device_id, &source_folder_id, &stem),
                stem: stem.rsplit('\0').next().unwrap_or(&stem).to_owned(),
                assets,
            });
        }
        let mut public = RemoteSession {
            device_id,
            source_folder_id,
            shots,
        };
        super::staging::restore(&mut public, &cache_directory)?;
        Ok(PrivateSession {
            storage,
            public,
            objects,
            cache_directory,
        })
    }

    fn import_assets(
        &mut self,
        object_ids: Vec<String>,
        destinations: super::planning::ImportPaths,
        progress: &mut dyn FnMut(String, u64, u64),
        cancel: &AtomicBool,
    ) -> Result<MtpEvent> {
        let session = self
            .session
            .as_mut()
            .ok_or_else(|| eyre!("start an MTP session before importing"))?;
        let mut wanted = object_ids.into_iter().collect::<HashSet<_>>();
        let selected = session
            .public
            .shots
            .iter()
            .cloned()
            .filter_map(|mut shot| {
                shot.assets.retain(|asset| wanted.remove(&asset.object_id));
                (!shot.assets.is_empty()).then_some(shot)
            })
            .collect::<Vec<_>>();
        if let Some(id) = wanted.into_iter().next() {
            bail!("remote asset {id:?} is not in the current session");
        }
        let plan = plan_import(selected, &destinations).map_err(|error| eyre!(error))?;
        let total = plan.iter().map(|(a, _)| a.size).sum();
        let mut completed = 0;
        let mut copied = 0;
        let mut skipped_existing = 0;
        for (asset, destination) in plan {
            let mut tick = |done| {
                if cancel.load(Ordering::Relaxed) {
                    bail!("Import cancelled; completed files and camera previews are retained");
                }
                progress(asset.name.clone(), completed + done, total);
                Ok(())
            };
            let outcome = if let Some(path) = &asset.preview_path {
                let mut input = std::fs::File::open(path)?;
                let expected = super::staging::verified(path, &asset)
                    .ok_or_else(|| eyre!("cached JPEG verification failed for {}", asset.name))?;
                copy_reader_verified(
                    &mut input,
                    asset.size,
                    &destination,
                    &asset.name,
                    Some(&expected),
                    &mut tick,
                )?
                .0
            } else {
                let handle = *session.objects.get(&asset.object_id).ok_or_else(|| {
                    eyre!("MTP object {:?} is no longer retained", asset.object_id)
                })?;
                block_on(copy_windowed_no_clobber(
                    &session.storage,
                    handle,
                    asset.size,
                    &destination,
                    &asset.name,
                    &mut tick,
                ))?
                .0
            };
            completed += asset.size;
            match outcome {
                CopyOutcome::Copied => copied += 1,
                CopyOutcome::SkippedExisting => skipped_existing += 1,
            }
        }
        Ok(MtpEvent::ImportFinished {
            copied,
            skipped_existing,
        })
    }

    fn cache_preview(
        &mut self,
        shot_id: String,
        progress: &mut dyn FnMut(String, u64, u64),
        cancel: &AtomicBool,
    ) -> Result<MtpEvent> {
        match self.cache_preview_inner(&shot_id, progress, cancel) {
            Ok(preview_path) => Ok(MtpEvent::PreviewCached {
                shot_id,
                preview_path,
            }),
            Err(error) => Ok(MtpEvent::PreviewFailed {
                shot_id,
                message: format!("{error:#}"),
            }),
        }
    }

    fn cache_preview_inner(
        &mut self,
        shot_id: &str,
        progress: &mut dyn FnMut(String, u64, u64),
        cancel: &AtomicBool,
    ) -> Result<Option<PathBuf>> {
        let session = self
            .session
            .as_mut()
            .ok_or_else(|| eyre!("start an MTP session before caching previews"))?;
        let asset = session
            .public
            .shots
            .iter()
            .find(|shot| shot.id == shot_id)
            .ok_or_else(|| eyre!("MTP shot {:?} is no longer retained", shot_id))?
            .assets
            .iter()
            .find(|asset| asset.kind.is_jpeg())
            .cloned();
        let Some(asset) = asset else {
            return Ok(None);
        };
        let path = super::staging::path(&session.cache_directory, &asset);
        if super::staging::verified(&path, &asset).is_none() {
            super::staging::discard_invalid(&path)?;
            let handle = *session
                .objects
                .get(&asset.object_id)
                .ok_or_else(|| eyre!("camera object disappeared"))?;
            let (_, hash) = block_on(copy_windowed_no_clobber(
                &session.storage,
                handle,
                asset.size,
                &path,
                &asset.name,
                &mut |done| {
                    if cancel.load(Ordering::Relaxed) {
                        bail!("Staging cancelled");
                    }
                    progress(asset.name.clone(), done, asset.size);
                    Ok(())
                },
            ))?;
            super::staging::record(&path, &asset, hash)?;
        }
        if let Some(shot) = session
            .public
            .shots
            .iter_mut()
            .find(|shot| shot.id == shot_id)
            && let Some(preview) = shot
                .assets
                .iter_mut()
                .find(|candidate| candidate.object_id == asset.object_id)
        {
            preview.preview_path = Some(path.clone());
        }
        Ok(Some(path))
    }
}

impl WorkerState {
    fn list_files(
        &mut self,
        device_id: &str,
        path: Option<&str>,
        events: &mut super::EventSink<'_>,
        cancel: &AtomicBool,
    ) -> Result<Vec<MtpFileInfo>> {
        self.session = None;
        self.legacy_session = None;
        let device = open_device(device_id)?;
        let storages = block_on(device.storages()).wrap_err("failed to enumerate MTP storages")?;
        let requested = path.map(normalize_source_path).transpose()?;
        let mut files = Vec::new();
        let mut storage_objects = HashMap::new();
        let mut legacy_storages = HashMap::new();
        let mut matched_storage = false;
        let mut progress = ProgressReporter::new(events, cancel, "list files");

        for storage in storages {
            check_cancelled(cancel)?;
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
                    let source = block_on(resolve_folder(&storage, &folder_components))?;
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
        events: &mut super::EventSink<'_>,
        cancel: &AtomicBool,
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
            check_cancelled(cancel)?;
            send_progress(
                events,
                cancel,
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
                    &mut |bytes| {
                        send_progress(
                            events,
                            cancel,
                            "copy files",
                            completed_bytes + bytes,
                            Some(total_bytes),
                            file.source_path.clone(),
                        )
                    },
                ))
                .map(|(outcome, _)| outcome)
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
}

impl super::runtime::Backend for WorkerState {
    fn devices(&mut self) -> Result<Vec<MtpDevice>> {
        self.list_devices()
    }
    fn folders(&mut self, id: &str) -> Result<Vec<SourceFolder>> {
        self.list_source_folders(id)
    }
    fn start(&mut self, device: String, folder: String) -> Result<RemoteSession> {
        let session = self.scan_session(device, folder)?;
        let public = session.public.clone();
        self.session = Some(session);
        Ok(public)
    }
    fn session(&self) -> Option<&RemoteSession> {
        self.session.as_ref().map(|s| &s.public)
    }
    fn preview(
        &mut self,
        id: String,
        progress: &mut dyn FnMut(String, u64, u64),
        cancel: &AtomicBool,
    ) -> Result<MtpEvent> {
        self.cache_preview(id, progress, cancel)
    }
    fn import(
        &mut self,
        ids: Vec<String>,
        destinations: super::ImportPaths,
        progress: &mut dyn FnMut(String, u64, u64),
        cancel: &AtomicBool,
    ) -> Result<MtpEvent> {
        self.import_assets(ids, destinations, progress, cancel)
    }
    fn files(
        &mut self,
        device: &str,
        path: Option<&str>,
        events: &mut super::EventSink<'_>,
        cancel: &AtomicBool,
    ) -> Result<Vec<MtpFileInfo>> {
        self.list_files(device, path, events, cancel)
    }
    fn copy(
        &mut self,
        files: Vec<MtpCopyItem>,
        keep_going: bool,
        events: &mut super::EventSink<'_>,
        cancel: &AtomicBool,
    ) -> Result<MtpEvent> {
        self.copy_files(files, keep_going, events, cancel)
    }
    fn close(&mut self) {
        self.legacy_session = None;
        self.session = None;
    }
}

fn open_device(id: &str) -> Result<UsbMtpDevice> {
    let location_id = parse_device_id(id)?;
    block_on(UsbMtpDevice::open_by_location(location_id))
        .wrap_err_with(|| format!("failed to open MTP device {id:?}"))
}

/// Preserve complete enumeration: mtp-rs 0.32's list_objects silently omits
/// recoverable per-object metadata failures, which could hide photos from review.
async fn list_objects_complete(
    storage: &Storage,
    parent: Option<ObjectHandle>,
) -> Result<Vec<ObjectInfo>> {
    complete_objects(storage.collect_objects(parent).await?)
}

fn complete_objects(collection: ObjectCollection) -> Result<Vec<ObjectInfo>> {
    if let Some(skipped) = collection.skipped.first() {
        bail!(
            "incomplete MTP listing: {} objects could not be read; handle {}: {}",
            collection.skipped.len(),
            skipped.handle.0,
            skipped.error
        );
    }
    Ok(collection.objects)
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
        for object in list_objects_complete(storage, parent)
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
        let folder = list_objects_complete(storage, parent)
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
    grouped: &mut BTreeMap<String, Vec<RemoteAsset>>,
    objects: &mut HashMap<String, ObjectHandle>,
) -> Result<()> {
    let mut pending = vec![(source, String::new())];
    while let Some((parent, parent_path)) = pending.pop() {
        for object in list_objects_complete(storage, parent)
            .await
            .wrap_err("failed to enumerate MTP media")?
        {
            if object.is_folder() {
                pending.push((
                    Some(object.handle),
                    join_remote_path(&parent_path, &object.filename),
                ));
                continue;
            }
            let Some((stem, kind)) = classify_media_name(&object.filename) else {
                continue;
            };
            let object_id = object_id(device_id, storage_id, object.handle);
            let preview_path = None;
            objects.insert(object_id.clone(), object.handle);
            let source_path = join_remote_path(&parent_path, &object.filename);
            let modified = object
                .modified
                .as_ref()
                .or(object.created.as_ref())
                .map(|t| format!("{t:?}"));
            grouped
                .entry(format!("{parent_path}\0{stem}"))
                .or_default()
                .push(RemoteAsset {
                    object_id,
                    name: object.filename,
                    kind,
                    size: object.size,
                    preview_path,
                    source_path,
                    modified,
                    cache_key: String::new(),
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
    progress: &mut dyn FnMut(u64) -> Result<()>,
) -> Result<(CopyOutcome, String)> {
    progress(0)?;
    let mut download = storage
        .download_windowed_default(handle)
        .await
        .wrap_err_with(|| format!("failed to read {source_label}"))?;
    let mut writer = CopyWriter::new(expected_size, target_path, source_label)?;
    let mut copied = 0;
    while let Some(window) = download.next_window().await {
        let bytes = window.wrap_err_with(|| format!("failed to read {source_label}"))?;
        writer.write(&bytes)?;
        copied += bytes.len() as u64;
        progress(copied)?;
    }
    progress(copied)?;
    let hash = writer.content_hash().to_hex().to_string();
    Ok((writer.finish()?, hash))
}

fn storage_name(storage: &Storage) -> String {
    let description = storage.info().description.trim();
    if description.is_empty() {
        format!("Storage {:016x}", storage.id().0)
    } else {
        description.to_owned()
    }
}

fn shot_id(device_id: &str, source_folder_id: &str, stem: &str) -> String {
    blake3::hash(format!("{device_id}\0{source_folder_id}\0{stem}").as_bytes())
        .to_hex()
        .to_string()
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

fn join_remote_path(parent: &str, name: &str) -> String {
    format!("{parent}/{name}")
}

struct LegacyCollection<'a, 'b, 'sink> {
    device_id: &'a str,
    storage_id: StorageId,
    root_path: &'a str,
    files: &'a mut Vec<MtpFileInfo>,
    objects: &'a mut HashMap<String, (StorageId, ObjectHandle)>,
    progress: &'a mut ProgressReporter<'b, 'sink>,
}
async fn collect_legacy_media(
    storage: &Storage,
    source: Option<ObjectHandle>,
    collection: &mut LegacyCollection<'_, '_, '_>,
) -> Result<()> {
    let mut pending = vec![(source, collection.root_path.to_owned())];
    while let Some((parent, parent_path)) = pending.pop() {
        collection.progress.check()?;
        for object in list_objects_complete(storage, parent).await? {
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

#[cfg(test)]
mod tests {
    use super::{complete_objects, device_id, folder_id, join_remote_path, parse_device_id};
    use mtp_rs::{Error, ObjectCollection, ObjectHandle, ObjectInfo, SkippedObject, StorageId};

    #[test]
    fn incomplete_listing_does_not_silently_hide_unreadable_objects() {
        let result = complete_objects(ObjectCollection {
            objects: vec![ObjectInfo::default()],
            skipped: vec![SkippedObject {
                handle: ObjectHandle(42),
                error: Error::AccessDenied,
            }],
        });
        let message = result.unwrap_err().to_string();
        assert!(message.contains("incomplete MTP listing"));
        assert!(message.contains("handle 42"));
        assert!(message.contains("access denied"));
    }

    #[test]
    fn complete_listing_preserves_objects_and_accepts_empty_folders() {
        let object = ObjectInfo::default();
        let objects = complete_objects(ObjectCollection {
            objects: vec![object.clone()],
            skipped: Vec::new(),
        })
        .unwrap();
        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0].handle, object.handle);
        assert!(
            complete_objects(ObjectCollection {
                objects: Vec::new(),
                skipped: Vec::new(),
            })
            .unwrap()
            .is_empty()
        );
    }

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
