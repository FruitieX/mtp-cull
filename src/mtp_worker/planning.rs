use super::{RemoteAsset, RemoteShot};
use chrono::NaiveDate;
use color_eyre::eyre::{Result, eyre};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum MtpMediaKind {
    Jpeg,
    Heif,
    Raw,
    Video,
}

impl MtpMediaKind {
    pub fn is_jpeg(self) -> bool {
        self == Self::Jpeg
    }
}

#[derive(Clone, Debug)]
pub struct ImportPaths {
    pub pictures: Option<PathBuf>,
    pub raw: Option<PathBuf>,
    pub videos: Option<PathBuf>,
    pub date: NaiveDate,
    pub album_name: Option<String>,
}

pub fn classify_media_name(name: &str) -> Option<(String, MtpMediaKind)> {
    let (stem, extension) = name.rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    let kind = match extension.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => MtpMediaKind::Jpeg,
        "heic" | "heif" => MtpMediaKind::Heif,
        "dng" | "raf" | "raw" | "crw" | "cr2" | "cr3" | "arw" | "srf" | "sr2" | "rw2" | "nef"
        | "nrw" => MtpMediaKind::Raw,
        "mp4" | "mov" | "avi" | "mkv" | "wmv" | "flv" | "webm" | "m4v" => MtpMediaKind::Video,
        _ => return None,
    };
    Some((stem.to_owned(), kind))
}

pub(crate) fn normalize_source_path(path: &str) -> Result<PathBuf> {
    let normalized = path.trim_start_matches(['/', '\\']);
    let path = PathBuf::from(normalized);
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return Err(eyre!(
            "MTP source path must be a relative path without '..'"
        ));
    }
    Ok(path)
}

pub(crate) fn plan_import(
    shots: impl IntoIterator<Item = RemoteShot>,
    destinations: &ImportPaths,
) -> Result<Vec<(RemoteAsset, PathBuf)>, String> {
    let mut planned = BTreeMap::new();
    for shot in shots {
        for asset in shot.assets {
            let destination = destination_for(&asset, destinations)?;
            // Imports run on Windows, whose normal MTP destination filesystems are
            // case-insensitive. Catch those collisions before copying any asset.
            let key = destination.to_string_lossy().to_ascii_lowercase();
            if let Some((_, previous)) = planned.insert(key, (destination.clone(), asset.clone())) {
                return Err(format!(
                    "{} and {} both map to {}; refusing an ambiguous flattened import",
                    previous.object_id,
                    asset.object_id,
                    destination.display()
                ));
            }
        }
    }
    Ok(planned
        .into_iter()
        .map(|(_, (destination, asset))| (asset, destination))
        .collect())
}

fn destination_for(asset: &RemoteAsset, destinations: &ImportPaths) -> Result<PathBuf, String> {
    let mut components = Path::new(&asset.name).components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return Err(format!(
            "MTP file name {:?} is not a single file name",
            asset.name
        ));
    }
    let (parent, label) = match asset.kind {
        MtpMediaKind::Jpeg | MtpMediaKind::Heif => (&destinations.pictures, "JPEG/HEIF"),
        MtpMediaKind::Raw => (&destinations.raw, "RAW"),
        MtpMediaKind::Video => (&destinations.videos, "video"),
    };
    let parent = parent
        .as_ref()
        .ok_or_else(|| format!("a {label} destination is required for {}", asset.name))?;
    if let Some(album_name) = &destinations.album_name {
        single_path_component(album_name, "album name")?;
    }
    let album = destinations
        .album_name
        .as_ref()
        .map(|name| format!("{} {name}", destinations.date.format("%Y-%m-%d")))
        .unwrap_or_else(|| destinations.date.format("%Y-%m-%d").to_string());
    Ok(parent
        .join(destinations.date.format("%Y").to_string())
        .join(album)
        .join(&asset.name))
}

fn single_path_component(value: &str, description: &str) -> Result<(), String> {
    let mut components = Path::new(value).components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return Err(format!(
            "{description} must be a single directory or file name"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ImportPaths, MtpMediaKind, classify_media_name, plan_import};
    use crate::mtp_worker::{RemoteAsset, RemoteShot};
    use chrono::NaiveDate;
    use std::path::PathBuf;

    fn asset(id: &str, name: &str, kind: MtpMediaKind) -> RemoteAsset {
        RemoteAsset {
            object_id: id.to_owned(),
            name: name.to_owned(),
            kind,
            size: 1,
            preview_path: None,
        }
    }

    fn paths() -> ImportPaths {
        ImportPaths {
            pictures: Some(PathBuf::from("pictures")),
            raw: Some(PathBuf::from("raw")),
            videos: Some(PathBuf::from("videos")),
            date: NaiveDate::from_ymd_opt(2026, 7, 11).unwrap(),
            album_name: Some("Trip".to_owned()),
        }
    }

    #[test]
    fn classifies_the_culling_media_extensions_and_preserves_exact_stems() {
        assert_eq!(
            classify_media_name("DSCF0001.JPEG"),
            Some(("DSCF0001".to_owned(), MtpMediaKind::Jpeg))
        );
        assert_eq!(
            classify_media_name("DSCF0001.HEIC"),
            Some(("DSCF0001".to_owned(), MtpMediaKind::Heif))
        );
        assert_eq!(
            classify_media_name("DSCF0001.RAF"),
            Some(("DSCF0001".to_owned(), MtpMediaKind::Raw))
        );
        assert_eq!(
            classify_media_name("clip.MOV"),
            Some(("clip".to_owned(), MtpMediaKind::Video))
        );
        assert_eq!(classify_media_name("preview.png"), None);
    }

    #[test]
    fn plans_all_companions_to_their_kind_directories() {
        let shot = RemoteShot {
            id: "shot".to_owned(),
            stem: "DSCF0001".to_owned(),
            assets: vec![
                asset("jpeg", "DSCF0001.JPG", MtpMediaKind::Jpeg),
                asset("raw", "DSCF0001.RAF", MtpMediaKind::Raw),
            ],
        };

        let plan = plan_import([shot], &paths()).unwrap();

        assert_eq!(
            plan[0].1,
            PathBuf::from("pictures/2026/2026-07-11 Trip/DSCF0001.JPG")
        );
        assert_eq!(
            plan[1].1,
            PathBuf::from("raw/2026/2026-07-11 Trip/DSCF0001.RAF")
        );
    }

    #[test]
    fn rejects_colliding_flattened_imports_before_copying() {
        let shots = [
            RemoteShot {
                id: "one".to_owned(),
                stem: "one".to_owned(),
                assets: vec![asset("one", "IMG_0001.JPG", MtpMediaKind::Jpeg)],
            },
            RemoteShot {
                id: "two".to_owned(),
                stem: "two".to_owned(),
                assets: vec![asset("two", "IMG_0001.JPG", MtpMediaKind::Jpeg)],
            },
        ];

        assert!(plan_import(shots, &paths()).is_err());
    }

    #[test]
    fn rejects_case_insensitive_destination_collisions() {
        let shots = [
            RemoteShot {
                id: "one".to_owned(),
                stem: "one".to_owned(),
                assets: vec![asset("one", "IMG_0001.JPG", MtpMediaKind::Jpeg)],
            },
            RemoteShot {
                id: "two".to_owned(),
                stem: "two".to_owned(),
                assets: vec![asset("two", "img_0001.jpg", MtpMediaKind::Jpeg)],
            },
        ];

        assert!(plan_import(shots, &paths()).is_err());
    }
}
