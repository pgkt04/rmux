// Ported from tmux cmd-capture-pane.c and format.c @ 8f25579c
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

//! Byte-exact dumps of a screen as `capture-pane` and `display -p` print
//! them, for differential tests against the oracle tmux.

use crate::cell::GridCell;
use crate::grid::string::StringCellsCtx;
use crate::grid::{GridLineFlags, GridStringFlags};
use crate::hyperlinks::HyperlinkRegistry;
use crate::screen::{Screen, ScreenMode};

/// `cmd_capture_pane_history` (`cmd-capture-pane.c:253-412`) for
/// `-p -S - -E -` with the given flags: every history and view line, each
/// followed by `\n`. `last` is `*gc`, allocated once as `grid_default_cell`
/// on the first `grid_string_cells` call and carried across lines
/// (`grid.c:1189-1192`); `strlen(line)` stops at the first NUL (`:356`).
fn history(
    screen: &Screen,
    registry: &HyperlinkRegistry,
    flags: GridStringFlags,
    show_flags: bool,
) -> Vec<u8> {
    let gd = &screen.grid;
    let sx = gd.sx();
    let bottom = gd.hsize() + gd.sy() - 1;
    let mut last = GridCell::default();
    let mut buf = Vec::new();
    for i in 0..=bottom {
        let mut ctx = StringCellsCtx {
            last: Some(&mut last),
            flags,
            hyperlinks: screen.hyperlinks.as_ref().map(|h| (registry, h)),
        };
        let mut line = gd.string_cells(0, i, sx, &mut ctx);
        if let Some(nul) = line.iter().position(|&b| b == 0) {
            line.truncate(nul);
        }
        if show_flags {
            let gl = gd.get_line(i);
            let start = buf.len();
            for (flag, letter) in [
                (GridLineFlags::DEAD, b'D'),
                (GridLineFlags::HYPERLINK, b'H'),
                (GridLineFlags::START_OUTPUT, b'O'),
                (GridLineFlags::START_PROMPT, b'P'),
                (GridLineFlags::WRAPPED, b'W'),
                (GridLineFlags::EXTENDED, b'X'),
            ] {
                if gl.flags.intersects(flag) {
                    buf.push(letter);
                }
            }
            if buf.len() == start {
                buf.push(b'-');
            }
            buf.push(b' ');
        }
        buf.extend_from_slice(&line);
        buf.push(b'\n');
    }
    buf
}

/// `capture-pane -p -e -N -S -`: `GRID_STRING_WITH_SEQUENCES |
/// GRID_STRING_EMPTY_CELLS` (`cmd-capture-pane.c:335-342`). The trailing
/// empty cells depend on each line's `cellsize`, which `grid_expand_line`
/// rounds per write batch (`grid.c:311-316`), so this output differs
/// between two feeds of the same bytes split differently.
pub fn capture_pane(screen: &Screen, registry: &HyperlinkRegistry) -> Vec<u8> {
    history(
        screen,
        registry,
        GridStringFlags::WITH_SEQUENCES | GridStringFlags::EMPTY_CELLS,
        false,
    )
}

/// `capture-pane -p -e -N -T -S -`: `GRID_STRING_WITH_SEQUENCES` only, so
/// each line stops at `cellused`; independent of write batching.
pub fn capture_pane_used(screen: &Screen, registry: &HyperlinkRegistry) -> Vec<u8> {
    history(screen, registry, GridStringFlags::WITH_SEQUENCES, false)
}

/// `capture-pane -p -F -N -T -S -`: the line flag prefix
/// (`cmd-capture-pane.c:379-400`) before the used cells.
pub fn capture_pane_flags(screen: &Screen, registry: &HyperlinkRegistry) -> Vec<u8> {
    history(screen, registry, GridStringFlags::default(), true)
}

fn flag(screen: &Screen, mode: ScreenMode) -> u8 {
    u8::from(screen.mode.intersects(mode))
}

/// `display -p` of the screen-state format used by the differential tests
/// (`format.c` callbacks: `cursor_x` `:1881`, `cursor_y` `:1890`,
/// `cursor_flag` `:1838`, `insert_flag` `:1960`, `keypad_cursor_flag`
/// `:1972`, `keypad_flag` `:1984`, `wrap_flag` `:3442`, `origin_flag`
/// `:2132`, `mouse_standard_flag` `:2059`, `mouse_button_flag` `:2020`,
/// `mouse_all_flag` `:1996`, `mouse_utf8_flag` `:2071`, `mouse_sgr_flag`
/// `:2047`, `bracket_paste_flag` `:1483`, `cursor_very_visible` `:1869`,
/// `alternate_on` `:1453`, `alternate_saved_x` `:1465`, `alternate_saved_y`
/// `:1474`, `scroll_region_upper` `:2843`, `scroll_region_lower` `:2834`,
/// `history_size` `:1951`, `pane_title` `:2679`). An unsaved cursor is
/// `UINT_MAX` (`screen.c:124-125`).
pub const STATE_FORMAT: &str = "#{cursor_x} #{cursor_y} #{cursor_flag} #{insert_flag} \
#{keypad_cursor_flag} #{keypad_flag} #{wrap_flag} #{origin_flag} #{mouse_standard_flag} \
#{mouse_button_flag} #{mouse_all_flag} #{mouse_utf8_flag} #{mouse_sgr_flag} \
#{bracket_paste_flag} #{cursor_very_visible} #{alternate_on} #{alternate_saved_x} \
#{alternate_saved_y} #{scroll_region_upper} #{scroll_region_lower} #{history_size} \
#{pane_title}";

/// The expansion of [`STATE_FORMAT`] for `screen`.
pub fn state_line(screen: &Screen) -> String {
    let (saved_x, saved_y) = screen.saved_cursor.unwrap_or((u32::MAX, u32::MAX));
    format!(
        "{} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {}",
        screen.cx,
        screen.cy,
        flag(screen, ScreenMode::CURSOR),
        flag(screen, ScreenMode::INSERT),
        flag(screen, ScreenMode::KCURSOR),
        flag(screen, ScreenMode::KKEYPAD),
        flag(screen, ScreenMode::WRAP),
        flag(screen, ScreenMode::ORIGIN),
        flag(screen, ScreenMode::MOUSE_STANDARD),
        flag(screen, ScreenMode::MOUSE_BUTTON),
        flag(screen, ScreenMode::MOUSE_ALL),
        flag(screen, ScreenMode::MOUSE_UTF8),
        flag(screen, ScreenMode::MOUSE_SGR),
        flag(screen, ScreenMode::BRACKETPASTE),
        flag(screen, ScreenMode::CURSOR_VERY_VISIBLE),
        u8::from(screen.saved_grid.is_some()),
        saved_x,
        saved_y,
        screen.rupper,
        screen.rlower,
        screen.grid.hsize(),
        String::from_utf8_lossy(&screen.title),
    )
}
