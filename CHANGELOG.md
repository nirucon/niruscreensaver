# Changelog

Releases before 0.18.0 are described in their GitHub release notes and the
git history.

## 0.19.0 (2026-11-26)

### Added

- **`c64` and `c64-sid`, new scenes: NIRUX as a Commodore 64 demo.** Each
  monitor gets a 320x200 C64 screen in a border that reaches its edges, at
  the largest whole scale with sharp pixels, centred to the pixel. A short
  BASIC boot (`LOAD"NIRUX",8,1`, loading stripes in the border) leads into
  three parts that loop, 24 seconds each, with the border closing over the
  screen between them: NIRUX (raster bars in the border, a bouncing and
  stretching NIRUX logo, a DYCP scroller with greetings and the time,
  mirrored in water), BIFROST (a rotating dot sphere among 24 balls on a
  Lissajous path, the Elder Futhark runes for NIRUX, rainbow bars) and
  YGGDRASIL (the world tree before a dithered character plasma, the time in
  the lower border). Only the sixteen C64 colours are used, picked by the
  theme's accent and text colour. `c64` is silent; `c64-sid` plays the
  pause music like `dark-sid`, shows the tune in the upper border and pulses
  the effects on the beat. About 2 % of one core at 1080p: frames are drawn
  in C64 pixels, only changed rows are sent (MIT-SHM where possible) and the
  X server scales them with RENDER. Nothing is drawn with the display off;
  `reduced_motion` shows a still part that changes every minute. Without
  RENDER it falls back to `dark`. Both are in `--list`, `--set scene`,
  `--preview` and can be added to `shuffle`; they are not in the default
  shuffle list.

### Changed

- `install.sh` installs 0.19.0 by default (it still named 0.13.1).

## 0.18.0 (2026-11-21)

### Added

- **`demo`, a new scene.** A realtime demoscene demo by NIRUDEMO on every
  monitor: the saver puts a black window on each one and starts
  `nirudemo --into 0xWINDOW --loop --silent` for it. The saver keeps the
  keyboard and pointer grabs, so any key or click ends it as before, and
  NIRUDEMO never gets input. NIRUDEMO runs in its own process group with no
  input or output, dies with the saver (`PR_SET_PDEATHSIG`) and is stopped
  (TERM, then KILL) when the saver ends or the display goes off. Without
  `nirudemo`, or when it exits within 2 seconds, that monitor shows `dark`
  instead. The scene is silent. `--list`, `--set scene demo`,
  `--preview demo` and `--shuffle add demo` know it; it is not in the
  default shuffle list.
- `visual = "music"` (from NIRULOCK 0.28.0): the spectrum while music plays,
  otherwise nothing.

### Changed

- **`dark-sid` and `ascii-sid` play NIRUX's own music by default.** The
  shared player (NIRULOCK 0.29.0) reads `~/.local/share/nirux/music`, where
  NIRUX installs eleven original SID-style chiptunes, unless NIRULOCK's
  `[pause_music] folder` says otherwise (`folder = "~/Music/SID"` for a SID
  collection). The folder may hold `.sid` tunes and `.ogg`, `.oga`, `.opus`,
  `.flac` or `.wav` files, mixed; audio files play with `pw-play`, `ffplay`
  or `mpv`. The tune line reads `♪ Ginnungagap  ·  NIRUX 1`.
- The NIRULOCK twins follow NIRULOCK 0.29.0 (`tests/nirulock-tag`).
