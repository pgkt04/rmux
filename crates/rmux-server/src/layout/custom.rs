// Ported from tmux layout-custom.c @ 8f25579c
/*
 * Copyright (c) 2010 Nicholas Marriott <nicholas.marriott@gmail.com>
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

//! Layouts as strings. The current (v2) format is JSON: `{"V":2,"L":<cell>}`
//! where a cell is `{"t":"h"|"v"|"p","w","h","x","y"}` plus `"c":[...]` for a
//! node, or `"a"`/`"l"`, `"i"`, `"z"` (floating) and `"I"` for a leaf. The
//! legacy v1 format is `<checksum>,<sx>x<sy>,<xoff>,<yoff>[,<pane id>]` with
//! `{}` around left-right children and `[]` around top-bottom children.

use std::io::Write as _;

use rmux_util::bytes::{ByteString, cstr};

use super::tree::{
    cell, cell_has_tiled_child, cell_is_tiled, count_cells, create_cell, debug_print, destroy_cell,
    fix_offsets, fix_panes, free_cell, link_child_tail, make_leaf, replace_with_node, unlink_child,
};
use super::{
    Cells, LayoutCellFlags, LayoutDumpFlags, LayoutError, LayoutGeometry, LayoutHost, LayoutType,
    PANE_MAXIMUM, PANE_MINIMUM, WINDOW_MAXIMUM,
};
use crate::format::json::{self, JsonNode};
use crate::ids::{LayoutCellId, PaneId, WindowId};
use crate::model::PaneFlags;

/// Maximum nesting depth for version 1 layouts (`layout-custom.c:61`).
pub const LAYOUT_V1_MAX_DEPTH: u32 = 1000;

/// `struct layout_parse_cell_ctx` (`layout-custom.c:71-77`).
#[derive(Clone, Copy, Debug)]
struct CellCtx {
    cell: LayoutCellId,
    active: i32,
    last: i32,
    index: i32,
    zindex: i32,
}

/// `struct layout_parse_ctx` (`layout-custom.c:80-89`).
struct ParseCtx {
    version: i64,
    num_active: i32,
    root: Option<LayoutCellId>,
    cctxs: Vec<CellCtx>,
}

impl ParseCtx {
    fn new() -> Self {
        Self {
            version: -1,
            num_active: 0,
            root: None,
            cctxs: Vec::new(),
        }
    }

    /// `layout_parse_free_ctx` (`layout-custom.c:201-210`).
    fn free(&mut self, cells: &mut Cells) {
        free_cell(cells, self.root.take(), false);
        self.cctxs.clear();
    }

    /// `layout_parse_remove_cctx` (`layout-custom.c:231-245`).
    fn remove_cctx(&mut self, lc: LayoutCellId) -> bool {
        match self.cctxs.iter().position(|c| c.cell == lc) {
            Some(i) => {
                self.cctxs.swap_remove(i);
                true
            }
            None => false,
        }
    }
}

/// `layout_find_bottomright` (`layout-custom.c:258-266`).
fn find_bottomright(cells: &Cells, mut lc: LayoutCellId) -> LayoutCellId {
    loop {
        let c = cell(cells, lc);
        if c.is_leaf() {
            return lc;
        }
        lc = *c.children.last().expect("layout node without children");
    }
}

/// `layout_checksum` (`layout-custom.c:268-280`): rotate right one bit in 16
/// bits, then add the byte (as a signed `char`).
pub fn checksum(layout: &[u8]) -> u16 {
    let mut csum: u16 = 0;
    for &b in cstr(layout) {
        csum = (csum >> 1).wrapping_add((csum & 1) << 15);
        csum = csum.wrapping_add((b as i8) as i16 as u16);
    }
    csum
}

/// `layout_dump` (`layout-custom.c:282-308`). `None` where C returns `NULL`;
/// with `OLD_FORMAT` a failure is `Some("0000,")`. The window is `__unused`
/// in C too; the cells carry their panes.
pub fn dump<H: LayoutHost>(
    host: &H,
    _w: WindowId,
    root: Option<LayoutCellId>,
    flags: LayoutDumpFlags,
) -> Option<ByteString> {
    let old = flags.contains(LayoutDumpFlags::OLD_FORMAT);
    let body = root.and_then(|root| append(host, root, flags));
    match body {
        Some(body) => {
            let mut out = ByteString::new();
            if old {
                let _ = write!(out.0, "{:04x},", checksum(&body));
                out.extend_from_slice(&body);
            } else {
                out.extend_from_slice(b"{\"V\":2,\"L\":");
                out.extend_from_slice(&body);
                out.push(b'}');
            }
            Some(out)
        }
        None if old => Some(ByteString::from("0000,")),
        None => None,
    }
}

/// `layout_cell_zindex` (`layout-custom.c:310-339`): floating panes before
/// this one in `w->z_index`, using saved cells when dumping the saved tree.
fn cell_zindex<H: LayoutHost>(host: &H, lc: LayoutCellId) -> u32 {
    let cells = host.cells();
    let wp = cell(cells, lc).pane.expect("leaf without a pane");
    let w = host.pane_window(wp);
    let saved = host.pane_saved_layout_cell(wp) == Some(lc);
    let mut i = 0;

    if saved
        && let Some(active) = host.window_active(w)
        && host.pane_flags(active).contains(PaneFlags::ZOOMED)
        && host
            .pane_saved_layout_cell(active)
            .is_some_and(|slc| cell(cells, slc).is_floating())
    {
        if wp == active {
            return 0;
        }
        i += 1;
    }
    for &wq in host.window_z_index(w) {
        if wq == wp {
            break;
        }
        let other = if saved {
            host.pane_saved_layout_cell(wq)
        } else {
            host.pane_layout_cell(wq)
        };
        if other.is_some_and(|o| cell(cells, o).is_floating()) {
            i += 1;
        }
    }
    i
}

/// `layout_append_v2` (`layout-custom.c:341-400`). Key order is fixed.
fn append_v2<H: LayoutHost>(host: &H, lc: LayoutCellId, out: &mut Vec<u8>) -> bool {
    let cells = host.cells();
    let c = cell(cells, lc);
    let t = match c.kind {
        LayoutType::Topbottom => 'v',
        LayoutType::Leftright => 'h',
        LayoutType::Windowpane => 'p',
    };
    let _ = write!(
        out,
        "{{\"t\":\"{}\",\"w\":{},\"h\":{},\"x\":{},\"y\":{}",
        t, c.g.sx, c.g.sy, c.g.xoff, c.g.yoff
    );
    if !c.is_leaf() {
        out.extend_from_slice(b",\"c\":[");
        let mut n = 0;
        for &child in &c.children {
            if !append_v2(host, child, out) {
                return false;
            }
            out.push(b',');
            n += 1;
        }
        if n == 0 {
            return false;
        }
        out.pop(); // trailing comma
        out.push(b']');
    } else {
        let Some(wp) = c.pane else {
            return false;
        };
        if Some(wp) == host.window_active(host.pane_window(wp)) {
            out.extend_from_slice(b",\"a\":true");
        } else if let Some(i) = host.pane_last_index(wp) {
            let _ = write!(out, ",\"l\":{i}");
        }
        let Some(i) = host.pane_index(wp) else {
            return false;
        };
        let _ = write!(out, ",\"i\":{i}");
        if c.is_floating() {
            let _ = write!(out, ",\"z\":{}", cell_zindex(host, lc));
        }
        let _ = write!(out, ",\"I\":\"%{}\"", host.pane_public_id(wp));
    }
    out.push(b'}');
    true
}

/// The tiled part of a layout as `layout_custom_copy_layout` builds it
/// (`layout-custom.c:441-506`): floating leaves dropped, empty nodes dropped,
/// one-child nodes collapsed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct V1Node {
    pub kind: LayoutType,
    pub g: LayoutGeometry,
    pub pane_public_id: Option<u32>,
    pub children: Vec<V1Node>,
}

/// `layout_custom_copy_layout`.
pub fn v1_view<H: LayoutHost>(host: &H, lc: LayoutCellId) -> Option<V1Node> {
    let cells = host.cells();
    let c = cell(cells, lc);
    if c.is_leaf() && c.is_floating() {
        return None;
    }
    let mut node = V1Node {
        kind: c.kind,
        g: c.g,
        pane_public_id: if c.is_leaf() {
            c.pane.map(|wp| host.pane_public_id(wp))
        } else {
            None
        },
        children: Vec::new(),
    };
    if !c.is_leaf() {
        for &child in &c.children {
            if let Some(copy) = v1_view(host, child) {
                node.children.push(copy);
            }
        }
        match node.children.len() {
            0 => return None,
            1 => return node.children.pop(),
            _ => {}
        }
    }
    Some(node)
}

/// `layout_custom_create_compat` (`layout-custom.c:509-520`).
fn v1_compat<H: LayoutHost>(host: &H, root: LayoutCellId) -> Option<V1Node> {
    let mut compat = v1_view(host, root)?;
    if compat.kind == LayoutType::Windowpane {
        compat.g.xoff = 0;
        compat.g.yoff = 0;
    }
    Some(compat)
}

/// `layout_append_v1` (`layout-custom.c:402-439`) on the compat tree.
fn append_v1(node: &V1Node, out: &mut Vec<u8>) {
    let _ = write!(
        out,
        "{}x{},{},{}",
        node.g.sx, node.g.sy, node.g.xoff, node.g.yoff
    );
    if let Some(id) = node.pane_public_id {
        let _ = write!(out, ",{id}");
    }
    let brackets: &[u8; 2] = match node.kind {
        LayoutType::Leftright => b"{}",
        LayoutType::Topbottom => b"[]",
        LayoutType::Windowpane => return,
    };
    out.push(brackets[0]);
    for child in &node.children {
        append_v1(child, out);
        out.push(b',');
    }
    out.pop(); // trailing comma
    out.push(brackets[1]);
}

/// `layout_append` (`layout-custom.c:548-566`).
fn append<H: LayoutHost>(
    host: &H,
    root: LayoutCellId,
    flags: LayoutDumpFlags,
) -> Option<ByteString> {
    let mut out = ByteString::with_capacity(1024);
    if flags.contains(LayoutDumpFlags::OLD_FORMAT) {
        let cells = host.cells();
        if !cell_is_tiled(cells, root) && !cell_has_tiled_child(cells, root) {
            return None;
        }
        let compat = v1_compat(host, root)?;
        append_v1(&compat, &mut out.0);
    } else if !append_v2(host, root, &mut out.0) {
        return None;
    }
    Some(out)
}

/// `layout_check` (`layout-custom.c:568-596`): tiled children share the
/// other-axis size and their split sizes plus borders fill the parent.
fn check(cells: &Cells, lc: LayoutCellId) -> bool {
    let c = cell(cells, lc);
    let mut n: u32 = 0;
    match c.kind {
        LayoutType::Windowpane => {}
        LayoutType::Leftright => {
            for &child in &c.children {
                if !cell_is_tiled(cells, child) && !cell_has_tiled_child(cells, child) {
                    continue;
                }
                let cg = cell(cells, child).g;
                if cg.sy != c.g.sy {
                    return false;
                }
                if !check(cells, child) {
                    return false;
                }
                n = n.wrapping_add(cg.sx.wrapping_add(1));
            }
            if n != 0 && n - 1 != c.g.sx {
                return false;
            }
        }
        LayoutType::Topbottom => {
            for &child in &c.children {
                if !cell_is_tiled(cells, child) && !cell_has_tiled_child(cells, child) {
                    continue;
                }
                let cg = cell(cells, child).g;
                if cg.sx != c.g.sx {
                    return false;
                }
                if !check(cells, child) {
                    return false;
                }
                n = n.wrapping_add(cg.sy.wrapping_add(1));
            }
            if n != 0 && n - 1 != c.g.sy {
                return false;
            }
        }
    }
    true
}

fn err(cause: &str) -> LayoutError {
    LayoutError::new(cause)
}

/// `layout_parse` (`layout-custom.c:598-730`): parse a layout string and
/// arrange the window. Every failure leaves the window unchanged.
pub fn parse<H: LayoutHost>(host: &mut H, w: WindowId, input: &[u8]) -> Result<(), LayoutError> {
    // Build the layout.
    let mut pctx = ParseCtx::new();
    if let Err(e) = construct(host.cells_mut(), input, &mut pctx) {
        pctx.free(host.cells_mut());
        return Err(e);
    }
    let with_floating = pctx.version > 1;

    match parse_fit(host, w, &mut pctx, with_floating) {
        Ok(()) => Ok(()),
        Err(e) => {
            pctx.free(host.cells_mut());
            Err(e)
        }
    }
}

fn parse_fit<H: LayoutHost>(
    host: &mut H,
    w: WindowId,
    pctx: &mut ParseCtx,
    with_floating: bool,
) -> Result<(), LayoutError> {
    // Check this window will fit into the layout.
    let npanes = host.window_count_panes(w, with_floating);
    if npanes == 0 {
        return Err(LayoutError::new(format!(
            "window @{} has no panes",
            host.window_public_id(w)
        )));
    }
    loop {
        let root = pctx.root.expect("constructed layout without a root");
        let ncells = count_cells(host.cells(), root, with_floating);
        if npanes > ncells {
            return Err(LayoutError::new(format!(
                "have {npanes} panes but need {ncells}"
            )));
        }
        if npanes == ncells {
            break;
        }

        // Fewer panes than cells, close the bottom right until none remain.
        let child = find_bottomright(host.cells(), root);
        if pctx.version > 1 && !pctx.remove_cctx(child) {
            return Err(err("empty/missing layout parse context"));
        }
        destroy_cell(host.cells_mut(), None, child, &mut pctx.root);
    }

    let lc = pctx.root.expect("fit loop lost the root");

    // Older versions of tmux could generate layouts with an incorrect top cell
    // size - if it is larger than the top child then correct that (if this is
    // still wrong the check code will catch it).
    {
        let cells = host.cells_mut();
        let root = cell(cells, lc);
        let mut sx: u32 = 0;
        let mut sy: u32 = 0;
        match root.kind {
            LayoutType::Windowpane => {}
            LayoutType::Leftright => {
                for &child in &root.children {
                    if cell_is_tiled(cells, child) || cell_has_tiled_child(cells, child) {
                        let g = cell(cells, child).g;
                        sy = g.sy.wrapping_add(1);
                        sx = sx.wrapping_add(g.sx.wrapping_add(1));
                    }
                }
            }
            LayoutType::Topbottom => {
                for &child in &root.children {
                    if cell_is_tiled(cells, child) || cell_has_tiled_child(cells, child) {
                        let g = cell(cells, child).g;
                        sx = g.sx.wrapping_add(1);
                        sy = sy.wrapping_add(g.sy.wrapping_add(1));
                    }
                }
            }
        }
        if root.kind != LayoutType::Windowpane
            && sx != 0
            && sy != 0
            && (root.g.sx != sx || root.g.sy != sy)
        {
            debug_print(cells, Some(lc), "layout_parse", 0);
            let root = super::tree::cell_mut(cells, lc);
            root.g.sx = sx - 1;
            root.g.sy = sy - 1;
        }
    }

    // Check the new layout.
    if !check(host.cells(), lc) {
        return Err(err("size mismatch after applying layout"));
    }

    // The root is now owned by this function; nothing below fails.
    pctx.root = None;

    // Resize window to the layout size.
    if cell_is_tiled(host.cells(), lc) || cell_has_tiled_child(host.cells(), lc) {
        let g = cell(host.cells(), lc).g;
        host.window_resize(w, g.sx, g.sy);
    }

    // Preserve floating panes for version 1.
    if pctx.version == 1 {
        let n = host.window_panes(w).len();
        for i in 0..n {
            let wp = host.window_panes(w)[i];
            if !host.pane_is_floating(wp) {
                continue;
            }
            let child = host
                .pane_layout_cell(wp)
                .expect("floating pane without a cell");
            let parent = cell(host.cells(), child)
                .parent
                .expect("floating cell without a parent");
            unlink_child(host.cells_mut(), parent, child);
        }
    }

    // Destroy the old layout and swap to the new.
    let old_root = host.window_layout_root(w);
    free_cell(host, old_root, false);
    host.set_window_layout_root(w, Some(lc));

    // Assign the panes into the cells.
    assign(host, w, pctx);

    // Update pane attributes.
    fix_offsets(host, w);
    fix_panes(host, w, None);
    if pctx.version > 1 {
        apply_ctx(host, w, pctx);
    }
    host.recalculate_sizes();
    debug_print(host.cells(), Some(lc), "layout_parse", 0);

    // Backwards compatibility.
    if pctx.version == 1 {
        host.fire_window_event(w, "window-layout-changed");
    }
    Ok(())
}

/// `layout_assign_from_ctx` (`layout-custom.c:733-749`): cells sorted by
/// index receive the window's panes in list order.
fn assign_from_ctx<H: LayoutHost>(host: &mut H, w: WindowId, pctx: &mut ParseCtx) {
    pctx.cctxs.sort_unstable_by_key(|c| c.index);
    for i in 0..pctx.cctxs.len() {
        let lc = pctx.cctxs[i].cell;
        let Some(&wp) = host.window_panes(w).get(i) else {
            break;
        };
        make_leaf(host, lc, wp);
    }
}

/// `layout_assign_fallback_tiled` (`layout-custom.c:755-779`): tiled cells
/// take the panes that have no cell yet, in tree order.
fn assign_fallback_tiled<H: LayoutHost>(
    host: &mut H,
    w: WindowId,
    cursor: &mut usize,
    lc: LayoutCellId,
) {
    let c = cell(host.cells(), lc);
    match c.kind {
        LayoutType::Windowpane => loop {
            let Some(&wp) = host.window_panes(w).get(*cursor) else {
                return;
            };
            if host.pane_layout_cell(wp).is_none() {
                make_leaf(host, lc, wp);
                *cursor += 1;
                return;
            }
            *cursor += 1;
        },
        LayoutType::Leftright | LayoutType::Topbottom => {
            let n = c.children.len();
            for i in 0..n {
                let child = cell(host.cells(), lc).children[i];
                assign_fallback_tiled(host, w, cursor, child);
            }
        }
    }
}

/// `layout_assign_fallback` (`layout-custom.c:785-806`).
fn assign_fallback<H: LayoutHost>(host: &mut H, w: WindowId, mut root: LayoutCellId) {
    let mut cursor = 0;
    assign_fallback_tiled(host, w, &mut cursor, root);

    if host.window_count_panes(w, true) > 1 && cell(host.cells(), root).is_leaf() {
        root = replace_with_node(host, w, root, LayoutType::Topbottom);
    }

    let n = host.window_panes(w).len();
    for i in 0..n {
        let wp = host.window_panes(w)[i];
        if host.pane_is_floating(wp) {
            let lc = host
                .pane_layout_cell(wp)
                .expect("floating pane without a cell");
            link_child_tail(host.cells_mut(), root, lc);
        }
    }
}

/// `layout_assign` (`layout-custom.c:809-816`).
fn assign<H: LayoutHost>(host: &mut H, w: WindowId, pctx: &mut ParseCtx) {
    if !pctx.cctxs.is_empty() {
        assign_from_ctx(host, w, pctx);
    } else {
        let root = host.window_layout_root(w).expect("assign without a root");
        assign_fallback(host, w, root);
    }
}

/// One `%5u`/`%5d` field of the v1 cell scanner: pure digits, the value from
/// the first five. `limit` rejects a sixth digit where the format's next
/// literal would fail to match.
fn v1_field(input: &[u8], pos: &mut usize, last: bool) -> Option<u32> {
    let start = *pos;
    let mut value: u32 = 0;
    while let Some(&b) = input.get(*pos)
        && b.is_ascii_digit()
    {
        if *pos - start < 5 {
            value = value * 10 + u32::from(b - b'0');
        } else if !last {
            return None;
        }
        *pos += 1;
    }
    if *pos == start {
        return None;
    }
    Some(value)
}

/// `layout_construct_cell` (`layout-custom.c:818-870`).
fn construct_cell(
    cells: &mut Cells,
    parent: Option<LayoutCellId>,
    input: &[u8],
    pos: &mut usize,
) -> Option<LayoutCellId> {
    if !input.get(*pos).is_some_and(u8::is_ascii_digit) {
        return None;
    }
    let sx = v1_field(input, pos, false)?;
    if input.get(*pos) != Some(&b'x') {
        return None;
    }
    *pos += 1;
    let sy = v1_field(input, pos, false)?;
    if input.get(*pos) != Some(&b',') {
        return None;
    }
    *pos += 1;
    let xoff = v1_field(input, pos, false)?;
    if input.get(*pos) != Some(&b',') {
        return None;
    }
    *pos += 1;
    let yoff = v1_field(input, pos, true)?;

    if !(PANE_MINIMUM..=PANE_MAXIMUM).contains(&sx)
        || !(PANE_MINIMUM..=PANE_MAXIMUM).contains(&sy)
        || xoff > WINDOW_MAXIMUM
        || yoff > WINDOW_MAXIMUM
    {
        return None;
    }

    // An optional pane id: a comma and digits not followed by `x`.
    if input.get(*pos) == Some(&b',') {
        let saved = *pos;
        *pos += 1;
        while input.get(*pos).is_some_and(u8::is_ascii_digit) {
            *pos += 1;
        }
        if input.get(*pos) == Some(&b'x') {
            *pos = saved;
        }
    }

    let lc = create_cell(cells, parent);
    let c = super::tree::cell_mut(cells, lc);
    c.g.sx = sx;
    c.g.sy = sy;
    c.g.xoff = xoff as i32;
    c.g.yoff = yoff as i32;
    Some(lc)
}

/// `layout_construct_v1` (`layout-custom.c:873-939`).
fn construct_v1(
    cells: &mut Cells,
    parent: Option<LayoutCellId>,
    input: &[u8],
    pos: &mut usize,
    depth: u32,
) -> Option<LayoutCellId> {
    if depth > LAYOUT_V1_MAX_DEPTH {
        return None;
    }

    let lc = construct_cell(cells, parent, input, pos)?;

    let kind = match input.get(*pos) {
        None | Some(b',' | b'}' | b']') => return Some(lc),
        Some(b'{') => LayoutType::Leftright,
        Some(b'[') => LayoutType::Topbottom,
        Some(_) => {
            free_cell(cells, Some(lc), false);
            return None;
        }
    };
    super::tree::cell_mut(cells, lc).kind = kind;

    loop {
        *pos += 1;
        match construct_v1(cells, Some(lc), input, pos, depth + 1) {
            Some(child) => link_child_tail(cells, lc, child),
            None => {
                free_cell(cells, Some(lc), false);
                return None;
            }
        }
        if input.get(*pos) != Some(&b',') {
            break;
        }
    }

    let closer = if kind == LayoutType::Leftright {
        b'}'
    } else {
        b']'
    };
    if input.get(*pos) != Some(&closer) {
        free_cell(cells, Some(lc), false);
        return None;
    }
    *pos += 1;
    Some(lc)
}

/// `layout_parse_json` (`layout-custom.c:945-965`).
fn parse_json(cells: &mut Cells, root: &JsonNode, pctx: &mut ParseCtx) -> Result<(), LayoutError> {
    if root.as_object().is_none() {
        return Err(err("invalid layout json"));
    }
    let num = root
        .find_number(b"V")
        .map_err(|e| LayoutError::new(e.cause()))?;
    pctx.version = num;

    let object = root
        .find_object(b"L")
        .map_err(|e| LayoutError::new(e.cause()))?;
    pctx.root = Some(parse_json_layout(cells, object, None, pctx)?);
    Ok(())
}

fn json_err(e: json::JsonError) -> LayoutError {
    LayoutError::new(e.cause())
}

/// `layout_parse_json_layout` (`layout-custom.c:968-1093`). A failed cell
/// frees its partial subtree.
fn parse_json_layout(
    cells: &mut Cells,
    node: &JsonNode,
    parent: Option<LayoutCellId>,
    pctx: &mut ParseCtx,
) -> Result<LayoutCellId, LayoutError> {
    let lc = create_cell(cells, parent);
    match parse_json_cell(cells, node, lc, pctx) {
        Ok(()) => Ok(lc),
        Err(e) => {
            free_cell(cells, Some(lc), false);
            Err(e)
        }
    }
}

fn parse_json_cell(
    cells: &mut Cells,
    node: &JsonNode,
    lc: LayoutCellId,
    pctx: &mut ParseCtx,
) -> Result<(), LayoutError> {
    let t = node.find_string(b"t").map_err(json_err)?;
    let kind = match t {
        b"p" => LayoutType::Windowpane,
        b"v" => LayoutType::Topbottom,
        b"h" => LayoutType::Leftright,
        other => {
            let mut cause = ByteString::from("unknown cell type \"");
            cause.extend_from_slice(cstr(other));
            cause.push(b'"');
            return Err(LayoutError { cause });
        }
    };
    super::tree::cell_mut(cells, lc).kind = kind;

    let num = node.find_number(b"w").map_err(json_err)?;
    if num < PANE_MINIMUM as i64 || num > PANE_MAXIMUM as i64 {
        return Err(LayoutError::new(format!("invalid width {num}")));
    }
    super::tree::cell_mut(cells, lc).g.sx = num as u32;

    let num = node.find_number(b"h").map_err(json_err)?;
    if num < PANE_MINIMUM as i64 || num > PANE_MAXIMUM as i64 {
        return Err(LayoutError::new(format!("invalid height {num}")));
    }
    super::tree::cell_mut(cells, lc).g.sy = num as u32;

    let num = node.find_number(b"x").map_err(json_err)?;
    if num < -(WINDOW_MAXIMUM as i64) || num > WINDOW_MAXIMUM as i64 {
        return Err(LayoutError::new(format!("invalid x-offset {num}")));
    }
    super::tree::cell_mut(cells, lc).g.xoff = num as i32;

    let num = node.find_number(b"y").map_err(json_err)?;
    if num < -(WINDOW_MAXIMUM as i64) || num > WINDOW_MAXIMUM as i64 {
        return Err(LayoutError::new(format!("invalid y-offset {num}")));
    }
    super::tree::cell_mut(cells, lc).g.yoff = num as i32;

    if kind == LayoutType::Windowpane {
        // "I" is currently ignored.
        if node.find(b"c").is_some() {
            return Err(err("panes cannot have children"));
        }
        let num = node.find_number(b"i").map_err(json_err)?;
        if num < 0 || num > i32::MAX as i64 {
            return Err(LayoutError::new(format!("invalid index {num}")));
        }
        let index = num as i32;

        let mut active = -1;
        let mut last = -1;
        if node.find(b"a").is_some() {
            let boolean = node.find_boolean(b"a").map_err(json_err)?;
            active = i32::from(boolean);
            if boolean {
                pctx.num_active += 1;
            }
        } else if node.find(b"l").is_some() {
            let num = node.find_number(b"l").map_err(json_err)?;
            if num < 0 || num > i32::MAX as i64 {
                return Err(LayoutError::new(format!("invalid last {num}")));
            }
            last = num as i32;
        }

        let zindex = if node.find(b"z").is_some() {
            let num = node.find_number(b"z").map_err(json_err)?;
            if num < 0 || num > i32::MAX as i64 - 1 {
                return Err(LayoutError::new(format!("invalid floating zindex {num}")));
            }
            super::tree::cell_mut(cells, lc)
                .flags
                .insert(LayoutCellFlags::FLOATING);
            num as i32
        } else {
            i32::MAX
        };

        pctx.cctxs.push(CellCtx {
            cell: lc,
            active,
            last,
            index,
            zindex,
        });
    } else {
        let array = node.find_array(b"c").map_err(json_err)?;
        if array.len() < 2 {
            return Err(err("nodes must have more than one child"));
        }
        for member in array {
            let child = parse_json_layout(cells, member, Some(lc), pctx)?;
            link_child_tail(cells, lc, child);
        }
    }
    Ok(())
}

/// `sscanf("%hx,%n") == 1 && n == 5` (`layout-custom.c:1107-1111`): C
/// hexadecimal syntax (sign, optional `0x`) with five bytes consumed
/// including the comma.
pub(super) fn v1_header(input: &[u8]) -> Option<u16> {
    let mut pos = 0;
    let mut negative = false;
    match input.first() {
        Some(b'+') => pos += 1,
        Some(b'-') => {
            negative = true;
            pos += 1;
        }
        _ => {}
    }
    if input.get(pos) == Some(&b'0')
        && matches!(input.get(pos + 1), Some(b'x' | b'X'))
        && input.get(pos + 2).is_some_and(u8::is_ascii_hexdigit)
    {
        pos += 2;
    }
    let start = pos;
    let mut value: u64 = 0;
    while let Some(&b) = input.get(pos)
        && b.is_ascii_hexdigit()
    {
        value = value
            .wrapping_mul(16)
            .wrapping_add(u64::from((b as char).to_digit(16)?));
        pos += 1;
    }
    if pos == start {
        return None;
    }
    if input.get(pos) != Some(&b',') {
        return None;
    }
    pos += 1;
    if pos != 5 {
        return None;
    }
    let value = if negative {
        value.wrapping_neg()
    } else {
        value
    };
    Some(value as u16)
}

/// `layout_construct` (`layout-custom.c:1095-1150`).
fn construct(cells: &mut Cells, input: &[u8], pctx: &mut ParseCtx) -> Result<(), LayoutError> {
    let input = cstr(input);
    let mut start = 0;
    while input.get(start).is_some_and(u8::is_ascii_whitespace) {
        start += 1;
    }
    let input = &input[start..];

    if input.first() != Some(&b'{') {
        // sniffing version
        let csum = v1_header(input).ok_or_else(|| err("malformed layout header"))?;
        let body = &input[5..];
        if csum != checksum(body) {
            return Err(err("invalid layout checksum"));
        }
        let mut pos = 0;
        pctx.root = construct_v1(cells, None, body, &mut pos, 0);
        if pctx.root.is_none() {
            return Err(err("invalid layout"));
        }
        if pos != body.len() {
            return Err(err("trailing data"));
        }
        pctx.version = 1;
    } else {
        let json = json::parse(input).map_err(json_err)?;
        parse_json(cells, &json, pctx)?;

        if pctx.version != 2 {
            return Err(err("version mismatch"));
        }
        if pctx.num_active > 1 {
            return Err(err("more than one active pane"));
        }
        if pctx.cctxs.is_empty() {
            return Err(err("no panes"));
        }
        check_indexes(pctx)?;
    }
    Ok(())
}

/// `layout_parse_apply_ctx` (`layout-custom.c:1153-1204`): z order, the
/// active pane and the last-pane stack from the cell contexts.
fn apply_ctx<H: LayoutHost>(host: &mut H, w: WindowId, pctx: &mut ParseCtx) {
    // Apply z-indexes: drop the now-floating panes, then insert them at the
    // head from the highest z down so the lowest z is first.
    let mut i = 0;
    while i < host.window_z_index(w).len() {
        let wp = host.window_z_index(w)[i];
        if host.pane_is_floating(wp) {
            host.window_z_index_mut(w).remove(i);
        } else {
            i += 1;
        }
    }

    pctx.cctxs
        .sort_unstable_by_key(|c| std::cmp::Reverse(c.zindex));
    for cctx in &pctx.cctxs {
        let wp = ctx_pane(host, cctx);
        if host.pane_is_floating(wp) {
            host.window_z_index_mut(w).insert(0, wp);
        }
    }

    // Set the active pane.
    if let Some(cctx) = pctx.cctxs.iter().find(|c| c.active == 1) {
        let wp = ctx_pane(host, cctx);
        host.window_set_active_pane(w, wp, true);
    }

    // Apply last panes.
    while let Some(&wp) = host.window_last_panes(w).first() {
        host.window_last_panes_remove(w, wp);
    }

    pctx.cctxs
        .sort_unstable_by_key(|c| std::cmp::Reverse(c.last));
    for cctx in &pctx.cctxs {
        if cctx.last < 0 || cctx.active == 1 {
            continue;
        }
        let wp = ctx_pane(host, cctx);
        host.window_last_panes_push(w, wp);
    }
}

fn ctx_pane<H: LayoutHost>(host: &H, cctx: &CellCtx) -> PaneId {
    cell(host.cells(), cctx.cell)
        .pane
        .expect("assigned layout leaf without a pane")
}

/// `layout_parse_ctx_check_indexes` (`layout-custom.c:1207-1257`): duplicate
/// pane indexes, z-indexes and last indexes, in that order.
fn check_indexes(pctx: &mut ParseCtx) -> Result<(), LayoutError> {
    let cctxs = &mut pctx.cctxs;
    cctxs.sort_unstable_by_key(|c| c.index);
    if cctxs.windows(2).any(|p| p[0].index == p[1].index) {
        return Err(err("duplicate pane index"));
    }

    // Sorted in descending order, so the panes without a z-index come first
    // and the floating panes run to the end.
    cctxs.sort_unstable_by_key(|c| std::cmp::Reverse(c.zindex));
    let n = cctxs.iter().take_while(|c| c.zindex == i32::MAX).count();
    if cctxs[n..].windows(2).any(|p| p[0].zindex == p[1].zindex) {
        return Err(err("duplicate pane z-index"));
    }

    // Sorted in descending order, so the panes without a last index come
    // last.
    cctxs.sort_unstable_by_key(|c| std::cmp::Reverse(c.last));
    let n = cctxs.iter().take_while(|c| c.last >= 0).count();
    if cctxs[..n].windows(2).any(|p| p[0].last == p[1].last) {
        return Err(err("duplicate last pane index"));
    }
    Ok(())
}
