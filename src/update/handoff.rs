//! Handing over to the downloaded installer. The app only quits once it can see the installer
//! is really running (its `SetupMutex` exists), so a blocked installer never leaves the friend
//! without vSoundboard.

use std::path::Path;
use std::time::Duration;

/// Must match `SetupMutex=` in installer/vsoundboard.iss.
pub const SETUP_MUTEX: &str = "vSoundboardSetup";
pub const INSTALLER_ARGS: [&str; 4] = [
    "/VERYSILENT",
    "/SUPPRESSMSGBOXES",
    "/NORESTART",
    "/UPDATE=1",
];

/// Start the installer detached (it outlives the app).
pub fn launch_installer(path: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    std::process::Command::new(path)
        .args(INSTALLER_ARGS)
        .creation_flags(DETACHED_PROCESS)
        .spawn()
        .map(drop)
        .map_err(|e| e.to_string())
}

/// Poll for the named mutex until it exists or `timeout` passes.
pub fn wait_for_setup_mutex(name: &str, timeout: Duration) -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::OpenMutexW;
    const SYNCHRONIZE: u32 = 0x0010_0000;
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let deadline = std::time::Instant::now() + timeout;
    loop {
        // SAFETY: valid NUL-terminated name; any handle returned is closed right away.
        let h = unsafe { OpenMutexW(SYNCHRONIZE, 0, wide.as_ptr()) };
        if !h.is_null() {
            unsafe { CloseHandle(h) };
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::CreateMutexW;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    #[test]
    fn wait_for_setup_mutex_finds_held_mutex() {
        let name = format!("vSoundboardSetup-test-{}", std::process::id());
        let w = wide(&name);
        // SAFETY: valid NUL-terminated name; handle closed below.
        let h = unsafe { CreateMutexW(std::ptr::null(), 0, w.as_ptr()) };
        assert!(!h.is_null());
        let t = Instant::now();
        assert!(wait_for_setup_mutex(&name, Duration::from_secs(2)));
        assert!(t.elapsed() < Duration::from_millis(200));
        unsafe { CloseHandle(h) };
    }

    #[test]
    fn wait_for_setup_mutex_times_out() {
        let name = format!("vSoundboardSetup-nobody-{}", std::process::id());
        let t = Instant::now();
        assert!(!wait_for_setup_mutex(&name, Duration::from_millis(300)));
        assert!(t.elapsed() >= Duration::from_millis(300));
    }
}
