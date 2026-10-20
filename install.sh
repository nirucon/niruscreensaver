#!/bin/sh
# Niruscreensaver installer.
#
#   ./install.sh          install the released static binary
#   ./install.sh --build  build from this checkout (needs Rust)
#   ./install.sh --check  verify what is installed
#   ./install.sh --uninstall
#
# Installs to ~/.local/bin unless --prefix is given. Never touches other
# programs.
set -eu

VERSION="0.8.3"
REPO="${NIRUSA_REPO:-nirucon/niruscreensaver}"
BIN="niruscreensaver"
PREFIX="${NIRUSA_PREFIX:-$HOME/.local}"
BINDIR="$PREFIX/bin"
CONFDIR="${XDG_CONFIG_HOME:-$HOME/.config}/niruui"
STATEDIR="${XDG_STATE_HOME:-$HOME/.local/state}/niruscreensaver"
MANIFEST="$STATEDIR/manifest.tsv"

die() { printf 'niruscreensaver install: %s\n' "$*" >&2; exit 1; }
say() { printf '%s\n' "$*"; }

guard_nirux() {
  if [ -L "$1" ]; then
    case "$(readlink "$1")" in
      *share/nirux/current/bin/*) die "managed by NIRUX: $1 (use nirux update)" ;;
    esac
  fi
}

usage() {
  cat <<'EOF'
Usage: ./install.sh [options]
  (no options)  install the released binary
  --build       build from this checkout and install the result
  --check       verify the installation; make no changes
  --uninstall   remove the installed binary
  --prefix DIR  install under DIR (default ~/.local)
EOF
}

record() {
  mkdir -p "$STATEDIR"
  entry_path="$1"
  entry_hash="$2"
  manifest_tmp="$STATEDIR/.manifest.$$"
  if [ -f "$MANIFEST" ]; then
    awk -F '\t' -v p="$entry_path" '$1 != p' "$MANIFEST" > "$manifest_tmp" || true
  fi
  printf '%s\t%s\n' "$entry_path" "$entry_hash" >> "$manifest_tmp"
  mv -f "$manifest_tmp" "$MANIFEST"
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

put() {
  src="$1"; dest="$2"; mode="${3:-0755}"
  guard_nirux "$dest"
  mkdir -p "$(dirname "$dest")"
  install -m "$mode" "$src" "$dest.new"
  mv -f "$dest.new" "$dest"
  record "$dest" "$(sha256 "$dest")"
}

fetch() {
  url="$1"; out="$2"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --retry 3 -o "$out" "$url"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "$out" "$url"
  else
    die "curl or wget is required"
  fi
}

install_release() {
  tag="${NIRUSA_VERSION:-$VERSION}"
  base="${NIRUSA_BASE_URL:-https://github.com/$REPO/releases/download/v$tag}"
  dl="$(mktemp -d)"
  trap 'rm -rf "$dl"' EXIT HUP INT TERM
  say "Downloading $BIN $tag ..."
  fetch "$base/$BIN" "$dl/$BIN" || die "cannot download $base/$BIN"
  fetch "$base/SHA256SUMS" "$dl/SHA256SUMS" || die "cannot download checksums"
  expected="$(awk -v n="$BIN" '$2 == n {print $1}' "$dl/SHA256SUMS")"
  [ -n "$expected" ] || die "no checksum for $BIN"
  actual="$(sha256 "$dl/$BIN")"
  [ "$expected" = "$actual" ] || die "checksum mismatch for $BIN"
  chmod 0755 "$dl/$BIN"
  put "$dl/$BIN" "$BINDIR/$BIN"
  say "Installed $BIN $("$BINDIR/$BIN" --version | awk '{print $2}') in $BINDIR"
  say "Settings: $CONFDIR/screensaver.toml (created when you first edit it)"
}

build_local() {
  command -v cargo >/dev/null 2>&1 || die "cargo is required for --build"
  say "Building from source ..."
  ( cd "$(dirname "$0")" && cargo build --release --locked )
  put "target/release/$BIN" "$BINDIR/$BIN"
  say "Installed $BIN $("$BINDIR/$BIN" --version | awk '{print $2}') in $BINDIR"
}

check() {
  ok=0
  if [ -x "$BINDIR/$BIN" ]; then say "OK   $BINDIR/$BIN"; else say "FAIL missing $BINDIR/$BIN"; ok=1; fi
  "$BINDIR/$BIN" --version || ok=1
  return "$ok"
}

uninstall() {
  [ -f "$MANIFEST" ] || { say "Nothing to remove."; return 0; }
  while IFS="$(printf '\t')" read -r path hash; do
    [ -n "$path" ] || continue
    case "$path" in
      "$BINDIR/$BIN")
        guard_nirux "$path"
        if [ -f "$path" ]; then
          if [ "$(sha256 "$path")" = "$hash" ]; then rm -f "$path"; say "Removed $path"
          else say "Keeping modified file: $path"; fi
        fi ;;
      *) say "Skipping unexpected entry: $path" ;;
    esac
  done < "$MANIFEST"
  rm -f "$MANIFEST"
  say "Settings kept in $CONFDIR."
}

MODE=install
while [ $# -gt 0 ]; do
  case "$1" in
    --build) MODE=build ;;
    --check) MODE=check ;;
    --uninstall) MODE=uninstall ;;
    --prefix) shift; PREFIX="${1:?--prefix needs a directory}"; BINDIR="$PREFIX/bin" ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
  shift
done

cd "$(dirname "$0")"
case "$MODE" in install|build|uninstall) guard_nirux "$BINDIR/$BIN" ;; esac
case "$MODE" in
  install) install_release ;;
  build) build_local ;;
  check) check; exit $? ;;
  uninstall) uninstall ;;
esac
say "Done."
