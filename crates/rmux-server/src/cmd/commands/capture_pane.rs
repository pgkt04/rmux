// Ported from tmux cmd-capture-pane.c @ 8f25579c
use crate::{
    cmd::{
        Command,
        queue::{self, CmdReturn},
    },
    ids::QueueItemId,
    model,
    server::{Server, operations},
};
use rmux_emu::{
    cell::GridCell,
    grid::{GridLineFlags, GridStringFlags, StringCellsCtx},
};

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let Some(pane) = server.queue.items.get(item).and_then(|i| i.target.wp) else {
        return CmdReturn::Error;
    };
    if command.entry.name == b"clear-history" {
        let _ = model::pane::pane_reset_mode_all(server, pane);
        if let Some(p) = server.panes.get_mut(pane) {
            p.base.grid.clear_history();
            if args.has(b'H') != 0 {
                if let Some(links) = p.base.hyperlinks.as_ref() {
                    let _ = server.hyperlinks.reset(links);
                }
            }
        }
        crate::tsp::broker::drain_anchors(server, pane);
        if let Some(window) = server.panes.get(pane).map(|pane| pane.window) {
            operations::server_redraw_window(server, window);
        }
        return CmdReturn::Normal;
    }
    let bounds = |server: &mut Server, flag: u8| {
        args.strtonum_and_expand(server, flag, i32::MIN as i64, i16::MAX as i64, item)
            .ok()
    };
    let start = bounds(server, b'S');
    let end = bounds(server, b'E');
    let Some(p) = server.panes.get(pane) else {
        return CmdReturn::Error;
    };
    let mut output = Vec::new();
    if args.has(b'R') != 0 {
        output = raw_grid(server, &p.base);
    } else if args.has(b'P') != 0 && args.has(b'H') == 0 {
        for byte in p.parser.pending() {
            if args.has(b'C') != 0
                && (*byte < b' '
                    || *byte == b'\\'
                    || (cfg!(any(target_os = "macos", target_arch = "x86_64")) && *byte >= 128))
            {
                output.extend_from_slice(format!("\\{byte:03o}").as_bytes());
            } else {
                output.push(*byte);
            }
        }
    } else {
        // cmd-capture-pane.c:280-288: the top mode's get_screen (only copy
        // and view mode have one), else the base screen.
        let screen = if args.has(b'a') == 0 && args.has(b'M') != 0 {
            p.modes
                .first()
                .and_then(|mode| crate::modes::copy::backing_screen(server, mode.id))
                .unwrap_or(&p.base)
        } else {
            &p.base
        };
        let grid = if args.has(b'a') != 0 {
            match screen.saved_grid.as_ref() {
                Some(grid) => grid,
                None => {
                    if args.has(b'q') == 0 {
                        queue::error(server, item, b"no alternate screen");
                        return CmdReturn::Error;
                    }
                    return deliver(server, command, item, Vec::new());
                }
            }
        } else {
            &screen.grid
        };
        let last_row = grid.hsize() + grid.sy() - 1;
        let coordinate = |value: Option<i64>, fallback: u32| {
            value
                .map(|n| (grid.hsize() as i64 + n).clamp(0, last_row as i64) as u32)
                .unwrap_or(fallback)
        };
        let mut top = if args.get(b'S') == Some(b"-") {
            0
        } else {
            coordinate(start, grid.hsize())
        };
        let mut bottom = if args.get(b'E') == Some(b"-") {
            last_row
        } else {
            coordinate(end, last_row)
        };
        if bottom < top {
            std::mem::swap(&mut top, &mut bottom);
        }
        let mut flags = GridStringFlags::default();
        if args.has(b'e') != 0 {
            flags.insert(GridStringFlags::WITH_SEQUENCES);
        }
        if args.has(b'C') != 0 {
            flags.insert(GridStringFlags::ESCAPE_SEQUENCES);
        }
        if args.has(b'J') == 0 {
            if args.has(b'T') == 0 {
                flags.insert(GridStringFlags::EMPTY_CELLS);
            }
            if args.has(b'N') == 0 {
                flags.insert(GridStringFlags::TRIM_SPACES);
            }
        }
        let mut last = GridCell::default();
        let mut links = Vec::new();
        for row in top..=bottom {
            let line = grid.get_line(row);
            let mut hyperlink_line = Vec::new();
            if args.has(b'H') != 0 {
                let before = hyperlink_line.len();
                if line.flags.contains(GridLineFlags::HYPERLINK) {
                    for x in 0..line.cellused() {
                        let cell = grid.get_cell(x, row);
                        if links.contains(&cell.link) {
                            continue;
                        }
                        let Some(link) = screen
                            .hyperlinks
                            .as_ref()
                            .and_then(|store| server.hyperlinks.get(store, cell.link))
                        else {
                            continue;
                        };
                        if links.len() == grid.sx() as usize {
                            break;
                        }
                        links.push(cell.link);
                        if hyperlink_line.len() != before {
                            hyperlink_line.push(b' ');
                        }
                        hyperlink_line.extend_from_slice(link.uri());
                    }
                }
                if hyperlink_line.is_empty() {
                    continue;
                }
            }
            if args.has(b'L') != 0 {
                output
                    .extend_from_slice(format!("{} ", row as i64 - grid.hsize() as i64).as_bytes());
            }
            if args.has(b'I') != 0 {
                output.extend_from_slice(
                    format!("{} ", line.time.to_wall(server.start_time.0)).as_bytes(),
                );
            }
            if args.has(b'F') != 0 {
                let before = output.len();
                for (flag, byte) in [
                    (GridLineFlags::DEAD, b'D'),
                    (GridLineFlags::HYPERLINK, b'H'),
                    (GridLineFlags::START_OUTPUT, b'O'),
                    (GridLineFlags::START_PROMPT, b'P'),
                    (GridLineFlags::WRAPPED, b'W'),
                    (GridLineFlags::EXTENDED, b'X'),
                ] {
                    if line.flags.contains(flag) {
                        output.push(byte);
                    }
                }
                if output.len() == before {
                    output.push(b'-');
                }
                output.push(b' ');
            }
            if args.has(b'H') != 0 {
                output.extend_from_slice(&hyperlink_line);
            } else {
                let mut context = StringCellsCtx {
                    last: Some(&mut last),
                    flags,
                    hyperlinks: screen
                        .hyperlinks
                        .as_ref()
                        .map(|links| (&server.hyperlinks, links)),
                };
                let bytes = grid.string_cells(0, row, p.base.grid.sx(), &mut context);
                output.extend_from_slice(rmux_util::bytes::cstr(&bytes));
            }
            if args.has(b'J') == 0 || !line.flags.contains(GridLineFlags::WRAPPED) {
                output.push(b'\n');
            }
        }
    }
    deliver(server, command, item, output)
}
fn deliver(
    server: &mut Server,
    command: &Command,
    item: QueueItemId,
    mut output: Vec<u8>,
) -> CmdReturn {
    if command.args.has(b'p') != 0 {
        let client = server.queue.items.get(item).and_then(|item| item.client);
        let control = client
            .and_then(|client| server.clients.get(client))
            .is_some_and(|client| client.flags.contains(crate::client::ClientFlags::CONTROL));
        if !control && !crate::server::file::can_print(server, client) {
            queue::error(server, item, b"can't write to client");
            return CmdReturn::Error;
        }
        if output.last() == Some(&b'\n') {
            output.pop();
        }
        queue::print(server, item, &output);
    } else {
        let limit = server
            .options
            .get_number(server.options.global, b"buffer-limit") as u32;
        if let Err(cause) = model::paste::paste_set(server, output, command.args.get(b'b'), limit) {
            queue::error(server, item, cause.error.to_string().as_bytes());
            return CmdReturn::Error;
        }
    }
    CmdReturn::Normal
}
fn raw_grid(server: &Server, screen: &rmux_emu::screen::Screen) -> Vec<u8> {
    use rmux_emu::{
        cell::GridCellFlags,
        colour::{ColourFlags, write_colour},
        grid::names,
    };
    use rmux_util::{utf8::strvis, vis::VisFlags};
    let grid = &screen.grid;
    let mut output = format!(
        "G {}x{} ({}/{})\n",
        grid.sx(),
        grid.sy(),
        grid.hsize(),
        grid.hlimit()
    )
    .into_bytes();
    for y in 0..grid.hsize() + grid.sy() {
        let line = grid.get_line(y);
        let position = if y < grid.hsize() {
            "-".to_owned()
        } else {
            (y - grid.hsize()).to_string()
        };
        output.extend_from_slice(
            format!(
                "\tL {y} ({position}) flags={}[{:x}] {}/{}",
                names::line_flags_string(line.flags),
                line.flags.bits(),
                line.cellused(),
                line.cellsize()
            )
            .as_bytes(),
        );
        if line.flags.intersects(GridLineFlags::OSC133_FLAGS) {
            let od = line.osc133;
            output.extend_from_slice(
                format!(
                    " osc133={},{},{},{},{}",
                    od.prompt_col, od.cmd_col, od.out_start_col, od.out_end_col, od.exit_status
                )
                .as_bytes(),
            );
        }
        output.push(b'\n');
        for x in 0..grid.sx() {
            let cell = grid.get_cell(x, y);
            let mut data = Vec::new();
            strvis(
                &mut data,
                rmux_util::bytes::cstr(&cell.data.data[..cell.data.size as usize]),
                VisFlags::OCTAL | VisFlags::CSTYLE | VisFlags::TAB | VisFlags::NL,
            );
            let mut flags = cell.flags;
            if cell.fg.0 & ColourFlags::_256.bits() as i32 != 0 {
                flags.insert(GridCellFlags::FG256);
            }
            if cell.bg.0 & ColourFlags::_256.bits() as i32 != 0 {
                flags.insert(GridCellFlags::BG256);
            }
            output.extend_from_slice(
                format!(
                    "\t\tC {y},{x} data=({},{},",
                    cell.data.width, cell.data.size
                )
                .as_bytes(),
            );
            output.extend_from_slice(&data);
            output.extend_from_slice(
                format!(
                    ") flags={}[{:x}] attr={}[{:x}]",
                    names::cell_flags_string(flags),
                    flags.bits(),
                    names::cell_attr_string(cell.attr),
                    cell.attr.bits()
                )
                .as_bytes(),
            );
            for (label, colour) in [
                (b" fg=".as_slice(), cell.fg),
                (b" bg=".as_slice(), cell.bg),
                (b" us=".as_slice(), cell.us),
            ] {
                output.extend_from_slice(label);
                write_colour(colour, &mut output);
                output.extend_from_slice(format!("[{:x}]", colour.0).as_bytes());
            }
            let link = screen
                .hyperlinks
                .as_ref()
                .and_then(|store| server.hyperlinks.get(store, cell.link));
            output.extend_from_slice(b" link=");
            output.extend_from_slice(link.map_or(b"NONE", |link| link.uri()));
            output.extend_from_slice(b" linkid=");
            output.extend_from_slice(
                link.map(|link| link.internal_id())
                    .filter(|id| !id.is_empty())
                    .unwrap_or(b"NONE"),
            );
            output.push(b'\n');
        }
    }
    output
}

#[cfg(test)]
mod tests {
    #[test]
    fn raw_empty_screen_records() {
        let mut server = crate::server::Server::default();
        let screen =
            rmux_emu::screen::Screen::new(1, 1, 20, Default::default(), &mut server.hyperlinks)
                .unwrap();
        assert_eq!(super::raw_grid(&server, &screen), b"G 1x1 (0/20)\n\tL 0 (0) flags=NONE[0] 0/0\n\t\tC 0,0 data=(1,1, ) flags=NONE[0] attr=NONE[0] fg=default[8] bg=default[8] us=default[8] link=NONE linkid=NONE\n");
    }
}
