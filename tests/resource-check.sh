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
Xvfb :95 -screen 0 800x600x24 -nolisten tcp > "$TEST/xvfb.log" 2>&1 & XVFB=$!
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
printf 'PASS: input dismissed the screensaver\n'
