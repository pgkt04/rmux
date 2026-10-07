rmux

A Rust port of tmux.

Same commands, key bindings, config files and formats as tmux (pinned to
tmux `8f25579c`, `next-3.9`). It passes all 201 of tmux's own regress tests
on macOS and Linux. rmux keeps its own sockets (`/tmp/rmux-<uid>/`, or
`$RMUX_TMPDIR`) and `$RMUX` variable, so it runs beside tmux and never
talks to a tmux server.

Supports the [Tern](https://stencil.so/tern) terminal: a program that
speaks the Tern Surface Protocol, such as omp, renders natively when it
runs in an rmux pane inside Tern. While a plain terminal is also attached,
every viewer sees only the normal grid, without the native view.

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

`#{pane_tsp}` and `#{client_tsp}` show the state (`rmux display -p
'#{pane_tsp}'`). Start the server with `RMUX_TSP_BROKER=0` to turn native
rendering off, or set `PI_TUI_NATIVE=0` to keep omp on its text renderer.

development: build, tests, the tmux oracle and the regress harness are in
[docs/development.md](docs/development.md).

ISC licensed; the tmux notice is kept in `LICENSE`.
