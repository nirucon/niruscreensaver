# How NIRUSCREENSAVER behaves

Technical details behind the [README](../README.md): the CAVA spectrum,
music detection, the media line and power use.

## Dismissing and compositing

NIRUSCREENSAVER is not a lock. A key or click dismisses the screen. The
dismissing key or click is swallowed, release included, so it never types
into or clicks the window underneath. Pointer motion does not dismiss by
default, so a bumped desk does not wake the screen; `dismiss_on_motion = true`
dismisses once the pointer has moved more than 24 px since the screen
appeared. Root and output resizes keep full coverage. If the keyboard or
pointer grab fails, the saver exits non-zero instead of claiming success. Use
[NIRULOCK](https://github.com/nirucon/nirulock) for locking.

The window asks compositors for full opacity (`_NET_WM_WINDOW_OPACITY`) and
has the class `NIRUSCREENSAVER`, so picom cannot dim it and show the desktop
through it. It fades in from black over 300 ms; `reduced_motion = true` shows
it at once.

## The CAVA spectrum

The `cava` layout samples system audio through [CAVA](https://github.com/karlstav/cava)
at 30 frames per second and uses the same fast-attack, slower-release smoothing
as NIRUAMP. Each CAVA frame is painted the moment it arrives, so every frame is
shown exactly once. Only the spectrum rectangle is repainted between full clock frames,
and CopyArea exposures are disabled. Bars are drawn from the active NIRUX
theme's focus colour, with a lighter peak, so the spectrum follows every suite
theme. Set `visualizer = false` to keep the cava composition without the
spectrum. If CAVA is not installed, the clock (and the hostname, with `show_hostname = true`) is still shown;
install `cava` to enable the bars. With the default `visual = "auto"` CAVA
only runs while music plays.

## Music detection (`visual = "auto"`)

With `auto`, a small PulseAudio client (PipeWire serves the same API through
`pipewire-pulse`; libpulse is loaded at run time, so it is not a build or run
dependency) watches the playback streams through subscription events. Only
while a stream is playing (not corked) does it open a peak monitor on the
default sink's monitor, at ten peaks a second delivered five times a second;
with no stream nothing is polled at all. CAVA is started when the peak has
stayed above about -54 dBFS for 300 ms, so an always-present but silent stream
(an idle MIDI synthesizer) never starts it, and it is stopped and reaped, not
just paused, after 5 s of silence. The change fades the quote out and the
spectrum in; the bars still rise from flat with the usual attack. Without a
sound server the panel shows quotes and the client retries in the background
(every 2 s, backing off to once a minute). With the display off nothing runs:
no CAVA, no peak monitor, no quote changes. `reduced_motion = true` changes
quotes and the spectrum without fades.

A quote changes to another, chosen at random, every minute with a soft fade;
between changes nothing is redrawn. It is drawn in the NIRUX theme font and
colours, centred where the spectrum would be, wrapped to about 64 characters,
with a smaller size (down to half, at least 11 px) when it is long and an
ellipsis when it still does not fit. Attributions are muted.

The quote collections themselves are described in [QUOTES.md](QUOTES.md).

## Media line

While audio plays, `show_media = true` (the default) shows the playing MPRIS
track as "Artist – Title" under the spectrum. It needs `playerctl`, which is
started once with `--follow` while the spectrum is live and reports each track
change, so nothing is polled.

## Power

The spectrum only runs while it can be seen and has something to show:

- **Display off** (DPMS not `On`, for example after `dpms_off_minutes`): CAVA
  and `playerctl` are stopped and reaped and one static frame stays. They
  restart within a second of the display coming back. DPMS 1.2 servers report
  the change as an event; older ones are checked every 2 s and at once on
  input.
- **Silence**: after 2 s with every bar below the noise floor the
  per-frame repaint stops after one flat frame, `playerctl` is stopped, and the first
  non-silent sample resumes it. CAVA's own `sleep_timer` pauses its output
  during the same silence. When sound returns the bars rise from the flat
  frame with the normal attack instead of jumping to the first sample, and
  quiet sounds just above the noise floor grow from zero.
- With nothing to animate the saver wakes only for the minute clock and the
  away text. CAVA runs with a fixed sensitivity and the levels are normalized
  in the saver, so the noise of an idle audio stream never looks like sound.
  CAVA dies with the saver (`PDEATHSIG`), and its temporary config is removed
  after the first frame.

## The demo scene

`demo` creates one black child window per monitor inside the saver's window
and starts `nirudemo --into 0xWINDOW --loop --silent` for each, once the
window exists on the server. The window has no event mask: every key, click
and motion goes to the saver, which holds the keyboard and pointer grabs, so
dismissing works exactly as in the other scenes and NIRUDEMO never sees input.
NIRUDEMO draws over its own X connection and follows the window's size; the
saver moves and resizes the windows when the monitors change and starts over
when their number changes.

Each NIRUDEMO runs in its own session and process group, with stdin, stdout
and stderr on `/dev/null`, and `PR_SET_PDEATHSIG` set to `SIGKILL`, so it
dies with the saver even if the saver is killed. When the saver ends or the
display goes off its group gets `SIGTERM`, then `SIGKILL` after 0.5 s, and it
is reaped; it starts again when the display comes back. Without `nirudemo` on
`PATH`, or when one exits within 2 s of its start, that monitor shows the
`dark` scene instead (one log line); one that ends later is started again.
While the demos run the saver looks at them every 2 s (every 0.2 s in their
first 2 s) and draws nothing itself. The scene plays no music.

## The c64 scene

`c64` keeps one frame buffer per monitor in C64 pixels, one byte a pixel
(about 480x270 for a 1080p monitor, whatever its size), drawn from the time
alone. Each frame is compared with the one before; only the bands of rows
that changed are turned into pixels through a 16 colour table, written to a
System V shared memory segment (MIT-SHM; a plain PutImage in bands where the
server cannot map it, as over ssh) and put into a pixmap of the same size.
The server then scales those bands onto the window with RENDER, nearest
neighbour, at a whole scale. The segment is marked for removal as soon as
the server has attached it, so it never outlives the saver.

Motion advances in whole frames: 50 a second up to 2560x1440 monitor pixels
in total, then slower, at most 25 a second. Every tenth frame waits for a
round trip, so a busy server is never flooded. With the display off no
frame is drawn and the loop sleeps; `c64-sid` also stops its music then
(when NIRULOCK's `stop_on_display_off` says so) and the CAVA child it
starts with the player to follow the beat (only when there are tunes to
play; CAVA pauses itself in silence as in the clock scene). Measured in Xvfb at 1920x1080
over a whole loop: about 1.8 % of one core for the saver (2 % in the busiest
part), 5 % for the X server's scaling (software in Xvfb, a GPU does it on
real hardware), 55 wakeups a second and 5.5 MB resident.

## Shared code

The CAVA, audio detector, panel and quotes code is shared verbatim with
NIRULOCK (`src/cava.rs`, `src/audio.rs`, `src/visual.rs`, `src/quotes.rs`,
`src/palette.rs` and the collections in `quotes/`, checked against the tag in
`tests/nirulock-tag` by `tests/twin-drift.sh`).

A missing runic font omits the ornament and never draws empty glyph boxes.

