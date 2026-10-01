use super::{
    MtpCopyError, MtpCopyItem, MtpCopyResult, MtpDevice, MtpEvent, MtpFileInfo, RemoteAsset,
    RemoteSession, RemoteShot, SourceFolder, check_cancelled, is_cancelled,
    planning::{classify_media_name, normalize_source_path, plan_import},
    send_progress,
};
use crate::mtp_file::MtpFileType;
use crate::safe_copy::{CopyOutcome, copy_reader_verified};
use color_eyre::eyre::{Result, WrapErr, bail, eyre};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use winmtp::PortableDevices::{WPD_OBJECT_DATE_CREATED, WPD_OBJECT_DATE_MODIFIED, WPD_OBJECT_SIZE};
use winmtp::Provider;
use winmtp::device::{BasicDevice, Device};
use winmtp::object::{Object, ObjectType};

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
    provider: Option<Provider>,
    session: Option<PrivateSession>,
    legacy_session: Option<LegacySession>,
}

struct PrivateSession {
    _device: Device,
    public: RemoteSession,
    objects: HashMap<String, Object>,
    cache_directory: PathBuf,
}

struct LegacySession {
    _device: Device,
    objects: HashMap<String, Object>,
}

impl WorkerState {
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
        let cache_directory = super::staging::directory(&device_id, &source_folder_id)?;
        let nonce = super::staging::nonce();
        let mut grouped = BTreeMap::<String, Vec<(RemoteAsset, Object)>>::new();
        collect_media(source, "", &mut grouped)?;

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
            for (mut asset, object) in assets {
                asset.cache_key =
                    super::staging::asset_key(&device_id, &source_folder_id, &asset, &nonce);
                objects.insert(asset.object_id.clone(), object);
                public_assets.push(asset);
            }
            shots.push(RemoteShot {
                id,
                stem: stem.rsplit('\0').next().unwrap_or(&stem).to_owned(),
                assets: public_assets,
            });
        }
        let mut public = RemoteSession {
            device_id,
            source_folder_id,
            shots,
        };
        super::staging::restore(&mut public, &cache_directory)?;
        Ok(PrivateSession {
            _device: device,
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
                let object = session.objects.get(&asset.object_id).ok_or_else(|| {
                    eyre!("MTP object {:?} is no longer retained", asset.object_id)
                })?;
                let mut input = object
                    .open_read_stream()
                    .wrap_err_with(|| format!("failed to read {}", asset.name))?;
                copy_reader_verified(
                    &mut input,
                    asset.size,
                    &destination,
                    &asset.name,
                    None,
                    &mut tick,
                )?
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
            let mut input = session
                .objects
                .get(&asset.object_id)
                .ok_or_else(|| eyre!("camera object disappeared"))?
                .open_read_stream()?;
            let (_, hash) =
                copy_reader_verified(&mut input, asset.size, &path, &asset.name, None, |done| {
                    if cancel.load(Ordering::Relaxed) {
                        bail!("Staging cancelled");
                    }
                    progress(asset.name.clone(), done, asset.size);
                    Ok(())
                })?;
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
        let device = self.open_device(device_id)?;
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
            cancel,
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
                let object = session.objects.get(&file.object_id).ok_or_else(|| {
                    eyre!("MTP object {:?} is no longer retained", file.object_id)
                })?;
                let mut input = object
                    .open_read_stream()
                    .wrap_err_with(|| format!("failed to read {}", file.source_path))?;
                let mut last_reported = 0;
                crate::safe_copy::copy_reader_with_progress(
                    &mut input,
                    file.size,
                    &file.destination,
                    &file.source_path,
                    |bytes| {
                        check_cancelled(cancel)?;
                        if bytes != file.size && bytes.saturating_sub(last_reported) < (1024 * 1024)
                        {
                            return Ok(());
                        }
                        last_reported = bytes;
                        send_progress(
                            events,
                            cancel,
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
    parent_path: &str,
    grouped: &mut BTreeMap<String, Vec<(RemoteAsset, Object)>>,
) -> Result<()> {
    for child in container
        .children()
        .wrap_err("failed to enumerate MTP media")?
    {
        if is_container(&child) {
            let child_path = join_remote_path(parent_path, &child.name().to_string_lossy());
            collect_media(child, &child_path, grouped)?;
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
        let metadata = child
            .properties(&[
                WPD_OBJECT_SIZE,
                WPD_OBJECT_DATE_MODIFIED,
                WPD_OBJECT_DATE_CREATED,
            ])
            .wrap_err_with(|| format!("failed to read metadata for {name}"))?;
        let modified = metadata
            .get_date(&WPD_OBJECT_DATE_MODIFIED)
            .or_else(|_| metadata.get_date(&WPD_OBJECT_DATE_CREATED))
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|t| format!("{t:?}"));
        let source_path = join_remote_path(parent_path, &name);
        let size = metadata
            .get_u64(&WPD_OBJECT_SIZE)
            .wrap_err_with(|| format!("failed to read file size for {name}"))?;
        let preview_path = None;
        grouped
            .entry(format!("{parent_path}\0{stem}"))
            .or_default()
            .push((
                RemoteAsset {
                    object_id,
                    name,
                    kind,
                    size,
                    preview_path,
                    source_path,
                    modified,
                    cache_key: String::new(),
                },
                child,
            ));
    }
    Ok(())
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

fn collect_legacy_files(
    container: Object,
    parent_path: &str,
    files: &mut Vec<MtpFileInfo>,
    objects: &mut HashMap<String, Object>,
    cancel: &AtomicBool,
    events: &mut super::EventSink<'_>,
    progress: &mut u64,
) -> Result<()> {
    check_cancelled(cancel)?;
    for child in container
        .children()
        .wrap_err("failed to enumerate MTP objects")?
    {
        check_cancelled(cancel)?;
        let name = child.name().to_string_lossy();
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
