//! The menu-bar item that keeps Velvet Hammer reachable while its window is closed (see
//! [`crate::lifecycle`]). It shows a hammer, with the count of what waits on the captain beside
//! it, and its menu says how the chief of staff stands, what waits and how many workers are in
//! flight, then offers Open Velvet Hammer and Quit Velvet Hammer. macOS only: Rune's tray covers
//! Windows and Linux but not the Mac, so the item is AppKit's own `NSStatusItem`
//! (`menubar/mac.rs`).

#[cfg(target_os = "macos")]
mod mac;

/// What the item shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    /// The count beside the hammer, or empty.
    pub title: String,
    /// The menu's lines over its two commands, which are not commands themselves.
    pub lines: Vec<String>,
}

impl State {
    /// The item for the chief of staff's state in words, the number of things that wait on the
    /// captain, and the number of workers in flight (None while the chief of staff is not
    /// firstmate, which has no workers).
    pub fn new(chief: &str, waiting: u32, workers: Option<usize>) -> State {
        let mut lines = vec![format!("Chief of staff: {chief}")];
        lines.push(match waiting {
            0 => "Nothing waits on you".into(),
            1 => "1 thing waits on you".into(),
            n => format!("{n} things wait on you"),
        });
        if let Some(workers) = workers {
            lines.push(match workers {
                0 => "No workers in flight".into(),
                1 => "1 worker in flight".into(),
                n => format!("{n} workers in flight"),
            });
        }
        State {
            title: if waiting == 0 {
                String::new()
            } else {
                waiting.to_string()
            },
            lines,
        }
    }
}

/// Shows `state` in the menu bar, adding the item the first time. Call it on the UI thread.
/// Does nothing off macOS, or for headless scenes and tests ([`crate::desktop`]).
pub fn show(state: &State) {
    #[cfg(target_os = "macos")]
    if crate::desktop() {
        mac::show(state);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = state;
}

/// What Open Velvet Hammer runs. Quit asks AppKit to quit, as ⌘Q does.
pub fn on_open(open: impl Fn() + Send + Sync + 'static) {
    #[cfg(target_os = "macos")]
    mac::on_open(open);
    #[cfg(not(target_os = "macos"))]
    let _ = open;
}

impl crate::App {
    /// What the menu-bar item shows for the app as it stands.
    pub fn menu_state(&self) -> State {
        use crate::model::Status;
        let chief = match &self.status {
            Status::Ready(_) if self.working => "working",
            Status::Ready(_) => "idle",
            Status::Checking => "starting",
            Status::SignedOut { .. } | Status::SigningIn { .. } => "not signed in",
            Status::Expired => "sign-in expired",
            Status::NotSubscription { .. } => "not on a subscription",
            Status::CliMissing => "Claude Code not found",
            Status::Failed(_) => "stopped with a problem",
        };
        let workers = self
            .chief
            .workplace()
            .map(|_| self.in_flight().workers.len());
        State::new(chief, self.attention.badge(), workers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_item_counts_what_waits_and_says_how_things_stand() {
        let busy = State::new("working", 3, Some(2));
        assert_eq!(busy.title, "3");
        assert_eq!(
            busy.lines,
            [
                "Chief of staff: working",
                "3 things wait on you",
                "2 workers in flight"
            ]
        );
        let quiet = State::new("idle", 0, None);
        assert_eq!(quiet.title, "");
        assert_eq!(
            quiet.lines,
            ["Chief of staff: idle", "Nothing waits on you"]
        );
    }
}
