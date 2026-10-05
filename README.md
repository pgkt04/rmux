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
