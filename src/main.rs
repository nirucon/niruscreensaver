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
use std::time::Duration;
use x11rb::{
    connection::{Connection, RequestConnection as _},
    protocol::{screensaver::ConnectionExt as _, xproto::*, Event},
};

const TICK: Duration = Duration::from_secs(1);

/// Idle time in seconds, read from the X server's MIT-SCREEN-SAVER extension
/// when available, otherwise from the time since the program started.
///
/// MIT-SCREEN-SAVER is the only portable way to ask X11 how long the user has
/// been idle without polling input devices.
fn idle_seconds(c: &x11rb::rust_connection::RustConnection, root: Window) -> u64 {
    idle_via_extension(c, root).unwrap_or(0)
}

fn idle_via_extension(
    c: &x11rb::rust_connection::RustConnection,
    root: Window,
) -> Option<u64> {
    let ext = c
        .extension_information(x11rb::protocol::screensaver::X11_EXTENSION_NAME)
        .ok()??;
    let _ = ext;
    let info = c.screensaver_query_info(root).ok()?.reply().ok()?;
    Some(u64::from(info.ms_since_user_input) / 1000)
}

fn draw(surface: &mut core::Surface, cfg: &config::Config) -> Result<()> {
    let font = core::find_font(cfg.font_path.as_deref());
    if font.is_empty() {
        return Err("no monospace font found; set font_path in the config".into());
    }
    let setup = surface.c.setup().clone();
    let mut text = core::TextRenderer::new(&font, cfg.font_size, &setup)?;
    let pal = &cfg.palette;

    surface.clear(pal.background)?;

    let centre_x = (surface.width / 2) as i16;
    let line = text.line_height();
    let clock_baseline = (surface.height / 2) as i16 - line;
    let big_size = core::TextRenderer::new(&font, cfg.clock_size, &setup)?;
    let big_line = big_size.line_height();
    let mut big = big_size;

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

    // Hostname under the clock.
    let host_baseline = clock_baseline + big_line;
    text.draw_centred(
        &surface.c,
        surface.gc,
        surface.back,
        surface.depth,
        centre_x,
        host_baseline,
        &core::hostname(),
        pal.muted,
        pal.background,
    )?;

    // Idle counter, only when the screen has been idle for a while.
    let idle = idle_seconds(&surface.c, surface.c.setup().roots[surface.screen_num].root);
    if idle >= u64::from(cfg.show_away_after_seconds) {
        text.draw_centred(
            &surface.c,
            surface.gc,
            surface.back,
            surface.depth,
            centre_x,
            host_baseline + line * 2,
            &core::away_text(idle),
            pal.border.max(pal.focus),
            pal.background,
        )?;
    }

    surface.present()?;
    Ok(())
}

/// Run the screensaver until a key or button dismisses it.
///
/// The loop blocks on the X socket with a one second timeout: the clock stays
/// correct, and an idle screensaver uses no CPU.
pub fn run(cfg: config::Config) -> Result<()> {
    let mut surface = core::Surface::new("NIRUSCREENSAVER")?;
    let mut last_minute = String::new();
    loop {
        let minute = Local::now().format("%Y-%m-%d %H:%M").to_string();
        if minute != last_minute {
            draw(&mut surface, &cfg)?;
            last_minute = minute;
        }
        surface.c.flush()?;

        // One second timeout so a new minute is drawn promptly.
        let ready = wait_readable(
            std::os::fd::AsRawFd::as_raw_fd(surface.c.stream()),
            TICK.as_millis() as i32,
        );
        if !ready {
            continue;
        }
        while let Some(event) = surface.c.poll_for_event()? {
            match event {
                Event::KeyPress(_) | Event::ButtonPress(_) => return Ok(()),
                Event::Expose(_) => {
                    last_minute.clear();
                }
                _ => {}
            }
        }
    }
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
