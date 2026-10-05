//! Log file behavior (spec g01 section 2.7, work item 7). Scenarios run in a
//! child process because the log is process-global and `fatal!` exits.

mod common;

use std::path::Path;
use std::process::Command;

use rmux_util::log;

fn run_child(scenario: &str, dir: &Path) -> std::process::Output {
    Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_hook", "--nocapture", "--test-threads=1"])
        .env("RMUX_LOG_CHILD", scenario)
        .current_dir(dir)
        .output()
        .unwrap()
}

fn log_files(dir: &Path, name: &str) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|f| f.starts_with(&format!("rmux-{name}-")) && f.ends_with(".log"))
        .collect();
    v.sort();
    v
}

fn read_only_log(dir: &Path, name: &str) -> String {
    let files = log_files(dir, name);
    assert_eq!(files.len(), 1, "{files:?}");
    std::fs::read_to_string(dir.join(&files[0])).unwrap()
}

fn check_line(line: &str, expected_rest: &str) {
    let (stamp, rest) = line.split_once(' ').unwrap();
    let (sec, usec) = stamp.split_once('.').unwrap();
    assert!(sec.parse::<u64>().unwrap() > 1_600_000_000, "{stamp}");
    assert_eq!(usec.len(), 6, "{stamp}");
    assert!(usec.bytes().all(|b| b.is_ascii_digit()), "{stamp}");
    assert_eq!(rest, expected_rest);
}

#[test]
fn child_hook() {
    let Ok(scenario) = std::env::var("RMUX_LOG_CHILD") else {
        return;
    };
    match scenario.as_str() {
        "level-zero" => {
            log::open("server");
            assert!(!log::enabled());
            rmux_util::log_debug!("dropped {}", 1);
        }
        "failed-open" => {
            log::add_level();
            log::open("server");
            assert!(!log::enabled());
            assert_eq!(log::level(), log::LogLevel(1));
        }
        "toggle" => {
            log::toggle("server");
            assert!(log::enabled());
            rmux_util::log_debug!("between");
            log::toggle("server");
            assert!(!log::enabled());
            assert_eq!(log::level(), log::LogLevel(0));
            rmux_util::log_debug!("after close");
        }
        "reopen" => {
            log::add_level();
            log::open("client");
            rmux_util::log_debug!("first");
            log::close();
            log::open("client");
            rmux_util::log_debug!("second");
            log::open("client");
            rmux_util::log_debug!("third");
        }
        "vis" => {
            log::add_level();
            log::open("server");
            rmux_util::log_debug!("tab\tnl\nesc\x1be\u{e9} nul\0cut");
        }
        "fatal" => {
            log::add_level();
            log::open("server");
            let _ = std::fs::File::open("/nonexistent/rmux-fatal-test");
            rmux_util::fatal!("open failed {}", 42);
        }
        "fatal-argument-errno" => {
            log::add_level();
            log::open("server");
            let _ = std::fs::File::open("/nonexistent/rmux-fatal-test");
            rmux_util::fatal!("open failed {}", {
                let _ = std::fs::File::open(".");
                let _ = std::fs::read(".");
                42
            });
        }
        "fatalx" => {
            log::add_level();
            log::open("server");
            rmux_util::fatalx!("bad {}", "thing");
        }
        "fatal-silent" => {
            rmux_util::fatalx!("nothing open");
        }
        other => panic!("unknown scenario {other}"),
    }
}

#[test]
fn level_zero_open_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let out = run_child("level-zero", dir.path());
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(log_files(dir.path(), "server").is_empty());
}

#[test]
fn failed_open_keeps_level_without_file() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("ro");
    std::fs::create_dir(&sub).unwrap();
    std::fs::set_permissions(&sub, std::os::unix::fs::PermissionsExt::from_mode(0o555)).unwrap();
    if std::fs::File::create(sub.join("probe")).is_ok() {
        eprintln!("failed-open skipped: directory is writable (running as root?)");
        return;
    }
    let out = run_child("failed-open", &sub);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(log_files(&sub, "server").is_empty());
}

#[test]
fn toggle_twice_writes_open_and_close() {
    let dir = tempfile::tempdir().unwrap();
    let out = run_child("toggle", dir.path());
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = read_only_log(dir.path(), "server");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3, "{text}");
    check_line(lines[0], "log opened");
    check_line(lines[1], "between");
    check_line(lines[2], "log closed");
}

#[test]
fn reopen_appends_to_the_same_file() {
    let dir = tempfile::tempdir().unwrap();
    let out = run_child("reopen", dir.path());
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = read_only_log(dir.path(), "client");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3, "{text}");
    check_line(lines[0], "first");
    check_line(lines[1], "second");
    check_line(lines[2], "third");
}

#[test]
fn message_passes_through_raw_vis() {
    let dir = tempfile::tempdir().unwrap();
    let out = run_child("vis", dir.path());
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = read_only_log(dir.path(), "server");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 1, "{text}");
    check_line(lines[0], "tab\\tnl\\nesc\\033e\\303\\251 nul");
}

#[test]
fn fatal_has_errno_text_and_exits_one() {
    let dir = tempfile::tempdir().unwrap();
    let out = run_child("fatal", dir.path());
    assert_eq!(out.status.code(), Some(1));
    assert!(
        out.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = read_only_log(dir.path(), "server");
    let lines: Vec<&str> = text.lines().collect();
    let expected = format!(
        "fatal: {}: open failed 42",
        String::from_utf8(rmux_sys::strerror(libc_enoent())).unwrap()
    );
    check_line(lines.last().unwrap(), &expected);
    assert!(!expected.contains("os error"));
}

#[test]
fn fatal_captures_errno_before_argument_evaluation() {
    let dir = tempfile::tempdir().unwrap();
    let out = run_child("fatal-argument-errno", dir.path());
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stderr.is_empty());
    let text = read_only_log(dir.path(), "server");
    check_line(
        text.lines().last().unwrap(),
        "fatal: No such file or directory: open failed 42",
    );
}

fn libc_enoent() -> i32 {
    std::fs::File::open("/nonexistent/rmux-enoent-probe")
        .err()
        .and_then(|e| e.raw_os_error())
        .unwrap()
}

#[test]
fn fatalx_exits_one_with_last_line() {
    let dir = tempfile::tempdir().unwrap();
    let out = run_child("fatalx", dir.path());
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stderr.is_empty());
    let text = read_only_log(dir.path(), "server");
    check_line(text.lines().last().unwrap(), "fatal: bad thing");
}

#[test]
fn fatal_without_file_is_silent() {
    let dir = tempfile::tempdir().unwrap();
    let out = run_child("fatal-silent", dir.path());
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stderr.is_empty());
    assert!(log_files(dir.path(), "server").is_empty());
}

#[test]
fn logging_and_fatal_match_pinned_c() {
    let Some(main) = common::write_c(
        "log-reference.c",
        r#"
#include "tmux.h"
#include <errno.h>
#include <string.h>
void event_set_log_callback(void (*callback)(int, const char *)) { (void)callback; }
int main(int argc, char **argv) {
    log_add_level();
    log_open("server");
    if (strcmp(argv[1], "fatal") == 0) {
        errno = ENOENT;
        fatal("open failed %d", 42);
    }
    log_debug("tab\tnl\nesc\033e\303\251 nul%c%s", 0, "cut");
    log_close();
    return 0;
}
"#,
    ) else {
        return;
    };
    let Some(bin) = common::build_c(
        "log",
        &[
            &main,
            Path::new("log.c"),
            Path::new("xmalloc.c"),
            Path::new("compat/vis.c"),
            Path::new("compat/reallocarray.c"),
            Path::new("compat/recallocarray.c"),
            Path::new("compat/explicit_bzero.c"),
        ],
        &[],
        false,
    ) else {
        return;
    };
    for scenario in ["vis", "fatal"] {
        let cdir = tempfile::tempdir().unwrap();
        let c = Command::new(&bin)
            .arg(scenario)
            .current_dir(cdir.path())
            .output()
            .unwrap();
        let cpath = std::fs::read_dir(cdir.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert!(
            cpath
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("tmux-server-")
        );
        let expected = std::fs::read_to_string(cpath).unwrap();
        let rdir = tempfile::tempdir().unwrap();
        let rust = run_child(scenario, rdir.path());
        assert_eq!(rust.status.code(), c.status.code());
        assert_eq!(rust.stderr, c.stderr);
        let actual = read_only_log(rdir.path(), "server");
        let (_, message) = expected.trim_end().split_once(' ').unwrap();
        check_line(actual.trim_end(), message);
    }
}
