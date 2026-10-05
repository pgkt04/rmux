// Ported from tmux cmd-bind-key.c @ 8f25579c
use crate::cmd::{
    Command,
    arguments::ArgsValueData,
    parse::{self, CmdParseInput},
    queue::{self, CmdReturn},
};
use crate::ids::QueueItemId;
use crate::server::Server;
use rmux_tty::key_string::parse_key_name;
use rmux_util::key::SpecialKey;

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let name = args.string(0).unwrap_or_default();
    let key = parse_key_name(name);
    if key.0 == SpecialKey::NONE || key.0 == SpecialKey::UNKNOWN {
        let mut cause = b"unknown key: ".to_vec();
        cause.extend_from_slice(name);
        queue::error(server, item, &cause);
        return CmdReturn::Error;
    }
    let table = args.get(b'T').unwrap_or(if args.has(b'n') != 0 {
        b"root"
    } else {
        b"prefix"
    });
    let list = if args.count() == 1 {
        None
    } else if args.count() == 2 {
        match &args.value(1).expect("second binding argument").data {
            ArgsValueData::Commands(list) => Some(Ok(list.clone())),
            _ => Some(parse::from_string(
                server,
                args.string(1).unwrap_or_default(),
                &mut CmdParseInput::default(),
            )),
        }
    } else {
        Some(parse::from_arguments(
            server,
            &args.values()[1..],
            &mut CmdParseInput::default(),
        ))
    };
    let list = match list {
        Some(Ok(list)) => Some(list),
        Some(Err(cause)) => {
            queue::error(server, item, cause.to_string().as_bytes());
            return CmdReturn::Error;
        }
        None => None,
    };
    if let Err(cause) =
        server
            .key_bindings
            .add(table, key, args.get(b'N'), args.has(b'r') != 0, list)
    {
        queue::error(server, item, cause.to_string().as_bytes());
        return CmdReturn::Error;
    }
    CmdReturn::Normal
}
