#!/bin/sh
# Bump the workspace version, push it, wait for CI, tag, and wait for the
# release workflow. If the Cargo.toml version has no tag yet, it releases HEAD
# as that version without a bump, so running it again after a failure resumes.
# Usage: scripts/release.sh [patch|minor|major|X.Y.Z]   (default: patch)
set -eu

die() {
	echo "release: $*" >&2
	exit 1
}
say() { echo "== $*"; }

workspace_version() {
	sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml | head -1
}

is_version() {
	echo "$1" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'
}

newer() {
	echo "$1 $2" | awk '{
		split($1, a, "."); split($2, b, ".")
		for (i = 1; i <= 3; i++) if (a[i] != b[i]) exit !(a[i] + 0 > b[i] + 0)
		exit 1
	}'
}

next_version() {
	case $1 in
	patch | minor | major)
		echo "$2" | awk -F. -v part="$1" '{
			if (part == "major") print $1 + 1 ".0.0"
			else if (part == "minor") print $1 "." $2 + 1 ".0"
			else print $1 "." $2 "." $3 + 1
		}'
		;;
	*)
		is_version "$1" || die "want patch, minor, major or X.Y.Z, got '$1'"
		newer "$1" "$2" || die "$1 is not newer than $2"
		echo "$1"
		;;
	esac
}

set_version() {
	awk -v v="$1" '
		/^\[/ { section = $0 }
		section == "[workspace.package]" && /^version = "/ && !done { print "version = \"" v "\""; done = 1; next }
		{ print }
	' Cargo.toml >Cargo.toml.tmp
	mv Cargo.toml.tmp Cargo.toml
	cargo update -q --workspace
	[ "$(workspace_version)" = "$1" ] || die "could not set the version in Cargo.toml"
	changed=$(git diff --name-only | tr '\n' ' ')
	[ "$changed" = "Cargo.lock Cargo.toml " ] || die "the bump changed '$changed', want Cargo.lock and Cargo.toml"
}

find_run() {
	tries=0
	while :; do
		id=$(gh run list --workflow "$1" --commit "$2" --event push --limit 1 --json databaseId -q '.[0].databaseId // empty')
		[ -n "$id" ] && break
		tries=$((tries + 1))
		[ "$tries" -lt 30 ] || die "no $1 run for $2 after 5 min"
		sleep 10
	done
	echo "$id"
}

watch_run() {
	gh run watch "$1" --exit-status --interval 20 >/dev/null 2>&1
}

show_failures() {
	gh run view "$1" --json jobs -q '.jobs[] | select(.conclusion == "failure") | "  failed: \(.name)"'
	gh run view "$1" --log-failed 2>/dev/null | grep -E 'FAIL \[|panicked at|error(\[|:)' |
		cut -f3- | sed 's/^[^ ]* *//' | cut -c1-200 | sort -u | head -20 || true
}

wait_ci() {
	run=$(find_run CI "$1")
	say "CI: $(gh run view "$run" --json url -q .url)"
	watch_run "$run" && return
	show_failures "$run"
	say "re-running the failed CI jobs once"
	gh run rerun "$run" --failed
	tries=0
	while [ "$(gh run view "$run" --json status -q .status)" = completed ]; do
		tries=$((tries + 1))
		[ "$tries" -lt 30 ] || die "the CI re-run did not start"
		sleep 2
	done
	watch_run "$run" && return
	show_failures "$run"
	die "CI failed twice; fix it, then run this again to re-check this commit"
}

cd "$(git rev-parse --show-toplevel)"
command -v gh >/dev/null || die "needs gh"
command -v cargo >/dev/null || die "needs cargo"
[ "$(git symbolic-ref -q --short HEAD)" = main ] || die "check out main first"
if ! git diff --quiet || ! git diff --cached --quiet; then
	die "commit or stash your tracked changes first"
fi
git fetch -q --tags origin
git pull -q --ff-only origin main

current=$(workspace_version)
is_version "$current" || die "Cargo.toml version '$current' is not X.Y.Z"
head=$(git rev-parse HEAD)
tagged=$(git rev-parse -q --verify "refs/tags/v$current^{commit}" || true)

if [ -z "$tagged" ]; then
	version=$current
	say "v$version is not tagged yet; releasing HEAD as v$version"
	[ $# -eq 0 ] || say "ignoring '$1': the untagged v$version goes first"
elif [ "$tagged" = "$head" ]; then
	version=$current
	say "HEAD is already tagged v$version; checking its release"
else
	version=$(next_version "${1:-patch}" "$current")
	! git rev-parse -q --verify "refs/tags/v$version" >/dev/null || die "tag v$version already exists"
	say "bumping $current -> $version"
	set_version "$version"
	git commit -q -m "chore: release v$version" Cargo.toml Cargo.lock
	head=$(git rev-parse HEAD)
fi
git push -q origin main

if ! git rev-parse -q --verify "refs/tags/v$version" >/dev/null; then
	wait_ci "$head"
	say "tagging v$version"
	git tag "v$version" "$head"
fi
git push -q origin "v$version"

run=$(find_run release.yml "$head")
say "release: $(gh run view "$run" --json url -q .url)"
watch_run "$run" || {
	show_failures "$run"
	die "the release workflow failed"
}

gh release view "v$version" --json url,assets -q '.url, (.assets[] | "  \(.name)")'
say "released v$version; on each host: rmux update && rmux update-plugin"
