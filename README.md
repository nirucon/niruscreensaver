# NIRUSCREENSAVER

Small X11 idle screen for the NIRU desktop, showing date, clock, hostname and
away time in the active theme on an opaque black background.

Part of the [NIRUX suite](https://github.com/nirucon/nirux): [NIRUWM](https://github.com/nirucon/niruwm) · [NIRUBAR](https://github.com/nirucon/nirubar) · [NIRUSCREENSAVER](https://github.com/nirucon/niruscreensaver) · [NIRULOCK](https://github.com/nirucon/nirulock) · [NIRULAUNCHER](https://github.com/nirucon/nirulauncher) · [NIRUNOTIFY](https://github.com/nirucon/nirunotify) · [NIRUSHOT](https://github.com/nirucon/nirushot).

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

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
sh tests/resource-check.sh
```

MIT. Copyright 2026 Nicklas Rudolfsson.
