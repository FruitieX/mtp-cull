use color_eyre::eyre::Result;
use size::Size;
use std::fmt::{Display, Formatter};
use winmtp::object::Object;

#[derive(Debug, PartialEq)]
pub enum MtpFileType {
    Image,
    RawImage,
    Video,
}

impl MtpFileType {
    pub fn try_from_file_name(name: &str) -> Result<Self> {
        match name.to_lowercase().split('.').last().unwrap() {
            "jpg" | "jpeg" | "heic" | "heif" | "png" | "gif" | "bmp" | "tif" | "tiff" => {
                Ok(Self::Image)
            }
            "raw" | "dng" | "raf" | "crw" | "cr2" | "cr3" | "arw" | "srf" | "sr2" | "rw2"
            | "nef" | "nrw" => Ok(Self::RawImage),
            "mp4" | "mov" | "avi" | "mkv" | "wmv" | "flv" | "webm" | "m4v" => Ok(Self::Video),
            _ => Err(eyre!("Unknown file type")),
        }
    }

    /// Copies should be done in this order, so that the files that we are
    /// likely interested in first (JPEGs) are copied first.
    pub fn copy_order(&self) -> usize {
        match self {
            Self::Image => 0,
            Self::RawImage => 1,
            Self::Video => 2,
        }
    }

    pub fn out_path_segment(&self) -> &str {
        match self {
            Self::Image => "Out-of-camera",
            Self::RawImage => "Undeveloped",
            Self::Video => "Video",
        }
    }
}

impl Display for MtpFileType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Image => write!(f, "Image"),
            Self::RawImage => write!(f, "Raw image"),
            Self::Video => write!(f, "Video"),
        }
    }
}

impl PartialOrd for MtpFileType {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.copy_order().partial_cmp(&other.copy_order())
    }
}

pub struct MtpFile {
    pub name: String,
    pub path: String,
    pub file_type: MtpFileType,
    pub size: Size,
    pub object: Object,
}

impl Display for MtpFile {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({:?}): {}", self.path, self.file_type, self.size)
    }
}
