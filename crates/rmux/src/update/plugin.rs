//! Whether an update changes the omp plugin the user exported earlier.

use std::path::Path;
use std::process::{Command, Stdio};

const PACKAGE_JSON: &[u8] = include_bytes!("../../omp/package.json");
const RMUX_TS: &[u8] = include_bytes!("../../omp/rmux.ts");

/// `Some(note)` when the omp plugin bundled in `new_exe` differs from this binary's.
pub fn note(new_exe: &Path) -> Option<String> {
    let dir = std::env::temp_dir().join(format!("rmux-plugin-check.{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let same = bundled_matches(new_exe, &dir);
    let _ = std::fs::remove_dir_all(&dir);
    if same? {
        None
    } else {
        Some(
            "the bundled omp plugin changed; run: rmux omp-plugin ~/.omp/local-plugins/rmux-terminal, then restart omp"
                .to_owned(),
        )
    }
}

fn bundled_matches(new_exe: &Path, dir: &Path) -> Option<bool> {
    let status = Command::new(new_exe)
        .arg("omp-plugin")
        .arg(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;
    if !status.success() {
        return None;
    }
    let package_json = std::fs::read(dir.join("package.json")).ok()?;
    let rmux_ts = std::fs::read(dir.join("rmux.ts")).ok()?;
    Some(package_json == PACKAGE_JSON && rmux_ts == RMUX_TS)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn fake_exe(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(
            &path,
            format!("#!/bin/sh\nset -e\nmkdir -p \"$2\"\n{body}\n"),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    // One test: `note` uses a per-process directory, so calls must not overlap.
    #[test]
    fn note_compares_exported_plugin() {
        let dir = std::env::temp_dir().join(format!("rmux-plugin-test.{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let omp = concat!(env!("CARGO_MANIFEST_DIR"), "/omp");

        let same = fake_exe(
            &dir,
            "same",
            &format!("cp '{omp}/package.json' '{omp}/rmux.ts' \"$2\"/"),
        );
        let changed = fake_exe(
            &dir,
            "changed",
            &format!("cp '{omp}/package.json' \"$2\"/\necho changed > \"$2\"/rmux.ts"),
        );
        let failing = fake_exe(&dir, "failing", "exit 1");
        let check_dir =
            std::env::temp_dir().join(format!("rmux-plugin-check.{}", std::process::id()));

        let results = [
            note(&same),
            note(&changed),
            note(&failing),
            note(&dir.join("missing")),
        ];
        let leftover = check_dir.exists();
        std::fs::remove_dir_all(&dir).unwrap();

        assert_eq!(results[0], None);
        assert_eq!(
            results[1].as_deref(),
            Some(
                "the bundled omp plugin changed; run: rmux omp-plugin ~/.omp/local-plugins/rmux-terminal, then restart omp"
            )
        );
        assert_eq!(results[2], None);
        assert_eq!(results[3], None);
        assert!(!leftover);
    }
}
