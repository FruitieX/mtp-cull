use crate::mtp_worker::{MtpDevice, SourceFolder};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const LIMIT: usize = 12;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RecentSource {
    Local {
        root: PathBuf,
        raw: Option<PathBuf>,
    },
    Camera {
        device_id: String,
        device_name: String,
        folder_id: String,
        folder_path: String,
    },
}

impl RecentSource {
    pub fn label(&self) -> String {
        match self {
            Self::Local { root, .. } => format!(
                "Folder: {}",
                root.file_name()
                    .unwrap_or(root.as_os_str())
                    .to_string_lossy()
            ),
            Self::Camera {
                device_name,
                folder_path,
                ..
            } => {
                format!(
                    "{device_name}: {}",
                    if folder_path.is_empty() {
                        "Device root"
                    } else {
                        folder_path
                    }
                )
            }
        }
    }

    pub fn detail(&self) -> String {
        match self {
            Self::Local { root, raw } => format!(
                "{}{}",
                root.display(),
                raw.as_ref()
                    .map(|path| format!("\nRAW: {}", path.display()))
                    .unwrap_or_default()
            ),
            Self::Camera {
                device_name,
                folder_path,
                ..
            } => {
                format!(
                    "{device_name}\n{}",
                    if folder_path.is_empty() {
                        "Device root"
                    } else {
                        folder_path
                    }
                )
            }
        }
    }

    /// Prefer the known device ID; accept a changed ID only when the name is unique.
    pub fn device<'a>(&self, devices: &'a [MtpDevice]) -> Option<&'a MtpDevice> {
        let Self::Camera {
            device_id,
            device_name,
            ..
        } = self
        else {
            return None;
        };
        if let Some(device) = devices.iter().find(|device| device.id == *device_id) {
            return Some(device);
        }
        let mut matches = devices.iter().filter(|device| device.name == *device_name);
        let device = matches.next()?;
        matches.next().is_none().then_some(device)
    }

    /// Resolve the remembered path against today's listing, never reuse a stale object ID.
    pub fn folder<'a>(&self, folders: &'a [SourceFolder]) -> Option<&'a SourceFolder> {
        let Self::Camera { folder_path, .. } = self else {
            return None;
        };
        let mut matches = folders.iter().filter(|folder| folder.path == *folder_path);
        let folder = matches.next()?;
        matches.next().is_none().then_some(folder)
    }
}

pub fn remember(history: &mut Vec<RecentSource>, source: RecentSource) {
    history.retain(|previous| match (previous, &source) {
        (RecentSource::Local { root: old, .. }, RecentSource::Local { root, .. }) => old != root,
        (
            RecentSource::Camera {
                device_id: old_device,
                folder_path: old_path,
                ..
            },
            RecentSource::Camera {
                device_id,
                folder_path,
                ..
            },
        ) => old_device != device_id || old_path != folder_path,
        _ => true,
    });
    history.insert(0, source);
    history.truncate(LIMIT);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> RecentSource {
        RecentSource::Camera {
            device_id: "old-device".into(),
            device_name: "X-T5".into(),
            folder_id: "old-folder".into(),
            folder_path: "DCIM/100_FUJI".into(),
        }
    }

    #[test]
    fn recent_folders_update_raw_companions_move_to_front_and_stay_bounded() {
        let mut history = Vec::new();
        for index in 0..20 {
            remember(
                &mut history,
                RecentSource::Local {
                    root: format!("album-{index}").into(),
                    raw: None,
                },
            );
        }
        let source = RecentSource::Local {
            root: "album-10".into(),
            raw: Some("raw-10".into()),
        };
        remember(&mut history, source.clone());
        assert_eq!(history.len(), LIMIT);
        assert_eq!(history[0], source);
        assert_eq!(history.iter().filter(|entry| matches!(entry, RecentSource::Local { root, .. } if root == &PathBuf::from("album-10"))).count(), 1);
    }

    #[test]
    fn reconnect_resolution_rejects_missing_or_ambiguous_devices_and_paths() {
        let recent = camera();
        assert!(recent.device(&[]).is_none());
        let changed = MtpDevice {
            id: "new-device".into(),
            name: "X-T5".into(),
        };
        assert_eq!(
            recent.device(std::slice::from_ref(&changed)).unwrap().id,
            "new-device"
        );
        let devices = vec![
            changed,
            MtpDevice {
                id: "other-device".into(),
                name: "X-T5".into(),
            },
        ];
        assert!(recent.device(&devices).is_none());
        let mut devices = devices;
        devices.push(MtpDevice {
            id: "old-device".into(),
            name: "Renamed camera".into(),
        });
        assert_eq!(recent.device(&devices).unwrap().id, "old-device");
        let wrong = SourceFolder {
            id: "old-folder".into(),
            name: "Other".into(),
            path: "DCIM/OTHER".into(),
        };
        assert!(recent.folder(std::slice::from_ref(&wrong)).is_none());
        let folder = SourceFolder {
            id: "new-folder".into(),
            name: "100_FUJI".into(),
            path: "DCIM/100_FUJI".into(),
        };
        assert_eq!(
            recent.folder(std::slice::from_ref(&folder)).unwrap().id,
            "new-folder"
        );
        assert!(recent.folder(&[folder.clone(), folder]).is_none());
    }
}
