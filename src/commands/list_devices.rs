use crate::mtp_worker::MtpBackend;
use color_eyre::Result;

pub fn list_devices() -> Result<()> {
    let backend = MtpBackend::spawn()?;
    let devices = backend.list_devices()?;
    let count = devices.len();
    println!("Found {count} MTP devices:");

    for device in devices {
        println!("{}", device.name);
    }

    Ok(())
}
