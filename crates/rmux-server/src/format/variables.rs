// Ported from tmux format.c @ 8f25579c
/*
 * Copyright (c) 2009 Nicholas Marriott <nicholas.marriott@gmail.com>
 *
 * Permission to use, copy, modify, and distribute this software for any
 * purpose with or without fee is hereby granted, provided that the above
 * copyright notice and this permission notice appear in all copies.
 *
 * THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
 * WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
 * MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
 * ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
 * WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
 * ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
 * OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
 */

use super::{FormatContext, FormatKind, FormatRuntime, FormatValue};
use crate::{
    client::ClientFlags,
    model::{PaneFlags, WindowFlags, WinlinkFlags},
};
use rmux_emu::{
    cell::GridCellFlags,
    colour::Colour,
    grid::Grid,
    hyperlinks::HyperlinkRegistry,
    screen::{ProgressBarState, Screen, ScreenCursorStyle, ScreenMode},
};
use rmux_util::{
    bytes::{ByteString, cstr},
    time::Timestamp,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValueKind {
    String,
    Time,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Builtin {
    pub key: &'static [u8],
    pub kind: ValueKind,
}

pub const REGISTRY: &[&[u8]] = &[
    b"active_window_index",
    b"alternate_on",
    b"alternate_saved_x",
    b"alternate_saved_y",
    b"bracket_paste_flag",
    b"buffer_created",
    b"buffer_full",
    b"buffer_mode_format",
    b"buffer_name",
    b"buffer_sample",
    b"buffer_size",
    b"client_activity",
    b"client_cell_height",
    b"client_cell_width",
    b"client_colours",
    b"client_control_mode",
    b"client_created",
    b"client_discarded",
    b"client_flags",
    b"client_height",
    b"client_key_table",
    b"client_last_session",
    b"client_mode_format",
    b"client_name",
    b"client_pid",
    b"client_prefix",
    b"client_readonly",
    b"client_session",
    b"client_termfeatures",
    b"client_termname",
    b"client_termtype",
    b"client_theme",
    b"client_tsp",
    b"client_tty",
    b"client_uid",
    b"client_user",
    b"client_utf8",
    b"client_width",
    b"client_written",
    b"config_files",
    b"cursor_blinking",
    b"cursor_character",
    b"cursor_colour",
    b"cursor_flag",
    b"cursor_shape",
    b"cursor_very_visible",
    b"cursor_x",
    b"cursor_y",
    b"history_added",
    b"history_all_bytes",
    b"history_bytes",
    b"history_collected",
    b"history_generation",
    b"history_limit",
    b"history_size",
    b"host",
    b"host_short",
    b"insert_flag",
    b"keypad_cursor_flag",
    b"keypad_flag",
    b"last_window_index",
    b"mouse_all_flag",
    b"mouse_any_flag",
    b"mouse_button_flag",
    b"mouse_hyperlink",
    b"mouse_line",
    b"mouse_pane",
    b"mouse_sgr_flag",
    b"mouse_standard_flag",
    b"mouse_status_line",
    b"mouse_status_range",
    b"mouse_utf8_flag",
    b"mouse_word",
    b"mouse_x",
    b"mouse_y",
    b"next_session_id",
    b"origin_flag",
    b"pane_active",
    b"pane_at_bottom",
    b"pane_at_left",
    b"pane_at_right",
    b"pane_at_top",
    b"pane_bg",
    b"pane_bottom",
    b"pane_command_duration",
    b"pane_command_end_time",
    b"pane_command_running",
    b"pane_command_start_time",
    b"pane_command_status",
    b"pane_current_command",
    b"pane_current_path",
    b"pane_dead",
    b"pane_dead_signal",
    b"pane_dead_status",
    b"pane_dead_time",
    b"pane_fg",
    b"pane_flags",
    b"pane_floating_flag",
    b"pane_format",
    b"pane_height",
    b"pane_id",
    b"pane_in_mode",
    b"pane_index",
    b"pane_input_off",
    b"pane_key_mode",
    b"pane_last",
    b"pane_last_output_time",
    b"pane_last_prompt_time",
    b"pane_left",
    b"pane_marked",
    b"pane_marked_set",
    b"pane_modal_flag",
    b"pane_mode",
    b"pane_output_generation",
    b"pane_path",
    b"pane_pb_progress",
    b"pane_pb_state",
    b"pane_pid",
    b"pane_pipe",
    b"pane_pipe_pid",
    b"pane_private_modes",
    b"pane_right",
    b"pane_search_string",
    b"pane_start_command",
    b"pane_start_command_list",
    b"pane_start_path",
    b"pane_synchronized",
    b"pane_tabs",
    b"pane_title",
    b"pane_top",
    b"pane_tsp",
    b"pane_tsp_epoch",
    b"pane_tty",
    b"pane_unseen_changes",
    b"pane_unzoomed_height",
    b"pane_unzoomed_width",
    b"pane_width",
    b"pane_x",
    b"pane_y",
    b"pane_z",
    b"pane_zoomed_flag",
    b"pid",
    b"scroll_region_lower",
    b"scroll_region_upper",
    b"server_sessions",
    b"session_active",
    b"session_activity",
    b"session_activity_flag",
    b"session_alert",
    b"session_alerts",
    b"session_attached",
    b"session_attached_list",
    b"session_bell_flag",
    b"session_created",
    b"session_format",
    b"session_group",
    b"session_group_attached",
    b"session_group_attached_list",
    b"session_group_list",
    b"session_group_many_attached",
    b"session_group_size",
    b"session_grouped",
    b"session_id",
    b"session_last_attached",
    b"session_many_attached",
    b"session_marked",
    b"session_name",
    b"session_path",
    b"session_silence_flag",
    b"session_stack",
    b"session_windows",
    b"sixel_support",
    b"socket_path",
    b"start_time",
    b"synchronized_output_flag",
    b"tree_mode_format",
    b"uid",
    b"user",
    b"version",
    b"window_active",
    b"window_active_clients",
    b"window_active_clients_list",
    b"window_active_sessions",
    b"window_active_sessions_list",
    b"window_activity",
    b"window_activity_flag",
    b"window_bell_flag",
    b"window_bigger",
    b"window_cell_height",
    b"window_cell_width",
    b"window_end_flag",
    b"window_flags",
    b"window_format",
    b"window_height",
    b"window_id",
    b"window_index",
    b"window_last_flag",
    b"window_layout",
    b"window_linked",
    b"window_linked_sessions",
    b"window_linked_sessions_list",
    b"window_manual_height",
    b"window_manual_width",
    b"window_marked_flag",
    b"window_modal_pane",
    b"window_name",
    b"window_offset_x",
    b"window_offset_y",
    b"window_panes",
    b"window_raw_flags",
    b"window_silence_flag",
    b"window_stack_index",
    b"window_start_flag",
    b"window_visible_layout",
    b"window_width",
    b"window_zoomed_flag",
    b"wrap_flag",
];

pub const BUILTINS: &[Builtin] = &[
    Builtin {
        key: b"active_window_index",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"alternate_on",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"alternate_saved_x",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"alternate_saved_y",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"bracket_paste_flag",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"buffer_created",
        kind: ValueKind::Time,
    },
    Builtin {
        key: b"buffer_full",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"buffer_mode_format",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"buffer_name",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"buffer_sample",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"buffer_size",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"client_activity",
        kind: ValueKind::Time,
    },
    Builtin {
        key: b"client_cell_height",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"client_cell_width",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"client_colours",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"client_control_mode",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"client_created",
        kind: ValueKind::Time,
    },
    Builtin {
        key: b"client_discarded",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"client_flags",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"client_height",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"client_key_table",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"client_last_session",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"client_mode_format",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"client_name",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"client_pid",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"client_prefix",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"client_readonly",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"client_session",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"client_termfeatures",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"client_termname",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"client_termtype",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"client_theme",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"client_tsp",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"client_tty",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"client_uid",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"client_user",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"client_utf8",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"client_width",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"client_written",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"config_files",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"cursor_blinking",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"cursor_character",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"cursor_colour",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"cursor_flag",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"cursor_shape",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"cursor_very_visible",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"cursor_x",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"cursor_y",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"history_added",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"history_all_bytes",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"history_bytes",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"history_collected",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"history_generation",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"history_limit",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"history_size",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"host",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"host_short",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"insert_flag",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"keypad_cursor_flag",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"keypad_flag",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"last_window_index",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"mouse_all_flag",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"mouse_any_flag",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"mouse_button_flag",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"mouse_hyperlink",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"mouse_line",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"mouse_pane",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"mouse_sgr_flag",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"mouse_standard_flag",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"mouse_status_line",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"mouse_status_range",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"mouse_utf8_flag",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"mouse_word",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"mouse_x",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"mouse_y",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"next_session_id",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"origin_flag",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_active",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_at_bottom",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_at_left",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_at_right",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_at_top",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_bg",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_bottom",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_command_duration",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_command_end_time",
        kind: ValueKind::Time,
    },
    Builtin {
        key: b"pane_command_running",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_command_start_time",
        kind: ValueKind::Time,
    },
    Builtin {
        key: b"pane_command_status",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_current_command",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_current_path",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_dead",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_dead_signal",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_dead_status",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_dead_time",
        kind: ValueKind::Time,
    },
    Builtin {
        key: b"pane_fg",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_flags",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_floating_flag",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_format",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_height",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_id",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_in_mode",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_index",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_input_off",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_key_mode",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_last",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_last_output_time",
        kind: ValueKind::Time,
    },
    Builtin {
        key: b"pane_last_prompt_time",
        kind: ValueKind::Time,
    },
    Builtin {
        key: b"pane_left",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_marked",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_marked_set",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_modal_flag",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_mode",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_output_generation",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_path",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_pb_progress",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_pb_state",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_pid",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_pipe",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_pipe_pid",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_private_modes",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_right",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_search_string",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_start_command",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_start_command_list",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_start_path",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_synchronized",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_tabs",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_title",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_top",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_tsp",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_tsp_epoch",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_tty",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_unseen_changes",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_unzoomed_height",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_unzoomed_width",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_width",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_x",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_y",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_z",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pane_zoomed_flag",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"pid",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"scroll_region_lower",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"scroll_region_upper",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"server_sessions",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"session_active",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"session_activity",
        kind: ValueKind::Time,
    },
    Builtin {
        key: b"session_activity_flag",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"session_alert",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"session_alerts",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"session_attached",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"session_attached_list",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"session_bell_flag",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"session_created",
        kind: ValueKind::Time,
    },
    Builtin {
        key: b"session_format",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"session_group",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"session_group_attached",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"session_group_attached_list",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"session_group_list",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"session_group_many_attached",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"session_group_size",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"session_grouped",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"session_id",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"session_last_attached",
        kind: ValueKind::Time,
    },
    Builtin {
        key: b"session_many_attached",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"session_marked",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"session_name",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"session_path",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"session_silence_flag",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"session_stack",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"session_windows",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"sixel_support",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"socket_path",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"start_time",
        kind: ValueKind::Time,
    },
    Builtin {
        key: b"synchronized_output_flag",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"tree_mode_format",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"uid",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"user",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"version",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_active",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_active_clients",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_active_clients_list",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_active_sessions",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_active_sessions_list",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_activity",
        kind: ValueKind::Time,
    },
    Builtin {
        key: b"window_activity_flag",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_bell_flag",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_bigger",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_cell_height",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_cell_width",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_end_flag",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_flags",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_format",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_height",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_id",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_index",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_last_flag",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_layout",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_linked",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_linked_sessions",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_linked_sessions_list",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_manual_height",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_manual_width",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_marked_flag",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_modal_pane",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_name",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_offset_x",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_offset_y",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_panes",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_raw_flags",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_silence_flag",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_stack_index",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_start_flag",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_visible_layout",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_width",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"window_zoomed_flag",
        kind: ValueKind::String,
    },
    Builtin {
        key: b"wrap_flag",
        kind: ValueKind::String,
    },
];

pub fn find(
    runtime: &mut dyn FormatRuntime,
    context: &FormatContext,
    key: &[u8],
) -> Option<FormatValue> {
    find_owned(runtime, context, None, key)
}

pub fn find_owned(
    runtime: &mut dyn FormatRuntime,
    context: &FormatContext,
    owner: Option<crate::ids::ClientId>,
    key: &[u8],
) -> Option<FormatValue> {
    REGISTRY.binary_search(&key).ok()?;
    match key {
        b"pid" => Some(FormatValue::Unsigned(u64::from(std::process::id()))),
        b"uid" => Some(FormatValue::Unsigned(u64::from(rmux_sys::proc::getuid().0))),
        b"sixel_support" => Some(boolean(cfg!(feature = "sixel"))),
        b"pane_format" => Some(boolean(context.kind == FormatKind::Pane)),
        b"window_format" => Some(boolean(context.kind == FormatKind::Window)),
        b"session_format" => Some(boolean(context.kind == FormatKind::Session)),
        _ => runtime.builtin_owned(context, owner, key),
    }
}

fn boolean(value: bool) -> FormatValue {
    FormatValue::Unsigned(u64::from(value))
}
fn bytes(value: &[u8]) -> FormatValue {
    FormatValue::Bytes(ByteString::from(cstr(value)))
}
fn seconds(value: i64) -> FormatValue {
    FormatValue::Time(Timestamp {
        sec: value,
        usec: 0,
    })
}
fn timestamp(value: (i64, i64)) -> FormatValue {
    seconds(value.0)
}

pub fn quote_shell(value: &[u8]) -> ByteString {
    let value = cstr(value);
    let special = b"|&;<>(){}$`\\\"'*?[# =%\n\t";
    let mut result =
        Vec::with_capacity(value.len() + value.iter().filter(|b| special.contains(b)).count());
    for &byte in value {
        if special.contains(&byte) {
            result.push(b'\\');
        }
        result.push(byte);
    }
    ByteString(result)
}

pub fn quote_single(value: &[u8]) -> ByteString {
    let value = cstr(value);
    let mut result =
        Vec::with_capacity(value.len() + 2 + 3 * value.iter().filter(|&&b| b == b'\'').count());
    result.push(b'\'');
    for &byte in value {
        if byte == b'\'' {
            result.extend_from_slice(b"'\\''");
        } else {
            result.push(byte);
        }
    }
    result.push(b'\'');
    ByteString(result)
}

pub fn quote_style(value: &[u8]) -> ByteString {
    let value = cstr(value);
    let mut result = Vec::with_capacity(value.len() + value.iter().filter(|&&b| b == b'#').count());
    for &byte in value {
        if byte == b'#' {
            result.push(b'#');
        }
        result.push(byte);
    }
    ByteString(result)
}

pub fn relative_time(value: i64, now: i64) -> Option<ByteString> {
    if value > now {
        return None;
    }
    let age = now.wrapping_sub(value) as u64;
    let d = (age / 86400) as u32;
    let h = ((age % 86400) / 3600) as u32;
    let m = ((age % 3600) / 60) as u32;
    let s = (age % 60) as u32;
    let result = if d != 0 {
        if h != 0 {
            format!("{d}d{h}h")
        } else {
            format!("{d}d")
        }
    } else if h != 0 {
        if m != 0 {
            format!("{h}h{m}m")
        } else {
            format!("{h}h")
        }
    } else if m != 0 {
        if s != 0 {
            format!("{m}m{s}s")
        } else {
            format!("{m}m")
        }
    } else {
        format!("{s}s")
    };
    Some(ByteString::from(result))
}

pub fn difference_time(value: i64, now: i64) -> ByteString {
    ByteString::from(now.wrapping_sub(value).to_string())
}

pub fn pretty_time_at(value: i64, now: i64, seconds: bool) -> ByteString {
    let now = now.max(value);
    let Some(time) = rmux_sys::time::localtime(value) else {
        return ByteString::default();
    };
    let Some(current) = rmux_sys::time::localtime(now) else {
        return ByteString::default();
    };
    let age = now.wrapping_sub(value);
    let format: &[u8] = if age < 86400 {
        if seconds { b"%H:%M:%S" } else { b"%H:%M" }
    } else if (time.year() == current.year() && time.mon() == current.mon()) || age < 28 * 86400 {
        b"%a%d"
    } else if (time.year() == current.year() && time.mon() < current.mon())
        || (time.year() == current.year() - 1 && time.mon() > current.mon())
    {
        b"%d%b"
    } else {
        b"%h%y"
    };
    let mut result = [0; 9];
    let len = rmux_sys::time::strftime(&mut result, format, &time);
    ByteString::from(&result[..len])
}

pub fn screen_value(base: &Screen, visible: Option<&Screen>, key: &[u8]) -> Option<FormatValue> {
    let mode = match key {
        b"cursor_flag" => Some(ScreenMode::CURSOR),
        b"insert_flag" => Some(ScreenMode::INSERT),
        b"keypad_cursor_flag" => Some(ScreenMode::KCURSOR),
        b"keypad_flag" => Some(ScreenMode::KKEYPAD),
        b"mouse_all_flag" => Some(ScreenMode::MOUSE_ALL),
        b"mouse_any_flag" => Some(ScreenMode::ALL_MOUSE_MODES),
        b"mouse_button_flag" => Some(ScreenMode::MOUSE_BUTTON),
        b"mouse_sgr_flag" => Some(ScreenMode::MOUSE_SGR),
        b"mouse_standard_flag" => Some(ScreenMode::MOUSE_STANDARD),
        b"mouse_utf8_flag" => Some(ScreenMode::MOUSE_UTF8),
        b"origin_flag" => Some(ScreenMode::ORIGIN),
        b"synchronized_output_flag" => Some(ScreenMode::SYNC),
        b"wrap_flag" => Some(ScreenMode::WRAP),
        _ => None,
    };
    if let Some(mode) = mode {
        return Some(boolean(base.mode.intersects(mode)));
    }
    let grid = &base.grid;
    Some(match key {
        b"alternate_on" => boolean(base.saved_grid.is_some()),
        b"alternate_saved_x" => {
            FormatValue::Unsigned(u64::from(base.saved_cursor.unwrap_or_default().0))
        }
        b"alternate_saved_y" => {
            FormatValue::Unsigned(u64::from(base.saved_cursor.unwrap_or_default().1))
        }
        b"bracket_paste_flag" => boolean(visible?.mode.contains(ScreenMode::BRACKETPASTE)),
        b"cursor_blinking" => boolean(visible?.mode.contains(ScreenMode::CURSOR_BLINKING)),
        b"cursor_very_visible" => boolean(visible?.mode.contains(ScreenMode::CURSOR_VERY_VISIBLE)),
        b"cursor_character" => {
            let cell = grid.get_cell(base.cx, grid.hsize() + base.cy);
            if cell.flags.contains(GridCellFlags::PADDING) {
                return None;
            }
            bytes(cell.data.bytes())
        }
        b"cursor_colour" => {
            let visible = visible?;
            let colour = if visible.ccolour == Colour(-1) {
                visible.default_ccolour
            } else {
                visible.ccolour
            };
            let mut result = Vec::new();
            rmux_emu::colour::write_colour(colour, &mut result);
            FormatValue::Bytes(ByteString(result))
        }
        b"cursor_shape" => bytes(match visible?.cstyle {
            ScreenCursorStyle::Block => b"block",
            ScreenCursorStyle::Underline => b"underline",
            ScreenCursorStyle::Bar => b"bar",
            ScreenCursorStyle::Default => b"default",
        }),
        b"cursor_x" => FormatValue::Unsigned(u64::from(base.cx)),
        b"cursor_y" => FormatValue::Unsigned(u64::from(base.cy)),
        b"history_added" => FormatValue::Unsigned(u64::from(grid.scroll_added)),
        b"history_collected" => FormatValue::Unsigned(u64::from(grid.scroll_collected)),
        b"history_generation" => FormatValue::Unsigned(u64::from(grid.scroll_generation)),
        b"history_limit" => FormatValue::Unsigned(u64::from(grid.hlimit())),
        b"history_size" => FormatValue::Unsigned(u64::from(grid.hsize())),
        b"history_bytes" => {
            let mut size =
                u64::from(grid.hsize() + grid.sy()) * u64::from(rmux_emu::grid::LINE_BYTES);
            for line in grid
                .lines()
                .iter()
                .take((grid.hsize() + grid.sy()) as usize)
            {
                size += u64::from(line.cellsize()) * u64::from(rmux_emu::grid::CELL_ENTRY_BYTES);
                size += u64::from(line.extdsize()) * u64::from(rmux_emu::grid::EXTD_ENTRY_BYTES);
            }
            FormatValue::Unsigned(size)
        }
        b"history_all_bytes" => {
            let (lines, cells, extended) = grid.storage_counts();
            FormatValue::Bytes(ByteString::from(format!(
                "{lines},{},{cells},{},{extended},{}",
                u64::from(lines) * u64::from(rmux_emu::grid::LINE_BYTES),
                u64::from(cells) * u64::from(rmux_emu::grid::CELL_ENTRY_BYTES),
                u64::from(extended) * u64::from(rmux_emu::grid::EXTD_ENTRY_BYTES)
            )))
        }
        b"pane_key_mode" => bytes(match visible?.mode & ScreenMode::EXTENDED_KEY_MODES {
            ScreenMode::KEYS_EXTENDED => b"Ext 1",
            ScreenMode::KEYS_EXTENDED_2 => b"Ext 2",
            _ => b"VT10x",
        }),
        b"pane_path" => bytes(base.path.as_deref().unwrap_or_default()),
        b"pane_pb_progress" => FormatValue::Signed(i64::from(base.progress_bar.progress)),
        b"pane_pb_state" => bytes(match base.progress_bar.state {
            ProgressBarState::Hidden => b"hidden",
            ProgressBarState::Normal => b"normal",
            ProgressBarState::Error => b"error",
            ProgressBarState::Indeterminate => b"indeterminate",
            ProgressBarState::Paused => b"paused",
        }),
        b"pane_private_modes" => FormatValue::Bytes(private_modes(base.mode)),
        b"pane_tabs" => {
            let mut result = Vec::new();
            for (index, &tab) in base.tabs.iter().take(grid.sx() as usize).enumerate() {
                if tab {
                    append_list(&mut result, index.to_string().as_bytes());
                }
            }
            if result.is_empty() {
                return None;
            }
            FormatValue::Bytes(ByteString(result))
        }
        b"pane_title" => bytes(&base.title),
        b"scroll_region_lower" => FormatValue::Unsigned(u64::from(base.rlower)),
        b"scroll_region_upper" => FormatValue::Unsigned(u64::from(base.rupper)),
        _ => return None,
    })
}

fn append_list(result: &mut Vec<u8>, value: &[u8]) {
    if !result.is_empty() {
        result.push(b',');
    }
    result.extend_from_slice(cstr(value));
}

fn append_bounded(result: &mut Vec<u8>, value: &[u8], limit: usize) {
    result.extend_from_slice(&value[..value.len().min(limit.saturating_sub(result.len()))]);
}

pub fn private_modes(mode: ScreenMode) -> ByteString {
    let table = [
        (ScreenMode::KCURSOR, 1),
        (ScreenMode::ORIGIN, 6),
        (ScreenMode::WRAP, 7),
        (ScreenMode::CURSOR_BLINKING, 12),
        (ScreenMode::CURSOR, 25),
        (ScreenMode::MOUSE_STANDARD, 1000),
        (ScreenMode::MOUSE_BUTTON, 1002),
        (ScreenMode::MOUSE_ALL, 1003),
        (ScreenMode::FOCUSON, 1004),
        (ScreenMode::MOUSE_UTF8, 1005),
        (ScreenMode::MOUSE_SGR, 1006),
        (ScreenMode::BRACKETPASTE, 2004),
        (ScreenMode::SYNC, 2026),
        (ScreenMode::THEME_UPDATES, 2031),
    ];
    let mut result = Vec::new();
    for (flag, number) in table {
        if !mode.contains(flag)
            || (flag == ScreenMode::CURSOR_BLINKING
                && !mode.contains(ScreenMode::CURSOR_BLINKING_SET))
        {
            continue;
        }
        append_list(&mut result, number.to_string().as_bytes());
    }
    ByteString(result)
}

pub struct ClientFacts<'a> {
    pub flags: ClientFlags,
    pub started: bool,
    pub width: u32,
    pub height: u32,
    pub xpixel: u32,
    pub ypixel: u32,
    pub term_rgb: bool,
    pub term_256: bool,
    pub term_colours: u32,
    pub pid: i32,
    pub uid: Option<u32>,
    pub user: Option<&'a [u8]>,
    pub name: Option<&'a [u8]>,
    pub termname: Option<&'a [u8]>,
    pub termtype: Option<&'a [u8]>,
    pub tty: Option<&'a [u8]>,
    pub key_table: &'a [u8],
    pub default_key_table: &'a [u8],
    pub features: &'a [u8],
    pub session: Option<&'a [u8]>,
    pub last_session: Option<&'a [u8]>,
    pub created: Timestamp,
    pub activity: Timestamp,
    pub discarded: u64,
    pub written: u64,
    pub pause_age_ms: u32,
    pub theme: Option<&'a [u8]>,
    pub viewport: Option<(u32, u32, u32, u32)>,
}

pub fn client_value(client: &ClientFacts<'_>, key: &[u8]) -> Option<FormatValue> {
    Some(match key {
        b"client_activity" => seconds(client.activity.sec),
        b"client_created" => seconds(client.created.sec),
        b"client_cell_height" if client.started => FormatValue::Unsigned(u64::from(client.ypixel)),
        b"client_cell_width" if client.started => FormatValue::Unsigned(u64::from(client.xpixel)),
        b"client_height" if client.started => FormatValue::Unsigned(u64::from(client.height)),
        b"client_width" => FormatValue::Unsigned(u64::from(client.width)),
        b"client_colours" if client.started => FormatValue::Unsigned(if client.term_rgb {
            16777216
        } else if client.term_256 {
            256
        } else if client.term_colours < 8 {
            2
        } else if client.term_colours < 16 {
            8
        } else {
            16
        }),
        b"client_control_mode" => boolean(client.flags.contains(ClientFlags::CONTROL)),
        b"client_discarded" => FormatValue::Unsigned(client.discarded),
        b"client_written" => FormatValue::Unsigned(client.written),
        b"client_flags" => FormatValue::Bytes(client_flags(client.flags, client.pause_age_ms)),
        b"client_key_table" => bytes(client.key_table),
        b"client_prefix" => boolean(cstr(client.key_table) != cstr(client.default_key_table)),
        b"client_last_session" => bytes(client.last_session?),
        b"client_name" => bytes(client.name?),
        b"client_pid" => FormatValue::Signed(i64::from(client.pid)),
        b"client_readonly" => boolean(client.flags.contains(ClientFlags::READONLY)),
        b"client_session" => bytes(client.session?),
        b"client_termfeatures" => bytes(client.features),
        b"client_termname" => bytes(client.termname?),
        b"client_termtype" => bytes(client.termtype.unwrap_or_default()),
        b"client_theme" => bytes(client.theme?),
        b"client_tty" => bytes(client.tty?),
        b"client_uid" => FormatValue::Unsigned(u64::from(client.uid?)),
        b"client_user" => bytes(client.user?),
        b"client_utf8" => boolean(client.flags.contains(ClientFlags::UTF8)),
        b"window_bigger" => boolean(client.viewport.is_some()),
        b"window_offset_x" => FormatValue::Unsigned(u64::from(client.viewport?.0)),
        b"window_offset_y" => FormatValue::Unsigned(u64::from(client.viewport?.1)),
        _ => return None,
    })
}

pub fn client_flags(flags: ClientFlags, pause_age_ms: u32) -> ByteString {
    let mut result = Vec::new();
    for (flag, name) in [
        (ClientFlags::ATTACHED, b"attached".as_slice()),
        (ClientFlags::FOCUSED, b"focused"),
        (ClientFlags::CONTROL, b"control-mode"),
        (ClientFlags::IGNORESIZE, b"ignore-size"),
        (ClientFlags::NO_DETACH_ON_DESTROY, b"no-detach-on-destroy"),
        (ClientFlags::CONTROL_NOOUTPUT, b"no-output"),
        (ClientFlags::CONTROL_WAITEXIT, b"wait-exit"),
        (ClientFlags::CONTROL_NEWLAYOUTS, b"new-layouts"),
    ] {
        if flags.contains(flag) {
            append_list(&mut result, name);
        }
    }
    if flags.contains(ClientFlags::CONTROL_PAUSEAFTER) {
        append_list(
            &mut result,
            format!("pause-after={}", pause_age_ms / 1000).as_bytes(),
        );
    }
    for (flag, name) in [
        (ClientFlags::READONLY, b"read-only".as_slice()),
        (ClientFlags::SUSPENDED, b"suspended"),
        (ClientFlags::UTF8, b"UTF-8"),
    ] {
        if flags.contains(flag) {
            append_list(&mut result, name);
        }
    }
    result.truncate(255);
    ByteString(result)
}

pub struct ServerFacts<'a> {
    pub hostname: &'a [u8],
    pub socket: &'a [u8],
    pub config_files: &'a [ByteString],
    pub next_session_id: u32,
    pub sessions: u32,
    pub start: Timestamp,
    pub uid: u32,
    pub user: Option<&'a [u8]>,
    pub version: &'a [u8],
    pub buffer_mode_format: &'a [u8],
    pub client_mode_format: &'a [u8],
    pub tree_mode_format: &'a [u8],
}

pub fn server_value(server: &ServerFacts<'_>, key: &[u8]) -> Option<FormatValue> {
    Some(match key {
        b"host" => bytes(server.hostname),
        b"host_short" => {
            let host = cstr(server.hostname);
            bytes(&host[..host.iter().position(|&b| b == b'.').unwrap_or(host.len())])
        }
        b"socket_path" => bytes(server.socket),
        b"config_files" => {
            let mut result = Vec::new();
            for (index, path) in server.config_files.iter().enumerate() {
                if index != 0 {
                    result.push(b',');
                }
                result.extend_from_slice(cstr(path));
            }
            FormatValue::Bytes(ByteString(result))
        }
        b"next_session_id" => {
            FormatValue::Bytes(ByteString::from(format!("${}", server.next_session_id)))
        }
        b"server_sessions" => FormatValue::Unsigned(u64::from(server.sessions)),
        b"start_time" => seconds(server.start.sec),
        b"uid" => FormatValue::Unsigned(u64::from(server.uid)),
        b"user" => bytes(server.user?),
        b"version" => bytes(server.version),
        b"buffer_mode_format" => bytes(server.buffer_mode_format),
        b"client_mode_format" => bytes(server.client_mode_format),
        b"tree_mode_format" => bytes(server.tree_mode_format),
        _ => return None,
    })
}

pub enum MouseBacking<'a> {
    Base {
        grid: &'a Grid,
        screen: &'a Screen,
        registry: &'a HyperlinkRegistry,
    },
    Copy {
        grid: &'a Grid,
        screen: &'a Screen,
        registry: &'a HyperlinkRegistry,
        first_row: u32,
    },
    Unsupported,
}

pub struct MouseFacts<'a> {
    pub valid: bool,
    pub pane_public_id: Option<u32>,
    pub pane_coordinates: Option<(u32, u32)>,
    pub x: u32,
    pub y: u32,
    pub status_at: i32,
    pub status_lines: u32,
    pub client_started: bool,
    pub status_range: Option<&'a rmux_emu::style::StyleRange>,
    pub backing: MouseBacking<'a>,
    pub word_separators: &'a [u8],
}

pub fn mouse_value(mouse: &MouseFacts<'_>, key: &[u8]) -> Option<FormatValue> {
    if !mouse.valid {
        return None;
    }
    let status_y = if !mouse.client_started {
        None
    } else if mouse.status_at == 0 && mouse.y < mouse.status_lines {
        Some(mouse.y)
    } else if mouse.status_at > 0 && mouse.y >= mouse.status_at as u32 {
        Some(mouse.y - mouse.status_at as u32)
    } else {
        None
    };
    match key {
        b"mouse_pane" => Some(FormatValue::Bytes(ByteString::from(format!(
            "%{}",
            mouse.pane_public_id?
        )))),
        b"mouse_x" => Some(FormatValue::Unsigned(u64::from(
            mouse
                .pane_coordinates
                .map(|p| p.0)
                .or(status_y.map(|_| mouse.x))?,
        ))),
        b"mouse_y" => Some(FormatValue::Unsigned(u64::from(
            mouse.pane_coordinates.map(|p| p.1).or(status_y)?,
        ))),
        b"mouse_status_line" => Some(FormatValue::Unsigned(u64::from(status_y?))),
        b"mouse_status_range" => {
            status_y?;
            let range = mouse.status_range?;
            use rmux_emu::style::StyleRangeType;
            Some(bytes(match range.range_type {
                StyleRangeType::None => return None,
                StyleRangeType::Left => b"left",
                StyleRangeType::Right => b"right",
                StyleRangeType::Pane => b"pane",
                StyleRangeType::Window => b"window",
                StyleRangeType::Session => b"session",
                StyleRangeType::User => &range.string,
                StyleRangeType::Control => b"control",
            }))
        }
        b"mouse_word" | b"mouse_line" | b"mouse_hyperlink" => {
            mouse.pane_public_id?;
            let (x, y) = mouse.pane_coordinates?;
            let (grid, screen, registry, y) = match &mouse.backing {
                MouseBacking::Base {
                    grid,
                    screen,
                    registry,
                } => (*grid, *screen, *registry, grid.hsize() + y),
                MouseBacking::Copy {
                    grid,
                    screen,
                    registry,
                    first_row,
                } => (*grid, *screen, *registry, first_row.checked_add(y)?),
                MouseBacking::Unsupported => return None,
            };
            let value = match key {
                b"mouse_word" => super::grid::word(grid, x, y, mouse.word_separators),
                b"mouse_line" => super::grid::line(grid, y),
                _ => super::grid::hyperlink(grid, x, y, screen, registry),
            }?;
            Some(FormatValue::Bytes(value))
        }
        _ => None,
    }
}

fn number_option(
    server: &crate::model::Server,
    options: crate::ids::OptionsId,
    key: &[u8],
) -> Option<i64> {
    server.options.get(options, key)?.1.value().as_number()
}

fn argv_string(argv: &[Vec<u8>], single_quotes: bool) -> ByteString {
    let mut result = Vec::new();
    for (index, arg) in argv.iter().enumerate() {
        if index != 0 {
            result.push(b' ');
        }
        let quoted = if single_quotes {
            quote_single(arg)
        } else {
            crate::cmd::arguments::escape(arg)
        };
        result.extend_from_slice(&quoted);
    }
    ByteString(result)
}

fn marked_valid(server: &crate::model::Server) -> bool {
    let Some(link_id) = server.marked_winlink else {
        return false;
    };
    let Some(pane_id) = server.marked_pane else {
        return false;
    };
    let Some(link) = server.winlinks.get(link_id) else {
        return false;
    };
    let Some(session) = server.sessions.get(link.session) else {
        return false;
    };
    let Some(window) = server.windows.get(link.window) else {
        return false;
    };
    server.session_names.get(&session.name) == Some(&link.session)
        && session.windows.values().any(|id| *id == link_id)
        && server.panes.get(pane_id).is_some()
        && window.panes.contains(&pane_id)
}

pub fn model_value(
    server: &crate::model::Server,
    context: &FormatContext,
    key: &[u8],
) -> Option<FormatValue> {
    let session = context.session.and_then(|id| server.sessions.get(id));
    let link = context.winlink.and_then(|id| server.winlinks.get(id));
    let window = context.window.and_then(|id| server.windows.get(id));
    let pane = context.pane.and_then(|id| server.panes.get(id));
    let marked_valid = matches!(
        key,
        b"session_marked"
            | b"window_marked_flag"
            | b"window_flags"
            | b"window_raw_flags"
            | b"pane_marked"
            | b"pane_marked_set"
    ) && marked_valid(server);
    if let Some(pane) = pane {
        if let Some(value) = screen_value(&pane.base, Some(pane.displayed_screen()), key) {
            return Some(value);
        }
    }
    Some(match key {
        b"server_sessions" => FormatValue::Unsigned(server.sessions.len() as u64),
        b"next_session_id" => {
            FormatValue::Bytes(ByteString::from(format!("${}", server.next_session_id)))
        }
        b"socket_path" => bytes(&server.socket_path),
        b"pane_format" => boolean(context.kind == FormatKind::Pane),
        b"window_format" => boolean(context.kind == FormatKind::Window),
        b"session_format" => boolean(context.kind == FormatKind::Session),
        b"buffer_created" => seconds(server.paste.get(context.buffer?)?.created),
        b"buffer_full" => bytes(&server.paste.get(context.buffer?)?.data),
        b"buffer_name" => bytes(&server.paste.get(context.buffer?)?.name),
        b"buffer_size" => {
            FormatValue::Unsigned(server.paste.get(context.buffer?)?.data.len() as u64)
        }
        b"buffer_sample" => {
            let mut sample = Vec::new();
            crate::model::paste::paste_make_sample(server.paste.get(context.buffer?)?, &mut sample);
            FormatValue::Bytes(ByteString(sample))
        }
        b"active_window_index" => {
            FormatValue::Unsigned(server.winlinks.get(session?.current?)?.index as u32 as u64)
        }
        b"last_window_index" => {
            FormatValue::Unsigned(*session?.windows.last_key_value()?.0 as u32 as u64)
        }
        b"session_activity" => timestamp(session?.activity),
        b"session_created" => timestamp(session?.created),
        b"session_last_attached" => timestamp(session?.last_attached),
        b"session_attached" => FormatValue::Unsigned(u64::from(session?.attached)),
        b"session_many_attached" => boolean(session?.attached > 1),
        b"session_grouped" => boolean(session?.group.is_some()),
        b"session_group" => bytes(&server.groups.get(session?.group?)?.name),
        b"session_group_size" => {
            FormatValue::Unsigned(server.groups.get(session?.group?)?.sessions.len() as u64)
        }
        b"session_group_attached" | b"session_group_many_attached" => {
            let group = server.groups.get(session?.group?)?;
            let count = group
                .sessions
                .iter()
                .filter_map(|id| server.sessions.get(*id))
                .fold(0u32, |count, s| count.wrapping_add(s.attached));
            if key == b"session_group_attached" {
                FormatValue::Unsigned(u64::from(count))
            } else {
                boolean(count > 1)
            }
        }
        b"session_group_list" => {
            let group = server.groups.get(session?.group?)?;
            let mut result = Vec::new();
            for s in group
                .sessions
                .iter()
                .filter_map(|id| server.sessions.get(*id))
            {
                append_list(&mut result, &s.name);
            }
            if result.is_empty() {
                return None;
            }
            FormatValue::Bytes(ByteString(result))
        }
        b"session_id" => FormatValue::Bytes(ByteString::from(format!("${}", session?.public_id))),
        b"session_name" => bytes(&session?.name),
        b"session_path" => bytes(&session?.cwd),
        b"session_windows" => FormatValue::Unsigned(session?.windows.len() as u64),
        b"session_marked" => {
            session?;
            boolean(
                marked_valid
                    && server
                        .marked_winlink
                        .and_then(|id| server.winlinks.get(id))
                        .is_some_and(|marked| Some(marked.session) == context.session),
            )
        }
        b"session_activity_flag" | b"session_bell_flag" | b"session_silence_flag" => {
            let mask = match key {
                b"session_activity_flag" => WinlinkFlags::ACTIVITY,
                b"session_bell_flag" => WinlinkFlags::BELL,
                _ => WinlinkFlags::SILENCE,
            };
            boolean(
                session?
                    .windows
                    .values()
                    .filter_map(|id| server.winlinks.get(*id))
                    .any(|wl| wl.flags.contains(mask)),
            )
        }
        b"session_alert" | b"session_alerts" => {
            let mut result = Vec::new();
            let mut seen = WinlinkFlags::default();
            for wl in session?
                .windows
                .values()
                .filter_map(|id| server.winlinks.get(*id))
            {
                if !wl.flags.intersects(WinlinkFlags::ALERTFLAGS) {
                    continue;
                }
                if key == b"session_alerts" {
                    if !result.is_empty() {
                        append_bounded(&mut result, b",", 1023);
                    }
                    append_bounded(&mut result, (wl.index as u32).to_string().as_bytes(), 1023);
                }
                for (flag, symbol) in [
                    (WinlinkFlags::ACTIVITY, b"#".as_slice()),
                    (WinlinkFlags::BELL, b"!"),
                    (WinlinkFlags::SILENCE, b"~"),
                ] {
                    if wl.flags.contains(flag) && (key == b"session_alerts" || !seen.contains(flag))
                    {
                        append_bounded(&mut result, symbol, 1023);
                        seen.insert(flag);
                    }
                }
            }
            FormatValue::Bytes(ByteString(result))
        }
        b"session_stack" => {
            let session = session?;
            let mut result = (server.winlinks.get(session.current?)?.index as u32)
                .to_string()
                .into_bytes();
            for wl in session
                .last
                .iter()
                .filter_map(|id| server.winlinks.get(*id))
            {
                if !result.is_empty() {
                    append_bounded(&mut result, b",", 1023);
                }
                append_bounded(&mut result, (wl.index as u32).to_string().as_bytes(), 1023);
            }
            FormatValue::Bytes(ByteString(result))
        }
        b"window_activity" => timestamp(window?.activity),
        b"window_cell_height" => FormatValue::Unsigned(u64::from(window?.ypixel)),
        b"window_cell_width" => FormatValue::Unsigned(u64::from(window?.xpixel)),
        b"window_height" => FormatValue::Unsigned(u64::from(window?.sy)),
        b"window_width" => FormatValue::Unsigned(u64::from(window?.sx)),
        b"window_id" => FormatValue::Bytes(ByteString::from(format!("@{}", window?.public_id))),
        b"window_name" => bytes(&window?.name),
        b"window_panes" => FormatValue::Unsigned(window?.panes.len() as u64),
        b"window_modal_pane" => FormatValue::Bytes(ByteString::from(format!(
            "%{}",
            server.panes.get(window?.modal?)?.public_id
        ))),
        b"window_zoomed_flag" => boolean(window?.flags.contains(WindowFlags::ZOOMED)),
        b"window_manual_height" | b"window_manual_width" => {
            let window = window?;
            if number_option(server, window.options, b"window-size")? != 0 {
                bytes(b"")
            } else {
                FormatValue::Unsigned(u64::from(if key == b"window_manual_height" {
                    window.manual_sy
                } else {
                    window.manual_sx
                }))
            }
        }
        b"window_index" => FormatValue::Signed(i64::from(link?.index)),
        b"window_activity_flag" => boolean(link?.flags.contains(WinlinkFlags::ACTIVITY)),
        b"window_bell_flag" => boolean(link?.flags.contains(WinlinkFlags::BELL)),
        b"window_silence_flag" => boolean(link?.flags.contains(WinlinkFlags::SILENCE)),
        b"window_active" => boolean(server.sessions.get(link?.session)?.current == context.winlink),
        b"window_last_flag" => {
            boolean(server.sessions.get(link?.session)?.last.first().copied() == context.winlink)
        }
        b"window_start_flag" => boolean(
            server
                .sessions
                .get(link?.session)?
                .windows
                .first_key_value()
                .map(|(_, id)| *id)
                == context.winlink,
        ),
        b"window_end_flag" => boolean(
            server
                .sessions
                .get(link?.session)?
                .windows
                .last_key_value()
                .map(|(_, id)| *id)
                == context.winlink,
        ),
        b"window_stack_index" => {
            let index = server
                .sessions
                .get(link?.session)?
                .last
                .iter()
                .position(|id| Some(*id) == context.winlink)
                .map_or(0, |index| index + 1);
            FormatValue::Unsigned(index as u64)
        }
        b"window_marked_flag" => {
            link?;
            boolean(marked_valid && server.marked_winlink == context.winlink)
        }
        b"window_linked" => {
            let target = link?.window;
            let count = server
                .session_names
                .values()
                .filter_map(|id| server.sessions.get(*id))
                .flat_map(|s| s.windows.values())
                .filter_map(|id| server.winlinks.get(*id))
                .filter(|wl| wl.window == target)
                .take(2)
                .count();
            boolean(count > 1)
        }
        b"window_linked_sessions" => {
            let target = link?.window;
            let has_window = |s: &crate::model::Session| {
                s.windows
                    .values()
                    .filter_map(|id| server.winlinks.get(*id))
                    .any(|wl| wl.window == target)
            };
            let groups = server
                .group_names
                .values()
                .filter_map(|id| server.groups.get(*id))
                .filter_map(|g| g.sessions.first())
                .filter_map(|id| server.sessions.get(*id))
                .filter(|s| has_window(s))
                .count();
            let ungrouped = server
                .session_names
                .values()
                .filter_map(|id| server.sessions.get(*id))
                .filter(|s| s.group.is_none() && has_window(s))
                .count();
            FormatValue::Unsigned((groups + ungrouped) as u64)
        }
        b"window_linked_sessions_list"
        | b"window_active_sessions_list"
        | b"window_active_sessions" => {
            let linked_window = server.windows.get(link?.window)?;
            let mut result = Vec::new();
            let mut count = 0;
            for (id, wl) in linked_window
                .links
                .iter()
                .filter_map(|id| server.winlinks.get(*id).map(|wl| (*id, wl)))
            {
                let s = server.sessions.get(wl.session)?;
                if key != b"window_linked_sessions_list" && s.current != Some(id) {
                    continue;
                }
                count += 1;
                if key != b"window_active_sessions" {
                    append_list(&mut result, &s.name);
                }
            }
            if key == b"window_active_sessions" {
                FormatValue::Unsigned(count)
            } else {
                if result.is_empty() {
                    return None;
                }
                FormatValue::Bytes(ByteString(result))
            }
        }
        b"window_flags" | b"window_raw_flags" => {
            let link = link?;
            let s = server.sessions.get(link.session)?;
            let w = server.windows.get(link.window)?;
            let mut result = Vec::new();
            if link.flags.contains(WinlinkFlags::ACTIVITY) {
                result.extend_from_slice(if key == b"window_flags" { b"##" } else { b"#" });
            }
            for (flag, byte) in [(WinlinkFlags::BELL, b'!'), (WinlinkFlags::SILENCE, b'~')] {
                if link.flags.contains(flag) {
                    result.push(byte);
                }
            }
            if s.current == context.winlink {
                result.push(b'*');
            }
            if s.last.first().copied() == context.winlink {
                result.push(b'-');
            }
            if marked_valid && server.marked_winlink == context.winlink {
                result.push(b'M');
            }
            if w.modal.is_some() {
                result.push(b'O');
            }
            if w.flags.contains(WindowFlags::ZOOMED) {
                result.push(b'Z');
            }
            FormatValue::Bytes(ByteString(result))
        }
        b"pane_active" => boolean(server.windows.get(pane?.window)?.active == context.pane),
        b"pane_modal_flag" => boolean(server.windows.get(pane?.window)?.modal == context.pane),
        b"pane_last" => {
            boolean(server.windows.get(pane?.window)?.last.first().copied() == context.pane)
        }
        b"pane_height" => FormatValue::Unsigned(u64::from(pane?.sy)),
        b"pane_width" => FormatValue::Unsigned(u64::from(pane?.sx)),
        b"pane_id" => FormatValue::Bytes(ByteString::from(format!("%{}", pane?.public_id))),
        b"pane_index" => FormatValue::Unsigned(u64::from(crate::model::pane::pane_index(
            server,
            context.pane?,
        )?)),
        b"pane_z" => FormatValue::Unsigned(u64::from(crate::model::pane::pane_z_index(
            server,
            context.pane?,
        )?)),
        b"pane_left" | b"pane_x" => FormatValue::Signed(i64::from(pane?.xoff)),
        b"pane_top" | b"pane_y" => FormatValue::Signed(i64::from(pane?.yoff)),
        b"pane_right" => FormatValue::Signed(i64::from(
            pane?.xoff.wrapping_add(pane?.sx as i32).wrapping_sub(1),
        )),
        b"pane_bottom" => FormatValue::Signed(i64::from(
            pane?.yoff.wrapping_add(pane?.sy as i32).wrapping_sub(1),
        )),
        b"pane_at_left" => boolean(pane?.xoff == 0),
        b"pane_at_right" => boolean(
            pane?.xoff.wrapping_add(pane?.sx as i32) == server.windows.get(pane?.window)?.sx as i32,
        ),
        b"pane_at_top" => boolean(
            pane?.yoff
                == i32::from(crate::model::pane::pane_get_pane_status(server, context.pane?) == 1),
        ),
        b"pane_at_bottom" => {
            let status =
                i32::from(crate::model::pane::pane_get_pane_status(server, context.pane?) == 2);
            boolean(
                pane?.yoff.wrapping_add(pane?.sy as i32)
                    == (server.windows.get(pane?.window)?.sy as i32).wrapping_sub(status),
            )
        }
        b"pane_unzoomed_height" => {
            let pane = pane?;
            let window = server.windows.get(pane.window)?;
            let cell_id = pane.saved_layout_cell.or(pane.layout_cell)?;
            let cell = server.layout_cells.get(cell_id)?;
            let floating = cell.is_floating();
            let status = if Some(cell_id) == pane.saved_layout_cell && !floating {
                crate::model::window::window_get_pane_status(server, pane.window)
            } else {
                crate::model::pane::pane_get_pane_status(server, context.pane?)
            };
            let status = crate::ui::status::PaneStatusPosition::try_from(status as i32).ok()?;
            let mut sy = cell.g.sy;
            if !floating
                && sy > 1
                && crate::layout::add_horizontal_border(
                    &server.layout_cells,
                    window.saved_layout_root.or(window.layout_root),
                    cell_id,
                    status,
                )
            {
                sy -= 1;
            }
            FormatValue::Unsigned(u64::from(sy))
        }
        b"pane_unzoomed_width" => {
            let pane = pane?;
            let window = server.windows.get(pane.window)?;
            let saved = pane.saved_layout_cell.is_some();
            let cell = server
                .layout_cells
                .get(pane.saved_layout_cell.or(pane.layout_cell)?)?;
            let mut sx = cell.g.sx;
            if (saved
                && pane.base.saved_grid.is_none()
                && window.sb == crate::ui::scrollbar::PaneScrollbarPolicy::Always)
                || (!saved && crate::model::pane::pane_scrollbar_reserve(server, context.pane?))
            {
                let width = pane.scrollbar_style.width.max(1);
                let pad = pane.scrollbar_style.pad.max(0);
                sx = (i64::from(sx) - i64::from(width) - i64::from(pad)).max(1) as u32;
            }
            FormatValue::Unsigned(u64::from(sx))
        }
        b"client_tsp" => bytes(
            server
                .clients
                .get(context.evaluated_client?)?
                .tsp
                .format()
                .as_bytes(),
        ),
        b"pane_tsp" => bytes(
            pane?
                .tsp
                .as_ref()
                .map_or("ansi", |tsp| tsp.format())
                .as_bytes(),
        ),
        b"pane_tsp_epoch" => FormatValue::Unsigned(pane?.tsp.as_ref().map_or(0, |tsp| tsp.epoch)),
        b"pane_in_mode" => FormatValue::Unsigned(pane?.modes.len() as u64),
        b"pane_mode" => bytes(&pane?.modes.first()?.name),
        b"pane_floating_flag" => {
            pane?;
            boolean(crate::model::pane::pane_is_floating(server, context.pane?))
        }
        b"pane_input_off" => boolean(pane?.flags.contains(PaneFlags::INPUTOFF)),
        b"pane_unseen_changes" => boolean(pane?.flags.contains(PaneFlags::UNSEENCHANGES)),
        b"pane_zoomed_flag" => boolean(pane?.flags.contains(PaneFlags::ZOOMED)),
        b"pane_marked" => {
            pane?;
            boolean(marked_valid && server.marked_pane == context.pane)
        }
        b"pane_marked_set" => {
            pane?;
            boolean(marked_valid)
        }
        b"pane_synchronized" => {
            boolean(number_option(server, pane?.options, b"synchronize-panes")? != 0)
        }
        b"pane_start_path" => bytes(&pane?.cwd),
        b"pane_start_command" => FormatValue::Bytes(argv_string(&pane?.argv, false)),
        b"pane_start_command_list" => FormatValue::Bytes(argv_string(&pane?.argv, true)),
        b"pane_current_path" => {
            use std::os::fd::AsFd;
            bytes(&rmux_sys::osdep::get_cwd(pane?.fd.as_ref()?.as_fd())?)
        }
        b"pane_current_command" => {
            use std::os::fd::AsFd;
            let pane = pane?;
            let name = pane
                .fd
                .as_ref()
                .and_then(|fd| rmux_sys::osdep::get_name(fd.as_fd()))
                .filter(|name| !name.is_empty());
            let command = name
                .map(ByteString)
                .unwrap_or_else(|| argv_string(&pane.argv, false));
            let command = if command.is_empty() {
                pane.shell.as_slice()
            } else {
                &command
            };
            FormatValue::Bytes(ByteString(crate::model::names::parse_window_name(command)))
        }
        b"pane_tty" => bytes(&pane?.tty),
        b"pane_pid" => {
            if !pane?.has_fd() {
                return None;
            }
            FormatValue::Signed(i64::from(pane?.pid?.0))
        }
        b"pane_search_string" => bytes(pane?.searchstr.as_deref().unwrap_or_default()),
        b"pane_dead" => boolean(!pane?.has_fd() && pane?.flags.contains(PaneFlags::STATUSREADY)),
        b"pane_dead_status" => {
            let pane = pane?;
            if !pane.flags.contains(PaneFlags::STATUSREADY) {
                return None;
            }
            FormatValue::Signed(i64::from(rmux_sys::proc::wait_exit_status(pane.status)?))
        }
        b"pane_dead_signal" => {
            let pane = pane?;
            if !pane.flags.contains(PaneFlags::STATUSREADY) {
                return None;
            }
            bytes(&rmux_sys::proc::wait_signal_name(pane.status)?)
        }
        b"pane_dead_time" => {
            if !pane?.flags.contains(PaneFlags::STATUSDRAWN) {
                return None;
            }
            timestamp(pane?.dead_time)
        }
        b"pane_last_output_time" => {
            let time = pane?.last_output_time;
            if time == 0 {
                return None;
            }
            seconds(time)
        }
        b"pane_last_prompt_time" => {
            let time = pane?.last_prompt_time;
            if time == 0 {
                return None;
            }
            seconds(time)
        }
        b"pane_command_start_time" => {
            let time = pane?.cmd_start_time;
            if time == 0 {
                return None;
            }
            seconds(time)
        }
        b"pane_command_end_time" => {
            let time = pane?.cmd_end_time;
            if time == 0 {
                return None;
            }
            seconds(time)
        }
        b"pane_output_generation" => FormatValue::Unsigned(pane?.output_generation),
        b"pane_command_running" => boolean(pane?.flags.contains(PaneFlags::CMDRUNNING)),
        b"pane_command_status" => {
            if pane?.cmd_status == -1 {
                return None;
            }
            FormatValue::Signed(i64::from(pane?.cmd_status))
        }
        b"pane_command_duration" => {
            let pane = pane?;
            if pane.cmd_start_time == 0 {
                return None;
            }
            let end = if pane.flags.contains(PaneFlags::CMDRUNNING) {
                server.current_time.0
            } else {
                pane.cmd_end_time
            };
            FormatValue::Signed(
                end.max(pane.cmd_start_time)
                    .wrapping_sub(pane.cmd_start_time),
            )
        }
        b"pane_flags" => {
            let pane = pane?;
            let w = server.windows.get(pane.window)?;
            let mut result = Vec::new();
            for (condition, byte) in [
                (w.active == context.pane, b'*'),
                (w.last.first().copied() == context.pane, b'-'),
                (pane.flags.contains(PaneFlags::ZOOMED), b'Z'),
                (
                    crate::model::pane::pane_is_floating(server, context.pane?),
                    b'F',
                ),
                (pane.flags.contains(PaneFlags::FLOATOVERZOOM), b'A'),
                (w.modal == context.pane, b'O'),
            ] {
                if condition {
                    result.push(byte);
                }
            }
            FormatValue::Bytes(ByteString(result))
        }
        _ => return None,
    })
}

pub fn model_layout_value(
    server: &crate::model::Server,
    context: &FormatContext,
    owner_flags: ClientFlags,
    key: &[u8],
) -> Option<FormatValue> {
    let window_id = context.window?;
    let window = server.windows.get(window_id)?;
    let root = match key {
        b"window_layout" => window.saved_layout_root.or(window.layout_root),
        b"window_visible_layout" => window.layout_root,
        _ => return None,
    };
    let flags = if owner_flags.contains(ClientFlags::CONTROL)
        && !owner_flags.contains(ClientFlags::CONTROL_NEWLAYOUTS)
    {
        crate::layout::LayoutDumpFlags::OLD_FORMAT
    } else {
        crate::layout::LayoutDumpFlags::default()
    };
    crate::layout::dump(server, window_id, root, flags).map(FormatValue::Bytes)
}

pub struct ClientLinkFacts<'a> {
    pub id: crate::ids::ClientId,
    pub session: Option<crate::ids::SessionId>,
    pub name: &'a [u8],
}

pub fn client_model_value(
    server: &crate::model::Server,
    context: &FormatContext,
    clients: &[ClientLinkFacts<'_>],
    key: &[u8],
) -> Option<FormatValue> {
    let session = context.session.and_then(|id| server.sessions.get(id));
    match key {
        b"session_active" => {
            session?;
            let client = clients
                .iter()
                .find(|client| Some(client.id) == context.evaluated_client)?;
            Some(boolean(client.session == context.session))
        }
        b"session_attached_list"
        | b"session_group_attached_list"
        | b"window_active_clients_list"
        | b"window_active_clients" => {
            let target_window = if key.starts_with(b"window_") {
                Some(server.winlinks.get(context.winlink?)?.window)
            } else {
                None
            };
            let group = if key == b"session_group_attached_list" {
                Some(server.groups.get(session?.group?)?)
            } else {
                None
            };
            if key == b"session_attached_list" {
                session?;
            }
            let mut result = Vec::new();
            let mut count = 0;
            for client in clients {
                let matches = if let Some(window) = target_window {
                    client
                        .session
                        .and_then(|id| server.sessions.get(id))
                        .and_then(|s| s.current)
                        .and_then(|id| server.winlinks.get(id))
                        .is_some_and(|wl| wl.window == window)
                } else if let Some(group) = group {
                    client
                        .session
                        .is_some_and(|id| group.sessions.contains(&id))
                } else {
                    client.session == context.session
                };
                if matches {
                    count += 1;
                    if key != b"window_active_clients" {
                        append_list(&mut result, client.name);
                    }
                }
            }
            if key == b"window_active_clients" {
                Some(FormatValue::Unsigned(count))
            } else if result.is_empty() {
                None
            } else {
                Some(FormatValue::Bytes(ByteString(result)))
            }
        }
        _ => None,
    }
}

pub struct PaneAuxFacts<'a> {
    pub mode_names: &'a [&'a [u8]],
    pub pipe_pid: Option<rmux_sys::ProcessId>,
    pub default_fg: Colour,
    pub default_bg: Colour,
    pub status_ready: bool,
    pub exit_status: Option<i32>,
    pub signal_name: Option<&'a [u8]>,
}

pub fn pane_aux_value(facts: &PaneAuxFacts<'_>, key: &[u8]) -> Option<FormatValue> {
    Some(match key {
        b"pane_in_mode" => FormatValue::Unsigned(facts.mode_names.len() as u64),
        b"pane_mode" => bytes(facts.mode_names.first()?),
        b"pane_pipe" => boolean(facts.pipe_pid.is_some()),
        b"pane_pipe_pid" => FormatValue::Signed(i64::from(facts.pipe_pid?.0)),
        b"pane_dead_status" if facts.status_ready => {
            FormatValue::Signed(i64::from(facts.exit_status?))
        }
        b"pane_dead_signal" if facts.status_ready => bytes(facts.signal_name?),
        b"pane_fg" | b"pane_bg" => {
            let mut result = Vec::new();
            rmux_emu::colour::write_colour(
                if key == b"pane_fg" {
                    facts.default_fg
                } else {
                    facts.default_bg
                },
                &mut result,
            );
            FormatValue::Bytes(ByteString(result))
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmux_emu::{cell::GridCell, screen::ScreenResetPolicy};
    use rmux_util::utf8::Utf8Data;

    #[test]
    fn sixel_support_reports_build_feature_not_client_capability() {
        let mut server = crate::model::Server::new();
        let value = find(&mut server, &FormatContext::default(), b"sixel_support").unwrap();
        assert_eq!(
            value.bytes(),
            if cfg!(feature = "sixel") { b"1" } else { b"0" }
        );
    }

    fn screen() -> Screen {
        Screen::new(
            12,
            3,
            100,
            ScreenResetPolicy::default(),
            &mut HyperlinkRegistry::new(),
        )
        .unwrap()
    }

    #[test]
    fn registry_matches_all_pinned_names_and_types() {
        assert_eq!(REGISTRY.len(), 217);
        assert!(REGISTRY.windows(2).all(|keys| keys[0] < keys[1]));
        assert!(
            REGISTRY
                .iter()
                .zip(BUILTINS)
                .all(|(name, builtin)| *name == builtin.key)
        );
        let output = std::process::Command::new("git")
            .args(["-C", "/Users/j/fun/tmux", "show", "8f25579c:format.c"])
            .output();
        let Ok(output) = output else {
            eprintln!("skipping pinned registry comparison: git unavailable");
            return;
        };
        if !output.status.success() {
            eprintln!("skipping pinned registry comparison: tmux 8f25579c source unavailable");
            return;
        }
        let source = String::from_utf8(output.stdout).unwrap();
        let table = source
            .split("static const struct format_table_entry format_table[] = {")
            .nth(1)
            .unwrap()
            .split("};")
            .next()
            .unwrap();
        let parsed: Vec<_> = table
            .lines()
            .filter_map(|line| {
                let line = line.trim().strip_prefix("{ \"")?;
                let (name, tail) = line.split_once('\"')?;
                Some((
                    name.as_bytes(),
                    if tail.contains("FORMAT_TABLE_TIME") {
                        ValueKind::Time
                    } else {
                        ValueKind::String
                    },
                ))
            })
            .collect();
        let pinned: Vec<_> = BUILTINS
            .iter()
            .filter(|builtin| {
                !matches!(builtin.key, b"client_tsp" | b"pane_tsp" | b"pane_tsp_epoch")
            })
            .collect();
        assert_eq!(parsed.len(), pinned.len());
        for ((name, kind), builtin) in parsed.into_iter().zip(pinned) {
            assert_eq!(name, builtin.key);
            assert_eq!(kind, builtin.kind);
        }
    }

    #[test]
    fn broker_formats_are_read_only_string_builtins() {
        for key in [b"client_tsp".as_slice(), b"pane_tsp", b"pane_tsp_epoch"] {
            assert!(REGISTRY.contains(&key));
            assert_eq!(
                BUILTINS
                    .iter()
                    .find(|builtin| builtin.key == key)
                    .unwrap()
                    .kind,
                ValueKind::String
            );
            assert!(crate::options::search(key).is_none());
        }
    }

    #[test]
    fn quote_helpers_preserve_bytes_and_c_string_boundaries() {
        assert_eq!(quote_shell(b"a |&;<>(){}$`\\\"'*?[# =%\n\t\xff\0end"),
            ByteString::from(b"a\\ \\|\\&\\;\\<\\>\\(\\)\\{\\}\\$\\`\\\\\\\"\\'\\*\\?\\[\\#\\ \\=\\%\\\n\\\t\xff".as_slice()));
        assert_eq!(quote_single(b"a'b\0z"), ByteString::from("'a'\\''b'"));
        assert_eq!(quote_single(b""), ByteString::from("''"));
        assert_eq!(quote_style(b"###[x]\0z"), ByteString::from("######[x]"));
    }

    #[test]
    fn relative_and_difference_time_boundaries() {
        for (age, expected) in [
            (0, "0s"),
            (1, "1s"),
            (59, "59s"),
            (60, "1m"),
            (61, "1m1s"),
            (3599, "59m59s"),
            (3600, "1h"),
            (3660, "1h1m"),
            (86400, "1d"),
            (90000, "1d1h"),
            (86461, "1d"),
        ] {
            assert_eq!(
                relative_time(100_000 - age, 100_000),
                Some(ByteString::from(expected))
            );
        }
        assert_eq!(relative_time(2, 1), None);
        assert_eq!(difference_time(2, 1), ByteString::from("-1"));
    }

    #[test]
    fn pretty_time_uses_pinned_month_year_bands() {
        let now = 1_782_864_000;
        for (value, seconds) in [
            (now + 1, false),
            (now, true),
            (now - 86399, false),
            (now - 86400, false),
            (now - 27 * 86400, false),
            (now - 29 * 86400, false),
            (now - 200 * 86400, false),
            (now - 370 * 86400, false),
        ] {
            let time = rmux_sys::time::localtime(value).unwrap();
            let current = rmux_sys::time::localtime(now.max(value)).unwrap();
            let age = now.max(value) - value;
            let format: &[u8] = if age < 86400 {
                if seconds { b"%H:%M:%S" } else { b"%H:%M" }
            } else if (time.year() == current.year() && time.mon() == current.mon())
                || age < 28 * 86400
            {
                b"%a%d"
            } else if (time.year() == current.year() && time.mon() < current.mon())
                || (time.year() == current.year() - 1 && time.mon() > current.mon())
            {
                b"%d%b"
            } else {
                b"%h%y"
            };
            let mut expected = [0; 9];
            let length = rmux_sys::time::strftime(&mut expected, format, &time);
            assert_eq!(
                pretty_time_at(value, now, seconds),
                ByteString::from(&expected[..length])
            );
        }
    }

    #[test]
    fn screen_callbacks_distinguish_base_and_visible_modes() {
        let mut base = screen();
        let mut visible = screen();
        base.mode = ScreenMode::INSERT | ScreenMode::KCURSOR | ScreenMode::MOUSE_BUTTON;
        visible.mode =
            ScreenMode::BRACKETPASTE | ScreenMode::KEYS_EXTENDED_2 | ScreenMode::CURSOR_BLINKING;
        base.cx = 2;
        base.cy = 1;
        visible.cx = 9;
        visible.cstyle = ScreenCursorStyle::Bar;
        assert_eq!(
            screen_value(&base, Some(&visible), b"cursor_x")
                .unwrap()
                .bytes(),
            b"2"
        );
        assert_eq!(
            screen_value(&base, Some(&visible), b"cursor_flag")
                .unwrap()
                .bytes(),
            b"0"
        );
        assert_eq!(
            screen_value(&base, Some(&visible), b"insert_flag")
                .unwrap()
                .bytes(),
            b"1"
        );
        assert_eq!(
            screen_value(&base, Some(&visible), b"bracket_paste_flag")
                .unwrap()
                .bytes(),
            b"1"
        );
        assert_eq!(
            screen_value(&base, Some(&visible), b"pane_key_mode")
                .unwrap()
                .bytes(),
            b"Ext 2"
        );
        assert_eq!(
            screen_value(&base, Some(&visible), b"cursor_shape")
                .unwrap()
                .bytes(),
            b"bar"
        );
        assert!(screen_value(&base, None, b"cursor_shape").is_none());
        let cell = GridCell {
            data: Utf8Data::set(b'x'),
            ..GridCell::default()
        };
        base.grid.set_cell(2, 1, &cell);
        assert_eq!(
            screen_value(&base, Some(&visible), b"cursor_character")
                .unwrap()
                .bytes(),
            b"x"
        );
        base.grid.set_cell(
            2,
            1,
            &GridCell {
                flags: GridCellFlags::PADDING,
                ..GridCell::default()
            },
        );
        assert!(screen_value(&base, Some(&visible), b"cursor_character").is_none());
    }

    #[test]
    fn history_accounting_uses_c_storage_capacities() {
        let mut base = screen();
        base.grid.set_cell(
            0,
            0,
            &GridCell {
                data: Utf8Data::set(b'a'),
                ..GridCell::default()
            },
        );
        let mut wide = Utf8Data {
            size: 4,
            have: 4,
            width: 2,
            ..Utf8Data::default()
        };
        wide.data[..4].copy_from_slice(b"\xf0\x9f\x98\x80");
        base.grid.set_cell(
            1,
            1,
            &GridCell {
                data: wide,
                ..GridCell::default()
            },
        );
        base.grid.scroll_added = 9;
        base.grid.scroll_collected = 3;
        base.grid.scroll_generation = 2;
        let (lines, compact, extended) = base.grid.storage_counts();
        let expected = format!(
            "{lines},{},{compact},{},{extended},{}",
            u64::from(lines) * 40,
            u64::from(compact) * 5,
            u64::from(extended) * 23
        );
        assert_eq!(
            screen_value(&base, Some(&base), b"history_all_bytes")
                .unwrap()
                .bytes(),
            expected.as_bytes()
        );
        assert_eq!(
            screen_value(&base, Some(&base), b"history_bytes")
                .unwrap()
                .bytes(),
            (u64::from(lines) * 40 + u64::from(compact) * 5 + u64::from(extended) * 23)
                .to_string()
                .as_bytes()
        );
        assert_eq!(
            screen_value(&base, Some(&base), b"history_added")
                .unwrap()
                .bytes(),
            b"9"
        );
        assert_eq!(
            screen_value(&base, Some(&base), b"history_limit")
                .unwrap()
                .bytes(),
            b"100"
        );
    }

    #[test]
    fn private_modes_keep_table_order_and_explicit_blink_rule() {
        let mode = ScreenMode::SYNC
            | ScreenMode::CURSOR_BLINKING
            | ScreenMode::KCURSOR
            | ScreenMode::THEME_UPDATES;
        assert_eq!(private_modes(mode), b"1,2026,2031");
        assert_eq!(
            private_modes(mode | ScreenMode::CURSOR_BLINKING_SET),
            b"1,12,2026,2031"
        );
        assert_eq!(private_modes(ScreenMode::default()), b"");
    }

    fn client() -> ClientFacts<'static> {
        ClientFacts {
            flags: ClientFlags::default(),
            started: false,
            width: 80,
            height: 24,
            xpixel: 8,
            ypixel: 16,
            term_rgb: false,
            term_256: false,
            term_colours: 8,
            pid: 10,
            uid: None,
            user: None,
            name: Some(b"tty"),
            termname: None,
            termtype: None,
            tty: None,
            key_table: b"root",
            default_key_table: b"root",
            features: b"",
            session: None,
            last_session: None,
            created: Timestamp::default(),
            activity: Timestamp::default(),
            discarded: 0,
            written: 0,
            pause_age_ms: 0,
            theme: None,
            viewport: None,
        }
    }

    #[test]
    fn client_started_and_colour_normalization_rules() {
        let mut facts = client();
        assert_eq!(
            client_value(&facts, b"client_width").unwrap().bytes(),
            b"80"
        );
        assert!(client_value(&facts, b"client_height").is_none());
        assert!(client_value(&facts, b"client_colours").is_none());
        assert_eq!(
            client_value(&facts, b"client_termtype").unwrap().bytes(),
            b""
        );
        facts.started = true;
        for (count, expected) in [
            (0, b"2".as_slice()),
            (7, b"2"),
            (8, b"8"),
            (15, b"8"),
            (16, b"16"),
            (256, b"16"),
        ] {
            facts.term_colours = count;
            assert_eq!(
                client_value(&facts, b"client_colours").unwrap().bytes(),
                expected
            );
        }
        facts.term_256 = true;
        assert_eq!(
            client_value(&facts, b"client_colours").unwrap().bytes(),
            b"256"
        );
        facts.term_rgb = true;
        assert_eq!(
            client_value(&facts, b"client_colours").unwrap().bytes(),
            b"16777216"
        );
        facts.key_table = b"prefix";
        assert_eq!(
            client_value(&facts, b"client_prefix").unwrap().bytes(),
            b"1"
        );
    }

    #[test]
    fn printable_client_flags_keep_source_order() {
        let flags = ClientFlags::CONTROL
            | ClientFlags::READONLY
            | ClientFlags::CONTROL_PAUSEAFTER
            | ClientFlags::UTF8
            | ClientFlags::ATTACHED;
        assert_eq!(
            client_flags(flags, 2500),
            b"attached,control-mode,pause-after=2,read-only,UTF-8"
        );
    }

    #[test]
    fn mouse_status_coordinates_and_unsupported_modes() {
        let grid = Grid::new(2, 1, 0);
        let screen = screen();
        let registry = HyperlinkRegistry::new();
        let mut mouse = MouseFacts {
            valid: true,
            pane_public_id: None,
            pane_coordinates: None,
            x: 7,
            y: 23,
            status_at: 22,
            status_lines: 2,
            client_started: true,
            status_range: None,
            backing: MouseBacking::Base {
                grid: &grid,
                screen: &screen,
                registry: &registry,
            },
            word_separators: b"-",
        };
        assert_eq!(mouse_value(&mouse, b"mouse_x").unwrap().bytes(), b"7");
        assert_eq!(mouse_value(&mouse, b"mouse_y").unwrap().bytes(), b"1");
        assert_eq!(
            mouse_value(&mouse, b"mouse_status_line").unwrap().bytes(),
            b"1"
        );
        mouse.pane_public_id = Some(99);
        mouse.pane_coordinates = Some((1, 0));
        mouse.backing = MouseBacking::Unsupported;
        assert_eq!(mouse_value(&mouse, b"mouse_pane").unwrap().bytes(), b"%99");
        assert_eq!(mouse_value(&mouse, b"mouse_x").unwrap().bytes(), b"1");
        assert!(mouse_value(&mouse, b"mouse_word").is_none());
        mouse.valid = false;
        assert!(mouse_value(&mouse, b"mouse_pane").is_none());
    }

    fn session(
        server: &mut crate::model::Server,
        name: &[u8],
        attached: u32,
    ) -> crate::ids::SessionId {
        let options = server.options.global_s;
        let id = crate::model::session::session_create(
            server,
            crate::model::session::SessionCreate {
                prefix: None,
                name: Some(name.to_vec()),
                cwd: b"/tmp".to_vec(),
                environment: crate::options::environment::Environment::default(),
                options,
                termios: None,
            },
        );
        server.sessions.get_mut(id).unwrap().attached = attached;
        id
    }

    #[test]
    fn model_session_alerts_stack_and_empty_context() {
        let mut server = crate::model::Server::new();
        server.current_time = (100, 12);
        let sid = session(&mut server, b"work", 2);
        use crate::ids::{ArenaId, WindowId};
        let wid = WindowId::from_parts(99, 1);
        let first = server
            .winlinks
            .insert(crate::model::Winlink {
                index: 2,
                session: sid,
                window: wid,
                flags: WinlinkFlags::BELL,
            })
            .unwrap();
        let last = server
            .winlinks
            .insert(crate::model::Winlink {
                index: 9,
                session: sid,
                window: wid,
                flags: WinlinkFlags::ACTIVITY | WinlinkFlags::SILENCE,
            })
            .unwrap();
        let s = server.sessions.get_mut(sid).unwrap();
        s.windows.insert(2, first);
        s.windows.insert(9, last);
        s.current = Some(first);
        s.last.push(last);
        let context = FormatContext {
            session: Some(sid),
            ..FormatContext::default()
        };
        for (key, expected) in [
            (b"session_alert".as_slice(), b"!#~".as_slice()),
            (b"session_alerts", b"2!,9#~"),
            (b"session_stack", b"2,9"),
            (b"last_window_index", b"9"),
            (b"active_window_index", b"2"),
            (b"session_windows", b"2"),
            (b"session_many_attached", b"1"),
            (b"session_created", b"100"),
        ] {
            assert_eq!(
                model_value(&server, &context, key).unwrap().bytes(),
                expected
            );
        }
        assert!(model_value(&server, &FormatContext::default(), b"session_name").is_none());
        let broken = FormatContext {
            session: Some(session(&mut server, b"empty", 0)),
            ..FormatContext::default()
        };
        assert!(model_value(&server, &broken, b"active_window_index").is_none());
        assert!(model_value(&server, &broken, b"last_window_index").is_none());
    }

    #[test]
    fn model_session_groups_and_client_lists_preserve_order() {
        use crate::ids::ArenaId;
        let mut server = crate::model::Server::new();
        let left = session(&mut server, b"left", 2);
        let right = session(&mut server, b"right", 1);
        let group = server
            .groups
            .insert(crate::model::SessionGroup {
                name: b"group".to_vec(),
                sessions: vec![right, left],
            })
            .unwrap();
        server.sessions.get_mut(left).unwrap().group = Some(group);
        server.sessions.get_mut(right).unwrap().group = Some(group);
        let c0 = crate::ids::ClientId::from_parts(0, 0);
        let c1 = crate::ids::ClientId::from_parts(1, 0);
        let clients = [
            ClientLinkFacts {
                id: c0,
                session: Some(left),
                name: b"second",
            },
            ClientLinkFacts {
                id: c1,
                session: Some(right),
                name: b"first",
            },
        ];
        let context = FormatContext {
            session: Some(left),
            evaluated_client: Some(c1),
            ..FormatContext::default()
        };
        assert_eq!(
            model_value(&server, &context, b"session_group_attached")
                .unwrap()
                .bytes(),
            b"3"
        );
        assert_eq!(
            model_value(&server, &context, b"session_group_list")
                .unwrap()
                .bytes(),
            b"right,left"
        );
        assert_eq!(
            client_model_value(&server, &context, &clients, b"session_active")
                .unwrap()
                .bytes(),
            b"0"
        );
        assert_eq!(
            client_model_value(&server, &context, &clients, b"session_attached_list")
                .unwrap()
                .bytes(),
            b"second"
        );
        assert_eq!(
            client_model_value(&server, &context, &clients, b"session_group_attached_list")
                .unwrap()
                .bytes(),
            b"second,first"
        );
    }

    #[test]
    fn model_alert_lists_obey_1024_byte_c_buffer() {
        use crate::ids::{ArenaId, WindowId};
        let mut server = crate::model::Server::new();
        let sid = session(&mut server, b"work", 0);
        for index in 0..512 {
            let id = server
                .winlinks
                .insert(crate::model::Winlink {
                    index,
                    session: sid,
                    window: WindowId::from_parts(99, 0),
                    flags: WinlinkFlags::ACTIVITY | WinlinkFlags::BELL | WinlinkFlags::SILENCE,
                })
                .unwrap();
            server
                .sessions
                .get_mut(sid)
                .unwrap()
                .windows
                .insert(index, id);
        }
        let context = FormatContext {
            session: Some(sid),
            ..FormatContext::default()
        };
        let alerts = model_value(&server, &context, b"session_alerts")
            .unwrap()
            .bytes();
        assert_eq!(alerts.len(), 1023);
        assert!(alerts.starts_with(b"0#!~,1#!~,2#!~"));
    }

    #[test]
    fn model_buffer_full_stops_at_nul_but_size_counts_all_bytes() {
        let mut server = crate::model::Server::new();
        let id =
            crate::model::paste::paste_set(&mut server, b"one\0two".to_vec(), Some(b"named"), 1)
                .ok()
                .unwrap()
                .unwrap();
        let context = FormatContext {
            buffer: Some(id),
            ..FormatContext::default()
        };
        assert_eq!(
            model_value(&server, &context, b"buffer_full")
                .unwrap()
                .bytes(),
            b"one"
        );
        assert_eq!(
            model_value(&server, &context, b"buffer_size")
                .unwrap()
                .bytes(),
            b"7"
        );
        assert_eq!(
            model_value(&server, &context, b"buffer_name")
                .unwrap()
                .bytes(),
            b"named"
        );
    }

    #[test]
    fn auxiliary_pane_callbacks_keep_absent_context_semantics() {
        let mut facts = PaneAuxFacts {
            mode_names: &[],
            pipe_pid: None,
            default_fg: Colour::DEFAULT,
            default_bg: Colour::DEFAULT,
            status_ready: false,
            exit_status: Some(4),
            signal_name: Some(b"TERM"),
        };
        assert_eq!(
            pane_aux_value(&facts, b"pane_in_mode").unwrap().bytes(),
            b"0"
        );
        assert!(pane_aux_value(&facts, b"pane_mode").is_none());
        assert_eq!(pane_aux_value(&facts, b"pane_pipe").unwrap().bytes(), b"0");
        assert!(pane_aux_value(&facts, b"pane_pipe_pid").is_none());
        assert!(pane_aux_value(&facts, b"pane_dead_status").is_none());
        facts.status_ready = true;
        assert_eq!(
            pane_aux_value(&facts, b"pane_dead_status").unwrap().bytes(),
            b"4"
        );
        assert_eq!(
            pane_aux_value(&facts, b"pane_dead_signal").unwrap().bytes(),
            b"TERM"
        );
    }

    #[test]
    fn mouse_status_range_representation_and_copy_backing() {
        use rmux_emu::style::{StyleRange, StyleRangeType};
        let mut backing = Grid::new(4, 2, 0);
        backing.set_cell(
            0,
            1,
            &GridCell {
                data: Utf8Data::set(b'x'),
                ..GridCell::default()
            },
        );
        let screen = screen();
        let registry = HyperlinkRegistry::new();
        let mut range = StyleRange {
            range_type: StyleRangeType::User,
            argument: 0,
            string: [0; 16],
            start: 0,
            end: 20,
        };
        range.string[..5].copy_from_slice(b"hello");
        let mouse = MouseFacts {
            valid: true,
            pane_public_id: Some(1),
            pane_coordinates: Some((0, 0)),
            x: 2,
            y: 0,
            status_at: 0,
            status_lines: 1,
            client_started: true,
            status_range: Some(&range),
            backing: MouseBacking::Copy {
                grid: &backing,
                screen: &screen,
                registry: &registry,
                first_row: 1,
            },
            word_separators: b"-",
        };
        assert_eq!(
            mouse_value(&mouse, b"mouse_status_range").unwrap().bytes(),
            b"hello"
        );
        assert_eq!(mouse_value(&mouse, b"mouse_word").unwrap().bytes(), b"x");
        assert_eq!(mouse_value(&mouse, b"mouse_line").unwrap().bytes(), b"x");
    }

    #[test]
    fn quote_and_time_helpers_match_pinned_c() {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let output = Command::new("git")
            .args(["-C", "/Users/j/fun/tmux", "show", "8f25579c:format.c"])
            .output();
        let Ok(output) = output else {
            eprintln!("skipping C helper comparison: git unavailable");
            return;
        };
        if !output.status.success() {
            eprintln!("skipping C helper comparison: pinned source unavailable");
            return;
        }
        let source = String::from_utf8(output.stdout).unwrap();
        let functions = source
            .split("/* Quote shell special characters in string. */")
            .nth(1)
            .unwrap()
            .split("/* Find a format entry. */")
            .next()
            .unwrap();
        let harness = r#"
#define _GNU_SOURCE
#include <sys/types.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <stdarg.h>
#include <stdint.h>
static time_t clock_value;
static time_t fixed_time(time_t *out) { if(out) *out=clock_value; return clock_value; }
static void *xmalloc(size_t n) { void *p=malloc(n); if(!p) abort(); return p; }
static char *xstrdup(const char *s) { char *p=strdup(s); if(!p) abort(); return p; }
static int xasprintf(char **p,const char *s,...) { va_list ap; va_start(ap,s); int n=vasprintf(p,s,ap); va_end(ap); if(n<0)abort(); return n; }
#define xsnprintf snprintf
#define time fixed_time
"#;
        let main = r#"
static void dump(const char *s) { if(!s) { puts("-"); return; } for(;*s;s++) printf("%02x",(unsigned char)*s); puts(""); }
int main(int argc,char **argv) {
  clock_value=strtoll(argv[2],NULL,10);
  time_t t=strtoll(argv[3],NULL,10);
  char *s=NULL;
  if(!strcmp(argv[1],"pretty"))s=format_pretty_time(t,atoi(argv[4]));
  else if(!strcmp(argv[1],"relative"))s=format_relative_time(t);
  else if(!strcmp(argv[1],"difference"))s=format_time_difference(t);
  else if(!strcmp(argv[1],"shell"))s=format_quote_shell(argv[4]);
  else if(!strcmp(argv[1],"single"))s=format_quote_shell_single(argv[4]);
  else s=format_quote_style(argv[4]);
  dump(s); free(s); return 0;
}
"#;
        let directory =
            std::env::temp_dir().join(format!("rmux-format-helper-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let binary = directory.join("helpers");
        let spawn = Command::new("cc")
            .args(["-x", "c", "-o"])
            .arg(&binary)
            .arg("-")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let Ok(mut compiler) = spawn else {
            eprintln!("skipping C helper comparison: C compiler unavailable");
            std::fs::remove_dir_all(&directory).unwrap();
            return;
        };
        compiler
            .stdin
            .as_mut()
            .unwrap()
            .write_all(format!("{harness}\n{functions}\n{main}").as_bytes())
            .unwrap();
        let output = compiler.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "C helper compile: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let encode = |value: Option<ByteString>| match value {
            None => b"-\n".to_vec(),
            Some(value) => {
                let mut text = String::new();
                for byte in value.iter() {
                    use std::fmt::Write;
                    write!(&mut text, "{byte:02x}").unwrap();
                }
                text.push('\n');
                text.into_bytes()
            }
        };
        for mode in ["pretty", "relative", "difference"] {
            for now in [0, 1_782_864_000, 1_798_128_000] {
                for age in [
                    -1,
                    0,
                    1,
                    59,
                    60,
                    61,
                    3599,
                    3600,
                    3660,
                    86399,
                    86400,
                    27 * 86400,
                    28 * 86400,
                    200 * 86400,
                    370 * 86400,
                ] {
                    let value = now - age;
                    for seconds in [false, true] {
                        let output = Command::new(&binary)
                            .args([
                                mode,
                                &now.to_string(),
                                &value.to_string(),
                                if seconds { "1" } else { "0" },
                            ])
                            .output()
                            .unwrap();
                        assert!(output.status.success());
                        let expected = match mode {
                            "pretty" => Some(pretty_time_at(value, now, seconds)),
                            "relative" => relative_time(value, now),
                            _ => Some(difference_time(value, now)),
                        };
                        assert_eq!(
                            output.stdout,
                            encode(expected),
                            "{mode} {now} {value} {seconds}"
                        );
                    }
                }
            }
        }
        for input in [
            "",
            "a'b",
            "|&;<>(){}$`\\\"'*?[# =%\n\t",
            "hello world",
            "###[x]",
            "é",
        ] {
            for mode in ["shell", "single", "style"] {
                let output = Command::new(&binary)
                    .args([mode, "0", "0", input])
                    .output()
                    .unwrap();
                assert!(output.status.success());
                let expected = match mode {
                    "shell" => quote_shell(input.as_bytes()),
                    "single" => quote_single(input.as_bytes()),
                    _ => quote_style(input.as_bytes()),
                };
                assert_eq!(output.stdout, encode(Some(expected)), "{mode} {input:?}");
            }
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn pane_tsp_reports_renderer_and_epoch_from_live_state() {
        use crate::tsp::broker::{PaneTspState, Renderer};
        let mut server = crate::model::Server::new();
        let window = crate::model::window::window_create(&mut server, 12, 4, 0, 0).unwrap();
        let pane = crate::model::pane::pane_create(&mut server, window, 12, 4, 0).unwrap();
        let context = FormatContext {
            pane: Some(pane),
            ..FormatContext::default()
        };
        assert_eq!(
            model_value(&server, &context, b"pane_tsp").unwrap().bytes(),
            b"ansi"
        );
        assert_eq!(
            model_value(&server, &context, b"pane_tsp_epoch")
                .unwrap()
                .bytes(),
            b"0"
        );
        server.panes.get_mut(pane).unwrap().tsp = Some(PaneTspState::default());
        for (renderer, expected) in [
            (Renderer::Ansi, b"ansi".as_slice()),
            (Renderer::Switching, b"switching"),
            (Renderer::Native, b"native"),
            (Renderer::Detached, b"detached"),
        ] {
            let tsp = server.panes.get_mut(pane).unwrap().tsp.as_mut().unwrap();
            tsp.renderer = renderer;
            tsp.epoch = 17;
            assert_eq!(
                model_value(&server, &context, b"pane_tsp").unwrap().bytes(),
                expected
            );
            assert_eq!(
                model_value(&server, &context, b"pane_tsp_epoch")
                    .unwrap()
                    .bytes(),
                b"17"
            );
        }
    }

    #[test]
    fn model_pane_geometry_command_state_and_wait_status() {
        let mut server = crate::model::Server::new();
        let window = crate::model::window::window_create(&mut server, 12, 4, 8, 16).unwrap();
        let pane = crate::model::pane::pane_create(&mut server, window, 12, 4, 100).unwrap();
        server.windows.get_mut(window).unwrap().panes.push(pane);
        server.windows.get_mut(window).unwrap().z_order.push(pane);
        server.windows.get_mut(window).unwrap().active = Some(pane);
        let p = server.panes.get_mut(pane).unwrap();
        p.xoff = -2;
        p.yoff = 1;
        p.argv = vec![b"a'b".to_vec(), Vec::new(), b"two words".to_vec()];
        p.cmd_start_time = 100;
        p.cmd_end_time = 90;
        p.cmd_status = 7;
        p.flags.insert(PaneFlags::STATUSREADY);
        p.status = 7 << 8;
        let context = FormatContext {
            pane: Some(pane),
            window: Some(window),
            ..FormatContext::default()
        };
        for (key, expected) in [
            (b"pane_left".as_slice(), b"-2".as_slice()),
            (b"pane_right", b"9"),
            (b"pane_bottom", b"4"),
            (b"pane_active", b"1"),
            (b"pane_in_mode", b"0"),
            (b"pane_command_status", b"7"),
            (b"pane_command_duration", b"0"),
            (b"pane_dead", b"1"),
            (b"pane_dead_status", b"7"),
            (b"pane_start_command_list", b"'a'\\''b' '' 'two words'"),
        ] {
            assert_eq!(
                model_value(&server, &context, key).unwrap().bytes(),
                expected
            );
        }
        assert!(model_value(&server, &context, b"pane_dead_signal").is_none());
        assert!(model_value(&server, &context, b"pane_pid").is_none());
        server
            .panes
            .get_mut(pane)
            .unwrap()
            .flags
            .insert(PaneFlags::CMDRUNNING);
        server.current_time = (135, 0);
        assert_eq!(
            model_value(&server, &context, b"pane_command_duration")
                .unwrap()
                .bytes(),
            b"35"
        );
    }

    #[test]
    fn pane_z_skips_hidden_floats_and_uses_tiled_sentinel() {
        use crate::layout::{self, LayoutGeometry};
        use crate::model::{pane, spawn::SpawnFlags, window};

        let mut server = crate::model::Server::new();
        let w = window::window_create(&mut server, 80, 24, 0, 0).unwrap();
        let tiled =
            window::window_add_pane(&mut server, w, None, 10, SpawnFlags::default()).unwrap();
        window::window_set_active_pane(&mut server, w, tiled, false).unwrap();
        layout::init(&mut server, w, tiled);
        let g = LayoutGeometry {
            sx: 20,
            sy: 6,
            xoff: 4,
            yoff: 4,
        };
        let mut floats = Vec::new();
        for _ in 0..3 {
            let lc = layout::floating_pane(&mut server, w, Some(tiled), &g);
            let p = window::window_add_pane(&mut server, w, Some(tiled), 10, SpawnFlags::FLOATING)
                .unwrap();
            layout::assign_pane(&mut server, lc, p, false);
            floats.push(p);
        }
        let [back, hidden, front] = [floats[0], floats[1], floats[2]];
        server.windows.get_mut(w).unwrap().z_order = vec![back, hidden, front, tiled];
        let p = server.panes.get_mut(hidden).unwrap();
        p.saved_layout_cell = p.layout_cell.take();
        for (p, expected) in [(back, b"0".as_slice()), (front, b"1"), (tiled, b"3")] {
            let context = FormatContext {
                pane: Some(p),
                window: Some(w),
                ..FormatContext::default()
            };
            assert_eq!(
                model_value(&server, &context, b"pane_z").unwrap().bytes(),
                expected
            );
        }
        let p = server.panes.get_mut(hidden).unwrap();
        p.layout_cell = p.saved_layout_cell.take();
        assert!(pane::pane_is_floating(&server, hidden));
        for (p, expected) in [(hidden, b"1".as_slice()), (front, b"2"), (tiled, b"4")] {
            let context = FormatContext {
                pane: Some(p),
                window: Some(w),
                ..FormatContext::default()
            };
            assert_eq!(
                model_value(&server, &context, b"pane_z").unwrap().bytes(),
                expected
            );
        }
    }
}
