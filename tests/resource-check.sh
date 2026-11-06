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

# Pointer motion dismisses only with dismiss_on_motion, beyond 24 px (S4).
# Moves are relative to where the pointer is: Xvfb may refuse to warp it.
pointer_to() {
  eval "$(xdotool getmouselocation --shell)"
  xdotool mousemove $((X + $1)) $((Y + $2))
}
"$BIN" > "$TEST/saver-motion.log" 2>&1 & SAVER=$!
sleep 1
pointer_to 200 150; sleep 0.5
kill -0 "$SAVER" || { echo "FAIL motion dismissed by default"; exit 1; }
xdotool key space; wait_gone
mkdir -p "$XDG_CONFIG_HOME/niruui"
printf 'dismiss_on_motion = true\n' > "$XDG_CONFIG_HOME/niruui/screensaver.toml"
"$BIN" > "$TEST/saver-motion.log" 2>&1 & SAVER=$!
sleep 1
pointer_to 10 10; sleep 0.5
kill -0 "$SAVER" || { echo "FAIL a 14 px move dismissed"; exit 1; }
pointer_to 40 0; sleep 0.2
wait_gone
rm "$XDG_CONFIG_HOME/niruui/screensaver.toml"

# CAVA pauses for silence and resumes on sound (S1); the media line follows
# only while the spectrum is live (U1). The fixture CAVA plays 3 s of bars,
# 2 s of zeros, then writes nothing, as CAVA's sleep_timer does, until told
# to resume. Wakeups are counted over every thread of the saver.
i=0; : > "$TEST/loud.bin"; : > "$TEST/zero.bin"
while [ "$i" -lt 48 ]; do printf '\320\007' >> "$TEST/loud.bin"; printf '\000\000' >> "$TEST/zero.bin"; i=$((i+1)); done
mkfifo "$TEST/never"
export FIXTURE=$TEST
cat > "$TEST/bin/cava" <<'HELPER'
#!/bin/sh
frames() { i=0; while [ "$i" -lt "$2" ]; do cat "$FIXTURE/$1.bin"; sleep 0.033; i=$((i+1)); done; }
frames loud 90
frames zero 60
while [ ! -e "$FIXTURE/resume" ]; do sleep 0.2; done
frames loud 100000
HELPER
cat > "$TEST/bin/playerctl" <<'HELPER'
#!/bin/sh
printf 'Playing\tFixture Artist\tFixture Title\n'
read -r _ < "$FIXTURE/never"
HELPER
chmod +x "$TEST/bin/cava" "$TEST/bin/playerctl"
wakeups() { cat /proc/"$SAVER"/task/*/status | awk '/ctxt_switches/{s+=$2} END{print s+0}'; }
ticks() { awk '{print $14+$15}' /proc/"$SAVER"/stat; }
children() { pgrep -c -P "$SAVER" -x "$1" || :; }
# The spectrum alone, as before 0.11: CAVA runs whenever the display is on.
mkdir -p "$XDG_CONFIG_HOME/niruui"
printf 'visual = "cava"\n' > "$XDG_CONFIG_HOME/niruui/screensaver.toml"
TMPDIR=$TEST NIRUSCREENSAVER_TRACE=1 "$BIN" > "$TEST/saver-cava.log" 2>&1 & SAVER=$!
sleep 1.5
[ "$(children cava)" -eq 1 ] || { echo "FAIL fixture cava not running"; exit 1; }
[ "$(children playerctl)" -eq 1 ] || { echo "FAIL media follower not running while live"; exit 1; }
ls "$TEST"/niru-cava-*.conf > /dev/null 2>&1 && { echo "FAIL cava config left after the first frame"; exit 1; }
w0=$(wakeups); t0=$(ticks); sleep 1; live=$(($(wakeups) - w0)); live_ticks=$(($(ticks) - t0))
# Silence starts at 3 s; the repaint must have stopped by 6 s.
sleep 4.5
w0=$(wakeups); t0=$(ticks); sleep 3; quiet=$(($(wakeups) - w0)); quiet_ticks=$(($(ticks) - t0))
[ "$(children playerctl)" -eq 0 ] || { echo "FAIL media follower kept running in silence"; exit 1; }
[ "$(children cava)" -eq 1 ] || { echo "FAIL cava should stay up in silence"; exit 1; }
touch "$TEST/resume"
sleep 1.5
w0=$(wakeups); sleep 1; resumed=$(($(wakeups) - w0))
printf 'cava fixture: live=%s/s (%s ticks) silent=%s/3s (%s ticks) resumed=%s/s\n' "$live" "$live_ticks" "$quiet" "$quiet_ticks" "$resumed"
[ "$live" -ge 30 ] || { echo "FAIL no 30 fps repaint while live"; exit 1; }
[ "$quiet" -le 6 ] || { echo "FAIL more than 2 wakeups/s in silence"; exit 1; }
[ "$quiet_ticks" -le 1 ] || { echo "FAIL CPU used in silence"; exit 1; }
[ "$resumed" -ge 30 ] || { echo "FAIL spectrum did not resume on sound"; exit 1; }
[ "$(children playerctl)" -eq 1 ] || { echo "FAIL media follower not back with sound"; exit 1; }
xdotool key space; wait_gone
pgrep -f "$TEST/bin/cava" > /dev/null && { echo "FAIL cava outlived the saver"; exit 1; }
pgrep -f "$TEST/bin/playerctl" > /dev/null && { echo "FAIL playerctl outlived the saver"; exit 1; }
rm "$XDG_CONFIG_HOME/niruui/screensaver.toml"

# visual = "auto" (the default) without a sound server: quotes, never CAVA,
# even though the fixture would play, and a static screen between changes.
touch "$TEST/resume"
PULSE_SERVER=unix:$TEST/no-server "$BIN" > "$TEST/saver-auto.log" 2>&1 & SAVER=$!
sleep 3
kill -0 "$SAVER" || { cat "$TEST/saver-auto.log"; exit 1; }
quotes=$XDG_CONFIG_HOME/niru/quotes
for name in havamal-en havamal-sv stoic-en stoic-sv; do
  [ -f "$quotes/$name.txt" ] || { echo "FAIL the collection $name was not written"; exit 1; }
  [ "$(stat -c %a "$quotes/$name.txt")" = 644 ] || { echo "FAIL $name.txt is not 0644"; exit 1; }
done
[ "$(children cava)" -eq 0 ] || { echo "FAIL CAVA started without music"; exit 1; }
# The drawing thread must sleep between quote changes. The audio detector's
# own thread retries the missing sound server with a back-off (2 s, 4 s, ...
# up to a minute), so it is bounded separately.
main_wakeups() { awk '/ctxt_switches/{s+=$2} END{print s+0}' /proc/"$SAVER"/task/"$SAVER"/status; }
w0=$(wakeups); m0=$(main_wakeups); t0=$(ticks); sleep 3
quiet=$(($(wakeups) - w0)); main=$(($(main_wakeups) - m0)); quiet_ticks=$(($(ticks) - t0))
printf 'quotes: %s wakeups/3s on the drawing thread, %s in all threads (%s ticks), no CAVA\n' "$main" "$quiet" "$quiet_ticks"
[ "$main" -le 2 ] || { echo "FAIL the quote view woke between changes"; exit 1; }
[ "$quiet" -le 30 ] || { echo "FAIL the audio detector retried too often"; exit 1; }
[ "$quiet_ticks" -le 1 ] || { echo "FAIL CPU used on the quote view"; exit 1; }
printf 'My own quote\n' > "$quotes/havamal-en.txt"
rm "$quotes/stoic-sv.txt"
xdotool key space; wait_gone
PULSE_SERVER=unix:$TEST/no-server "$BIN" > "$TEST/saver-auto.log" 2>&1 & SAVER=$!
sleep 2
[ "$(cat "$quotes/havamal-en.txt")" = "My own quote" ] || { echo "FAIL a collection was overwritten"; exit 1; }
[ ! -e "$quotes/stoic-sv.txt" ] || { echo "FAIL a deleted collection was recreated"; exit 1; }
xdotool key space; wait_gone
rm "$TEST/bin/cava" "$TEST/bin/playerctl"

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
printf 'PASS: input, mouse, resize coverage, opacity hint, swallowed dismissal input, motion, CAVA silence pause, media follower and failed-grab handling\n'
