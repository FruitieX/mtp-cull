use winmtp::Provider;

pub fn list_devices() {
    let provider = Provider::new().unwrap();
    let devices = provider.enumerate_devices().unwrap();

    let count = devices.len();
    println!("Found {count} MTP devices:");

    for device in devices {
        println!("{}", device.friendly_name());
    }
}
