//! `c64` and `c64-sid`: NIRUX as a Commodore 64 demo on every monitor.
//!
//! Each monitor gets a 320x200 C64 screen in a border that reaches its
//! edges, at the largest whole scale with sharp pixels, centred to the
//! pixel. The pictures are in [`art`]: a short boot with loading stripes,
//! then three parts that loop (NIRUX, BIFROST, YGGDRASIL) with the border
//! closing over the screen between them. The colours are the sixteen C64
//! colours, picked by the active NIRUX theme's accent and text colour.
//!
//! It is cheap: each frame is drawn in C64 pixels (about 480x270 bytes on a
//! 1080p monitor, whatever the monitor's size), turned into pixels through
//! a 16 colour table, handed to the X server through MIT-SHM (or a plain
//! PutImage where shared memory is not available, as over ssh), and scaled
//! up by the server with RENDER, nearest neighbour. It runs at [`FRAME`]
//! (50 frames a second, the PAL rate) and slower on very large screens
//! ([`frame_for`]). With the display off nothing is drawn and the loop
//! sleeps. With `reduced_motion` it shows one still picture of a part, a
//! new one every minute.
//!
//! `c64` is silent. `c64-sid` plays the pause music exactly like `dark-sid`
//! and shows the tune in the upper border; while it plays, the raster bars,
//! the logo, the scroller and the sphere follow the beat (CAVA's bass).

use super::{audio, music, tune_line, Show};
use crate::cava::Cava;
use crate::config::Config;
use crate::core::{Area, Surface};
use crate::Result;
use std::os::fd::RawFd;
use std::time::{Duration, Instant};
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::render::{self, ConnectionExt as _};
use x11rb::protocol::shm::{self, ConnectionExt as _};
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;

mod art;

/// The frame interval: 50 frames a second.
pub const FRAME: Duration = Duration::from_millis(20);
/// The longest frame interval, for very large screens.
pub const SLOW_FRAME: Duration = Duration::from_millis(40);
/// Monitor pixels a frame that still run at [`FRAME`]: one 2560x1440 or
/// almost two 1080p monitors. Beyond that the interval grows (to
/// [`SLOW_FRAME`] at most), so an X server without a GPU scales no more
/// than this many pixels in a fiftieth of a second.
const FULL_RATE_PIXELS: i64 = 2560 * 1440;
/// A round trip to the X server every this many frames.
const SYNC_EVERY: u32 = 10;
/// With `reduced_motion` the still picture changes this often.
const STILL: Duration = Duration::from_secs(60);

/// The frame interval for `pixels` monitor pixels.
pub fn frame_for(pixels: i64) -> Duration {
    if pixels <= FULL_RATE_PIXELS {
        return FRAME;
    }
    let scaled = FRAME.as_secs_f64() * pixels as f64 / FULL_RATE_PIXELS as f64;
    Duration::from_secs_f64(scaled.min(SLOW_FRAME.as_secs_f64()))
}

/// Follow the bass for the beat: a jump over its running average lights
/// it, and it fades over a third of a second (as NIRULOCK's demo layout).
pub fn beat(flash: f32, bass: f32, average: f32, dt: f32) -> (f32, f32) {
    let faded = flash * (0.0005f32).powf(dt.clamp(0.0, 1.0));
    let lit = if bass > 0.22 && bass > average * 1.25 + 0.06 {
        1.0
    } else {
        faded
    };
    let k = (dt * 4.0).clamp(0.0, 1.0);
    (lit, average + (bass - average) * k)
}

/// The scale transform for RENDER: monitor pixel to C64 pixel, with the
/// rounding error of 16.16 fixed point centred over `span` monitor pixels,
/// so even an 8K monitor samples every C64 pixel the same number of times.
fn transform(scale: i32, span: i32) -> render::Transform {
    let exact = 65536.0 / f64::from(scale.max(1));
    let fixed = exact.round();
    let drift = f64::from(span) * (fixed - exact);
    render::Transform {
        matrix11: fixed as i32,
        matrix12: 0,
        matrix13: (-drift / 2.0).round() as i32,
        matrix21: 0,
        matrix22: fixed as i32,
        matrix23: (-drift / 2.0).round() as i32,
        matrix31: 0,
        matrix32: 0,
        matrix33: 65536,
    }
}

/// A System V shared memory segment attached to the X server.
struct Shm {
    seg: shm::Seg,
    addr: *mut u8,
    len: usize,
}

impl Shm {
    /// A segment of `len` bytes, or `None` where the server cannot map it
    /// (another machine, or MIT-SHM missing).
    fn new(c: &RustConnection, len: usize) -> Option<Self> {
        // SAFETY: plain System V calls; every failure is checked, and the
        // segment is marked for removal once the server has attached, so it
        // goes when both sides detach (also when the saver is killed).
        unsafe {
            let id = libc::shmget(libc::IPC_PRIVATE, len, libc::IPC_CREAT | 0o600);
            if id < 0 {
                return None;
            }
            let addr = libc::shmat(id, std::ptr::null(), 0);
            if addr as isize == -1 {
                libc::shmctl(id, libc::IPC_RMID, std::ptr::null_mut());
                return None;
            }
            let attached = c.generate_id().ok().and_then(|seg| {
                c.shm_attach(seg, id as u32, true)
                    .ok()?
                    .check()
                    .ok()
                    .map(|()| seg)
            });
            libc::shmctl(id, libc::IPC_RMID, std::ptr::null_mut());
            match attached {
                Some(seg) => Some(Self {
                    seg,
                    addr: addr.cast(),
                    len,
                }),
                None => {
                    libc::shmdt(addr);
                    None
                }
            }
        }
    }

    fn pixels(&mut self) -> &mut [u32] {
        // SAFETY: `addr` is page aligned and maps `len` bytes for as long as
        // `self` lives; the server only reads it during a request the saver
        // waits for.
        unsafe { std::slice::from_raw_parts_mut(self.addr.cast(), self.len / 4) }
    }

    fn free(self, c: &RustConnection) {
        let _ = c.shm_detach(self.seg);
        // SAFETY: the address came from shmat and is detached once.
        unsafe {
            libc::shmdt(self.addr.cast());
        }
    }
}

/// One monitor's frame buffer, pixmap and picture.
struct Out {
    area: Area,
    geo: art::Geometry,
    canvas: art::Canvas,
    /// The frame shown before, to send only what changed.
    shown: art::Canvas,
    pixmap: Pixmap,
    picture: render::Picture,
    shm: Option<Shm>,
    /// The pixels for a plain PutImage, without shared memory.
    pixels: Vec<u32>,
}

/// A rectangle of the frame buffer: left, top, right, bottom (exclusive).
type Rect = (usize, usize, usize, usize);

/// A frame buffer rectangle in monitor pixels, relative to the monitor
/// and clipped to it: left, top, width, height; `None` when nothing of it
/// is on the monitor.
pub fn on_monitor(g: &art::Geometry, area: &Area, rect: Rect) -> Option<(i32, i32, u16, u16)> {
    let (aw, ah) = (i32::from(area.width), i32::from(area.height));
    let (x0, y0, x1, y1) = (rect.0 as i32, rect.1 as i32, rect.2 as i32, rect.3 as i32);
    let dx0 = (x0 * g.scale - g.phase_x).clamp(0, aw);
    let dx1 = (x1 * g.scale - g.phase_x).clamp(0, aw);
    let dy0 = (y0 * g.scale - g.phase_y).clamp(0, ah);
    let dy1 = (y1 * g.scale - g.phase_y).clamp(0, ah);
    (dx1 > dx0 && dy1 > dy0).then(|| (dx0, dy0, (dx1 - dx0) as u16, (dy1 - dy0) as u16))
}

/// The rectangles where `now` differs from `before` (both `width` wide):
/// changed rows in bands, each as wide as its changes, with bands less
/// than four rows apart joined. Everything when the sizes differ.
pub fn changes(before: &[u8], now: &[u8], width: usize) -> Vec<Rect> {
    let width = width.max(1);
    let height = now.len() / width;
    if before.len() != now.len() {
        return vec![(0, 0, width, height)];
    }
    let mut out: Vec<Rect> = Vec::new();
    let mut open: Option<Rect> = None;
    for y in 0..height {
        let (a, b) = (&before[y * width..][..width], &now[y * width..][..width]);
        if a == b {
            if let Some(r) = open {
                if y - r.3 >= 3 {
                    out.push(r);
                    open = None;
                }
            }
            continue;
        }
        let first = a.iter().zip(b).position(|(p, q)| p != q).unwrap_or(0);
        let last = width
            - a.iter()
                .rev()
                .zip(b.iter().rev())
                .position(|(p, q)| p != q)
                .unwrap_or(0);
        open = Some(match open {
            Some((x0, y0, x1, _)) => (x0.min(first), y0, x1.max(last), y + 1),
            None => (first, y, last, y + 1),
        });
    }
    out.extend(open);
    out
}

pub(super) struct C64 {
    cfg: Config,
    theme: art::Theme,
    start: Instant,
    outs: Vec<Out>,
    areas: Vec<Area>,
    window: render::Picture,
    format: render::Pictformat,
    /// The server's pixel bytes are the other way round from ours.
    swap: bool,
    use_shm: bool,
    /// Frames painted, for [`SYNC_EVERY`].
    painted: u32,
    /// The colour table the shown frames were sent with.
    lut: [u32; 16],
    /// The time of day for the frames, and when it was read.
    clock: (Instant, String),
    on: bool,
    next: Option<Instant>,
    frame: Duration,
    host: Option<String>,
    music: Option<music::Music>,
    listen: Option<audio::Audio>,
    stop_on_display_off: bool,
    tune: Option<String>,
    cava: Option<Cava>,
    flash: f32,
    bass: f32,
    last: Instant,
}

impl C64 {
    /// The scene, or why it cannot run on this display (no RENDER, or not
    /// 32 bits a pixel); then the caller shows `dark`.
    pub(super) fn new(cfg: &Config, s: &Surface, sid: bool) -> Result<Self> {
        let c = &s.c;
        if c.extension_information(render::X11_EXTENSION_NAME)?
            .is_none()
        {
            return Err("the X server has no RENDER".into());
        }
        c.render_query_version(0, 11)?.reply()?;
        let setup = c.setup();
        let bpp = setup
            .pixmap_formats
            .iter()
            .find(|f| f.depth == s.depth)
            .map(|f| f.bits_per_pixel);
        if s.depth < 24 || bpp != Some(32) {
            return Err(
                format!("depth {} at {bpp:?} bits a pixel is not supported", s.depth).into(),
            );
        }
        let visual = setup.roots[s.screen_num].root_visual;
        let formats = c.render_query_pict_formats()?.reply()?;
        let format = formats
            .screens
            .iter()
            .flat_map(|screen| &screen.depths)
            .flat_map(|depth| &depth.visuals)
            .find(|v| v.visual == visual)
            .map(|v| v.format)
            .ok_or("no RENDER format for the window")?;
        let window = c.generate_id()?;
        c.render_create_picture(window, s.win, format, &render::CreatePictureAux::new())?;
        let use_shm = c.extension_information(shm::X11_EXTENSION_NAME)?.is_some()
            && c.shm_query_version()
                .ok()
                .and_then(|cookie| cookie.reply().ok())
                .is_some();
        let swap =
            (setup.image_byte_order == ImageOrder::LSB_FIRST) != cfg!(target_endian = "little");
        let settings = super::lock_music_settings();
        let music = sid.then(|| music::Music::start(&settings)).flatten();
        let listen = music
            .is_some()
            .then(|| audio::Audio::start("niruscreensaver", None));
        let cava = music.is_some().then(Cava::new);
        let p = &cfg.palette;
        let now = Instant::now();
        let mut me = Self {
            cfg: cfg.clone(),
            theme: art::Theme::new(p.focus, p.foreground),
            start: now,
            outs: Vec::new(),
            areas: Vec::new(),
            window,
            format,
            swap,
            use_shm,
            painted: 0,
            lut: [u32::MAX; 16],
            clock: (now, chrono::Local::now().format("%H:%M").to_string()),
            on: true,
            next: Some(now),
            frame: FRAME,
            host: cfg.show_hostname.then(crate::core::hostname),
            music,
            listen,
            stop_on_display_off: settings.stop_on_display_off,
            tune: None,
            cava,
            flash: 0.0,
            bass: 0.0,
            last: now,
        };
        me.build(s)?;
        Ok(me)
    }

    /// A frame buffer, a pixmap and a scaling picture for every monitor.
    fn build(&mut self, s: &Surface) -> Result<()> {
        self.free(&s.c);
        let c = &s.c;
        let root = c.setup().roots[s.screen_num].root;
        let mut pixels = 0i64;
        for area in &s.areas {
            let geo = art::geometry(area.width, area.height);
            let (w, h) = (geo.width as u16, geo.height as u16);
            let pixmap = c.generate_id()?;
            c.create_pixmap(s.depth, pixmap, root, w, h)?;
            let picture = c.generate_id()?;
            c.render_create_picture(
                picture,
                pixmap,
                self.format,
                &render::CreatePictureAux::new(),
            )?;
            let span = i32::from(area.width.max(area.height)) + geo.scale;
            c.render_set_picture_transform(picture, transform(geo.scale, span))?;
            c.render_set_picture_filter(picture, b"nearest", &[])?;
            let len = usize::from(w) * usize::from(h) * 4;
            let shm = if self.use_shm { Shm::new(c, len) } else { None };
            if self.use_shm && shm.is_none() {
                // Not on this server after all (a remote display).
                self.use_shm = false;
            }
            let fallback = if shm.is_some() {
                Vec::new()
            } else {
                vec![0; len / 4]
            };
            pixels += i64::from(area.width) * i64::from(area.height);
            self.outs.push(Out {
                area: *area,
                geo,
                canvas: art::Canvas::default(),
                shown: art::Canvas::default(),
                pixmap,
                picture,
                shm,
                pixels: fallback,
            });
        }
        self.areas = s.areas.clone();
        self.frame = frame_for(pixels);
        Ok(())
    }

    fn free(&mut self, c: &RustConnection) {
        for out in self.outs.drain(..) {
            let _ = c.render_free_picture(out.picture);
            let _ = c.free_pixmap(out.pixmap);
            if let Some(shm) = out.shm {
                shm.free(c);
            }
        }
    }

    /// The scene's clock: seconds since the start in whole frames, or a
    /// still moment.
    fn moment_time(&self, now: Instant) -> (f32, bool) {
        if self.cfg.reduced_motion {
            let minute = now.saturating_duration_since(self.start).as_secs() / 60;
            let part = (minute % art::PARTS as u64) as f32;
            return (part * art::PART + 9.0, false);
        }
        // Whole frames, so everything moves in even steps however late the
        // timer fires.
        let frame = self.frame.as_secs_f64();
        let elapsed = now.saturating_duration_since(self.start).as_secs_f64();
        (((elapsed / frame).round() * frame) as f32, true)
    }

    /// Follow the bass while the music plays.
    fn listen_beat(&mut self, now: Instant) {
        let dt = now.saturating_duration_since(self.last).as_secs_f32();
        self.last = now;
        let playing = self.tune.is_some();
        let bass = match self.cava.as_mut() {
            Some(cava) if playing && self.on => {
                let values = cava.values();
                let n = (values.len() / 6).max(1).min(values.len());
                if n == 0 {
                    0.0
                } else {
                    values[..n].iter().sum::<f32>() / n as f32
                }
            }
            _ => 0.0,
        };
        let (flash, average) = beat(self.flash, bass, self.bass, dt);
        self.flash = flash;
        self.bass = average;
    }

    /// Draw one frame on every monitor and show what changed, or all of
    /// it with `full` (after an exposure).
    fn paint(&mut self, s: &Surface, now: Instant, full: bool) -> Result<()> {
        // A round trip now and then keeps a slow server from falling behind.
        // The server copies a frame out of shared memory as it reads the
        // request, long before the next frame is written 20 ms later; a
        // round trip every frame cost a fifth of the scene's CPU time.
        self.painted = self.painted.wrapping_add(1);
        if full || self.painted % SYNC_EVERY == 0 {
            s.c.get_input_focus()?.reply()?;
        }
        self.listen_beat(now);
        if now.saturating_duration_since(self.clock.0) >= Duration::from_secs(1) {
            self.clock = (now, chrono::Local::now().format("%H:%M").to_string());
        }
        let (t, boot) = self.moment_time(now);
        let tune = self.tune.clone();
        let moment = art::Moment {
            t,
            boot,
            clock: &self.clock.1,
            host: self.host.as_deref(),
            tune: tune.as_deref(),
            beat: if self.cfg.reduced_motion {
                0.0
            } else {
                self.flash
            },
        };
        let level = if self.cfg.reduced_motion {
            1.0
        } else {
            crate::fade_level(now.saturating_duration_since(self.start))
        };
        let mut lut = art::lut(((1.0 - level) * 4.0).round() as u8);
        if self.swap {
            lut = lut.map(u32::swap_bytes);
        }
        let all = full || lut != self.lut;
        self.lut = lut;
        let (c, gc, depth) = (&s.c, s.gc, s.depth);
        let max_bytes = c.maximum_request_bytes();
        for out in &mut self.outs {
            art::frame(&mut out.canvas, &out.geo, &self.theme, &moment);
            let width = out.canvas.width as usize;
            let rects = if all {
                vec![(0, 0, width, out.canvas.pixels.len() / width)]
            } else {
                changes(&out.shown.pixels, &out.canvas.pixels, width)
            };
            let (w, h) = (out.geo.width as u16, out.geo.height as u16);
            let dst: &mut [u32] = match out.shm.as_mut() {
                Some(shm) => shm.pixels(),
                None => &mut out.pixels,
            };
            for &(x0, y0, x1, y1) in &rects {
                for y in y0..y1 {
                    let row = y * width;
                    for (d, &i) in dst[row + x0..row + x1]
                        .iter_mut()
                        .zip(&out.canvas.pixels[row + x0..row + x1])
                    {
                        *d = lut[usize::from(i & 15)];
                    }
                }
            }
            for &(x0, y0, x1, y1) in &rects {
                let (rw, rh) = ((x1 - x0) as u16, (y1 - y0) as u16);
                if let Some(shm) = out.shm.as_ref() {
                    c.shm_put_image(
                        out.pixmap,
                        gc,
                        w,
                        h,
                        x0 as u16,
                        y0 as u16,
                        rw,
                        rh,
                        x0 as i16,
                        y0 as i16,
                        depth,
                        ImageFormat::Z_PIXMAP.into(),
                        false,
                        shm.seg,
                        0,
                    )?;
                } else {
                    // In bands that fit a request.
                    let line = usize::from(rw) * 4;
                    let rows = (max_bytes.saturating_sub(64) / line.max(1)).max(1);
                    let mut y = y0;
                    while y < y1 {
                        let n = rows.min(y1 - y);
                        let mut bytes = Vec::with_capacity(n * line);
                        for yy in y..y + n {
                            for p in &out.pixels[yy * width + x0..yy * width + x1] {
                                bytes.extend_from_slice(&p.to_ne_bytes());
                            }
                        }
                        c.put_image(
                            ImageFormat::Z_PIXMAP,
                            out.pixmap,
                            gc,
                            rw,
                            n as u16,
                            x0 as i16,
                            y as i16,
                            0,
                            depth,
                            &bytes,
                        )?;
                        y += n;
                    }
                }
                let Some((dx, dy, dw, dh)) = on_monitor(&out.geo, &out.area, (x0, y0, x1, y1))
                else {
                    continue;
                };
                c.render_composite(
                    render::PictOp::SRC,
                    out.picture,
                    x11rb::NONE,
                    self.window,
                    (out.geo.phase_x + dx) as i16,
                    (out.geo.phase_y + dy) as i16,
                    0,
                    0,
                    (i32::from(out.area.x) + dx) as i16,
                    (i32::from(out.area.y) + dy) as i16,
                    dw,
                    dh,
                )?;
            }
            std::mem::swap(&mut out.canvas, &mut out.shown);
        }
        c.flush()?;
        Ok(())
    }

    fn schedule(&mut self, now: Instant) {
        self.next = if !self.on {
            None
        } else if self.cfg.reduced_motion {
            Some(now + STILL)
        } else {
            let due = self.next.map_or(now, |d| d + self.frame);
            Some(if due <= now { now + self.frame } else { due })
        };
    }
}

impl Show for C64 {
    fn redraw(&mut self, s: &mut Surface, now: Instant) -> Result<()> {
        if s.areas != self.areas {
            self.build(s)?;
        }
        if self.on {
            self.paint(s, now, true)?;
        }
        if self.on && self.next.is_none() {
            self.next = Some(now + self.frame);
        }
        Ok(())
    }

    fn step(&mut self, s: &mut Surface, now: Instant) -> Result<()> {
        if self.on {
            self.paint(s, now, false)?;
        }
        self.schedule(now);
        Ok(())
    }

    fn deadline(&self) -> Option<Instant> {
        let mine = if self.on { self.next } else { None };
        let music = self.music.as_ref().and_then(music::Music::deadline);
        match (mine, music) {
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
        // The tune is drawn with the next frame; no extra redraw.
        self.tune = music.info().map(|info| tune_line(&info, false));
        false
    }

    fn display(&mut self, on: bool) {
        self.on = on;
        if let Some(cava) = self.cava.as_mut() {
            cava.set_display(on);
        }
        if !on && self.stop_on_display_off {
            if let Some(music) = self.music.as_mut() {
                music.stop();
            }
        }
        if !on {
            self.next = None;
            self.flash = 0.0;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_changed_rows_are_sent() {
        let w = 10;
        let before = vec![0u8; w * 20];
        assert!(changes(&before, &before, w).is_empty());
        let mut now = before.clone();
        now[2 * w + 3] = 1;
        now[3 * w + 7] = 1;
        assert_eq!(changes(&before, &now, w), vec![(3, 2, 8, 4)]);
        // Rows far apart are separate bands; close ones are joined.
        now[5 * w] = 2;
        now[15 * w + 9] = 2;
        assert_eq!(
            changes(&before, &now, w),
            vec![(0, 2, 8, 6), (9, 15, 10, 16)]
        );
        // A new size sends everything.
        assert_eq!(changes(&[], &now, w), vec![(0, 0, 10, 20)]);
    }

    #[test]
    fn every_change_lands_inside_its_monitor() {
        let monitors = [
            (0i16, 0i16, 1920u16, 1080u16),
            (1920, 28, 1280, 1024),
            (-1080, 0, 1080, 1920),
            (3200, 0, 3840, 2160),
            (0, 1080, 1366, 768),
        ];
        for (x, y, width, height) in monitors {
            let area = Area {
                x,
                y,
                width,
                height,
                primary: false,
            };
            let g = art::geometry(width, height);
            let (w, h) = (g.width as usize, g.height as usize);
            // The whole buffer covers the whole monitor, exactly.
            assert_eq!(
                on_monitor(&g, &area, (0, 0, w, h)),
                Some((0, 0, width, height))
            );
            for rect in [(0, 0, 1, 1), (w - 1, h - 1, w, h), (3, 7, w / 2, h / 3)] {
                let (dx, dy, dw, dh) = on_monitor(&g, &area, rect).unwrap();
                assert!(dx >= 0 && dy >= 0);
                assert!(dx + i32::from(dw) <= i32::from(width));
                assert!(dy + i32::from(dh) <= i32::from(height));
            }
        }
    }

    #[test]
    fn large_screens_lower_the_frame_rate_within_limits() {
        assert_eq!(frame_for(1920 * 1080), FRAME);
        assert_eq!(frame_for(2560 * 1440), FRAME);
        assert!(frame_for(2 * 2560 * 1440) > FRAME);
        assert_eq!(frame_for(3 * 3840 * 2160), SLOW_FRAME);
        assert!(frame_for(4 * 1920 * 1080) <= SLOW_FRAME);
    }

    #[test]
    fn a_beat_flashes_and_fades() {
        let (lit, avg) = beat(0.0, 0.8, 0.2, 0.02);
        assert_eq!(lit, 1.0);
        assert!(avg > 0.2);
        let (faded, _) = beat(1.0, 0.0, 0.5, 0.33);
        assert!(faded < 0.1, "{faded}");
        assert_eq!(beat(0.0, 0.0, 0.0, 0.02).0, 0.0, "silence stays dark");
    }

    #[test]
    fn the_transform_samples_every_c64_pixel_evenly() {
        // Pixel centres, as the server samples them, land in the right
        // C64 pixel across the whole width, also on an 8K monitor.
        for (scale, span) in [(1, 400), (3, 1400), (4, 1960), (9, 3850), (19, 7700)] {
            let m = transform(scale, span);
            for x in (0..span).step_by(1) {
                let dst = (i64::from(x) << 16) + 32768;
                let src = ((dst * i64::from(m.matrix11)) >> 16) + i64::from(m.matrix13);
                assert_eq!(src >> 16, i64::from(x / scale), "scale {scale}, x {x}");
            }
        }
    }

    #[test]
    fn frames_stay_in_bounds_on_many_monitors() {
        let th = art::Theme::new(0x8aa87e, 0xd8ded2);
        for (w, h) in [
            (1920u16, 1080u16),
            (1080, 1920),
            (3840, 2160),
            (1366, 768),
            (640, 480),
            (320, 200),
            (100, 50),
            (7680, 4320),
        ] {
            let g = art::geometry(w, h);
            let mut cv = art::Canvas::default();
            for t in [0.2, 3.0, 8.0, 20.0, 40.0, 70.0] {
                let m = art::Moment {
                    t,
                    boot: true,
                    clock: "00:00",
                    host: Some("niruxwork"),
                    tune: Some("♪ Ginnungagap  ·  NIRUX 1"),
                    beat: 1.0,
                };
                art::frame(&mut cv, &g, &th, &m);
                assert_eq!(cv.pixels.len(), (g.width * g.height) as usize);
            }
            // The pixmap fits X's 16-bit sizes.
            assert!(g.width <= i32::from(u16::MAX) && g.height <= i32::from(u16::MAX));
        }
    }
}
