// Ported from tmux format.c, window-buffer.c, window-client.c, window-tree.c @ 8f25579c
/*
 * Copyright (c) 2011 Nicholas Marriott <nicholas.marriott@gmail.com>
 *
 * Permission to use, copy, modify, and distribute this software for any
 * purpose with or without fee is hereby granted, provided that the above
 * copyright notice and this permission notice appear in all copies.
 *
 * THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
 * WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
 * MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
 * ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
 * WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION
 * OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN
 * CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
 */

use std::collections::BTreeMap;
use std::sync::LazyLock;
use std::time::Duration;

use crate::client::{self, ClientFlags};
use crate::cmd::{find, queue};
use crate::format::jobs::{FormatCycleToken, FormatJobLaunch, FormatJobRuntime, FormatJobToken};
use crate::format::runtime::FormatAction;
use crate::format::sort::SortClients;
use crate::format::variables::{
    self, ClientFacts, ClientLinkFacts, MouseBacking, MouseFacts, ServerFacts,
};
use crate::format::{FormatContext, FormatFlags, FormatRuntime, FormatValue};
use crate::ids::{ClientId, JobId, QueueItemId};
use crate::model::Server;
use crate::options::environment::{Environment, TMUX_VERSION};
use rmux_emu::colour::ClientTheme;
use rmux_tty::term::{TtyCodeCode, TtyTermFlags};
use rmux_tty::tty::TtyFlags;
use rmux_util::buffer::ByteBuffer;
use rmux_util::bytes::{ByteString, cstr};
use rmux_util::time::Timestamp;

use super::event_loop::LoopAction;
use super::job::{self, JobCommand, JobLaunch};

#[derive(Default)]
pub struct FormatLiveState {
    tokens: BTreeMap<JobId, FormatJobToken>,
    cycles: BTreeMap<ClientId, FormatCycleToken>,
}

struct LiveJobs<'a>(&'a mut Server);

impl FormatJobRuntime for LiveJobs<'_> {
    fn run(&mut self, launch: FormatJobLaunch<'_>) -> Option<JobId> {
        let cwd = client::registry::get_cwd(self.0, launch.owner, None);
        let id = job::run(
            self.0,
            JobLaunch {
                command: JobCommand::Shell(launch.command.to_vec()),
                environment: Environment::new(),
                cwd: Some(cwd),
                session: None,
                flags: launch.flags,
                size: (u32::MAX, u32::MAX),
                update: Some(job_update),
                complete: Some(Box::new(job_complete)),
                free: None,
            },
        )
        .ok()?;
        self.0.format_live.tokens.insert(id, launch.callback);
        Some(id)
    }

    fn cancel(&mut self, id: JobId) {
        self.0.format_live.tokens.remove(&id);
        let _ = job::free(self.0, id);
    }

    fn status(&mut self, owner: ClientId) {
        super::operations::server_status_client(self.0, owner);
    }

    fn cycle_start(&mut self, token: FormatCycleToken, delay: Duration) {
        if let Some(client) = self.0.clients.get_mut(token.owner) {
            if let Some(timer) = client.cycle_timer.take() {
                self.0.event_loop.cancel(timer);
            }
            client.cycle_timer = Some(
                self.0
                    .event_loop
                    .schedule(delay, LoopAction::ClientCycleTimer(token.owner)),
            );
            self.0.format_live.cycles.insert(token.owner, token);
        }
    }

    fn cycle_cancel(&mut self, token: FormatCycleToken) {
        if self.0.format_live.cycles.get(&token.owner) != Some(&token) {
            return;
        }
        self.0.format_live.cycles.remove(&token.owner);
        if let Some(timer) = self
            .0
            .clients
            .get_mut(token.owner)
            .and_then(|client| client.cycle_timer.take())
        {
            self.0.event_loop.cancel(timer);
        }
    }

    fn redraw_status(&mut self, owner: ClientId) {
        if let Some(client) = self.0.clients.get_mut(owner) {
            client.flags.insert(ClientFlags::REDRAWSTATUS);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn job(
    server: &mut Server,
    owner: Option<ClientId>,
    tag: u32,
    flags: FormatFlags,
    raw: &[u8],
    expanded: &[u8],
    now: i64,
) -> ByteString {
    let mut jobs = std::mem::take(&mut server.format_jobs);
    let output = jobs.get(&mut LiveJobs(server), owner, tag, flags, raw, expanded, now);
    server.format_jobs = jobs;
    output
}

fn job_update(server: &mut Server, id: JobId) {
    let Some(token) = server.format_live.tokens.remove(&id) else {
        return;
    };
    let Some(job) = server.jobs.get_mut(id) else {
        return;
    };
    let mut input = ByteBuffer::from_vec(job.take_input());
    let mut jobs = std::mem::take(&mut server.format_jobs);
    let now = server.current_time.0;
    jobs.update(&mut LiveJobs(server), &token, &mut input, now);
    server.format_jobs = jobs;
    if let Some(job) = server.jobs.get_mut(id) {
        job.io.restore_input(input.into_vec());
        server.format_live.tokens.insert(id, token);
    }
}

fn job_complete(server: &mut Server, id: JobId) {
    let Some(token) = server.format_live.tokens.remove(&id) else {
        return;
    };
    let Some(job) = server.jobs.get_mut(id) else {
        return;
    };
    let mut input = ByteBuffer::from_vec(job.take_input());
    let mut jobs = std::mem::take(&mut server.format_jobs);
    jobs.complete(&mut LiveJobs(server), &token, &mut input);
    server.format_jobs = jobs;
    if let Some(job) = server.jobs.get_mut(id) {
        job.io.restore_input(input.into_vec());
    }
}

pub fn cycle(server: &mut Server, owner: ClientId) {
    if server.clients.get(owner).is_none() {
        return;
    }
    let mut jobs = std::mem::take(&mut server.format_jobs);
    jobs.cycle_start(&mut LiveJobs(server), owner);
    server.format_jobs = jobs;
}

pub fn cycle_timer(server: &mut Server, owner: ClientId) {
    let Some(token) = server.format_live.cycles.remove(&owner) else {
        return;
    };
    let Some(client) = server.clients.get_mut(owner) else {
        return;
    };
    if let Some(timer) = client.cycle_timer.take() {
        server.event_loop.cancel(timer);
    }
    let message = client.message.text.is_some();
    let prompt = client.prompt.is_some();
    let mut jobs = std::mem::take(&mut server.format_jobs);
    jobs.cycle_complete(&mut LiveJobs(server), token, message, prompt);
    server.format_jobs = jobs;
}

pub fn format_lost_client(server: &mut Server, owner: ClientId) {
    let mut jobs = std::mem::take(&mut server.format_jobs);
    jobs.lost_client(&mut LiveJobs(server), owner);
    server.format_jobs = jobs;
}

pub fn tidy(server: &mut Server) {
    let now = server.current_time.0;
    let mut jobs = std::mem::take(&mut server.format_jobs);
    jobs.tidy_owner(&mut LiveJobs(server), None, now, false);
    for index in 0..server.client_order.len() {
        let owner = server.client_order[index];
        jobs.tidy_owner(&mut LiveJobs(server), Some(owner), now, false);
    }
    server.format_jobs = jobs;
}

pub fn apply_action(server: &mut Server, action: FormatAction) -> Option<ByteString> {
    match action {
        FormatAction::RetainClient(id) => FormatRuntime::retain_client(server, id),
        FormatAction::ReleaseClient(id) => FormatRuntime::release_client(server, id),
        FormatAction::Cycle(id) => cycle(server, id),
        FormatAction::Job {
            owner,
            tag,
            flags,
            raw,
            expanded,
            now,
        } => {
            return Some(job(server, owner, tag, flags, &raw, &expanded, now));
        }
        FormatAction::Log {
            item,
            depth,
            message,
            verbose,
        } => log(server, item, depth, &message, verbose),
        FormatAction::ParsePrint { message, input } => {
            crate::cmd::parse::ParseContext::print(server, &message, &input)
        }
    }
    None
}

pub fn log(
    server: &mut Server,
    item: Option<QueueItemId>,
    depth: u32,
    message: &[u8],
    verbose: bool,
) {
    rmux_util::log::write(
        "",
        format_args!("format: {}", String::from_utf8_lossy(cstr(message))),
    );
    if verbose && let Some(item) = item {
        let mut output = ByteString::with_capacity(1 + (depth as usize).min(10) + message.len());
        output.push(b'#');
        output.extend_from_slice(&b"          "[..(depth as usize).min(10)]);
        output.extend_from_slice(cstr(message));
        queue::print(server, item, &output);
    }
}

impl SortClients for Server {
    fn clients(&self, out: &mut Vec<ClientId>) {
        out.extend(
            self.client_order
                .iter()
                .copied()
                .filter(|id| self.clients.get(*id).is_some()),
        );
    }
    fn client_sortable(&self, id: ClientId) -> bool {
        self.clients.get(id).is_some_and(|client| {
            !client.flags.intersects(ClientFlags::UNATTACHEDFLAGS)
                && client.flags.contains(ClientFlags::ATTACHED)
        })
    }
    fn client_name(&self, id: ClientId) -> &[u8] {
        self.clients
            .get(id)
            .map_or(b"", |client| client.name_bytes())
    }
    fn client_size(&self, id: ClientId) -> (u32, u32) {
        self.clients
            .get(id)
            .map_or((0, 0), |client| client.tty_size())
    }
    fn client_created(&self, id: ClientId) -> (i64, i64) {
        self.clients
            .get(id)
            .map_or((0, 0), |client| client.creation_time)
    }
    fn client_activity(&self, id: ClientId) -> (i64, i64) {
        self.clients
            .get(id)
            .map_or((0, 0), |client| client.activity_time)
    }
}

pub fn client_query(
    server: &Server,
    context: &FormatContext,
    kind: u8,
    key: &[u8],
) -> Option<ByteString> {
    let client = server.clients.get(context.evaluated_client?)?;
    let tty = client
        .tty
        .as_ref()
        .filter(|tty| tty.flags().contains(TtyFlags::STARTED))?;
    if kind == b'e' {
        return client.environ.find(cstr(key))?.value.clone();
    }
    let key = std::str::from_utf8(cstr(key)).ok()?;
    let present = match kind {
        b'c' => tty.term().has_name(key),
        b'f' => tty
            .term()
            .feature_present(key, client.flags.contains(ClientFlags::UTF8)),
        _ => return None,
    };
    Some(
        if present {
            b"1".as_slice()
        } else {
            b"0".as_slice()
        }
        .into(),
    )
}

fn client_value(server: &mut Server, context: &FormatContext, key: &[u8]) -> Option<FormatValue> {
    let id = context.evaluated_client?;
    if key == b"client_user" {
        let client = server.clients.get(id)?;
        if client.user.is_none() {
            let user = client
                .peer
                .and_then(|peer| server.process.peer_uid(peer))
                .and_then(rmux_sys::server::user_name);
            server.clients.get_mut(id)?.user = user;
        }
    }
    let default_table = if matches!(key, b"client_key_table" | b"client_prefix") {
        client::keys::get_key_table(server, id)
    } else {
        Vec::new()
    };
    let features = if key == b"client_termfeatures" {
        rmux_tty::features::feature_names(server.clients.get(id)?.term_features.enabled)
    } else {
        String::new()
    };
    let client = server.clients.get(id)?;
    let (width, height) = client.tty_size();
    let tty = client.tty.as_ref();
    let started = tty.is_some_and(|tty| tty.flags().contains(TtyFlags::STARTED));
    let (xpixel, ypixel) = tty.map_or((0, 0), |tty| tty.pixel_size());
    let term = tty
        .filter(|tty| tty.flags().contains(TtyFlags::OPENED))
        .map(|tty| tty.term());
    let viewport = tty.and_then(|tty| {
        let (bigger, x, y, width, height) = tty.window_offset();
        bigger.then_some((x, y, width, height))
    });
    variables::client_value(
        &ClientFacts {
            flags: client.flags,
            started,
            width,
            height,
            xpixel,
            ypixel,
            term_rgb: term.is_some_and(|term| term.flags().contains(TtyTermFlags::RGBCOLOURS)),
            term_256: term.is_some_and(|term| term.flags().contains(TtyTermFlags::_256COLOURS)),
            term_colours: term.map_or(0, |term| term.number(TtyCodeCode::Colors) as u32),
            pid: client.pid.map_or(0, |pid| pid.0),
            uid: client
                .peer
                .and_then(|peer| server.process.peer_uid(peer))
                .map(|uid| uid.0),
            user: client.user.as_deref(),
            name: client.name.as_deref(),
            termname: client.term_name.as_deref(),
            termtype: client.term_type.as_deref(),
            tty: client.ttyname.as_deref(),
            key_table: client
                .keytable
                .and_then(|id| server.key_bindings.tables.get(id))
                .map_or(b"".as_slice(), |table| table.name.as_ref()),
            default_key_table: &default_table,
            features: features.as_bytes(),
            session: client
                .session
                .and_then(|id| server.sessions.get(id))
                .map(|session| session.name.as_slice()),
            last_session: client
                .last_session
                .filter(|id| crate::model::session::session_alive(server, *id))
                .and_then(|id| server.sessions.get(id))
                .map(|session| session.name.as_slice()),
            created: Timestamp::new(client.creation_time.0, client.creation_time.1 as i32),
            activity: Timestamp::new(client.activity_time.0, client.activity_time.1 as i32),
            discarded: client.discarded as u64,
            written: client.written as u64,
            pause_age_ms: client.pause_age,
            theme: match client.theme {
                ClientTheme::Unknown => None,
                ClientTheme::Dark => Some(b"dark"),
                ClientTheme::Light => Some(b"light"),
            },
            viewport,
        },
        key,
    )
}

const BUFFER_MODE_FORMAT: &[u8] = b"#{t/p:buffer_created}: #{buffer_sample}";
const CLIENT_MODE_FORMAT: &[u8] =
    b"#[fg=themelightgrey]#{t/p:client_activity}: session #[default]#{session_name}";
const TREE_MODE_FORMAT: &[u8] = concat!(
    "#{?pane_format,",
    "#{?pane_marked,#[fg=thememagenta],}#{?pane_floating_flag,#[underscore],}",
    "#{pane_current_command}#[fg=themelightgrey]#{pane_flags}",
    "#{?#{&&:#{pane_title},#{!=:#{pane_title},#{host_short}}},: \"#{pane_title}\",}",
    ",window_format,",
    "#{?window_marked_flag,#[fg=thememagenta],}",
    "#{window_name}#[fg=themelightgrey]#{window_flags}",
    "#{?#{&&:#{==:#{window_panes},1},#{&&:#{pane_title},#{!=:#{pane_title},#{host_short}}}},: \"#{pane_title}\",}",
    ",#[fg=themelightgrey]#{session_windows} windows",
    "#{?session_grouped, (group #{session_group}: #{session_group_list}),}",
    "#{?session_attached, (attached),}",
    "}"
).as_bytes();

fn server_value(server: &Server, key: &[u8]) -> Option<FormatValue> {
    static HOST: LazyLock<Option<Vec<u8>>> = LazyLock::new(|| rmux_sys::osdep::hostname().ok());
    static USER: LazyLock<Option<Vec<u8>>> =
        LazyLock::new(|| rmux_sys::server::user_name(rmux_sys::proc::getuid()));
    if matches!(key, b"host" | b"host_short") && HOST.is_none() {
        return None;
    }
    variables::server_value(
        &ServerFacts {
            hostname: HOST.as_deref().unwrap_or_default(),
            socket: &server.socket_path,
            config_files: &server.cfg.files,
            next_session_id: server.next_session_id,
            sessions: server.sessions.len() as u32,
            start: Timestamp::new(server.start_time.0, server.start_time.1 as i32),
            uid: rmux_sys::proc::getuid().0,
            user: USER.as_deref(),
            version: TMUX_VERSION,
            buffer_mode_format: BUFFER_MODE_FORMAT,
            client_mode_format: CLIENT_MODE_FORMAT,
            tree_mode_format: TREE_MODE_FORMAT,
        },
        key,
    )
}

fn mouse_value(server: &Server, context: &FormatContext, key: &[u8]) -> Option<FormatValue> {
    let mouse = context.mouse.as_ref()?;
    let pane = find::mouse_pane(server, mouse).and_then(|(_, _, pane)| server.panes.get(pane));
    let coordinates = pane.and_then(|pane| {
        find::mouse_at(
            find::PaneGeometry {
                x: pane.xoff,
                y: pane.yoff,
                width: pane.sx,
                height: pane.sy,
            },
            mouse,
            false,
        )
    });
    let client = context
        .evaluated_client
        .and_then(|id| server.clients.get(id));
    let started = client
        .and_then(|client| client.tty.as_ref())
        .is_some_and(|tty| tty.flags().contains(TtyFlags::STARTED));
    let status_y = if mouse.status_at == 0 && mouse.y < mouse.status_lines {
        Some(mouse.y)
    } else if mouse.status_at > 0 && mouse.y >= mouse.status_at as u32 {
        Some(mouse.y - mouse.status_at as u32)
    } else {
        None
    };
    variables::mouse_value(
        &MouseFacts {
            valid: mouse.valid,
            pane_public_id: pane.map(|pane| pane.public_id),
            pane_coordinates: coordinates,
            x: mouse.x,
            y: mouse.y,
            status_at: mouse.status_at,
            status_lines: mouse.status_lines,
            client_started: started,
            status_range: client
                .and_then(|client| crate::ui::status::status_get_range(client, mouse.x, status_y?)),
            backing: match pane {
                Some(pane) if pane.modes.is_empty() => MouseBacking::Base {
                    grid: &pane.base.grid,
                    screen: &pane.base,
                    registry: &server.hyperlinks,
                },
                _ => MouseBacking::Unsupported,
            },
            word_separators: server
                .options
                .get_string(server.options.global_s, b"word-separators"),
        },
        key,
    )
}

pub fn builtin(server: &mut Server, context: &FormatContext, key: &[u8]) -> Option<FormatValue> {
    if (key.starts_with(b"client_") && key != b"client_mode_format")
        || matches!(
            key,
            b"window_bigger" | b"window_offset_x" | b"window_offset_y"
        )
    {
        return client_value(server, context, key);
    }
    if key.starts_with(b"mouse_") {
        return mouse_value(server, context, key);
    }
    if matches!(
        key,
        b"session_active"
            | b"session_attached_list"
            | b"session_group_attached_list"
            | b"window_active_clients_list"
            | b"window_active_clients"
    ) {
        let clients: Vec<_> = server
            .client_order
            .iter()
            .filter_map(|id| {
                server.clients.get(*id).map(|client| ClientLinkFacts {
                    id: *id,
                    session: client.session,
                    name: client.name_bytes(),
                })
            })
            .collect();
        return variables::client_model_value(server, context, &clients, key);
    }
    if matches!(key, b"pane_pipe" | b"pane_pipe_pid") {
        let pane = server.panes.get(context.pane?)?;
        return Some(if key == b"pane_pipe" {
            FormatValue::Unsigned(u64::from(pane.pipe.is_some()))
        } else {
            FormatValue::Signed(i64::from(pane.pipe.as_ref()?.pid.0))
        });
    }
    if matches!(key, b"pane_fg" | b"pane_bg") {
        let pane_id = context.pane?;
        server.panes.get(pane_id)?;
        let (colours, _) = crate::ui::fanout::tty_default_colours(server, pane_id);
        let mut value = Vec::new();
        rmux_emu::colour::write_colour(
            if key == b"pane_fg" {
                colours.fg
            } else {
                colours.bg
            },
            &mut value,
        );
        return Some(FormatValue::Bytes(ByteString(value)));
    }
    server_value(server, key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::Client;
    use crate::cmd::find::{CmdFindState, MouseInput};
    use crate::cmd::queue::{CmdReturn, callback_for};
    use crate::format::FormatTree;
    use crate::model::session::{SessionCreate, session_create};
    use crate::options::environment::EnvironmentFlags;
    use std::time::Instant;

    fn client(server: &mut Server, name: &[u8]) -> ClientId {
        let mut client = Client::new(None, (12, 0));
        client.name = Some(name.to_vec());
        client.flags.insert(ClientFlags::ATTACHED);
        let id = server.clients.insert(client).unwrap();
        server.client_order.push_back(id);
        id
    }

    fn pump(server: &mut Server, id: JobId) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while server.jobs.get(id).is_some() {
            for ready in server
                .event_loop
                .poll(Some(Duration::from_millis(5)))
                .unwrap()
            {
                if let LoopAction::Job(id) = ready.action {
                    job::on_ready(server, id, ready.readable, ready.writable).unwrap();
                }
            }
            if let Some(pid) = server.jobs.get(id).and_then(|job| job.pid)
                && let Some(status) = rmux_sys::proc::wait_process(pid, true).unwrap()
            {
                job::check_died(server, pid, status).unwrap();
            }
            assert!(Instant::now() < deadline, "format job timed out");
        }
    }

    #[test]
    fn queue_target_owner_formats_and_mouse_are_live_snapshots() {
        let mut server = Server::new();
        let owner = client(&mut server, b"owner");
        let evaluated = client(&mut server, b"evaluated");
        let options = server.options.create(Some(server.options.global_s));
        let session = session_create(
            &mut server,
            SessionCreate {
                prefix: None,
                name: Some(b"live".to_vec()),
                cwd: b"/".to_vec(),
                environment: Environment::new(),
                options,
                termios: None,
            },
        );
        server.clients.get_mut(evaluated).unwrap().session = Some(session);
        server.clients.get_mut(evaluated).unwrap().theme = ClientTheme::Dark;
        let batch = server
            .queue
            .get_callback(
                "format-test",
                callback_for::<Server>(|_, _| CmdReturn::Normal),
            )
            .unwrap();
        let item = batch.items[0];
        let queued = server.queue.items.get_mut(item).unwrap();
        queued.client = Some(owner);
        queued.target_client = Some(evaluated);
        queued.target = CmdFindState {
            s: Some(session),
            ..CmdFindState::default()
        };
        let state = queued.state;
        let mouse = MouseInput {
            valid: true,
            x: 7,
            y: 1,
            status_at: 0,
            status_lines: 2,
            ..MouseInput::default()
        };
        server.queue.states.get_mut(state).unwrap().event.mouse = mouse;
        server
            .queue
            .add_format(state, b"queue_field", b"current")
            .unwrap();
        let mut tree = crate::format::create_from_target(&mut server, item);
        assert_eq!(tree.owner, Some(owner));
        assert_eq!(tree.context.evaluated_client, Some(evaluated));
        assert_eq!(tree.context.mouse, Some(mouse));
        assert_eq!(
            &*tree.expand(
                &mut server,
                b"#{queue_field}:#{client_name}:#{client_session}:#{client_theme}:#{client_width}"
            ),
            b"current:evaluated:live:dark:80"
        );
        assert_eq!(FormatRuntime::owner_client(&server, item), Some(owner));
        tree.release(&mut server);
        server.queue.discard_batch(batch).unwrap();
        assert!(
            !server
                .effects
                .iter()
                .any(|effect| matches!(effect, crate::model::ModelEffect::Format(_)))
        );
    }

    #[test]
    fn client_loops_sort_live_clients_and_use_owner_environment() {
        let mut server = Server::new();
        let z = client(&mut server, b"z");
        let a = client(&mut server, b"a");
        server
            .clients
            .get_mut(z)
            .unwrap()
            .environ
            .set(b"OWNER", EnvironmentFlags::HIDDEN, b"yes");
        server.clients.get_mut(a).unwrap().environ.set(
            b"OWNER",
            EnvironmentFlags::default(),
            b"no",
        );
        let context = FormatContext {
            evaluated_client: Some(a),
            ..FormatContext::default()
        };
        let entries = server.loop_entries(&context, Some(z), b'L', b"n").unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.context.evaluated_client)
                .collect::<Vec<_>>(),
            vec![Some(a), Some(z)]
        );
        server
            .clients
            .get_mut(a)
            .unwrap()
            .flags
            .insert(ClientFlags::SUSPENDED);
        assert_eq!(
            server
                .loop_entries(&context, Some(z), b'L', b"n")
                .unwrap()
                .len(),
            1
        );
        let entries = server.loop_entries(&context, Some(z), b'V', b"c").unwrap();
        assert_eq!(&*entries[0].fields[b"environ_value".as_slice()], b"yes");
        assert_eq!(&*entries[0].fields[b"environ_hidden".as_slice()], b"1");
        assert!(client_query(&server, &context, b'e', b"OWNER").is_none());
    }

    #[test]
    fn format_owner_lease_exists_before_event_loop_dispatch() {
        let mut server = Server::new();
        let owner = client(&mut server, b"retained");
        let tree = FormatTree::create(Some(owner), None, 0, FormatFlags::NONE, &mut server);
        assert!(server.clients.request_remove(owner).unwrap().is_none());
        assert!(server.clients.get(owner).is_some());
        assert!(server.effects.is_empty());
        tree.release(&mut server);
        assert!(server.clients.get(owner).is_none());
    }

    #[test]
    fn job_callbacks_keep_newest_line_and_complete_trailing_output() {
        let mut server = Server::new();
        server.cfg.finished = true;
        server.current_time = (10, 0);
        assert!(
            job(
                &mut server,
                None,
                3,
                FormatFlags::NONE,
                b"raw",
                b"printf 'first\\nsecond\\ntail'",
                10
            )
            .is_empty()
        );
        let id = server
            .format_jobs
            .record(None, 3, b"raw")
            .unwrap()
            .job
            .unwrap();
        pump(&mut server, id);
        assert_eq!(
            &**server
                .format_jobs
                .record(None, 3, b"raw")
                .unwrap()
                .out
                .as_ref()
                .unwrap(),
            b"tail"
        );
        assert!(server.format_live.tokens.is_empty());
        assert!(
            server
                .format_jobs
                .record(None, 3, b"raw")
                .unwrap()
                .job
                .is_none()
        );
        assert_eq!(
            &*job(
                &mut server,
                None,
                3,
                FormatFlags::NONE,
                b"raw",
                b"printf 'first\\nsecond\\ntail'",
                10
            ),
            b"tail"
        );
        assert!(server.jobs.is_empty());
        job(
            &mut server,
            None,
            4,
            FormatFlags::NONE,
            b"lines",
            b"printf 'older\\nnewest\\n'",
            10,
        );
        let id = server
            .format_jobs
            .record(None, 4, b"lines")
            .unwrap()
            .job
            .unwrap();
        pump(&mut server, id);
        assert_eq!(
            &**server
                .format_jobs
                .record(None, 4, b"lines")
                .unwrap()
                .out
                .as_ref()
                .unwrap(),
            b"newest"
        );
    }

    #[test]
    fn update_callback_consumes_only_complete_lines_and_marks_status() {
        let mut server = Server::new();
        server.cfg.finished = true;
        server.current_time = (12, 0);
        let owner = client(&mut server, b"status");
        job(
            &mut server,
            Some(owner),
            9,
            FormatFlags::STATUS,
            b"stream",
            b"printf 'old\\nnew\\npartial'; exec sleep 60",
            10,
        );
        let id = server
            .format_jobs
            .record(Some(owner), 9, b"stream")
            .unwrap()
            .job
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while server
            .format_jobs
            .record(Some(owner), 9, b"stream")
            .unwrap()
            .out
            .as_ref()
            .map(ByteString::as_bytes)
            != Some(b"new".as_slice())
            || server.jobs.get(id).unwrap().input() != b"partial"
        {
            for ready in server
                .event_loop
                .poll(Some(Duration::from_millis(5)))
                .unwrap()
            {
                if let LoopAction::Job(id) = ready.action {
                    job::on_ready(&mut server, id, ready.readable, ready.writable).unwrap();
                }
            }
            assert!(Instant::now() < deadline, "format update timed out");
        }
        assert_eq!(
            &**server
                .format_jobs
                .record(Some(owner), 9, b"stream")
                .unwrap()
                .out
                .as_ref()
                .unwrap(),
            b"new"
        );
        assert_eq!(server.jobs.get(id).unwrap().input(), b"partial");
        assert!(
            server
                .clients
                .get(owner)
                .unwrap()
                .flags
                .contains(ClientFlags::REDRAWSTATUS)
        );
        let pid = server.jobs.get(id).unwrap().pid.unwrap();
        format_lost_client(&mut server, owner);
        rmux_sys::proc::wait_process(pid, false).unwrap();
    }

    #[test]
    fn force_restart_tidy_and_lost_client_cancel_real_jobs_and_cycles() {
        let mut server = Server::new();
        server.cfg.finished = true;
        let owner = client(&mut server, b"owner");
        job(
            &mut server,
            Some(owner),
            1,
            FormatFlags::NONE,
            b"sleep",
            b"exec sleep 60",
            1,
        );
        let old = server
            .format_jobs
            .record(Some(owner), 1, b"sleep")
            .unwrap()
            .job
            .unwrap();
        let old_pid = server.jobs.get(old).unwrap().pid.unwrap();
        job(
            &mut server,
            Some(owner),
            1,
            FormatFlags::FORCE,
            b"sleep",
            b"exec sleep 60",
            1,
        );
        let fresh = server
            .format_jobs
            .record(Some(owner), 1, b"sleep")
            .unwrap()
            .job
            .unwrap();
        assert_ne!(old, fresh);
        assert!(server.jobs.get(old).is_none());
        assert!(!server.format_live.tokens.contains_key(&old));
        rmux_sys::proc::wait_process(old_pid, false).unwrap();
        let pid = server.jobs.get(fresh).unwrap().pid.unwrap();
        cycle(&mut server, owner);
        let timer = server.clients.get(owner).unwrap().cycle_timer;
        cycle(&mut server, owner);
        assert_eq!(server.clients.get(owner).unwrap().cycle_timer, timer);
        format_lost_client(&mut server, owner);
        assert!(server.jobs.is_empty());
        assert!(server.format_jobs.is_empty());
        assert!(server.format_live.tokens.is_empty());
        assert!(server.format_live.cycles.is_empty());
        assert!(server.clients.get(owner).unwrap().cycle_timer.is_none());
        rmux_sys::proc::wait_process(pid, false).unwrap();
        job(
            &mut server,
            None,
            2,
            FormatFlags::NONE,
            b"old",
            b"exec sleep 60",
            1,
        );
        let id = server
            .format_jobs
            .record(None, 2, b"old")
            .unwrap()
            .job
            .unwrap();
        let pid = server.jobs.get(id).unwrap().pid.unwrap();
        server.current_time = (3601, 0);
        tidy(&mut server);
        assert!(server.format_jobs.is_empty());
        assert!(server.jobs.is_empty());
        rmux_sys::proc::wait_process(pid, false).unwrap();
    }

    #[test]
    fn cycle_redraw_is_one_shot_and_suppressed_by_status_message() {
        let mut server = Server::new();
        let owner = client(&mut server, b"animated");
        cycle(&mut server, owner);
        cycle_timer(&mut server, owner);
        assert!(!server.format_jobs.cycle_pending(owner));
        assert!(
            server
                .clients
                .get(owner)
                .unwrap()
                .flags
                .contains(ClientFlags::REDRAWSTATUS)
        );
        server
            .clients
            .get_mut(owner)
            .unwrap()
            .flags
            .remove(ClientFlags::REDRAWSTATUS);
        cycle_timer(&mut server, owner);
        assert!(
            !server
                .clients
                .get(owner)
                .unwrap()
                .flags
                .contains(ClientFlags::REDRAWSTATUS)
        );
        server.clients.get_mut(owner).unwrap().message.text = Some("message".into());
        cycle(&mut server, owner);
        cycle_timer(&mut server, owner);
        assert!(
            !server
                .clients
                .get(owner)
                .unwrap()
                .flags
                .contains(ClientFlags::REDRAWSTATUS)
        );
    }

    #[test]
    fn terminal_queries_and_started_client_facts_use_real_terminal() {
        use rmux_tty::tty::{Tty, TtyHostInfo, TtyOptions};
        use std::os::fd::AsFd;
        let mut server = Server::new();
        let id = client(&mut server, b"terminal");
        let (_master, slave, _) = rmux_sys::pty::openpty().unwrap();
        let termios = rmux_sys::TermiosState::get(slave.as_fd()).unwrap();
        let mut tty = Tty::new(slave, termios, TtyHostInfo::default());
        tty.set_size(90, 30, 8, 16);
        let caps = ["clear=C", "cup=U", "colors=16", "bel=B"]
            .map(ByteString::from)
            .to_vec();
        tty.open(
            &mut server.tparm,
            b"format-fixture",
            &caps,
            &TtyOptions::default(),
            None,
        )
        .unwrap();
        server.clients.get_mut(id).unwrap().tty = Some(tty);
        server
            .clients
            .get_mut(id)
            .unwrap()
            .flags
            .insert(ClientFlags::UTF8);
        server.clients.get_mut(id).unwrap().environ.set(
            b"QUERY",
            EnvironmentFlags::default(),
            b"value",
        );
        let context = FormatContext {
            evaluated_client: Some(id),
            ..FormatContext::default()
        };
        assert_eq!(
            &*client_query(&server, &context, b'c', b"bel").unwrap(),
            b"1"
        );
        assert_eq!(
            &*client_query(&server, &context, b'c', b"unknown").unwrap(),
            b"0"
        );
        assert_eq!(
            &*client_query(&server, &context, b'f', b"utf8").unwrap(),
            b"1"
        );
        assert_eq!(
            &*client_query(&server, &context, b'e', b"QUERY").unwrap(),
            b"value"
        );
        assert_eq!(
            &*builtin(&mut server, &context, b"client_height")
                .unwrap()
                .bytes(),
            b"30"
        );
        assert_eq!(
            &*builtin(&mut server, &context, b"client_cell_width")
                .unwrap()
                .bytes(),
            b"8"
        );
        assert_eq!(
            &*builtin(&mut server, &context, b"client_colours")
                .unwrap()
                .bytes(),
            b"16"
        );
        server
            .clients
            .get_mut(id)
            .unwrap()
            .tty
            .as_mut()
            .unwrap()
            .flags_mut()
            .remove(TtyFlags::STARTED);
        assert!(client_query(&server, &context, b'c', b"bel").is_none());
        assert!(builtin(&mut server, &context, b"client_height").is_none());
        assert_eq!(
            &*builtin(&mut server, &context, b"client_width")
                .unwrap()
                .bytes(),
            b"90"
        );
    }

    #[test]
    fn mouse_pane_coordinates_and_base_line_use_captured_event() {
        use crate::model::session::session_attach;
        use crate::model::window::{window_add_pane, window_create, window_set_active_pane};
        let mut server = Server::new();
        let options = server.options.create(Some(server.options.global_s));
        let session = session_create(
            &mut server,
            SessionCreate {
                prefix: None,
                name: Some(b"mouse".to_vec()),
                cwd: b"/".to_vec(),
                environment: Environment::new(),
                options,
                termios: None,
            },
        );
        let window = window_create(&mut server, 20, 5, 0, 0).unwrap();
        let pane = window_add_pane(
            &mut server,
            window,
            None,
            10,
            crate::model::spawn::SpawnFlags::default(),
        )
        .unwrap();
        window_set_active_pane(&mut server, window, pane, false).unwrap();
        session_attach(&mut server, session, window, 0).unwrap();
        let mut cell = rmux_emu::cell::DEFAULT_CELL;
        cell.data = rmux_util::utf8::Utf8Data::set(b'X');
        server
            .panes
            .get_mut(pane)
            .unwrap()
            .base
            .grid
            .set_cell(0, 1, &cell);
        let context = FormatContext {
            mouse: Some(MouseInput {
                valid: true,
                session: Some(session),
                window: Some(window),
                pane: Some(pane),
                x: 2,
                y: 3,
                status_at: 0,
                status_lines: 2,
                ..MouseInput::default()
            }),
            ..FormatContext::default()
        };
        assert_eq!(
            &*builtin(&mut server, &context, b"mouse_x").unwrap().bytes(),
            b"2"
        );
        assert_eq!(
            &*builtin(&mut server, &context, b"mouse_y").unwrap().bytes(),
            b"1"
        );
        assert_eq!(
            &*builtin(&mut server, &context, b"mouse_pane")
                .unwrap()
                .bytes(),
            format!("%{}", server.panes.get(pane).unwrap().public_id).as_bytes()
        );
        assert!(builtin(&mut server, &context, b"mouse_line").is_some());
        assert!(builtin(&mut server, &context, b"mouse_status_line").is_none());
    }
}
