//! Checking GitHub Releases for a newer vSoundboard. This module is the pure decision logic;
//! the network, worker thread and installer handoff live in the submodules.

use std::path::{Path, PathBuf};

use semver::Version;

pub const API_URL: &str = "https://api.github.com/repos/mmhy2003/vSoundboard/releases/latest";
pub const RELEASES_PAGE: &str = "https://github.com/mmhy2003/vSoundboard/releases";
/// "Later" hides the dialog for a day.
pub const SNOOZE_SECS: u64 = 86_400;
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{16385C1B-9E26-44CD-BD5A-3022320628A0}_is1";

/// A newer release we can install.
#[derive(Clone, Debug, PartialEq)]
pub struct Update {
    pub version: Version,
    pub url: String,
    pub size: u64,
    /// Lowercase hex.
    pub sha256: String,
    /// The release's page on GitHub ("What's new").
    pub page: String,
}

/// Read a GitHub "latest release" response and decide whether it's an update for `current`.
/// A release without our installer asset or a SHA-256 digest is not an update.
pub fn decide(release_json: &str, current: &Version) -> Result<Option<Update>, String> {
    let json: serde_json::Value =
        serde_json::from_str(release_json).map_err(|e| format!("unexpected response: {e}"))?;
    let tag = json["tag_name"].as_str().unwrap_or_default();
    let Ok(version) = Version::parse(tag.trim_start_matches('v')) else {
        return Ok(None);
    };
    if version <= *current {
        return Ok(None);
    }
    let wanted = format!("vSoundboard-Setup-{version}.exe");
    let Some(asset) = json["assets"].as_array().and_then(|a| {
        a.iter()
            .find(|x| x["name"].as_str() == Some(wanted.as_str()))
    }) else {
        return Ok(None);
    };
    let sha256 = asset["digest"]
        .as_str()
        .and_then(|d| d.strip_prefix("sha256:"));
    let (Some(sha256), Some(url), Some(size)) = (
        sha256.filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit())),
        asset["browser_download_url"].as_str(),
        asset["size"].as_u64(),
    ) else {
        return Ok(None);
    };
    Ok(Some(Update {
        version,
        url: url.to_string(),
        size,
        sha256: sha256.to_ascii_lowercase(),
        page: json["html_url"]
            .as_str()
            .unwrap_or(RELEASES_PAGE)
            .to_string(),
    }))
}

/// Whether the update dialog may be shown now. A snooze more than a day ahead (clock moved
/// back) counts as expired; a manual check ignores the snooze.
pub fn should_prompt(now: u64, snoozed_until: Option<u64>, manual: bool) -> bool {
    match snoozed_until {
        _ if manual => true,
        None => true,
        Some(until) => now >= until || until > now + SNOOZE_SECS,
    }
}

#[derive(Debug, PartialEq)]
pub enum StartupNotice {
    None,
    Updated(String),
    Failed(String),
}

/// What to tell the friend at startup, from the version running now, the version an update
/// was attempted for, and the version that last ran.
pub fn startup_notice(
    running: &Version,
    attempted: Option<&str>,
    last_run: Option<&str>,
) -> StartupNotice {
    if let Some(target) = attempted.and_then(|a| Version::parse(a).ok())
        && *running < target
    {
        return StartupNotice::Failed(target.to_string());
    }
    let previous = last_run.and_then(|l| Version::parse(l).ok());
    if attempted.is_some() || previous.is_some_and(|p| p < *running) {
        StartupNotice::Updated(running.to_string())
    } else {
        StartupNotice::None
    }
}

/// Whether `exe` is the installed copy (dev builds never update themselves).
pub fn is_installed_copy(exe: &Path, install_location: Option<&Path>) -> bool {
    let norm = |p: &Path| {
        p.to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .to_lowercase()
    };
    match (exe.parent(), install_location) {
        (Some(dir), Some(loc)) => norm(dir) == norm(loc),
        _ => false,
    }
}

/// The installer's install folder, from its uninstall entry.
pub fn install_location() -> Option<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(UNINSTALL_KEY)
        .ok()?;
    let loc: String = key.get_value("InstallLocation").ok()?;
    Some(PathBuf::from(loc))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../tests/fixtures/release-v0.2.0.json");

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn fixture_is_newer_than_0_1_0() {
        let u = decide(FIXTURE, &v("0.1.0")).unwrap().expect("update");
        assert_eq!(u.version, v("0.2.0"));
        assert_eq!(u.size, 5_947_717);
        assert_eq!(
            u.sha256,
            "aa2953fe10a03c28b59212132e54e3ee319e6e2714ae1314d44444f9a00d90b2"
        );
        assert!(u.url.ends_with("/vSoundboard-Setup-0.2.0.exe"), "{}", u.url);
        assert_eq!(
            u.page,
            "https://github.com/mmhy2003/vSoundboard/releases/tag/v0.2.0"
        );
    }

    #[test]
    fn same_or_older_is_none() {
        assert_eq!(decide(FIXTURE, &v("0.2.0")).unwrap(), None);
        assert_eq!(decide(FIXTURE, &v("0.3.0")).unwrap(), None);
    }

    #[test]
    fn missing_asset_or_digest_is_none() {
        let renamed = FIXTURE.replace("\"vSoundboard-Setup-0.2.0.exe\"", "\"other.exe\"");
        assert_eq!(decide(&renamed, &v("0.1.0")).unwrap(), None);
        let md5 = FIXTURE.replace("\"sha256:", "\"md5:");
        assert_eq!(decide(&md5, &v("0.1.0")).unwrap(), None);
        let mut j: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
        j["assets"][0].as_object_mut().unwrap().remove("digest");
        assert_eq!(decide(&j.to_string(), &v("0.1.0")).unwrap(), None);
    }

    #[test]
    fn bad_tag_or_json_is_err_or_none() {
        let latest = FIXTURE.replace("\"tag_name\":\"v0.2.0\"", "\"tag_name\":\"latest\"");
        assert_ne!(latest, FIXTURE, "fixture uses compact JSON");
        assert_eq!(decide(&latest, &v("0.1.0")).unwrap(), None);
        assert!(decide("not json", &v("0.1.0")).is_err());
    }

    #[test]
    fn snooze_rules() {
        assert!(should_prompt(100, None, false));
        assert!(!should_prompt(100, Some(200), false));
        assert!(should_prompt(200, Some(200), false));
        assert!(should_prompt(100, Some(200), true));
    }

    #[test]
    fn snooze_far_in_future_is_capped() {
        assert!(should_prompt(1000, Some(1000 + 3 * SNOOZE_SECS), false));
    }

    #[test]
    fn startup_notices() {
        assert_eq!(
            startup_notice(&v("0.3.0"), Some("0.3.0"), Some("0.2.0")),
            StartupNotice::Updated("0.3.0".into())
        );
        assert_eq!(
            startup_notice(&v("0.2.0"), Some("0.3.0"), Some("0.2.0")),
            StartupNotice::Failed("0.3.0".into())
        );
        assert_eq!(
            startup_notice(&v("0.3.0"), None, Some("0.2.0")),
            StartupNotice::Updated("0.3.0".into())
        );
        assert_eq!(
            startup_notice(&v("0.3.0"), None, Some("0.3.0")),
            StartupNotice::None
        );
        assert_eq!(startup_notice(&v("0.3.0"), None, None), StartupNotice::None);
    }

    #[test]
    fn installed_copy_rule() {
        let installed = Path::new(r"C:\Users\x\AppData\Local\Programs\vSoundboard\vsoundboard.exe");
        let loc = Path::new(r"c:\users\X\appdata\local\programs\vsoundboard\");
        assert!(is_installed_copy(installed, Some(loc)));
        let dev = Path::new(r"C:\Projects\vSoundboard\target\release\vsoundboard.exe");
        assert!(!is_installed_copy(dev, Some(loc)));
        assert!(!is_installed_copy(installed, None));
    }
}
