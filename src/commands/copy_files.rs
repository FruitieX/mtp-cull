use crate::{
    cli::CopyArgs,
    mtp_file::MtpFileType,
    mtp_worker::{MtpBackend, MtpCopyItem, MtpFileInfo},
};
use color_eyre::eyre::{Result, bail};
use humantime::format_duration;
use log::error;
use size::Size;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

fn single_path_component(value: &str, description: &str) -> Result<()> {
    let mut components = Path::new(value).components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        bail!("{description} must be a single directory or file name");
    }
    Ok(())
}

fn destination_for(
    file: &MtpFileInfo,
    args: &CopyArgs,
    date: chrono::NaiveDate,
) -> Result<PathBuf> {
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
    let backend = MtpBackend::spawn()?;
    let device = backend.select_device(args.device.as_deref())?;
    let files = backend.list_files(device.id, args.source_path.clone())?;
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
    let copy_items = files
        .iter()
        .zip(&destinations)
        .map(|(file, destination)| MtpCopyItem {
            object_id: file.object_id.clone(),
            source_path: file.path.clone(),
            size: file.size,
            destination: destination.clone(),
        })
        .collect();
    let start_time = std::time::Instant::now();
    let result = backend.copy_files(copy_items, args.keep_going)?;
    let elapsed = start_time.elapsed();

    let speed = if elapsed.is_zero() {
        0
    } else {
        (result.copied_bytes as f64 / elapsed.as_secs_f64()) as u64
    };
    println!(
        "Copied {} files ({}) of {total_files} ({}) ({} skipped) in {}",
        result.copied_files,
        Size::from_bytes(result.copied_bytes),
        Size::from_bytes(total_size),
        result.skipped_files,
        format_duration(elapsed)
    );
    println!("Effective speed: {}/s", Size::from_bytes(speed));

    if !result.errors.is_empty() {
        for error in &result.errors {
            error!("Error copying {}: {}", error.source_path, error.message);
            eprintln!("Failed: {}: {}", error.source_path, error.message);
        }
        bail!("{} file(s) failed to copy", result.errors.len());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::destination_for;
    use crate::cli::CopyArgs;
    use crate::mtp_file::MtpFileType;
    use crate::mtp_worker::MtpFileInfo;
    use chrono::NaiveDate;

    fn file(name: &str) -> MtpFileInfo {
        MtpFileInfo {
            object_id: format!("id-{name}"),
            name: name.to_owned(),
            path: format!("DCIM/{name}"),
            file_type: MtpFileType::Image,
            size: 5,
        }
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
            &file("photo.jpg"),
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
                &MtpFileInfo {
                    name: "photo.jpg".to_owned(),
                    ..file("photo.jpg")
                },
                &args,
                NaiveDate::from_ymd_opt(2026, 7, 11).unwrap(),
            )
            .is_err()
        );
    }
}
