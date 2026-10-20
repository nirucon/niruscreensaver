#!/bin/sh
# Repeated exposures must not reload fonts; input still dismisses immediately.
set -eu
BIN=${1:?path to niruscreensaver}
MODE=${2:-check}
TEST=$(mktemp -d)
SAVER=; XVFB=
cleanup() {
  if [ -n "$SAVER" ]; then kill "$SAVER" 2>/dev/null || :; wait "$SAVER" 2>/dev/null || :; fi
  if [ -n "$XVFB" ]; then kill "$XVFB" 2>/dev/null || :; wait "$XVFB" 2>/dev/null || :; fi
  rm -rf "$TEST"
}
trap cleanup EXIT INT TERM
mkdir -p "$TEST/bin"
TEST_FONT=$(fc-match -f '%{file}' monospace)
export TEST_FONT
export FONT_LOG=$TEST/font-probes DISPLAY=:95 XDG_CONFIG_HOME=$TEST/config
cat > "$TEST/bin/fc-match" <<'HELPER'
#!/bin/sh
printf 'font\n' >> "$FONT_LOG"
printf '%s' "$TEST_FONT"
HELPER
chmod +x "$TEST/bin/fc-match"
export PATH=$TEST/bin:$PATH
Xvfb :95 -screen 0 1024x768x24 -nolisten tcp > "$TEST/xvfb.log" 2>&1 & XVFB=$!
sleep 1
"$BIN" > "$TEST/saver.log" 2>&1 & SAVER=$!
sleep 1
for _ in 1 2 3 4 5; do xrefresh; sleep 0.2; done
sleep 1
kill -0 "$SAVER"
probes=$(wc -l < "$FONT_LOG")
rss=$(awk '/VmRSS:/{print $2}' "/proc/$SAVER/status")
printf 'five exposures: font_discoveries=%s RSS_KiB=%s\n' "$probes" "$rss"
if [ "$MODE" != baseline ]; then [ "$probes" -eq 1 ]; fi
xdotool key space
i=0
while kill -0 "$SAVER" 2>/dev/null; do
  state=$(ps -o stat= -p "$SAVER" 2>/dev/null || true)
  case "$state" in ''|Z*) break ;; esac
  i=$((i+1)); [ "$i" -lt 40 ]; sleep 0.05
done
wait "$SAVER"
SAVER=

# A mouse click is an equally explicit dismissal.
"$BIN" > "$TEST/saver-click.log" 2>&1 & SAVER=$!
sleep 1
xdotool click 1
i=0
while kill -0 "$SAVER" 2>/dev/null; do
  state=$(ps -o stat= -p "$SAVER" 2>/dev/null || true)
  case "$state" in ''|Z*) break ;; esac
  i=$((i+1)); [ "$i" -lt 40 ]; sleep 0.05
done
wait "$SAVER"
SAVER=

# A root resize must resize the covering window and its backing pixmap.
geometry() {
  wid=$(xwininfo -root -tree | awk '/"NIRUSCREENSAVER"/{print $1; exit}')
  [ -n "$wid" ] || return 1
  xwininfo -id "$wid" | awk '/Width:/{w=$2} /Height:/{h=$2} END{print w "x" h}'
}
"$BIN" > "$TEST/saver-resize.log" 2>&1 & SAVER=$!
sleep 1
[ "$(geometry)" = 1024x768 ] || { echo "FAIL initial geometry: $(geometry)"; exit 1; }
xrandr --fb 800x600 >/dev/null 2>&1 || true
sleep 1
[ "$(geometry)" = 800x600 ] || { echo "FAIL shrink geometry: $(geometry)"; exit 1; }
xrandr --fb 1024x768 >/dev/null 2>&1 || true
sleep 1
[ "$(geometry)" = 1024x768 ] || { echo "FAIL restore geometry: $(geometry)"; exit 1; }
xdotool key space
i=0
while kill -0 "$SAVER" 2>/dev/null; do
  state=$(ps -o stat= -p "$SAVER" 2>/dev/null || true)
  case "$state" in ''|Z*) break ;; esac
  i=$((i+1)); [ "$i" -lt 40 ]; sleep 0.05
done
wait "$SAVER"
SAVER=

# A second saver cannot claim success when the first already owns the grabs.
"$BIN" > "$TEST/saver-first.log" 2>&1 & SAVER=$!
sleep 1
if "$BIN" > "$TEST/saver-second.log" 2>&1; then
  echo "FAIL second saver claimed success while input was grabbed" >&2
  exit 1
fi
grep -q 'cannot grab' "$TEST/saver-second.log" || { cat "$TEST/saver-second.log"; echo "FAIL failed grab was not diagnosed" >&2; exit 1; }
kill "$SAVER" 2>/dev/null || true
wait "$SAVER" 2>/dev/null || true
SAVER=
printf 'PASS: input, mouse, resize coverage and failed-grab handling\n'
