//! `ascii` and `ascii-sid` ("NIRUX ascii"): black, with a large NIRUX banner
//! in ASCII and "A MINIMAL X11 DESKTOP" under it, typed out one character at
//! a time like an old terminal, with a block cursor that blinks once a second
//! when the text is done. After a minute the text is erased backwards and
//! typed again somewhere else on the monitor, so no pixel stays lit.
//!
//! Only the characters that change are drawn and copied to the screen, so it
//! costs about as little as `dark`. With `reduced_motion` the text appears
//! whole, the cursor stands still and it moves once a minute. `ascii-sid`
//! plays C64 SID tunes exactly like `dark-sid` and shows the tune under the
//! text.

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

/// Typing pace for the banner and the line, and erasing pace.
const TYPE_BANNER: Duration = Duration::from_millis(28);
const TYPE_LINE: Duration = Duration::from_millis(70);
const ERASE: Duration = Duration::from_millis(10);
/// The pause between the banner and the line.
const BREATH: Duration = Duration::from_millis(500);
/// How long the finished text stays before it is erased.
const HOLD: Duration = Duration::from_secs(60);
/// Black between erasing and typing again.
const GAP: Duration = Duration::from_millis(1500);
/// The cursor's half period: on, then off, for this long each.
const BLINK: Duration = Duration::from_millis(530);
/// The banner's share of the narrowest monitor's width.
const SHARE: f32 = 0.7;

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
/// and the place of the cursor once everything is typed, for a banner font
/// `adv` pixels per character and `lh` pixels per row and a line font `adv2`
/// per character. The line is centred under the banner.
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

pub(super) struct Ascii {
    cfg: Config,
    banner: TextRenderer,
    line: TextRenderer,
    info: TextRenderer,
    base_text: TextRenderer,
    /// The banner size the renderers were made for.
    size: f32,
    cells: Vec<Cell>,
    end: Cell,
    /// The text block's size, the tune line included.
    block: (i32, i32),
    /// The line's baseline in the block, for the tune under it.
    line_base: i32,
    /// Where the block's top left is on each monitor, window coordinates.
    origins: Vec<(i32, i32)>,
    areas: Vec<Area>,
    /// How many cells are typed.
    shown: usize,
    phase: Phase,
    cursor_on: bool,
    /// When the hold, gap or blink step is due; with `typing` the next key.
    next: Option<Instant>,
    hold_until: Option<Instant>,
    on: bool,
    note: bool,
    music: Option<music::Music>,
    listen: Option<audio::Audio>,
    stop_on_display_off: bool,
    tune: Option<String>,
    seed: u64,
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
        let mut me = Self {
            cfg: cfg.clone(),
            banner: text.at_size(cfg.font_size),
            line: text.at_size(cfg.font_size),
            info,
            base_text: text.at_size(cfg.font_size),
            size: 0.0,
            cells: Vec::new(),
            end: Cell {
                ch: ' ',
                row_kind: 1,
                x: 0,
                y: 0,
            },
            block: (1, 1),
            line_base: 0,
            origins: Vec::new(),
            areas: Vec::new(),
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
            seed: random(),
        };
        me.fit(&surface.areas);
        if me.cfg.reduced_motion {
            me.shown = me.cells.len();
            me.phase = Phase::Hold;
            me.next = Some(Instant::now() + HOLD);
        }
        me
    }

    /// Size the banner for the narrowest monitor and place the block.
    fn fit(&mut self, areas: &[Area]) {
        self.areas = areas.to_vec();
        let narrow = areas.iter().map(|a| a.width).min().unwrap_or(1024);
        let columns = BANNER[0].chars().count() as f32;
        // A monospace advance is about 0.6 of the size.
        let size = (f32::from(narrow) * SHARE / (columns * 0.6)).clamp(8.0, 72.0);
        if (size - self.size).abs() > 0.5 {
            self.size = size;
            self.banner = self.base_text.at_size(size);
            self.line = self
                .base_text
                .at_size((size * 1.1).max(self.cfg.font_size * 0.9));
        }
        let probe = "#".repeat(100);
        let adv = f32::from(self.banner.width(&probe)) / 100.0;
        let adv2 = f32::from(self.line.width(&"A".repeat(100))) / 100.0;
        let lh = i32::from(self.banner.ascent() + self.banner.descent());
        let gap = i32::from(self.line.line_height());
        let (cells, end, width, line_base) = layout(
            adv,
            i32::from(self.banner.ascent()),
            lh,
            adv2,
            i32::from(self.line.ascent()),
            gap,
        );
        self.cells = cells;
        self.end = end;
        self.line_base = line_base;
        let tune_h = i32::from(self.info.line_height()) * 2;
        self.block = (
            width + 2,
            line_base + i32::from(self.line.descent()) + tune_h + 2,
        );
        self.place();
    }

    /// A new random top left on every monitor, inside it.
    fn place(&mut self) {
        let (w, h) = self.block;
        self.origins = self
            .areas
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let r = self.seed.rotate_left(i as u32 * 23);
                let room_x = (i32::from(a.width) - w).max(0);
                let room_y = (i32::from(a.height) - h).max(0);
                let fx = (r & 0xffff) as f32 / 65535.0;
                let fy = ((r >> 16) & 0xffff) as f32 / 65535.0;
                (
                    i32::from(a.x) + (fx * room_x as f32) as i32,
                    i32::from(a.y) + (fy * room_y as f32) as i32,
                )
            })
            .collect();
    }

    fn renderer(&mut self, kind: u8) -> &mut TextRenderer {
        if kind == 0 {
            &mut self.banner
        } else {
            &mut self.line
        }
    }

    /// The rectangle a cell covers, relative to the block.
    fn cell_rect(&self, cell: &Cell) -> (i32, i32, u16, u16) {
        let r = if cell.row_kind == 0 {
            &self.banner
        } else {
            &self.line
        };
        let adv = if cell.row_kind == 0 {
            f32::from(self.banner.width(&"#".repeat(100))) / 100.0
        } else {
            f32::from(self.line.width(&"A".repeat(100))) / 100.0
        };
        let top = cell.y - i32::from(r.ascent());
        let h = i32::from(r.ascent() + r.descent());
        (
            cell.x,
            top,
            adv.ceil().max(1.0) as u16,
            h.clamp(1, i32::from(u16::MAX)) as u16,
        )
    }

    /// The cell the cursor sits on: the next one to type, or the end.
    fn cursor_cell(&self) -> Cell {
        self.cells.get(self.shown).copied().unwrap_or(self.end)
    }

    fn colour(&self, kind: u8) -> u32 {
        if kind == 0 {
            self.cfg.palette.foreground
        } else {
            self.cfg.palette.muted
        }
    }

    /// Fill one cell with `colour` on every monitor, into the back buffer.
    fn fill(&self, s: &Surface, cell: &Cell, colour: u32, present: bool) -> Result<()> {
        let (x, y, w, h) = self.cell_rect(cell);
        s.c.change_gc(s.gc, &ChangeGCAux::new().foreground(colour))?;
        for &(ox, oy) in &self.origins {
            let (rx, ry) = (clamp(ox + x), clamp(oy + y));
            s.c.poly_fill_rectangle(
                s.back,
                s.gc,
                &[Rectangle {
                    x: rx,
                    y: ry,
                    width: w,
                    height: h,
                }],
            )?;
            if present {
                s.present_rect(rx, ry, w, h)?;
            }
        }
        Ok(())
    }

    /// Draw one typed cell on every monitor.
    fn draw(&mut self, s: &Surface, cell: Cell, present: bool) -> Result<()> {
        let bg = self.cfg.palette.background;
        self.fill(s, &cell, bg, false)?;
        let fg = self.colour(cell.row_kind);
        let origins = self.origins.clone();
        let text = cell.ch.to_string();
        let rect = self.cell_rect(&cell);
        for (ox, oy) in origins {
            self.renderer(cell.row_kind).draw_left(
                &s.c,
                s.gc,
                s.back,
                s.depth,
                clamp(ox + cell.x),
                clamp(oy + cell.y),
                &text,
                fg,
                bg,
            )?;
            if present {
                s.present_rect(clamp(ox + rect.0), clamp(oy + rect.1), rect.2, rect.3)?;
            }
        }
        Ok(())
    }

    fn draw_cursor(&mut self, s: &Surface, on: bool, present: bool) -> Result<()> {
        let cell = self.cursor_cell();
        // Under the cursor there is never a typed character: it sits on the
        // next one to type, or one space after the line.
        let colour = if on {
            self.cfg.palette.foreground
        } else {
            self.cfg.palette.background
        };
        self.fill(s, &cell, colour, present)
    }

    /// The tune line under the text, redrawn whole.
    fn draw_tune(&mut self, s: &Surface, present: bool) -> Result<()> {
        let bg = self.cfg.palette.background;
        let top = self.line_base + i32::from(self.line.descent()) + 1;
        let h = (self.block.1 - top).clamp(1, i32::from(u16::MAX)) as u16;
        let w = self.block.0.clamp(1, i32::from(u16::MAX)) as u16;
        let centre = self.block.0 / 2;
        let baseline = top + i32::from(self.info.line_height()) / 2 + i32::from(self.info.ascent());
        let tune = self.tune.clone();
        let colour = self.cfg.palette.muted;
        for (ox, oy) in self.origins.clone() {
            s.c.change_gc(s.gc, &ChangeGCAux::new().foreground(bg))?;
            s.c.poly_fill_rectangle(
                s.back,
                s.gc,
                &[Rectangle {
                    x: clamp(ox),
                    y: clamp(oy + top),
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
                    clamp(ox + centre),
                    clamp(oy + baseline),
                    tune,
                    colour,
                    bg,
                )?;
            }
            if present {
                s.present_rect(clamp(ox), clamp(oy + top), w, h)?;
            }
        }
        Ok(())
    }

    /// The pause before the next key: the banner is quick, the line slower,
    /// with a breath between them.
    fn key_delay(&self) -> Duration {
        match (
            self.shown.checked_sub(1).map(|i| self.cells[i].row_kind),
            self.cells.get(self.shown),
        ) {
            (Some(0), Some(next)) if next.row_kind == 1 => BREATH,
            (_, Some(next)) if next.row_kind == 1 => TYPE_LINE,
            _ => TYPE_BANNER,
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
            for i in 0..self.shown.min(self.cells.len()) {
                let cell = self.cells[i];
                self.draw(s, cell, false)?;
            }
            if !self.cfg.reduced_motion || self.phase == Phase::Hold {
                self.draw_cursor(s, self.cursor_on, false)?;
            }
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
            // Whole text, still cursor, a new place every minute.
            self.seed = random();
            self.place();
            self.shown = self.cells.len();
            self.phase = Phase::Hold;
            self.cursor_on = true;
            self.next = Some(now + HOLD);
            return self.redraw(s, now);
        }
        match self.phase {
            Phase::Typing => {
                if self.shown < self.cells.len() {
                    let cell = self.cells[self.shown];
                    self.draw(s, cell, true)?;
                    self.shown += 1;
                    self.cursor_on = true;
                    self.draw_cursor(s, true, true)?;
                }
                if self.shown >= self.cells.len() {
                    self.phase = Phase::Hold;
                    self.hold_until = Some(now + HOLD);
                    self.next = Some(now + BLINK);
                } else {
                    self.next = Some(now + self.key_delay());
                }
            }
            Phase::Hold => {
                if self.hold_until.is_some_and(|t| now >= t) {
                    self.cursor_on = false;
                    self.draw_cursor(s, false, true)?;
                    self.phase = Phase::Erasing;
                    self.next = Some(now + ERASE);
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
                    let cell = self.cells[self.shown];
                    self.fill(s, &cell, self.cfg.palette.background, true)?;
                    self.draw_cursor(s, true, true)?;
                    self.next = Some(now + ERASE);
                } else {
                    self.phase = Phase::Gap;
                    self.next = Some(now + GAP);
                    // Leave nothing behind, the tune included.
                    s.clear(self.cfg.palette.background)?;
                    s.present()?;
                }
            }
            Phase::Gap => {
                self.seed = random();
                self.place();
                self.phase = Phase::Typing;
                self.cursor_on = true;
                self.draw_cursor(s, true, true)?;
                self.draw_tune(s, true)?;
                self.next = Some(now + TYPE_BANNER * 8);
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
    fn typing_takes_a_few_seconds() {
        let hashes: u32 = BANNER.iter().map(|r| r.matches('#').count() as u32).sum();
        let letters = TAGLINE.chars().filter(|c| *c != ' ').count() as u32;
        let total = TYPE_BANNER * hashes + BREATH + TYPE_LINE * letters;
        assert!(
            total > Duration::from_secs(4) && total < Duration::from_secs(10),
            "{total:?}"
        );
    }
}
