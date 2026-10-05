// Ported from tmux cmd-show-options.c @ 8f25579c
use crate::cmd::Command;
use crate::cmd::hooks;
use crate::cmd::metadata::{CMD_SHOW_HOOKS, CMD_SHOW_WINDOW_OPTIONS};
use crate::cmd::queue::{self, CmdReturn};
use crate::format::{self, FormatTree};
use crate::ids::{OptionsId, QueueItemId};
use crate::model::monitor::MonitorType;
use crate::options::scope::{scope_from_flags, scope_from_name};
use crate::options::{
    Ambiguous, OPTIONS_TABLE, OptionName, OptionsArrayKey, OptionsScope, match_name,
};
use crate::server::Server;
use rmux_util::bytes::ByteString;
use rmux_util::time::Timestamp;

use super::set_option::{scope_flags, scope_target};
use super::support::{concat, fail, item_target};

/// `SHOW_OPTIONS_TEMPLATE` (`cmd-show-options.c:30-37`).
pub(super) const SHOW_OPTIONS_TEMPLATE: &[u8] = b"#{?option_value_only,\
#{option_value},\
#{option_name}#{?option_has_array_key,\
[#{option_array_key}],}\
#{?option_is_parent,*,}\
#{?option_has_value, \
#{?option_is_string,#{q/a:option_value},#{option_value}},}}";

/// `SHOW_HOOKS_MONITOR_TEMPLATE` (`cmd-show-options.c:38-39`).
const SHOW_HOOKS_MONITOR_TEMPLATE: &[u8] =
    b"#{option_name}:#{hook_monitor_target}:#{hook_monitor_format}";

/// The `option_*` format variables one printed line carries
/// (`cmd-show-options.c:229-237,248-254` and `306-316`).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct OptionFields {
    pub name: ByteString,
    pub value: ByteString,
    pub value_only: bool,
    pub parent: bool,
    pub is_array: bool,
    pub is_string: bool,
    pub is_hook: bool,
    pub is_user: bool,
    pub has_value: bool,
    pub array_key: Option<ByteString>,
}

fn flag(value: bool) -> ByteString {
    ByteString::from(if value { "1" } else { "0" })
}

/// `format_add` every `option_*` variable of `fields` to `tree`.
pub(super) fn add_option_fields(tree: &mut FormatTree, fields: &OptionFields) {
    tree.add(b"option_name", fields.name.clone());
    tree.add(b"option_value", fields.value.clone());
    tree.add(b"option_value_only", flag(fields.value_only));
    tree.add(b"option_is_parent", flag(fields.parent));
    tree.add(b"option_is_array", flag(fields.is_array));
    tree.add(b"option_is_string", flag(fields.is_string));
    tree.add(b"option_is_hook", flag(fields.is_hook));
    tree.add(b"option_is_user", flag(fields.is_user));
    tree.add(b"option_has_value", flag(fields.has_value));
    match &fields.array_key {
        Some(key) => {
            tree.add(b"option_array_key", key.clone());
            tree.add(b"option_has_array_key", flag(true));
        }
        None => {
            tree.add(b"option_array_key", ByteString::new());
            tree.add(b"option_has_array_key", flag(false));
        }
    }
}

/// `hook_fire_count` always, `hook_fire_time` only when nonzero
/// (`cmd-show-options.c:238-247,321-328`).
fn add_fire_fields(tree: &mut FormatTree, fire_count: u32, fire_time: i64) {
    tree.add(b"hook_fire_count", ByteString::from(fire_count.to_string()));
    if fire_time != 0 {
        tree.add_time(b"hook_fire_time", Timestamp::new(fire_time, 0));
    }
}

struct Show<'a> {
    command: &'a Command,
    item: QueueItemId,
    hooks: bool,
}

impl Show<'_> {
    fn template(&self) -> Option<&[u8]> {
        self.command.args.get(b'F')
    }

    /// `cmd_show_options_print` (`cmd-show-options.c:183-261`); the entry
    /// is named by its owning tree and name so no store borrow outlives a
    /// format expansion.
    fn print(
        &self,
        server: &mut Server,
        owner: OptionsId,
        name: &[u8],
        array_key: Option<&OptionsArrayKey>,
        parent: bool,
    ) {
        let args = &self.command.args;
        let Some(o) = server.options.get_only(owner, name) else {
            return;
        };
        if array_key.is_none() && o.is_array() {
            let keys: Vec<OptionsArrayKey> = o.array_items().map(|(k, _)| k.clone()).collect();
            if !keys.is_empty() {
                for key in &keys {
                    self.print(server, owner, name, Some(key), parent);
                }
                return;
            }
            if self.template().is_none() && args.has(b'v') != 0 {
                return;
            }
        }
        let o = server.options.get_only(owner, name).expect("option entry");
        let (value, has_value) = match array_key {
            Some(key) => (o.to_string(Some(key), false), true),
            None if o.is_array() => (ByteString::new(), false),
            None => (o.to_string(None, false), true),
        };
        let oe = o.table_entry();
        let fields = OptionFields {
            name: ByteString::from(o.name()),
            value,
            value_only: args.has(b'v') != 0,
            parent,
            is_array: o.is_array(),
            is_string: o.is_string(),
            is_hook: oe.is_some_and(|oe| oe.is_hook()),
            is_user: oe.is_none(),
            has_value,
            array_key: array_key.map(OptionsArrayKey::to_bytes),
        };
        let fire = self.hooks.then(|| (o.fire_count(), o.fire_time().sec));

        let mut tree = format::create_from_target(server, self.item);
        add_option_fields(&mut tree, &fields);
        if let Some((count, time)) = fire {
            add_fire_fields(&mut tree, count, time);
        }
        let line = tree.expand(server, self.template().unwrap_or(SHOW_OPTIONS_TEMPLATE));
        tree.release(server);
        queue::print(server, self.item, &line);
    }

    /// `cmd_show_hooks_print_monitor` (`cmd-show-options.c:263-337`).
    fn print_monitor(&self, server: &mut Server, owner: OptionsId, name: &[u8]) {
        let Some(monitor_id) = server
            .options
            .get_only(owner, name)
            .and_then(|o| o.monitor())
        else {
            return;
        };
        let Some(monitor) = server
            .hooks
            .monitors
            .get(monitor_id)
            .and_then(Option::as_ref)
        else {
            return;
        };
        let value = hooks::monitor_to_string(name, monitor);
        let Some((kind, id, monitor_format)) = hooks::monitor_get(&server.hooks, monitor_id) else {
            return;
        };
        let target: ByteString = match kind {
            MonitorType::Session => ByteString::new(),
            MonitorType::Pane => ByteString::from(format!("%{id}")),
            MonitorType::AllPanes => ByteString::from("%*"),
            MonitorType::Window => ByteString::from(format!("@{id}")),
            MonitorType::AllWindows => ByteString::from("@*"),
        };
        let monitor_format = ByteString::from(monitor_format);
        let fire_count = hooks::monitor_get_fire_count(&*server, monitor_id, name);
        let fire_time = hooks::monitor_get_fire_time(&*server, monitor_id, name);
        let fields = OptionFields {
            name: ByteString::from(name),
            value,
            value_only: false,
            parent: false,
            is_array: false,
            is_string: true,
            is_hook: true,
            is_user: true,
            has_value: true,
            array_key: None,
        };

        let mut tree = format::create_from_target(server, self.item);
        add_option_fields(&mut tree, &fields);
        tree.add(b"hook_monitor_target", target);
        tree.add(b"hook_monitor_format", monitor_format);
        add_fire_fields(&mut tree, fire_count, fire_time);
        let line = tree.expand(
            server,
            self.template().unwrap_or(SHOW_HOOKS_MONITOR_TEMPLATE),
        );
        tree.release(server);
        queue::print(server, self.item, &line);
    }

    /// `cmd_show_options_all` (`cmd-show-options.c:339-395`).
    fn all(&self, server: &mut Server, scope: OptionsScope, oo: OptionsId) -> CmdReturn {
        let args = &self.command.args;
        let user: Vec<(ByteString, bool)> = server
            .options
            .entries(oo)
            .filter(|o| o.table_entry().is_none())
            .map(|o| {
                let name = o.name();
                let is_user_hook = name.first() == Some(&b'@')
                    && (hooks::is_event(&server.hooks, name) || o.monitor().is_some());
                (ByteString::from(name), is_user_hook)
            })
            .collect();
        for (name, is_user_hook) in &user {
            if !self.hooks {
                if !is_user_hook || args.has(b'H') != 0 {
                    self.print(server, oo, name, None, false);
                }
            } else if *is_user_hook {
                self.print(server, oo, name, None, false);
            }
        }
        for oe in OPTIONS_TABLE {
            if !oe.scope.contains(scope) {
                continue;
            }
            if (!self.hooks && args.has(b'H') == 0 && oe.is_hook()) || (self.hooks && !oe.is_hook())
            {
                continue;
            }
            let (owner, parent) = if server.options.get_only(oo, oe.name).is_some() {
                (oo, false)
            } else {
                if args.has(b'A') == 0 {
                    continue;
                }
                match server.options.get(oo, oe.name) {
                    Some((owner, _)) => (owner, true),
                    None => continue,
                }
            };
            self.print(server, owner, oe.name, None, parent);
        }
        CmdReturn::Normal
    }
}

/// `cmd_show_options_exec` (`cmd-show-options.c:89-181`).
pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let window = std::ptr::eq(command.entry, &CMD_SHOW_WINDOW_OPTIONS);
    let show = Show {
        command,
        item,
        hooks: std::ptr::eq(command.entry, &CMD_SHOW_HOOKS),
    };
    let quiet = args.has(b'q') != 0;
    let monitors = show.hooks && args.has(b'B') != 0;
    let target = item_target(server, item);
    let flags = scope_flags(args, window);

    if args.count() == 0 {
        let scope_target = scope_target(server, &target, args);
        let (scope, oo) = match scope_from_flags(flags, &scope_target, &server.options) {
            Ok(found) => found,
            Err(cause) => {
                if quiet {
                    return CmdReturn::Normal;
                }
                return fail(server, item, cause.as_bytes());
            }
        };
        if monitors {
            let names: Vec<ByteString> = server
                .options
                .entries(oo)
                .map(|o| ByteString::from(o.name()))
                .collect();
            for name in &names {
                show.print_monitor(server, oo, name);
            }
            return CmdReturn::Normal;
        }
        return show.all(server, scope, oo);
    }

    let argument = format::single_from_target(server, item, args.string(0).unwrap_or(b""));
    let (name, array_key): (OptionName, Option<OptionsArrayKey>) = match match_name(&argument) {
        Ok(Some(found)) => found,
        Ok(None) => {
            if quiet {
                return CmdReturn::Normal;
            }
            return fail(server, item, concat(&[b"invalid option: ", &argument]));
        }
        Err(Ambiguous) => {
            if quiet {
                return CmdReturn::Normal;
            }
            return fail(server, item, concat(&[b"ambiguous option: ", &argument]));
        }
    };
    let is_user = matches!(name, OptionName::User(_));
    let name = name.as_bytes();
    let scope_target = scope_target(server, &target, args);
    let (_, oo) = match scope_from_name(flags, name, &scope_target, &server.options) {
        Ok(found) => found,
        Err(cause) => {
            if quiet {
                return CmdReturn::Normal;
            }
            return fail(server, item, cause.as_bytes());
        }
    };
    let mut parent = false;
    let mut found = server.options.get_only(oo, name).map(|_| oo);
    if args.has(b'A') != 0 && found.is_none() {
        found = server.options.get(oo, name).map(|(owner, _)| owner);
        parent = true;
    }
    match found {
        Some(owner) => {
            if monitors {
                show.print_monitor(server, owner, name);
            } else {
                let empty_array = array_key.is_none()
                    && server
                        .options
                        .get_only(owner, name)
                        .is_some_and(|o| o.is_array() && o.array_items().next().is_none());
                let print_parent = parent && !empty_array;
                show.print(server, owner, name, array_key.as_ref(), print_parent);
            }
        }
        None if is_user => {
            if quiet {
                return CmdReturn::Normal;
            }
            return fail(server, item, concat(&[b"invalid option: ", &argument]));
        }
        None => {}
    }
    CmdReturn::Normal
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::FormatFlags;

    fn expand(fields: &OptionFields) -> ByteString {
        let mut server = Server::new();
        let mut tree = FormatTree::create(None, None, 0, FormatFlags::NONE, &mut server);
        add_option_fields(&mut tree, fields);
        let line = tree.expand(&mut server, SHOW_OPTIONS_TEMPLATE);
        tree.release(&mut server);
        line
    }

    fn string_option(name: &str, value: &str) -> OptionFields {
        OptionFields {
            name: name.into(),
            value: value.into(),
            is_string: true,
            has_value: true,
            ..OptionFields::default()
        }
    }

    /// Spec unit test 8: the default template with a fake tree.
    #[test]
    fn default_template_quotes_strings_and_marks_parents() {
        assert_eq!(
            expand(&string_option("status-left", "[#S] ")),
            b"status-left \"[#S] \""
        );
        let mut parent = string_option("status-left", "x");
        parent.parent = true;
        assert_eq!(expand(&parent), b"status-left* x");
    }

    #[test]
    fn default_template_prints_array_keys_and_numbers() {
        let fields = OptionFields {
            name: "command-alias".into(),
            value: "split-pane=split-window".into(),
            is_array: true,
            is_string: true,
            has_value: true,
            array_key: Some("0".into()),
            ..OptionFields::default()
        };
        assert_eq!(expand(&fields), b"command-alias[0] split-pane=split-window");
        let mut number = string_option("status", "on");
        number.is_string = false;
        assert_eq!(expand(&number), b"status on");
    }

    #[test]
    fn default_template_empty_array_and_value_only() {
        let empty = OptionFields {
            name: "update-environment".into(),
            is_array: true,
            is_string: true,
            ..OptionFields::default()
        };
        assert_eq!(expand(&empty), b"update-environment");
        let mut only = string_option("status-left", "a b");
        only.value_only = true;
        assert_eq!(expand(&only), b"a b");
    }
}
