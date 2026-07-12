use color_eyre::eyre::{Result, WrapErr, eyre};
use directories::ProjectDirs;
use log::warn;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

const LOCK_FILE_NAME: &str = ".active.lock";

pub(crate) struct PreviewCache {
    path: PathBuf,
    lock: File,
}

impl PreviewCache {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

fn cache_root() -> Result<PathBuf> {
    ProjectDirs::from("com", "fruit", "mtp-cull")
        .map(|directories| directories.data_local_dir().join("mtp-preview-cache"))
        .ok_or_else(|| eyre!("could not determine the local application data directory"))
}

pub(crate) fn create_preview_cache_directory() -> Result<PreviewCache> {
    let root = cache_root()?;
    std::fs::create_dir_all(&root)
        .wrap_err_with(|| format!("failed to create {}", root.display()))?;
    let path = tempfile::Builder::new()
        .prefix("session-")
        .tempdir_in(root)
        .wrap_err("failed to create an MTP preview session cache")?
        .keep();
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path.join(LOCK_FILE_NAME))
        .wrap_err("failed to create an MTP preview cache ownership lock")?;
    lock.try_lock()
        .wrap_err("failed to lock the MTP preview session cache")?;
    Ok(PreviewCache { path, lock })
}

pub(crate) fn remove_preview_cache(cache: PreviewCache) {
    let PreviewCache { path, lock } = cache;
    if let Err(error) = lock.unlock() {
        warn!(
            "failed to unlock MTP preview cache {}: {error}",
            path.display()
        );
    }
    drop(lock);
    remove_preview_cache_directory(&path);
}

fn remove_preview_cache_directory(path: &Path) {
    if let Err(error) = std::fs::remove_dir_all(path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        warn!(
            "failed to remove MTP preview cache {}: {error}",
            path.display()
        );
    }
}

/// Remove session directories left behind by a previous process or a worker failure.
pub(crate) fn reconcile_preview_cache() -> Result<usize> {
    let root = cache_root()?;
    if !root.try_exists()? {
        return Ok(0);
    }

    reconcile_preview_cache_root(&root)
}

fn reconcile_preview_cache_root(root: &Path) -> Result<usize> {
    let mut removed = 0;
    for entry in
        std::fs::read_dir(root).wrap_err_with(|| format!("failed to inspect {}", root.display()))?
    {
        let entry = entry?;
        let file_name = entry.file_name();
        if !file_name.to_string_lossy().starts_with("session-") || !entry.file_type()?.is_dir() {
            continue;
        }
        let path = entry.path();
        let lock_path = path.join(LOCK_FILE_NAME);
        let lock = match OpenOptions::new().read(true).write(true).open(&lock_path) {
            Ok(lock) => match lock.try_lock() {
                Ok(()) => Some(lock),
                Err(TryLockError::WouldBlock) => None,
                Err(TryLockError::Error(error)) => {
                    warn!(
                        "failed to check MTP preview cache lock {}: {error}",
                        lock_path.display()
                    );
                    continue;
                }
            },
            Err(error) if error.kind() == ErrorKind::NotFound => {
                let lock = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .open(&lock_path)
                    .wrap_err_with(|| format!("failed to claim stale cache {}", path.display()))?;
                lock.try_lock()
                    .wrap_err_with(|| format!("failed to lock stale cache {}", path.display()))?;
                Some(lock)
            }
            Err(error) => {
                warn!(
                    "failed to open MTP preview cache lock {}: {error}",
                    lock_path.display()
                );
                continue;
            }
        };
        let Some(lock) = lock else {
            continue;
        };
        drop(lock);
        match std::fs::remove_dir_all(&path) {
            Ok(()) => removed += 1,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => warn!(
                "failed to reconcile stale MTP preview cache {}: {error}",
                path.display()
            ),
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::{
        LOCK_FILE_NAME, PreviewCache, reconcile_preview_cache_root, remove_preview_cache,
        remove_preview_cache_directory,
    };
    use std::fs::OpenOptions;

    #[test]
    fn removing_a_missing_cache_is_harmless() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("session-missing");
        remove_preview_cache_directory(&missing);
    }

    #[test]
    fn reconciliation_removes_only_session_directories() {
        let root = tempfile::tempdir().unwrap();
        let stale = root.path().join("session-stale");
        let unrelated = root.path().join("manual-backup");
        std::fs::create_dir(&stale).unwrap();
        std::fs::create_dir(&unrelated).unwrap();

        assert_eq!(reconcile_preview_cache_root(root.path()).unwrap(), 1);
        assert!(!stale.exists());
        assert!(unrelated.exists());
    }

    #[test]
    fn reconciliation_preserves_a_locked_active_session() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("session-active");
        std::fs::create_dir(&path).unwrap();
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path.join(LOCK_FILE_NAME))
            .unwrap();
        lock.try_lock().unwrap();

        assert_eq!(reconcile_preview_cache_root(root.path()).unwrap(), 0);
        assert!(path.exists());

        remove_preview_cache(PreviewCache { path, lock });
    }
}
