//! `niruscreensaver --list`, `--settings`, `--set` and `--shuffle`: read and
//! change `~/.config/niruui/screensaver.toml` from the command line and from
//! NIRULAUNCHER's Screensaver view.
//!
//! The file stays the only description of the screensaver. A change edits
//! one key with `toml_edit`, so comments and every other line are kept; the
//! whole result is loaded as the screensaver would load it before it is
//! written, and it is written in one step. Nothing is restarted: the next
//! time the screensaver appears it reads the file.

use crate::config::{self, Config, Layout, Scene};
use crate::Result;
use std::fs;

pub const USAGE: &str = "\
Usage: niruscreensaver --list
       niruscreensaver --settings
       niruscreensaver --set KEY VALUE
       niruscreensaver --shuffle add|remove|toggle SCENE

Keys for --set:
  scene              clock, photos, dark, dark-sid, ascii, ascii-sid, demo, c64, c64-sid or shuffle
  layout             cava, minimal, terminal, runic or corner (the clock scene)
  visual             auto, music, cava, quotes or none (the clock scene)
  shuffle            scenes separated by commas, such as clock,photos,dark
  photos_folder      a folder of PNG or JPEG pictures
  photos_seconds     5 to 3600
  show_media         true or false
  reduced_motion     true or false
  show_hostname      true or false
  dismiss_on_motion  true or false";

/// The keys `--set` accepts and what kind of value each one takes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Word,
    Path,
    Number,
    Switch,
    List,
}

const KEYS: [(&str, Kind); 10] = [
    ("scene", Kind::Word),
    ("layout", Kind::Word),
    ("visual", Kind::Word),
    ("shuffle", Kind::List),
    ("photos_folder", Kind::Path),
    ("photos_seconds", Kind::Number),
    ("show_media", Kind::Switch),
    ("reduced_motion", Kind::Switch),
    ("show_hostname", Kind::Switch),
    ("dismiss_on_motion", Kind::Switch),
];

/// `name<TAB>on|off<TAB>shuffle|-<TAB>description`, one line per scene.
pub fn list_lines(cfg: &Config) -> String {
    Scene::ALL
        .iter()
        .map(|scene| {
            format!(
                "{}\t{}\t{}\t{}\n",
                scene.as_str(),
                if *scene == cfg.scene { "on" } else { "off" },
                if cfg.shuffle.contains(scene) {
                    "shuffle"
                } else {
                    "-"
                },
                scene.description()
            )
        })
        .collect()
}

/// `key<TAB>value`, one line per setting the Screensaver view shows.
pub fn settings_lines(cfg: &Config) -> String {
    let on = |b: bool| if b { "true" } else { "false" };
    let shuffle: Vec<&str> = cfg.shuffle.iter().map(|s| s.as_str()).collect();
    [
        ("scene", cfg.scene.as_str().to_string()),
        ("layout", cfg.layout.as_str().to_string()),
        ("visual", cfg.visual.as_str().to_string()),
        ("shuffle", shuffle.join(",")),
        ("photos_folder", cfg.photos_folder.clone()),
        ("photos_seconds", cfg.photos_seconds.to_string()),
        ("show_media", on(cfg.show_media).into()),
        ("reduced_motion", on(cfg.reduced_motion).into()),
        ("show_hostname", on(cfg.show_hostname).into()),
        ("dismiss_on_motion", on(cfg.dismiss_on_motion).into()),
    ]
    .iter()
    .map(|(k, v)| format!("{k}\t{v}\n"))
    .collect()
}

/// The TOML value for `key`, or why `value` does not fit.
fn value_for(key: &str, value: &str) -> Result<toml_edit::Value> {
    let kind = KEYS
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, kind)| *kind)
        .ok_or_else(|| format!("unknown key {key:?}\n{USAGE}"))?;
    let value = value.trim();
    Ok(match kind {
        Kind::Word => {
            let word = match key {
                "scene" => Scene::parse(value).map(Scene::as_str),
                "layout" => Layout::parse(value).map(Layout::as_str),
                _ => crate::visual::Mode::parse(value).map(crate::visual::Mode::as_str),
            }
            .ok_or_else(|| format!("{value:?} is not a {key}\n{USAGE}"))?;
            word.into()
        }
        Kind::Path => value.into(),
        Kind::Number => value
            .parse::<i64>()
            .map_err(|_| format!("{key} must be a whole number"))?
            .into(),
        Kind::Switch => match value.to_ascii_lowercase().as_str() {
            "true" | "on" | "yes" | "1" => true.into(),
            "false" | "off" | "no" | "0" => false.into(),
            _ => return Err(format!("{key} must be true or false").into()),
        },
        Kind::List => {
            let mut array = toml_edit::Array::new();
            for name in value.split(',').map(str::trim).filter(|n| !n.is_empty()) {
                let scene = Scene::parse(name)
                    .ok_or_else(|| format!("{name:?} is not a scene\n{USAGE}"))?;
                array.push(scene.as_str());
            }
            array.into()
        }
    })
}

/// `text` with `key` set to `value`; the rest of the file is untouched.
pub fn with_setting(text: &str, key: &str, value: &str) -> Result<String> {
    let mut document: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
    let value = value_for(key, value)?;
    match document.get_mut(key).and_then(|item| item.as_value_mut()) {
        // Keep the comment after the value, if any.
        Some(old) => {
            let decor = old.decor().clone();
            *old = value;
            *old.decor_mut() = decor;
        }
        None => {
            document.insert(key, toml_edit::Item::Value(value));
        }
    }
    checked(document)
}

/// `text` with `scene` added to, removed from or toggled in `shuffle`.
pub fn with_shuffle(text: &str, change: &str, scene: &str) -> Result<String> {
    let cfg = Config::check_text(text)?;
    let scene = Scene::parse(scene)
        .filter(|s| *s != Scene::Shuffle)
        .ok_or_else(|| format!("{scene:?} is not a scene for shuffle"))?;
    let mut list = cfg.shuffle;
    let present = list.contains(&scene);
    let add = match change {
        "add" => true,
        "remove" => false,
        "toggle" => !present,
        _ => return Err(USAGE.into()),
    };
    if add && !present {
        list.push(scene);
    } else if !add {
        list.retain(|s| *s != scene);
        if list.is_empty() {
            return Err("shuffle needs at least one scene".into());
        }
    }
    let names: Vec<&str> = list.iter().map(|s| s.as_str()).collect();
    with_setting(text, "shuffle", &names.join(","))
}

fn checked(document: toml_edit::DocumentMut) -> Result<String> {
    let text = document.to_string();
    Config::check_text(&text)?;
    Ok(text)
}

/// The current file text ("" when there is none).
pub fn read() -> Result<String> {
    match fs::read_to_string(config::config_file()) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e.into()),
    }
}

/// Replace the file in one step, keeping its permissions.
pub fn write(text: &str) -> Result<()> {
    let path = config::config_file();
    let parent = path.parent().ok_or("configuration directory is missing")?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(".screensaver.toml.{}", std::process::id()));
    fs::write(&temp, text)?;
    if let Ok(metadata) = fs::metadata(&path) {
        let _ = fs::set_permissions(&temp, metadata.permissions());
    }
    fs::rename(&temp, &path)?;
    Ok(())
}

/// Run one editing command; `args` starts with the flag.
pub fn run(args: &[String]) -> Result<()> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["--list"] => print!("{}", list_lines(&Config::load()?)),
        ["--settings"] => print!("{}", settings_lines(&Config::load()?)),
        ["--set", key, value] => {
            let text = with_setting(&read()?, key, value)?;
            write(&text)?;
        }
        ["--shuffle", change, scene] => {
            let text = with_shuffle(&read()?, change, scene)?;
            write(&text)?;
        }
        _ => return Err(USAGE.into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "# my saver\nlayout = \"minimal\" # calm\nscene = \"clock\"\n";

    #[test]
    fn a_setting_changes_one_line_and_keeps_comments() {
        let text = with_setting(FILE, "scene", "dark_sid").unwrap();
        assert!(text.starts_with("# my saver\nlayout = \"minimal\" # calm\n"));
        assert!(text.contains("scene = \"dark-sid\""));
        let text = with_setting(&text, "layout", "corner").unwrap();
        assert!(text.contains("layout = \"corner\" # calm"));
        let text = with_setting("", "reduced_motion", "on").unwrap();
        assert_eq!(text.trim(), "reduced_motion = true");
        let text = with_setting("", "shuffle", "dark, photos").unwrap();
        assert_eq!(text.trim(), "shuffle = [\"dark\", \"photos\"]");
    }

    #[test]
    fn bad_values_are_refused() {
        assert!(with_setting(FILE, "scene", "aurora").is_err());
        assert!(with_setting(FILE, "photos_seconds", "2").is_err());
        assert!(with_setting(FILE, "photos_seconds", "soon").is_err());
        assert!(with_setting(FILE, "colour", "red").is_err());
        assert!(with_setting(FILE, "shuffle", "").is_err());
    }

    #[test]
    fn shuffle_membership_toggles_but_never_empties() {
        let text = with_shuffle("", "toggle", "dark-sid").unwrap();
        assert!(text.contains("shuffle = [\"clock\", \"photos\", \"dark\", \"dark-sid\"]"));
        let text = with_shuffle(&text, "remove", "clock").unwrap();
        let cfg = Config::check_text(&text).unwrap();
        assert_eq!(
            cfg.shuffle,
            vec![Scene::Photos, Scene::Dark, Scene::DarkSid]
        );
        let one = with_setting("", "shuffle", "dark").unwrap();
        assert!(with_shuffle(&one, "remove", "dark").is_err());
        assert!(with_shuffle("", "add", "shuffle").is_err());
    }

    #[test]
    fn demo_can_be_set_and_shuffled() {
        let text = with_setting(FILE, "scene", "demo").unwrap();
        assert!(text.contains("scene = \"demo\""));
        let text = with_shuffle("", "add", "demo").unwrap();
        assert!(text.contains("shuffle = [\"clock\", \"photos\", \"dark\", \"demo\"]"));
        let lines = list_lines(&Config::check_text("").unwrap());
        assert!(lines.contains("demo\toff\t-\t"));
        assert!(with_setting("", "visual", "music").is_ok());
    }

    #[test]
    fn the_list_marks_the_scene_and_the_shuffle() {
        let cfg = Config::check_text("scene = \"photos\"").unwrap();
        let lines = list_lines(&cfg);
        assert!(lines.contains("photos\ton\tshuffle\t"));
        assert!(lines.contains("dark-sid\toff\t-\t"));
        assert_eq!(lines.lines().count(), Scene::ALL.len());
        assert!(settings_lines(&cfg).contains("shuffle\tclock,photos,dark\n"));
    }
}
