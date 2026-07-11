use crate::mtp_file::MtpFile;
use color_eyre::eyre::{Result, bail};

fn unsupported() -> Result<()> {
    bail!("MTP access is not implemented on this platform yet")
}

pub fn list_devices() -> Result<Vec<String>> {
    unsupported()?;
    unreachable!()
}

pub fn get_files_list(_device_name: Option<&str>, _path: Option<String>) -> Result<Vec<MtpFile>> {
    unsupported()?;
    unreachable!()
}
