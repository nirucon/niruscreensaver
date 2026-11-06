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

mod audio;
mod cava;
mod config;
mod core;
mod media;
mod palette;
mod quotes;
mod visual;

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
    visual: Option<&mut visual::Visual>,
    media: Option<&str>,
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
            if let Some(visual) = visual {
                visual.paint(surface, pal, (spec_x, spec_y, spec_w, spec_h))?;
            }
            if let Some(media) = media {
                let media = fit(text, media, i32::from(spec_w));
                text.draw_centred(
                    &surface.c,
                    surface.gc,
                    surface.back,
                    surface.depth,
                    centre_x,
                    spec_y.saturating_add(spec_h as i16).saturating_add(line),
                    &media,
                    pal.muted,
                    pal.background,
                )?;
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

/// Shorten `value` with an ellipsis until it is at most `width` pixels wide.
fn fit(text: &core::TextRenderer, value: &str, width: i32) -> String {
    if i32::from(text.width(value)) <= width {
        return value.to_string();
    }
    let mut chars: Vec<char> = value.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let candidate: String = chars.iter().collect::<String>().trim_end().to_string() + "…";
        if i32::from(text.width(&candidate)) <= width {
            return candidate;
        }
    }
    String::new()
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

/// Repaint only the CAVA panel (spectrum or quote) between full frames.
fn draw_cava_frame(
    surface: &core::Surface,
    cfg: &config::Config,
    visual: &mut Option<visual::Visual>,
) -> Result<()> {
    if cfg.layout != config::Layout::Cava {
        return Ok(());
    }
    let Some(visual) = visual.as_mut() else {
        return Ok(());
    };
    let (x, y, width, height) = cava_rect(surface);
    visual.paint(surface, &cfg.palette, (x, y, width, height))?;
    surface.present_rect(x, y, width, height)?;
    Ok(())
}

/// Run the screensaver until a key or button dismisses it.
///
/// Wait for X input, the CAVA silence wake-up, the panel's detector/loader
/// wake-up or the next visible clock/away/quote deadline, never a polling
/// tick. The panel is repainted at 30 fps only while the display is on and
/// the spectrum has sound (see `cava::PowerGate`) or a fade runs.
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
    // The quote collections and the audio detector are read on their own threads.
    let mut visual =
        (cfg.layout == config::Layout::Cava && cfg.visual != visual::Mode::None).then(|| {
            visual::Visual::new(
                cfg.visual,
                !cfg.reduced_motion,
                &text,
                cfg.font_size,
                "niruscreensaver",
                &cfg.quotes,
            )
        });
    // The display power is only needed to pause the panel.
    let mut power = visual.as_ref().map(|_| cava::DisplayPower::new(&surface.c));
    let trace = std::env::var_os("NIRUSCREENSAVER_TRACE").is_some();
    if let (Some(power), Some(visual)) = (power.as_ref(), visual.as_mut()) {
        if trace {
            eprintln!("niruscreensaver: display power: {}", power.mode());
        }
        visual.set_display(power.on());
    }
    // Where the pointer was when the saver appeared, for dismiss_on_motion.
    let origin = cfg
        .dismiss_on_motion
        .then(|| {
            let root = surface.c.setup().roots[surface.screen_num].root;
            surface.c.query_pointer(root).ok()?.reply().ok()
        })
        .flatten()
        .map(|p| (p.root_x, p.root_y));
    // The playing track under the spectrum, followed only while it is live.
    let mut media = (visual.is_some() && cfg.show_media).then(media::Media::new);
    let started = Instant::now();
    // The first frame fades in from black unless reduced motion is asked for.
    let mut fade = (!cfg.reduced_motion).then(Instant::now);
    let mut faded = cfg.clone();
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
                // Off by default: a bumped desk must not wake the screen.
                Event::MotionNotify(e)
                    if origin.is_some_and(|origin| moved_far(origin, (e.root_x, e.root_y))) =>
                {
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
            // Off stops and reaps CAVA and pauses the detector; one static
            // frame stays. On restarts them.
            if let Some(visual) = visual.as_mut() {
                visual.set_display(on);
            }
            next_full = Instant::now();
        }
        let now = Instant::now();
        // The panel follows the audio detector, the quotes loader and its own
        // fade and rotation timers; this reads atomics only.
        let mut panel_dirty = false;
        if let Some(visual) = visual.as_mut() {
            visual.tick(now);
            panel_dirty = visual.take_repaint();
        }
        if now >= next_full {
            let idle =
                idle_via_extension(&surface.c, root).unwrap_or_else(|| started.elapsed().as_secs());
            let track = media.as_ref().and_then(media::Media::line);
            let level = fade.map_or(1.0, |start| {
                fade_level(now.saturating_duration_since(start))
            });
            if level >= 1.0 {
                fade = None;
            }
            faded.palette = dimmed(&cfg.palette, level);
            draw(
                &mut surface,
                &faded,
                &mut text,
                &mut big,
                rune.as_mut(),
                &hostname,
                idle,
                visual.as_mut(),
                track.as_deref(),
            )?;
            let ms = next_redraw_ms(
                Local::now().timestamp_millis(),
                idle,
                cfg.show_away_after_seconds,
            );
            next_full = now + Duration::from_millis(ms);
            if fade.is_some() {
                next_full = next_full.min(now + FADE_STEP);
            }
            next_frame = now + FRAME;
        } else if fade.is_none()
            && ((now >= next_frame && visual.as_ref().is_some_and(visual::Visual::animating))
                || panel_dirty)
        {
            // The 30 fps panel path makes no X round trip: the idle time is
            // only needed for the away text of a full frame. The frame that
            // detects two seconds of silence is drawn flat and is the last.
            draw_cava_frame(&surface, &cfg, &mut visual)?;
            // A steady cadence, as in NIRULOCK: the time spent drawing is not
            // added to the interval, and a late frame restarts it.
            next_frame += FRAME;
            if next_frame <= Instant::now() {
                next_frame = Instant::now() + FRAME;
            }
        }
        if let Some(media) = media.as_mut() {
            // Starts or stops playerctl only when the spectrum starts or stops.
            media.set_active(visual.as_ref().is_some_and(visual::Visual::cava_live));
            if media.take_changed() {
                next_full = Instant::now();
            }
        }
        surface.c.flush()?;
        // Round trips above may have queued events in x11rb, away from the fd.
        pending = surface.c.poll_for_event()?;
        if pending.is_some() {
            continue;
        }
        let mut deadline = next_full;
        if visual.as_ref().is_some_and(visual::Visual::animating) {
            deadline = deadline.min(next_frame);
        }
        for due in [
            power.as_ref().and_then(cava::DisplayPower::deadline),
            visual.as_ref().and_then(visual::Visual::deadline),
        ]
        .into_iter()
        .flatten()
        {
            deadline = deadline.min(due);
        }
        let timeout = deadline
            .saturating_duration_since(Instant::now())
            .as_millis()
            .min(60_000) as i32;
        let [cava_wake, visual_wake] = visual
            .as_ref()
            .map_or([None, None], visual::Visual::wake_fds);
        let ready = wait_readable(
            [
                Some(std::os::fd::AsRawFd::as_raw_fd(surface.c.stream())),
                cava_wake,
                visual_wake,
            ],
            timeout.max(1),
        );
        if ready[1] || ready[2] {
            if let Some(visual) = visual.as_mut() {
                visual.woke([ready[1], ready[2]]);
            }
            next_frame = Instant::now();
        }
    }
}

/// How long the first frame takes to fade in from black, and how often it is
/// redrawn meanwhile: ten frames, once.
const FADE: Duration = Duration::from_millis(300);
const FADE_STEP: Duration = Duration::from_millis(30);

/// Brightness `elapsed` into the fade, eased so it neither pops nor lingers.
fn fade_level(elapsed: Duration) -> f32 {
    let t = (elapsed.as_secs_f32() / FADE.as_secs_f32()).clamp(0.0, 1.0);
    if t >= 1.0 {
        return 1.0;
    }
    t * t * (3.0 - 2.0 * t)
}

/// Every palette colour scaled towards black by `level` (1.0 is unchanged).
fn dimmed(palette: &palette::Palette, level: f32) -> palette::Palette {
    let dim = |colour: u32| {
        let channel = |shift: u32| {
            let value = f32::from(((colour >> shift) & 255) as u8);
            ((value * level.clamp(0.0, 1.0)).round() as u32) << shift
        };
        channel(16) | channel(8) | channel(0)
    };
    palette::Palette {
        background: dim(palette.background),
        foreground: dim(palette.foreground),
        muted: dim(palette.muted),
        border: dim(palette.border),
        focus: dim(palette.focus),
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
const FRAME: Duration = Duration::from_micros(33_333);

/// True once the pointer is more than `MOTION_DISMISS_PX` from `origin`.
fn moved_far(origin: (i16, i16), now: (i16, i16)) -> bool {
    let dx = i64::from(now.0) - i64::from(origin.0);
    let dy = i64::from(now.1) - i64::from(origin.1);
    let limit = i64::from(config::MOTION_DISMISS_PX);
    dx * dx + dy * dy > limit * limit
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

/// Wait on X, the CAVA silence wake-up and the panel's detector/loader
/// wake-up without periodic polling. Returns which descriptors became
/// readable.
fn wait_readable(fds: [Option<std::os::fd::RawFd>; 3], timeout_ms: i32) -> [bool; 3] {
    let mut poll = fds.map(|fd| libc::pollfd {
        fd: fd.unwrap_or(-1),
        events: libc::POLLIN,
        revents: 0,
    });
    // SAFETY: three valid pollfd values; poll ignores a negative unused fd.
    let n = unsafe { libc::poll(poll.as_mut_ptr(), poll.len() as libc::nfds_t, timeout_ms) };
    if n <= 0 {
        return [false; 3];
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
    use super::{moved_far, next_redraw_ms};

    #[test]
    fn the_fade_starts_black_and_ends_at_the_theme() {
        use super::{dimmed, fade_level, Duration, FADE};
        assert_eq!(fade_level(Duration::ZERO), 0.0);
        let half = fade_level(FADE / 2);
        assert!(half > 0.4 && half < 0.6);
        assert_eq!(fade_level(FADE), 1.0);
        assert_eq!(fade_level(FADE * 3), 1.0);
        let theme = crate::palette::Palette::default();
        let black = dimmed(&theme, 0.0);
        assert_eq!(
            [black.foreground, black.muted, black.focus, black.background],
            [0; 4]
        );
        let full = dimmed(&theme, 1.0);
        assert_eq!(full.foreground, theme.foreground);
        assert_eq!(full.focus, theme.focus);
        assert_eq!(dimmed(&theme, 0.5).foreground, 0x737373);
    }

    #[test]
    fn motion_dismisses_only_beyond_24_px() {
        assert!(!moved_far((100, 100), (124, 100)));
        assert!(!moved_far((100, 100), (116, 116)));
        assert!(moved_far((100, 100), (125, 100)));
        assert!(moved_far((100, 100), (80, 80)));
        assert!(moved_far((i16::MAX, i16::MAX), (i16::MIN, i16::MIN)));
    }

    #[test]
    fn waits_for_the_first_visible_change() {
        assert_eq!(next_redraw_ms(0, 0, 60), 60_000);
        assert_eq!(next_redraw_ms(1_500, 58, 60), 2_000);
        assert_eq!(next_redraw_ms(59_750, 60, 60), 250);
        assert_eq!(next_redraw_ms(0, 61, 60), 59_000);
    }
}
