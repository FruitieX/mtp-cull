use crate::{
    cli::CopyArgs,
    mtp::get_files_list,
    mtp_file::{MtpFile, MtpFileType},
};
use color_eyre::Result;
use humantime::format_duration;
use size::Size;
use std::{path::Path, time::Duration};

enum CopyFileResult {
    Copied,
    Skipped,
}

fn copy_file(
    file: &MtpFile,
    target_path: &str,
    progress_str: Option<String>,
) -> Result<CopyFileResult> {
    let file_path = Path::new(target_path);
    let dirname = file_path
        .parent()
        .ok_or_else(|| eyre!("No parent directory for file {}", file))?;

    if file_path.try_exists().unwrap_or_default() {
        let target_file = std::fs::File::open(file_path)?;
        let target_size = target_file.metadata().unwrap().len();
        if file.size == Size::from_bytes(target_size) {
            warn!(
                "File {} already exists and has identical size, skipping",
                file
            );

            return Ok(CopyFileResult::Skipped);
        }
    }

    if let Some(progress_str) = progress_str {
        info!("({progress_str}) Copying {file} to {target_path}...");
    } else {
        info!("Copying {file} to {target_path}...");
    }
    std::fs::create_dir_all(dirname)?;

    let mut input_stream = file.object.open_read_stream()?;
    let mut output_file = std::fs::File::create(file_path)?;
    std::io::copy(&mut input_stream, &mut output_file)?;

    Ok(CopyFileResult::Copied)
}

pub fn copy_files(args: &CopyArgs) -> Result<()> {
    let date = args
        .date
        .unwrap_or_else(|| chrono::Local::now().naive_local().date());

    let files = get_files_list(args.device.as_deref(), args.source_path.clone())?;

    info!(
        "Copying files to {target_path}",
        target_path = args.pictures_path
    );
    let total_original_size_bytes = files.iter().map(|file| file.size.bytes()).sum::<i64>();
    let mut total_size_bytes = total_original_size_bytes;
    let start_time = std::time::Instant::now();

    let mut copied_size_bytes = 0;
    let mut copied_files = 0;
    let mut skipped_files = 0;
    let mut errored_files = vec![];
    let total_files = files.len();

    for file in files {
        let album_name = if let Some(album_name) = &args.album_name {
            format!("{} {}", &date.format("%Y-%m-%d"), &album_name)
        } else {
            format!("{}", &date.format("%Y-%m-%d"))
        };

        let prefix = match file.file_type {
            MtpFileType::Image => &args.pictures_path,
            MtpFileType::RawImage => &args.raw_path,
            MtpFileType::Video => &args.videos_path,
        };

        let out_path = [
            prefix.to_string(),
            date.format("%Y").to_string(),
            album_name,
            file.name.clone(),
        ]
        .to_vec()
        .join("/");

        let progress = copied_size_bytes as f64 / total_size_bytes as f64 * 100.0;
        let eta = if progress == 0. {
            None
        } else {
            Some(start_time.elapsed().as_secs_f64() / progress * (100.0 - progress))
        };

        let eta = if let Some(eta) = eta {
            format_duration(Duration::from_secs(eta as u64)).to_string()
        } else {
            "N/A".to_string()
        };
        let progress_str = format!("{progress:.2}% ETA: {eta}");

        let result = copy_file(&file, &out_path, Some(progress_str));

        match result {
            Ok(CopyFileResult::Copied) => {
                copied_size_bytes += file.size.bytes();
                copied_files += 1;
            }
            Ok(CopyFileResult::Skipped) => {
                total_size_bytes -= file.size.bytes();
                skipped_files += 1;
            }
            Err(e) => {
                total_size_bytes -= file.size.bytes();
                errored_files.push(file.name.clone());

                if args.keep_going {
                    error!("Error copying file {name}: {e}", name = file.name);
                } else {
                    return Err(e);
                }
            }
        }
    }

    println!("All done!");
    println!(
        "Copied {copied_files} files ({copied_size}) of {total_files} ({total_size}) ({skipped_files} files skipped) in {duration}",
        copied_size = Size::from_bytes(copied_size_bytes),
        total_size = Size::from_bytes(total_original_size_bytes),
        duration = format_duration(Duration::from_secs(start_time.elapsed().as_secs()))
    );
    println!(
        "Effective speed: {speed}/s",
        speed = Size::from_bytes(copied_size_bytes / start_time.elapsed().as_secs() as i64)
    );

    if !errored_files.is_empty() {
        println!("The following files failed to copy:");
        for file in errored_files {
            println!("{}", file);
        }
    }

    Ok(())
}
