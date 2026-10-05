// Ported from tmux layout.c, layout-custom.c, layout-set.c @ 8f25579c (oracle differential)
/*
 * Copyright (c) 2007 Nicholas Marriott <nicholas.marriott@gmail.com>
 *
 * Permission to use, copy, modify, and distribute this software for any
 * purpose with or without fee is hereby granted, provided that the above
 * copyright notice and this permission notice appear in all copies.
 *
 * THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
 * WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
 * MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
 * ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
 * WHATSOEVER RESULTING FROM LOSS OF MIND, USE, DATA OR PROFITS, WHETHER
 * IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING
 * OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
 */

//! Differential tests against the oracle tmux (`oracle/bin/tmux`): the same
//! split, float, resize, select-layout and kill sequences run on a private
//! oracle server and on the layout model; after every step the v2 and v1
//! layout strings, the window size and every pane's geometry must match.
//! The tests skip with a message when the oracle binary is missing.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::Command;

use rmux_emu::screen::PaneLines;
use rmux_util::bytes::ByteString;

use super::fixture::FakeServer;
use super::*;
use crate::cmd::arguments::{Args, ArgsEntryFlags, ArgsValue};
use crate::ids::{ArenaId, PaneId, QueueItemId, WindowId};
use crate::model::spawn::SpawnFlags;
use crate::ui::scrollbar::{PaneScrollbarPolicy, PaneScrollbarPosition};
use crate::ui::status::PaneStatusPosition;

fn oracle_path() -> Option<PathBuf> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../oracle/bin/tmux");
    path.is_file().then_some(path)
}

struct Oracle {
    tmux: PathBuf,
    socket: PathBuf,
}

impl Oracle {
    fn start(sx: u32, sy: u32) -> Option<Self> {
        let tmux = oracle_path()?;
        let socket = std::env::temp_dir().join(format!(
            "rmux-layout-oracle-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let oracle = Self { tmux, socket };
        oracle
            .run(&[
                "-f",
                "/dev/null",
                "new-session",
                "-d",
                "-s",
                "L",
                "-x",
                &sx.to_string(),
                "-y",
                &sy.to_string(),
            ])
            .expect("oracle new-session");
        Some(oracle)
    }

    fn command(&self, control: bool) -> Command {
        let mut cmd = Command::new(&self.tmux);
        cmd.arg("-S").arg(&self.socket);
        if control {
            cmd.arg("-C");
        }
        cmd
    }

    /// Run a command; `Err` carries the trimmed stderr (the `cmdq_error`).
    fn run(&self, args: &[&str]) -> Result<String, String> {
        let out = self.command(false).args(args).output().expect("run oracle");
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    }

    fn format(&self, format: &str) -> String {
        self.run(&["display-message", "-p", "-t", "L:0", format])
            .unwrap()
    }

    /// `#{window_layout}` seen by a control client without new-layouts: v1.
    fn v1(&self) -> String {
        let out = self
            .command(true)
            .args(["display-message", "-p", "-t", "L:0", "#{window_layout}"])
            .output()
            .expect("run oracle -C");
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .find(|l| !l.starts_with('%'))
            .unwrap_or("")
            .to_string()
    }

    fn panes(&self) -> String {
        self.run(&[
            "list-panes",
            "-t",
            "L:0",
            "-F",
            "#{pane_id} #{pane_index} #{pane_left} #{pane_top} #{pane_width} #{pane_height} #{pane_active}",
        ])
        .unwrap()
    }
}

impl Drop for Oracle {
    fn drop(&mut self) {
        let _ = self.run(&["kill-server"]);
    }
}

/// One command of the corpus. Pane targets are list positions (`L:0.N`).
#[derive(Clone, Debug)]
enum Step {
    /// `split-window -d [-h] [-b] [-f] [-l N | -p N] -t L:0.N`
    Split {
        target: usize,
        horizontal: bool,
        before: bool,
        full: bool,
        size: Option<(char, String)>,
    },
    /// `new-pane -d [-x] [-y] [-X] [-Y] -t L:0.N`
    Float {
        target: usize,
        x: Option<String>,
        y: Option<String>,
        px: Option<String>,
        py: Option<String>,
    },
    /// `split-window -d [-h] [-b] [-f] -t <floating pane>`: a float split.
    FloatSplit {
        target: usize,
        horizontal: bool,
        before: bool,
        full: bool,
    },
    /// `resize-pane -{L,R,U,D} N -t L:0.N`
    ResizeDir {
        target: usize,
        flag: char,
        amount: i32,
    },
    /// `resize-pane -x N | -y N -t L:0.N`
    ResizeTo {
        target: usize,
        flag: char,
        size: u32,
    },
    ResizeWindow {
        sx: u32,
        sy: u32,
    },
    SelectLayout(&'static str),
    NextLayout,
    PreviousLayout,
    SelectPane(usize),
    KillPane(usize),
    PaneBorderStatus(&'static str),
    Scrollbars {
        on: bool,
        left: bool,
    },
    PaneBorderLines(&'static str),
}

struct Model {
    srv: FakeServer,
    w: WindowId,
}

fn item() -> QueueItemId {
    QueueItemId::from_parts(0, 0)
}

impl Model {
    fn new(sx: u32, sy: u32) -> Self {
        let mut srv = FakeServer::new();
        let (w, _) = srv.window(sx, sy);
        Self { srv, w }
    }

    /// `cmd-find` resolution of `L:0.N`.
    fn pane(&self, index: usize) -> Result<PaneId, String> {
        self.srv
            .win(self.w)
            .panes
            .get(index)
            .copied()
            .ok_or_else(|| format!("can't find pane: {index}"))
    }

    /// The window's `pane-border-lines`; every pane carries the same value.
    fn lines(&self) -> PaneLines {
        self.srv.pane(self.srv.win(self.w).panes[0]).lines
    }

    fn v2(&self) -> String {
        let root = self.srv.win(self.w).layout_root;
        dump(&self.srv, self.w, root, LayoutDumpFlags::default())
            .map(|s| String::from_utf8(s.into_vec()).unwrap())
            .unwrap_or_default()
    }

    fn v1(&self) -> String {
        let root = self.srv.win(self.w).layout_root;
        dump(&self.srv, self.w, root, LayoutDumpFlags::OLD_FORMAT)
            .map(|s| String::from_utf8(s.into_vec()).unwrap())
            .unwrap_or_default()
    }

    fn window_size(&self) -> String {
        let w = self.srv.win(self.w);
        format!("{}x{}", w.sx, w.sy)
    }

    fn panes(&self) -> String {
        let mut out = String::new();
        let w = self.srv.win(self.w);
        for (i, &wp) in w.panes.iter().enumerate() {
            let p = self.srv.pane(wp);
            let _ = writeln!(
                out,
                "%{} {} {} {} {} {} {}",
                p.public_id,
                i,
                p.xoff,
                p.yoff,
                p.sx,
                p.sy,
                u8::from(w.active == Some(wp))
            );
        }
        out.trim_end().to_string()
    }

    /// `spawn_pane` after `layout_get_*_cell`: add the pane and assign it.
    fn spawn(&mut self, target: PaneId, lc: LayoutCellId, flags: SpawnFlags) {
        let lines = self.lines();
        let new = self.srv.add_pane_after(self.w, target, flags);
        assign_pane(&mut self.srv, lc, new, false);
        if flags.contains(SpawnFlags::FLOATING) {
            self.srv.pane_mut(new).lines = lines;
            let cell = self.srv.pane(new).layout_cell.unwrap();
            self.srv
                .cells
                .get_mut(cell)
                .unwrap()
                .flags
                .insert(LayoutCellFlags::FLOATING);
        }
        self.srv
            .events
            .push((self.w, "window-layout-changed".into()));
    }

    /// `cmd-resize-window.c` with `window-size manual`: `resize_window`.
    fn resize_window(&mut self, sx: u32, sy: u32) {
        let sx = sx.clamp(1, WINDOW_MAXIMUM);
        let sy = sy.clamp(1, WINDOW_MAXIMUM);
        resize(&mut self.srv, self.w, sx, sy);
        let root = self.srv.win(self.w).layout_root.unwrap();
        let g = self.srv.cells.get(root).unwrap().g;
        self.srv.window_resize(self.w, sx.max(g.sx), sy.max(g.sy));
    }

    fn apply(&mut self, step: &Step) -> Result<(), String> {
        let cause = |e: LayoutError| String::from_utf8(e.cause.into_vec()).unwrap();
        match step {
            Step::Split {
                target,
                horizontal,
                before,
                full,
                size,
            } => {
                let wp = self.pane(*target)?;
                let mut flags = SpawnFlags::DETACHED | SpawnFlags::SPLIT;
                if *horizontal {
                    flags.insert(SpawnFlags::HORIZONTAL);
                }
                if *before {
                    flags.insert(SpawnFlags::BEFORE);
                }
                if *full {
                    flags.insert(SpawnFlags::FULLSIZE);
                }
                let mut args = Args::create();
                if let Some((flag, value)) = size {
                    args.set(
                        *flag as u8,
                        Some(ArgsValue::string(ByteString::from(value.as_str()))),
                        ArgsEntryFlags::default(),
                    );
                }
                if self.srv.pane_is_floating(wp) {
                    flags.insert(SpawnFlags::FLOATING);
                    let lines = self.lines();
                    let lc =
                        get_floating_cell(&mut self.srv, item(), &args, lines, self.w, wp, flags)
                            .map_err(cause)?;
                    self.spawn(wp, lc, flags);
                } else {
                    let lc = get_tiled_cell(&mut self.srv, item(), &args, self.w, wp, flags)
                        .map_err(cause)?;
                    self.spawn(wp, lc, flags);
                }
            }
            Step::FloatSplit {
                target,
                horizontal,
                before,
                full,
            } => {
                let wp = self.pane(*target)?;
                let mut flags = SpawnFlags::DETACHED | SpawnFlags::SPLIT | SpawnFlags::FLOATING;
                if *horizontal {
                    flags.insert(SpawnFlags::HORIZONTAL);
                }
                if *before {
                    flags.insert(SpawnFlags::BEFORE);
                }
                if *full {
                    flags.insert(SpawnFlags::FULLSIZE);
                }
                let lines = self.lines();
                let lc = get_floating_cell(
                    &mut self.srv,
                    item(),
                    &Args::create(),
                    lines,
                    self.w,
                    wp,
                    flags,
                )
                .map_err(cause)?;
                self.spawn(wp, lc, flags);
            }
            Step::Float {
                target,
                x,
                y,
                px,
                py,
            } => {
                let wp = self.pane(*target)?;
                let flags = SpawnFlags::DETACHED | SpawnFlags::FLOATING;
                let mut args = Args::create();
                for (flag, value) in [(b'x', x), (b'y', y), (b'X', px), (b'Y', py)] {
                    if let Some(value) = value {
                        args.set(
                            flag,
                            Some(ArgsValue::string(ByteString::from(value.as_str()))),
                            ArgsEntryFlags::default(),
                        );
                    }
                }
                let lines = self.lines();
                let lc = get_floating_cell(&mut self.srv, item(), &args, lines, self.w, wp, flags)
                    .map_err(cause)?;
                self.spawn(wp, lc, flags);
            }
            Step::ResizeDir {
                target,
                flag,
                amount,
            } => {
                let wp = self.pane(*target)?;
                let kind = if matches!(flag, 'L' | 'R') {
                    LayoutType::Leftright
                } else {
                    LayoutType::Topbottom
                };
                if self.srv.pane_is_floating(wp) {
                    let opposite = matches!(flag, 'L' | 'U');
                    resize_floating_pane(&mut self.srv, wp, kind, *amount, opposite)
                        .map_err(|e| format!("adjustment {}", cause(e)))?;
                } else {
                    let adjust = if matches!(flag, 'L' | 'U') {
                        -*amount
                    } else {
                        *amount
                    };
                    resize_pane(&mut self.srv, wp, kind, adjust, true);
                }
                self.after_resize_pane(wp);
            }
            Step::ResizeTo { target, flag, size } => {
                let wp = self.pane(*target)?;
                let kind = if *flag == 'x' {
                    LayoutType::Leftright
                } else {
                    LayoutType::Topbottom
                };
                // cmd-resize-pane.c:98-142: the size is a percentage of the
                // window in [0, PANE_MAXIMUM]; -y adds the status row for the
                // top or bottom pane.
                let (wsx, wsy) = self.srv.window_size(self.w);
                let what = if *flag == 'x' { "width" } else { "height" };
                let mut size = crate::cmd::arguments::string_percentage(
                    size.to_string().as_bytes(),
                    0,
                    PANE_MAXIMUM as i64,
                    if *flag == 'x' { wsx } else { wsy } as i64,
                )
                .map_err(|e| format!("{what} {e}"))? as u32;
                if *flag == 'y' {
                    let (_, yoff, _, sy) = self.srv.pane_geometry(wp);
                    match self.srv.window_pane_status(self.w) {
                        PaneStatusPosition::Top if yoff == 1 => size += 1,
                        PaneStatusPosition::Bottom if yoff + sy as i32 == wsy as i32 - 1 => {
                            size += 1
                        }
                        _ => {}
                    }
                }
                if self.srv.pane_is_floating(wp) {
                    resize_floating_pane_to(&mut self.srv, wp, kind, size)
                        .map_err(|e| format!("size {}", cause(e)))?;
                } else {
                    resize_pane_to(&mut self.srv, wp, kind, size);
                }
                self.after_resize_pane(wp);
            }
            Step::ResizeWindow { sx, sy } => self.resize_window(*sx, *sy),
            Step::SelectLayout(name) => {
                let index = set_lookup(name.as_bytes())
                    .ok_or_else(|| format!("unknown layout or ambiguous: {name}"))?;
                set_select(&mut self.srv, self.w, u32::from(index.0));
            }
            Step::NextLayout => {
                set_next(&mut self.srv, self.w);
            }
            Step::PreviousLayout => {
                set_previous(&mut self.srv, self.w);
            }
            Step::SelectPane(target) => {
                let wp = self.pane(*target)?;
                self.srv.select_pane(wp);
            }
            Step::KillPane(target) => {
                let wp = self.pane(*target)?;
                close_pane(&mut self.srv, wp);
                self.srv.remove_pane(wp);
            }
            Step::PaneBorderStatus(value) => {
                self.srv.win_mut(self.w).pane_status = match *value {
                    "top" => PaneStatusPosition::Top,
                    "bottom" => PaneStatusPosition::Bottom,
                    _ => PaneStatusPosition::Off,
                };
                fix_panes(&mut self.srv, self.w, None);
            }
            Step::Scrollbars { on, left } => {
                let win = self.srv.win_mut(self.w);
                win.sb = if *on {
                    PaneScrollbarPolicy::Always
                } else {
                    PaneScrollbarPolicy::Off
                };
                win.sb_pos = if *left {
                    PaneScrollbarPosition::Left
                } else {
                    PaneScrollbarPosition::Right
                };
                fix_panes(&mut self.srv, self.w, None);
            }
            Step::PaneBorderLines(value) => {
                let lines = if *value == "none" {
                    PaneLines::None
                } else {
                    PaneLines::Single
                };
                let panes = self.srv.win(self.w).panes.clone();
                for wp in panes {
                    self.srv.pane_mut(wp).lines = lines;
                }
            }
        }
        Ok(())
    }

    /// `cmd-resize-pane.c:184-188`.
    fn after_resize_pane(&mut self, wp: PaneId) {
        let lc = self.srv.pane(wp).layout_cell.unwrap();
        if self.srv.cells.get(lc).unwrap().parent.is_some() {
            fix_offsets(&mut self.srv, self.w);
        }
        fix_panes(&mut self.srv, self.w, None);
    }
}

fn oracle_args(step: &Step) -> Vec<String> {
    let t = |i: &usize| format!("L:0.{i}");
    let mut v: Vec<String> = Vec::new();
    match step {
        Step::Split {
            target,
            horizontal,
            before,
            full,
            size,
        } => {
            v.extend(["split-window".into(), "-d".into()]);
            if *horizontal {
                v.push("-h".into());
            }
            if *before {
                v.push("-b".into());
            }
            if *full {
                v.push("-f".into());
            }
            if let Some((flag, value)) = size {
                v.push(format!("-{flag}"));
                v.push(value.clone());
            }
            v.extend(["-t".into(), t(target)]);
        }
        Step::FloatSplit {
            target,
            horizontal,
            before,
            full,
        } => {
            v.extend(["split-window".into(), "-d".into()]);
            if *horizontal {
                v.push("-h".into());
            }
            if *before {
                v.push("-b".into());
            }
            if *full {
                v.push("-f".into());
            }
            v.extend(["-t".into(), t(target)]);
        }
        Step::Float {
            target,
            x,
            y,
            px,
            py,
        } => {
            v.extend(["new-pane".into(), "-d".into()]);
            for (flag, value) in [("-x", x), ("-y", y), ("-X", px), ("-Y", py)] {
                if let Some(value) = value {
                    v.push(flag.into());
                    v.push(value.clone());
                }
            }
            v.extend(["-t".into(), t(target)]);
        }
        Step::ResizeDir {
            target,
            flag,
            amount,
        } => {
            v.extend([
                "resize-pane".into(),
                format!("-{flag}"),
                amount.to_string(),
                "-t".into(),
                t(target),
            ]);
        }
        Step::ResizeTo { target, flag, size } => {
            v.extend([
                "resize-pane".into(),
                format!("-{flag}"),
                size.to_string(),
                "-t".into(),
                t(target),
            ]);
        }
        Step::ResizeWindow { sx, sy } => {
            v.extend([
                "resize-window".into(),
                "-x".into(),
                sx.to_string(),
                "-y".into(),
                sy.to_string(),
                "-t".into(),
                "L:0".into(),
            ]);
        }
        Step::SelectLayout(name) => v.extend([
            "select-layout".into(),
            "-t".into(),
            "L:0".into(),
            (*name).into(),
        ]),
        Step::NextLayout => v.extend(["next-layout".into(), "-t".into(), "L:0".into()]),
        Step::PreviousLayout => v.extend(["previous-layout".into(), "-t".into(), "L:0".into()]),
        Step::SelectPane(target) => v.extend(["select-pane".into(), "-t".into(), t(target)]),
        Step::KillPane(target) => v.extend(["kill-pane".into(), "-t".into(), t(target)]),
        Step::PaneBorderStatus(value) => {
            v.extend([
                "set-option".into(),
                "-w".into(),
                "-t".into(),
                "L:0".into(),
                "pane-border-status".into(),
                (*value).into(),
            ]);
        }
        Step::Scrollbars { on, left } => {
            v.extend([
                "set-option".into(),
                "-w".into(),
                "-t".into(),
                "L:0".into(),
                "pane-scrollbars".into(),
                if *on { "on" } else { "off" }.into(),
                ";".into(),
                "set-option".into(),
                "-w".into(),
                "-t".into(),
                "L:0".into(),
                "pane-scrollbars-position".into(),
                if *left { "left" } else { "right" }.into(),
            ]);
        }
        Step::PaneBorderLines(value) => {
            v.extend([
                "set-option".into(),
                "-w".into(),
                "-t".into(),
                "L:0".into(),
                "pane-border-lines".into(),
                (*value).into(),
            ]);
        }
    }
    v
}

/// Run `steps` on both sides and compare after every step. Returns the
/// number of steps compared, or `None` when the oracle is missing.
fn differential(name: &str, sx: u32, sy: u32, steps: &[Step]) -> Option<usize> {
    let oracle = Oracle::start(sx, sy)?;
    let mut model = Model::new(sx, sy);
    for (n, step) in steps.iter().enumerate() {
        let args = oracle_args(step);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let expected = oracle.run(&refs);
        let actual = model.apply(step);
        let ctx = format!("{name} {sx}x{sy} step {n}: {step:?}");
        match (&expected, &actual) {
            (Ok(_), Ok(())) => {}
            (Err(e), Err(a)) => assert_eq!(a, e, "{ctx}: error cause"),
            (Err(e), Ok(())) => panic!("{ctx}: oracle failed ({e}) but the model succeeded"),
            (Ok(_), Err(a)) => panic!("{ctx}: model failed ({a}) but the oracle succeeded"),
        }
        assert_eq!(
            model.v2(),
            oracle.format("#{window_layout}"),
            "{ctx}: window_layout"
        );
        assert_eq!(
            model.v2(),
            oracle.format("#{window_visible_layout}"),
            "{ctx}: window_visible_layout"
        );
        assert_eq!(model.v1(), oracle.v1(), "{ctx}: v1 layout");
        assert_eq!(
            model.window_size(),
            oracle.format("#{window_width}x#{window_height}"),
            "{ctx}: window size"
        );
        assert_eq!(model.panes(), oracle.panes(), "{ctx}: pane geometry");
    }
    Some(steps.len())
}

fn skip() -> bool {
    if oracle_path().is_none() {
        eprintln!("skipping: oracle tmux not found at oracle/bin/tmux");
        return true;
    }
    false
}

fn split(target: usize, horizontal: bool) -> Step {
    Step::Split {
        target,
        horizontal,
        before: false,
        full: false,
        size: None,
    }
}

fn split_size(
    target: usize,
    horizontal: bool,
    before: bool,
    full: bool,
    flag: char,
    value: &str,
) -> Step {
    Step::Split {
        target,
        horizontal,
        before,
        full,
        size: Some((flag, value.to_string())),
    }
}

fn float(
    target: usize,
    x: Option<&str>,
    y: Option<&str>,
    px: Option<&str>,
    py: Option<&str>,
) -> Step {
    Step::Float {
        target,
        x: x.map(str::to_string),
        y: y.map(str::to_string),
        px: px.map(str::to_string),
        py: py.map(str::to_string),
    }
}

/// §6.2 item 1 and 2: 2 to 9 panes with mixed splits, dumps after each step,
/// then every preset, next/previous seven times each.
fn mixed_splits() -> Vec<Step> {
    let mut steps = vec![
        split(0, false),
        split(1, true),
        split_size(0, true, false, false, 'l', "30"),
        split_size(2, false, true, false, 'p', "40"),
        split_size(1, false, false, true, 'l', "5"),
        split_size(3, true, true, true, 'l', "25%"),
        split(4, false),
        split_size(5, true, false, false, 'l', "7"),
        Step::SelectPane(3),
        Step::SelectPane(1),
    ];
    for name in [
        "even-horizontal",
        "even-vertical",
        "main-horizontal",
        "main-horizontal-mirrored",
        "main-vertical",
        "main-vertical-mirrored",
        "tiled",
        "even-h",
        "main-vertical-",
    ] {
        steps.push(Step::SelectLayout(name));
    }
    steps.extend(std::iter::repeat_n(Step::NextLayout, 7));
    steps.extend(std::iter::repeat_n(Step::PreviousLayout, 7));
    steps.push(Step::KillPane(2));
    steps.push(Step::SelectLayout("tiled"));
    steps.push(Step::KillPane(0));
    steps.push(Step::SelectLayout("main-horizontal"));
    steps
}

#[test]
fn oracle_mixed_splits_and_presets() {
    if skip() {
        return;
    }
    for (sx, sy) in [(80, 24), (100, 40), (10, 6)] {
        differential("mixed", sx, sy, &mixed_splits()).unwrap();
    }
}

/// §6.2 item 3: resize-pane in every direction, beyond the limit, and -x/-y.
#[test]
fn oracle_resize_pane_sequences() {
    if skip() {
        return;
    }
    let steps = vec![
        split(0, true),
        split(1, false),
        split(0, false),
        Step::ResizeDir {
            target: 0,
            flag: 'R',
            amount: 5,
        },
        Step::ResizeDir {
            target: 0,
            flag: 'D',
            amount: 3,
        },
        Step::ResizeDir {
            target: 2,
            flag: 'L',
            amount: 7,
        },
        Step::ResizeDir {
            target: 3,
            flag: 'U',
            amount: 2,
        },
        Step::ResizeDir {
            target: 1,
            flag: 'R',
            amount: 200,
        },
        Step::ResizeDir {
            target: 1,
            flag: 'L',
            amount: 200,
        },
        Step::ResizeDir {
            target: 3,
            flag: 'D',
            amount: 100,
        },
        Step::ResizeDir {
            target: 2,
            flag: 'U',
            amount: 100,
        },
        Step::ResizeTo {
            target: 0,
            flag: 'x',
            size: 20,
        },
        Step::ResizeTo {
            target: 0,
            flag: 'y',
            size: 3,
        },
        Step::ResizeTo {
            target: 3,
            flag: 'x',
            size: 10,
        },
        Step::ResizeTo {
            target: 2,
            flag: 'y',
            size: 30,
        },
        Step::ResizeTo {
            target: 1,
            flag: 'x',
            size: 1,
        },
        Step::ResizeDir {
            target: 1,
            flag: 'R',
            amount: 1,
        },
        Step::PaneBorderStatus("top"),
        Step::ResizeDir {
            target: 0,
            flag: 'D',
            amount: 50,
        },
        Step::ResizeDir {
            target: 2,
            flag: 'U',
            amount: 50,
        },
        Step::PaneBorderStatus("bottom"),
        Step::ResizeTo {
            target: 3,
            flag: 'y',
            size: 1,
        },
        Step::PaneBorderStatus("off"),
        Step::Scrollbars {
            on: true,
            left: false,
        },
        Step::ResizeDir {
            target: 0,
            flag: 'L',
            amount: 100,
        },
        Step::ResizeTo {
            target: 2,
            flag: 'x',
            size: 4,
        },
        Step::Scrollbars {
            on: true,
            left: true,
        },
        Step::ResizeDir {
            target: 1,
            flag: 'R',
            amount: 100,
        },
        split(0, true),
        Step::Scrollbars {
            on: false,
            left: false,
        },
    ];
    for (sx, sy) in [(80, 24), (100, 40)] {
        differential("resize", sx, sy, &steps).unwrap();
    }
}

/// §6.2 item 4: resize-window to 20x5 and back to 200x50 over a 3x3 tiled
/// layout, plus floats clamped by the window.
#[test]
fn oracle_resize_window() {
    if skip() {
        return;
    }
    let mut steps: Vec<Step> = (0..8).map(|_| split(0, false)).collect();
    steps.push(Step::SelectLayout("tiled"));
    steps.push(float(0, Some("30"), Some("8"), Some("60"), Some("30")));
    steps.push(Step::ResizeWindow { sx: 20, sy: 5 });
    steps.push(Step::ResizeWindow { sx: 200, sy: 50 });
    steps.push(Step::ResizeWindow { sx: 1, sy: 1 });
    steps.push(Step::ResizeWindow { sx: 80, sy: 24 });
    steps.push(Step::SelectLayout("even-vertical"));
    steps.push(Step::ResizeWindow { sx: 40, sy: 10 });
    steps.push(Step::ResizeWindow { sx: 120, sy: 60 });
    differential("resize-window", 80, 24, &steps).unwrap();
}

/// §6.2 item 5: floating panes with every combination of -x -y -X -Y, with
/// pane-border-lines none and default, resized, then a smaller window.
#[test]
fn oracle_floating_panes() {
    if skip() {
        return;
    }
    for lines in ["single", "none"] {
        let mut steps = vec![Step::PaneBorderLines(lines), split(0, false)];
        let x = ["20", "50%"];
        let y = ["6", "25%"];
        for i in 0..16u32 {
            steps.push(float(
                0,
                (i & 1 != 0).then_some(x[(i as usize / 2) % 2]),
                (i & 2 != 0).then_some(y[(i as usize / 4) % 2]),
                (i & 4 != 0).then_some("8"),
                (i & 8 != 0).then_some("3"),
            ));
        }
        steps.push(float(0, Some("30"), Some("10"), Some("-5"), Some("-2")));
        steps.push(float(0, Some("30"), Some("10"), Some("70"), Some("20")));
        steps.push(float(0, Some("200"), Some("100"), None, None));
        steps.push(Step::SelectPane(3));
        steps.push(Step::SelectPane(5));
        steps.push(Step::ResizeDir {
            target: 2,
            flag: 'R',
            amount: 5,
        });
        steps.push(Step::ResizeDir {
            target: 2,
            flag: 'L',
            amount: 3,
        });
        steps.push(Step::ResizeDir {
            target: 2,
            flag: 'U',
            amount: 2,
        });
        steps.push(Step::ResizeDir {
            target: 3,
            flag: 'D',
            amount: 1000,
        });
        steps.push(Step::ResizeTo {
            target: 4,
            flag: 'x',
            size: 10,
        });
        steps.push(Step::ResizeTo {
            target: 4,
            flag: 'y',
            size: 2,
        });
        steps.push(Step::ResizeTo {
            target: 4,
            flag: 'y',
            size: 20000,
        });
        steps.push(Step::FloatSplit {
            target: 2,
            horizontal: false,
            before: false,
            full: false,
        });
        steps.push(Step::FloatSplit {
            target: 2,
            horizontal: true,
            before: true,
            full: false,
        });
        steps.push(Step::FloatSplit {
            target: 3,
            horizontal: true,
            before: false,
            full: true,
        });
        steps.push(Step::FloatSplit {
            target: 3,
            horizontal: false,
            before: true,
            full: true,
        });
        steps.push(Step::ResizeWindow { sx: 40, sy: 12 });
        steps.push(Step::ResizeWindow { sx: 6, sy: 4 });
        steps.push(Step::ResizeWindow { sx: 100, sy: 40 });
        steps.push(Step::KillPane(0));
        steps.push(Step::KillPane(0));
        steps.push(Step::SelectLayout("tiled"));
        steps.push(Step::ResizeWindow { sx: 50, sy: 20 });
        differential(&format!("float-{lines}"), 80, 24, &steps).unwrap();
    }
}

/// Error causes of the command-facing paths (`layout.c:1640-1833`).
#[test]
fn oracle_error_causes() {
    if skip() {
        return;
    }
    let steps = vec![
        split_size(0, false, false, false, 'l', "abc"),
        split_size(0, false, false, false, 'p', "150"),
        split_size(0, false, false, false, 'l', "-5"),
        float(0, Some("0"), None, None, None),
        float(0, None, Some("2"), None, None),
        float(0, Some("x"), None, None, None),
        float(0, None, None, Some("-500"), None),
        float(0, None, None, None, Some("500")),
        float(0, Some("20"), Some("6"), None, None),
        split(1, false),
        Step::ResizeDir {
            target: 1,
            flag: 'L',
            amount: 100,
        },
        Step::ResizeTo {
            target: 1,
            flag: 'x',
            size: 20000,
        },
        Step::ResizeTo {
            target: 1,
            flag: 'y',
            size: 2,
        },
        Step::ResizeWindow { sx: 3, sy: 3 },
        split(0, true),
        split(0, true),
        split(0, false),
        Step::FloatSplit {
            target: 1,
            horizontal: true,
            before: false,
            full: false,
        },
    ];
    differential("errors", 80, 24, &steps).unwrap();
}

/// A seeded random walk over the whole command set.
#[test]
fn oracle_random_walk() {
    if skip() {
        return;
    }
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = move |n: u64| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % n) as usize
    };
    for round in 0..6 {
        let (sx, sy) = [(80, 24), (100, 40), (24, 8)][round % 3];
        let mut steps = Vec::new();
        let mut npanes = 1usize;
        for _ in 0..40 {
            let target = next(npanes as u64);
            let step = match next(14) {
                0 | 1 => {
                    npanes += 1;
                    split(target, next(2) == 0)
                }
                2 => {
                    npanes += 1;
                    let flag = if next(2) == 0 { 'l' } else { 'p' };
                    let value = if flag == 'l' {
                        format!("{}", 1 + next(30))
                    } else {
                        format!("{}", 5 + next(90))
                    };
                    Step::Split {
                        target,
                        horizontal: next(2) == 0,
                        before: next(2) == 0,
                        full: next(3) == 0,
                        size: Some((flag, value)),
                    }
                }
                3 => {
                    npanes += 1;
                    float(
                        target,
                        (next(2) == 0).then_some("20"),
                        (next(2) == 0).then_some("6"),
                        (next(2) == 0).then_some("5"),
                        (next(2) == 0).then_some("3"),
                    )
                }
                4 => Step::ResizeDir {
                    target,
                    flag: ['L', 'R', 'U', 'D'][next(4)],
                    amount: 1 + next(12) as i32,
                },
                5 => Step::ResizeTo {
                    target,
                    flag: ['x', 'y'][next(2)],
                    size: 1 + next(40) as u32,
                },
                6 => Step::SelectLayout(
                    [
                        "even-horizontal",
                        "even-vertical",
                        "main-horizontal",
                        "main-horizontal-mirrored",
                        "main-vertical",
                        "main-vertical-mirrored",
                        "tiled",
                    ][next(7)],
                ),
                7 => Step::NextLayout,
                8 => Step::PreviousLayout,
                9 => Step::SelectPane(target),
                10 => {
                    if npanes > 1 {
                        npanes -= 1;
                        Step::KillPane(target)
                    } else {
                        Step::NextLayout
                    }
                }
                11 => Step::ResizeWindow {
                    sx: 5 + next(150) as u32,
                    sy: 3 + next(50) as u32,
                },
                12 => Step::PaneBorderStatus(["off", "top", "bottom"][next(3)]),
                _ => Step::Scrollbars {
                    on: next(2) == 0,
                    left: next(2) == 0,
                },
            };
            steps.push(step);
        }
        // A step may fail on both sides (no space); the pane count then
        // drifts from `npanes`, so keep targets in range by clamping.
        let steps = clamp_targets(steps);
        differential(&format!("random-{round}"), sx, sy, &steps).unwrap();
    }
}

/// Replay the step list on the model alone to keep every pane target inside
/// the current pane list (failed splits do not add a pane).
fn clamp_targets(steps: Vec<Step>) -> Vec<Step> {
    let mut probe = Model::new(200, 100);
    let mut out = Vec::new();
    for mut step in steps {
        let n = probe.srv.win(probe.w).panes.len();
        match &mut step {
            Step::Split { target, .. }
            | Step::FloatSplit { target, .. }
            | Step::Float { target, .. }
            | Step::ResizeDir { target, .. }
            | Step::ResizeTo { target, .. }
            | Step::SelectPane(target)
            | Step::KillPane(target) => *target %= n,
            _ => {}
        }
        if matches!(step, Step::KillPane(_)) && n == 1 {
            continue;
        }
        let _ = probe.apply(&step);
        out.push(step);
    }
    out
}
