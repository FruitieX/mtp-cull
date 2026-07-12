use super::{MtpEvent, MtpRequest};
use color_eyre::eyre::{Result, bail};

/// Direct MTP is currently implemented only on Windows and Linux.
pub struct MtpWorker;

impl MtpWorker {
    pub fn spawn() -> Result<Self> {
        bail!("the direct MTP worker is only available on Windows and Linux")
    }

    pub fn send(&self, _request: MtpRequest) -> Result<()> {
        bail!("the direct MTP worker is only available on Windows and Linux")
    }

    pub fn try_recv(&self) -> Option<MtpEvent> {
        None
    }
}
