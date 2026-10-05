// Ported from tmux cmd.c @ 8f25579c
//! Command executors. G20 (commands A) and G21 (commands B) each own their
//! `pub mod` lines and their `execute_*` dispatcher; P7Server's
//! `QueueRuntime::execute` tries A then B.

use super::Command;
use super::metadata as m;
use super::queue::CmdReturn;
use crate::ids::QueueItemId;
use crate::server::Server;

pub mod attach_session;
pub mod bind_key;
pub mod break_pane;
pub mod capture_pane;
pub mod choose_tree;
pub mod command_prompt;
pub mod confirm_before;
pub mod copy_mode;
pub mod detach_client;
pub mod display_menu;
pub mod display_message;
pub mod find_window;
pub mod if_shell;
pub mod join_pane;
pub mod kill_pane;
pub mod kill_server;
pub mod kill_session;
pub mod kill_window;
pub mod list_buffers;
pub mod list_clients;
pub mod list_commands;
pub mod list_keys;
pub mod list_panes;
pub mod list_sessions;
pub mod list_windows;
pub mod load_buffer;
pub mod lock_server;
pub mod move_window;
pub mod new_session;
pub mod new_window;

pub fn execute_a(server: &mut Server, command: &Command, item: QueueItemId) -> Option<CmdReturn> {
    let is = |entry: &'static super::CommandEntry| std::ptr::eq(command.entry, entry);
    Some(if is(&m::CMD_BIND_KEY) {
        bind_key::execute(server, command, item)
    } else if is(&m::CMD_ATTACH_SESSION) {
        attach_session::execute(server, command, item)
    } else if is(&m::CMD_BREAK_PANE) {
        break_pane::execute(server, command, item)
    } else if is(&m::CMD_CHOOSE_TREE)
        || is(&m::CMD_CHOOSE_CLIENT)
        || is(&m::CMD_CHOOSE_BUFFER)
        || is(&m::CMD_CUSTOMIZE_MODE)
        || is(&m::CMD_SWITCH_MODE)
        || is(&m::CMD_DISPLAY_PANES)
    {
        choose_tree::execute(server, command, item)
    } else if is(&m::CMD_CAPTURE_PANE) || is(&m::CMD_CLEAR_HISTORY) {
        capture_pane::execute(server, command, item)
    } else if is(&m::CMD_COMMAND_PROMPT) {
        command_prompt::execute(server, command, item)
    } else if is(&m::CMD_CONFIRM_BEFORE) {
        confirm_before::execute(server, command, item)
    } else if is(&m::CMD_COPY_MODE) || is(&m::CMD_CLOCK_MODE) {
        copy_mode::execute(server, command, item)
    } else if is(&m::CMD_FIND_WINDOW) {
        find_window::execute(server, command, item)
    } else if is(&m::CMD_DETACH_CLIENT) || is(&m::CMD_SUSPEND_CLIENT) {
        detach_client::execute(server, command, item)
    } else if is(&m::CMD_DISPLAY_MESSAGE) {
        display_message::execute(server, command, item)
    } else if is(&m::CMD_DISPLAY_MENU) {
        display_menu::execute_menu(server, command, item)
    } else if is(&m::CMD_DISPLAY_POPUP) {
        display_menu::execute_popup(server, command, item)
    } else if is(&m::CMD_JOIN_PANE) || is(&m::CMD_MOVE_PANE) {
        join_pane::execute(server, command, item)
    } else if is(&m::CMD_KILL_PANE) {
        kill_pane::execute(server, command, item)
    } else if is(&m::CMD_KILL_SESSION) {
        kill_session::execute(server, command, item)
    } else if is(&m::CMD_KILL_WINDOW) || is(&m::CMD_UNLINK_WINDOW) {
        kill_window::execute(server, command, item)
    } else if is(&m::CMD_KILL_SERVER) || is(&m::CMD_START_SERVER) {
        kill_server::execute(server, command, item)
    } else if is(&m::CMD_LIST_BUFFERS) {
        list_buffers::execute(server, command, item)
    } else if is(&m::CMD_LIST_CLIENTS) {
        list_clients::execute(server, command, item)
    } else if is(&m::CMD_LIST_COMMANDS) {
        list_commands::execute(server, command, item)
    } else if is(&m::CMD_LIST_KEYS) {
        list_keys::execute(server, command, item)
    } else if is(&m::CMD_LIST_PANES) {
        list_panes::execute(server, command, item)
    } else if is(&m::CMD_LIST_SESSIONS) {
        list_sessions::execute(server, command, item)
    } else if is(&m::CMD_LIST_WINDOWS) {
        list_windows::execute(server, command, item)
    } else if is(&m::CMD_LOCK_SERVER) || is(&m::CMD_LOCK_SESSION) || is(&m::CMD_LOCK_CLIENT) {
        lock_server::execute(server, command, item)
    } else if is(&m::CMD_IF_SHELL) {
        if_shell::execute(server, command, item)
    } else if is(&m::CMD_LOAD_BUFFER) {
        load_buffer::execute(server, command, item)
    } else if is(&m::CMD_MOVE_WINDOW) || is(&m::CMD_LINK_WINDOW) {
        move_window::execute(server, command, item)
    } else if is(&m::CMD_NEW_SESSION) || is(&m::CMD_HAS_SESSION) {
        new_session::execute(server, command, item)
    } else if is(&m::CMD_NEW_WINDOW) {
        new_window::execute(server, command, item)
    } else {
        return None;
    })
}

// G21 commands B (P9CmdB)
pub mod paste_buffer;
pub mod pipe_pane;
pub mod refresh_client;
pub mod rename_session;
pub mod rename_window;
pub mod resize_pane;
pub mod resize_window;
pub mod respawn_pane;
pub mod respawn_window;
pub mod rotate_window;
pub mod run_shell;
pub mod save_buffer;
pub mod select_layout;
pub mod select_pane;
pub mod select_window;
pub mod send_keys;
pub mod server_access;
pub mod set_buffer;
pub mod set_environment;
pub mod set_option;
pub mod show_environment;
pub mod show_messages;
pub mod show_options;
pub mod show_prompt_history;
pub mod source_file;
pub mod split_window;
pub mod support;
pub mod swap_pane;
pub mod swap_window;
pub mod switch_client;
pub mod unbind_key;
pub mod wait_for;

/// Dispatch a G21 entry; `None` when `command.entry` is not a G21 command.
pub fn execute_b(server: &mut Server, command: &Command, item: QueueItemId) -> Option<CmdReturn> {
    let e = command.entry;
    let is = |x: &'static super::CommandEntry| std::ptr::eq(e, x);
    let r = if is(&m::CMD_PASTE_BUFFER) {
        paste_buffer::execute(server, command, item)
    } else if is(&m::CMD_PIPE_PANE) {
        pipe_pane::execute(server, command, item)
    } else if is(&m::CMD_REFRESH_CLIENT) {
        refresh_client::execute(server, command, item)
    } else if is(&m::CMD_RENAME_SESSION) {
        rename_session::execute(server, command, item)
    } else if is(&m::CMD_RENAME_WINDOW) {
        rename_window::execute(server, command, item)
    } else if is(&m::CMD_RESIZE_PANE) {
        resize_pane::execute(server, command, item)
    } else if is(&m::CMD_RESIZE_WINDOW) {
        resize_window::execute(server, command, item)
    } else if is(&m::CMD_RESPAWN_PANE) {
        respawn_pane::execute(server, command, item)
    } else if is(&m::CMD_RESPAWN_WINDOW) {
        respawn_window::execute(server, command, item)
    } else if is(&m::CMD_ROTATE_WINDOW) {
        rotate_window::execute(server, command, item)
    } else if is(&m::CMD_RUN_SHELL) {
        run_shell::execute(server, command, item)
    } else if is(&m::CMD_SAVE_BUFFER) || is(&m::CMD_SHOW_BUFFER) {
        save_buffer::execute(server, command, item)
    } else if is(&m::CMD_SELECT_LAYOUT) || is(&m::CMD_NEXT_LAYOUT) || is(&m::CMD_PREVIOUS_LAYOUT) {
        select_layout::execute(server, command, item)
    } else if is(&m::CMD_SELECT_PANE) || is(&m::CMD_LAST_PANE) {
        select_pane::execute(server, command, item)
    } else if is(&m::CMD_SELECT_WINDOW)
        || is(&m::CMD_NEXT_WINDOW)
        || is(&m::CMD_PREVIOUS_WINDOW)
        || is(&m::CMD_LAST_WINDOW)
    {
        select_window::execute(server, command, item)
    } else if is(&m::CMD_SEND_KEYS) || is(&m::CMD_SEND_PREFIX) {
        send_keys::execute(server, command, item)
    } else if is(&m::CMD_SERVER_ACCESS) {
        server_access::execute(server, command, item)
    } else if is(&m::CMD_SET_BUFFER) || is(&m::CMD_DELETE_BUFFER) {
        set_buffer::execute(server, command, item)
    } else if is(&m::CMD_SET_ENVIRONMENT) {
        set_environment::execute(server, command, item)
    } else if is(&m::CMD_SET_OPTION) || is(&m::CMD_SET_WINDOW_OPTION) || is(&m::CMD_SET_HOOK) {
        set_option::execute(server, command, item)
    } else if is(&m::CMD_SHOW_ENVIRONMENT) {
        show_environment::execute(server, command, item)
    } else if is(&m::CMD_SHOW_MESSAGES) {
        show_messages::execute(server, command, item)
    } else if is(&m::CMD_SHOW_OPTIONS) || is(&m::CMD_SHOW_WINDOW_OPTIONS) || is(&m::CMD_SHOW_HOOKS)
    {
        show_options::execute(server, command, item)
    } else if is(&m::CMD_SHOW_PROMPT_HISTORY) || is(&m::CMD_CLEAR_PROMPT_HISTORY) {
        show_prompt_history::execute(server, command, item)
    } else if is(&m::CMD_SOURCE_FILE) {
        source_file::execute(server, command, item)
    } else if is(&m::CMD_NEW_PANE) || is(&m::CMD_SPLIT_WINDOW) {
        split_window::execute(server, command, item)
    } else if is(&m::CMD_SWAP_PANE) {
        swap_pane::execute(server, command, item)
    } else if is(&m::CMD_SWAP_WINDOW) {
        swap_window::execute(server, command, item)
    } else if is(&m::CMD_SWITCH_CLIENT) {
        switch_client::execute(server, command, item)
    } else if is(&m::CMD_UNBIND_KEY) {
        unbind_key::execute(server, command, item)
    } else if is(&m::CMD_WAIT_FOR) {
        wait_for::execute(server, command, item)
    } else {
        return None;
    };
    Some(r)
}
