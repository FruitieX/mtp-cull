use super::{MtpEvent, MtpRequest};
use color_eyre::eyre::{Result, bail};

/// Direct MTP uses the Windows Portable Devices COM API and is unavailable here.
pub struct MtpWorker;

impl MtpWorker {
    pub fn spawn() -> Result<Self> {
        bail!("the direct MTP worker is only available on Windows")
    }

    pub fn send(&self, _request: MtpRequest) -> Result<()> {
        bail!("the direct MTP worker is only available on Windows")
    }

    pub fn try_recv(&self) -> Option<MtpEvent> {
        None
    }
}
