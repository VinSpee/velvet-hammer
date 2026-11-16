//! The fleet panel on the right of the window: what waits on the captain, pinned at the top, then
//! firstmate's fleet by section, all from the bearings snapshot (see [`crate::bearings`]). The
//! panel costs no model use. The chief of staff is asked for its own digest, `/bearings`, or for a
//! catch-up, `/ahoy`, only when the captain presses a button, and for `/bearings` once more when
//! the app first opens on a new day, which also starts the session so firstmate supervises.

use std::path::{Path, PathBuf};

use jiff::Timestamp;
use jiff::civil::Date;

use crate::App;
use crate::bearings::Update;

/// What the Bearings button sends: firstmate's digest of the fleet.
pub const BEARINGS: &str = "/bearings";

/// What the Ahoy button sends: firstmate's catch-up on this session.
pub const AHOY: &str = "/ahoy";

/// A part of the panel the captain can open and close.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Underway,
    Landed,
    Charted,
}

impl Section {
    pub const ALL: [Section; 3] = [Section::Underway, Section::Landed, Section::Charted];

    pub fn key(self) -> &'static str {
        match self {
            Section::Underway => "underway",
            Section::Landed => "landed",
            Section::Charted => "charted",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Section::Underway => "Underway",
            Section::Landed => "Recently landed",
            Section::Charted => "Charted next",
        }
    }

    /// Charted next starts closed: it is the longest, and nothing in it is moving.
    fn open_at_first(self) -> bool {
        self != Section::Charted
    }
}

/// How the panel stands, beside the snapshot itself in [`App::captain`].
#[derive(Clone, Debug, Default)]
pub struct Fleet {
    /// When the snapshot shown was read; `None` until one is.
    pub at: Option<Timestamp>,
    /// The snapshot shown was kept from an earlier launch, and this launch has not read one yet.
    pub kept: bool,
    /// A run is under way; the snapshot shown stands until it finishes.
    pub refreshing: bool,
    /// The sections the captain opened or closed, against how each starts.
    toggled: Vec<Section>,
}

impl Fleet {
    pub fn is_open(&self, section: Section) -> bool {
        section.open_at_first() != self.toggled.contains(&section)
    }

    pub fn toggle(&mut self, section: Section) {
        match self.toggled.iter().position(|s| *s == section) {
            Some(i) => {
                self.toggled.remove(i);
            }
            None => self.toggled.push(section),
        }
    }
}

/// The day the app last sent `/bearings` by itself, kept in the app's data directory.
pub struct Daily {
    path: PathBuf,
    last: Option<Date>,
}

impl Daily {
    const FILE: &str = "bearings-day";

    pub fn open(data: &Path) -> Daily {
        let path = data.join(Daily::FILE);
        let last = std::fs::read_to_string(&path)
            .ok()
            .and_then(|day| day.trim().parse().ok());
        Daily { path, last }
    }

    /// Whether `today` has had its digest; if not, it counts as had from now. A day that cannot
    /// be written down still has its digest, and the next launch may send another.
    fn take(&mut self, today: Date) -> bool {
        if self.last == Some(today) {
            return false;
        }
        self.last = Some(today);
        let _ = std::fs::write(&self.path, format!("{today}\n"));
        true
    }
}

impl App {
    /// Takes in what the bearings watcher reported.
    pub(crate) fn take_fleet(&mut self, update: Update) {
        match update {
            Update::Running => self.fleet.refreshing = true,
            Update::Read(read) => {
                self.captain = read.bearings;
                self.fleet.at = Some(read.at);
                self.fleet.kept = read.kept;
                self.fleet.refreshing = false;
                self.captain_problem = None;
            }
            Update::Failed(problem) => {
                self.fleet.refreshing = false;
                self.captain_problem = Some(problem);
            }
        }
    }

    /// Whether the panel shows firstmate's fleet: the chief of staff is firstmate.
    pub fn has_fleet(&self) -> bool {
        self.watcher.is_some()
    }

    /// The captain pressed Refresh: run the snapshot now.
    pub fn refresh_fleet(&self) {
        if let Some(watcher) = &self.watcher {
            watcher.refresh();
        }
    }

    /// Something the snapshot reads may have changed, such as a finished turn or an answered
    /// card: run it again soon.
    pub(crate) fn invalidate_fleet(&self) {
        if let Some(watcher) = &self.watcher {
            watcher.invalidate();
        }
    }

    /// Sends `command`, [`BEARINGS`] or [`AHOY`], to the chief of staff like any message.
    pub fn ask(&mut self, command: &str) {
        if self.chat_ready() {
            self.say(command.to_string(), Vec::new());
        }
    }

    /// The first time the chat can run in a launch, sends `/bearings` if today has not had it;
    /// true when it did. Only the app the captain opens does this, never a scene or a test.
    pub(crate) fn first_of_the_day(&mut self) -> bool {
        if !self.chat_ready() || !self.has_fleet() {
            return false;
        }
        let Some(mut daily) = self.daily.take() else {
            return false;
        };
        let today = self.reminders.now().date();
        if !daily.take(today) {
            return false;
        }
        self.ask(BEARINGS);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn charted_next_starts_closed_and_every_section_toggles() {
        let mut fleet = Fleet::default();
        assert!(fleet.is_open(Section::Underway));
        assert!(fleet.is_open(Section::Landed));
        assert!(!fleet.is_open(Section::Charted));
        fleet.toggle(Section::Charted);
        fleet.toggle(Section::Underway);
        assert!(fleet.is_open(Section::Charted));
        assert!(!fleet.is_open(Section::Underway));
        fleet.toggle(Section::Underway);
        assert!(fleet.is_open(Section::Underway));
    }

    #[test]
    fn a_day_has_its_digest_once_across_launches() {
        let data = std::env::temp_dir().join(format!("vh-daily-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        std::fs::create_dir_all(&data).unwrap();
        let today: Date = "2026-11-16".parse().unwrap();
        assert!(Daily::open(&data).take(today));
        assert!(!Daily::open(&data).take(today), "a second launch that day");
        assert!(Daily::open(&data).take(today.tomorrow().unwrap()));
        let _ = std::fs::remove_dir_all(&data);
    }
}
