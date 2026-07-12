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
        self.copied = copied;
        Ok(())
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

/// Copies a reader through the same safe finalization path while reporting bytes written.
///
/// The callback runs after each source chunk has been written to the temporary file. Returning an
/// error abandons the temporary file and never creates or replaces the destination.
#[cfg(windows)]
pub(crate) fn copy_reader_no_clobber_with_progress(
    input: &mut impl Read,
    expected_size: u64,
    target_path: &Path,
    source_label: &str,
    mut on_progress: impl FnMut(u64) -> Result<()>,
) -> Result<CopyOutcome> {
    let mut writer = CopyWriter::new(expected_size, target_path, source_label)?;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        writer.write(&buffer[..count])?;
        on_progress(writer.copied)?;
    }
    writer.finish()
}

#[cfg(test)]
mod tests {
    use super::{CopyOutcome, CopyWriter};

    fn copy_bytes(
        bytes: &[u8],
        expected_size: u64,
        target: &std::path::Path,
    ) -> color_eyre::eyre::Result<CopyOutcome> {
        let mut writer = CopyWriter::new(expected_size, target, "test source")?;
        writer.write(bytes)?;
        writer.finish()
    }

    #[test]
    fn copies_atomically_and_confirms_identical_existing_content() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("photo.jpg");

        assert_eq!(
            copy_bytes(b"photo", 5, &target).unwrap(),
            CopyOutcome::Copied
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"photo");
        assert_eq!(
            copy_bytes(b"photo", 5, &target).unwrap(),
            CopyOutcome::SkippedExisting
        );
    }

    #[test]
    fn incomplete_transfer_never_creates_destination() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("photo.jpg");

        assert!(copy_bytes(b"short", 10, &target).is_err());
        assert!(!target.exists());
    }

    #[test]
    fn differing_existing_content_is_never_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("photo.jpg");
        std::fs::write(&target, b"other").unwrap();

        assert!(copy_bytes(b"photo", 5, &target).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"other");
    }

    #[test]
    fn abandoning_a_writer_never_creates_destination() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("photo.jpg");
        let mut writer = CopyWriter::new(10, &target, "test source").unwrap();
        writer.write(b"partial").unwrap();
        drop(writer);

        assert!(!target.exists());
    }
}
