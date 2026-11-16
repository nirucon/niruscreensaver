//! Pictures for the `photos` scene: which files, decoding and scaling.
//!
//! Pure functions without X, so that they can be tested. Pixels are
//! `0xRRGGBB` in a `u32`, rows top to bottom.

use std::path::{Path, PathBuf};

/// A decoded or scaled picture.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u32>,
}

/// Pictures larger than this are skipped rather than decoded (an 8K PNG is
/// 33 million pixels; this allows that and refuses anything absurd).
const MAX_PIXELS: usize = 80_000_000;
/// At most this many pictures are listed.
const MAX_FILES: usize = 5_000;

fn is_picture(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("png" | "jpg" | "jpeg")
    )
}

/// The pictures in `folder` and its subfolders (two levels), sorted by
/// path. When a picture exists as `-4k` and as `-8k` (the NIRUX wallpaper
/// sets), only the 4K one is kept: it is sharp on any screen and decodes in
/// a quarter of the time.
pub fn list(folder: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            if out.len() >= MAX_FILES {
                return;
            }
            let path = entry.path();
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            match entry.file_type() {
                Ok(kind) if kind.is_dir() && depth < 2 => walk(&path, depth + 1, out),
                Ok(kind) if (kind.is_file() || kind.is_symlink()) && is_picture(&path) => {
                    out.push(path)
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(folder, 0, &mut out);
    out.sort();
    let all = out.clone();
    out.retain(|path| {
        let name = path.to_string_lossy();
        !(name.contains("-8k.")
            && all
                .iter()
                .any(|p| *p.to_string_lossy() == name.replace("-8k.", "-4k.")))
    });
    out
}

/// Decode a PNG or JPEG file. A JPEG is decoded at a reduced size (1/2,
/// 1/4 or 1/8) when that still covers `want`, which saves most of the time
/// and memory for camera pictures.
pub fn decode(path: &Path, want: (usize, usize)) -> Result<Picture, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if bytes.starts_with(b"\x89PNG") {
        decode_png(&bytes)
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        decode_jpeg(&bytes, want)
    } else {
        Err("not a PNG or JPEG file".into())
    }
}

fn decode_png(bytes: &[u8]) -> Result<Picture, String> {
    let mut decoder = png::Decoder::new(bytes);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let (width, height) = {
        let info = reader.info();
        (info.width as usize, info.height as usize)
    };
    if width == 0 || height == 0 || width.saturating_mul(height) > MAX_PIXELS {
        return Err("the picture is empty or too large".into());
    }
    let mut buf = vec![0; reader.output_buffer_size()];
    let frame = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let data = &buf[..frame.buffer_size()];
    let channels = match frame.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return Err("unexpanded palette".into()),
    };
    let stride = frame.line_size;
    let mut pixels = Vec::with_capacity(width * height);
    for row in data.chunks(stride).take(height) {
        for px in row.chunks(channels).take(width) {
            pixels.push(match channels {
                1 | 2 => grey(px[0]),
                _ => rgb(px[0], px[1], px[2]),
            });
        }
    }
    finish(width, height, pixels)
}

fn decode_jpeg(bytes: &[u8], want: (usize, usize)) -> Result<Picture, String> {
    let mut decoder = jpeg_decoder::Decoder::new(bytes);
    decoder.read_info().map_err(|e| e.to_string())?;
    let full = decoder.info().ok_or("no JPEG header")?;
    if want.0 > 0 && want.1 > 0 {
        // Cover `want`: the side that needs the most pixels decides.
        let (fw, fh) = (f32::from(full.width), f32::from(full.height));
        let scale = (want.0 as f32 / fw).max(want.1 as f32 / fh).min(1.0);
        let w = (fw * scale).ceil().clamp(1.0, fw) as u16;
        let h = (fh * scale).ceil().clamp(1.0, fh) as u16;
        decoder.scale(w, h).map_err(|e| e.to_string())?;
    }
    let info = decoder.info().ok_or("no JPEG header")?;
    let (width, height) = (usize::from(info.width), usize::from(info.height));
    if width == 0 || height == 0 || width.saturating_mul(height) > MAX_PIXELS {
        return Err("the picture is empty or too large".into());
    }
    let data = decoder.decode().map_err(|e| e.to_string())?;
    let pixels: Vec<u32> = match info.pixel_format {
        jpeg_decoder::PixelFormat::L8 => data.iter().map(|&g| grey(g)).collect(),
        jpeg_decoder::PixelFormat::L16 => data.chunks(2).map(|g| grey(g[0])).collect(),
        jpeg_decoder::PixelFormat::RGB24 => data.chunks(3).map(|p| rgb(p[0], p[1], p[2])).collect(),
        jpeg_decoder::PixelFormat::CMYK32 => data
            .chunks(4)
            .map(|p| {
                // Adobe JPEGs store inverted CMYK; this is the usual approximation.
                let k = u32::from(p[3]);
                let ch = |c: u8| ((u32::from(c) * k) / 255) as u8;
                rgb(ch(p[0]), ch(p[1]), ch(p[2]))
            })
            .collect(),
    };
    finish(width, height, pixels)
}

fn finish(width: usize, height: usize, pixels: Vec<u32>) -> Result<Picture, String> {
    if pixels.len() != width * height {
        return Err("truncated picture".into());
    }
    Ok(Picture {
        width,
        height,
        pixels,
    })
}

fn rgb(r: u8, g: u8, b: u8) -> u32 {
    (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
}

fn grey(v: u8) -> u32 {
    rgb(v, v, v)
}

/// For each output pixel along one axis: the first source pixel and the
/// weights from there. A tent filter as wide as one output pixel in source
/// pixels (at least one), so downscaling averages and upscaling interpolates.
fn weights(src_start: f32, src_len: f32, out: usize, src_size: usize) -> Vec<(usize, Vec<f32>)> {
    let scale = src_len / out as f32;
    let radius = scale.max(1.0);
    (0..out)
        .map(|i| {
            let centre = src_start + (i as f32 + 0.5) * scale;
            let first = ((centre - radius).floor().max(0.0)) as usize;
            let last = ((centre + radius).ceil() as usize).min(src_size);
            let mut taps: Vec<f32> = (first..last)
                .map(|s| (1.0 - ((s as f32 + 0.5) - centre).abs() / radius).max(0.0))
                .collect();
            let sum: f32 = taps.iter().sum();
            if sum > 0.0 {
                taps.iter_mut().for_each(|w| *w /= sum);
                (first, taps)
            } else {
                let nearest = (centre as usize).min(src_size - 1);
                (nearest, vec![1.0])
            }
        })
        .collect()
}

/// Scale `picture` to cover `width` x `height`, cropping the overflow
/// evenly, like a wallpaper in "fill" mode.
pub fn cover(picture: &Picture, width: usize, height: usize) -> Picture {
    let (sw, sh) = (picture.width as f32, picture.height as f32);
    let scale = (width as f32 / sw).max(height as f32 / sh);
    let crop_w = width as f32 / scale;
    let crop_h = height as f32 / scale;
    let x0 = (sw - crop_w) / 2.0;
    let y0 = (sh - crop_h) / 2.0;
    let wx = weights(x0, crop_w, width, picture.width);
    let wy = weights(y0, crop_h, height, picture.height);
    // Only the source rows some output row uses.
    let row_first = wy.first().map_or(0, |w| w.0);
    let row_last = wy
        .iter()
        .map(|(s, t)| s + t.len())
        .max()
        .unwrap_or(0)
        .min(picture.height);
    let rows = row_last.saturating_sub(row_first);
    // Horizontal pass into floating point, then vertical.
    let mut mid = vec![0f32; rows * width * 3];
    for r in 0..rows {
        let src = &picture.pixels[(row_first + r) * picture.width..][..picture.width];
        let dst = &mut mid[r * width * 3..][..width * 3];
        for (x, (start, taps)) in wx.iter().enumerate() {
            let (mut rr, mut gg, mut bb) = (0f32, 0f32, 0f32);
            for (k, w) in taps.iter().enumerate() {
                let p = src[start + k];
                rr += w * ((p >> 16) & 255) as f32;
                gg += w * ((p >> 8) & 255) as f32;
                bb += w * (p & 255) as f32;
            }
            dst[x * 3] = rr;
            dst[x * 3 + 1] = gg;
            dst[x * 3 + 2] = bb;
        }
    }
    let mut pixels = vec![0u32; width * height];
    for (y, (start, taps)) in wy.iter().enumerate() {
        let out = &mut pixels[y * width..][..width];
        for (x, px) in out.iter_mut().enumerate() {
            let (mut rr, mut gg, mut bb) = (0f32, 0f32, 0f32);
            for (k, w) in taps.iter().enumerate() {
                let at = ((start + k - row_first) * width + x) * 3;
                rr += w * mid[at];
                gg += w * mid[at + 1];
                bb += w * mid[at + 2];
            }
            let c = |v: f32| v.round().clamp(0.0, 255.0) as u32;
            *px = (c(rr) << 16) | (c(gg) << 8) | c(bb);
        }
    }
    Picture {
        width,
        height,
        pixels,
    }
}

/// `a` mixed towards `b` by `t` (0.0 is `a`, 1.0 is `b`).
pub fn mix(a: u32, b: u32, t: f32) -> u32 {
    let t = (t.clamp(0.0, 1.0) * 256.0) as u32;
    let ch = |shift: u32| {
        let (x, y) = ((a >> shift) & 255, (b >> shift) & 255);
        ((x * (256 - t) + y * t) >> 8) << shift
    };
    ch(16) | ch(8) | ch(0)
}

/// Darken `buf` (`width` wide) inside the rectangle by up to `amount`, with
/// edges that fade over `feather` pixels: a soft backdrop that keeps text
/// readable over a bright picture.
pub fn shade(
    buf: &mut [u32],
    width: usize,
    rect: (usize, usize, usize, usize),
    amount: f32,
    feather: usize,
) {
    let (rx, ry, rw, rh) = rect;
    let height = buf.len().checked_div(width).unwrap_or(0);
    for y in ry..(ry + rh).min(height) {
        for x in rx..(rx + rw).min(width) {
            let edge = (x - rx)
                .min(rx + rw - 1 - x)
                .min(y - ry)
                .min(ry + rh - 1 - y);
            let t = if feather == 0 {
                1.0
            } else {
                (edge as f32 / feather as f32).min(1.0)
            };
            let at = y * width + x;
            buf[at] = mix(buf[at], 0, amount * t * t * (3.0 - 2.0 * t));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(width: usize, height: usize, f: impl Fn(usize, usize) -> u32) -> Picture {
        Picture {
            width,
            height,
            pixels: (0..height)
                .flat_map(|y| (0..width).map(move |x| (x, y)))
                .map(|(x, y)| f(x, y))
                .collect(),
        }
    }

    #[test]
    fn a_flat_picture_stays_flat_at_any_size() {
        let p = picture(64, 48, |_, _| 0x336699);
        for (w, h) in [(16, 9), (64, 48), (200, 100), (7, 300)] {
            let out = cover(&p, w, h);
            assert_eq!((out.width, out.height), (w, h));
            assert!(out.pixels.iter().all(|&px| px == 0x336699), "{w}x{h}");
        }
    }

    #[test]
    fn cover_crops_the_long_side_evenly() {
        // Left half black, right half white, 200 x 100, into a square:
        // the middle is kept, so the left edge is black, the right white.
        let p = picture(200, 100, |x, _| if x < 100 { 0 } else { 0xffffff });
        let out = cover(&p, 50, 50);
        assert_eq!(out.pixels[25 * 50], 0);
        assert_eq!(out.pixels[25 * 50 + 49], 0xffffff);
        // The seam sits in the middle.
        assert!(out.pixels[25 * 50 + 24] < 0x808080);
        assert!(out.pixels[25 * 50 + 25] > 0x808080);
    }

    #[test]
    fn mix_and_shade() {
        assert_eq!(mix(0xffffff, 0, 0.0), 0xffffff);
        assert_eq!(mix(0xffffff, 0, 1.0), 0);
        assert_eq!(mix(0xff0000, 0x0000ff, 0.5) & 0xff0000, 0x7f0000);
        let mut buf = vec![0xffffffu32; 20 * 10];
        shade(&mut buf, 20, (0, 0, 20, 10), 0.5, 0);
        assert!(buf.iter().all(|&p| p == 0x7f7f7f));
        let mut buf = vec![0xffffffu32; 20 * 10];
        shade(&mut buf, 20, (2, 2, 16, 6), 0.5, 2);
        assert_eq!(buf[0], 0xffffff);
        assert_eq!(buf[2 * 20 + 2], 0xffffff);
        assert_eq!(buf[4 * 20 + 10], 0x7f7f7f);
    }

    #[test]
    fn pictures_are_listed_and_4k_beats_8k() {
        let dir = std::env::temp_dir().join(format!("niruss-list-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        for name in [
            "a-4k.png",
            "a-8k.png",
            "b-8k.png",
            "c.JPG",
            "notes.txt",
            ".hidden.png",
            "sub/d.jpeg",
        ] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let names: Vec<String> = list(&dir)
            .iter()
            .map(|p| p.strip_prefix(&dir).unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a-4k.png", "b-8k.png", "c.JPG", "sub/d.jpeg"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_small_png_decodes() {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[255, 0, 0, 0, 0, 255]).unwrap();
        }
        let p = decode_png(&bytes).unwrap();
        assert_eq!(p.pixels, vec![0xff0000, 0x0000ff]);
        assert!(decode_png(b"\x89PNG broken").is_err());
    }
}
