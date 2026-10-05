// Ported from tmux options.c @ 8f25579c
//! `options_push_changes` as an ordered plan of server effects. The server
//! (G14) applies each `OptionsChange` in sequence with the iteration order
//! named on the variant; the plan itself touches no server state.

use rmux_util::bytes::ByteString;

/// One side effect of `options_push_changes(name)` (`options.c:1358-1476`).
/// Collections iterate in C container order: clients in attach order,
/// windows by id, panes by id, sessions by name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OptionsChange {
    /// For every client: `server_client_update_theme_colours`,
    /// `tty_invalidate` if the tty is open, `server_redraw_client`.
    ClientThemeColours,
    /// For every window with an active pane whose window option
    /// `automatic-rename` is on: set `PANE_CHANGED` on the active pane.
    WindowAutomaticRename,
    /// For every pane: `window_pane_default_cursor`.
    PaneDefaultCursor,
    /// For every window: `window_set_fill_cells`.
    WindowFillCells,
    /// For every client: `server_client_set_key_table(c, NULL)`.
    ClientKeyTable,
    /// For every client with an open tty: `tty_keys_build`.
    ClientTtyKeysBuild,
    /// `status_timer_start_all()`
    StatusTimerStartAll,
    /// `redraw_invalidate_all_scenes()`
    RedrawInvalidateAllScenes,
    /// `alerts_reset_all()`
    AlertsResetAll,
    /// For every pane: set `PANE_STYLECHANGED|PANE_THEMECHANGED`.
    PaneStyleAndThemeChanged,
    /// For every pane: set `PANE_STYLECHANGED`.
    PaneStyleChanged,
    /// For every pane: `colour_palette_from_option(&wp->palette, wp->options)`.
    PanePaletteFromOption,
    /// For every window: reload `w->sb` from `pane-scrollbars` and
    /// `w->sb_pos` from `pane-scrollbars-position`, then `layout_fix_panes`.
    WindowScrollbarsReload,
    /// For every pane: `window_pane_scrollbar_hide`.
    PaneScrollbarHide,
    /// For every pane: `style_set_scrollbar_style_from_option`; then for
    /// every window: `layout_fix_panes`.
    PaneScrollbarStyle,
    /// `utf8_update_width_cache()`
    Utf8UpdateWidthCache,
    /// `input_set_buffer_size(options_get_number(global_options, name))`
    InputSetBufferSize,
    /// For every session: `session_update_history`.
    SessionUpdateHistory,
    /// For every session: `status_update_cache`.
    SessionStatusUpdateCache,
    /// `recalculate_sizes()`
    RecalculateSizes,
    /// For every client with a session: `server_redraw_client`.
    ClientRedrawWithSession,
}

/// Names and the steps they trigger, in source order. Each row is one `if`
/// of `options_push_changes`; a name may appear in several rows.
const STEPS: &[(&[u8], OptionsChange)] = &[
    (b"automatic-rename", OptionsChange::WindowAutomaticRename),
    (b"cursor-colour", OptionsChange::PaneDefaultCursor),
    (b"cursor-style", OptionsChange::PaneDefaultCursor),
    (b"fill-character", OptionsChange::WindowFillCells),
    (b"key-table", OptionsChange::ClientKeyTable),
    (b"user-keys", OptionsChange::ClientTtyKeysBuild),
    (b"status", OptionsChange::StatusTimerStartAll),
    (b"status-interval", OptionsChange::StatusTimerStartAll),
    (b"status", OptionsChange::RedrawInvalidateAllScenes),
    (b"status-position", OptionsChange::RedrawInvalidateAllScenes),
    (
        b"pane-border-indicators",
        OptionsChange::RedrawInvalidateAllScenes,
    ),
    (
        b"pane-border-lines",
        OptionsChange::RedrawInvalidateAllScenes,
    ),
    (
        b"pane-border-status",
        OptionsChange::RedrawInvalidateAllScenes,
    ),
    (b"pane-scrollbars", OptionsChange::RedrawInvalidateAllScenes),
    (
        b"pane-scrollbars-timeout",
        OptionsChange::RedrawInvalidateAllScenes,
    ),
    (
        b"pane-scrollbars-position",
        OptionsChange::RedrawInvalidateAllScenes,
    ),
    (
        b"pane-scrollbars-style",
        OptionsChange::RedrawInvalidateAllScenes,
    ),
    (b"monitor-silence", OptionsChange::AlertsResetAll),
    (b"window-style", OptionsChange::PaneStyleAndThemeChanged),
    (
        b"window-active-style",
        OptionsChange::PaneStyleAndThemeChanged,
    ),
    (b"pane-colours", OptionsChange::PanePaletteFromOption),
    (b"pane-border-status", OptionsChange::WindowScrollbarsReload),
    (b"pane-scrollbars", OptionsChange::WindowScrollbarsReload),
    (
        b"pane-scrollbars-position",
        OptionsChange::WindowScrollbarsReload,
    ),
    (b"pane-scrollbars", OptionsChange::PaneScrollbarHide),
    (b"pane-scrollbars-style", OptionsChange::PaneScrollbarStyle),
    (b"codepoint-widths", OptionsChange::Utf8UpdateWidthCache),
    (b"input-buffer-size", OptionsChange::InputSetBufferSize),
    (b"history-limit", OptionsChange::SessionUpdateHistory),
];

/// Steps that always run last (`options.c:1468-1475`).
const ALWAYS: [OptionsChange; 3] = [
    OptionsChange::SessionStatusUpdateCache,
    OptionsChange::RecalculateSizes,
    OptionsChange::ClientRedrawWithSession,
];

/// `options_push_changes(name)`: the effects to apply, in order. The
/// `PaneStyleChanged` step for `@` names sits between the window-style and
/// `pane-colours` rows (`options.c:1429-1432`).
pub fn push_changes(name: &[u8]) -> Vec<OptionsChange> {
    let mut out = Vec::with_capacity(6);
    rmux_util::log_debug!("options_push_changes: {}", ByteString::from(name));
    if name == b"theme" || name.starts_with(b"dark-theme-") || name.starts_with(b"light-theme-") {
        out.push(OptionsChange::ClientThemeColours);
    }
    let user = name.first() == Some(&b'@');
    let mut user_done = false;
    for (step_name, change) in STEPS {
        if user && !user_done && *change == OptionsChange::PanePaletteFromOption {
            out.push(OptionsChange::PaneStyleChanged);
            user_done = true;
        }
        if *step_name == name {
            out.push(*change);
        }
    }
    out.extend_from_slice(&ALWAYS);
    out
}

#[cfg(test)]
mod tests {
    use super::OptionsChange::*;
    use super::*;

    fn plan(name: &str) -> Vec<OptionsChange> {
        push_changes(name.as_bytes())
    }

    #[test]
    fn always_steps_run_last_in_order() {
        assert_eq!(
            plan("escape-time"),
            [
                SessionStatusUpdateCache,
                RecalculateSizes,
                ClientRedrawWithSession
            ]
        );
    }

    #[test]
    fn theme_runs_first() {
        for name in ["theme", "dark-theme-black", "light-theme-fg"] {
            assert_eq!(
                plan(name),
                [
                    ClientThemeColours,
                    SessionStatusUpdateCache,
                    RecalculateSizes,
                    ClientRedrawWithSession
                ]
            );
        }
        assert_eq!(plan("themes").len(), 3);
    }

    #[test]
    fn single_step_names() {
        let tail = [
            SessionStatusUpdateCache,
            RecalculateSizes,
            ClientRedrawWithSession,
        ];
        let one = |step: OptionsChange| {
            let mut v = vec![step];
            v.extend_from_slice(&tail);
            v
        };
        assert_eq!(plan("automatic-rename"), one(WindowAutomaticRename));
        assert_eq!(plan("cursor-colour"), one(PaneDefaultCursor));
        assert_eq!(plan("cursor-style"), one(PaneDefaultCursor));
        assert_eq!(plan("fill-character"), one(WindowFillCells));
        assert_eq!(plan("key-table"), one(ClientKeyTable));
        assert_eq!(plan("user-keys"), one(ClientTtyKeysBuild));
        assert_eq!(plan("status-interval"), one(StatusTimerStartAll));
        assert_eq!(plan("status-position"), one(RedrawInvalidateAllScenes));
        assert_eq!(
            plan("pane-border-indicators"),
            one(RedrawInvalidateAllScenes)
        );
        assert_eq!(plan("pane-border-lines"), one(RedrawInvalidateAllScenes));
        assert_eq!(
            plan("pane-scrollbars-timeout"),
            one(RedrawInvalidateAllScenes)
        );
        assert_eq!(plan("monitor-silence"), one(AlertsResetAll));
        assert_eq!(plan("window-style"), one(PaneStyleAndThemeChanged));
        assert_eq!(plan("window-active-style"), one(PaneStyleAndThemeChanged));
        assert_eq!(plan("pane-colours"), one(PanePaletteFromOption));
        assert_eq!(plan("codepoint-widths"), one(Utf8UpdateWidthCache));
        assert_eq!(plan("input-buffer-size"), one(InputSetBufferSize));
        assert_eq!(plan("history-limit"), one(SessionUpdateHistory));
        assert_eq!(plan("@anything"), one(PaneStyleChanged));
    }

    #[test]
    fn names_in_two_steps_keep_both() {
        let tail = [
            SessionStatusUpdateCache,
            RecalculateSizes,
            ClientRedrawWithSession,
        ];
        let with = |steps: &[OptionsChange]| {
            let mut v = steps.to_vec();
            v.extend_from_slice(&tail);
            v
        };
        assert_eq!(
            plan("status"),
            with(&[StatusTimerStartAll, RedrawInvalidateAllScenes])
        );
        assert_eq!(
            plan("pane-border-status"),
            with(&[RedrawInvalidateAllScenes, WindowScrollbarsReload])
        );
        assert_eq!(
            plan("pane-scrollbars"),
            with(&[
                RedrawInvalidateAllScenes,
                WindowScrollbarsReload,
                PaneScrollbarHide
            ])
        );
        assert_eq!(
            plan("pane-scrollbars-position"),
            with(&[RedrawInvalidateAllScenes, WindowScrollbarsReload])
        );
        assert_eq!(
            plan("pane-scrollbars-style"),
            with(&[RedrawInvalidateAllScenes, PaneScrollbarStyle])
        );
    }

    #[test]
    fn every_table_row_name_is_a_live_option() {
        for (name, _) in STEPS {
            assert!(
                crate::options::search(name).is_some(),
                "{}",
                ByteString::from(*name)
            );
        }
    }
}
