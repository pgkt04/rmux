# rmux

A Rust port of tmux for Linux and macOS. Same commands, keys, and config;
separate sessions and sockets, so it can run alongside tmux.

In [Tern](https://stencil.so/tern), apps such as omp can render natively.

## Install

Requires Rust 1.85+, a C compiler, pkg-config, and ncurses.
On macOS, also install utf8proc: `brew install utf8proc`.

```sh
git clone https://github.com/pgkt04/rmux.git
cd rmux
cargo install --locked --path crates/rmux
```

Optional: add `--features sixel` for images or `--features systemd` for
Linux socket activation.

## Use

```sh
rmux                    # start a session
rmux attach             # reattach
rmux ls                 # list sessions
rmux new -s NAME         # start a named session
rmux kill-server        # end all sessions
```

Config is read from `/etc/tmux.conf`, `~/.tmux.conf`, and
`~/.config/tmux/tmux.conf`. Reload changes with:

```sh
rmux source-file ~/.tmux.conf
```

Press **Ctrl-b**, then a key:

| Key | Action |
| --- | --- |
| `%` / `"` | Split side by side / top and bottom |
| `o` | Switch pane |
| `z` | Zoom or unzoom |
| `,` | Rename window |
| `:` | Enter a command |
| `d` | Detach |

## Native omp in Tern

Install the bundled plugin once, then start a fresh omp inside rmux:

```sh
rmux omp-plugin ~/.omp/local-plugins/rmux-terminal
omp plugin install ~/.omp/local-plugins/rmux-terminal
rmux
# Inside the rmux session:
omp
```

The plugin switches between native and text views without losing your
conversation or draft. Splits use text; zoom a pane or close the other pane
to return to native. Rename and command prompts stay native.

The native bottom bar uses your configured status text, with Tern's styling.
Use `/terminal-reprobe` in omp if the view gets stuck, or `PI_TUI_NATIVE=0 omp`
to use text only.

After updating rmux, re-export the plugin and start a fresh omp. Existing
servers keep their old code. To try the new version without ending sessions:

```sh
rmux -L updated new-session
```

**Save work before restarting a server: restarting ends its sessions.**

## Debugging

Panic and fatal-error reports are saved in `~/.local/state/rmux/`
(or `$XDG_STATE_HOME/rmux/`) without enabling debug logs.

For detailed logs, run `rmux -vv -L debug new-session` from a writable directory.
It writes `rmux-*.log` files there, including terminal output. Treat these logs
as sensitive; they are not rotated.

Builds, tests, native rendering details, and diagnostic limits:
[development guide](docs/development.md).

ISC licensed; see [LICENSE](LICENSE).
