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

fn draw(
    surface: &mut core::Surface,
    cfg: &config::Config,
    text: &mut core::TextRenderer,
    big: &mut core::TextRenderer,
    rune: Option<&mut core::TextRenderer>,
    hostname: &str,
    idle: u64,
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
                &surface.c, surface.gc, surface.back, surface.depth,
                centre_x, clock_baseline, &core::clock_text(Local::now()),
                pal.foreground, pal.background,
            )?;
            let host_baseline = clock_baseline + big_line;
            text.draw_centred(
                &surface.c, surface.gc, surface.back, surface.depth,
                centre_x, host_baseline, hostname, pal.muted, pal.background,
            )?;
            if show_away {
                text.draw_centred(
                    &surface.c, surface.gc, surface.back, surface.depth,
                    centre_x, host_baseline + line * 2, &away,
                    pal.border.max(pal.focus), pal.background,
                )?;
            }
        }
        config::Layout::Terminal => {
            let x = left((width / 10).max(24));
            let start = top((height / 3).max(i32::from(big_line) * 2));
            text.draw_left(
                &surface.c, surface.gc, surface.back, surface.depth,
                x, start, &format!("{hostname}:~$ idle"), pal.muted, pal.background,
            )?;
            big.draw_left(
                &surface.c, surface.gc, surface.back, surface.depth,
                x, start + big_line * 2, &core::clock_text(Local::now()),
                pal.foreground, pal.background,
            )?;
            if show_away {
                text.draw_left(
                    &surface.c, surface.gc, surface.back, surface.depth,
                    x, start + big_line * 2 + line * 2, &away,
                    pal.border.max(pal.focus), pal.background,
                )?;
            }
        }
        config::Layout::Runic => {
            let clock_baseline = (surface.height / 2) as i16 - line;
            if let Some(rune) = rune {
                rune.draw_centred(
                    &surface.c, surface.gc, surface.back, surface.depth,
                    centre_x, clock_baseline - line * 3,
                    "ᚠ  ᚢ  ᚦ  ᚨ  ᚱ  ᚲ  ᚷ  ᚹ",
                    pal.border.max(pal.focus), pal.background,
                )?;
            }
            big.draw_centred(
                &surface.c, surface.gc, surface.back, surface.depth,
                centre_x, clock_baseline, &core::clock_text(Local::now()),
                pal.foreground, pal.background,
            )?;
            let host_baseline = clock_baseline + big_line;
            text.draw_centred(
                &surface.c, surface.gc, surface.back, surface.depth,
                centre_x, host_baseline, hostname, pal.muted, pal.background,
            )?;
            if show_away {
                text.draw_centred(
                    &surface.c, surface.gc, surface.back, surface.depth,
                    centre_x, host_baseline + line * 2, &away,
                    pal.border.max(pal.focus), pal.background,
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
                &surface.c, surface.gc, surface.back, surface.depth,
                left_x, top_y, &core::clock_text(Local::now()),
                pal.foreground, pal.background,
            )?;
            text.draw_left(
                &surface.c, surface.gc, surface.back, surface.depth,
                left_x, bottom_y.saturating_sub(line * 2), hostname,
                pal.muted, pal.background,
            )?;
            if show_away {
                let away_width = text.width(&away);
                text.draw_left(
                    &surface.c, surface.gc, surface.back, surface.depth,
                    right_x.saturating_sub(away_width), bottom_y, &away,
                    pal.border.max(pal.focus), pal.background,
                )?;
            }
        }
    }
    surface.present()?;
    Ok(())
}

/// Run the screensaver until a key or button dismisses it.
///
/// Wait for X input or the next visible clock/away deadline, not a polling tick.
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
        .map(|font| core::TextRenderer::new(&font, (cfg.font_size * 2.2).min(cfg.clock_size * 0.6), &setup))
        .transpose()?;
    let hostname = core::hostname();
    let started = Instant::now();
    let mut repaint_at = Instant::now();
    loop {
        let root = surface.c.setup().roots[surface.screen_num].root;
        // Drain x11rb's buffered events before blocking on its socket.
        while let Some(event) = surface.c.poll_for_event()? {
            match event {
                // Any key or click dismisses it. The grabs are released so the
                // session can be used normally afterwards.
                Event::KeyPress(_) | Event::ButtonPress(_) => {
                    surface.release_input()?;
                    return Ok(());
                }
                Event::Expose(_) => {
                    repaint_at = Instant::now();
                }
                // A resolution change or monitor hotplug changes the root
                // geometry; the saver must not leave uncovered pixels.
                // ConfigureNotify can also be a child window; resize()
                // compares the actual root geometry and is cheap.
                Event::RandrScreenChangeNotify(_)
                | Event::RandrNotify(_)
                | Event::ConfigureNotify(_)
                    if surface.resize()? =>
                {
                    repaint_at = Instant::now();
                }
                _ => {}
            }
        }
        if Instant::now() >= repaint_at {
            let idle =
                idle_via_extension(&surface.c, root).unwrap_or_else(|| started.elapsed().as_secs());
            draw(&mut surface, &cfg, &mut text, &mut big, rune.as_mut(), &hostname, idle)?;
            let ms = next_redraw_ms(
                Local::now().timestamp_millis(),
                idle,
                cfg.show_away_after_seconds,
            );
            repaint_at = Instant::now() + Duration::from_millis(ms);
        }
        surface.c.flush()?;
        let timeout = repaint_at
            .saturating_duration_since(Instant::now())
            .as_millis()
            .min(60_000) as i32;
        wait_readable(
            std::os::fd::AsRawFd::as_raw_fd(surface.c.stream()),
            timeout.max(1),
        );
    }
}

fn next_redraw_ms(wall_ms: i64, idle: u64, show_after: u32) -> u64 {
    let clock = (60_000 - wall_ms.rem_euclid(60_000)) as u64;
    let away = if idle < u64::from(show_after) {
        (u64::from(show_after) - idle) * 1000
    } else {
        (60 - idle % 60) * 1000
    };
    clock.min(away).max(1)
}

/// Block until the X socket is readable or the timeout expires.
fn wait_readable(fd: std::os::fd::RawFd, timeout_ms: i32) -> bool {
    #[repr(C)]
    struct PollFd {
        fd: std::os::fd::RawFd,
        events: i16,
        revents: i16,
    }
    extern "C" {
        fn poll(fds: *mut PollFd, nfds: u64, timeout: i32) -> i32;
    }
    let mut pfd = PollFd {
        fd,
        events: 1, // POLLIN
        revents: 0,
    };
    // SAFETY: a single valid pollfd for the duration of the call.
    unsafe { poll(&mut pfd, 1, timeout_ms) > 0 }
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
