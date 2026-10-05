//! G21 (commands B) differential checks: the same scripted command sequence runs
//! against the pinned oracle (`oracle/bin/tmux`) and the rmux binary on private
//! sockets; stdout, stderr and exit status must match for every step
//! (specs/g21_commands_b.md section 6 "Differential tests to add").
//!
//! Skips with a message when either binary is missing. Set `RMUX_BINARY` to
//! point at a freshly built `rmux`; `RMUX_ORACLE` overrides the oracle path.

use rmux_harness::differential::{Difference, Step, compare};
use std::path::{Path, PathBuf};

fn oracle() -> Option<PathBuf> {
    let path = std::env::var_os("RMUX_ORACLE")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../oracle/bin/tmux"));
    path.exists().then_some(path)
}

fn rmux() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("RMUX_BINARY") {
        let path = PathBuf::from(path);
        return path.exists().then_some(path);
    }
    let mut candidates = Vec::new();
    if let Some(dir) = std::env::var_os("CARGO_TARGET_DIR") {
        candidates.push(PathBuf::from(dir).join("debug/rmux"));
    }
    candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/debug/rmux"));
    candidates.into_iter().find(|p| p.exists())
}

fn script(steps: &[&[&str]]) -> Vec<Step> {
    steps
        .iter()
        .map(|step| match *step {
            ["wait-for-pane-output", target, expected] => Step::WaitForPaneOutput {
                target: (*target).to_string(),
                expected: expected.as_bytes().to_vec(),
            },
            _ => Step::Command(step.iter().map(|s| (*s).to_string()).collect()),
        })
        .collect()
}

fn report(name: &str, differences: &[Difference]) {
    for d in differences {
        eprintln!(
            "{name}: step {} differs\n  oracle: status={:?}\n  stdout={:?}\n  stderr={:?}\n  rmux:   status={:?}\n  stdout={:?}\n  stderr={:?}",
            d.command_index,
            d.left.status,
            String::from_utf8_lossy(&d.left.stdout),
            String::from_utf8_lossy(&d.left.stderr),
            d.right.status,
            String::from_utf8_lossy(&d.right.stdout),
            String::from_utf8_lossy(&d.right.stderr),
        );
    }
}

/// Replace the per-server temp directory token (`rd-XXXXXX`) so paths compare equal.
fn normalize(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"rd-")
            && bytes.len() >= i + 9
            && bytes[i + 3..i + 9].iter().all(u8::is_ascii_alphanumeric)
        {
            out.extend_from_slice(b"rd-XXXXXX");
            i += 9;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

fn same_after_normalize(d: &Difference) -> bool {
    d.left.status == d.right.status
        && normalize(&d.left.stdout) == normalize(&d.right.stdout)
        && normalize(&d.left.stderr) == normalize(&d.right.stderr)
}

/// Runs `steps` on both binaries; `None` when a binary is missing (skip).
fn differential(name: &str, steps: &[&[&str]]) -> Option<Vec<Difference>> {
    let (Some(oracle), Some(rmux)) = (oracle(), rmux()) else {
        eprintln!("{name}: skipped, oracle or rmux binary missing (set RMUX_ORACLE / RMUX_BINARY)");
        return None;
    };
    let mut differences = compare(&oracle, &rmux, &script(steps)).expect("differential run");
    differences.retain(|d| !same_after_normalize(d));
    report(name, &differences);
    Some(differences)
}

macro_rules! differential_test {
    ($name:ident, $steps:expr) => {
        #[test]
        fn $name() {
            if let Some(differences) = differential(stringify!($name), $steps) {
                assert!(differences.is_empty(), "{} steps differ", differences.len());
            }
        }
    };
}

// A complete marker line proves cat has consumed all preceding input before
// capture or reset. The harness polls both servers with the same bounded wait.
const CAT_WINDOW: &[&str] = &[
    "new-window",
    "-d",
    "-n",
    "cat",
    "stty -echo; printf '__cat_ready__\n'; exec cat",
];
const WAIT_FOR_CAT: &[&str] = &["wait-for-pane-output", "cat", "__cat_ready__\n"];
const OUTPUT_MARKER: &[&str] = &["send-keys", "-t", "cat", "-l", "\n__capture_ready__\n"];
const WAIT_FOR_OUTPUT: &[&str] = &["wait-for-pane-output", "cat", "\n__capture_ready__\n"];

differential_test!(
    show_options_scopes,
    &[
        &["show-options", "-g"],
        &["show-options", "-gw"],
        &["show-options", "-s"],
        &["show-hooks", "-g"],
        &["show-options", "-gA"],
        &["show-options", "-gv", "status"],
        &["show-options", "-gH"],
        &[
            "show-options",
            "-gw",
            "-F",
            "#{option_name}=#{option_value}"
        ],
        &[
            "show-options",
            "-g",
            "-F",
            "x#{option_is_array}#{option_has_value}",
            "status-format"
        ],
        &["set-option", "-g", "@user", "value with spaces"],
        &["show-options", "-g", "@user"],
        &["show-options", "-gv", "@user"],
        &["show-options", "-g", "@missing"],
        &["show-options", "-gq", "@missing"],
        &["show-options", "-g", "stat"],
        &["show-options", "-g", "nosuchoption"],
        &["show-window-options", "-g"],
        &["show-options", "status-format[1]"],
        &["show-options", "-gv", "status-format"],
        &["set-option", "-g", "status-format[7]", "x"],
        &["show-options", "-g", "status-format"],
        &["set-option", "-gu", "status-format[7]"],
        &["show-options", "-g", "status-format[7]"],
        &["set-option", "-g", "status", "nope"],
        &["set-option", "-g", "status-left", "left"],
        &["set-option", "-ga", "status-left", "+more"],
        &["show-options", "-g", "status-left"],
        &["set-option", "-go", "status-left", "x"],
        &["set-option", "-goq", "status-left", "x"],
        &["set-option", "status-left", "session"],
        &["show-options", "status-left"],
        &["show-options", "-A", "status-right"],
        &["set-option", "-u", "status-left"],
        &["show-options", "-q", "status-left"],
        &["set-option", "-g", "@user"],
        &["set-option", "-g", "status-left[1]", "x"],
        &["set-option", "-w", "pane-border-status", "top"],
        &["show-window-options", "pane-border-status"],
        &["set-window-option", "-u", "pane-border-status"],
        &["set-option", "-gU", "pane-border-status"],
        &["set-option", "-p", "window-style", "bg=red"],
        &["show-options", "-p"],
        &["set-option", "-gq", "nosuch", "1"],
        &["set-option", "-g", "nosuch", "1"],
        &["set-option", "-g", "stat", "off"],
        &["set-option", "-g", "escape-time", "abc"],
        &["set-option", "-g", "escape-time", "-5"],
        &["set-hook", "-g", "after-new-window", "display -p hooked"],
        &["show-hooks", "-g", "after-new-window"],
        &["show-hooks", "-g", "-F", "#{option_name}"],
        &["set-hook", "-gu", "after-new-window"],
        &["set-hook", "-g", "@evt", "display -p user"],
        &["show-hooks", "-g"],
        &["set-hook", "-g", "nosuch-hook", "x"],
        &["set-hook", "-E"],
        &["set-hook", "-E", "a", "b"],
        &["set-hook", "-E", "nope"],
        &["set-hook", "-E", "@evt"],
        &["set-hook", "-gB", "bad", "x"],
        &["set-hook", "-gB", "%*:#{pane_id}", "x"],
        &["set-hook", "-gB", "@mon::#{pane_id}", "display -p m"],
        &["show-hooks", "-gB"],
        &["set-hook", "-guB", "@mon::#{pane_id}"],
        &["set-hook", "-R", "@evt"],
    ]
);

differential_test!(
    show_environment_escaping,
    &[
        &["set-environment", "PLAIN", "value"],
        &["set-environment", "SPECIAL", "a$b`c\"d\\e"],
        &["set-environment", "-g", "GLOBAL", "g"],
        &["set-environment", "-h", "HIDDEN", "h"],
        &["set-environment", "-r", "REMOVED"],
        &["set-environment", "-u", "UNSET"],
        &["set-environment", "", "x"],
        &["set-environment", "A=B", "x"],
        &["set-environment", "-u", "X", "value"],
        &["set-environment", "-r", "X", "value"],
        &["set-environment", "NOVALUE"],
        &["set-environment", "-F", "FMT", "#{session_name}"],
        &["show-environment", "PLAIN"],
        &["show-environment", "SPECIAL"],
        &["show-environment", "-s", "SPECIAL"],
        &["show-environment", "FMT"],
        &["show-environment", "REMOVED"],
        &["show-environment", "-s", "REMOVED"],
        &["show-environment", "HIDDEN"],
        &["show-environment", "-h", "HIDDEN"],
        &["show-environment", "-g", "GLOBAL"],
        &["show-environment", "MISSING"],
        &["show-environment", "-t", "nosuch"],
        &["show-environment", "-gs", "GLOBAL"],
    ]
);

differential_test!(
    buffers_and_paste,
    &[
        CAT_WINDOW,
        WAIT_FOR_CAT,
        &["set-buffer", "hello"],
        &["set-buffer", "-b", "named", "line1\nline2\n"],
        &["set-buffer", "-a", "-b", "named", "tail"],
        &["set-buffer", "-a", "appended-to-nothing"],
        &["set-buffer", ""],
        &["set-buffer"],
        &["set-buffer", "-b", "named", "-n", "renamed"],
        &["set-buffer", "-b", "nosuch", "-n", "x"],
        &[
            "list-buffers",
            "-F",
            "#{buffer_name}:#{buffer_size}:#{buffer_sample}"
        ],
        &["show-buffer"],
        &["show-buffer", "-b", "renamed"],
        &["show-buffer", "-b", "missing"],
        &["paste-buffer", "-t", "cat", "-b", "missing"],
        &["paste-buffer", "-t", "cat", "-b", "renamed", "-s", "|"],
        &["paste-buffer", "-t", "cat", "-r", "-b", "renamed"],
        &["set-buffer", "-b", "esc", "x\x1b[31my"],
        &["paste-buffer", "-t", "cat", "-b", "esc"],
        &["paste-buffer", "-t", "cat", "-S", "-b", "esc"],
        &["paste-buffer", "-t", "cat", "-p", "-d"],
        OUTPUT_MARKER,
        WAIT_FOR_OUTPUT,
        &["capture-pane", "-p", "-t", "cat"],
        &["list-buffers", "-F", "#{buffer_name}"],
        &["delete-buffer", "-b", "renamed"],
        &["delete-buffer", "-b", "renamed"],
        &["delete-buffer"],
        &["delete-buffer"],
        &["show-buffer"],
        &["save-buffer", "out.txt"],
        &["set-buffer", "saved"],
        &["save-buffer", "out.txt"],
        &["save-buffer", "-a", "out.txt"],
        &["run-shell", "cat out.txt"],
        &["save-buffer", "/nonexistent-dir/x"],
        &["paste-buffer", "-t", "nosuchpane"],
        &["select-pane", "-d", "-t", "cat"],
        &["paste-buffer", "-t", "cat", "-d"],
        &["list-buffers"],
    ]
);

differential_test!(
    send_keys_forms,
    &[
        CAT_WINDOW,
        WAIT_FOR_CAT,
        &["send-keys", "-t", "cat", "-l", "héllo wörld"],
        &["send-keys", "-t", "cat", "Enter"],
        &[
            "send-keys",
            "-t",
            "cat",
            "-H",
            "41",
            "0x42",
            "+43",
            " 44",
            "Enter"
        ],
        &["send-keys", "-t", "cat", "-H", "-1"],
        &["send-keys", "-t", "cat", "-H", "100"],
        &["send-keys", "-t", "cat", "-H", "4g"],
        &["send-keys", "-t", "cat", "-N", "3", "ab"],
        &["send-keys", "-t", "cat", "Enter"],
        &["send-keys", "-t", "cat", "-N", "0", "x"],
        &["send-keys", "-t", "cat", "-N", "zz", "x"],
        &["send-keys", "-t", "cat", "-N", "2"],
        &["send-keys", "-t", "cat", "-X", "cancel"],
        &["send-keys", "-t", "cat", "-M"],
        &[
            "send-keys",
            "-t",
            "cat",
            "Space",
            "Tab",
            "C-a",
            "F1",
            "Enter"
        ],
        OUTPUT_MARKER,
        WAIT_FOR_OUTPUT,
        &["send-keys", "-t", "cat", "-R"],
        &["capture-pane", "-p", "-t", "cat"],
        &["send-prefix", "-t", "cat"],
        &["send-prefix", "-2", "-t", "cat"],
        &["send-keys", "-t", "nosuch", "x"],
    ]
);

differential_test!(
    resize_and_layout,
    &[
        &["split-window", "-d", "-h"],
        &["split-window", "-d", "-v", "-t", "0"],
        &[
            "list-panes",
            "-F",
            "#{pane_index}:#{pane_left},#{pane_top},#{pane_width},#{pane_height}"
        ],
        &["resize-pane", "-t", "0", "-U", "3", "-L", "2"],
        &[
            "list-panes",
            "-F",
            "#{pane_index}:#{pane_left},#{pane_top},#{pane_width},#{pane_height}"
        ],
        &["resize-pane", "-t", "0", "-x", "50%"],
        &[
            "list-panes",
            "-F",
            "#{pane_index}:#{pane_left},#{pane_top},#{pane_width},#{pane_height}"
        ],
        &["set-option", "-w", "pane-border-status", "top"],
        &["resize-pane", "-t", "0", "-y", "100%"],
        &[
            "list-panes",
            "-F",
            "#{pane_index}:#{pane_left},#{pane_top},#{pane_width},#{pane_height}"
        ],
        &["resize-pane", "-t", "0", "-D"],
        &["resize-pane", "-t", "0", "-R", "abc"],
        &["resize-pane", "-t", "0", "-x", "abc"],
        &["resize-pane", "-t", "0", "-y", "0"],
        &["resize-pane", "-Z"],
        &["display", "-p", "#{window_zoomed_flag}"],
        &["resize-pane", "-Z"],
        &["display", "-p", "#{window_zoomed_flag} #{window_layout}"],
        &["resize-pane", "-T"],
        &["resize-window", "-x", "100", "-y", "30"],
        &[
            "display",
            "-p",
            "#{window_width}x#{window_height} #{window_size}"
        ],
        &["resize-window", "-L", "10"],
        &["resize-window", "-U"],
        &["resize-window", "-R", "5", "-D", "3"],
        &["display", "-p", "#{window_width}x#{window_height}"],
        &["resize-window", "-x", "1"],
        &["resize-window", "-y", "abc"],
        &["resize-window", "nope"],
        &["resize-window", "-A"],
        &["display", "-p", "#{window_width}x#{window_height}"],
        &["select-layout", "even-horizontal"],
        &["display", "-p", "#{window_layout}"],
        &["select-layout", "main-vertical"],
        &["display", "-p", "#{window_layout}"],
        &["next-layout"],
        &["display", "-p", "#{window_layout}"],
        &["previous-layout"],
        &["display", "-p", "#{window_layout}"],
        &["select-layout", "-E"],
        &["display", "-p", "#{window_layout}"],
        &["select-layout", "bad-layout"],
        &["select-layout", "-o"],
        &["display", "-p", "#{window_layout}"],
        &["select-layout", "-n"],
        &["select-layout", "-p"],
        &["select-layout"],
        &["display", "-p", "#{window_layout}"],
    ]
);

differential_test!(
    rotate_and_swap,
    &[
        &["split-window", "-d"],
        &["split-window", "-d", "-h"],
        &["new-window", "-d", "-n", "other"],
        &["split-window", "-d", "-t", "other"],
        &[
            "list-panes",
            "-F",
            "#{pane_id}:#{pane_index}:#{pane_active}"
        ],
        &["rotate-window", "-D"],
        &[
            "list-panes",
            "-F",
            "#{pane_id}:#{pane_index}:#{pane_active}"
        ],
        &["display", "-p", "#{window_layout}"],
        &["rotate-window", "-U"],
        &["rotate-window"],
        &[
            "list-panes",
            "-F",
            "#{pane_id}:#{pane_index}:#{pane_active}"
        ],
        &["display", "-p", "#{window_layout}"],
        &["swap-pane", "-D"],
        &[
            "list-panes",
            "-F",
            "#{pane_id}:#{pane_index}:#{pane_active}"
        ],
        &["swap-pane", "-U", "-d"],
        &[
            "list-panes",
            "-F",
            "#{pane_id}:#{pane_index}:#{pane_active}"
        ],
        &["swap-pane", "-s", "0", "-t", "2"],
        &[
            "list-panes",
            "-F",
            "#{pane_id}:#{pane_index}:#{pane_active}"
        ],
        &["swap-pane", "-s", "other.0", "-t", "test:0.1"],
        &[
            "list-panes",
            "-a",
            "-F",
            "#{window_index}.#{pane_index}:#{pane_id}:#{pane_active}"
        ],
        &["display", "-p", "#{window_layout}"],
        &["swap-pane", "-s", "0", "-t", "0"],
        &["swap-pane", "-s", "nosuch"],
        &["swap-window", "-s", "0", "-t", "other"],
        &["run-shell", "sleep 0.6"],
        &[
            "list-windows",
            "-F",
            "#{window_index}:#{window_name}:#{window_active}"
        ],
        &["swap-window", "-d", "-s", "0", "-t", "1"],
        &["run-shell", "sleep 0.6"],
        &[
            "list-windows",
            "-F",
            "#{window_index}:#{window_name}:#{window_active}"
        ],
        &["swap-window", "-s", "0", "-t", "0"],
        &["swap-window", "-s", "0", "-t", "99"],
    ]
);

differential_test!(
    select_pane_and_window,
    &[
        &["split-window", "-d"],
        &["split-window", "-d", "-h"],
        &["select-pane", "-t", "0"],
        &["select-pane", "-D"],
        &["display", "-p", "#{pane_index}"],
        &["select-pane", "-U"],
        &["select-pane", "-R"],
        &["select-pane", "-L"],
        &["display", "-p", "#{pane_index}"],
        &["select-pane", "-l"],
        &["display", "-p", "#{pane_index}"],
        &["last-pane"],
        &["display", "-p", "#{pane_index}"],
        &["select-pane", "-T", "my title"],
        &["display", "-p", "#{pane_title}"],
        &["select-pane", "-T", "#{session_name}-title"],
        &["display", "-p", "#{pane_title}"],
        &["select-pane", "-d", "-t", "1"],
        &["list-panes", "-F", "#{pane_index}:#{pane_input_off}"],
        &["select-pane", "-e", "-t", "1"],
        &["list-panes", "-F", "#{pane_index}:#{pane_input_off}"],
        &["select-pane", "-m", "-t", "2"],
        &["display", "-p", "#{pane_marked_set} #{pane_marked}"],
        &["list-panes", "-F", "#{pane_index}:#{pane_marked}"],
        &["select-pane", "-M"],
        &["display", "-p", "#{pane_marked_set}"],
        &["select-pane", "-P", "bg=red"],
        &["select-pane", "-g"],
        &["select-pane", "-P", "bg=notacolour"],
        &["select-pane", "-t", "0"],
        &["select-pane", "-t", "0"],
        &["display", "-p", "#{pane_index}"],
        &["new-window", "-d", "-n", "w1"],
        &["new-window", "-d", "-n", "w2"],
        &["select-window", "-t", "w2"],
        &["display", "-p", "#{window_index}"],
        &["select-window", "-l"],
        &["display", "-p", "#{window_index}"],
        &["last-window"],
        &["next-window"],
        &["display", "-p", "#{window_index}"],
        &["previous-window"],
        &["previous-window"],
        &["display", "-p", "#{window_index}"],
        &["select-window", "-n"],
        &["select-window", "-p"],
        &["select-window", "-T", "-t", "0"],
        &["display", "-p", "#{window_index}"],
        &["select-window", "-T", "-t", "w1"],
        &["display", "-p", "#{window_index}"],
        &["select-window", "-t", "99"],
        &["next-window", "-a"],
        &[
            "set-hook",
            "-g",
            "after-select-window",
            "set -g @hook selected"
        ],
        &["select-window", "-t", "0"],
        &["show-options", "-gv", "@hook"],
        &["rename-window", "renamed"],
        &["display", "-p", "#{window_name} #{automatic-rename}"],
        &["show-window-options", "automatic-rename"],
        &["rename-window", "-t", "w1", ""],
        &["rename-session", "newname"],
        &["display", "-p", "#{session_name}"],
        &["rename-session", "newname"],
        &["rename-session", "bad:name"],
        &["rename-session", "a.b"],
        &["new-session", "-d", "-s", "second"],
        &["rename-session", "second"],
        &["rename-session", "-t", "second", "#{session_name}x"],
        &["list-sessions", "-F", "#{session_name}"],
        &["switch-client", "-t", "second"],
        &["switch-client", "-n"],
        &["switch-client", "-T", "nosuchtable"],
        &["switch-client", "-O", "bad"],
    ]
);

differential_test!(
    wait_for_channels,
    &[
        &["wait-for", "-S", "chan"],
        &["wait-for", "chan"],
        &["wait-for", "-S", "chan"],
        &["wait-for", "-S", "chan"],
        &["wait-for", "-L", "lock"],
        &["wait-for", "-U", "lock"],
        &["wait-for", "-U", "lock"],
        &["wait-for", "-U", "never"],
        &["wait-for", "-l", "chan"],
        &["wait-for", "-l", "nothing"],
        &["wait-for", "-w", "ghost", "chan"],
        &["wait-for", "-E", "bad name"],
        &["wait-for", "-E", "-l", "@evt"],
        &["wait-for", "-E", "-w", "ghost", "@evt"],
        &["wait-for", "-E", "-l", "session-renamed"],
    ]
);

differential_test!(
    source_file_paths,
    &[
        &[
            "run-shell",
            "printf 'set -g @one 1\\n' > one.conf; printf 'set -g @two 2\\n' > two.conf; printf 'this is not a command\\n' > bad.conf; printf 'source-file self.conf\\n' > self.conf"
        ],
        &["source-file", "one.conf", "two.conf"],
        &["show-options", "-gv", "@one"],
        &["show-options", "-gv", "@two"],
        &["set-option", "-gu", "@one"],
        &["set-option", "-gu", "@two"],
        &["source-file", "*.conf"],
        &["source-file", "missing.conf"],
        &["source-file", "-q", "missing.conf"],
        &["source-file", "-q", "nomatch*.conf"],
        &["source-file", "nomatch*.conf"],
        &["source-file", "bad.conf"],
        &["source-file", "-n", "bad.conf"],
        &["source-file", "-n", "one.conf"],
        &["source-file", "-v", "one.conf"],
        &["source-file", "-F", "#{session_name}.conf"],
        &["run-shell", "printf 'set -g @three 3\\n' > test.conf"],
        &["source-file", "-F", "#{session_name}.conf"],
        &["show-options", "-gv", "@three"],
        &["source-file", "self.conf"],
        &["source-file", "/dev/null"],
    ]
);

differential_test!(
    run_shell_modes,
    &[
        &["run-shell"],
        &["run-shell", "echo hello"],
        &["run-shell", "printf 'a\\nb\\nc'"],
        &["run-shell", "printf 'x\\0hidden\\ny'"],
        &["run-shell", "exit 3"],
        &["run-shell", "kill -TERM $$"],
        &["run-shell", "-d", "0.1", "echo delayed"],
        &["run-shell", "-d", "abc", "echo x"],
        &["run-shell", "-d", "0.05"],
        &["run-shell", "-b", "echo background"],
        &["run-shell", "-C", "display -p from-C"],
        &["run-shell", "-C", "nosuchcommand"],
        &["run-shell", "-c", "/", "pwd"],
        &["run-shell", "echo $RMUX_PANE $TMUX_PANE | wc -w"],
        &["run-shell", "echo #{session_name} 1 2", "x", "y"],
        &["run-shell", "nosuchbinary-zz"],
        &["run-shell", "-E", "echo err >&2"],
        &["run-shell", "echo err >&2"],
    ]
);

differential_test!(
    misc_commands,
    &[
        &["unbind-key", "-a", "-T", "nosuch"],
        &["unbind-key", "nosuchkey"],
        &["unbind-key"],
        &["unbind-key", "-a", "x"],
        &["unbind-key", "-q", "nosuchkey"],
        &["unbind-key", "-T", "nosuch", "x"],
        &["bind-key", "-T", "mine", "x", "display x"],
        &["unbind-key", "-T", "mine", "x"],
        &["list-keys", "-T", "mine"],
        &["unbind-key", "C-b"],
        &["list-keys", "-T", "prefix", "C-b"],
        &["unbind-key", "-n", "MouseDown1Pane"],
        &["unbind-key", "-a"],
        &["list-keys", "-T", "prefix"],
        &["unbind-key", "-a", "-n"],
        &["list-keys", "-T", "root"],
        &["show-prompt-history"],
        &["show-prompt-history", "-T", "command"],
        &["show-prompt-history", "-T", "nosuch"],
        &["clear-prompt-history", "-T", "nosuch"],
        &["clear-prompt-history"],
        &["server-access"],
        &["server-access", "-l"],
        &["server-access", "-a", "nosuchuser-zz"],
        &["server-access", "-g", "-a", "nosuchgroup-zz"],
        &["server-access", "-a", "-d", "root"],
        &["server-access", "-r", "-w", "root"],
        &["server-access", "-a", "root"],
        &["server-access", "-d", "nobody"],
        &["refresh-client"],
        &["refresh-client", "-C", "100x30"],
        &["refresh-client", "-A", "%0:on"],
        &["refresh-client", "-S"],
        &["refresh-client", "-l"],
        &["respawn-pane", "-k", "-t", "0"],
        &["respawn-pane", "-t", "0"],
        &["respawn-window", "-k"],
        &["respawn-window"],
        &["split-window", "-d", "-I"],
        &["split-window", "-d", "-E", "cmd"],
        &["split-window", "-d", "-B", "nope"],
        &["split-window", "-d", "-l", "nope"],
        &["split-window", "-d", "-P", "-F", "#{pane_index}", "-c", "/"],
        &["split-window", "-d", "-P"],
        &[
            "new-pane",
            "-d",
            "-x",
            "20",
            "-y",
            "5",
            "-X",
            "3",
            "-Y",
            "2",
            "-P",
            "-F",
            "#{pane_x},#{pane_y} #{pane_width}x#{pane_height}"
        ],
        &["new-pane", "-d", "-O", "-L"],
        &["new-pane", "-d", "-O"],
        &["new-pane", "-d", "-O"],
        &[
            "list-panes",
            "-F",
            "#{pane_index}:#{pane_floating_flag}:#{pane_modal_flag}"
        ],
        &["pipe-pane"],
        &["pipe-pane", "-t", "nosuch", "cat"],
        &[
            "new-window",
            "-d",
            "-n",
            "pp",
            "sh -c 'sleep 0.2; echo piped; sleep 0.3'"
        ],
        &["pipe-pane", "-t", "pp", "cat > piped.txt"],
        &["run-shell", "sleep 0.8"],
        &["pipe-pane", "-t", "pp"],
        &["run-shell", "cat piped.txt"],
    ]
);
