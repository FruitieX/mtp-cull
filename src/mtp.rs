use crate::mtp_file::{MtpFile, MtpFileType};
use color_eyre::eyre::Result;
use size::Size;
use std::path::Path;
use winmtp::device::device_values::AppIdentifiers;
use winmtp::device::{BasicDevice, Device};
use winmtp::object::Object;
use winmtp::PortableDevices::{WPD_OBJECT_DATE_AUTHORED, WPD_OBJECT_SIZE};
use winmtp::Provider;

pub fn get_device_by_name(name: &str) -> Result<BasicDevice> {
    let provider = Provider::new().unwrap();
    let devices = provider.enumerate_devices().unwrap();

    for device in devices {
        if device.friendly_name() == name {
            return Ok(device);
        }
    }

    Err(eyre!("No device with name {name} found"))
}

pub fn open_device(name: Option<&str>) -> Result<Device> {
    let provider = Provider::new().unwrap();
    let devices = provider.enumerate_devices().unwrap();

    let basic_device = match name {
        Some(name) => get_device_by_name(name),
        None => devices
            .first()
            .ok_or_else(|| eyre!("No MTP devices found"))
            .cloned(),
    }?;

    let app_ident = winmtp::make_current_app_identifiers!();

    let device = basic_device.open(&app_ident, true)?;

    Ok(device)
}

pub fn get_mtp_files_recursive(obj: Object, parent_path: Option<String>) -> Result<Vec<MtpFile>> {
    let mut files = vec![];

    for child in obj.children()? {
        let name = child.name().to_string_lossy();
        let path = if let Some(parent_path) = parent_path.as_ref() {
            format!("{}/{}", parent_path, name)
        } else {
            name.to_string()
        };

        let properties = child.properties(&[WPD_OBJECT_SIZE, WPD_OBJECT_DATE_AUTHORED])?;
        let object_size = properties.get_u32(&WPD_OBJECT_SIZE);

        let file_type = MtpFileType::try_from_file_name(&name);

        if let (Ok(object_size), Ok(file_type)) = (object_size, file_type) {
            let file_size = Size::from_bytes(object_size);

            let file = MtpFile {
                name: name.to_string(),
                path: path.clone(),
                file_type,
                size: file_size,
                object: child.clone(),
            };

            debug!("Found file: {file}");

            files.push(file);
        }

        let mut child_files = get_mtp_files_recursive(child, Some(path.clone()))?;
        files.append(&mut child_files);
    }

    Ok(files)
}

fn get_object_by_path(device_name: Option<&str>, path: Option<String>) -> Result<Object> {
    let device = open_device(device_name)?;
    let content = device.content()?;
    let root_obj = content.root()?;

    if let Some(path) = path {
        let object_by_path = root_obj.object_by_path(Path::new(&path))?;

        Ok(object_by_path)
    } else {
        Ok(root_obj)
    }
}

fn sort_files(files: &mut [MtpFile]) {
    files.sort_by(|a, b| {
        a.file_type
            .partial_cmp(&b.file_type)
            .unwrap()
            .then(a.name.cmp(&b.name))
    });
}

pub fn get_files_list(device_name: Option<&str>, path: Option<String>) -> Result<Vec<MtpFile>> {
    let object_by_path = get_object_by_path(device_name, path.clone())?;

    info!(
        "Gathering files list from {}...",
        path.as_ref().unwrap_or(&"device root".to_string())
    );
    let mut files = get_mtp_files_recursive(object_by_path, path.clone())?;
    sort_files(&mut files);

    let total_files = files.len();
    let total_size = Size::from_bytes(files.iter().map(|file| file.size.bytes()).sum::<i64>());

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

    info!("Found {total_files} files, totalling {total_size} ({image_count} photos, {raw_count} RAW files, {video_count} videos)");

    Ok(files)
}
