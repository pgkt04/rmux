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

`#{pane_tsp}` and `#{client_tsp}` show the state (`rmux display -p
'#{pane_tsp}'`). Start the server with `RMUX_TSP_BROKER=0` to turn native
rendering off, or set `PI_TUI_NATIVE=0` to keep omp on its text renderer.

development: build, tests, the tmux oracle and the regress harness are in
[docs/development.md](docs/development.md).

ISC licensed; the tmux notice is kept in `LICENSE`.
