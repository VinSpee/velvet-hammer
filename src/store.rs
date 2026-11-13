//! A settings file in the app's data directory, shared with the chat session's hooks. Tool
//! approval ([`crate::approval`]), visual replies ([`crate::visual`]) and firstmate's rules
//! ([`crate::guard`]) each keep one.
//!
//! A file is written whole or not at all, and a change applies to a running session at once. A
//! file the app cannot read is never dropped quietly: the app runs on that setting's safe
//! fallback, the page says why, and the next change keeps the old file as `<file>.bad`.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use serde_json::Value;

/// Settings the hooks read at each call, so a change applies to a session that is running.
pub type Shared<T> = Arc<RwLock<T>>;

/// Where an unreadable settings file is kept when the captain saves over it.
const BAD_SUFFIX: &str = ".bad";

/// Settings kept in a file of their own.
pub trait Saved: Clone + Default + PartialEq {
    /// The file's name in the data directory.
    const FILE: &'static str;
    /// What the app runs on while the file cannot be read.
    fn fallback() -> Self;
    /// What [`Saved::fallback`] means for the captain, such as "every tool use asks you".
    const FALLBACK: &'static str;
    fn to_json(&self) -> Value;
    /// Reads the file's JSON. Anything it does not understand fails the whole file.
    fn from_json(value: &Value) -> Result<Self, String>;
}

/// One settings file.
pub struct Store<T: Saved> {
    path: PathBuf,
    settings: T,
    shared: Shared<T>,
    /// Why the file could not be read or saved, for the settings page.
    problem: Option<String>,
    /// The file exists but could not be read; the next save keeps it as `<file>.bad`.
    unreadable: bool,
}

impl<T: Saved> Store<T> {
    /// Reads `dir/<file>`. A missing file gives the default; one that cannot be read gives the
    /// fallback and a problem to show.
    pub fn open(dir: &Path) -> Store<T> {
        let path = dir.join(T::FILE);
        let read = match fs::read(&path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
            Ok(bytes) => serde_json::from_slice::<Value>(&bytes)
                .map_err(|_| "it is not valid JSON".to_string())
                .and_then(|value| T::from_json(&value))
                .map(Some),
        };
        let (settings, problem) = match read {
            Ok(settings) => (settings.unwrap_or_default(), None),
            Err(why) => (
                T::fallback(),
                Some(format!(
                    "Velvet Hammer could not read {} ({why}), so {} until you change a setting \
                     here. The old file is then kept as {}{BAD_SUFFIX}.",
                    path.display(),
                    T::FALLBACK,
                    T::FILE
                )),
            ),
        };
        Store {
            path,
            shared: Arc::new(RwLock::new(settings.clone())),
            unreadable: problem.is_some(),
            settings,
            problem,
        }
    }

    pub fn settings(&self) -> &T {
        &self.settings
    }

    /// The settings as the hooks read them.
    pub fn shared(&self) -> Shared<T> {
        self.shared.clone()
    }

    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Changes the settings and saves them. If the save fails, nothing changes and the page says
    /// why, so the app never runs on settings it could not keep.
    pub fn change(&mut self, edit: impl FnOnce(&mut T)) {
        let mut next = self.settings.clone();
        edit(&mut next);
        match self.save(&next) {
            Ok(()) => {
                *self.shared.write().unwrap_or_else(|e| e.into_inner()) = next.clone();
                self.settings = next;
                self.problem = None;
                self.unreadable = false;
            }
            Err(e) => {
                self.problem = Some(format!(
                    "Velvet Hammer could not save {}: {e}. The change was not made.",
                    self.path.display()
                ))
            }
        }
    }

    /// Writes the file whole or not at all: a temporary file beside it, synced, then renamed
    /// over it.
    fn save(&self, settings: &T) -> io::Result<()> {
        let dir = self.path.parent().expect("the file is in a directory");
        fs::create_dir_all(dir)?;
        let temporary = suffixed(&self.path, &format!(".tmp-{}", std::process::id()));
        let mut file = File::create(&temporary)?;
        let written = serde_json::to_vec_pretty(&settings.to_json())
            .map_err(io::Error::other)
            .and_then(|bytes| file.write_all(&bytes))
            .and_then(|()| file.sync_all())
            .and_then(|()| {
                if self.unreadable && self.path.exists() {
                    fs::copy(&self.path, suffixed(&self.path, BAD_SUFFIX))?;
                }
                fs::rename(&temporary, &self.path)
            });
        if written.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        written?;
        // Makes the rename itself durable.
        if let Ok(dir) = File::open(dir) {
            let _ = dir.sync_all();
        }
        Ok(())
    }
}

pub fn suffixed(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// The schema version every settings file carries; a file with another version is not read.
pub const VERSION: u64 = 1;

/// Checks a settings file's schema version.
pub fn check_version(value: &Value) -> Result<(), String> {
    match value["version"].as_u64() {
        Some(VERSION) => Ok(()),
        Some(other) => Err(format!("it has schema version {other}, not {VERSION}")),
        None => Err("it has no schema version".into()),
    }
}
