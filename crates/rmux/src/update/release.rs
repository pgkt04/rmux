//! Release lookup and download. curl/wget do the transport and `tar` the
//! extraction; a TLS client is not worth it for a command run once a month.

use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use sha2::{Digest, Sha256};

const DEFAULT_BASE: &str = "https://github.com/pgkt04/rmux";

/// Release site root. `RMUX_UPDATE_BASE_URL` is a test hook pointing the
/// updater at a fake release server.
pub fn base_url() -> String {
    match std::env::var("RMUX_UPDATE_BASE_URL") {
        Ok(url) if !url.trim().is_empty() => url.trim().trim_end_matches('/').to_string(),
        _ => DEFAULT_BASE.to_string(),
    }
}

/// Tag out of the /releases/latest redirect target,
/// e.g. .../releases/tag/v1.2.0 -> v1.2.0
pub fn tag_from_redirect(url: &str) -> Option<String> {
    let (_, tag) = url.trim().rsplit_once("/tag/")?;
    let tag = tag.trim_matches('/');
    (!tag.is_empty() && !tag.contains('/') && !tag.contains(char::is_whitespace))
        .then(|| tag.to_string())
}

/// "v1.2.3" or "1.2.3" -> [1,2,3]; anything else None.
pub fn semver(s: &str) -> Option<[u64; 3]> {
    let s = s.trim();
    let mut it = s.strip_prefix('v').unwrap_or(s).splitn(3, '.');
    let mut out = [0u64; 3];
    for slot in &mut out {
        let part = it.next()?;
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        *slot = part.parse().ok()?;
    }
    Some(out)
}

/// "1.2.3" or "v1.2.3" -> "v1.2.3"; None for something that cannot be a tag.
pub fn normalize_tag(s: &str) -> Option<String> {
    let s = s.trim();
    let bare = s.strip_prefix('v').unwrap_or(s);
    if bare.is_empty() || bare.contains('/') || bare.contains(char::is_whitespace) {
        return None;
    }
    Some(format!("v{bare}"))
}

pub fn asset_name(tag: &str, target: &str) -> String {
    format!("rmux-{tag}-{target}.tar.gz")
}

pub fn on_path(bin: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        std::fs::metadata(dir.join(bin))
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    })
}

/// Resolve the /releases/latest redirect instead of the api, which rate
/// limits by ip; curl first, wget for the minimal distros without it.
pub fn latest_tag(base: &str) -> Result<String, String> {
    let url = format!("{base}/releases/latest");
    if on_path("curl") {
        let out = Command::new("curl")
            .args(["-fsSLI", "-o", "/dev/null", "-w", "%{url_effective}"])
            .args(["--max-time", "30"])
            .arg(&url)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("could not run curl: {e}"))?;
        if out.status.success()
            && let Some(tag) = tag_from_redirect(&String::from_utf8_lossy(&out.stdout))
        {
            return Ok(tag);
        }
    } else if on_path("wget") {
        // -S prints the headers on stderr; the last Location is the final URL
        let out = Command::new("wget")
            .args(["-q", "-S", "--spider", "--max-redirect=5", "--timeout=30"])
            .arg(&url)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("could not run wget: {e}"))?;
        let headers = String::from_utf8_lossy(&out.stderr);
        if let Some(tag) = headers
            .lines()
            .rev()
            .filter_map(|l| {
                let l = l.trim_start();
                l.get(..9)
                    .filter(|k| k.eq_ignore_ascii_case("location:"))
                    .map(|_| &l[9..])
            })
            .find_map(tag_from_redirect)
        {
            return Ok(tag);
        }
    } else {
        return Err("need curl or wget on PATH".into());
    }
    Err(format!("could not resolve the latest release from {url}"))
}

pub enum FetchError {
    /// The server answered but has no such file.
    Missing,
    Failed(String),
}

pub fn fetch(url: &str, dest: &Path) -> Result<(), FetchError> {
    let (tool, status, missing): (&str, _, &[i32]) = if on_path("curl") {
        let status = Command::new("curl")
            .args(["-fsSL", "--retry", "2", "--max-time", "300", "-o"])
            .arg(dest)
            .arg(url)
            .stdin(Stdio::null())
            .status();
        // 22: HTTP status >= 400 under -f; 37: file:// path does not exist
        ("curl", status, &[22, 37])
    } else if on_path("wget") {
        let status = Command::new("wget")
            .args(["-q", "-O"])
            .arg(dest)
            .arg(url)
            .stdin(Stdio::null())
            .status();
        // 8: the server issued an error response
        ("wget", status, &[8])
    } else {
        return Err(FetchError::Failed("need curl or wget on PATH".into()));
    };
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) if s.code().is_some_and(|c| missing.contains(&c)) => Err(FetchError::Missing),
        Ok(_) => Err(FetchError::Failed(format!("could not download {url}"))),
        Err(e) => Err(FetchError::Failed(format!("could not run {tool}: {e}"))),
    }
}

fn nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn decode_hex(hash: &str) -> Option<[u8; 32]> {
    let bytes = hash.as_bytes();
    if bytes.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (slot, pair) in out.iter_mut().zip(bytes.chunks_exact(2)) {
        *slot = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    Some(out)
}

pub fn hex(digest: &[u8; 32]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// The digest `sha256sum` printed for `asset`: `<hex>  <name>` (text mode)
/// or `<hex> *<name>` (binary mode).
pub fn parse_sha256sums(text: &str, asset: &str) -> Option<[u8; 32]> {
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        let Some((hash, rest)) = line.split_once(' ') else {
            continue;
        };
        let name = rest.strip_prefix(' ').or_else(|| rest.strip_prefix('*'));
        if name == Some(asset) {
            return decode_hex(hash);
        }
    }
    None
}

pub fn sha256_file(path: &Path) -> io::Result<[u8; 32]> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher)?;
    Ok(hasher.finalize().into())
}

/// Unpack `archive` into `dir` and return the `rmux` it must contain.
pub fn extract(archive: &Path, dir: &Path) -> Result<PathBuf, String> {
    let out = Command::new("tar")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(dir)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run tar: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let first = err.lines().next().unwrap_or("").trim();
        return Err(format!("could not unpack {}: {first}", archive.display()));
    }
    let bin = dir.join("rmux");
    match std::fs::symlink_metadata(&bin) {
        Ok(m) if m.is_file() => Ok(bin),
        _ => Err(format!(
            "{} does not contain an rmux binary",
            archive.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_parses_from_redirect_url() {
        assert_eq!(
            tag_from_redirect("https://github.com/pgkt04/rmux/releases/tag/v0.2.0"),
            Some("v0.2.0".into())
        );
        assert_eq!(
            tag_from_redirect(" https://github.com/x/y/releases/tag/v2.0.0\r\n"),
            Some("v2.0.0".into())
        );
        assert_eq!(
            tag_from_redirect("https://github.com/x/y/releases/tag/v2.0.0/"),
            Some("v2.0.0".into())
        );
        // no redirect happened (rate limit page, error html, plain latest url)
        assert_eq!(
            tag_from_redirect("https://github.com/pgkt04/rmux/releases/latest"),
            None
        );
        assert_eq!(tag_from_redirect(""), None);
        assert_eq!(tag_from_redirect("https://g.com/releases/tag/"), None);
        assert_eq!(
            tag_from_redirect("https://g.com/releases/tag/v1/extra"),
            None
        );
    }

    #[test]
    fn semver_parses_and_orders() {
        assert_eq!(semver("v1.2.3"), Some([1, 2, 3]));
        assert_eq!(semver("1.0.0"), Some([1, 0, 0]));
        assert_eq!(semver(" v1.10.0 "), Some([1, 10, 0]));
        assert_eq!(semver("nightly"), None);
        assert_eq!(semver("v1.2"), None);
        assert_eq!(semver("v1.2.3-rc1"), None);
        assert_eq!(semver("vv1.2.3"), None);
        assert_eq!(semver("v1.+2.3"), None);
        assert_eq!(semver("v1..3"), None);
        assert!(semver("v1.10.0") > semver("v1.9.9"));
        assert!(semver("v2.0.0") > semver("1.99.99"));
        assert_eq!(semver("v0.2.0"), semver("0.2.0"));
    }

    #[test]
    fn tags_normalize() {
        assert_eq!(normalize_tag("1.2.3").as_deref(), Some("v1.2.3"));
        assert_eq!(normalize_tag("v1.2.3").as_deref(), Some("v1.2.3"));
        assert_eq!(normalize_tag(" 0.2.0 ").as_deref(), Some("v0.2.0"));
        assert_eq!(normalize_tag(""), None);
        assert_eq!(normalize_tag("v"), None);
        assert_eq!(normalize_tag("../x"), None);
        assert_eq!(normalize_tag("1 2"), None);
    }

    #[test]
    fn asset_names_match_release_workflow() {
        assert_eq!(
            asset_name("v0.2.0", "x86_64-unknown-linux-gnu"),
            "rmux-v0.2.0-x86_64-unknown-linux-gnu.tar.gz"
        );
        assert_eq!(
            asset_name("v1.0.0", "aarch64-apple-darwin"),
            "rmux-v1.0.0-aarch64-apple-darwin.tar.gz"
        );
    }

    const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn sha256sums_lines_parse() {
        let asset = "rmux-v0.2.0-aarch64-apple-darwin.tar.gz";
        let other = "1111111111111111111111111111111111111111111111111111111111111111";
        let text =
            format!("{other}  rmux-v0.2.0-x86_64-unknown-linux-gnu.tar.gz\n{ABC}  {asset}\n");
        let want = decode_hex(ABC).unwrap();
        assert_eq!(parse_sha256sums(&text, asset), Some(want));
        assert_eq!(hex(&want), ABC);

        let binary = format!("{ABC} *{asset}\r\n");
        assert_eq!(parse_sha256sums(&binary, asset), Some(want));

        let upper = format!("{}  {asset}", ABC.to_ascii_uppercase());
        assert_eq!(parse_sha256sums(&upper, asset), Some(want));

        assert_eq!(parse_sha256sums(&text, "rmux-v0.2.0-other.tar.gz"), None);
        // a name that only shares a prefix is a different asset
        assert_eq!(
            parse_sha256sums(&format!("{ABC}  {asset}.sig"), asset),
            None
        );
        assert_eq!(parse_sha256sums(&format!("{ABC} {asset}"), asset), None);

        let bad = format!("{}zz  {asset}", &ABC[..62]);
        assert_eq!(parse_sha256sums(&bad, asset), None);
        let short = format!("{}  {asset}", &ABC[..62]);
        assert_eq!(parse_sha256sums(&short, asset), None);
        let long = format!("{ABC}00  {asset}");
        assert_eq!(parse_sha256sums(&long, asset), None);
        assert_eq!(parse_sha256sums("", asset), None);
    }

    #[test]
    fn sha256_of_file() {
        let path = std::env::temp_dir().join(format!("rmux-update-sha-{}", std::process::id()));
        std::fs::write(&path, b"abc").unwrap();
        let digest = sha256_file(&path);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(hex(&digest.unwrap()), ABC);
    }
}
