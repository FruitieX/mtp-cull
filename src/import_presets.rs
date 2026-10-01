//! User-owned copy profiles, independent of review settings and the executable.
use crate::cli::CopyArgs;
use chrono::NaiveDate;
use color_eyre::eyre::{Result, WrapErr, bail, eyre};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ImportPreset {
    pub name: String,
    pub device: String,
    pub source_path: String,
    pub pictures_path: String,
    pub videos_path: String,
    pub raw_path: String,
    /// Empty means today, evaluated each time the preset runs.
    pub date: String,
    pub album_name: String,
    pub keep_going: bool,
}

impl ImportPreset {
    pub fn args(&self, today: NaiveDate) -> Result<CopyArgs> {
        if self.name.trim().is_empty() {
            bail!("Give the preset a name");
        }
        for (label, path) in [
            ("Photo", &self.pictures_path),
            ("Video", &self.videos_path),
            ("RAW", &self.raw_path),
        ] {
            if path.trim().is_empty() {
                bail!("{label} destination is required");
            }
        }
        let optional = |value: &str| (!value.trim().is_empty()).then(|| value.trim().to_owned());
        let album_name = optional(&self.album_name);
        if let Some(album) = &album_name {
            let mut components = Path::new(album).components();
            if !matches!(components.next(), Some(Component::Normal(_)))
                || components.next().is_some()
                || album.contains(['/', '\\'])
            {
                bail!("Album name must be a single directory name");
            }
        }
        let date = if self.date.trim().is_empty() {
            today
        } else {
            NaiveDate::parse_from_str(self.date.trim(), "%Y-%m-%d")
                .wrap_err("Date must be YYYY-MM-DD, or empty for today")?
        };
        Ok(CopyArgs {
            device: optional(&self.device),
            source_path: optional(&self.source_path),
            date: Some(date),
            pictures_path: self.pictures_path.trim().into(),
            videos_path: self.videos_path.trim().into(),
            raw_path: self.raw_path.trim().into(),
            album_name,
            keep_going: self.keep_going,
        })
    }

    pub fn detail(&self) -> String {
        format!(
            "{} / {}\nPhotos: {}\nVideos: {}\nRAW: {}\nDate: {}{}",
            if self.device.trim().is_empty() {
                "First connected device"
            } else {
                &self.device
            },
            if self.source_path.trim().is_empty() {
                "Device root"
            } else {
                &self.source_path
            },
            self.pictures_path,
            self.videos_path,
            self.raw_path,
            if self.date.trim().is_empty() {
                "Today"
            } else {
                &self.date
            },
            if self.album_name.is_empty() {
                String::new()
            } else {
                format!(" · {}", self.album_name)
            }
        )
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Document {
    presets: Vec<ImportPreset>,
}

pub fn config_path() -> Result<PathBuf> {
    // Development/smoke isolation applies to the config as well as the database.
    if let Some(root) = std::env::var_os("MTP_CULL_DATA_DIR") {
        return Ok(PathBuf::from(root).join("import-presets.json"));
    }
    let directories = ProjectDirs::from("com", "fruit", "mtp-cull")
        .ok_or_else(|| eyre!("Cannot find the user configuration directory"))?;
    Ok(directories.config_dir().join("import-presets.json"))
}

pub fn load(path: &Path) -> Result<Vec<ImportPreset>> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let document: Document = serde_json::from_reader(std::io::BufReader::new(file))
        .wrap_err_with(|| format!("Could not read presets from {}", path.display()))?;
    Ok(document.presets)
}

pub fn save(path: &Path, presets: &[ImportPreset]) -> Result<()> {
    // Refuse to replace malformed or unsupported externally edited configuration.
    load(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| eyre!("Invalid preset configuration path"))?;
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(
        &mut temporary,
        &Document {
            presets: presets.to_vec(),
        },
    )?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).wrap_err("Could not save presets")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn preset() -> ImportPreset {
        ImportPreset {
            name: "Phone backup".into(),
            device: "Phone".into(),
            source_path: "DCIM/Camera".into(),
            pictures_path: "photos".into(),
            videos_path: "videos".into(),
            raw_path: "raw".into(),
            ..Default::default()
        }
    }
    #[test]
    fn config_starts_empty_and_round_trips_edits_without_touching_bad_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config/import-presets.json");
        assert!(load(&path).unwrap().is_empty());
        assert!(!path.exists());
        let mut entries = vec![preset()];
        save(&path, &entries).unwrap();
        assert_eq!(load(&path).unwrap(), entries);
        entries[0].name = "Renamed photographer".into();
        save(&path, &entries).unwrap();
        assert_eq!(load(&path).unwrap(), entries);
        save(&path, &[]).unwrap();
        assert!(load(&path).unwrap().is_empty());
        std::fs::write(&path, "{broken").unwrap();
        assert!(load(&path).is_err());
        assert!(save(&path, &entries).is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "{broken");
    }
    #[test]
    fn today_is_resolved_per_run_and_invalid_dates_destinations_and_albums_are_rejected() {
        let mut preset = preset();
        for day in [1, 2] {
            let today = NaiveDate::from_ymd_opt(2026, 10, day).unwrap();
            assert_eq!(preset.args(today).unwrap().date, Some(today));
        }
        let today = NaiveDate::from_ymd_opt(2026, 10, 2).unwrap();
        preset.date = "2026-09-15".into();
        assert_eq!(
            preset.args(today).unwrap().date,
            NaiveDate::from_ymd_opt(2026, 9, 15)
        );
        preset.date = "bad date".into();
        assert!(preset.args(today).is_err());
        preset.date.clear();
        preset.album_name = "../escape".into();
        assert!(preset.args(today).is_err());
        preset.album_name.clear();
        preset.raw_path.clear();
        assert!(preset.args(today).is_err());
    }
}
