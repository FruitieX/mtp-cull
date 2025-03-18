#[macro_use]
extern crate eyre;

#[macro_use]
extern crate log;

extern crate native_windows_derive as nwd;
extern crate native_windows_gui as nwg;

mod cli;
mod commands;
mod mtp;
mod mtp_file;
mod ui;

fn main() -> color_eyre::eyre::Result<()> {
    color_eyre::install()?;
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info")
    }
    pretty_env_logger::init();

    let cli = cli::parse_args();

    match &cli.command {
        cli::Commands::List => commands::list_devices(),
        cli::Commands::ListContent(args) => commands::list_content(args)?,
        cli::Commands::Copy(args) => commands::copy_files(args)?,
        cli::Commands::Ui => ui::init()?,
    };

    Ok(())
}
