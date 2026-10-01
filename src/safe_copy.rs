use color_eyre::eyre::{Result, WrapErr, bail, eyre};
use std::fs::File;
use std::io::{BufReader, Read, Seek, Write};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CopyOutcome {
    Copied,
    SkippedExisting,
}

fn files_equal(left: &mut File, right: &mut File) -> Result<bool> {
    left.rewind()?;
    right.rewind()?;
    let mut left = BufReader::new(left);
    let mut right = BufReader::new(right);
    let mut left_buffer = [0_u8; 64 * 1024];
    let mut right_buffer = [0_u8; 64 * 1024];

    loop {
        let left_count = left.read(&mut left_buffer)?;
        let right_count = right.read(&mut right_buffer)?;
        if left_count != right_count || left_buffer[..left_count] != right_buffer[..right_count] {
            return Ok(false);
        }
        if left_count == 0 {
            return Ok(true);
        }
    }
}

/// An in-progress atomic copy through a same-directory temporary file.
pub(crate) struct CopyWriter {
    temporary: tempfile::NamedTempFile,
    expected_size: u64,
    copied: u64,
    target_path: PathBuf,
    source_label: String,
    hash: blake3::Hasher,
}

impl CopyWriter {
    pub(crate) fn new(expected_size: u64, target_path: &Path, source_label: &str) -> Result<Self> {
        let directory = target_path
            .parent()
            .ok_or_else(|| eyre!("no parent directory for {}", target_path.display()))?;
        std::fs::create_dir_all(directory)
            .wrap_err_with(|| format!("failed to create {}", directory.display()))?;

        let temporary = tempfile::NamedTempFile::new_in(directory).wrap_err_with(|| {
            format!(
                "failed to create a temporary file in {}",
                directory.display()
            )
        })?;
        Ok(Self {
            temporary,
            expected_size,
            copied: 0,
            target_path: target_path.to_owned(),
            source_label: source_label.to_owned(),
            hash: blake3::Hasher::new(),
        })
    }

    pub(crate) fn write(&mut self, bytes: &[u8]) -> Result<()> {
        let written = u64::try_from(bytes.len()).expect("slice length exceeds u64");
        let copied = self
            .copied
            .checked_add(written)
            .ok_or_else(|| eyre!("transfer size overflow for {}", self.source_label))?;
        if copied > self.expected_size {
            bail!(
                "incomplete transfer for {}: expected {} bytes but received more than expected",
                self.source_label,
                self.expected_size
            );
        }
        self.temporary.as_file_mut().write_all(bytes)?;
        self.hash.update(bytes);
        self.copied = copied;
        Ok(())
    }

    pub(crate) fn content_hash(&self) -> blake3::Hash {
        self.hash.finalize()
    }

    pub(crate) fn finish(mut self) -> Result<CopyOutcome> {
        self.temporary.as_file_mut().flush()?;
        if self.copied != self.expected_size {
            bail!(
                "incomplete transfer for {}: expected {} bytes but received {}",
                self.source_label,
                self.expected_size,
                self.copied
            );
        }

        if self.target_path.try_exists()? {
            let mut target = File::open(&self.target_path)?;
            if files_equal(self.temporary.as_file_mut(), &mut target)? {
                return Ok(CopyOutcome::SkippedExisting);
            }
            bail!(
                "{} already exists with different content; refusing to overwrite it",
                self.target_path.display()
            );
        }

        self.temporary.as_file_mut().sync_all()?;
        self.temporary
            .persist_noclobber(&self.target_path)
            .map_err(|error| error.error)
            .wrap_err_with(|| format!("failed to finalize {}", self.target_path.display()))?;
        Ok(CopyOutcome::Copied)
    }
}

/// Copies a reader through a same-directory temporary file without replacing an existing target.
pub fn copy_reader_no_clobber(
    input: &mut impl Read,
    expected_size: u64,
    target_path: &Path,
    source_label: &str,
) -> Result<CopyOutcome> {
    copy_reader_with_progress(input, expected_size, target_path, source_label, |_| Ok(()))
}

/// Progress runs between bounded chunks and may abort before publication.
pub fn copy_reader_with_progress(
    input: &mut impl Read,
    expected_size: u64,
    target_path: &Path,
    source_label: &str,
    mut progress: impl FnMut(u64) -> Result<()>,
) -> Result<CopyOutcome> {
    copy_reader_verified(
        input,
        expected_size,
        target_path,
        source_label,
        None,
        &mut progress,
    )
    .map(|(outcome, _)| outcome)
}

pub(crate) fn copy_reader_verified(
    input: &mut impl Read,
    expected_size: u64,
    target_path: &Path,
    source_label: &str,
    expected_hash: Option<&str>,
    mut progress: impl FnMut(u64) -> Result<()>,
) -> Result<(CopyOutcome, String)> {
    let mut writer = CopyWriter::new(expected_size, target_path, source_label)?;
    progress(0)?;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        writer.write(&buffer[..count])?;
        progress(writer.copied)?;
    }
    progress(writer.copied)?;
    let hash = writer.content_hash().to_hex().to_string();
    if expected_hash.is_some_and(|expected| expected != hash) {
        bail!("cached content for {source_label} changed; refusing to publish it");
    }
    Ok((writer.finish()?, hash))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_and_bad_hash_never_publish_partial_files_and_retry_is_safe() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("photo.jpg");
        let bytes = vec![7; 200_000];
        assert!(
            copy_reader_with_progress(
                &mut bytes.as_slice(),
                bytes.len() as u64,
                &target,
                "photo",
                |done| {
                    if done > 0 {
                        bail!("cancelled");
                    }
                    Ok(())
                }
            )
            .is_err()
        );
        assert!(!target.exists());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
        assert!(
            copy_reader_verified(
                &mut bytes.as_slice(),
                bytes.len() as u64,
                &target,
                "photo",
                Some("invalid"),
                |_| Ok(())
            )
            .is_err()
        );
        assert!(!target.exists());
        let hash = blake3::hash(&bytes).to_hex().to_string();
        let first = copy_reader_verified(
            &mut bytes.as_slice(),
            bytes.len() as u64,
            &target,
            "photo",
            Some(&hash),
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(first.0, CopyOutcome::Copied);
        let second = copy_reader_verified(
            &mut bytes.as_slice(),
            bytes.len() as u64,
            &target,
            "photo",
            Some(&hash),
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(second.0, CopyOutcome::SkippedExisting);
        assert_eq!(std::fs::read(&target).unwrap(), bytes);
    }
}
