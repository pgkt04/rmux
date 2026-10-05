# rmux-emu cell and style foundations

P1 G02 ports tmux `8f25579c` `colour.c`, `attributes.c`, `style.c`, and
`hyperlinks.c`. `GridCell` is a fixed, copyable expanded cell; `DEFAULT_CELL`
is the canonical space with colours 8 and link 0. Existing G00 numeric enum
and flag values remain unchanged.

Colour and attribute entry points accept bytes and stop at NUL. Colour tables
include all pinned X11 entries and both complete indexed conversion tables.
The safe X11 scanner preserves scanf field widths, conversion precedence,
trailing-input acceptance, space-only name trimming, and floating-point rules.
`ColourPalette` owns independent lazy runtime/default arrays: clearing runtime
preserves defaults; clearing storage preserves foreground/background.

`Style::from_cell` copies the entire cell. `option_fallback` deliberately keeps
the pinned static fallback's underscore colour 0 instead of the grid default's
8. General parsing restores the whole style on failure, but not already
performed registry insertions. Single-colour parsing resets to its base even
on failure. Overlay changes only nondefault colour channels and ORs attributes.
Serialization preserves pinned ordering, the missing Control-range branch,
and the 2048-byte output bound. `StyleRanges` retains insertion order and
returns the first half-open match.

A server or standalone emulator owns one `HyperlinkRegistry`. Screen/mode
owners explicitly `create`, `share`, `reset`, and consume leases with `release`;
`Hyperlinks` is neither Copy nor Clone. Leases reject another registry and
stale generations. Records expose borrowed URI, internal-ID and external-ID
bytes. Named pairs deduplicate; anonymous insertions do not. A process-wide
FIFO retains 4999 records and reuse does not refresh it. Reset preserves
counters and affects shared leases. External IDs deliberately use `tmux`.
Style links use a lazy registry-owned store. `copy_style_link_to_store` shares
an immutable input buffer for its call, then performs the normal second escape
and URI-length check. A missing/evicted link is ordinary absence.

The source's SGR capability predicates cannot downgrade colours at this pin;
`write_colour_escape` therefore has no misleading capability parameter. Actual
tty reduction belongs to G07. Option/format resolution, scrollbar composition,
and real grid/input/tty consumer glue belong to later G09/G10/G17/G03/G05/G07
work, not speculative P1 adapters. The registry, overlay, palette replacement,
and style transfer APIs have executable behavior today.

Tests `cells_cref` compile the unmodified pinned C files with a test driver in
a temporary archive, never in the source checkout. They compare all names,
indexed tables, every attribute bit combination, colour arithmetic and SGR
capability cases, style state/output/registry traces, palette operations,
shared leases, escaping, cross-store eviction and style transfer. `cells`
adds lifecycle/invariant tests and deterministic arbitrary-byte randomized
parser/registry tests. No cargo-fuzz dependency or global tooling is required.
Set `TMUX_SRC` to a checkout containing the pin; missing source/compiler prints
a clear skip message. A present compiler/reference that fails is a test failure.

Run from the workspace root:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Platform evidence for this change is macOS. Linux scanner/width parity must
also run in CI; the C-reference harness uses libc widths there rather than
macOS's utf8proc.
