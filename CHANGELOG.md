# Changelog

Releases before 0.18.0 are described in their GitHub release notes and the
git history.

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
