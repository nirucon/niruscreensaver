#!/bin/sh
# Repeated exposures must not reload fonts; input still dismisses immediately.
set -eu
BIN=${1:?path to niruscreensaver}
MODE=${2:-check}
TEST=$(mktemp -d)
SAVER=; XVFB=; XEV=
cleanup() {
  if [ -n "$SAVER" ]; then kill "$SAVER" 2>/dev/null || :; wait "$SAVER" 2>/dev/null || :; fi
  if [ -n "$XEV" ]; then kill "$XEV" 2>/dev/null || :; fi
  if [ -n "$XVFB" ]; then kill "$XVFB" 2>/dev/null || :; wait "$XVFB" 2>/dev/null || :; fi
  rm -rf "$TEST"
}
trap cleanup EXIT INT TERM
wait_gone() {
  i=0
  while kill -0 "$SAVER" 2>/dev/null; do
    state=$(ps -o stat= -p "$SAVER" 2>/dev/null || true)
    case "$state" in ''|Z*) break ;; esac
    i=$((i+1)); [ "$i" -lt 40 ] || { echo "FAIL saver did not exit"; exit 1; }; sleep 0.05
  done
  wait "$SAVER"
  SAVER=
}
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
# Start-up looks up the text and the rune font once each; exposures add none.
startup=$(wc -l < "$FONT_LOG")
for _ in 1 2 3 4 5; do xrefresh; sleep 0.2; done
sleep 1
kill -0 "$SAVER"
probes=$(($(wc -l < "$FONT_LOG") - startup))
rss=$(awk '/VmRSS:/{print $2}' "/proc/$SAVER/status")
printf 'five exposures: startup_font_lookups=%s font_discoveries=%s RSS_KiB=%s\n' "$startup" "$probes" "$rss"
if [ "$MODE" != baseline ]; then [ "$probes" -eq 0 ] && [ "$startup" -le 2 ]; fi
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
# A compositor must not dim the saver: full opacity and a stable class (S2).
wid=$(xwininfo -root -tree | awk '/"NIRUSCREENSAVER"/{print $1; exit}')
xprop -id "$wid" _NET_WM_WINDOW_OPACITY WM_CLASS > "$TEST/props"
grep -qx '_NET_WM_WINDOW_OPACITY(CARDINAL) = 4294967295' "$TEST/props" || { cat "$TEST/props"; echo "FAIL opacity hint"; exit 1; }
grep -qx 'WM_CLASS(STRING) = "NIRUSCREENSAVER", "NIRUSCREENSAVER"' "$TEST/props" || { cat "$TEST/props"; echo "FAIL WM_CLASS"; exit 1; }
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

# The dismissing key or click never reaches the focused application, neither
# its press nor its release (S3). xev stands in for a terminal under the pointer.
xev -event keyboard -event button -geometry 400x300+0+0 > "$TEST/xev.log" 2>&1 & XEV=$!
sleep 1
xdotool mousemove 100 100
xdotool windowfocus --sync "$(xdotool search --name '^Event Tester$' | head -1)"
for input in key button; do
  "$BIN" > "$TEST/saver-$input.log" 2>&1 & SAVER=$!
  sleep 1
  if [ "$input" = key ]; then xdotool keydown a; else xdotool mousedown 1; fi
  sleep 0.3
  kill -0 "$SAVER" || { echo "FAIL saver ungrabbed with the $input still down"; exit 1; }
  if [ "$input" = key ]; then xdotool keyup a; else xdotool mouseup 1; fi
  wait_gone
done
xdotool key b
sleep 0.5
kill "$XEV" 2>/dev/null || :; wait "$XEV" 2>/dev/null || :
grep -q 'keysym 0x62, b' "$TEST/xev.log" || { echo "FAIL xev saw no control key"; exit 1; }
if grep -q 'keysym 0x61, a' "$TEST/xev.log" || grep -q '^Button' "$TEST/xev.log"; then
  grep -E '^(Key|Button)' -A2 "$TEST/xev.log"; echo "FAIL dismissing input leaked to the focused window"; exit 1
fi

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
printf 'PASS: input, mouse, resize coverage, opacity hint, swallowed dismissal input and failed-grab handling\n'
