//! What gets the captain's attention while they work in another app: a macOS notification and a
//! count on the Dock icon for each thing that waits on them, and how long a tool-use question
//! waits for their answer.
//!
//! Four kinds of thing wait on the captain. A tool approval comes from the chief of staff's hook;
//! a merge ready for the captain, a decision held for them and a worker that is blocked or failed
//! come from firstmate's records ([`crate::bearings`]). The first three post a notification and
//! count on the badge; a blocked or failed worker only counts. A finished turn is not one of them.
//! Each kind can be turned off, which takes it off the badge and stops its notifications.
//!
//! [`Attention`] keeps which waiting things it has already notified about, by a key that stays
//! the same while the thing waits, so each one notifies once, even across the many redraws and
//! snapshot runs while it waits, and its notification is taken back when it is answered or gone.
//! A thing that arrives while the window is in front is shown there and not notified.

use std::time::Duration;

use serde_json::{Value, json};

use crate::bearings::Bearings;
use crate::model::Workspace;
use crate::store::{self, Saved};

/// The settings file in the data directory.
pub const SETTINGS_FILE: &str = "attention.json";

/// Every notification the app posts for a waiting thing has this before the thing's key, so a
/// click can be told from a reminder's.
pub const NOTIFICATION_PREFIX: &str = "attention-";

/// The longest a question may be set to wait. Claude Code is told to wait a little longer for
/// the hook (see [`crate::bridge`]), so the app always answers first.
pub const LONGEST_WAIT: Duration = Duration::from_secs(24 * 60 * 60);

/// What can wait on the captain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A tool use the chief of staff's hook asks the captain about, merges, pushes and project
    /// writes among them.
    Approval,
    /// A pull request firstmate reports green and waiting for the captain to approve the merge.
    Merge,
    /// A task held for the captain's decision.
    Decision,
    /// A worker whose state is blocked or failed.
    Worker,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Approval, Kind::Merge, Kind::Decision, Kind::Worker];

    pub fn key(self) -> &'static str {
        match self {
            Kind::Approval => "approval",
            Kind::Merge => "merge",
            Kind::Decision => "decision",
            Kind::Worker => "worker",
        }
    }

    fn from_key(key: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Approval => "A tool use waits for your approval",
            Kind::Merge => "A merge is ready for your approval",
            Kind::Decision => "A decision is held for you",
            Kind::Worker => "A worker is blocked or failed",
        }
    }

    /// Whether it posts a notification as well as counting on the badge.
    pub fn notifies(self) -> bool {
        self != Kind::Worker
    }
}

/// The captain's choices: which kinds get their attention, and how long a question waits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// The kinds turned off, in [`Kind::ALL`] order.
    pub off: Vec<Kind>,
    /// How long the chief of staff's hook waits for an answer before it denies the tool use.
    pub wait: Duration,
}

impl Default for Settings {
    /// The recommended default: every kind on, and questions wait eight hours.
    fn default() -> Self {
        Settings {
            off: Vec::new(),
            wait: Wait::RECOMMENDED.duration(),
        }
    }
}

impl Settings {
    pub fn on(&self, kind: Kind) -> bool {
        !self.off.contains(&kind)
    }

    pub fn set(&mut self, kind: Kind, on: bool) {
        self.off.retain(|k| *k != kind);
        if !on {
            self.off.push(kind);
            self.off
                .sort_by_key(|k| Kind::ALL.iter().position(|a| a == k));
        }
    }

    /// The settings in a few words, for their card on the Settings page.
    pub fn summary(&self) -> String {
        let on = Kind::ALL.iter().filter(|k| self.on(**k)).count();
        format!(
            "{on} of {} kinds on \u{b7} questions wait {}",
            Kind::ALL.len(),
            words(self.wait)
        )
    }
}

/// The waits the settings page offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wait {
    TenMinutes,
    OneHour,
    EightHours,
    Day,
}

impl Wait {
    pub const ALL: [Wait; 4] = [Wait::TenMinutes, Wait::OneHour, Wait::EightHours, Wait::Day];
    pub const RECOMMENDED: Wait = Wait::EightHours;

    pub fn duration(self) -> Duration {
        Duration::from_secs(match self {
            Wait::TenMinutes => 10 * 60,
            Wait::OneHour => 60 * 60,
            Wait::EightHours => 8 * 60 * 60,
            Wait::Day => 24 * 60 * 60,
        })
    }

    pub fn key(self) -> &'static str {
        match self {
            Wait::TenMinutes => "10m",
            Wait::OneHour => "1h",
            Wait::EightHours => "8h",
            Wait::Day => "24h",
        }
    }

    pub fn explanation(self) -> &'static str {
        match self {
            Wait::TenMinutes => {
                "As before: a question you miss for ten minutes is denied, and your chief of \
                 staff carries on without it."
            }
            Wait::OneHour => "Long enough for a meeting.",
            Wait::EightHours => {
                "Long enough to step away for a working day: a merge, push or project write \
                 waits for you, and your chief of staff waits with it."
            }
            Wait::Day => "The longest wait: a question asked in the evening waits until the next.",
        }
    }
}

/// A wait in words, such as "8 hours" or "10 minutes".
pub fn words(wait: Duration) -> String {
    let seconds = wait.as_secs().max(1);
    let minutes = seconds.div_ceil(60);
    let (n, unit) = if seconds < 60 {
        (seconds, "second")
    } else if minutes.is_multiple_of(60) {
        (minutes / 60, "hour")
    } else {
        (minutes, "minute")
    };
    format!("{n} {unit}{}", if n == 1 { "" } else { "s" })
}

impl Saved for Settings {
    const FILE: &'static str = SETTINGS_FILE;
    const FALLBACK: &'static str = "every kind gets your attention and questions wait 8 hours";

    fn fallback() -> Self {
        Settings::default()
    }

    fn to_json(&self) -> Value {
        let off: Vec<&str> = self.off.iter().map(|k| k.key()).collect();
        json!({"version": store::VERSION, "off": off, "wait_seconds": self.wait.as_secs()})
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        store::check_version(value)?;
        let mut off = Vec::new();
        for kind in value["off"]
            .as_array()
            .ok_or("it has no list of kinds turned off")?
        {
            let kind = kind
                .as_str()
                .and_then(Kind::from_key)
                .ok_or("it turns off a kind Velvet Hammer does not know")?;
            if !off.contains(&kind) {
                off.push(kind);
            }
        }
        let wait = value["wait_seconds"]
            .as_u64()
            .map(Duration::from_secs)
            .filter(|wait| !wait.is_zero() && *wait <= LONGEST_WAIT)
            .ok_or("its question wait is not between one second and 24 hours")?;
        let mut settings = Settings {
            off: Vec::new(),
            wait,
        };
        for kind in off {
            settings.set(kind, false);
        }
        Ok(settings)
    }
}

/// One thing that waits on the captain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Waiting {
    /// Stays the same while the thing waits, and names its notification.
    pub key: String,
    pub kind: Kind,
    /// The notification's title and body.
    pub title: String,
    pub body: String,
    /// The key of the card that shows it in the window, which a click on its notification
    /// reveals.
    pub card: String,
}

/// Everything that waits on the captain now: the hook's questions, then firstmate's merges,
/// held decisions and workers in trouble, each with the card the window shows it on.
pub fn waiting(ws: &Workspace, captain: &Bearings) -> Vec<Waiting> {
    let mut all = Vec::new();
    for item in ws.needs_input() {
        all.push(Waiting {
            key: format!("approval-{}", item.id),
            kind: Kind::Approval,
            title: item.title.clone(),
            body: first_line(&item.detail),
            card: format!("input-{}", item.id),
        });
    }
    for merge in &captain.merges {
        all.push(Waiting {
            key: format!("merge-{}", merge.task),
            kind: Kind::Merge,
            title: format!("Approve the merge for {}?", merge.task),
            body: merge.url.clone(),
            card: format!("merge-{}", merge.task),
        });
    }
    for hold in &captain.holds {
        let (key, card, title) = match &hold.owner {
            Some(owner) => (
                format!("decision-{owner}/{}", hold.key),
                format!("hold-{owner}-{}", hold.key),
                format!("{} needs your decision (second mate {owner})", hold.key),
            ),
            None => (
                format!("decision-{}", hold.key),
                format!("hold-{}", hold.key),
                format!("{} needs your decision", hold.key),
            ),
        };
        all.push(Waiting {
            key,
            kind: Kind::Decision,
            title,
            body: first_line(&hold.question),
            card,
        });
    }
    for crew in &captain.crew {
        if matches!(crew.state.as_str(), "blocked" | "failed") {
            all.push(Waiting {
                // A worker blocked again after it worked on is a new thing to look at.
                key: format!("worker-{}-{}", crew.id, crew.state),
                kind: Kind::Worker,
                title: format!("{} is {}", crew.name, crew.state),
                body: crew.doing.clone(),
                card: format!("crew-{}", crew.id),
            });
        }
    }
    all
}

/// The first line of `text`, which is what a notification has room for.
fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .to_string()
}

/// What changed for the system's notifications and the Dock badge.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Update {
    /// Notifications to post.
    pub post: Vec<Waiting>,
    /// The keys whose notifications to take back.
    pub take_back: Vec<String>,
    /// The badge's new count, when it changed.
    pub badge: Option<u32>,
}

/// Which waiting things the captain has been told about, and the badge as last shown.
#[derive(Debug, Default)]
pub struct Attention {
    /// The keys notified, or seen in the window when they arrived, while they wait.
    told: Vec<String>,
    badge: u32,
}

impl Attention {
    /// Compares what waits now with what the captain was told: each new thing of a kind that is
    /// on notifies once, unless the window is `in_front`, and what is answered, gone or turned
    /// off is taken back.
    pub fn update(&mut self, waiting: &[Waiting], settings: &Settings, in_front: bool) -> Update {
        let wanted: Vec<&Waiting> = waiting.iter().filter(|w| settings.on(w.kind)).collect();
        let mut update = Update::default();
        self.told.retain(|key| {
            let still = wanted.iter().any(|w| &w.key == key);
            if !still {
                update.take_back.push(key.clone());
            }
            still
        });
        for w in &wanted {
            if w.kind.notifies() && !self.told.contains(&w.key) {
                self.told.push(w.key.clone());
                if !in_front {
                    update.post.push((*w).clone());
                }
            }
        }
        let badge = wanted.len() as u32;
        if badge != self.badge {
            self.badge = badge;
            update.badge = Some(badge);
        }
        update
    }

    /// The count on the Dock icon.
    pub fn badge(&self) -> u32 {
        self.badge
    }
}

/// Posts and takes back the notifications and sets the Dock badge, where the platform lets the
/// app: notifications only from a bundled app the captain allowed.
fn show(update: &Update) {
    if rune_notify::available() {
        for w in &update.post {
            rune_notify::post(
                &rune_notify::Notification::new(
                    format!("{NOTIFICATION_PREFIX}{}", w.key),
                    w.title.clone(),
                )
                .body(w.body.clone()),
            );
        }
    }
    let gone: Vec<String> = update
        .take_back
        .iter()
        .map(|key| format!("{NOTIFICATION_PREFIX}{key}"))
        .collect();
    if !gone.is_empty() {
        rune_notify::remove(&gone.iter().map(String::as_str).collect::<Vec<_>>());
    }
    if let Some(count) = update.badge {
        dock_badge(count);
    }
}

/// Shows `count` on the app's Dock icon, or no badge for 0. Set on the app rather than through
/// Rune's per-window taskbar state, since the count belongs to the app and stays while its window
/// is closed. Call it on the UI thread; does nothing off macOS.
fn dock_badge(count: u32) {
    #[cfg(target_os = "macos")]
    if let Some(mtm) = objc2_foundation::MainThreadMarker::new() {
        let label = (count > 0).then(|| objc2_foundation::NSString::from_str(&count.to_string()));
        // SAFETY: AppKit's Dock tile, on the main thread; nil clears the badge.
        unsafe {
            objc2_app_kit::NSApplication::sharedApplication(mtm)
                .dockTile()
                .setBadgeLabel(label.as_deref())
        };
    }
    #[cfg(not(target_os = "macos"))]
    let _ = count;
}

/// The waiting thing a clicked notification is about, by its key.
pub fn clicked(id: &str) -> Option<&str> {
    id.strip_prefix(NOTIFICATION_PREFIX)
}

/// How long a card a notification click revealed stays highlighted, in seconds.
const SPOTLIGHT: f64 = 4.0;

impl crate::App {
    /// Tells the captain about what waits on them, through notifications, the Dock badge and
    /// the menu-bar item, after each change the app took in.
    pub(crate) fn attend(&mut self) {
        let waiting = waiting(&self.ws, &self.captain);
        // Headless scenes and tests reach no system: they keep the count and what was told.
        let desktop = crate::desktop();
        let in_front = desktop && !waiting.is_empty() && rune_notify::app_focused();
        let update = self
            .attention
            .update(&waiting, self.attention_settings.settings(), in_front);
        if desktop {
            show(&update);
        }
        crate::menubar::show(&self.menu_state());
        crate::lifecycle::publish(&self.in_flight());
    }

    /// Shows the card of the waiting thing with this key, as a click on its notification asks:
    /// scrolled into view and highlighted for a few seconds. A thing already answered shows
    /// nothing.
    pub fn reveal(&mut self, key: &str, tx: &mut rune::Tx) {
        let Some(card) = waiting(&self.ws, &self.captain)
            .into_iter()
            .find(|w| w.key == key)
            .map(|w| w.card)
        else {
            return;
        };
        self.scroll_to = Some(card.clone());
        self.spotlight = Some((card, self.now + SPOTLIGHT));
        tx.wake_after(Duration::from_secs_f64(SPOTLIGHT));
    }

    /// Whether the card keyed `card` is highlighted now.
    pub fn spotlit(&self, card: &str) -> bool {
        self.spotlight
            .as_ref()
            .is_some_and(|(key, until)| key == card && self.now < *until)
    }

    /// Asks the chief of staff to try an expired question's tool use again, which asks the
    /// captain again; the expired card goes.
    pub fn ask_again(&mut self, item: crate::model::Id) {
        let Some(expired) = self.ws.item(item).cloned() else {
            return;
        };
        if !self.chat_ready() {
            return;
        }
        self.ws.items.retain(|i| i.id != item);
        let detail = shortened(&expired.detail, ASKED_AGAIN_MAX);
        self.say(
            format!(
                "Your question \u{201c}{}\u{201d} expired before I answered it, so that tool use \
                 did not run. Please try it again so I can answer this time. It was:\n\n{detail}",
                expired.title
            ),
            Vec::new(),
        );
    }

    /// Takes an expired question's card away without asking again.
    pub fn dismiss(&mut self, item: crate::model::Id) {
        self.ws.items.retain(|i| i.id != item);
    }
}

/// The most of an expired question's detail that asking again quotes.
const ASKED_AGAIN_MAX: usize = 2000;

fn shortened(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}\u{2026}", &text[..at]),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bearings::{Crew, Hold, Merge};
    use crate::model::{Item, ItemKind};

    fn question(id: crate::model::Id) -> Item {
        Item {
            id,
            kind: ItemKind::NeedsInput {
                options: vec!["Allow".into(), "Deny".into()],
            },
            title: "Allow Bash?".into(),
            detail: "by firstmate rule \u{201c}Merges\u{201d}\n\ncommand: bin/fm-pr-merge.sh"
                .into(),
            agent: None,
        }
    }

    fn fleet() -> Bearings {
        Bearings {
            read: true,
            holds: vec![Hold {
                key: "window-size".into(),
                question: "Window size\n1100x720 or 1280x800?".into(),
                owner: None,
            }],
            merges: vec![Merge {
                task: "site-docs".into(),
                url: "https://github.com/captain/site/pull/7".into(),
                reason: "checks green".into(),
            }],
            crew: vec![
                Crew {
                    id: "lab".into(),
                    name: "Lab window".into(),
                    state: "blocked".into(),
                    doing: "needs gh auth".into(),
                    runs: None,
                },
                Crew {
                    id: "docs".into(),
                    name: "Docs".into(),
                    state: "working".into(),
                    doing: "writing".into(),
                    runs: None,
                },
            ],
            ..Bearings::default()
        }
    }

    fn all_waiting() -> Vec<Waiting> {
        let ws = Workspace {
            items: vec![question(3)],
            ..Workspace::default()
        };
        waiting(&ws, &fleet())
    }

    #[test]
    fn each_kind_waits_on_its_own_card() {
        let all = all_waiting();
        let found: Vec<(Kind, &str, &str)> = all
            .iter()
            .map(|w| (w.kind, w.key.as_str(), w.card.as_str()))
            .collect();
        assert_eq!(
            found,
            [
                (Kind::Approval, "approval-3", "input-3"),
                (Kind::Merge, "merge-site-docs", "merge-site-docs"),
                (Kind::Decision, "decision-window-size", "hold-window-size"),
                (Kind::Worker, "worker-lab-blocked", "crew-lab"),
            ]
        );
        assert_eq!(all[0].body, "by firstmate rule \u{201c}Merges\u{201d}");
        assert_eq!(all[2].body, "Window size");
    }

    #[test]
    fn a_waiting_thing_notifies_once_and_is_taken_back_when_answered() {
        let settings = Settings::default();
        let mut attention = Attention::default();
        let all = all_waiting();
        let first = attention.update(&all, &settings, false);
        let posted: Vec<&str> = first.post.iter().map(|w| w.key.as_str()).collect();
        assert_eq!(
            posted,
            ["approval-3", "merge-site-docs", "decision-window-size"]
        );
        assert_eq!(
            first.badge,
            Some(4),
            "the blocked worker counts without notifying"
        );
        assert_eq!(attention.update(&all, &settings, false), Update::default());
        let answered: Vec<Waiting> = all
            .into_iter()
            .filter(|w| w.kind != Kind::Approval)
            .collect();
        let next = attention.update(&answered, &settings, false);
        assert_eq!(next.take_back, ["approval-3"]);
        assert!(next.post.is_empty());
        assert_eq!(next.badge, Some(3));
    }

    #[test]
    fn what_arrives_while_the_window_is_in_front_is_never_notified() {
        let settings = Settings::default();
        let mut attention = Attention::default();
        let all = all_waiting();
        let seen = attention.update(&all, &settings, true);
        assert!(seen.post.is_empty());
        assert_eq!(seen.badge, Some(4));
        assert!(attention.update(&all, &settings, false).post.is_empty());
    }

    #[test]
    fn a_kind_turned_off_leaves_the_badge_and_takes_its_notifications_back() {
        let mut settings = Settings::default();
        let mut attention = Attention::default();
        let all = all_waiting();
        attention.update(&all, &settings, false);
        settings.set(Kind::Merge, false);
        settings.set(Kind::Worker, false);
        let update = attention.update(&all, &settings, false);
        assert_eq!(update.take_back, ["merge-site-docs"]);
        assert_eq!(update.badge, Some(2));
        settings.set(Kind::Merge, true);
        let back = attention.update(&all, &settings, false);
        assert_eq!(
            back.post.len(),
            1,
            "a kind turned back on notifies what waits"
        );
        assert_eq!(attention.badge(), 3);
    }

    #[test]
    fn the_settings_read_back_as_written_and_refuse_what_they_do_not_know() {
        let mut settings = Settings::default();
        settings.set(Kind::Worker, false);
        settings.set(Kind::Approval, false);
        settings.wait = Wait::OneHour.duration();
        assert_eq!(settings.off, [Kind::Approval, Kind::Worker]);
        assert_eq!(Settings::from_json(&settings.to_json()), Ok(settings));
        for bad in [
            json!({"version": 1, "off": ["sound"], "wait_seconds": 60}),
            json!({"version": 1, "off": [], "wait_seconds": 0}),
            json!({"version": 1, "off": [], "wait_seconds": 90000}),
            json!({"version": 1, "wait_seconds": 60}),
        ] {
            assert!(Settings::from_json(&bad).is_err(), "{bad}");
        }
        assert_eq!(Settings::default().wait, Duration::from_secs(8 * 3600));
    }

    #[test]
    fn waits_read_in_words() {
        assert_eq!(words(Duration::from_secs(8 * 3600)), "8 hours");
        assert_eq!(words(Duration::from_secs(3600)), "1 hour");
        assert_eq!(words(Duration::from_secs(600)), "10 minutes");
        assert_eq!(words(Duration::from_secs(1)), "1 second");
        assert_eq!(words(Duration::from_secs(61)), "2 minutes");
    }
}
