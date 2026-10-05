// Ported from tmux tmux.c @ 8f25579c (CLI behaviour differential against the oracle)
//! Spec g15_client.md section 6, differential test 1: the CLI matrix. The
//! oracle (`oracle/bin/tmux`) output is mapped `tmux` -> `rmux` and
//! `TMUX_TMPDIR`/`tmux-<uid>` -> `RMUX_TMPDIR`/`rmux-<uid>` before comparison.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn oracle() -> Option<PathBuf> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../oracle/bin/tmux");
    path.is_file().then_some(path)
}

fn rmux() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rmux"))
}

fn run(bin: &Path, args: &[&str], env: &[(&str, &str)], clear: &[&str]) -> Output {
    let mut command = Command::new(bin);
    command.args(args);
    for name in clear {
        command.env_remove(name);
    }
    for (name, value) in env {
        command.env(name, value);
    }
    command.output().expect("spawn")
}

fn map(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .replace("tmux: ", "rmux: ")
        .replace("usage: tmux", "usage: rmux")
        .replace("tmux next-", "rmux next-")
        .replace("tmux-", "rmux-")
        .replace("TMUX_TMPDIR", "RMUX_TMPDIR")
}

/// Compare one invocation: status, mapped stdout and mapped stderr.
fn compare(args: &[&str], tmp: &Path) {
    let Some(oracle) = oracle() else {
        eprintln!("skipping: oracle binary missing");
        return;
    };
    let clear = ["TMUX", "RMUX", "TMUX_TMPDIR", "RMUX_TMPDIR"];
    let tmp = tmp.to_str().unwrap();
    let theirs = run(&oracle, args, &[("TMUX_TMPDIR", tmp)], &clear);
    let ours = run(&rmux(), args, &[("RMUX_TMPDIR", tmp)], &clear);
    assert_eq!(
        ours.status.code(),
        theirs.status.code(),
        "exit status for {args:?}"
    );
    assert_eq!(
        map(&ours.stdout),
        map(&theirs.stdout),
        "stdout for {args:?}"
    );
    assert_eq!(
        map(&ours.stderr),
        map(&theirs.stderr),
        "stderr for {args:?}"
    );
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rmux-cli-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn version_help_and_bad_options_match_the_oracle() {
    let tmp = temp_dir("opts");
    compare(&["-V"], &tmp);
    compare(&["-h"], &tmp);
    compare(&["-x"], &tmp);
    // -d and -U are in the getopt string but have no case label (tmux.c:526-527).
    compare(&["-d"], &tmp);
    compare(&["-U"], &tmp);
    // -c with a positional command and -D with a command are usage errors.
    compare(&["-c", "true", "ls"], &tmp);
    compare(&["-D", "ls"], &tmp);
    // A missing option argument.
    compare(&["-L"], &tmp);
}

#[test]
fn socket_path_errors_match_the_oracle() {
    let Some(oracle) = oracle() else {
        eprintln!("skipping: oracle binary missing");
        return;
    };
    let clear = ["TMUX", "RMUX", "TMUX_TMPDIR", "RMUX_TMPDIR"];
    let cases: &[(&str, &[&str])] = &[
        ("relative", &["-N", "ls"]),
        ("/a/../b", &["-N", "ls"]),
        ("/nonexistent/rmux-cli-dir", &["-N", "ls"]),
    ];
    for (dir, args) in cases {
        let theirs = run(&oracle, args, &[("TMUX_TMPDIR", dir)], &clear);
        let ours = run(&rmux(), args, &[("RMUX_TMPDIR", dir)], &clear);
        assert_eq!(ours.status.code(), theirs.status.code(), "status for {dir}");
        assert_eq!(map(&ours.stderr), map(&theirs.stderr), "stderr for {dir}");
    }
    // Not a directory, then unsafe permissions.
    let base = temp_dir("sock");
    let uid = rmux_sys::proc::getuid().0;
    std::fs::write(base.join(format!("tmux-{uid}")), b"").unwrap();
    std::fs::write(base.join(format!("rmux-{uid}")), b"").unwrap();
    let dir = base.to_str().unwrap();
    let theirs = run(&oracle, &["-N", "ls"], &[("TMUX_TMPDIR", dir)], &clear);
    let ours = run(&rmux(), &["-N", "ls"], &[("RMUX_TMPDIR", dir)], &clear);
    assert_eq!(ours.status.code(), theirs.status.code());
    assert_eq!(map(&ours.stderr), map(&theirs.stderr));

    let base = temp_dir("perm");
    use std::os::unix::fs::PermissionsExt;
    for name in [format!("tmux-{uid}"), format!("rmux-{uid}")] {
        let d = base.join(name);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o707)).unwrap();
    }
    let dir = base.to_str().unwrap();
    let theirs = run(&oracle, &["-N", "ls"], &[("TMUX_TMPDIR", dir)], &clear);
    let ours = run(&rmux(), &["-N", "ls"], &[("RMUX_TMPDIR", dir)], &clear);
    assert_eq!(ours.status.code(), theirs.status.code());
    assert_eq!(map(&ours.stderr), map(&theirs.stderr));
}

#[test]
fn no_server_running_text_matches_the_oracle() {
    let tmp = temp_dir("noserver");
    // -N: never start a server; the connect error text is printed.
    compare(&["-N", "-L", "cli-none", "ls"], &tmp);
}
