mod planning;

#[cfg(not(windows))]
mod unsupported;
#[cfg(windows)]
mod windows;

use std::path::PathBuf;

pub use planning::{ImportPaths, MtpMediaKind};
#[cfg(not(windows))]
pub use unsupported::MtpWorker;
#[cfg(windows)]
pub use windows::MtpWorker;

/// Stable Windows Portable Device identifier, not a user-visible friendly name.
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteAsset {
    pub object_id: String,
    pub name: String,
    pub kind: MtpMediaKind,
    pub size: u64,
    pub preview_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteShot {
    /// Opaque ID stable for this device, source container, and exact file stem.
    pub id: String,
    pub stem: String,
    pub assets: Vec<RemoteAsset>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
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
    ImportKept {
        shot_ids: Vec<String>,
        destinations: ImportPaths,
    },
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
    ImportFinished {
        copied: usize,
        skipped_existing: usize,
    },
    Error {
        operation: &'static str,
        message: String,
    },
}
