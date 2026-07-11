use crate::{
    cli::CopyArgs,
    mtp::get_files_list,
    mtp_file::{MtpFile, MtpFileType},
    safe_copy::{CopyOutcome, copy_reader_no_clobber},
};
use color_eyre::eyre::{Result, bail};
use humantime::format_duration;
use log::{error, info, warn};
use size::Size;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

enum CopyFileResult {
    Copied,
    Skipped,
}

fn copy_file(file: &MtpFile, target_path: &Path, progress: &str) -> Result<CopyFileResult> {
    info!(
        "({progress}) Copying {file} to {}...",
        target_path.display()
    );
    let mut input = file.open()?;
    match copy_reader_no_clobber(&mut input, file.size, target_path, &file.path)? {
        CopyOutcome::Copied => Ok(CopyFileResult::Copied),
        CopyOutcome::SkippedExisting => {
            warn!(
                "{} already exists with identical content, skipping",
                target_path.display()
            );
            Ok(CopyFileResult::Skipped)
        }
    }
}

fn single_path_component(value: &str, description: &str) -> Result<()> {
    let mut components = Path::new(value).components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        bail!("{description} must be a single directory or file name");
    }
    Ok(())
}

fn destination_for(file: &MtpFile, args: &CopyArgs, date: chrono::NaiveDate) -> Result<PathBuf> {
    single_path_component(&file.name, "MTP file name")?;
    if let Some(album_name) = &args.album_name {
        single_path_component(album_name, "album name")?;
    }

    let album = args
        .album_name
        .as_ref()
        .map(|name| format!("{} {name}", date.format("%Y-%m-%d")))
        .unwrap_or_else(|| date.format("%Y-%m-%d").to_string());
    let prefix = match file.file_type {
        MtpFileType::Image => &args.pictures_path,
        MtpFileType::RawImage => &args.raw_path,
        MtpFileType::Video => &args.videos_path,
    };

    Ok(Path::new(prefix)
        .join(date.format("%Y").to_string())
        .join(album)
        .join(&file.name))
}

pub fn copy_files(args: &CopyArgs) -> Result<()> {
    let date = args
        .date
        .unwrap_or_else(|| chrono::Local::now().date_naive());
    let files = get_files_list(args.device.as_deref(), args.source_path.clone())?;
    let destinations = files
        .iter()
        .map(|file| destination_for(file, args, date))
        .collect::<Result<Vec<_>>>()?;

    let mut planned = HashMap::new();
    for (file, destination) in files.iter().zip(&destinations) {
        if let Some(previous) = planned.insert(destination.clone(), file.path.as_str()) {
            bail!(
                "{} and {} both map to {}; refusing an ambiguous flattened copy",
                previous,
                file.path,
                destination.display()
            );
        }
    }

    let total_size = files.iter().map(|file| file.size).sum::<u64>();
    let total_files = files.len();
    let start_time = std::time::Instant::now();
    let mut processed_size = 0_u64;
    let mut copied_size = 0_u64;
    let mut copied_files = 0_usize;
    let mut skipped_files = 0_usize;
    let mut errors = Vec::new();

    for (file, destination) in files.iter().zip(&destinations) {
        let progress = if total_size == 0 {
            100.0
        } else {
            processed_size as f64 / total_size as f64 * 100.0
        };
        let eta = if processed_size == 0 {
            "N/A".to_owned()
        } else {
            let remaining = total_size.saturating_sub(processed_size) as f64;
            let seconds = start_time.elapsed().as_secs_f64() * remaining / processed_size as f64;
            format_duration(Duration::from_secs_f64(seconds)).to_string()
        };
        let progress_text = format!("{progress:.2}% ETA: {eta}");

        match copy_file(file, destination, &progress_text) {
            Ok(CopyFileResult::Copied) => {
                copied_size += file.size;
                copied_files += 1;
            }
            Ok(CopyFileResult::Skipped) => skipped_files += 1,
            Err(error) if args.keep_going => {
                error!("Error copying {}: {error:#}", file.path);
                errors.push((file.path.clone(), error));
            }
            Err(error) => return Err(error),
        }
        processed_size += file.size;
    }

    let elapsed = start_time.elapsed();
    let speed = if elapsed.is_zero() {
        0
    } else {
        (copied_size as f64 / elapsed.as_secs_f64()) as u64
    };
    println!(
        "Copied {copied_files} files ({}) of {total_files} ({}) ({skipped_files} skipped) in {}",
        Size::from_bytes(copied_size),
        Size::from_bytes(total_size),
        format_duration(elapsed)
    );
    println!("Effective speed: {}/s", Size::from_bytes(speed));

    if !errors.is_empty() {
        for (path, error) in &errors {
            eprintln!("Failed: {path}: {error:#}");
        }
        bail!("{} file(s) failed to copy", errors.len());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{CopyFileResult, copy_file, destination_for};
    use crate::cli::CopyArgs;
    use crate::mtp_file::{MtpFile, MtpFileSource, MtpFileType};
    use chrono::NaiveDate;
    use color_eyre::Result;
    use std::io::{Cursor, Read};

    struct Bytes(Vec<u8>);

    impl MtpFileSource for Bytes {
        fn open(&self) -> Result<Box<dyn Read + '_>> {
            Ok(Box::new(Cursor::new(&self.0)))
        }
    }

    fn file(name: &str, bytes: &[u8], advertised_size: u64) -> MtpFile {
        MtpFile::new(
            name.to_owned(),
            format!("DCIM/{name}"),
            MtpFileType::Image,
            advertised_size,
            Box::new(Bytes(bytes.to_vec())),
        )
    }

    #[test]
    fn builds_destination_with_path_joins() {
        let args = CopyArgs {
            device: None,
            source_path: None,
            date: None,
            pictures_path: "pictures".to_owned(),
            videos_path: "videos".to_owned(),
            raw_path: "raw".to_owned(),
            album_name: Some("Trip".to_owned()),
            keep_going: false,
        };
        let destination = destination_for(
            &file("photo.jpg", b"photo", 5),
            &args,
            NaiveDate::from_ymd_opt(2026, 7, 11).unwrap(),
        )
        .unwrap();

        assert_eq!(
            destination,
            std::path::Path::new("pictures")
                .join("2026")
                .join("2026-07-11 Trip")
                .join("photo.jpg")
        );
    }

    #[test]
    fn rejects_album_path_traversal() {
        let args = CopyArgs {
            device: None,
            source_path: None,
            date: None,
            pictures_path: "pictures".to_owned(),
            videos_path: "videos".to_owned(),
            raw_path: "raw".to_owned(),
            album_name: Some("../escape".to_owned()),
            keep_going: false,
        };
        assert!(
            destination_for(
                &file("photo.jpg", b"photo", 5),
                &args,
                NaiveDate::from_ymd_opt(2026, 7, 11).unwrap(),
            )
            .is_err()
        );
    }

    #[test]
    fn copies_atomically_and_confirms_identical_existing_content() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("photo.jpg");
        let source = file("photo.jpg", b"photo", 5);

        assert!(matches!(
            copy_file(&source, &target, "test").unwrap(),
            CopyFileResult::Copied
        ));
        assert_eq!(std::fs::read(&target).unwrap(), b"photo");
        assert!(matches!(
            copy_file(&source, &target, "test").unwrap(),
            CopyFileResult::Skipped
        ));
    }

    #[test]
    fn incomplete_transfer_never_creates_destination() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("photo.jpg");
        let source = file("photo.jpg", b"short", 10);

        assert!(copy_file(&source, &target, "test").is_err());
        assert!(!target.exists());
    }

    #[test]
    fn differing_existing_content_is_never_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("photo.jpg");
        std::fs::write(&target, b"other").unwrap();
        let source = file("photo.jpg", b"photo", 5);

        assert!(copy_file(&source, &target, "test").is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"other");
    }
}
