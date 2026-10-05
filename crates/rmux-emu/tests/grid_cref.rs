// Ported from tmux grid.c, grid-view.c and grid-reader.c @ 8f25579c
//! Differential tests: the same command script runs through a C driver
//! built on the pinned grid sources (`tests/grid_reference.c`) and through
//! `rmux_emu::grid`; every printed line must match.
#[path = "../../rmux-util/tests/common/mod.rs"]
mod common;

use rmux_emu::cell::*;
use rmux_emu::colour::*;
use rmux_emu::grid::names::*;
use rmux_emu::grid::reader::GridReader;
use rmux_emu::grid::*;
use rmux_emu::hyperlinks::*;
use rmux_util::utf8::Utf8Data;
use std::fmt::Write;
use std::path::Path;

fn reference() -> Option<std::path::PathBuf> {
    let driver = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/grid_reference.c");
    common::build_c(
        "grid",
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
        &[],
        cfg!(target_os = "macos"),
    )
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::new();
    for b in bytes {
        write!(s, "{b:02x}").unwrap();
    }
    s
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

/// Interpreter for the driver protocol on the Rust grid.
struct Machine {
    gd: Grid,
    registry: HyperlinkRegistry,
    store: Hyperlinks,
    last: Option<GridCell>,
    cx: u32,
    cy: u32,
    out: String,
}

struct Toks<'a>(std::str::SplitWhitespace<'a>);
impl Toks<'_> {
    fn s(&mut self) -> &str {
        self.0.next().unwrap_or("")
    }
    fn n(&mut self) -> u32 {
        self.s().parse::<u32>().unwrap_or(0)
    }
    fn i(&mut self) -> i32 {
        self.s().parse::<i32>().unwrap_or(0)
    }
    fn b(&mut self) -> bool {
        self.i() != 0
    }
    fn c(&mut self) -> Colour {
        Colour(self.i())
    }
    fn cell(&mut self) -> GridCell {
        let attr = GridAttributes(self.n() as u16);
        let flags = GridCellFlags(self.n() as u8);
        let (fg, bg, us) = (self.c(), self.c(), self.c());
        let link = HyperlinkId(self.n());
        let width = self.n() as u8;
        let bytes = unhex(self.s());
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
    fn ud(&mut self) -> Utf8Data {
        let bytes = unhex(self.s());
        let mut ud = Utf8Data {
            size: bytes.len() as u8,
            have: bytes.len() as u8,
            width: 0,
            ..Utf8Data::default()
        };
        ud.data[..bytes.len()].copy_from_slice(&bytes);
        ud.width = self.n() as u8;
        ud
    }
}

impl Machine {
    fn new() -> Self {
        let mut registry = HyperlinkRegistry::new();
        let store = registry.create().unwrap();
        Machine {
            gd: Grid::new(1, 1, 0),
            registry,
            store,
            last: None,
            cx: 0,
            cy: 0,
            out: String::new(),
        }
    }

    fn dump(&mut self) {
        let gd = &self.gd;
        writeln!(
            self.out,
            "grid {} {} {} {} {} {} {} {} {}",
            gd.sx(),
            gd.sy(),
            gd.hsize(),
            gd.hscrolled,
            gd.hlimit(),
            gd.scroll_added,
            gd.scroll_collected,
            gd.scroll_generation,
            gd.flags.bits()
        )
        .unwrap();
        for yy in 0..gd.hsize() + gd.sy() {
            let gl = gd.get_line(yy);
            writeln!(
                self.out,
                "line {} flags={} used={} size={} extd={} time={}",
                yy,
                gl.flags.bits(),
                gl.cellused(),
                gl.cellsize(),
                gl.extdsize(),
                gl.time.0
            )
            .unwrap();
            writeln!(
                self.out,
                "osc {} {} {} {} {}",
                gl.osc133.prompt_col,
                gl.osc133.cmd_col,
                gl.osc133.out_start_col,
                gl.osc133.out_end_col,
                gl.osc133.exit_status
            )
            .unwrap();
            for (xx, e) in gl.entries().iter().enumerate() {
                let gc = gd.get_cell(xx as u32, yy);
                let c = e.compact();
                write!(
                    self.out,
                    " {}:e{}/{:02x}{:02x}{:02x}{:02x}/{} {} {} {} {} {} {} {}",
                    xx,
                    e.flags().bits(),
                    c.attr,
                    c.fg,
                    c.bg,
                    c.data,
                    gc.attr.bits(),
                    gc.flags.bits(),
                    gc.fg.0,
                    gc.bg.0,
                    gc.us.0,
                    gc.link.0,
                    gc.data.width,
                    hex(gc.data.bytes())
                )
                .unwrap();
            }
            if gl.cellsize() != 0 {
                self.out.push('\n');
            }
        }
    }

    fn reader<R>(&mut self, f: impl FnOnce(&mut GridReader) -> R) -> R {
        let mut gr = GridReader::new(&self.gd, self.cx, self.cy);
        let r = f(&mut gr);
        (self.cx, self.cy) = gr.cursor();
        r
    }

    fn run(&mut self, script: &str) {
        for line in script.lines() {
            let mut t = Toks(line.split_whitespace());
            let op = t.s().to_string();
            let gd = &mut self.gd;
            match op.as_str() {
                "" => {}
                "new" => {
                    let (sx, sy, hl) = (t.n(), t.n(), t.n());
                    self.gd = Grid::new(sx, sy, hl);
                    self.registry.reset(&self.store).unwrap();
                    self.last = None;
                }
                "clock" => {
                    let cur = t.i();
                    let start = t.i();
                    gd.set_line_clock(if cur == 0 {
                        LineTime(0)
                    } else {
                        LineTime((cur - start + 1) as u32)
                    });
                }
                "set" => {
                    let (px, py) = (t.n(), t.n());
                    let gc = t.cell();
                    gd.set_cell(px, py, &gc);
                }
                "vset" => {
                    let (px, py) = (t.n(), t.n());
                    let gc = t.cell();
                    gd.view_set_cell(px, py, &gc);
                }
                "tab" => {
                    let (px, py, w) = (t.n(), t.n(), t.n());
                    let mut gc = t.cell();
                    gc.set_tab(w);
                    gd.set_cell(px, py, &gc);
                }
                "pad" => {
                    let (px, py, bg) = (t.n(), t.n(), t.c());
                    gd.set_padding(px, py, bg);
                }
                "vpad" => {
                    let (px, py, bg) = (t.n(), t.n(), t.c());
                    gd.view_set_padding(px, py, bg);
                }
                "cells" => {
                    let (px, py) = (t.n(), t.n());
                    let gc = t.cell();
                    let s = unhex(t.s());
                    gd.set_cells(px, py, &gc, &s);
                }
                "vcells" => {
                    let (px, py) = (t.n(), t.n());
                    let gc = t.cell();
                    let s = unhex(t.s());
                    gd.view_set_cells(px, py, &gc, &s);
                }
                "clear" => {
                    let (px, py, nx, ny, bg) = (t.n(), t.n(), t.n(), t.n(), t.c());
                    gd.clear(px, py, nx, ny, bg);
                }
                "clearlines" => {
                    let (py, ny, bg) = (t.n(), t.n(), t.c());
                    gd.clear_lines(py, ny, bg);
                }
                "movelines" => {
                    let (dy, py, ny, bg) = (t.n(), t.n(), t.n(), t.c());
                    gd.move_lines(dy, py, ny, bg);
                }
                "movecells" => {
                    let (dx, px, py, nx, bg) = (t.n(), t.n(), t.n(), t.n(), t.c());
                    gd.move_cells(dx, px, py, nx, bg);
                }
                "scroll" => gd.scroll_history(t.c()),
                "scrollregion" => {
                    let (u, l, bg) = (t.n(), t.n(), t.c());
                    gd.scroll_history_region(u, l, bg);
                }
                "collect" => gd.collect_history(t.b()),
                "removehist" => gd.remove_history(t.n()),
                "clearhist" => gd.clear_history(),
                "vclearhist" => gd.view_clear_history(t.c()),
                "vclear" => {
                    let (px, py, nx, ny, bg) = (t.n(), t.n(), t.n(), t.n(), t.c());
                    gd.view_clear(px, py, nx, ny, bg);
                }
                "vscrollup" => {
                    let (u, l, bg) = (t.n(), t.n(), t.c());
                    gd.view_scroll_region_up(u, l, bg);
                }
                "vscrolldown" => {
                    let (u, l, bg) = (t.n(), t.n(), t.c());
                    gd.view_scroll_region_down(u, l, bg);
                }
                "vinslines" => {
                    let (py, ny, bg) = (t.n(), t.n(), t.c());
                    gd.view_insert_lines(py, ny, bg);
                }
                "vinslinesreg" => {
                    let (rl, py, ny, bg) = (t.n(), t.n(), t.n(), t.c());
                    gd.view_insert_lines_region(rl, py, ny, bg);
                }
                "vdellines" => {
                    let (py, ny, bg) = (t.n(), t.n(), t.c());
                    gd.view_delete_lines(py, ny, bg);
                }
                "vdellinesreg" => {
                    let (rl, py, ny, bg) = (t.n(), t.n(), t.n(), t.c());
                    gd.view_delete_lines_region(rl, py, ny, bg);
                }
                "vinscells" => {
                    let (px, py, nx, bg) = (t.n(), t.n(), t.n(), t.c());
                    gd.view_insert_cells(px, py, nx, bg);
                }
                "vdelcells" => {
                    let (px, py, nx, bg) = (t.n(), t.n(), t.n(), t.c());
                    gd.view_delete_cells(px, py, nx, bg);
                }
                "reflow" => gd.reflow(t.n()),
                "setsx" => gd.set_sx(t.n()),
                "tail" => gd.adjust_lines(gd.hsize() + gd.sy() + t.n()),
                "metadata" => {
                    let gl = gd.get_line_mut(t.n());
                    gl.flags = GridLineFlags(t.n() as u16);
                    gl.time = LineTime(t.n());
                    gl.osc133 = Osc133Data {
                        prompt_col: t.n() as u16,
                        cmd_col: t.n() as u16,
                        out_start_col: t.n() as u16,
                        out_end_col: t.n() as u16,
                        exit_status: t.n() as u8,
                    };
                }
                "wrappos" => {
                    let (px, py) = (t.n(), t.n());
                    let (wx, wy) = gd.wrap_position(px, py);
                    writeln!(self.out, "{wx} {wy}").unwrap();
                }
                "unwrappos" => {
                    let (wx, wy) = (t.n(), t.n());
                    let (px, py) = gd.unwrap_position(wx, wy);
                    writeln!(self.out, "{px} {py}").unwrap();
                }
                "linelen" => {
                    let py = t.n();
                    writeln!(self.out, "{} {}", gd.line_length(py), gd.line_limit(py)).unwrap();
                }
                "inset" => {
                    let (px, py) = (t.n(), t.n());
                    let set = unhex(t.s());
                    writeln!(self.out, "{}", gd.in_set(px, py, &set) as i32).unwrap();
                }
                "link" => {
                    let uri = unhex(t.s());
                    let id = unhex(t.s());
                    let id = if id.is_empty() { None } else { Some(&id[..]) };
                    let r = self.registry.put(&self.store, &uri, id).unwrap();
                    writeln!(self.out, "{}", r.0).unwrap();
                }
                "resetlast" => self.last = None,
                "string" => {
                    let (px, py, nx) = (t.n(), t.n(), t.n());
                    let flags = GridStringFlags(t.n());
                    let (uselast, usesc) = (t.b(), t.b());
                    if uselast && self.last.is_none() {
                        self.last = Some(DEFAULT_CELL);
                    }
                    let mut ctx = StringCellsCtx {
                        last: if uselast { self.last.as_mut() } else { None },
                        flags,
                        hyperlinks: usesc.then_some((&self.registry, &self.store)),
                    };
                    let s = gd.string_cells(px, py, nx, &mut ctx);
                    let s = rmux_util::bytes::cstr(&s);
                    writeln!(self.out, "{}", hex(s)).unwrap();
                }
                "vstring" => {
                    let (px, py, nx) = (t.n(), t.n(), t.n());
                    let s = gd.view_string_cells(px, py, nx);
                    writeln!(self.out, "{}", hex(rmux_util::bytes::cstr(&s))).unwrap();
                }
                "dump" => self.dump(),
                "rstart" => {
                    self.cx = t.n();
                    self.cy = t.n();
                }
                "rright" => {
                    let (w, a, o) = (t.b(), t.b(), t.b());
                    self.reader(|r| r.cursor_right(w, a, o));
                }
                "rleft" => {
                    let w = t.b();
                    self.reader(|r| r.cursor_left(w));
                }
                "rdown" => self.reader(|r| r.cursor_down()),
                "rup" => self.reader(|r| r.cursor_up()),
                "rsol" => {
                    let w = t.b();
                    self.reader(|r| r.cursor_start_of_line(w));
                }
                "reol" => {
                    let (w, a) = (t.b(), t.b());
                    self.reader(|r| r.cursor_end_of_line(w, a));
                }
                "rnextword" => {
                    let sep = unhex(t.s());
                    self.reader(|r| r.cursor_next_word(&sep));
                }
                "rnextwordend" => {
                    let sep = unhex(t.s());
                    self.reader(|r| r.cursor_next_word_end(&sep));
                }
                "rprevword" => {
                    let sep = unhex(t.s());
                    let (al, st) = (t.b(), t.b());
                    self.reader(|r| r.cursor_previous_word(&sep, al, st));
                }
                "rjump" => {
                    let ud = t.ud();
                    let r = self.reader(|r| r.cursor_jump(&ud));
                    writeln!(self.out, "{}", r as i32).unwrap();
                }
                "rjumpback" => {
                    let ud = t.ud();
                    let r = self.reader(|r| r.cursor_jump_back(&ud));
                    writeln!(self.out, "{}", r as i32).unwrap();
                }
                "rindent" => self.reader(|r| r.cursor_back_to_indentation()),
                "rcursor" => writeln!(self.out, "{} {}", self.cx, self.cy).unwrap(),
                "rlinelen" => {
                    let n = self.reader(|r| r.line_length());
                    writeln!(self.out, "{n}").unwrap();
                }
                "rinset" => {
                    let set = unhex(t.s());
                    let n = self.reader(|r| r.in_set(&set));
                    writeln!(self.out, "{}", n as i32).unwrap();
                }
                "names" => {
                    let (lf, cf, at) = (t.n(), t.n(), t.n());
                    writeln!(
                        self.out,
                        "{} {} {}",
                        line_flags_string(GridLineFlags(lf as u16)),
                        cell_flags_string(GridCellFlags(cf as u8)),
                        cell_attr_string(GridAttributes(at as u16))
                    )
                    .unwrap();
                }
                other => panic!("unknown op {other}"),
            }
        }
    }
}

/// Run `script` through both implementations; returns the Rust output.
fn compare(script: &str) -> String {
    let mut m = Machine::new();
    m.run(script);
    if let Some(bin) = reference() {
        let actual = common::run(&bin, &[], script.as_bytes());
        let actual = String::from_utf8(actual).unwrap();
        let c: Vec<_> = actual.lines().collect();
        let r: Vec<_> = m.out.lines().collect();
        for (n, (a, b)) in c.iter().zip(&r).enumerate() {
            assert_eq!(a, b, "output line {n} differs\nscript:\n{script}");
        }
        assert_eq!(c.len(), r.len(), "line count differs\nscript:\n{script}");
    } else {
        eprintln!("C reference skipped (pinned tmux checkout missing)");
    }
    m.out
}

#[allow(clippy::too_many_arguments)]
fn cell(
    attr: u16,
    flags: u8,
    fg: i32,
    bg: i32,
    us: i32,
    link: u32,
    width: u8,
    text: &str,
) -> String {
    format!(
        "{attr} {flags} {fg} {bg} {us} {link} {width} {}",
        hex(text.as_bytes())
    )
}
fn plain(text: &str) -> String {
    cell(0, 0, 8, 8, 8, 0, 1, text)
}
fn text_line(script: &mut String, py: u32, text: &str) {
    for (i, ch) in text.chars().enumerate() {
        let mut buf = [0u8; 4];
        let s = ch.encode_utf8(&mut buf);
        let w: u8 = if (ch as u32) >= 0x1100 { 2 } else { 1 };
        writeln!(script, "set {i} {py} {}", cell(0, 0, 8, 8, 8, 0, w, s)).unwrap();
    }
}

const RGB: i32 = 0x2000000;
const C256: i32 = 0x1000000;
const THEME: i32 = 0x4000000;

#[test]
fn storage_and_padding_match_c() {
    let mut s = String::new();
    s.push_str("new 10 3 5\n");
    writeln!(s, "set 2 0 {}", cell(1, 0, 1, 2, 8, 0, 1, "A")).unwrap();
    writeln!(
        s,
        "set 3 0 {}",
        cell(0, 64, C256 | 200, C256 | 17, 8, 0, 1, "B")
    )
    .unwrap();
    writeln!(s, "set 4 0 {}", cell(0x100, 0, 8, 8, 8, 0, 1, "C")).unwrap();
    writeln!(s, "set 5 0 {}", cell(0, 0, RGB | 0x123456, 8, 8, 0, 1, "D")).unwrap();
    writeln!(s, "set 6 0 {}", cell(0, 0, 8, THEME | 3, 8, 0, 1, "E")).unwrap();
    writeln!(s, "set 7 0 {}", cell(0, 0, 8, 8, C256 | 4, 0, 1, "F")).unwrap();
    writeln!(s, "set 8 0 {}", cell(0, 0, 8, 8, 8, 0, 1, "é")).unwrap();
    writeln!(s, "set 0 1 {}", cell(0, 0, 8, 8, 8, 0, 2, "中")).unwrap();
    s.push_str("pad 1 1 8\n");
    s.push_str("pad 2 1 4\n");
    writeln!(s, "set 3 1 {}", cell(0, 0, 8, 8, 8, 0, 2, "中")).unwrap();
    s.push_str("pad 3 1 8\n");
    writeln!(s, "set 4 1 {}", cell(0, 0, RGB | 7, 8, 8, 0, 1, "x")).unwrap();
    s.push_str("pad 4 1 8\n");
    writeln!(s, "tab 5 1 4 {}", cell(0, 0, 8, 3, 8, 0, 1, " ")).unwrap();
    writeln!(
        s,
        "cells 0 2 {} {}",
        cell(0, 0, 2, 8, 8, 0, 1, " "),
        hex(b"hello")
    )
    .unwrap();
    writeln!(s, "set 1 2 {}", cell(0, 0, RGB, 8, 8, 0, 1, "q")).unwrap();
    writeln!(
        s,
        "cells 0 2 {} {}",
        cell(0, 32, 8, 8, 8, 0, 1, " "),
        hex(b"ab\\")
    )
    .unwrap();
    s.push_str("dump\nlinelen 0\nlinelen 1\nlinelen 2\n");
    for px in 0..10 {
        writeln!(s, "inset {px} 1 {}", hex(b"\t ")).unwrap();
        writeln!(s, "inset {px} 1 {}", hex(b" ")).unwrap();
        writeln!(s, "inset {px} 1 {}", hex(b"\t")).unwrap();
        writeln!(s, "inset {px} 1 {}", hex("中-".as_bytes())).unwrap();
    }
    for flags in 0..32 {
        for py in 0..3 {
            writeln!(
                s,
                "resetlast\nstring 0 {py} 10 {flags} 1 0\nstring 0 {py} 10 {flags} 0 0"
            )
            .unwrap();
        }
    }
    compare(&s);
}

#[test]
fn expand_and_clear_match_c() {
    let mut s = String::new();
    s.push_str("new 80 4 0\n");
    for (px, py) in [(5, 0), (25, 1), (45, 2), (85, 3)] {
        writeln!(s, "set {px} {py} {}", plain("x")).unwrap();
    }
    s.push_str("dump\n");
    s.push_str("new 8 4 0\n");
    writeln!(s, "set 1 0 {}", cell(0, 0, RGB | 1, RGB | 2, 8, 0, 1, "a")).unwrap();
    writeln!(s, "set 2 0 {}", cell(0, 0, 8, 8, 8, 0, 1, "b")).unwrap();
    s.push_str(
        "clear 1 0 1 1 8\nclear 2 0 1 1 3\nclear 3 0 1 1 16777260\nclear 4 0 1 1 33554433\n",
    );
    s.push_str("clear 5 0 1 1 67108866\n");
    s.push_str("dump\n");
    writeln!(s, "set 0 1 {}", cell(0, 0, RGB | 1, 8, 8, 0, 1, "a")).unwrap();
    writeln!(s, "set 1 1 {}", cell(0, 0, RGB | 1, 8, 8, 0, 1, "b")).unwrap();
    s.push_str("movecells 3 0 1 2 8\ndump\nmovecells 0 3 1 2 5\ndump\n");
    s.push_str("clear 0 2 8 2 7\nclear 0 2 8 1 8\nclear 2 3 20 1 8\nclear 2 3 20 1 2\ndump\n");
    s.push_str("clear 9 0 1 1 8\nclear 9 0 1 1 1\ndump\n");
    compare(&s);
}

#[test]
fn history_and_scroll_match_c() {
    let mut s = String::new();
    s.push_str("new 6 3 20\nclock 1000 900\n");
    for i in 0..25 {
        writeln!(s, "set 0 {} {}", 2, plain(&format!("{}", i % 10))).unwrap();
        writeln!(s, "set 1 {} {}", 2, cell(0, 0, RGB | i, 8, 8, 0, 1, "r")).unwrap();
        s.push_str("collect 0\nscroll 8\ndump\n");
    }
    s.push_str("collect 1\ndump\nremovehist 3\ndump\nclock 0 0\nscroll 4\ndump\nclearhist\ndump\n");
    s.push_str("new 6 5 0\nclock 50 40\n");
    for py in 0..5 {
        writeln!(s, "set 0 {py} {}", plain(&py.to_string())).unwrap();
    }
    s.push_str("scrollregion 1 3 8\ndump\nscrollregion 1 1 2\ndump\nscrollregion 0 5 8\ndump\n");
    s.push_str("collect 1\ndump\ncollect 0\ndump\n");
    compare(&s);
}

#[test]
fn move_lines_match_c() {
    let mut s = String::new();
    s.push_str("new 4 8 0\n");
    for py in 0..8 {
        writeln!(s, "set 0 {py} {}", plain(&py.to_string())).unwrap();
    }
    for (dy, py, ny) in [
        (0, 2, 3),
        (5, 1, 2),
        (1, 3, 4),
        (3, 1, 4),
        (0, 4, 4),
        (4, 0, 4),
        (2, 2, 2),
        (7, 0, 3),
    ] {
        for py in 0..7 {
            writeln!(s, "set 1 {py} {}", cell(0, 0, 8, 8, 8, 0, 1, "w")).unwrap();
        }
        writeln!(s, "movelines {dy} {py} {ny} 8\ndump").unwrap();
    }
    s.push_str("movelines 1 0 2 3\ndump\n");
    compare(&s);
}

#[test]
fn string_cells_sgr_match_c() {
    let mut s = String::new();
    s.push_str("new 20 6 0\n");
    let attrs = [
        1u16, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 0x3fff, 0,
    ];
    for (i, a) in attrs.iter().enumerate() {
        writeln!(s, "set {i} 0 {}", cell(*a, 0, 8, 8, 8, 0, 1, "a")).unwrap();
    }
    let colours = [
        0,
        7,
        8,
        90,
        97,
        C256,
        C256 | 255,
        RGB | 0x010203,
        THEME,
        THEME | 8,
        THEME | 9,
        THEME | 200,
        5,
    ];
    for (i, c) in colours.iter().enumerate() {
        writeln!(s, "set {i} 1 {}", cell(0, 0, *c, 8, 8, 0, 1, "f")).unwrap();
        writeln!(s, "set {i} 2 {}", cell(0, 0, 8, *c, 8, 0, 1, "b")).unwrap();
        writeln!(s, "set {i} 3 {}", cell(0, 0, 8, 8, *c, 0, 1, "u")).unwrap();
    }
    writeln!(s, "set 0 4 {}", cell(128, 0, 8, 8, 8, 0, 1, "q")).unwrap();
    writeln!(s, "set 1 4 {}", cell(128 | 1, 0, 1, 8, 8, 0, 1, "\\")).unwrap();
    writeln!(s, "set 2 4 {}", cell(1, 0, 1, 8, 8, 0, 1, " ")).unwrap();
    writeln!(s, "set 3 4 {}", cell(0, 0, 8, 8, 8, 0, 1, " ")).unwrap();
    writeln!(s, "tab 4 4 3 {}", cell(0, 0, 8, 8, 8, 0, 1, " ")).unwrap();
    writeln!(s, "set 5 4 {}", cell(0, 0, 8, 8, RGB | 5, 0, 1, "u")).unwrap();
    writeln!(s, "set 6 4 {}", cell(0, 0, 8, 8, 8, 0, 1, "n")).unwrap();
    writeln!(s, "set 7 4 {}", cell(4, 0, 8, 8, C256 | 5, 0, 1, "n")).unwrap();
    writeln!(s, "set 8 4 {}", cell(0, 0, 8, 8, 8, 0, 1, " ")).unwrap();
    for flags in [1, 3, 5, 7, 17, 19, 21, 23] {
        writeln!(s, "resetlast").unwrap();
        for py in 0..6 {
            writeln!(s, "string 0 {py} 20 {flags} 1 1").unwrap();
        }
        for py in 0..6 {
            writeln!(s, "resetlast\nstring 0 {py} 20 {flags} 1 1").unwrap();
            writeln!(s, "string 2 {py} 3 {flags} 1 1").unwrap();
        }
        writeln!(s, "string 0 9 20 {flags} 1 1").unwrap();
    }
    s.push_str("vstring 0 4 20\nvstring 3 0 2\n");
    compare(&s);
}

#[test]
fn string_cells_hyperlinks_match_c() {
    let mut s = String::new();
    s.push_str("new 12 6 0\n");
    writeln!(s, "link {} {}", hex(b"http://a/"), hex(b"")).unwrap();
    writeln!(s, "link {} {}", hex(b"http://b/"), hex(b"id1")).unwrap();
    writeln!(s, "link {} {}", hex(b"http://c/\x01"), hex(b"i d")).unwrap();
    let long_id = "x".repeat(8192 - 17 - 9);
    writeln!(s, "link {} {}", hex(b"http://d/"), hex(long_id.as_bytes())).unwrap();
    let long_id2 = "y".repeat(8192 - 17 - 9 - 1);
    writeln!(s, "link {} {}", hex(b"http://e/"), hex(long_id2.as_bytes())).unwrap();
    let long_id3 = "z".repeat(8192 - 17 - 9 - 40);
    writeln!(s, "link {} {}", hex(b"http://f/"), hex(long_id3.as_bytes())).unwrap();
    // one-cell link with end close
    writeln!(s, "set 0 0 {}", cell(0, 0, 8, 8, 8, 1, 1, "a")).unwrap();
    // linked last cell with SGR delta
    writeln!(s, "set 0 1 {}", cell(0, 0, 8, 8, 8, 0, 1, "p")).unwrap();
    writeln!(s, "set 1 1 {}", cell(1, 0, 2, 8, 8, 2, 1, "q")).unwrap();
    // consecutive rows with the same link
    writeln!(s, "set 0 2 {}", cell(0, 0, 8, 8, 8, 2, 1, "r")).unwrap();
    writeln!(s, "set 1 2 {}", cell(0, 0, 8, 8, 8, 3, 1, "s")).unwrap();
    writeln!(s, "set 2 2 {}", cell(0, 0, 8, 8, 8, 0, 1, "t")).unwrap();
    writeln!(s, "set 3 2 {}", cell(0, 0, 8, 8, 8, 2, 1, "u")).unwrap();
    // unknown link
    writeln!(s, "set 0 3 {}", cell(0, 0, 8, 8, 8, 2, 1, "v")).unwrap();
    writeln!(s, "set 1 3 {}", cell(0, 0, 8, 8, 8, 99, 1, "w")).unwrap();
    writeln!(s, "set 2 3 {}", cell(0, 0, 8, 8, 8, 99, 1, "x")).unwrap();
    // near-limit links, raw and escaped
    writeln!(s, "set 0 4 {}", cell(1, 0, 1, 8, 8, 4, 1, "A")).unwrap();
    writeln!(s, "set 1 4 {}", cell(1, 0, 1, 8, 8, 5, 1, "B")).unwrap();
    writeln!(s, "set 2 4 {}", cell(2, 0, 2, 8, 8, 6, 1, "C")).unwrap();
    writeln!(s, "set 3 4 {}", cell(2, 0, 2, 8, 8, 6, 1, " ")).unwrap();
    writeln!(s, "set 0 5 {}", cell(0, 0, 8, 8, 8, 6, 1, "D")).unwrap();
    for flags in [1, 3, 5, 7, 17, 19, 21, 23] {
        writeln!(s, "resetlast").unwrap();
        for py in 0..6 {
            writeln!(s, "string 0 {py} 12 {flags} 1 1").unwrap();
        }
        for py in 0..6 {
            writeln!(s, "resetlast\nstring 0 {py} 12 {flags} 1 1").unwrap();
            writeln!(s, "string 0 {py} 12 {flags} 1 0").unwrap();
            writeln!(s, "string 0 {py} 12 {flags} 0 1").unwrap();
        }
    }
    compare(&s);
}

#[test]
fn reflow_cases_match_c() {
    let mut s = String::new();
    // Plain text split and join, with an empty wrapped line in the middle.
    s.push_str("new 10 5 100\n");
    text_line(&mut s, 0, "abcdefghij");
    text_line(&mut s, 1, "klm");
    text_line(&mut s, 3, "0123456789");
    text_line(&mut s, 4, "wide 中文 x");
    s.push_str("metadata 0 9 7 1 2 3 4 5\nmetadata 1 1 8 2 3 4 5 6\nmetadata 2 1 9 3 4 5 6 7\n");
    s.push_str("scroll 8\nscroll 8\n");
    s.push_str("dump\n");
    for sx in [4, 7, 10, 13, 25, 3, 80, 10] {
        for px in [0, 3, 9, 10, 12] {
            writeln!(s, "wrappos {px} 2").unwrap();
        }
        writeln!(s, "reflow {sx}\nsetsx {sx}\ndump").unwrap();
        s.push_str("unwrappos 4294967295 0\nunwrappos 4294967295 1\nunwrappos 2 1\nunwrappos 11 0\nunwrappos 0 3\n");
    }
    // Wrapped chains built by hand, including a trailing wide char that does not fit.
    s.push_str("new 6 6 50\n");
    text_line(&mut s, 0, "abcdef");
    text_line(&mut s, 1, "ghi中");
    text_line(&mut s, 2, "jk");
    text_line(&mut s, 4, "zz");
    writeln!(s, "set 0 5 {}", cell(0, 0, RGB | 9, 8, 8, 0, 1, "q")).unwrap();
    s.push_str("metadata 0 9 11 1 2 3 4 5\nmetadata 1 3 12 2 3 4 5 6\nmetadata 2 1 13 3 4 5 6 7\n");
    s.push_str("scroll 8\nscroll 8\nscroll 8\n");
    s.push_str("movelines 0 1 1 8\n"); // leaves a wrap flag edit path
    s.push_str("dump\n");
    for sx in [4, 9, 2, 12, 6] {
        writeln!(s, "reflow {sx}\nsetsx {sx}\ndump").unwrap();
    }
    compare(&s);
}

#[test]
fn reader_motions_match_c() {
    let mut s = String::new();
    s.push_str("new 12 6 0\n");
    text_line(&mut s, 0, "  foo-bar 中文x");
    text_line(&mut s, 1, "abc--  d");
    text_line(&mut s, 2, "   ");
    text_line(&mut s, 3, "wrappedline1");
    text_line(&mut s, 4, "cont\tt end ");
    writeln!(s, "tab 4 4 4 {}", plain(" ")).unwrap();
    text_line(&mut s, 5, "last");
    s.push_str("metadata 3 1 0 0 0 0 0 0\n");
    s.push_str("dump\n");
    let sep = hex(b"-");
    for (cx, cy) in [
        (0, 0),
        (3, 0),
        (10, 0),
        (11, 0),
        (12, 0),
        (0, 1),
        (5, 1),
        (0, 2),
        (2, 2),
        (11, 3),
        (0, 4),
        (4, 4),
        (5, 4),
        (3, 5),
        (12, 5),
    ] {
        for op in [
            "rright 0 0 0",
            "rright 1 0 0",
            "rright 0 1 0",
            "rright 0 0 1",
            "rleft 0",
            "rleft 1",
            "rdown",
            "rup",
            "rsol 0",
            "rsol 1",
            "reol 0 0",
            "reol 1 0",
            "reol 0 1",
            &format!("rnextword {sep}"),
            &format!("rnextwordend {sep}"),
            &format!("rprevword {sep} 0 0"),
            &format!("rprevword {sep} 1 0"),
            &format!("rprevword {sep} 0 1"),
            &format!("rprevword {} 0 0", hex("中".as_bytes())),
            &format!("rprevword {} 1 0", hex("中".as_bytes())),
            &format!("rprevword {sep} 1 1"),
            "rnextword ",
            "rnextwordend ",
            "rprevword  0 0",
            &format!("rjump {} 1", hex(b"a")),
            &format!("rjump {} 1", hex(b"\t")),
            &format!("rjump {} 2", hex("中".as_bytes())),
            &format!("rjumpback {} 1", hex(b"o")),
            &format!("rjumpback {} 1", hex(b"a")),
            &format!("rjumpback {} 1", hex(b"-")),
            "rindent",
            "rlinelen",
            &format!("rinset {}", hex(b"\t ")),
            &format!("rinset {}", hex(b"-")),
        ] {
            writeln!(s, "rstart {cx} {cy}\n{op}\nrcursor").unwrap();
        }
    }
    compare(&s);
}

#[test]
fn nul_sets_and_first_column_padding_match_c() {
    let mut s = String::from("new 4 1 0\n");
    writeln!(s, "set 0 0 {}", plain(" ")).unwrap();
    s.push_str("inset 0 0 0020\npad 0 0 8\ninset 0 0 20\n");
    writeln!(s, "set 0 0 {}", cell(0, 0, 8, 8, 8, 0, 1, "é")).unwrap();
    s.push_str("inset 0 0 c3a9\ninset 0 0 c3\n");
    s.push_str("set 0 0 0 0 8 8 8 0 1 ff\ninset 0 0 c3ff\n");
    compare(&s);
}

#[test]
fn reflow_and_unwrap_ignore_allocation_tail_match_c() {
    let mut s = String::from("new 4 2 10\n");
    text_line(&mut s, 0, "abcd");
    text_line(&mut s, 1, "ef");
    s.push_str("tail 2\nunwrappos 4294967295 2\nreflow 4\ndump\n");
    compare(&s);
}

#[test]
fn names_match_c() {
    let mut s = String::new();
    s.push_str("new 1 1 0\n");
    for i in 0..10 {
        writeln!(
            s,
            "names {} {} {}",
            1u32 << i,
            1u32 << (i % 8),
            1u32 << (i + 4)
        )
        .unwrap();
    }
    s.push_str("names 0 0 0\nnames 511 255 16383\nnames 65535 255 65535\nnames 1024 0 16384\n");
    compare(&s);
}

struct Gen {
    rng: common::Rng,
    m: Machine,
    links: Vec<u32>,
    script: String,
}

impl Gen {
    fn n(&mut self, bound: u32) -> u32 {
        if bound == 0 {
            0
        } else {
            (self.rng.next_u64() % u64::from(bound)) as u32
        }
    }
    fn colour(&mut self) -> i32 {
        match self.n(10) {
            0..=3 => 8,
            4 => self.n(8) as i32,
            5 => 90 + self.n(8) as i32,
            6 => C256 | self.n(256) as i32,
            7 => RGB | self.n(0x1000000) as i32,
            8 => THEME | self.n(12) as i32,
            _ => 9,
        }
    }
    fn us(&mut self) -> i32 {
        match self.n(6) {
            0..=2 => 8,
            3 => C256 | self.n(256) as i32,
            4 => RGB | self.n(0x1000000) as i32,
            _ => THEME | self.n(10) as i32,
        }
    }
    fn cellspec(&mut self) -> (String, u8) {
        let attr = if self.n(3) == 0 {
            self.n(0x4000) as u16
        } else {
            0
        };
        let flags = [0u8, 16, 32, 64, 48][self.n(5) as usize];
        let (fg, bg, us) = (self.colour(), self.colour(), self.us());
        let link = if !self.links.is_empty() && self.n(4) == 0 {
            let i = self.n(self.links.len() as u32) as usize;
            self.links[i]
        } else {
            0
        };
        let (text, width): (&str, u8) = match self.n(12) {
            0 => ("中", 2),
            1 => ("é", 1),
            2 => ("e\u{301}", 1),
            3 => ("😀", 2),
            4 => (" ", 1),
            5 => ("\\", 1),
            6 => ("-", 1),
            _ => (["a", "b", "x", "Z", "0", "."][self.n(6) as usize], 1),
        };
        (cell(attr, flags, fg, bg, us, link, width, text), width)
    }
    fn emit(&mut self, line: &str) {
        self.script.push_str(line);
        self.script.push('\n');
    }
    fn step(&mut self) {
        let sx = self.m.gd.sx();
        let sy = self.m.gd.sy();
        let total = self.m.gd.hsize() + sy;
        let bg = match self.n(4) {
            0 => 8,
            1 => self.n(8) as i32,
            2 => C256 | self.n(256) as i32,
            _ => RGB | self.n(0x1000000) as i32,
        };
        let line = match self.n(34) {
            0..=7 => {
                let (px, py) = (self.n(sx + 1), self.n(total));
                let (spec, w) = self.cellspec();
                if w == 2 && self.n(3) != 0 {
                    format!("set {px} {py} {spec}\npad {} {py} {bg}", px + 1)
                } else {
                    format!("set {px} {py} {spec}")
                }
            }
            8 => {
                let (px, py, w) = (self.n(sx), self.n(sy), 1 + self.n(8));
                let (spec, _) = self.cellspec();
                format!("tab {px} {} {w} {spec}", py + self.m.gd.hsize())
            }
            9 => {
                let (px, py) = (self.n(sx), self.n(sy));
                let (spec, _) = self.cellspec();
                let n = 1 + self.n(5) as usize;
                format!("vcells {px} {py} {spec} {}", hex(&b"hello world"[..n]))
            }
            10 => {
                let (px, py) = (self.n(sx + 1), self.n(sy));
                let (nx, ny) = (self.n(sx + 2), 1 + self.n(sy - py));
                format!("vclear {px} {py} {nx} {ny} {bg}")
            }
            11 => {
                let py = self.n(sy);
                format!(
                    "clearlines {} {} {bg}",
                    py + self.m.gd.hsize(),
                    1 + self.n(sy - py)
                )
            }
            12 => {
                let (u, l) = (self.n(sy), self.n(sy));
                let (u, l) = (u.min(l), u.max(l));
                if self.n(2) == 0 {
                    format!("vscrollup {u} {l} {bg}")
                } else {
                    format!("vscrolldown {u} {l} {bg}")
                }
            }
            13 => {
                let py = self.n(sy);
                let ny = 1 + self.n(sy - py);
                match self.n(4) {
                    0 => format!("vinslines {py} {ny} {bg}"),
                    1 => format!("vdellines {py} {ny} {bg}"),
                    2 => format!(
                        "vinslinesreg {} {py} {ny} {bg}",
                        py + ny - 1 + self.n(sy - py - ny + 1)
                    ),
                    _ => format!(
                        "vdellinesreg {} {py} {ny} {bg}",
                        py + ny - 1 + self.n(sy - py - ny + 1)
                    ),
                }
            }
            14 => {
                let (px, py) = (self.n(sx), self.n(sy));
                let nx = 1 + self.n(sx - px);
                if self.n(2) == 0 {
                    format!("vinscells {px} {py} {nx} {bg}")
                } else {
                    format!("vdelcells {px} {py} {nx} {bg}")
                }
            }
            15 => {
                let py = self.n(total);
                let (dx, px) = (self.n(sx), self.n(sx));
                format!("movecells {dx} {px} {py} {} {bg}", 1 + self.n(sx))
            }
            16 => format!("scroll {bg}"),
            17 => format!("collect {}", self.n(2)),
            18 => format!("removehist {}", self.n(3)),
            19 => "clearhist".into(),
            20 => format!("vclearhist {bg}"),
            21..=23 => {
                let nsx = 1 + self.n(14);
                format!("reflow {nsx}\nsetsx {nsx}")
            }
            24 => {
                let (px, py) = (self.n(sx + 1), self.n(total));
                let (nx, flags) = (1 + self.n(sx + 1), self.n(32));
                format!("string {px} {py} {nx} {flags} {} {}", self.n(2), self.n(2))
            }
            25 => {
                if self.n(3) == 0 {
                    "resetlast".into()
                } else {
                    let py = self.n(total);
                    format!(
                        "linelen {py}\ninset {} {py} {}",
                        self.n(sx),
                        hex([&b"\t "[..], b" ", b"-", "中".as_bytes(), b""][self.n(5) as usize])
                    )
                }
            }
            26 => {
                let uri = format!("http://h/{}", self.n(1000));
                let id = if self.n(2) == 0 {
                    String::new()
                } else {
                    format!("id{}", self.n(5))
                };
                format!("link {} {}", hex(uri.as_bytes()), hex(id.as_bytes()))
            }
            27 => {
                let (cx, cy) = (self.n(sx + 2), self.n(total));
                let sep = hex([&b"-"[..], b"", b"-_."][self.n(3) as usize]);
                let op = match self.n(16) {
                    0 => format!("rright {} {} {}", self.n(2), self.n(2), self.n(2)),
                    1 => format!("rleft {}", self.n(2)),
                    2 => "rdown".into(),
                    3 => "rup".into(),
                    4 => format!("rsol {}", self.n(2)),
                    5 => format!("reol {} {}", self.n(2), self.n(2)),
                    6 => format!("rnextword {sep}"),
                    7 => format!("rnextwordend {sep}"),
                    8 => format!("rprevword {sep} {} {}", self.n(2), self.n(2)),
                    9 => format!("rjump {} 1", hex(b"a")),
                    10 => format!("rjumpback {} 1", hex(b"a")),
                    11 => format!("rjump {} 2", hex("中".as_bytes())),
                    12 => format!("rjump {} 1", hex(b"\t")),
                    13 => "rindent".into(),
                    14 => format!("rinset {sep}"),
                    _ => "rlinelen".into(),
                };
                format!("rstart {cx} {cy}\n{op}\nrcursor")
            }
            28 => format!("clock {} {}", self.n(3) * 1000, 500),
            29 => {
                let py = self.n(total);
                format!("wrappos {} {py}", self.n(sx + 2))
            }
            _ => "dump".into(),
        };
        self.emit(&line);
        // Keep the generator's shadow machine in step so later bounds are valid.
        self.m.out.clear();
        self.m.run(&line);
        if line.starts_with("link") {
            self.links.push(self.m.out.trim().parse().unwrap());
        }
    }
}

#[test]
fn random_scripts_match_c() {
    let mut rng = common::Rng::new(0x9e37_79b9_7f4a_7c15);
    for _ in 0..400u32 {
        let (sx, sy) = (1 + rng.below(12) as u32, 1 + rng.below(6) as u32);
        let hl = [0, 0, 3, 8, 20][rng.below(5) as usize];
        let first = format!("new {sx} {sy} {hl}\n");
        let mut g = Gen {
            rng: common::Rng::new(rng.next_u64()),
            m: Machine::new(),
            links: Vec::new(),
            script: first.clone(),
        };
        g.m.run(&first);
        for _ in 0..80 {
            g.step();
        }
        g.emit("dump");
        compare(&g.script);
    }
}
