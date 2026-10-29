//! Shared rendering for NIRUSCREENSAVER and NIRULOCK.
//!
//! Both programs draw the same screen: hostname, a large clock and, while
//! idle, how long the machine has been away. They differ only in what they do
//! with input, which is why the drawing lives here and the input policy does
//! not.
//!
//! The code is duplicated deliberately: each repository builds on its own with
//! no private dependency, which keeps installation simple. The two copies are
//! kept identical; `tests/identical.sh` compares their checksums.

use ab_glyph::{Font, ScaleFont};
use x11rb::{
    connection::Connection,
    protocol::{randr::ConnectionExt as _, xproto::*},
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

/// Text height and glyph cache for one font at one size.
///
/// This is the same technique as NIRUBAR's renderer: rasterise each glyph once
/// into a coverage mask, then blend it against the background on upload. X11
/// has no alpha blending in CopyArea, so pre-blending is the only way to get
/// antialiased text without a compositor.
pub struct TextRenderer {
    font: ab_glyph::FontArc,
    size: f32,
    line_height: i16,
    masks: std::collections::HashMap<char, Mask>,
    image_byte_order: ImageOrder,
}

#[derive(Clone)]
struct Mask {
    width: u16,
    height: u16,
    x: i16,
    y: i16,
    coverage: Vec<u8>,
}

impl TextRenderer {
    pub fn new(
        font_path: &str,
        size: f32,
        setup: &x11rb::protocol::xproto::Setup,
    ) -> crate::Result<Self> {
        let bytes = std::fs::read(font_path)?;
        let font =
            ab_glyph::FontArc::try_from_vec(bytes).map_err(|_| "cannot parse the font file")?;
        let line_height = {
            let scaled = font.as_scaled(ab_glyph::PxScale::from(size));
            scaled.height().ceil() as i16
        };
        Ok(Self {
            font,
            size,
            line_height,
            masks: std::collections::HashMap::new(),
            image_byte_order: setup.image_byte_order,
        })
    }

    pub fn line_height(&self) -> i16 {
        self.line_height
    }

    /// A separate glyph cache at another size, sharing the loaded font bytes.
    pub fn at_size(&self, size: f32) -> Self {
        Self {
            font: self.font.clone(),
            size,
            line_height: self
                .font
                .as_scaled(ab_glyph::PxScale::from(size))
                .height()
                .ceil() as i16,
            masks: std::collections::HashMap::new(),
            image_byte_order: self.image_byte_order,
        }
    }

    /// Width of a string, using the font's advances.
    pub fn width(&self, text: &str) -> i16 {
        let scaled = self.font.as_scaled(ab_glyph::PxScale::from(self.size));
        text.chars()
            .map(|c| scaled.h_advance(self.font.glyph_id(c)))
            .sum::<f32>()
            .ceil() as i16
    }

    /// Draw text centred on `centre_x`, with the baseline at `baseline`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_centred(
        &mut self,
        c: &RustConnection,
        gc: Gcontext,
        target: Drawable,
        depth: u8,
        centre_x: i16,
        baseline: i16,
        text: &str,
        fg: u32,
        bg: u32,
    ) -> crate::Result<()> {
        let width = self.width(text);
        self.draw_left(c, gc, target, depth, centre_x - width / 2, baseline, text, fg, bg)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_left(
        &mut self,
        c: &RustConnection,
        gc: Gcontext,
        target: Drawable,
        depth: u8,
        left_x: i16,
        baseline: i16,
        text: &str,
        fg: u32,
        bg: u32,
    ) -> crate::Result<()> {
        let mut pen = left_x;
        for ch in text.chars() {
            let mask = self.mask(ch);
            self.blend(c, gc, target, depth, &mask, pen, baseline, fg, bg)?;
            pen = pen.saturating_add(self.advance(ch));
        }
        Ok(())
    }

    fn advance(&self, ch: char) -> i16 {
        let scaled = self.font.as_scaled(ab_glyph::PxScale::from(self.size));
        scaled.h_advance(self.font.glyph_id(ch)).ceil() as i16
    }

    fn mask(&mut self, ch: char) -> Mask {
        if let Some(m) = self.masks.get(&ch) {
            return m.clone();
        }
        let id = self.font.glyph_id(ch);
        let glyph = id.with_scale_and_position(self.size, ab_glyph::point(0.0, 0.0));
        let mut out = Mask {
            width: 0,
            height: 0,
            x: 0,
            y: 0,
            coverage: Vec::new(),
        };
        if let Some(outline) = self.font.outline_glyph(glyph) {
            let b = outline.px_bounds();
            let w = b.width().ceil() as u16;
            let h = b.height().ceil() as u16;
            if w > 0 && h > 0 {
                let mut coverage = vec![0u8; usize::from(w) * usize::from(h)];
                outline.draw(|gx, gy, cov| {
                    let px = gx as usize;
                    let py = gy as usize;
                    if px < usize::from(w) && py < usize::from(h) {
                        coverage[py * usize::from(w) + px] =
                            (cov.clamp(0.0, 1.0) * 255.0).round() as u8;
                    }
                });
                out = Mask {
                    width: w,
                    height: h,
                    x: b.min.x.floor() as i16,
                    y: b.min.y.floor() as i16,
                    coverage,
                };
            }
        }
        self.masks.insert(ch, out.clone());
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn blend(
        &self,
        c: &RustConnection,
        gc: Gcontext,
        target: Drawable,
        depth: u8,
        mask: &Mask,
        pen: i16,
        baseline: i16,
        fg: u32,
        bg: u32,
    ) -> crate::Result<()> {
        if mask.width == 0 || mask.height == 0 {
            return Ok(());
        }
        let (fr, fgc, fb) = split(fg);
        let (br, bgc, bb) = split(bg);
        let pixels: Vec<u32> = mask
            .coverage
            .iter()
            .map(|&a| {
                if a == 0 {
                    return bg;
                }
                let a = u32::from(a);
                let mix = |f: u32, b: u32| ((f * a + b * (255 - a)) / 255) & 255;
                (mix(fr, br) << 16) | (mix(fgc, bgc) << 8) | mix(fb, bb)
            })
            .collect();
        let bytes: Vec<u8> = pixels
            .iter()
            .flat_map(|p| {
                if self.image_byte_order == ImageOrder::MSB_FIRST {
                    p.to_be_bytes()
                } else {
                    p.to_le_bytes()
                }
            })
            .collect();
        // A single glyph is small; one upload is enough.
        c.put_image(
            ImageFormat::Z_PIXMAP,
            target,
            gc,
            mask.width,
            mask.height,
            pen.saturating_add(mask.x),
            baseline.saturating_add(mask.y),
            0,
            depth,
            &bytes,
        )?;
        Ok(())
    }
}

fn split(colour: u32) -> (u32, u32, u32) {
    ((colour >> 16) & 255, (colour >> 8) & 255, colour & 255)
}

/// Hostname without the domain, or `unknown`.
pub fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|s| s.trim().to_string())
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "unknown".into())
}

/// Clock text: ISO date and `hh:mm`, Swedish order, English locale.
pub fn clock_text(now: chrono::DateTime<chrono::Local>) -> String {
    format!("{}  {}", now.format("%Y-%m-%d"), now.format("%H:%M"))
}

/// "away for 1 h 04 min" style text for the idle counter.
pub fn away_text(seconds: u64) -> String {
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let rest = minutes % 60;
    if hours == 0 {
        format!("away for {rest} min")
    } else {
        format!("away for {hours} h {rest:02} min")
    }
}

/// A full-screen override-redirect window and the resources to draw on it.
pub struct Surface {
    pub c: RustConnection,
    pub screen_num: usize,
    pub width: u16,
    pub height: u16,
    pub depth: u8,
    pub win: Window,
    pub gc: Gcontext,
    /// Off-screen copy: every frame is composed here and copied once, so the
    /// screen never shows a half-drawn frame.
    pub back: Pixmap,
}

impl Surface {
    pub fn new(name: &str) -> crate::Result<Self> {
        let (c, screen_num) = x11rb::connect(None)?;
        let screen = &c.setup().roots[screen_num];
        let width = screen.width_in_pixels;
        let height = screen.height_in_pixels;
        let depth = screen.root_depth;
        let win = c.generate_id()?;
        let gc = c.generate_id()?;
        let back = c.generate_id()?;
        c.create_window(
            depth,
            win,
            screen.root,
            0,
            0,
            width,
            height,
            0,
            WindowClass::INPUT_OUTPUT,
            screen.root_visual,
            &CreateWindowAux::new()
                // Override redirect: no window manager involvement, so the
                // screen cannot be moved, resized or covered.
                .override_redirect(1)
                // Opaque black, never 0. A pixel value of 0 means "transparent"
                // on a 32-bit visual, which made the idle screen see-through
                // and showed the desktop behind it.
                .background_pixel(0x000000)
                .event_mask(EventMask::EXPOSURE | EventMask::KEY_PRESS | EventMask::BUTTON_PRESS),
        )?;
        c.create_gc(gc, win, &CreateGCAux::new())?;
        c.create_pixmap(depth, back, screen.root, width, height)?;
        c.change_property8(
            PropMode::REPLACE,
            win,
            c.intern_atom(false, b"_NET_WM_NAME")?.reply()?.atom,
            c.intern_atom(false, b"UTF8_STRING")?.reply()?.atom,
            name.as_bytes(),
        )?;
        c.map_window(win)?;
        c.randr_select_input(
            screen.root,
            x11rb::protocol::randr::NotifyMask::SCREEN_CHANGE
                | x11rb::protocol::randr::NotifyMask::CRTC_CHANGE
                | x11rb::protocol::randr::NotifyMask::OUTPUT_CHANGE
                | x11rb::protocol::randr::NotifyMask::RESOURCE_CHANGE,
        )?;
        // Hide the pointer. A glyph cursor with no glyph still drew one pixel,
        // so this is a cursor built from an empty pixmap, which is invisible.
        let blank = c.generate_id()?;
        let shape = c.generate_id()?;
        c.create_pixmap(1, shape, screen.root, 1, 1)?;
        c.create_cursor(blank, shape, shape, 0, 0, 0, 0, 0, 0, 0, 0)?;
        c.change_window_attributes(win, &ChangeWindowAttributesAux::new().cursor(blank))?;
        c.free_cursor(blank)?;
        c.free_pixmap(shape)?;

        // Take the keyboard and the pointer. Without this every key press went
        // to the application underneath, and typing a password meant typing
        // into an editor. The grab also stops clicks from reaching anything
        // else while the screen is covered.
        let keyboard = c
            .grab_keyboard(
                true,
                win,
                x11rb::CURRENT_TIME,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
            )?
            .reply()?;
        if keyboard.status != GrabStatus::SUCCESS {
            return Err(format!("cannot grab the keyboard: {:?}", keyboard.status).into());
        }
        let pointer = c
            .grab_pointer(
                true,
                win,
                EventMask::BUTTON_PRESS | EventMask::POINTER_MOTION,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
                x11rb::NONE,
                x11rb::NONE,
                x11rb::CURRENT_TIME,
            )?
            .reply()?;
        if pointer.status != GrabStatus::SUCCESS {
            let _ = c.ungrab_keyboard(x11rb::CURRENT_TIME);
            return Err(format!("cannot grab the pointer: {:?}", pointer.status).into());
        }
        c.set_input_focus(InputFocus::PARENT, win, x11rb::CURRENT_TIME)?;
        c.flush()?;
        Ok(Self {
            c,
            screen_num,
            width,
            height,
            depth,
            win,
            gc,
            back,
        })
    }

    /// Fill the backing pixmap with a solid colour.
    pub fn clear(&self, colour: u32) -> crate::Result<()> {
        self.c
            .change_gc(self.gc, &ChangeGCAux::new().foreground(colour))?;
        self.c.poly_fill_rectangle(
            self.back,
            self.gc,
            &[Rectangle {
                x: 0,
                y: 0,
                width: self.width,
                height: self.height,
            }],
        )?;
        Ok(())
    }

    /// Follow a root resize or output change and keep full-screen coverage.
    ///
    /// The window and backing pixmap are resized atomically as seen by the
    /// user: one configure request, then a fresh pixmap and a full redraw.
    pub fn resize(&mut self) -> crate::Result<bool> {
        let root = self.c.setup().roots[self.screen_num].root;
        let geometry = self.c.get_geometry(root)?.reply()?;
        let (width, height) = (geometry.width, geometry.height);
        if width == self.width && height == self.height {
            return Ok(false);
        }
        self.c.configure_window(
            self.win,
            &ConfigureWindowAux::new()
                .width(u32::from(width))
                .height(u32::from(height))
                .stack_mode(StackMode::ABOVE),
        )?;
        self.c.free_pixmap(self.back)?;
        let back = self.c.generate_id()?;
        self.c
            .create_pixmap(self.depth, back, root, width, height)?;
        self.back = back;
        self.width = width;
        self.height = height;
        self.c
            .set_input_focus(InputFocus::PARENT, self.win, x11rb::CURRENT_TIME)?;
        self.c.flush()?;
        Ok(true)
    }

    /// Release the keyboard and pointer grabs.
    pub fn release_input(&self) -> crate::Result<()> {
        self.c.ungrab_keyboard(x11rb::CURRENT_TIME)?;
        self.c.ungrab_pointer(x11rb::CURRENT_TIME)?;
        self.c.flush()?;
        Ok(())
    }

    /// Copy the backing pixmap to the screen in one request.
    pub fn present(&self) -> crate::Result<()> {
        self.c.copy_area(
            self.back,
            self.win,
            self.gc,
            0,
            0,
            0,
            0,
            self.width,
            self.height,
        )?;
        self.c.flush()?;
        Ok(())
    }
}

/// Pick a monospace font: an explicit path, then fontconfig, then common paths.
pub fn find_font(explicit: Option<&str>) -> String {
    if let Some(p) = explicit {
        if std::path::Path::new(p).is_file() {
            return p.to_string();
        }
    }
    if let Ok(out) = std::process::Command::new("fc-match")
        .args([
            "-f",
            "%{file}",
            "JetBrainsMono Nerd Font,DejaVu Sans Mono:style=Regular",
        ])
        .output()
    {
        if out.status.success() {
            let path = String::from_utf8_lossy(&out.stdout).to_string();
            if std::path::Path::new(&path).is_file() {
                return path;
            }
        }
    }
    for candidate in [
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
        "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
        "/usr/share/fonts/dejavu/DejaVuSansMono.ttf",
        "/usr/share/fonts/dejavu-fonts-ttf/DejaVuSansMono.ttf",
    ] {
        if std::path::Path::new(candidate).is_file() {
            return candidate.to_string();
        }
    }
    String::new()
}

/// Pick an optional runic font. The lock/saver never requires one: if it is
/// missing, the runic layout simply omits the ornament rather than drawing
/// missing-glyph boxes.
pub fn find_rune_font(explicit: Option<&str>) -> Option<String> {
    if let Some(path) = explicit {
        if std::path::Path::new(path).is_file() {
            return Some(path.to_string());
        }
    }
    if let Ok(out) = std::process::Command::new("fc-match")
        .args(["-f", "%{file}", "Noto Sans Runic:style=Regular"])
        .output()
    {
        if out.status.success() {
            let path = String::from_utf8_lossy(&out.stdout).to_string();
            if std::path::Path::new(&path).is_file() {
                return Some(path);
            }
        }
    }
    for candidate in [
        "/usr/share/fonts/truetype/noto/NotoSansRunic-Regular.ttf",
        "/usr/share/fonts/noto/NotoSansRunic-Regular.ttf",
    ] {
        if std::path::Path::new(candidate).is_file() {
            return Some(candidate.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn away_text_formats_hours_and_minutes() {
        assert_eq!(away_text(0), "away for 0 min");
        assert_eq!(away_text(59), "away for 0 min");
        assert_eq!(away_text(12 * 60), "away for 12 min");
        assert_eq!(away_text(64 * 60), "away for 1 h 04 min");
        assert_eq!(away_text(3 * 3600 + 5 * 60), "away for 3 h 05 min");
    }

    #[test]
    fn clock_is_iso_without_weekday() {
        let now = chrono::Local::now();
        let t = clock_text(now);
        assert_eq!(t.len(), 17);
        assert!(!t.contains("lör"));
    }

    #[test]
    fn split_channels() {
        assert_eq!(split(0x0f10ff), (0x0f, 0x10, 0xff));
    }
}
