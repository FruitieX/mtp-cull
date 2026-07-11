use color_eyre::eyre::{Result, WrapErr, bail, eyre};
use std::fs::File;
use std::io::{BufReader, Read, Seek, Write};
use std::path::Path;

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

/// Copies a reader through a same-directory temporary file without replacing an existing target.
pub fn copy_reader_no_clobber(
    input: &mut impl Read,
    expected_size: u64,
    target_path: &Path,
    source_label: &str,
) -> Result<CopyOutcome> {
    let directory = target_path
        .parent()
        .ok_or_else(|| eyre!("no parent directory for {}", target_path.display()))?;
    std::fs::create_dir_all(directory)
        .wrap_err_with(|| format!("failed to create {}", directory.display()))?;

    let mut temporary = tempfile::NamedTempFile::new_in(directory).wrap_err_with(|| {
        format!(
            "failed to create a temporary file in {}",
            directory.display()
        )
    })?;
    let copied = std::io::copy(input, temporary.as_file_mut())?;
    temporary.as_file_mut().flush()?;
    if copied != expected_size {
        bail!(
            "incomplete transfer for {source_label}: expected {expected_size} bytes but received {copied}"
        );
    }

    if target_path.try_exists()? {
        let mut target = File::open(target_path)?;
        if files_equal(temporary.as_file_mut(), &mut target)? {
            return Ok(CopyOutcome::SkippedExisting);
        }
        bail!(
            "{} already exists with different content; refusing to overwrite it",
            target_path.display()
        );
    }

    temporary.as_file_mut().sync_all()?;
    temporary
        .persist_noclobber(target_path)
        .map_err(|error| error.error)
        .wrap_err_with(|| format!("failed to finalize {}", target_path.display()))?;
    Ok(CopyOutcome::Copied)
}
