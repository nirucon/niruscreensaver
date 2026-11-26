//! The `c64` scene's pictures: every frame is computed here from the time
//! alone, free of I/O, into a small indexed frame buffer that holds the
//! sixteen C64 colours. [`super`] uploads it and lets the X server scale it.
//!
//! The frame is a whole monitor in C64 pixels: a 320x200 screen in the
//! middle of a border that reaches every edge ([`geometry`]). The demo:
//!
//! - a short boot once at the start: the blue BASIC screen,
//!   `LOAD"NIRUX",8,1` typed out, loading stripes in the border, `RUN`;
//! - part 1, NIRUX: raster bars in the border, a big chunky NIRUX logo
//!   that bounces and stretches with a sheen running through it, a colour
//!   washed line and a DYCP scroller (every letter at its own height) with
//!   the greetings and the time, mirrored in water below it;
//! - part 2, BIFROST: a rotating dot sphere, 24 shaded balls on a slowly
//!   changing Lissajous path in front of and behind it, the five runes
//!   NIRUX in Elder Futhark, rainbow bars in the border;
//! - part 3, YGGDRASIL: a character plasma behind the black silhouette of
//!   the world tree, its roots and crown, and the time in the lower border.
//!
//! Between the parts the border closes over the screen while the picture
//! fades through the C64 fade table, and opens again on the next part.

use crate::pixfont;
use std::f32::consts::{PI, TAU};
use std::sync::OnceLock;

/// The C64 screen in its own pixels.
pub const SCREEN_W: i32 = 320;
pub const SCREEN_H: i32 = 200;
/// The thinnest border, in C64 pixels, that the scale leaves on each side.
const MIN_BORDER: i32 = 16;
/// The boot screen's length, once at the start.
pub const BOOT: f32 = 7.0;
/// Each part's length, and the curtain at either end of it.
pub const PART: f32 = 24.0;
const CURTAIN: f32 = 0.7;
/// The number of parts.
pub const PARTS: usize = 3;
/// The scroller's speed in C64 pixels a second: two a frame at 50 Hz.
const SCROLL: f32 = 100.0;

/// The sixteen C64 colours, by their C64 number.
pub const C64: [u32; 16] = [
    0x000000, // 0 black
    0xffffff, // 1 white
    0x813338, // 2 red
    0x75cec8, // 3 cyan
    0x8e3c97, // 4 purple
    0x56ac4d, // 5 green
    0x2e2c9b, // 6 blue
    0xedf171, // 7 yellow
    0x8e5029, // 8 orange
    0x553800, // 9 brown
    0xc46c71, // 10 light red
    0x4a4a4a, // 11 dark grey
    0x7b7b7b, // 12 grey
    0xa9ff9f, // 13 light green
    0x706deb, // 14 light blue
    0xb2b2b2, // 15 light grey
];

/// One step darker for every colour, keeping its hue where it can: four
/// steps take anything to black, like a C64 fade table.
const DARKER: [u8; 16] = [0, 15, 9, 14, 6, 11, 0, 8, 9, 0, 2, 0, 11, 5, 6, 12];

/// Colour ramps from black to white, as on a C64 raster bar.
pub type Ramp = &'static [u8];
pub const STEEL: Ramp = &[0, 11, 12, 15, 1];
pub const BLUE: Ramp = &[0, 6, 14, 3, 1];
pub const FIRE: Ramp = &[0, 9, 2, 8, 10, 7, 1];
pub const GOLD: Ramp = &[0, 9, 8, 7, 1];
pub const GREEN: Ramp = &[0, 11, 5, 13, 1];
pub const CYAN: Ramp = &[0, 6, 11, 3, 13, 1];
pub const PURPLE: Ramp = &[0, 6, 4, 10, 15, 1];
/// The two rainbow bars of BIFROST.
const RAINBOW: [u8; 10] = [2, 8, 7, 13, 5, 3, 14, 6, 4, 10];

/// The colours of one theme: the logo, the bars and the washes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub logo: Ramp,
    pub bars: Ramp,
    pub wash: Ramp,
}

fn hue_sat(colour: u32) -> (f32, f32, f32) {
    let r = ((colour >> 16) & 255) as f32 / 255.0;
    let g = ((colour >> 8) & 255) as f32 / 255.0;
    let b = (colour & 255) as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let sat = if max > 0.0 { d / max } else { 0.0 };
    let hue = if d <= f32::EPSILON {
        0.0
    } else if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (hue, sat, max)
}

/// The ramp nearest a theme colour: grey (STEEL) for a colour without
/// much hue.
pub fn ramp_for(colour: u32) -> Ramp {
    let (hue, sat, value) = hue_sat(colour);
    if sat < 0.2 || value < 0.12 {
        return STEEL;
    }
    match hue {
        h if !(20.0..330.0).contains(&h) => FIRE,
        h if h < 50.0 => FIRE,
        h if h < 72.0 => GOLD,
        h if h < 160.0 => GREEN,
        h if h < 200.0 => CYAN,
        h if h < 255.0 => BLUE,
        _ => PURPLE,
    }
}

/// The ramp that goes with `r`, for contrast.
fn companion(r: Ramp) -> Ramp {
    match r {
        r if r == STEEL => BLUE,
        r if r == BLUE => FIRE,
        r if r == FIRE || r == GOLD => BLUE,
        r if r == GREEN => GOLD,
        r if r == CYAN => PURPLE,
        _ => CYAN,
    }
}

impl Theme {
    /// The logo takes the theme's accent, the bars its text colour when that
    /// has a hue of its own, otherwise the accent's companion; the washes
    /// a third ramp. A grey theme gives steel with the C64's own blue.
    pub fn new(accent: u32, foreground: u32) -> Self {
        let logo = ramp_for(accent);
        let fg = ramp_for(foreground);
        let bars = if fg != logo && fg != STEEL {
            fg
        } else {
            companion(logo)
        };
        let mut wash = companion(bars);
        if wash == logo {
            wash = if logo == FIRE { GREEN } else { FIRE };
        }
        Self { logo, bars, wash }
    }
}

/// A ramp's colour at `level` (0 dark, 1 white).
fn shade(r: Ramp, level: f32) -> u8 {
    let i = (level.clamp(0.0, 1.0) * (r.len() - 1) as f32).round() as usize;
    r[i.min(r.len() - 1)]
}

/// The sixteen colours `steps` steps darker, for the fade-in.
pub fn lut(steps: u8) -> [u32; 16] {
    let mut out = [0u32; 16];
    for (i, slot) in out.iter_mut().enumerate() {
        let mut c = i as u8;
        for _ in 0..steps {
            c = DARKER[c as usize];
        }
        *slot = C64[c as usize];
    }
    out
}

/// Where the C64 picture sits on a monitor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    /// Monitor pixels per C64 pixel.
    pub scale: i32,
    /// The frame buffer in C64 pixels: the whole monitor.
    pub width: i32,
    pub height: i32,
    /// The screen's top left in the frame buffer.
    pub sx: i32,
    pub sy: i32,
    /// Monitor pixels cut from the buffer's left and top edge, so the
    /// screen is centred to the monitor pixel.
    pub phase_x: i32,
    pub phase_y: i32,
}

/// The largest whole scale that leaves a border of at least
/// [`MIN_BORDER`] around the screen, and the buffer that covers the rest.
pub fn geometry(width: u16, height: u16) -> Geometry {
    let (w, h) = (i32::from(width).max(1), i32::from(height).max(1));
    let scale = (w / (SCREEN_W + 2 * MIN_BORDER))
        .min(h / (SCREEN_H + 2 * MIN_BORDER))
        .max(1);
    let axis = |device: i32, screen: i32| {
        let start = (device - screen * scale) / 2;
        let first = start.div_euclid(scale) + i32::from(start.rem_euclid(scale) != 0);
        let phase = first * scale - start;
        let len = (device + phase + scale - 1) / scale;
        (first, phase, len)
    };
    let (sx, phase_x, width) = axis(w, SCREEN_W);
    let (sy, phase_y, height) = axis(h, SCREEN_H);
    Geometry {
        scale,
        width,
        height,
        sx,
        sy,
        phase_x,
        phase_y,
    }
}

/// An indexed frame buffer: one C64 colour number a pixel.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Canvas {
    pub width: i32,
    pub height: i32,
    pub pixels: Vec<u8>,
}

impl Canvas {
    fn fit(&mut self, width: i32, height: i32) {
        self.width = width.max(1);
        self.height = height.max(1);
        self.pixels.resize((self.width * self.height) as usize, 0);
    }

    fn set(&mut self, x: i32, y: i32, c: u8) {
        if x >= 0 && y >= 0 && x < self.width && y < self.height {
            self.pixels[(y * self.width + x) as usize] = c;
        }
    }

    fn row(&mut self, y: i32, x0: i32, x1: i32, c: u8) {
        if y < 0 || y >= self.height {
            return;
        }
        let (a, b) = (x0.max(0), x1.min(self.width));
        if a < b {
            let base = (y * self.width) as usize;
            self.pixels[base + a as usize..base + b as usize].fill(c);
        }
    }
}

/// Drawing on the screen: screen coordinates, clipped to the screen.
struct Scr<'a> {
    cv: &'a mut Canvas,
    ox: i32,
    oy: i32,
    /// The visible screen columns and rows in the canvas.
    x0: i32,
    x1: i32,
    y0: i32,
    y1: i32,
}

impl<'a> Scr<'a> {
    fn new(cv: &'a mut Canvas, g: &Geometry) -> Self {
        let (x0, x1) = (g.sx.max(0), (g.sx + SCREEN_W).min(cv.width));
        let (y0, y1) = (g.sy.max(0), (g.sy + SCREEN_H).min(cv.height));
        Self {
            cv,
            ox: g.sx,
            oy: g.sy,
            x0,
            x1,
            y0,
            y1,
        }
    }

    #[inline]
    fn set(&mut self, x: i32, y: i32, c: u8) {
        let (cx, cy) = (x + self.ox, y + self.oy);
        if cx >= self.x0 && cx < self.x1 && cy >= self.y0 && cy < self.y1 {
            self.cv.pixels[(cy * self.cv.width + cx) as usize] = c;
        }
    }

    #[inline]
    fn get(&self, x: i32, y: i32) -> u8 {
        let (cx, cy) = (x + self.ox, y + self.oy);
        if cx >= self.x0 && cx < self.x1 && cy >= self.y0 && cy < self.y1 {
            self.cv.pixels[(cy * self.cv.width + cx) as usize]
        } else {
            0
        }
    }

    /// Screen row `y` as a slice, when the whole row is visible.
    fn row_mut(&mut self, y: i32) -> Option<&mut [u8]> {
        let cy = y + self.oy;
        let whole = self.x0 == self.ox && self.x1 == self.ox + SCREEN_W;
        if !whole || cy < self.y0 || cy >= self.y1 {
            return None;
        }
        let base = (cy * self.cv.width + self.ox) as usize;
        Some(&mut self.cv.pixels[base..base + SCREEN_W as usize])
    }

    fn clear(&mut self, c: u8) {
        for y in self.y0..self.y1 {
            self.cv.row(y, self.x0, self.x1, c);
        }
    }
}

/// Every lit pixel of `text` in the 8x8 font at `scale`: `plot(x, y,
/// char index, glyph row)`.
fn glyphs(text: &str, x: i32, y: i32, scale: i32, mut plot: impl FnMut(i32, i32, usize, i32)) {
    for (i, ch) in text.chars().enumerate() {
        let g = pixfont::glyph(ch);
        let gx = x + i as i32 * 8 * scale;
        for (row, bits) in g.iter().enumerate() {
            if *bits == 0 {
                continue;
            }
            for col in 0..8 {
                if bits & (0x80 >> col) != 0 {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            plot(
                                gx + col * scale + dx,
                                y + row as i32 * scale + dy,
                                i,
                                row as i32,
                            );
                        }
                    }
                }
            }
        }
    }
}

/// Where in the demo a moment is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stage {
    /// The boot screen, seconds since the start.
    Boot(f32),
    /// A part: which one, seconds into it, how often it has run before
    /// and how far the border is closed over it (0 open, 1 closed).
    Part {
        part: usize,
        local: f32,
        round: u32,
        curtain: f32,
    },
}

/// The stage `t` seconds after the start, with or without the boot.
pub fn stage(t: f32, boot: bool) -> Stage {
    let t = t.max(0.0);
    if boot && t < BOOT {
        return Stage::Boot(t);
    }
    let u = if boot { t - BOOT } else { t };
    let n = (u / PART).floor();
    let local = u - n * PART;
    let n = n as u64;
    let edge = if local < CURTAIN {
        1.0 - local / CURTAIN
    } else if local > PART - CURTAIN {
        (local - (PART - CURTAIN)) / CURTAIN
    } else {
        0.0
    };
    let curtain = edge * edge * (3.0 - 2.0 * edge);
    Stage::Part {
        part: (n % PARTS as u64) as usize,
        local,
        round: (n / PARTS as u64) as u32,
        curtain,
    }
}

/// What a frame shows besides the time.
#[derive(Clone, Copy, Debug)]
pub struct Moment<'a> {
    /// Seconds since the scene started.
    pub t: f32,
    /// The boot screen comes first.
    pub boot: bool,
    /// The time of day, `HH:MM`.
    pub clock: &'a str,
    /// The host for the greetings, only with `show_hostname`.
    pub host: Option<&'a str>,
    /// The tune that plays (`c64-sid`), shown in the upper border.
    pub tune: Option<&'a str>,
    /// The beat, 0 to 1 (`c64-sid`).
    pub beat: f32,
}

/// The scroller's text. Its length never changes with the time.
pub fn greeting(clock: &str, host: Option<&str>) -> String {
    let mut text = format!(
        "NIRUX 64 PRESENTS AN IDLE SCREEN FOR A MINIMAL X11 DESKTOP ...    \
         THE TIME IS {clock} ...    "
    );
    if let Some(host) = host {
        let host = pixfont::plain(host).to_uppercase();
        if !host.trim().is_empty() {
            text.push_str(&format!("HAIL FROM {} ...    ", host.trim()));
        }
    }
    text.push_str(
        "GREETINGS TO NIRUWM, NIRUBAR, NIRULOCK, NIRUSID, NIRUDEMO, NIRUAMP, \
         NIRUTERM AND EVERY SCENER STILL CODING FOR THE BREADBIN ...    \
         NINE WORLDS, ONE TREE: YGGDRASIL ...    \
         PRESS ANY KEY TO RETURN ...    SKAL!          ",
    );
    text
}

/// Draw the frame for `m` into `cv`, sized for `g`.
pub fn frame(cv: &mut Canvas, g: &Geometry, th: &Theme, m: &Moment) {
    cv.fit(g.width, g.height);
    match stage(m.t, m.boot) {
        Stage::Boot(t) => boot(cv, g, th, t),
        Stage::Part {
            part,
            local,
            round,
            curtain,
        } => {
            for y in 0..cv.height {
                let c = border_colour(part, y, g, th, m.t, m.beat);
                cv.row(y, 0, cv.width, c);
            }
            {
                let mut s = Scr::new(cv, g);
                s.clear(0);
                match part {
                    0 => nirux(&mut s, th, local, round, m),
                    1 => bifrost(&mut s, th, local, m),
                    _ => yggdrasil(&mut s, th, m),
                }
            }
            if part == 2 {
                let line = format!("YGGDRASIL   {}", m.clock);
                caption(cv, g, &line, false, shade(th.wash, 0.8));
            }
            if let Some(tune) = m.tune {
                caption(cv, g, tune, true, 15);
            }
            close(cv, g, th, part, m, curtain);
        }
    }
}

/// The border's colour on canvas row `y`.
fn border_colour(part: usize, y: i32, g: &Geometry, th: &Theme, t: f32, beat: f32) -> u8 {
    match part {
        0 => {
            // Three raster bars sweeping the whole height.
            let mut c = 0;
            let half = g.height as f32 / 2.0;
            let size = 15.0 + beat * 6.0;
            for i in 0..3 {
                let centre = half + (half - 10.0) * (t * 0.9 + i as f32 * 2.1).sin();
                let d = (y as f32 + 0.5 - centre) / (size / 2.0);
                if d.abs() < 1.0 {
                    let level = (1.0 - d.abs()).powf(0.7) * (0.85 + beat * 0.3);
                    c = shade(th.bars, level.max(0.18));
                }
            }
            c
        }
        1 => {
            // Two rainbow bars, two rows a colour, crossing.
            let size = RAINBOW.len() as f32 * 2.0;
            let half = g.height as f32 / 2.0;
            let mut c = 0;
            for i in 0..2 {
                let centre = half + (half - size) * (t * 0.7 + i as f32 * PI).sin();
                let top = centre - size / 2.0;
                let k = ((y as f32 - top) / 2.0).floor();
                if (0.0..RAINBOW.len() as f32).contains(&k) {
                    c = RAINBOW[k as usize];
                }
            }
            c
        }
        _ => {
            // Black with a thin split line at the screen's edges.
            let edge = y == g.sy - 2 || y == g.sy + SCREEN_H + 1;
            if edge {
                shade(th.wash, 0.6)
            } else if beat > 0.6 {
                th.logo[1]
            } else {
                0
            }
        }
    }
}

/// A line of text centred in the upper or lower border.
fn caption(cv: &mut Canvas, g: &Geometry, text: &str, upper: bool, colour: u8) {
    let text = pixfont::plain(text).to_uppercase();
    let room = (cv.width / 8 - 2).max(0) as usize;
    let text: String = text.trim().chars().take(room).collect();
    let border = if upper {
        g.sy
    } else {
        cv.height - (g.sy + SCREEN_H)
    };
    if border < 10 {
        return;
    }
    let y = if upper {
        (g.sy - 8) / 2
    } else {
        g.sy + SCREEN_H + (border - 8) / 2
    };
    let x = (cv.width - text.chars().count() as i32 * 8) / 2;
    glyphs(&text, x + 1, y + 1, 1, |px, py, _, _| cv.set(px, py, 0));
    glyphs(&text, x, y, 1, |px, py, _, _| cv.set(px, py, colour));
}

/// The curtain: the border closes over the screen from the top and the
/// bottom while the picture fades.
fn close(cv: &mut Canvas, g: &Geometry, th: &Theme, part: usize, m: &Moment, curtain: f32) {
    if curtain <= 0.0 {
        return;
    }
    let steps = (curtain * 4.0).round() as u32;
    let cover = (curtain * SCREEN_H as f32 / 2.0).round() as i32;
    let (x0, x1) = (g.sx.max(0), (g.sx + SCREEN_W).min(cv.width));
    for r in 0..SCREEN_H {
        let y = g.sy + r;
        if y < 0 || y >= cv.height {
            continue;
        }
        if r < cover || r >= SCREEN_H - cover {
            let c = border_colour(part, y, g, th, m.t, m.beat);
            cv.row(y, x0, x1, c);
        } else if steps > 0 {
            let base = (y * cv.width) as usize;
            for p in &mut cv.pixels[base + x0 as usize..base + x1 as usize] {
                for _ in 0..steps {
                    *p = DARKER[*p as usize];
                }
            }
        }
    }
}

// ---- the boot -------------------------------------------------------------

const BOOT_LINES: [(f32, &str); 3] = [
    (0.0, "    **** NIRUX 64 BASIC V2 ****"),
    (0.0, " 64K RAM SYSTEM  38911 BASIC BYTES FREE"),
    (0.0, "READY."),
];

fn boot(cv: &mut Canvas, g: &Geometry, th: &Theme, t: f32) {
    let loading = (2.9..5.4).contains(&t);
    for y in 0..cv.height {
        let c = if loading {
            // Loader stripes in the theme's colours, rolling upwards.
            let k = ((y as f32 + t * 140.0) / 3.0).floor() as u32;
            let h = k.wrapping_mul(0x9e37_79b1).rotate_left(7) ^ (k >> 3);
            let r = th.bars;
            r[1 + (h % (r.len() as u32 - 1)) as usize]
        } else {
            14
        };
        cv.row(y, 0, cv.width, c);
    }
    let dark = t > BOOT - 0.8;
    let mut s = Scr::new(cv, g);
    s.clear(if dark { 0 } else { 6 });
    if dark {
        return;
    }
    let mut lines: Vec<(i32, String)> = BOOT_LINES
        .iter()
        .zip([1, 3, 5])
        .filter(|((at, _), _)| t >= *at)
        .map(|((_, text), row)| (row, (*text).to_string()))
        .collect();
    let typed = |text: &str, from: f32, pace: f32| -> String {
        let n = ((t - from) / pace).floor().max(0.0) as usize;
        text.chars().take(n).collect()
    };
    let load = "LOAD\"NIRUX\",8,1";
    let mut cursor = (6, 0);
    if t >= 0.6 {
        let shown = typed(load, 0.6, 0.085);
        cursor = (6, shown.chars().count() as i32);
        lines.push((6, shown));
    }
    if t >= 2.1 {
        lines.push((8, "SEARCHING FOR NIRUX".into()));
        cursor = (9, 0);
    }
    if t >= 2.7 {
        lines.push((9, "LOADING".into()));
        cursor = (10, 0);
    }
    if t >= 5.4 {
        lines.push((10, "READY.".into()));
        let run = typed("RUN", 5.7, 0.1);
        cursor = (11, run.chars().count() as i32);
        lines.push((11, run));
    }
    for (row, text) in &lines {
        glyphs(text, 0, row * 8, 1, |x, y, _, _| s.set(x, y, 14));
    }
    // The block cursor blinks while the machine waits.
    if !(2.1..5.4).contains(&t) && (t * 3.0) as i32 % 2 == 0 {
        let (row, col) = cursor;
        for y in 0..8 {
            for x in 0..8 {
                s.set(col * 8 + x, row * 8 + y, 14);
            }
        }
    }
}

// ---- part 1: NIRUX --------------------------------------------------------

/// The logo, one character a cell, 9 rows.
const LOGO: [&[&str; 9]; 5] = [
    &[
        "###......##",
        "####.....##",
        "##.##....##",
        "##..##...##",
        "##...##..##",
        "##....##.##",
        "##.....####",
        "##......###",
        "##.......##",
    ],
    &[
        "######", "..##..", "..##..", "..##..", "..##..", "..##..", "..##..", "..##..", "######",
    ],
    &[
        "########..",
        "##.....##.",
        "##......##",
        "##.....##.",
        "########..",
        "##...##...",
        "##....##..",
        "##.....##.",
        "##......##",
    ],
    &[
        "##......##",
        "##......##",
        "##......##",
        "##......##",
        "##......##",
        "##......##",
        "##......##",
        ".##....##.",
        "..######..",
    ],
    &[
        "##......##",
        ".##....##.",
        "..##..##..",
        "...####...",
        "....##....",
        "...####...",
        "..##..##..",
        ".##....##.",
        "##......##",
    ],
];
const LOGO_CELL: i32 = 5;
const LOGO_GAP: i32 = 2;

/// The logo as a bitmap: (width, height, lit cells row by row).
fn logo_bitmap() -> &'static (i32, i32, Vec<bool>) {
    static B: OnceLock<(i32, i32, Vec<bool>)> = OnceLock::new();
    B.get_or_init(|| {
        let cols: i32 = LOGO.iter().map(|l| l[0].len() as i32).sum::<i32>()
            + LOGO_GAP * (LOGO.len() as i32 - 1);
        let mut cells = vec![false; (cols * 9) as usize];
        let mut x0 = 0;
        for letter in LOGO {
            for (r, row) in letter.iter().enumerate() {
                for (c, ch) in row.bytes().enumerate() {
                    if ch == b'#' {
                        cells[r * cols as usize + x0 as usize + c] = true;
                    }
                }
            }
            x0 += letter[0].len() as i32 + LOGO_GAP;
        }
        (cols * LOGO_CELL, 9 * LOGO_CELL, cells)
    })
}

fn nirux(s: &mut Scr, th: &Theme, local: f32, round: u32, m: &Moment) {
    let t = m.t;
    let (lw, lh, cells) = logo_bitmap();
    let (lw, lh) = (*lw, *lh);
    let cols = lw / LOGO_CELL;
    let lit = |c: i32, r: i32| (0..9).contains(&r) && cells[(r * cols + c) as usize];
    // FLD bounce: the logo falls on a floor and leaves it again, squashed
    // a little at the floor and stretched in the air.
    let bounce = (local * 2.1).sin().abs();
    let stretch = 0.82 + 0.3 * bounce + m.beat * 0.12;
    let out_h = (lh as f32 * stretch).round() as i32;
    let floor = 76;
    let top = floor - out_h - (22.0 * bounce) as i32;
    let left = (SCREEN_W - lw) / 2 + (16.0 * (local * 0.8).sin()) as i32;
    for yo in 0..out_h {
        // The line of the logo shown on this row, with a soft wave.
        let v = (yo as f32 / stretch + 2.0 * (yo as f32 * 0.16 + t * 3.0).sin())
            .clamp(0.0, lh as f32 - 1.0) as i32;
        let sheen = 0.62 + 0.38 * (v as f32 * 0.13 - t * 2.6).sin();
        let body = shade(th.logo, sheen.max(0.3));
        let cell_top = v % LOGO_CELL == 0;
        let cell_bottom = v % LOGO_CELL == LOGO_CELL - 1;
        let r = v / LOGO_CELL;
        for col in 0..cols {
            if !lit(col, r) {
                continue;
            }
            let c = if cell_top && !lit(col, r - 1) {
                1
            } else if cell_bottom && !lit(col, r + 1) {
                th.logo[1]
            } else {
                body
            };
            for x in col * LOGO_CELL..(col + 1) * LOGO_CELL {
                // A hard shadow down and right: rows and columns drawn
                // later paint over it where the logo is.
                s.set(left + x + 3, top + yo + 3, 11);
                s.set(left + x, top + yo, c);
            }
        }
    }

    // A colour washed line and a quiet hint.
    let line = "NIRUX 64  *  A MINIMAL X11 DESKTOP";
    let lx = (SCREEN_W - line.len() as i32 * 8) / 2;
    let wash = th.wash;
    let washed: Vec<u8> = (0..line.len())
        .map(|i| {
            let phase = (i as f32 * 0.45 - t * 5.0).rem_euclid(TAU);
            shade(wash, 0.35 + 0.65 * (0.5 + 0.5 * phase.sin()))
        })
        .collect();
    glyphs(line, lx, 90, 1, |x, y, i, _| s.set(x, y, washed[i]));
    let hint = "PRESS ANY KEY";
    let hx = (SCREEN_W - hint.len() as i32 * 8) / 2;
    glyphs(hint, hx, 102, 1, |x, y, _, _| s.set(x, y, 11));

    // The DYCP scroller: every letter at its own height.
    let text = greeting(m.clock, m.host);
    let chars: Vec<char> = text.chars().collect();
    let run = chars.len() as f32 * 16.0 + SCREEN_W as f32;
    let travelled = (round as f32 * PART + local) * SCROLL;
    let pos = travelled.rem_euclid(run);
    let first = ((pos - SCREEN_W as f32) / 16.0).floor().max(0.0) as usize;
    let amp = 13.0 + m.beat * 5.0;
    let rows: [u8; 16] = std::array::from_fn(|row| {
        shade(th.bars, 0.3 + 0.7 * (PI * (row as f32 + 0.5) / 16.0).sin())
    });
    for (k, ch) in chars.iter().enumerate().skip(first) {
        let x = SCREEN_W as f32 - pos + k as f32 * 16.0;
        if x > SCREEN_W as f32 {
            break;
        }
        if x < -16.0 || *ch == ' ' {
            continue;
        }
        let xc = x + 8.0;
        let y = 138.0 + amp * (xc * 0.024 + t * 2.3).sin() + 4.0 * (xc * 0.061 - t * 3.4).sin();
        let (xi, yi) = (x.round() as i32, y.round() as i32 - 8);
        let mut buf = [0u8; 4];
        glyphs(ch.encode_utf8(&mut buf), xi, yi, 2, |px, py, _, _| {
            s.set(px, py, rows[(py - yi).clamp(0, 15) as usize]);
        });
    }
    // Water: the rows above the surface mirrored, rippled and in blue,
    // with darker lines like small waves.
    let surface = 170;
    for r in 0..SCREEN_H - surface {
        let src = surface - 1 - r;
        let dx = (1.4 * (r as f32 * 0.8 - t * 4.0).sin()).round() as i32;
        let y = surface + r;
        let dim = r % 3 == 2 || r > 20;
        for x in 0..SCREEN_W {
            let w = match s.get(x + dx, src) {
                0 => continue,
                1 | 3 | 7 | 13 | 15 if !dim => 14,
                _ => 6,
            };
            s.set(x, y, w);
        }
    }
}

// ---- part 2: BIFROST ------------------------------------------------------

/// N I R U X in Elder Futhark (naudiz, isaz, raido, uruz and gebo for the
/// X), 8 by 12.
const RUNES: [[&str; 12]; 5] = [
    [
        "...##...", "...##...", "...##...", ".####...", "..###...", "...##...", "...###..",
        "...####.", "...##.##", "...##...", "...##...", "...##...",
    ],
    [
        "...##...", "...##...", "...##...", "...##...", "...##...", "...##...", "...##...",
        "...##...", "...##...", "...##...", "...##...", "...##...",
    ],
    [
        "#####...", "##..##..", "##...##.", "##..##..", "####....", "##.##...", "##..##..",
        "##...##.", "##....##", "##......", "##......", "##......",
    ],
    [
        "#####...", "##..##..", "##...##.", "##....##", "##....##", "##....##", "##....##",
        "##....##", "##....##", "##....##", "##....##", "##....##",
    ],
    [
        "##....##", "##....##", ".##..##.", ".##..##.", "..####..", "...##...", "...##...",
        "..####..", ".##..##.", ".##..##.", "##....##", "##....##",
    ],
];

/// A ball: 0 outside, then dark to highlight 1 to 4, 12 by 12.
fn ball() -> &'static [u8; 144] {
    static B: OnceLock<[u8; 144]> = OnceLock::new();
    B.get_or_init(|| {
        let mut b = [0u8; 144];
        for y in 0..12 {
            for x in 0..12 {
                let (dx, dy) = (x as f32 - 5.5, y as f32 - 5.5);
                let r = (dx * dx + dy * dy).sqrt() / 5.8;
                if r > 1.0 {
                    continue;
                }
                let nz = (1.0 - r * r).sqrt();
                let light = (-dx * 0.45 - dy * 0.55) / 5.8 + nz * 0.7;
                b[y * 12 + x] = match light {
                    l if l > 0.9 => 4,
                    l if l > 0.6 => 3,
                    l if l > 0.25 => 2,
                    _ => 1,
                };
            }
        }
        b
    })
}

fn bifrost(s: &mut Scr, th: &Theme, local: f32, m: &Moment) {
    let t = m.t;
    // The runes, with a sheen running down them and a shadow.
    let left = (SCREEN_W - (5 * 16 + 4 * 10)) / 2;
    for pass in 0..2 {
        for (i, rune) in RUNES.iter().enumerate() {
            let x0 = left + i as i32 * 26 + if pass == 0 { 1 } else { 0 };
            let lift = (2.5 * (t * 2.0 + i as f32 * 0.9).sin()).round() as i32;
            let top = 6 + lift + if pass == 0 { 1 } else { 0 };
            for (r, row) in rune.iter().enumerate() {
                for (col, ch) in row.bytes().enumerate() {
                    if ch != b'#' {
                        continue;
                    }
                    for dy in 0..2 {
                        let y = top + r as i32 * 2 + dy;
                        let level = 0.55 + 0.45 * (y as f32 * 0.25 - t * 3.0 + i as f32).sin();
                        let c = if pass == 0 {
                            0
                        } else {
                            shade(th.logo, level.max(0.35))
                        };
                        for dx in 0..2 {
                            s.set(x0 + col as i32 * 2 + dx, y, c);
                        }
                    }
                }
            }
        }
    }

    // Balls on a Lissajous path whose shape changes slowly; the ones with
    // z < 0 pass behind the sphere.
    let (cx, cy) = (160.0, 118.0);
    let ay = 1.6 + 0.5 * (local * 0.11 + 0.7).sin();
    let mut balls: Vec<(f32, i32, i32)> = (0..24)
        .map(|i| {
            let a = t * 1.05 + i as f32 * 0.27;
            let x = cx + 138.0 * a.sin();
            let y = cy + 62.0 * (t * ay + i as f32 * 0.39).sin();
            (a.cos(), x.round() as i32 - 6, y.round() as i32 - 6)
        })
        .collect();
    balls.sort_by(|a, b| a.0.total_cmp(&b.0));
    let draw_ball = |s: &mut Scr, x: i32, y: i32, far: bool| {
        let ramp = th.bars;
        let top = ramp.len() - 1;
        for (k, &v) in ball().iter().enumerate() {
            if v == 0 {
                continue;
            }
            let idx = match v {
                4 => top,
                3 => top.saturating_sub(1),
                2 => (top / 2).max(1),
                _ => 1,
            };
            let mut c = ramp[idx.min(top)];
            if far {
                c = DARKER[c as usize];
            }
            s.set(x + (k % 12) as i32, y + (k / 12) as i32, c);
        }
    };
    for &(z, x, y) in balls.iter().filter(|b| b.0 < 0.0) {
        draw_ball(s, x, y, z < -0.5);
    }

    // The dot sphere.
    let radius = 50.0 * (1.0 + m.beat * 0.08);
    let (ya, xa) = (t * 0.9, 0.45 + 0.3 * (t * 0.37).sin());
    let (sy, cyr) = ya.sin_cos();
    let (sx, cxr) = xa.sin_cos();
    let mut dots: Vec<(f32, i32, i32)> = Vec::with_capacity(320);
    for ring in 1..16 {
        let lat = -PI / 2.0 + PI * ring as f32 / 16.0;
        let (sl, cl) = lat.sin_cos();
        let count = (32.0 * cl).round().max(4.0) as i32;
        for j in 0..count {
            let lon = TAU * j as f32 / count as f32;
            let (x, y, z) = (cl * lon.cos(), sl, cl * lon.sin());
            // Turn about the vertical axis, then tilt.
            let (x, z) = (x * cyr - z * sy, x * sy + z * cyr);
            let (y, z) = (y * cxr - z * sx, y * sx + z * cxr);
            let p = 3.2 / (3.2 + z);
            dots.push((
                z,
                (cx + x * radius * p).round() as i32,
                (cy + y * radius * p).round() as i32,
            ));
        }
    }
    dots.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (z, x, y) in dots {
        let level = 0.5 - z * 0.5;
        let c = shade(th.logo, 0.2 + 0.8 * level);
        if z < 0.0 {
            s.set(x, y, c);
            s.set(x + 1, y, c);
            s.set(x, y + 1, c);
            s.set(x + 1, y + 1, c);
        } else {
            s.set(x, y, c);
        }
    }
    for &(_, x, y) in balls.iter().filter(|b| b.0 >= 0.0) {
        draw_ball(s, x, y, false);
    }

    let line = format!("BIFROST  *  {}", m.clock);
    let lx = (SCREEN_W - line.len() as i32 * 8) / 2;
    let wash = th.wash;
    glyphs(&line, lx, 190, 1, |x, y, i, _| {
        let level = 0.45 + 0.55 * (0.5 + 0.5 * (i as f32 * 0.5 - t * 4.0).sin());
        s.set(x, y, shade(wash, level));
    });
}

// ---- part 3: YGGDRASIL ----------------------------------------------------

/// The world tree as a mask over the screen, drawn once.
fn tree() -> &'static Vec<bool> {
    static T: OnceLock<Vec<bool>> = OnceLock::new();
    T.get_or_init(|| {
        let mut mask = vec![false; (SCREEN_W * SCREEN_H) as usize];
        let mut rng = 0x5eed_1e55_u32;
        let mut rnd = move || {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            (rng % 10_000) as f32 / 10_000.0
        };
        let ground = 158.0;
        let crown = 112.0;
        line(&mut mask, 160.0, ground + 6.0, 160.0, crown, 9.0, 6.0);
        for (angle, len) in [(1.15f32, 30.0f32), (2.0, 30.0), (0.55, 34.0), (2.6, 34.0)] {
            branch(&mut mask, 160.0, crown, angle, len, 5.5, 6, &mut rnd);
        }
        branch(&mut mask, 160.0, crown, PI / 2.0, 26.0, 5.0, 6, &mut rnd);
        for (angle, len) in [
            (-PI / 2.0 - 0.25, 18.0f32),
            (-PI / 2.0 + 0.3, 17.0),
            (-PI + 0.25, 30.0),
            (-0.25, 30.0),
            (-PI + 0.6, 24.0),
            (-0.6, 24.0),
        ] {
            root(&mut mask, 160.0, ground, angle, len, 5.0, 4, &mut rnd);
        }
        mask
    })
}

fn disc(mask: &mut [bool], x: f32, y: f32, r: f32) {
    let ri = r.ceil() as i32;
    for dy in -ri..=ri {
        for dx in -ri..=ri {
            if (dx * dx + dy * dy) as f32 <= r * r + 0.5 {
                let (px, py) = (x.round() as i32 + dx, y.round() as i32 + dy);
                if px >= 0 && py >= 0 && px < SCREEN_W && py < SCREEN_H {
                    mask[(py * SCREEN_W + px) as usize] = true;
                }
            }
        }
    }
}

fn line(mask: &mut [bool], x0: f32, y0: f32, x1: f32, y1: f32, w0: f32, w1: f32) {
    let steps = ((x1 - x0).hypot(y1 - y0)).ceil().max(1.0) as i32;
    for i in 0..=steps {
        let f = i as f32 / steps as f32;
        let w = w0 + (w1 - w0) * f;
        disc(
            mask,
            x0 + (x1 - x0) * f,
            y0 + (y1 - y0) * f,
            (w / 2.0).max(0.5),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn branch(
    mask: &mut [bool],
    x: f32,
    y: f32,
    angle: f32,
    len: f32,
    width: f32,
    depth: u32,
    rnd: &mut impl FnMut() -> f32,
) {
    let (x2, y2) = (x + len * angle.cos(), y - len * angle.sin());
    line(mask, x, y, x2, y2, width, width * 0.7);
    if depth == 0 {
        for _ in 0..3 {
            disc(
                mask,
                x2 + (rnd() - 0.5) * 7.0,
                y2 + (rnd() - 0.5) * 6.0,
                1.6,
            );
        }
        return;
    }
    let spread = 0.38 + rnd() * 0.22;
    // Ash boughs droop a little as they reach out.
    let droop = if angle.cos().abs() > 0.6 { -0.04 } else { 0.0 };
    for side in [-1.0f32, 1.0] {
        let a = angle + side * spread * (0.8 + rnd() * 0.4) + droop;
        let l = len * (0.68 + rnd() * 0.12);
        branch(mask, x2, y2, a, l, width * 0.68, depth - 1, rnd);
    }
    if depth % 2 == 0 && rnd() > 0.45 {
        branch(mask, x2, y2, angle, len * 0.55, width * 0.6, depth - 2, rnd);
    }
}

#[allow(clippy::too_many_arguments)]
fn root(
    mask: &mut [bool],
    x: f32,
    y: f32,
    angle: f32,
    len: f32,
    width: f32,
    depth: u32,
    rnd: &mut impl FnMut() -> f32,
) {
    let (x2, y2) = (x + len * angle.cos(), y - len * angle.sin());
    line(mask, x, y, x2, y2, width, width * 0.6);
    if depth == 0 {
        return;
    }
    for side in [-1.0f32, 1.0] {
        let a = angle + side * (0.3 + rnd() * 0.3);
        root(mask, x2, y2, a, len * 0.7, width * 0.6, depth - 1, rnd);
    }
}

fn yggdrasil(s: &mut Scr, th: &Theme, m: &Moment) {
    // The plasma moves slowly: 25 steps a second are enough, and every
    // other frame is then the same and costs nothing to show.
    let t = (m.t * 25.0).floor() / 25.0 * 0.8;
    // The plasma's colours: the bars up to white, then the washes down.
    let mut cycle: Vec<u8> = th.bars[1..].to_vec();
    cycle.extend(th.wash[1..th.wash.len() - 1].iter().rev());
    let n = cycle.len() as f32;
    let mask = tree();
    // Character cells of 8x8, dithered in 2x2 between neighbours.
    const DITHER: [f32; 4] = [0.125, 0.625, 0.875, 0.375];
    let mut cells = [[0u8; 4]; 40];
    for cy in 0..25 {
        for (cx, cell) in cells.iter_mut().enumerate() {
            let (fx, fy) = (cx as f32, cy as f32);
            let v = (fx * 0.21 + t * 1.1).sin()
                + (fy * 0.27 - t * 0.9).sin()
                + ((fx + fy) * 0.13 + t * 0.7).sin()
                + ((fx - 20.0).hypot(fy - 12.5) * 0.3 - t * 1.6).sin();
            let p = ((v / 4.0 + 1.0) * 0.5 * n * 1.5 + t * 0.6 + m.beat).rem_euclid(n);
            let k = p.floor() as usize % cycle.len();
            let frac = p - p.floor();
            let (a, b) = (cycle[k], cycle[(k + 1) % cycle.len()]);
            *cell = DITHER.map(|d| if frac > d { b } else { a });
        }
        for y in 0..8 {
            let py = cy * 8 + y;
            let lit = &mask[(py * SCREEN_W) as usize..][..SCREEN_W as usize];
            let pattern = ((y & 1) * 2) as usize;
            if let Some(row) = s.row_mut(py) {
                for (px, (dst, &tree)) in row.iter_mut().zip(lit).enumerate() {
                    if !tree {
                        *dst = cells[px >> 3][pattern + (px & 1)];
                    }
                }
            } else {
                for (px, &tree) in lit.iter().enumerate() {
                    if !tree {
                        s.set(px as i32, py, cells[px >> 3][pattern + (px & 1)]);
                    }
                }
            }
        }
    }
    // The tree stays black (the screen was cleared to black).
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(cv: &Canvas) -> u64 {
        cv.pixels.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &p| {
            (h ^ u64::from(p)).wrapping_mul(0x100_0000_01b3)
        })
    }

    fn moment(t: f32) -> Moment<'static> {
        Moment {
            t,
            boot: true,
            clock: "12:34",
            host: None,
            tune: None,
            beat: 0.0,
        }
    }

    #[test]
    fn the_scale_is_whole_and_the_screen_centred_on_the_monitor_pixel() {
        for (w, h, scale) in [
            (1920u16, 1080u16, 4),
            (3840, 2160, 9),
            (1366, 768, 3),
            (1280, 1024, 3),
            (2560, 1440, 6),
            (800, 600, 2),
            (300, 200, 1),
        ] {
            let g = geometry(w, h);
            assert_eq!(g.scale, scale, "{w}x{h}");
            assert!((0..g.scale).contains(&g.phase_x) && (0..g.scale).contains(&g.phase_y));
            // The buffer covers the monitor, and no more than a pixel more.
            assert!(g.width * g.scale - g.phase_x >= i32::from(w));
            assert!((g.width - 1) * g.scale - g.phase_x < i32::from(w));
            assert!(g.height * g.scale - g.phase_y >= i32::from(h));
            // The screen's left edge in monitor pixels is centred.
            let left = g.sx * g.scale - g.phase_x;
            let expected = (i32::from(w) - SCREEN_W * g.scale) / 2;
            assert_eq!(left, expected, "{w}x{h}");
            if g.scale > 1 {
                assert!(g.sx >= MIN_BORDER && g.sy >= MIN_BORDER, "{g:?}");
            }
        }
    }

    #[test]
    fn a_frame_depends_only_on_the_time() {
        let g = geometry(1920, 1080);
        let th = Theme::new(0x8aa87e, 0xd8ded2);
        for t in [0.5, 3.5, 9.0, 20.0, 35.0, 60.0, 100.0] {
            let mut a = Canvas::default();
            let mut b = Canvas::default();
            frame(&mut a, &g, &th, &moment(t));
            frame(&mut b, &g, &th, &moment(t));
            assert_eq!(hash(&a), hash(&b), "t = {t}");
            assert_eq!(a.pixels.len(), (g.width * g.height) as usize);
            assert!(a.pixels.iter().all(|&p| p < 16));
        }
        let mut a = Canvas::default();
        let mut b = Canvas::default();
        frame(&mut a, &g, &th, &moment(20.0));
        frame(&mut b, &g, &th, &moment(20.04));
        assert_ne!(hash(&a), hash(&b), "it moves");
    }

    #[test]
    fn every_part_draws_something_on_the_screen_and_in_the_border() {
        let g = geometry(1920, 1080);
        let th = Theme::new(0x5a5a60, 0xe5e5e5);
        for (part, t) in [
            (0usize, BOOT + 6.0),
            (1, BOOT + PART + 6.0),
            (2, BOOT + 2.0 * PART + 6.0),
        ] {
            assert!(
                matches!(stage(t, true), Stage::Part { part: p, curtain, .. } if p == part && curtain == 0.0)
            );
            let mut cv = Canvas::default();
            frame(&mut cv, &g, &th, &moment(t));
            let screen_lit = (0..SCREEN_H)
                .flat_map(|y| (0..SCREEN_W).map(move |x| (x, y)))
                .filter(|(x, y)| cv.pixels[((g.sy + y) * cv.width + g.sx + x) as usize] != 0)
                .count();
            assert!(screen_lit > 2000, "part {part}: {screen_lit}");
        }
        // The border bars of part 1 are in the border, left of the screen.
        let mut cv = Canvas::default();
        frame(&mut cv, &g, &th, &moment(BOOT + 6.0));
        assert!((0..cv.height).any(|y| cv.pixels[(y * cv.width) as usize] != 0));
    }

    #[test]
    fn the_boot_comes_once_and_the_parts_loop_with_curtains() {
        assert_eq!(stage(1.0, true), Stage::Boot(1.0));
        assert!(matches!(stage(1.0, false), Stage::Part { part: 0, .. }));
        let at = |t| match stage(t, true) {
            Stage::Part {
                part,
                round,
                curtain,
                ..
            } => (part, round, curtain),
            Stage::Boot(_) => panic!("boot at {t}"),
        };
        assert_eq!(at(BOOT).2, 1.0, "opens from closed");
        assert_eq!(at(BOOT + PART * 0.5), (0, 0, 0.0));
        assert_eq!(at(BOOT + PART * 1.5), (1, 0, 0.0));
        assert_eq!(at(BOOT + PART * 2.5), (2, 0, 0.0));
        assert_eq!(at(BOOT + PART * 3.5), (0, 1, 0.0));
        assert!(at(BOOT + PART - 0.01).2 > 0.95, "closes at the end");
    }

    #[test]
    fn the_border_closes_over_the_screen() {
        let g = geometry(1920, 1080);
        let th = Theme::new(0x8aa87e, 0xd8ded2);
        let mut cv = Canvas::default();
        // Fully closed: the screen rows are border rows, same colour
        // across the row as at the left edge.
        frame(&mut cv, &g, &th, &moment(BOOT + PART - 0.0001));
        for r in 0..SCREEN_H {
            let y = g.sy + r;
            let left = cv.pixels[(y * cv.width) as usize];
            let mid = cv.pixels[(y * cv.width + g.sx + 160) as usize];
            assert_eq!(left, mid, "row {r}");
        }
    }

    #[test]
    fn the_time_is_in_the_greeting_and_the_length_stays() {
        let a = greeting("09:05", None);
        let b = greeting("23:59", None);
        assert!(a.contains("THE TIME IS 09:05"));
        assert_eq!(a.len(), b.len());
        assert!(!a.contains("HAIL"));
        let c = greeting("09:05", Some("Yggdrasil"));
        assert!(c.contains("HAIL FROM YGGDRASIL"));
        assert!(pixfont::plain(&a) == a, "the font has every character");
    }

    #[test]
    fn theme_colours_map_to_c64_ramps() {
        // Niru Noir: grey accent, grey text: steel with the C64's blue.
        let noir = Theme::new(0x5a5a60, 0xe5e5e5);
        assert_eq!(noir.logo, STEEL);
        assert_eq!(noir.bars, BLUE);
        assert_ne!(noir.wash, noir.bars);
        // A green accent.
        let forest = Theme::new(0x8aa87e, 0xd8ded2);
        assert_eq!(forest.logo, GREEN);
        assert_eq!(ramp_for(0xd04040), FIRE);
        assert_eq!(ramp_for(0x4060e0), BLUE);
        assert_eq!(ramp_for(0xa050c0), PURPLE);
        for th in [noir, forest, Theme::new(0xd04040, 0x40d0d0)] {
            assert_ne!(th.logo, th.wash);
            for r in [th.logo, th.bars, th.wash] {
                assert_eq!(r[0], 0);
                assert_eq!(*r.last().unwrap(), 1);
            }
        }
    }

    #[test]
    fn the_fade_table_reaches_black() {
        assert_eq!(lut(0), C64);
        assert!(lut(4).iter().all(|&c| c == 0));
    }

    #[test]
    fn the_logo_and_the_runes_are_well_formed() {
        for letter in LOGO {
            let w = letter[0].len();
            assert!(letter.iter().all(|r| r.len() == w));
        }
        let (w, h, _) = *logo_bitmap();
        assert!(w <= SCREEN_W - 32 && h == 45, "{w}x{h}");
        for rune in RUNES {
            assert!(rune.iter().all(|r| r.len() == 8));
        }
        // The tree is a silhouette with room around it.
        let lit = tree().iter().filter(|&&b| b).count();
        assert!(lit > 2500 && lit < 30000, "{lit}");
    }

    #[test]
    fn a_frame_fits_the_budget() {
        // Release builds are many times faster; this is the debug build.
        let g = geometry(1920, 1080);
        let th = Theme::new(0x8aa87e, 0xd8ded2);
        let mut cv = Canvas::default();
        let start = std::time::Instant::now();
        let mut frames = 0;
        for part in 0..3 {
            for k in 0..20 {
                let t = BOOT + part as f32 * PART + 3.0 + k as f32 * 0.02;
                frame(&mut cv, &g, &th, &moment(t));
                frames += 1;
            }
        }
        let each = start.elapsed() / frames;
        assert!(each < std::time::Duration::from_millis(40), "{each:?}");
    }
}

#[cfg(test)]
mod bench {
    use super::*;

    /// `cargo test --release c64_bench -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn c64_bench() {
        let g = geometry(1920, 1080);
        let th = Theme::new(0x8aa87e, 0xd8ded2);
        let mut cv = Canvas::default();
        for part in 0..3 {
            let start = std::time::Instant::now();
            for k in 0..500 {
                let m = Moment {
                    t: BOOT + part as f32 * PART + 3.0 + k as f32 * 0.02,
                    boot: true,
                    clock: "12:34",
                    host: None,
                    tune: None,
                    beat: 0.0,
                };
                frame(&mut cv, &g, &th, &m);
            }
            eprintln!("part {part}: {:?} a frame", start.elapsed() / 500);
        }
    }
}
