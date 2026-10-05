// Ported from tmux tmux.h, menu.c, screen-write.c (screen_write_menu, screen_write_box title) @ 8f25579c
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

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct MenuFlags(pub u32);
impl MenuFlags {
    pub const NOMOUSE: Self = Self(1);
    pub const TAB: Self = Self(2);
    pub const STAYOPEN: Self = Self(4);
    pub const fn bits(self) -> u32 {
        self.0
    }
    pub const fn from_bits_retain(bits: u32) -> Self {
        Self(bits)
    }
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }
    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }
}
impl std::ops::BitOr for MenuFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for MenuFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for MenuFlags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

use crate::client::KeyEvent;
use crate::cmd::find::{CmdFindFlags, CmdFindState};
use crate::cmd::queue::{QueueEvent, QueueStateFlags};
use crate::ids::{ClientId, QueueItemId, WindowId};
use crate::model::Server;
use crate::ui::redraw::redraw_invalidate_scene;
use crate::ui::styles;
use rmux_emu::cell::{DEFAULT_CELL, GridAttributes, GridCell, GridCellFlags};
use rmux_emu::colour::Colour;
use rmux_emu::hyperlinks::HyperlinkRegistry;
use rmux_emu::screen::borders::border_cell;
use rmux_emu::screen::write::{ScreenOnlySink, ScreenWriteCtx, ScreenWritePolicy};
use rmux_emu::screen::{BorderCell, BoxLines, Screen, ScreenMode, ScreenResetPolicy};
use rmux_emu::style::Style;
use rmux_util::bytes::ByteString;
use rmux_util::key::{KeyCode, KeyMasks, KeyModifiers, MouseButton, MouseButtonBits, SpecialKey};
use rmux_util::utf8::Utf8Data;

/// A menu item; `name` None is a separator line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MenuItem {
    pub name: Option<ByteString>,
    pub key: KeyCode,
    pub command: Option<ByteString>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Menu {
    pub title: ByteString,
    pub items: Vec<MenuItem>,
    pub width: u32,
    pub item_width: u32,
}

impl Menu {
    pub fn count(&self) -> u32 {
        self.items.len() as u32
    }
}

/// menu_choice_cb; `None` choice is UINT_MAX (cancelled).
pub type MenuChoice = Box<dyn FnOnce(&mut Server, &Menu, Option<u32>, KeyCode)>;

pub struct MenuData {
    pub window: WindowId,
    pub flags: MenuFlags,
    style: Option<ByteString>,
    border_style: Option<ByteString>,
    selected_style: Option<ByteString>,
    style_gc: GridCell,
    border_style_gc: GridCell,
    selected_style_gc: GridCell,
    border_lines: BoxLines,
    fs: CmdFindState,
    key: KeyCode,
    event: QueueEvent,
    registry: HyperlinkRegistry,
    screen: Screen,
    px: u32,
    py: u32,
    menu: Menu,
    choice: i32,
    cb: Option<MenuChoice>,
}

impl MenuData {
    pub fn x(&self) -> u32 {
        self.px
    }
    pub fn y(&self) -> u32 {
        self.py
    }
    pub fn width(&self) -> u32 {
        menu_get_size(&self.menu, self.border_lines).0
    }
    pub fn height(&self) -> u32 {
        menu_get_size(&self.menu, self.border_lines).1
    }
    pub fn menu(&self) -> &Menu {
        &self.menu
    }
    pub fn choice(&self) -> i32 {
        self.choice
    }
}

pub fn menu_get_size(menu: &Menu, lines: BoxLines) -> (u32, u32) {
    if lines == BoxLines::None {
        (menu.item_width + 2, menu.count())
    } else {
        (menu.width + 4, menu.count() + 2)
    }
}

pub fn menu_add_items(
    srv: &mut Server,
    menu: &mut Menu,
    items: &[MenuItem],
    item: Option<QueueItemId>,
    c: ClientId,
    fs: Option<&CmdFindState>,
) {
    for entry in items {
        if entry.name.is_none() {
            break;
        }
        menu_add_item(srv, menu, Some(entry), item, c, fs);
    }
}

fn format_item(
    srv: &mut Server,
    item: Option<QueueItemId>,
    text: &[u8],
    c: ClientId,
    fs: Option<&CmdFindState>,
) -> ByteString {
    match fs {
        Some(fs) => crate::format::single_from_state(srv, item, Some(c), fs, text),
        None => crate::format::single(
            srv,
            item,
            crate::format::FormatContext {
                evaluated_client: Some(c),
                ..Default::default()
            },
            text,
        ),
    }
}

pub fn menu_add_item(
    srv: &mut Server,
    menu: &mut Menu,
    entry: Option<&MenuItem>,
    item: Option<QueueItemId>,
    c: ClientId,
    fs: Option<&CmdFindState>,
) {
    let line = entry.is_none_or(|e| e.name.as_ref().is_none_or(|n| n.is_empty()));
    if line && menu.items.is_empty() {
        return;
    }
    if line && menu.items.last().is_some_and(|i| i.name.is_none()) {
        return;
    }
    if line {
        menu.items.push(MenuItem::default());
        return;
    }
    let entry = entry.expect("item");
    let name = entry.name.as_ref().map_or(&[][..], |n| n.as_bytes());
    let s = format_item(srv, item, name, c, fs);
    if s.is_empty() {
        return;
    }
    let (tty_sx, _) = styles::client_size(srv, c);
    let mut max_width = tty_sx.saturating_sub(4);

    let slen = s.len() as u32;
    let mut key: Option<Vec<u8>> = None;
    if s[0] != b'-' && entry.key.0 != SpecialKey::UNKNOWN && entry.key.0 != SpecialKey::NONE {
        let name = rmux_tty::key_string::key_name(entry.key, false);
        let keylen = name.len() as u32 + 3; /* 3 = space and two brackets */
        if keylen <= max_width / 4 {
            max_width -= keylen;
            key = Some(name);
        } else if keylen >= max_width || slen >= max_width - keylen {
            key = None;
        } else {
            key = Some(name);
        }
    }

    let mut suffix: &[u8] = b"";
    if slen > max_width {
        max_width -= 1;
        suffix = b">";
    }
    let trimmed = crate::format::draw::trim_right(&s, max_width);
    let mut new_name: Vec<u8> = trimmed.to_vec();
    new_name.extend_from_slice(suffix);
    if let Some(key) = &key {
        new_name.extend_from_slice(b"#[default] #[align=right](");
        new_name.extend_from_slice(key);
        new_name.push(b')');
    }

    let command = entry
        .command
        .as_ref()
        .map(|cmd| format_item(srv, item, cmd, c, fs));

    let mut width = crate::format::draw::width(&new_name);
    if new_name[0] == b'-' {
        width -= 1;
    }
    if width > menu.item_width {
        menu.item_width = width;
    }
    if width > menu.width {
        menu.width = width;
    }
    menu.items.push(MenuItem {
        name: Some(new_name.into()),
        key: entry.key,
        command,
    });
}

pub fn menu_create(title: &[u8]) -> Menu {
    Menu {
        title: title.into(),
        items: Vec::new(),
        width: crate::format::draw::width(title),
        item_width: 0,
    }
}

/// menu_free: drop an unattached menu.
pub fn menu_free(menu: Menu) {
    drop(menu);
}

fn menu_reapply_styles(srv: &mut Server, w: WindowId) {
    let Some(md) = srv.windows.get(w).and_then(|w| w.menu.as_ref()) else {
        return;
    };
    let fs = md.fs;
    let (style, selected, border) = (
        md.style.clone(),
        md.selected_style.clone(),
        md.border_style.clone(),
    );
    let o = srv.windows.get(w).expect("window").options;
    let mut ft = styles::create_defaults(srv, None, None, fs.s, fs.wl, fs.wp);

    let mut apply = |srv: &mut Server, option: &[u8], override_: &Option<ByteString>| {
        let mut gc = DEFAULT_CELL;
        styles::style_apply(srv, &mut gc, o, option, Some(&mut ft));
        if let Some(s) = override_ {
            let mut sytmp = Style::from_cell(DEFAULT_CELL);
            if sytmp.parse(&gc, s, &mut srv.hyperlinks).is_ok() {
                gc.fg = sytmp.gc.fg;
                gc.bg = sytmp.gc.bg;
            }
        }
        gc
    };
    let style_gc = apply(srv, b"menu-style", &style);
    let selected_gc = apply(srv, b"menu-selected-style", &selected);
    let border_gc = apply(srv, b"menu-border-style", &border);
    ft.release(srv);
    if let Some(md) = srv.windows.get_mut(w).and_then(|w| w.menu.as_mut()) {
        md.style_gc = style_gc;
        md.selected_style_gc = selected_gc;
        md.border_style_gc = border_gc;
    }
}

/// Redraw the menu screen.
pub fn menu_update(srv: &mut Server, w: WindowId) {
    menu_reapply_styles(srv, w);
    let Some(md) = srv.windows.get_mut(w).and_then(|w| w.menu.as_mut()) else {
        return;
    };
    let (width, height) = menu_get_size(&md.menu, md.border_lines);
    let MenuData {
        registry,
        screen,
        menu,
        choice,
        border_lines,
        style_gc,
        border_style_gc,
        selected_style_gc,
        ..
    } = md;
    let mut sink = ScreenOnlySink;
    let mut ctx = ScreenWriteCtx::start(screen, &mut sink, ScreenWritePolicy::default(), registry);
    ctx.clearscreen(Colour::DEFAULT);
    if *border_lines != BoxLines::None {
        draw_box(
            &mut ctx,
            width,
            height,
            *border_lines,
            Some(border_style_gc),
            Some(&menu.title),
        );
    }
    draw_menu(
        &mut ctx,
        menu,
        *choice,
        *border_lines,
        style_gc,
        border_style_gc,
        selected_style_gc,
    );
    ctx.finish();
}

/// Free menu data: fires an unused callback with no choice.
fn menu_free_data(srv: &mut Server, mut md: MenuData) {
    if let Some(cb) = md.cb.take() {
        cb(srv, &md.menu, None, KeyCode(SpecialKey::NONE));
    }
    let _ = md.screen.release(&mut md.registry);
}

pub fn menu_close(srv: &mut Server, w: WindowId) {
    let Some(md) = srv.windows.get_mut(w).and_then(|win| win.menu.take()) else {
        return;
    };
    if let Some(win) = srv.windows.get_mut(w) {
        win.menu_active = false;
    }
    menu_free_data(srv, md);
    if let Some(win) = srv.windows.get_mut(w) {
        redraw_invalidate_scene(win);
    }
    crate::model::window::window_update_focus(srv, w);
    crate::server::operations::server_redraw_window(srv, w);
}

pub fn menu_destroy(srv: &mut Server, w: WindowId) {
    let Some(md) = srv.windows.get_mut(w).and_then(|win| win.menu.take()) else {
        return;
    };
    if let Some(win) = srv.windows.get_mut(w) {
        win.menu_active = false;
    }
    menu_free_data(srv, md);
}

pub fn menu_get_cursor(md: &MenuData) -> (u32, u32) {
    let border = u32::from(md.border_lines != BoxLines::None);
    let cx = md.px + 1 + border;
    let cy = if md.choice == -1 {
        md.py
    } else {
        md.py + border + md.choice as u32
    };
    (cx, cy)
}

pub fn menu_screen(md: &MenuData) -> &Screen {
    &md.screen
}

pub fn menu_width(md: &MenuData) -> u32 {
    md.width()
}
pub fn menu_height(md: &MenuData) -> u32 {
    md.height()
}
pub fn menu_x(md: &MenuData) -> u32 {
    md.px
}
pub fn menu_y(md: &MenuData) -> u32 {
    md.py
}

fn is_disabled(name: &Option<ByteString>) -> bool {
    match name {
        None => true,
        Some(n) => n.first() == Some(&b'-'),
    }
}

/// Handle a key for the window menu. Returns true when the menu closes.
pub fn menu_key(srv: &mut Server, c: ClientId, w: WindowId, event: &KeyEvent) -> bool {
    let Some(md) = srv.windows.get_mut(w).and_then(|win| win.menu.as_mut()) else {
        return false;
    };
    let n = md.menu.count() as i32;
    let mut old = md.choice;
    let m = &event.mouse;

    if event.key.is_mouse() {
        let b = MouseButtonBits(m.b);
        // A mouse move with no button held reports as a release: highlight
        // only, never select or close (menu.c:365-373).
        let mv = b.is_drag() && b.is_release();
        if md.flags.contains(MenuFlags::NOMOUSE) {
            return b.buttons() != MouseButton::Button1 as u32;
        }
        let border = u32::from(md.border_lines != BoxLines::None);
        let width = md.width();
        if m.x < md.px
            || m.x >= md.px + width
            || m.y < md.py + border
            || m.y >= md.py + border + n as u32
        {
            if !md.flags.contains(MenuFlags::STAYOPEN) {
                if !mv && b.is_release() {
                    return true;
                }
            } else if !b.is_release() && !b.is_wheel() && !b.is_drag() {
                return true;
            }
            if md.choice != -1 {
                md.choice = -1;
                crate::server::operations::server_redraw_window_menu(srv, w);
            }
            return false;
        }
        let chosen = if !md.flags.contains(MenuFlags::STAYOPEN) {
            !mv && b.is_release()
        } else {
            !b.is_wheel() && !b.is_drag()
        };
        if chosen {
            return menu_chosen(srv, c, w);
        }
        md.choice = (m.y - (md.py + border)) as i32;
        if md.choice != old {
            crate::server::operations::server_redraw_window_menu(srv, w);
        }
        return false;
    }

    let key = event.key.0 & !KeyMasks::FLAGS;
    for i in 0..n as usize {
        let item = &md.menu.items[i];
        if is_disabled(&item.name) {
            continue;
        }
        if key == item.key.0 & !KeyMasks::FLAGS {
            md.choice = i as i32;
            return menu_chosen(srv, c, w);
        }
    }
    let items = &md.menu.items;
    let disabled = |i: i32| is_disabled(&items[i as usize].name);
    let ctrl = |ch: u8| u64::from(ch) | KeyModifiers::CTRL.0;
    match key {
        k if k == SpecialKey::BTAB || k == SpecialKey::UP || k == u64::from(b'k') => {
            if old == -1 {
                old = 0;
            }
            loop {
                if md.choice == -1 || md.choice == 0 {
                    md.choice = n - 1;
                } else {
                    md.choice -= 1;
                }
                if !(disabled(md.choice) && md.choice != old) {
                    break;
                }
            }
            crate::server::operations::server_redraw_window_menu(srv, w);
            return false;
        }
        k if k == SpecialKey::BSPACE => {
            if !md.flags.contains(MenuFlags::TAB) {
                return false;
            }
            return true;
        }
        k if k == 0x09 || k == SpecialKey::DOWN || k == u64::from(b'j') => {
            if k == 0x09 {
                if !md.flags.contains(MenuFlags::TAB) {
                    return false;
                }
                if md.choice == n - 1 {
                    return true;
                }
            }
            if old == -1 {
                old = 0;
            }
            loop {
                if md.choice == -1 || md.choice == n - 1 {
                    md.choice = 0;
                } else {
                    md.choice += 1;
                }
                if !(disabled(md.choice) && md.choice != old) {
                    break;
                }
            }
            crate::server::operations::server_redraw_window_menu(srv, w);
            return false;
        }
        k if k == SpecialKey::PPAGE || k == ctrl(b'b') => {
            if md.choice < 6 {
                md.choice = 0;
            } else {
                let mut i = 5;
                while i > 0 {
                    md.choice -= 1;
                    if md.choice != 0 && !disabled(md.choice) {
                        i -= 1;
                    } else if md.choice == 0 {
                        break;
                    }
                }
            }
            crate::server::operations::server_redraw_window_menu(srv, w);
        }
        k if k == SpecialKey::NPAGE => {
            if md.choice > n - 6 {
                md.choice = n - 1;
            } else {
                let mut i = 5;
                while i > 0 {
                    md.choice += 1;
                    if md.choice != n - 1 && !disabled(md.choice) {
                        i -= 1;
                    } else if md.choice == n - 1 {
                        break;
                    }
                }
            }
            while disabled(md.choice) && md.choice != 0 {
                md.choice -= 1;
            }
            crate::server::operations::server_redraw_window_menu(srv, w);
        }
        k if k == u64::from(b'g') || k == SpecialKey::HOME => {
            md.choice = 0;
            while disabled(md.choice) && md.choice != n - 1 {
                md.choice += 1;
            }
            crate::server::operations::server_redraw_window_menu(srv, w);
        }
        k if k == u64::from(b'G') || k == SpecialKey::END => {
            md.choice = n - 1;
            while disabled(md.choice) && md.choice != 0 {
                md.choice -= 1;
            }
            crate::server::operations::server_redraw_window_menu(srv, w);
        }
        k if k == ctrl(b'f') => {}
        k if k == u64::from(b'\r') => return menu_chosen(srv, c, w),
        k if k == 0x1b
            || k == ctrl(b'[')
            || k == ctrl(b'c')
            || k == ctrl(b'g')
            || k == u64::from(b'q') =>
        {
            return true;
        }
        _ => {}
    }
    false
}

/// The `chosen:` label of menu_key.
fn menu_chosen(srv: &mut Server, c: ClientId, w: WindowId) -> bool {
    let Some(md) = srv.windows.get_mut(w).and_then(|win| win.menu.as_mut()) else {
        return true;
    };
    if md.choice == -1 {
        return true;
    }
    let item = md.menu.items[md.choice as usize].clone();
    if is_disabled(&item.name) {
        return !md.flags.contains(MenuFlags::STAYOPEN);
    }
    if md.cb.is_some() {
        // Take the menu data out while the callback runs so it may install
        // a replacement; the old data goes back only if nothing replaced it.
        let mut md = srv
            .windows
            .get_mut(w)
            .and_then(|win| win.menu.take())
            .expect("menu");
        let cb = md.cb.take().expect("callback");
        let choice = md.choice as u32;
        cb(srv, &md.menu, Some(choice), item.key);
        match srv.windows.get_mut(w) {
            Some(win) if win.menu.is_none() => win.menu = Some(md),
            _ => menu_free_data(srv, md),
        }
        return true;
    }

    let event = if md.key.0 != SpecialKey::NONE {
        Some(md.event)
    } else {
        None
    };
    let fs = md.fs;
    let mut store = std::mem::take(&mut srv.queue);
    let state = store.new_state(
        &*srv as &dyn crate::cmd::find::ModelView,
        Some(&fs),
        event.as_ref(),
        QueueStateFlags::default(),
    );
    srv.queue = store;
    let Ok(state) = state else {
        return true;
    };
    let mut input = crate::cmd::parse::CmdParseInput {
        client: Some(c),
        ..Default::default()
    };
    let command = item.command.clone().unwrap_or_default();
    if let Err(error) = crate::cmd::parse::and_append(srv, &command, &mut input, Some(c), state) {
        if let Ok(batch) = srv.queue.get_error(error.message()) {
            let _ = crate::cmd::queue::append(srv, Some(c), batch);
        }
    }
    let _ = srv.queue.free_state(state);
    true
}

/// Clamp the menu position after a window resize.
pub fn menu_resize(md: &mut MenuData, sx: u32, sy: u32) {
    let (msx, msy) = menu_get_size(&md.menu, md.border_lines);
    let mut nx = md.px;
    let mut ny = md.py;
    if nx + msx > sx {
        nx = sx.saturating_sub(msx);
    }
    if ny + msy > sy {
        ny = sy.saturating_sub(msy);
    }
    md.px = nx;
    md.py = ny;
}

#[allow(clippy::too_many_arguments)]
pub fn menu_display(
    srv: &mut Server,
    menu: Menu,
    flags: MenuFlags,
    starting_choice: i32,
    item: Option<QueueItemId>,
    mut px: u32,
    mut py: u32,
    c: ClientId,
    mut lines: BoxLines,
    style: Option<&[u8]>,
    selected_style: Option<&[u8]>,
    border_style: Option<&[u8]>,
    fs: Option<&CmdFindState>,
    cb: Option<MenuChoice>,
) {
    let w = match fs {
        None => styles::client_window(srv, c),
        Some(fs) => fs.w,
    };
    let (Some(w), Some(win)) = (w, w.and_then(|w| srv.windows.get(w))) else {
        menu_free(menu);
        return;
    };
    let (o, wsx, wsy) = (win.options, win.sx, win.sy);

    if lines == BoxLines::Default {
        lines = BoxLines::try_from(srv.options.get_number(o, b"menu-border-lines") as i32)
            .unwrap_or(BoxLines::Single);
    }
    let (sx, sy) = menu_get_size(&menu, lines);
    if sx >= wsx {
        px = 0;
    } else if px + sx > wsx {
        px = wsx - sx;
    }
    if sy >= wsy {
        py = 0;
    } else if py + sy > wsy {
        py = wsy - sy;
    }
    if let Some(win) = srv.windows.get_mut(w) {
        win.menu_last_px = px;
        win.menu_last_py = py;
    }

    let mut key = KeyCode(SpecialKey::NONE);
    let mut event = QueueEvent::default();
    if let Some(item) = item {
        if let Some(st) = srv
            .queue
            .items
            .get(item)
            .and_then(|it| srv.queue.states.get(it.state))
        {
            event = st.event;
            key = event.key;
        }
    }

    let md_fs = match fs {
        Some(fs) => {
            let mut s = CmdFindState::clear(CmdFindFlags(0));
            s.copy_target_from(fs);
            s
        }
        None => crate::cmd::find::from_window(srv, w, CmdFindFlags(0))
            .unwrap_or_else(|| CmdFindState::clear(CmdFindFlags(0))),
    };
    let mut registry = HyperlinkRegistry::new();
    let Ok(mut screen) = Screen::new(
        sx.max(1),
        sy.max(1),
        0,
        ScreenResetPolicy::default(),
        &mut registry,
    ) else {
        menu_free(menu);
        return;
    };
    if !flags.contains(MenuFlags::NOMOUSE) {
        screen
            .mode
            .insert(ScreenMode::MOUSE_ALL | ScreenMode::MOUSE_BUTTON);
    }
    screen.mode.remove(ScreenMode::CURSOR);

    let mut choice = -1;
    if flags.contains(MenuFlags::NOMOUSE) {
        let count = menu.count() as i32;
        if starting_choice >= count {
            let starting_choice = count - 1;
            let mut i = starting_choice + 1;
            loop {
                if !is_disabled(&menu.items[(i - 1) as usize].name) {
                    choice = i - 1;
                    break;
                }
                i -= 1;
                if i == 0 {
                    i = count;
                }
                if i == starting_choice + 1 {
                    break;
                }
            }
        } else if starting_choice >= 0 {
            let mut i = starting_choice;
            loop {
                if !is_disabled(&menu.items[i as usize].name) {
                    choice = i;
                    break;
                }
                i += 1;
                if i == count {
                    i = 0;
                }
                if i == starting_choice {
                    break;
                }
            }
        }
    }

    let md = MenuData {
        window: w,
        flags,
        style: style.map(ByteString::from),
        border_style: border_style.map(ByteString::from),
        selected_style: selected_style.map(ByteString::from),
        style_gc: DEFAULT_CELL,
        border_style_gc: DEFAULT_CELL,
        selected_style_gc: DEFAULT_CELL,
        border_lines: lines,
        fs: md_fs,
        key,
        event,
        registry,
        screen,
        px,
        py,
        menu,
        choice,
        cb,
    };

    menu_close(srv, w);
    let Some(win) = srv.windows.get_mut(w) else {
        menu_free_data(srv, md);
        return;
    };
    win.menu = Some(md);
    win.menu_active = true;
    redraw_invalidate_scene(win);
    crate::model::window::window_update_focus(srv, w);
    crate::server::operations::server_redraw_window(srv, w);
}

/* screen-write.c: screen_write_menu and the titled screen_write_box. */

fn box_border_set(lines: BoxLines, cell_type: BorderCell, gc: &mut GridCell) {
    border_cell(lines, cell_type, gc);
}

/// screen_write_box with a formatted title.
pub fn draw_box(
    ctx: &mut ScreenWriteCtx<'_>,
    nx: u32,
    ny: u32,
    lines: BoxLines,
    gcp: Option<&GridCell>,
    title: Option<&[u8]>,
) {
    let cx = ctx.screen.cx;
    let cy = ctx.screen.cy;
    let mut gc = gcp.copied().unwrap_or(DEFAULT_CELL);
    gc.attr.insert(GridAttributes::CHARSET);
    gc.flags.insert(GridCellFlags::NOPALETTE);

    box_border_set(lines, BorderCell::Rd, &mut gc);
    ctx.cell(&gc);
    box_border_set(lines, BorderCell::Lr, &mut gc);
    for _ in 1..nx.saturating_sub(1) {
        ctx.cell(&gc);
    }
    box_border_set(lines, BorderCell::Ld, &mut gc);
    ctx.cell(&gc);

    ctx.cursormove(cx as i32, (cy + ny - 1) as i32, false);
    box_border_set(lines, BorderCell::Ru, &mut gc);
    ctx.cell(&gc);
    box_border_set(lines, BorderCell::Lr, &mut gc);
    for _ in 1..nx.saturating_sub(1) {
        ctx.cell(&gc);
    }
    box_border_set(lines, BorderCell::Lu, &mut gc);
    ctx.cell(&gc);

    box_border_set(lines, BorderCell::Ud, &mut gc);
    for i in 1..ny.saturating_sub(1) {
        ctx.cursormove(cx as i32, (cy + i) as i32, false);
        ctx.cell(&gc);
        ctx.cursormove((cx + nx - 1) as i32, (cy + i) as i32, false);
        ctx.cell(&gc);
    }

    if let Some(title) = title {
        gc.attr.remove(GridAttributes::CHARSET);
        ctx.cursormove((cx + 2) as i32, cy as i32, false);
        crate::format::draw::draw(ctx, &gc, nx.saturating_sub(4), title, None, false);
    }
    ctx.cursormove(cx as i32, cy as i32, false);
}

fn putc(ctx: &mut ScreenWriteCtx<'_>, gc: &GridCell, ch: u8) {
    let mut cell = *gc;
    cell.data = Utf8Data::set(ch);
    ctx.cell(&cell);
}

/// screen_write_menu.
pub fn draw_menu(
    ctx: &mut ScreenWriteCtx<'_>,
    menu: &Menu,
    choice: i32,
    lines: BoxLines,
    menu_gc: &GridCell,
    border_gc: &GridCell,
    choice_gc: &GridCell,
) {
    let cx = ctx.screen.cx;
    let cy = ctx.screen.cy;
    let mut default_gc = *menu_gc;
    let (border, width) = if lines == BoxLines::None {
        (0, menu.item_width)
    } else {
        (1, menu.width)
    };
    if border == 1 {
        draw_box(
            ctx,
            width + 4,
            menu.count() + 2,
            lines,
            Some(border_gc),
            Some(&menu.title),
        );
    }
    let mut gc = default_gc;
    for (i, item) in menu.items.iter().enumerate() {
        let i = i as u32;
        let Some(name) = &item.name else {
            ctx.cursormove(cx as i32, (cy + border + i) as i32, false);
            ctx.hline(width + 2 + (2 * border), true, true, lines, Some(border_gc));
            continue;
        };
        if choice >= 0 && i == choice as u32 && name.first() != Some(&b'-') {
            gc = *choice_gc;
        }
        ctx.cursormove((cx + border) as i32, (cy + border + i) as i32, false);
        for _ in 0..width + 2 {
            putc(ctx, &gc, b' ');
        }
        ctx.cursormove((cx + border + 1) as i32, (cy + border + i) as i32, false);
        if name.first() == Some(&b'-') {
            default_gc.attr.insert(GridAttributes::DIM);
            let dim = if gc == *choice_gc { gc } else { default_gc };
            crate::format::draw::draw(ctx, &dim, width, &name[1..], None, false);
            default_gc.attr.remove(GridAttributes::DIM);
            continue;
        }
        crate::format::draw::draw(ctx, &gc, width, name, None, false);
        gc = default_gc;
    }
    ctx.cursormove(cx as i32, cy as i32, false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_with_and_without_borders() {
        let mut m = menu_create(b"title");
        assert_eq!(m.width, 5);
        m.items.push(MenuItem {
            name: Some(b"abc".as_slice().into()),
            key: KeyCode(SpecialKey::NONE),
            command: None,
        });
        m.items.push(MenuItem::default());
        m.item_width = 3;
        m.width = 5;
        assert_eq!(menu_get_size(&m, BoxLines::Single), (9, 4));
        assert_eq!(menu_get_size(&m, BoxLines::None), (5, 2));
    }

    #[test]
    fn resize_clamps_position_only() {
        let mut md = MenuData {
            window: crate::ids::ArenaId::from_parts(0, 0),
            flags: MenuFlags::default(),
            style: None,
            border_style: None,
            selected_style: None,
            style_gc: DEFAULT_CELL,
            border_style_gc: DEFAULT_CELL,
            selected_style_gc: DEFAULT_CELL,
            border_lines: BoxLines::Single,
            fs: CmdFindState::default(),
            key: KeyCode(SpecialKey::NONE),
            event: QueueEvent::default(),
            registry: HyperlinkRegistry::new(),
            screen: Screen::new(
                1,
                1,
                0,
                ScreenResetPolicy::default(),
                &mut HyperlinkRegistry::new(),
            )
            .unwrap(),
            px: 70,
            py: 20,
            menu: Menu {
                title: b"t".as_slice().into(),
                items: vec![MenuItem::default(); 3],
                width: 10,
                item_width: 10,
            },
            choice: -1,
            cb: None,
        };
        menu_resize(&mut md, 80, 24);
        assert_eq!((md.px, md.py), (66, 19));
        menu_resize(&mut md, 10, 3);
        assert_eq!((md.px, md.py), (0, 0));
        assert_eq!(menu_get_cursor(&md), (2, 0));
        md.choice = 1;
        assert_eq!(menu_get_cursor(&md), (2, 2));
    }
}
