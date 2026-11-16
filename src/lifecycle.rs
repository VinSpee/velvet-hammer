//! Closing the window and quitting. On macOS, closing the window only hides it: the app keeps
//! running behind its menu-bar item ([`crate::menubar`]), so the chief of staff, firstmate's
//! watcher and the hook's waiting questions stay alive. Quit (⌘Q, the menu-bar item, the Dock)
//! ends them, so it asks first when that would stop work: a turn of the chief of staff's, or
//! workers in flight. With nothing in flight it quits at once.
//!
//! AppKit asks the app delegate `applicationShouldTerminate:` before every quit. Rune answers it
//! to start its own quit; [`install`] puts the app's question in front of Rune's answer (see
//! `lifecycle/mac.rs`). Elsewhere closing the window quits, as before, without asking.

use std::sync::Mutex;

#[cfg(target_os = "macos")]
mod mac;

/// What a quit would stop now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InFlight {
    /// The chief of staff is in the middle of a turn.
    pub turn: bool,
    /// The names of the workers firstmate has in flight.
    pub workers: Vec<String>,
}

impl InFlight {
    pub fn is_empty(&self) -> bool {
        !self.turn && self.workers.is_empty()
    }

    /// What is in flight, in a sentence.
    pub fn words(&self) -> String {
        let workers = match self.workers.as_slice() {
            [] => None,
            [one] => Some(format!("1 worker is in flight: {one}")),
            [rest @ .., last] => Some(format!(
                "{} workers are in flight: {} and {last}",
                self.workers.len(),
                rest.join(", ")
            )),
        };
        match (self.turn, workers) {
            (true, Some(workers)) => {
                format!("Your chief of staff is in the middle of a turn, and {workers}.")
            }
            (true, None) => "Your chief of staff is in the middle of a turn.".into(),
            (false, Some(workers)) => format!("{workers}."),
            (false, None) => "Nothing is in flight.".into(),
        }
    }
}

/// What quitting does to what is in flight, for the question.
pub const CONSEQUENCE: &str = "Quitting stops your chief of staff and firstmate's watcher. \
    Workers keep running in their own terminals, but nothing supervises them until you open \
    Velvet Hammer again and send a message.";

/// What the system's quit request reads, kept current from the app's state on the UI thread.
struct Guard {
    /// Work is in flight, so a quit asks first.
    busy: bool,
    /// The captain said yes to quitting: the next quit goes ahead.
    confirmed: bool,
    /// Asks the captain, once a quit was held for it.
    ask: Option<Box<dyn Fn() + Send>>,
}

static GUARD: Mutex<Guard> = Mutex::new(Guard {
    busy: false,
    confirmed: false,
    ask: None,
});

fn guard() -> std::sync::MutexGuard<'static, Guard> {
    GUARD.lock().unwrap_or_else(|e| e.into_inner())
}

/// Says whether a quit should ask first, from the app's state as it stands.
pub fn publish(in_flight: &InFlight) {
    guard().busy = !in_flight.is_empty();
}

/// What runs when a quit is held to ask the captain: open the window and show the question.
pub fn on_ask(ask: impl Fn() + Send + 'static) {
    guard().ask = Some(Box::new(ask));
}

/// The captain said yes: the quit that follows goes ahead whatever is in flight.
pub fn confirm() {
    guard().confirmed = true;
}

impl Guard {
    /// Whether a quit the system asks for now goes ahead. When it may not, the captain is asked.
    fn may_quit(&self) -> bool {
        if self.confirmed || !self.busy {
            return true;
        }
        if let Some(ask) = &self.ask {
            ask();
        }
        false
    }
}

/// [`Guard::may_quit`] for the app's guard.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn may_quit() -> bool {
    guard().may_quit()
}

/// Shows the app if it was hidden (⌘H) and makes it the active app, so a window it opens is seen.
/// Call it on the UI thread; does nothing off macOS.
pub fn come_forward() {
    #[cfg(target_os = "macos")]
    mac::come_forward();
}

/// Puts the app's question in front of every quit the system asks for. Call it on the UI thread
/// once Rune has started its event loop; later calls do nothing. Does nothing off macOS.
pub fn install() {
    #[cfg(target_os = "macos")]
    mac::install();
}

/// A worker in one of these states is no longer in flight.
const FINISHED: [&str; 2] = ["done", "failed"];

impl crate::App {
    /// What a quit would stop now.
    pub fn in_flight(&self) -> InFlight {
        InFlight {
            turn: self.working,
            workers: self
                .captain
                .crew
                .iter()
                .filter(|c| !FINISHED.contains(&c.state.as_str()))
                .map(|c| c.name.clone())
                .collect(),
        }
    }

    /// A quit was held: asks the captain whether to quit anyway, saying what is in flight.
    pub fn ask_to_quit(&mut self) {
        self.quit_ask = Some(self.in_flight());
    }

    /// The captain said yes: quits, whatever is in flight now.
    pub fn quit_anyway(&mut self, tx: &mut rune::Tx) {
        self.quit_ask = None;
        confirm();
        tx.quit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_in_flight_reads_as_a_sentence() {
        let turn = InFlight {
            turn: true,
            workers: vec![],
        };
        assert_eq!(
            turn.words(),
            "Your chief of staff is in the middle of a turn."
        );
        let crew = InFlight {
            turn: true,
            workers: vec!["Docs".into(), "Lab window".into(), "Billing".into()],
        };
        assert_eq!(
            crew.words(),
            "Your chief of staff is in the middle of a turn, and 3 workers are in flight: \
             Docs, Lab window and Billing."
        );
        let one = InFlight {
            turn: false,
            workers: vec!["Docs".into()],
        };
        assert_eq!(one.words(), "1 worker is in flight: Docs.");
        assert!(InFlight::default().is_empty());
    }

    #[test]
    fn a_quit_asks_only_while_work_is_in_flight_and_goes_ahead_once_confirmed() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        let asked = Arc::new(AtomicUsize::new(0));
        let mut guard = Guard {
            busy: false,
            confirmed: false,
            ask: Some(Box::new({
                let asked = asked.clone();
                move || {
                    asked.fetch_add(1, Ordering::SeqCst);
                }
            })),
        };
        assert!(guard.may_quit());
        guard.busy = true;
        assert!(!guard.may_quit());
        assert_eq!(asked.load(Ordering::SeqCst), 1);
        guard.confirmed = true;
        assert!(guard.may_quit());
        assert_eq!(asked.load(Ordering::SeqCst), 1);
    }
}
