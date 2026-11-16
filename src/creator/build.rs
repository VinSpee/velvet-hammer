//! The build step: it puts a change a worker made to the app itself into the running
//! app, and takes it out again. The change is a commit on a branch in a worktree of the app's
//! project; nothing is pushed.
//!
//! **Try** fetches that commit into a checkout the app owns, checks every path the branch changed
//! since it left the project's `main` against [`gate`], builds it there with `--locked --offline` in a warm target folder, makes a
//! bundle from the installed app with the new binary, keeps a copy of the installed bundle as the
//! known-good build, and swaps the new bundle in with Rune's installer, which rolls back a build
//! that is not ready within 15 seconds. Then it stays as the trial's watchdog: when the trial app
//! stops without quitting cleanly, it swaps the known-good build back. **Undo** swaps the
//! known-good build back at once.
//!
//! Each runs as its own process (`velvet-hammer --creator try|undo DATA`), started apart from
//! the app with [`start`], because the swap quits the app. They report through `trial.json`; the
//! app never swaps itself from a tool the chief of staff can call, only when the captain asks.
//! Everything lives in `creator/` in the app's data directory, readable by the captain only.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::store::{create_private_dir, make_private, open_private, suffixed};

/// The installed app, as `scripts/install-app.sh` puts it.
pub const APP: &str = "Velvet Hammer Dev";
const ID: &str = "io.github.vinspee.velvet-hammer";
const EXE: &str = "velvet-hammer";

/// How often the watchdog looks at the trial app.
const WATCH: Duration = Duration::from_secs(1);

/// Whether the paths a branch changed may be tried from inside the app: in this version only
/// views and styling, `src/view.rs` and `src/view/**`. Everything else, such as the hook rules,
/// the Claude Code and firstmate boundaries, the stores, this build step and the build inputs
/// (`Cargo.toml`, `Cargo.lock`, `build.rs`, `scripts/`), is delivered as a normal pull request
/// instead. The check limits accidents; a view can still run any code the app can.
pub fn gate(paths: &[String]) -> Result<(), String> {
    let view = |p: &str| p == "src/view.rs" || p.starts_with("src/view/");
    let locked: Vec<&str> = paths
        .iter()
        .map(String::as_str)
        .filter(|p| !view(p))
        .collect();
    match (paths.is_empty(), locked.as_slice()) {
        (true, _) => Err("The branch changes nothing.".into()),
        (false, []) => Ok(()),
        (false, locked) => Err(format!(
            "Only views and styling (src/view.rs and src/view/) can be tried in the app; this \
             branch also changes {}. Deliver it as a pull request instead.",
            locked.join(", ")
        )),
    }
}

/// Where a trial stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// The commit is being fetched, checked and built.
    Building,
    /// The branch changes paths that cannot be tried in the app.
    Refused,
    /// The build or the swap failed; the app was not changed.
    Failed,
    /// The trial build is running.
    Trying,
    /// The captain undid the trial: the known-good build is back.
    Undone,
    /// The captain kept the trial build: it stays, and the worker delivers its branch.
    Kept,
    /// The trial build stopped without quitting cleanly, and the known-good build is back.
    Restored,
}

impl State {
    fn key(self) -> &'static str {
        match self {
            State::Building => "building",
            State::Refused => "refused",
            State::Failed => "failed",
            State::Trying => "trying",
            State::Undone => "undone",
            State::Kept => "kept",
            State::Restored => "restored",
        }
    }

    fn from_key(key: &str) -> Option<State> {
        [
            State::Building,
            State::Refused,
            State::Failed,
            State::Trying,
            State::Undone,
            State::Kept,
            State::Restored,
        ]
        .into_iter()
        .find(|s| s.key() == key)
    }
}

/// A change being tried, as `trial.json` records it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trial {
    pub state: State,
    /// The commit tried.
    pub commit: String,
    /// The worktree it came from.
    pub worktree: PathBuf,
    /// Why it was refused or failed, or what happened, in words for the captain.
    pub detail: Option<String>,
    /// When the state last changed, in seconds since the epoch.
    pub at: u64,
}

impl Trial {
    fn to_json(&self) -> Value {
        json!({
            "version": 1,
            "state": self.state.key(),
            "commit": self.commit,
            "worktree": self.worktree,
            "detail": self.detail,
            "at": self.at,
        })
    }

    fn from_json(value: &Value) -> Option<Trial> {
        (value["version"] == 1).then_some(())?;
        Some(Trial {
            state: State::from_key(value["state"].as_str()?)?,
            commit: value["commit"].as_str()?.to_string(),
            worktree: PathBuf::from(value["worktree"].as_str()?),
            detail: value["detail"].as_str().map(str::to_string),
            at: value["at"].as_u64().unwrap_or_default(),
        })
    }
}

/// Creator mode's files in the data directory.
pub struct Dir(PathBuf);

impl Dir {
    pub fn new(data: &Path) -> Dir {
        Dir(data.join("creator"))
    }

    fn record(&self) -> PathBuf {
        self.0.join("trial.json")
    }

    /// The app's own checkout that trials are built in.
    fn checkout(&self) -> PathBuf {
        self.0.join("checkout")
    }

    fn target(&self) -> PathBuf {
        self.0.join("target")
    }

    fn stage(&self) -> PathBuf {
        self.0.join("stage").join(format!("{APP}.app"))
    }

    /// The last kept build, a complete bundle a trial never overwrites. Opening it by hand ends
    /// a trial.
    pub fn known_good(&self) -> PathBuf {
        self.0.join("known-good").join(format!("{APP}.app"))
    }

    /// What the last job printed, for the captain and the worker.
    pub fn log(&self) -> PathBuf {
        self.0.join("job.log")
    }

    /// The trial as last recorded; `None` before the first.
    pub fn trial(&self) -> Option<Trial> {
        let text = fs::read(self.record()).ok()?;
        Trial::from_json(&serde_json::from_slice(&text).ok()?)
    }

    /// Records the trial, whole or not at all, readable by the captain only.
    fn set(&self, trial: &Trial) -> io::Result<()> {
        create_private_dir(&self.0)?;
        let path = self.record();
        let temporary = suffixed(&path, &format!(".tmp-{}", std::process::id()));
        fs::write(&temporary, trial.to_json().to_string())?;
        make_private(&temporary)?;
        fs::rename(&temporary, &path)
    }

    /// Moves the recorded trial to `state`, saying `detail`.
    fn mark(&self, trial: &mut Trial, state: State, detail: Option<String>) {
        trial.state = state;
        trial.detail = detail;
        trial.at = now();
        if let Err(e) = self.set(trial) {
            eprintln!("creator: could not record the trial: {e}");
        }
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The installed app this build step swaps.
fn installed() -> PathBuf {
    let home = std::env::var_os("HOME").unwrap_or_default();
    PathBuf::from(home)
        .join("Applications")
        .join(format!("{APP}.app"))
}

/// A job for the build step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Job {
    /// Builds and swaps in `commit` from the worktree at `worktree`, then watches it.
    Try { commit: String, worktree: PathBuf },
    /// Watches the trial of `commit` again, for an app opened into a trial that is still on.
    Watch { commit: String },
    /// Swaps the known-good build back.
    Undo,
}

/// Starts `job` as its own process, apart from the app, so it outlives the app's quit for the
/// swap. Its output goes to the job log.
pub fn start(data: &Path, job: &Job) -> io::Result<()> {
    let dir = Dir::new(data);
    create_private_dir(&dir.0)?;
    let log = open_private(&dir.log())?;
    let mut command = Command::new(std::env::current_exe()?);
    command.arg("--creator");
    match job {
        Job::Try { commit, worktree } => {
            command.arg("try").arg(data).arg(commit).arg(worktree);
        }
        Job::Watch { commit } => {
            command.arg("watch").arg(data).arg(commit);
        }
        Job::Undo => {
            command.arg("undo").arg(data);
        }
    }
    command
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: `setsid` is async-signal-safe and touches nothing of the parent's.
        unsafe {
            command.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
    command.spawn().map(drop)
}

/// The job named by `--creator JOB DATA ...` on the command line, if the app was started as one.
pub fn job_from_args() -> Option<(PathBuf, Job)> {
    job_from(&std::env::args().collect::<Vec<_>>())
}

fn job_from(args: &[String]) -> Option<(PathBuf, Job)> {
    let at = args.iter().position(|a| a == "--creator")?;
    let rest = &args[at + 1..];
    match rest {
        [job, data, commit, worktree] if job == "try" => Some((
            PathBuf::from(data),
            Job::Try {
                commit: commit.clone(),
                worktree: PathBuf::from(worktree),
            },
        )),
        [job, data, commit] if job == "watch" => Some((
            PathBuf::from(data),
            Job::Watch {
                commit: commit.clone(),
            },
        )),
        [job, data] if job == "undo" => Some((PathBuf::from(data), Job::Undo)),
        _ => None,
    }
}

/// Runs a job to its end; for `velvet-hammer --creator`.
pub fn run(data: &Path, job: Job) {
    let dir = Dir::new(data);
    match job {
        Job::Try { commit, worktree } => try_change(&dir, data, commit, worktree),
        Job::Watch { commit } => guard(&dir, data, &commit),
        Job::Undo => undo(&dir, data),
    }
}

/// An app opened into a trial that is still on has no watchdog if the captain quit it cleanly
/// before: the old one stopped then. Starts one, unless the trial's job still runs.
pub fn resume_watch(data: &Path) {
    let dir = Dir::new(data);
    let watching = Command::new("pgrep")
        .arg("-f")
        .arg(format!(
            "--creator (try|watch) {}( |$)",
            escaped(&data.display().to_string())
        ))
        .output()
        .is_ok_and(|o| others(&String::from_utf8_lossy(&o.stdout), std::process::id()));
    if let Some(commit) = rewatch(&dir, watching) {
        let _ = start(data, &Job::Watch { commit });
    }
}

/// The commit whose trial needs a new watchdog. The clean quit that ended the last one is over.
fn rewatch(dir: &Dir, watching: bool) -> Option<String> {
    let trial = dir.trial().filter(|t| t.state == State::Trying)?;
    let _ = fs::remove_file(dir.0.join("quit"));
    (!watching).then_some(trial.commit)
}

fn escaped(text: &str) -> String {
    text.chars()
        .flat_map(|c| {
            let meta = ".[]()*+?{}|^$\\".contains(c);
            meta.then_some('\\').into_iter().chain([c])
        })
        .collect()
}

fn try_change(dir: &Dir, data: &Path, commit: String, worktree: PathBuf) {
    if dir.trial().is_some_and(|t| t.state == State::Trying) {
        return;
    }
    let _ = fs::remove_file(dir.0.join("quit"));
    let mut trial = Trial {
        state: State::Building,
        commit,
        worktree,
        detail: None,
        at: now(),
    };
    dir.mark(&mut trial, State::Building, None);
    let paths = match prepare(dir, &trial.worktree, &trial.commit) {
        Ok(paths) => paths,
        Err(why) => return dir.mark(&mut trial, State::Failed, Some(why)),
    };
    if let Err(why) = gate(&paths) {
        return dir.mark(&mut trial, State::Refused, Some(why));
    }
    let built = build(dir).and_then(|binary| stage(dir, &binary));
    if let Err(why) = built {
        return dir.mark(&mut trial, State::Failed, Some(why));
    }
    // No trial is running, so the installed build is a kept one.
    if let Err(why) = copy_bundle(&installed(), &dir.known_good()) {
        return dir.mark(&mut trial, State::Failed, Some(why));
    }
    if let Err(why) = swap(dir, data, &dir.stage()) {
        return dir.mark(&mut trial, State::Failed, Some(why));
    }
    dir.mark(&mut trial, State::Trying, None);
    guard(dir, data, &trial.commit);
}

fn guard(dir: &Dir, data: &Path, commit: &str) {
    let exe = installed().join("Contents/MacOS").join(EXE);
    watch(
        dir,
        commit,
        || running(&exe) || crate::lifecycle::relaunching_in(data),
        || swap(dir, data, &dir.known_good()),
    );
}

fn undo(dir: &Dir, data: &Path) {
    let Some(mut trial) = dir.trial() else {
        return;
    };
    // Recorded first, so the watchdog does not take the swap for a crash.
    dir.mark(&mut trial, State::Undone, None);
    if let Err(why) = swap(dir, data, &dir.known_good()) {
        dir.mark(&mut trial, State::Failed, Some(why));
    }
}

/// Waits while the trial of `commit` runs. When no copy of the app runs any more and the trial
/// is still this one, the app stopped without quitting through the menu (which ends the trial's
/// watch, see [`quit_cleanly`]), so the known-good build goes back.
fn watch(
    dir: &Dir,
    commit: &str,
    alive: impl Fn() -> bool,
    restore: impl Fn() -> Result<(), String>,
) {
    loop {
        std::thread::sleep(WATCH);
        let still = |t: &Trial| t.state == State::Trying && t.commit == commit;
        let Some(mut trial) = dir.trial().filter(still) else {
            return;
        };
        // A swap leaves no app running for a moment: `scripts/install-app.sh` replacing the
        // trial build, say. That is not a crash.
        if alive() {
            continue;
        }
        if dir.0.join("quit").exists() {
            let _ = fs::remove_file(dir.0.join("quit"));
            return;
        }
        let why = "The trial build stopped unexpectedly, so the last kept build is back.";
        dir.mark(&mut trial, State::Restored, Some(why.into()));
        if let Err(why) = restore() {
            dir.mark(&mut trial, State::Failed, Some(why));
        }
        return;
    }
}

/// The captain kept the trial build: it stays installed, and the watchdog stops watching it.
pub fn keep(data: &Path) {
    let dir = Dir::new(data);
    if let Some(mut trial) = dir.trial().filter(|t| t.state == State::Trying) {
        dir.mark(&mut trial, State::Kept, None);
    }
}

/// The captain quit the app on purpose: the watchdog stops watching instead of restoring the
/// known-good build. The trial stays, and opening the app again runs it.
pub fn quit_cleanly(data: &Path) {
    let dir = Dir::new(data);
    if dir.trial().is_some_and(|t| t.state == State::Trying) {
        let _ = fs::write(dir.0.join("quit"), "");
    }
}

/// Whether a copy of the app at `exe` runs other than this job, whose own command line names the
/// same executable.
fn running(exe: &Path) -> bool {
    Command::new("pgrep")
        .arg("-f")
        .arg(format!("^{}( |$)", exe.display()))
        .output()
        .is_ok_and(|o| others(&String::from_utf8_lossy(&o.stdout), std::process::id()))
}

fn others(pgrep: &str, own: u32) -> bool {
    pgrep
        .split_whitespace()
        .filter_map(|pid| pid.parse::<u32>().ok())
        .any(|pid| pid != own)
}

/// Runs `command` and returns what it printed. What it says on standard error goes to the job
/// log, and its last lines are the error when it fails.
fn step(what: &str, command: &mut Command) -> Result<String, String> {
    println!("creator: {what}");
    let output = command
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Could not {what}: {e}"))?;
    let _ = io::stdout().write_all(&output.stderr);
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
    }
    let said = String::from_utf8_lossy(&output.stderr);
    let tail: Vec<&str> = said.lines().rev().take(12).collect();
    let tail: Vec<&str> = tail.into_iter().rev().collect();
    Err(format!("Could not {what}:\n{}", tail.join("\n")))
}

fn git(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(dir);
    command
}

/// Fetches the worktree's branch into the app's checkout and checks `commit` out there. Returns
/// every path the branch changed since it left the project's `main`. What `main` itself gained
/// since the installed build was reviewed and merged, so it is built but not checked.
fn prepare(dir: &Dir, worktree: &Path, commit: &str) -> Result<Vec<String>, String> {
    let checkout = dir.checkout();
    if !checkout.join(".git").exists() {
        create_private_dir(&dir.0).map_err(|e| e.to_string())?;
        step(
            "make the app's checkout",
            Command::new("git")
                .args(["clone", "--quiet", "--no-checkout"])
                .arg(worktree)
                .arg(&checkout),
        )?;
    }
    step(
        "fetch the change",
        git(&checkout)
            .args(["fetch", "--quiet", "--no-tags"])
            .arg(worktree)
            .args(["+refs/heads/*:refs/remotes/project/*", "HEAD"]),
    )?;
    let fetched = step("read the change", git(worktree).args(["rev-parse", "HEAD"]))?;
    let commit = step(
        "find the commit",
        git(&checkout).args(["rev-parse", "--verify", &format!("{commit}^{{commit}}")]),
    )?;
    if fetched != commit {
        return Err(format!(
            "The worktree has moved on from {commit} to {fetched}; ask for the change to be tried \
             again."
        ));
    }
    let base = step(
        "find where the branch left main",
        git(&checkout).args(["merge-base", "project/main", &commit]),
    )?;
    let changed = step(
        "list what the branch changed",
        git(&checkout).args(["diff", "--name-only", "--no-renames", &base, &commit]),
    )?;
    step(
        "check the change out",
        git(&checkout).args(["checkout", "--quiet", "--force", "--detach", &commit]),
    )?;
    Ok(changed.lines().map(str::to_string).collect())
}

/// Builds the checkout in the trial profile: the development profile without debug information,
/// which rebuilds a view change in seconds. Returns the binary.
fn build(dir: &Dir) -> Result<PathBuf, String> {
    step(
        "build the change",
        Command::new("cargo")
            .args(["build", "--locked", "--offline", "--manifest-path"])
            .arg(dir.checkout().join("Cargo.toml"))
            .env("CARGO_TARGET_DIR", dir.target())
            .env("CARGO_PROFILE_DEV_DEBUG", "0"),
    )?;
    Ok(dir.target().join("debug").join(EXE))
}

/// Makes the trial bundle: a copy of the installed app with `binary` in it, signed by the same
/// identity, so macOS treats it as the same app.
fn stage(dir: &Dir, binary: &Path) -> Result<(), String> {
    let stage = dir.stage();
    copy_bundle(&installed(), &stage)?;
    let exe = stage.join("Contents/MacOS").join(EXE);
    fs::copy(binary, &exe).map_err(|e| format!("Could not put the build in the bundle: {e}"))?;
    let identity = signer(&installed()).unwrap_or_else(|| "-".into());
    step(
        "sign the trial bundle",
        Command::new("codesign")
            .args(["--force", "--sign", &identity, "--identifier", ID])
            .arg(&stage),
    )?;
    Ok(())
}

/// The identity that signed `bundle`; `None` for an ad hoc signature.
fn signer(bundle: &Path) -> Option<String> {
    let output = Command::new("codesign")
        .arg("-dvv")
        .arg(bundle)
        .output()
        .ok()?;
    // codesign describes a signature on standard error.
    let said = String::from_utf8_lossy(&output.stderr).into_owned();
    said.lines()
        .find_map(|l| l.strip_prefix("Authority="))
        .map(str::to_string)
}

/// Replaces `to` with a complete copy of the bundle at `from`.
fn copy_bundle(from: &Path, to: &Path) -> Result<(), String> {
    if !from.exists() {
        return Err(format!(
            "{} is not installed. Install it with scripts/install-app.sh first.",
            from.display()
        ));
    }
    let parent = to.parent().expect("a bundle is in a folder");
    create_private_dir(parent).map_err(|e| e.to_string())?;
    if to.exists() {
        fs::remove_dir_all(to).map_err(|e| format!("Could not clear {}: {e}", to.display()))?;
    }
    step("copy the bundle", Command::new("ditto").arg(from).arg(to)).map(drop)
}

/// Swaps `bundle` in for the installed app with Rune's installer, which quits the app, exchanges
/// the bundles atomically, opens the new one, and puts the old one back if the new one is not
/// ready within 15 seconds. While it runs, the app's quit does not ask about workers in flight
/// (see `src/lifecycle.rs`).
fn swap(dir: &Dir, data: &Path, bundle: &Path) -> Result<(), String> {
    let installer = rune_installer(&dir.checkout())?;
    let marker = data.join("relaunch");
    fs::write(&marker, std::process::id().to_string())
        .map_err(|e| format!("Could not mark the relaunch: {e}"))?;
    let swapped = step(
        "swap the app",
        Command::new("python3")
            .arg(installer)
            .args(["--name", APP, "--id", ID, "--exe", EXE, "--bundle"])
            .arg(bundle),
    );
    let _ = fs::remove_file(marker);
    swapped.map(drop)
}

/// Rune's installer at the revision the checkout's `Cargo.lock` pins.
fn rune_installer(checkout: &Path) -> Result<PathBuf, String> {
    let metadata = step(
        "find Rune's installer",
        Command::new("cargo")
            .args(["metadata", "--format-version", "1", "--locked", "--offline"])
            .arg("--manifest-path")
            .arg(checkout.join("Cargo.toml")),
    )?;
    let metadata: Value = serde_json::from_str(&metadata).map_err(|e| e.to_string())?;
    let rune = metadata["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| p["name"] == "rune")
        .and_then(|p| p["manifest_path"].as_str())
        .ok_or("Cargo does not know the Rune crate.")?;
    let root = rune.rsplit_once("/crates/").map_or(rune, |(root, _)| root);
    let installer = Path::new(root).join("scripts/mac_app_install.py");
    if installer.is_file() {
        Ok(installer)
    } else {
        Err(format!(
            "Rune's installer is not at {}.",
            installer.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(list: &[&str]) -> Vec<String> {
        list.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn only_views_and_styling_pass_the_gate() {
        assert!(gate(&paths(&["src/view.rs", "src/view/settings.rs"])).is_ok());
        let refused = gate(&paths(&["src/view.rs", "src/guard.rs", "Cargo.toml"])).unwrap_err();
        assert!(refused.contains("src/guard.rs, Cargo.toml"), "{refused}");
        assert!(gate(&paths(&["src/viewer.rs"])).is_err());
        assert!(gate(&paths(&["src/creator.rs"])).is_err());
        assert!(gate(&[]).is_err(), "an empty branch has nothing to try");
    }

    #[test]
    fn the_trial_record_reads_back_and_is_the_captains_only() {
        let data = scratch("record");
        let dir = Dir::new(&data);
        let mut trial = Trial {
            state: State::Building,
            commit: "abc".into(),
            worktree: "/tmp/w".into(),
            detail: None,
            at: 0,
        };
        dir.mark(&mut trial, State::Refused, Some("why".into()));
        assert_eq!(dir.trial(), Some(trial));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.record()).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
            let mode = fs::metadata(&dir.0).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
    }

    #[test]
    fn the_job_comes_from_its_command_line() {
        let args = |list: &[&str]| paths(list);
        assert_eq!(
            job_from(&args(&[
                "velvet-hammer",
                "--creator",
                "try",
                "/d",
                "abc",
                "/w"
            ])),
            Some((
                PathBuf::from("/d"),
                Job::Try {
                    commit: "abc".into(),
                    worktree: "/w".into()
                }
            ))
        );
        assert_eq!(
            job_from(&args(&["velvet-hammer", "--creator", "undo", "/d"])),
            Some((PathBuf::from("/d"), Job::Undo))
        );
        assert_eq!(job_from(&args(&["velvet-hammer", "--scene", "chat"])), None);
        assert_eq!(
            job_from(&args(&["velvet-hammer", "--creator", "try"])),
            None
        );
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/creator-tests")
            .join(format!("{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sh(dir: &Path, script: &str) {
        let ok = Command::new("sh")
            .arg("-c")
            .arg(script)
            .current_dir(dir)
            .status()
            .unwrap()
            .success();
        assert!(ok, "{script}");
    }

    /// A project on main, then `on_main` committed there, and a worker's branch in a worktree
    /// that makes `change`.
    fn project(data: &Path, on_main: &str, change: &str) -> (PathBuf, String) {
        let repo = data.join("project");
        fs::create_dir_all(repo.join("src/view")).unwrap();
        sh(
            &repo,
            &format!(
                "git init -q -b main && git config user.email t@t && git config user.name t \
                 && echo a > src/view.rs && echo b > src/guard.rs && git add -A \
                 && git commit -qm first && {on_main} && git add -A && git commit -q --allow-empty -m main"
            ),
        );
        let worktree = data.join("worktree");
        sh(
            &repo,
            &format!(
                "git worktree add -q -b fm/change '{}' && cd '{}' && {change} && git add -A \
                 && git commit -qm change",
                worktree.display(),
                worktree.display()
            ),
        );
        let commit = String::from_utf8(
            Command::new("git")
                .arg("-C")
                .arg(&worktree)
                .args(["rev-parse", "HEAD"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap();
        (worktree, commit.trim().to_string())
    }

    #[test]
    fn a_view_change_is_fetched_into_the_apps_checkout_and_passes() {
        let data = scratch("view");
        // Main gained a change to a locked file after the installed build; it was reviewed, so
        // only the branch's own change counts.
        let (worktree, commit) = project(
            &data,
            "echo y > src/guard.rs",
            "mkdir -p src/view && echo c > src/view/card.rs",
        );
        let dir = Dir::new(&data);
        let changed = prepare(&dir, &worktree, &commit).unwrap();
        assert_eq!(changed, ["src/view/card.rs"]);
        assert!(gate(&changed).is_ok());
        assert!(dir.checkout().join("src/view/card.rs").is_file());
    }

    #[test]
    fn a_change_to_a_locked_file_is_refused() {
        let data = scratch("locked");
        let (worktree, commit) = project(&data, "true", "echo x > src/guard.rs");
        let dir = Dir::new(&data);
        let changed = prepare(&dir, &worktree, &commit).unwrap();
        assert!(gate(&changed).unwrap_err().contains("src/guard.rs"));
    }

    #[test]
    fn an_earlier_sorting_branch_does_not_stand_in_for_the_worktree() {
        let data = scratch("sorted");
        let (worktree, commit) = project(
            &data,
            "true",
            "mkdir -p src/view && echo c > src/view/card.rs",
        );
        sh(&data.join("project"), "git branch aaa-other main");
        let changed = prepare(&Dir::new(&data), &worktree, &commit).unwrap();
        assert_eq!(changed, ["src/view/card.rs"]);
    }

    #[test]
    fn the_job_does_not_count_itself_as_the_app() {
        assert!(!others("", 7));
        assert!(!others("7\n", 7));
        assert!(others("7\n9\n", 7));
    }

    fn trying(dir: &Dir, commit: &str) -> Trial {
        let mut trial = Trial {
            state: State::Building,
            commit: commit.into(),
            worktree: "/w".into(),
            detail: None,
            at: 0,
        };
        dir.mark(&mut trial, State::Trying, None);
        trial
    }

    #[test]
    fn a_crashed_trial_restores_the_known_good_build() {
        let data = scratch("crash");
        let dir = Dir::new(&data);
        trying(&dir, "abc");
        let restored = std::cell::Cell::new(false);
        watch(
            &dir,
            "abc",
            || false,
            || {
                restored.set(true);
                Ok(())
            },
        );
        assert!(restored.get());
        assert_eq!(dir.trial().unwrap().state, State::Restored);
    }

    #[test]
    fn a_trial_reopened_after_a_clean_quit_is_watched_again() {
        let data = scratch("reopen");
        let dir = Dir::new(&data);
        trying(&dir, "abc");
        quit_cleanly(&data);
        watch(&dir, "abc", || false, || panic!("restored a clean quit"));
        assert_eq!(dir.trial().unwrap().state, State::Trying);

        assert_eq!(rewatch(&dir, true), None);
        assert_eq!(rewatch(&dir, false), Some("abc".into()));
        let restored = std::cell::Cell::new(false);
        watch(
            &dir,
            "abc",
            || false,
            || {
                restored.set(true);
                Ok(())
            },
        );
        assert!(restored.get());
        assert_eq!(dir.trial().unwrap().state, State::Restored);
    }

    #[test]
    fn reopening_clears_the_quit_marker_even_when_a_watch_still_runs() {
        let data = scratch("reopen-running");
        let dir = Dir::new(&data);
        trying(&dir, "abc");
        quit_cleanly(&data);
        assert_eq!(rewatch(&dir, true), None);
        assert!(!dir.0.join("quit").exists());
    }

    #[test]
    fn the_watch_job_comes_from_its_command_line() {
        let args: Vec<String> = ["velvet-hammer", "--creator", "watch", "/d", "abc"]
            .map(String::from)
            .to_vec();
        assert_eq!(
            job_from(&args),
            Some((
                PathBuf::from("/d"),
                Job::Watch {
                    commit: "abc".into()
                }
            ))
        );
    }

    #[test]
    fn the_job_exits_without_restoring_after_keep_or_undo() {
        for end in [State::Kept, State::Undone] {
            let data = scratch("ended");
            let dir = Dir::new(&data);
            let mut trial = trying(&dir, "abc");
            dir.mark(&mut trial, end, None);
            watch(&dir, "abc", || false, || panic!("restored an ended trial"));
            assert_eq!(dir.trial().unwrap().state, end);
        }
    }

    #[test]
    fn a_new_trial_ignores_a_stale_quit_marker() {
        let data = scratch("stale-quit");
        let dir = Dir::new(&data);
        let mut trial = trying(&dir, "abc");
        quit_cleanly(&data);
        dir.mark(&mut trial, State::Kept, None);
        try_change(&dir, &data, "def".into(), "/missing".into());
        assert!(!dir.0.join("quit").exists());
    }

    #[test]
    fn a_try_while_another_trial_runs_leaves_that_trial_alone() {
        let data = scratch("busy");
        let dir = Dir::new(&data);
        let before = trying(&dir, "abc");
        try_change(&dir, &data, "def".into(), "/missing".into());
        assert_eq!(dir.trial(), Some(before));
    }

    #[test]
    fn a_worktree_that_moved_on_is_not_built() {
        let data = scratch("moved");
        let (worktree, commit) = project(
            &data,
            "true",
            "mkdir -p src/view && echo c > src/view/card.rs",
        );
        sh(
            &worktree,
            "echo d >> src/view/card.rs && git commit -qam more",
        );
        let why = prepare(&Dir::new(&data), &worktree, &commit).unwrap_err();
        assert!(why.contains("moved on"), "{why}");
    }
}
