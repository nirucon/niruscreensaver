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
install `cava` to enable the bars.

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
  during the same silence.
- With nothing to animate the saver wakes only for the minute clock and the
  away text. CAVA runs with a fixed sensitivity and the levels are normalized
  in the saver, so the noise of an idle audio stream never looks like sound.
  CAVA dies with the saver (`PDEATHSIG`), and its temporary config is removed
  after the first frame.

The CAVA code is shared verbatim with NIRULOCK (`src/cava.rs`, checked against
the tag in `tests/nirulock-tag` by `tests/twin-drift.sh`).

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

MIT. Copyright 2026 Nicklas Rudolfsson.
