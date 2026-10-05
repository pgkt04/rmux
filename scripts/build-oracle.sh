#!/bin/sh
# Build the pinned C tmux used as the test oracle.
# Output: oracle/bin/tmux (gitignored). Source comes from git archive, so
# the tmux checkout itself is never touched.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
TMUX_SRC=${TMUX_SRC:-"$HOME/fun/tmux"}
PIN=$(cat "$ROOT/oracle/PIN")
OUT="$ROOT/oracle"
BUILD="$OUT/build-$PIN"

if [ -x "$OUT/bin/tmux" ] && [ "$(cat "$OUT/bin/.pin" 2>/dev/null)" = "$PIN" ]; then
	echo "oracle up to date: $OUT/bin/tmux ($PIN)"
	exit 0
fi

rm -rf "$BUILD"
mkdir -p "$BUILD" "$OUT/bin"
git -C "$TMUX_SRC" archive "$PIN" | tar -x -C "$BUILD"

cd "$BUILD"
sh autogen.sh >/dev/null 2>&1
case "$(uname -s)" in
Darwin)
	PREFIX=$(brew --prefix)
	export PKG_CONFIG_PATH="$PREFIX/opt/libevent/lib/pkgconfig:$PREFIX/opt/ncurses/lib/pkgconfig:$PREFIX/opt/utf8proc/lib/pkgconfig:${PKG_CONFIG_PATH:-}"
	;;
esac
# Default options only, so the oracle matches a stock tmux build. On macOS
# configure.ac requires utf8proc widths and a jemalloc choice; the allocator
# does not change behavior, so jemalloc stays off. Sixel is off by default.
case "$(uname -s)" in
Darwin) ./configure --disable-jemalloc >/dev/null ;;
*) ./configure >/dev/null ;;
esac
make -j"$(getconf _NPROCESSORS_ONLN)" >/dev/null

cp tmux "$OUT/bin/tmux"
echo "$PIN" >"$OUT/bin/.pin"
rm -rf "$BUILD"
"$OUT/bin/tmux" -V
