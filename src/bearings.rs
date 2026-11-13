//! What waits on the captain, read from firstmate's own records. Firstmate publishes them as one
//! structured document, `bin/fm-bearings-snapshot.sh --json`, and asks every view to render that
//! instead of parsing its record files. The app runs that script from the managed clone against
//! the app's firstmate home, on a background thread, whenever the home's records change. It
//! reads the two things firstmate marks as the captain's: tasks held for the captain, and pull
//! requests whose checks are green and which wait for a merge approval. Nothing is written.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use rune::Waker;
use serde_json::Value;

/// How often the watcher looks at the home's records for a change.
const POLL: Duration = Duration::from_secs(2);

/// How long after a failed snapshot to run it again when nothing in the home has changed.
const RETRY: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bearings {
    pub holds: Vec<Hold>,
    pub merges: Vec<Merge>,
}

/// A task held for the captain: firstmate's one record of a captain call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hold {
    /// The task id, which is also the key an answer names.
    pub key: String,
    /// The task's title and the whole hold reason, as firstmate wrote them.
    pub question: String,
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
        Ok(Bearings {
            holds: rows(&doc["decisions_open"])
                .iter()
                .map(|row| Hold {
                    key: word(row, "key"),
                    question: word(row, "question"),
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
        })
    }

    pub fn is_empty(&self) -> bool {
        self.holds.is_empty() && self.merges.is_empty()
    }
}

/// Keeps a [`Bearings`] current for one firstmate home until dropped.
pub struct Watcher {
    rx: Receiver<Result<Bearings, String>>,
    wake: Arc<OnceLock<Waker>>,
    stop: Arc<AtomicBool>,
}

impl Watcher {
    /// Watches `home`'s records with the scripts of the firstmate clone at `clone`. It waits
    /// quietly while the clone is not installed.
    pub fn start(clone: &Path, home: &Path) -> Watcher {
        Watcher::start_with(clone, home, POLL, RETRY)
    }

    fn start_with(clone: &Path, home: &Path, poll: Duration, retry: Duration) -> Watcher {
        let (tx, rx) = channel();
        let wake: Arc<OnceLock<Waker>> = Arc::new(OnceLock::new());
        let stop = Arc::new(AtomicBool::new(false));
        let (clone, home) = (clone.to_path_buf(), home.to_path_buf());
        let (waker, stopped) = (wake.clone(), stop.clone());
        thread::spawn(move || {
            let mut seen = None;
            let mut failed_at: Option<Instant> = None;
            let mut last_problem: Option<String> = None;
            while !stopped.load(Ordering::Relaxed) {
                let script = clone.join("bin/fm-bearings-snapshot.sh");
                let stamp = script.exists().then(|| stamp(&home));
                let retry_due = failed_at.is_some_and(|at| at.elapsed() >= retry);
                if stamp.is_some() && (stamp != seen || retry_due) {
                    seen = stamp;
                    let result = snapshot(&script, &clone, &home);
                    failed_at = result.is_err().then(Instant::now);
                    let repeat = matches!(&result, Err(p) if last_problem.as_ref() == Some(p));
                    last_problem = result.as_ref().err().cloned();
                    if !repeat {
                        if tx.send(result).is_err() {
                            return;
                        }
                        if let Some(wake) = waker.get() {
                            wake();
                        }
                    }
                }
                thread::sleep(poll);
            }
        });
        Watcher { rx, wake, stop }
    }

    pub fn connect(&mut self, wake: Waker) {
        let _ = self.wake.set(wake);
        // A snapshot may have arrived before the waker did.
        if let Some(wake) = self.wake.get() {
            wake();
        }
    }

    /// The newest snapshot since the last call, if one arrived.
    pub fn poll(&mut self) -> Option<Result<Bearings, String>> {
        self.rx.try_iter().last()
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// When anything in the home's `state/` or `data/` last changed. Firstmate appends to status
/// logs and rewrites its backlog there, so a newer time means the snapshot may differ.
fn stamp(home: &Path) -> SystemTime {
    let mut newest = SystemTime::UNIX_EPOCH;
    for dir in [home.join("state"), home.join("data")] {
        let entries = std::fs::read_dir(&dir).into_iter().flatten().flatten();
        let paths = entries.map(|e| e.path()).chain([dir.clone()]);
        for modified in paths.filter_map(|p| p.metadata().ok()?.modified().ok()) {
            newest = newest.max(modified);
        }
    }
    newest
}

fn snapshot(script: &PathBuf, clone: &Path, home: &Path) -> Result<Bearings, String> {
    crate::firstmate::prepare_home(home)
        .map_err(|e| format!("Could not create {}: {e}", home.display()))?;
    let output = Command::new(script)
        .arg("--json")
        .current_dir(clone)
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
    Bearings::parse(&output.stdout)
}

/// What `bin/fm-bearings-snapshot.sh --json` printed for a lab home with two captain holds
/// (firstmate 427056c4). The two `contributions.captain` rows were added by hand in the shape
/// firstmate's `bin/fm-contributions.jq` writes: one merge approval, and one row that carries a
/// hold. Tests and headless scenes use it.
pub const SNAPSHOT: &str = include_str!("../scripts/fixtures/bearings.json");

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
                },
                Hold {
                    key: "lab-gh-login".into(),
                    question: "Sign in to GitHub: Run gh auth login in a terminal; agents \
                               cannot sign in for you."
                        .into(),
                },
            ]
        );
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
    fn another_schema_is_refused_rather_than_guessed_at() {
        let problem = Bearings::parse(br#"{"schema":"fm-bearings.v2"}"#).unwrap_err();
        assert!(problem.contains("fm-bearings.v2"), "{problem}");
        assert!(Bearings::parse(b"not json").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_snapshot_is_retried_without_any_change_in_the_home() {
        use std::os::unix::fs::PermissionsExt;
        let lab = std::env::temp_dir().join(format!("vh-bearings-retry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&lab);
        let (clone, home) = (lab.join("clone"), lab.join("home"));
        std::fs::create_dir_all(clone.join("bin")).unwrap();
        crate::firstmate::prepare_home(&home).unwrap();
        std::fs::write(lab.join("good.json"), SNAPSHOT).unwrap();
        let script = clone.join("bin/fm-bearings-snapshot.sh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nif [ ! -e '{0}/ran' ]; then touch '{0}/ran'; echo 'jq is missing' >&2; \
                 exit 1; fi\ncat '{0}/good.json'\n",
                lab.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let watcher = Watcher::start_with(
            &clone,
            &home,
            Duration::from_millis(20),
            Duration::from_millis(100),
        );
        let mut got = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while got.len() < 2 && Instant::now() < deadline {
            got.extend(watcher.rx.try_iter());
            thread::sleep(Duration::from_millis(20));
        }
        drop(watcher);
        let _ = std::fs::remove_dir_all(&lab);
        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!(got[0], Err("jq is missing".to_string()));
        assert_eq!(got[1], Bearings::parse(SNAPSHOT.as_bytes()));
    }

    #[test]
    fn an_empty_home_has_nothing_for_the_captain() {
        let bearings = Bearings::parse(br#"{"schema":"fm-bearings.v1"}"#).unwrap();
        assert!(bearings.is_empty());
    }
}
