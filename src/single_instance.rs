//! Keeps a second copy of the app from starting: two copies would both feed the mic into the
//! cable (doubling the voice) and fight over every shortcut.

/// Held for the life of the process; dropping it lets another instance start.
pub struct InstanceGuard {
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Claim the named instance lock; `None` if another process (or guard) already holds it.
#[cfg(windows)]
pub fn acquire(name: &str) -> Option<InstanceGuard> {
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError};
    use windows_sys::Win32::System::Threading::CreateMutexW;
    let name = wide(&format!("Local\\{name}"));
    // SAFETY: plain Win32 calls with a valid NUL-terminated name; the handle is closed on drop.
    unsafe {
        let handle = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        if handle.is_null() {
            return None;
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            CloseHandle(handle);
            return None;
        }
        Some(InstanceGuard { handle })
    }
}

#[cfg(not(windows))]
pub fn acquire(_name: &str) -> Option<InstanceGuard> {
    Some(InstanceGuard {})
}

/// Bring the already-running instance's window to the front (it may be hidden in the tray).
#[cfg(windows)]
pub fn show_existing(title: &str) {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, GetWindowRect, SW_RESTORE, SW_SHOW, SWP_NOSIZE, SWP_NOZORDER,
        SetForegroundWindow, SetWindowPos, ShowWindow,
    };
    let title = wide(title);
    // SAFETY: FindWindowW takes a NUL-terminated title; the other calls accept any window handle.
    unsafe {
        let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
        if !hwnd.is_null() {
            ShowWindow(hwnd, SW_SHOW);
            ShowWindow(hwnd, SW_RESTORE);
            // A hidden start parks the window off-screen; bring it somewhere visible.
            let mut r: RECT = std::mem::zeroed();
            if GetWindowRect(hwnd, &mut r) != 0 && (r.left < -10_000 || r.top < -10_000) {
                SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    160,
                    120,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER,
                );
            }
            SetForegroundWindow(hwnd);
        }
    }
}

#[cfg(not(windows))]
pub fn show_existing(_title: &str) {}

#[cfg(windows)]
impl Drop for InstanceGuard {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateMutexW and is closed exactly once.
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.handle) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_acquire_fails_until_released() {
        let name = format!("vSoundboard-test-{}", std::process::id());
        let first = acquire(&name).expect("first instance");
        assert!(acquire(&name).is_none(), "second instance must be refused");
        drop(first);
        assert!(acquire(&name).is_some(), "free again once the first exits");
    }
}
