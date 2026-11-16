//! The fleet's status and what waits on the captain, read from firstmate's own records. Firstmate
//! publishes them as one structured document, `bin/fm-bearings-snapshot.sh --json`, and asks every
//! view to render that instead of parsing its record files. The app runs that script from the
//! managed clone against the app's firstmate home, on a background thread, when something says the
//! last run is out of date (see [`crate::refresh`]). It reads the two things firstmate marks as the
//! captain's: tasks held for the captain, and pull requests whose checks are green and which wait
//! for a merge approval. It also reads the tasks in flight, which are the crewmates the chief of
//! staff manages, what landed lately, and what is charted next. Nothing in the home is written;
//! the last snapshot is kept in the app's data directory, so the panel shows it at once at launch.
//!
//! For second mates (see [`crate::secondmates`]) it reads how each one stands, from
//! `secondmates[]`, which `--all-secondmates` keeps whole, and whether its session is up, from
//! the endpoint of each task, which the script prints with `--fields endpoints`.
//!
//! A [`Peek`] shows one crewmate's terminal, read with firstmate's `bin/fm-peek.sh`. It is
//! read-only too: steering a crewmate is the chief of staff's job.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use jiff::Timestamp;
use rune::Waker;
use serde_json::{Value, json};

use crate::firstmate::Place;
use crate::refresh::Schedule;

/// How often the watcher looks at the times firstmate's status and task records last changed.
/// Looking costs a directory listing; the snapshot itself runs only when one changed.
const POLL: Duration = Duration::from_secs(2);

/// How long after a failed snapshot to run it again when nothing in the home has changed.
const RETRY: Duration = Duration::from_secs(30);

/// How long a snapshot waits after the first change, so changes made together share one run.
const SETTLE: Duration = Duration::from_millis(300);

/// The least time between two snapshots that changes asked for. Tests use a shorter one so
/// that a test which changes the records twice does not wait for it; [`crate::refresh`]'s own
/// tests cover the real one.
const GAP: Duration = if cfg!(test) {
    Duration::from_millis(500)
} else {
    Duration::from_secs(5)
};

/// The file in the app's data directory that keeps the last snapshot, for the next launch.
pub const CACHE: &str = "bearings-snapshot.json";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bearings {
    /// A snapshot was read: until then the rest is empty because nothing is known yet.
    pub read: bool,
    pub holds: Vec<Hold>,
    pub merges: Vec<Merge>,
    pub crew: Vec<Crew>,
    /// How each second mate stands, by firstmate's own reading of its home.
    pub mates: Vec<MateState>,
    /// The endpoint of each task in this home, a second mate's session among them.
    pub endpoints: Vec<Endpoint>,
    /// Work that finished lately, newest first.
    pub landed: Vec<Landed>,
    /// Work charted next: queued tasks and why each waits.
    pub gates: Vec<Gate>,
}

/// Work that finished: `landed[]` in the snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Landed {
    pub id: String,
    /// What it was, in a line.
    pub what: String,
    /// Its pull request's address, its report's path, or a local note; `None` when firstmate
    /// recorded none.
    pub artifact: Option<String>,
    /// The second mate whose home it landed in, or `None` for this home.
    pub owner: Option<String>,
}

/// Work charted next: `gates[]` in the snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gate {
    pub id: String,
    pub title: String,
    /// Why it waits, in firstmate's words; empty when it gave none.
    pub reason: String,
    /// The tasks it waits for, comma-separated; empty when none.
    pub blocked_by: String,
    pub owner: Option<String>,
}

/// How a second mate stands: `secondmates[]` in the snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MateState {
    pub id: String,
    /// Firstmate's word: `no_active_work`, `active_child_work`, `captain_decision`,
    /// `externally_held` or `unknown`.
    pub state: String,
    /// One line on what it is doing, or why its state is unknown.
    pub doing: String,
    /// The mate's reports on the parent channel contradict its own home's records.
    pub contradiction: bool,
    /// Why the state is unknown, in firstmate's words; `-` when there is no reason.
    pub reason: String,
}

/// A task's terminal endpoint: whether it exists and whether its agent runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    pub id: String,
    pub exists: bool,
    /// Firstmate's word for the agent: `alive`, `dead`, `unknown` or `not_checked`.
    pub agent: String,
}

/// A task in flight: a crewmate (or a secondmate's child) the chief of staff manages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Crew {
    /// The task id, which `bin/fm-peek.sh` takes.
    pub id: String,
    /// The task's title, or its id when it has none.
    pub name: String,
    /// Firstmate's word for where it stands, such as `working`, `parked` or `blocked`.
    pub state: String,
    /// One line on what it is doing now.
    pub doing: String,
    /// What the worker runs on, such as "pi · opencode-go/deepseek-v4.1-flash", from the
    /// `harness=` and `model=` lines of firstmate's `state/<id>.meta`. The snapshot does not
    /// carry it.
    pub runs: Option<String>,
}

/// A task held for the captain: firstmate's one record of a captain call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hold {
    /// The task id, which is also the key an answer names.
    pub key: String,
    /// The task's title and the whole hold reason, as firstmate wrote them.
    pub question: String,
    /// The second mate whose home holds it, or `None` for this home.
    pub owner: Option<String>,
}

/// A pull request firstmate reports as green and waiting for the captain to approve the merge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Merge {
    pub task: String,
    pub url: String,
    pub reason: String,
}

impl Bearings {
    /// Reads an `fm-bearings.v1` document.
    pub fn parse(json: &[u8]) -> Result<Bearings, String> {
        let doc: Value = serde_json::from_slice(json)
            .map_err(|e| format!("firstmate's bearings snapshot is not JSON: {e}"))?;
        if doc["schema"] != "fm-bearings.v1" {
            return Err(format!(
                "firstmate's bearings snapshot has schema {}, and Velvet Hammer reads \
                 fm-bearings.v1.",
                doc["schema"]
            ));
        }
        let rows = |value: &Value| value.as_array().cloned().unwrap_or_default();
        let word = |row: &Value, field: &str| row[field].as_str().unwrap_or_default().to_string();
        // Firstmate writes `-` for a field it has nothing for, and `(main)` for this home.
        let given = |row: &Value, field: &str| {
            row[field]
                .as_str()
                .filter(|v| !v.is_empty() && *v != "-")
                .map(str::to_string)
        };
        let owner = |row: &Value| given(row, "owner").filter(|o| o != "(main)");
        Ok(Bearings {
            read: true,
            holds: rows(&doc["decisions_open"])
                .iter()
                .map(|row| Hold {
                    key: word(row, "key"),
                    question: word(row, "question"),
                    owner: owner(row),
                })
                .collect(),
            // A row with a hold is the same captain call as that hold, already listed above.
            merges: rows(&doc["contributions"]["captain"])
                .iter()
                .filter(|row| row["hold"].is_null())
                .map(|row| Merge {
                    task: word(row, "task"),
                    url: word(row, "url"),
                    reason: word(row, "reason"),
                })
                .collect(),
            crew: rows(&doc["in_flight"])
                .iter()
                .map(|row| Crew {
                    id: word(row, "id"),
                    name: word(row, "name"),
                    state: word(row, "state"),
                    doing: word(row, "doing"),
                    runs: None,
                })
                .collect(),
            mates: rows(&doc["secondmates"])
                .iter()
                .map(|row| MateState {
                    id: word(row, "id"),
                    state: word(row, "state"),
                    doing: word(row, "doing"),
                    contradiction: row["contradiction"] == true,
                    reason: word(row, "reason"),
                })
                .collect(),
            endpoints: rows(&doc["endpoints"])
                .iter()
                .map(|row| Endpoint {
                    id: word(row, "id"),
                    exists: row["exists"] == true,
                    agent: word(row, "agent"),
                })
                .collect(),
            landed: rows(&doc["landed"])
                .iter()
                .map(|row| Landed {
                    id: word(row, "id"),
                    what: word(row, "what"),
                    artifact: given(row, "artifact"),
                    owner: owner(row),
                })
                .collect(),
            gates: rows(&doc["gates"])
                .iter()
                .map(|row| Gate {
                    id: word(row, "id"),
                    title: word(row, "title"),
                    reason: given(row, "reason").unwrap_or_default(),
                    blocked_by: given(row, "blocked_by").unwrap_or_default(),
                    owner: owner(row),
                })
                .collect(),
        })
    }

    pub fn is_empty(&self) -> bool {
        self.holds.is_empty() && self.merges.is_empty()
    }

    pub fn mate(&self, id: &str) -> Option<&MateState> {
        self.mates.iter().find(|m| m.id == id)
    }

    pub fn endpoint(&self, id: &str) -> Option<&Endpoint> {
        self.endpoints.iter().find(|e| e.id == id)
    }
}

/// A snapshot the watcher read, and when.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub bearings: Bearings,
    /// When the script finished.
    pub at: Timestamp,
    /// It was kept from an earlier launch and is shown until this launch's first run finishes.
    pub kept: bool,
}

/// What the watcher reports, in the order it happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Update {
    /// A run started; the last snapshot still stands until it finishes.
    Running,
    Read(Snapshot),
    /// The run failed, in firstmate's words; the last snapshot still stands.
    Failed(String),
}

/// What the app tells the watcher.
enum Ask {
    /// Something the snapshot reads may have changed, such as a turn that finished.
    Invalidate,
    /// The captain pressed Refresh.
    Refresh,
}

/// The watcher's timings, shorter in tests.
#[derive(Clone, Copy)]
struct Timing {
    poll: Duration,
    retry: Duration,
    settle: Duration,
    gap: Duration,
}

const TIMING: Timing = Timing {
    poll: POLL,
    retry: RETRY,
    settle: SETTLE,
    gap: GAP,
};

/// Keeps a [`Bearings`] current for one firstmate home until dropped.
pub struct Watcher {
    rx: Receiver<Update>,
    asks: Sender<Ask>,
    wake: Arc<OnceLock<Waker>>,
}

impl Watcher {
    /// Watches the records of the home at `place` with the scripts of its checkout, keeping the
    /// last snapshot at `cache`. It waits quietly while the checkout is not installed.
    pub fn start(place: &Place, cache: &Path) -> Watcher {
        Watcher::start_with(place, cache, TIMING)
    }

    fn start_with(place: &Place, cache: &Path, timing: Timing) -> Watcher {
        let (tx, rx) = channel();
        let (asks, asked) = channel();
        let wake: Arc<OnceLock<Waker>> = Arc::new(OnceLock::new());
        let (place, cache) = (place.clone(), cache.to_path_buf());
        let waker = wake.clone();
        thread::spawn(move || {
            let send = |update: Update| {
                let sent = tx.send(update).is_ok();
                if let Some(wake) = waker.get() {
                    wake();
                }
                sent
            };
            if let Some(kept) = read_cache(&cache, &place.home)
                && !send(Update::Read(kept))
            {
                return;
            }
            let mut schedule = Schedule::new(timing.settle, timing.gap);
            let mut seen = None;
            let mut failed_at: Option<Instant> = None;
            loop {
                let script = place.code.join("bin/fm-bearings-snapshot.sh");
                let now = Instant::now();
                let present = script.exists();
                if present {
                    let stamp = Some(stamp(&place.home));
                    if seen.is_none() {
                        // The first run of a launch does not wait.
                        schedule.refresh(now);
                    } else if stamp != seen {
                        schedule.invalidate(now);
                    }
                    seen = stamp;
                    if failed_at.is_some_and(|at| at.elapsed() >= timing.retry) {
                        failed_at = None;
                        schedule.invalidate(now);
                    }
                    if schedule.start(now) {
                        if !send(Update::Running) {
                            return;
                        }
                        let update = match snapshot(&script, &place) {
                            Ok((bearings, json)) => {
                                failed_at = None;
                                let at = Timestamp::now();
                                write_cache(&cache, &place.home, &json, at);
                                Update::Read(Snapshot {
                                    bearings,
                                    at,
                                    kept: false,
                                })
                            }
                            Err(problem) => {
                                failed_at = Some(Instant::now());
                                Update::Failed(problem)
                            }
                        };
                        if !send(update) {
                            return;
                        }
                        continue;
                    }
                }
                let wait = if present {
                    schedule
                        .wait(now)
                        .map_or(timing.poll, |w| w.min(timing.poll))
                } else {
                    timing.poll
                };
                match asked.recv_timeout(wait) {
                    Ok(Ask::Invalidate) => schedule.invalidate(Instant::now()),
                    Ok(Ask::Refresh) => schedule.refresh(Instant::now()),
                    Err(RecvTimeoutError::Timeout) => {}
                    // The watcher was dropped.
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
        });
        Watcher { rx, asks, wake }
    }

    pub fn connect(&mut self, wake: Waker) {
        let _ = self.wake.set(wake);
        // A snapshot may have arrived before the waker did.
        if let Some(wake) = self.wake.get() {
            wake();
        }
    }

    /// What happened since the last call, oldest first.
    pub fn poll(&mut self) -> Vec<Update> {
        self.rx.try_iter().collect()
    }

    /// Something the snapshot reads may have changed: run it again soon.
    pub fn invalidate(&self) {
        let _ = self.asks.send(Ask::Invalidate);
    }

    /// Runs the snapshot again now, or as soon as the run under way finishes.
    pub fn refresh(&self) {
        let _ = self.asks.send(Ask::Refresh);
    }
}

/// The snapshot kept at `cache`, if it is for `home`.
fn read_cache(cache: &Path, home: &Path) -> Option<Snapshot> {
    let doc: Value = serde_json::from_slice(&std::fs::read(cache).ok()?).ok()?;
    if doc["home"].as_str() != Some(&home.to_string_lossy()) {
        return None;
    }
    let mut bearings = Bearings::parse(doc["snapshot"].to_string().as_bytes()).ok()?;
    add_runs(&mut bearings, home);
    Some(Snapshot {
        bearings,
        at: doc["at"].as_str()?.parse().ok()?,
        kept: true,
    })
}

/// Keeps firstmate's document `json`, read at `at`, at `cache` for the next launch. A cache that
/// cannot be written only means the next launch starts empty.
pub fn write_cache(cache: &Path, home: &Path, json: &[u8], at: Timestamp) {
    let Ok(snapshot) = serde_json::from_slice::<Value>(json) else {
        return;
    };
    let doc = json!({
        "home": home.to_string_lossy(),
        "at": at.to_string(),
        "snapshot": snapshot,
    });
    let partial = cache.with_extension("json.partial");
    if std::fs::write(&partial, doc.to_string()).is_ok() {
        let _ = std::fs::rename(&partial, cache);
    }
}

/// How many lines of a crewmate's terminal a peek shows; `bin/fm-peek.sh`'s own default.
const PEEK_LINES: &str = "40";

/// One crewmate's terminal, kept current with `bin/fm-peek.sh` until dropped.
pub struct Peek {
    pub id: String,
    /// The terminal's last lines, or why firstmate could not read them. `None` until the
    /// first read finishes.
    pub pane: Option<Result<String, String>>,
    rx: Receiver<Result<String, String>>,
    stop: Arc<AtomicBool>,
}

impl Peek {
    /// Reads crewmate `id`'s terminal every [`POLL`], calling `wake` when what it shows changes.
    pub fn start(id: &str, clone: &Path, home: &Path, wake: Option<Waker>) -> Peek {
        let (tx, rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let (task, clone, home) = (id.to_string(), clone.to_path_buf(), home.to_path_buf());
        let stopped = stop.clone();
        thread::spawn(move || {
            let mut shown = None;
            while !stopped.load(Ordering::Relaxed) {
                let pane = peek(&task, &clone, &home);
                if shown.as_ref() != Some(&pane) {
                    shown = Some(pane.clone());
                    if tx.send(pane).is_err() {
                        return;
                    }
                    if let Some(wake) = &wake {
                        wake();
                    }
                }
                thread::sleep(POLL);
            }
        });
        Peek {
            id: id.to_string(),
            pane: None,
            rx,
            stop,
        }
    }

    /// Takes in the newest read; true when one arrived.
    pub fn poll(&mut self) -> bool {
        match self.rx.try_iter().last() {
            Some(pane) => {
                self.pane = Some(pane);
                true
            }
            None => false,
        }
    }
}

impl Drop for Peek {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn peek(task: &str, clone: &Path, home: &Path) -> Result<String, String> {
    let output = Command::new(clone.join("bin/fm-peek.sh"))
        .args([task, PEEK_LINES])
        .current_dir(clone)
        .env("FM_HOME", home)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Could not run firstmate's peek: {e}"))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_string());
    }
    // Firstmate prints its supervision banner first; the reason the read failed is last.
    let said = String::from_utf8_lossy(&output.stderr);
    Err(
        match said.lines().rev().find(|line| !line.trim().is_empty()) {
            Some(reason) => reason.trim().to_string(),
            None => format!("Firstmate's peek failed ({}).", output.status),
        },
    )
}

/// When the records the snapshot reads last changed: the status logs workers append to and the
/// task records (`state/*.status` and `state/*.meta`), a second mate's published summary
/// (`state/home-summary.json`), and anything directly in `data/`, where firstmate keeps its
/// backlog and reports. The rest of `state/` is firstmate's own bookkeeping, such as locks and
/// counters, which changes far more often and never changes the snapshot. A second mate's home
/// counts too.
fn stamp(home: &Path) -> SystemTime {
    let mut newest = SystemTime::UNIX_EPOCH;
    let mates = crate::secondmates::registry(home).unwrap_or_default();
    let mate_states = mates.iter().map(|m| Path::new(&m.home).join("state"));
    let states = [home.join("state")].into_iter().chain(mate_states);
    let record = |name: &str| {
        name.ends_with(".status") || name.ends_with(".meta") || name == "home-summary.json"
    };
    let watched = states
        .map(|dir| (dir, true))
        .chain([(home.join("data"), false)]);
    for (dir, only_records) in watched {
        let entries = std::fs::read_dir(&dir).into_iter().flatten().flatten();
        let paths = entries
            .filter(|e| !only_records || record(&e.file_name().to_string_lossy()))
            .map(|e| e.path());
        for modified in paths.filter_map(|p| p.metadata().ok()?.modified().ok()) {
            newest = newest.max(modified);
        }
    }
    newest
}

/// Runs the snapshot: what it read, and firstmate's document as it printed it.
fn snapshot(script: &PathBuf, place: &Place) -> Result<(Bearings, Vec<u8>), String> {
    let home = &place.home;
    place
        .prepare()
        .map_err(|e| format!("Could not create {}: {e}", home.display()))?;
    let output = Command::new(script)
        .args(["--json", "--fields", "endpoints", "--all-secondmates"])
        .current_dir(&place.code)
        .env("FM_HOME", home)
        // Firstmate's own switch for a snapshot that reads no forge.
        .env("FM_CREW_STATE_NO_FORGE", "1")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Could not run firstmate's bearings snapshot: {e}"))?;
    if !output.status.success() {
        let said = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if said.is_empty() {
            format!("Firstmate's bearings snapshot failed ({}).", output.status)
        } else {
            said
        });
    }
    let mut bearings = Bearings::parse(&output.stdout)?;
    add_runs(&mut bearings, home);
    Ok((bearings, output.stdout))
}

fn add_runs(bearings: &mut Bearings, home: &Path) {
    for crew in &mut bearings.crew {
        crew.runs = runs(home, &crew.id);
    }
}

/// What task `id` runs on, from its `state/<id>.meta`: the harness, and the model when firstmate
/// recorded one other than `default`.
pub fn runs(home: &Path, id: &str) -> Option<String> {
    if id.is_empty() || id.contains('/') || id.starts_with('.') {
        return None;
    }
    let meta = std::fs::read_to_string(home.join("state").join(format!("{id}.meta"))).ok()?;
    let field = |key: &str| {
        meta.lines()
            .rev()
            .find_map(|l| l.strip_prefix(key))
            .map(str::trim)
            .filter(|v| !v.is_empty() && *v != "default" && *v != "-")
    };
    let harness = field("harness=")?;
    Some(match field("model=") {
        Some(model) => format!("{harness} \u{b7} {model}"),
        None => harness.to_string(),
    })
}

/// What `bin/fm-bearings-snapshot.sh --json` printed for a lab home with two captain holds
/// (firstmate 427056c4). The two `contributions.captain` rows were added by hand in the shape
/// firstmate's `bin/fm-contributions.jq` writes: one merge approval, and one row that carries a
/// hold. The two `in_flight` rows are also added by hand, in the shape the same script printed
/// for a lab task. Tests and headless scenes use it.
pub const SNAPSHOT: &str = include_str!("../scripts/fixtures/bearings.json");

/// A snapshot with five second mates, one in each state the app shows: working with two child
/// tasks, idle, stopped, a home that does not match the registry, and waiting on the captain.
/// Its shape is what `bin/fm-bearings-snapshot.sh --json --fields endpoints` printed for a
/// scratch home with a second mate seeded by `bin/fm-home-seed.sh` (firstmate 427056c4): the
/// idle, stopped, mismatch and escalation rows were each printed by the real script, then
/// renamed. The working mate's rows and the `alive` agents are written by hand in the same
/// shape, because firstmate reads a child as working, and an agent as alive, only with a real
/// agent in the terminal.
pub const MATES: &str = include_str!("../scripts/fixtures/bearings-mates.json");

/// A fleet with something in every section: a captain hold, a merge approval, a worker working
/// and one blocked, two landed (a merged pull request and a scout's report) and two charted next,
/// one waiting on another task. `bin/fm-bearings-snapshot.sh --json --fields endpoints
/// --all-secondmates` printed it for a scratch home seeded with a backlog, task records and a hold
/// made with `bin/fm-captain-hold.sh` (firstmate 427056c4). The workers' states and endpoints and
/// the merge approval are written by hand in the same shape, because firstmate reads them only
/// from a live terminal and a real pull request.
pub const FLEET: &str = include_str!("../scripts/fixtures/bearings-fleet.json");

/// What the same script printed for an empty scratch home.
pub const EMPTY: &str = include_str!("../scripts/fixtures/bearings-empty.json");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holds_and_merge_approvals_are_read_from_a_real_snapshot() {
        let bearings = Bearings::parse(SNAPSHOT.as_bytes()).unwrap();
        assert_eq!(
            bearings.holds,
            [
                Hold {
                    key: "lab-window-size".into(),
                    question: "Default window size: Pick the default window size: 1100x720 \
                               (fits small laptops) or 1280x800 (shows more chat)."
                        .into(),
                    owner: None,
                },
                Hold {
                    key: "lab-gh-login".into(),
                    question: "Sign in to GitHub: Run gh auth login in a terminal; agents \
                               cannot sign in for you."
                        .into(),
                    owner: None,
                },
            ]
        );
        assert_eq!(
            bearings.crew[0],
            Crew {
                id: "lab-verify".into(),
                name: "Render every state headlessly".into(),
                state: "working".into(),
                doing: "rendering chat, empty and peek scenes".into(),
                runs: None,
            }
        );
        assert_eq!(bearings.crew.len(), 2);
        // The row that carries a hold is not listed twice.
        assert_eq!(
            bearings.merges,
            [Merge {
                task: "lab-panels".into(),
                url: "https://github.com/example/app/pull/7".into(),
                reason: "checks green; merge approval needed".into(),
            }]
        );
    }

    #[test]
    fn what_a_worker_runs_on_is_read_from_its_meta_record() {
        let home = std::env::temp_dir().join(format!("vh-bearings-runs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("state")).unwrap();
        let meta = |id: &str, text: &str| {
            std::fs::write(home.join(format!("state/{id}.meta")), text).unwrap()
        };
        meta(
            "hack",
            "harness=pi\nmodel=opencode-go/deepseek-v4.1-flash\neffort=default\n",
        );
        meta("plain", "harness=claude\nmodel=default\n");
        assert_eq!(
            runs(&home, "hack").as_deref(),
            Some("pi \u{b7} opencode-go/deepseek-v4.1-flash")
        );
        assert_eq!(runs(&home, "plain").as_deref(), Some("claude"));
        assert_eq!(runs(&home, "gone"), None);
        assert_eq!(runs(&home, "../state/hack"), None);
    }

    #[test]
    fn second_mates_their_sessions_and_escalations_are_read() {
        let bearings = Bearings::parse(MATES.as_bytes()).unwrap();
        assert_eq!(bearings.mates.len(), 5);
        let web = bearings.mate("web").unwrap();
        assert_eq!(web.state, "unknown");
        assert_eq!(web.reason, "invalid home: not a seeded secondmate home");
        assert!(bearings.endpoint("docs").is_some_and(|e| e.exists));
        assert!(bearings.endpoint("infra").is_some_and(|e| !e.exists));
        assert_eq!(bearings.holds[0].owner.as_deref(), Some("support"));
        assert_eq!(bearings.holds[0].key, "refund-window");
        assert_eq!(bearings.crew[0].id, "billing/invoice-retries");
    }

    #[test]
    fn another_schema_is_refused_rather_than_guessed_at() {
        let problem = Bearings::parse(br#"{"schema":"fm-bearings.v2"}"#).unwrap_err();
        assert!(problem.contains("fm-bearings.v2"), "{problem}");
        assert!(Bearings::parse(b"not json").is_err());
    }

    #[test]
    fn every_section_of_the_fleet_is_read() {
        let fleet = Bearings::parse(FLEET.as_bytes()).unwrap();
        assert_eq!(fleet.holds.len(), 1);
        assert_eq!(fleet.holds[0].key, "lab-app-bundle");
        assert_eq!(fleet.merges.len(), 1);
        assert_eq!(fleet.crew[1].state, "blocked");
        assert_eq!(
            fleet.landed,
            [
                Landed {
                    id: "lab-stop-button".into(),
                    what: "A stop button for a running turn".into(),
                    artifact: Some("https://github.com/example/velvet-hammer/pull/31".into()),
                    owner: None,
                },
                Landed {
                    id: "lab-daily-review".into(),
                    what: "Scout: review the app for day-to-day use".into(),
                    artifact: Some("data/lab-daily-review/report.md".into()),
                    owner: None,
                },
            ]
        );
        assert_eq!(
            fleet.gates[0],
            Gate {
                id: "lab-multiline".into(),
                title: "Multi-line message field".into(),
                reason: String::new(),
                blocked_by: "lab-fleet-panel".into(),
                owner: None,
            }
        );
        assert_eq!(fleet.gates[1].blocked_by, "");
    }

    #[test]
    fn an_empty_fleet_has_nothing_in_any_section() {
        let fleet = Bearings::parse(EMPTY.as_bytes()).unwrap();
        assert!(fleet.read && fleet.is_empty());
        assert!(fleet.crew.is_empty() && fleet.landed.is_empty() && fleet.gates.is_empty());
    }

    /// A scratch clone whose snapshot script fails once with "jq is missing", then prints the
    /// two-hold fixture, and appends a line to `runs` each time it runs; and its home.
    #[cfg(unix)]
    fn lab(name: &str) -> (PathBuf, Place) {
        use std::os::unix::fs::PermissionsExt;
        let lab = std::env::temp_dir().join(format!("vh-bearings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&lab);
        let (clone, home) = (lab.join("clone"), lab.join("home"));
        std::fs::create_dir_all(clone.join("bin")).unwrap();
        crate::firstmate::prepare_home(&home).unwrap();
        std::fs::write(lab.join("good.json"), SNAPSHOT).unwrap();
        let script = clone.join("bin/fm-bearings-snapshot.sh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\necho run >> '{0}/runs'\nif [ ! -e '{0}/ran' ]; then touch '{0}/ran'; \
                 echo 'jq is missing' >&2; exit 1; fi\ncat '{0}/good.json'\n",
                lab.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        (
            lab,
            Place {
                code: clone,
                home,
                custom: false,
            },
        )
    }

    const QUICK: Timing = Timing {
        poll: Duration::from_millis(20),
        retry: Duration::from_millis(100),
        settle: Duration::from_millis(10),
        gap: Duration::from_millis(50),
    };

    /// The updates `watcher` reports until `done` holds of them.
    fn updates_until(watcher: &mut Watcher, done: impl Fn(&[Update]) -> bool) -> Vec<Update> {
        let mut got = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done(&got) {
            assert!(Instant::now() < deadline, "{got:?}");
            got.extend(watcher.poll());
            thread::sleep(Duration::from_millis(10));
        }
        got
    }

    fn reads(got: &[Update]) -> Vec<&Snapshot> {
        got.iter()
            .filter_map(|u| match u {
                Update::Read(read) => Some(read),
                _ => None,
            })
            .collect()
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_snapshot_is_retried_without_any_change_in_the_home() {
        let (lab, place) = lab("retry");
        let mut watcher = Watcher::start_with(&place, &lab.join(CACHE), QUICK);
        let got = updates_until(&mut watcher, |got| !reads(got).is_empty());
        drop(watcher);
        let _ = std::fs::remove_dir_all(&lab);
        assert_eq!(got[0], Update::Running, "{got:?}");
        assert_eq!(got[1], Update::Failed("jq is missing".to_string()));
        assert_eq!(got[2], Update::Running);
        let read = reads(&got)[0];
        assert_eq!(
            Ok(read.bearings.clone()),
            Bearings::parse(SNAPSHOT.as_bytes())
        );
        assert!(!read.kept);
    }

    #[cfg(unix)]
    #[test]
    fn the_last_snapshot_shows_at_launch_and_stands_while_a_run_fails() {
        let (lab, place) = lab("kept");
        let cache = lab.join(CACHE);
        std::fs::write(lab.join("ran"), "").unwrap();
        let mut first = Watcher::start_with(&place, &cache, QUICK);
        let got = updates_until(&mut first, |got| !reads(got).is_empty());
        let at = reads(&got)[0].at;
        drop(first);
        // The next launch: the kept snapshot arrives before any run, and stays the last one
        // read when the run fails.
        std::fs::remove_file(lab.join("ran")).unwrap();
        let mut next = Watcher::start_with(&place, &cache, QUICK);
        let got = updates_until(&mut next, |got| {
            got.iter().any(|u| matches!(u, Update::Failed(_)))
        });
        drop(next);
        // A snapshot kept for another home is not shown.
        let elsewhere = Watcher::start_with(
            &Place {
                home: lab.join("other"),
                ..place.clone()
            },
            &cache,
            QUICK,
        );
        thread::sleep(Duration::from_millis(100));
        let other: Vec<Update> = elsewhere.rx.try_iter().collect();
        let _ = std::fs::remove_dir_all(&lab);
        let Update::Read(kept) = &got[0] else {
            panic!("{got:?}")
        };
        assert!(kept.kept);
        assert_eq!(kept.at, at);
        assert_eq!(
            Ok(kept.bearings.clone()),
            Bearings::parse(SNAPSHOT.as_bytes())
        );
        assert_eq!(
            &got[1..],
            [Update::Running, Update::Failed("jq is missing".into())]
        );
        assert!(reads(&other).iter().all(|read| !read.kept), "{other:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_finished_turn_or_refresh_runs_the_snapshot_with_nothing_changed_on_disk() {
        let (lab, place) = lab("asked");
        std::fs::write(lab.join("ran"), "").unwrap();
        // Looking for changes on disk only once a minute, so only the asks start runs.
        let timing = Timing {
            poll: Duration::from_secs(60),
            ..QUICK
        };
        let mut watcher = Watcher::start_with(&place, &lab.join(CACHE), timing);
        updates_until(&mut watcher, |got| reads(got).len() == 1);
        watcher.invalidate();
        updates_until(&mut watcher, |got| reads(got).len() == 1);
        watcher.refresh();
        updates_until(&mut watcher, |got| reads(got).len() == 1);
        // Three asks in a row share one run.
        watcher.invalidate();
        watcher.invalidate();
        watcher.invalidate();
        updates_until(&mut watcher, |got| reads(got).len() == 1);
        thread::sleep(Duration::from_millis(200));
        let runs = std::fs::read_to_string(lab.join("runs")).unwrap();
        drop(watcher);
        let _ = std::fs::remove_dir_all(&lab);
        assert_eq!(runs.lines().count(), 4);
    }

    #[cfg(unix)]
    #[test]
    fn a_refresh_asked_before_the_script_is_installed_runs_once_it_appears() {
        let (lab, place) = lab("late");
        let script = place.code.join("bin/fm-bearings-snapshot.sh");
        let installed = std::fs::read(&script).unwrap();
        std::fs::remove_file(&script).unwrap();
        std::fs::write(lab.join("ran"), "").unwrap();
        let mut watcher = Watcher::start_with(&place, &lab.join(CACHE), QUICK);
        watcher.refresh();
        thread::sleep(Duration::from_millis(200));
        assert!(watcher.poll().is_empty());
        assert!(!lab.join("runs").exists());
        std::fs::write(&script, installed).unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        updates_until(&mut watcher, |got| reads(got).len() == 1);
        thread::sleep(Duration::from_millis(200));
        let runs = std::fs::read_to_string(lab.join("runs")).unwrap();
        drop(watcher);
        let _ = std::fs::remove_dir_all(&lab);
        assert_eq!(runs.lines().count(), 1);
    }

    #[test]
    fn an_empty_home_has_nothing_for_the_captain() {
        let bearings = Bearings::parse(br#"{"schema":"fm-bearings.v1"}"#).unwrap();
        assert!(bearings.is_empty());
    }
}
