// Ported from tmux cmd-show-prompt-history.c @ 8f25579c
use super::support::{concat, fail};
use crate::cmd::metadata::CMD_CLEAR_PROMPT_HISTORY;
use crate::cmd::{
    Command,
    queue::{self, CmdReturn},
};
use crate::ids::QueueItemId;
use crate::server::Server;
use crate::ui::prompt::{PromptType, history, prompt_type, prompt_type_string};

/// `for (t = 0; t < PROMPT_NTYPES; t++)`: every real prompt type in enum order.
fn all_types() -> impl Iterator<Item = PromptType> {
    (0i32..)
        .map(PromptType::try_from)
        .take_while(|t| t.is_ok_and(|t| t != PromptType::Invalid))
        .flatten()
}

/// cmd-show-prompt-history.c:76-82, 90-96: one type's history block.
fn show(server: &mut Server, item: QueueItemId, ty: PromptType) {
    let heading = concat(&[b"History for ", prompt_type_string(ty).as_bytes(), b":\n"]);
    queue::print(server, item, &heading);
    let count = history::size(&server.prompt_history, ty);
    for h in 0..count {
        let text = concat(&[
            (h + 1).to_string().as_bytes(),
            b": ",
            history::get(&server.prompt_history, ty, h).unwrap_or_default(),
        ]);
        queue::print(server, item, &text);
    }
    queue::print(server, item, b"");
}

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let typestr = args.get(b'T');

    let ty = match typestr {
        None => None,
        Some(typestr) => match prompt_type(typestr) {
            PromptType::Invalid => {
                return fail(server, item, concat(&[b"invalid type: ", typestr]));
            }
            ty => Some(ty),
        },
    };

    if std::ptr::eq(command.entry, &CMD_CLEAR_PROMPT_HISTORY) {
        match ty {
            None => {
                for t in all_types() {
                    history::clear(&mut server.prompt_history, t);
                }
            }
            Some(t) => history::clear(&mut server.prompt_history, t),
        }
        return CmdReturn::Normal;
    }

    match ty {
        None => {
            for t in all_types() {
                show(server, item, t);
            }
        }
        Some(t) => show(server, item, t),
    }
    CmdReturn::Normal
}
