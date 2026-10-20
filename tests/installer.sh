#!/bin/sh
# Offline ownership regression; never writes the real home.
set -eu
SOURCE=$(CDPATH='' cd -- "$(dirname "$0")/.." && pwd)
TEST=$(mktemp -d)
trap 'rm -rf "$TEST"' EXIT
# Keep the real Rust toolchain available while the installer test isolates HOME.
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
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

# The released installer uses the selected release's binary and never replaces
# a modified installed file.
[ -x "$SOURCE/target/release/niruscreensaver" ] || (cd "$SOURCE" && cargo build --release --locked)
release=$TEST/release
mkdir -p "$release"
cp "$SOURCE/target/release/niruscreensaver" "$release/niruscreensaver"
(cd "$release" && sha256sum niruscreensaver > SHA256SUMS)
NIRUSA_BASE_URL="file://$release" sh "$SOURCE/install.sh" --prefix "$TEST/prebuilt"
"$TEST/prebuilt/bin/niruscreensaver" --version >/dev/null
printf 'user replacement\n' > "$TEST/prebuilt/bin/niruscreensaver"
sh "$SOURCE/install.sh" --uninstall --prefix "$TEST/prebuilt"
[ "$(cat "$TEST/prebuilt/bin/niruscreensaver")" = 'user replacement' ] || { printf 'FAIL: modified release binary was removed\n' >&2; exit 1; }

# A corrupt release fails before replacing the installed binary.
NIRUSA_BASE_URL="file://$release" sh "$SOURCE/install.sh" --prefix "$TEST/prebuilt2"
installed_hash=$(sha256sum "$TEST/prebuilt2/bin/niruscreensaver" | cut -d' ' -f1)
cp "$release/niruscreensaver" "$release/niruscreensaver.good"
printf 'corrupt\n' >> "$release/niruscreensaver"
if NIRUSA_BASE_URL="file://$release" sh "$SOURCE/install.sh" --prefix "$TEST/prebuilt2" > "$TEST/corrupt.log" 2>&1; then
  printf 'FAIL: corrupt release install succeeded\n' >&2; exit 1
fi
grep -q 'checksum mismatch' "$TEST/corrupt.log"
[ "$(sha256sum "$TEST/prebuilt2/bin/niruscreensaver" | cut -d' ' -f1)" = "$installed_hash" ] || { printf 'FAIL: corrupt release changed installed binary\n' >&2; exit 1; }
mv "$release/niruscreensaver.good" "$release/niruscreensaver"

# A source build installs the checkout binary and records it.
if command -v cargo >/dev/null 2>&1; then
  sh "$SOURCE/install.sh" --build --prefix "$TEST/source"
  "$TEST/source/bin/niruscreensaver" --version >/dev/null
fi
printf 'PASS: NIRUX ownership, pinned release install, failed assets and modified-file preservation\n'
