use crate::{cli::ShowContentArgs, mtp_worker::MtpBackend};
use color_eyre::Result;

pub fn list_content(args: &ShowContentArgs) -> Result<()> {
    let backend = MtpBackend::spawn()?;
    let device = backend.select_device(args.device.as_deref())?;
    let files = backend.list_files(device.id, args.path.clone())?;

    for file in files {
        println!("{file}");
    }

    Ok(())
}
