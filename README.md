# rmux

rmux is a Rust port of tmux, targeting drop-in command and terminal behavior on
macOS and Linux. The upstream behavioral pin is **tmux `8f25579c`**, version
`next-3.9` (see `oracle/PIN`). rmux has its own sockets and `RMUX*` environment
namespace; it never connects to a C tmux server.

P0 provides the workspace, oracle, regression/differential harnesses and platform
probes. It does **not** implement a server: `rmux -V` prints its version, `-h`
prints usage, and other valid invocations fail with
`rmux: server not implemented yet (P0)`. P1 adds typed generational server
arenas with deferred lease-based removal and canonical header enums, flags,
and encoded key constants; it does not add terminal or server behavior.

## Build

Stable Rust 1.85 or newer (edition 2024):

```sh
cargo build --workspace
cargo run -p rmux -- -V
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Only `rmux-sys` permits unsafe code. Workspace clippy warnings are errors.

The G00 header-value test extracts `8f25579c` into a temporary directory and
compiles a C reference to compare all enum, key, flag and composite-mask values.
Set `RMUX_TMUX_SOURCE` to the pinned tmux git checkout (default:
`/Users/j/fun/tmux`). The test reports a skip when the checkout or C compiler
is unavailable; libevent headers are required when the reference is present.

The G01 foundation tests (`rmux-util`, `rmux-sys`) compile small C programs
from the pinned `utf8.c`, `utf8-combined.c` and `compat/*.c` (`TMUX_SRC`,
default `$HOME/fun/tmux`) and compare decoder widths for every code point,
vis/unvis, strtonum and base64 against them; the width programs link the same
Homebrew `libutf8proc` the oracle uses (`pkg-config libutf8proc`). On macOS
`rmux-sys` links `libutf8proc` for `utf8proc_charwidth`; Linux uses libc
`wcwidth`. One test drives `oracle/bin/tmux` with `#{w:...}`. Missing
prerequisites print a skip; a failing C compile is a test failure.

The G03 grid tests (`rmux-emu/tests/grid*.rs`) drive a C driver built from the
pinned `grid.c`, `grid-view.c` and `grid-reader.c` (`tests/grid_reference.c`)
with scripted and randomized operation sequences and compare the full grid
dump, `grid_string_cells` output and reader cursors line by line. The oracle
replay feeds `regress/copy-mode-test.txt` to `oracle/bin/tmux` on a private
socket and compares the copy-mode cursor after each word motion.
Set membership stops at the C-string NUL boundary and decodes separators without
per-motion heap allocation. First-column padding, empty grids with history and
resize allocation tails have pinned-behavior regressions; reflow and unwrap
use logical row counts, not spare line storage. P2 accepts the in-process million-
operation grid invariant test; standalone cargo-fuzz targets remain P11 hardening.
The G05 parser owns the end-to-end byte-stream/capture comparison.

The G04 screen port adds lifecycle, selection, resizing and alternate state,
screen writing, reusable row collection, Unicode combination, borders and
preview helpers. `screen::write::TtySink` receives borrowed screen state with
pre-mutation draw snapshots; `ScreenOnlySink` supports emulator-only execution.
Recording-sink tests exercise drawing without a terminal implementation. The
screen C reference compiles pinned `screen.c` and `screen-write.c` in a temporary
directory and compares scripted and randomized operation-boundary grid dumps.
Image integration remains P10; server-owned timers, sync replay and alternate
layout repair stay at the adapter boundary rather than importing server types.

The G05 port (`rmux-emu/src/input/`) is the `input.c` state machine and the
`input-keys.c` key encoder. `InputCtx::parse_step` runs the parser until the
next server-visible point (reply, request, title, bell, OSC 133 event, sync or
alternate switch, ground-timer change) and returns it as an `InputEffect`
borrowing only parser scratch; handlers resume mid-sequence (private mode
lists, WINOPS, OSC 4 pairs, OSC 10/11 style-then-redraw, OSC 133 D
event-then-marker). `InputCtx::parse` drives the steps with a synchronous
`InputSink`. Server glue (pane flags, request FIFOs, timers, `input_key_pane`)
lands with G12/G15; the sixel decode waits for G06. `input::dump` reproduces
`capture-pane -p -e -N -S -`, `-F` and the cursor/mode format line byte for
byte; `tests/input_oracle.rs` feeds 71 streams covering every sequence family
to `oracle/bin/tmux` (`cat` in an 80x24 pane) and to the emulator and requires
exact dumps (three extra byte-split runs compare used cells because storage
rounding depends on writer boundaries). `tests/input_random.rs` runs 2000 random streams for panics,
invariants and whole-versus-split agreement, and `tests/input_parser.rs` and
`tests/input_keys.rs` hold the spec unit cases.

The G07 outer-terminal port (`rmux-tty`) replaces the ncurses runtime with a
compiled terminfo reader and typed parameter interpreter, then applies the pinned
capability overrides, feature table and ACS mappings. `Tty` owns buffered terminal
output, cursor and attributes, lifecycle, flow-control effects and timer requests;
the server must drain effects after each call, synchronously refresh `TtyOptions`
before the next command or hook, and synchronize UTF-8/theme changes through
`TtyHostInfo`. Capability-expanding operations borrow the server's single
`TparmState`, preserving uppercase variables across terminals. `draw_line` also
borrows the shared `HyperlinkRegistry` so its default style can resolve screen
links. G17 maps G04 draw snapshots to per-client `TtyCtx` and applies returned
redraw requests before the next draw; no server lookup lives in `rmux-tty`.
The terminfo tests compile a capability/parameter helper against the oracle's
Homebrew ncurses 6.6 and compare raw capabilities and `infocmp`; tty byte tests
compile pinned C output routines and capture their output buffers. Missing
reference prerequisites report a skip; an available reference must run.
The selected macOS ncurses build supports directory and inline databases, not
hashed databases. Optional sixel output remains with G06/P10.

The G08 port (`rmux-tty/src/keys/`, `rmux-tty/src/key_string.rs`) decodes bytes
from the outer terminal and parses or prints key names. `TtyKeyDecoder` owns
the ternary key tree (xterm templates, raw table, terminfo `k*` capabilities,
then `user-keys` in index order; a later insertion replaces the node an earlier
lookup stops at, as in C; capability and user strings stop at their first NUL,
and an empty user string is inert instead of reading past its terminator), the
bracket-paste flag, the last mouse position and the
escape-timer phase. `next` borrows the unread input and returns one
`DecodeStep`: a key or mouse event with its raw bytes, or a typed terminal reply
(OSC 52 clipboard, OSC 4 palette, OSC 10/11 colours, primary/secondary/extended
DA, DECRPM sync, window size), plus the timer request and theme notification the
server must apply before the next step. Input forms are the C ones: legacy
`ESC [ M` and SGR mouse reports, `ESC [ 27 ; m ; k ~` and `ESC [ k ; m u`
extended keys, and the Meta/NUL/VERASE/C0 byte fallback. `parse_key_name` and
`write_key_name` keep the C quirks (lowercase `0x` only, `^x`, case-sensitive
`User%u`, NUL-terminated 64-byte caller output, `Invalid#` for unprintable values).
Closing a decoder releases its tree storage. The G01 UTF-8 registry is
thread-local, so neither API takes a context argument.
`tests/keys_cref.rs` drives the pinned `tty-keys.c`/`key-string.c` through
`tests/keys_reference.c` with the same scripts (tables, splits at every byte
with and without expiry, replies, timers, 1500 random sequences, names);
`tests/keys.rs` holds the tree, timer, `regress/tty-keys.sh` and oracle
`list-keys` round-trip checks.

## Pinned oracle

Supply an existing tmux git checkout containing the pin. The build script uses
`git archive` into `oracle/build-*`; it never builds in your checkout. Install
C build tools, autoconf, automake, pkg-config, libevent, ncurses and utf8proc
(macOS requires utf8proc). Then:

```sh
TMUX_SRC=/path/to/tmux sh scripts/build-oracle.sh
oracle/bin/tmux -V
```

The macOS oracle uses utf8proc widths, not libc wcwidth, with sixel and jemalloc
off. Its generated binary is ignored by git.

## Regress harness

```sh
cargo run -p rmux-harness --bin regress -- \
  --source /path/to/tmux --binary oracle/bin/tmux \
  --test new-session-base-index.sh --json harness/runs/oracle-one.json
cargo run -p rmux-harness --bin regress -- \
  --source /path/to/tmux --binary target/debug/rmux --rmux \
  --baseline harness/baseline/oracle-macos.json \
  --test new-session-base-index.sh --json harness/runs/rmux-one.json
```

Omit `--test` for all 201 tests; repeat it to select a chunk. `--timeout` sets a
positive per-test limit in seconds (default 120). Each script runs sequentially
from a temporary archive, never the tmux checkout. stdout/stderr, elapsed time,
exit code and pass/fail/timeout are written to JSON. A failing suite exits 1.
Isolated temp directories and exact recorded sockets are used for server cleanup;
the upstream socket-path test intentionally also checks a unique default `/tmp`
socket. Timeout cleanup terminates only the script's own process group, falling
back to killing the script and exact-socket cleanup if group signals are denied.
`RMUX_TEST_TMPDIR` overrides the short `/tmp` harness root; keep its path short
enough for Unix socket limits.
Three upstream scripts (`display-message-client.sh`, `terminal-feature-utf8.sh`,
`new-session-environment.sh`) start clients with `env -i`, which drops the
harness `TMUX_TMPDIR`; the `TEST_TMUX` wrapper restores an unset (not empty)
namespace TMPDIR so those clients reach the server started under the harness
root instead of `/tmp/tmux-<uid>/`. Scripts themselves remain unchanged on
oracle. Note that macOS `/usr/bin/mktemp` ignores `TMPDIR`, so scripts that set
`TMUX_TMPDIR=$(mktemp -d)` place their (unique, self-cleaned) socket directories
under the per-user `/var/folders/.../T/` rather than the harness root.

`harness/regress-manifest.toml` is the checked-in classification: unchanged,
namespace-mapped, or adapted. The one raw-imsg test is `needs-fixture` for rmux
until P7 provides its semantic protocol fixture. Namespace mapping is applied
only with `--rmux` and printed per test. It preserves `TEST_TMUX`, terminfo names
and command behavior. Oracle failures/timeouts from `--baseline` are reported as
`oracle-fail`, not rmux failures. `cargo test` validates exact manifest coverage
against a fresh pinned archive; set `TMUX_SRC` to your checkout.

`harness/baseline/oracle-macos.json` is the unedited output of one full run
(`--timeout 240`, 2026-10-04 UTC) against the oracle on this Mac: **201 pass,
0 fail, 0 timeout**, about 25 minutes wall time. An earlier run reported
`display-message-client.sh` and `terminal-feature-utf8.sh` failing; both were
caused by the harness (`env -i` clients lost `TMUX_TMPDIR`, see above), not by
upstream, and both pass by hand in a clean environment. These are host-specific
observations, not Linux claims.

## Differential harness and probes

`rmux_harness::differential::compare` starts two binaries on separate temporary
sockets, applies the same ordered commands and compares status/stdout/stderr.
The unit tests prove oracle-vs-oracle parity for display, capture-pane and
list-panes, and deliberately change a session name to prove mismatch detection.
They use `oracle/bin/tmux` or `RMUX_ORACLE`; without an oracle they print a skip.

```sh
cargo test -p rmux-harness differential -- --nocapture
cargo run -p rmux-sys --example readiness
```

Measured readiness and pinned C grid layouts, reproduction commands and the
macOS EventLoop decision are in [docs/p0-probes.md](docs/p0-probes.md).

ISC licensed; the tmux notice is retained in `LICENSE`.
