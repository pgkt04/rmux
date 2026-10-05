// Ported from tmux screen.c and screen-write.c @ 8f25579c
//! Operation-boundary comparisons against the unmodified pinned C writer,
//! including every ordered tty command with its old-state context.
#[path = "../../rmux-util/tests/common/mod.rs"]
mod common;

use rmux_emu::cell::{DEFAULT_CELL, GridAttributes, GridCell, GridCellFlags};
use rmux_emu::colour::Colour;
use rmux_emu::grid::{Grid, GridFlags, GridLineFlags, Osc133Data};
use rmux_emu::hyperlinks::{HyperlinkId, HyperlinkRegistry};
use rmux_emu::screen::write::{
    DrawCommand, DrawOp, DrawSnapshot, ScreenRenderEffects, ScreenWriteCtx, ScreenWritePolicy,
    TtySink,
};
use rmux_emu::screen::{Screen, ScreenMode, ScreenResetPolicy};
use rmux_util::utf8::Utf8Data;
use std::cell::RefCell;
use std::fmt::Write;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;

fn cell_fields(cell: &GridCell) -> String {
    format!(
        " {} {} {} {} {} {} {} {}",
        cell.attr.bits(),
        cell.flags.bits(),
        cell.fg.0,
        cell.bg.0,
        cell.us.0,
        cell.link.0,
        cell.data.width,
        hex(cell.data.bytes())
    )
}

/// Standalone sink that logs each draw in the C driver's `draw` line format.
struct DrawLog(Rc<RefCell<String>>);
impl TtySink for DrawLog {
    fn draw(&mut self, op: DrawOp<'_>, snapshot: &DrawSnapshot) {
        let (name, bg, payload) = match op.command {
            DrawCommand::SyncStart => ("syncstart", 0, String::new()),
            DrawCommand::Cell(cell) => ("cell", 0, cell_fields(cell)),
            DrawCommand::Cells { cell, data } => {
                ("cells", 0, format!("{} {}", cell_fields(cell), hex(data)))
            }
            DrawCommand::RedrawLine { count, .. } => ("redrawline", 0, format!(" {count}")),
            DrawCommand::AlignmentTest => ("alignmenttest", 0, String::new()),
            DrawCommand::InsertCharacter { count, bg } => {
                ("insertcharacter", bg.0, format!(" {count}"))
            }
            DrawCommand::DeleteCharacter { count, bg } => {
                ("deletecharacter", bg.0, format!(" {count}"))
            }
            DrawCommand::ClearCharacter { count, bg } => {
                ("clearcharacter", bg.0, format!(" {count}"))
            }
            DrawCommand::InsertLine { count, bg } => ("insertline", bg.0, format!(" {count}")),
            DrawCommand::DeleteLine { count, bg } => ("deleteline", bg.0, format!(" {count}")),
            DrawCommand::ClearEndOfScreen { bg } => ("clearendofscreen", bg.0, String::new()),
            DrawCommand::ClearStartOfScreen { bg } => ("clearstartofscreen", bg.0, String::new()),
            DrawCommand::ClearScreen { bg } => ("clearscreen", bg.0, String::new()),
            DrawCommand::ScrollUp { count, bg } => ("scrollup", bg.0, format!(" {count}")),
            DrawCommand::ScrollDown { count, bg } => ("scrolldown", bg.0, format!(" {count}")),
            DrawCommand::ReverseIndex { bg } => ("reverseindex", bg.0, String::new()),
            DrawCommand::SetSelection { .. } | DrawCommand::RawString { .. } => {
                panic!("unexpected draw in the reference corpus")
            }
        };
        writeln!(
            self.0.borrow_mut(),
            "draw {name} {} {} {} {} {} {} {} {}{payload}",
            snapshot.old_cx,
            snapshot.old_cy,
            snapshot.rupper,
            snapshot.rlower,
            bg as u32,
            u8::from(snapshot.sync),
            u8::from(snapshot.wrapped),
            u8::from(snapshot.invalidate_cursor)
        )
        .unwrap();
    }
    fn visible_columns(&mut self, x: u32, _: u32, n: u32, out: &mut Vec<Range<u32>>) {
        out.clear();
        if n != 0 {
            out.push(x..x + n);
        }
    }
    fn obscured(&mut self) -> bool {
        false
    }
    fn redraw_pending(&self) -> bool {
        false
    }
    fn effect(&mut self, _: ScreenRenderEffects, _: &Screen) {}
    fn begin_write(&mut self) {}
}

fn reference() -> Option<PathBuf> {
    let driver = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/screen_reference.c");
    let mut flags = vec!["-ffunction-sections"];
    if cfg!(target_os = "macos") {
        flags.extend(["-Wl,-dead_strip", "-L/opt/homebrew/opt/libevent/lib"]);
    } else {
        flags.extend(["-Wl,--gc-sections", "-Wl,--no-as-needed"]);
    }
    flags.push("-levent");
    common::build_c(
        "screen",
        &[
            &driver,
            Path::new("utf8.c"),
            Path::new("utf8-combined.c"),
            Path::new("xmalloc.c"),
            Path::new("compat/utf8proc.c"),
            Path::new("compat/vis.c"),
            Path::new("compat/strtonum.c"),
            Path::new("compat/reallocarray.c"),
            Path::new("compat/recallocarray.c"),
            Path::new("compat/explicit_bzero.c"),
        ],
        &flags,
        cfg!(target_os = "macos"),
    )
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::new();
    for byte in bytes {
        write!(output, "{byte:02x}").unwrap();
    }
    output
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len() / 2)
        .map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

struct Tokens<'a>(std::str::SplitWhitespace<'a>);
impl Tokens<'_> {
    fn text(&mut self) -> &str {
        self.0.next().unwrap_or("")
    }
    fn number(&mut self) -> u32 {
        self.text().parse().unwrap()
    }
    fn signed(&mut self) -> i32 {
        self.text().parse().unwrap()
    }
    fn boolean(&mut self) -> bool {
        self.number() != 0
    }
    fn colour(&mut self) -> Colour {
        Colour(self.signed())
    }
    fn cell(&mut self) -> GridCell {
        let attr = GridAttributes(self.number() as u16);
        let flags = GridCellFlags(self.number() as u8);
        let (fg, bg, us) = (self.colour(), self.colour(), self.colour());
        let link = HyperlinkId(self.number());
        let width = self.number() as u8;
        let bytes = unhex(self.text());
        let mut data = Utf8Data {
            size: bytes.len() as u8,
            have: bytes.len() as u8,
            width,
            ..Utf8Data::default()
        };
        data.data[..bytes.len()].copy_from_slice(&bytes);
        GridCell {
            data,
            attr,
            flags,
            fg,
            bg,
            us,
            link,
        }
    }
}

fn dump_grid(output: &mut String, grid: &Grid) {
    writeln!(
        output,
        "grid {} {} {} {} {} {} {} {} {}",
        grid.sx(),
        grid.sy(),
        grid.hsize(),
        grid.hscrolled,
        grid.hlimit(),
        grid.scroll_added,
        grid.scroll_collected,
        grid.scroll_generation,
        grid.flags.bits()
    )
    .unwrap();
    for y in 0..grid.hsize() + grid.sy() {
        let line = grid.get_line(y);
        let osc = line.osc133;
        writeln!(
            output,
            "line {y} {} {} {} {} {} osc={}/{}/{}/{}/{}",
            line.flags.bits(),
            line.cellused(),
            line.cellsize(),
            line.extdsize(),
            line.time.0,
            osc.prompt_col,
            osc.cmd_col,
            osc.out_start_col,
            osc.out_end_col,
            osc.exit_status
        )
        .unwrap();
        for x in 0..line.cellsize() {
            let cell = grid.get_cell(x, y);
            writeln!(
                output,
                "cell {x} {} {} {} {} {} {} {} {}",
                cell.attr.bits(),
                cell.flags.bits(),
                cell.fg.0,
                cell.bg.0,
                cell.us.0,
                cell.link.0,
                cell.data.width,
                hex(cell.data.bytes())
            )
            .unwrap();
        }
    }
}

fn dump(output: &mut String, screen: &Screen, step: &mut u32) {
    writeln!(
        output,
        "boundary {}\nscreen {} {} {} {} {} {} {} {} {}",
        *step,
        screen.cx,
        screen.cy,
        screen.rupper,
        screen.rlower,
        screen.mode.bits(),
        screen.saved_cursor.map_or(u32::MAX, |cursor| cursor.0),
        screen.saved_cursor.map_or(u32::MAX, |cursor| cursor.1),
        screen.saved_flags.bits(),
        u8::from(screen.saved_grid.is_some())
    )
    .unwrap();
    *step += 1;
    dump_grid(output, &screen.grid);
    if screen.saved_grid.is_some() || screen.saved_cursor.is_some() {
        let cell = screen.saved_cell;
        writeln!(
            output,
            "savedcell {} {} {} {} {} {} {} {}",
            cell.attr.bits(),
            cell.flags.bits(),
            cell.fg.0,
            cell.bg.0,
            cell.us.0,
            cell.link.0,
            cell.data.width,
            hex(cell.data.bytes())
        )
        .unwrap();
    }
    if let Some(saved) = &screen.saved_grid {
        output.push_str("saved\n");
        dump_grid(output, saved);
    }
}

fn writer_script(
    writer: &mut ScreenWriteCtx<'_>,
    lines: &mut std::str::Lines<'_>,
    rendition: &mut GridCell,
    output: &RefCell<String>,
    step: &mut u32,
) {
    for line in lines.by_ref() {
        let mut tokens = Tokens(line.split_whitespace());
        let op = tokens.text().to_owned();
        if op == "end" {
            writer.collect_end();
            dump(&mut output.borrow_mut(), writer.screen, step);
            return;
        }
        if op == "add" {
            writer.collect_add(&tokens.cell());
            continue;
        }
        writer.collect_end();
        match op.as_str() {
            "" => {}
            "boundary" => dump(&mut output.borrow_mut(), writer.screen, step),
            "cell" => writer.cell(&tokens.cell()),
            "text" => {
                *rendition = tokens.cell();
                for byte in unhex(tokens.text()) {
                    rendition.data = Utf8Data::set(byte);
                    writer.collect_add(rendition);
                }
            }
            "move" => writer.cursormove(tokens.signed(), tokens.signed(), tokens.boolean()),
            "up" => writer.cursorup(tokens.number()),
            "down" => writer.cursordown(tokens.number()),
            "right" => writer.cursorright(tokens.number()),
            "left" => writer.cursorleft(tokens.number()),
            "bs" => writer.backspace(),
            "cr" => writer.carriagereturn(),
            "region" => writer.scrollregion(tokens.number(), tokens.number()),
            "lf" => writer.linefeed(tokens.boolean(), tokens.colour()),
            "su" => writer.scrollup(tokens.number(), tokens.colour()),
            "sd" => writer.scrolldown(tokens.number(), tokens.colour()),
            "ri" => writer.reverseindex(tokens.colour()),
            "ich" => writer.insertcharacter(tokens.number(), tokens.colour()),
            "dch" => writer.deletecharacter(tokens.number(), tokens.colour()),
            "ech" => writer.clearcharacter(tokens.number(), tokens.colour()),
            "il" => writer.insertline(tokens.number(), tokens.colour()),
            "dl" => writer.deleteline(tokens.number(), tokens.colour()),
            "el" => writer.clearline(tokens.colour()),
            "el0" => writer.clearendofline(tokens.colour()),
            "el1" => writer.clearstartofline(tokens.colour()),
            "ed0" => writer.clearendofscreen(tokens.colour()),
            "ed1" => writer.clearstartofscreen(tokens.colour()),
            "ed2" => writer.clearscreen(tokens.colour()),
            "histclear" => writer.clearhistory(),
            "align" => writer.alignmenttest(),
            "reset" => writer.reset(ScreenResetPolicy {
                extended_keys: writer.policy.extended_keys,
            }),
            "setmode" => writer.mode_set(ScreenMode(tokens.number())),
            "clearmode" => writer.mode_clear(ScreenMode(tokens.number())),
            _ => panic!("unknown writer operation: {line}"),
        }
    }
    panic!("writer script missing end");
}

fn rust_script(script: &str) -> String {
    let mut registry = HyperlinkRegistry::new();
    let mut screen = Screen::new(1, 1, 0, ScreenResetPolicy::default(), &mut registry).unwrap();
    let output = Rc::new(RefCell::new(String::new()));
    let mut sink = DrawLog(output.clone());
    let mut policy = ScreenWritePolicy::default();
    let mut rendition = DEFAULT_CELL;
    let mut step = 0;
    let mut lines = script.lines();
    while let Some(line) = lines.next() {
        let mut tokens = Tokens(line.split_whitespace());
        let op = tokens.text().to_owned();
        match op.as_str() {
            "" => {}
            "new" => {
                let (width, height, limit) = (tokens.number(), tokens.number(), tokens.number());
                policy.extended_keys = tokens.boolean();
                policy.variation_selector_always_wide = tokens.boolean();
                screen.release(&mut registry).unwrap();
                screen = Screen::new(
                    width,
                    height,
                    limit,
                    ScreenResetPolicy {
                        extended_keys: policy.extended_keys,
                    },
                    &mut registry,
                )
                .unwrap();
                rendition = DEFAULT_CELL;
            }
            "begin" => {
                let mut writer =
                    ScreenWriteCtx::start(&mut screen, &mut sink, policy, &mut registry);
                writer_script(&mut writer, &mut lines, &mut rendition, &output, &mut step);
                writer.finish();
                dump(&mut output.borrow_mut(), &screen, &mut step);
            }
            "boundary" => dump(&mut output.borrow_mut(), &screen, &mut step),
            "resize" => screen.resize_cursor(
                tokens.number(),
                tokens.number(),
                tokens.boolean(),
                tokens.boolean(),
                tokens.boolean(),
            ),
            "reinit" => screen
                .reinit(
                    tokens.boolean(),
                    ScreenResetPolicy {
                        extended_keys: policy.extended_keys,
                    },
                    &mut registry,
                )
                .unwrap(),
            "on" => {
                screen.alternate_on(&rendition, tokens.boolean());
            }
            "off" => {
                screen.alternate_off(Some(&mut rendition), tokens.boolean());
            }
            "rendition" => rendition = tokens.cell(),
            "history" => {
                if tokens.boolean() {
                    screen.grid.flags.insert(GridFlags::HISTORY);
                } else {
                    screen.grid.flags.remove(GridFlags::HISTORY);
                }
            }
            "osc" => {
                let y = tokens.number() + screen.grid.hsize();
                let line = screen.grid.get_line_mut(y);
                line.flags.insert(GridLineFlags(tokens.number() as u16));
                line.osc133 = Osc133Data {
                    prompt_col: tokens.number() as u16,
                    cmd_col: tokens.number() as u16,
                    out_start_col: tokens.number() as u16,
                    out_end_col: tokens.number() as u16,
                    exit_status: tokens.number() as u8,
                };
            }
            _ => panic!("unknown screen operation: {line}"),
        }
    }
    screen.release(&mut registry).unwrap();
    output.take()
}

fn compare(script: &str) {
    let Some(binary) = reference() else {
        return;
    };
    let expected = String::from_utf8(common::run(&binary, &[], script.as_bytes())).unwrap();
    let actual = rust_script(script);
    let mut boundary = "initial";
    for (index, (c, rust)) in expected.lines().zip(actual.lines()).enumerate() {
        if c.starts_with("boundary ") {
            boundary = c;
        }
        assert_eq!(rust, c, "{boundary}, dump line {index}\nscript:\n{script}");
    }
    assert_eq!(
        actual.lines().count(),
        expected.lines().count(),
        "dump length\nscript:\n{script}"
    );
}

fn cell_spec(width: u8, bytes: &[u8], bg: i32) -> String {
    format!("0 0 8 {bg} 8 0 {width} {}", hex(bytes))
}

fn text(script: &mut String, bytes: &[u8], bg: i32) {
    writeln!(script, "text {} {}", cell_spec(1, b" ", bg), hex(bytes)).unwrap();
}

fn operation(script: &mut String, op: &str) {
    writeln!(script, "{op}\nboundary").unwrap();
}

#[test]
fn pending_wrap_movement_edits_and_erase_match_c() {
    let mut script = String::from("new 5 4 20 0 0\nbegin\n");
    text(&mut script, b"abcde", 8);
    operation(&mut script, "boundary");
    for op in [
        "ich 0 2", "dch 9 3", "ech 1 4", "el0 8", "right 0", "left 0", "down 99", "up 0",
    ] {
        operation(&mut script, op);
    }
    text(&mut script, b"1234567890ABCDE", 8);
    operation(&mut script, "move 0 2 0");
    operation(&mut script, "bs");
    operation(&mut script, "region 1 2");
    operation(&mut script, "setmode 8192");
    operation(&mut script, "move 99 99 1");
    operation(&mut script, "move -1 -1 0");
    operation(&mut script, "clearmode 8192");
    for op in [
        "move 0 0 0",
        "il 0 1",
        "dl 9 2",
        "move 1 3 0",
        "il 2 3",
        "dl 0 4",
        "el1 5",
        "el 8",
        "ed1 6",
        "ed0 7",
        "ed2 8",
    ] {
        operation(&mut script, op);
    }
    operation(&mut script, "region 2 2");
    operation(&mut script, "region 3 1");
    operation(&mut script, "align");
    operation(&mut script, "reset");
    script.push_str("end\nboundary\n");
    compare(&script);
}

#[test]
fn collection_scroll_background_and_osc133_match_c() {
    let mut script = String::from("new 9 5 12 1 0\nosc 1 248 2 3 4 5 17\nbegin\n");
    for row in 0..5 {
        operation(&mut script, &format!("move 0 {row} 0"));
        text(&mut script, b"abcdefghi", row);
    }
    operation(&mut script, "move 3 1 0");
    operation(&mut script, "el 4");
    operation(&mut script, "region 1 3");
    for op in [
        "move 0 3 0",
        "lf 1 1",
        "lf 0 2",
        "su 0 2",
        "su 99 3",
        "sd 0 4",
        "move 0 1 0",
        "ri 5",
        "move 0 0 0",
        "ri 8",
        "histclear",
    ] {
        operation(&mut script, op);
    }
    script.push_str("end\nboundary\nbegin\n");
    text(&mut script, b"012345678", 1);
    operation(&mut script, "move 2 0 0");
    text(&mut script, b"XYZ", 2);
    operation(&mut script, "move 1 0 0");
    operation(&mut script, "ech 5 3");
    text(&mut script, b"q", 4);
    script.push_str("end\nboundary\n");
    compare(&script);
}

#[test]
fn wide_tabs_combining_and_nowrap_match_c() {
    let mut script = String::from("new 7 3 15 0 1\nbegin\n");
    for (width, data) in [
        (0, "\u{301}"),
        (2, "界"),
        (0, "\u{301}"),
        (1, "x"),
        (0, "\u{200d}"),
        (2, "🚀"),
        (2, "\u{3164}"),
    ] {
        operation(
            &mut script,
            &format!("cell {}", cell_spec(width, data.as_bytes(), 3)),
        );
    }
    operation(&mut script, "move 0 0 0");
    operation(&mut script, &format!("cell {}", cell_spec(1, b"A", 2)));
    for _ in 0..18 {
        operation(
            &mut script,
            &format!("cell {}", cell_spec(0, "\u{301}".as_bytes(), 2)),
        );
    }
    operation(&mut script, "move 0 2 0");
    operation(
        &mut script,
        &format!("cell {}", cell_spec(1, "❤".as_bytes(), 2)),
    );
    operation(
        &mut script,
        &format!("cell {}", cell_spec(0, "\u{fe0f}".as_bytes(), 2)),
    );
    operation(&mut script, "move 3 2 0");
    for data in ["ᄀ", "ᅡ", "ᆨ"] {
        operation(
            &mut script,
            &format!("cell {}", cell_spec(2, data.as_bytes(), 2)),
        );
    }
    operation(&mut script, "move 1 0 0");
    text(&mut script, b"a", 4);
    operation(&mut script, "boundary");
    operation(&mut script, "move 0 1 0");
    operation(&mut script, "cell 0 128 8 5 8 0 4 09");
    operation(&mut script, "move 2 1 0");
    operation(&mut script, &format!("cell {}", cell_spec(1, b"Z", 6)));
    operation(&mut script, "clearmode 16");
    operation(&mut script, "move 6 2 0");
    operation(
        &mut script,
        &format!("cell {}", cell_spec(2, "界".as_bytes(), 7)),
    );
    text(&mut script, b"abc", 8);
    operation(&mut script, "boundary");
    operation(&mut script, "setmode 2");
    operation(&mut script, "move 1 2 0");
    operation(
        &mut script,
        &format!("cell {}", cell_spec(2, "好".as_bytes(), 2)),
    );
    operation(&mut script, "clearmode 2");
    script.push_str("end\nboundary\n");
    compare(&script);
}

#[test]
fn resize_history_reflow_and_alternate_match_c() {
    for history in [0, 1] {
        for eat_empty in [0, 1] {
            for track in [0, 1] {
                let mut script = format!("new 8 5 20 1 0\nhistory {history}\nbegin\n");
                text(&mut script, b"0123456789abcdefghijklmnopqrstuvwxyz", 8);
                script.push_str("end\nboundary\n");
                operation(&mut script, &format!("resize 8 3 1 {eat_empty} {track}"));
                operation(&mut script, "resize 8 7 1 1 1");
                operation(&mut script, &format!("resize 4 7 1 1 {track}"));
                operation(&mut script, &format!("resize 12 4 1 {eat_empty} {track}"));
                script.push_str("rendition 8193 32 16777221 33554481 2 7 1 52\n");
                operation(&mut script, "on 1");
                operation(&mut script, "on 0");
                script.push_str("begin\n");
                text(&mut script, b"ALTERNATE-SCREEN", 3);
                script.push_str("end\nboundary\n");
                operation(&mut script, "resize 6 3 0 1 1");
                operation(&mut script, "off 1");
                operation(&mut script, "off 1");
                operation(&mut script, "on 0");
                operation(&mut script, "off 0");
                operation(&mut script, "reinit 0");
                operation(&mut script, "resize 0 0 1 1 1");
                compare(&script);
            }
        }
    }
}

#[test]
fn reset_policies_preserve_crlf_only_for_reinit() {
    for extended_keys in [0, 1] {
        let mut script = format!("new 1 1 0 {extended_keys} 0\nbegin\n");
        operation(&mut script, "setmode 16384");
        text(&mut script, b"abc", 8);
        operation(&mut script, "reset");
        operation(&mut script, "setmode 16384");
        script.push_str("end\nboundary\n");
        operation(&mut script, "reinit 0");
        compare(&script);
    }
}

#[test]
fn collection_transaction_splits_keep_the_same_grid() {
    let bytes = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let cell = cell_spec(1, b" ", 8);
    let mut whole = String::from("new 7 3 12 0 0\nbegin\n");
    for byte in bytes {
        writeln!(whole, "add {}", cell_spec(1, &[*byte], 8)).unwrap();
    }
    whole.push_str("end\nboundary\n");
    compare(&whole);
    let whole_dump = rust_script(&whole);
    let whole_grid = &whole_dump[whole_dump.rfind("screen ").unwrap()..];
    for split in 0..=bytes.len() {
        let mut script = String::from("new 7 3 12 0 0\nbegin\n");
        writeln!(
            script,
            "text {cell} {}\nend\nboundary\nbegin",
            hex(&bytes[..split])
        )
        .unwrap();
        writeln!(
            script,
            "text {cell} {}\nend\nboundary",
            hex(&bytes[split..])
        )
        .unwrap();
        compare(&script);
        let split_dump = rust_script(&script);
        let split_grid = &split_dump[split_dump.rfind("screen ").unwrap()..];
        assert_eq!(split_grid, whole_grid, "transaction split {split}");
    }
}

#[test]
fn deterministic_random_writer_boundaries_match_c() {
    for seed in [0x0804_0001, 0x0804_5137, 0xfeed_cafe, 0x1234_9876] {
        let mut rng = common::Rng::new(seed);
        let mut width = 11;
        let mut height = 6;
        let mut script = format!("new {width} {height} 18 0 0\nbegin\n");
        for _ in 0..240 {
            let count = rng.below(18);
            let bg = rng.below(10);
            let op = match rng.below(36) {
                0..=3 => {
                    let bytes: Vec<_> = (0..rng.below(18) + 1)
                        .map(|_| b'!' + rng.below(94) as u8)
                        .collect();
                    text(&mut script, &bytes, bg as i32);
                    "boundary".to_owned()
                }
                4 => format!("move {} {} 0", rng.below(width + 4), rng.below(height + 3)),
                5 => format!("up {count}"),
                6 => format!("down {count}"),
                7 => format!("left {count}"),
                8 => format!("right {count}"),
                9 => "bs".to_owned(),
                10 => "cr".to_owned(),
                11 => format!("lf {} {bg}", rng.below(2)),
                12 => format!("su {count} {bg}"),
                13 => format!("sd {count} {bg}"),
                14 => format!("ri {bg}"),
                15 => format!("ich {count} {bg}"),
                16 => format!("dch {count} {bg}"),
                17 => format!("ech {count} {bg}"),
                18 => format!("il {count} {bg}"),
                19 => format!("dl {count} {bg}"),
                20 => format!("el0 {bg}"),
                21 => format!("el1 {bg}"),
                22 => format!("ed0 {bg}"),
                23 => format!("ed1 {bg}"),
                24 => format!("region {} {}", rng.below(height + 2), rng.below(height + 2)),
                25 => format!("cell {}", cell_spec(2, "界".as_bytes(), bg as i32)),
                26 => {
                    script.push_str("end\nboundary\n");
                    width = rng.below(12) + 2;
                    height = rng.below(6) + 2;
                    operation(&mut script, &format!("resize {width} {height} 1 1 1"));
                    script.push_str("begin\n");
                    "cr".to_owned()
                }
                27 => {
                    script.push_str("end\nboundary\n");
                    operation(&mut script, &format!("on {}", rng.below(2)));
                    operation(&mut script, &format!("off {}", rng.below(2)));
                    script.push_str("begin\n");
                    "cr".to_owned()
                }
                28 => format!("el {bg}"),
                29 => format!("ed2 {bg}"),
                30 => "align".to_owned(),
                31 => format!("{} 2", ["setmode", "clearmode"][rng.below(2) as usize]),
                32 => format!("{} 16", ["setmode", "clearmode"][rng.below(2) as usize]),
                33 => format!("cell {}", cell_spec(0, "\u{301}".as_bytes(), bg as i32)),
                34 => format!("cell {}", cell_spec(1, b"\t", bg as i32)),
                // input.c keeps TAB width within the row; wider tabs are
                // outside the writer's domain (grid_move_cells underflows).
                _ => format!("cell 0 128 8 {bg} 8 0 {} 09", rng.below(2) + 1),
            };
            operation(&mut script, &op);
        }
        script.push_str("end\nboundary\n");
        compare(&script);
    }
}

#[derive(Debug)]
struct RecordedDraw {
    name: &'static str,
    snapshot: DrawSnapshot,
    new_cursor: (u32, u32),
    row: Vec<GridCell>,
}
#[derive(Default)]
struct Recorder {
    draws: Vec<RecordedDraw>,
}
impl TtySink for Recorder {
    fn draw(&mut self, op: DrawOp<'_>, snapshot: &DrawSnapshot) {
        let name = match op.command {
            DrawCommand::SyncStart => "sync",
            DrawCommand::Cell(_) => "cell",
            DrawCommand::InsertCharacter { .. } => "insert",
            DrawCommand::ClearCharacter { .. } => "erase",
            DrawCommand::ScrollDown { .. } => "scroll",
            _ => "other",
        };
        self.draws.push(RecordedDraw {
            name,
            snapshot: *snapshot,
            new_cursor: (op.screen.cx, op.screen.cy),
            row: (0..op.screen.grid.sx())
                .map(|x| op.screen.grid.view_get_cell(x, 0))
                .collect(),
        });
    }
    fn visible_columns(&mut self, x: u32, _: u32, count: u32, out: &mut Vec<Range<u32>>) {
        out.clear();
        if count != 0 {
            out.push(x..x + count);
        }
    }
    fn obscured(&mut self) -> bool {
        false
    }
    fn redraw_pending(&self) -> bool {
        false
    }
    fn effect(&mut self, _: ScreenRenderEffects, _: &Screen) {}
    fn begin_write(&mut self) {}
}

#[test]
fn structural_draws_borrow_new_grid_and_keep_old_cursor_snapshot() {
    let mut registry = HyperlinkRegistry::new();
    let mut screen = Screen::new(5, 3, 8, ScreenResetPolicy::default(), &mut registry).unwrap();
    let mut sink = Recorder::default();
    let mut writer = ScreenWriteCtx::start(
        &mut screen,
        &mut sink,
        ScreenWritePolicy::default(),
        &mut registry,
    );
    let mut cell = DEFAULT_CELL;
    cell.data = Utf8Data::set(b'A');
    writer.cell(&cell);
    writer.cursormove(0, 0, false);
    writer.insertcharacter(1, Colour(2));
    writer.clearcharacter(1, Colour(3));
    writer.scrolldown(1, Colour(4));
    writer.finish();
    let find = |name| sink.draws.iter().find(|draw| draw.name == name).unwrap();
    let cell = find("cell");
    assert_eq!((cell.snapshot.old_cx, cell.snapshot.old_cy), (0, 0));
    assert_eq!(cell.new_cursor, (1, 0));
    assert_eq!(cell.row[0].data.bytes(), b"A");
    let insert = find("insert");
    assert_eq!((insert.snapshot.old_cx, insert.snapshot.old_cy), (0, 0));
    assert_eq!(insert.row[0].bg, Colour(2));
    assert_eq!(insert.row[1].data.bytes(), b"A");
    let erase = find("erase");
    assert_eq!(erase.row[0].bg, Colour(3));
    let scroll = find("scroll");
    assert_eq!((scroll.snapshot.rupper, scroll.snapshot.rlower), (0, 2));
    assert_eq!(scroll.row[0].bg, Colour(4));
    assert_eq!(
        sink.draws.iter().filter(|draw| draw.name == "sync").count(),
        1
    );
    screen.release(&mut registry).unwrap();
}
