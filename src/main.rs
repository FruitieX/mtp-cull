mod cli;
mod commands;
mod culling;
mod mtp;
mod mtp_file;
mod ui;

fn main() -> color_eyre::eyre::Result<()> {
    color_eyre::install()?;
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let cli = cli::parse_args();

    match &cli.command {
        cli::Commands::List => commands::list_devices()?,
        cli::Commands::ListContent(args) => commands::list_content(args)?,
        cli::Commands::Copy(args) => commands::copy_files(args)?,
        cli::Commands::Ui => ui::init()?,
    };

    Ok(())
}
