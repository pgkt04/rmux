#!/bin/sh
# rmux Linux test gate: fmt, clippy, builds, nextest, and the tmux regress
# suite on the pinned oracle and on release rmux. Results go to
# /tmp/zb-test/gate (summary.txt, one log per step, oracle-linux.json,
# rmux-linux.json). It clones rmux and tmux under /tmp/zb-test if missing.
# Usage: linux-gate.sh <commit>
# Long runs: start it detached, for example
#   systemd-run --user --unit=rmux-linux-gate --collect \
#     -p CPUAffinity=16-27 -p Nice=19 -p IOSchedulingClass=idle \
#     ./scripts/linux-gate.sh <commit>
set -u
COMMIT=$1
G=/tmp/zb-test/gate
mkdir -p "$G"
: >"$G/summary.txt"
exec >"$G/gate.log" 2>&1
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR=/tmp/zb-test/target TMUX_SRC=/tmp/zb-test/tmux CARGO_BUILD_JOBS=4

step() {
	name=$1
	shift
	start=$(date +%s)
	echo "== $name: $*"
	"$@" >"$G/$name.log" 2>&1
	rc=$?
	echo "$name rc=$rc secs=$(($(date +%s) - start))" >>"$G/summary.txt"
	return $rc
}

cd /tmp/zb-test || exit 1
[ -d rmux ] || git clone -q git@github.com:pgkt04/rmux.git rmux
git -C rmux fetch -q origin && git -C rmux checkout -q "$COMMIT" || { echo "checkout rc=1" >>"$G/summary.txt"; exit 1; }
[ -d tmux ] || git clone -q https://github.com/tmux/tmux.git tmux
git -C tmux checkout -q 8f25579c || { echo "tmux-checkout rc=1" >>"$G/summary.txt"; exit 1; }
command -v cargo-nextest >/dev/null || curl -LsSf https://get.nexte.st/latest/linux | tar zxf - -C "$HOME/.cargo/bin"
{
	echo "rustc: $(rustc -V)"
	echo "nextest: $(cargo nextest --version 2>&1 | head -1)"
	echo "commit: $(git -C rmux rev-parse --short HEAD)"
} >>"$G/summary.txt"

cd rmux
step oracle sh scripts/build-oracle.sh
echo "oracle: $(oracle/bin/tmux -V 2>&1)" >>"$G/summary.txt"
step fmt cargo fmt --check
step clippy cargo clippy --workspace --all-targets -- -D warnings
step clippy_features cargo clippy --workspace --all-targets --features rmux/sixel,rmux/systemd -- -D warnings
step build cargo build -p rmux -p rmux-harness
# The regress run compares against the optimized C oracle. A debug rmux takes
# about twice as long per command on Linux, which loses timing races that
# tmux wins (format-modifiers.sh P/z vs the login shell's OSC 0 title).
step build_release cargo build --release -p rmux
step nextest cargo nextest run --workspace --no-fail-fast
step regress_oracle "$CARGO_TARGET_DIR/debug/regress" --source /tmp/zb-test/tmux --binary oracle/bin/tmux --timeout 150 --json "$G/oracle-linux.json"
step regress_rmux "$CARGO_TARGET_DIR/debug/regress" --source /tmp/zb-test/tmux --binary "$CARGO_TARGET_DIR/release/rmux" --rmux --baseline "$G/oracle-linux.json" --timeout 150 --json "$G/rmux-linux.json"

# Leftover test servers from this run only.
ps -axo pid=,command= | grep -E '/tmp/zb-test/(target/(debug|release)/rmux|rmux/oracle/bin/tmux)' | grep -v grep | awk '{print $1}' | xargs -r kill 2>/dev/null
echo DONE >>"$G/summary.txt"
