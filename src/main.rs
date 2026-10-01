mod bursts;
mod cli;
mod commands;
// Retain regression coverage for the previous database and orientation pipeline.
#[cfg(test)]
#[allow(dead_code)]
mod culling;
mod focus;
mod image_cache;
mod import_presets;
mod imports;
mod mipmaps;
mod mtp_file;
mod mtp_worker;
#[cfg(test)]
mod performance;
mod quick_import;
mod recent_sources;
mod reel;
mod review;
mod review_commands;
mod review_store;
mod safe_copy;
mod theme;
mod transfer_progress;
#[path = "app.rs"]
mod ui;
mod viewer;
mod windows_app;

fn main() -> color_eyre::eyre::Result<()> {
    color_eyre::install()?;
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let cli = cli::parse_args();

    let command = cli
        .command
        .unwrap_or_else(|| cli::Commands::Ui(cli::UiArgs::default()));
    let _lifetime = windows_app::AppLifetime::new();
    match &command {
        cli::Commands::List => commands::list_devices()?,
        cli::Commands::ListContent(args) => commands::list_content(args)?,
        cli::Commands::Copy(args) => commands::copy_files(args)?,
        cli::Commands::Ui(args) => {
            let detached = windows_app::detach_owned_console();
            windows_app::set_app_id();
            if let Err(error) = ui::init(args) {
                if detached {
                    windows_app::show_startup_error(&format!("{error:#}"));
                }
                return Err(error);
            }
        }
    };

    Ok(())
}
