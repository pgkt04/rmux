# P0 probes

Measured 2026-10-04 UTC on macOS 25.6.0, arm64. Linux has not been measured.

## Descriptor readiness

Command: `cargo run -p rmux-sys --example readiness` (mio 1.2.4, kqueue on macOS).
The probe uses raw ptys, writes to the opposite endpoint before testing readable
interest, and registers read and write interest separately. It also measures
poll(2) on the same descriptor without consuming the test byte.

| Descriptor | mio read | mio write | poll read | poll write |
|---|---|---|---|---|
| pty master | ready | ready | ready | ready |
| client tty (pty slave) | ready | ready | ready | ready |
| /dev/null | EINVAL on registration | EINVAL on registration | POLLNVAL (32) | POLLNVAL (32) |
| pipe (respective ends) | ready | ready | ready | ready |
| Unix socket | ready | ready | ready | ready |

**macOS EventLoop default: mio (kqueue)**, by Main's revised P0 decision:
it supports pty master/slave, pipes and sockets, while poll adds no support for
`/dev/null`. G01/G14 must handle `/dev/null` and descriptors rejected by
registration with EINVAL as null endpoints without registration: reads yield
immediate EOF, writes discard/complete immediately. This requirement is not
implemented in P0. Regular files remain direct bounded I/O.
Pinned tmux disables kqueue and poll (`osdep-darwin.c:98-107`) and uses libevent
select on macOS; select was not probed here. The probe is portable to Linux;
CI runs it but no Linux result is claimed here.

## C grid ABI

Source: pinned `tmux.h` at `8f25579c`, extracted by `git archive` into
`/tmp/swarm-rmux-p0/source`. Probe: `scripts/grid-layout.c`.

Command:

```sh
cc -DHAVE_CLOCK_GETTIME -DHAVE_EVENT2_EVENT_H -DHAVE_SYS_QUEUE_H \
  -DHAVE_SYS_TREE_H -DHAVE_BITSTRING_H -DHAVE_U_INT -DHAVE_U_CHAR \
  -I/tmp/swarm-rmux-p0/source -I/opt/homebrew/opt/libevent/include \
  scripts/grid-layout.c -o /tmp/swarm-rmux-p0/grid-layout
/tmp/swarm-rmux-p0/grid-layout
```

The header compiled with platform feature macros (without `HAVE_CLOCK_GETTIME`,
its fallback prototype conflicts with the SDK). No struct definitions were copied.

| Struct | sizeof | offsetof fields |
|---|---:|---|
| grid_line | 40 | celldata=0, extddata=8, cellused=16, cellsize=18, extdsize=20, time=24, osc133_data=28, flags=38 |
| grid_cell | 56 | data=0, attr=36, flags=38, fg=40, bg=44, us=48, link=52 |
| grid_cell_entry | 5 | offset=0, data=0, flags=4 |
| grid_extd_entry | 23 | data=0, attr=4, flags=6, fg=7, bg=11, us=15, link=19 |

These are C accounting sizes, not proposed Rust struct sizes.

## Oracle width configuration

The pinned oracle is `tmux next-3.9`, utf8proc enabled, sixel disabled, jemalloc
disabled. On Darwin, `configure.ac:449-457` requires utf8proc; stock macOS tmux
therefore uses utf8proc widths, not libc wcwidth. Later width parity work must
compare with this oracle configuration rather than assume libc agrees.
