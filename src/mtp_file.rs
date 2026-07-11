use color_eyre::eyre::{Result, eyre};
use std::fmt::{Display, Formatter};
use std::io::Read;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MtpFileType {
    Image,
    RawImage,
    Video,
}

impl MtpFileType {
    pub fn try_from_file_name(name: &str) -> Result<Self> {
        let extension = name
            .rsplit_once('.')
            .map(|(_, extension)| extension)
            .unwrap_or_default();
        match extension.to_ascii_lowercase().as_str() {
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
            Self::Video => 1,
            Self::RawImage => 2,
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

pub trait MtpFileSource {
    fn open(&self) -> Result<Box<dyn Read + '_>>;
}

pub struct MtpFile {
    pub name: String,
    pub path: String,
    pub file_type: MtpFileType,
    pub size: u64,
    source: Box<dyn MtpFileSource>,
}

impl MtpFile {
    pub fn new(
        name: String,
        path: String,
        file_type: MtpFileType,
        size: u64,
        source: Box<dyn MtpFileSource>,
    ) -> Self {
        Self {
            name,
            path,
            file_type,
            size,
            source,
        }
    }

    pub fn open(&self) -> Result<Box<dyn Read + '_>> {
        self.source.open()
    }
}

impl Display for MtpFile {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} ({:?}): {}",
            self.path,
            self.file_type,
            size::Size::from_bytes(self.size)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::MtpFileType;

    #[test]
    fn classifies_supported_files_case_insensitively() {
        assert_eq!(
            MtpFileType::try_from_file_name("DSCF0001.JPEG").unwrap(),
            MtpFileType::Image
        );
        assert_eq!(
            MtpFileType::try_from_file_name("DSCF0001.RAF").unwrap(),
            MtpFileType::RawImage
        );
        assert_eq!(
            MtpFileType::try_from_file_name("clip.MOV").unwrap(),
            MtpFileType::Video
        );
    }

    #[test]
    fn rejects_unknown_and_extensionless_files() {
        assert!(MtpFileType::try_from_file_name("README").is_err());
        assert!(MtpFileType::try_from_file_name("notes.txt").is_err());
    }
}
