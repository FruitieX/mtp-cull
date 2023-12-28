use crate::{cli::ShowContentArgs, mtp::get_files_list};
use color_eyre::Result;

pub fn list_content(args: &ShowContentArgs) -> Result<()> {
    let files = get_files_list(args.device.as_deref(), args.path.clone())?;

    for file in files {
        println!("{file}");
    }

    Ok(())
}
