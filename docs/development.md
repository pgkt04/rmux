# rmux development notes

How rmux is built and tested, and how each ported module behaves. The user
README is [../README.md](../README.md).

rmux is a Rust port of tmux, targeting drop-in command and terminal behavior on
macOS and Linux. The upstream behavioral pin is **tmux `8f25579c`**, version
`next-3.9` (see `oracle/PIN`). rmux has its own sockets and `RMUX*` environment
namespace; it never connects to a C tmux server.

P1 added typed generational server arenas with deferred lease-based removal and
canonical header enums, flags, and encoded key constants.

The G12 model (`rmux-server::model`) owns generation-bearing session, group,
winlink, window and pane arenas, separate public-id indexes, ordered selection
history, paste buffers and format subscriptions. Model mutations expose owned
events and timer requests; the event loop and client/redraw owners consume them
in P7/P8. Pane parsing preserves synchronous emulator barriers, including
alternate-screen geometry repair, and request replies follow a pane-owned FIFO.
Process launch prepares arguments and environment before fork and changes cwd
only in the child. This deliberately does not preserve the pinned `spawn.c`
parent-cwd change on fork failure: the parent's cwd remains unchanged on every
launch path. The child still tries requested cwd, home and `/`, and sets PWD to
the selected directory. Model tests cover lifecycle/selection ordering, store
semantics, monitor cache identity, resize and alerts, plus pinned C/oracle
comparisons and a real pty command parsed into a pane screen.
Spawned pane, job and pipe children share allocation-free descriptor cleanup:
macOS enumerates open descriptors in fixed stack batches instead of closing
every number below `OPEN_MAX`; Linux uses `close_range`. Both retain the
bounded-descriptor fallback if the native operation fails.
`PaneInputHost` supplies synchronous pipe/control delivery, draw and geometry
repair. `PaneModeDriver` and `PanePromptEngine` delegate to the owning mode and
prompt engines rather than installing parallel engines in the model. Server
`model_event` and `option_monitor_removed` dispatch callbacks run synchronously;
in particular, a window-closed callback may retain the window before its final
release. Hook payloads preserve pane/window transitions and prompt types;
activity updates queue alerts, and option-owner destruction releases scoped
hook monitors. Hook-monitor subscriptions require an installed dispatch callback.
`ModelWithClients` lends live G15 client views to G11 target lookup without
creating a second client arena. `rmux_sys::pty::{LaunchOptions, PreparedLaunch,
LaunchedProcess}` is the owned launch boundary.


The G10 format core (`rmux-server::format`) expands arbitrary byte strings with
the pinned lookup order, lazy callbacks/conditionals, POSIX regex modifiers,
time conversion, ordered model/options/environment loops and styled widths.
Its 214-entry registry computes model and emulator facts directly; borrowed
`FormatExternal` facts supply client, mouse, mode, default-colour and pipe data
owned by later phases. Owner and evaluated clients remain distinct, including
job namespaces and legacy layout selection. `ServerFormatRuntime` connects
real job transports and client leases; the model-only runtime emits owned
actions rather than claiming an unstarted child exists. Cached jobs reject
stale callbacks and preserve the pinned update/completion line distinction;
animation uses one generation-checked 100-ms owner timer. G11 argument,
configuration-condition and hook expansion use this same engine.
Mouse word, line and hyperlink formats use copy/view-mode content while those
modes are active rather than the live pane grid.
Format tests include a private-socket pinned-oracle corpus and extracted-C
registry, quoting and time comparisons. Full command-driven regression tests
remain dependent on the P7 server and later client/redraw/mode consumers.

G10 helpers provide restricted byte-preserving JSON, duplicate-preserving model
sorting, POSIX substitution, scored fuzzy masks and eight-section styled drawing.
Drawing comparisons include cells, colours, attributes, links, ranges and cursor
restoration, not just visible text. JSON/substitution tests compile extracted C
automatically; full drawing/fuzzy/sort references can be rebuilt with:

```sh
python3 scripts/build-format-drivers.py --tmux-source /Users/j/fun/tmux --output /tmp/swarm-rmux-build/g10-reference
export RMUX_G10_HELPER_DRIVER=/tmp/swarm-rmux-build/g10-reference/helper-driver
export RMUX_FORMAT_HELPER_DRIVER=/tmp/swarm-rmux-build/g10-reference/helper-driver
export RMUX_G10_SORT_DRIVER=/tmp/swarm-rmux-build/g10-reference/sort-driver
cargo test -p rmux-server pinned_
```

The full reference build needs the oracle's compiler, autotools, libevent,
ncurses and (on macOS) utf8proc dependencies. Missing references report a clear
skip; supplied references must match. Existing configured oracle objects can
also be linked with `scripts/build-format-helper-driver.sh BUILD_DIR OUTPUT`.
Run `TMUX_SRC=/Users/j/fun/tmux cargo test -p rmux-server --test format_json_regsub`
for automatic JSON/substitution C extraction; `RMUX_JSON_REGSUB_DRIVER` selects
an already compiled driver for the parser, typed accessor and substitution corpus.

The G16 control transport (`rmux-server::control`) keeps LF command framing,
typed command guards, deferred notification FIFO, independent pane OFF/PAUSED
flags, and two retained consumer offsets. Its reply-barrier chain and per-pane
chains share generation-bearing block slots; raw bytes remain in the model pane
buffer and are octal-encoded only when serviced. Control notification sinks are
registered before hooks. Subscription timers use the model monitor engine.
Regular identified input/output descriptors use bounded direct I/O scheduled on
the server loop, rather than mio registration; pipe/socket/tty endpoints retain
normal readiness ownership. The differential also attaches to an existing
session with regular-file stdout to cover that startup path.
Run `cargo test -p rmux-server --test control_oracle` for the private-socket
command-stream differential. It builds rmux in the running test's Cargo target
directory; `RMUX_CONTROL_BINARY` explicitly selects another binary.
Only guard timestamps and command sequence numbers are normalized; protocol
bytes and notification ordering remain exact.

The G20 command handlers connect session/window creation, pane break/join/move,
pane capture, prompts, menus and modal popups to the model and UI owners. Initial session dimensions
are passed explicitly to window spawning; repeated environment overrides apply
after client environment updates. Prompt cancellation and failed asynchronous
conditions release prepared command state and resume waiting queue items.
Multi-input prompts update the in-flight engine before accepting the next
answer, preserving independent indexed template values and input prefills.
Selecting a pane with `attach-session` invalidates the target and former active
pane contents when their active/inactive styles differ, before switching panes;
unchanged styles and unrelated pane contents do not need a redraw.
Run the private-socket G20 creation/capture/pane-transfer comparison with
`RMUX_COMMANDS_A_BINARY=target/debug/rmux cargo test -p rmux-server --test commands_a_oracle`.

G06 SIXEL support is off by default; build `rmux` with `--features sixel`.
The server owns one image registry, including saved alternate-screen images,
with the pinned nineteen-image global FIFO limit. Pane screens explicitly bind
image owners; text-only status, menu and format screens remain image-free.
The codec preserves sparse rows, exact compression and full cached fallback
bytes for clients without SIXEL or known pixel metrics. `#{sixel_support}` and
graphics replies report the compile-time feature, not outer-client capability.
Direct pinned C tests compare codec state, registry orders and screen hooks;
`images_oracle` compares graphics replies and outer-terminal image/control bytes
using `RMUX_BIN` and a separate `RMUX_SIXEL_ORACLE` built with `--enable-sixel`.
The ordinary `oracle/bin/tmux` stays feature-disabled. Bounded randomized codec
pipelines supplement these exact-byte comparisons.

Linux `--features systemd` enables socket activation without libsystemd.
The process-start boundary harvests inherited descriptors exactly once before
other descriptor opens, clears activation environment variables, and passes an
owned listener into the `-D` foreground server. It adopts the listener's actual
pathname without unlinking or rebinding. Normal daemon startup drops activation
and binds normally, matching the pin's changed-PID behavior after fork.
Activation has no effect on macOS.

G18 copy/view modes (`rmux-server::modes::copy`) own snapshot or parser-backed
history, absolute selection anchors and a separate visible screen. Snapshot
refresh reconciles monotonic scroll counters in place and falls back to cloning
only for changed geometry, generation or invalid balance. Character/word/line
and rectangular extraction preserve tmux's vi/emacs endpoints, raw UTF-8 bytes,
tabs, ACS translation, soft wraps and final-newline rules. Search uses libc
POSIX regex, byte smart-case and viewport generation marks with row-level
200-ms/10-s deadlines. Styles, line-number gutters, position indicators, marks,
selection and lazy cursor/search formats share the existing format/UI engines.
Refresh, drag and command-view parser recovery timers resolve exact ModeIds;
mode teardown cancels registrations and releases both screens and hyperlink
leases. Run `cargo test -p rmux-server modes::copy`; pinned-C differentials
include failure switches `RMUX_COPY_SNAPSHOT_MUTATE`, `RMUX_COPY_RENDER_MUTATE`
and `RMUX_COPY_SEARCH_MUTATE` (setting any switch must fail its comparison).
The CLI differentials `copy_core_oracle` and `copy_commands_oracle` require a
built binary (`RMUX_COPY_BINARY`) and the pinned oracle (`RMUX_ORACLE`). Core
cases compare parsed PTY cells/styles/cursor and exact clipboard payload/order,
not renderer packetization. Command cases use two private key-mode lanes,
retaining all 99 commands in both modes and distributing prefixes 1, 2 and 5.
The ignored `exhaustive_copy_commands_modes_prefixes_match_pinned_oracle` test
runs every command/mode/prefix combination on demand with `--ignored`.
Generic argument errors and read-only checks share representative dispatch probes.

## Build

Stable Rust 1.85 or newer (edition 2024):

```sh
cargo build --workspace
cargo run -p rmux -- -V
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace       # fast default run
cargo nextest run --workspace -P full
cargo test --workspace --doc
```

Before publishing changes, run the formatting and full-workspace Clippy gates
above as well as the tests. CI uses the current stable toolchain on Linux and
macOS; a passing targeted test does not cover formatting or new stable lints.

Tests run under [cargo-nextest](https://nexte.st) (`brew install cargo-nextest`),
configured in `.config/nextest.toml`. The default profile skips the copy-mode
oracle grids (`copy_core_oracle`, `copy_commands_oracle`) and three oracle
matrices of 90 s or more each (two in `g19_modes_rest`, plus
`client_screen_matches_oracle`). It runs pty oracle tests at most two at a time
and kills a test that runs longer than 3 minutes. The
`full` profile runs every test with a 20-minute limit per test. Plain
`cargo test --workspace` still works but runs the test binaries one after
another with no hang limit.

Most oracle tests drive the rmux binary at `$CARGO_TARGET_DIR/debug/rmux` (or
`target/debug/rmux`) as it is; a test run does not rebuild it, because it is
another package's binary. Run `cargo build -p rmux` first, or the tests compare
the oracle against an old build.

Only `rmux-sys` permits unsafe code. Workspace clippy warnings are errors.

The re-executed server installs its own handled signal dispositions and unblocks
signals only after its wake descriptor is registered. A client's inherited blocked
or ignored `SIGTERM` cannot leave an idle server immune to shutdown; the CLI
regression exercises that process boundary in an isolated child.

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
`tests/input_keys.rs` hold the spec unit cases. `send-keys -R` reads the current
`extended-keys` option on each reset: `always` restores mode 1, including extended
Ctrl-Tab and Ctrl-Shift-Tab sequences; `on` and `off` restore standard mode.

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

The G09 option table and environment store (`rmux-server::options`) retain all
269 pinned entries and seven aliases. Native builds probe ncurses `setupterm`
for `screen-256color`, `tmux`, then `tmux-256color`, matching `configure.ac`;
cross builds default to `screen`. Set `RMUX_DEFAULT_TERM` at build time to match
an oracle configured with `--with-TERM`. Linux selects `vlock` when present in
the build PATH, otherwise `lock -np`; `RMUX_LOCK_COMMAND` explicitly overrides
that configured default. Mouse defaults to on on both platforms, as in
`Makefile.am`. The `systemd` feature clears activation variables in child
environments. Generated pane identity uses `RMUX` and `RMUX_PANE`. The P12 TSP
broker starts enabled; `RMUX_TSP_BROKER=0` in the server process disables it
for that process, and there is no live option to change it afterward. An
explicit `refresh-client` still re-probes. Pane programs
receive `TERM_PROGRAM=rmux`, the actual rmux package version in
`TERM_PROGRAM_VERSION`, and `RMUX_TSP=1` only while the broker is enabled at
spawn; configured `TERM` is preserved. While the broker is enabled, a pane
without its own `PI_TUI_NATIVE` gets `PI_TUI_NATIVE=1`: released omp skips its
TSP probe under a tmux `TERM` unless that override asks for it, and an explicit
`PI_TUI_NATIVE=0` still wins. Read-only formats
`client_tsp` (`unknown`, `no`, `v1`), `pane_tsp` (`ansi`, `switching`, `native`,
`detached`), and `pane_tsp_epoch` report broker state, not terminal-environment
guesses. `pane_tsp_view` reports the desired renderer independently of the app's
chosen renderer: `native`, `ansi`, `detached` (no physical viewers), or `pending`
(a visible client's TSP probe is unresolved). Control observers do not count as
viewers. Changes expose an ordered `OptionsChange` plan for the G14 host;
monitor removal releases values before monitor cleanup and option unlink.
TSP replay holds one immutable revision across tty queue-full retries; subsequent
frames and latest palette/sheet updates stay pending until that revision is
admitted. Source-op coverage becomes visible only at frame admission, and blob
base64/APC bodies are generated as the tty drains rather than eagerly buffered.

The rmux-specific `rmux-reprobe`/epoch/`rmux-ready` contract selects one renderer
for every viewer of a pane: a plain or read-only plain attachment forces all
viewers to ANSI; a client on another window does not. Native rendering requires
only eligible TSP viewers and a sole visible or zoomed pane, without cropping,
panning, floating panes, popups, pane modes, or menus. Client command prompts
and messages remain native when the viewer supports the dock/status kinds
(prompts also require `input`); other overlays retain the cell fallback. The
outer surface hides cell status, borders, titles, and scrollbars without
changing saved options or layout. Its native dock includes the status strip and
active prompt/message; prefix keys remain rmux input. With `styles` and `el`,
the strip projects color/attribute runs from the same rendered status grid as
the tty, retaining inline styles and highlighted padding. Default backgrounds
remain transparent; default foregrounds and ANSI 0–15 colors use Tern's terminal
variables rather than the app palette. The broker-owned sheet is sent before
the matching frame and replaced when styles change, including after a blocked
send. Without those capabilities, text and fitting remain host-styled.
Opening a native projection clears cell mouse reporting
before the surface open, letting Tern scroll locally without a resize. Cell UI
restores its own mouse modes; stray native wheel reports do not enter copy mode.
Surface visibility is reported to the program, not used as projection eligibility:
an offscreen surface stays open so its next visible event can still be routed.
For broker-aware programs, UI entry first requests a complete ANSI paint;
leaving the ineligible view can return the same live program to a fresh native document.
This is not simultaneous native/ANSI rendering or a TSP-to-cells converter.

A program whose `hello` lacks the broker feature (any TSP program, released omp
included) is a stock program: it negotiates once and cannot be asked to switch.
It gets a real `hello` reply only when it starts with only eligible TSP viewers,
and then stays native until it exits or sends its next `hello`; otherwise it
gets no reply and paints ANSI rows. A native stock program survives detach and
reattach (the retained document replays) and new TSP viewers. While a plain
viewer is attached or the pane is otherwise ineligible, viewers see the plain
grid, which a native program does not paint; keys still reach the program, and
native rendering resumes once the pane is eligible again. rmux acknowledges a
stock program's frames itself whenever no viewer draws them.
Stock programs enter cell modes and menus immediately on the existing grid,
without waiting for an ANSI repaint they cannot provide. Leaving the cell UI
replays the retained native document; copy-mode commands never block later
prefix keys such as detach.
OSC 133 prompt zones do not end a registered reader while its captured process
group still owns the pane PTY. omp's text user-message renderer emits these
zones too; treating them as a shell handoff erased the hello and prevented
split-to-native recovery. Actual foreground-group changes still retire the
reader, and prompt markers without an identifiable live group keep the existing
teardown behavior. Regression coverage uses a real PTY foreground program;
real-omp prefix split/kill recovery preserves the PID/draft and epoch after
the same prompt-zone sequence.
Native prompts use generation-bound `rmux:prompt:*` input nodes and the existing
tmux prompt engine. UTF-16 edit events validate pre-edit length and scalar
boundaries, map the raw host cursor through C0/DEL sanitization, and resynchronize
rejected edits. Stale IDs and app edits while the client prompt owns input are
ignored. Host send uses normal Enter continuation; native undo keeps at most
100 edits and raw text mutation resets it. Vi command mode and empty
backspace-exit prompts are readonly so their keys use rmux's existing command
engine. Messages retain readonly prompt focus; dock updates keep the input node
in place without moving it, including multi-line status bars.
Program hellos are forwarded unchanged: no feature is injected into generic
apps, and prompts use raw keys when the program did not request host editing.
Cancel/submit restore the canonical application focus without changing
its document. Stock-native column-only changes update the contract and resize
event in place; capability/limit changes still replace the projection.
Exact outgoing app `hello` is cached per tty generation so retained-pane
projection replacements skip redundant queries/DA1. Each replacement still
closes/opens and replays the retained document; no unsupported hidden-surface
cache is implied.
Replay serialization reads canonical nodes/properties directly rather than
allocating a temporary JSON document. One topology pass prepares routing and
error coverage; root properties, child/settled order, focus/suspension, and all
five elapsed-age paths retain their semantics. Blob discovery and the client
send path borrow retained state instead of cloning properties, confirmed blob
sets, and open metadata on each tick. Chunk buffers reserve their final payload
capacity only when the tty drains them; queued replay revisions remain immutable.

A release preparation benchmark with 4,100 retained nodes dropped from 6.9 to
2.6 ms and about 23.4 to 6.1 MB of allocated bytes; 16,388 nodes dropped from
52.2 to 23.4 ms and 93.5 to 24.3 MB. Encoded frame sizes were unchanged. Matched
real-omp PTY switch smoke for a small document stayed about 1.7 ms to first
frame before/after: these optimizations target large replay preparation, not
Tern rendering time or reduced wire payload. Native rename, split/zoom, and
500 rapid resizes retained the app/draft without protocol errors.
Native tty teardown closes the outer surface and restores terminal modes before
a final owned DA1 barrier. rmux keeps raw input active to consume delayed TSP
replies and delays `MSG_EXITED` until the barrier completes. If the terminal
does not answer, a one-second timeout after output drains releases the tty;
pending replies are drained before restoring the shell's termios.

`crates/rmux/omp/{package.json,rmux.ts}` is embedded in the executable;
`rmux omp-plugin DIRECTORY` exports the managed `rmux-terminal` package, while
the command without a directory prints the standalone extension. Cargo packages
include both files. `omp plugin install DIRECTORY` registers the package so it
appears in `/plugins`; do not also load a loose copy of the extension.
The extension uses omp's public widget factory to obtain the live TUI and
stops/starts only that UI to repeat
the stock program's `hello`, never restarting the agent or changing sessions.
A read-only `rmux -C` observer with `no-output,ignore-size` subscribes to
`pane_tsp_view`. Those subscriptions are reevaluated immediately after broker
recomputation; unrelated monitor formats keep their normal periodic cadence.
The plugin coalesces subscription changes and TUI starts into a managed one-shot
reconciliation. It ignores detached/pending desired views, settles native-to-ANSI
changes for 150 ms, and waits for negotiation/editor ownership with a 50 ms
one-shot. Persistent mismatch gets at most three reprobes with 100/250/1000 ms
backoff, then a warning; transient pending/detached states do not reset that
budget. Matched renderers have no background reconciliation timer.
It does not restart while `process.stdin.isPaused()` indicates an external UI
owns the tty; lookup/reprobe completions are generation-bound. Listeners/timers
are disposed on session replacement, shutdown, and observer completion/failure.
`/terminal-reprobe` repeats negotiation manually. The extension leaves the
caller environment unchanged, honors startup `PI_TUI_NATIVE=0`, and terminates
its observer on session shutdown. No omp source changes or private-field
patches are required. Real omp 18.8.3 PTY acceptance covers automatic native
to ANSI to native, mixed viewers, split/zoom, retained app PID/session/draft,
and an existing transcript entry rendered by both backends. Two real omp
instances were exercised in side-by-side and top/bottom layouts: both text,
zoom either pane to native, unzoom back to text, then close one and restore the
remaining pane to native, with independent session/draft/transcript retention.
Real external-editor acceptance reproduced the old plugin reclaiming input
while the editor was live and confirmed the corrected plugin waits for exit.
A controlled transient split during hello negotiation reproduced `ansi/native`
actual/desired mismatch before and `native/native` convergence after the fix.
Real omp 18.8.4 PTY acceptance additionally covers native window/session/pane
rename prompts, UTF-16 prompt edits, cancel and colon commands without replacing
the outer surface, 20 native window switches without additional hello queries,
and prefix swap/break-pane, split/zoom/unzoom, and mixed/plain viewer removal
returning to native. A 2,000-resize stress run (including 1x1, status messages,
and a rename prompt) retained the app PID and ended with a live native document
and no protocol errors; it did not reproduce the reported server crash.
The installed rsttyd browser terminal does not advertise TSP: browser smoke
verified its real ANSI rename prompt; native rendering was verified with the
protocol-aware PTY surface simulator, not a visual Tern renderer.
Post-review real-omp acceptance also verified newline/tab paste cursor mapping,
host undo/submit, vi command mode after the escape timer, message-over-prompt
ownership, and a transient split/zoom without restarting the TUI. A further
2,000-resize run kept the server/native document alive with no TUI restarts.
Deterministic observer runtime smoke verified bounded recovery across exhausted,
pending/detached, and manual-reprobe states.

The executable installs panic/fatal reporting for both process roles before
starting the client/internal server. Reports go to an owned state directory
under absolute `XDG_STATE_HOME` or `$HOME/.local/state`, with 0600 files and forced
backtraces independent of `RUST_BACKTRACE`/`-v`. No env/terminal-content dump is
included. Fatal diagnostics are saved before debug logging. Release builds
keep line tables; OS-signal/OOM termination needs OS diagnostics/core dumps.

Transitions hold pane input until matching ready, with a 64 KiB admission bound
and tty backpressure. A missing completion after five seconds closes projections
and shows a diagnostic plus the last real grid, not a fabricated usable program
view. Detached native documents remain retained; a broker-aware program's frame
credits require actual draws from every current native viewer and are never
acknowledged without viewers.
`capture-pane` still captures the grid, not a semantic transcript. Replay and
automated fake-terminal checks do not replace the real omp/Tern smoke acceptance.

The G11 command framework (`rmux-server::cmd`) ports the handwritten configuration
lexer and grammar, argument parsing and printing, the 92-command metadata registry,
target resolution, linked command queues, the 308 default bindings, configuration
barriers and hook insertion/monitors. Command bodies remain G20/G21; the absent
server, format and model groups connect through explicit parser, model and runtime
traits, rather than placeholder production objects. Oracle comparisons use private
sockets with `-f/dev/null`; parser fixtures compare `source-file -n -v`, and binding
fixtures compare `list-keys` and `list-keys -N`. Prepared command state releases its
client lease explicitly on completion or cancellation. Hook option removal frees
the value, destroys its monitor sink/set, then unlinks the option. For C invalid-union
or empty-string under-read paths, percentage helpers return a normal numeric error
instead of dereferencing invalid storage.
The lexer preserves C's entrypoint distinction: a signed buffer byte `0xff` is
EOF, while file `getc` returns byte 255. `source-file` uses the buffer path, so
oracle fuzz comparisons use that same path; file parsing is exercised separately.
Variable-name byte classification uses safe libc ctype bridges under the startup
locale, checked for every byte against C in C and UTF-8 locales.
`parse::OptionsParseContext` uses G09's actual option store and global environment;
the host supplies condition formatting, home lookup and verbose output through
`ParseServices`. Default-key initialization errors must be fatal at the host
boundary. Update pinned binding data with
`python3 crates/rmux-server/src/cmd/key_bindings/generate_defaults.py <key-bindings.c> <defaults.rs>`;
its `--check` mode compares decoded literals independently of Rust formatting.
The ignored `parser_fuzz_ten_minutes_parseonly_oracle_status` test accepts
`RMUX_CMD_FUZZ_SECONDS` and `RMUX_CMD_FUZZ_SEED`; two 300-second runs with seeds
1 and 2 provide the ten-minute parser/oracle acceptance run within bounded jobs.

The G13 layout port (`rmux-server::layout`) keeps the cell tree in the server
arena (`Server.layout_cells`, `LayoutCellId`) and reaches the window and pane
model through the `LayoutHost` trait, implemented for `model::Server` in
`layout/host.rs`. `tree.rs` is `layout.c` (split, destroy, resize, spread,
floating cells, tile and untile), `custom.rs` is `layout-custom.c` (v2 JSON and
legacy v1 strings with the rotate-and-add checksum, byte-exact parse causes)
and `set.rs` is `layout-set.c` (the seven presets). C unsigned wrap is kept
where it is observable (float clamping, tiled sizes); one deliberate deviation:
a float split in a window too small for its borders reports `no space for a new
pane` where the pinned server crashes. Differential tests start a private
oracle server and compare `#{window_layout}`, the control-client v1 dump, the
window size and every pane's geometry after each step of split, `new-pane`,
`resize-pane`, `resize-window`, preset and kill sequences, plus a seeded random
walk; they skip when `oracle/bin/tmux` is missing. `parse_fuzz_never_panics`
accepts `RMUX_LAYOUT_FUZZ_SECONDS` and `RMUX_LAYOUT_FUZZ_SEED`.

The G14 server runtime extends the existing model `Server`; it does not wrap a
second object graph. A single-threaded mio loop owns generational registrations,
monotonic timers and deferred callbacks. Rejected null endpoints complete reads
as EOF and writes immediately; regular files advance in bounded 64 KiB turns.
Pane parsing restores the client root before synchronous effects, hooks and
control notifications. Command queues run to a fixed point before client redraw
and the exit predicate. ACLs use verified socket credentials (UID before the
primary GID), and jobs complete only after both child status and output closure.
Freeing a live job and killing all jobs signal owned child PIDs with SIGTERM;
transferring a job moves its PID and descriptor without signaling or closing them.

Attached client terminals participate in the same event loop: `ClientTty`
readiness drains queued tty output and decodes outer-terminal keys and replies;
`ClientTtyTimer` delivers start, clipboard, flow-control and escape timers.
Read/write interests are rearmed from the tty state, and client teardown cancels
registrations and timers. The UI oracle compares the scene before detach restores
the outer screen, so border/status/prompt/menu rendering—not the detach message—
must match the pinned binary.
Applied terminal features (including UTF-8) synchronize back to the client flags
and format snapshot. Control reads deliver data before a later EOF callback, so
queued size changes take effect before the client enters its exiting state.
On macOS, FIFO read readiness supplements kqueue with bounded select checks,
so writer closure removes control clients from format loops even without input.


Client/server transport uses RMUX version 1 frames: a 16-byte little-endian
header, a 32768-byte physical maximum, explicit string lengths and owned
SCM_RIGHTS descriptors. Legacy command and file size budgets remain separate
from framing overhead. There is no imsg compatibility backend or TMUX runtime
namespace alias. File transfers retain deferred completion and late write-error
acknowledgments; the CLI owns original standard descriptors so requested closes
close the originals, not extra duplicates.

Server startup deliberately re-executes the current binary through a hidden
internal selector instead of returning into Rust after `fork`. The lock-file
protocol, inherited environment and cwd, socketpair initial peer, foreground
`-D` behavior and listener-error handshake remain shared with ordinary startup.
The fresh server process calls `setsid`; prepared pane/job/pipe launches execute
only prebuilt child actions and never return to arbitrary Rust in the child.

Live format expansion reads client, command-item and mouse snapshots, retains
owners synchronously, and uses the same job registry for asynchronous output,
cycle timers, cancellation and hourly cleanup. Configuration errors and shell
command output enter the G18 view-mode driver: owned unlimited-history backing,
plain or VT-parsed appends, CRLF between appends and anchored viewport scrolling.
Copy/view mouse drags use resolved pane geometry and cancellable 50 ms edge
timers; mode commands retain their queue event, including raw mouse fields.

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
namespace-mapped, or adapted. The raw-imsg configuration-client-loss test runs
its equivalent RMUX identify-then-malformed-command fixture from
`harness/fixtures/`; the original causal log assertions remain unchanged.
Namespace mapping is applied
only with `--rmux` and printed per test. It preserves `TEST_TMUX`, terminfo names
and command behavior. Anchored `-V` program-name stripping maps `^tmux ` to
`^rmux `; the shared `#{version}` value remains `next-3.9`. Oracle
failures/timeouts from `--baseline` are reported as
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
`Step::WaitForPaneOutput` polls each server's capture for expected output with a
five-second limit; G21 cat fixtures use complete output markers before capture
or reset instead of relying on sleeps. Socket guards send `kill-server` even
when startup or a test panics; the regress runner also guards recorded sockets
on early errors. Teardown checks compare last-session exit, `kill-server`, and
panic cleanup against the oracle and verify the server and pane processes exit.
The unit tests prove oracle-vs-oracle parity for display, capture-pane and
list-panes, and deliberately change a session name to prove mismatch detection.
They use `oracle/bin/tmux` or `RMUX_ORACLE`; without an oracle they print a skip.

```sh
cargo test -p rmux-harness differential -- --nocapture
cargo run -p rmux-sys --example readiness
```

Measured readiness and pinned C grid layouts, reproduction commands and the
macOS EventLoop decision are in [docs/p0-probes.md](docs/p0-probes.md).

G19 chooser modes share the pinned tree engine: styled/aligned prefix columns,
boxed help and previews, live menu continuations, libc byte search, preserved
tags and scroll position, and top/bottom mode prompts. Window trees squash
session groups unless `-G`, preserve hidden-pane filter matches with `-h`, and
dispatch swaps and destructive prompt actions only after restoring mode state.
Customize filters apply at initialization; option reset walks the displayed
owner and its parents. Buffer and customize editors likewise spawn with their
mode state restored before modal zoom or resize.
`RMUX_BINARY=target/debug/rmux cargo test -p rmux-harness --test g19_modes_rest`
compares actual attached inner-client screens against the oracle, including
styles, rather than detached panes' backing screens. Clock captures reject
witnessed second rollovers; fixed wall-time oracle fixtures are not bundled.

ISC licensed; the tmux notice is retained in `LICENSE`.
