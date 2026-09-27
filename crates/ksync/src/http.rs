//! Transport via the device's system `curl` (no TLS/HTTP crates in the
//! binary — the Kindle's curl handles TLS, redirects and timeouts). Resolves
//! `curl` on PATH first, falling back to `/usr/bin/curl`; a curl exit 60
//! (certificate verify failure) retries once with `-k` unless the catalog
//! already requested insecure mode — mirroring tailscale install.sh's TLS
//! fallback. Downloads go to `dest + ".part"` and are renamed into place, so a
//! failed transfer never leaves a partial file under its final name.

use crate::config::Catalog;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

const USER_AGENT: &str = "ksync/0.1";
const CONNECT_TIMEOUT: &str = "15";
const MAX_TIME: &str = "300";

/// Build the shared curl argument prefix for a catalog (auth, insecure flag).
fn base_args(catalog: &Catalog) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-f".into(),  // fail on HTTP errors
        "-sS".into(), // silent, but show errors on stderr
        "-L".into(),  // follow redirects
        // Catalog links are untrusted input. Keep curl from reading local
        // files or switching to another protocol after a redirect.
        "--proto".into(),
        "=http,https".into(),
        "--proto-redir".into(),
        "=http,https".into(),
        "--connect-timeout".into(),
        CONNECT_TIMEOUT.into(),
        "--max-time".into(),
        MAX_TIME.into(),
        "-A".into(),
        USER_AGENT.into(),
    ];
    if catalog.insecure {
        args.push("-k".into());
    }
    if let (Some(u), Some(p)) = (&catalog.username, &catalog.password) {
        args.push("-u".into());
        args.push(format!("{}:{}", u, p));
    }
    args
}

/// Run curl, retrying once with `-k` on exit 60 (cert verify failure) when the
/// catalog is not already marked insecure.
fn run_with_retry(
    url: &str,
    catalog: &Catalog,
    mut args: Vec<String>,
) -> Result<std::process::Output, String> {
    validate_http_url(url)?;
    // End option parsing before the URL as an additional guard for malformed
    // input that begins with a dash.
    args.push("--".into());
    args.push(url.to_string());
    let out = run_curl(&args)?;
    if out.status.code() == Some(60) && !catalog.insecure {
        let mut retry = args.clone();
        retry.insert(1, "-k".into());
        return run_curl(&retry);
    }
    Ok(out)
}

fn validate_http_url(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("invalid URL {}: {}", url, e))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host().is_none() {
        return Err(format!("only absolute HTTP(S) URLs are allowed: {}", url));
    }
    Ok(())
}

fn run_curl(args: &[String]) -> Result<std::process::Output, String> {
    // Resolve `curl`, then `/usr/bin/curl`: a spawn failure for the first
    // falls through to the absolute path.
    let mut last_err = None;
    for bin in ["curl", "/usr/bin/curl"] {
        match Command::new(bin).args(args).output() {
            Ok(out) => return Ok(out),
            Err(e) => last_err = Some(e),
        }
    }
    Err(format!(
        "no curl binary found: {}",
        last_err.map(|e| e.to_string()).unwrap_or_default()
    ))
}

/// Fetch a URL's body. `accept` sets the Accept header (used for OPDS feeds).
pub fn fetch(url: &str, catalog: &Catalog, accept: Option<&str>) -> Result<Vec<u8>, String> {
    let mut args = base_args(catalog);
    if let Some(a) = accept {
        args.push("-H".into());
        args.push(format!("Accept: {}", a));
    }
    let out = run_with_retry(url, catalog, args)?;
    if !out.status.success() {
        return Err(stderr_tail(&out.stderr, url));
    }
    Ok(out.stdout)
}

/// Download a URL to `dest` (via `dest.part` + rename). Creates parent dirs.
pub fn download(url: &str, catalog: &Catalog, dest: &Path) -> Result<(), String> {
    let parent = dest
        .parent()
        .ok_or_else(|| format!("no parent directory for {}", dest.display()))?;
    std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {}", parent.display(), e))?;
    let part = PathBuf::from(format!("{}.part", dest.display()));
    let mut args = base_args(catalog);
    args.push("-o".into());
    args.push(part.to_string_lossy().into_owned());
    let out = run_with_retry(url, catalog, args)?;
    if !out.status.success() {
        let _ = std::fs::remove_file(&part);
        return Err(stderr_tail(&out.stderr, url));
    }
    std::fs::rename(&part, dest).map_err(|e| {
        let _ = std::fs::remove_file(&part);
        format!("rename {} -> {}: {}", part.display(), dest.display(), e)
    })
}

fn last_bytes_on_char_boundary(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut start = s.len() - max;
    while start < s.len() && !s.is_char_boundary(start) {
        start += 1;
    }
    &s[start..]
}

fn stderr_tail(stderr: &[u8], url: &str) -> String {
    let s = String::from_utf8_lossy(stderr);
    let s = s.trim();
    format!(
        "curl failed for {}: {}",
        url,
        last_bytes_on_char_boundary(s, 300)
    )
}

#[cfg(test)]
mod tests {
    use super::{last_bytes_on_char_boundary, validate_http_url};

    #[test]
    fn short_string_unchanged() {
        assert_eq!(last_bytes_on_char_boundary("hello", 300), "hello");
    }

    #[test]
    fn does_not_panic_on_multibyte_boundary() {
        // 200 × 'é' (2 bytes each) = 400 bytes. Cutting 300 bytes from the
        // end lands inside a character; the helper must walk forward.
        let s: String = std::iter::repeat('é').take(200).collect();
        let tail = last_bytes_on_char_boundary(&s, 300);
        assert!(tail.is_char_boundary(0));
        assert!(tail.chars().all(|c| c == 'é'));
        assert!(tail.len() <= 300);
    }

    #[test]
    fn accepts_only_absolute_http_urls() {
        assert!(validate_http_url("https://catalog.example/opds").is_ok());
        assert!(validate_http_url("http://catalog.example/opds").is_ok());
        assert!(validate_http_url("file:///var/local/cc.db").is_err());
        assert!(validate_http_url("ftp://catalog.example/book").is_err());
        assert!(validate_http_url("/relative/feed").is_err());
    }
}
