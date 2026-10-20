use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn rerun_if_exists(path: &Path) {
    if path.exists() {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let manifest = Path::new(&manifest);
    let git_dir = manifest.join("../../.git");

    let head = git_dir.join("HEAD");
    rerun_if_exists(&head);
    if let Some(reference) = std::fs::read_to_string(&head)
        .ok()
        .and_then(|contents| contents.trim().strip_prefix("ref: ").map(str::to_string))
    {
        rerun_if_exists(&git_dir.join(reference));
    }
    rerun_if_exists(&git_dir.join("packed-refs"));
    println!("cargo:rerun-if-env-changed=RMUX_GIT_SHA");

    let sha = match git(manifest, &["rev-parse", "--short=12", "HEAD"]) {
        Some(sha) if !sha.is_empty() => {
            let dirty = git(manifest, &["status", "--porcelain"]).is_some_and(|s| !s.is_empty());
            if dirty { format!("{sha}-dirty") } else { sha }
        }
        _ => std::env::var("RMUX_GIT_SHA")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "unknown".to_string()),
    };
    println!("cargo:rustc-env=RMUX_GIT_SHA={sha}");

    let target = std::env::var("TARGET").expect("TARGET");
    println!("cargo:rustc-env=RMUX_TARGET={target}");
}
