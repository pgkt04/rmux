# rmux-util

Safe (`#![forbid(unsafe_code)]`) ports of the tmux `8f25579c` foundation code:
UTF-8 handling, logging, `vis`, `strtonum`, base64, timers, and the small
containers that replace `evbuffer` and `bitstring.h`. Every libc call goes
through `rmux-sys`.

## Single-thread contract

tmux keeps one process-wide intern table for UTF-8 characters longer than
three bytes (`utf8.c:238,251`) and one width cache (`utf8.c:50-51`). rmux
keeps both in `thread_local!` storage (`Utf8Table`, `WidthCache`) owned by
the event-loop thread. A `Utf8Char` whose payload is a table index is only
meaningful on the thread that created it: never send `Utf8Char` values, or
anything that embeds them (grid cells), to another thread. The table is never
cleared while handles exist, matching the C table that never frees entries.

## Log file policy

tmux writes `tmux-<name>-<pid>.log`. rmux writes `rmux-<name>-<pid>.log` in
the current directory, in the rmux namespace like the socket directory and
the environment variables. Format, prefix, and message text are unchanged:
`<sec>.<usec> <prefix><message>` with the message passed through raw
`stravis(VIS_OCTAL|VIS_CSTYLE|VIS_TAB|VIS_NL)`. The regress harness maps the
globs `tmux-server-*.log`, `tmux-client-*.log`, and `tmux-out-*.log` to the
`rmux-*` names for the tests that grep the server log (`cfg-client-free.sh`,
`cfg-load-once.sh`, `cfg-client-lost-before-wait.sh`).

`fatal!` and `fatalx!` write `fatal: <strerror(errno)>: <msg>` and
`fatal: <msg>` to the log and exit with status 1; nothing goes to stderr.
`fatal!` captures errno before evaluating formatting arguments. Descriptor-wide
process operations (`rmux-sys::proc::{closefrom,daemon}` and `pty::login_tty`)
are unsafe sys-only boundaries: G12/G14 launch code must establish single-thread
and exclusive fd ownership before calling them.

## Pinned crash-path deviation

Pinned `utf8_sanitize` aborts on a leading zero-width sequence because its first
reallocation requests zero bytes. rmux skips the sequence and continues with the
remaining text (`sanitize("\u{200b}suffix")` returns `suffix`), avoiding that
allocation bug; the C-reference test exercises the abort separately.


## xmalloc porting rule

`xmalloc.c` is not ported. Rust allocation failure aborts, and the zero-size
`fatalx` checks guard C bugs that `Vec` and `String` cannot have. When porting:

| C | Rust |
|---|---|
| `xasprintf` / `xvasprintf` | `format!` |
| `xstrdup` / `xstrndup` / `xmemdup` | `.to_owned()` / `.to_vec()` |
| `xreallocarray` / `xrecallocarray` growth loop | `Vec::push` |
| `xsnprintf` into a fixed buffer | `String`, unless the buffer size is observable (for example a terminfo string limit) |
