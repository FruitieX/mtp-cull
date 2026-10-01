//! Keep the console subsystem for CLI output, detach only a UI-owned console.
#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetConsoleProcessList(processes: *mut u32, count: u32) -> u32;
    fn FreeConsole() -> i32;
    fn CreateMutexW(
        attributes: *const std::ffi::c_void,
        owner: i32,
        name: *const u16,
    ) -> *mut std::ffi::c_void;
    fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
}

/// A terminal started by Explorer belongs only to this process. A shell's
/// console has other attached processes and must retain its output and lifetime.
pub fn detach_owned_console() -> bool {
    #[cfg(windows)]
    {
        let mut processes = [0_u32; 2];
        // SAFETY: valid output buffer; FreeConsole affects only this process.
        unsafe {
            if GetConsoleProcessList(processes.as_mut_ptr(), processes.len() as u32) == 1 {
                return FreeConsole() != 0;
            }
        }
    }
    false
}

#[cfg(all(windows, feature = "ui-smoke"))]
pub fn console_process_count() -> u32 {
    let mut processes = [0_u32; 2];
    // SAFETY: valid output buffer; returns zero when no console is attached.
    unsafe { GetConsoleProcessList(processes.as_mut_ptr(), processes.len() as u32) }
}

/// Match the installer shortcuts so pinned taskbar entries use one identity.
pub fn set_app_id() {
    #[cfg(windows)]
    {
        #[link(name = "shell32")]
        unsafe extern "system" {
            fn SetCurrentProcessExplicitAppUserModelID(id: *const u16) -> i32;
        }
        let id: Vec<u16> = "FruitieX.mtp-cull".encode_utf16().chain([0]).collect();
        // SAFETY: valid NUL-terminated application ID; applies to this process.
        unsafe {
            SetCurrentProcessExplicitAppUserModelID(id.as_ptr());
        }
    }
}

/// Installer guard, not a single-instance restriction. Multiple processes remain
/// possible; the mutex lives until the last process exits normally.
pub struct AppLifetime {
    #[cfg(windows)]
    handle: *mut std::ffi::c_void,
}
impl AppLifetime {
    pub fn new() -> Self {
        #[cfg(windows)]
        {
            let name: Vec<u16> = "Local\\FruitieX.mtp-cull.running"
                .encode_utf16()
                .chain([0])
                .collect();
            // SAFETY: no custom attributes, NUL-terminated name, no ownership.
            let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
            Self { handle }
        }
        #[cfg(not(windows))]
        Self {}
    }
}
impl Drop for AppLifetime {
    fn drop(&mut self) {
        #[cfg(windows)]
        if !self.handle.is_null() {
            // SAFETY: this handle was returned by CreateMutexW and closes once.
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }
}

pub fn show_startup_error(error: &str) {
    #[cfg(windows)]
    {
        #[link(name = "user32")]
        unsafe extern "system" {
            fn MessageBoxW(
                window: *mut std::ffi::c_void,
                text: *const u16,
                caption: *const u16,
                flags: u32,
            ) -> i32;
        }
        let text: Vec<u16> = error.encode_utf16().chain([0]).collect();
        let caption: Vec<u16> = "mtp-cull could not start"
            .encode_utf16()
            .chain([0])
            .collect();
        // SAFETY: valid NUL-terminated strings, no parent window.
        unsafe {
            MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), 0x10);
        }
    }
    #[cfg(not(windows))]
    let _ = error;
}
