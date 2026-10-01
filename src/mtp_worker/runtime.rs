use super::{ImportPaths, MtpDevice, MtpEvent, MtpRequest, RemoteSession, SourceFolder};
use color_eyre::eyre::{Result, eyre};
use std::collections::HashSet;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc::{self, Receiver, Sender},
};
use std::thread::JoinHandle;

pub(super) trait Backend {
    fn devices(&mut self) -> Result<Vec<MtpDevice>>;
    fn folders(&mut self, id: &str) -> Result<Vec<SourceFolder>>;
    fn start(&mut self, device: String, folder: String) -> Result<RemoteSession>;
    fn session(&self) -> Option<&RemoteSession>;
    fn check_space(&self, next: u64, limit: u64) -> Result<()> {
        let session = self.session().ok_or_else(|| eyre!("no camera session"))?;
        let active = super::staging::directory(&session.device_id, &session.source_folder_id)?;
        super::staging::maintain(&active, next, limit)
    }
    fn preview(
        &mut self,
        id: String,
        progress: &mut dyn FnMut(String, u64, u64),
        cancel: &AtomicBool,
    ) -> Result<MtpEvent>;
    fn import(
        &mut self,
        ids: Vec<String>,
        destinations: ImportPaths,
        progress: &mut dyn FnMut(String, u64, u64),
        cancel: &AtomicBool,
    ) -> Result<MtpEvent>;
    fn files(
        &mut self,
        _device: &str,
        _path: Option<&str>,
        _events: &mut super::EventSink<'_>,
        _cancel: &AtomicBool,
    ) -> Result<Vec<super::MtpFileInfo>> {
        Err(eyre!("file listing unsupported"))
    }
    fn copy(
        &mut self,
        _files: Vec<super::MtpCopyItem>,
        _keep_going: bool,
        _events: &mut super::EventSink<'_>,
        _cancel: &AtomicBool,
    ) -> Result<MtpEvent> {
        Err(eyre!("CLI copying unsupported"))
    }
    fn close(&mut self);
}
struct Request {
    generation: u64,
    request: MtpRequest,
}
struct Event {
    generation: u64,
    event: MtpEvent,
}
pub(super) struct Worker {
    sender: Sender<Request>,
    receiver: Receiver<Event>,
    generation: Arc<AtomicU64>,
    cancel: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl Worker {
    pub(super) fn spawn<B: Backend + 'static>(
        factory: impl FnOnce() -> B + Send + 'static,
    ) -> Result<Self> {
        let (sender, requests) = mpsc::channel();
        let (events, receiver) = mpsc::channel();
        let generation = Arc::new(AtomicU64::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let live_generation = generation.clone();
        let thread = std::thread::Builder::new()
            .name("mtp-device".into())
            .spawn(move || run(factory(), requests, events, worker_cancel, live_generation))?;
        Ok(Self {
            sender,
            receiver,
            generation,
            cancel,
            thread: Some(thread),
        })
    }
    pub fn send(&self, request: MtpRequest) -> Result<()> {
        if matches!(
            request,
            MtpRequest::Cancel | MtpRequest::Shutdown | MtpRequest::CloseSession
        ) {
            self.cancel.store(true, Ordering::Relaxed);
        }
        if matches!(
            request,
            MtpRequest::StartSession { .. } | MtpRequest::CloseSession
        ) {
            self.generation.fetch_add(1, Ordering::Relaxed);
        }
        if matches!(
            request,
            MtpRequest::ImportAssets { .. }
                | MtpRequest::StartSession { .. }
                | MtpRequest::ListFiles { .. }
                | MtpRequest::CopyFiles { .. }
        ) {
            self.cancel.store(false, Ordering::Relaxed);
        }
        self.sender
            .send(Request {
                generation: self.generation.load(Ordering::Relaxed),
                request,
            })
            .map_err(|_| eyre!("MTP worker stopped"))
    }
    pub fn recv(&self) -> Result<MtpEvent> {
        loop {
            let event = self
                .receiver
                .recv()
                .map_err(|_| eyre!("MTP worker stopped"))?;
            if event.generation == self.generation.load(Ordering::Relaxed) {
                return Ok(event.event);
            }
        }
    }
    pub fn try_recv(&self) -> Option<MtpEvent> {
        while let Ok(event) = self.receiver.try_recv() {
            if event.generation == self.generation.load(Ordering::Relaxed) {
                return Some(event.event);
            }
        }
        None
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.send(MtpRequest::Shutdown);
        if let Some(thread) = self.thread.take()
            && thread.is_finished()
        {
            let _ = thread.join();
        }
        // Native driver calls cannot be forcibly cancelled safely. Detach an
        // outstanding call so application close cannot hang behind the device.
    }
}
fn run<B: Backend>(
    mut backend: B,
    requests: Receiver<Request>,
    events: Sender<Event>,
    cancel: Arc<AtomicBool>,
    live_generation: Arc<AtomicU64>,
) {
    let mut generation = 0;
    let mut priorities = Vec::new();
    let mut paused = true;
    let mut disk_limit = 32 * 1024 * 1024 * 1024;
    let mut failed = HashSet::new();
    loop {
        let work = backend.session().and_then(|session| {
            if paused {
                return None;
            }
            priorities
                .iter()
                .chain(session.shots.iter().map(|s| &s.id))
                .find(|id| {
                    !failed.contains(*id)
                        && session
                            .shots
                            .iter()
                            .find(|s| s.id == **id)
                            .is_some_and(|s| {
                                s.assets
                                    .iter()
                                    .any(|a| a.kind.is_jpeg() && a.preview_path.is_none())
                            })
                })
                .cloned()
        });
        let request = if work.is_some() {
            match requests.try_recv() {
                Ok(r) => Some(r),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => return,
            }
        } else {
            match requests.recv() {
                Ok(r) => Some(r),
                Err(_) => return,
            }
        };
        if let Some(Request {
            generation: request_generation,
            request,
        }) = request
        {
            if matches!(request, MtpRequest::Shutdown) {
                backend.close();
                return;
            }
            if request_generation != live_generation.load(Ordering::Relaxed) {
                continue;
            }
            generation = request_generation;
            let send = |event| {
                let _ = events.send(Event { generation, event });
            };
            let (operation, result): (&'static str, Result<Option<MtpEvent>>) = match request {
                MtpRequest::ListDevices => (
                    "list devices",
                    backend.devices().map(|d| Some(MtpEvent::Devices(d))),
                ),
                MtpRequest::ListSourceFolders { device_id } => (
                    "list folders",
                    backend
                        .folders(&device_id)
                        .map(|folders| Some(MtpEvent::SourceFolders { device_id, folders })),
                ),
                MtpRequest::StartSession {
                    device_id,
                    source_folder_id,
                } => {
                    failed.clear();
                    priorities.clear();
                    paused = false;
                    (
                        "start review",
                        backend
                            .start(device_id, source_folder_id)
                            .map(|s| Some(MtpEvent::SessionScanned(s))),
                    )
                }
                MtpRequest::SetPriority {
                    shot_ids,
                    paused: pause,
                    disk_limit: limit,
                } => {
                    priorities = shot_ids;
                    paused = pause;
                    disk_limit = limit;
                    if !paused {
                        cancel.store(false, Ordering::Relaxed);
                    }
                    ("staging", Ok(None))
                }
                MtpRequest::ImportAssets {
                    object_ids,
                    destinations,
                } => {
                    paused = true;
                    let mut gate = crate::transfer_progress::ProgressGate::new();
                    let mut progress = |name, done, total| {
                        if !gate.ready(done, total) {
                            return;
                        }
                        send(MtpEvent::Progress {
                            operation: "import",
                            name,
                            done,
                            total,
                        })
                    };
                    (
                        "import",
                        backend
                            .import(object_ids, destinations, &mut progress, &cancel)
                            .map(Some),
                    )
                }
                MtpRequest::ListFiles { device_id, path } => {
                    paused = true;
                    (
                        "list files",
                        backend
                            .files(
                                &device_id,
                                path.as_deref(),
                                &mut |event| send(event),
                                &cancel,
                            )
                            .map(|files| Some(MtpEvent::FilesListed(files))),
                    )
                }
                MtpRequest::CopyFiles { files, keep_going } => {
                    paused = true;
                    (
                        "copy files",
                        backend
                            .copy(files, keep_going, &mut |event| send(event), &cancel)
                            .map(Some),
                    )
                }
                MtpRequest::CloseSession => {
                    backend.close();
                    priorities.clear();
                    failed.clear();
                    paused = true;
                    ("close", Ok(None))
                }
                MtpRequest::Cancel => {
                    paused = true;
                    ("cancel", Ok(None))
                }
                MtpRequest::Retry => {
                    failed.clear();
                    paused = false;
                    cancel.store(false, Ordering::Relaxed);
                    ("retry staging", Ok(None))
                }
                MtpRequest::Shutdown => unreachable!(),
            };
            match result {
                Ok(Some(event)) => send(event),
                Ok(None) => {}
                Err(error) => send(MtpEvent::Error {
                    operation,
                    message: format!("{error:#}"),
                }),
            }
        } else if let Some(id) = work {
            if generation != live_generation.load(Ordering::Relaxed) {
                continue;
            }
            let session = backend.session().unwrap();
            let used = session
                .shots
                .iter()
                .flat_map(|s| &s.assets)
                .filter(|a| a.preview_path.is_some())
                .map(|a| a.size)
                .sum::<u64>();
            let next = session
                .shots
                .iter()
                .find(|s| s.id == id)
                .and_then(|s| s.assets.iter().find(|a| a.kind.is_jpeg()))
                .map_or(0, |a| a.size);
            let space = backend.check_space(next, disk_limit);
            let event = if used.saturating_add(next) > disk_limit || space.is_err() {
                paused = true;
                MtpEvent::Error{operation:"staging",message:"JPEG staging quota reached. Increase the cache quota or import selected files.".into()}
            } else {
                let mut gate = crate::transfer_progress::ProgressGate::new();
                let mut progress = |name, done, total| {
                    if gate.ready(done, total) {
                        let _ = events.send(Event {
                            generation,
                            event: MtpEvent::Progress {
                                operation: "staging",
                                name,
                                done,
                                total,
                            },
                        });
                    }
                };
                match backend.preview(id.clone(), &mut progress, &cancel) {
                    Ok(event) => {
                        if matches!(event, MtpEvent::PreviewFailed { .. }) {
                            failed.insert(id);
                        }
                        event
                    }
                    Err(error) => {
                        failed.insert(id.clone());
                        MtpEvent::PreviewFailed {
                            shot_id: id,
                            message: format!("{error:#}"),
                        }
                    }
                }
            };
            if events.send(Event { generation, event }).is_err() {
                return;
            }
        }
        if let Some(session) = backend.session() {
            let total = session
                .shots
                .iter()
                .filter(|s| s.assets.iter().any(|a| a.kind.is_jpeg()))
                .count();
            let ready = session
                .shots
                .iter()
                .filter(|s| {
                    s.assets
                        .iter()
                        .any(|a| a.kind.is_jpeg() && a.preview_path.is_some())
                })
                .count();
            let bytes = session
                .shots
                .iter()
                .flat_map(|s| &s.assets)
                .filter(|a| a.preview_path.is_some())
                .map(|a| a.size)
                .sum();
            if events
                .send(Event {
                    generation,
                    event: MtpEvent::Staging {
                        ready,
                        total,
                        bytes,
                        paused,
                    },
                })
                .is_err()
            {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mtp_worker::{MtpMediaKind, RemoteAsset, RemoteShot};
    use std::time::{Duration, Instant};
    struct Fake {
        session: Option<RemoteSession>,
        started: Sender<String>,
        release: Receiver<()>,
        previews: Sender<String>,
    }
    impl Backend for Fake {
        fn devices(&mut self) -> Result<Vec<MtpDevice>> {
            self.started.send("devices".into()).unwrap();
            self.release.recv().unwrap();
            Ok(vec![MtpDevice {
                id: "camera".into(),
                name: "Camera".into(),
            }])
        }
        fn folders(&mut self, _: &str) -> Result<Vec<SourceFolder>> {
            Ok(Vec::new())
        }
        fn start(&mut self, device: String, folder: String) -> Result<RemoteSession> {
            let shots = (0..3)
                .map(|i| {
                    let id = i.to_string();
                    RemoteShot {
                        id: id.clone(),
                        stem: id.clone(),
                        assets: vec![RemoteAsset {
                            object_id: id.clone(),
                            name: format!("{i}.JPG"),
                            kind: MtpMediaKind::Jpeg,
                            size: 10,
                            preview_path: None,
                            source_path: id.clone(),
                            modified: Some("stamp".into()),
                            cache_key: id,
                        }],
                    }
                })
                .collect();
            let session = RemoteSession {
                device_id: device,
                source_folder_id: folder,
                shots,
            };
            self.session = Some(session.clone());
            Ok(session)
        }
        fn session(&self) -> Option<&RemoteSession> {
            self.session.as_ref()
        }
        fn check_space(&self, _: u64, _: u64) -> Result<()> {
            Ok(())
        }
        fn preview(
            &mut self,
            id: String,
            progress: &mut dyn FnMut(String, u64, u64),
            cancel: &AtomicBool,
        ) -> Result<MtpEvent> {
            if cancel.load(Ordering::Relaxed) {
                return Err(eyre!("cancelled"));
            }
            progress(id.clone(), 10, 10);
            self.previews.send(id.clone()).unwrap();
            let path = std::path::PathBuf::from(format!("{id}.jpg"));
            self.session
                .as_mut()
                .unwrap()
                .shots
                .iter_mut()
                .find(|s| s.id == id)
                .unwrap()
                .assets[0]
                .preview_path = Some(path.clone());
            Ok(MtpEvent::PreviewCached {
                shot_id: id,
                preview_path: Some(path),
            })
        }
        fn import(
            &mut self,
            _: Vec<String>,
            _: ImportPaths,
            _: &mut dyn FnMut(String, u64, u64),
            _: &AtomicBool,
        ) -> Result<MtpEvent> {
            Ok(MtpEvent::ImportFinished {
                copied: 0,
                skipped_existing: 0,
            })
        }
        fn files(
            &mut self,
            device: &str,
            path: Option<&str>,
            events: &mut super::super::EventSink<'_>,
            cancel: &AtomicBool,
        ) -> Result<Vec<super::super::MtpFileInfo>> {
            assert_eq!(device, "camera");
            assert_eq!(path, Some("DCIM"));
            super::super::send_progress(events, cancel, "list files", 1, None, "DCIM/photo.jpg")?;
            Ok(vec![super::super::MtpFileInfo {
                object_id: "photo".into(),
                name: "photo.jpg".into(),
                path: "DCIM/photo.jpg".into(),
                file_type: crate::mtp_file::MtpFileType::Image,
                size: 3,
            }])
        }
        fn copy(
            &mut self,
            files: Vec<super::super::MtpCopyItem>,
            _: bool,
            events: &mut super::super::EventSink<'_>,
            cancel: &AtomicBool,
        ) -> Result<MtpEvent> {
            let mut copied = 0;
            let mut skipped = 0;
            for file in files {
                assert_eq!(file.object_id, "photo");
                let outcome = crate::safe_copy::copy_reader_with_progress(
                    &mut b"abc".as_slice(),
                    file.size,
                    &file.destination,
                    &file.source_path,
                    |done| {
                        super::super::send_progress(
                            events,
                            cancel,
                            "copy files",
                            done,
                            Some(file.size),
                            &file.source_path,
                        )
                    },
                )?;
                match outcome {
                    crate::safe_copy::CopyOutcome::Copied => copied += 1,
                    crate::safe_copy::CopyOutcome::SkippedExisting => skipped += 1,
                }
            }
            Ok(MtpEvent::CopyFinished(super::super::MtpCopyResult {
                copied_files: copied,
                skipped_files: skipped,
                copied_bytes: copied as u64 * 3,
                total_bytes: 3,
                errors: vec![],
            }))
        }
        fn close(&mut self) {
            self.session = None;
        }
    }
    #[test]
    fn cli_requests_deliver_progress_and_copy_results_without_background_staging() {
        let (worker, _, _, previews) = fixture();
        worker
            .send(MtpRequest::ListFiles {
                device_id: "camera".into(),
                path: Some("DCIM".into()),
            })
            .unwrap();
        assert!(matches!(
            worker.recv().unwrap(),
            MtpEvent::Progress { done: 1, .. }
        ));
        let MtpEvent::FilesListed(files) = worker.recv().unwrap() else {
            panic!("missing file listing")
        };
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("photo.jpg");
        let file = &files[0];
        let item = super::super::MtpCopyItem {
            object_id: file.object_id.clone(),
            source_path: file.path.clone(),
            size: file.size,
            destination: destination.clone(),
        };
        for retry in [false, true] {
            worker
                .send(MtpRequest::CopyFiles {
                    files: vec![item.clone()],
                    keep_going: false,
                })
                .unwrap();
            let MtpEvent::CopyFinished(result) =
                until(&worker, |e| matches!(e, MtpEvent::CopyFinished(_)))
            else {
                panic!("missing copy result")
            };
            assert_eq!(result.copied_files, usize::from(!retry));
            assert_eq!(result.skipped_files, usize::from(retry));
            assert_eq!(std::fs::read(&destination).unwrap(), b"abc");
        }
        assert!(previews.try_recv().is_err());
    }
    #[test]
    fn preset_workflow_uses_worker_copy_with_retry_and_no_staging() {
        use crate::import_presets::ImportPreset;
        use crate::quick_import::{Action, Run};
        let directory = tempfile::tempdir().unwrap();
        let preset = ImportPreset {
            name: "Backup".into(),
            device: "Camera".into(),
            source_path: "DCIM".into(),
            pictures_path: directory.path().display().to_string(),
            videos_path: directory.path().display().to_string(),
            raw_path: directory.path().display().to_string(),
            date: "2026-10-01".into(),
            ..Default::default()
        };
        let (worker, started, release, previews) = fixture();
        for retry in [false, true] {
            let mut run = Run::new(&preset).unwrap();
            worker.send(MtpRequest::CloseSession).unwrap();
            worker.send(MtpRequest::ListDevices).unwrap();
            started.recv_timeout(Duration::from_secs(1)).unwrap();
            release.send(()).unwrap();
            loop {
                let event = worker.recv().unwrap();
                if let MtpEvent::Error { operation, message } = &event {
                    panic!("{operation}: {message}");
                }
                match run.event(&event).unwrap() {
                    Action::None => {}
                    Action::Request(request) => worker.send(request).unwrap(),
                    Action::Finished(result) => {
                        assert_eq!(result.copied_files, usize::from(!retry));
                        assert_eq!(result.skipped_files, usize::from(retry));
                        assert!(result.errors.is_empty());
                        break;
                    }
                }
            }
        }
        assert_eq!(
            std::fs::read(directory.path().join("2026/2026-10-01/photo.jpg")).unwrap(),
            b"abc"
        );
        assert!(previews.try_recv().is_err());
    }
    fn fixture() -> (Worker, Receiver<String>, Sender<()>, Receiver<String>) {
        let (started, start) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let (previews, preview) = mpsc::channel();
        let worker = Worker::spawn(move || Fake {
            session: None,
            started,
            release: wait,
            previews,
        })
        .unwrap();
        (worker, start, release, preview)
    }
    fn until(worker: &Worker, predicate: impl Fn(&MtpEvent) -> bool) -> MtpEvent {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(event) = worker.try_recv()
                && predicate(&event)
            {
                return event;
            }
            assert!(Instant::now() < deadline, "worker event timeout");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn close_invalidates_an_in_flight_device_result() {
        let (worker, started, release, _) = fixture();
        worker.send(MtpRequest::ListDevices).unwrap();
        started.recv_timeout(Duration::from_secs(1)).unwrap();
        worker.send(MtpRequest::CloseSession).unwrap();
        worker.send(MtpRequest::ListDevices).unwrap();
        release.send(()).unwrap();
        started.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(
            worker.try_recv().is_none(),
            "stale result escaped generation filter"
        );
        release.send(()).unwrap();
        assert!(matches!(
            until(&worker, |e| matches!(e, MtpEvent::Devices(_))),
            MtpEvent::Devices(_)
        ));
    }
    #[test]
    fn queued_priorities_precede_background_staging_and_quota_pauses() {
        let (worker, started, release, previews) = fixture();
        worker.send(MtpRequest::ListDevices).unwrap();
        started.recv_timeout(Duration::from_secs(1)).unwrap();
        worker
            .send(MtpRequest::StartSession {
                device_id: "camera".into(),
                source_folder_id: "folder".into(),
            })
            .unwrap();
        worker
            .send(MtpRequest::SetPriority {
                shot_ids: vec!["2".into()],
                paused: false,
                disk_limit: 20,
            })
            .unwrap();
        release.send(()).unwrap();
        assert_eq!(previews.recv_timeout(Duration::from_secs(2)).unwrap(), "2");
        assert_eq!(previews.recv_timeout(Duration::from_secs(2)).unwrap(), "0");
        until(&worker, |e| {
            matches!(
                e,
                MtpEvent::Staging {
                    ready: 2,
                    paused: true,
                    ..
                }
            )
        });
        assert!(previews.try_recv().is_err());
        worker
            .send(MtpRequest::SetPriority {
                shot_ids: vec![],
                paused: false,
                disk_limit: 30,
            })
            .unwrap();
        assert_eq!(previews.recv_timeout(Duration::from_secs(2)).unwrap(), "1");
        until(&worker, |e| matches!(e, MtpEvent::Staging { ready: 3, .. }));
    }
}
