use super::{RemoteAsset, RemoteSession};
use color_eyre::eyre::{Result, eyre};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

pub(super) fn root() -> Result<PathBuf> {
    if let Some(root) = std::env::var_os("MTP_CULL_DATA_DIR") {
        let root = PathBuf::from(root).join("mtp-staging");
        std::fs::create_dir_all(&root)?;
        return Ok(root);
    }
    let dirs = ProjectDirs::from("com", "fruit", "mtp-cull")
        .ok_or_else(|| eyre!("cannot find application cache"))?;
    let root = dirs.data_local_dir().join("mtp-staging");
    std::fs::create_dir_all(&root)?;
    Ok(root)
}
pub(super) fn directory(device: &str, folder: &str) -> Result<PathBuf> {
    let key = blake3::hash(format!("{device}\0{folder}").as_bytes())
        .to_hex()
        .to_string();
    let path = root()?.join(key);
    std::fs::create_dir_all(&path)?;
    Ok(path)
}
pub(super) fn nonce() -> String {
    format!(
        "{:?}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
    )
}
pub(super) fn asset_key(device: &str, folder: &str, asset: &RemoteAsset, nonce: &str) -> String {
    let stamp = asset.modified.as_deref().unwrap_or(nonce);
    let key = blake3::hash(
        format!(
            "{device}\0{folder}\0{}\0{}\0{stamp}",
            asset.source_path, asset.size
        )
        .as_bytes(),
    )
    .to_hex()
    .to_string();
    if asset.modified.is_none() {
        format!("ephemeral-{key}")
    } else {
        key
    }
}
pub(super) fn path(directory: &Path, asset: &RemoteAsset) -> PathBuf {
    // Keys are generated internally, but hash again to guarantee one safe component.
    directory.join(format!(
        "{}.jpg",
        blake3::hash(asset.cache_key.as_bytes()).to_hex()
    ))
}
#[derive(Serialize, Deserialize)]
struct Verification {
    key: String,
    size: u64,
    modified: String,
    hash: String,
}
fn stamp(path: &Path) -> Result<String> {
    Ok(format!(
        "{:?}",
        std::fs::metadata(path)?
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)?
    ))
}
fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut temporary = tempfile::NamedTempFile::new_in(
        path.parent().ok_or_else(|| eyre!("missing cache parent"))?,
    )?;
    temporary.write_all(&serde_json::to_vec(value)?)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}
pub(super) fn record(path: &Path, asset: &RemoteAsset, hash: String) -> Result<()> {
    let verification = Verification {
        key: asset.cache_key.clone(),
        size: asset.size,
        modified: stamp(path)?,
        hash,
    };
    atomic_json(&path.with_extension("json"), &verification)
}
pub(super) fn verified(path: &Path, asset: &RemoteAsset) -> Option<String> {
    let verification: Verification =
        serde_json::from_slice(&std::fs::read(path.with_extension("json")).ok()?).ok()?;
    if verification.key != asset.cache_key
        || verification.size != asset.size
        || std::fs::metadata(path).ok()?.len() != asset.size
        || stamp(path).ok()? != verification.modified
    {
        return None;
    }
    Some(verification.hash)
}
pub(super) fn restore(session: &mut RemoteSession, directory: &Path) -> Result<()> {
    let retained: std::collections::HashSet<_> = session
        .shots
        .iter()
        .flat_map(|s| &s.assets)
        .filter(|a| a.kind.is_jpeg())
        .flat_map(|a| {
            let p = path(directory, a);
            [p.clone(), p.with_extension("json")]
        })
        .collect();
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_type()?.is_file()
            && cache_file(&entry.path())
            && !retained.contains(&entry.path())
        {
            std::fs::remove_file(entry.path())?;
        }
    }
    for asset in session
        .shots
        .iter_mut()
        .flat_map(|s| &mut s.assets)
        .filter(|a| a.kind.is_jpeg())
    {
        let cached = path(directory, asset);
        if verified(&cached, asset).is_some() {
            asset.preview_path = Some(cached);
        }
    }
    atomic_json(&directory.join("session.json"), session)
}
fn hex_name(name: &str) -> bool {
    name.len() == 64 && name.bytes().all(|c| c.is_ascii_hexdigit())
}
fn cache_file(path: &Path) -> bool {
    path.file_stem()
        .and_then(|s| s.to_str())
        .is_some_and(hex_name)
        && path
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s == "jpg" || s == "json")
}
/// Flat, internally named cache directories only. Never recursively delete paths.
pub(super) fn maintain(active: &Path, required: u64, limit: u64) -> Result<()> {
    maintain_at(&root()?, active, required, limit)
}
fn maintain_at(root: &Path, active: &Path, required: u64, limit: u64) -> Result<()> {
    let root = root.canonicalize()?;
    let active = active.canonicalize()?;
    let mut directories = Vec::new();
    let mut used = 0_u64;
    for entry in std::fs::read_dir(&root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() || !entry.file_name().to_str().is_some_and(hex_name) {
            continue;
        }
        let directory = entry.path().canonicalize()?;
        if directory.parent() != Some(root.as_path()) {
            continue;
        }
        let mut files = Vec::new();
        let mut bytes = 0_u64;
        let mut newest = std::time::UNIX_EPOCH;
        let mut removable = true;
        for file in std::fs::read_dir(&directory)? {
            let file = file?;
            let metadata = file.metadata()?;
            if !file.file_type()?.is_file()
                || !(cache_file(&file.path()) || file.file_name() == "session.json")
            {
                removable = false;
                continue;
            }
            bytes = bytes.saturating_add(metadata.len());
            newest = newest.max(metadata.modified()?);
            files.push(file.path());
        }
        used = used.saturating_add(bytes);
        if directory != active && removable {
            directories.push((newest, directory, files, bytes));
        }
    }
    directories.sort_by_key(|d| d.0);
    for (modified, directory, files, bytes) in directories {
        let stale = modified
            .elapsed()
            .is_ok_and(|age| age.as_secs() > 30 * 86400);
        if used.saturating_add(required) <= limit && !stale {
            break;
        }
        for file in files {
            // Resolve immediately before deleting and reject reparse links out of the cache.
            if std::fs::symlink_metadata(&file)?.file_type().is_symlink()
                || file.canonicalize()?.parent() != Some(directory.as_path())
            {
                return Err(eyre!("cache path changed during maintenance"));
            }
            std::fs::remove_file(file)?;
        }
        std::fs::remove_dir(directory)?;
        used = used.saturating_sub(bytes);
    }
    if used.saturating_add(required) > limit {
        return Err(eyre!("staging cache quota reached"));
    }
    Ok(())
}
pub(super) fn discard_invalid(path: &Path) -> Result<()> {
    for path in [path.to_owned(), path.with_extension("json")] {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mtp_worker::MtpMediaKind;
    #[test]
    fn maintenance_preserves_active_and_unrecognized_paths() {
        let root = tempfile::tempdir().unwrap();
        let active = root.path().join("a".repeat(64));
        let old = root.path().join("b".repeat(64));
        let unknown = root.path().join("personal");
        for dir in [&active, &old, &unknown] {
            std::fs::create_dir(dir).unwrap();
            std::fs::write(dir.join(format!("{}.jpg", "c".repeat(64))), [0; 100]).unwrap();
        }
        maintain_at(root.path(), &active, 50, 150).unwrap();
        assert!(active.exists());
        assert!(!old.exists());
        assert!(unknown.exists());
        assert!(maintain_at(root.path(), &active, 51, 150).is_err());
    }
    #[test]
    fn staged_metadata_rejects_modified_cache_and_unknown_remote_identity() {
        let dir = tempfile::tempdir().unwrap();
        let mut asset = RemoteAsset {
            object_id: "1".into(),
            name: "a.JPG".into(),
            kind: MtpMediaKind::Jpeg,
            size: 5,
            preview_path: None,
            source_path: "DCIM/a.JPG".into(),
            modified: Some("time".into()),
            cache_key: String::new(),
        };
        asset.cache_key = asset_key("camera", "folder", &asset, "one");
        assert_eq!(
            asset.cache_key,
            asset_key("camera", "folder", &asset, "two")
        );
        let cached = path(dir.path(), &asset);
        std::fs::write(&cached, b"image").unwrap();
        record(&cached, &asset, "hash".into()).unwrap();
        assert_eq!(verified(&cached, &asset), Some("hash".into()));
        std::fs::write(&cached, b"changed").unwrap();
        assert!(verified(&cached, &asset).is_none());
        asset.modified = None;
        assert_ne!(
            asset_key("camera", "folder", &asset, "one"),
            asset_key("camera", "folder", &asset, "two")
        );
    }
}
