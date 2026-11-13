//! The managed firstmate clone. Firstmate (https://github.com/kunchenguid/firstmate) is the
//! chief of staff's instructions, skills and scripts, and it ships as a git repository. The app
//! keeps its own clone in its data directory, installs it on first run, and updates it from
//! upstream on request. The clone is the user's to change: an update saves their edits as a
//! local commit and merges upstream into it, and when the two conflict the app stops and says
//! which files, without changing anything. The app only ever runs `git clone`, `fetch`, `add`,
//! `commit` and `merge` here, plus read-only queries; it never resets, cleans, forces or pushes.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, OnceLock};
use std::thread;

use rune::Waker;

/// Where the clone comes from. After install the clone's own remote decides, so pointing it at
/// a fork is a `git remote set-url` away.
pub const UPSTREAM: &str = "https://github.com/kunchenguid/firstmate";

/// The app's data directory: `$VELVET_HAMMER_DATA`, or `~/.velvet-hammer`. The default has no
/// spaces in it because firstmate's shell scripts run from inside it.
pub fn data_dir() -> PathBuf {
    match std::env::var_os("VELVET_HAMMER_DATA") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".velvet-hammer"),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Distro {
    Missing,
    Installed(Checkout),
}

/// What git says about the clone, as of the last fetch.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Checkout {
    /// The short commit id the clone is on.
    pub head: String,
    /// The upstream branch the current branch tracks, such as `origin/main`, if it tracks one.
    pub upstream: Option<String>,
    /// Local commits upstream does not have.
    pub ahead: u32,
    /// Upstream commits the clone does not have.
    pub behind: u32,
    /// Files changed or added in the working tree and not committed.
    pub edited: Vec<String>,
    /// Files in which the local changes and the waiting update conflict.
    pub conflicts: Vec<String>,
    /// A merge someone started in the clone is unfinished.
    pub merging: bool,
}

/// The one thing to tell the user about a [`Checkout`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    UpToDate,
    /// Upstream has new commits and merging them is clean, with or without local changes.
    UpdateAvailable,
    /// Local changes, and nothing new upstream.
    Modified,
    /// An update would conflict with local changes, or a merge is unfinished. The app stops here.
    Blocked,
    /// Not on a branch that tracks upstream, so there is nothing to update against.
    NotTracking,
}

impl Checkout {
    pub fn standing(&self) -> Standing {
        if self.merging || !self.conflicts.is_empty() {
            Standing::Blocked
        } else if self.upstream.is_none() {
            Standing::NotTracking
        } else if self.behind > 0 {
            Standing::UpdateAvailable
        } else if self.modified() {
            Standing::Modified
        } else {
            Standing::UpToDate
        }
    }

    pub fn modified(&self) -> bool {
        self.ahead > 0 || !self.edited.is_empty()
    }
}

/// What a background git run is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Work {
    Installing,
    Checking,
    Updating,
}

pub struct Firstmate {
    clone: PathBuf,
    url: String,
    distro: Distro,
    work: Option<Work>,
    /// Why the last install, check or update stopped, in git's own words.
    problem: Option<String>,
    tx: Sender<Finished>,
    rx: Receiver<Finished>,
    wake: Arc<OnceLock<Waker>>,
}

/// A background run's result: the clone as it now stands, if git could say, and the failure if
/// there was one.
struct Finished {
    distro: Option<Distro>,
    problem: Option<String>,
}

impl Firstmate {
    /// The clone at `data/firstmate`, as it stands on disk now. Nothing is fetched or installed.
    pub fn open(data: &Path, url: &str) -> Self {
        let clone = data.join("firstmate");
        let (distro, problem) = match inspect(&clone) {
            Ok(distro) => (distro, None),
            Err(problem) => (Distro::Missing, Some(problem)),
        };
        Self::showing(clone, url, distro, None).with_problem(problem)
    }

    /// A fixed state that runs no git, for the stub and for headless scenes of states that pass
    /// too quickly to catch.
    pub fn showing(clone: PathBuf, url: &str, distro: Distro, work: Option<Work>) -> Self {
        let (tx, rx) = channel();
        Firstmate {
            clone,
            url: url.to_string(),
            distro,
            work,
            problem: None,
            tx,
            rx,
            wake: Arc::new(OnceLock::new()),
        }
    }

    fn with_problem(mut self, problem: Option<String>) -> Self {
        self.problem = problem;
        self
    }

    pub fn connect(&mut self, wake: Waker) {
        let _ = self.wake.set(wake);
    }

    pub fn clone_dir(&self) -> &Path {
        &self.clone
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn distro(&self) -> &Distro {
        &self.distro
    }

    pub fn work(&self) -> Option<Work> {
        self.work
    }

    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    /// What the app does when it opens: install on first run, otherwise look for an update.
    pub fn start(&mut self) {
        match self.distro {
            Distro::Missing => self.install(),
            Distro::Installed(_) => self.check(),
        }
    }

    /// Clones upstream into the data directory.
    pub fn install(&mut self) {
        let url = self.url.clone();
        self.run(Work::Installing, move |clone| install(clone, &url));
    }

    /// Fetches upstream and reports how the clone stands against it. Changes no file.
    pub fn check(&mut self) {
        self.run(Work::Checking, fetch);
    }

    /// Fetches upstream and merges it, keeping local changes; does nothing when blocked.
    pub fn update(&mut self) {
        self.run(Work::Updating, update);
    }

    fn run(&mut self, work: Work, op: impl FnOnce(&Path) -> Result<(), String> + Send + 'static) {
        if self.work.is_some() {
            return;
        }
        self.work = Some(work);
        self.problem = None;
        let (clone, tx, wake) = (self.clone.clone(), self.tx.clone(), self.wake.clone());
        thread::spawn(move || {
            let mut problem = op(&clone).err();
            let distro = inspect(&clone).map_err(|e| problem.get_or_insert(e)).ok();
            if tx.send(Finished { distro, problem }).is_ok()
                && let Some(wake) = wake.get()
            {
                wake();
            }
        });
    }

    /// Takes in a finished background run; true when one finished.
    pub fn poll(&mut self) -> bool {
        let Ok(done) = self.rx.try_recv() else {
            return false;
        };
        if let Some(distro) = done.distro {
            self.distro = distro;
        }
        self.problem = done.problem;
        self.work = None;
        true
    }
}

/// Every git the app runs: never prompting, signing or reading the caller's repository, and
/// able to commit on a machine with no git identity. Saved commits are the app's, so it signs
/// them with its own name.
fn git(dir: Option<&Path>, args: &[&str]) -> Result<Ran, String> {
    let mut cmd = Command::new("git");
    if let Some(dir) = dir {
        cmd.arg("-C").arg(dir);
    }
    cmd.args([
        "-c",
        "user.name=Velvet Hammer",
        "-c",
        "user.email=velvet-hammer@localhost",
        "-c",
        "commit.gpgsign=false",
    ])
    .args(args)
    .env("GIT_TERMINAL_PROMPT", "0")
    .env("LC_ALL", "C")
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    // Set when the app is started from inside a git hook; they would redirect every command.
    for key in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE"] {
        cmd.env_remove(key);
    }
    let output = cmd.output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "git is not installed, or not on Velvet Hammer's PATH.".to_string()
        } else {
            format!("Could not run git: {e}")
        }
    })?;
    Ok(Ran {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
    })
}

struct Ran {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl Ran {
    fn ok(&self) -> bool {
        self.code == Some(0)
    }
}

/// Runs a git command that must succeed, returning what it printed or its own error.
fn git_ok(dir: &Path, args: &[&str]) -> Result<String, String> {
    let ran = git(Some(dir), args)?;
    if ran.ok() {
        Ok(ran.stdout)
    } else if ran.stderr.is_empty() {
        Err(format!("`git {}` failed.", args.join(" ")))
    } else {
        Err(ran.stderr)
    }
}

/// Reads how the clone stands from its own records, without the network.
fn inspect(clone: &Path) -> Result<Distro, String> {
    if !clone.join(".git").exists() {
        return Ok(Distro::Missing);
    }
    let head = git_ok(clone, &["rev-parse", "--short", "HEAD"])?;
    let merging = git(Some(clone), &["rev-parse", "-q", "--verify", "MERGE_HEAD"])?.ok();
    let edited: Vec<String> = git_ok(clone, &["status", "--porcelain"])?
        .lines()
        .filter_map(|line| line.get(3..))
        .map(str::to_string)
        .collect();
    let upstream = git(
        Some(clone),
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
    )?;
    let mut checkout = Checkout {
        head,
        edited,
        merging,
        ..Checkout::default()
    };
    if !upstream.ok() {
        return Ok(Distro::Installed(checkout));
    }
    let counts = git_ok(
        clone,
        &["rev-list", "--left-right", "--count", "HEAD...@{u}"],
    )?;
    let mut counts = counts.split_whitespace().map(|n| n.parse().unwrap_or(0));
    checkout.ahead = counts.next().unwrap_or(0);
    checkout.behind = counts.next().unwrap_or(0);
    checkout.upstream = Some(upstream.stdout);
    if checkout.behind > 0 && checkout.modified() && !merging {
        checkout.conflicts = conflicts(clone)?;
    }
    Ok(Distro::Installed(checkout))
}

/// The files a merge of upstream into the local changes would conflict in. Git works this out
/// in its object store; the working tree and index are not touched.
fn conflicts(clone: &Path) -> Result<Vec<String>, String> {
    // The uncommitted edits as a commit nothing points to, or nothing when there are none.
    let edits = git_ok(clone, &["stash", "create"])?;
    let ours = if edits.is_empty() { "HEAD" } else { &edits[..] };
    let merge = git(
        Some(clone),
        &[
            "merge-tree",
            "--write-tree",
            "--name-only",
            "--no-messages",
            ours,
            "@{u}",
        ],
    )?;
    match merge.code {
        Some(0) => Ok(Vec::new()),
        // The merged tree's id, then the conflicted files.
        Some(1) => Ok(merge.stdout.lines().skip(1).map(str::to_string).collect()),
        _ => Err(merge.stderr),
    }
}

fn install(clone: &Path, url: &str) -> Result<(), String> {
    if clone.exists() {
        return Ok(());
    }
    let data = clone
        .parent()
        .expect("the clone lives in the data directory");
    std::fs::create_dir_all(data)
        .map_err(|e| format!("Could not create {}: {e}", data.display()))?;
    // Cloned beside its final place and renamed, so an interrupted install never looks
    // installed. The leftovers of one are the app's own and are replaced.
    let partial = clone.with_extension("partial");
    if partial.exists() {
        std::fs::remove_dir_all(&partial)
            .map_err(|e| format!("Could not remove {}: {e}", partial.display()))?;
    }
    let ran = git(None, &["clone", "--quiet", url, &partial.to_string_lossy()])?;
    if !ran.ok() {
        return Err(ran.stderr);
    }
    std::fs::rename(&partial, clone)
        .map_err(|e| format!("Could not move the clone into {}: {e}", clone.display()))
}

fn fetch(clone: &Path) -> Result<(), String> {
    git_ok(clone, &["fetch", "--quiet"]).map(drop)
}

fn update(clone: &Path) -> Result<(), String> {
    fetch(clone)?;
    if !update_is_clean(clone)? {
        return Ok(());
    }
    if !git_ok(clone, &["status", "--porcelain"])?.is_empty() {
        git_ok(clone, &["add", "-A"])?;
        git_ok(
            clone,
            &[
                "commit",
                "--quiet",
                "-m",
                "Local changes saved by Velvet Hammer",
            ],
        )?;
        // New files joined the history just now, so ask again whether the merge is clean.
        if !update_is_clean(clone)? {
            return Ok(());
        }
    }
    git_ok(clone, &["merge", "--quiet", "--no-edit", "@{u}"]).map(drop)
}

fn update_is_clean(clone: &Path) -> Result<bool, String> {
    Ok(matches!(
        inspect(clone)?,
        Distro::Installed(c) if c.standing() == Standing::UpdateAvailable
    ))
}

/// Scratch firstmate repositories for tests and headless scenes: a small local "upstream" to
/// clone from, and the edits each side makes. They live under `target/`, so `cargo clean`
/// removes them.
pub mod lab {
    use super::*;

    pub struct Lab {
        pub upstream: PathBuf,
        pub data: PathBuf,
    }

    impl Lab {
        /// A fresh upstream with one commit, and an empty data directory.
        pub fn new() -> Lab {
            use std::sync::atomic::{AtomicU32, Ordering};
            static RUN: AtomicU32 = AtomicU32::new(0);
            let root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target/firstmate-lab")
                .join(format!(
                    "{}-{}",
                    std::process::id(),
                    RUN.fetch_add(1, Ordering::Relaxed)
                ));
            let _ = std::fs::remove_dir_all(&root);
            let lab = Lab {
                upstream: root.join("upstream"),
                data: root.join("data"),
            };
            std::fs::create_dir_all(&lab.upstream).expect("scratch directory");
            git_ok(&lab.upstream, &["init", "--quiet", "-b", "main"]).expect("git init");
            lab.release("AGENTS.md", "# Firstmate\n\nYou are the first mate.\n");
            lab
        }

        pub fn url(&self) -> String {
            self.upstream.to_string_lossy().into_owned()
        }

        pub fn clone_dir(&self) -> PathBuf {
            self.data.join("firstmate")
        }

        pub fn firstmate(&self) -> Firstmate {
            Firstmate::open(&self.data, &self.url())
        }

        /// Upstream publishes `text` as `file`.
        pub fn release(&self, file: &str, text: &str) {
            std::fs::write(self.upstream.join(file), text).expect("write upstream file");
            git_ok(&self.upstream, &["add", "-A"]).expect("git add");
            git_ok(&self.upstream, &["commit", "--quiet", "-m", file]).expect("git commit");
        }

        /// The user changes `file` in the managed clone, without committing.
        pub fn edit(&self, file: &str, text: &str) {
            std::fs::write(self.clone_dir().join(file), text).expect("write clone file");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::lab::Lab;
    use super::*;
    use std::time::{Duration, Instant};

    fn finish(fm: &mut Firstmate) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !fm.poll() {
            assert!(Instant::now() < deadline, "git never finished");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn installed(lab: &Lab) -> Firstmate {
        let mut fm = lab.firstmate();
        fm.install();
        finish(&mut fm);
        assert_eq!(fm.problem(), None);
        fm
    }

    fn checkout(fm: &Firstmate) -> &Checkout {
        match fm.distro() {
            Distro::Installed(checkout) => checkout,
            Distro::Missing => panic!("the clone is missing: {:?}", fm.problem()),
        }
    }

    fn read(lab: &Lab, file: &str) -> String {
        std::fs::read_to_string(lab.clone_dir().join(file)).expect("read clone file")
    }

    fn commits(lab: &Lab) -> String {
        git_ok(&lab.clone_dir(), &["rev-list", "--count", "HEAD"]).unwrap()
    }

    #[test]
    fn the_first_run_installs_the_clone_into_the_data_directory() {
        let lab = Lab::new();
        let mut fm = lab.firstmate();
        assert_eq!(fm.distro(), &Distro::Missing);
        fm.start();
        assert_eq!(fm.work(), Some(Work::Installing));
        finish(&mut fm);
        assert_eq!(checkout(&fm).standing(), Standing::UpToDate);
        assert_eq!(checkout(&fm).upstream.as_deref(), Some("origin/main"));
        assert!(read(&lab, "AGENTS.md").contains("first mate"));
        // Opening again finds it installed and installs nothing.
        assert_eq!(lab.firstmate().distro(), fm.distro());
    }

    #[test]
    fn a_failed_install_leaves_nothing_behind_and_says_why() {
        let lab = Lab::new();
        let mut fm = Firstmate::open(&lab.data, "/nonexistent/firstmate");
        fm.install();
        finish(&mut fm);
        assert_eq!(fm.distro(), &Distro::Missing);
        assert!(
            fm.problem()
                .is_some_and(|p| p.contains("/nonexistent/firstmate"))
        );
        assert!(!lab.clone_dir().exists());
    }

    #[test]
    fn an_interrupted_install_is_replaced_by_the_next_one() {
        let lab = Lab::new();
        let partial = lab.data.join("firstmate.partial");
        std::fs::create_dir_all(partial.join(".git")).unwrap();
        assert_eq!(lab.firstmate().distro(), &Distro::Missing);
        installed(&lab);
        assert!(!partial.exists());
    }

    #[test]
    fn an_update_fast_forwards_a_clean_clone() {
        let lab = Lab::new();
        let mut fm = installed(&lab);
        lab.release("bin-fm.sh", "echo two\n");
        // Nothing is known until the app fetches.
        assert_eq!(checkout(&fm).standing(), Standing::UpToDate);
        fm.check();
        finish(&mut fm);
        assert_eq!(checkout(&fm).standing(), Standing::UpdateAvailable);
        assert_eq!(checkout(&fm).behind, 1);
        assert!(!lab.clone_dir().join("bin-fm.sh").exists());
        fm.update();
        finish(&mut fm);
        assert_eq!(checkout(&fm).standing(), Standing::UpToDate);
        assert_eq!(read(&lab, "bin-fm.sh"), "echo two\n");
        assert_eq!(commits(&lab), "2");
    }

    #[test]
    fn an_edit_is_reported_and_kept_across_an_update() {
        let lab = Lab::new();
        let mut fm = installed(&lab);
        lab.edit("AGENTS.md", "# Firstmate\n\nYou are my first mate.\n");
        lab.edit("notes.md", "mine\n");
        fm.check();
        finish(&mut fm);
        assert_eq!(checkout(&fm).standing(), Standing::Modified);
        assert_eq!(checkout(&fm).edited, ["AGENTS.md", "notes.md"]);

        lab.release("bin-fm.sh", "echo two\n");
        fm.check();
        finish(&mut fm);
        assert_eq!(checkout(&fm).standing(), Standing::UpdateAvailable);
        fm.update();
        finish(&mut fm);
        assert_eq!(fm.problem(), None);
        // The edits are saved as a local commit and upstream is merged into it.
        let after = checkout(&fm);
        assert_eq!(after.standing(), Standing::Modified);
        assert_eq!((after.behind, after.edited.len()), (0, 0));
        assert_eq!(
            read(&lab, "AGENTS.md"),
            "# Firstmate\n\nYou are my first mate.\n"
        );
        assert_eq!(read(&lab, "notes.md"), "mine\n");
        assert_eq!(read(&lab, "bin-fm.sh"), "echo two\n");
    }

    #[test]
    fn edits_to_different_parts_of_one_file_merge() {
        let lab = Lab::new();
        let body = "one\n\n\n\n\n\n\n\nnine\n";
        lab.release("skill.md", body);
        let mut fm = installed(&lab);
        lab.edit("skill.md", &body.replace("one", "uno"));
        lab.release("skill.md", &body.replace("nine", "nueve"));
        fm.update();
        finish(&mut fm);
        assert_eq!(fm.problem(), None);
        assert_eq!(read(&lab, "skill.md"), "uno\n\n\n\n\n\n\n\nnueve\n");
    }

    #[test]
    fn a_conflicting_update_stops_and_changes_nothing() {
        let lab = Lab::new();
        let mut fm = installed(&lab);
        lab.edit("AGENTS.md", "# Firstmate\n\nYou are my first mate.\n");
        lab.release("AGENTS.md", "# Firstmate\n\nYou are the bosun.\n");
        fm.check();
        finish(&mut fm);
        assert_eq!(checkout(&fm).standing(), Standing::Blocked);
        assert_eq!(checkout(&fm).conflicts, ["AGENTS.md"]);

        let before = fm.distro().clone();
        fm.update();
        finish(&mut fm);
        assert_eq!(fm.distro(), &before);
        assert_eq!(
            read(&lab, "AGENTS.md"),
            "# Firstmate\n\nYou are my first mate.\n"
        );
        assert_eq!(commits(&lab), "1");
    }

    #[test]
    fn a_new_local_file_that_upstream_also_adds_blocks_after_saving() {
        let lab = Lab::new();
        let mut fm = installed(&lab);
        lab.edit("notes.md", "mine\n");
        lab.release("notes.md", "theirs\n");
        fm.update();
        finish(&mut fm);
        // The file is saved as a local commit, and the merge is not attempted.
        let after = checkout(&fm);
        assert_eq!(after.standing(), Standing::Blocked);
        assert_eq!(after.conflicts, ["notes.md"]);
        assert_eq!((after.ahead, after.merging), (1, false));
        assert_eq!(read(&lab, "notes.md"), "mine\n");
    }

    #[test]
    fn a_merge_the_user_left_unfinished_blocks_updates() {
        let lab = Lab::new();
        let mut fm = installed(&lab);
        lab.edit("AGENTS.md", "mine\n");
        git_ok(&lab.clone_dir(), &["commit", "--quiet", "-am", "mine"]).unwrap();
        lab.release("AGENTS.md", "theirs\n");
        fm.check();
        finish(&mut fm);
        assert!(git_ok(&lab.clone_dir(), &["merge", "origin/main"]).is_err());
        fm.update();
        finish(&mut fm);
        assert!(checkout(&fm).merging);
        assert_eq!(checkout(&fm).standing(), Standing::Blocked);
        assert!(read(&lab, "AGENTS.md").contains("<<<<<<<"));
    }

    #[test]
    fn a_clone_off_its_tracking_branch_is_left_alone() {
        let lab = Lab::new();
        let mut fm = installed(&lab);
        git_ok(&lab.clone_dir(), &["switch", "--quiet", "--detach"]).unwrap();
        lab.release("bin-fm.sh", "echo two\n");
        fm.update();
        finish(&mut fm);
        assert_eq!(fm.problem(), None);
        assert_eq!(checkout(&fm).standing(), Standing::NotTracking);
        assert!(!lab.clone_dir().join("bin-fm.sh").exists());
    }
}
