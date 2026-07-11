#[cfg(not(windows))]
mod unsupported;
#[cfg(windows)]
mod windows;

#[cfg(not(windows))]
pub use unsupported::{get_files_list, list_devices};
#[cfg(windows)]
pub use windows::{get_files_list, list_devices};
