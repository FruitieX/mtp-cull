use crate::mtp_file::{MtpFile, MtpFileSource, MtpFileType};
use color_eyre::eyre::{Result, WrapErr, eyre};
use log::{debug, info};
use size::Size;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use winmtp::PortableDevices::WPD_OBJECT_SIZE;
use winmtp::Provider;
use winmtp::device::{BasicDevice, Device};
use winmtp::object::{Object, ObjectType};

struct WindowsMtpFileSource(Object);

impl MtpFileSource for WindowsMtpFileSource {
    fn open(&self) -> Result<Box<dyn Read + '_>> {
        Ok(Box::new(self.0.open_read_stream()?))
    }
}

pub fn list_devices() -> Result<Vec<String>> {
    let provider = Provider::new().wrap_err("failed to initialize Windows Portable Devices")?;
    let devices = provider
        .enumerate_devices()
        .wrap_err("failed to enumerate MTP devices")?;
    Ok(devices
        .into_iter()
        .map(|device| device.friendly_name().to_owned())
        .collect())
}

fn open_device(name: Option<&str>) -> Result<Device> {
    let provider = Provider::new().wrap_err("failed to initialize Windows Portable Devices")?;
    let devices = provider
        .enumerate_devices()
        .wrap_err("failed to enumerate MTP devices")?;

    let basic_device: BasicDevice = match name {
        Some(name) => devices
            .into_iter()
            .find(|device| device.friendly_name() == name)
            .ok_or_else(|| eyre!("no MTP device named {name:?} found"))?,
        None => devices
            .into_iter()
            .next()
            .ok_or_else(|| eyre!("no MTP devices found"))?,
    };

    basic_device
        .open(&winmtp::make_current_app_identifiers!(), true)
        .wrap_err_with(|| {
            format!(
                "failed to open MTP device {:?}",
                basic_device.friendly_name()
            )
        })
}

fn get_mtp_files_recursive(obj: Object, parent_path: Option<&str>) -> Result<Vec<MtpFile>> {
    let mut files = Vec::new();

    for child in obj.children().wrap_err("failed to enumerate MTP objects")? {
        let name = child.name().to_string_lossy();
        let path = parent_path
            .map(|parent| format!("{parent}/{name}"))
            .unwrap_or_else(|| name.clone());

        if child.object_type().is_file_like()
            && let Ok(file_type) = MtpFileType::try_from_file_name(&name)
        {
            let properties = child
                .properties(&[WPD_OBJECT_SIZE])
                .wrap_err_with(|| format!("failed to read metadata for {path}"))?;
            let size = properties
                .get_u64(&WPD_OBJECT_SIZE)
                .wrap_err_with(|| format!("failed to read 64-bit file size for {path}"))?;
            let file = MtpFile::new(
                name,
                path.clone(),
                file_type,
                size,
                Box::new(WindowsMtpFileSource(child.clone())),
            );
            debug!("Found file: {file}");
            files.push(file);
        }

        if child.object_type() == ObjectType::Folder {
            files.extend(get_mtp_files_recursive(child, Some(&path))?);
        }
    }

    Ok(files)
}

fn normalize_device_path(path: &str) -> Result<PathBuf> {
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

pub fn get_files_list(device_name: Option<&str>, path: Option<String>) -> Result<Vec<MtpFile>> {
    let device = open_device(device_name)?;
    let content = device
        .content()
        .wrap_err("failed to access MTP device content")?;
    let root = content
        .root()
        .wrap_err("failed to access MTP device root")?;
    let object = match path.as_deref() {
        Some(path) => root
            .object_by_path(Path::new(&normalize_device_path(path)?))
            .wrap_err_with(|| format!("failed to find MTP path {path:?}"))?,
        None => root,
    };

    info!(
        "Gathering files list from {}...",
        path.as_deref().unwrap_or("device root")
    );
    let mut files = get_mtp_files_recursive(object, path.as_deref())?;
    files.sort_by(|a, b| {
        a.file_type
            .copy_order()
            .cmp(&b.file_type.copy_order())
            .then(a.name.cmp(&b.name))
    });

    let total_size = files.iter().map(|file| file.size).sum::<u64>();
    let image_count = files
        .iter()
        .filter(|file| file.file_type == MtpFileType::Image)
        .count();
    let raw_count = files
        .iter()
        .filter(|file| file.file_type == MtpFileType::RawImage)
        .count();
    let video_count = files
        .iter()
        .filter(|file| file.file_type == MtpFileType::Video)
        .count();
    let total_files = files.len();
    info!(
        "Found {total_files} files, totalling {} ({image_count} photos, {raw_count} RAW files, {video_count} videos)",
        Size::from_bytes(total_size)
    );

    Ok(files)
}
