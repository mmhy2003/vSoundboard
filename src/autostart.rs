//! "Start with Windows": a value under the current user's Run key. The installer writes the
//! same value, so both must agree on `command_for`.

use std::path::Path;

use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE};

pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
pub const VALUE_NAME: &str = "vSoundboard";

/// `"<exe>" --minimized`
pub fn command_for(exe: &Path) -> String {
    format!("\"{}\" --minimized", exe.display())
}

/// True when the value exists under `key` (relative to HKCU) and launches this `exe`.
pub fn is_enabled_in(key: &str, exe: &Path) -> bool {
    let value: Option<String> = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(key)
        .and_then(|k| k.get_value(VALUE_NAME))
        .ok();
    // Windows paths are case-insensitive; to_lowercase covers non-ASCII names too.
    value.is_some_and(|v| v.to_lowercase() == command_for(exe).to_lowercase())
}

/// Write or remove the value under `key` (relative to HKCU). Removing a missing value is fine.
pub fn set_enabled_in(key: &str, exe: &Path, on: bool) -> std::io::Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if on {
        let (k, _) = hkcu.create_subkey(key)?;
        k.set_value(VALUE_NAME, &command_for(exe))
    } else {
        match hkcu.open_subkey_with_flags(key, KEY_SET_VALUE) {
            Ok(k) => match k.delete_value(VALUE_NAME) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }
}

pub fn is_enabled(exe: &Path) -> bool {
    is_enabled_in(RUN_KEY, exe)
}

pub fn set_enabled(exe: &Path, on: bool) -> std::io::Result<()> {
    set_enabled_in(RUN_KEY, exe, on)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway HKCU key, deleted when dropped.
    struct TestKey(String);

    impl TestKey {
        fn new(name: &str) -> Self {
            Self(format!(
                r"Software\vSoundboard-test-{}-{name}",
                std::process::id()
            ))
        }
    }

    impl Drop for TestKey {
        fn drop(&mut self) {
            let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(&self.0);
        }
    }

    #[test]
    fn command_quotes_unicode_path_with_spaces() {
        let exe =
            Path::new(r"C:\Users\محمد علي\AppData\Local\Programs\vSoundboard\vsoundboard.exe");
        assert_eq!(
            command_for(exe),
            "\"C:\\Users\\محمد علي\\AppData\\Local\\Programs\\vSoundboard\\vsoundboard.exe\" --minimized"
        );
    }

    #[test]
    fn round_trip_on_off() {
        let k = TestKey::new("roundtrip");
        let exe = Path::new(r"C:\Users\محمد علي\vSoundboard\vsoundboard.exe");
        assert!(!is_enabled_in(&k.0, exe));
        set_enabled_in(&k.0, exe, true).unwrap();
        assert!(is_enabled_in(&k.0, exe));
        set_enabled_in(&k.0, exe, false).unwrap();
        assert!(!is_enabled_in(&k.0, exe));
        set_enabled_in(&k.0, exe, false).unwrap();
    }

    #[test]
    fn other_exe_is_not_enabled() {
        let k = TestKey::new("other");
        set_enabled_in(&k.0, Path::new(r"C:\a\vsoundboard.exe"), true).unwrap();
        assert!(!is_enabled_in(&k.0, Path::new(r"C:\b\vsoundboard.exe")));
        assert!(
            is_enabled_in(&k.0, Path::new(r"c:\A\VSOUNDBOARD.EXE")),
            "paths compare case-insensitively"
        );
    }
}
