//! Niruscreensaver: a dark idle screen with a clock.
//!
//! It draws the current time, the hostname and, once idle, how long the
//! machine has been away. A key press or button press dismisses it.
//!
//! By default it does nothing on its own: NIRUWM starts it after the idle
//! timeout. Run it directly to see it:
//!
//! ```text
//! niruscreensaver --preview     show it now
//! niruscreensaver --config      print the effective configuration
//! ```

mod cava;
mod config;
mod core;
mod palette;

/// One error type for the whole program, so `?` works with X11, IO and
/// configuration errors alike.
#[derive(Debug)]
pub struct Error(String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for Error {
    fn from(s: String) -> Self {
        Error(s)
    }
}

impl From<&str> for Error {
    fn from(s: &str) -> Self {
        Error(s.to_string())
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error(e.to_string())
    }
}

impl From<x11rb::errors::ConnectError> for Error {
    fn from(e: x11rb::errors::ConnectError) -> Self {
        Error(format!("cannot connect to X11: {e}"))
    }
}

impl From<x11rb::errors::ConnectionError> for Error {
    fn from(e: x11rb::errors::ConnectionError) -> Self {
        Error(format!("X11 connection error: {e}"))
    }
}

impl From<x11rb::errors::ReplyOrIdError> for Error {
    fn from(e: x11rb::errors::ReplyOrIdError) -> Self {
        Error(format!("X11 request failed: {e}"))
    }
}

impl From<x11rb::errors::ReplyError> for Error {
    fn from(e: x11rb::errors::ReplyError) -> Self {
        Error(format!("X11 reply failed: {e}"))
    }
}

pub type Result<T> = std::result::Result<T, Error>;

use chrono::Local;
use std::time::{Duration, Instant};
use x11rb::{
    connection::{Connection, RequestConnection as _},
    protocol::{screensaver::ConnectionExt as _, xproto::*, Event},
};

/// Idle time in seconds, read from the X server's MIT-SCREEN-SAVER extension
/// when available, otherwise from the time since the program started.
///
/// MIT-SCREEN-SAVER is the only portable way to ask X11 how long the user has
/// been idle without polling input devices.
fn idle_via_extension(c: &x11rb::rust_connection::RustConnection, root: Window) -> Option<u64> {
    let ext = c
        .extension_information(x11rb::protocol::screensaver::X11_EXTENSION_NAME)
        .ok()??;
    let _ = ext;
    let info = c.screensaver_query_info(root).ok()?.reply().ok()?;
    Some(u64::from(info.ms_since_user_input) / 1000)
}

#[allow(clippy::too_many_arguments)]
fn draw(
    surface: &mut core::Surface,
    cfg: &config::Config,
    text: &mut core::TextRenderer,
    big: &mut core::TextRenderer,
    rune: Option<&mut core::TextRenderer>,
    hostname: &str,
    idle: u64,
    spectrum: Option<&[f32]>,
) -> Result<()> {
    let pal = &cfg.palette;
    surface.clear(pal.background)?;
    let centre_x = (surface.width / 2) as i16;
    let line = text.line_height();
    let big_line = big.line_height();
    let width = i32::from(surface.width);
    let height = i32::from(surface.height);
    let left = |offset: i32| offset.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
    let top = |offset: i32| offset.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
    let show_away = idle >= u64::from(cfg.show_away_after_seconds);
    let away = core::away_text(idle);
    match cfg.layout {
        config::Layout::Minimal => {
            let clock_baseline = (surface.height / 2) as i16 - line;
            big.draw_centred(
                &surface.c,
                surface.gc,
                surface.back,
                surface.depth,
                centre_x,
                clock_baseline,
                &core::clock_text(Local::now()),
                pal.foreground,
                pal.background,
            )?;
            let host_baseline = clock_baseline + big_line;
            text.draw_centred(
                &surface.c,
                surface.gc,
                surface.back,
                surface.depth,
                centre_x,
                host_baseline,
                hostname,
                pal.muted,
                pal.background,
            )?;
            if show_away {
                text.draw_centred(
                    &surface.c,
                    surface.gc,
                    surface.back,
                    surface.depth,
                    centre_x,
                    host_baseline + line * 2,
                    &away,
                    pal.border.max(pal.focus),
                    pal.background,
                )?;
            }
        }
        config::Layout::Terminal => {
            let x = left((width / 10).max(24));
            let start = top((height / 3).max(i32::from(big_line) * 2));
            text.draw_left(
                &surface.c,
                surface.gc,
                surface.back,
                surface.depth,
                x,
                start,
                &format!("{hostname}:~$ idle"),
                pal.muted,
                pal.background,
            )?;
            big.draw_left(
                &surface.c,
                surface.gc,
                surface.back,
                surface.depth,
                x,
                start + big_line * 2,
                &core::clock_text(Local::now()),
                pal.foreground,
                pal.background,
            )?;
            if show_away {
                text.draw_left(
                    &surface.c,
                    surface.gc,
                    surface.back,
                    surface.depth,
                    x,
                    start + big_line * 2 + line * 2,
                    &away,
                    pal.border.max(pal.focus),
                    pal.background,
                )?;
            }
        }
        config::Layout::Runic => {
            let clock_baseline = (surface.height / 2) as i16 - line;
            if let Some(rune) = rune {
                rune.draw_centred(
                    &surface.c,
                    surface.gc,
                    surface.back,
                    surface.depth,
                    centre_x,
                    clock_baseline - line * 3,
                    "ᚠ  ᚢ  ᚦ  ᚨ  ᚱ  ᚲ  ᚷ  ᚹ",
                    pal.border.max(pal.focus),
                    pal.background,
                )?;
            }
            big.draw_centred(
                &surface.c,
                surface.gc,
                surface.back,
                surface.depth,
                centre_x,
                clock_baseline,
                &core::clock_text(Local::now()),
                pal.foreground,
                pal.background,
            )?;
            let host_baseline = clock_baseline + big_line;
            text.draw_centred(
                &surface.c,
                surface.gc,
                surface.back,
                surface.depth,
                centre_x,
                host_baseline,
                hostname,
                pal.muted,
                pal.background,
            )?;
            if show_away {
                text.draw_centred(
                    &surface.c,
                    surface.gc,
                    surface.back,
                    surface.depth,
                    centre_x,
                    host_baseline + line * 2,
                    &away,
                    pal.border.max(pal.focus),
                    pal.background,
                )?;
            }
        }
        config::Layout::Corner => {
            let margin = (width / 10).max(16);
            let left_x = left(margin);
            let right_x = left(width - margin);
            let top_y = top(margin + i32::from(big_line));
            let bottom_y = top(height - margin - i32::from(line));
            big.draw_left(
                &surface.c,
                surface.gc,
                surface.back,
                surface.depth,
                left_x,
                top_y,
                &core::clock_text(Local::now()),
                pal.foreground,
                pal.background,
            )?;
            text.draw_left(
                &surface.c,
                surface.gc,
                surface.back,
                surface.depth,
                left_x,
                bottom_y.saturating_sub(line * 2),
                hostname,
                pal.muted,
                pal.background,
            )?;
            if show_away {
                let away_width = text.width(&away);
                text.draw_left(
                    &surface.c,
                    surface.gc,
                    surface.back,
                    surface.depth,
                    right_x.saturating_sub(away_width),
                    bottom_y,
                    &away,
                    pal.border.max(pal.focus),
                    pal.background,
                )?;
            }
        }
        config::Layout::Cava => {
            let (spec_x, spec_y, spec_w, spec_h) = cava_rect(surface);
            let clock_baseline = spec_y.saturating_sub(line * 2);
            big.draw_centred(
                &surface.c,
                surface.gc,
                surface.back,
                surface.depth,
                centre_x,
                clock_baseline,
                &core::clock_text(Local::now()),
                pal.foreground,
                pal.background,
            )?;
            text.draw_centred(
                &surface.c,
                surface.gc,
                surface.back,
                surface.depth,
                centre_x,
                spec_y.saturating_sub(line / 2),
                hostname,
                pal.muted,
                pal.background,
            )?;
            if let Some(values) = spectrum {
                cava::paint(surface, pal, spec_x, spec_y, spec_w, spec_h, values)?;
            }
            if show_away {
                text.draw_centred(
                    &surface.c,
                    surface.gc,
                    surface.back,
                    surface.depth,
                    centre_x,
                    spec_y
                        .saturating_add(spec_h as i16)
                        .saturating_add(line * 2),
                    &away,
                    pal.border.max(pal.focus),
                    pal.background,
                )?;
            }
        }
    }
    surface.present()?;
    Ok(())
}

fn cava_rect(surface: &core::Surface) -> (i16, i16, u16, u16) {
    let width = i32::from(surface.width);
    let height = i32::from(surface.height);
    let spec_w = ((width as f32 * 0.58) as i32).clamp(320, 1100) as u16;
    let spec_h = ((height as f32 * 0.17) as i32).clamp(96, 210) as u16;
    let spec_x = ((width - i32::from(spec_w)) / 2).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
    let spec_y = ((height - i32::from(spec_h)) / 2).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
    (spec_x, spec_y, spec_w, spec_h)
}

fn draw_cava_frame(
    surface: &core::Surface,
    cfg: &config::Config,
    cava: &mut Option<cava::Cava>,
) -> Result<()> {
    if cfg.layout != config::Layout::Cava {
        return Ok(());
    }
    let Some(cava) = cava.as_mut() else {
        return Ok(());
    };
    let (x, y, width, height) = cava_rect(surface);
    let values = cava.values().to_vec();
    cava::paint(surface, &cfg.palette, x, y, width, height, &values)?;
    surface.present_rect(x, y, width, height)?;
    Ok(())
}

/// Run the screensaver until a key or button dismisses it.
///
/// Wait for X input, the CAVA silence wake-up or the next visible clock/away
/// deadline, never a polling tick. The spectrum is repainted at 30 fps only
/// while the display is on and there is sound (see `cava::PowerGate`).
pub fn run(cfg: config::Config) -> Result<()> {
    let mut surface = core::Surface::new("NIRUSCREENSAVER")?;
    let font = core::find_font(cfg.font_path.as_deref());
    if font.is_empty() {
        return Err("no monospace font found; set font_path in the config".into());
    }
    let setup = surface.c.setup().clone();
    let mut text = core::TextRenderer::new(&font, cfg.font_size, &setup)?;
    let mut big = text.at_size(cfg.clock_size);
    let mut rune = core::find_rune_font(cfg.rune_font_path.as_deref())
        .map(|font| {
            core::TextRenderer::new(
                &font,
                (cfg.font_size * 2.2).min(cfg.clock_size * 0.6),
                &setup,
            )
        })
        .transpose()?;
    let hostname = core::hostname();
    let mut cava = if cfg.layout == config::Layout::Cava && cfg.visualizer {
        Some(cava::Cava::new())
    } else {
        None
    };
    // The display power is only needed to pause the spectrum.
    let mut power = cava.as_ref().map(|_| cava::DisplayPower::new(&surface.c));
    let trace = std::env::var_os("NIRUSCREENSAVER_TRACE").is_some();
    if let (Some(power), Some(cava)) = (power.as_ref(), cava.as_mut()) {
        if trace {
            eprintln!("niruscreensaver: display power: {}", power.mode());
        }
        cava.set_display(power.on());
    }
    let started = Instant::now();
    let mut next_full = Instant::now();
    let mut next_frame = Instant::now();
    let mut pending: Option<Event> = None;
    loop {
        let root = surface.c.setup().roots[surface.screen_num].root;
        let mut display_change = None;
        // Drain x11rb's buffered events before blocking on its socket.
        while let Some(event) = match pending.take() {
            Some(event) => Some(event),
            None => surface.c.poll_for_event()?,
        } {
            if let Some(power) = power.as_mut() {
                let changed = match &event {
                    // Input is what wakes a display; check it at once.
                    Event::KeyPress(_) | Event::ButtonPress(_) | Event::MotionNotify(_) => {
                        power.input(&surface.c, Instant::now())
                    }
                    event => power.event(event),
                };
                display_change = changed.or(display_change);
            }
            match event {
                // Any key or click dismisses it. The grabs are released so the
                // session can be used normally afterwards.
                Event::KeyPress(_) | Event::ButtonPress(_) => {
                    wait_for_release(&surface)?;
                    surface.release_input()?;
                    return Ok(());
                }
                Event::Expose(_) => next_full = Instant::now(),
                // A resolution change or monitor hotplug changes the root
                // geometry; the saver must not leave uncovered pixels.
                // ConfigureNotify can also be a child window; resize()
                // compares the actual root geometry and is cheap.
                Event::RandrScreenChangeNotify(_)
                | Event::RandrNotify(_)
                | Event::ConfigureNotify(_)
                    if surface.resize()? =>
                {
                    next_full = Instant::now();
                }
                _ => {}
            }
        }
        if let Some(on) = power
            .as_mut()
            .and_then(|power| power.poll(&surface.c, Instant::now()))
        {
            display_change = Some(on);
        }
        if let Some(on) = display_change {
            if trace {
                eprintln!("niruscreensaver: display {}", if on { "on" } else { "off" });
            }
            // Off stops and reaps CAVA; one static frame stays. On restarts it.
            if let Some(cava) = cava.as_mut() {
                cava.set_display(on);
            }
            next_full = Instant::now();
        }
        let now = Instant::now();
        if now >= next_full {
            let idle =
                idle_via_extension(&surface.c, root).unwrap_or_else(|| started.elapsed().as_secs());
            let spectrum = cava.as_mut().map(|cava| cava.values().to_vec());
            draw(
                &mut surface,
                &cfg,
                &mut text,
                &mut big,
                rune.as_mut(),
                &hostname,
                idle,
                spectrum.as_deref(),
            )?;
            let ms = next_redraw_ms(
                Local::now().timestamp_millis(),
                idle,
                cfg.show_away_after_seconds,
            );
            next_full = now + Duration::from_millis(ms);
            next_frame = now + FRAME;
        } else if now >= next_frame && cava.as_ref().is_some_and(cava::Cava::animating) {
            // The 30 fps spectrum path makes no X round trip: the idle time is
            // only needed for the away text of a full frame. The frame that
            // detects two seconds of silence is drawn flat and is the last.
            draw_cava_frame(&surface, &cfg, &mut cava)?;
            next_frame = now + FRAME;
        }
        surface.c.flush()?;
        // Round trips above may have queued events in x11rb, away from the fd.
        pending = surface.c.poll_for_event()?;
        if pending.is_some() {
            continue;
        }
        let mut deadline = next_full;
        if cava.as_ref().is_some_and(cava::Cava::animating) {
            deadline = deadline.min(next_frame);
        }
        if let Some(poll) = power.as_ref().and_then(cava::DisplayPower::deadline) {
            deadline = deadline.min(poll);
        }
        let timeout = deadline
            .saturating_duration_since(Instant::now())
            .as_millis()
            .min(60_000) as i32;
        let cava_wake = cava.as_ref().and_then(cava::Cava::wake_fd);
        let ready = wait_readable(
            [
                Some(std::os::fd::AsRawFd::as_raw_fd(surface.c.stream())),
                cava_wake,
            ],
            timeout.max(1),
        );
        if ready[1] {
            if let Some(cava) = cava.as_mut() {
                cava.woke();
            }
            next_frame = Instant::now();
        }
    }
}

/// Longest wait for the dismissing key or button to come up.
const RELEASE_WAIT: Duration = Duration::from_secs(1);

/// Keep the grabs until every key and button is up, at most [`RELEASE_WAIT`].
///
/// The grab swallows the dismissing KeyPress, but its KeyRelease came after
/// the ungrab and reached the window under the pointer, a stray event for a
/// terminal or editor. Everything that arrives meanwhile was meant for the
/// saver and is discarded. Event-driven: one check per release, no polling.
fn wait_for_release(surface: &core::Surface) -> Result<()> {
    let deadline = Instant::now() + RELEASE_WAIT;
    loop {
        while surface.c.poll_for_event()?.is_some() {}
        if !input_held(surface)? {
            return Ok(());
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Ok(());
        }
        surface.c.flush()?;
        if surface.c.poll_for_event()?.is_some() {
            continue;
        }
        wait_readable(
            [
                Some(std::os::fd::AsRawFd::as_raw_fd(surface.c.stream())),
                None,
            ],
            (left.as_millis() as i32).max(1),
        );
    }
}

/// True while any key or pointer button is down.
fn input_held(surface: &core::Surface) -> Result<bool> {
    let keys = surface.c.query_keymap()?.reply()?.keys;
    let pointer = surface.c.query_pointer(surface.win)?.reply()?;
    let buttons = KeyButMask::BUTTON1
        | KeyButMask::BUTTON2
        | KeyButMask::BUTTON3
        | KeyButMask::BUTTON4
        | KeyButMask::BUTTON5;
    Ok(keys.iter().any(|byte| *byte != 0) || u16::from(pointer.mask) & u16::from(buttons) != 0)
}

/// One spectrum frame, the same 30 fps as CAVA produces.
const FRAME: Duration = Duration::from_millis(33);

fn next_redraw_ms(wall_ms: i64, idle: u64, show_after: u32) -> u64 {
    let clock = (60_000 - wall_ms.rem_euclid(60_000)) as u64;
    let away = if idle < u64::from(show_after) {
        (u64::from(show_after) - idle) * 1000
    } else {
        (60 - idle % 60) * 1000
    };
    clock.min(away).max(1)
}

/// Wait on X and the CAVA silence wake-up without periodic polling. Returns
/// which descriptors became readable.
fn wait_readable(fds: [Option<std::os::fd::RawFd>; 2], timeout_ms: i32) -> [bool; 2] {
    let mut poll = fds.map(|fd| libc::pollfd {
        fd: fd.unwrap_or(-1),
        events: libc::POLLIN,
        revents: 0,
    });
    // SAFETY: two valid pollfd values; poll ignores a negative unused fd.
    let n = unsafe { libc::poll(poll.as_mut_ptr(), poll.len() as libc::nfds_t, timeout_ms) };
    if n <= 0 {
        return [false; 2];
    }
    poll.map(|p| p.fd >= 0 && p.revents & libc::POLLIN != 0)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = args.first().map(String::as_str).unwrap_or("");
    match arg {
        "--version" => {
            println!("NIRUSCREENSAVER {}", env!("CARGO_PKG_VERSION"));
        }
        "--config" => match config::Config::load() {
            Ok(cfg) => println!("{}", cfg.describe()),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1)
            }
        },
        "--preview" | "" => match config::Config::load() {
            Ok(cfg) => {
                if let Err(e) = run(cfg) {
                    eprintln!("niruscreensaver: {e}");
                    std::process::exit(1)
                }
            }
            Err(e) => {
                eprintln!("niruscreensaver: {e}");
                std::process::exit(1)
            }
        },
        _ => {
            eprintln!("Usage: niruscreensaver [--version|--config|--preview]");
            std::process::exit(2)
        }
    }
}

#[cfg(test)]
mod scheduling_tests {
    use super::next_redraw_ms;
    #[test]
    fn waits_for_the_first_visible_change() {
        assert_eq!(next_redraw_ms(0, 0, 60), 60_000);
        assert_eq!(next_redraw_ms(1_500, 58, 60), 2_000);
        assert_eq!(next_redraw_ms(59_750, 60, 60), 250);
        assert_eq!(next_redraw_ms(0, 61, 60), 59_000);
    }
}
