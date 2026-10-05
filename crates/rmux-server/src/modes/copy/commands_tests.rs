// Ported from tmux window-copy.c @ 8f25579c (tests for the command table,
// dispatch transaction and copy effects)
use super::*;
use crate::cmd::arguments::ArgsValue;
use crate::ids::WindowId;
use crate::model::paste::{paste_buffer_data, paste_buffer_name, paste_get_top};
use crate::model::session::{SessionCreate, session_attach, session_create};
use crate::modes::copy::state::CopyModeData;
use crate::modes::copy::{CopyModeDriver, CopyModeKind, keys};
use rmux_emu::cell::DEFAULT_CELL;
use rmux_emu::colour::Colour;
use rmux_emu::screen::write::{ScreenOnlySink, ScreenWriteCtx, ScreenWritePolicy};
use rmux_util::bytes::ByteString;
use std::path::Path;

const PINNED_TABLE: &str = include_str!("testdata/window_copy_cmd_table.tsv");

/// `(name, template, lower, upper, read-only, clear)` of one C table row.
type CRow = (Vec<u8>, Vec<u8>, i32, i32, bool, CopyMarkClear);

/// The pinned C rows, from the live pinned checkout when present and from the
/// durable extraction otherwise (or always when `pinned_only`).
fn c_rows(pinned_only: bool) -> Vec<CRow> {
    let live = (!pinned_only && Path::new("/Users/j/fun/tmux").is_dir())
        .then(|| {
            std::process::Command::new("git")
                .args(["-C", "/Users/j/fun/tmux", "show", "8f25579c:window-copy.c"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| o.stdout)
        })
        .flatten();
    if let Some(source) = live {
        let text = String::from_utf8_lossy(&source);
        let lines: Vec<&str> = text.lines().collect();
        let table = lines[3299..3891].join("\n");
        let mut rows = Vec::new();
        for entry in table.split("{ .command").skip(1) {
            let field = |key: &str| {
                let start = entry.find(key)? + key.len();
                let rest = &entry[start..];
                Some(rest[..rest.find(',').unwrap_or(rest.len())].trim())
            };
            let Some(command) = field(" = \"") else {
                continue;
            };
            let command = command.trim_end_matches('"');
            let args = field(".args = { \"").expect("args rule");
            let template = args.trim_end_matches('"');
            let after = &entry[entry.find(".args = {").unwrap()..];
            let parts: Vec<&str> = after.split(',').collect();
            let lower: i32 = parts[1].trim().parse().expect("lower bound");
            let upper: i32 = parts[2].trim().parse().expect("upper bound");
            let read_only = entry.contains("WINDOW_COPY_CMD_FLAG_READONLY");
            let clear = match field(".clear = ").expect("clear policy") {
                "WINDOW_COPY_CMD_CLEAR_NEVER" => CopyMarkClear::Never,
                "WINDOW_COPY_CMD_CLEAR_EMACS_ONLY" => CopyMarkClear::EmacsOnly,
                _ => CopyMarkClear::Always,
            };
            rows.push((
                command.as_bytes().to_vec(),
                template.as_bytes().to_vec(),
                lower,
                upper,
                read_only,
                clear,
            ));
        }
        return rows;
    }
    PINNED_TABLE
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(|line| {
            let f: Vec<&str> = line.split('\t').collect();
            let clear = match f[5] {
                "NEVER" => CopyMarkClear::Never,
                "EMACS_ONLY" => CopyMarkClear::EmacsOnly,
                _ => CopyMarkClear::Always,
            };
            (
                f[0].as_bytes().to_vec(),
                f[1].as_bytes().to_vec(),
                f[2].parse().unwrap(),
                f[3].parse().unwrap(),
                f[4] == "1",
                clear,
            )
        })
        .collect()
}

fn check_table(rows: &[CRow]) {
    assert_eq!(rows.len(), 99);
    assert_eq!(COMMAND_TABLE.len(), rows.len());
    for (spec, (name, template, lower, upper, read_only, clear)) in COMMAND_TABLE.iter().zip(rows) {
        let label = String::from_utf8_lossy(name);
        assert_eq!(spec.name, name.as_slice(), "order of {label}");
        assert_eq!(
            spec.args.template,
            template.as_slice(),
            "template of {label}"
        );
        assert_eq!(
            (spec.args.lower, spec.args.upper),
            (*lower, *upper),
            "range of {label}"
        );
        assert_eq!(spec.read_only, *read_only, "read-only of {label}");
        assert_eq!(spec.clear, *clear, "clear policy of {label}");
        assert!(spec.args.cb.is_none(), "callback of {label}");
    }
}

/// Against the pinned checkout when it exists, else the durable extraction.
#[test]
fn table_matches_pinned_window_copy_c_rows() {
    check_table(&c_rows(false));
}

/// Always against the durable extraction, so both reference paths run here.
#[test]
fn table_matches_durable_c_extraction() {
    let rows = c_rows(true);
    check_table(&rows);
    if Path::new("/Users/j/fun/tmux").is_dir() {
        assert_eq!(
            rows,
            c_rows(false),
            "the durable extraction tracks the pinned source"
        );
    }
}

#[test]
fn unknown_name_is_absent() {
    assert!(lookup(b"search").is_none());
    assert!(lookup(b"").is_none());
    assert!(lookup(b"copy-pipe").is_some());
}

struct Fixture {
    server: Server,
    mode: ModeId,
    session: SessionId,
    window: WindowId,
}

/// A pane whose base screen received `lines` (each followed by CRLF), in
/// copy mode with the given key mode.
fn fixture(lines: &[&str], width: u32, height: u32, hlimit: u32, vi: bool) -> Fixture {
    let mut server = Server::default();
    let options = server.options.create(Some(server.options.global_s));
    let session = session_create(
        &mut server,
        SessionCreate {
            prefix: None,
            name: Some(b"copy".to_vec()),
            cwd: b"/tmp".to_vec(),
            environment: Default::default(),
            options,
            termios: None,
        },
    );
    let window = crate::model::window::window_create(&mut server, width, height, 0, 0).unwrap();
    session_attach(&mut server, session, window, 0).unwrap();
    let pane = crate::model::pane::pane_create(&mut server, window, width, height, hlimit).unwrap();
    {
        let p = server.panes.get_mut(pane).unwrap();
        let mut sink = ScreenOnlySink;
        let mut ctx = ScreenWriteCtx::start(
            &mut p.base,
            &mut sink,
            ScreenWritePolicy::default(),
            &mut server.hyperlinks,
            #[cfg(feature = "sixel")]
            None,
        );
        for line in lines {
            ctx.puts(&DEFAULT_CELL, line.as_bytes());
            ctx.carriagereturn();
            ctx.linefeed(false, Colour::DEFAULT);
        }
        ctx.finish();
    }
    if vi {
        let wo = server.windows.get(window).unwrap().options;
        let mut store = std::mem::take(&mut server.options);
        store.set_number(wo, b"mode-keys", 1, &mut server);
        server.options = store;
    }
    let driver = std::rc::Rc::new(CopyModeDriver {
        kind: CopyModeKind::Copy {
            source: None,
            args: Args::default(),
        },
    });
    let mode = crate::model::pane::pane_set_mode(
        &mut server,
        pane,
        b"copy-mode",
        crate::modes::WindowModeFlags::default(),
        driver,
        false,
    )
    .unwrap()
    .unwrap();
    Fixture {
        server,
        mode,
        session,
        window,
    }
}

impl Fixture {
    fn args(argv: &[&str]) -> Args {
        let mut args = Args::create();
        for a in argv {
            args.values
                .push(ArgsValue::string(ByteString::from(a.as_bytes())));
        }
        args
    }
    fn run(&mut self, argv: &[&str]) {
        let args = Self::args(argv);
        keys::command(
            &mut self.server,
            self.mode,
            None,
            Some(self.session),
            None,
            &args,
            None,
        );
    }
    fn run_n(&mut self, n: u32, argv: &[&str]) {
        set_prefix(&mut self.server, self.mode, n);
        self.run(argv);
    }
    fn data(&self) -> &CopyModeData {
        state::data(&self.server, self.mode).expect("copy mode data")
    }
    fn cursor(&self) -> (u32, u32, u32) {
        let d = self.data();
        (d.cx, d.cy, d.oy)
    }
    fn in_mode(&self) -> bool {
        self.server
            .panes
            .get(self.mode.owner)
            .is_some_and(|p| p.modes.first().is_some_and(|m| m.id == self.mode))
    }
    fn top_buffer(&self) -> Option<(Vec<u8>, Vec<u8>)> {
        let id = paste_get_top(&self.server)?;
        Some((
            paste_buffer_name(&self.server, id)?.to_vec(),
            paste_buffer_data(&self.server, id)?.to_vec(),
        ))
    }
}

#[test]
fn entry_places_cursor_at_source_cursor() {
    let f = fixture(&["hello world", "foo"], 20, 4, 50, false);
    assert_eq!(f.cursor(), (0, 2, 0));
    assert_eq!(f.data().backing.screen().grid.hsize(), 0);
}

#[test]
fn horizontal_and_word_motion_emacs() {
    let mut f = fixture(&["hello world", "foo"], 20, 4, 50, false);
    f.run(&["history-top"]);
    assert_eq!(f.cursor(), (0, 0, 0));
    f.run(&["end-of-line"]);
    assert_eq!(f.cursor().0, 11, "emacs end of line is one past the text");
    f.run(&["cursor-left"]);
    assert_eq!(f.cursor().0, 10);
    f.run(&["start-of-line"]);
    assert_eq!(f.cursor().0, 0);
    f.run(&["next-word"]);
    assert_eq!(f.cursor().0, 6);
    f.run(&["next-word-end"]);
    assert_eq!(f.cursor().0, 11);
    f.run(&["previous-word"]);
    assert_eq!(f.cursor().0, 6);
    f.run_n(2, &["cursor-left"]);
    assert_eq!(f.cursor().0, 4);
    f.run(&["cursor-down"]);
    assert_eq!(f.cursor(), (3, 1, 0), "shorter row clamps to its end");
    f.run(&["cursor-up"]);
    assert_eq!(f.cursor(), (4, 0, 0), "the preferred column is restored");
    f.run(&["back-to-indentation"]);
    assert_eq!(f.cursor().0, 0);
}

#[test]
fn vi_limits_differ_from_emacs() {
    let mut f = fixture(&["hello world", "foo"], 20, 4, 50, true);
    f.run(&["history-top"]);
    f.run(&["end-of-line"]);
    assert_eq!(f.cursor().0, 10, "vi stops on the last character");
    f.run(&["start-of-line"]);
    f.run(&["next-word-end"]);
    assert_eq!(f.cursor().0, 4, "vi word end lands on the last letter");
    f.run_n(6, &["cursor-right"]);
    assert_eq!(f.cursor().0, 10);
    f.run(&["cursor-right"]);
    assert_eq!(
        f.cursor(),
        (0, 1, 0),
        "right motion wraps onto the next row"
    );
    assert_eq!(keys::key_table(&f.server, f.mode), b"copy-mode-vi");
}

#[test]
fn top_middle_bottom_line_and_centre() {
    let mut f = fixture(&["a", "b", "c", "d"], 10, 5, 50, false);
    f.run(&["top-line"]);
    assert_eq!(f.cursor(), (0, 0, 0));
    f.run(&["bottom-line"]);
    assert_eq!(f.cursor(), (0, 4, 0));
    f.run(&["middle-line"]);
    assert_eq!(f.cursor(), (0, 2, 0));
    f.run(&["cursor-centre-vertical"]);
    assert_eq!(f.cursor().1, 2);
    f.run(&["cursor-centre-horizontal"]);
    assert_eq!(f.cursor().0, 1, "cursor clamps to the row text length");
}

#[test]
fn page_and_history_motion() {
    let lines: Vec<String> = (0..30).map(|i| format!("line {i}")).collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let mut f = fixture(&refs, 20, 5, 50, false);
    assert_eq!(f.data().backing.screen().grid.hsize(), 26);
    assert_eq!(f.cursor(), (0, 4, 0));
    f.run(&["page-up"]);
    // lastsx starts at zero, so column zero of an empty row snaps to the end
    // of the destination row, as in C.
    assert_eq!(f.cursor(), (7, 4, 3), "a full page is V - 2 rows");
    f.run(&["halfpage-up"]);
    assert_eq!(f.cursor(), (7, 4, 5), "a half page is V / 2 rows");
    f.run(&["history-top"]);
    assert_eq!(f.cursor(), (0, 0, 26));
    f.run(&["page-down"]);
    assert_eq!(f.cursor(), (0, 0, 23));
    f.run(&["halfpage-down"]);
    assert_eq!(f.cursor(), (0, 0, 21));
    f.run_n(3, &["scroll-down"]);
    assert_eq!(
        f.cursor(),
        (0, 0, 18),
        "scroll-down moves only the viewport in emacs"
    );
    f.run_n(2, &["scroll-up"]);
    assert_eq!(f.cursor(), (0, 0, 20));
    f.run(&["history-bottom"]);
    assert_eq!(f.cursor(), (0, 4, 0));
    f.run(&["scroll-down"]);
    assert!(
        f.in_mode(),
        "scroll-down at the bottom without scroll-exit does nothing"
    );
    f.run(&["scroll-up"]);
    assert_eq!(f.cursor().2, 1);
    f.run(&["scroll-exit-on"]);
    assert!(f.data().scroll_exit);
    f.run(&["scroll-down"]);
    assert!(
        !f.in_mode(),
        "reaching the bottom with scroll-exit leaves the mode"
    );
}

#[test]
fn vi_scroll_moves_the_cursor_first() {
    let lines: Vec<String> = (0..30).map(|i| format!("line {i}")).collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let mut f = fixture(&refs, 20, 5, 50, true);
    f.run(&["top-line"]);
    f.run(&["scroll-up"]);
    assert_eq!(
        f.cursor(),
        (0, 1, 1),
        "vi first moves the cursor down, then scrolls"
    );
    f.run_n(3, &["scroll-down"]);
    assert_eq!(f.cursor(), (0, 0, 0));
}

/// regress/copy-mode-scroll-exit.sh: `send-keys -N200 -X scroll-down` with a
/// selection scrolls the view by more rows than the screen has.
#[test]
fn view_only_scroll_larger_than_the_screen_does_not_panic() {
    let lines: Vec<String> = (0..30).map(|i| format!("line {i}")).collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    for vi in [false, true] {
        let mut f = fixture(&refs, 20, 5, 50, vi);
        f.run(&["history-top"]);
        f.run(&["start-of-line"]);
        f.run(&["begin-selection"]);
        f.run(&["cursor-down"]);
        f.run_n(200, &["scroll-down"]);
        assert!(f.in_mode());
        assert_eq!(f.cursor().2, 0, "the view reaches the bottom");
        assert!(f.data().selection.active);
        assert_eq!(f.data().selection.cursordrag, CursorDrag::None);
        f.run_n(200, &["scroll-up"]);
        // window_copy_scroll_down returns when ny exceeds the history size.
        assert_eq!(f.cursor().2, 0, "a count above the history does nothing");
        f.run_n(26, &["scroll-up"]);
        assert_eq!(f.cursor().2, 26, "the view reaches the history top");
        assert!(f.in_mode());
    }
}

#[test]
fn scroll_down_and_cancel_and_cursor_down_and_cancel() {
    let lines: Vec<String> = (0..30).map(|i| format!("line {i}")).collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let mut f = fixture(&refs, 20, 5, 50, false);
    f.run(&["page-up"]);
    f.run_n(3, &["scroll-down-and-cancel"]);
    assert!(!f.in_mode(), "scroll-down-and-cancel exits at oy == 0");
    let mut f = fixture(&refs, 20, 5, 50, false);
    f.run(&["cursor-down-and-cancel"]);
    assert!(
        !f.in_mode(),
        "cursor-down on the last row at the bottom cancels"
    );
    let mut f = fixture(&refs, 20, 5, 50, false);
    f.run(&["history-top"]);
    f.run(&["cursor-down-and-cancel"]);
    assert!(f.in_mode());
    assert_eq!(f.cursor().1, 1);
}

#[test]
fn selection_blocks_scroll_exit_until_cleared() {
    let lines: Vec<String> = (0..30).map(|i| format!("line {i}")).collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let mut f = fixture(&refs, 20, 5, 50, false);
    f.run(&["scroll-exit-on"]);
    f.run(&["page-up"]);
    f.run(&["begin-selection"]);
    f.run_n(5, &["scroll-down"]);
    assert!(f.in_mode(), "a selection prevents scroll-exit");
    assert_eq!(
        f.data().selection.cursordrag,
        CursorDrag::None,
        "view scroll freezes the endpoint"
    );
    assert_eq!(f.cursor().2, 0);
    f.run(&["clear-selection"]);
    f.run(&["scroll-down"]);
    assert!(!f.in_mode());
}

#[test]
fn copy_selection_creates_a_buffer_and_clears() {
    let mut f = fixture(&["abcdef"], 10, 2, 50, false);
    f.run(&["history-top"]);
    f.run(&["begin-selection"]);
    assert!(f.data().selection.active);
    f.run_n(3, &["cursor-right"]);
    f.run(&["copy-selection"]);
    assert_eq!(f.top_buffer(), Some((b"buffer0".to_vec(), b"abc".to_vec())));
    assert!(
        !f.data().selection.active,
        "copy-selection clears the selection"
    );
    assert!(f.in_mode());
    f.run(&["begin-selection"]);
    f.run_n(2, &["cursor-right"]);
    f.run(&["copy-selection-no-clear", "pfx"]);
    assert!(
        f.data().selection.active,
        "the no-clear form keeps the selection"
    );
    let (name, data) = f.top_buffer().unwrap();
    assert!(
        name.starts_with(b"pfx"),
        "{}",
        String::from_utf8_lossy(&name)
    );
    assert_eq!(data, b"de");
    f.run(&["copy-selection-and-cancel", "-P"]);
    assert!(!f.in_mode(), "the and-cancel form leaves the mode");
    assert_eq!(
        f.top_buffer().unwrap().1,
        b"de",
        "-P suppresses the paste buffer"
    );
}

#[test]
fn vi_selection_includes_both_ends() {
    let mut f = fixture(&["abcdef"], 10, 2, 50, true);
    f.run(&["history-top"]);
    f.run(&["begin-selection"]);
    f.run_n(3, &["cursor-right"]);
    f.run(&["copy-selection"]);
    assert_eq!(f.top_buffer().unwrap().1, b"abcd");
}

#[test]
fn append_selection_prefixes_the_top_buffer() {
    let mut f = fixture(&["abcdef"], 10, 2, 50, false);
    f.run(&["history-top"]);
    f.run(&["begin-selection"]);
    f.run_n(3, &["cursor-right"]);
    f.run(&["append-selection"]);
    assert_eq!(
        f.top_buffer(),
        Some((b"buffer0".to_vec(), b"abc".to_vec())),
        "no top buffer: a new one"
    );
    f.run(&["begin-selection"]);
    f.run_n(3, &["cursor-right"]);
    f.run(&["append-selection-and-cancel"]);
    let id = crate::model::paste::paste_get_name(&f.server, b"buffer0").unwrap();
    assert_eq!(paste_buffer_data(&f.server, id), Some(&b"abcdef"[..]));
    assert_eq!(
        f.top_buffer(),
        None,
        "paste_set makes the named buffer non-automatic"
    );
    assert!(!f.in_mode());
}

#[test]
fn copy_line_forms_restore_the_cursor() {
    let mut f = fixture(&["abcdef", "ghi"], 10, 3, 50, false);
    f.run(&["history-top"]);
    f.run_n(2, &["cursor-right"]);
    f.run(&["copy-line"]);
    assert_eq!(f.top_buffer().unwrap().1, b"abcdef");
    assert_eq!(f.cursor(), (2, 0, 0));
    assert!(!f.data().selection.active);
    f.run(&["copy-end-of-line"]);
    assert_eq!(f.top_buffer().unwrap().1, b"cdef");
    assert_eq!(f.cursor(), (2, 0, 0));
    f.run_n(2, &["copy-line"]);
    assert_eq!(
        f.top_buffer().unwrap().1,
        b"abcdef\nghi",
        "the repeat count adds rows"
    );
    f.run(&["copy-line-and-cancel"]);
    assert!(!f.in_mode());
}

#[test]
fn copy_pipe_without_a_command_still_copies() {
    let mut f = fixture(&["abcdef"], 10, 2, 50, false);
    f.run(&["history-top"]);
    f.run(&["begin-selection"]);
    f.run_n(3, &["cursor-right"]);
    f.run(&["copy-pipe", "", "named"]);
    let (name, data) = f.top_buffer().unwrap();
    assert!(name.starts_with(b"named"));
    assert_eq!(data, b"abc");
    assert!(!f.data().selection.active);
    f.run(&["begin-selection"]);
    f.run(&["cursor-right"]);
    f.run(&["pipe-and-cancel"]);
    assert!(!f.in_mode());
    assert_eq!(
        f.top_buffer().unwrap().1,
        b"abc",
        "pipe alone creates no buffer"
    );
}

#[test]
fn jump_commands_store_and_reuse_the_character() {
    let mut f = fixture(&["a-b-c-d"], 10, 2, 50, false);
    f.run(&["history-top"]);
    f.run(&["jump-forward", "-"]);
    assert_eq!(f.cursor().0, 1);
    assert_eq!(f.data().jump.kind, JumpKind::Forward);
    assert_eq!(f.data().jump.character, b"-");
    f.run(&["jump-again"]);
    assert_eq!(f.cursor().0, 3);
    f.run(&["jump-reverse"]);
    assert_eq!(f.cursor().0, 1);
    assert_eq!(
        f.data().jump.kind,
        JumpKind::Forward,
        "reverse keeps the stored type"
    );
    f.run(&["jump-to-forward", "d"]);
    assert_eq!(f.cursor().0, 5);
    assert_eq!(f.data().jump.kind, JumpKind::ToForward);
    f.run(&["jump-backward", ""]);
    assert_eq!(
        f.data().jump.character,
        b"d",
        "an empty argument does not replace the sequence"
    );
    f.run(&["jump-to-backward", "a"]);
    assert_eq!(f.cursor().0, 1);
}

#[test]
fn selection_mode_values() {
    let mut f = fixture(&["abc def", "ghi"], 10, 3, 50, false);
    f.run(&["selection-mode", "W"]);
    assert_eq!(f.data().selection.selflag, SelectionMode::Word);
    assert!(!f.data().selection.separators.is_empty());
    f.run(&["selection-mode", "bogus"]);
    assert_eq!(
        f.data().selection.selflag,
        SelectionMode::Word,
        "unknown values leave it alone"
    );
    f.run(&["selection-mode"]);
    assert_eq!(f.data().selection.selflag, SelectionMode::Char);
    f.run(&["selection-mode", "l"]);
    assert_eq!(f.data().selection.selflag, SelectionMode::Line);
    assert!(!f.data().selection.active);
    f.run(&["history-top"]);
    f.run_n(2, &["cursor-right"]);
    f.run(&["begin-selection"]);
    f.run(&["cursor-down"]);
    f.run(&["selection-mode", "line"]);
    let d = f.data();
    assert_eq!(d.selection.lineflag, LineSelectionDirection::LeftToRight);
    assert_eq!(
        (d.selection.selx, d.selection.sely),
        (0, 0),
        "start expands to the line start"
    );
    assert_eq!(
        (d.selection.endselx, d.selection.endsely),
        (3, 1),
        "end expands to the line end"
    );
    assert_eq!(d.selection.cursordrag, CursorDrag::End);
    assert!(!d.selection.rectflag);
}

#[test]
fn select_line_and_select_word() {
    let mut f = fixture(&["abc def", "ghi"], 10, 3, 50, false);
    f.run(&["history-top"]);
    f.run_n(5, &["cursor-right"]);
    f.run(&["select-word"]);
    let d = f.data();
    assert_eq!(d.selection.selflag, SelectionMode::Word);
    assert_eq!((d.selection.selx, d.selection.sely), (4, 0));
    assert_eq!(f.cursor().0, 7, "emacs word end sits after the word");
    f.run(&["copy-selection"]);
    assert_eq!(f.top_buffer().unwrap().1, b"def");
    f.run(&["history-top"]);
    f.run(&["select-line"]);
    let d = f.data();
    assert_eq!(d.selection.selflag, SelectionMode::Line);
    assert_eq!((d.selection.selrx, d.selection.selry), (0, 0));
    assert_eq!((d.selection.endselrx, d.selection.endselry), (7, 0));
    f.run(&["copy-selection"]);
    assert_eq!(f.top_buffer().unwrap().1, b"abc def");
}

#[test]
fn other_end_uses_repeat_parity() {
    let mut f = fixture(&["abcdef"], 10, 2, 50, false);
    f.run(&["history-top"]);
    f.run(&["begin-selection"]);
    f.run_n(3, &["cursor-right"]);
    f.run_n(2, &["other-end"]);
    assert_eq!(f.cursor().0, 3, "an even count changes nothing");
    f.run(&["other-end"]);
    assert_eq!(f.cursor().0, 0);
    assert_eq!(f.data().selection.cursordrag, CursorDrag::Start);
    f.run(&["stop-selection"]);
    assert_eq!(f.data().selection.cursordrag, CursorDrag::None);
    assert!(
        f.data().selection.active,
        "stop-selection keeps the screen selection"
    );
}

#[test]
fn marks_toggles_and_rectangle() {
    let mut f = fixture(&["abcdef", "gh"], 10, 3, 50, false);
    f.run(&["history-top"]);
    f.run_n(4, &["cursor-right"]);
    f.run(&["set-mark"]);
    let d = f.data();
    assert!(d.showmark);
    assert_eq!((d.mx, d.my), (4, 0));
    f.run(&["toggle-position"]);
    assert!(f.data().hide_position);
    f.run(&["rectangle-toggle"]);
    assert!(f.data().selection.rectflag);
    f.run(&["rectangle-off"]);
    assert!(!f.data().selection.rectflag);
    f.run(&["rectangle-on"]);
    assert!(f.data().selection.rectflag);
    f.run(&["scroll-exit-toggle"]);
    assert!(f.data().scroll_exit);
    f.run(&["scroll-exit-off"]);
    assert!(!f.data().scroll_exit);
    f.run(&["begin-selection"]);
    f.run(&["cursor-down"]);
    assert_eq!(
        f.cursor(),
        (4, 1, 0),
        "a rectangle keeps the virtual column"
    );
    f.run(&["clear-selection"]);
    f.run(&["cursor-up"]);
    f.run(&["cursor-down"]);
    assert_eq!(
        f.cursor(),
        (2, 1, 0),
        "without a selection the row length clamps"
    );
}

#[test]
fn paragraph_and_bracket_motion() {
    let mut f = fixture(&["one", "two", "", "three", "(a [b] c)"], 20, 6, 50, false);
    f.run(&["history-top"]);
    f.run(&["next-paragraph"]);
    assert_eq!(f.cursor(), (0, 2, 0));
    f.run(&["next-paragraph"]);
    assert_eq!(
        f.cursor(),
        (0, 5, 0),
        "the last paragraph ends at the final row"
    );
    f.run(&["previous-paragraph"]);
    assert_eq!(f.cursor(), (0, 2, 0));
    f.run(&["cursor-down"]);
    f.run(&["cursor-down"]);
    // Moving down from an empty row snaps to the end of the destination row.
    assert_eq!(f.cursor(), (9, 4, 0));
    f.run(&["start-of-line"]);
    f.run(&["next-matching-bracket"]);
    assert_eq!(f.cursor().0, 8, "( matches the final )");
    f.run(&["previous-matching-bracket"]);
    assert_eq!(f.cursor().0, 0);
    f.run_n(3, &["cursor-right"]);
    f.run(&["next-matching-bracket"]);
    assert_eq!(f.cursor().0, 5);
}

#[test]
fn recentre_cycles_middle_top_bottom() {
    let lines: Vec<String> = (0..30).map(|i| format!("line {i}")).collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let mut f = fixture(&refs, 20, 5, 50, false);
    f.run(&["history-top"]);
    f.run_n(2, &["cursor-down"]);
    assert_eq!(f.cursor(), (0, 2, 26));
    f.run(&["recentre-top-bottom"]);
    assert_eq!(f.cursor(), (0, 2, 26), "already in the middle");
    assert_eq!(f.data().recentre_state, RecentreState::Top);
    f.run(&["recentre-top-bottom"]);
    assert_eq!(f.cursor(), (0, 0, 24));
    f.run(&["recentre-top-bottom"]);
    assert_eq!(
        f.cursor(),
        (0, 2, 26),
        "bottom is clamped at the history top"
    );
    assert_eq!(f.data().recentre_state, RecentreState::Middle);
}

#[test]
fn scroll_top_middle_bottom_keep_the_backing_row() {
    let lines: Vec<String> = (0..30).map(|i| format!("line {i}")).collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let mut f = fixture(&refs, 20, 5, 50, false);
    f.run(&["page-up"]);
    f.run(&["middle-line"]);
    let before = f.data().backing_y();
    f.run(&["scroll-top"]);
    assert_eq!(f.cursor().1, 0);
    assert_eq!(f.data().backing_y(), before);
    f.run(&["scroll-bottom"]);
    assert_eq!(f.cursor().1, 4);
    assert_eq!(f.data().backing_y(), before);
    f.run(&["scroll-middle"]);
    assert_eq!(f.cursor().1, 2);
    assert_eq!(f.data().backing_y(), before);
}

#[test]
fn line_number_commands_return_nothing_but_change_state() {
    let mut f = fixture(&["abc"], 10, 2, 50, false);
    assert!(!render::line_numbers_active(&f.server, f.mode));
    f.run(&["line-numbers-on"]);
    assert!(render::line_numbers_active(&f.server, f.mode));
    f.run(&["line-numbers-toggle"]);
    assert!(!render::line_numbers_active(&f.server, f.mode));
    f.run(&["line-numbers-toggle"]);
    f.run(&["line-numbers-off"]);
    assert!(!render::line_numbers_active(&f.server, f.mode));
}

#[test]
fn refresh_flags_only_apply_to_own_pane_copy_mode() {
    let mut f = fixture(&["abc"], 10, 2, 50, false);
    f.run(&["refresh-on"]);
    assert!(f.data().refresh_active);
    assert!(f.data().refresh_timer.is_some());
    f.run(&["refresh-toggle"]);
    assert!(!f.data().refresh_active);
    assert!(f.data().refresh_timer.is_none());
    f.run(&["refresh-now"]);
    assert!(f.in_mode());
}

#[test]
fn dispatch_resets_prefix_only_after_a_dispatch() {
    let mut f = fixture(&["abc"], 10, 2, 50, false);
    f.run_n(5, &[]);
    assert_eq!(
        prefix(&f.server, f.mode),
        5,
        "no positional command returns early"
    );
    f.run(&["unknown-command"]);
    assert_eq!(
        prefix(&f.server, f.mode),
        1,
        "unknown commands still reset the prefix"
    );
    f.run_n(4, &["cancel", "extra"]);
    assert!(f.in_mode(), "a parse failure runs nothing");
    assert_eq!(prefix(&f.server, f.mode), 1);
    f.run(&["cursor-right", "-z"]);
    assert_eq!(f.cursor().0, 0, "an unknown flag runs nothing");
    f.run(&["goto-line"]);
    assert!(f.in_mode());
    f.run(&["cancel"]);
    assert!(!f.in_mode());
    assert_eq!(f.window, f.window);
}

#[test]
fn read_only_clients_are_rejected_for_writable_commands() {
    let mut f = fixture(&["abcdef"], 10, 2, 50, false);
    let mut client = crate::client::Client::new(None, (0, 0));
    client.flags.insert(crate::client::ClientFlags::READONLY);
    client.session = Some(f.session);
    let client = f.server.clients.insert(client).unwrap();
    f.run(&["history-top"]);
    set_prefix(&mut f.server, f.mode, 3);
    let args = Fixture::args(&["begin-selection"]);
    keys::command(
        &mut f.server,
        f.mode,
        Some(client),
        Some(f.session),
        None,
        &args,
        None,
    );
    assert!(!f.data().selection.active, "a writable command is rejected");
    assert_eq!(
        prefix(&f.server, f.mode),
        3,
        "rejection returns before the prefix reset"
    );
    let args = Fixture::args(&["cursor-right"]);
    keys::command(
        &mut f.server,
        f.mode,
        Some(client),
        Some(f.session),
        None,
        &args,
        None,
    );
    assert_eq!(f.cursor().0, 3, "a read-only command runs with the prefix");
    assert_eq!(prefix(&f.server, f.mode), 1);
}

#[test]
fn search_commands_use_stored_term_and_clear_marks_policy() {
    let mut f = fixture(&["abc abc", "xyz abc"], 10, 3, 50, false);
    f.run(&["history-top"]);
    f.run(&["search-forward", "abc"]);
    assert_eq!(f.data().search.term.as_deref(), Some(&b"abc"[..]));
    assert_eq!(f.data().search.searchtype, SearchDirection::Down);
    assert!(f.data().search.regex);
    assert!(f.data().search.marks.is_some());
    assert_eq!(
        f.cursor(),
        (3, 0, 0),
        "emacs leaves the cursor after the match"
    );
    f.run(&["search-forward", ""]);
    assert_eq!(f.cursor(), (3, 0, 0), "an empty argument does not search");
    f.run(&["search-again"]);
    assert_eq!(f.cursor(), (7, 0, 0));
    f.run(&["search-reverse"]);
    assert_eq!(
        f.cursor(),
        (4, 0, 0),
        "backward search lands on the match start"
    );
    assert_eq!(
        f.data().search.searchtype,
        SearchDirection::Down,
        "reverse keeps the type"
    );
    f.run(&["cursor-left"]);
    assert!(
        f.data().search.marks.is_none(),
        "emacs clears marks on an EmacsOnly command"
    );
    f.run(&["search-backward-text", "xyz"]);
    assert_eq!(f.cursor(), (0, 1, 0));
    assert!(!f.data().search.regex);
    f.run(&["toggle-position"]);
    assert!(
        f.data().search.marks.is_some(),
        "Never-clear commands keep marks"
    );
    f.run(&["cancel"]);
}

#[test]
fn vi_keeps_marks_on_emacs_only_commands() {
    let mut f = fixture(&["abc abc", "xyz abc"], 10, 3, 50, true);
    f.run(&["history-top"]);
    f.run(&["search-forward", "abc"]);
    assert_eq!(
        f.cursor(),
        (4, 0, 0),
        "vi leaves the cursor at the match start"
    );
    f.run(&["cursor-left"]);
    assert!(f.data().search.marks.is_some());
    f.run(&["begin-selection"]);
    assert!(
        f.data().search.marks.is_none(),
        "Always-clear commands clear marks in vi too"
    );
}

#[test]
fn incremental_search_restores_the_origin_on_a_changed_term() {
    let mut f = fixture(&["abc abc", "xyz abc"], 10, 3, 50, false);
    f.run(&["history-top"]);
    f.run_n(2, &["cursor-right"]);
    f.run(&["search-forward-incremental", "=x"]);
    assert_eq!((f.data().search.x, f.data().search.y), (Some(2), Some(0)));
    assert_eq!(f.cursor(), (1, 1, 0));
    f.run(&["search-forward-incremental", "=xy"]);
    assert_eq!(f.cursor(), (2, 1, 0));
    f.run(&["search-forward-incremental", "-a"]);
    // "-a" parses as an unknown flag, so the handler never runs (as in C).
    assert_eq!(f.data().search.searchtype, SearchDirection::Down);
    assert_eq!(f.cursor(), (2, 1, 0));
    f.run(&["search-backward-incremental", "=a"]);
    assert_eq!(f.data().search.searchtype, SearchDirection::Up);
    // C restores the origin row, then replaces cx with the cursor limit (the
    // row end, 7), so the backward search starts from there.
    assert_eq!(
        f.cursor(),
        (4, 0, 0),
        "the changed term restores the origin before searching up"
    );
    f.run(&["search-forward-incremental", "="]);
    assert!(
        f.data().search.marks.is_none(),
        "an empty term clears marks"
    );
    f.run(&["search-backward-incremental", "+nomatch"]);
    assert!(
        f.data().search.marks.is_none(),
        "a failed search clears marks"
    );
}
