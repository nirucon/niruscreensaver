# NIRUSCREENSAVER

**Part of [NIRUX - A minimal X11 desktop by Nicklas Rudolfsson](https://github.com/nirucon/nirux).**

**Early beta.** Small X11 idle screen for the NIRU desktop, showing date, clock, hostname,
away time and an optional CAVA spectrum in the active theme on an opaque black background.


## Install

Use [NIRUX](https://github.com/nirucon/nirux#install) for the complete desktop,
or install the standalone x86_64/glibc release:

```sh
git clone https://github.com/nirucon/niruscreensaver
cd niruscreensaver
./install.sh
```

`./install.sh --build` needs Rust 1.75+, a C linker, fontconfig and a monospace
font. Prefix: `~/.local`; `--prefix DIR` overrides it. NIRUX-owned binaries are
maintained with `nirux update`.

## Use

```sh
niruscreensaver
niruscreensaver --config
```

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

Settings live in `~/.config/niruui/screensaver.toml` (XDG respected).
`idle_minutes`, `lock_after_minutes` and `dpms_off_minutes` are read by
`niruidle`. A lock never expires automatically. Colours follow the selected
theme in `~/.config/niruwm/config.toml`; optional `[palette]` overrides affect
content only.

### Layouts

Choose one with `layout = "minimal" | "terminal" | "runic" | "corner" | "cava"`:

- `cava` — the default: large clock and hostname above a live CAVA spectrum.
- `minimal` — the original large centred clock and hostname.
- `terminal` — left-aligned monospace idle screen with a `hostname:~$ idle` line.
- `runic` — the clock with a discreet rune row when a runic font is available.
- `corner` — clock top-left, hostname and away time at the bottom.

The `cava` layout samples system audio through [CAVA](https://github.com/karlstav/cava)
at 30 frames per second and uses the same fast-attack, slower-release smoothing
as NIRUAMP. Only the spectrum rectangle is repainted between full clock frames,
and CopyArea exposures are disabled. Bars are drawn from the active NIRUX
theme's focus colour, with a lighter peak, so the spectrum follows every suite
theme. Set `visualizer = false` to keep the cava composition without the
spectrum. If CAVA is not installed, the clock and hostname are still shown;
install `cava` to enable the bars. With the default `visual = "auto"` CAVA
only runs while music plays; see below.

### Music or quotes (`visual`)

`visual` decides what the `cava` layout shows where the spectrum is drawn:

- `auto` (the default): the CAVA spectrum only while music or radio plays, and
  otherwise one quote from the [quote collections](#quotes) below.
- `cava`: the spectrum alone, exactly as before 0.19 (CAVA runs whenever the
  display is on).
- `quotes`: quotes alone; CAVA never starts.
- `none`: neither; the panel stays empty. `visualizer = false` without a
  `visual` setting means `none`, as before.

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

### Quotes

The quotes live in `${XDG_CONFIG_HOME:-~/.config}/niru/quotes/`, shared by
NIRULOCK and NIRUSCREENSAVER. Every `NAME.txt` file there is one
*collection*, named `NAME`. When the directory is missing, either program
creates it (atomically; directory 0755, files 0644) with the built-in
collections:

| Collection | Contents |
|---|---|
| `havamal-en` | Hávamál in English |
| `havamal-sv` | Hávamál in Swedish |
| `stoic-en` | Stoic quotes in English |
| `stoic-sv` | Stoic quotes in Swedish |

After that the directory is yours: neither program ever writes into it
again, so an edited collection stays edited and a deleted one stays deleted.
To get the built-in collections back, remove (or rename) the directory; it
is created again the next time the lock or screensaver starts. Each file's
own header explains the format with an example.

#### The format

```text
# Lines starting with # are comments.
Quotes are separated by one or more blank lines.
A quote may span several lines;
its line breaks are kept.
— A line starting with "— " (em dash, space) or "-- " is the attribution

He who waits for something good
never waits too long.
-- Swedish proverb
```

- **Add a quote:** write it anywhere in a collection, for example at the end,
  with a blank line before and after it.
- **Remove a quote:** delete its lines, or put `#` in front of each of them.
- **Add a collection:** create another `.txt` file in the directory, for
  example `mine.txt` (the collection `mine`). Files whose names start with a
  dot, and files that are not `.txt`, are ignored.
- **Remove a collection:** delete its file, or leave it out of `quotes` below.

Files must be UTF-8. Very long quotes are shortened with an ellipsis so that
they fit. At most 64 collections are read (in name order), and a file larger
than 256 KiB is skipped. Changes are picked up the next time the lock or
screensaver starts and, while one is showing, at the next quote change. The
files are read on their own thread, never on the input or drawing path.

#### Choosing collections (`quotes`)

`quotes` in `~/.config/niruui/lock.toml` (NIRULOCK) and
`~/.config/niruui/screensaver.toml` (NIRUSCREENSAVER) selects collections by
name. `*` matches any run of characters and `?` any single character. One
pattern or an array of them:

```toml
quotes = ["*"]                       # the default: every collection
quotes = "havamal-sv"                # one collection
quotes = ["havamal-*", "stoic-en"]   # several, with a pattern
quotes = ["*-en", "mine"]            # English collections and your own
```

The quotes of all selected collections are put in one pool and shown in a
shuffled order: every quote once before any is repeated, and never the same
quote twice in a row, so a small collection is not shown more often than a
large one. The two programs can select different collections.

If nothing is selected, or the selected collections are missing, unreadable
or hold no quote, the built-in collections are shown instead (those the
patterns name, otherwise all of them), so the panel is never empty. To show
no quotes at all, set `visual = "cava"` or `visual = "none"`.

#### The old `quotes.txt`

NIRULOCK 0.19 and NIRUSCREENSAVER 0.11 used a single file,
`~/.config/niru/quotes.txt`. If you never edited it, it is ignored now (its
stanzas are the `havamal-sv` collection) and you can delete it. If you did
edit it, it still counts as the collection `quotes`; move your quotes into a
file in `quotes/` when convenient and delete the old file.

#### Sources

- `havamal-sv`: 22 stanzas of Hávamál ("Den höges sång") in Erik Brate's
  Swedish translation, *Eddan. De nordiska guda- och hjältesångerna* (1913),
  public domain (Brate died in 1924), verbatim from Project Runeberg,
  <https://runeberg.org/eddan/se-02.html>. These are the stanzas of the old
  default `quotes.txt`.
- `havamal-en`: the same 22 stanzas in Henry Adams Bellows' English
  translation, *The Poetic Edda* (1923), public domain (Bellows died in 1939),
  from Wikisource. Stanza numbers are Bellows'; four differ by one from
  Brate's.
- `stoic-en`: 32 passages on moderation, few words, friendship, fate,
  mortality and simple living: Marcus Aurelius and Epictetus' *Encheiridion*
  in George Long's translations (1862 and later; Long died in 1879), and
  Seneca's *Minor Dialogues* and *On Benefits* in Aubrey Stewart's
  translations (1887–1889; Stewart died in 1918), from Project Gutenberg.
- `stoic-sv`: the same 32 passages in Swedish. Three Seneca passages are
  verbatim from Johan Bergman's translation, *Om livets korthet* (2nd ed.
  1918), public domain (Bergman died in 1951), from Project Runeberg. The
  other 29 are **not** a published translation: they are Swedish renderings
  made for NIRUX from Long's and Stewart's English, and their attributions
  say "NIRUX-tolkning".

`…` marks an omission inside a passage. Every quote was checked word for
word against its source; the exact passages, URLs and editorial changes are
listed in [`quotes/SOURCES.md`](quotes/SOURCES.md).

While audio plays, `show_media = true` (the default) shows the playing MPRIS
track as "Artist – Title" under the spectrum. It needs `playerctl`, which is
started once with `--follow` while the spectrum is live and reports each track
change, so nothing is polled.

#### Power

The spectrum only runs while it can be seen and has something to show:

- **Display off** (DPMS not `On`, for example after `dpms_off_minutes`): CAVA
  and `playerctl` are stopped and reaped and one static frame stays. They
  restart within a second of the display coming back. DPMS 1.2 servers report
  the change as an event; older ones are checked every 2 s and at once on
  input.
- **Silence**: after 2 s with every bar below the noise floor the 30 fps
  repaint stops after one flat frame, `playerctl` is stopped, and the first
  non-silent sample resumes it. CAVA's own `sleep_timer` pauses its output
  during the same silence. When sound returns the bars rise from the flat
  frame with the normal attack instead of jumping to the first sample, and
  quiet sounds just above the noise floor grow from zero.
- With nothing to animate the saver wakes only for the minute clock and the
  away text. CAVA runs with a fixed sensitivity and the levels are normalized
  in the saver, so the noise of an idle audio stream never looks like sound.
  CAVA dies with the saver (`PDEATHSIG`), and its temporary config is removed
  after the first frame.

The CAVA, audio detector, panel and quotes code is shared verbatim with
NIRULOCK (`src/cava.rs`, `src/audio.rs`, `src/visual.rs`, `src/quotes.rs`,
`src/palette.rs` and the collections in `quotes/`, checked against the tag in
`tests/nirulock-tag` by `tests/twin-drift.sh`).

Layout only changes composition; the NIRUX theme still owns colours. The screen
stays opaque black. A missing runic font omits the ornament, never draws empty
glyph boxes. See [config.example.toml](config.example.toml).

## NIRUX suite

[NIRUX](https://github.com/nirucon/nirux) · [NIRUWM](https://github.com/nirucon/niruwm) · [NIRUBAR](https://github.com/nirucon/nirubar) · [NIRUSCREENSAVER](https://github.com/nirucon/niruscreensaver) · [NIRULOCK](https://github.com/nirucon/nirulock) · [NIRULAUNCHER](https://github.com/nirucon/nirulauncher) · [NIRUNOTIFY](https://github.com/nirucon/nirunotify) · [NIRUSHOT](https://github.com/nirucon/nirushot)

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
sh tests/resource-check.sh target/release/niruscreensaver   # Xvfb, xdotool, xev
sh tests/installer.sh
sh tests/twin-drift.sh                                      # needs network
```

`NIRUSCREENSAVER_TRACE=1` logs how the display power is followed and its
changes.

`NIRU_CAVA_TRACE=/path/to/file` appends one line per spectrum frame (the
milliseconds since start, `L`ive, `S`ilent or display `O`ff, and the 48 bar
heights) for measuring motion and frame pacing; it records nothing else.

MIT. Copyright 2026 Nicklas Rudolfsson.
