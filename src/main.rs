#[macro_use]
extern crate eyre;

#[macro_use]
extern crate log;

extern crate pretty_env_logger;

mod commands;
mod cli;
mod mtp;
mod mtp_file;

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
    };

    Ok(())
}
