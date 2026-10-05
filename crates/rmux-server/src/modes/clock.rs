// Ported from tmux window-clock.c @ 8f25579c
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

use crate::ids::{ClientId, ModeId, TimerId};
use crate::model::pane::{PaneMode, PaneModeDriver, pane_reset_mode};
use crate::model::{ModelError, PaneFlags};
use crate::server::Server;
use crate::server::event_loop::schedule_deferred;
use crate::ui::styles::{create_defaults, style_apply};
use rmux_emu::cell::{DEFAULT_CELL, GridCell, GridCellFlags};
use rmux_emu::colour::Colour;
use rmux_emu::screen::write::{ScreenOnlySink, ScreenWriteCtx, ScreenWritePolicy};
use rmux_emu::screen::{Screen, ScreenMode, ScreenResetPolicy};
use rmux_util::key::KeyCode;
use rmux_util::utf8::Utf8Data;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const NAME: &[u8] = b"clock-mode";

/// `window_clock_table`: 5x5 glyphs for `0`-`9`, `:`, `A`, `P`, `M`.
pub static CLOCK_TABLE: [[[u8; 5]; 5]; 14] = [
    [
        [1, 1, 1, 1, 1],
        [1, 0, 0, 0, 1],
        [1, 0, 0, 0, 1],
        [1, 0, 0, 0, 1],
        [1, 1, 1, 1, 1],
    ],
    [
        [0, 0, 0, 0, 1],
        [0, 0, 0, 0, 1],
        [0, 0, 0, 0, 1],
        [0, 0, 0, 0, 1],
        [0, 0, 0, 0, 1],
    ],
    [
        [1, 1, 1, 1, 1],
        [0, 0, 0, 0, 1],
        [1, 1, 1, 1, 1],
        [1, 0, 0, 0, 0],
        [1, 1, 1, 1, 1],
    ],
    [
        [1, 1, 1, 1, 1],
        [0, 0, 0, 0, 1],
        [1, 1, 1, 1, 1],
        [0, 0, 0, 0, 1],
        [1, 1, 1, 1, 1],
    ],
    [
        [1, 0, 0, 0, 1],
        [1, 0, 0, 0, 1],
        [1, 1, 1, 1, 1],
        [0, 0, 0, 0, 1],
        [0, 0, 0, 0, 1],
    ],
    [
        [1, 1, 1, 1, 1],
        [1, 0, 0, 0, 0],
        [1, 1, 1, 1, 1],
        [0, 0, 0, 0, 1],
        [1, 1, 1, 1, 1],
    ],
    [
        [1, 1, 1, 1, 1],
        [1, 0, 0, 0, 0],
        [1, 1, 1, 1, 1],
        [1, 0, 0, 0, 1],
        [1, 1, 1, 1, 1],
    ],
    [
        [1, 1, 1, 1, 1],
        [0, 0, 0, 0, 1],
        [0, 0, 0, 0, 1],
        [0, 0, 0, 0, 1],
        [0, 0, 0, 0, 1],
    ],
    [
        [1, 1, 1, 1, 1],
        [1, 0, 0, 0, 1],
        [1, 1, 1, 1, 1],
        [1, 0, 0, 0, 1],
        [1, 1, 1, 1, 1],
    ],
    [
        [1, 1, 1, 1, 1],
        [1, 0, 0, 0, 1],
        [1, 1, 1, 1, 1],
        [0, 0, 0, 0, 1],
        [1, 1, 1, 1, 1],
    ],
    [
        [0, 0, 0, 0, 0],
        [0, 0, 1, 0, 0],
        [0, 0, 0, 0, 0],
        [0, 0, 1, 0, 0],
        [0, 0, 0, 0, 0],
    ],
    [
        [1, 1, 1, 1, 1],
        [1, 0, 0, 0, 1],
        [1, 1, 1, 1, 1],
        [1, 0, 0, 0, 1],
        [1, 0, 0, 0, 1],
    ],
    [
        [1, 1, 1, 1, 1],
        [1, 0, 0, 0, 1],
        [1, 1, 1, 1, 1],
        [1, 0, 0, 0, 0],
        [1, 0, 0, 0, 0],
    ],
    [
        [1, 0, 0, 0, 1],
        [1, 1, 0, 1, 1],
        [1, 0, 1, 0, 1],
        [1, 0, 0, 0, 1],
        [1, 0, 0, 0, 1],
    ],
];

pub struct ClockModeData {
    pub timer: Option<(TimerId, u64)>,
    pub last: i64,
}

pub struct ClockMode;

fn mode_mut(server: &mut Server, id: ModeId) -> Option<&mut PaneMode> {
    server
        .panes
        .get_mut(id.owner)?
        .modes
        .iter_mut()
        .find(|m| m.id == id)
}

fn data_mut(server: &mut Server, id: ModeId) -> Option<&mut ClockModeData> {
    mode_mut(server, id)?
        .data
        .as_mut()?
        .downcast_mut::<ClockModeData>()
}

fn now_seconds() -> i64 {
    rmux_util::time::Timestamp::now().sec
}

fn realtime_nanos() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos())
}

/// `window_clock_start_timer` delay: the rest of the current second, or one
/// second when that is not positive.
pub fn timer_delay(nanos: u32) -> Duration {
    let delay = 1_000_000 - i64::from(nanos / 1000);
    if delay <= 0 {
        Duration::from_secs(1)
    } else {
        Duration::from_micros(delay as u64)
    }
}

/// `gmtime_r(&t)->tm_sec`: POSIX seconds have no leap seconds.
pub fn utc_second(seconds: i64) -> i64 {
    seconds.rem_euclid(60)
}

fn start_timer(server: &mut Server, id: ModeId) {
    let delay = timer_delay(realtime_nanos());
    let timer = schedule_deferred(
        server,
        delay,
        Box::new(move |server| timer_callback(server, id)),
    );
    if let Some(data) = data_mut(server, id) {
        data.timer = Some(timer);
    }
}

fn timer_callback(server: &mut Server, id: ModeId) {
    let Some(data) = data_mut(server, id) else {
        return;
    };
    data.timer = None;
    let t = now_seconds();
    if utc_second(t) != utc_second(data.last) {
        data.last = t;
        with_screen(server, id, |server, screen| draw_screen(server, id, screen));
        if let Some(p) = server.panes.get_mut(id.owner) {
            p.flags.insert(PaneFlags::REDRAW);
        }
    }
    start_timer(server, id);
}

fn with_screen(server: &mut Server, id: ModeId, f: impl FnOnce(&mut Server, &mut Screen)) {
    let Some(mut screen) = mode_mut(server, id).and_then(|m| m.screen.take()) else {
        return;
    };
    f(server, &mut screen);
    match mode_mut(server, id) {
        Some(mode) => mode.screen = Some(screen),
        None => {
            let _ = screen.release(
                &mut server.hyperlinks,
                #[cfg(feature = "sixel")]
                None,
            );
        }
    }
}

/// `strftime` text for one clock style: `%l:%M[:%S] AM|PM` for 0 and 2,
/// `%H:%M[:%S]` for 1 and 3.
pub fn clock_text(style: i64, seconds: i64) -> Vec<u8> {
    let mut tim = [0u8; 64];
    let Some(tm) = rmux_sys::time::localtime(seconds) else {
        return Vec::new();
    };
    let mut len;
    if style == 0 || style == 2 {
        let fmt: &[u8] = if style == 2 { b"%l:%M:%S " } else { b"%l:%M " };
        len = rmux_sys::time::strftime(&mut tim, fmt, &tm);
        let suffix: &[u8] = if tm.hour() >= 12 { b"PM" } else { b"AM" };
        for &b in suffix {
            if len + 1 >= tim.len() {
                break;
            }
            tim[len] = b;
            len += 1;
        }
    } else {
        let fmt: &[u8] = if style == 3 { b"%H:%M:%S" } else { b"%H:%M" };
        len = rmux_sys::time::strftime(&mut tim, fmt, &tm);
    }
    tim[..len].to_vec()
}

/// `window_clock_table` index for one byte of the time text.
pub fn glyph_index(ch: u8) -> Option<usize> {
    match ch {
        b'0'..=b'9' => Some(usize::from(ch - b'0')),
        b':' => Some(10),
        b'A' => Some(11),
        b'P' => Some(12),
        b'M' => Some(13),
        _ => None,
    }
}

/// `window_clock_draw_screen` with the colour and style already read.
pub fn render(
    screen: &mut Screen,
    registry: &mut rmux_emu::hyperlinks::HyperlinkRegistry,
    colour: Colour,
    tim: &[u8],
) {
    let mut sink = ScreenOnlySink;
    let mut ctx = ScreenWriteCtx::start(
        screen,
        &mut sink,
        ScreenWritePolicy::default(),
        registry,
        #[cfg(feature = "sixel")]
        None,
    );
    let sx = ctx.screen.grid.sx();
    let sy = ctx.screen.grid.sy();
    let len = tim.len() as u32;

    ctx.clearscreen(Colour::DEFAULT);

    if sx < 6 * len || sy < 6 {
        if sx >= len && sy != 0 {
            let x = (sx / 2) - (len / 2);
            let y = sy / 2;
            ctx.cursormove(x as i32, y as i32, false);
            let mut gc = DEFAULT_CELL;
            gc.flags.insert(GridCellFlags::NOPALETTE);
            gc.fg = colour;
            ctx.puts(&gc, tim);
        }
        ctx.finish();
        return;
    }

    let mut x = (sx / 2) - 3 * len;
    let y = (sy / 2) - 3;
    let mut gc = DEFAULT_CELL;
    gc.flags.insert(GridCellFlags::NOPALETTE);
    gc.bg = colour;
    gc.fg = colour;
    gc.data = Utf8Data::set(b'#');
    for &ch in tim {
        let Some(idx) = glyph_index(ch) else {
            x += 6;
            continue;
        };
        for j in 0..5u32 {
            for i in 0..5u32 {
                ctx.cursormove((x + i) as i32, (y + j) as i32, false);
                if CLOCK_TABLE[idx][j as usize][i as usize] != 0 {
                    ctx.cell(&gc);
                }
            }
        }
        x += 6;
    }
    ctx.finish();
}

fn draw_screen(server: &mut Server, id: ModeId, screen: &mut Screen) {
    let Some(w) = server.panes.get(id.owner).map(|p| p.window) else {
        return;
    };
    let Some(wo) = server.windows.get(w).map(|w| w.options) else {
        return;
    };
    let mut ft = create_defaults(server, None, None, None, None, Some(id.owner));
    let mut gc = GridCell::default();
    style_apply(server, &mut gc, wo, b"clock-mode-colour", Some(&mut ft));
    ft.release(server);
    let colour = gc.fg;
    let style = server.options.get_number(wo, b"clock-mode-style");
    let tim = clock_text(style, now_seconds());
    render(screen, &mut server.hyperlinks, colour, &tim);
}

impl PaneModeDriver for ClockMode {
    fn init(&self, server: &mut Server, id: ModeId) -> Option<Screen> {
        let (sx, sy) = {
            let p = server.panes.get(id.owner)?;
            (p.base.grid.sx(), p.base.grid.sy())
        };
        mode_mut(server, id)?.data = Some(Box::new(ClockModeData {
            timer: None,
            last: now_seconds(),
        }));
        start_timer(server, id);

        let mut screen = Screen::new(
            sx,
            sy,
            0,
            ScreenResetPolicy::default(),
            &mut server.hyperlinks,
        )
        .ok()?;
        screen.mode.remove(ScreenMode::CURSOR);
        draw_screen(server, id, &mut screen);
        Some(screen)
    }

    fn free(&self, server: &mut Server, mut mode: PaneMode) {
        if let Some(data) = mode
            .data
            .take()
            .and_then(|d| d.downcast::<ClockModeData>().ok())
            && let Some((timer, key)) = data.timer
        {
            server.event_loop.cancel(timer);
            server.deferred.remove(&key);
        }
        if let Some(mut screen) = mode.screen.take() {
            let _ = screen.release(
                &mut server.hyperlinks,
                #[cfg(feature = "sixel")]
                None,
            );
        }
    }

    fn resize(&self, server: &mut Server, id: ModeId, sx: u32, sy: u32) {
        with_screen(server, id, |server, screen| {
            screen.resize(
                sx,
                sy,
                false,
                #[cfg(feature = "sixel")]
                None,
            );
            draw_screen(server, id, screen);
        });
    }

    fn key(
        &self,
        server: &mut Server,
        id: ModeId,
        _client: ClientId,
        _key: KeyCode,
        _mouse: Option<&crate::client::ResolvedMouseEvent>,
    ) {
        let _ = pane_reset_mode(server, id.owner);
    }

    fn append_output(
        &self,
        _server: &mut Server,
        _id: ModeId,
        _bytes: &[u8],
    ) -> Result<(), ModelError> {
        Err(ModelError::Message(b"clock-mode has no output".to_vec()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmux_emu::hyperlinks::HyperlinkRegistry;

    #[test]
    fn timer_delay_math() {
        assert_eq!(timer_delay(0), Duration::from_secs(1));
        assert_eq!(timer_delay(999_999_999), Duration::from_micros(1));
        assert_eq!(timer_delay(250_000_000), Duration::from_micros(750_000));
    }

    #[test]
    fn utc_second_uses_tm_sec_only() {
        assert_eq!(utc_second(60), utc_second(120));
        assert_ne!(utc_second(61), utc_second(120));
        assert_eq!(utc_second(-1), 59);
    }

    fn cells(screen: &Screen, y: u32) -> Vec<u8> {
        (0..screen.grid.sx())
            .map(|x| screen.grid.view_get_cell(x, y).data.data[0])
            .collect()
    }

    #[test]
    fn glyph_positions_for_five_character_time() {
        let mut registry = HyperlinkRegistry::new();
        let mut screen =
            Screen::new(40, 10, 0, ScreenResetPolicy::default(), &mut registry).unwrap();
        render(&mut screen, &mut registry, Colour::from_raw(1), b"12:05");
        // x = 40/2 - 3*5 = 5, y = 10/2 - 3 = 2.
        let row0 = cells(&screen, 2);
        let expect = |xs: &[u32]| -> Vec<u8> {
            (0..40u32)
                .map(|x| if xs.contains(&x) { b'#' } else { b' ' })
                .collect()
        };
        // '1' top row: column 4 only; '2' top row: all five.
        let mut on = vec![5 + 4];
        on.extend(11..16);
        // ':' top row empty; '0' top row all five at 23..28; '5' at 29..34.
        on.extend(23..28);
        on.extend(29..34);
        assert_eq!(row0, expect(&on));
        // Colon dots at rows y+1 and y+3, column 17+2.
        assert_eq!(cells(&screen, 3)[19], b'#');
        assert_eq!(cells(&screen, 5)[19], b'#');
        assert_eq!(cells(&screen, 4)[19], b' ');
        assert_eq!(screen.grid.view_get_cell(9, 2).bg, Colour::from_raw(1));
        assert!(
            screen
                .grid
                .view_get_cell(9, 2)
                .flags
                .contains(GridCellFlags::NOPALETTE)
        );
        screen
            .release(
                &mut registry,
                #[cfg(feature = "sixel")]
                None,
            )
            .unwrap();
    }

    #[test]
    fn compact_path_centres_or_clears() {
        let mut registry = HyperlinkRegistry::new();
        let mut screen =
            Screen::new(20, 5, 0, ScreenResetPolicy::default(), &mut registry).unwrap();
        render(&mut screen, &mut registry, Colour::from_raw(2), b"12:05");
        let row = cells(&screen, 2);
        assert_eq!(&row[8..13], b"12:05");
        assert_eq!(screen.grid.view_get_cell(8, 2).fg, Colour::from_raw(2));
        assert_eq!(screen.grid.view_get_cell(8, 2).bg, Colour::DEFAULT);
        // Too narrow: cleared screen.
        let mut small = Screen::new(4, 5, 0, ScreenResetPolicy::default(), &mut registry).unwrap();
        render(&mut small, &mut registry, Colour::from_raw(2), b"12:05");
        assert!((0..5).all(|y| cells(&small, y).iter().all(|&c| c == b' ')));
        screen
            .release(
                &mut registry,
                #[cfg(feature = "sixel")]
                None,
            )
            .unwrap();
        small
            .release(
                &mut registry,
                #[cfg(feature = "sixel")]
                None,
            )
            .unwrap();
    }

    #[test]
    fn clock_text_styles() {
        let t = 1_700_000_000;
        let tm = rmux_sys::time::localtime(t).unwrap();
        let s1 = clock_text(1, t);
        assert_eq!(s1, format!("{:02}:{:02}", tm.hour(), tm.min()).into_bytes());
        let s3 = clock_text(3, t);
        assert_eq!(
            s3,
            format!("{:02}:{:02}:{:02}", tm.hour(), tm.min(), tm.sec()).into_bytes()
        );
        let s0 = clock_text(0, t);
        assert!(s0.ends_with(if tm.hour() >= 12 { b"PM" } else { b"AM" }));
        assert_eq!(s0.len(), 8);
        let s2 = clock_text(2, t);
        assert_eq!(s2.len(), 11);
    }
    #[test]
    fn fixed_noon_and_midnight_glyphs_match_pinned_table_in_all_styles() {
        const GLYPHS: [[&[u8]; 5]; 14] = [
            [b"#####", b"#   #", b"#   #", b"#   #", b"#####"],
            [b"    #", b"    #", b"    #", b"    #", b"    #"],
            [b"#####", b"    #", b"#####", b"#    ", b"#####"],
            [b"#####", b"    #", b"#####", b"    #", b"#####"],
            [b"#   #", b"#   #", b"#####", b"    #", b"    #"],
            [b"#####", b"#    ", b"#####", b"    #", b"#####"],
            [b"#####", b"#    ", b"#####", b"#   #", b"#####"],
            [b"#####", b"    #", b"    #", b"    #", b"    #"],
            [b"#####", b"#   #", b"#####", b"#   #", b"#####"],
            [b"#####", b"#   #", b"#####", b"    #", b"#####"],
            [b"     ", b"  #  ", b"     ", b"  #  ", b"     "],
            [b"#####", b"#   #", b"#####", b"#   #", b"#   #"],
            [b"#####", b"#   #", b"#####", b"#    ", b"#    "],
            [b"#   #", b"## ##", b"# # #", b"#   #", b"#   #"],
        ];
        let times: [&[u8]; 8] = [
            b"12:05 PM",
            b"12:05",
            b"12:05:00 PM",
            b"12:05:00",
            b"11:59 PM",
            b"23:59",
            b"11:59:59 PM",
            b"23:59:59",
        ];
        let mut registry = HyperlinkRegistry::new();
        let mut large =
            Screen::new(90, 14, 0, ScreenResetPolicy::default(), &mut registry).unwrap();
        let mut compact =
            Screen::new(20, 5, 0, ScreenResetPolicy::default(), &mut registry).unwrap();
        let colour = Colour::from_raw(4);
        for tim in times {
            render(&mut large, &mut registry, colour, tim);
            let start = 45 - tim.len() * 3;
            for y in 0..14 {
                for x in 0..90 {
                    let expected = if (4..9).contains(&y) && x >= start && x < start + tim.len() * 6
                    {
                        let slot = (x - start) / 6;
                        let column = (x - start) % 6;
                        glyph_index(tim[slot])
                            .filter(|_| column < 5)
                            .map_or(b' ', |idx| GLYPHS[idx][y - 4][column])
                    } else {
                        b' '
                    };
                    let gc = large.grid.view_get_cell(x as u32, y as u32);
                    assert_eq!(gc.data.data[0], expected, "time {tim:?}, ({x},{y})");
                    if expected == b'#' {
                        assert_eq!((gc.fg, gc.bg), (colour, colour));
                        assert!(gc.flags.contains(GridCellFlags::NOPALETTE));
                    }
                }
            }
            render(&mut compact, &mut registry, colour, tim);
            let start = 10 - tim.len() / 2;
            assert_eq!(&cells(&compact, 2)[start..start + tim.len()], tim);
            for x in start..start + tim.len() {
                let gc = compact.grid.view_get_cell(x as u32, 2);
                assert_eq!((gc.fg, gc.bg), (colour, Colour::DEFAULT));
            }
        }
        for screen in [&mut large, &mut compact] {
            screen
                .release(
                    &mut registry,
                    #[cfg(feature = "sixel")]
                    None,
                )
                .unwrap();
        }
    }
}
