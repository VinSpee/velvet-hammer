//! Creator mode: the captain changes Velvet Hammer from inside it, and tries the change in the
//! same session. The captain asks the chief of staff for a change to the app; a worker makes it
//! on a branch in a worktree of the app's project, builds it, commits, and stops before pushing,
//! with a last status line that says `ready to try <commit>`. The change then shows in Waiting on
//! you with **Try it** and **Discard**. Try it builds and swaps the change in ([`build`]), and the
//! new build carries the conversation, the chief of staff's session and the captain's place over
//! (see `claude.rs` and `carry.rs`). While it runs, a banner offers **Keep**, which asks the chief
//! of staff to deliver the branch the normal way (the pipeline, a pull request, and a merge the
//! captain approves), and **Undo**, which swaps the last kept build back.
//!
//! Only the captain's press swaps the app: no tool the chief of staff can call does. Creator mode
//! is off until the captain turns it on in Settings, because it needs the app built from source.

pub mod build;

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::App;
use crate::bearings::Crew;
use crate::store::{self, Saved, Store};
use build::{Job, State, Trial};

/// How often, in seconds, the trial record and the workers' status lines are read again.
const LOOK_EVERY: f64 = 1.0;

/// Whether creator mode is on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mode(pub bool);

impl Mode {
    pub const ALL: [Mode; 2] = [Mode(false), Mode(true)];

    pub fn key(self) -> &'static str {
        if self.0 { "on" } else { "off" }
    }

    pub fn label(self) -> &'static str {
        if self.0 { "On" } else { "Off" }
    }

    pub fn explanation(self) -> &'static str {
        if self.0 {
            "A change to Velvet Hammer that a worker has ready shows in Waiting on you with Try \
             it. Trying builds it from source on this Mac and swaps it in, keeping the \
             conversation. Needs the app installed with scripts/install-app.sh and a Rust \
             toolchain from rustup. Views and styling only."
        } else {
            "Changes to Velvet Hammer are delivered as pull requests only."
        }
    }
}

impl Saved for Mode {
    const FILE: &'static str = "creator.json";
    const FALLBACK: &'static str = "creator mode is off";

    fn fallback() -> Self {
        Mode(false)
    }

    fn to_json(&self) -> Value {
        json!({"version": store::VERSION, "on": self.0})
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        store::check_version(value)?;
        value["on"]
            .as_bool()
            .map(Mode)
            .ok_or_else(|| "it does not say whether creator mode is on".into())
    }
}

/// A change to the app that a worker has ready to try.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    /// The worker's task id.
    pub task: String,
    /// The task's title.
    pub name: String,
    pub commit: String,
    pub worktree: PathBuf,
    /// What the worker said about it.
    pub summary: String,
}

impl Change {
    pub fn short(&self) -> &str {
        &self.commit[..7]
    }
}

/// The changes ready to try among the tasks in flight in the firstmate home at `home`: a task
/// of the app's own project whose last status line says `ready to try <commit>`.
pub fn changes(home: &Path, crew: &[Crew]) -> Vec<Change> {
    crew.iter().filter_map(|c| change(home, c)).collect()
}

fn change(home: &Path, crew: &Crew) -> Option<Change> {
    let id = &crew.id;
    if id.is_empty() || id.contains('/') || id.starts_with('.') {
        return None;
    }
    let state = home.join("state");
    let meta = std::fs::read_to_string(state.join(format!("{id}.meta"))).ok()?;
    let field = |key: &str| {
        meta.lines()
            .rev()
            .find_map(|l| l.strip_prefix(key))
            .map(str::trim)
            .filter(|v| !v.is_empty())
    };
    let project = Path::new(field("project=")?);
    let worktree = PathBuf::from(field("worktree=")?);
    if !is_this_app(project) {
        return None;
    }
    let status = std::fs::read_to_string(state.join(format!("{id}.status"))).ok()?;
    let line = status.lines().rev().find(|l| !l.trim().is_empty())?;
    let (commit, summary) = ready(line)?;
    Some(Change {
        task: id.clone(),
        name: crew.name.clone(),
        commit,
        worktree,
        summary,
    })
}

/// Whether the project at `dir` is Velvet Hammer itself.
fn is_this_app(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join("Cargo.toml")).is_ok_and(|manifest| {
        manifest
            .lines()
            .any(|l| l.trim() == "name = \"velvet-hammer\"")
    })
}

/// The commit and the words of a status line that says `ready to try <commit>`.
fn ready(line: &str) -> Option<(String, String)> {
    const MARK: &str = "ready to try ";
    let at = line.find(MARK)?;
    let rest = &line[at + MARK.len()..];
    let commit: String = rest.chars().take_while(char::is_ascii_hexdigit).collect();
    if commit.len() != 40 {
        return None;
    }
    let after = rest[40..].trim_start_matches([':', ' ', '.', '-']).trim();
    let summary = if after.is_empty() {
        "A change to Velvet Hammer is ready to try.".to_string()
    } else {
        after.to_string()
    };
    Some((commit, summary))
}

/// Creator mode as the app holds it.
pub struct Creator {
    pub mode: Store<Mode>,
    data: PathBuf,
    /// The changes ready to try, as last read.
    pub changes: Vec<Change>,
    /// The trial, as last read.
    pub trial: Option<Trial>,
    /// Why the last Try it or Undo could not start.
    pub problem: Option<String>,
    /// Reads the workers' records and the trial record; off in scenes and tests, which set
    /// what they show.
    live: bool,
    looked_at: Option<f64>,
}

impl Creator {
    pub fn open(data: &Path) -> Creator {
        Creator {
            mode: Store::open(data),
            data: data.to_path_buf(),
            changes: Vec::new(),
            trial: None,
            problem: None,
            live: false,
            looked_at: None,
        }
    }

    /// Reads the records from now on, as the app on the desktop does.
    pub fn go_live(&mut self) {
        self.live = true;
    }

    pub fn on(&self) -> bool {
        self.mode.settings().0
    }

    /// Whether the window should wake to read the records again: creator mode is on, or a
    /// trial is under way.
    pub fn watching(&self) -> bool {
        self.live && (self.on() || self.trial.is_some())
    }

    /// The trial running now, if this build is one.
    pub fn trying(&self) -> Option<&Trial> {
        self.trial.as_ref().filter(|t| t.state == State::Trying)
    }

    /// The trial of `change`, if one was started.
    pub fn trial_of(&self, change: &Change) -> Option<&Trial> {
        self.trial.as_ref().filter(|t| t.commit == change.commit)
    }

    fn start(&mut self, job: &Job) {
        self.problem = build::start(&self.data, job)
            .err()
            .map(|e| format!("Could not start the build step: {e}"));
    }
}

impl App {
    /// Reads the trial record and the workers' records again, at most once a second.
    pub fn look_at_changes(&mut self, now: f64) -> bool {
        let creator = &mut self.creator;
        if !creator.live || creator.looked_at.is_some_and(|at| now - at < LOOK_EVERY) {
            return false;
        }
        creator.looked_at = Some(now);
        let trial = build::Dir::new(&creator.data).trial();
        let changes = if creator.on() {
            changes(&self.firstmate.home_dir(), &self.captain.crew)
        } else {
            Vec::new()
        };
        let changed = trial != creator.trial || changes != creator.changes;
        creator.trial = trial;
        creator.changes = changes;
        changed
    }

    /// Builds and swaps in `change`, at the captain's press of Try it.
    pub fn try_change(&mut self, change: &Change) {
        self.creator.start(&Job::Try {
            commit: change.commit.clone(),
            worktree: change.worktree.clone(),
        });
    }

    /// Swaps the last kept build back, at the captain's press of Undo.
    pub fn undo_change(&mut self) {
        self.creator.start(&Job::Undo);
    }

    /// Keeps the trial build and asks the chief of staff to deliver the change the normal way.
    pub fn keep_change(&mut self, change: &Change) {
        build::keep(&self.creator.data);
        self.creator.looked_at = None;
        self.say(
            format!(
                "Keep the Velvet Hammer change {} from task {}: deliver its branch the normal \
                 way, through the pipeline and a pull request. I approve the merge when it asks.",
                change.short(),
                change.task
            ),
            Vec::new(),
        );
    }

    /// Drops the change: the last kept build comes back if it is being tried, and the chief of
    /// staff is asked to have the worker drop the branch.
    pub fn discard_change(&mut self, change: &Change) {
        if self
            .creator
            .trying()
            .is_some_and(|t| t.commit == change.commit)
        {
            self.undo_change();
        }
        self.say(
            format!(
                "Discard the Velvet Hammer change {} from task {}: drop its branch; nothing of \
                 it is kept.",
                change.short(),
                change.task
            ),
            Vec::new(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn a_ready_line_gives_its_commit_and_words() {
        assert_eq!(
            ready(&format!(
                "needs-decision [at=1]: ready to try {COMMIT}: wider message cards"
            )),
            Some((COMMIT.into(), "wider message cards".into()))
        );
        assert_eq!(
            ready(&format!("paused [at=1]: ready to try {COMMIT}"))
                .unwrap()
                .1,
            "A change to Velvet Hammer is ready to try."
        );
        assert_eq!(ready("working [at=1]: ready to try abc123"), None);
        assert_eq!(ready("done [at=1]: shipped"), None);
    }

    fn home(name: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/creator-tests")
            .join(format!("changes-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("state")).unwrap();
        dir
    }

    fn task(home: &Path, id: &str, project: &Path, last: &str) -> Crew {
        let state = home.join("state");
        std::fs::write(
            state.join(format!("{id}.meta")),
            format!("worktree=/w/{id}\nproject={}\n", project.display()),
        )
        .unwrap();
        std::fs::write(
            state.join(format!("{id}.status")),
            format!("working [at=1]: started\n{last}\n"),
        )
        .unwrap();
        Crew {
            id: id.into(),
            name: format!("Task {id}"),
            state: "needs-decision".into(),
            doing: String::new(),
            runs: None,
        }
    }

    #[test]
    fn only_a_task_of_this_app_whose_last_line_is_ready_is_a_change() {
        let home = home("find");
        let app = home.join("velvet-hammer");
        let other = home.join("billing");
        for (dir, name) in [(&app, "velvet-hammer"), (&other, "billing")] {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(
                dir.join("Cargo.toml"),
                format!("[package]\nname = \"{name}\"\n"),
            )
            .unwrap();
        }
        let ready_line = format!("needs-decision [at=2]: ready to try {COMMIT}: wider cards");
        let crew = [
            task(&home, "vh-cards", &app, &ready_line),
            task(&home, "billing-fix", &other, &ready_line),
            task(&home, "vh-later", &app, "working [at=3]: still going"),
        ];
        assert_eq!(
            changes(&home, &crew),
            [Change {
                task: "vh-cards".into(),
                name: "Task vh-cards".into(),
                commit: COMMIT.into(),
                worktree: "/w/vh-cards".into(),
                summary: "wider cards".into(),
            }]
        );
    }
}
