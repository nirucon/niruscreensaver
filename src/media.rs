//! The "Artist – Title" line under the CAVA spectrum.
//!
//! One `playerctl --follow` child prints the MPRIS status and metadata of the
//! active player whenever they change, so the line is read once per track or
//! status change and never polled. The child runs only while the spectrum is
//! live (sound and a lit display). It is stopped and reaped as soon as the
//! spectrum pauses for silence or the display goes off, and the line is then
//! cleared. NIRUSCREENSAVER-only: NIRULOCK has no media line.

use std::{
    io::{BufRead, BufReader},
    os::unix::process::CommandExt,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
};

/// The fields are separated by tabs, which do not occur in MPRIS strings in
/// practice; a tab inside a title only shortens what is shown.
const FORMAT: &str = "{{status}}\t{{artist}}\t{{title}}";

/// A running `playerctl --follow` and the line it last reported.
struct Follower {
    child: Child,
    reader: Option<JoinHandle<()>>,
}

impl Drop for Follower {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        // The pipe closes with the child, so the reader ends.
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

pub struct Media {
    follower: Option<Follower>,
    /// Whether the caller last asked for the line. A follower is started only
    /// when this turns true, so a missing or crashing `playerctl` is never
    /// respawned in a loop.
    wanted: bool,
    line: Arc<Mutex<Option<String>>>,
    changed: Arc<AtomicBool>,
}

impl Default for Media {
    fn default() -> Self {
        Self::new()
    }
}

impl Media {
    pub fn new() -> Self {
        Self {
            follower: None,
            wanted: false,
            line: Arc::new(Mutex::new(None)),
            changed: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Start following on a false-to-true change, stop on true-to-false.
    pub fn set_active(&mut self, active: bool) {
        if active == std::mem::replace(&mut self.wanted, active) {
            return;
        }
        if active {
            self.follower = self.start();
        } else {
            self.follower = None;
            self.set_line(None);
        }
    }

    /// True once after the shown line changed.
    pub fn take_changed(&self) -> bool {
        self.changed.swap(false, Ordering::AcqRel)
    }

    pub fn line(&self) -> Option<String> {
        self.line.lock().ok().and_then(|line| line.clone())
    }

    fn set_line(&self, value: Option<String>) {
        store(&self.line, &self.changed, value);
    }

    fn start(&self) -> Option<Follower> {
        let mut command = Command::new("playerctl");
        command
            .args(["--follow", "metadata", "--format", FORMAT])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        // Never outlive the saver, even if it is killed.
        // SAFETY: prctl is async-signal-safe and touches no Rust state.
        unsafe {
            command.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM, 0, 0, 0);
                Ok(())
            });
        }
        let mut child = command.spawn().ok()?;
        let stdout = child.stdout.take()?;
        let line = Arc::clone(&self.line);
        let changed = Arc::clone(&self.changed);
        let reader = thread::Builder::new()
            .name("niru-media-reader".into())
            .spawn(move || {
                for text in BufReader::new(stdout).lines() {
                    let Ok(text) = text else { break };
                    store(&line, &changed, parse(&text));
                }
            });
        match reader {
            Ok(reader) => Some(Follower {
                child,
                reader: Some(reader),
            }),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                None
            }
        }
    }
}

fn store(line: &Mutex<Option<String>>, changed: &AtomicBool, value: Option<String>) {
    if let Ok(mut guard) = line.lock() {
        if *guard != value {
            *guard = value;
            changed.store(true, Ordering::Release);
        }
    }
}

/// "Artist – Title" for a playing player, nothing otherwise.
fn parse(text: &str) -> Option<String> {
    let mut fields = text.split('\t').map(str::trim);
    let status = fields.next()?;
    let artist = fields.next().unwrap_or("");
    let title = fields.next().unwrap_or("");
    if !status.eq_ignore_ascii_case("playing") {
        return None;
    }
    match (artist.is_empty(), title.is_empty()) {
        (_, true) => None,
        (true, false) => Some(title.to_string()),
        (false, false) => Some(format!("{artist} – {title}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_playing_track_is_shown() {
        assert_eq!(
            parse("Playing\tWardruna\tHelvegen"),
            Some("Wardruna – Helvegen".into())
        );
        assert_eq!(parse("Playing\t\tHelvegen"), Some("Helvegen".into()));
        assert_eq!(parse("Paused\tWardruna\tHelvegen"), None);
        assert_eq!(parse("Stopped\t\t"), None);
        assert_eq!(parse("Playing\tWardruna\t"), None);
        assert_eq!(parse(""), None);
    }

    #[test]
    fn a_missing_player_never_respawns_in_a_loop() {
        let mut media = Media::new();
        media.wanted = true;
        // Already wanted: no start, whatever happened to the last child.
        media.set_active(true);
        assert!(media.follower.is_none());
        media.set_active(false);
        assert!(!media.wanted);
    }
}
