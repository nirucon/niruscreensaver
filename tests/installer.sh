#!/bin/sh
# Offline ownership regression; never writes the real home.
set -eu
SOURCE=$(CDPATH='' cd -- "$(dirname "$0")/.." && pwd)
TEST=$(mktemp -d)
trap 'rm -rf "$TEST"' EXIT
export HOME=$TEST/home XDG_STATE_HOME=$TEST/state
prefix=$TEST/prefix
target=$prefix/share/nirux/current/bin/niruscreensaver
mkdir -p "$prefix/bin" "$(dirname "$target")" "$XDG_STATE_HOME/niruscreensaver"
printf 'suite binary\n' > "$target"
ln -s "$target" "$prefix/bin/niruscreensaver"
for mode in --build --uninstall; do
  if sh "$SOURCE/install.sh" "$mode" --prefix "$prefix" > "$TEST/log" 2>&1; then
    printf 'FAIL: accepted NIRUX ownership (%s)\n' "$mode" >&2; exit 1
  fi
  grep -q 'managed by NIRUX' "$TEST/log"
  [ -L "$prefix/bin/niruscreensaver" ]
  [ "$(cat "$target")" = 'suite binary' ]
done
rm "$prefix/bin/niruscreensaver"
printf 'standalone binary\n' > "$prefix/bin/niruscreensaver"
hash=$(sha256sum "$prefix/bin/niruscreensaver" | cut -d ' ' -f1)
printf '%s\t%s\n' "$prefix/bin/niruscreensaver" "$hash" > "$XDG_STATE_HOME/niruscreensaver/manifest.tsv"
printf 'user replacement\n' > "$prefix/bin/niruscreensaver"
sh "$SOURCE/install.sh" --uninstall --prefix "$prefix"
[ "$(cat "$prefix/bin/niruscreensaver")" = 'user replacement' ]
printf '%s\t%s\n' "$prefix/bin/niruscreensaver" "$(sha256sum "$prefix/bin/niruscreensaver" | cut -d ' ' -f1)" > "$XDG_STATE_HOME/niruscreensaver/manifest.tsv"
sh "$SOURCE/install.sh" --uninstall --prefix "$prefix"
[ ! -e "$prefix/bin/niruscreensaver" ]
printf 'PASS: NIRUX ownership and modified-file preservation\n'
