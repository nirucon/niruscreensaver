//! `ascii` and `ascii-sid` ("NIRUX ascii"): black, with a large NIRUX banner
//! in ASCII and "A MINIMAL X11 DESKTOP" under it, centred on every monitor
//! and sized for each one. The text is typed one character at a time like an
//! old terminal, with a block cursor that blinks once a second when it is
//! done. After a minute it is erased backwards at the same calm pace, the
//! screen stays black for a few seconds, and it is typed again, shifted a
//! little from the centre each time so the same pixels do not stay lit.
//!
//! The banner takes the theme's accent colour when it reads well on black,
//! otherwise the theme's text colour; the line under it is the muted colour.
//! Only the characters that change are drawn and copied to the screen, so it
//! costs about as little as `dark`. With `reduced_motion` the text appears
//! whole with a still cursor and shifts once a minute. `ascii-sid` plays C64
//! SID tunes exactly like `dark-sid` and shows the tune under the text.

use super::{audio, music, random, tune_line, Show};
use crate::config::Config;
use crate::core::{Area, Surface, TextRenderer};
use crate::Result;
use std::os::fd::RawFd;
use std::time::{Duration, Instant};
use x11rb::protocol::xproto::*;

/// The banner, seven rows of `#`, all the same width.
pub const BANNER: [&str; 7] = [
    "##    ##  ####  #######   ##    ##  ##    ##",
    "###   ##   ##   ##    ##  ##    ##   ##  ## ",
    "####  ##   ##   ##    ##  ##    ##    ####  ",
    "## ## ##   ##   #######   ##    ##     ##   ",
    "##  ####   ##   ##  ##    ##    ##    ####  ",
    "##   ###   ##   ##   ##   ##    ##   ##  ## ",
    "##    ##  ####  ##    ##   ######   ##    ##",
];

/// The line under the banner.
pub const TAGLINE: &str = "A MINIMAL X11 DESKTOP";

/// Typing pace for the banner and the line.
const TYPE_BANNER: Duration = Duration::from_millis(28);
const TYPE_LINE: Duration = Duration::from_millis(70);
/// Erasing pace: the same rhythm backwards, a little quicker.
const ERASE_BANNER: Duration = Duration::from_millis(18);
const ERASE_LINE: Duration = Duration::from_millis(45);
/// The pause between the banner and the line, both ways.
const BREATH: Duration = Duration::from_millis(500);
/// How long the finished text stays before it is erased.
const HOLD: Duration = Duration::from_secs(60);
/// Black between erasing and typing again.
const GAP: Duration = Duration::from_secs(4);
/// The cursor's half period: on, then off, for this long each.
const BLINK: Duration = Duration::from_millis(530);
/// The banner's share of each monitor's width.
const SHARE: f32 = 0.7;
/// The largest shift from the centre, as a share of the monitor.
const SHIFT: f32 = 0.025;

/// One character to type: where it sits relative to the text's top left.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Cell {
    ch: char,
    /// 0 is the banner, 1 the line under it.
    row_kind: u8,
    x: i32,
    /// The baseline.
    y: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Typing,
    Hold,
    Erasing,
    Gap,
}

/// The cells of the banner and the line, in typing order (spaces left out),
/// the place of the cursor once everything is typed, the text's width and
/// the line's baseline, for a banner font `adv` pixels per character and
/// `lh` pixels per row and a line font `adv2` per character. The line is
/// centred under the banner.
fn layout(
    adv: f32,
    ascent: i32,
    lh: i32,
    adv2: f32,
    ascent2: i32,
    gap: i32,
) -> (Vec<Cell>, Cell, i32, i32) {
    let mut cells = Vec::new();
    let banner_w = (BANNER[0].chars().count() as f32 * adv).round() as i32;
    for (row, line) in BANNER.iter().enumerate() {
        for (col, ch) in line.chars().enumerate() {
            if ch != ' ' {
                cells.push(Cell {
                    ch,
                    row_kind: 0,
                    x: (col as f32 * adv).round() as i32,
                    y: row as i32 * lh + ascent,
                });
            }
        }
    }
    let line_w = (TAGLINE.chars().count() as f32 * adv2).round() as i32;
    let left = (banner_w - line_w) / 2;
    let base = BANNER.len() as i32 * lh + gap + ascent2;
    for (col, ch) in TAGLINE.chars().enumerate() {
        if ch != ' ' {
            cells.push(Cell {
                ch,
                row_kind: 1,
                x: left + (col as f32 * adv2).round() as i32,
                y: base,
            });
        }
    }
    // One space after the line, where a terminal's cursor would wait.
    let end = Cell {
        ch: ' ',
        row_kind: 1,
        x: left + line_w + (adv2 * 0.5).round() as i32,
        y: base,
    };
    (cells, end, banner_w.max(end.x + adv2.ceil() as i32), base)
}

/// The banner size for a monitor `width` pixels wide: `SHARE` of it, given
/// that a monospace advance is about 0.6 of the size.
fn banner_size(width: u16) -> f32 {
    let columns = BANNER[0].chars().count() as f32;
    (f32::from(width) * SHARE / (columns * 0.6)).clamp(8.0, 96.0)
}

/// Relative luminance of `0xRRGGBB` (WCAG).
fn luminance(colour: u32) -> f32 {
    let channel = |shift: u32| {
        let c = ((colour >> shift) & 0xff) as f32 / 255.0;
        if c <= 0.039_28 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
}

/// WCAG contrast between two colours, 1 to 21.
fn contrast(a: u32, b: u32) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// The banner colour: the theme's accent when it reads well on the
/// background (contrast 4.5 or more), otherwise the text colour.
pub fn banner_colour(accent: u32, foreground: u32, background: u32) -> u32 {
    if contrast(accent, background) >= 4.5 {
        accent
    } else {
        foreground
    }
}

/// The text laid out for one monitor.
struct Screen {
    area: Area,
    banner: TextRenderer,
    line: TextRenderer,
    cells: Vec<Cell>,
    end: Cell,
    /// Pixels per character, banner and line.
    adv: (f32, f32),
    /// Width and height of the banner and the line together.
    text: (i32, i32),
    /// The text's top left, window coordinates.
    origin: (i32, i32),
}

impl Screen {
    fn new(area: &Area, base: &TextRenderer, font_size: f32) -> Self {
        let size = banner_size(area.width);
        let banner = base.at_size(size);
        let line = base.at_size((size * 1.1).max(font_size * 0.9));
        let adv = f32::from(banner.width(&"#".repeat(100))) / 100.0;
        let adv2 = f32::from(line.width(&"A".repeat(100))) / 100.0;
        let lh = i32::from(banner.ascent() + banner.descent());
        let (cells, end, width, line_base) = layout(
            adv,
            i32::from(banner.ascent()),
            lh,
            adv2,
            i32::from(line.ascent()),
            i32::from(line.line_height()),
        );
        let height = line_base + i32::from(line.descent());
        Self {
            area: *area,
            banner,
            line,
            cells,
            end,
            adv: (adv, adv2),
            text: (width, height),
            origin: (i32::from(area.x), i32::from(area.y)),
        }
    }

    /// Centre the text on the monitor, shifted by `r` (any number) at most
    /// `SHIFT` of the monitor each way.
    fn centre(&mut self, r: u64) {
        let a = &self.area;
        let unit = |bits: u64| (bits & 0xffff) as f32 / 65535.0 * 2.0 - 1.0;
        let dx = (unit(r) * SHIFT * f32::from(a.width)) as i32;
        let dy = (unit(r >> 16) * SHIFT * f32::from(a.height)) as i32;
        let x = i32::from(a.x) + (i32::from(a.width) - self.text.0) / 2 + dx;
        let y = i32::from(a.y) + (i32::from(a.height) - self.text.1) / 2 + dy;
        let max_x = i32::from(a.x) + (i32::from(a.width) - self.text.0).max(0);
        let max_y = i32::from(a.y) + (i32::from(a.height) - self.text.1).max(0);
        self.origin = (
            x.clamp(i32::from(a.x), max_x),
            y.clamp(i32::from(a.y), max_y),
        );
    }

    /// The rectangle a cell covers, window coordinates.
    fn rect(&self, cell: &Cell) -> (i16, i16, u16, u16) {
        let (r, adv) = if cell.row_kind == 0 {
            (&self.banner, self.adv.0)
        } else {
            (&self.line, self.adv.1)
        };
        let top = cell.y - i32::from(r.ascent());
        let h = i32::from(r.ascent() + r.descent());
        (
            clamp(self.origin.0 + cell.x),
            clamp(self.origin.1 + top),
            adv.ceil().max(1.0) as u16,
            h.clamp(1, i32::from(u16::MAX)) as u16,
        )
    }

    /// The cell at `index`, or the end when everything is typed.
    fn cell(&self, index: usize) -> Cell {
        self.cells.get(index).copied().unwrap_or(self.end)
    }
}

pub(super) struct Ascii {
    cfg: Config,
    base: TextRenderer,
    info: TextRenderer,
    screens: Vec<Screen>,
    areas: Vec<Area>,
    /// The banner's colour, from the theme.
    accent: u32,
    /// How many cells are typed (the same on every monitor).
    shown: usize,
    phase: Phase,
    cursor_on: bool,
    /// When the next step is due.
    next: Option<Instant>,
    hold_until: Option<Instant>,
    on: bool,
    note: bool,
    music: Option<music::Music>,
    listen: Option<audio::Audio>,
    stop_on_display_off: bool,
    tune: Option<String>,
}

impl Ascii {
    pub(super) fn new(cfg: &Config, text: &TextRenderer, surface: &Surface, sid: bool) -> Self {
        let info = text.at_size((cfg.font_size * 0.8).max(8.0));
        let note = info.has_glyph('♪');
        let settings = super::lock_music_settings();
        let music = sid.then(|| music::Music::start(&settings)).flatten();
        let listen = music
            .is_some()
            .then(|| audio::Audio::start("niruscreensaver", None));
        let p = &cfg.palette;
        let mut me = Self {
            cfg: cfg.clone(),
            base: text.at_size(cfg.font_size),
            info,
            screens: Vec::new(),
            areas: Vec::new(),
            accent: banner_colour(p.focus, p.foreground, p.background),
            shown: 0,
            phase: Phase::Typing,
            cursor_on: true,
            next: Some(Instant::now()),
            hold_until: None,
            on: true,
            note,
            music,
            listen,
            stop_on_display_off: settings.stop_on_display_off,
            tune: None,
        };
        me.fit(&surface.areas);
        if me.cfg.reduced_motion {
            me.shown = me.cells_len();
            me.phase = Phase::Hold;
            me.next = Some(Instant::now() + HOLD);
        }
        me
    }

    fn cells_len(&self) -> usize {
        self.screens.first().map_or(0, |s| s.cells.len())
    }

    /// Lay the text out for every monitor and centre it.
    fn fit(&mut self, areas: &[Area]) {
        self.areas = areas.to_vec();
        self.screens = areas
            .iter()
            .map(|a| Screen::new(a, &self.base, self.cfg.font_size))
            .collect();
        self.shift();
    }

    /// Centre on every monitor with a new small shift.
    fn shift(&mut self) {
        let seed = random();
        for (i, screen) in self.screens.iter_mut().enumerate() {
            screen.centre(seed.rotate_left(i as u32 * 23));
        }
    }

    fn colour(&self, kind: u8) -> u32 {
        if kind == 0 {
            self.accent
        } else {
            self.cfg.palette.muted
        }
    }

    /// Fill cell `index` with `colour` on every monitor.
    fn fill(&self, s: &Surface, index: usize, colour: u32, present: bool) -> Result<()> {
        s.c.change_gc(s.gc, &ChangeGCAux::new().foreground(colour))?;
        for screen in &self.screens {
            let (x, y, width, height) = screen.rect(&screen.cell(index));
            s.c.poly_fill_rectangle(
                s.back,
                s.gc,
                &[Rectangle {
                    x,
                    y,
                    width,
                    height,
                }],
            )?;
            if present {
                s.present_rect(x, y, width, height)?;
            }
        }
        Ok(())
    }

    /// Draw typed cell `index` on every monitor.
    fn draw(&mut self, s: &Surface, index: usize, present: bool) -> Result<()> {
        let bg = self.cfg.palette.background;
        self.fill(s, index, bg, false)?;
        let Some(kind) = self.kind(index) else {
            return Ok(());
        };
        let fg = self.colour(kind);
        for screen in &mut self.screens {
            let cell = screen.cell(index);
            let (ox, oy) = screen.origin;
            let renderer = if kind == 0 {
                &mut screen.banner
            } else {
                &mut screen.line
            };
            renderer.draw_left(
                &s.c,
                s.gc,
                s.back,
                s.depth,
                clamp(ox + cell.x),
                clamp(oy + cell.y),
                &cell.ch.to_string(),
                fg,
                bg,
            )?;
            if present {
                let (x, y, w, h) = screen.rect(&cell);
                s.present_rect(x, y, w, h)?;
            }
        }
        Ok(())
    }

    /// The cursor sits on the next cell to type, or one space after the
    /// line; there is never a typed character under it.
    fn draw_cursor(&self, s: &Surface, on: bool, present: bool) -> Result<()> {
        let colour = if on {
            self.cfg.palette.foreground
        } else {
            self.cfg.palette.background
        };
        self.fill(s, self.shown, colour, present)
    }

    /// The tune line under the text, redrawn whole.
    fn draw_tune(&mut self, s: &Surface, present: bool) -> Result<()> {
        let bg = self.cfg.palette.background;
        let colour = self.cfg.palette.muted;
        let tune = self.tune.clone();
        let line_h = i32::from(self.info.line_height());
        let ascent = i32::from(self.info.ascent());
        for screen in &self.screens {
            let top = screen.origin.1 + screen.text.1 + line_h / 2;
            let (x0, w) = (i32::from(screen.area.x), screen.area.width);
            let h = (line_h * 2).clamp(1, i32::from(u16::MAX)) as u16;
            s.c.change_gc(s.gc, &ChangeGCAux::new().foreground(bg))?;
            s.c.poly_fill_rectangle(
                s.back,
                s.gc,
                &[Rectangle {
                    x: clamp(x0),
                    y: clamp(top),
                    width: w,
                    height: h,
                }],
            )?;
            if let Some(tune) = tune.as_deref() {
                self.info.draw_centred(
                    &s.c,
                    s.gc,
                    s.back,
                    s.depth,
                    clamp(screen.origin.0 + screen.text.0 / 2),
                    clamp(top + line_h / 2 + ascent),
                    tune,
                    colour,
                    bg,
                )?;
            }
            if present {
                s.present_rect(clamp(x0), clamp(top), w, h)?;
            }
        }
        Ok(())
    }

    fn kind(&self, index: usize) -> Option<u8> {
        self.screens
            .first()
            .and_then(|s| s.cells.get(index))
            .map(|c| c.row_kind)
    }

    /// The pause before the next key: the banner is quick, the line slower,
    /// with a breath between them.
    fn key_delay(&self) -> Duration {
        let typed = self.shown.checked_sub(1).and_then(|i| self.kind(i));
        match (typed, self.kind(self.shown)) {
            (Some(0), Some(1)) => BREATH,
            (_, Some(1)) => TYPE_LINE,
            _ => TYPE_BANNER,
        }
    }

    /// The pause before the next erased character: the line first, then a
    /// breath, then the banner.
    fn erase_delay(&self) -> Duration {
        let erased = self.kind(self.shown);
        let next = self.shown.checked_sub(1).and_then(|i| self.kind(i));
        match (erased, next) {
            (Some(1), Some(0)) => BREATH,
            (Some(1), _) | (None, _) => ERASE_LINE,
            _ => ERASE_BANNER,
        }
    }
}

fn clamp(v: i32) -> i16 {
    v.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

impl Show for Ascii {
    fn redraw(&mut self, s: &mut Surface, now: Instant) -> Result<()> {
        if s.areas != self.areas {
            self.fit(&s.areas.clone());
        }
        s.clear(self.cfg.palette.background)?;
        if self.phase != Phase::Gap {
            for i in 0..self.shown.min(self.cells_len()) {
                self.draw(s, i, false)?;
            }
            self.draw_cursor(s, self.cursor_on, false)?;
            self.draw_tune(s, false)?;
        }
        s.present()?;
        if self.on && self.next.is_none() {
            self.next = Some(now);
        }
        Ok(())
    }

    fn step(&mut self, s: &mut Surface, now: Instant) -> Result<()> {
        if self.cfg.reduced_motion {
            // Whole text, still cursor, a new small shift every minute.
            self.shift();
            self.shown = self.cells_len();
            self.phase = Phase::Hold;
            self.cursor_on = true;
            self.next = Some(now + HOLD);
            return self.redraw(s, now);
        }
        match self.phase {
            Phase::Typing => {
                if self.shown < self.cells_len() {
                    self.draw(s, self.shown, true)?;
                    self.shown += 1;
                    self.cursor_on = true;
                    self.draw_cursor(s, true, true)?;
                }
                if self.shown >= self.cells_len() {
                    self.phase = Phase::Hold;
                    self.hold_until = Some(now + HOLD);
                    self.next = Some(now + BLINK);
                } else {
                    self.next = Some(now + self.key_delay());
                }
            }
            Phase::Hold => {
                if self.hold_until.is_some_and(|t| now >= t) {
                    self.cursor_on = true;
                    self.draw_cursor(s, true, true)?;
                    self.phase = Phase::Erasing;
                    self.next = Some(now + BREATH);
                } else {
                    self.cursor_on = !self.cursor_on;
                    self.draw_cursor(s, self.cursor_on, true)?;
                    self.next = Some(now + BLINK);
                }
            }
            Phase::Erasing => {
                if self.shown > 0 {
                    self.draw_cursor(s, false, true)?;
                    self.shown -= 1;
                    self.draw_cursor(s, true, true)?;
                    self.next = Some(now + self.erase_delay());
                } else {
                    self.phase = Phase::Gap;
                    self.next = Some(now + GAP);
                    // Leave nothing behind, the tune included.
                    s.clear(self.cfg.palette.background)?;
                    s.present()?;
                }
            }
            Phase::Gap => {
                self.shift();
                self.phase = Phase::Typing;
                self.cursor_on = true;
                self.draw_cursor(s, true, true)?;
                self.draw_tune(s, true)?;
                self.next = Some(now + BREATH);
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_banner_rows_are_one_width_and_only_hashes() {
        let width = BANNER[0].chars().count();
        for row in BANNER {
            assert_eq!(row.chars().count(), width, "{row:?}");
            assert!(row.chars().all(|c| c == '#' || c == ' '));
        }
    }

    #[test]
    fn the_banner_reads_nirux() {
        // Columns of only spaces split the letters: N I R U X.
        let width = BANNER[0].chars().count();
        let blank: Vec<bool> = (0..width)
            .map(|c| BANNER.iter().all(|r| r.as_bytes()[c] == b' '))
            .collect();
        let mut letters = 0;
        let mut inside = false;
        for b in blank {
            if !b && !inside {
                letters += 1;
            }
            inside = !b;
        }
        assert_eq!(letters, 5);
    }

    #[test]
    fn cells_skip_spaces_and_type_the_banner_before_the_line() {
        let (cells, end, width, base) = layout(10.0, 12, 16, 12.0, 14, 20);
        let hashes: usize = BANNER.iter().map(|r| r.matches('#').count()).sum();
        let letters = TAGLINE.chars().filter(|c| *c != ' ').count();
        assert_eq!(cells.len(), hashes + letters);
        assert!(cells[..hashes]
            .iter()
            .all(|c| c.row_kind == 0 && c.ch == '#'));
        assert!(cells[hashes..]
            .iter()
            .all(|c| c.row_kind == 1 && c.y == base));
        assert_eq!(cells[hashes].ch, 'A');
        assert!(end.x > cells.last().unwrap().x);
        assert!(width >= end.x);
        // The line is centred under the banner.
        let banner_w = (BANNER[0].len() as f32 * 10.0).round() as i32;
        let line_w = (TAGLINE.len() as f32 * 12.0).round() as i32;
        assert_eq!(cells[hashes].x, (banner_w - line_w) / 2);
    }

    #[test]
    fn typing_and_erasing_take_a_few_seconds_each() {
        let hashes: u32 = BANNER.iter().map(|r| r.matches('#').count() as u32).sum();
        let letters = TAGLINE.chars().filter(|c| *c != ' ').count() as u32;
        let typing = TYPE_BANNER * hashes + BREATH + TYPE_LINE * letters;
        let erasing = ERASE_LINE * letters + BREATH + ERASE_BANNER * hashes;
        assert!(
            typing > Duration::from_secs(4) && typing < Duration::from_secs(10),
            "{typing:?}"
        );
        assert!(
            erasing > Duration::from_secs(2) && erasing < typing,
            "{erasing:?}"
        );
        assert!(GAP >= Duration::from_secs(3));
    }

    #[test]
    fn each_monitor_gets_its_own_size() {
        assert!(banner_size(3840) > banner_size(1920));
        assert!(banner_size(1920) > banner_size(1366));
        assert_eq!(banner_size(100), 8.0);
    }

    #[test]
    fn the_accent_is_used_only_when_it_reads_on_black() {
        // Forest Vigil's green reads well; Noir's dark grey does not.
        assert_eq!(banner_colour(0x8aa87e, 0xd8ded2, 0), 0x8aa87e);
        assert_eq!(banner_colour(0x5a5a60, 0xe5e5e5, 0), 0xe5e5e5);
        assert!(contrast(0xffffff, 0) > 20.0);
    }
}
