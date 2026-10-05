// Ported from tmux cmd-respawn-window.c @ 8f25579c
use super::split_window::spawn_cause;
use super::support::{concat, fail, item_target, item_target_client};
use crate::cmd::{Command, queue::CmdReturn};
use crate::ids::QueueItemId;
use crate::model::spawn::{self, SpawnContext, SpawnFlags};
use crate::options::environment::EnvironmentFlags;
use crate::server::{Server, operations};

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let target = item_target(server, item);
    let (Some(s), Some(wl)) = (target.s, target.wl) else {
        return fail(server, item, b"no current window");
    };

    let mut sc = SpawnContext::new(s);
    sc.item = Some(item);
    sc.winlink = Some(wl);
    sc.client = item_target_client(server, item);
    sc.argv = args
        .values()
        .iter()
        .map(|value| value.as_string().to_vec())
        .collect();
    for value in args.values_of(b'e') {
        sc.environment
            .put(value.as_string(), EnvironmentFlags::default());
    }
    sc.index = -1;
    sc.cwd = args.get(b'c').map(<[u8]>::to_vec);
    sc.flags = SpawnFlags::RESPAWN;
    if args.has(b'E') != 0 {
        sc.flags.insert(SpawnFlags::EMPTY);
    }
    if args.has(b'k') != 0 {
        sc.flags.insert(SpawnFlags::KILL);
    }

    if let Err(cause) = spawn::spawn_window(server, &mut sc) {
        return fail(
            server,
            item,
            concat(&[b"respawn window failed: ", &spawn_cause(&cause)]),
        );
    }

    let Some(window) = server.winlinks.get(wl).map(|link| link.window) else {
        return CmdReturn::Error;
    };
    operations::server_redraw_window(server, window);
    CmdReturn::Normal
}
