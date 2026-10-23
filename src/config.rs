//! `~/.config/niruui/screensaver.toml`
//!
//! The NIRU desktop programs share one directory for their settings so they
//! can be found in one place, but each program has its own file and never
//! reads the others'. That means installing one, two or all of them is fine.

use crate::{palette, Result};
use std::path::Path;

pub const DEFAULT_PATH: &str = "screensaver.toml";

#[derive(Clone, Debug)]
pub struct Config {
    pub font_path: Option<String>,
    pub font_size: f32,
    pub clock_size: f32,
    /// Start the screensaver after this many minutes of idle time.
    pub idle_minutes: u32,
    /// Lock the screen this many minutes after the screensaver appeared.
    /// 0 disables locking. Read by NIRUIDLE.
    pub lock_after_minutes: u32,
    /// Turn the display off this many minutes after the screen locked.
    /// 0 disables it. Read by NIRUIDLE.
    pub dpms_off_minutes: u32,
    /// Deprecated compatibility input. Never dismisses a lock screen.
    pub lock_timeout_minutes: u32,
    /// Show "away for ..." once the screen has been idle this long.
    pub show_away_after_seconds: u32,
    pub palette: palette::Palette,
}

/// The built-in suite themes, for a component that does not carry the
/// theme files itself. Only the text colours are used: the saver and locker
/// keep an opaque black background so nothing behind them can show through.
fn builtin_palette(name: &str) -> Option<palette::Palette> {
    let (foreground, muted, border, focus) = match name {
        "niru-noir" => (0xe5e5e5, 0xa8a8a8, 0x2a2a2d, 0x5a5a60),
        "niru-nord" => (0xeceff4, 0x9aa5b1, 0x3b4252, 0x88c0d0),
        "niru-satie" => (0xdedad4, 0x8f8b86, 0x2a2724, 0xb08d7d),
        "niru-carl-larsson" => (0x3b3228, 0x7d6f5f, 0xd8cdbb, 0xa63d2f),
        "niru-forest-vigil" => (0xd8ded2, 0x96a292, 0x2b362c, 0x8aa87e),
        "niru-blood-of-urdr" => (0xe8d6cf, 0xb09a92, 0x3a1d1b, 0xc96050),
        "niru-root-and-stone" => (0xeadfd2, 0xb5a08c, 0x3a2a1e, 0xc19768),
        "niru-blue-horizon" => (0xdce6eb, 0x93a6b2, 0x243846, 0x86aec8),
        _ => return None,
    };
    Some(palette::Palette {
        background: 0x000000,
        foreground,
        muted,
        border,
        focus,
    })
}

/// Versioned appearance contract. Invalid input is ignored; the working theme
/// stays in effect. The saver always renders on opaque black.
fn appearance_value(base: &Path) -> Option<toml::Value> {
    let path = base.join("niruwm/appearance.toml");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            eprintln!("niruscreensaver: {}: {e}", path.display());
            return None;
        }
    };
    let value: toml::Value = match text.parse() {
        Ok(value) => value,
        Err(e) => {
            eprintln!("niruscreensaver: {}: {e}", path.display());
            return None;
        }
    };
    if value.get("schema").and_then(|v| v.as_integer()) != Some(1) {
        eprintln!(
            "niruscreensaver: {}: unsupported appearance schema",
            path.display()
        );
        return None;
    }
    if let Some(colors) = value.get("colors") {
        let Some(colors) = colors.as_table() else {
            eprintln!(
                "niruscreensaver: {}: colors must be a table",
                path.display()
            );
            return None;
        };
        for colour in colors.values() {
            let Some(text) = colour.as_str() else {
                eprintln!(
                    "niruscreensaver: {}: colours must be strings",
                    path.display()
                );
                return None;
            };
            if palette::parse_hex(text).is_none() {
                eprintln!(
                    "niruscreensaver: {}: invalid colour {text:?}",
                    path.display()
                );
                return None;
            }
        }
    }
    if let Some(size) = value.get("font").and_then(|font| font.get("size")) {
        if !size
            .as_float()
            .is_some_and(|size| size.is_finite() && size > 0.0)
        {
            eprintln!(
                "niruscreensaver: {}: font.size must be positive",
                path.display()
            );
            return None;
        }
    }
    Some(value)
}

fn appearance_colour(appearance: &toml::Value, key: &str) -> Option<String> {
    appearance
        .get("colors")?
        .get(key)?
        .as_str()
        .map(str::to_string)
}

/// Read the suite's selected theme without requiring it to exist.
///
/// A user theme in `~/.config/niruwm/themes/NAME.toml` wins over the built-in
/// palette, exactly as it does in NIRUWM and NIRUBAR. The background stays
/// black for the saver and locker; only the content colours follow the theme.
fn suite_palette_from(base: &Path) -> Option<palette::Palette> {
    let config = std::fs::read_to_string(base.join("niruwm/config.toml")).ok()?;
    let value: toml::Value = config.parse().ok()?;
    let name = value
        .get("theme")
        .and_then(|v| v.as_str())
        .unwrap_or("niru-noir");
    let file = base.join("niruwm/themes").join(format!("{name}.toml"));
    if !file.exists() {
        if let Some(appearance) = appearance_value(base) {
            let mut result = builtin_palette(name).unwrap_or_default();
            result.background = 0x000000;
            for (field, key) in [
                (&mut result.foreground, "foreground"),
                (&mut result.muted, "muted"),
                (&mut result.border, "border"),
                (&mut result.focus, "focus"),
            ] {
                if let Some(colour) =
                    appearance_colour(&appearance, key).and_then(|s| palette::parse_hex(&s))
                {
                    *field = colour;
                }
            }
            return Some(result);
        }
    }
    if let Ok(text) = std::fs::read_to_string(&file) {
        if let Ok(theme) = text.parse::<toml::Value>() {
            let mut result = builtin_palette(name).unwrap_or_default();
            result.background = 0x000000;
            let get = |field: &str, fallback: u32| {
                theme
                    .get(field)
                    .and_then(|v| v.as_str())
                    .and_then(palette::parse_hex)
                    .unwrap_or(fallback)
            };
            result.foreground = get("foreground", result.foreground);
            result.muted = get("muted", result.muted);
            result.border = get("border", result.border);
            result.focus = get("focus", result.focus);
            return Some(result);
        }
    }
    builtin_palette(name)
}

fn suite_base() -> std::path::PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
        })
}

fn suite_palette() -> Option<palette::Palette> {
    suite_palette_from(&suite_base())
}

impl Default for Config {
    fn default() -> Self {
        Self {
            font_path: None,
            font_size: 22.0,
            clock_size: 88.0,
            idle_minutes: 15,
            lock_after_minutes: 15,
            dpms_off_minutes: 15,
            lock_timeout_minutes: 0,
            show_away_after_seconds: 60,
            palette: palette::Palette::default(),
        }
    }
}

fn config_file() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
        });
    base.join("niruui").join(DEFAULT_PATH)
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = config_file();
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let empty = toml::Value::Table(Default::default());
                return Self::from_value_with(&empty, suite_palette());
            }
            Err(e) => return Err(format!("{}: {e}", path.display()).into()),
        };
        let value: toml::Value = text
            .parse()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        Self::from_value_with(&value, suite_palette())
            .map_err(|e| format!("{}: {e}", path.display()).into())
    }

    #[cfg(test)]
    fn from_value(v: &toml::Value) -> Result<Self> {
        Self::from_value_with(v, None)
    }

    fn from_value_with(v: &toml::Value, suite: Option<palette::Palette>) -> Result<Self> {
        let mut cfg = Self::default();
        if let Some(suite) = suite {
            cfg.palette = suite;
        }
        if let Some(t) = v.get("font_path").and_then(|x| x.as_str()) {
            cfg.font_path = Some(t.to_string());
        }
        if let Some(n) = v.get("font_size").and_then(|x| x.as_float()) {
            cfg.font_size = n as f32;
        }
        if let Some(n) = v.get("clock_size").and_then(|x| x.as_float()) {
            cfg.clock_size = n as f32;
        }
        if let Some(n) = v.get("idle_minutes").and_then(|x| x.as_integer()) {
            cfg.idle_minutes = n.clamp(0, 100_000) as u32;
        }
        if let Some(n) = v.get("lock_after_minutes").and_then(|x| x.as_integer()) {
            cfg.lock_after_minutes = n.clamp(0, 100_000) as u32;
        }
        if let Some(n) = v.get("dpms_off_minutes").and_then(|x| x.as_integer()) {
            cfg.dpms_off_minutes = n.clamp(0, 100_000) as u32;
        }
        if let Some(n) = v.get("lock_timeout_minutes").and_then(|x| x.as_integer()) {
            cfg.lock_timeout_minutes = n.clamp(0, 100_000) as u32;
        }
        if let Some(n) = v
            .get("show_away_after_seconds")
            .and_then(|x| x.as_integer())
        {
            cfg.show_away_after_seconds = n.clamp(0, 100_000) as u32;
        }
        if let Some(t) = v.get("palette").and_then(|x| x.as_table()) {
            let get = |name: &str, fallback: u32| -> u32 {
                t.get(name)
                    .and_then(|x| x.as_str())
                    .and_then(palette::parse_hex)
                    .unwrap_or(fallback)
            };
            let d = cfg.palette.clone();
            cfg.palette = palette::Palette {
                background: get("background", d.background),
                foreground: get("foreground", d.foreground),
                muted: get("muted", d.muted),
                border: get("border", d.border),
                focus: get("focus", d.focus),
            };
        }
        if let Some(appearance) = appearance_value(&suite_base()) {
            if v.get("font_size").is_none() {
                if let Some(size) = appearance
                    .get("font")
                    .and_then(|font| font.get("size"))
                    .and_then(|size| size.as_float())
                {
                    cfg.font_size = size as f32;
                }
            }
            if v.get("font_path").is_none() {
                if let Some(path) = appearance
                    .get("font")
                    .and_then(|font| font.get("path"))
                    .and_then(|path| path.as_str())
                {
                    cfg.font_path = Some(path.to_string());
                }
            }
        }
        if !(8.0..=400.0).contains(&cfg.font_size) || !(8.0..=800.0).contains(&cfg.clock_size) {
            return Err("font_size or clock_size out of range".into());
        }
        Ok(cfg)
    }

    pub fn describe(&self) -> String {
        format!(
            "config: {}\nidle after: {} min\nlock {} min later\ndisplay off: {} min after locking\nlegacy lock_timeout_minutes: {} (ignored)\naway text after: {} s\nclock size: {}\nfont: {}",
            config_file().display(),
            self.idle_minutes,
            self.lock_after_minutes,
            if self.dpms_off_minutes == 0 {
                "never".to_string()
            } else {
                self.dpms_off_minutes.to_string()
            },
            self.lock_timeout_minutes,
            self.show_away_after_seconds,
            self.clock_size,
            self.font_path.as_deref().unwrap_or("auto"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_request() {
        let c = Config::default();
        assert_eq!(c.idle_minutes, 15);
        assert_eq!(c.lock_after_minutes, 15);
        assert_eq!(c.dpms_off_minutes, 15);
    }

    #[test]
    fn partial_file_keeps_defaults() {
        let v: toml::Value = "idle_minutes = 5".parse().unwrap();
        let c = Config::from_value(&v).unwrap();
        assert_eq!(c.idle_minutes, 5);
        assert_eq!(c.dpms_off_minutes, 15);
    }

    #[test]
    fn palette_overrides() {
        let v: toml::Value = "[palette]\nbackground = \"101010\"".parse().unwrap();
        let c = Config::from_value(&v).unwrap();
        assert_eq!(c.palette.background, 0x101010);
        assert_eq!(c.palette.foreground, 0xe5e5e5);
    }

    #[test]
    fn suite_theme_supplies_content_colours_but_never_the_background() {
        let dir = std::env::temp_dir().join(format!("saver-suite-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("niruwm/themes")).unwrap();
        std::fs::write(dir.join("niruwm/config.toml"), "theme = \"niru-nord\"\n").unwrap();
        let p = suite_palette_from(&dir).expect("built-in suite theme");
        assert_eq!(p.background, 0x000000, "the saver must stay opaque");
        assert_eq!(p.foreground, 0xeceff4);
        assert_eq!(p.focus, 0x88c0d0);
        std::fs::write(
            dir.join("niruwm/themes/niru-nord.toml"),
            "foreground = \"010203\"\nfocus = \"040506\"\n",
        )
        .unwrap();
        let p = suite_palette_from(&dir).expect("user suite theme");
        assert_eq!(p.foreground, 0x010203);
        assert_eq!(p.focus, 0x040506);
        assert_eq!(p.background, 0x000000);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
