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

A key or click dismisses the screen. Root and output resizes keep full coverage.
If the keyboard or pointer grab fails, the saver exits instead of claiming
success. Use [NIRULOCK](https://github.com/nirucon/nirulock) for locking.

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
and CopyArea exposures are disabled, so an idle screen does not burn CPU or spin
the event loop. Bars are drawn from the active NIRUX theme's focus colour, with
a lighter peak, so the spectrum follows every suite theme. Set `visualizer = false`
to keep the cava composition without the spectrum. If CAVA is not installed, the
clock and hostname are still shown; install `cava` to enable the bars.

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
sh tests/resource-check.sh
```

MIT. Copyright 2026 Nicklas Rudolfsson.
