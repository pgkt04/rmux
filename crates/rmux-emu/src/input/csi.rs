// Ported from tmux input.c @ 8f25579c
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

//! CSI dispatch: cursor, erase, modes, WINOPS, DECRQM, DSR, DA
//! (`input.c:1462-2218`).

use super::c0_esc::table_lookup;
use super::{Env, ExtendedKeys, FLAG_DISCARD, FLAG_LAST, Flow, InputCtx, Pending, Sub};
use crate::cell::GridAttributes;
use crate::screen::{ScreenCursorStyle, ScreenMode, set_cursor_style};
use std::io::Write;

/// `getversion()` at the pin (`configure.ac:3`).
pub const TMUX_VERSION: &str = "next-3.9";

/// `SIXEL_COLOUR_REGISTERS` (`tmux.h`).
const SIXEL_COLOUR_REGISTERS: u32 = 1024;

/// `enum input_csi_type` (`input.c:258-300`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Csi {
    Cbt,
    Cnl,
    Cpl,
    Cub,
    Cud,
    Cuf,
    Cup,
    Cuu,
    Da,
    DaTwo,
    Dch,
    Decscusr,
    Decstbm,
    Dl,
    Dsr,
    DsrPrivate,
    Ech,
    Ed,
    El,
    Hpa,
    Ich,
    Il,
    Modoff,
    Modset,
    Query,
    QueryPrivate,
    Rcp,
    Rep,
    Rm,
    RmPrivate,
    Scp,
    Sd,
    Sgr,
    Sm,
    SmGraphics,
    SmPrivate,
    Su,
    Tbc,
    Vpa,
    Winops,
    Xda,
}

/// `input_csi_table` (`input.c:303-347`).
const CSI_TABLE: [(u8, &[u8], Csi); 43] = [
    (b'@', b"", Csi::Ich),
    (b'A', b"", Csi::Cuu),
    (b'B', b"", Csi::Cud),
    (b'C', b"", Csi::Cuf),
    (b'D', b"", Csi::Cub),
    (b'E', b"", Csi::Cnl),
    (b'F', b"", Csi::Cpl),
    (b'G', b"", Csi::Hpa),
    (b'H', b"", Csi::Cup),
    (b'J', b"", Csi::Ed),
    (b'K', b"", Csi::El),
    (b'L', b"", Csi::Il),
    (b'M', b"", Csi::Dl),
    (b'P', b"", Csi::Dch),
    (b'S', b"", Csi::Su),
    (b'S', b"?", Csi::SmGraphics),
    (b'T', b"", Csi::Sd),
    (b'X', b"", Csi::Ech),
    (b'Z', b"", Csi::Cbt),
    (b'`', b"", Csi::Hpa),
    (b'b', b"", Csi::Rep),
    (b'c', b"", Csi::Da),
    (b'c', b">", Csi::DaTwo),
    (b'd', b"", Csi::Vpa),
    (b'f', b"", Csi::Cup),
    (b'g', b"", Csi::Tbc),
    (b'h', b"", Csi::Sm),
    (b'h', b"?", Csi::SmPrivate),
    (b'l', b"", Csi::Rm),
    (b'l', b"?", Csi::RmPrivate),
    (b'm', b"", Csi::Sgr),
    (b'm', b">", Csi::Modset),
    (b'n', b"", Csi::Dsr),
    (b'n', b">", Csi::Modoff),
    (b'n', b"?", Csi::DsrPrivate),
    (b'p', b"$", Csi::Query),
    (b'p', b"?$", Csi::QueryPrivate),
    (b'q', b" ", Csi::Decscusr),
    (b'q', b">", Csi::Xda),
    (b'r', b"", Csi::Decstbm),
    (b's', b"", Csi::Scp),
    (b't', b"", Csi::Winops),
    (b'u', b"", Csi::Rcp),
];

fn flag(mode: ScreenMode, bit: ScreenMode) -> i32 {
    if mode.contains(bit) { 1 } else { 2 }
}

trait ModeToggle {
    fn mode_toggle(&mut self, mode: ScreenMode, set: bool);
}

impl ModeToggle for crate::screen::write::ScreenWriteCtx<'_> {
    fn mode_toggle(&mut self, mode: ScreenMode, set: bool) {
        if set {
            self.mode_set(mode);
        } else {
            self.mode_clear(mode);
        }
    }
}
impl InputCtx {
    /// Stage a queued reply (`input_reply(ictx, 1, ...)`).
    pub(super) fn reply(&mut self, args: std::fmt::Arguments<'_>) -> Flow {
        self.scratch.clear();
        self.scratch.write_fmt(args).expect("reply fits");
        Flow::Yield(Pending::Reply, Sub::Done)
    }

    /// `input_csi_dispatch` (`input.c:1463-1895`). The split, the lookup and
    /// the `INPUT_LAST` clear run once on `Sub::Start`; the mode and WINOPS
    /// loops resume at their parameter index.
    pub(super) fn csi_dispatch(&mut self, sub: Sub, env: &mut Env<'_, '_>) -> Flow {
        match sub {
            Sub::Modes { set, i } => return self.csi_modes_private(set, usize::from(i), env),
            Sub::Winops { m } => return self.csi_winops(usize::from(m), env),
            Sub::Start => {}
            _ => return Flow::Done,
        }
        if self.flags & FLAG_DISCARD != 0 {
            return Flow::Done;
        }
        if self.params.split(self.param_buf.as_slice()).is_err() {
            return Flow::Done;
        }
        let Some(entry) = table_lookup(&CSI_TABLE, self.ch, self.interm.as_slice()) else {
            return Flow::Done;
        };
        // REP reads INPUT_LAST; every known command clears it on the way out
        // (input.c:1893) and nothing in between observes it.
        let had_last = self.flags & FLAG_LAST != 0;
        self.flags &= !FLAG_LAST;
        let bg = self.cell.cell.bg;
        let sw = &mut *env.sw;
        let s = &mut *sw.screen;
        match entry {
            Csi::Cbt => {
                let sx = s.grid.sx();
                let mut cx = s.cx.min(sx - 1);
                let mut n = self.params.get(0, 1, 1);
                if n == -1 {
                    return Flow::Done;
                }
                while cx > 0 && n > 0 {
                    n -= 1;
                    loop {
                        cx -= 1;
                        if cx == 0 || s.tabs[cx as usize] {
                            break;
                        }
                    }
                }
                s.cx = cx;
            }
            Csi::Cub => {
                let n = self.params.get(0, 1, 1);
                if n != -1 {
                    sw.cursorleft(n as u32);
                }
            }
            Csi::Cud => {
                let n = self.params.get(0, 1, 1);
                if n != -1 {
                    sw.cursordown(n as u32);
                }
            }
            Csi::Cuf => {
                let n = self.params.get(0, 1, 1);
                if n != -1 {
                    sw.cursorright(n as u32);
                }
            }
            Csi::Cup => {
                let n = self.params.get(0, 1, 1);
                let m = self.params.get(1, 1, 1);
                if n != -1 && m != -1 {
                    sw.cursormove(m - 1, n - 1, true);
                }
            }
            Csi::Modset => {
                if self.params.get(0, 0, 0) != 4 {
                    return Flow::Done;
                }
                let m = self.params.get(1, 0, 0);
                let ek = env.policy.extended_keys;
                if ek == ExtendedKeys::Off {
                    return Flow::Done;
                }
                sw.mode_clear(ScreenMode::EXTENDED_KEY_MODES);
                if m == 2 {
                    sw.mode_set(ScreenMode::KEYS_EXTENDED_2);
                } else if m == 1 || ek == ExtendedKeys::Always {
                    sw.mode_set(ScreenMode::KEYS_EXTENDED);
                }
            }
            Csi::Modoff => {
                if self.params.get(0, 0, 0) != 4 {
                    return Flow::Done;
                }
                sw.mode_clear(ScreenMode::KEYS_EXTENDED | ScreenMode::KEYS_EXTENDED_2);
                if env.policy.extended_keys == ExtendedKeys::Always {
                    sw.mode_set(ScreenMode::KEYS_EXTENDED);
                }
            }
            Csi::Winops => return self.csi_winops(0, env),
            Csi::Cuu => {
                let n = self.params.get(0, 1, 1);
                if n != -1 {
                    sw.cursorup(n as u32);
                }
            }
            Csi::Cnl => {
                let n = self.params.get(0, 1, 1);
                if n != -1 {
                    sw.carriagereturn();
                    sw.cursordown(n as u32);
                }
            }
            Csi::Cpl => {
                let n = self.params.get(0, 1, 1);
                if n != -1 {
                    sw.carriagereturn();
                    sw.cursorup(n as u32);
                }
            }
            Csi::Da => {
                if self.params.get(0, 0, 0) == 0 {
                    return if cfg!(feature = "sixel") {
                        self.reply(format_args!("\x1b[?1;2;4c"))
                    } else {
                        self.reply(format_args!("\x1b[?1;2c"))
                    };
                }
            }
            Csi::DaTwo => {
                if self.params.get(0, 0, 0) == 0 {
                    return self.reply(format_args!("\x1b[>84;0;0c"));
                }
            }
            Csi::Ech => {
                let n = self.params.get(0, 1, 1);
                if n != -1 {
                    sw.clearcharacter(n as u32, bg);
                }
            }
            Csi::Dch => {
                let n = self.params.get(0, 1, 1);
                if n != -1 {
                    sw.deletecharacter(n as u32, bg);
                }
            }
            Csi::Decstbm => {
                let n = self.params.get(0, 1, 1);
                let m = self.params.get(1, 1, s.grid.sy() as i32);
                if n != -1 && m != -1 {
                    sw.scrollregion((n - 1) as u32, (m - 1) as u32);
                }
            }
            Csi::Dl => {
                let n = self.params.get(0, 1, 1);
                if n != -1 {
                    sw.deleteline(n as u32, bg);
                }
            }
            Csi::DsrPrivate => {
                if self.params.get(0, 0, 0) == 996 && env.policy.has_pane {
                    return Flow::Yield(Pending::ThemeReport, Sub::Done);
                }
            }
            Csi::Query => {
                let m = self.params.get(0, 0, 0);
                let n = match m {
                    4 => flag(s.mode, ScreenMode::INSERT),
                    _ => 0,
                };
                if m > 0 {
                    return self.reply(format_args!("\x1b[{m};{n}$y"));
                }
            }
            Csi::QueryPrivate => {
                let m = self.params.get(0, 0, 0);
                let mode = s.mode;
                let n = match m {
                    1 => flag(mode, ScreenMode::KCURSOR),
                    3 => 4,
                    6 => flag(mode, ScreenMode::ORIGIN),
                    7 => flag(mode, ScreenMode::WRAP),
                    12 => {
                        if s.cstyle != ScreenCursorStyle::Default
                            || mode.contains(ScreenMode::CURSOR_BLINKING_SET)
                        {
                            flag(mode, ScreenMode::CURSOR_BLINKING)
                        } else {
                            let p = env.policy.cursor_style;
                            if p == 1 || p == 3 || p == 5 { 1 } else { 2 }
                        }
                    }
                    25 => flag(mode, ScreenMode::CURSOR),
                    47 | 1047 | 1049 => {
                        if s.is_alternate() {
                            1
                        } else {
                            2
                        }
                    }
                    1000 => flag(mode, ScreenMode::MOUSE_STANDARD),
                    1002 => flag(mode, ScreenMode::MOUSE_BUTTON),
                    1003 => flag(mode, ScreenMode::MOUSE_ALL),
                    1004 => flag(mode, ScreenMode::FOCUSON),
                    1005 => flag(mode, ScreenMode::MOUSE_UTF8),
                    1006 => flag(mode, ScreenMode::MOUSE_SGR),
                    2004 => flag(mode, ScreenMode::BRACKETPASTE),
                    2026 => flag(mode, ScreenMode::SYNC),
                    2031 => flag(mode, ScreenMode::THEME_UPDATES),
                    _ => 0,
                };
                if m > 0 {
                    return self.reply(format_args!("\x1b[?{m};{n}$y"));
                }
            }
            Csi::Dsr => match self.params.get(0, 0, 0) {
                5 => return self.reply(format_args!("\x1b[0n")),
                6 => {
                    let (cy, cx) = (s.cy + 1, s.cx + 1);
                    return self.reply(format_args!("\x1b[{cy};{cx}R"));
                }
                _ => {}
            },
            Csi::Ed => match self.params.get(0, 0, 0) {
                0 => sw.clearendofscreen(bg),
                1 => sw.clearstartofscreen(bg),
                2 => sw.clearscreen(bg),
                3 if self.params.get(1, 0, 0) == 0 => sw.clearhistory(),
                _ => {}
            },
            Csi::El => match self.params.get(0, 0, 0) {
                0 => sw.clearendofline(bg),
                1 => sw.clearstartofline(bg),
                2 => sw.clearline(bg),
                _ => {}
            },
            Csi::Hpa => {
                let n = self.params.get(0, 1, 1);
                if n != -1 {
                    sw.cursormove(n - 1, -1, true);
                }
            }
            Csi::Ich => {
                let n = self.params.get(0, 1, 1);
                if n != -1 {
                    sw.insertcharacter(n as u32, bg);
                }
            }
            Csi::Il => {
                let n = self.params.get(0, 1, 1);
                if n != -1 {
                    sw.insertline(n as u32, bg);
                }
            }
            Csi::Rep => {
                let mut n = self.params.get(0, 1, 1);
                if n == -1 {
                    return Flow::Done;
                }
                let m = (s.grid.sx() - s.cx) as i32;
                if n > m {
                    n = m;
                }
                if !had_last {
                    return Flow::Done;
                }
                let set = if self.cell.set == 0 {
                    self.cell.g0set
                } else {
                    self.cell.g1set
                };
                if set {
                    self.cell.cell.attr.insert(GridAttributes::CHARSET);
                } else {
                    self.cell.cell.attr.remove(GridAttributes::CHARSET);
                }
                self.cell.cell.data.copy_from(&self.last);
                for _ in 0..n {
                    sw.collect_add(&self.cell.cell);
                }
            }
            Csi::Rcp => self.restore_state(env),
            Csi::Rm => self.csi_modes(false, env),
            Csi::RmPrivate => return self.csi_modes_private(false, 0, env),
            Csi::Scp => self.save_state(env),
            Csi::Sgr => self.sgr(),
            Csi::Sm => self.csi_modes(true, env),
            Csi::SmPrivate => return self.csi_modes_private(true, 0, env),
            Csi::SmGraphics => {
                if cfg!(feature = "sixel") && self.params.len() <= 3 {
                    let n = self.params.get(0, 0, 0);
                    let m = self.params.get(1, 0, 0);
                    let o = self.params.get(2, 0, 0);
                    return if n == 1 && (m == 1 || m == 2 || m == 4) {
                        self.reply(format_args!("\x1b[?{n};0;{SIXEL_COLOUR_REGISTERS}S"))
                    } else {
                        self.reply(format_args!("\x1b[?{n};3;{o}S"))
                    };
                }
            }
            Csi::Su => {
                let n = self.params.get(0, 1, 1);
                if n != -1 {
                    sw.scrollup(n as u32, bg);
                }
            }
            Csi::Sd => {
                let n = self.params.get(0, 1, 1);
                if n != -1 {
                    sw.scrolldown(n as u32, bg);
                }
            }
            Csi::Tbc => match self.params.get(0, 0, 0) {
                0 => {
                    if s.cx < s.grid.sx() {
                        s.tabs[s.cx as usize] = false;
                    }
                }
                3 => s.tabs.fill(false),
                _ => {}
            },
            Csi::Vpa => {
                let n = self.params.get(0, 1, 1);
                if n != -1 {
                    sw.cursormove(-1, n - 1, true);
                }
            }
            Csi::Decscusr => {
                let n = self.params.get(0, 0, 0);
                if n == -1 {
                    return Flow::Done;
                }
                set_cursor_style(n as u32, &mut s.cstyle, &mut s.mode);
                if n == 0 {
                    sw.mode_clear(ScreenMode::CURSOR_BLINKING_SET);
                }
            }
            Csi::Xda => {
                if self.params.get(0, 0, 0) == 0 {
                    return self.reply(format_args!("\x1bP>|tmux {TMUX_VERSION}\x1b\\"));
                }
            }
        }
        Flow::Done
    }

    /// `input_csi_dispatch_rm` and `_sm` (`input.c:1899-1919,1996-2016`).
    fn csi_modes(&mut self, set: bool, env: &mut Env<'_, '_>) {
        for i in 0..self.params.len() {
            match self.params.get(i, 0, -1) {
                4 => {
                    if set {
                        env.sw.mode_set(ScreenMode::INSERT);
                    } else {
                        env.sw.mode_clear(ScreenMode::INSERT);
                    }
                }
                34 => {
                    if set {
                        env.sw.mode_clear(ScreenMode::CURSOR_VERY_VISIBLE);
                    } else {
                        env.sw.mode_set(ScreenMode::CURSOR_VERY_VISIBLE);
                    }
                }
                _ => {}
            }
        }
    }

    /// `input_csi_dispatch_rm_private` and `_sm_private`
    /// (`input.c:1923-1992,2020-2097`), resumable at parameter `start`.
    fn csi_modes_private(&mut self, set: bool, start: usize, env: &mut Env<'_, '_>) -> Flow {
        let sw = &mut *env.sw;
        for i in start..self.params.len() {
            let next = Sub::Modes {
                set,
                i: (i + 1) as u8,
            };
            match self.params.get(i, 0, -1) {
                1 => sw.mode_toggle(ScreenMode::KCURSOR, set),
                3 => {
                    sw.cursormove(0, 0, true);
                    sw.clearscreen(self.cell.cell.bg);
                }
                6 => {
                    sw.mode_toggle(ScreenMode::ORIGIN, set);
                    sw.cursormove(0, 0, true);
                }
                7 => sw.mode_toggle(ScreenMode::WRAP, set),
                12 => {
                    sw.mode_toggle(ScreenMode::CURSOR_BLINKING, set);
                    sw.mode_set(ScreenMode::CURSOR_BLINKING_SET);
                }
                25 => sw.mode_toggle(ScreenMode::CURSOR, set),
                1000 => {
                    sw.mode_clear(ScreenMode::ALL_MOUSE_MODES);
                    if set {
                        sw.mode_set(ScreenMode::MOUSE_STANDARD);
                    }
                }
                1001 => {
                    if !set {
                        sw.mode_clear(ScreenMode::ALL_MOUSE_MODES);
                    }
                }
                1002 => {
                    sw.mode_clear(ScreenMode::ALL_MOUSE_MODES);
                    if set {
                        sw.mode_set(ScreenMode::MOUSE_BUTTON);
                    }
                }
                1003 => {
                    sw.mode_clear(ScreenMode::ALL_MOUSE_MODES);
                    if set {
                        sw.mode_set(ScreenMode::MOUSE_ALL);
                    }
                }
                1004 => sw.mode_toggle(ScreenMode::FOCUSON, set),
                1005 => sw.mode_toggle(ScreenMode::MOUSE_UTF8, set),
                1006 => sw.mode_toggle(ScreenMode::MOUSE_SGR, set),
                47 | 1047 | 1049 => {
                    let cursor = self.params.get(i, 0, -1) == 1049;
                    let changed = if set {
                        sw.alternateon(&self.cell.cell, cursor)
                    } else {
                        sw.alternateoff(Some(&mut self.cell.cell), cursor)
                    };
                    if changed && env.policy.writer_has_pane {
                        return Flow::Yield(Pending::AlternateChanged(set), next);
                    }
                }
                2004 => sw.mode_toggle(ScreenMode::BRACKETPASTE, set),
                2026 => {
                    if set {
                        // screen_write_start_sync(ictx->wp): the parser pane
                        // owns the start (screen-write.c:1067-1080).
                        if env.policy.has_pane {
                            sw.screen.mode.insert(ScreenMode::SYNC);
                            return Flow::Yield(Pending::SyncStart, next);
                        }
                    } else {
                        let was_sync = sw.screen.mode.contains(ScreenMode::SYNC);
                        sw.end_sync();
                        if was_sync && env.policy.writer_has_pane {
                            return Flow::Yield(Pending::SyncEnd, next);
                        }
                    }
                }
                2031 => {
                    sw.mode_toggle(ScreenMode::THEME_UPDATES, set);
                    if env.policy.has_pane {
                        let pending = if set {
                            Pending::ThemeUpdatesEnabled
                        } else {
                            Pending::ThemeUpdatesDisabled
                        };
                        return Flow::Yield(pending, next);
                    }
                }
                _ => {}
            }
        }
        Flow::Done
    }

    /// `input_csi_dispatch_winops` (`input.c:2122-2218`), resumable at
    /// parameter `m`.
    fn csi_winops(&mut self, start: usize, env: &mut Env<'_, '_>) -> Flow {
        let s = &mut *env.sw.screen;
        let (x, y) = (s.grid.sx(), s.grid.sy());
        let mut m = start;
        loop {
            let n = self.params.get(m, 0, -1);
            if n == -1 {
                break;
            }
            let next = Sub::Winops { m: (m + 1) as u8 };
            match n {
                1 | 2 | 5 | 6 | 7 | 11 | 13 | 20 | 21 | 24 => {}
                3 | 4 | 8 | 9 | 10 => {
                    if n != 9 && n != 10 {
                        m += 1;
                        if self.params.get(m, 0, -1) == -1 {
                            return Flow::Done;
                        }
                    }
                    m += 1;
                    if self.params.get(m, 0, -1) == -1 {
                        return Flow::Done;
                    }
                }
                14..=16 => {
                    if let Some((xpixel, ypixel)) =
                        env.policy.pixels.filter(|_| env.policy.has_pane)
                    {
                        let next = Sub::Winops { m: (m + 1) as u8 };
                        let flow = match n {
                            14 => self.reply(format_args!(
                                "\x1b[4;{};{}t",
                                y.wrapping_mul(ypixel),
                                x.wrapping_mul(xpixel)
                            )),
                            15 => self.reply(format_args!(
                                "\x1b[5;{};{}t",
                                y.wrapping_mul(ypixel),
                                x.wrapping_mul(xpixel)
                            )),
                            _ => self.reply(format_args!("\x1b[6;{ypixel};{xpixel}t")),
                        };
                        return Self::retarget(flow, next);
                    }
                }
                18 => {
                    let flow = self.reply(format_args!("\x1b[8;{y};{x}t"));
                    return Self::retarget(flow, next);
                }
                19 => {
                    let flow = self.reply(format_args!("\x1b[9;{y};{x}t"));
                    return Self::retarget(flow, next);
                }
                22 => {
                    m += 1;
                    match self.params.get(m, 0, -1) {
                        -1 => return Flow::Done,
                        0 | 2 => s.push_title(),
                        _ => {}
                    }
                }
                23 => {
                    m += 1;
                    match self.params.get(m, 0, -1) {
                        -1 => return Flow::Done,
                        0 | 2 => {
                            s.pop_title();
                            if env.policy.has_pane {
                                self.scratch.clear();
                                self.scratch.extend_from_slice(&s.title);
                                return Flow::Yield(
                                    Pending::TitlePopped,
                                    Sub::Winops { m: (m + 1) as u8 },
                                );
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
            m += 1;
        }
        Flow::Done
    }

    fn retarget(flow: Flow, next: Sub) -> Flow {
        match flow {
            Flow::Yield(pending, _) => Flow::Yield(pending, next),
            Flow::Done => Flow::Done,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csi_table_is_sorted() {
        for pair in CSI_TABLE.windows(2) {
            assert!((pair[0].0, pair[0].1) < (pair[1].0, pair[1].1));
        }
        assert_eq!(
            table_lookup(&CSI_TABLE, b'p', b"?$"),
            Some(Csi::QueryPrivate)
        );
        assert_eq!(table_lookup(&CSI_TABLE, b'q', b" "), Some(Csi::Decscusr));
        assert_eq!(table_lookup(&CSI_TABLE, b'q', b""), None);
    }
}
