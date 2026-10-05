// Ported from tmux cmd-rename-window.c @ 8f25579c
use super::support::{concat, fail, item_target};
use crate::cmd::{Command, queue::CmdReturn};
use crate::format;
use crate::ids::QueueItemId;
use crate::model::window;
use crate::server::Server;
use crate::server::operations::{server_redraw_window_borders, server_status_window};

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let target = item_target(server, item);
    let Some(w) = target.w.or_else(|| {
        target
            .wl
            .and_then(|wl| server.winlinks.get(wl))
            .map(|wl| wl.window)
    }) else {
        return fail(server, item, b"no current window");
    };
    let template = command.args.string(0).unwrap_or(b"");
    let name = format::single_from_target(server, item, template);
    // check_name is utf8_isvalid (tmux.c:362-367).
    if !rmux_util::utf8::is_valid(&name) {
        return fail(server, item, concat(&[b"invalid window name: ", &name]));
    }

    let _ = window::window_set_name(server, w, &name, false);
    let Some(oo) = server.windows.get(w).map(|w| w.options) else {
        return fail(server, item, b"no current window");
    };
    let mut options = std::mem::take(&mut server.options);
    options.set_number(oo, b"automatic-rename", 0, server);
    server.options = options;

    server_redraw_window_borders(server, w);
    server_status_window(server, w);

    CmdReturn::Normal
}
