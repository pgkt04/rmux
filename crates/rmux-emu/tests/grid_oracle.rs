// Ported from tmux grid-reader.c @ 8f25579c
//! Replays the copy-mode word motions of `regress/copy-mode-test-vi.sh`
//! (and the emacs variant) against the oracle tmux and compares the copy
//! cursor after every motion with `GridReader`.
#[path = "../../rmux-util/tests/common/mod.rs"]
mod common;

use rmux_emu::cell::{DEFAULT_CELL, GridCell};
use rmux_emu::colour::Colour;
use rmux_emu::grid::Grid;
use rmux_emu::grid::reader::{GridReader, WHITESPACE};
use rmux_util::utf8::Utf8Data;
use std::process::Command;

const SX: u32 = 40;
const SY: u32 = 10;
const SEPARATORS: &[u8] = b"!\"#$%&'()*+,-./:;<=>?@[\\]^`{|}~";

/// Load the regress text into a grid the way the input parser does for
/// plain ASCII and tabs (`input.c:1335-1358`, tab stops every 8 columns).
fn load(text: &str) -> Grid {
    let mut gd = Grid::new(SX, SY, 2000);
    for (cy, line) in text.lines().enumerate() {
        let mut cx = 0u32;
        for ch in line.chars() {
            if ch == '\t' {
                let next = ((cx / 8) + 1) * 8;
                let next = next.min(SX - 1);
                let width = next - cx;
                let mut gc = DEFAULT_CELL;
                gc.set_tab(width);
                gd.view_set_cell(cx, cy as u32, &gc);
                for px in cx + 1..cx + width {
                    gd.view_set_padding(px, cy as u32, Colour::DEFAULT);
                }
                cx = next;
            } else {
                let mut buf = [0u8; 4];
                let s = ch.encode_utf8(&mut buf);
                let mut gc = GridCell {
                    data: Utf8Data {
                        size: s.len() as u8,
                        have: s.len() as u8,
                        width: 1,
                        ..Utf8Data::default()
                    },
                    ..DEFAULT_CELL
                };
                gc.data.data[..s.len()].copy_from_slice(s.as_bytes());
                gd.view_set_cell(cx, cy as u32, &gc);
                cx += 1;
            }
        }
    }
    gd
}

/// The `window_copy_cursor_*` wrappers around the reader
/// (`window-copy.c:6341-6484, 6733-6860`), with the cursor clamp of
/// `window_copy_update_cursor`.
fn motion(gd: &Grid, cx: &mut u32, cy: &mut u32, cmd: &str, vi: bool) {
    let mut gr = GridReader::new(gd, *cx, *cy);
    match cmd {
        "next-word" => gr.cursor_next_word(SEPARATORS),
        "next-space" => gr.cursor_next_word(b""),
        "next-word-end" | "next-space-end" => {
            let sep = if cmd == "next-word-end" {
                SEPARATORS
            } else {
                b""
            };
            if vi {
                if gr.in_set(WHITESPACE) == 0 {
                    gr.cursor_right(false, false, false);
                }
                gr.cursor_next_word_end(sep);
                gr.cursor_left(true);
            } else {
                gr.cursor_next_word_end(sep);
            }
        }
        "previous-word" => gr.cursor_previous_word(SEPARATORS, true, !vi),
        "previous-space" => gr.cursor_previous_word(b"", true, !vi),
        "start-of-line" => gr.cursor_start_of_line(true),
        "end-of-line" => gr.cursor_end_of_line(true, false),
        "back-to-indentation" => gr.cursor_back_to_indentation(),
        "cursor-left" => gr.cursor_left(true),
        "cursor-right" => gr.cursor_right(true, false, !vi),
        other => panic!("unknown motion {other}"),
    }
    (*cx, *cy) = gr.cursor();
    // `window_copy_update_cursor` clamps to `window_copy_cursor_limit`
    // (`window-copy.c:5691-5696, 6316-6326`).
    let maxx = if vi {
        gd.line_limit(*cy)
    } else {
        gd.line_length(*cy)
    };
    if *cx > maxx {
        *cx = maxx;
    }
}

const MOTIONS: &[&str] = &[
    "previous-word",
    "previous-space",
    "previous-word",
    "next-word-end",
    "previous-word",
    "next-word",
    "next-word",
    "next-word-end",
    "next-word-end",
    "next-word",
    "next-word",
    "next-word",
    "next-word",
    "next-word-end",
    "next-word",
    "next-word-end",
    "next-word",
    "next-word-end",
    "next-space",
    "next-space-end",
    "next-word",
    "next-word",
    "next-word",
    "next-word",
    "next-word-end",
    "next-word",
    "next-word",
    "next-word-end",
    "next-word-end",
    "previous-word",
    "previous-space",
    "previous-space",
    "next-space",
    "next-word",
    "next-word-end",
    "previous-word",
    "previous-word",
    "previous-word",
    "previous-word",
    "end-of-line",
    "previous-word",
    "back-to-indentation",
    "cursor-right",
    "cursor-right",
    "next-word",
    "next-word",
    "next-word",
    "next-word",
    "next-word",
    "next-word",
    "cursor-left",
    "cursor-left",
    "previous-space",
    "start-of-line",
    "next-space",
    "next-space",
    "next-space",
    "next-space",
    "next-space-end",
    "next-space-end",
    "next-space-end",
    "next-space-end",
    "next-space-end",
    "next-word-end",
    "next-word-end",
    "next-word-end",
    "next-word",
    "next-word",
    "end-of-line",
    "cursor-right",
    "cursor-right",
    "previous-word",
    "previous-word",
];

struct Oracle {
    tmux: std::path::PathBuf,
    socket: String,
}

impl Oracle {
    fn start(text: &std::path::Path, mode_keys: &str) -> Option<Oracle> {
        let tmux = common::oracle()?;
        let socket = std::env::temp_dir()
            .join(format!("rmux-g03-{}-{}", std::process::id(), mode_keys))
            .to_string_lossy()
            .into_owned();
        let o = Oracle { tmux, socket };
        let shell = format!("cat '{}'; printf '\\033[9;15H'; cat", text.display());
        let ok = o.run(&[
            "new",
            "-d",
            "-x",
            &SX.to_string(),
            "-y",
            &SY.to_string(),
            &shell,
        ]);
        assert!(
            ok.status.success(),
            "{}",
            String::from_utf8_lossy(&ok.stderr)
        );
        o.run(&["set", "-g", "window-size", "manual"]);
        o.run(&["set-window-option", "-g", "mode-keys", mode_keys]);
        for _ in 0..200 {
            let out = o.run(&["capture-pane", "-p"]);
            if String::from_utf8_lossy(&out.stdout).contains("500xyz") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        o.run(&["copy-mode"]);
        o.run(&["send-keys", "-X", "history-top"]);
        o.run(&["send-keys", "-X", "start-of-line"]);
        Some(o)
    }
    fn run(&self, args: &[&str]) -> std::process::Output {
        Command::new(&self.tmux)
            .args(["-f", "/dev/null", "-S", &self.socket])
            .args(args)
            .output()
            .expect("run oracle")
    }
    fn cursor(&self) -> (u32, u32) {
        let out = self.run(&["display", "-p", "#{copy_cursor_x} #{copy_cursor_y}"]);
        let s = String::from_utf8_lossy(&out.stdout);
        let mut it = s.split_whitespace().map(|v| v.parse::<u32>().unwrap());
        (it.next().unwrap(), it.next().unwrap())
    }
}

impl Drop for Oracle {
    fn drop(&mut self) {
        let _ = self.run(&["kill-server"]);
    }
}

fn replay(mode_keys: &str) {
    let Some(src) = common::pinned_source() else {
        eprintln!("oracle replay skipped: pinned tmux checkout missing");
        return;
    };
    let text_path = src.join("regress/copy-mode-test.txt");
    let text = std::fs::read_to_string(&text_path).unwrap();
    let Some(oracle) = Oracle::start(&text_path, mode_keys) else {
        eprintln!("oracle replay skipped: oracle tmux missing");
        return;
    };
    let gd = load(&text);
    let (mut cx, mut cy) = (0, 0);
    assert_eq!(oracle.cursor(), (cx, cy));
    let vi = mode_keys == "vi";
    for (i, cmd) in MOTIONS.iter().enumerate() {
        oracle.run(&["send-keys", "-X", cmd]);
        motion(&gd, &mut cx, &mut cy, cmd, vi);
        let expected = oracle.cursor();
        assert_eq!((cx, cy), expected, "motion {i} ({cmd}) in {mode_keys} mode");
    }
}

#[test]
fn copy_mode_motions_match_oracle_vi() {
    replay("vi");
}

#[test]
fn copy_mode_motions_match_oracle_emacs() {
    replay("emacs");
}
