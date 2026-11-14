//! When the captain's messages reach the chief of staff. By default a message goes out the moment
//! the captain sends it, even while the chief of staff is working: Claude Code takes it in at the
//! next step of the turn (after the tool call that is running), or starts the next turn with it
//! when the turn has no step left. It never stops a tool call that is running. The other mode
//! keeps new messages in the app until the chief of staff is done, then sends them one at a time.
//! The queue lives in [`crate::App`], in memory only.

use serde_json::{Value, json};

use crate::store::{self, Saved};

/// The settings file in the data directory.
pub const SETTINGS_FILE: &str = "messages.json";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Delivery {
    /// Send each message at once; the chief of staff takes it in at its next step. The
    /// recommended default.
    #[default]
    Now,
    /// Keep new messages while the chief of staff works, and send them one at a time when it is
    /// done.
    Queue,
}

impl Delivery {
    pub const ALL: [Delivery; 2] = [Delivery::Now, Delivery::Queue];
    pub const RECOMMENDED: Delivery = Delivery::Now;

    pub fn label(self) -> &'static str {
        match self {
            Delivery::Now => "Send right away",
            Delivery::Queue => "Queue new messages until the chief of staff is done",
        }
    }

    /// The setting in a few words, for its card.
    pub fn summary(self) -> &'static str {
        match self {
            Delivery::Now => "Send right away",
            Delivery::Queue => "Queue until done",
        }
    }

    /// The trade-off in one sentence.
    pub fn explanation(self) -> &'static str {
        match self {
            Delivery::Now => {
                "A message you send while your chief of staff is working reaches it at its next \
                 step, so you can steer it or add to what it is doing without waiting. Nothing it \
                 is running is stopped."
            }
            Delivery::Queue => {
                "Messages you send while your chief of staff is working wait under the message \
                 field, where you can edit or delete them, and go out one at a time, in order, \
                 when it is done."
            }
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Delivery::Now => "now",
            Delivery::Queue => "queue",
        }
    }

    fn from_key(key: &str) -> Option<Delivery> {
        Delivery::ALL.into_iter().find(|d| d.key() == key)
    }
}

impl Saved for Delivery {
    const FILE: &'static str = SETTINGS_FILE;
    const FALLBACK: &'static str = "messages are sent right away";

    fn fallback() -> Self {
        Delivery::Now
    }

    fn to_json(&self) -> Value {
        json!({"version": store::VERSION, "delivery": self.key()})
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        store::check_version(value)?;
        value["delivery"]
            .as_str()
            .and_then(Delivery::from_key)
            .ok_or_else(|| "its delivery is not one Velvet Hammer knows".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_delivery_reads_back_as_written() {
        for delivery in Delivery::ALL {
            assert_eq!(Delivery::from_json(&delivery.to_json()), Ok(delivery));
        }
        assert!(Delivery::from_json(&json!({"version": 1, "delivery": "later"})).is_err());
        assert_eq!(Delivery::default(), Delivery::RECOMMENDED);
    }
}
