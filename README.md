# Niruscreensaver

A dark idle screen for X11, showing the date, the time, the hostname and how
long the machine has been away.

Part of a small set of programs that share one look. Each works on its own:

- [niruwm](https://github.com/nirucon/niruwm), the window manager
- [nirubar](https://github.com/nirucon/nirubar), the status bar
- [nirulock](https://github.com/nirucon/nirulock), the lock screen

**A personal hobby project.** I build this for my own machines. It is public so
others can read it and try it, but there is no support promise. Early software,
used at your own risk.

## What it is

A full screen, Niru Noir dark, with white text. No animation, no graphics.
Just the clock. A key press or a click dismisses it.

It does not lock the screen. For that, use [nirulock](https://github.com/nirucon/nirulock).

## How it works

The program draws one window that covers the screen, then waits. It redraws
only when the minute changes, so an idle screensaver uses no CPU. It reads the
idle time from the X server, and the time from the system clock. No compositor
and no GPU are needed.

Starting and stopping it is up to the session, not the program. NIRUWM's
session script does it; from a terminal:

```sh
niruscreensaver          # show it now
niruscreensaver --config # print the settings in use
```

## Install

Releases contain a static x86_64 binary, so no Rust is needed.

```sh
git clone https://github.com/nirucon/niruscreensaver
cd niruscreensaver
./install.sh
```

To build from source (Rust 1.75 or newer):

```sh
./install.sh --build
```

It installs to `~/.local/bin`. Add `--prefix DIR` to change that.

## Settings

`~/.config/niruui/screensaver.toml`. Everything is optional.

```toml
font_path = "/path/to/monospace.ttf"  # default: fontconfig lookup
font_size = 22.0
clock_size = 88.0

# How long the session waits before showing this screen.
idle_minutes = 15

# Lock this many minutes after the screen appears, then give the lock screen
# this many more minutes before releasing it. 0 disables either step.
lock_after_minutes = 15
lock_timeout_minutes = 5

# Turn the display off this many minutes after it appears. 0 disables.
dpms_off_minutes = 15

# Show "away for ..." after this many idle seconds.
show_away_after_seconds = 60

[palette]
background = "0f0f10"
foreground = "e5e5e5"
muted = "a8a8a8"
border = "2a2a2d"
focus = "5a5a60"
```

These numbers are read by `niruidle` and by `niruwm` itself, and
`lock_timeout_minutes` is read by `nirulock`, so one file decides when every
NIRU program sleeps, locks and wakes. Running this program by hand shows the
screen immediately.

## Status

Version 0.6.0. Shows the date, time, hostname and away counter.

Missing: password protection (use nirulock), multiple screens, and a preview in
the settings file.

## Build needs

Rust 1.75 or newer, a C linker, `fontconfig` and a monospace font.

## License

MIT. Copyright 2026 Nicklas Rudolfsson.
