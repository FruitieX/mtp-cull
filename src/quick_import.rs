//! Nonblocking preset workflow using the same worker requests as CLI copy.
use crate::cli::CopyArgs;
use crate::commands::plan_copy;
use crate::import_presets::ImportPreset;
use crate::mtp_worker::{MtpCopyResult, MtpEvent, MtpRequest};
use color_eyre::eyre::{Result, bail, eyre};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    Devices,
    Files,
    Copying,
    Finished,
}

pub struct Run {
    pub name: String,
    args: CopyArgs,
    phase: Phase,
}
pub enum Action {
    None,
    Request(MtpRequest),
    Finished(MtpCopyResult),
}
impl Run {
    pub fn new(preset: &ImportPreset) -> Result<Self> {
        Ok(Self {
            name: preset.name.clone(),
            args: preset.args(chrono::Local::now().date_naive())?,
            phase: Phase::Devices,
        })
    }
    pub fn event(&mut self, event: &MtpEvent) -> Result<Action> {
        match (self.phase, event) {
            (Phase::Devices, MtpEvent::Devices(devices)) => {
                let device = if let Some(name) = &self.args.device {
                    let mut matches = devices.iter().filter(|device| device.name == *name);
                    let device = matches.next().ok_or_else(|| {
                        eyre!("No connected device named {name:?}; reconnect it and try again")
                    })?;
                    if matches.next().is_some() {
                        bail!(
                            "More than one device is named {name:?}; connect only the intended device"
                        );
                    }
                    device
                } else {
                    devices.first().ok_or_else(|| {
                        eyre!("No connected MTP devices; reconnect your device and try again")
                    })?
                };
                self.phase = Phase::Files;
                Ok(Action::Request(MtpRequest::ListFiles {
                    device_id: device.id.clone(),
                    path: self.args.source_path.clone(),
                }))
            }
            (Phase::Files, MtpEvent::FilesListed(files)) => {
                let plan = plan_copy(
                    files,
                    &self.args,
                    self.args.date.expect("preset date resolved at start"),
                )?;
                self.phase = Phase::Copying;
                Ok(Action::Request(MtpRequest::CopyFiles {
                    files: plan,
                    keep_going: self.args.keep_going,
                }))
            }
            (Phase::Copying, MtpEvent::CopyFinished(result)) => {
                self.phase = Phase::Finished;
                Ok(Action::Finished(result.clone()))
            }
            _ => Ok(Action::None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mtp_file::MtpFileType;
    use crate::mtp_worker::{MtpDevice, MtpFileInfo};

    fn preset() -> ImportPreset {
        ImportPreset {
            name: "Backup".into(),
            device: "Phone".into(),
            source_path: "Internal shared storage/DCIM/Camera".into(),
            pictures_path: "pictures".into(),
            videos_path: "videos".into(),
            raw_path: "raw".into(),
            date: "2026-10-01".into(),
            album_name: "Trip".into(),
            keep_going: true,
        }
    }
    fn devices() -> MtpEvent {
        MtpEvent::Devices(vec![MtpDevice {
            id: "fresh-id".into(),
            name: "Phone".into(),
        }])
    }
    fn file(name: &str, kind: MtpFileType) -> MtpFileInfo {
        MtpFileInfo {
            object_id: name.into(),
            name: name.into(),
            path: format!("DCIM/{name}"),
            file_type: kind,
            size: 123,
        }
    }
    #[test]
    fn preset_copies_all_types_with_cli_layout_and_ignores_duplicate_events() {
        let mut run = Run::new(&preset()).unwrap();
        let Action::Request(MtpRequest::ListFiles { device_id, path }) =
            run.event(&devices()).unwrap()
        else {
            panic!("missing listing")
        };
        assert_eq!(device_id, "fresh-id");
        assert_eq!(path.as_deref(), Some("Internal shared storage/DCIM/Camera"));
        assert!(matches!(run.event(&devices()).unwrap(), Action::None));
        let event = MtpEvent::FilesListed(vec![
            file("a.JPG", MtpFileType::Image),
            file("a.DNG", MtpFileType::RawImage),
            file("a.MP4", MtpFileType::Video),
        ]);
        let Action::Request(MtpRequest::CopyFiles { files, keep_going }) =
            run.event(&event).unwrap()
        else {
            panic!("missing copy")
        };
        assert!(keep_going);
        assert_eq!(files.len(), 3);
        for (file, root) in files.iter().zip(["pictures", "raw", "videos"]) {
            assert_eq!(
                file.destination,
                std::path::Path::new(root)
                    .join("2026")
                    .join("2026-10-01 Trip")
                    .join(&file.object_id)
            );
        }
        assert!(matches!(run.event(&event).unwrap(), Action::None));
        let result = MtpCopyResult {
            copied_files: 3,
            skipped_files: 0,
            copied_bytes: 369,
            total_bytes: 369,
            errors: vec![],
        };
        assert!(matches!(
            run.event(&MtpEvent::CopyFinished(result)).unwrap(),
            Action::Finished(_)
        ));
    }
    #[test]
    fn missing_or_ambiguous_named_devices_and_flattened_collisions_fail_before_copy() {
        let mut run = Run::new(&preset()).unwrap();
        assert!(run.event(&MtpEvent::Devices(vec![])).is_err());
        assert!(
            run.event(&MtpEvent::Devices(vec![
                MtpDevice {
                    id: "one".into(),
                    name: "Phone".into()
                },
                MtpDevice {
                    id: "two".into(),
                    name: "Phone".into()
                }
            ]))
            .is_err()
        );
        run.event(&devices()).unwrap();
        let event = MtpEvent::FilesListed(vec![
            file("a.JPG", MtpFileType::Image),
            file("A.jpg", MtpFileType::Image),
        ]);
        assert!(run.event(&event).is_err());
        assert_eq!(run.phase, Phase::Files);
    }
}
