// Ported from tmux mode-tree.c @ 8f25579c
/*
 * Copyright (c) 2017 Nicholas Marriott <nicholas.marriott@gmail.com>
 * Permission to use, copy, modify, and distribute this software for any
 * purpose with or without fee is hereby granted, provided that the above
 * copyright notice and this permission notice appear in all copies.
 * THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
 * WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
 * MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
 * ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
 * WHATSOEVER RESULTING FROM LOSS OF MIND, USE, DATA OR PROFITS, WHETHER
 * IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING
 * OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
 */
pub use crate::ids::ModeTreeItemId;
use crate::{
    format::sort::SortCriteria,
    ids::{Arena, ClientId, ModeId, PaneId, SessionId, WinlinkId},
    server::Server,
};
use rmux_emu::screen::write::ScreenWriteCtx;
use rmux_util::{bytes::ByteString, key::KeyCode};
use std::collections::HashMap;

pub type ModeAction = Box<dyn FnOnce(&mut Server)>;
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum ModeTreeTag {
    #[default]
    Unset,
    Session(SessionId),
    Winlink(WinlinkId),
    Pane(PaneId),
    Client(ClientId),
    BufferOrder(u32),
    OptionTable(usize),
    Section {
        kind: u8,
        scope: u8,
    },
    Serial {
        kind: u8,
        value: u64,
        subitem: u8,
    },
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ModeTreePreview {
    Off,
    #[default]
    Normal,
    Big,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModeTreeSearchDir {
    Forward,
    Backward,
}
pub struct ModeTreeKeyResult {
    pub finished: bool,
    pub key: KeyCode,
    pub mouse_x: u32,
    pub mouse_y: u32,
}
pub struct ModeTreeItem {
    pub parent: Option<ModeTreeItemId>,
    pub children: Vec<ModeTreeItemId>,
    pub item: Option<u32>,
    pub tag: ModeTreeTag,
    pub name: ByteString,
    pub text: Option<ByteString>,
    pub expanded: bool,
    pub tagged: bool,
    pub draw_as_parent: bool,
    pub no_tag: bool,
    pub align: bool,
    pub line: usize,
    pub key: Option<KeyCode>,
}
#[derive(Clone, Copy, Debug)]
pub struct ModeTreeLine {
    pub item: ModeTreeItemId,
    pub depth: u32,
    pub last: bool,
    pub flat: bool,
}
#[derive(Default)]
pub struct ModeTreeData {
    pub items: Arena<ModeTreeItem, ModeTreeItemId>,
    pub children: Vec<ModeTreeItemId>,
    pub lines: Vec<ModeTreeLine>,
    pub current: usize,
    pub offset: usize,
    pub width: u32,
    pub height: usize,
    pub screen_sy: u32,
    pub maxdepth: u32,
    pub preview: ModeTreePreview,
    pub search: Option<ByteString>,
    pub filter: Option<ByteString>,
    pub prompt: Option<crate::ui::prompt::Prompt>,
    pub owner: Option<PaneId>,
    pub pending_action: Option<ModeAction>,
    prompt_backend: bool,
    prompt_top: bool,
    prompt_filter: bool,
    prompt_action: std::rc::Rc<std::cell::RefCell<Option<ModeAction>>>,
    pub no_matches: bool,
    pub help: bool,
    pub view_name: Option<ByteString>,
    pub sort: SortCriteria,
    pub zoomed: Option<(crate::ids::WindowId, bool)>,
    pub menu_items: &'static [ModeTreeMenuItem],
    saved: HashMap<ModeTreeTag, (bool, bool)>,
    draw_text: Vec<u8>,
    draw_alignment: Vec<usize>,
}
pub struct TreeModeState<B> {
    pub tree: ModeTreeData,
    pub backend: B,
}
pub trait ModeTreeCallbacks {
    type Item;
    fn build(
        &mut self,
        s: &mut Server,
        tree: &mut ModeTreeData,
        sort: &SortCriteria,
        tag: &mut ModeTreeTag,
        filter: Option<&[u8]>,
    );
    fn draw(
        &self,
        s: &Server,
        item: Option<&Self::Item>,
        ctx: &mut ScreenWriteCtx<'_>,
        sx: u32,
        sy: u32,
    );
    fn prepare_draw(&mut self, _s: &mut Server, _item: Option<u32>, _sx: u32, _sy: u32) {}
    fn has_draw(&self) -> bool;
    fn search(
        &self,
        s: &Server,
        item: Option<&Self::Item>,
        needle: &[u8],
        icase: bool,
    ) -> Option<bool>;
    fn menu(&mut self, mode: ModeId, client: Option<ClientId>, key: KeyCode) -> ModeAction;
    fn height(&self, screen_sy: u32) -> Option<u32>;
    fn key(&mut self, s: &mut Server, item: Option<u32>, line: u32) -> Option<KeyCode>;
    fn swap(&self, cur: &Self::Item, other: &Self::Item, sort: &SortCriteria) -> ModeAction;
    fn can_swap(
        &self,
        _s: &Server,
        _cur: &Self::Item,
        _other: &Self::Item,
        _sort: &SortCriteria,
    ) -> bool {
        false
    }
    fn sort(&self, sort: &mut SortCriteria) -> bool;
    fn help(&self) -> Option<(u32, &'static str, &'static [&'static str])>;
    fn items(&self) -> &[Self::Item];
}
impl ModeTreeData {
    pub fn start(sx: u32, sy: u32, preview: ModeTreePreview) -> Self {
        Self {
            width: sx,
            screen_sy: sy,
            height: sy as usize,
            preview,
            ..Self::default()
        }
    }
    pub fn add(
        &mut self,
        parent: Option<ModeTreeItemId>,
        item: Option<u32>,
        tag: ModeTreeTag,
        name: &[u8],
        text: Option<&[u8]>,
        expanded: i32,
    ) -> ModeTreeItemId {
        let (expanded, tagged) =
            self.saved
                .get(&tag)
                .copied()
                .map_or((expanded != 0, false), |(e, t)| {
                    (
                        e,
                        t && parent.is_none_or(|p| self.items.get(p).is_some_and(|p| p.expanded)),
                    )
                });
        let id = self
            .items
            .insert(ModeTreeItem {
                parent,
                children: Vec::new(),
                item,
                tag,
                name: name.into(),
                text: text.map(ByteString::from),
                expanded,
                tagged,
                draw_as_parent: false,
                no_tag: false,
                align: false,
                line: 0,
                key: None,
            })
            .expect("tree item arena exhausted");
        if let Some(parent) = parent {
            self.items
                .get_mut(parent)
                .expect("tree parent")
                .children
                .push(id);
        } else {
            self.children.push(id);
        }
        id
    }
    fn walk(&self, roots: &[ModeTreeItemId], out: &mut Vec<ModeTreeItemId>) {
        for &id in roots {
            out.push(id);
            if let Some(item) = self.items.get(id) {
                self.walk(&item.children, out);
            }
        }
    }
    pub fn save(&mut self) {
        self.saved.clear();
        let mut ids = Vec::new();
        self.walk(&self.children, &mut ids);
        for id in ids {
            let item = self.items.get(id).expect("tree item");
            self.saved
                .entry(item.tag)
                .or_insert((item.expanded, item.tagged));
        }
        self.items = Arena::new();
        self.children.clear();
        self.lines.clear();
    }
    pub fn build_lines(&mut self) {
        self.lines.clear();
        self.maxdepth = 0;
        fn visit(tree: &mut ModeTreeData, ids: &[ModeTreeItemId], depth: u32) {
            let flat = ids
                .iter()
                .all(|id| tree.items.get(*id).is_none_or(|i| i.children.is_empty()));
            tree.maxdepth = tree.maxdepth.max(depth);
            for (n, &id) in ids.iter().enumerate() {
                let item = tree.items.get_mut(id).expect("tree item");
                item.line = tree.lines.len();
                let expanded = item.expanded;
                let children = item.children.clone();
                tree.lines.push(ModeTreeLine {
                    item: id,
                    depth,
                    last: n + 1 == ids.len(),
                    flat,
                });
                if expanded {
                    visit(tree, &children, depth + 1);
                }
            }
        }
        visit(self, &self.children.clone(), 0);
    }
    pub fn get_current(&self) -> Option<&ModeTreeItem> {
        self.lines
            .get(self.current)
            .and_then(|l| self.items.get(l.item))
    }
    pub fn get_current_name(&self) -> Option<&[u8]> {
        self.get_current().map(|i| i.name.as_ref())
    }
    pub fn set_current(&mut self, tag: ModeTreeTag) {
        if let Some(n) = self
            .lines
            .iter()
            .position(|l| self.items.get(l.item).is_some_and(|i| i.tag == tag))
        {
            self.current = n;
        } else {
            self.current = self.current.min(self.lines.len().saturating_sub(1));
        }
        self.check_selected();
    }
    pub fn check_selected(&mut self) {
        if self.height == 0 {
            return;
        }
        self.offset = self
            .offset
            .min(self.lines.len().saturating_sub(self.height));
        if self.current < self.offset {
            self.offset = self.current;
        } else if self.current >= self.offset + self.height {
            self.offset = self.current - self.height + 1;
        }
    }
    pub fn up(&mut self, wrap: bool) {
        if self.lines.is_empty() {
            return;
        }
        if self.current == 0 {
            if wrap {
                self.current = self.lines.len() - 1;
                self.offset = self.lines.len().saturating_sub(self.height);
            }
        } else {
            self.current -= 1;
            if self.current < self.offset {
                self.offset = self.offset.saturating_sub(1);
            }
        }
    }
    pub fn down(&mut self, wrap: bool) -> bool {
        if self.lines.is_empty() {
            return false;
        }
        if self.current + 1 == self.lines.len() {
            if !wrap {
                return false;
            }
            self.current = 0;
            self.offset = 0;
        } else {
            self.current += 1;
            if self.current >= self.offset + self.height {
                self.offset += 1;
            }
        }
        true
    }
    pub fn set_height(&mut self, reserved: Option<u32>) {
        let sy = self.screen_sy as usize;
        if self.preview == ModeTreePreview::Off {
            self.height = sy;
        } else if let Some(reserved) = reserved {
            if (reserved as usize) < sy {
                self.height = sy - reserved as usize;
            }
        } else {
            self.height = match self.preview {
                ModeTreePreview::Normal => {
                    let mut h = (sy / 3) * 2;
                    if h > self.lines.len() {
                        h = sy / 2;
                    }
                    if h < 10 { sy } else { h }
                }
                ModeTreePreview::Big => (sy / 4).min(self.lines.len()).max(2),
                ModeTreePreview::Off => sy,
            };
        }
        if sy.saturating_sub(self.height) < 2 {
            self.height = sy;
        }
    }
    pub fn count_tagged(&self) -> usize {
        self.lines
            .iter()
            .filter(|l| self.items.get(l.item).is_some_and(|i| i.tagged))
            .count()
    }
    pub fn each_tagged(&self, current: bool, mut f: impl FnMut(&ModeTreeItem)) {
        let mut fired = false;
        for line in &self.lines {
            if let Some(item) = self.items.get(line.item).filter(|i| i.tagged) {
                fired = true;
                f(item);
            }
        }
        if current && !fired {
            if let Some(item) = self.get_current() {
                f(item);
            }
        }
    }
    pub fn expand_current(&mut self) {
        let Some(id) = self.lines.get(self.current).map(|l| l.item) else {
            return;
        };
        if self
            .items
            .get(id)
            .is_some_and(|i| i.expanded || i.children.is_empty())
        {
            self.down(false);
            return;
        }
        if let Some(item) = self.items.get_mut(id) {
            item.expanded = true;
        }
        self.build_lines();
        self.check_selected();
    }
    pub fn collapse_current(&mut self) {
        let Some(id) = self.lines.get(self.current).map(|l| l.item) else {
            return;
        };
        let Some(item) = self.items.get(id) else {
            return;
        };
        let id = if self.lines.get(self.current).is_some_and(|l| l.flat) || !item.expanded {
            if let Some(parent) = item.parent {
                parent
            } else {
                self.up(false);
                return;
            }
        } else {
            id
        };
        let tag = self.items.get(id).expect("collapse item").tag;
        self.items.get_mut(id).expect("collapse item").expanded = false;
        self.build_lines();
        self.set_current(tag);
    }
    pub fn swap_with(&self, direction: i32) -> Option<usize> {
        let line = self.lines.get(self.current)?;
        let mut n = self.current;
        loop {
            n = if direction < 0 {
                n.checked_sub(1)?
            } else {
                n.checked_add(1)?
            };
            let other = self.lines.get(n)?;
            if other.depth > line.depth {
                continue;
            }
            return (other.depth == line.depth
                && self.items.get(other.item)?.parent == self.items.get(line.item)?.parent)
                .then_some(n);
        }
    }
}
impl<B: ModeTreeCallbacks> TreeModeState<B> {
    pub fn build(&mut self, s: &mut Server) {
        let mut tag = self
            .tree
            .get_current()
            .map_or(ModeTreeTag::Unset, |i| i.tag);
        self.tree.save();
        self.backend.sort(&mut self.tree.sort);
        let sort = self.tree.sort;
        let filter = self.tree.filter.clone();
        self.backend.build(
            s,
            &mut self.tree,
            &sort,
            &mut tag,
            filter.as_ref().map(|f| f.as_slice()),
        );
        self.tree.no_matches = self.tree.children.is_empty();
        if self.tree.no_matches {
            self.backend.build(s, &mut self.tree, &sort, &mut tag, None);
        }
        self.tree.saved.clear();
        self.tree.build_lines();
        for (n, line) in self.tree.lines.iter().enumerate() {
            let item = self.tree.items.get_mut(line.item).expect("line item");
            item.key = self.backend.key(s, item.item, n as u32);
        }
        self.tree
            .set_height(self.backend.height(self.tree.screen_sy));
        self.tree.set_current(tag);
    }
}
impl ModeTreeData {
    pub fn view_name(&mut self, name: &[u8]) {
        self.view_name = Some(name.into());
    }
    pub fn tagged_items(&self, current: bool) -> Vec<u32> {
        let mut out = Vec::new();
        self.each_tagged(current, |item| {
            if let Some(n) = item.item {
                out.push(n);
            }
        });
        out
    }
}
pub fn run_command(
    s: &mut Server,
    c: Option<ClientId>,
    fs: Option<&crate::cmd::find::CmdFindState>,
    template: &[u8],
    name: &[u8],
) {
    use crate::cmd::{
        parse,
        queue::{QueueStateFlags, QueueStore},
    };
    let command = crate::cmd::template_replace(template, name, 1);
    let mut queue = std::mem::replace(&mut s.queue, QueueStore::new());
    let state = queue
        .new_state(s, fs, None, QueueStateFlags::default())
        .expect("mode command state");
    s.queue = queue;
    let mut input = parse::CmdParseInput {
        client: c,
        target: fs.cloned().unwrap_or_default(),
        ..Default::default()
    };
    if let Err(error) = parse::and_append(s, &command, &mut input, c, state) {
        let mut message = error.message().to_vec();
        if let Some(first) = message.first_mut() {
            first.make_ascii_uppercase();
        }
        crate::ui::status::status_message_set(s, c, -1, true, false, false, &message);
    }
    let _ = s.queue.free_state(state);
}
const PREFIX_FORMAT: &[u8] = concat!(
    "#[fg=themelightgrey]#[bg=default]#[noacs]",
    "#{p/#{mode_tree_key_width}:#{?#{!=:#{mode_tree_key},},(#{mode_tree_key}),}}",
    "#{R:#{?mode_tree_parent_last,    ,#[acs]x#[fg=themelightgrey]#[bg=default]#[noacs]   },#{mode_tree_repeat}}",
    "#{?mode_tree_branch,#[acs]#{?mode_tree_last,mq,tq}+#[fg=themelightgrey]#[bg=default]#[noacs] ,}",
    "#{?mode_tree_has_children,#{?mode_tree_expanded,#[fg=themered]-#[fg=themelightgrey]#[bg=default]#[noacs] ,#[fg=themegreen]+#[fg=themelightgrey]#[bg=default]#[noacs] },#{?mode_tree_flat,,  }}"
).as_bytes();
const HELP_START: &[&str] = &[
    "#[fg=themelightgrey]      Up, k #[#{E:tree-mode-border-style},acs]x#[default] Move cursor up",
    "#[fg=themelightgrey]    Down, j #[#{E:tree-mode-border-style},acs]x#[default] Move cursor down",
    "#[fg=themelightgrey]          g #[#{E:tree-mode-border-style},acs]x#[default] Go to top",
    "#[fg=themelightgrey]          G #[#{E:tree-mode-border-style},acs]x#[default] Go to bottom",
    "#[fg=themelightgrey] PPage, C-b #[#{E:tree-mode-border-style},acs]x#[default] Page up",
    "#[fg=themelightgrey] NPage, C-f #[#{E:tree-mode-border-style},acs]x#[default] Page down",
    "#[fg=themelightgrey]    Left, h #[#{E:tree-mode-border-style},acs]x#[default] Collapse %1",
    "#[fg=themelightgrey]   Right, l #[#{E:tree-mode-border-style},acs]x#[default] Expand %1",
    "#[fg=themelightgrey]        M-- #[#{E:tree-mode-border-style},acs]x#[default] Collapse all %1s",
    "#[fg=themelightgrey]        M-+ #[#{E:tree-mode-border-style},acs]x#[default] Expand all %1s",
    "#[fg=themelightgrey]          t #[#{E:tree-mode-border-style},acs]x#[default] Toggle %1 tag",
    "#[fg=themelightgrey]          T #[#{E:tree-mode-border-style},acs]x#[default] Untag all %1s",
    "#[fg=themelightgrey]        C-t #[#{E:tree-mode-border-style},acs]x#[default] Tag all %1s",
    "#[fg=themelightgrey]        C-s #[#{E:tree-mode-border-style},acs]x#[default] Search forward",
    "#[fg=themelightgrey]          n #[#{E:tree-mode-border-style},acs]x#[default] Repeat search forward",
    "#[fg=themelightgrey]          N #[#{E:tree-mode-border-style},acs]x#[default] Repeat search backward",
    "#[fg=themelightgrey]          f #[#{E:tree-mode-border-style},acs]x#[default] Filter %1s",
    "#[fg=themelightgrey]          O #[#{E:tree-mode-border-style},acs]x#[default] Change sort order",
    "#[fg=themelightgrey]          r #[#{E:tree-mode-border-style},acs]x#[default] Reverse sort order",
    "#[fg=themelightgrey]          v #[#{E:tree-mode-border-style},acs]x#[default] Toggle preview",
];
const HELP_END: &str =
    "#[fg=themelightgrey]  q, Escape #[#{E:tree-mode-border-style},acs]x#[default] Exit mode";
fn line_key(item: &ModeTreeItem) -> KeyCode {
    use rmux_util::key::{KeyModifiers, SpecialKey};
    match item.key {
        Some(key) if key.0 == SpecialKey::UNKNOWN => KeyCode(SpecialKey::NONE),
        Some(key) => key,
        None => KeyCode(if item.line < 10 {
            48 + item.line as u64
        } else if item.line < 36 {
            KeyModifiers::META.bits() | (97 + item.line as u64 - 10)
        } else {
            SpecialKey::NONE
        }),
    }
}
impl<B: ModeTreeCallbacks> TreeModeState<B> {
    pub fn draw(&mut self, s: &mut Server, screen: &mut rmux_emu::screen::Screen) {
        use crate::format::{
            FormatContext,
            draw::{draw, width},
        };
        use rmux_emu::{
            cell::DEFAULT_CELL,
            colour::{Colour, ColourFlags, ColourTheme},
            hyperlinks::HyperlinkRegistry,
            screen::write::{ScreenOnlySink, ScreenWritePolicy},
            screen::{BoxLines, ScreenMode},
        };
        if self.tree.lines.is_empty() || self.tree.width == 0 || self.tree.height == 0 {
            return;
        }
        let wp = self.tree.owner;
        let oo = wp
            .and_then(|p| s.panes.get(p))
            .and_then(|p| s.windows.get(p.window))
            .map_or(s.options.global_w, |w| w.options);
        let mut selected_gc = DEFAULT_CELL;
        let mut border_gc = DEFAULT_CELL;
        crate::ui::styles::style_apply(s, &mut selected_gc, oo, b"tree-mode-selection-style", None);
        crate::ui::styles::style_apply(s, &mut border_gc, oo, b"tree-mode-border-style", None);
        let mut ft = crate::format::create_defaults(
            s,
            None,
            FormatContext {
                pane: wp,
                ..Default::default()
            },
        );
        let mut align = std::mem::take(&mut self.tree.draw_alignment);
        let mut text = std::mem::take(&mut self.tree.draw_text);
        let current = self.tree.get_current().and_then(|i| {
            if i.draw_as_parent {
                i.parent.and_then(|p| self.tree.items.get(p))
            } else {
                Some(i)
            }
        });
        let selected = current.and_then(|i| i.item);
        self.backend.prepare_draw(
            s,
            selected,
            self.tree.width.saturating_sub(4),
            self.tree
                .screen_sy
                .saturating_sub(self.tree.height as u32 + 2),
        );
        let prompt_plan = self
            .tree
            .prompt
            .as_ref()
            .map(|p| crate::ui::prompt::prompt_draw(p, s, 0, self.tree.width));
        let keys: Vec<_> = self
            .tree
            .lines
            .iter()
            .map(|l| line_key(self.tree.items.get(l.item).expect("tree item")))
            .map(|k| {
                if k.0 == rmux_util::key::SpecialKey::NONE {
                    Vec::new()
                } else {
                    rmux_tty::key_string::key_name(k, false)
                }
            })
            .collect();
        let key_width = keys
            .iter()
            .filter(|k| !k.is_empty())
            .map(|k| k.len() + 3)
            .max()
            .unwrap_or(0);
        align.clear();
        align.resize(self.tree.maxdepth as usize + 1, 0);
        for line in &self.tree.lines {
            let item = self.tree.items.get(line.item).expect("tree item");
            if item.align {
                align[line.depth as usize] = align[line.depth as usize].max(item.name.len());
            }
        }
        let mut registry = HyperlinkRegistry::default();
        let mut sink = ScreenOnlySink;
        let mut ctx = ScreenWriteCtx::start(
            screen,
            &mut sink,
            ScreenWritePolicy::default(),
            &mut registry,
            #[cfg(feature = "sixel")]
            None,
        );
        ctx.clearscreen(Colour::DEFAULT);
        let w = self.tree.width;
        for (n, line) in self
            .tree
            .lines
            .iter()
            .enumerate()
            .skip(self.tree.offset)
            .take(self.tree.height)
        {
            let item = self.tree.items.get(line.item).expect("tree item");
            ft.add(b"mode_tree_key", keys[n].as_slice().into());
            for (name, value) in [
                (b"mode_tree_key_width".as_slice(), key_width),
                (b"mode_tree_selected", usize::from(n == self.tree.current)),
                (b"mode_tree_repeat", line.depth.saturating_sub(1) as usize),
                (b"mode_tree_branch", usize::from(line.depth != 0)),
                (
                    b"mode_tree_parent_last",
                    usize::from(
                        item.parent
                            .and_then(|p| self.tree.items.get(p))
                            .and_then(|p| self.tree.lines.get(p.line))
                            .is_some_and(|l| l.last),
                    ),
                ),
                (
                    b"mode_tree_has_children",
                    usize::from(!item.children.is_empty()),
                ),
                (b"mode_tree_last", usize::from(line.last)),
                (b"mode_tree_expanded", usize::from(item.expanded)),
                (b"mode_tree_flat", usize::from(line.flat)),
            ] {
                ft.add(name, value.to_string().into());
            }
            let prefix = ft.expand(s, PREFIX_FORMAT);
            let prefix_width = width(&prefix).min(w);
            let left = w - prefix_width;
            text.clear();
            if item.align {
                text.resize(
                    align[line.depth as usize].saturating_sub(item.name.len()),
                    b' ',
                );
            }
            text.extend_from_slice(&item.name);
            if item.tagged {
                text.push(b'*');
            }
            if item.text.is_some() {
                text.extend_from_slice(b"#[fg=themelightgrey]: #[default]");
            }
            let text_width = width(&text).min(left);
            let mut gc = if n == self.tree.current {
                selected_gc
            } else {
                DEFAULT_CELL
            };
            if item.tagged {
                gc.fg =
                    Colour::from_raw(ColourFlags::THEME.bits() as i32 | ColourTheme::Cyan as i32);
            }
            let y = (n - self.tree.offset) as i32;
            ctx.cursormove(0, y, false);
            ctx.clearendofline(if n == self.tree.current {
                gc.bg
            } else {
                Colour::DEFAULT
            });
            draw(
                &mut ctx,
                &gc,
                prefix_width,
                &prefix,
                None,
                n == self.tree.current,
            );
            if left != 0 {
                ctx.cursormove(prefix_width as i32, y, false);
                draw(&mut ctx, &gc, left, &text, None, n == self.tree.current);
            }
            if let Some(extra) = item.text.as_ref().filter(|_| prefix_width + text_width < w) {
                ctx.cursormove((prefix_width + text_width) as i32, y, false);
                draw(
                    &mut ctx,
                    &gc,
                    w - prefix_width - text_width,
                    extra,
                    None,
                    n == self.tree.current,
                );
            }
        }
        let sy = self.tree.screen_sy;
        if self.tree.preview != ModeTreePreview::Off
            && self.backend.has_draw()
            && sy > 4
            && self.tree.height >= 2
            && sy.saturating_sub(self.tree.height as u32) > 4
            && w > 4
        {
            let h = self.tree.height as u32;
            ctx.cursormove(0, h as i32, false);
            crate::ui::menu::draw_box(
                &mut ctx,
                w,
                sy - h,
                BoxLines::Default,
                Some(&border_gc),
                None,
            );
            text.clear();
            text.push(b' ');
            if let Some(item) = current {
                text.extend_from_slice(&item.name);
            }
            if let Some(order) = self
                .tree
                .sort
                .order_seq
                .and_then(|_| crate::format::sort::order_to_string(self.tree.sort.order))
            {
                text.extend_from_slice(b" (sort: ");
                text.extend_from_slice(order);
                if self.tree.sort.reversed {
                    text.extend_from_slice(b", reversed");
                }
                text.push(b')');
                if let Some(view) = &self.tree.view_name {
                    text.extend_from_slice(b" (view: ");
                    text.extend_from_slice(view);
                    text.push(b')');
                }
            }
            if text.len() <= (w - 2) as usize {
                ctx.cursormove(1, h as i32, false);
                ctx.puts(&border_gc, &text);
                let filter = if self.tree.no_matches {
                    b"no matches".as_slice()
                } else {
                    b"active"
                };
                if self.tree.filter.is_some() && (w - 2) as usize >= text.len() + 12 + filter.len()
                {
                    ctx.puts(&border_gc, b" (filter: ");
                    ctx.puts(&border_gc, filter);
                    ctx.puts(&border_gc, b") ");
                } else {
                    ctx.puts(&border_gc, b" ");
                }
            }
            ctx.cursormove(2, h as i32 + 1, false);
            self.backend.draw(
                s,
                selected.and_then(|i| self.backend.items().get(i as usize)),
                &mut ctx,
                w - 4,
                sy - h - 2,
            );
        }
        if self.tree.help {
            let (hw, word, backend) = self.backend.help().unwrap_or((39, "item", &[]));
            let hw = hw.max(39);
            let hh = HELP_START.len() as u32 + backend.len() as u32 + 1;
            if w >= hw + 2 && sy >= hh + 2 {
                let x = (w - hw - 2) / 2;
                let y = (sy - hh - 2) / 2;
                ctx.cursormove(x as i32, y as i32, false);
                crate::ui::menu::draw_box(
                    &mut ctx,
                    hw + 2,
                    hh + 2,
                    BoxLines::Default,
                    Some(&border_gc),
                    None,
                );
                for (n, line) in HELP_START
                    .iter()
                    .chain(backend)
                    .chain(std::iter::once(&HELP_END))
                    .enumerate()
                {
                    let replaced =
                        crate::cmd::template_replace(line.as_bytes(), word.as_bytes(), 1);
                    let expanded = ft.expand(s, &replaced);
                    ctx.cursormove(x as i32 + 1, y as i32 + n as i32 + 1, false);
                    ctx.clearcharacter(hw, Colour::DEFAULT);
                    ctx.cursormove(x as i32 + 1, y as i32 + n as i32 + 1, false);
                    draw(&mut ctx, &DEFAULT_CELL, hw, &expanded, None, false);
                }
            }
        }
        ft.release(s);
        if let (Some(prompt), Some(plan)) = (&self.tree.prompt, prompt_plan) {
            let mut cursor = 0;
            let line = if self.tree.prompt_top {
                0
            } else {
                sy.saturating_sub(1)
            };
            plan.render(
                prompt,
                &mut ctx,
                &mut crate::ui::prompt::PromptDrawData {
                    cursor_x: &mut cursor,
                    area_x: 0,
                    area_width: w,
                    prompt_line: line,
                },
            );
            ctx.screen.mode.insert(ScreenMode::CURSOR);
            ctx.cursormove(cursor as i32, line as i32, false);
        } else {
            ctx.screen.mode.remove(ScreenMode::CURSOR);
            ctx.cursormove(
                0,
                self.tree.current.saturating_sub(self.tree.offset) as i32,
                false,
            );
        }
        self.tree.draw_text = text;
        self.tree.draw_alignment = align;
    }
}
impl<B: ModeTreeCallbacks + 'static> TreeModeState<B> {
    pub fn key(
        &mut self,
        s: &mut Server,
        mode: ModeId,
        c: Option<ClientId>,
        key: KeyCode,
        m: Option<&crate::client::ResolvedMouseEvent>,
        _screen: &mut rmux_emu::screen::Screen,
    ) -> ModeTreeKeyResult {
        use rmux_util::key::SpecialKey as Key;
        let engine_key = if key.0 & rmux_util::key::KeyModifiers::CTRL.bits() != 0
            && key.0 & !rmux_util::key::KeyModifiers::CTRL.bits() < 128
        {
            key.0 & 31
        } else {
            key.0
        };
        if self.tree.lines.is_empty() {
            return ModeTreeKeyResult {
                finished: true,
                key: KeyCode(Key::NONE),
                mouse_x: 0,
                mouse_y: 0,
            };
        }
        let mut result = ModeTreeKeyResult {
            finished: false,
            key,
            mouse_x: 0,
            mouse_y: 0,
        };
        if let Some(mut prompt) = self.tree.prompt.take() {
            let mut redraw = false;
            let handled = if let Some(mouse) = m.filter(|_| key.is_mouse()) {
                if let Some((x, y)) = mouse_at(s, mode.owner, mouse) {
                    let py = if self.tree.prompt_top {
                        0
                    } else {
                        self.tree.screen_sy.saturating_sub(1)
                    };
                    if key.0 == Key::MOUSEDOWN1_PANE && y == py {
                        crate::ui::prompt::prompt_mouse(
                            &mut prompt,
                            s,
                            x,
                            0,
                            self.tree.width,
                            &mut redraw,
                        )
                    } else {
                        crate::ui::prompt::PromptKeyResult::NotHandled
                    }
                } else {
                    crate::ui::prompt::PromptKeyResult::NotHandled
                }
            } else {
                crate::ui::prompt::prompt_key(s, &mut prompt, key, &mut redraw)
            };
            self.tree.pending_action = self.tree.prompt_action.borrow_mut().take();
            if crate::ui::prompt::prompt_closed(&prompt) {
                if !self.tree.prompt_backend && engine_key == 13 {
                    let text = prompt.input();
                    if self.tree.prompt_filter {
                        self.tree.filter = (!text.is_empty()).then_some(text);
                        self.build(s);
                    } else {
                        self.search(s, &text, ModeTreeSearchDir::Forward);
                    }
                }
                crate::ui::prompt::prompt_free(s, prompt);
            } else {
                self.tree.prompt = Some(prompt);
            }
            if handled != crate::ui::prompt::PromptKeyResult::NotHandled {
                result.key = KeyCode(Key::NONE);
                return result;
            }
        }
        if self.tree.help {
            if !key.is_mouse() && !matches!(key.0, Key::FOCUS_IN | Key::FOCUS_OUT) {
                self.tree.help = false;
            }
            result.key = KeyCode(Key::NONE);
            return result;
        }
        if matches!(engine_key, 47 | 63 | 19 | 102) {
            self.tree.prompt_filter = engine_key == 102;
            let mut data = crate::ui::prompt::PromptCreateData {
                prompt: if self.tree.prompt_filter {
                    b"(filter) ".as_slice().into()
                } else {
                    b"(search) ".as_slice().into()
                },
                input: if self.tree.prompt_filter {
                    self.tree.filter.clone()
                } else {
                    None
                },
                flags: crate::ui::prompt::PromptFlags::ISMODE
                    | crate::ui::prompt::PromptFlags::NOFORMAT,
                ty: crate::ui::prompt::PromptType::Search,
                ..Default::default()
            };
            let oo = c.map_or(s.options.global_s, |c| {
                crate::ui::styles::session_options(s, c)
            });
            self.tree.prompt_backend = false;
            self.tree.prompt_top = s.options.get_number(oo, b"status-position") == 0;
            crate::ui::prompt::prompt_set_options(s, &mut data, oo);
            self.tree.prompt = Some(crate::ui::prompt::prompt_create(
                s,
                data,
                Box::new(TreePromptHost),
            ));
            result.key = KeyCode(Key::NONE);
            return result;
        }
        if let Some(mouse) = m.filter(|_| key.is_mouse()) {
            if !matches!(key.0, Key::WHEELUP_PANE | Key::WHEELDOWN_PANE) {
                let Some((x, y)) = mouse_at(s, mode.owner, mouse) else {
                    result.key = KeyCode(Key::NONE);
                    return result;
                };
                result.mouse_x = x;
                result.mouse_y = y;
                let outside = x > self.tree.width || y > self.tree.height as u32;
                if outside {
                    if key.0 == Key::MOUSEDOWN3_PANE {
                        self.display_menu(s, mode, c, x, y, true);
                    }
                    if self.tree.preview == ModeTreePreview::Off {
                        result.key = KeyCode(Key::NONE);
                    }
                    return result;
                }
                let line = self.tree.offset + y as usize;
                if line < self.tree.lines.len()
                    && matches!(
                        key.0,
                        Key::MOUSEDOWN1_PANE | Key::MOUSEDOWN3_PANE | Key::DOUBLECLICK1_PANE
                    )
                {
                    self.tree.current = line;
                }
                if key.0 == Key::DOUBLECLICK1_PANE && line < self.tree.lines.len() {
                    result.key = KeyCode(13);
                } else {
                    if key.0 == Key::MOUSEDOWN3_PANE {
                        self.display_menu(s, mode, c, x, y, false);
                    }
                    result.key = KeyCode(Key::NONE);
                }
                return result;
            }
        }
        if let Some(line) = self.tree.lines.iter().position(|line| {
            let item = self.tree.items.get(line.item).expect("line item");
            let expected = line_key(item).0;
            expected == key.0 && expected != Key::NONE
        }) {
            self.tree.current = line;
            self.tree.check_selected();
            result.key = KeyCode(13);
            return result;
        }
        match engine_key {
            113 | 27 | 7 => result.finished = true,
            Key::F1 | 8 => {
                self.tree.help = true;
            }
            Key::UP | Key::WHEELUP_PANE | 107 | 16 => self.tree.up(true),
            Key::DOWN | Key::WHEELDOWN_PANE | 106 | 14 => {
                self.tree.down(true);
            }
            Key::PPAGE | 2 => {
                for _ in 0..self.tree.height {
                    self.tree.up(false);
                }
            }
            Key::NPAGE | 6 => {
                for _ in 0..self.tree.height {
                    self.tree.down(false);
                }
            }
            Key::HOME | 103 => {
                self.tree.current = 0;
                self.tree.offset = 0;
            }
            Key::END | 71 => {
                self.tree.current = self.tree.lines.len().saturating_sub(1);
                self.tree.check_selected();
            }
            k if k == 75
                || k == (Key::UP | rmux_util::key::KeyModifiers::SHIFT.bits())
                || k == 74
                || k == (Key::DOWN | rmux_util::key::KeyModifiers::SHIFT.bits()) =>
            {
                let direction =
                    if k == 75 || k == (Key::UP | rmux_util::key::KeyModifiers::SHIFT.bits()) {
                        -1
                    } else {
                        1
                    };
                if let Some(other) = self.tree.swap_with(direction) {
                    let cur = self
                        .tree
                        .get_current()
                        .and_then(|i| i.item)
                        .and_then(|i| self.backend.items().get(i as usize));
                    let other = self.tree.items.get(self.tree.lines[other].item);
                    if let (Some(cur), Some(other)) = (cur, other) {
                        if let Some(item) = other
                            .item
                            .and_then(|i| self.backend.items().get(i as usize))
                        {
                            if self.backend.can_swap(s, cur, item, &self.tree.sort) {
                                let tag = other.tag;
                                let action = self.backend.swap(cur, item, &self.tree.sort);
                                self.tree.pending_action = Some(Box::new(move |s| {
                                    action(s);
                                    with_live::<B>(s, mode, |s, state, screen| {
                                        state.build(s);
                                        state.tree.set_current(tag);
                                        state.draw(s, screen);
                                    });
                                }));
                            }
                        }
                    }
                }
            }
            k if k == (45 | rmux_util::key::KeyModifiers::META.bits())
                || k == (43 | rmux_util::key::KeyModifiers::META.bits()) =>
            {
                for id in &self.tree.children {
                    if let Some(item) = self.tree.items.get_mut(*id) {
                        item.expanded = k == (43 | rmux_util::key::KeyModifiers::META.bits());
                    }
                }
                self.build(s);
            }
            Key::LEFT | 104 | 45 => {
                self.tree.collapse_current();
                self.build(s);
            }
            Key::RIGHT | 108 | 43 => {
                self.tree.expand_current();
                self.build(s);
            }
            84 => {
                for line in &self.tree.lines {
                    if let Some(item) = self.tree.items.get_mut(line.item) {
                        item.tagged = false;
                    }
                }
            }
            116 => {
                if let Some(id) = self.tree.lines.get(self.tree.current).map(|l| l.item) {
                    if let Some(item) = self.tree.items.get(id).filter(|i| !i.no_tag) {
                        let setting = !item.tagged;
                        let mut parent = item.parent;
                        let children = item.children.clone();
                        if setting {
                            while let Some(id) = parent {
                                let item = self.tree.items.get_mut(id).expect("tag parent");
                                item.tagged = false;
                                parent = item.parent;
                            }
                            let mut ids = Vec::new();
                            self.tree.walk(&children, &mut ids);
                            for id in ids {
                                if let Some(item) = self.tree.items.get_mut(id) {
                                    item.tagged = false;
                                }
                            }
                        }
                        self.tree.items.get_mut(id).expect("tag item").tagged = setting;
                    }
                }
            }
            118 => {
                self.tree.preview = match self.tree.preview {
                    ModeTreePreview::Off => ModeTreePreview::Big,
                    ModeTreePreview::Normal => ModeTreePreview::Off,
                    ModeTreePreview::Big => ModeTreePreview::Normal,
                };
                self.tree
                    .set_height(self.backend.height(self.tree.screen_sy));
            }
            99 => {
                self.tree.filter = None;
                self.build(s);
            }
            79 => {
                crate::format::sort::next_order(&mut self.tree.sort);
                self.build(s);
            }
            114 => {
                self.tree.sort.reversed = !self.tree.sort.reversed;
                self.build(s);
            }
            110 | 78 => {
                if let Some(needle) = self.tree.search.clone() {
                    self.search(
                        s,
                        &needle,
                        if key.0 == 110 {
                            ModeTreeSearchDir::Forward
                        } else {
                            ModeTreeSearchDir::Backward
                        },
                    );
                }
            }
            20 => {
                let ids: Vec<_> = self.tree.lines.iter().map(|l| l.item).collect();
                for id in ids {
                    let parent = self.tree.items.get(id).and_then(|i| i.parent);
                    let allow =
                        parent.is_none_or(|p| self.tree.items.get(p).is_some_and(|i| i.no_tag));
                    if let Some(item) = self.tree.items.get_mut(id) {
                        item.tagged = allow && !item.no_tag;
                    }
                }
            }
            _ => {}
        }
        result
    }
}
fn enqueue_action(s: &mut Server, c: Option<ClientId>, action: ModeAction) {
    let batch = s
        .queue
        .get_callback(
            "mode_tree_callback",
            crate::cmd::queue::callback_for::<Server>(move |s, _| {
                action(s);
                crate::cmd::queue::CmdReturn::Normal
            }),
        )
        .expect("mode callback");
    crate::cmd::queue::append(s, c, batch).expect("mode queue");
}
fn with_live<B: ModeTreeCallbacks + 'static>(
    s: &mut Server,
    mode: ModeId,
    f: impl FnOnce(&mut Server, &mut TreeModeState<B>, &mut rmux_emu::screen::Screen),
) {
    let Some(entry) = s
        .panes
        .get_mut(mode.owner)
        .and_then(|p| p.modes.first_mut())
        .filter(|e| e.id == mode)
    else {
        return;
    };
    let Some(data) = entry.data.take() else {
        return;
    };
    let Ok(mut state) = data.downcast::<TreeModeState<B>>() else {
        return;
    };
    let Some(mut screen) = entry.screen.take() else {
        entry.data = Some(state);
        return;
    };
    f(s, &mut state, &mut screen);
    if let Some(pane) = s.panes.get_mut(mode.owner) {
        if let Some(entry) = pane.modes.first_mut().filter(|e| e.id == mode) {
            entry.data = Some(state);
            entry.screen = Some(screen);
            pane.flags.insert(crate::model::PaneFlags::REDRAW);
        }
    }
}
type TreePromptCallback =
    Box<dyn FnMut(Option<&[u8]>, crate::ui::prompt::PromptKeyResult) -> Option<ModeAction>>;
struct BackendPromptHost {
    action: std::rc::Rc<std::cell::RefCell<Option<ModeAction>>>,
    callback: TreePromptCallback,
}
impl crate::ui::prompt::PromptHost for BackendPromptHost {
    fn fire(
        &mut self,
        _s: &mut Server,
        text: Option<&[u8]>,
        key: crate::ui::prompt::PromptKeyResult,
    ) -> crate::ui::prompt::PromptResult {
        if let Some(action) = (self.callback)(text, key) {
            *self.action.borrow_mut() = Some(action);
        }
        crate::ui::prompt::PromptResult::Close
    }
}
impl<B: ModeTreeCallbacks + 'static> TreeModeState<B> {
    pub fn set_prompt(
        &mut self,
        s: &mut Server,
        mode: ModeId,
        c: Option<ClientId>,
        mut data: crate::ui::prompt::PromptCreateData,
        callback: impl FnMut(Option<&[u8]>, crate::ui::prompt::PromptKeyResult) -> Option<ModeAction>
        + 'static,
    ) {
        use crate::ui::prompt::{self, PromptFlags};
        if let Some(p) = self.tree.prompt.take() {
            prompt::prompt_free(s, p);
        }
        let oo = c.map_or(s.options.global_s, |c| {
            crate::ui::styles::session_options(s, c)
        });
        self.tree.prompt_top = s.options.get_number(oo, b"status-position") == 0;
        self.tree.prompt_backend = true;
        let accept =
            data.flags.contains(PromptFlags::SINGLE) && data.flags.contains(PromptFlags::ACCEPT);
        data.flags.insert(PromptFlags::ISMODE);
        prompt::prompt_set_options(s, &mut data, oo);
        self.tree.prompt = Some(prompt::prompt_create(
            s,
            data,
            Box::new(BackendPromptHost {
                action: self.tree.prompt_action.clone(),
                callback: Box::new(callback),
            }),
        ));
        if let Some(c) = c.filter(|_| accept) {
            enqueue_action(
                s,
                Some(c),
                Box::new(move |s| {
                    let driver = s
                        .panes
                        .get(mode.owner)
                        .and_then(|p| p.modes.first())
                        .filter(|e| e.id == mode)
                        .map(|e| e.driver.clone());
                    if let Some(driver) = driver {
                        driver.key(s, mode, c, KeyCode(121), None);
                    }
                }),
            );
        }
    }
    fn search(&mut self, s: &mut Server, needle: &[u8], direction: ModeTreeSearchDir) -> bool {
        if needle.is_empty() {
            self.tree.search = None;
            return false;
        }
        self.tree.search = Some(needle.into());
        let mut all = Vec::new();
        self.tree.walk(&self.tree.children, &mut all);
        let Some(start) = self
            .tree
            .lines
            .get(self.tree.current)
            .and_then(|l| all.iter().position(|id| *id == l.item))
        else {
            return false;
        };
        let icase = is_lowercase(needle);
        for step in 1..all.len() {
            let n = match direction {
                ModeTreeSearchDir::Forward => (start + step) % all.len(),
                ModeTreeSearchDir::Backward => (start + all.len() - step) % all.len(),
            };
            let item = self.tree.items.get(all[n]).expect("search item");
            let matched = self
                .backend
                .search(
                    s,
                    item.item.and_then(|i| self.backend.items().get(i as usize)),
                    needle,
                    icase,
                )
                .unwrap_or_else(|| {
                    item.name.windows(needle.len()).any(|w| {
                        w.iter().zip(needle).all(|(&a, &b)| {
                            if icase {
                                tolower(a) == tolower(b)
                            } else {
                                a == b
                            }
                        })
                    })
                });
            if matched {
                let tag = item.tag;
                let mut parent = item.parent;
                while let Some(id) = parent {
                    let item = self.tree.items.get_mut(id).expect("search parent");
                    item.expanded = true;
                    parent = item.parent;
                }
                self.tree.build_lines();
                for (n, line) in self.tree.lines.iter().enumerate() {
                    let item = self.tree.items.get_mut(line.item).expect("search line");
                    item.key = self.backend.key(s, item.item, n as u32);
                }
                self.tree.set_current(tag);
                return true;
            }
        }
        false
    }
    fn display_menu(
        &mut self,
        s: &mut Server,
        mode: ModeId,
        c: Option<ClientId>,
        x: u32,
        y: u32,
        outside: bool,
    ) {
        use crate::ui::menu::{self, MenuItem};
        let Some(c) = c else {
            return;
        };
        let line = if self.tree.offset + y as usize >= self.tree.lines.len() {
            self.tree.current
        } else {
            self.tree.offset + y as usize
        };
        let Some(item) = self
            .tree
            .lines
            .get(line)
            .and_then(|l| self.tree.items.get(l.item))
        else {
            return;
        };
        let title = if outside {
            Vec::new()
        } else {
            let mut title = b"#[align=centre]".to_vec();
            title.extend_from_slice(&item.name);
            title
        };
        let mut menu = menu::menu_create(&title);
        let outside_items = [
            ModeTreeMenuItem {
                name: "Scroll Left",
                key: 60,
            },
            ModeTreeMenuItem {
                name: "Scroll Right",
                key: 62,
            },
            ModeTreeMenuItem {
                name: "",
                key: rmux_util::key::SpecialKey::NONE,
            },
            ModeTreeMenuItem {
                name: "Cancel",
                key: 113,
            },
        ];
        for entry in if outside {
            outside_items.as_slice()
        } else {
            self.tree.menu_items
        } {
            menu::menu_add_item(
                s,
                &mut menu,
                Some(&MenuItem {
                    name: Some(entry.name.into()),
                    key: KeyCode(entry.key),
                    command: None,
                }),
                None,
                c,
                None,
            );
        }
        let Some(pane) = s.panes.get(mode.owner) else {
            return;
        };
        let oo = s
            .windows
            .get(pane.window)
            .map_or(s.options.global_w, |w| w.options);
        let lines = rmux_emu::screen::BoxLines::try_from(
            s.options.get_number(oo, b"menu-border-lines") as i32,
        )
        .unwrap_or(rmux_emu::screen::BoxLines::Default);
        let (width, _) = menu::menu_get_size(&menu, lines);
        let px = x.saturating_sub(width / 2).saturating_add_signed(pane.xoff);
        let py = y.saturating_add_signed(pane.yoff);
        menu::menu_display(
            s,
            menu,
            menu::MenuFlags::default(),
            0,
            None,
            px,
            py,
            c,
            lines,
            None,
            None,
            None,
            None,
            Some(Box::new(move |s, _, _, key| {
                if key.0 == rmux_util::key::SpecialKey::NONE {
                    return;
                }
                let mut action = None;
                with_live::<B>(s, mode, |_, state, _| {
                    if line < state.tree.lines.len() {
                        state.tree.current = line;
                        action = Some(state.backend.menu(mode, Some(c), key));
                    }
                });
                if let Some(action) = action {
                    action(s);
                }
            })),
        );
    }
}
impl ModeTreeData {
    pub fn preview_from_args(
        args: Option<&crate::cmd::arguments::Args>,
        has_draw: bool,
    ) -> ModeTreePreview {
        if !has_draw {
            return ModeTreePreview::Off;
        }
        match args.map_or(0, |a| a.has(b'N')) {
            0 => ModeTreePreview::Normal,
            1 => ModeTreePreview::Off,
            _ => ModeTreePreview::Big,
        }
    }
}
impl<B: ModeTreeCallbacks> TreeModeState<B> {
    pub fn zoom(&mut self, s: &mut Server, wp: PaneId, args: Option<&crate::cmd::arguments::Args>) {
        if !args.is_some_and(|a| a.has(b'Z') > 0) {
            return;
        }
        let Some(window) = s.panes.get(wp).map(|p| p.window) else {
            return;
        };
        let already = crate::model::window::window_zoomed_pane(s, window).is_some();
        self.tree.zoomed = Some((window, already));
        if !already {
            let _ = crate::model::window::window_zoom(s, window, wp);
        }
    }
    pub fn free(mut self, s: &mut Server) {
        if let Some((window, false)) = self.tree.zoomed {
            let _ = crate::model::window::window_unzoom(s, window, true);
        }
        if let Some(prompt) = self.tree.prompt.take() {
            crate::ui::prompt::prompt_free(s, prompt);
        }
    }
    pub fn resize(
        &mut self,
        s: &mut Server,
        screen: &mut rmux_emu::screen::Screen,
        sx: u32,
        sy: u32,
    ) {
        screen.resize(
            sx,
            sy,
            false,
            #[cfg(feature = "sixel")]
            None,
        );
        self.tree.width = sx;
        self.tree.screen_sy = sy;
        self.build(s);
        self.draw(s, screen);
    }
}
pub fn tolower(c: u8) -> u8 {
    rmux_sys::locale::to_lower(c)
}
pub fn is_lowercase(bytes: &[u8]) -> bool {
    bytes.iter().all(|&c| tolower(c) == c)
}
impl ModeTreeData {
    pub fn search_name(&mut self, needle: &[u8], direction: ModeTreeSearchDir) -> bool {
        if needle.is_empty() {
            self.search = None;
            return false;
        }
        self.search = Some(needle.into());
        let mut all = Vec::new();
        self.walk(&self.children, &mut all);
        let Some(current) = self.lines.get(self.current).map(|l| l.item) else {
            return false;
        };
        let Some(start) = all.iter().position(|&id| id == current) else {
            return false;
        };
        let icase = is_lowercase(needle);
        for step in 1..all.len() {
            let index = match direction {
                ModeTreeSearchDir::Forward => (start + step) % all.len(),
                ModeTreeSearchDir::Backward => (start + all.len() - step) % all.len(),
            };
            let id = all[index];
            let item = self.items.get(id).expect("search tree item");
            let matches = item.name.windows(needle.len()).any(|window| {
                window.iter().zip(needle).all(|(&a, &b)| {
                    if icase {
                        tolower(a) == tolower(b)
                    } else {
                        a == b
                    }
                })
            });
            if !matches {
                continue;
            }
            let tag = item.tag;
            let mut parent = item.parent;
            while let Some(id) = parent {
                let item = self.items.get_mut(id).expect("search parent");
                item.expanded = true;
                parent = item.parent;
            }
            self.build_lines();
            self.set_current(tag);
            return true;
        }
        false
    }
}
pub fn mouse_at(
    s: &Server,
    wp: PaneId,
    m: &crate::client::ResolvedMouseEvent,
) -> Option<(u32, u32)> {
    let pane = s.panes.get(wp)?;
    crate::cmd::find::mouse_at(
        crate::cmd::find::PaneGeometry {
            x: pane.xoff,
            y: pane.yoff,
            width: pane.sx,
            height: pane.sy,
        },
        &crate::cmd::find::MouseInput {
            valid: m.target.valid,
            session: m.target.session,
            window: m.target.window,
            pane: m.target.pane,
            x: m.event.x,
            y: m.event.y,
            offset_x: m.target.ox,
            offset_y: m.target.oy,
            status_at: m.target.status_at,
            status_lines: m.target.status_lines,
            b: m.event.b,
            lb: m.event.lb,
            sgr_type: m.event.sgr_type,
            sgr_b: m.event.sgr_b,
            ..Default::default()
        },
        false,
    )
}
struct TreePromptHost;
impl crate::ui::prompt::PromptHost for TreePromptHost {
    fn fire(
        &mut self,
        _s: &mut Server,
        _text: Option<&[u8]>,
        _key: crate::ui::prompt::PromptKeyResult,
    ) -> crate::ui::prompt::PromptResult {
        crate::ui::prompt::PromptResult::Close
    }
}
pub struct ModeTreeMenuItem {
    pub name: &'static str,
    pub key: u64,
}
pub fn init_zoom<B: ModeTreeCallbacks + 'static>(
    s: &mut Server,
    id: ModeId,
    state: TreeModeState<B>,
    screen: rmux_emu::screen::Screen,
    args: Option<&crate::cmd::arguments::Args>,
) -> Option<(TreeModeState<B>, rmux_emu::screen::Screen)> {
    let zoom = args.is_some_and(|a| a.has(b'Z') > 0);
    let window = s.panes.get(id.owner)?.window;
    let already = crate::model::window::window_zoomed_pane(s, window).is_some();
    let mut state = state;
    state.tree.owner = Some(id.owner);
    if zoom {
        state.tree.zoomed = Some((window, already));
    }
    let entry = s
        .panes
        .get_mut(id.owner)?
        .modes
        .iter_mut()
        .find(|m| m.id == id)?;
    entry.data = Some(Box::new(state));
    entry.screen = Some(screen);
    if zoom && !already {
        let _ = crate::model::window::window_zoom(s, window, id.owner);
    }
    let entry = s
        .panes
        .get_mut(id.owner)?
        .modes
        .iter_mut()
        .find(|m| m.id == id)?;
    Some((
        *entry.data.take()?.downcast::<TreeModeState<B>>().ok()?,
        entry.screen.take()?,
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_clamps_after_shrink_and_both_edges() {
        let mut tree = ModeTreeData::start(80, 10, ModeTreePreview::Off);
        for n in 0..8 {
            tree.add(None, None, ModeTreeTag::BufferOrder(n), b"item", None, 0);
        }
        tree.build_lines();
        tree.height = 3;
        tree.current = 6;
        tree.offset = 99;
        tree.check_selected();
        assert_eq!(tree.offset, 5);
        tree.current = 1;
        tree.check_selected();
        assert_eq!(tree.offset, 1);
        tree.current = 7;
        tree.check_selected();
        assert_eq!(tree.offset, 5);
        tree.height = 10;
        tree.check_selected();
        assert_eq!(tree.offset, 0);
    }
    #[test]
    fn nested_search_wraps_excludes_current_and_expands_parents() {
        let mut tree = ModeTreeData::start(80, 24, ModeTreePreview::Off);
        let root = tree.add(None, None, ModeTreeTag::BufferOrder(0), b"root", None, 0);
        let child = tree.add(
            Some(root),
            None,
            ModeTreeTag::BufferOrder(1),
            b"Needle child",
            None,
            0,
        );
        let other = tree.add(
            None,
            None,
            ModeTreeTag::BufferOrder(2),
            b"needle other",
            None,
            0,
        );
        tree.build_lines();
        assert!(tree.search_name(b"needle", ModeTreeSearchDir::Forward));
        assert_eq!(tree.lines[tree.current].item, child);
        assert!(tree.items.get(root).unwrap().expanded);
        assert!(tree.search_name(b"needle", ModeTreeSearchDir::Backward));
        assert_eq!(tree.lines[tree.current].item, other);
        assert!(!tree.search_name(b"other", ModeTreeSearchDir::Forward));
        assert!(!tree.search_name(b"", ModeTreeSearchDir::Forward));
        assert!(tree.search.is_none());
        assert!(is_lowercase(b"needle"));
        assert!(!is_lowercase(b"Needle"));
    }
    #[test]
    fn swap_walk_skips_descendants_and_rejects_other_parent() {
        let mut tree = ModeTreeData::default();
        let a = tree.add(None, None, ModeTreeTag::BufferOrder(0), b"a", None, -1);
        tree.add(Some(a), None, ModeTreeTag::BufferOrder(1), b"a0", None, -1);
        tree.add(Some(a), None, ModeTreeTag::BufferOrder(2), b"a1", None, 0);
        let b = tree.add(None, None, ModeTreeTag::BufferOrder(3), b"b", None, -1);
        tree.add(Some(b), None, ModeTreeTag::BufferOrder(4), b"b0", None, 0);
        tree.build_lines();
        assert_eq!(tree.swap_with(1), Some(3));
        tree.current = 1;
        assert_eq!(tree.swap_with(1), Some(2));
        tree.current = 2;
        assert_eq!(tree.swap_with(1), None);
        tree.current = 4;
        assert_eq!(tree.swap_with(-1), None);
    }
    #[test]
    fn key_columns_and_preview_threshold_matrix() {
        let mut tree = ModeTreeData::start(80, 50, ModeTreePreview::Normal);
        for n in 0..40 {
            tree.add(None, None, ModeTreeTag::BufferOrder(n), b"item", None, 0);
        }
        tree.build_lines();
        for (n, expected) in [
            (0, 48),
            (9, 57),
            (10, 97 | rmux_util::key::KeyModifiers::META.bits()),
            (35, 122 | rmux_util::key::KeyModifiers::META.bits()),
            (36, rmux_util::key::SpecialKey::NONE),
        ] {
            assert_eq!(
                line_key(tree.items.get(tree.lines[n].item).unwrap()).0,
                expected
            );
        }
        for (sy, normal, big) in [(10, 10, 2), (24, 16, 6), (50, 32, 12)] {
            tree.screen_sy = sy;
            tree.preview = ModeTreePreview::Normal;
            tree.set_height(None);
            assert_eq!(tree.height, normal);
            tree.preview = ModeTreePreview::Big;
            tree.set_height(None);
            assert_eq!(tree.height, big);
            tree.preview = ModeTreePreview::Off;
            tree.set_height(Some(12));
            assert_eq!(tree.height, sy as usize);
        }
    }
    #[test]
    fn depth_flat_last_and_selection() {
        let mut tree = ModeTreeData::start(80, 24, ModeTreePreview::Normal);
        let root = tree.add(None, None, ModeTreeTag::BufferOrder(1), b"root", None, -1);
        tree.add(
            Some(root),
            Some(0),
            ModeTreeTag::BufferOrder(2),
            b"child",
            None,
            0,
        );
        tree.add(None, Some(1), ModeTreeTag::BufferOrder(3), b"last", None, 0);
        tree.build_lines();
        assert_eq!(tree.lines.len(), 3);
        assert_eq!(tree.lines[1].depth, 1);
        assert!(tree.lines[1].flat);
        assert!(!tree.lines[0].last);
        assert!(tree.lines[2].last);
        tree.height = 2;
        tree.current = 2;
        tree.check_selected();
        assert_eq!(tree.offset, 1);
        tree.up(true);
        assert_eq!(tree.current, 1);
        tree.set_current(ModeTreeTag::BufferOrder(1));
        assert_eq!(tree.current, 0);
    }
    #[test]
    fn preview_height_thresholds() {
        let mut tree = ModeTreeData::start(80, 24, ModeTreePreview::Normal);
        tree.set_height(None);
        assert_eq!(tree.height, 12);
        tree.preview = ModeTreePreview::Big;
        tree.set_height(None);
        assert_eq!(tree.height, 2);
        tree.preview = ModeTreePreview::Off;
        tree.set_height(Some(12));
        assert_eq!(tree.height, 24);
    }
    #[test]
    fn saved_first_occurrence_and_collapsed_parent() {
        let mut tree = ModeTreeData::default();
        let tag = ModeTreeTag::BufferOrder(1);
        let first = tree.add(None, None, tag, b"first", None, -1);
        tree.items.get_mut(first).unwrap().tagged = true;
        tree.add(None, None, tag, b"second", None, 0);
        tree.save();
        let parent = tree.add(None, None, ModeTreeTag::BufferOrder(2), b"parent", None, 0);
        let child = tree.add(Some(parent), None, tag, b"child", None, 0);
        assert!(tree.items.get(child).unwrap().expanded);
        assert!(!tree.items.get(child).unwrap().tagged);
    }
}
