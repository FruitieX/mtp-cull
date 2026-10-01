mod planning;
#[cfg(any(windows, target_os = "linux", test))]
mod runtime;
#[cfg(any(windows, target_os = "linux", test))]
mod staging;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(not(any(windows, target_os = "linux")))]
mod unsupported;
#[cfg(windows)]
mod windows;

use crate::mtp_file::MtpFileType;
use color_eyre::eyre::{Report, Result, bail, eyre};
use log::info;
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(target_os = "linux")]
pub use linux::MtpWorker;
pub(crate) use planning::plan_import;
pub use planning::{ImportPaths, MtpMediaKind, classify_media_name};
#[cfg(not(any(windows, target_os = "linux")))]
pub use unsupported::MtpWorker;
#[cfg(windows)]
pub use windows::MtpWorker;

/// Opaque backend device locator, not a user-visible friendly name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MtpDevice {
    pub id: String,
    pub name: String,
}

/// A selectable MTP container. `id` is an MTP object ID and is opaque to callers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceFolder {
    pub id: String,
    pub name: String,
    pub path: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RemoteAsset {
    pub object_id: String,
    pub name: String,
    pub kind: MtpMediaKind,
    pub size: u64,
    pub preview_path: Option<PathBuf>,
    pub source_path: String,
    pub modified: Option<String>,
    pub cache_key: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RemoteShot {
    /// Opaque ID stable for this device, source container, and exact file stem.
    pub id: String,
    pub stem: String,
    pub assets: Vec<RemoteAsset>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RemoteSession {
    pub device_id: String,
    pub source_folder_id: String,
    pub shots: Vec<RemoteShot>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MtpFileInfo {
    pub object_id: String,
    pub name: String,
    pub path: String,
    pub file_type: MtpFileType,
    pub size: u64,
}

impl Display for MtpFileInfo {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} ({:?}): {}",
            self.path,
            self.file_type,
            size::Size::from_bytes(self.size)
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MtpCopyItem {
    pub object_id: String,
    pub source_path: String,
    pub size: u64,
    pub destination: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MtpCopyError {
    pub source_path: String,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MtpCopyResult {
    pub copied_files: usize,
    pub skipped_files: usize,
    pub copied_bytes: u64,
    pub total_bytes: u64,
    pub errors: Vec<MtpCopyError>,
}

#[derive(Clone, Debug)]
pub enum MtpRequest {
    ListDevices,
    ListSourceFolders {
        device_id: String,
    },
    StartSession {
        device_id: String,
        source_folder_id: String,
    },
    SetPriority {
        shot_ids: Vec<String>,
        paused: bool,
        disk_limit: u64,
    },
    ImportAssets {
        object_ids: Vec<String>,
        destinations: ImportPaths,
    },
    ListFiles {
        device_id: String,
        path: Option<String>,
    },
    CopyFiles {
        files: Vec<MtpCopyItem>,
        keep_going: bool,
    },
    CloseSession,
    Retry,
    Cancel,
    Shutdown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MtpEvent {
    Devices(Vec<MtpDevice>),
    SourceFolders {
        device_id: String,
        folders: Vec<SourceFolder>,
    },
    SessionScanned(RemoteSession),
    PreviewCached {
        shot_id: String,
        preview_path: Option<PathBuf>,
    },
    PreviewFailed {
        shot_id: String,
        message: String,
    },
    ImportFinished {
        copied: usize,
        skipped_existing: usize,
    },
    Progress {
        operation: &'static str,
        name: String,
        done: u64,
        total: u64,
    },
    Staging {
        ready: usize,
        total: usize,
        bytes: u64,
        paused: bool,
    },
    FilesListed(Vec<MtpFileInfo>),
    CopyFinished(MtpCopyResult),
    Error {
        operation: &'static str,
        message: String,
    },
}

pub(crate) type EventSink<'a> = dyn FnMut(MtpEvent) + 'a;
#[derive(Debug)]
pub(crate) struct OperationCancelled;

impl std::fmt::Display for OperationCancelled {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("MTP operation cancelled")
    }
}

impl std::error::Error for OperationCancelled {}

pub(crate) fn check_cancelled(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Acquire) {
        Err(Report::new(OperationCancelled))
    } else {
        Ok(())
    }
}

pub(crate) fn send_progress(
    events: &mut EventSink<'_>,
    cancel: &AtomicBool,
    operation: &'static str,
    completed: u64,
    total: Option<u64>,
    detail: impl Into<String>,
) -> Result<()> {
    check_cancelled(cancel)?;
    events(MtpEvent::Progress {
        operation,
        name: detail.into(),
        done: completed,
        total: total.unwrap_or(0),
    });
    Ok(())
}

#[cfg(target_os = "linux")]
pub(crate) struct ProgressReporter<'a, 'sink> {
    events: &'a mut EventSink<'sink>,
    cancel: &'a AtomicBool,
    operation: &'static str,
    completed: u64,
}

#[cfg(target_os = "linux")]
impl<'a, 'sink> ProgressReporter<'a, 'sink> {
    pub(crate) fn new(
        events: &'a mut EventSink<'sink>,
        cancel: &'a AtomicBool,
        operation: &'static str,
    ) -> Self {
        Self {
            events,
            cancel,
            operation,
            completed: 0,
        }
    }

    pub(crate) fn check(&self) -> Result<()> {
        check_cancelled(self.cancel)
    }

    pub(crate) fn step(&mut self, detail: impl Into<String>) -> Result<()> {
        self.completed += 1;
        self.report(self.completed, None, detail)
    }

    pub(crate) fn report(
        &mut self,
        completed: u64,
        total: Option<u64>,
        detail: impl Into<String>,
    ) -> Result<()> {
        send_progress(
            self.events,
            self.cancel,
            self.operation,
            completed,
            total,
            detail,
        )
    }
}

pub(crate) fn is_cancelled(error: &color_eyre::Report) -> bool {
    error.downcast_ref::<OperationCancelled>().is_some()
}

/// Synchronous facade for the worker/backend boundary used by the legacy CLI.
pub struct MtpBackend {
    worker: MtpWorker,
}

impl MtpBackend {
    pub fn spawn() -> Result<Self> {
        Ok(Self {
            worker: MtpWorker::spawn()?,
        })
    }

    pub fn list_devices(&self) -> Result<Vec<MtpDevice>> {
        self.worker.send(MtpRequest::ListDevices)?;
        loop {
            match self.worker.recv()? {
                MtpEvent::Devices(devices) => return Ok(devices),
                MtpEvent::Progress {
                    operation, name, ..
                } => {
                    info!("{operation}: {name}")
                }
                MtpEvent::Error { operation, message } => {
                    bail!("MTP {operation} failed: {message}")
                }
                event => bail!("unexpected MTP event while listing devices: {event:?}"),
            }
        }
    }

    pub fn select_device(&self, name: Option<&str>) -> Result<MtpDevice> {
        let devices = self.list_devices()?;
        match name {
            Some(name) => devices
                .into_iter()
                .find(|device| device.name == name)
                .ok_or_else(|| eyre!("no MTP device named {name:?} found")),
            None => devices
                .into_iter()
                .next()
                .ok_or_else(|| eyre!("no MTP devices found")),
        }
    }

    pub fn list_files(&self, device_id: String, path: Option<String>) -> Result<Vec<MtpFileInfo>> {
        self.worker
            .send(MtpRequest::ListFiles { device_id, path })?;
        loop {
            match self.worker.recv()? {
                MtpEvent::FilesListed(files) => return Ok(files),
                MtpEvent::Progress {
                    operation, name, ..
                } => {
                    info!("{operation}: {name}")
                }
                MtpEvent::Error { operation, message } => {
                    bail!("MTP {operation} failed: {message}")
                }
                event => bail!("unexpected MTP event while listing files: {event:?}"),
            }
        }
    }

    pub fn copy_files(&self, files: Vec<MtpCopyItem>, keep_going: bool) -> Result<MtpCopyResult> {
        self.worker
            .send(MtpRequest::CopyFiles { files, keep_going })?;
        loop {
            match self.worker.recv()? {
                MtpEvent::CopyFinished(result) => return Ok(result),
                MtpEvent::Progress {
                    operation, name, ..
                } => {
                    info!("{operation}: {name}")
                }
                MtpEvent::Error { operation, message } => {
                    bail!("MTP {operation} failed: {message}")
                }
                event => bail!("unexpected MTP event while copying files: {event:?}"),
            }
        }
    }
}
