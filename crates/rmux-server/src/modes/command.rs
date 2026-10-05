// Ported from tmux cmd-choose-tree.c, cmd-copy-mode.c @ 8f25579c
use crate::{
    cmd::{
        arguments::Args,
        find::CmdFindState,
        queue::{self, CmdReturn},
    },
    ids::{ClientId, QueueItemId},
    server::Server,
};
use rmux_util::bytes::ByteString;

pub struct CommandModeRequest {
    pub command: ByteString,
    pub args: Args,
    pub target: CmdFindState,
    pub source: CmdFindState,
    pub client: Option<ClientId>,
    pub item: QueueItemId,
}

pub fn run_mode_command(server: &mut Server, request: CommandModeRequest) -> CmdReturn {
    use super::{WindowModeFlags, buffer, client, clock, customize, panes, switch, window_tree};
    use crate::model::pane::{PaneModeDriver, pane_set_mode};
    use std::rc::Rc;
    let Some(wp) = request.target.wp else {
        return CmdReturn::Normal;
    };
    if request.command.as_slice() != b"copy-mode"
        && !crate::tsp::broker::pane_cell_ready(server, wp)
    {
        return defer_mode(server, wp, request);
    }
    let kill = request.args.has(b'k') != 0;
    let mut flags = WindowModeFlags::default();
    let (name, driver): (&[u8], Rc<dyn PaneModeDriver>) = match request.command.as_slice() {
        b"choose-tree" | b"find-window" => (
            b"tree-mode",
            Rc::new(window_tree::WindowTreeDriver {
                args: request.args,
                target: request.target,
            }),
        ),
        b"choose-buffer" => (
            buffer::NAME,
            Rc::new(buffer::BufferMode::new(request.args, request.target)),
        ),
        b"choose-client" => (client::NAME, Rc::new(client::ClientMode::new(request.args))),
        b"customize-mode" => (
            customize::NAME,
            Rc::new(customize::CustomizeMode::new(request.args, request.target)),
        ),
        b"display-panes" => {
            flags = panes::FLAGS;
            (
                panes::NAME,
                Rc::new(panes::PanesMode::new(
                    request.args,
                    request.item,
                    request.target,
                    request.source,
                )),
            )
        }
        b"switch-mode" => (
            switch::NAME,
            Rc::new(switch::SwitchMode::new(request.args, request.target)),
        ),
        b"clock-mode" => (clock::NAME, Rc::new(clock::ClockMode)),
        b"copy-mode" => return enter_copy(server, request),
        _ => {
            queue::error(server, request.item, b"unknown mode command");
            return CmdReturn::Error;
        }
    };
    if pane_set_mode(server, wp, name, flags, driver, kill).is_err() {
        return CmdReturn::Error;
    }
    CmdReturn::Normal
}
fn defer_mode(
    server: &mut Server,
    pane: crate::ids::PaneId,
    request: CommandModeRequest,
) -> CmdReturn {
    let item = request.item;
    crate::tsp::broker::defer_cell_ui(
        server,
        pane,
        Box::new(move |server| {
            let failed = server
                .panes
                .get(pane)
                .and_then(|pane| pane.tsp.as_ref())
                .and_then(|state| state.switch.as_ref())
                .is_some_and(|switch| switch.failed);
            if !failed
                && server.panes.get(pane).is_some()
                && server.queue.items.get(item).is_some()
                && run_mode_command(server, request) == CmdReturn::Wait
            {
                return;
            }
            queue::continue_item(&mut server.queue, item);
        }),
    );
    CmdReturn::Wait
}

fn enter_copy(server: &mut Server, request: CommandModeRequest) -> CmdReturn {
    use crate::model::pane::pane_set_mode;
    use std::rc::Rc;
    let Some(mut pane) = request.target.wp else {
        return CmdReturn::Normal;
    };
    let event = crate::cmd::commands::support::item_event(server, request.item);
    if request.args.has(b'M') != 0 {
        let Some((session, _, mouse_pane)) = crate::cmd::find::mouse_pane(server, &event.mouse)
        else {
            return CmdReturn::Normal;
        };
        if request
            .client
            .and_then(|c| server.clients.get(c))
            .and_then(|c| c.session)
            != Some(session)
        {
            return CmdReturn::Normal;
        }
        pane = mouse_pane;
    }
    let source = if request.args.has(b's') != 0 {
        request.source.wp
    } else {
        Some(pane)
    };
    if let Some(wait_for) = [Some(pane), source]
        .into_iter()
        .flatten()
        .find(|pane| !crate::tsp::broker::pane_cell_ready(server, *pane))
    {
        return defer_mode(server, wait_for, request);
    }
    let driver = Rc::new(super::copy::CopyModeDriver {
        kind: super::copy::CopyModeKind::Copy {
            source,
            args: request.args.clone(),
        },
    });
    let entered = match pane_set_mode(
        server,
        pane,
        b"copy-mode",
        super::WindowModeFlags::default(),
        driver,
        request.args.has(b'k') != 0,
    ) {
        Ok(mode) => mode.is_some(),
        Err(_) => return CmdReturn::Error,
    };
    super::copy::set_line_numbers(server, pane, !event.key.is_mouse());
    if entered && request.args.has(b'M') != 0 {
        super::copy::mouse::start_drag(server, request.client, &event.mouse);
    }
    if request.args.has(b'u') != 0 {
        super::copy::motion::page_up(server, pane, false);
    }
    if request.args.has(b'd') != 0 {
        super::copy::motion::page_down(server, pane, false, request.args.has(b'e') != 0);
    }
    if request.args.has(b'S') != 0 {
        let Some(client) = request.client.and_then(|c| server.clients.get(c)) else {
            return CmdReturn::Normal;
        };
        let grab = client.drag.slider_mpos.map_or(-1, |row| row as i32);
        let pan = event.mouse.offset_y;
        super::copy::motion::scrollbar_scroll(
            server,
            pane,
            grab,
            event.mouse.y,
            pan,
            request.args.has(b'e') != 0,
        );
    }
    CmdReturn::Normal
}
