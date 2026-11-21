//! `demo`: a realtime demoscene demo by NIRUDEMO on every monitor.
//!
//! The saver keeps its own window, grabs and dismissal. On each monitor it
//! puts a black child window and starts `nirudemo --into 0xWINDOW --loop
//! --silent` ([`command`]), which draws into that window over its own X
//! connection and follows its size. NIRUDEMO never gets input: the saver
//! holds the keyboard and pointer grabs, so every key, click and motion
//! still reaches the saver, which ends exactly as in the other scenes.
//!
//! Each NIRUDEMO is spawned like NIRULOCK's music player: its own session
//! and so its own process group, stdin, stdout and stderr on `/dev/null`,
//! and `PR_SET_PDEATHSIG` set to `SIGKILL`, so it never outlives the saver.
//! It gets TERM, then KILL after [`TERM_WAIT`], when the saver ends and when
//! the display goes off; it starts again when the display comes back.
//!
//! Without `nirudemo` on `PATH`, or when one exits within [`QUICK_EXIT`] of
//! its start, that monitor shows the `dark` scene instead (logged once). A
//! NIRUDEMO that ends later is started again. The scene is silent.

use super::{Dark, Show};
use crate::config::Config;
use crate::core::{Area, Surface, TextRenderer};
use crate::Result;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

/// The program that draws the demo.
pub const PROGRAM: &str = "nirudemo";
/// A NIRUDEMO that exits sooner than this after its start cannot run here:
/// its monitor falls back to `dark`.
pub const QUICK_EXIT: Duration = Duration::from_secs(2);
/// How long NIRUDEMO gets to exit after TERM before KILL.
pub const TERM_WAIT: Duration = Duration::from_millis(500);
/// How often a running NIRUDEMO is looked at: often while a quick exit
/// would still mean "fall back", rarely after that.
const POLL_YOUNG: Duration = Duration::from_millis(200);
const POLL_OLD: Duration = Duration::from_secs(2);

/// The arguments for drawing into `window`: `--into 0xWINDOW --loop --silent`.
pub fn command(window: u32) -> Vec<String> {
    vec![
        "--into".into(),
        format!("0x{window:x}"),
        "--loop".into(),
        "--silent".into(),
    ]
}

/// What to do when a NIRUDEMO ended after running for `ran`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AfterExit {
    /// Start it again on the same window.
    Restart,
    /// Show `dark` on that monitor for the rest of this saver.
    Fallback,
}

pub fn after_exit(ran: Duration) -> AfterExit {
    if ran < QUICK_EXIT {
        AfterExit::Fallback
    } else {
        AfterExit::Restart
    }
}

/// The first executable `name` in `path` (a `PATH` value).
pub fn find_program(name: &str, path: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    std::env::split_paths(path?)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(name))
        .find(|candidate| {
            use std::os::unix::fs::PermissionsExt;
            std::fs::metadata(candidate)
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
}

/// Start `program` with `args`: its own session, no input or output, and
/// killed when the saver goes.
fn spawn(program: &Path, args: &[String]) -> std::io::Result<Child> {
    // SAFETY: getpid is async-signal-safe and has no preconditions.
    let parent = unsafe { libc::getpid() };
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: only async-signal-safe calls between fork and exec.
    unsafe {
        command.pre_exec(move || {
            // A new session and process group, no controlling terminal;
            // SIGKILL when the saver's main thread ends.
            if libc::setsid() < 0
                || libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) != 0
                || libc::getppid() != parent
            {
                libc::_exit(127);
            }
            Ok(())
        });
    }
    command.spawn()
}

/// TERM to the process group, a short wait, KILL, reap.
fn finish(child: &mut Child) {
    let group = child.id() as i32;
    let signal = |sig| {
        if group > 1 {
            // SAFETY: plain kill(2) on a process group the saver created.
            unsafe {
                libc::kill(-group, sig);
            }
        }
    };
    if matches!(child.try_wait(), Ok(None)) {
        signal(libc::SIGTERM);
        let until = Instant::now() + TERM_WAIT;
        while matches!(child.try_wait(), Ok(None)) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        if matches!(child.try_wait(), Ok(None)) {
            signal(libc::SIGKILL);
            let _ = child.wait();
        }
    }
    // Anything else NIRUDEMO left in its group.
    signal(libc::SIGKILL);
}

/// One monitor's demo window.
struct Slot {
    window: Window,
    area: Area,
    child: Option<(Child, Instant)>,
}

pub(super) struct Demo {
    /// Draws `dark` where no demo runs (the demo windows cover the rest).
    dark: Dark,
    program: Option<PathBuf>,
    slots: Vec<Slot>,
    areas: Vec<Area>,
    on: bool,
    poll: Option<Instant>,
    logged: bool,
}

impl Demo {
    pub(super) fn new(cfg: &Config, text: &TextRenderer, surface: &Surface) -> Result<Self> {
        let program = find_program(PROGRAM, std::env::var_os("PATH").as_deref());
        let mut demo = Self {
            dark: Dark::new(cfg, text, surface, false),
            program,
            slots: Vec::new(),
            areas: Vec::new(),
            on: true,
            poll: None,
            logged: false,
        };
        if demo.program.is_none() {
            demo.fall_back(&format!("{PROGRAM} is not installed"));
        }
        demo.build(surface)?;
        Ok(demo)
    }

    fn fall_back(&mut self, why: &str) {
        if !self.logged {
            eprintln!("niruscreensaver: demo: {why}; showing the dark scene");
            self.logged = true;
        }
    }

    /// One black child window per monitor, each with its NIRUDEMO.
    fn build(&mut self, s: &Surface) -> Result<()> {
        self.close(s)?;
        self.areas = s.areas.clone();
        if self.program.is_some() {
            for area in &s.areas {
                let window = s.c.generate_id()?;
                s.c.create_window(
                    x11rb::COPY_DEPTH_FROM_PARENT,
                    window,
                    s.win,
                    area.x,
                    area.y,
                    area.width.max(1),
                    area.height.max(1),
                    0,
                    WindowClass::INPUT_OUTPUT,
                    x11rb::COPY_FROM_PARENT,
                    // No event mask: input goes to the saver's window, and
                    // the grabs keep it there.
                    &CreateWindowAux::new().background_pixel(self.dark.cfg.palette.background),
                )?;
                s.c.map_window(window)?;
                self.slots.push(Slot {
                    window,
                    area: *area,
                    child: None,
                });
            }
            // The windows must exist before NIRUDEMO looks for them.
            s.c.flush()?;
            if self.on {
                self.start_all();
            }
        }
        self.cover();
        Ok(())
    }

    /// Stop every NIRUDEMO and remove the windows.
    fn close(&mut self, s: &Surface) -> Result<()> {
        self.stop_all();
        for slot in self.slots.drain(..) {
            s.c.destroy_window(slot.window)?;
        }
        Ok(())
    }

    /// `dark` draws only where no demo window is.
    fn cover(&mut self) {
        self.dark.covered = self.slots.iter().map(|slot| slot.area).collect();
    }

    fn start_all(&mut self) {
        let Some(program) = self.program.clone() else {
            return;
        };
        let mut failed = None;
        for slot in &mut self.slots {
            if slot.child.is_none() {
                match spawn(&program, &command(slot.window)) {
                    Ok(child) => slot.child = Some((child, Instant::now())),
                    Err(e) => failed = Some(e),
                }
            }
        }
        if let Some(e) = failed {
            // Not startable at all: every monitor shows dark.
            self.fall_back(&format!("cannot start {PROGRAM}: {e}"));
            self.program = None;
        }
        self.poll = Some(Instant::now() + POLL_YOUNG);
    }

    fn stop_all(&mut self) {
        for slot in &mut self.slots {
            if let Some((mut child, _)) = slot.child.take() {
                finish(&mut child);
            }
        }
        self.poll = None;
    }

    /// Look at the running demos: restart one that ended after a while,
    /// give up a monitor whose demo ended at once.
    fn check(&mut self, s: &Surface, now: Instant) -> Result<()> {
        let mut gone = Vec::new();
        let mut restart = false;
        for (i, slot) in self.slots.iter_mut().enumerate() {
            let Some((child, since)) = slot.child.as_mut() else {
                continue;
            };
            if !matches!(child.try_wait(), Ok(None)) {
                let ran = now.saturating_duration_since(*since);
                let _ = child.wait();
                slot.child = None;
                match after_exit(ran) {
                    AfterExit::Restart => restart = true,
                    AfterExit::Fallback => gone.push(i),
                }
            }
        }
        if !gone.is_empty() {
            self.fall_back(&format!(
                "{PROGRAM} exited within {} s",
                QUICK_EXIT.as_secs()
            ));
            for i in gone.into_iter().rev() {
                let slot = self.slots.remove(i);
                s.c.destroy_window(slot.window)?;
            }
            self.cover();
        }
        if restart && self.on {
            self.start_all();
        }
        let young = self.slots.iter().any(|slot| {
            slot.child
                .as_ref()
                .is_some_and(|(_, since)| now.saturating_duration_since(*since) < QUICK_EXIT)
        });
        let running = self.slots.iter().any(|slot| slot.child.is_some());
        self.poll = running.then(|| now + if young { POLL_YOUNG } else { POLL_OLD });
        Ok(())
    }

    /// Follow new monitor sizes: move the windows, or start over when the
    /// number of monitors changed.
    fn follow(&mut self, s: &Surface) -> Result<()> {
        if s.areas == self.areas {
            return Ok(());
        }
        let same_count = self.program.is_some() && self.slots.len() == s.areas.len();
        if !same_count {
            return self.build(s);
        }
        for (slot, area) in self.slots.iter_mut().zip(&s.areas) {
            s.c.configure_window(
                slot.window,
                &ConfigureWindowAux::new()
                    .x(i32::from(area.x))
                    .y(i32::from(area.y))
                    .width(u32::from(area.width.max(1)))
                    .height(u32::from(area.height.max(1))),
            )?;
            slot.area = *area;
        }
        self.areas = s.areas.clone();
        self.cover();
        s.c.flush()?;
        Ok(())
    }
}

impl Show for Demo {
    fn redraw(&mut self, s: &mut Surface, now: Instant) -> Result<()> {
        self.follow(s)?;
        self.dark.redraw(s, now)
    }

    fn step(&mut self, s: &mut Surface, now: Instant) -> Result<()> {
        if self.dark.deadline().is_some_and(|due| now >= due) {
            self.dark.step(s, now)?;
        }
        Ok(())
    }

    fn deadline(&self) -> Option<Instant> {
        match (self.dark.deadline(), self.poll) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    fn tick(&mut self, now: Instant) -> bool {
        self.dark.tick(now)
    }

    fn poll(&mut self, s: &mut Surface, now: Instant) -> Result<bool> {
        if self.poll.is_some_and(|due| now >= due) {
            let before = self.slots.len();
            self.check(s, now)?;
            // A monitor that fell back to dark is drawn anew.
            return Ok(self.slots.len() != before);
        }
        Ok(false)
    }

    fn display(&mut self, on: bool) {
        self.dark.display(on);
        if on == self.on {
            return;
        }
        self.on = on;
        if on {
            self.start_all();
        } else {
            self.stop_all();
        }
    }
}

impl Drop for Demo {
    fn drop(&mut self) {
        self.stop_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_child_gets_the_window_in_hex() {
        assert_eq!(
            command(0x1a0_0003),
            ["--into", "0x1a00003", "--loop", "--silent"]
        );
        assert_eq!(command(255)[1], "0xff");
    }

    #[test]
    fn a_quick_exit_falls_back_and_a_late_one_restarts() {
        assert_eq!(after_exit(Duration::ZERO), AfterExit::Fallback);
        assert_eq!(after_exit(Duration::from_millis(1999)), AfterExit::Fallback);
        assert_eq!(after_exit(QUICK_EXIT), AfterExit::Restart);
        assert_eq!(after_exit(Duration::from_secs(600)), AfterExit::Restart);
    }

    #[test]
    fn the_program_is_looked_up_on_path() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("saver-demo-path-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a")).unwrap();
        std::fs::create_dir_all(dir.join("b")).unwrap();
        // Not executable: skipped.
        std::fs::write(dir.join("a/nirudemo"), "").unwrap();
        std::fs::write(dir.join("b/nirudemo"), "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(
            dir.join("b/nirudemo"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        let path = std::env::join_paths([dir.join("a"), dir.join("b")]).unwrap();
        assert_eq!(
            find_program(PROGRAM, Some(&path)),
            Some(dir.join("b/nirudemo"))
        );
        let only_a = std::env::join_paths([dir.join("a")]).unwrap();
        assert_eq!(find_program(PROGRAM, Some(&only_a)), None);
        assert_eq!(find_program(PROGRAM, None), None);
        // A relative PATH entry is never used.
        assert_eq!(
            find_program(PROGRAM, Some(std::ffi::OsStr::new("relative/dir"))),
            None
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_child_is_isolated_and_stops_on_term() {
        let mut child =
            spawn(Path::new("/bin/sh"), &["-c".into(), "exec sleep 30".into()]).unwrap();
        let pid = child.id();
        let until = Instant::now() + Duration::from_secs(2);
        while std::fs::read_to_string(format!("/proc/{pid}/comm"))
            .map_or(true, |c| c.trim() != "sleep")
            && Instant::now() < until
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        let fields: Vec<i64> = stat
            .rsplit_once(')')
            .unwrap()
            .1
            .split_whitespace()
            .skip(1)
            .take(6)
            .map(|v| v.parse().unwrap())
            .collect();
        // Process group, session, controlling terminal.
        assert_eq!(fields[1], i64::from(pid), "its own process group");
        assert_eq!(fields[2], i64::from(pid), "its own session");
        assert_eq!(fields[3], 0, "no controlling terminal");
        let started = Instant::now();
        finish(&mut child);
        assert!(started.elapsed() < TERM_WAIT, "sleep ends on TERM");
        assert!(child.try_wait().unwrap().is_some());
    }
}
