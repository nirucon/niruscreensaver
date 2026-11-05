#!/bin/sh
# The verbatim NIRULOCK twins (CAVA, palette, audio detector, panel and
# quotes) must match the pinned NIRULOCK tag. Only the
# "// Twin of" header lines, which exist only here, are ignored.
set -eu
SOURCE=$(CDPATH='' cd -- "$(dirname "$0")/.." && pwd)
TAG=${NIRULOCK_TAG:-$(cat "$SOURCE/tests/nirulock-tag")}
TEST=$(mktemp -d)
trap 'rm -rf "$TEST"' EXIT
status=0
for file in cava.rs palette.rs audio.rs visual.rs quotes.rs quotes.txt; do
  curl -fsSL --retry 3 "https://raw.githubusercontent.com/nirucon/nirulock/$TAG/src/$file" > "$TEST/upstream"
  # The two header lines are the only difference allowed.
  sed '1,2{/^\/\/ /d;}' "$SOURCE/src/$file" > "$TEST/local"
  if ! diff -u "$TEST/upstream" "$TEST/local" > "$TEST/diff"; then
    cat "$TEST/diff"
    printf 'FAIL: src/%s drifted from nirulock %s; change both repositories\n' "$file" "$TAG" >&2
    status=1
  fi
done
[ "$status" -eq 0 ] && printf 'PASS: the shared files match nirulock %s\n' "$TAG"
exit "$status"
