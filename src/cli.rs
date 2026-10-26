/// Command-line switches.
#[derive(Debug, Default, PartialEq)]
pub struct Args {
    /// Start hidden in the tray (used by the Start with Windows entry).
    pub minimized: bool,
    /// Exit 0 if VB-Cable is installed, 1 if not; no window (used by the installer).
    pub check_vbcable: bool,
    /// Test switch for rehearsing updates: use this URL instead of GitHub's latest-release API.
    pub update_feed: Option<String>,
}

/// Parse `std::env::args()`-style input; the first item is the program name. Unknown args are ignored.
pub fn parse(args: impl IntoIterator<Item = String>) -> Args {
    let mut out = Args::default();
    for arg in args.into_iter().skip(1) {
        match arg.as_str() {
            "--minimized" => out.minimized = true,
            "--check-vbcable" => out.check_vbcable = true,
            other => {
                if let Some(url) = other.strip_prefix("--update-feed=") {
                    out.update_feed = Some(url.to_string());
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(v: &[&str]) -> Args {
        parse(v.iter().map(|s| s.to_string()))
    }

    #[test]
    fn no_args_is_default() {
        assert_eq!(p(&["vsoundboard.exe"]), Args::default());
    }

    #[test]
    fn flags_are_recognised() {
        assert_eq!(
            p(&["x", "--minimized", "--check-vbcable"]),
            Args {
                minimized: true,
                check_vbcable: true,
                update_feed: None
            }
        );
    }

    #[test]
    fn unknown_args_ignored() {
        assert_eq!(
            p(&["x", "--bogus", "--minimized"]),
            Args {
                minimized: true,
                check_vbcable: false,
                update_feed: None
            }
        );
    }

    #[test]
    fn cli_parses_update_feed() {
        let a = p(&["x", "--update-feed=http://127.0.0.1:8765/latest.json"]);
        assert_eq!(
            a.update_feed.as_deref(),
            Some("http://127.0.0.1:8765/latest.json")
        );
    }

    #[test]
    fn program_name_never_matches() {
        assert_eq!(p(&["--minimized"]), Args::default());
    }
}
