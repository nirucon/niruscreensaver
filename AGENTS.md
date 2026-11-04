# Agent assignment: NIRUSCREENSAVER 0.10.0, a one-shot energy and correctness update

## 0. Quick start

1. Read this file, `README.md`, `src/main.rs`, `src/cava.rs`, `src/core.rs` and
   `tests/resource-check.sh`. The audit baseline is `cfaadda` (0.9.1, pinned by NIRUX
   0.12.199). Revalidate each item against current HEAD and skip items that are already
   fixed, noting that you did.
2. If the NIRULOCK 0.18.0 assignment (nirucon/nirulock `AGENTS.md`, item K3) has
   already landed, **copy its CAVA pause logic from `cava.rs` verbatim** instead of
   writing a second version. Otherwise implement S1 here, keep it self-contained in
   `cava.rs`, and say so in the report.
3. Implement **S1–S6, then U1–U2**. Commit per item. Keep `cargo fmt --check`,
   `clippy -D warnings`, tests, `tests/installer.sh` and `tests/resource-check.sh` green.
4. Pass the gates in §4, release in §5 and report in §6.

This is an **execution** assignment. Do not stop after a plan or a branch. Make sensible
decisions, document them, and continue without routine questions.

## 1. Invariants

- NIRUSCREENSAVER is **not** a lock. A key or a click dismisses it. It must never
  claim to lock, and it must keep exiting non-zero when it cannot grab input (the
  existing test checks this).
- Full coverage on root resize and output hotplug stays.
- `niruidle` (in nirucon/niruwm) owns the idle ladder (`idle_minutes`,
  `lock_after_minutes`, `dpms_off_minutes`). Do not change niruidle in this assignment.
- Keep `rust-version = 1.75`. Locked builds must work on 1.75 and 1.85.

## 2. Findings (verified 2026-11-04 in a separate Xvfb on niruxwork)

### S1: CAVA renders at 30 fps into a dark or silent screen (high, battery)
`cava` is the **default** layout. With no audio playing, the saver used about **2.7 %
CPU**, the `cava` child about **1.8 %**, and the process woke about **180 times/s**. The
loop (`run()` ~371) never checks DPMS, so after `dpms_off_minutes` the panel is off while
the saver keeps rendering and capturing audio. This happens every time the laptop idles.
**Fix:** the same behavior as NIRULOCK K3:
- Enable the x11rb `dpms` feature and pause while DPMS is not `On`. While paused, kill and
  reap `cava` and draw one static frame. Restart it within 1 s of `On`.
- Stop the 30 fps spectrum repaint after 2 s of silence (all bars ≈ 0), draw one flat
  frame, and resume on the first non-zero sample.
- With nothing to animate, the loop wakes only for the minute clock and away-time changes,
  as `next_redraw_ms` already computes.

**Done when:** the resource check (§4) shows ≤ 2 wakeups/s and no 30 fps repaint during a
silent fixture, 0 `cava` processes with the display forced off on real hardware, and bars
returning within 1 s after wake or when audio starts.

### S2: The compositor can dim the saver (correctness, diverged from NIRULOCK)
NIRULOCK's `core::Surface::new` sets `_NET_WM_WINDOW_OPACITY = 0xffffffff` and a
`WM_CLASS`, because picom dims unknown frames. The copy in NIRUSCREENSAVER lacks both,
so the desktop can show through an "opaque black" idle screen, which also fails the
burn-in purpose. **Fix:** port both properties exactly. Add an x11 test that reads them
back.

### S3: The dismissing key leaks or is lost unpredictably
Check what happens to the KeyPress that dismisses the saver: it must not reach the focused
application, for example by typing into a terminal. With an active grab it should be
swallowed. Verify that, and also that the following KeyRelease does not reach the app as a
stray event. If it does, swallow the release as well before ungrabbing. Add a test with
xdotool and an xev-like test client.

### S4: Pointer motion does not dismiss
Today only a key or a click dismisses the saver. This is intentional, so a bumped desk does
not wake it, but the README does not say so. Add a config option
`dismiss_on_motion = false` (default unchanged). When it is true, dismiss after the pointer
has moved more than 24 px since the saver appeared. Document it.

### S5: Every frame queries idle time synchronously
`idle_via_extension` does a round trip per full draw. That is fine for the minute clock,
but keep it out of the 30 fps CAVA path and verify that it is.

### S6: Duplicated code with NIRULOCK
`core.rs`, `cava.rs`, `palette.rs` and `config.rs` are copies that have already drifted
(S2). Do not build a shared crate in this assignment. Instead:
- add a header comment to each copied file naming its NIRULOCK twin, and
- add a CI check that diffs `cava.rs` and `palette.rs` against the pinned NIRULOCK tag
  (fetch them with `curl` from the tag) and fails on unexpected drift. Changes must then be
  made in both repos.

## 3. UX

### U1: Show playing media in the CAVA layout
When CAVA is active and audio is playing, show the MPRIS "Artist – Title" line under the
spectrum. Read it once per track change using `playerctl metadata --format`, bounded with a
1 s deadline, or D-Bus if trivially available. Do not poll faster than every 5 s, and stop
completely while paused (S1). Make it optional with `show_media = true` and document it.

### U2: Fade in
Fade the first frame in from black over 300 ms so the saver does not appear abruptly. A
single short animation is enough. Respect `reduced_motion = true` (new option, default
false), which skips it.

Out of scope: new layouts, images or slideshows, lock behavior, and changes to the
niruidle ladder.

## 4. Gates

Automated:
- `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`,
  `cargo test --locked`, a release build, `sh tests/installer.sh`.
- An MSRV job in CI (`dtolnay/rust-toolchain@1.75`, `cargo build --locked`).
- Extend `tests/resource-check.sh`:
  - a fixture `cava` on PATH that prints audible bars for 3 s and then zeros,
  - assert that the 30 fps repaint stops within 3 s after silence starts (measure context
    switches and CPU ticks), and that the spectrum resumes when the fixture prints
    non-zero values,
  - a DPMS-off unit test of the pause state machine (Xvfb has no DPMS),
  - S2 properties, S3 key swallowing and S4 motion.

Native:
- On **niruxwork** (the owner allows local testing): run the saver directly with real
  audio and with silence, force DPMS off and on (`xset dpms force off`, then a key), and
  check `pgrep cava`, CPU and that the bars return.
- Also check with the owner's compositor running (picom, if used) that the saver is
  fully opaque.
- Running the saver on the owner's session is safe: a key dismisses it. Do not change the
  owner's `screensaver.toml`. Use `--config`/env overrides or a temporary
  `XDG_CONFIG_HOME` for option tests.

## 5. Release

Bump to **0.10.0**. Update the README: the power behavior, the new options and the
dismissal rules. Tag `v0.10.0` through the normal workflow and verify the assets and
checksums. Then pin it in `nirucon/nirux` `manifest`:
- If `nirux/tools/suite-pin` exists, use it.
- Otherwise bump the manifest the usual way. Change only the niruscreensaver line, and
  rebase if other pins land at the same time.

Install through `nirux update` on niruxwork and verify that the running binary comes from
the new generation.

## 6. Final report

For each item S1–S6 and U1–U2: the change, the tests and the evidence (CPU, wakeups and
`cava` counts before and after, with silence and with the display off). Include release
URLs, installed versions and remaining limits. Mark anything unfinished as **PARTIAL** and
give the exact remaining steps.
