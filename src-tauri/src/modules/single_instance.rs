#[cfg(target_os = "windows")]
use std::ffi::OsStr;
#[cfg(target_os = "windows")]
use std::os::windows::ffi::OsStrExt;

#[cfg(target_os = "windows")]
use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE},
    System::Threading::CreateMutexW,
    UI::WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION, MB_OK, MB_SETFOREGROUND},
};

pub struct SingleInstanceGuard {
    #[cfg(target_os = "windows")]
    handle: HANDLE,
}

impl SingleInstanceGuard {
    #[cfg(target_os = "windows")]
    fn new(handle: HANDLE) -> Self {
        Self { handle }
    }

    #[cfg(not(target_os = "windows"))]
    fn new() -> Self {
        Self {}
    }
}

#[cfg(target_os = "windows")]
unsafe impl Send for SingleInstanceGuard {}

#[cfg(target_os = "windows")]
unsafe impl Sync for SingleInstanceGuard {}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        #[cfg(target_os = "windows")]
        unsafe {
            if !self.handle.is_null() {
                CloseHandle(self.handle);
            }
        }
    }
}

pub fn acquire_single_instance_guard() -> Result<Option<SingleInstanceGuard>, String> {
    #[cfg(target_os = "windows")]
    {
        let mutex_name = to_wide("APIManager.SingleInstance.com.allapihub.apimanager");
        let handle = unsafe { CreateMutexW(std::ptr::null(), 1, mutex_name.as_ptr()) };
        if handle.is_null() {
            return Err(format!(
                "Failed to create single-instance mutex: {}",
                unsafe { GetLastError() }
            ));
        }

        let already_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        if already_exists {
            show_already_running_message();
            unsafe {
                CloseHandle(handle);
            }
            return Ok(None);
        }

        return Ok(Some(SingleInstanceGuard::new(handle)));
    }

    #[cfg(not(target_os = "windows"))]
    {
        Ok(Some(SingleInstanceGuard::new()))
    }
}

#[cfg(target_os = "windows")]
fn show_already_running_message() {
    let title = to_wide("APIManager");
    let message = to_wide("APIManager 已经在运行中，请勿重复打开。");
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONINFORMATION | MB_SETFOREGROUND,
        );
    }
}

#[cfg(target_os = "windows")]
fn to_wide(value: &str) -> Vec<u16> {
    OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}
