//! The projects the chief of staff works on, read from firstmate's own records. Firstmate keeps
//! its projects as clones under the home's `projects/` directory and lists them in its registry,
//! `data/projects.md`, whose line format the header of its `bin/fm-project-mode.sh` owns:
//!
//! ```text
//! - <name> [<posture tokens>] - <description> (added <date>)
//! ```
//!
//! The bracket is optional, and the name ends at the first ` [` or ` - `. Firstmate rebuilds a
//! missing registry from the clones, so a clone the registry does not list is a project too.
//! Firstmate publishes no document of its projects, so the app reads these two records, never
//! writes them, and leaves adding a project to the chief of staff, which follows firstmate's
//! `project-management` procedure.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::Duration;

use rune::Waker;

/// How often the watcher reads the records again.
const POLL: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub name: String,
    /// The registry's delivery posture, such as `no-mistakes-prod-only` or `local-only +yolo`;
    /// empty for a legacy entry with no bracket, and `None` for a clone the registry does not
    /// list.
    pub posture: Option<String>,
    /// The registry's description, without the date it was added.
    pub about: String,
}

impl Project {
    /// One line under the project's name.
    pub fn summary(&self) -> String {
        match &self.posture {
            None => "a clone the registry does not list yet".into(),
            // Firstmate reads a legacy entry with no bracket as no-mistakes.
            Some(posture) if posture.is_empty() => "no-mistakes".into(),
            Some(posture) => posture.clone(),
        }
    }
}

/// The projects in firstmate home `home`: its registry entries in order, then any clone under
/// `projects/` the registry does not list. A home or registry that does not exist yet has none.
pub fn read(home: &Path) -> Result<Vec<Project>, String> {
    let registry = home.join("data/projects.md");
    let mut projects = match std::fs::read_to_string(&registry) {
        Ok(text) => text.lines().filter_map(entry).collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(format!("Could not read {}: {e}", registry.display())),
    };
    let clones = home.join("projects");
    let mut unlisted: Vec<String> = match std::fs::read_dir(&clones) {
        Ok(entries) => entries
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| !name.starts_with('.'))
            .filter(|name| !projects.iter().any(|p: &Project| &p.name == name))
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(format!("Could not read {}: {e}", clones.display())),
    };
    unlisted.sort();
    projects.extend(unlisted.into_iter().map(|name| Project {
        name,
        posture: None,
        about: String::new(),
    }));
    Ok(projects)
}

/// One registry line, or `None` for a line that is not an entry.
fn entry(line: &str) -> Option<Project> {
    let rest = line.strip_prefix("- ")?;
    let end = [rest.find(" ["), rest.find(" - ")]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(rest.len());
    let name = rest[..end].trim().to_string();
    if name.is_empty() {
        return None;
    }
    let mut after = &rest[end..];
    let mut posture = String::new();
    if let Some(bracket) = after.strip_prefix(" [")
        && let Some(close) = bracket.find(']')
    {
        posture = bracket[..close].trim().to_string();
        after = &bracket[close + 1..];
    }
    let about = after.strip_prefix(" - ").unwrap_or("").trim();
    let about = match about.rfind(" (added ") {
        Some(at) if about.ends_with(')') => &about[..at],
        _ => about,
    };
    Some(Project {
        name,
        posture: Some(posture),
        about: about.to_string(),
    })
}

/// Keeps the projects of one firstmate home current until dropped.
pub struct Watcher {
    rx: Receiver<Result<Vec<Project>, String>>,
    wake: Arc<OnceLock<Waker>>,
    stop: Arc<AtomicBool>,
}

impl Watcher {
    /// Reads `home`'s projects now and every [`POLL`] after, reporting each change.
    pub fn start(home: &Path) -> Watcher {
        let (tx, rx) = channel();
        let wake: Arc<OnceLock<Waker>> = Arc::new(OnceLock::new());
        let stop = Arc::new(AtomicBool::new(false));
        let home = home.to_path_buf();
        let (waker, stopped) = (wake.clone(), stop.clone());
        thread::spawn(move || {
            let mut shown = None;
            while !stopped.load(Ordering::Relaxed) {
                let projects = read(&home);
                if shown.as_ref() != Some(&projects) {
                    shown = Some(projects.clone());
                    if tx.send(projects).is_err() {
                        return;
                    }
                    if let Some(wake) = waker.get() {
                        wake();
                    }
                }
                thread::sleep(POLL);
            }
        });
        Watcher { rx, wake, stop }
    }

    pub fn connect(&mut self, wake: Waker) {
        let _ = self.wake.set(wake);
        // A read may have arrived before the waker did.
        if let Some(wake) = self.wake.get() {
            wake();
        }
    }

    /// The newest read since the last call, if one arrived.
    pub fn poll(&mut self) -> Option<Result<Vec<Project>, String>> {
        self.rx.try_iter().last()
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Asks the captain for a folder with the system's own folder dialog: `osascript` on macOS,
/// `zenity` elsewhere. `None` when they cancel it.
pub fn pick_folder() -> Result<Option<PathBuf>, String> {
    let (program, args): (&str, &[&str]) = if cfg!(target_os = "macos") {
        (
            "osascript",
            &[
                "-e",
                "POSIX path of (choose folder with prompt \"Choose the project's folder\")",
            ],
        )
    } else {
        ("zenity", &["--file-selection", "--directory"])
    };
    let output = Command::new(program).args(args).output().map_err(|e| {
        format!("Could not open a folder dialog ({program}: {e}). Type the folder's path instead.")
    })?;
    let chosen = String::from_utf8_lossy(&output.stdout).trim().to_string();
    // Both dialogs exit 1 with nothing printed when the captain cancels.
    if !output.status.success() || chosen.is_empty() {
        return Ok(None);
    }
    Ok(Some(PathBuf::from(
        chosen.strip_suffix('/').unwrap_or(&chosen),
    )))
}

/// A registry in the shapes `bin/fm-project-mode.sh`'s header documents, for tests and scenes.
pub const SAMPLE_REGISTRY: &str = "# Projects\n\n\
    - velvet-hammer [no-mistakes-prod-only] - The desktop app (added 2026-11-13)\n\
    - notes - Meeting notes (added 2026-10-01)\n\
    - site [direct-PR +yolo branch=web/] - The marketing site (added 2026-11-02)\n";

#[cfg(test)]
mod tests {
    use super::*;

    fn lab(name: &str) -> PathBuf {
        let home = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/projects-lab")
            .join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        home
    }

    #[test]
    fn a_home_that_does_not_exist_yet_has_no_projects() {
        assert_eq!(read(&lab("missing")), Ok(vec![]));
    }

    #[test]
    fn registry_entries_are_read_in_the_format_firstmate_owns() {
        let home = lab("registry");
        std::fs::create_dir_all(home.join("data")).unwrap();
        std::fs::write(home.join("data/projects.md"), SAMPLE_REGISTRY).unwrap();
        let projects = read(&home).unwrap();
        let names: Vec<_> = projects.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["velvet-hammer", "notes", "site"]);
        assert_eq!(
            projects[0],
            Project {
                name: "velvet-hammer".into(),
                posture: Some("no-mistakes-prod-only".into()),
                about: "The desktop app".into(),
            }
        );
        assert_eq!(projects[1].summary(), "no-mistakes");
        assert_eq!(projects[2].summary(), "direct-PR +yolo branch=web/");
    }

    #[test]
    fn a_name_with_spaces_ends_at_its_bracket_or_dash() {
        let p = entry("- My App [local-only] - Notes - and more (added 2026-01-01)").unwrap();
        assert_eq!(p.name, "My App");
        assert_eq!(p.about, "Notes - and more");
        assert_eq!(entry("- bare").unwrap().name, "bare");
        assert_eq!(entry("Some prose."), None);
        assert_eq!(entry("- "), None);
    }

    #[test]
    fn a_clone_the_registry_does_not_list_is_a_project_too() {
        let home = lab("clones");
        std::fs::create_dir_all(home.join("projects/notes")).unwrap();
        std::fs::create_dir_all(home.join("projects/zeta")).unwrap();
        std::fs::create_dir_all(home.join("projects/.cache")).unwrap();
        std::fs::create_dir_all(home.join("data")).unwrap();
        std::fs::write(
            home.join("data/projects.md"),
            "- notes - Notes (added 2026-10-01)\n",
        )
        .unwrap();
        let projects = read(&home).unwrap();
        let names: Vec<_> = projects.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["notes", "zeta"]);
        assert_eq!(projects[1].posture, None);
    }
}
