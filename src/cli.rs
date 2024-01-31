use chrono::NaiveDate;
use clap::{Args, Parser, Subcommand};

/// Simple program to greet a person
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Lists all devices
    List,

    /// Shows the content of a device
    ListContent(ShowContentArgs),

    #[clap(verbatim_doc_comment)]
    /// Copy files from a device into a directory structure like:
    /// <TARGET_PATH>/{file_type}/{year}/<DATE> <ALBUM_NAME>/{file_name}
    ///
    /// Where:
    /// - <TARGET_PATH>, <DATE> and <ALBUM_NAME> are arguments.
    /// - {file_type} is either "Out-of-camera", "Undeveloped" or "Video" depending on filename extension.
    /// - {year} is computed from the <DATE> argument.
    /// - {file_name} is the original filename.
    Copy(CopyArgs),
}

#[derive(Args, Debug)]
pub struct ShowContentArgs {
    /// The device to show the content of, defaults to first device
    #[clap(long, short)]
    pub device: Option<String>,

    /// File path to show the content of, defaults to device root
    #[clap(long, short)]
    pub path: Option<String>,
}

#[derive(Args, Debug)]
pub struct CopyArgs {
    /// The MTP device name to copy from, defaults to first device.
    ///
    /// Example: "Samsung Galaxy S10"
    #[clap(long, short)]
    pub device: Option<String>,

    /// File path that will be copied from recursively, ignoring subdirectory structure.
    /// You can find this path from explorer.exe, look at the address bar after the device name.
    ///
    /// Defaults to device root.
    ///
    /// Example: /DCIM/Camera
    #[clap(long, short, verbatim_doc_comment)]
    pub source_path: Option<String>,

    /// Date to use for the album directory, defaults to today.
    ///
    /// Example: 2023-12-28
    #[clap(long)]
    pub date: Option<NaiveDate>,

    /// Prefix for path that will be copied to.
    ///
    /// Example: /home/user/Pictures
    #[clap(long, short)]
    pub target_path: String,

    /// "Album name" i.e. directory name that will be created in the target paths.
    /// Automatically prepended by the date. Defaults to just the date.
    ///
    /// Example: "New Years Eve"
    pub album_name: Option<String>,
}

pub fn parse_args() -> Cli {
    Cli::parse()
}
