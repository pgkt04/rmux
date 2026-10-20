//! `rmux update`: replace this binary (and any `--to` copies) with a
//! published release.

mod plugin;
mod release;
mod servers;
mod swap;

use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::version::{RELEASE, TARGET};

const USAGE: &str = "usage: rmux update [--check] [--version tag] [--to path]...";

#[derive(Debug, Default, PartialEq, Eq)]
struct Options {
    check: bool,
    version: Option<String>,
    to: Vec<PathBuf>,
}

#[derive(Debug, PartialEq, Eq)]
enum Parsed {
    Help,
    Run(Options),
}

fn parse(args: &[Vec<u8>]) -> Option<Parsed> {
    let mut opts = Options::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_slice() {
            b"-h" | b"--help" => return Some(Parsed::Help),
            b"--check" => opts.check = true,
            b"--version" => {
                let tag = std::str::from_utf8(it.next()?).ok()?;
                opts.version = Some(release::normalize_tag(tag)?);
            }
            b"--to" => {
                let path = it.next()?;
                if path.is_empty() {
                    return None;
                }
                opts.to
                    .push(PathBuf::from(OsString::from_vec(path.clone())));
            }
            _ => return None,
        }
    }
    Some(Parsed::Run(opts))
}

pub fn main(args: &[Vec<u8>]) -> i32 {
    let opts = match parse(args) {
        Some(Parsed::Help) => {
            println!("{USAGE}");
            return 0;
        }
        Some(Parsed::Run(opts)) => opts,
        None => {
            eprintln!("{USAGE}");
            return 1;
        }
    };
    match run(&opts) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("rmux update: {e}");
            1
        }
    }
}

/// Download directory beside the primary target, removed however we leave.
struct TempDir(PathBuf);

impl TempDir {
    fn create(path: PathBuf) -> Result<Self, String> {
        let _ = fs::remove_dir_all(&path);
        fs::create_dir(&path).map_err(|e| format!("cannot create {}: {e}", path.display()))?;
        Ok(Self(path))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn bare(tag: &str) -> &str {
    tag.strip_prefix('v').unwrap_or(tag)
}

fn is_newer(tag: &str, current: &str) -> bool {
    match (release::semver(tag), release::semver(current)) {
        (Some(t), Some(c)) => t > c,
        _ => bare(tag) != bare(current),
    }
}

fn targets(extra: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    let primary = std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .map_err(|e| format!("cannot locate the running binary: {e}"))?;
    let mut out = vec![primary];
    for path in extra {
        let real = path
            .canonicalize()
            .map_err(|e| format!("cannot find {}: {e}", path.display()))?;
        if !real.is_file() {
            return Err(format!("{} is not a file", path.display()));
        }
        if !out.contains(&real) {
            out.push(real);
        }
    }
    Ok(out)
}

fn parent(path: &Path) -> Result<&Path, String> {
    path.parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))
}

fn run(opts: &Options) -> Result<i32, String> {
    let base = release::base_url();
    let tag = match &opts.version {
        Some(tag) => tag.clone(),
        None => release::latest_tag(&base)?,
    };
    let version = bare(&tag);
    let newer = is_newer(&tag, RELEASE);
    if opts.check {
        if newer {
            println!("rmux {version} is available (installed {RELEASE})");
            return Ok(10);
        }
        println!("rmux {RELEASE} is up to date");
        return Ok(0);
    }
    if opts.version.is_none() && !newer {
        println!("rmux {RELEASE} is up to date");
        return Ok(0);
    }

    let targets = targets(&opts.to)?;
    let primary = &targets[0];
    if !swap::is_root() {
        for target in &targets {
            let dir = parent(target)?;
            if swap::writable(dir) {
                continue;
            }
            if !release::on_path("sudo") {
                return Err(format!("cannot write {}; rerun as root", dir.display()));
            }
            return swap::sudo_reexec(primary, &tag, &targets[1..]);
        }
    }

    let tmp =
        TempDir::create(parent(primary)?.join(format!(".rmux-update.{}", std::process::id())))?;
    let new = download(&base, &tag, &tmp.0)?;
    sanity_check(&new, version)?;

    let running = servers::running(primary);
    let note = plugin::note(&new);
    let date = swap::local_date()?;
    let mut done: Vec<&Path> = Vec::new();
    for target in &targets {
        match swap::install(&new, target, &date) {
            Ok(rollback) => {
                println!(
                    "installed rmux {version} to {} (rollback {})",
                    target.display(),
                    rollback.display()
                );
                done.push(target);
            }
            Err(e) if done.is_empty() => return Err(e),
            Err(e) => {
                let list: Vec<String> = done.iter().map(|p| p.display().to_string()).collect();
                return Err(format!("{e} (already updated: {})", list.join(", ")));
            }
        }
    }
    drop(tmp);

    for line in servers::report(&running) {
        println!("{line}");
    }
    if let Some(note) = note {
        println!("{note}");
    }
    Ok(0)
}

/// Fetch and verify the release archive for this target; returns the
/// unpacked binary inside `dir`.
fn download(base: &str, tag: &str, dir: &Path) -> Result<PathBuf, String> {
    let asset = release::asset_name(tag, TARGET);
    let archive = dir.join(&asset);
    let url = format!("{base}/releases/download/{tag}/{asset}");
    release::fetch(&url, &archive).map_err(|e| match e {
        release::FetchError::Missing => format!("no rmux build for {TARGET} at {tag}"),
        release::FetchError::Failed(msg) => msg,
    })?;

    let sums_path = dir.join("SHA256SUMS");
    let sums_url = format!("{base}/releases/download/{tag}/SHA256SUMS");
    release::fetch(&sums_url, &sums_path).map_err(|e| match e {
        release::FetchError::Missing => format!("could not download {sums_url}"),
        release::FetchError::Failed(msg) => msg,
    })?;
    let sums = fs::read(&sums_path).map_err(|e| format!("cannot read SHA256SUMS: {e}"))?;
    let expected = release::parse_sha256sums(&String::from_utf8_lossy(&sums), &asset)
        .ok_or_else(|| format!("{asset} is not listed in SHA256SUMS"))?;
    let got = release::sha256_file(&archive)
        .map_err(|e| format!("cannot read {}: {e}", archive.display()))?;
    if got != expected {
        return Err(format!(
            "checksum mismatch for {asset}: expected {}, got {}",
            release::hex(&expected),
            release::hex(&got)
        ));
    }
    release::extract(&archive, dir)
}

fn sanity_check(new: &Path, version: &str) -> Result<(), String> {
    let fail = |why: &str| format!("the downloaded rmux does not run here: {why}");
    let out = Command::new(new)
        .arg("version")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| fail(&e.to_string()))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    if out.status.success() && stdout.starts_with(&format!("rmux {version} ")) {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let why = [stderr.as_ref(), stdout.as_ref()]
        .into_iter()
        .find_map(|s| s.lines().map(str::trim).find(|l| !l.is_empty()))
        .map_or_else(|| out.status.to_string(), str::to_string);
    Err(fail(&why))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<Vec<u8>> {
        list.iter().map(|s| s.as_bytes().to_vec()).collect()
    }

    #[test]
    fn parses_options() {
        assert_eq!(parse(&args(&[])), Some(Parsed::Run(Options::default())));
        assert_eq!(parse(&args(&["-h"])), Some(Parsed::Help));
        assert_eq!(parse(&args(&["--check", "--help"])), Some(Parsed::Help));
        assert_eq!(
            parse(&args(&[
                "--check",
                "--version",
                "1.2.3",
                "--to",
                "/a",
                "--to",
                "/b"
            ])),
            Some(Parsed::Run(Options {
                check: true,
                version: Some("v1.2.3".into()),
                to: vec![PathBuf::from("/a"), PathBuf::from("/b")],
            }))
        );
        assert_eq!(
            parse(&args(&["--version", "v0.2.0"])),
            Some(Parsed::Run(Options {
                version: Some("v0.2.0".into()),
                ..Options::default()
            }))
        );
    }

    #[test]
    fn rejects_bad_arguments() {
        assert_eq!(parse(&args(&["--bogus"])), None);
        assert_eq!(parse(&args(&["check"])), None);
        assert_eq!(parse(&args(&["--version"])), None);
        assert_eq!(parse(&args(&["--version", ""])), None);
        assert_eq!(parse(&args(&["--version", "a/b"])), None);
        assert_eq!(parse(&[b"--version".to_vec(), vec![0xff]]), None);
        assert_eq!(parse(&args(&["--to"])), None);
        assert_eq!(parse(&args(&["--to", ""])), None);
        assert_eq!(parse(&args(&["--check", "-x"])), None);
    }

    #[test]
    fn newer_compares_semver_then_text() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("v0.10.0", "0.9.0"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(is_newer("nightly", "0.1.0"));
        assert!(!is_newer("vnightly", "nightly"));
    }

    #[test]
    fn temp_dir_is_removed_on_drop() {
        let path = std::env::temp_dir().join(format!("rmux-update-tmp-{}", std::process::id()));
        {
            let tmp = TempDir::create(path.clone()).unwrap();
            fs::write(tmp.0.join("asset"), b"x").unwrap();
            assert!(path.is_dir());
        }
        assert!(!path.exists());
    }

    #[test]
    fn sanity_check_reports_failures() {
        let missing = std::env::temp_dir().join("rmux-update-no-such-binary");
        let err = sanity_check(&missing, "0.1.0").unwrap_err();
        assert!(
            err.starts_with("the downloaded rmux does not run here: "),
            "{err}"
        );
        assert!(sanity_check(Path::new("/bin/echo"), "0.1.0").is_err());
    }
}
