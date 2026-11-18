//! The scenes besides the clock: `photos`, `dark`, `dark-sid`, `ascii` and
//! `ascii-sid` (the last two in [`ascii`]).
//!
//! They share the clock's window, grabs, monitors and dismissal (any key or
//! click, the swallowed release, `dismiss_on_motion`) and its power rule:
//! with the display off nothing is drawn and nothing moves. Each scene only
//! draws; [`run`] owns the event loop.
//!
//! - `dark`: black, with one line, "NIRUX - A MINIMAL X11 DESKTOP", gliding
//!   slowly across every monitor and bouncing off its edges, so no pixel
//!   stays lit. With `reduced_motion` it moves once a minute instead.
//! - `dark-sid`: the same, and C64 SID tunes through NIRULOCK's pause-music
//!   player (`crate::music`, a verbatim twin): only when nothing else plays,
//!   from the folder in NIRULOCK's `[pause_music]` (default `~/Music/SID`),
//!   with the tune under the line. It stops when the display goes off and
//!   when the saver goes; NIRUIDLE ends the saver when it locks, and the
//!   player dies with it (`PR_SET_PDEATHSIG`).
//! - `photos`: the pictures in `photos_folder`, each for `photos_seconds`,
//!   drifting slowly and crossfading, with a small clock on a soft shade.

use crate::config::{Config, Scene};
use crate::core::{self, Area, Surface, TextRenderer};
use crate::images::{self, Picture};
use crate::{audio, cava, music, Result};
use chrono::Local;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::xproto::*;
use x11rb::protocol::Event;

mod ascii;

/// The line the dark scenes show.
pub const DARK_LINE: &str = "NIRUX - A MINIMAL X11 DESKTOP";

/// What a scene does; [`run`] calls it.
trait Show {
    /// Draw everything into the back buffer and present it.
    fn redraw(&mut self, s: &mut Surface, now: Instant) -> Result<()>;
    /// The next timed step is due.
    fn step(&mut self, s: &mut Surface, now: Instant) -> Result<()>;
    /// When [`Show::step`] is next due; `None` while nothing moves.
    fn deadline(&self) -> Option<Instant>;
    /// Called once per loop turn; true asks for a full redraw.
    fn tick(&mut self, _now: Instant) -> bool {
        false
    }
    fn display(&mut self, on: bool);
    fn wake_fds(&self) -> [Option<RawFd>; 2] {
        [None, None]
    }
    fn woke(&mut self, _ready: [bool; 2]) {}
}

/// A random-ish number: the clock and the process id, mixed. Enough to
/// vary a start position or a shuffle pick.
pub fn random() -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    let mut x = nanos ^ (u64::from(std::process::id()) << 32) ^ 0x9e37_79b9_7f4a_7c15;
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    x = x.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    x ^ (x >> 33)
}

/// The scene `shuffle` shows this time.
pub fn pick(list: &[Scene], seed: u64) -> Scene {
    if list.is_empty() {
        return Scene::Clock;
    }
    list[(seed % list.len() as u64) as usize]
}

/// The pictures `photos` would show; empty means "show the clock instead".
pub fn photos(cfg: &Config) -> Vec<PathBuf> {
    images::list(&crate::config::expand_home(&cfg.photos_folder))
}

/// Run `scene` (photos, dark, dark-sid, ascii or ascii-sid) until a key or click.
pub fn run(
    cfg: Config,
    coverage: core::Coverage,
    scene: Scene,
    pictures: Vec<PathBuf>,
) -> Result<()> {
    let mut surface = Surface::new("NIRUSCREENSAVER", coverage)?;
    let font = core::find_font(cfg.font_path.as_deref());
    if font.is_empty() {
        return Err("no monospace font found; set font_path in the config".into());
    }
    let setup = surface.c.setup().clone();
    let text = TextRenderer::new(&font, cfg.font_size, &setup)?;
    let mut show: Box<dyn Show> = match scene {
        Scene::Photos => Box::new(Photos::new(&cfg, &text, &surface, pictures)?),
        Scene::Ascii | Scene::AsciiSid => Box::new(ascii::Ascii::new(
            &cfg,
            &text,
            &surface,
            scene == Scene::AsciiSid,
        )),
        _ => Box::new(Dark::new(&cfg, &text, &surface, scene == Scene::DarkSid)),
    };
    let mut power = cava::DisplayPower::new(&surface.c);
    show.display(power.on());
    let origin = cfg
        .dismiss_on_motion
        .then(|| {
            let root = surface.c.setup().roots[surface.screen_num].root;
            surface.c.query_pointer(root).ok()?.reply().ok()
        })
        .flatten()
        .map(|p| (p.root_x, p.root_y));
    let mut full = true;
    let mut pending: Option<Event> = None;
    loop {
        let mut display_change = None;
        let mut check_layout = false;
        while let Some(event) = match pending.take() {
            Some(event) => Some(event),
            None => surface.c.poll_for_event()?,
        } {
            let changed = match &event {
                Event::KeyPress(_) | Event::ButtonPress(_) | Event::MotionNotify(_) => {
                    power.input(&surface.c, Instant::now())
                }
                event => power.event(event),
            };
            display_change = changed.or(display_change);
            match event {
                Event::KeyPress(_) | Event::ButtonPress(_) => {
                    crate::wait_for_release(&surface)?;
                    surface.release_input()?;
                    return Ok(());
                }
                Event::MotionNotify(e)
                    if origin.is_some_and(|o| crate::moved_far(o, (e.root_x, e.root_y))) =>
                {
                    surface.release_input()?;
                    return Ok(());
                }
                Event::Expose(_) => full = true,
                Event::RandrScreenChangeNotify(_)
                | Event::RandrNotify(_)
                | Event::ConfigureNotify(_)
                | Event::MapNotify(_)
                | Event::CirculateNotify(_) => check_layout = true,
                _ => {}
            }
        }
        if check_layout {
            if surface.resize()? {
                full = true;
            }
            surface.keep_on_top()?;
        }
        if let Some(on) = power.poll(&surface.c, Instant::now()) {
            display_change = Some(on);
        }
        if let Some(on) = display_change {
            show.display(on);
            full = true;
        }
        let now = Instant::now();
        if show.tick(now) {
            full = true;
        }
        if full {
            show.redraw(&mut surface, now)?;
            full = false;
        } else if show.deadline().is_some_and(|due| now >= due) {
            show.step(&mut surface, now)?;
        }
        surface.c.flush()?;
        pending = surface.c.poll_for_event()?;
        if pending.is_some() {
            continue;
        }
        let mut deadline = now + Duration::from_secs(60);
        for due in [show.deadline(), power.deadline()].into_iter().flatten() {
            deadline = deadline.min(due);
        }
        let timeout = deadline
            .saturating_duration_since(Instant::now())
            .as_micros()
            .div_ceil(1000)
            .min(60_000) as i32;
        let [a, b] = show.wake_fds();
        let ready =
            crate::wait_readable([Some(surface.c.stream().as_raw_fd()), a, b], timeout.max(1));
        if ready[1] || ready[2] {
            show.woke([ready[1], ready[2]]);
        }
    }
}

/// The 300 ms fade-in level at `now`, or 1.0 with `reduced_motion`.
fn fade(start: Option<Instant>, now: Instant) -> f32 {
    start.map_or(1.0, |s| crate::fade_level(now.saturating_duration_since(s)))
}

// ---- dark and dark-sid ----------------------------------------------------------

/// Gliding speed in pixels per second.
const GLIDE: f32 = 26.0;
/// Black border around the sprite: a step of at most this many pixels
/// erases its own trail.
const MARGIN: i32 = 2;
/// With `reduced_motion` the line jumps to a new place this often.
const JUMP: Duration = Duration::from_secs(60);

/// The line's position on one monitor.
struct Walker {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    /// Where the sprite was last drawn, window coordinates.
    drawn: Option<(i32, i32)>,
}

struct Sprite {
    pixmap: Pixmap,
    width: u16,
    height: u16,
}

struct Dark {
    cfg: Config,
    title: TextRenderer,
    info: TextRenderer,
    /// The note glyph, when the font has one.
    note: bool,
    sprite: Option<Sprite>,
    walkers: Vec<Walker>,
    areas: Vec<Area>,
    seed: u64,
    on: bool,
    last: Instant,
    next: Option<Instant>,
    fade: Option<Instant>,
    music: Option<music::Music>,
    listen: Option<audio::Audio>,
    stop_on_display_off: bool,
    tune: Option<String>,
}

/// NIRULOCK's `[pause_music]` settings from `lock.toml`, so the folder is set
/// in one place. `enabled` and `on` are NIRULOCK's own: choosing `dark-sid`
/// is the choice to play.
fn lock_music_settings() -> music::Settings {
    let path = crate::config::config_file().with_file_name("lock.toml");
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| text.parse::<toml::Value>().ok())
        .and_then(|value| value.get("pause_music").cloned())
        .and_then(|table| music::Settings::from_value(&table).ok())
        .unwrap_or_default()
}

/// The tune line without NIRULOCK's "Esc stops the music" (any key ends
/// the saver) and without the note when the font cannot draw it.
fn tune_line(info: &str, note: bool) -> String {
    let line = info
        .trim_end_matches("  ·  Esc stops the music")
        .to_string();
    if note {
        line
    } else {
        line.trim_start_matches("♪ ").to_string()
    }
}

impl Dark {
    fn new(cfg: &Config, text: &TextRenderer, surface: &Surface, sid: bool) -> Self {
        let title = text.at_size((cfg.font_size * 1.25).clamp(12.0, 120.0));
        let info = text.at_size((cfg.font_size * 0.8).max(8.0));
        let note = info.has_glyph('♪');
        let settings = lock_music_settings();
        let music = sid.then(|| music::Music::start(&settings)).flatten();
        let listen = music
            .is_some()
            .then(|| audio::Audio::start("niruscreensaver", None));
        let now = Instant::now();
        let mut dark = Self {
            cfg: cfg.clone(),
            title,
            info,
            note,
            sprite: None,
            walkers: Vec::new(),
            areas: Vec::new(),
            seed: random(),
            on: true,
            last: now,
            next: Some(now),
            fade: (!cfg.reduced_motion).then_some(now),
            music,
            listen,
            stop_on_display_off: settings.stop_on_display_off,
            tune: None,
        };
        dark.place(&surface.areas);
        dark
    }

    /// A start position and direction on every monitor.
    fn place(&mut self, areas: &[Area]) {
        self.areas = areas.to_vec();
        self.walkers = areas
            .iter()
            .enumerate()
            .map(|(i, area)| {
                let r = self.seed.rotate_left(i as u32 * 17);
                let fx = (r & 0xffff) as f32 / 65535.0;
                let fy = ((r >> 16) & 0xffff) as f32 / 65535.0;
                // 25 to 65 degrees, so it never runs along an edge.
                let angle = (25.0 + ((r >> 32) % 40) as f32).to_radians();
                let sx = if r & (1 << 60) == 0 { 1.0 } else { -1.0 };
                let sy = if r & (1 << 61) == 0 { 1.0 } else { -1.0 };
                Walker {
                    x: f32::from(area.x) + fx * f32::from(area.width) * 0.6,
                    y: f32::from(area.y) + fy * f32::from(area.height) * 0.8,
                    vx: sx * GLIDE * angle.cos(),
                    vy: sy * GLIDE * angle.sin(),
                    drawn: None,
                }
            })
            .collect();
    }

    /// Render the line (and the tune) into the sprite, black around it.
    fn render(&mut self, s: &Surface, level: f32) -> Result<()> {
        let pal = crate::dimmed(&self.cfg.palette, level);
        let tune = self.tune.clone();
        let title_w = i32::from(self.title.width(DARK_LINE));
        let tune_w = tune.as_deref().map_or(0, |t| i32::from(self.info.width(t)));
        let gap = i32::from(self.info.line_height()) / 2;
        let title_h = i32::from(self.title.ascent() + self.title.descent());
        let tune_h = if tune.is_some() {
            gap + i32::from(self.info.ascent() + self.info.descent())
        } else {
            0
        };
        let width = (title_w.max(tune_w) + 2 * MARGIN).clamp(1, i32::from(u16::MAX)) as u16;
        let height = (title_h + tune_h + 2 * MARGIN).clamp(1, i32::from(u16::MAX)) as u16;
        if let Some(old) = self.sprite.take() {
            s.c.free_pixmap(old.pixmap)?;
        }
        let pixmap = s.c.generate_id()?;
        let root = s.c.setup().roots[s.screen_num].root;
        s.c.create_pixmap(s.depth, pixmap, root, width, height)?;
        s.c.change_gc(
            s.gc,
            &ChangeGCAux::new().foreground(self.cfg.palette.background),
        )?;
        s.c.poly_fill_rectangle(
            pixmap,
            s.gc,
            &[Rectangle {
                x: 0,
                y: 0,
                width,
                height,
            }],
        )?;
        let centre = (i32::from(width) / 2) as i16;
        let baseline = (MARGIN + i32::from(self.title.ascent())) as i16;
        self.title.draw_centred(
            &s.c,
            s.gc,
            pixmap,
            s.depth,
            centre,
            baseline,
            DARK_LINE,
            pal.foreground,
            self.cfg.palette.background,
        )?;
        if let Some(tune) = tune.as_deref() {
            let info_baseline = baseline + self.title.descent() + gap as i16 + self.info.ascent();
            self.info.draw_centred(
                &s.c,
                s.gc,
                pixmap,
                s.depth,
                centre,
                info_baseline,
                tune,
                pal.muted,
                self.cfg.palette.background,
            )?;
        }
        self.sprite = Some(Sprite {
            pixmap,
            width,
            height,
        });
        Ok(())
    }

    /// Keep every walker inside its monitor, bouncing off the edges.
    fn bounce(walker: &mut Walker, area: &Area, w: f32, h: f32) {
        let (x0, y0) = (f32::from(area.x), f32::from(area.y));
        let x1 = (x0 + f32::from(area.width) - w).max(x0);
        let y1 = (y0 + f32::from(area.height) - h).max(y0);
        if walker.x <= x0 {
            walker.x = x0;
            walker.vx = walker.vx.abs();
        } else if walker.x >= x1 {
            walker.x = x1;
            walker.vx = -walker.vx.abs();
        }
        if walker.y <= y0 {
            walker.y = y0;
            walker.vy = walker.vy.abs();
        } else if walker.y >= y1 {
            walker.y = y1;
            walker.vy = -walker.vy.abs();
        }
    }

    /// One pixel along the faster axis.
    fn step_time(&self) -> Duration {
        let fast = self
            .walkers
            .first()
            .map_or(GLIDE, |w| w.vx.abs().max(w.vy.abs()));
        Duration::from_secs_f32(1.0 / fast.max(1.0))
    }

    fn schedule(&mut self, now: Instant) {
        self.next = if !self.on {
            None
        } else if self.fade.is_some() {
            Some(now + Duration::from_millis(30))
        } else if self.cfg.reduced_motion {
            Some(now + JUMP)
        } else {
            Some(now + self.step_time())
        };
    }

    /// Draw the sprite at every walker into the back buffer; `present` the
    /// touched rectangles.
    fn paint(&mut self, s: &Surface, present: bool) -> Result<()> {
        let Some(sprite) = self.sprite.as_ref() else {
            return Ok(());
        };
        let (w, h) = (i32::from(sprite.width), i32::from(sprite.height));
        for walker in &mut self.walkers {
            let at = (walker.x.round() as i32, walker.y.round() as i32);
            if walker.drawn == Some(at) {
                continue;
            }
            let clamp = |v: i32| v.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
            if let Some(old) = walker.drawn {
                // A step wider than the margin would leave a trail.
                if (old.0 - at.0).abs() > MARGIN || (old.1 - at.1).abs() > MARGIN {
                    s.c.change_gc(
                        s.gc,
                        &ChangeGCAux::new().foreground(self.cfg.palette.background),
                    )?;
                    s.c.poly_fill_rectangle(
                        s.back,
                        s.gc,
                        &[Rectangle {
                            x: clamp(old.0),
                            y: clamp(old.1),
                            width: sprite.width,
                            height: sprite.height,
                        }],
                    )?;
                    if present {
                        s.present_rect(clamp(old.0), clamp(old.1), sprite.width, sprite.height)?;
                    }
                }
            }
            s.c.copy_area(
                sprite.pixmap,
                s.back,
                s.gc,
                0,
                0,
                clamp(at.0),
                clamp(at.1),
                sprite.width,
                sprite.height,
            )?;
            if present {
                let (x0, y0) = walker.drawn.map_or(at, |o| (o.0.min(at.0), o.1.min(at.1)));
                let x1 = walker.drawn.map_or(at.0, |o| o.0.max(at.0)) + w;
                let y1 = walker.drawn.map_or(at.1, |o| o.1.max(at.1)) + h;
                s.present_rect(
                    clamp(x0),
                    clamp(y0),
                    (x1 - x0).clamp(1, i32::from(u16::MAX)) as u16,
                    (y1 - y0).clamp(1, i32::from(u16::MAX)) as u16,
                )?;
            }
            walker.drawn = Some(at);
        }
        Ok(())
    }
}

impl Show for Dark {
    fn redraw(&mut self, s: &mut Surface, now: Instant) -> Result<()> {
        if s.areas != self.areas {
            self.place(&s.areas.clone());
        }
        let level = fade(self.fade, now);
        self.render(s, level)?;
        let (w, h) = self
            .sprite
            .as_ref()
            .map_or((0.0, 0.0), |sp| (f32::from(sp.width), f32::from(sp.height)));
        for (walker, area) in self.walkers.iter_mut().zip(&self.areas) {
            Self::bounce(walker, area, w, h);
            walker.drawn = None;
        }
        s.clear(self.cfg.palette.background)?;
        self.paint(s, false)?;
        s.present()?;
        self.last = now;
        self.schedule(now);
        Ok(())
    }

    fn step(&mut self, s: &mut Surface, now: Instant) -> Result<()> {
        if let Some(start) = self.fade {
            let level = fade(Some(start), now);
            if level >= 1.0 {
                self.fade = None;
            }
            // Same size, new colours: repaint in place.
            self.render(s, level)?;
            for walker in &mut self.walkers {
                walker.drawn = None;
            }
            self.paint(s, true)?;
            self.last = now;
            self.schedule(now);
            return Ok(());
        }
        let (w, h) = self
            .sprite
            .as_ref()
            .map_or((0.0, 0.0), |sp| (f32::from(sp.width), f32::from(sp.height)));
        if self.cfg.reduced_motion {
            self.seed = random();
            let areas = self.areas.clone();
            self.place(&areas);
            for (walker, area) in self.walkers.iter_mut().zip(&areas) {
                Self::bounce(walker, area, w, h);
            }
            self.last = now;
            self.schedule(now);
            return self.redraw_after_jump(s, now);
        }
        // At most a few pixels per step, even after a stall, so the margin
        // keeps erasing the trail in the common case.
        let dt = now
            .saturating_duration_since(self.last)
            .as_secs_f32()
            .min(0.5);
        for (walker, area) in self.walkers.iter_mut().zip(&self.areas) {
            walker.x += walker.vx * dt;
            walker.y += walker.vy * dt;
            Self::bounce(walker, area, w, h);
        }
        self.paint(s, true)?;
        self.last = now;
        self.schedule(now);
        Ok(())
    }

    fn deadline(&self) -> Option<Instant> {
        let music = self.music.as_ref().and_then(music::Music::deadline);
        match (self.next, music) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    fn tick(&mut self, _now: Instant) -> bool {
        let Some(music) = self.music.as_mut() else {
            return false;
        };
        let other = self
            .listen
            .as_ref()
            .map_or(audio::Heard::Silent, audio::Audio::other_heard);
        let display_on = !self.stop_on_display_off || self.on;
        match music.tick(other, display_on, None) {
            music::Tick::Nothing => {}
            music::Tick::Started | music::Tick::Skipped(_) => self.listen = None,
        }
        let tune = music.info().map(|info| tune_line(&info, self.note));
        if tune != self.tune {
            self.tune = tune;
            return true;
        }
        false
    }

    fn display(&mut self, on: bool) {
        self.on = on;
        if !on && self.stop_on_display_off {
            if let Some(music) = self.music.as_mut() {
                music.stop();
            }
        }
        if !on {
            self.next = None;
        }
    }

    fn wake_fds(&self) -> [Option<RawFd>; 2] {
        [self.music.as_ref().and_then(music::Music::wake_fd), None]
    }

    fn woke(&mut self, ready: [bool; 2]) {
        if ready[0] {
            if let Some(music) = self.music.as_mut() {
                music.woke();
            }
        }
    }
}

impl Dark {
    fn redraw_after_jump(&mut self, s: &mut Surface, now: Instant) -> Result<()> {
        for walker in &mut self.walkers {
            walker.drawn = None;
        }
        s.clear(self.cfg.palette.background)?;
        self.paint(s, false)?;
        s.present()?;
        self.schedule(now);
        Ok(())
    }
}

// ---- photos -----------------------------------------------------------------------

/// How much larger than the screen a picture is scaled, the room it drifts in.
const DRIFT: f32 = 0.06;
/// The crossfade: this many frames over this long.
const CROSS_STEPS: u32 = 10;
const CROSS: Duration = Duration::from_millis(1600);

/// One picture scaled for each monitor size.
struct Slide {
    frames: Vec<((u16, u16), Picture)>,
    /// Drift direction: which corner it starts in.
    corner: u8,
}

impl Slide {
    fn frame(&self, size: (u16, u16)) -> Option<&Picture> {
        self.frames.iter().find(|(s, _)| *s == size).map(|(_, p)| p)
    }
}

/// A picture on the screen: the scaled frames as pixmaps, and the drift.
struct Shown {
    slide: Slide,
    pixmaps: Vec<((u16, u16), Pixmap)>,
    since: Instant,
}

enum Loaded {
    Slide(Slide),
    Failed(PathBuf, String),
}

struct Loader {
    send: mpsc::Sender<(PathBuf, Vec<(u16, u16)>)>,
    receive: mpsc::Receiver<Loaded>,
    wake: UnixStream,
    busy: bool,
}

impl Loader {
    fn start(drift: f32) -> Result<Self> {
        let (wake, notify) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        let (send, jobs) = mpsc::channel::<(PathBuf, Vec<(u16, u16)>)>();
        let (done, receive) = mpsc::channel();
        std::thread::Builder::new()
            .name("niruscreensaver-photos".into())
            .spawn(move || {
                for (path, sizes) in jobs {
                    let want = sizes.iter().fold((0, 0), |m, &(w, h)| {
                        let (sw, sh) = scaled(w, h, drift);
                        (m.0.max(sw), m.1.max(sh))
                    });
                    let result = images::decode(&path, want).map(|picture| Slide {
                        corner: (random() & 3) as u8,
                        frames: sizes
                            .iter()
                            .map(|&(w, h)| {
                                let (sw, sh) = scaled(w, h, drift);
                                ((w, h), images::cover(&picture, sw, sh))
                            })
                            .collect(),
                    });
                    let message = match result {
                        Ok(slide) => Loaded::Slide(slide),
                        Err(e) => Loaded::Failed(path, e),
                    };
                    if done.send(message).is_err() {
                        return;
                    }
                    let _ = std::io::Write::write(&mut &notify, &[1]);
                }
            })?;
        Ok(Self {
            send,
            receive,
            wake,
            busy: false,
        })
    }

    fn load(&mut self, path: PathBuf, sizes: Vec<(u16, u16)>) {
        if self.send.send((path, sizes)).is_ok() {
            self.busy = true;
        }
    }
}

/// The size a picture is scaled to for a `w` x `h` monitor.
fn scaled(w: u16, h: u16, drift: f32) -> (usize, usize) {
    let grow = |v: u16| ((f32::from(v) * (1.0 + drift)).round() as usize).max(usize::from(v));
    (grow(w), grow(h))
}

/// Where the monitor-sized window into a `frame` sits after `t` (0 to 1)
/// of the slide's time, drifting from `corner` to the opposite one.
fn drift_offset(frame: (usize, usize), area: (usize, usize), corner: u8, t: f32) -> (usize, usize) {
    let room_x = frame.0.saturating_sub(area.0) as f32;
    let room_y = frame.1.saturating_sub(area.1) as f32;
    let t = t.clamp(0.0, 1.0);
    let along = |room: f32, reverse: bool| {
        let v = if reverse { room * (1.0 - t) } else { room * t };
        v.round() as usize
    };
    (
        along(room_x, corner & 1 == 1),
        along(room_y, corner & 2 == 2),
    )
}

/// The small clock of the slide show, drawn over the picture in memory.
struct Label {
    clock: TextRenderer,
    date: TextRenderer,
    foreground: u32,
    muted: u32,
}

impl Label {
    /// The clock and date text now.
    fn texts() -> (String, String) {
        let now = Local::now();
        (
            now.format("%H:%M").to_string(),
            now.format("%A %-d %B").to_string(),
        )
    }

    fn pad(&self) -> usize {
        (self.date.line_height().max(0) as usize).max(8)
    }

    /// The box in area coordinates: bottom left, above the margin.
    fn rect(&self, area: &Area) -> (usize, usize, usize, usize) {
        let (clock, date) = Self::texts();
        let pad = self.pad();
        let text_w =
            usize::from(self.clock.width(&clock).max(self.date.width(&date)).max(0) as u16);
        let text_h = (self.clock.line_height().max(0) + self.date.line_height().max(0)) as usize;
        let (aw, ah) = (usize::from(area.width), usize::from(area.height));
        let w = (text_w + pad * 3).min(aw);
        let h = (text_h + pad * 2).min(ah);
        let margin = (ah / 18).max(16);
        (margin.min(aw - w), ah.saturating_sub(h + margin), w, h)
    }

    /// Shade `rect` of `pixels` (`width` wide) softly and write the text in it.
    fn draw(&mut self, pixels: &mut [u32], width: usize, rect: (usize, usize, usize, usize)) {
        let (clock, date) = Self::texts();
        images::shade(pixels, width, rect, 0.55, rect.3 / 3);
        let pad = self.pad() as i32;
        let left = rect.0 as i32 + pad + pad / 2;
        let base = rect.1 as i32 + pad + i32::from(self.clock.ascent());
        self.clock
            .composite(pixels, width, left, base, &clock, self.foreground);
        let date_base = base + i32::from(self.clock.descent()) + i32::from(self.date.ascent());
        self.date
            .composite(pixels, width, left, date_base, &date, self.muted);
    }
}

struct Photos {
    cfg: Config,
    label: Label,
    pictures: Vec<PathBuf>,
    index: usize,
    loader: Loader,
    shown: Option<Shown>,
    /// The next picture, loaded and waiting for its turn.
    ready: Option<Slide>,
    /// A crossfade from the shown picture (or black) to `ready`.
    cross: Option<(Instant, Option<Shown>)>,
    areas: Vec<Area>,
    on: bool,
    next: Option<Instant>,
    seconds: f32,
    /// The last drawn drift offsets and minute, to skip identical frames.
    drawn: Vec<(usize, usize)>,
    minute: i64,
    byte_order: ImageOrder,
    max_request: usize,
}

impl Photos {
    fn new(
        cfg: &Config,
        text: &TextRenderer,
        surface: &Surface,
        mut pictures: Vec<PathBuf>,
    ) -> Result<Self> {
        // A new order every time, rotated rather than shuffled: cheap, and
        // each picture still comes once per round.
        let mut seed = random();
        for i in (1..pictures.len()).rev() {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            pictures.swap(i, (seed >> 33) as usize % (i + 1));
        }
        let drift = if cfg.reduced_motion { 0.0 } else { DRIFT };
        let mut photos = Self {
            cfg: cfg.clone(),
            label: Label {
                clock: text.at_size((cfg.font_size * 2.2).clamp(16.0, 200.0)),
                date: text.at_size((cfg.font_size * 0.8).max(8.0)),
                foreground: cfg.palette.foreground,
                muted: cfg.palette.muted,
            },
            pictures,
            index: 0,
            loader: Loader::start(drift)?,
            shown: None,
            ready: None,
            cross: None,
            areas: surface.areas.clone(),
            on: true,
            next: None,
            seconds: cfg.photos_seconds as f32,
            drawn: Vec::new(),
            minute: -1,
            byte_order: surface.c.setup().image_byte_order,
            max_request: surface.c.maximum_request_bytes(),
        };
        photos.request_next();
        Ok(photos)
    }

    fn sizes(&self) -> Vec<(u16, u16)> {
        let mut sizes: Vec<(u16, u16)> = Vec::new();
        for a in &self.areas {
            if !sizes.contains(&(a.width, a.height)) {
                sizes.push((a.width, a.height));
            }
        }
        sizes
    }

    fn request_next(&mut self) {
        if self.loader.busy || self.ready.is_some() || self.pictures.is_empty() {
            return;
        }
        let path = self.pictures[self.index % self.pictures.len()].clone();
        self.index = (self.index + 1) % self.pictures.len().max(1);
        let sizes = self.sizes();
        self.loader.load(path, sizes);
    }

    /// Upload one frame into a new pixmap, in strips that fit a request.
    fn upload(&self, s: &Surface, picture: &Picture) -> Result<Pixmap> {
        let pixmap = s.c.generate_id()?;
        let root = s.c.setup().roots[s.screen_num].root;
        let (w, h) = (picture.width as u16, picture.height as u16);
        s.c.create_pixmap(s.depth, pixmap, root, w, h)?;
        self.put(s, pixmap, &picture.pixels, picture.width, 0, 0)?;
        Ok(pixmap)
    }

    /// `pixels` (`width` wide) into `target` at (`x`, `y`).
    fn put(
        &self,
        s: &Surface,
        target: Drawable,
        pixels: &[u32],
        width: usize,
        x: i16,
        y: i16,
    ) -> Result<()> {
        if width == 0 {
            return Ok(());
        }
        let rows = pixels.len() / width;
        let per = ((self.max_request.saturating_sub(64)) / (width * 4)).clamp(1, rows.max(1));
        let mut row = 0;
        while row < rows {
            let n = per.min(rows - row);
            let bytes: Vec<u8> = pixels[row * width..(row + n) * width]
                .iter()
                .flat_map(|p| {
                    if self.byte_order == ImageOrder::MSB_FIRST {
                        p.to_be_bytes()
                    } else {
                        p.to_le_bytes()
                    }
                })
                .collect();
            s.c.put_image(
                ImageFormat::Z_PIXMAP,
                target,
                s.gc,
                width as u16,
                n as u16,
                x,
                y.saturating_add(row as i16),
                0,
                s.depth,
                &bytes,
            )?;
            row += n;
        }
        Ok(())
    }

    fn show(&self, s: &Surface, slide: Slide, now: Instant) -> Result<Shown> {
        let mut pixmaps = Vec::new();
        for (size, picture) in &slide.frames {
            pixmaps.push((*size, self.upload(s, picture)?));
        }
        Ok(Shown {
            slide,
            pixmaps,
            since: now,
        })
    }

    fn free(&self, s: &Surface, shown: Shown) -> Result<()> {
        for (_, pixmap) in shown.pixmaps {
            s.c.free_pixmap(pixmap)?;
        }
        Ok(())
    }

    /// The drift offset of `shown` on `area` at `now`.
    fn offset(&self, shown: &Shown, area: &Area, now: Instant) -> Option<(usize, usize)> {
        let frame = shown.slide.frame((area.width, area.height))?;
        let t = now.saturating_duration_since(shown.since).as_secs_f32() / self.seconds;
        Some(drift_offset(
            (frame.width, frame.height),
            (usize::from(area.width), usize::from(area.height)),
            shown.slide.corner,
            t,
        ))
    }

    /// The crop of `picture` at `offset`, area-sized.
    fn crop(picture: &Picture, offset: (usize, usize), area: &Area) -> Vec<u32> {
        let (w, h) = (usize::from(area.width), usize::from(area.height));
        let mut out = vec![0u32; w * h];
        for y in 0..h.min(picture.height.saturating_sub(offset.1)) {
            let src = &picture.pixels[(offset.1 + y) * picture.width + offset.0..];
            let n = w.min(picture.width.saturating_sub(offset.0));
            out[y * w..y * w + n].copy_from_slice(&src[..n]);
        }
        out
    }

    /// One frame of every area. `level` is the crossfade (0 old, 1 new).
    fn compose(&mut self, s: &Surface, now: Instant) -> Result<()> {
        let areas = self.areas.clone();
        let cross = self.cross.as_ref().map(|(start, _)| {
            let t = now.saturating_duration_since(*start).as_secs_f32() / CROSS.as_secs_f32();
            let t = t.clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        });
        self.drawn.clear();
        for area in &areas {
            let size = (area.width, area.height);
            if let Some(level) = cross {
                // Old (at its last offset) into new (at its first), in memory.
                let new = self.ready.as_ref().and_then(|slide| slide.frame(size));
                let old = self.cross.as_ref().and_then(|(_, old)| old.as_ref());
                let old_px = old.and_then(|o| {
                    let off = self.offset(o, area, now)?;
                    Some(Self::crop(o.slide.frame(size)?, off, area))
                });
                let corner = self.ready.as_ref().map_or(0, |slide| slide.corner);
                let new_px = new.map(|frame| {
                    let start = drift_offset(
                        (frame.width, frame.height),
                        (usize::from(area.width), usize::from(area.height)),
                        corner,
                        0.0,
                    );
                    Self::crop(frame, start, area)
                });
                let n = usize::from(area.width) * usize::from(area.height);
                let mut pixels = vec![0u32; n];
                for (i, px) in pixels.iter_mut().enumerate() {
                    let a = old_px.as_ref().map_or(0, |o| o[i]);
                    let b = new_px.as_ref().map_or(0, |o| o[i]);
                    *px = images::mix(a, b, level);
                }
                let rect = self.label.rect(area);
                self.label.draw(&mut pixels, usize::from(area.width), rect);
                self.put(s, s.back, &pixels, usize::from(area.width), area.x, area.y)?;
                self.drawn.push((usize::MAX, usize::MAX));
            } else if let Some(shown) = self.shown.as_ref() {
                let Some(off) = self.offset(shown, area, now) else {
                    continue;
                };
                let Some(pixmap) = shown
                    .pixmaps
                    .iter()
                    .find(|(sz, _)| *sz == size)
                    .map(|p| p.1)
                else {
                    continue;
                };
                s.c.copy_area(
                    pixmap,
                    s.back,
                    s.gc,
                    off.0 as i16,
                    off.1 as i16,
                    area.x,
                    area.y,
                    area.width,
                    area.height,
                )?;
                // The clock's box only, blended in memory from the frame.
                let rect = self.label.rect(area);
                let mut boxed = Vec::with_capacity(rect.2 * rect.3);
                if let Some(frame) = shown.slide.frame(size) {
                    for y in rect.1..rect.1 + rect.3 {
                        let at = (off.1 + y) * frame.width + off.0 + rect.0;
                        boxed.extend_from_slice(&frame.pixels[at..at + rect.2]);
                    }
                }
                if boxed.len() != rect.2 * rect.3 {
                    continue;
                }
                self.label.draw(&mut boxed, rect.2, (0, 0, rect.2, rect.3));
                self.put(
                    s,
                    s.back,
                    &boxed,
                    rect.2,
                    area.x.saturating_add(rect.0 as i16),
                    area.y.saturating_add(rect.1 as i16),
                )?;
                self.drawn.push(off);
            } else {
                // Nothing loaded yet: black, with the clock.
                let n = usize::from(area.width) * usize::from(area.height);
                let mut pixels = vec![self.cfg.palette.background; n];
                let rect = self.label.rect(area);
                self.label.draw(&mut pixels, usize::from(area.width), rect);
                self.put(s, s.back, &pixels, usize::from(area.width), area.x, area.y)?;
                self.drawn.push((usize::MAX, usize::MAX));
            }
        }
        self.minute = Local::now().timestamp() / 60;
        Ok(())
    }

    /// When the next frame is due.
    fn schedule(&mut self, now: Instant) {
        if !self.on {
            self.next = None;
            return;
        }
        let minute = {
            let ms = Local::now().timestamp_millis();
            now + Duration::from_millis((60_000 - ms.rem_euclid(60_000)) as u64 + 5)
        };
        if self.cross.is_some() {
            self.next = Some(now + CROSS / CROSS_STEPS);
            return;
        }
        let Some(shown) = self.shown.as_ref() else {
            self.next = Some(minute);
            return;
        };
        let end = shown.since + Duration::from_secs_f32(self.seconds);
        // The next whole-pixel drift step: the widest room over the time.
        let room = self
            .areas
            .iter()
            .filter_map(|a| {
                let f = shown.slide.frame((a.width, a.height))?;
                Some(
                    f.width
                        .saturating_sub(usize::from(a.width))
                        .max(f.height.saturating_sub(usize::from(a.height))),
                )
            })
            .max()
            .unwrap_or(0);
        let mut next = if now >= end {
            // Waiting for the next picture: only the clock changes.
            minute
        } else {
            end.min(minute)
        };
        if room > 0 && now < end {
            let step = Duration::from_secs_f32(self.seconds / room as f32);
            next = next.min(now + step.max(Duration::from_millis(40)));
        }
        self.next = Some(next);
    }

    fn present_all(&self, s: &Surface) -> Result<()> {
        for a in &self.areas {
            s.present_rect(a.x, a.y, a.width, a.height)?;
        }
        Ok(())
    }
}

impl Show for Photos {
    fn redraw(&mut self, s: &mut Surface, now: Instant) -> Result<()> {
        if s.areas != self.areas {
            // New monitor sizes: the frames must be scaled again.
            self.areas = s.areas.clone();
            if let Some(old) = self.shown.take() {
                self.free(s, old)?;
            }
            if let Some((_, Some(old))) = self.cross.take() {
                self.free(s, old)?;
            }
            self.cross = None;
            self.ready = None;
            self.index = self.index.saturating_sub(1);
            self.request_next();
        }
        s.clear(self.cfg.palette.background)?;
        self.compose(s, now)?;
        s.present()?;
        self.schedule(now);
        Ok(())
    }

    fn step(&mut self, s: &mut Surface, now: Instant) -> Result<()> {
        // The crossfade ends: the new picture is the shown one.
        if let Some((start, _)) = self.cross.as_ref() {
            if now.saturating_duration_since(*start) >= CROSS || self.cfg.reduced_motion {
                let (_, old) = self.cross.take().expect("cross");
                if let Some(old) = old {
                    self.free(s, old)?;
                }
                if let Some(slide) = self.ready.take() {
                    self.shown = Some(self.show(s, slide, now)?);
                }
                self.request_next();
            }
        } else if let Some(shown) = self.shown.as_ref() {
            let end = shown.since + Duration::from_secs_f32(self.seconds);
            if now >= end && self.ready.is_some() {
                let old = self.shown.take();
                self.cross = Some((now, old));
            }
        } else if self.ready.is_some() {
            self.cross = Some((now, None));
        }
        let offsets: Vec<(usize, usize)> = match (&self.cross, &self.shown) {
            (None, Some(shown)) => self
                .areas
                .iter()
                .filter_map(|a| self.offset(shown, a, now))
                .collect(),
            _ => Vec::new(),
        };
        let minute = Local::now().timestamp() / 60;
        if self.cross.is_some() || offsets != self.drawn || minute != self.minute {
            self.compose(s, now)?;
            self.present_all(s)?;
        }
        self.schedule(now);
        Ok(())
    }

    fn deadline(&self) -> Option<Instant> {
        self.next
    }

    fn display(&mut self, on: bool) {
        self.on = on;
        if !on {
            self.next = None;
        } else {
            self.next = Some(Instant::now());
        }
    }

    fn wake_fds(&self) -> [Option<RawFd>; 2] {
        [Some(self.loader.wake.as_raw_fd()), None]
    }

    fn woke(&mut self, ready: [bool; 2]) {
        if !ready[0] {
            return;
        }
        let mut buf = [0u8; 16];
        while matches!(std::io::Read::read(&mut &self.loader.wake, &mut buf), Ok(n) if n > 0) {}
        while let Ok(loaded) = self.loader.receive.try_recv() {
            self.loader.busy = false;
            match loaded {
                Loaded::Slide(slide) => {
                    // Scaled for monitors that are still there?
                    let sizes = self.sizes();
                    if sizes.iter().all(|s| slide.frame(*s).is_some()) {
                        self.ready = Some(slide);
                    }
                }
                Loaded::Failed(path, why) => {
                    eprintln!("niruscreensaver: {}: {why}", path.display());
                    self.pictures.retain(|p| *p != path);
                    if self.index > self.pictures.len() {
                        self.index = 0;
                    }
                }
            }
        }
        self.request_next();
        // The first picture, or the next one when the shown one is due.
        if self.on {
            self.next = Some(Instant::now());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shuffle_picks_from_its_list() {
        let list = [Scene::Dark, Scene::Photos];
        for seed in 0..10 {
            assert!(list.contains(&pick(&list, seed)));
        }
        assert_eq!(pick(&[], 3), Scene::Clock);
        assert_ne!(pick(&list, 0), pick(&list, 1));
    }

    #[test]
    fn the_drift_runs_corner_to_corner_inside_the_frame() {
        let frame = (2035, 1272);
        let area = (1920, 1200);
        assert_eq!(drift_offset(frame, area, 0, 0.0), (0, 0));
        assert_eq!(drift_offset(frame, area, 0, 1.0), (115, 72));
        assert_eq!(drift_offset(frame, area, 3, 0.0), (115, 72));
        assert_eq!(drift_offset(frame, area, 3, 2.0), (0, 0));
        assert_eq!(drift_offset(area, area, 1, 0.5), (0, 0));
        assert_eq!(scaled(1920, 1200, 0.0), (1920, 1200));
        assert_eq!(scaled(1920, 1200, DRIFT), (2035, 1272));
    }

    #[test]
    fn the_tune_line_drops_the_esc_hint() {
        let info = "♪ Commando — Rob Hubbard · 1985  ·  Esc stops the music";
        assert_eq!(tune_line(info, true), "♪ Commando — Rob Hubbard · 1985");
        assert_eq!(tune_line(info, false), "Commando — Rob Hubbard · 1985");
    }
}
