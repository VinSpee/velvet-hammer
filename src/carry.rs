//! The captain's place, carried across a relaunch: the page showing, the draft and the files
//! attached to it, and the messages queued for the chief of staff. It is kept in `place.json` in
//! the data directory once it has stood for [`SETTLE`], and put back when the app opens, so a
//! new build (see `scripts/install-app.sh`) opens where the old one was. The chat opens at its
//! newest message, as it always does.

use std::path::Path;
use std::time::Duration;

use rune::{TextEdit, Tx};
use serde_json::{Value, json};

use crate::attach::Attachment;
use crate::store::{self, Saved, Store};
use crate::{App, Queued, Showing};

/// How long, in seconds, a change stands before it is written, so typing does not write the
/// file at every key.
pub const SETTLE: f64 = 1.0;

/// What is carried.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Place {
    /// The page showing, by [`Showing::key`].
    pub page: Option<String>,
    pub draft: String,
    pub attached: Vec<Attachment>,
    pub queue: Vec<Queued>,
}

impl Saved for Place {
    const FILE: &'static str = "place.json";
    const FALLBACK: &'static str = "the app opens on the chat with an empty draft";

    fn fallback() -> Self {
        Place::default()
    }

    fn to_json(&self) -> Value {
        let queue: Vec<Value> = self
            .queue
            .iter()
            .map(|q| json!({"text": q.text, "attachments": attachments_json(&q.attachments)}))
            .collect();
        json!({
            "version": store::VERSION,
            "page": self.page,
            "draft": self.draft,
            "attached": attachments_json(&self.attached),
            "queue": queue,
        })
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        store::check_version(value)?;
        let queue = value["queue"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|q| {
                Some(Queued {
                    text: q["text"].as_str()?.to_string(),
                    attachments: attachments_from(&q["attachments"]),
                })
            })
            .collect();
        Ok(Place {
            page: value["page"].as_str().map(str::to_string),
            draft: value["draft"].as_str().unwrap_or_default().to_string(),
            attached: attachments_from(&value["attached"]),
            queue,
        })
    }
}

fn attachments_json(attachments: &[Attachment]) -> Vec<Value> {
    attachments.iter().map(Attachment::to_json).collect()
}

/// The attachments kept, leaving out any whose copy is gone.
fn attachments_from(value: &Value) -> Vec<Attachment> {
    let all = value.as_array().into_iter().flatten();
    all.filter_map(Attachment::from_json)
        .filter(|a| a.path.exists())
        .collect()
}

/// The kept place, and since when the place has differed from it.
pub struct Carry {
    store: Store<Place>,
    changed_at: Option<f64>,
}

impl Carry {
    pub fn open(dir: &Path) -> Carry {
        Carry {
            store: Store::open(dir),
            changed_at: None,
        }
    }

    /// Keeps `place` once it has stood for [`SETTLE`] at `now`. While it waits, says how long is
    /// left.
    fn keep(&mut self, place: Place, now: f64) -> Option<f64> {
        if place == *self.store.settings() {
            self.changed_at = None;
            return None;
        }
        let since = *self.changed_at.get_or_insert(now);
        if now - since < SETTLE {
            return Some(SETTLE - (now - since));
        }
        self.store.change(|kept| *kept = place);
        self.changed_at = None;
        None
    }
}

impl Showing {
    /// The page by a name that lasts across launches. Pages about something that may be gone
    /// by then (a picture, a subagent, a worker, a second mate) have none, and the chat shows.
    fn key(&self) -> Option<&'static str> {
        Some(match self {
            Showing::Chat => "chat",
            Showing::Firstmate => "firstmate",
            Showing::Approvals => "approvals",
            Showing::Attention => "attention",
            Showing::Activity => "activity",
            Showing::Visuals => "visuals",
            Showing::Rules => "rules",
            Showing::Messages => "messages",
            Showing::Workers => "workers",
            Showing::Settings => "settings",
            Showing::Reminders => "reminders",
            Showing::ReminderSettings => "reminder-settings",
            Showing::Peek(_)
            | Showing::Crew(_)
            | Showing::Mate(_)
            | Showing::AddMate
            | Showing::Picture(_) => return None,
        })
    }

    fn from_key(key: &str) -> Option<Showing> {
        [
            Showing::Chat,
            Showing::Firstmate,
            Showing::Approvals,
            Showing::Attention,
            Showing::Activity,
            Showing::Visuals,
            Showing::Rules,
            Showing::Messages,
            Showing::Workers,
            Showing::Settings,
            Showing::Reminders,
            Showing::ReminderSettings,
        ]
        .into_iter()
        .find(|page| page.key() == Some(key))
    }
}

impl App {
    /// The captain's place as it stands.
    fn place(&self) -> Place {
        Place {
            page: self.showing.key().map(str::to_string),
            draft: self.draft.text.clone(),
            attached: self.attached.clone(),
            queue: self.queue.clone(),
        }
    }

    /// Puts the kept place back, when the app opens.
    pub fn restore_place(&mut self) {
        let Some(carry) = &self.carry else {
            return;
        };
        let place = carry.store.settings().clone();
        if let Some(page) = place.page.as_deref().and_then(Showing::from_key) {
            self.showing = page;
        }
        self.draft = TextEdit::new(&place.draft);
        self.attached = place.attached;
        self.queue = place.queue;
    }

    /// Keeps the captain's place as it changes. The window calls this from each poll.
    pub fn carry_place(&mut self, now: f64, tx: &mut Tx) {
        let place = self.place();
        if let Some(carry) = &mut self.carry
            && let Some(left) = carry.keep(place, now)
        {
            tx.wake_after(Duration::from_secs_f64(left));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attach::Kind;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("velvet-hammer-carry-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_change_is_kept_once_it_has_stood_and_read_back_whole() {
        let dir = scratch("keep");
        let file = dir.join("photo.png");
        std::fs::write(&file, b"png").unwrap();
        let photo = Attachment {
            path: file,
            name: "photo.png".into(),
            bytes: 3,
            kind: Kind::Image,
        };
        let place = Place {
            page: Some("rules".into()),
            draft: "half a thought".into(),
            attached: vec![photo.clone()],
            queue: vec![Queued {
                text: "after this turn".into(),
                attachments: vec![photo],
            }],
        };
        let mut carry = Carry::open(&dir);
        assert_eq!(carry.keep(place.clone(), 10.0), Some(SETTLE));
        assert!(
            !dir.join(Place::FILE).exists(),
            "not while it is still changing"
        );
        assert_eq!(carry.keep(place.clone(), 10.0 + SETTLE), None);
        assert_eq!(*Carry::open(&dir).store.settings(), place);
    }

    #[test]
    fn an_attachment_whose_copy_is_gone_is_left_out() {
        let dir = scratch("gone");
        let gone = Attachment {
            path: dir.join("deleted.png"),
            name: "deleted.png".into(),
            bytes: 3,
            kind: Kind::Image,
        };
        let kept = Place {
            attached: vec![gone],
            ..Place::default()
        }
        .to_json();
        assert!(Place::from_json(&kept).unwrap().attached.is_empty());
    }

    #[test]
    fn only_pages_that_last_are_carried() {
        assert_eq!(Showing::from_key("rules"), Some(Showing::Rules));
        assert_eq!(Showing::Picture("toolu_1".into()).key(), None);
        assert_eq!(Showing::from_key("nonsense"), None);
    }
}
