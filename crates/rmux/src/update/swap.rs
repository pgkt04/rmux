//! Swapping the new binary into place. A running rmux keeps executing its
//! old inode, so the new file is staged next to the target and renamed over
//! it; the target itself is never opened for writing.

use std::fs;
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::version::RELEASE;

pub fn is_root() -> bool {
    rmux_sys::proc::getuid().0 == 0
}

/// Probe by creating and removing a file: `access(W_OK)` misses read-only
/// mounts and ACLs.
pub fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".rmux-update-probe.{}", std::process::id()));
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(_) => {
            let _ = fs::remove_file(&probe);
            true
        }
        Err(e) => e.kind() == io::ErrorKind::AlreadyExists,
    }
}

/// Run `sudo -- <exe> update --version <tag> [--to <path>]...` and return
/// its exit status.
pub fn sudo_reexec(exe: &Path, tag: &str, extra: &[PathBuf]) -> Result<i32, String> {
    let mut cmd = Command::new("sudo");
    cmd.arg("--").arg(exe).args(["update", "--version", tag]);
    for path in extra {
        cmd.arg("--to").arg(path);
    }
    let status = cmd
        .status()
        .map_err(|e| format!("could not run sudo: {e}"))?;
    Ok(status.code().unwrap_or(1))
}

/// Today's local date as YYYYMMDD.
pub fn local_date() -> Result<String, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "the system clock is before 1970".to_string())?;
    let seconds = i64::try_from(now.as_secs()).map_err(|_| "the system clock is out of range")?;
    let tm = rmux_sys::time::localtime(seconds).ok_or("cannot read the local date")?;
    Ok(format!(
        "{:04}{:02}{:02}",
        tm.year() + 1900,
        tm.mon() + 1,
        tm.mday()
    ))
}

pub fn rollback_name(date: &str, release: &str) -> String {
    format!("rmux.rollback-{date}-pre-v{release}")
}

pub fn staging_name(pid: u32) -> String {
    format!(".rmux.new.{pid}")
}

/// Install `new` over `target`, keeping a copy of the old binary next to it.
/// Returns the rollback path.
pub fn install(new: &Path, target: &Path, date: &str) -> Result<PathBuf, String> {
    install_with(new, target, date, std::process::id(), &|from, to| {
        fs::rename(from, to)
    })
}

type Rename = dyn Fn(&Path, &Path) -> io::Result<()>;

fn install_with(
    new: &Path,
    target: &Path,
    date: &str,
    pid: u32,
    rename: &Rename,
) -> Result<PathBuf, String> {
    let dir = target
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", target.display()))?;
    let staged = dir.join(staging_name(pid));
    let result = swap(new, target, dir, &staged, date, rename);
    if result.is_err() {
        let _ = fs::remove_file(&staged);
    }
    result
}

fn swap(
    new: &Path,
    target: &Path,
    dir: &Path,
    staged: &Path,
    date: &str,
    rename: &Rename,
) -> Result<PathBuf, String> {
    let meta =
        fs::metadata(target).map_err(|e| format!("cannot read {}: {e}", target.display()))?;
    let rollback = dir.join(rollback_name(date, RELEASE));

    // staged then renamed so an existing read-only rollback is replaced too
    stage(target, staged, &meta)?;
    rename(staged, &rollback).map_err(|e| format!("cannot write {}: {e}", rollback.display()))?;

    stage(new, staged, &meta)?;
    rename(staged, target).map_err(|e| format!("cannot replace {}: {e}", target.display()))?;
    Ok(rollback)
}

/// Copy `src` to `staged` with `like`'s mode and, as root, its owner.
fn stage(src: &Path, staged: &Path, like: &fs::Metadata) -> Result<(), String> {
    let err = |e: io::Error| format!("cannot write {}: {e}", staged.display());
    if let Err(e) = fs::remove_file(staged)
        && e.kind() != io::ErrorKind::NotFound
    {
        return Err(err(e));
    }
    fs::copy(src, staged).map_err(err)?;
    if is_root() {
        std::os::unix::fs::chown(staged, Some(like.uid()), Some(like.gid())).map_err(err)?;
    }
    // after chown, which clears set-id bits
    fs::set_permissions(staged, fs::Permissions::from_mode(like.mode() & 0o7777)).map_err(err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rmux-update-swap-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut out: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        out.sort();
        out
    }

    #[test]
    fn names_follow_contract() {
        assert_eq!(
            rollback_name("20261020", "0.1.0"),
            "rmux.rollback-20261020-pre-v0.1.0"
        );
        assert_eq!(staging_name(4242), ".rmux.new.4242");
        let date = local_date().unwrap();
        assert_eq!(date.len(), 8);
        assert!(date.bytes().all(|b| b.is_ascii_digit()));
    }

    #[test]
    fn swap_keeps_old_inode_mode_and_rollback() {
        let dir = scratch("ok");
        let target = dir.join("rmux");
        let new = dir.join("new-rmux");
        fs::write(&target, b"old binary").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o750)).unwrap();
        fs::write(&new, b"new binary").unwrap();
        fs::set_permissions(&new, fs::Permissions::from_mode(0o644)).unwrap();
        let old_ino = fs::metadata(&target).unwrap().ino();
        let mut running = fs::File::open(&target).unwrap();

        let rollback =
            install_with(&new, &target, "20261020", 7, &|a, b| fs::rename(a, b)).unwrap();

        assert_eq!(rollback, dir.join(rollback_name("20261020", RELEASE)));
        let mut seen = Vec::new();
        running.read_to_end(&mut seen).unwrap();
        assert_eq!(seen, b"old binary");
        assert_eq!(running.metadata().unwrap().ino(), old_ino);
        assert_ne!(fs::metadata(&target).unwrap().ino(), old_ino);
        assert_eq!(fs::read(&target).unwrap(), b"new binary");
        assert_eq!(fs::metadata(&target).unwrap().mode() & 0o7777, 0o750);
        assert_eq!(fs::read(&rollback).unwrap(), b"old binary");
        assert_eq!(fs::metadata(&rollback).unwrap().mode() & 0o7777, 0o750);
        assert!(!dir.join(staging_name(7)).exists());

        // a second run the same day replaces the rollback, even read-only
        fs::set_permissions(&rollback, fs::Permissions::from_mode(0o444)).unwrap();
        fs::write(&new, b"newer binary").unwrap();
        install_with(&new, &target, "20261020", 7, &|a, b| fs::rename(a, b)).unwrap();
        assert_eq!(fs::read(&rollback).unwrap(), b"new binary");
        assert_eq!(fs::read(&target).unwrap(), b"newer binary");
        let mut want = vec![
            "new-rmux".to_string(),
            "rmux".to_string(),
            rollback_name("20261020", RELEASE),
        ];
        want.sort();
        assert_eq!(names(&dir), want);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn failed_swap_removes_staged_file_and_leaves_target() {
        let dir = scratch("fail");
        let target = dir.join("rmux");
        let new = dir.join("new-rmux");
        fs::write(&target, b"old binary").unwrap();
        fs::write(&new, b"new binary").unwrap();
        let fail_on_target = |from: &Path, to: &Path| {
            if to.file_name().is_some_and(|n| n == "rmux") {
                Err(io::Error::other("simulated"))
            } else {
                fs::rename(from, to)
            }
        };

        let err = install_with(&new, &target, "20261020", 9, &fail_on_target).unwrap_err();

        assert!(err.contains("simulated"), "{err}");
        assert!(!dir.join(staging_name(9)).exists());
        assert_eq!(fs::read(&target).unwrap(), b"old binary");

        let missing = install_with(&dir.join("absent"), &target, "20261020", 9, &|a, b| {
            fs::rename(a, b)
        });
        assert!(missing.is_err());
        assert!(!dir.join(staging_name(9)).exists());
        assert_eq!(fs::read(&target).unwrap(), b"old binary");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn writable_probe() {
        let dir = scratch("probe");
        assert!(writable(&dir));
        assert_eq!(names(&dir), Vec::<String>::new());
        assert!(!writable(&dir.join("absent")));
        fs::remove_dir_all(&dir).unwrap();
    }
}
