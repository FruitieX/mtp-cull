use color_eyre::Result;

pub fn list_devices() -> Result<()> {
    let devices = crate::mtp::list_devices()?;
    let count = devices.len();
    println!("Found {count} MTP devices:");

    for device in devices {
        println!("{device}");
    }

    Ok(())
}
