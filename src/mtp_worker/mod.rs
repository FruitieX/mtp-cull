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

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
    Error {
        operation: &'static str,
        message: String,
    },
}
