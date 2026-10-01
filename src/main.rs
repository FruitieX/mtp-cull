mod bursts;
mod cli;
mod commands;
// Retain regression coverage for the previous database and orientation pipeline.
#[cfg(test)]
#[allow(dead_code)]
mod culling;
mod focus;
mod image_cache;
mod imports;
mod mtp_file;
mod mtp_worker;
#[cfg(test)]
mod performance;
mod review;
mod review_commands;
mod review_store;
mod safe_copy;
mod transfer_progress;
#[path = "app.rs"]
mod ui;
mod viewer;

fn main() -> color_eyre::eyre::Result<()> {
    color_eyre::install()?;
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let cli = cli::parse_args();

    match &cli.command {
        cli::Commands::List => commands::list_devices()?,
        cli::Commands::ListContent(args) => commands::list_content(args)?,
        cli::Commands::Copy(args) => commands::copy_files(args)?,
        cli::Commands::Ui(args) => ui::init(args)?,
    };

    Ok(())
}
