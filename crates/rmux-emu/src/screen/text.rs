// Ported from tmux screen-write.c @ 8f25579c
use super::borders::border_cell;
use super::write::{DrawCommand, ScreenWriteCtx};
use super::{BorderCell, BoxLines, Screen, ScreenMode};
use crate::cell::{DEFAULT_CELL, GridAttributes, GridCell, GridCellFlags};
use crate::colour::Colour;
use rmux_util::utf8::{self, Utf8Data, Utf8State};

fn cbytes(bytes: &[u8]) -> &[u8] {
    &bytes[..bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len())]
}

// An incomplete sequence stops the caller; malformed complete sequences are consumed.
fn next_utf8(bytes: &[u8], pos: &mut usize) -> Option<Result<Utf8Data, ()>> {
    let mut data = Utf8Data::open(bytes[*pos]).ok()?;
    *pos += 1;
    if bytes.len() - *pos < usize::from(data.size - 1) {
        *pos = bytes.len();
        return None;
    }
    loop {
        let state = data.append(bytes[*pos]);
        *pos += 1;
        match state {
            Utf8State::More => (),
            Utf8State::Done => return Some(Ok(data)),
            Utf8State::Error => return Some(Err(())),
        }
    }
}

impl ScreenWriteCtx<'_> {
    pub fn strlen(bytes: &[u8]) -> usize {
        let bytes = cbytes(bytes);
        let mut pos = 0;
        let mut width = 0;
        while pos < bytes.len() {
            let byte = bytes[pos];
            if byte > 0x7f && Utf8Data::open(byte).is_ok() {
                match next_utf8(bytes, &mut pos) {
                    Some(Ok(data)) => width += usize::from(data.width),
                    Some(Err(())) => (),
                    None => break,
                }
            } else {
                if byte == b'\t' || (0x20..0x7f).contains(&byte) {
                    width += 1;
                }
                pos += 1;
            }
        }
        width
    }

    pub fn puts(&mut self, style: &GridCell, bytes: &[u8]) {
        self.nputs(-1, style, bytes);
    }

    pub fn nputs(&mut self, maxlen: isize, style: &GridCell, bytes: &[u8]) {
        let bytes = cbytes(bytes);
        let mut cell = *style;
        let mut pos = 0;
        let mut width = 0;
        while pos < bytes.len() {
            let byte = bytes[pos];
            if byte > 0x7f && Utf8Data::open(byte).is_ok() {
                let data = match next_utf8(bytes, &mut pos) {
                    Some(Ok(data)) => data,
                    Some(Err(())) => continue,
                    None => break,
                };
                cell.data = data;
                if maxlen > 0 && width + usize::from(data.width) > maxlen as usize {
                    while width < maxlen as usize {
                        cell.data = Utf8Data::set(b' ');
                        self.cell(&cell);
                        width += 1;
                    }
                    break;
                }
                width += usize::from(data.width);
                self.cell(&cell);
            } else {
                if maxlen > 0 && width + 1 > maxlen as usize {
                    break;
                }
                if byte == 1 {
                    cell.attr.0 ^= GridAttributes::CHARSET.0;
                } else if byte == b'\n' {
                    self.linefeed(false, Colour::DEFAULT);
                    self.carriagereturn();
                } else if byte == b'\t' || (0x20..0x7f).contains(&byte) {
                    width += 1;
                    cell.data = Utf8Data::set(byte);
                    self.cell(&cell);
                }
                pos += 1;
            }
        }
    }

    pub fn text(
        &mut self,
        cx: u32,
        width: u32,
        lines: u32,
        more: bool,
        style: &GridCell,
        bytes: &[u8],
    ) -> bool {
        let text = utf8::from_cstr(bytes);
        let mut cell = *style;
        let cy = self.screen.cy;
        let mut idx = 0;
        let mut left = cx.wrapping_add(width).wrapping_sub(self.screen.cx);
        loop {
            let mut at = 0;
            let mut end = idx;
            while end < text.len() {
                if text[end].bytes() == b"\n" || at + u32::from(text[end].width) > left {
                    break;
                }
                at += u32::from(text[end].width);
                end += 1;
            }
            let next = if end == text.len() {
                end
            } else if text[end].bytes() == b"\n" || text[end].bytes() == b" " {
                end + 1
            } else {
                let mut i = end;
                while i > idx && text[i].bytes() != b" " {
                    i -= 1;
                }
                if i != idx {
                    end = i;
                    i + 1
                } else {
                    end
                }
            };
            for data in &text[idx..end] {
                cell.data.copy_from(data);
                self.cell(&cell);
            }
            idx = next;
            if self.screen.cy == cy.wrapping_add(lines).wrapping_sub(1) || idx == text.len() {
                break;
            }
            self.cursormove(cx as i32, self.screen.cy.wrapping_add(1) as i32, false);
            left = width;
        }
        if (self.screen.cy == cy.wrapping_add(lines).wrapping_sub(1)
            && (!more || self.screen.cx == cx.wrapping_add(width)))
            || idx != text.len()
        {
            return false;
        }
        if !more || self.screen.cx == cx.wrapping_add(width) {
            self.cursormove(cx as i32, self.screen.cy.wrapping_add(1) as i32, false);
        }
        true
    }

    pub fn fast_copy(&mut self, src: &Screen, px: u32, py: u32, nx: u32, ny: u32) {
        if nx == 0 || ny == 0 {
            return;
        }
        let (cx, cy) = (self.screen.cx, self.screen.cy);
        for yy in py..py.wrapping_add(ny) {
            if yy >= src.grid.hsize() + src.grid.sy() {
                break;
            }
            self.screen.cx = cx;
            let mut snapshot = self.snapshot(false);
            for xx in px..px.wrapping_add(nx) {
                // Pinned C indexes the destination line by cy without hsize.
                let dest_row = self.screen.cy;
                if xx >= src.grid.get_line(yy).cellsize()
                    && self.screen.cx >= self.screen.grid.get_line(dest_row).cellsize()
                {
                    break;
                }
                let cell = src.grid.get_cell(xx, yy);
                if xx + u32::from(cell.data.width) > px + nx {
                    break;
                }
                self.screen
                    .grid
                    .view_set_cell(self.screen.cx, self.screen.cy, &cell);
                if !self.fully_visible(self.screen.cx, self.screen.cy, 1) {
                    break;
                }
                snapshot.wrapped = false;
                snapshot.invalidate_cursor = false;
                self.emit(DrawCommand::Cell(&cell), snapshot);
                snapshot.old_cx += 1;
                self.screen.cx += 1;
            }
            self.screen.cy += 1;
        }
        self.screen.cx = cx;
        self.screen.cy = cy;
    }

    pub fn preview(&mut self, src: &Screen, nx: u32, ny: u32) {
        let (cx, cy) = (self.screen.cx, self.screen.cy);
        let cursor = src.mode.contains(ScreenMode::CURSOR);
        let offset = |position: u32, count: u32, size: u32| {
            let mut start = position.saturating_sub(count / 3);
            if start + count > size {
                start = size.saturating_sub(count);
            }
            start
        };
        let (px, py) = if cursor {
            (
                offset(src.cx, nx, src.grid.sx()),
                offset(src.cy, ny, src.grid.sy()),
            )
        } else {
            (0, 0)
        };
        self.fast_copy(src, px, src.grid.hsize() + py, nx, ny);
        if cursor {
            let mut cell = src.grid.view_get_cell(src.cx, src.cy);
            cell.attr.insert(GridAttributes::REVERSE);
            self.set_cursor(Some(cx + src.cx - px), Some(cy + src.cy - py));
            self.cell(&cell);
        }
    }

    pub fn hline(
        &mut self,
        nx: u32,
        left: bool,
        right: bool,
        lines: BoxLines,
        style: Option<&GridCell>,
    ) {
        let (cx, cy) = (self.screen.cx, self.screen.cy);
        let mut cell = style.copied().unwrap_or(DEFAULT_CELL);
        cell.attr.insert(GridAttributes::CHARSET);
        border_cell(
            lines,
            if left {
                BorderCell::Urd
            } else {
                BorderCell::Lr
            },
            &mut cell,
        );
        self.cell(&cell);
        border_cell(lines, BorderCell::Lr, &mut cell);
        for _ in 1..nx - 1 {
            self.cell(&cell);
        }
        border_cell(
            lines,
            if right {
                BorderCell::Uld
            } else {
                BorderCell::Lr
            },
            &mut cell,
        );
        self.cell(&cell);
        self.set_cursor(Some(cx), Some(cy));
    }

    pub fn vline(&mut self, ny: u32, top: bool, bottom: bool, style: Option<&GridCell>) {
        let (cx, cy) = (self.screen.cx, self.screen.cy);
        let mut cell = style.copied().unwrap_or(DEFAULT_CELL);
        cell.attr.insert(GridAttributes::CHARSET);
        cell.data = Utf8Data::set(if top { b'w' } else { b'x' });
        self.cell(&cell);
        cell.data = Utf8Data::set(b'x');
        for i in 1..ny - 1 {
            self.set_cursor(Some(cx), Some(cy + i));
            self.cell(&cell);
        }
        self.set_cursor(Some(cx), Some(cy + ny - 1));
        cell.data = Utf8Data::set(if bottom { b'v' } else { b'x' });
        self.cell(&cell);
        self.set_cursor(Some(cx), Some(cy));
    }

    pub fn draw_box(&mut self, nx: u32, ny: u32, lines: BoxLines, style: Option<&GridCell>) {
        let (cx, cy) = (self.screen.cx, self.screen.cy);
        let mut cell = style.copied().unwrap_or(DEFAULT_CELL);
        cell.attr.insert(GridAttributes::CHARSET);
        cell.flags.insert(GridCellFlags::NOPALETTE);
        for (index, (y, first, last)) in [
            (cy, BorderCell::Rd, BorderCell::Ld),
            (cy + ny - 1, BorderCell::Ru, BorderCell::Lu),
        ]
        .into_iter()
        .enumerate()
        {
            if index != 0 {
                self.set_cursor(Some(cx), Some(y));
            }
            border_cell(lines, first, &mut cell);
            self.cell(&cell);
            border_cell(lines, BorderCell::Lr, &mut cell);
            for _ in 1..nx - 1 {
                self.cell(&cell);
            }
            border_cell(lines, last, &mut cell);
            self.cell(&cell);
        }
        border_cell(lines, BorderCell::Ud, &mut cell);
        for i in 1..ny - 1 {
            self.set_cursor(Some(cx), Some(cy + i));
            self.cell(&cell);
            self.set_cursor(Some(cx + nx - 1), Some(cy + i));
            self.cell(&cell);
        }
        self.set_cursor(Some(cx), Some(cy));
    }
}
