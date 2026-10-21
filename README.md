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

## Install and update

Releases ship prebuilt binaries for `aarch64-apple-darwin`,
`x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu` (glibc 2.17+).
Download `rmux-<tag>-<target>.tar.gz` and `SHA256SUMS` from
[releases](https://github.com/pgkt04/rmux/releases), then:

```sh
sha256sum --ignore-missing -c SHA256SUMS   # macOS: shasum -a 256 --ignore-missing -c
tar -xzf rmux-v0.2.0-x86_64-unknown-linux-gnu.tar.gz
mv rmux ~/.local/bin/                      # anywhere on PATH
```

After that, rmux updates itself:

```sh
rmux update                     # install the latest release
rmux update --check             # exit 10 if a newer release exists
rmux update --version v0.2.0    # install that release (also downgrades)
rmux update --to /home/aria/.local/bin/rmux   # also update another copy, e.g. as root
rmux version                    # release, commit and target of this binary
```

It keeps the old binary next to the new one as
`rmux.rollback-<date>-pre-v<old>`. Running servers are never restarted: they
keep the old build until `kill-server`, which ends their sessions.

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
rmux install-plugin
rmux
# Inside the rmux session:
omp
```

`rmux install-plugin` writes the plugin to `~/.omp/local-plugins/rmux-terminal`
and runs `omp plugin install` on it. Pass `--dir PATH` to use another location
(and pass the same `--dir` to the other plugin commands). `rmux uninstall-plugin`
removes it again.

The plugin switches between native and text views without losing your
conversation or draft. Splits use text; zoom a pane or close the other pane
to return to native. Rename and command prompts stay native.

The native bottom bar keeps your configured status text, colors and spacing,
including window highlights, separators and inline `#[...]` styles.
`bg=default` leaves it transparent. In light themes, 256-color and RGB colors
too faint to read are darkened (or lightened on dark backgrounds), as Tern does
for terminal text.
Exact colors require a host with TSP `styles` and `el` support; older hosts use
their own styling. Native layout and font sizing still follow the host.
Use `/terminal-reprobe` in omp if the view gets stuck, or `PI_TUI_NATIVE=0 omp`
to use text only.

To update, run `rmux update`, then `rmux update-plugin`, then start a fresh
omp. `rmux update` tells you when the bundled plugin changed;
`rmux update-plugin --check` exits 10 when the bundled plugin differs from the
installed one, without changing anything. Existing servers keep their old code.
To try the new version without ending sessions:

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

To log a server that is already running, send it `SIGUSR2` (`kill -USR2 <pid>`)
once to start and once to stop; it writes `rmux-server-<pid>.log` in the
server's working directory. Native frames log their handling time as
`tsp …: frame … <µs>us`.

Builds, tests, native rendering details, and diagnostic limits:
[development guide](docs/development.md).

ISC licensed; see [LICENSE](LICENSE).
