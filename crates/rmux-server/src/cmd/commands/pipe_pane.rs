// Ported from tmux cmd-pipe-pane.c @ 8f25579c
use super::support::{concat, fail, item_target, item_target_client};
use crate::cmd::{Command, queue::CmdReturn};
use crate::format::{self, FormatContext};
use crate::ids::{EventToken, PaneId, QueueItemId};
use crate::model::pane::{self, PaneOffset};
use crate::model::pane_input::{pane_get_new_data, pane_update_used_data};
use crate::server::event_loop::LoopAction;
use crate::server::io::BufferedIo;
use crate::server::{Server, operations};
use rmux_sys::server::{PipeSpawnError, spawn_pipe_child};
use rmux_util::log_debug;
use std::os::fd::AsFd;

/// `wp->pipe_fd`, `pipe_event`, `pipe_pid` and `pipe_offset` (`tmux.h:1401-1404`), plus the
/// `-I`/`-O` directions enabled on the bufferevent (`cmd-pipe-pane.c:183-186`).
pub struct PipePaneState {
    pub io: BufferedIo,
    pub pid: rmux_sys::ProcessId,
    pub offset: PaneOffset,
    /// `-I`: the child's stdout is read and fed to the pane (`EV_READ`).
    pub input_enabled: bool,
    /// `-O`: pane output is written to the child's stdin (`EV_WRITE`).
    pub output_enabled: bool,
    pub token: Option<EventToken>,
}

/// `cmd-pipe-pane.c:102-109`: neither `-I` nor `-O` is `-O`.
fn directions(has_input: bool, has_output: bool) -> (bool, bool) {
    if has_input {
        (true, has_output)
    } else {
        (false, true)
    }
}

fn errno_text(prefix: &[u8], error: &std::io::Error) -> Vec<u8> {
    concat(&[
        prefix,
        &rmux_sys::strerror(error.raw_os_error().unwrap_or(libc::EINVAL)),
    ])
}

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let target = item_target(server, item);
    let tc = item_target_client(server, item);
    let Some(wp) = target.wp else {
        return fail(server, item, b"no current window");
    };

    if pane::pane_exited(server, wp) {
        return fail(server, item, b"target pane has exited");
    }

    let had_pipe = close(server, wp);
    if had_pipe && destroy_ready(server, wp) {
        let _ = operations::server_destroy_pane(server, wp, true);
        return CmdReturn::Normal;
    }

    let Some(template) = args.string(0).filter(|s| !s.is_empty()) else {
        return CmdReturn::Normal;
    };

    // -o only opens the new pipe when there was no previous one (toggle).
    if args.has(b'o') != 0 && had_pipe {
        return CmdReturn::Normal;
    }

    let (input, output) = directions(args.has(b'I') != 0, args.has(b'O') != 0);

    let mut tree = format::create_defaults(
        server,
        Some(item),
        FormatContext {
            evaluated_client: tc,
            session: target.s,
            winlink: target.wl,
            window: target.w,
            pane: Some(wp),
            ..FormatContext::default()
        },
    );
    let expanded = tree.expand_time(server, template);
    tree.release(server);

    let (fd, pid) = match spawn_pipe_child(&expanded, input, output) {
        Ok(child) => child,
        Err(PipeSpawnError::Socketpair(error)) => {
            return fail(server, item, errno_text(b"socketpair error: ", &error));
        }
        Err(PipeSpawnError::Fork(error)) => {
            return fail(server, item, errno_text(b"fork error: ", &error));
        }
    };

    let Some(offset) = server.panes.get(wp).map(|p| PaneOffset {
        used: p.parser_offset,
    }) else {
        return CmdReturn::Error;
    };
    let mut io = BufferedIo::new(fd);
    io.enable_write(output);
    io.enable_read(input);
    let (read, write) = io.interests();
    let token = match server
        .event_loop
        .register(io.fd(), read, write, LoopAction::PanePipe(wp))
    {
        Ok(token) => token,
        Err(_) => return fail(server, item, b"out of memory"),
    };
    if let Some(p) = server.panes.get_mut(wp) {
        p.pipe = Some(PipePaneState {
            io,
            pid,
            offset,
            input_enabled: input,
            output_enabled: output,
            token: Some(token),
        });
    }
    CmdReturn::Normal
}

/// `bufferevent_free` + `close(pipe_fd)` + `pipe_fd = -1` (`cmd-pipe-pane.c:78-81`,
/// `window.c:1610-1614`); true when a pipe was open.
pub fn close(server: &mut Server, pane: PaneId) -> bool {
    let Some(state) = server.panes.get_mut(pane).and_then(|p| p.pipe.take()) else {
        return false;
    };
    if let Some(token) = state.token {
        server.event_loop.deregister(token);
    }
    drop(state);
    true
}

/// `sc->wp0->pipe_offset.used = 0` on respawn (`spawn.c:342`).
pub fn reset_offset(server: &mut Server, pane: PaneId) {
    if let Some(state) = server.panes.get_mut(pane).and_then(|p| p.pipe.as_mut()) {
        state.offset.used = 0;
    }
}

/// `window_pane_destroy_ready` (`window.c:495-510`) with the pipe output check folded in.
/// `ioctl(FIONREAD)` failure counts as nothing pending, as in the C.
pub fn destroy_ready(server: &Server, pane: PaneId) -> bool {
    let Some(p) = server.panes.get(pane) else {
        return false;
    };
    let pipe_output_empty = p.pipe.as_ref().is_none_or(|s| s.io.output_len() == 0);
    let unread = p.fd.as_ref().map_or(0, |fd| {
        rmux_sys::server::pending_bytes(fd.as_fd()).unwrap_or(0)
    });
    pane::pane_destroy_ready(server, pane, pipe_output_empty, unread)
}

fn check_destroy(server: &mut Server, pane: PaneId) {
    if destroy_ready(server, pane) {
        let _ = operations::server_destroy_pane(server, pane, true);
    }
}

/// Re-arm the event loop after the buffered state changed.
fn sync_interest(server: &mut Server, pane: PaneId) {
    let Some((token, read, write)) = server
        .panes
        .get(pane)
        .and_then(|p| p.pipe.as_ref())
        .and_then(|s| s.token.map(|t| (t, s.io.interests())))
        .map(|(t, (r, w))| (t, r, w))
    else {
        return;
    };
    if let Err(error) = server.event_loop.reregister(token, read, write) {
        log_debug!("%{} pipe reregister: {error}", pane_public_id(server, pane));
    }
}

fn pane_public_id(server: &Server, pane: PaneId) -> u32 {
    server.panes.get(pane).map_or(0, |p| p.public_id)
}

/// Pipe side of `window_pane_read_callback` (`window.c:1652-1663`): queue the pane's new pty
/// output for the child and advance `pipe_offset`.
pub fn output(server: &mut Server, pane: PaneId) {
    let Some(mut offset) = server
        .panes
        .get(pane)
        .and_then(|p| p.pipe.as_ref())
        .filter(|s| s.output_enabled && !s.io.write_closed())
        .map(|s| s.offset)
    else {
        return;
    };
    let data = match pane_get_new_data(server, pane, &offset) {
        Ok(data) if !data.is_empty() => data.to_vec(),
        _ => return,
    };
    if pane_update_used_data(server, pane, &mut offset, data.len()).is_err() {
        return;
    }
    if let Some(state) = server.panes.get_mut(pane).and_then(|p| p.pipe.as_mut()) {
        state.io.queue(data);
        state.offset = offset;
    }
    sync_interest(server, pane);
}

/// `LoopAction::PanePipe` dispatch: the libevent read/write/error callbacks.
pub fn on_ready(server: &mut Server, pane: PaneId, readable: bool, writable: bool) {
    if readable {
        on_read(server, pane);
    }
    if writable {
        on_write(server, pane);
    }
}

/// `cmd_pipe_pane_read_callback` (`cmd-pipe-pane.c:193-212`): the child's output goes to the
/// pane's pty. Bytes for a pane whose process is gone are dropped rather than queued.
pub fn on_read(server: &mut Server, pane: PaneId) {
    let Some(p) = server.panes.get_mut(pane) else {
        return;
    };
    let Some(state) = p.pipe.as_mut() else {
        return;
    };
    let progress = state.io.read_ready();
    let data = state.io.take_input();
    log_debug!("%{} pipe read {}", p.public_id, data.len());
    if p.fd.is_some() && !data.is_empty() {
        p.output.extend_from_slice(&data);
    }
    match progress {
        Ok(progress) if !progress.eof => {}
        _ => {
            on_error(server, pane);
            return;
        }
    }
    sync_interest(server, pane);
    check_destroy(server, pane);
}

/// `cmd_pipe_pane_write_callback` (`cmd-pipe-pane.c:214-223`).
pub fn on_write(server: &mut Server, pane: PaneId) {
    let Some(p) = server.panes.get_mut(pane) else {
        return;
    };
    let Some(state) = p.pipe.as_mut() else {
        return;
    };
    match state.io.write_ready() {
        Ok(progress) => {
            if progress.drained {
                log_debug!("%{} pipe empty", p.public_id);
            }
        }
        Err(_) => {
            on_error(server, pane);
            return;
        }
    }
    sync_interest(server, pane);
    check_destroy(server, pane);
}

/// `cmd_pipe_pane_error_callback` (`cmd-pipe-pane.c:225-239`).
pub fn on_error(server: &mut Server, pane: PaneId) {
    if let Some(p) = server.panes.get(pane) {
        log_debug!("%{} pipe error", p.public_id);
    }
    close(server, pane);
    check_destroy(server, pane);
}

#[cfg(test)]
mod tests {
    use super::directions;

    #[test]
    fn directions_default_to_output_only() {
        assert_eq!(directions(false, false), (false, true));
        assert_eq!(directions(false, true), (false, true));
        assert_eq!(directions(true, false), (true, false));
        assert_eq!(directions(true, true), (true, true));
    }
}
