//! Which firstmate home the chief of staff runs on. By default it is the managed one, the app's
//! own clone of firstmate and the home beside it (see [`crate::firstmate`]). The captain can
//! choose a custom home instead, such as the `~/firstmate` a terminal firstmate already runs in,
//! so the app shows and steers that fleet.
//!
//! A custom home is one folder: a firstmate checkout that is also its own home, which is how
//! firstmate runs when `FM_HOME` is unset. Firstmate's scripts find their code root from their
//! own path and their home from `FM_HOME`, so the app runs the chief of staff in that folder with
//! `FM_HOME` set to it, and the captain's `AGENTS.md`, skills, hooks and records all come from the
//! one place a terminal session there uses.
//!
//! The app checks the folder and never writes to it, installs it or updates it. It never takes
//! the home's session lock either: firstmate's own session start takes it, or runs read-only when
//! another session holds it, and the app reads who holds it with firstmate's
//! `bin/fm-lock.sh status`, which changes nothing.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime};

use rune::{TextEdit, Waker};
use serde_json::{Value, json};

use crate::firstmate::Place;
use crate::store::{self, Saved};

/// The settings file in the data directory.
pub const SETTINGS_FILE: &str = "firstmate-home.json";

/// How often the lock is looked at for a change.
const POLL: Duration = Duration::from_secs(2);

/// The home the captain chose: a custom one, or `None` for the managed one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Choice(pub Option<PathBuf>);

impl Saved for Choice {
    const FILE: &'static str = SETTINGS_FILE;
    const FALLBACK: &'static str = "your chief of staff runs on the managed firstmate home";

    fn fallback() -> Self {
        Choice(None)
    }

    fn to_json(&self) -> Value {
        json!({"version": store::VERSION, "home": self.0})
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        store::check_version(value)?;
        match &value["home"] {
            Value::Null => Ok(Choice(None)),
            Value::String(path) if Path::new(path).is_absolute() => {
                Ok(Choice(Some(PathBuf::from(path))))
            }
            _ => Err("its home is not a full path".into()),
        }
    }
}

/// A folder that can be used as the home, and what the check found in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checked {
    /// The folder, with `~` expanded and links resolved.
    pub path: PathBuf,
    /// Each thing the check confirmed, in plain words.
    pub found: Vec<String>,
}

/// Checks that `typed` names a firstmate home the chief of staff can run on, other than the
/// managed one (`managed`). It reads the folder and writes nothing. The error says, in plain
/// words, what is wrong.
pub fn check(typed: &str, managed: &Place) -> Result<Checked, String> {
    let typed = typed.trim();
    if typed.is_empty() {
        return Err("Type the folder's path, or choose it.".into());
    }
    let path = expand(typed);
    if !path.is_absolute() {
        return Err(format!(
            "{typed} is not a full path. Type one that starts with / or ~."
        ));
    }
    let path = match std::fs::canonicalize(&path) {
        Ok(path) => path,
        Err(_) => return Err(format!("There is no folder at {}.", path.display())),
    };
    let shown = path.display();
    if !path.is_dir() {
        return Err(format!("{shown} is a file, not a folder."));
    }
    let same = |other: &Path| std::fs::canonicalize(other).is_ok_and(|o| o == path);
    if same(&managed.code) || same(&managed.home) {
        return Err(format!(
            "{shown} is the managed firstmate. Choose Use the managed home instead."
        ));
    }
    if shown.to_string().contains(char::is_whitespace) {
        return Err(format!(
            "{shown} has a space in its path, and firstmate's scripts run from inside the home. \
             Choose a home whose path has no spaces."
        ));
    }
    let missing = |names: &[&str], dirs: bool| -> Vec<String> {
        names
            .iter()
            .filter(|n| {
                let p = path.join(n);
                if dirs { !p.is_dir() } else { !p.is_file() }
            })
            .map(|n| n.to_string())
            .collect()
    };
    let code = missing(&["bin/fm-session-start.sh", "AGENTS.md"], false);
    if !code.is_empty() {
        return Err(format!(
            "{shown} is not a firstmate checkout: it has no {}. Velvet Hammer runs firstmate's \
             own instructions and scripts from the home, so the home must be a clone of \
             firstmate, as ~/firstmate is.",
            code.join(" and ")
        ));
    }
    let dirs = missing(&["state", "data", "config"], true);
    if !dirs.is_empty() {
        return Err(format!(
            "{shown} is a firstmate checkout, but firstmate has not run there as a home: it has \
             no {}. Run firstmate there once in a terminal, or use the managed home. Velvet \
             Hammer does not create them in a home you chose.",
            dirs.iter()
                .map(|d| format!("{d}/"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let mut records = vec![path.clone()];
    records.extend(
        ["state", "data", "config", "projects"]
            .iter()
            .map(|d| path.join(d))
            .filter(|d| d.is_dir()),
    );
    if let Some(blocked) = records.iter().find(|d| !read_write(d)) {
        return Err(format!(
            "You cannot read and write {}, and firstmate keeps its records there.",
            blocked.display()
        ));
    }
    Ok(Checked {
        found: vec![
            format!("A folder at {shown}."),
            "A firstmate checkout: bin/fm-session-start.sh and AGENTS.md are there.".into(),
            "A home firstmate has run in: state/, data/ and config/ are there.".into(),
            "You can read and write it.".into(),
        ],
        path,
    })
}

/// `~` and `~/…` as the user's home directory.
fn expand(typed: &str) -> PathBuf {
    let home = || PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    match typed.strip_prefix('~') {
        Some("") => home(),
        Some(rest) if rest.starts_with('/') => home().join(&rest[1..]),
        _ => PathBuf::from(typed),
    }
}

/// Whether this user may list, read and write in `dir`, as the system decides it. Nothing is
/// written to find out.
fn read_write(dir: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let Ok(c) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else {
            return false;
        };
        // SAFETY: `access` reads the NUL-terminated path and nothing else.
        unsafe { libc::access(c.as_ptr(), libc::R_OK | libc::W_OK | libc::X_OK) == 0 }
    }
    #[cfg(not(unix))]
    {
        std::fs::metadata(dir).is_ok_and(|m| !m.permissions().readonly())
    }
}

/// The home's session lock, as firstmate reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lock {
    /// What `bin/fm-lock.sh status` printed, word for word.
    pub said: String,
    /// The live session that holds it, when firstmate says one does.
    pub holder: Option<Holder>,
}

/// The process firstmate says holds a home's lock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Holder {
    pub pid: u32,
    /// Its command line, as `ps` prints it, or empty when `ps` could not say.
    pub command: String,
    /// It and the processes that started it, nearest first.
    pub line: Vec<u32>,
}

impl Lock {
    /// The holder, when it is not the chief of staff's own session `ours` (the `claude` the app
    /// runs) or a process under it.
    pub fn other(&self, ours: Option<u32>) -> Option<&Holder> {
        self.holder
            .as_ref()
            .filter(|h| !ours.is_some_and(|pid| h.line.contains(&pid)))
    }
}

/// Reads the lock of the home at `place` with firstmate's `bin/fm-lock.sh status`. `None` when
/// the checkout has no such script.
pub fn read_lock(place: &Place) -> Option<Result<Lock, String>> {
    let script = place.code.join("bin/fm-lock.sh");
    if !script.is_file() {
        return None;
    }
    let ran = Command::new(&script)
        .arg("status")
        .current_dir(&place.code)
        .env("FM_HOME", &place.home)
        .stdin(Stdio::null())
        .output();
    Some(match ran {
        Err(e) => Err(format!(
            "Could not run firstmate's bin/fm-lock.sh status: {e}"
        )),
        Ok(out) if !out.status.success() => Err(format!(
            "Firstmate's bin/fm-lock.sh status failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
        Ok(out) => {
            let said = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let holder = said
                .strip_prefix("lock: held by live harness pid ")
                .and_then(|pid| pid.trim().parse().ok())
                .map(|pid| Holder {
                    pid,
                    command: ps(&["-o", "command=", "-p", &pid.to_string()]),
                    line: line(pid),
                });
            Ok(Lock { said, holder })
        }
    })
}

/// What `ps` prints for `args`, trimmed, or empty.
fn ps(args: &[&str]) -> String {
    Command::new("ps")
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// `pid` and its ancestors, nearest first.
fn line(pid: u32) -> Vec<u32> {
    let mut line = vec![pid];
    while line.len() < 64 {
        let last = line[line.len() - 1];
        match ps(&["-o", "ppid=", "-p", &last.to_string()]).parse() {
            Ok(parent) if parent > 1 && !line.contains(&parent) => line.push(parent),
            _ => break,
        }
    }
    line
}

/// What decides whether the lock may have changed: the lock file's contents and time, and
/// whether the pid in it still runs.
fn lock_stamp(home: &Path) -> (String, Option<SystemTime>, bool) {
    let file = home.join("state/.lock");
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    let modified = file.metadata().and_then(|m| m.modified()).ok();
    let alive = text
        .lines()
        .next()
        .and_then(|pid| pid.trim().parse::<i32>().ok())
        .is_some_and(|pid| {
            // SAFETY: signal 0 only asks whether the process exists.
            pid > 0
                && (unsafe { libc::kill(pid, 0) } == 0
                    || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM))
        });
    (text, modified, alive)
}

/// Keeps a home's [`Lock`] current until dropped: it reads it again whenever the lock file
/// changes or its holder stops running.
pub struct LockWatch {
    rx: Receiver<Result<Lock, String>>,
    wake: Arc<OnceLock<Waker>>,
    stop: Arc<AtomicBool>,
}

impl LockWatch {
    pub fn start(place: &Place) -> LockWatch {
        let (tx, rx) = channel();
        let wake: Arc<OnceLock<Waker>> = Arc::new(OnceLock::new());
        let stop = Arc::new(AtomicBool::new(false));
        let (place, waker, stopped) = (place.clone(), wake.clone(), stop.clone());
        thread::spawn(move || {
            let mut seen = None;
            while !stopped.load(Ordering::Relaxed) {
                // A managed clone being installed gets its lock script later.
                let script = place.code.join("bin/fm-lock.sh").is_file();
                let stamp = Some((script, lock_stamp(&place.home)));
                if stamp != seen {
                    seen = stamp;
                    if let Some(lock) = read_lock(&place) {
                        if tx.send(lock).is_err() {
                            return;
                        }
                        if let Some(wake) = waker.get() {
                            wake();
                        }
                    }
                }
                thread::sleep(POLL);
            }
        });
        LockWatch { rx, wake, stop }
    }

    pub fn connect(&mut self, wake: Waker) {
        let _ = self.wake.set(wake);
        if let Some(wake) = self.wake.get() {
            wake();
        }
    }

    /// The newest read since the last call, if one arrived.
    pub fn poll(&mut self) -> Option<Result<Lock, String>> {
        self.rx.try_iter().last()
    }
}

impl Drop for LockWatch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// The Firstmate page's home chooser.
#[derive(Default)]
pub struct Form {
    /// The folder typed or chosen.
    pub path: TextEdit,
    /// What the last check of `path` found.
    pub checked: Option<Result<Checked, String>>,
    /// Who holds the checked folder's lock, as firstmate said at the check.
    pub lock: Option<Result<Lock, String>>,
    /// A switch waiting for the captain's yes because work is in flight.
    pub confirming: Option<Choice>,
    /// Why the last switch was not made.
    pub problem: Option<String>,
}

/// Every path under `dir`, git's records included, with its time, so a test can show that
/// nothing in a home was written.
#[cfg(test)]
pub fn tree(dir: &Path) -> Vec<(PathBuf, Option<SystemTime>)> {
    let mut found = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        found.push((d.clone(), d.metadata().and_then(|m| m.modified()).ok()));
        for entry in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = entry.path();
            if p.is_dir() {
                todo.push(p);
            } else {
                found.push((p.clone(), p.metadata().and_then(|m| m.modified()).ok()));
            }
        }
    }
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::firstmate::lab::Lab;

    fn managed(lab: &Lab) -> Place {
        Place {
            code: lab.clone_dir(),
            home: lab.home(),
            custom: false,
        }
    }

    #[test]
    fn the_choice_reads_back_as_written() {
        for choice in [
            Choice(None),
            Choice(Some("/Users/captain/firstmate".into())),
        ] {
            assert_eq!(Choice::from_json(&choice.to_json()), Ok(choice));
        }
        assert!(Choice::from_json(&json!({"version": 1, "home": "firstmate"})).is_err());
        assert!(Choice::from_json(&json!({"version": 1, "home": 7})).is_err());
        assert_eq!(Choice::default(), Choice(None));
    }

    #[test]
    fn an_existing_firstmate_home_passes_and_nothing_is_written_to_it() {
        let lab = Lab::new();
        let home = lab.captain_home();
        let before = tree(&home);
        let checked = check(&home.display().to_string(), &managed(&lab)).unwrap();
        assert_eq!(checked.path, std::fs::canonicalize(&home).unwrap());
        assert_eq!(checked.found.len(), 4);
        assert!(checked.found[1].contains("bin/fm-session-start.sh and AGENTS.md"));
        assert_eq!(tree(&home), before);
    }

    #[test]
    fn each_kind_of_wrong_folder_is_refused_in_plain_words() {
        let lab = Lab::new();
        let home = lab.captain_home();
        let managed = managed(&lab);
        let refused = |typed: &str| check(typed, &managed).unwrap_err();
        assert!(refused("  ").contains("Type the folder's path"));
        assert!(refused("firstmate").contains("is not a full path"));
        assert!(refused(&format!("{}/nowhere", home.display())).contains("There is no folder"));
        assert!(refused(&home.join("AGENTS.md").display().to_string()).contains("is a file"));
        std::fs::create_dir_all(lab.clone_dir()).unwrap();
        assert!(
            refused(&lab.clone_dir().display().to_string()).contains("is the managed firstmate")
        );

        let spaced = home.with_file_name("my firstmate");
        std::fs::create_dir_all(&spaced).unwrap();
        assert!(refused(&spaced.display().to_string()).contains("has a space"));

        let plain = home.with_file_name("plain");
        std::fs::create_dir_all(&plain).unwrap();
        let why = refused(&plain.display().to_string());
        assert!(why.contains("is not a firstmate checkout"), "{why}");
        assert!(
            why.contains("bin/fm-session-start.sh and AGENTS.md"),
            "{why}"
        );

        std::fs::remove_dir_all(home.join("state")).unwrap();
        std::fs::remove_dir_all(home.join("config")).unwrap();
        let why = refused(&home.display().to_string());
        assert!(why.contains("has not run there as a home"), "{why}");
        assert!(why.contains("state/, config/"), "{why}");
        assert!(!home.join("state").exists(), "the check created nothing");
    }

    #[cfg(unix)]
    #[test]
    fn a_home_the_captain_cannot_write_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let lab = Lab::new();
        let home = lab.captain_home();
        let data = home.join("data");
        std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o555)).unwrap();
        let why = check(&home.display().to_string(), &managed(&lab)).unwrap_err();
        std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(why.contains("You cannot read and write"), "{why}");
        assert!(why.contains("/data"), "{why}");
    }

    #[test]
    fn a_tilde_is_the_captains_home_directory() {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        assert_eq!(expand("~"), home);
        assert_eq!(expand("~/firstmate"), home.join("firstmate"));
        assert_eq!(expand("/srv/firstmate"), PathBuf::from("/srv/firstmate"));
    }

    #[test]
    fn the_lock_is_read_with_firstmates_status_and_never_taken() {
        let lab = Lab::new();
        let home = lab.captain_home();
        let place = Place {
            code: home.clone(),
            home: home.clone(),
            custom: true,
        };
        assert_eq!(read_lock(&place).unwrap().unwrap().said, "lock: free");
        assert!(!home.join("state/.lock").exists());

        // A terminal session elsewhere holds it.
        let mut holder = Command::new("sleep").arg("30").spawn().unwrap();
        std::fs::write(home.join("state/.lock"), format!("{}\n", holder.id())).unwrap();
        let lock = read_lock(&place).unwrap().unwrap();
        assert_eq!(
            lock.said,
            format!("lock: held by live harness pid {}", holder.id())
        );
        let other = lock.other(None).expect("another session holds it");
        assert_eq!(other.pid, holder.id());
        assert!(other.command.contains("sleep 30"), "{}", other.command);
        // The app's own session is the holder, or its parent.
        assert!(lock.other(Some(holder.id())).is_none());
        assert!(lock.other(Some(std::process::id())).is_none());
        assert!(lock.other(Some(1)).is_some());

        holder.kill().unwrap();
        holder.wait().unwrap();
        let lock = read_lock(&place).unwrap().unwrap();
        assert!(lock.said.starts_with("lock: stale"), "{}", lock.said);
        assert_eq!(lock.holder, None);
    }

    #[test]
    fn a_checkout_without_the_lock_script_has_no_lock_to_read() {
        let lab = Lab::new();
        let place = Place {
            code: lab.data.join("nowhere"),
            home: lab.data.join("nowhere"),
            custom: true,
        };
        assert_eq!(read_lock(&place), None);
    }
}
