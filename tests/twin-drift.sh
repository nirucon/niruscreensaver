#!/bin/sh
# The verbatim NIRULOCK twins (CAVA, palette, audio detector, panel, quotes
# and the built-in quote collections) must match the pinned NIRULOCK tag.
# Only the "// Twin of" header lines, which exist only here, are ignored.
set -eu
SOURCE=$(CDPATH='' cd -- "$(dirname "$0")/.." && pwd)
TAG=${NIRULOCK_TAG:-$(cat "$SOURCE/tests/nirulock-tag")}
TEST=$(mktemp -d)
trap 'rm -rf "$TEST"' EXIT
status=0
COLLECTIONS="havamal-en havamal-sv stoic-en stoic-sv"
files="src/cava.rs src/palette.rs src/audio.rs src/visual.rs src/quotes.rs"
for name in $COLLECTIONS; do files="$files quotes/$name.txt"; done
files="$files quotes/SOURCES.md"
for file in $files; do
  curl -fsSL --retry 3 "https://raw.githubusercontent.com/nirucon/nirulock/$TAG/$file" > "$TEST/upstream"
  # The two header lines are the only difference allowed.
  sed '1,2{/^\/\/ /d;}' "$SOURCE/$file" > "$TEST/local"
  if ! diff -u "$TEST/upstream" "$TEST/local" > "$TEST/diff"; then
    cat "$TEST/diff"
    printf 'FAIL: %s drifted from nirulock %s; change both repositories\n' "$file" "$TAG" >&2
    status=1
  fi
done
# Every built-in collection here is checked above.
for path in "$SOURCE"/quotes/*.txt; do
  case " $COLLECTIONS " in
    *" $(basename "$path" .txt) "*) ;;
    *) printf 'FAIL: %s is not in the twin list\n' "$path" >&2; status=1 ;;
  esac
done
[ "$status" -eq 0 ] && printf 'PASS: the shared files match nirulock %s\n' "$TAG"
exit "$status"
