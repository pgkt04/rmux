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

//! OSC, APC and rename string handlers (`input.c:2685-3472`).

use super::reply;
use super::{
    ColourQueryKind, Env, FLAG_DISCARD, Flow, GetClipboard, InputCtx, InputRequestKind,
    Osc133Event, Pending, Sub,
};
use crate::colour::{Colour, ColourFlags, parse_x11_colour};
use crate::grid::GridLineFlags;
use crate::hyperlinks::HyperlinkId;
use crate::screen::ProgressBarState;
use rmux_util::strtonum::strtonum;

/// `strtol(s, &end, 10)`: leading whitespace, optional sign, digits with
/// saturation; no digits leaves `end` at the start.
fn strtol(s: &[u8]) -> (i64, usize) {
    let mut i = 0;
    while i < s.len() && matches!(s[i], b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r') {
        i += 1;
    }
    let mut negative = false;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        negative = s[i] == b'-';
        i += 1;
    }
    let digits = i;
    let mut value: i64 = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        let d = i64::from(s[i] - b'0');
        value = if negative {
            value.saturating_mul(10).saturating_sub(d)
        } else {
            value.saturating_mul(10).saturating_add(d)
        };
        i += 1;
    }
    if i == digits {
        return (0, 0);
    }
    (value, i)
}

fn cstr(s: &[u8]) -> &[u8] {
    &s[..s.iter().position(|&b| b == 0).unwrap_or(s.len())]
}

/// `input_osc_133_exit_status` (`input.c:3195-3220`).
fn osc_133_exit_status(p: &[u8]) -> u8 {
    if p.get(1) != Some(&b';') || p.get(2).is_none_or(|&b| b == b'=') {
        return 0;
    }
    let rest = &p[2..];
    let value = &rest[..rest.iter().position(|&b| b == b';').unwrap_or(rest.len())];
    if value.is_empty() || value.contains(&b'=') {
        return 0;
    }
    match strtonum(value, 0, 255) {
        Ok(v) => v as u8,
        Err(_) => 255,
    }
}

impl InputCtx {
    /// `input_exit_osc` (`input.c:2697-2777`), resumable inside OSC 4, 10,
    /// 11, 110, 111 and 133.
    pub(super) fn exit_osc(&mut self, sub: Sub, env: &mut Env<'_, '_>) -> Flow {
        match sub {
            Sub::Start => {}
            Sub::Osc4 { off, redraw } => return self.osc_4(off, redraw, env),
            Sub::Redraw => {
                env.sw.fullredraw();
                return Flow::Done;
            }
            Sub::Osc133End { status } => {
                self.osc_133_end(status, env);
                return Flow::Done;
            }
            _ => return Flow::Done,
        }
        if self.flags & FLAG_DISCARD != 0 {
            return Flow::Done;
        }
        let Some(&first) = self.string.first() else {
            return Flow::Done;
        };
        if !first.is_ascii_digit() {
            return Flow::Done;
        }
        let mut option: u32 = 0;
        let mut off = 0;
        while off < self.string.len() && self.string[off].is_ascii_digit() {
            option = option
                .wrapping_mul(10)
                .wrapping_add(u32::from(self.string[off] - b'0'));
            off += 1;
        }
        if off < self.string.len() {
            if self.string[off] != b';' {
                return Flow::Done;
            }
            off += 1;
        }
        match option {
            0 | 2 => {
                if env.policy.has_pane
                    && env.policy.allow_set_title
                    && env.sw.screen.set_title(&self.string[off..], true)
                {
                    return Flow::Yield(Pending::TitleChanged(off), Sub::Done);
                }
            }
            4 => return self.osc_4(off, false, env),
            7 => {
                if env.policy.has_pane && env.sw.screen.set_path(&self.string[off..], true) {
                    return Flow::Yield(Pending::PathChanged, Sub::Done);
                }
            }
            8 => self.osc_8(off, env),
            9 => return self.osc_9(off, env),
            10 | 11 => return self.osc_colour(option, off, env),
            12 => return self.osc_12(off, env),
            52 => return self.osc_52(off, env),
            104 => self.osc_104(off, env),
            110 | 111 => return self.osc_reset_colour(option, off, env),
            112 => {
                if self.string.len() == off {
                    env.sw.screen.set_cursor_colour(Colour::NONE);
                }
            }
            133 => return self.osc_133(off, env),
            _ => {}
        }
        Flow::Done
    }

    /// `input_osc_4` (`input.c:2926-2971`), resumable at byte `off`.
    fn osc_4(&mut self, mut off: usize, mut redraw: bool, env: &mut Env<'_, '_>) -> Flow {
        loop {
            if off >= self.string.len() {
                break;
            }
            let (idx, used) = strtol(&self.string[off..]);
            let mut next = off + used;
            if self.string.get(next) != Some(&b';') {
                break;
            }
            next += 1;
            if !(0..256).contains(&idx) {
                break;
            }
            let spec_end = self.string[next..]
                .iter()
                .position(|&b| b == b';')
                .map_or(self.string.len(), |p| next + p);
            let spec = &self.string[next..spec_end];
            let after = (spec_end + 1).min(self.string.len());
            let resume = Sub::Osc4 { off: after, redraw };
            if spec == b"?" {
                let c = env
                    .palette
                    .as_deref()
                    .and_then(|p| p.get(Colour(idx as i32 | ColourFlags::_256.bits() as i32)));
                if let Some(c) = c {
                    self.scratch.clear();
                    if reply::colour(4, Some(idx as u8), c, self.string_end, &mut self.scratch) {
                        return Flow::Yield(Pending::Reply, resume);
                    }
                } else if env.policy.has_pane {
                    return Flow::Yield(
                        Pending::Request(
                            InputRequestKind::Palette { idx: idx as u8 },
                            self.string_end,
                        ),
                        resume,
                    );
                }
                off = after;
                continue;
            }
            if let Ok(c) = parse_x11_colour(spec)
                && let Some(palette) = env.palette.as_deref_mut()
                && palette.set(idx as u32, c)
            {
                redraw = true;
            }
            off = after;
        }
        if redraw {
            env.sw.fullredraw();
        }
        Flow::Done
    }

    /// `input_osc_8` (`input.c:2975-3012`).
    fn osc_8(&mut self, off: usize, env: &mut Env<'_, '_>) {
        let p = &self.string[off..];
        let mut id: Option<&[u8]> = None;
        let mut start = 0;
        let mut end = None;
        while let Some(pos) = p[start..].iter().position(|&b| b == b':' || b == b';') {
            let e = start + pos;
            if e - start >= 4 && &p[start..start + 3] == b"id=" {
                if id.is_some() {
                    return;
                }
                id = Some(&p[start + 3..e]);
            }
            if p[e] == b';' {
                end = Some(e);
                break;
            }
            start = e + 1;
        }
        let Some(end) = end else {
            return;
        };
        let uri = &p[end + 1..];
        if uri.is_empty() {
            self.cell.cell.link = HyperlinkId::NONE;
            return;
        }
        if let Some(store) = &env.sw.screen.hyperlinks
            && let Ok(link) = env.sw.registry.put(store, uri, id)
        {
            self.cell.cell.link = link;
        }
    }

    /// `input_osc_9` (`input.c:3028-3064`).
    fn osc_9(&mut self, off: usize, env: &mut Env<'_, '_>) -> Flow {
        let p = &self.string[off..];
        if p.first() != Some(&b'4') {
            return Flow::Done;
        }
        let p = &p[1..];
        if p.is_empty() || (p[0] == b';' && p.len() == 1) {
            return Flow::Done;
        }
        if p[0] != b';' {
            return Flow::Done;
        }
        let p = &p[1..];
        let Some(&state) = p.first().filter(|b| (b'0'..=b'4').contains(b)) else {
            return Flow::Done;
        };
        let state = ProgressBarState::try_from(i32::from(state - b'0')).expect("0..4");
        let p = &p[1..];
        if p.is_empty() || (p[0] == b';' && p.len() == 1) {
            return self.set_progress_bar(state, -1, env);
        }
        if p[0] != b';' {
            return Flow::Done;
        }
        let mut progress: i32 = 0;
        let mut i = 1;
        while i < p.len() && p[i].is_ascii_digit() {
            if progress > 100 {
                return Flow::Done;
            }
            progress = progress * 10 + i32::from(p[i] - b'0');
            i += 1;
        }
        if i != p.len() || progress > 100 {
            return Flow::Done;
        }
        self.set_progress_bar(state, progress, env)
    }

    /// `input_set_progress_bar` (`input.c:3016-3024`).
    fn set_progress_bar(&mut self, state: ProgressBarState, p: i32, env: &mut Env<'_, '_>) -> Flow {
        env.sw.screen.set_progress_bar(state, p);
        if env.policy.has_pane {
            return Flow::Yield(Pending::ProgressChanged, Sub::Done);
        }
        Flow::Done
    }

    /// `input_osc_10` and `input_osc_11` (`input.c:3068-3099,3119-3142`).
    fn osc_colour(&mut self, n: u32, off: usize, env: &mut Env<'_, '_>) -> Flow {
        let p = &self.string[off..];
        let which = if n == 10 {
            ColourQueryKind::Foreground
        } else {
            ColourQueryKind::Background
        };
        if p == b"?" {
            if env.policy.has_pane {
                return Flow::Yield(Pending::ColourQuery(which, self.string_end), Sub::Done);
            }
            return Flow::Done;
        }
        let Ok(c) = parse_x11_colour(p) else {
            return Flow::Done;
        };
        let Some(palette) = env.palette.as_deref_mut() else {
            return Flow::Done;
        };
        if n == 10 {
            palette.fg = c;
        } else {
            palette.bg = c;
        }
        self.style_then_redraw(n == 11, env)
    }

    /// `input_osc_110` and `input_osc_111` (`input.c:3103-3115,3146-3158`).
    fn osc_reset_colour(&mut self, n: u32, off: usize, env: &mut Env<'_, '_>) -> Flow {
        if self.string.len() != off {
            return Flow::Done;
        }
        let Some(palette) = env.palette.as_deref_mut() else {
            return Flow::Done;
        };
        if n == 110 {
            palette.fg = Colour::DEFAULT;
        } else {
            palette.bg = Colour::DEFAULT;
        }
        self.style_then_redraw(n == 111, env)
    }

    /// Pane style flags first, then the full redraw (`input.c:3095-3097`).
    fn style_then_redraw(&mut self, theme: bool, env: &mut Env<'_, '_>) -> Flow {
        if env.policy.has_pane {
            return Flow::Yield(Pending::StyleChanged(theme), Sub::Redraw);
        }
        env.sw.fullredraw();
        Flow::Done
    }

    /// `input_osc_12` (`input.c:3162-3183`).
    fn osc_12(&mut self, off: usize, env: &mut Env<'_, '_>) -> Flow {
        let p = &self.string[off..];
        if p == b"?" {
            if env.policy.has_pane {
                let s = &env.sw.screen;
                let c = if s.ccolour == Colour::NONE {
                    s.default_ccolour
                } else {
                    s.ccolour
                };
                self.scratch.clear();
                if reply::colour(12, None, c, self.string_end, &mut self.scratch) {
                    return Flow::Yield(Pending::Reply, Sub::Done);
                }
            }
            return Flow::Done;
        }
        if let Ok(c) = parse_x11_colour(p) {
            env.sw.screen.set_cursor_colour(c);
        }
        Flow::Done
    }

    /// `input_osc_52` with `input_osc_52_parse` and `input_osc_52_reply`
    /// (`input.c:3339-3435`).
    fn osc_52(&mut self, off: usize, env: &mut Env<'_, '_>) -> Flow {
        if !env.policy.set_clipboard_on {
            return Flow::Done;
        }
        let p = &self.string[off..];
        let Some(semi) = p.iter().position(|&b| b == b';') else {
            return Flow::Done;
        };
        let data = &p[semi + 1..];
        if data.is_empty() {
            return Flow::Done;
        }
        self.clip.clear();
        for &ch in &p[..semi] {
            if b"cpqs01234567".contains(&ch) && !self.clip.as_slice().contains(&ch) {
                self.clip.push(ch);
            }
        }
        let clip = self.clip.as_slice().first().copied().unwrap_or(0);
        if data == b"?" {
            if !env.policy.has_pane {
                return Flow::Done;
            }
            return match env.policy.get_clipboard {
                GetClipboard::Off => Flow::Done,
                GetClipboard::Buffer => {
                    Flow::Yield(Pending::ClipboardQuery(clip, self.string_end), Sub::Done)
                }
                GetClipboard::Request | GetClipboard::Both => Flow::Yield(
                    Pending::Request(InputRequestKind::Clipboard { clip }, self.string_end),
                    Sub::Done,
                ),
            };
        }
        let Some(decoded) = rmux_util::base64::pton(data) else {
            return Flow::Done;
        };
        if !env.policy.has_pane {
            return Flow::Done;
        }
        self.decoded = decoded;
        Flow::Yield(Pending::ClipboardReceived, Sub::Done)
    }

    /// `input_osc_104` (`input.c:3439-3472`).
    fn osc_104(&mut self, off: usize, env: &mut Env<'_, '_>) {
        let p = cstr(&self.string[off..]);
        if p.is_empty() {
            if let Some(palette) = env.palette.as_deref_mut() {
                palette.clear_runtime();
            }
            env.sw.fullredraw();
            return;
        }
        let mut redraw = false;
        let mut s = 0;
        while s < p.len() {
            let (idx, used) = strtol(&p[s..]);
            s += used;
            if s < p.len() && p[s] != b';' {
                break;
            }
            if !(0..256).contains(&idx) {
                break;
            }
            if let Some(palette) = env.palette.as_deref_mut()
                && palette.set(idx as u32, Colour::NONE)
            {
                redraw = true;
            }
            if s < p.len() && p[s] == b';' {
                s += 1;
            }
        }
        if redraw {
            env.sw.fullredraw();
        }
    }

    /// `input_osc_133` (`input.c:3264-3335`); `D` fires its event before the
    /// line end marker.
    fn osc_133(&mut self, off: usize, env: &mut Env<'_, '_>) -> Flow {
        let p = &self.string[off..];
        let s = &mut *env.sw.screen;
        let line = s.cy + s.grid.hsize();
        let has_line = line < s.grid.hsize() + s.grid.sy();
        let cx = s.cx as u16;
        match p.first() {
            Some(b'A' | b'N') => {
                if has_line {
                    let gl = s.grid.get_line_mut(line);
                    gl.osc133 = Default::default();
                    gl.osc133.prompt_col = cx;
                    gl.flags.insert(GridLineFlags::START_PROMPT);
                }
                if env.policy.has_pane {
                    return Flow::Yield(Pending::Osc133(Osc133Event::Prompt), Sub::Done);
                }
            }
            Some(b'P') => {
                if has_line {
                    let gl = s.grid.get_line_mut(line);
                    let second = p
                        .windows(4)
                        .position(|w| w == b";k=s")
                        .is_some_and(|i| p.get(i + 4).is_none_or(|&b| b == b';'));
                    if second {
                        gl.flags.insert(GridLineFlags::SECOND_PROMPT);
                    } else {
                        gl.flags.insert(GridLineFlags::START_PROMPT);
                    }
                    gl.osc133.prompt_col = cx;
                }
            }
            Some(b'B' | b'I') => {
                if has_line {
                    let gl = s.grid.get_line_mut(line);
                    gl.flags.insert(GridLineFlags::START_COMMAND);
                    gl.osc133.cmd_col = cx;
                }
            }
            Some(b'C') => {
                if has_line {
                    let gl = s.grid.get_line_mut(line);
                    gl.flags.insert(GridLineFlags::START_OUTPUT);
                    gl.osc133.out_start_col = cx;
                }
                if env.policy.has_pane {
                    return Flow::Yield(Pending::Osc133(Osc133Event::CommandStarted), Sub::Done);
                }
            }
            Some(b'D') => {
                let status = osc_133_exit_status(p);
                if env.policy.has_pane {
                    return Flow::Yield(
                        Pending::Osc133(Osc133Event::CommandFinished { status }),
                        Sub::Osc133End { status },
                    );
                }
                self.osc_133_end(status, env);
            }
            _ => {}
        }
        Flow::Done
    }

    fn osc_133_end(&mut self, status: u8, env: &mut Env<'_, '_>) {
        let s = &mut *env.sw.screen;
        let line = s.cy + s.grid.hsize();
        if line < s.grid.hsize() + s.grid.sy() {
            let cx = s.cx as u16;
            let gl = s.grid.get_line_mut(line);
            gl.flags.insert(GridLineFlags::END_OUTPUT);
            gl.osc133.out_end_col = cx;
            gl.osc133.exit_status = status;
        }
    }

    /// `input_exit_apc` (`input.c:2792-2808`).
    pub(super) fn exit_apc(&mut self, env: &mut Env<'_, '_>) -> Flow {
        if self.flags & FLAG_DISCARD != 0 {
            return Flow::Done;
        }
        if env.policy.has_pane
            && env.policy.allow_set_title
            && env.sw.screen.set_title(&self.string, true)
        {
            return Flow::Yield(Pending::TitleChanged(0), Sub::Done);
        }
        Flow::Done
    }

    /// `input_exit_rename` (`input.c:2823-2853`).
    pub(super) fn exit_rename(&mut self, env: &mut Env<'_, '_>) -> Flow {
        if !env.policy.has_pane || self.flags & FLAG_DISCARD != 0 || !env.policy.allow_rename {
            return Flow::Done;
        }
        if !rmux_util::utf8::is_valid(&self.string) {
            return Flow::Done;
        }
        Flow::Yield(Pending::Rename(!self.string.is_empty()), Sub::Done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_status_cases() {
        assert_eq!(osc_133_exit_status(b"D;7"), 7);
        assert_eq!(osc_133_exit_status(b"D;-1"), 255);
        assert_eq!(osc_133_exit_status(b"D;300"), 255);
        assert_eq!(osc_133_exit_status(b"D"), 0);
        assert_eq!(osc_133_exit_status(b"D;"), 0);
        assert_eq!(osc_133_exit_status(b"D;=x"), 0);
        assert_eq!(osc_133_exit_status(b"D;a=b"), 0);
        assert_eq!(osc_133_exit_status(b"D;;x"), 0);
        assert_eq!(osc_133_exit_status(b"D;9;k=s"), 9);
    }

    #[test]
    fn strtol_cases() {
        assert_eq!(strtol(b"12;x"), (12, 2));
        assert_eq!(strtol(b";red"), (0, 0));
        assert_eq!(strtol(b" +3"), (3, 3));
        assert_eq!(strtol(b"-4"), (-4, 2));
        assert_eq!(strtol(b"99999999999999999999"), (i64::MAX, 20));
    }
}
