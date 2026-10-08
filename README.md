rmux

A Rust port of tmux.

It works just like tmux, with the same commands, keys and config, and it
can run alongside tmux without getting in its way.

It also passes the [Tern](https://stencil.so/tern) Surface Protocol
through, so apps like omp render natively when you use rmux in Tern.

Runs on Linux and macOS.

install (Rust 1.85 or newer, a C compiler, pkg-config and ncurses; on
macOS also `brew install utf8proc`):

    git clone git@github.com:pgkt04/rmux.git
    cd rmux
    cargo install --locked --path crates/rmux

Add `--features sixel` for sixel images, or `--features systemd` on Linux
for socket activation.

usage:

    rmux                    new session
    rmux attach             attach to the last session
    rmux ls                 list sessions
    rmux new -s NAME        new named session
    rmux kill-server        stop everything
    rmux -V                 version

The prefix key is `C-b`, as in tmux. Config is read from `/etc/tmux.conf`,
`~/.tmux.conf` and `~/.config/tmux/tmux.conf`.

tern:

    rmux                    in a Tern pane
    omp                     inside the rmux pane: native view

In the native view the status line is drawn by Tern as a bar under omp's
composer: the same text as `status-format`, the current window highlighted,
bells and activity coloured, always at the bottom.

Without the omp extension, the native view needs the pane to be alone in its
window (or zoomed). In a split, a native omp pane shows a short note instead;
zooming brings the native view back.

Scrolling stays in Tern's native view without resizing first, including when
its surface scrolls offscreen.
`C-b [` opens rmux copy mode on the saved text grid (which may be empty for
a native-only app); `q` returns to the native view. `C-b d` detaches normally.

`#{pane_tsp}` and `#{client_tsp}` show the state (`rmux display -p
'#{pane_tsp}'`). Start the server with `RMUX_TSP_BROKER=0` to turn native
rendering off, or set `PI_TUI_NATIVE=0` to keep omp on its text renderer.

bundled omp plugin (tested with omp 18.8.3):

rmux includes the `rmux-terminal` plugin in its binary; no omp source changes
are needed. Export and register it once, then start omp to load it:

    rmux omp-plugin ~/.omp/local-plugins/rmux-terminal
    omp plugin install ~/.omp/local-plugins/rmux-terminal

`/plugins` lists it as `rmux-terminal`. If you previously exported the loose
`~/.omp/agent/extensions/rmux.ts`, remove it to avoid loading the extension twice.

The plugin automatically switches the same running omp between native and
text rendering when you reattach from a different terminal. Mixed TSP/plain
viewers and split panes use text; returning to a sole or zoomed TSP pane uses
native. The app process, conversation, transcript, and unsent draft are retained.
Detection uses a read-only, no-output control client; renderer changes are
delivered immediately instead of waiting for periodic polling. Native window
switches reuse terminal negotiation, and resizing updates the existing native
document rather than rebuilding it when the pane remains eligible.
Brief native-to-text changes settle for 150 ms so rapid resize/layout changes
do not repeatedly restart the renderer. Failed negotiation retries are bounded
and report a warning instead of looping.
Returning to a native window serializes the retained document without copying
a temporary JSON tree; this reduces rmux preparation work for large transcripts.
The terminal still receives a complete replay and must draw it.
`/terminal-reprobe` manually repeats negotiation. `PI_TUI_NATIVE=0` keeps text
rendering and disables automatic switching.

To use two omp instances, start omp, press `C-b %` for a side-by-side split
(or `C-b "` for top/bottom), and start omp in the new pane. Both panes render
as text automatically. `C-b o` selects the next pane; `C-b z` zooms the selected
pane into a native view in Tern, and pressing it again returns to text splits.
Close the other pane (quit omp, then exit its shell) to return the remaining
sole pane to native.
Native rendering still covers only one sole or zoomed pane, not both splits.
Automatic switching waits while an external editor owns omp's terminal and
resumes after the editor exits; in-flight terminal negotiation is not restarted.
Prompt markers emitted by the foreground app's text renderer do not end its
native registration; closing a split can still return that same app to native.

Native prefix prompts stay in the dock beneath omp: `C-b ,` renames the window,
`C-b $` the session, `C-b T` the pane title, and `C-b :` runs a command. Editing,
Enter, and cancel use the normal rmux prompt behavior without restarting omp's
renderer. `C-b {` / `C-b }` swap panes and `C-b !` breaks a pane into a window.
Splits, full-screen modes, menus, and popups still use the text view.

For an omp profile, run `omp --profile NAME plugin install /path/to/package`.
`rmux omp-plugin` without a directory still prints the standalone extension for
`omp -e /path/to/rmux.ts`. Re-export the package after updating rmux.

The plugin requires an updated **running server**, not just an updated binary.
Old servers lack `#{pane_tsp_view}`; the plugin reports an error instead of
switching. From outside rmux, start a separate updated server without ending
your existing sessions:

    rmux -L updated new-session
    # Start omp inside it; later reattach from either terminal:
    rmux -L updated attach

Restart an old server only after saving work: server restart ends its sessions.

debugging:

Panic and fatal-error reports are saved even without verbose logging in
`${XDG_STATE_HOME:-$HOME/.local/state}/rmux/rmux-crash-*.log`. Reports contain the
process role/PID, executable, working directory, error/location, and a forced
backtrace; files are private (0600). Release builds retain source line tables.
This does not capture `SIGKILL`, OOM kills, or native faults such as `SIGSEGV`;
those need OS/kernel logs or a core dump. Reports require a writable state directory.

For detailed event logs, start a separate server from a writable directory:

    rmux -vv -L debug new-session

`-v` writes `rmux-client-PID.log` and `rmux-server-PID.log` in each process's
working directory; `-vv` also records raw terminal output in `rmux-out-PID.log`.
An existing server's debug log can be toggled without ending sessions:

    kill -USR2 "$(rmux display-message -p '#{pid}')"

The server log appears in the directory where that server started, not the
directory of a later client. A second signal disables it. Debug/raw logs can
contain terminal content and are not rotated: treat them as sensitive and
enable them only while diagnosing a problem.

Existing servers do not gain crash reporting or fixes from a new binary. Save
work before restarting, or use the separate `-L debug` server above.

development: build, tests, the tmux oracle and the regress harness are in
[docs/development.md](docs/development.md).

ISC licensed; the tmux notice is kept in `LICENSE`.
