//! HTTPS for the updater: fetch the latest-release JSON, and download + verify the installer.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use sha2::{Digest, Sha256};

use super::Update;

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_per_read(Some(Duration::from_secs(30)))
        .build()
        .into()
}

fn user_agent() -> String {
    format!("vSoundboard/{}", env!("CARGO_PKG_VERSION"))
}

fn describe(e: ureq::Error) -> String {
    match e {
        ureq::Error::StatusCode(code) => format!("HTTP {code}"),
        other => other.to_string(),
    }
}

/// The body of the latest-release API response.
pub fn fetch_latest(api_url: &str) -> Result<String, String> {
    agent()
        .get(api_url)
        .header("User-Agent", &user_agent())
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(describe)?
        .body_mut()
        .read_to_string()
        .map_err(describe)
}

/// Stream the installer to `dest`, counting bytes in `done`, then check its size and SHA-256.
/// Any failure (including cancel) deletes `dest`.
pub fn download(
    u: &Update,
    dest: &Path,
    done: &AtomicU64,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let result = download_inner(u, dest, done, cancel);
    if result.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    result
}

fn download_inner(
    u: &Update,
    dest: &Path,
    done: &AtomicU64,
    cancel: &AtomicBool,
) -> Result<(), String> {
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let resp = agent()
        .get(&u.url)
        .header("User-Agent", &user_agent())
        .call()
        .map_err(describe)?;
    let mut reader = resp.into_body().into_reader();
    let mut file = std::fs::File::create(dest).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    done.store(0, Ordering::Relaxed);
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".to_string());
        }
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > u.size {
            return Err("verification failed: the file is larger than expected".to_string());
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done.store(total, Ordering::Relaxed);
    }
    file.flush().map_err(|e| e.to_string())?;
    drop(file);
    if total != u.size {
        return Err(format!(
            "verification failed: got {total} bytes, expected {}",
            u.size
        ));
    }
    let hex: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if hex != u.sha256 {
        return Err("verification failed: the file's checksum doesn't match GitHub's".to_string());
    }
    Ok(())
}

/// A tiny HTTP server for tests: answers each request in order with (status, body, delay per KB).
#[cfg(test)]
pub(crate) mod test_server {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::Duration;

    pub struct Reply {
        pub status: u16,
        pub body: Vec<u8>,
        /// Claimed Content-Length (defaults to the body length).
        pub length: Option<usize>,
        pub delay_per_kb: Duration,
    }

    impl Reply {
        pub fn ok(body: impl Into<Vec<u8>>) -> Self {
            Self {
                status: 200,
                body: body.into(),
                length: None,
                delay_per_kb: Duration::ZERO,
            }
        }
    }

    /// Returns the base URL, e.g. "http://127.0.0.1:12345".
    pub fn serve(replies: Vec<Reply>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for reply in replies {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let len = reply.length.unwrap_or(reply.body.len());
                let head = format!(
                    "HTTP/1.1 {} X\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n",
                    reply.status
                );
                let _ = stream.write_all(head.as_bytes());
                for chunk in reply.body.chunks(1024) {
                    if stream.write_all(chunk).is_err() {
                        break;
                    }
                    if !reply.delay_per_kb.is_zero() {
                        std::thread::sleep(reply.delay_per_kb);
                    }
                }
            }
        });
        format!("http://{addr}")
    }
}

#[cfg(test)]
mod tests {
    use super::test_server::{Reply, serve};
    use super::*;
    use std::time::Duration;

    fn update_for(url: String, body: &[u8]) -> Update {
        Update {
            version: semver::Version::new(9, 9, 9),
            url,
            size: body.len() as u64,
            sha256: Sha256::digest(body)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
            page: String::new(),
        }
    }

    fn body(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i % 251) as u8).collect()
    }

    #[test]
    #[ignore = "talks to api.github.com"]
    fn real_github_latest_is_an_update_from_0_1_0() {
        let body = fetch_latest(crate::update::API_URL).unwrap();
        let u = crate::update::decide(&body, &semver::Version::new(0, 1, 0))
            .unwrap()
            .unwrap();
        assert!(
            u.url
                .starts_with("https://github.com/mmhy2003/vSoundboard/releases/download/")
        );
    }

    #[test]
    fn fetch_returns_body_and_maps_status() {
        let base = serve(vec![
            Reply::ok("{\"ok\":1}"),
            Reply {
                status: 500,
                ..Reply::ok("")
            },
        ]);
        assert_eq!(fetch_latest(&format!("{base}/a")).unwrap(), "{\"ok\":1}");
        assert_eq!(
            fetch_latest(&format!("{base}/b")),
            Err("HTTP 500".to_string())
        );
    }

    #[test]
    fn download_verifies_and_reports_progress() {
        let data = body(100_000);
        let base = serve(vec![Reply::ok(data.clone())]);
        let d = tempfile::tempdir().unwrap();
        let dest = d.path().join("setup.exe");
        let done = AtomicU64::new(0);
        download(
            &update_for(format!("{base}/s"), &data),
            &dest,
            &done,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        assert_eq!(done.load(Ordering::Relaxed), 100_000);
    }

    #[test]
    fn download_rejects_wrong_hash() {
        let data = body(10_000);
        let base = serve(vec![Reply::ok(data.clone())]);
        let d = tempfile::tempdir().unwrap();
        let dest = d.path().join("setup.exe");
        let mut u = update_for(format!("{base}/s"), &data);
        u.sha256 = "0".repeat(64);
        let err = download(&u, &dest, &AtomicU64::new(0), &AtomicBool::new(false)).unwrap_err();
        assert!(err.contains("verification"), "{err}");
        assert!(!dest.exists());
    }

    #[test]
    fn download_rejects_short_file() {
        let data = body(100_000);
        let base = serve(vec![Reply {
            length: Some(50_000),
            ..Reply::ok(data[..50_000].to_vec())
        }]);
        let d = tempfile::tempdir().unwrap();
        let dest = d.path().join("setup.exe");
        let u = update_for(format!("{base}/s"), &data);
        assert!(download(&u, &dest, &AtomicU64::new(0), &AtomicBool::new(false)).is_err());
        assert!(!dest.exists());
    }

    #[test]
    fn cancel_stops_download() {
        let data = body(200_000);
        let base = serve(vec![Reply {
            delay_per_kb: Duration::from_millis(10),
            ..Reply::ok(data.clone())
        }]);
        let d = tempfile::tempdir().unwrap();
        let dest = d.path().join("setup.exe");
        let cancel = AtomicBool::new(false);
        let u = update_for(format!("{base}/s"), &data);
        let res = std::thread::scope(|s| {
            s.spawn(|| {
                std::thread::sleep(Duration::from_millis(50));
                cancel.store(true, Ordering::Relaxed);
            });
            download(&u, &dest, &AtomicU64::new(0), &cancel)
        });
        assert_eq!(res, Err("Cancelled".to_string()));
        assert!(!dest.exists());
    }
}
