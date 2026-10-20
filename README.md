# Niruscreensaver

Small X11 idle screen with date, clock, hostname and away time. Theme-coloured
content on opaque black. A key or click dismisses it, and root/output resizes keep
full coverage; use **nirulock** for locking. If the keyboard or pointer grab fails,
the saver exits instead of claiming success.
Supports **Void glibc, Debian and Arch Linux**.

## Install

Use [NIRUX](https://github.com/nirucon/nirux#install) for the complete desktop,
or install the standalone x86_64/glibc release:

```sh
git clone https://github.com/nirucon/niruscreensaver
cd niruscreensaver
./install.sh
```

`./install.sh --build` needs Rust 1.75+, a C linker, fontconfig and a monospace
font. Prefix: `~/.local` (`--prefix DIR` overrides). NIRUX-owned binaries must be
maintained with `nirux update`.

## Use and settings

```sh
niruscreensaver          # show immediately
niruscreensaver --config # print settings
```

Settings: `~/.config/niruui/screensaver.toml` (XDG respected).
`idle_minutes`, `lock_after_minutes` and `dpms_off_minutes` are read by niruidle.
Legacy `lock_timeout_minutes` is ignored. A lock never expires automatically.
Colours follow the selected `~/.config/niruwm/config.toml` theme; optional
`[palette]` overrides affect content. Fonts and hostname are cached; drawing
follows visible clock/away changes and X events.

[Roadmap](https://github.com/nirucon/nirux/blob/main/ROADMAP.md) ·
[Resource test](tests/resource-check.sh) · [Installer test](tests/installer.sh)

MIT. Copyright 2026 Nicklas Rudolfsson.
