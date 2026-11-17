//! Calm mode: the chief of staff's tool uses fold away. Each stretch of tool uses between two
//! messages shows as one line saying how many ran, which, and whether any failed or still runs;
//! pressing it shows the cards as they are drawn without calm mode. Every stretch starts folded
//! and stays as the captain left it while more messages arrive. Views (the `app_ui` tool) are
//! replies, not tool noise, so they always show and end a stretch. Calm mode is on until the
//! captain turns it off in Settings.

use std::ops::Range;

use serde_json::{Value, json};

use crate::model::Message;
use crate::store::{self, Saved};
use crate::tools::{self, Handler};

/// The settings file in the data directory.
pub const SETTINGS_FILE: &str = "calm.json";

/// Whether calm mode is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Calm(pub bool);

impl Default for Calm {
    fn default() -> Self {
        Calm::RECOMMENDED
    }
}

impl Calm {
    pub const RECOMMENDED: Calm = Calm(true);
    pub const ALL: [Calm; 2] = [Calm(true), Calm(false)];

    pub fn key(self) -> &'static str {
        if self.0 { "on" } else { "off" }
    }

    pub fn label(self) -> &'static str {
        if self.0 { "On" } else { "Off" }
    }

    pub fn explanation(self) -> &'static str {
        if self.0 {
            "Your chief of staff's tool uses between two messages fold into one line that says \
             how many ran and whether any failed. Press the line to see them. Views always show."
        } else {
            "Every tool use shows as its own card where it happened."
        }
    }
}

impl Saved for Calm {
    const FILE: &'static str = SETTINGS_FILE;
    const FALLBACK: &'static str = "calm mode is on";

    fn fallback() -> Self {
        Calm::RECOMMENDED
    }

    fn to_json(&self) -> Value {
        json!({"version": store::VERSION, "on": self.0})
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        store::check_version(value)?;
        value["on"]
            .as_bool()
            .map(Calm)
            .ok_or_else(|| "it does not say whether calm mode is on".into())
    }
}

/// Whether calm mode folds `m` away: a tool use other than a view.
pub fn folds(m: &Message) -> bool {
    m.tool
        .as_ref()
        .is_some_and(|tool| tools::handler(tool) != Handler::Ui)
}

/// Each stretch of messages calm mode folds, in order. A stretch never crosses `split`, where
/// this session's messages start after the earlier sessions'.
pub fn stretches(messages: &[Message], split: usize) -> Vec<Range<usize>> {
    let mut all = Vec::new();
    let mut start = None;
    for (i, m) in messages.iter().enumerate() {
        if start.is_some() && i == split {
            all.extend(start.take().map(|s| s..i));
        }
        match (folds(m), start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                all.push(s..i);
                start = None;
            }
            _ => {}
        }
    }
    all.extend(start.map(|s| s..messages.len()));
    all
}

/// The key a stretch is opened by: its first tool use's id, which stays the same as tool uses
/// join the stretch and messages arrive after it.
pub fn stretch_key(messages: &[Message], stretch: &Range<usize>) -> String {
    messages[stretch.start]
        .tool
        .as_ref()
        .map(|tool| tool.id.clone())
        .unwrap_or_default()
}

/// The folded line of a stretch: how many tool uses, the tools by name, and how many failed or
/// still run, in words.
pub fn summary(messages: &[Message]) -> String {
    const NAMED: usize = 3;
    let uses: Vec<&tools::ToolUse> = messages.iter().filter_map(|m| m.tool.as_ref()).collect();
    let mut names: Vec<&str> = Vec::new();
    for tool in &uses {
        if !names.contains(&tool.name.as_str()) {
            names.push(&tool.name);
        }
    }
    let mut named = names[..names.len().min(NAMED)].join(", ");
    if names.len() > NAMED {
        named.push_str(&format!(" and {} more", names.len() - NAMED));
    }
    let n = uses.len();
    let mut parts = vec![format!(
        "{n} tool {}: {named}",
        if n == 1 { "use" } else { "uses" }
    )];
    let failed = uses
        .iter()
        .filter(|t| t.result.as_ref().is_some_and(|r| r.is_error))
        .count();
    if failed > 0 {
        parts.push(format!("{failed} failed"));
    }
    if uses.iter().any(|t| t.result.is_none()) {
        parts.push("running\u{2026}".into());
    }
    parts.join(" \u{b7} ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Author;
    use crate::tools::{ToolResult, ToolUse};

    fn tool(id: &str, name: &str, result: Option<bool>) -> Message {
        Message::tool(ToolUse {
            id: id.into(),
            name: name.into(),
            input: json!({}),
            result: result.map(|is_error| ToolResult {
                is_error,
                content: Vec::new(),
            }),
        })
    }

    fn said(words: &str) -> Message {
        Message::new(Author::ChiefOfStaff, words)
    }

    #[test]
    fn tool_uses_between_messages_make_one_stretch_each() {
        let chat = [
            said("a"),
            tool("1", "Bash", Some(false)),
            tool("2", "Read", Some(false)),
            said("b"),
            tool("3", "Grep", None),
        ];
        assert_eq!(stretches(&chat, 0), [1..3, 4..5]);
        assert_eq!(stretch_key(&chat, &(1..3)), "1");
    }

    #[test]
    fn a_view_shows_and_ends_a_stretch() {
        let chat = [
            tool("1", "Bash", Some(false)),
            tool("2", &crate::ui::tool_name(), Some(false)),
            tool("3", "Read", Some(false)),
        ];
        assert_eq!(stretches(&chat, 0), [0..1, 2..3]);
    }

    #[test]
    fn a_stretch_stops_where_this_session_starts() {
        let chat = [
            tool("1", "Bash", Some(false)),
            tool("2", "Bash", Some(false)),
            tool("3", "Bash", Some(false)),
        ];
        assert_eq!(stretches(&chat, 2), [0..2, 2..3]);
    }

    #[test]
    fn the_folded_line_counts_names_and_says_what_failed_or_runs() {
        let chat = [
            tool("1", "Bash", Some(true)),
            tool("2", "Read", Some(false)),
            tool("3", "Bash", Some(false)),
            tool("4", "Grep", Some(false)),
            tool("5", "Edit", None),
        ];
        assert_eq!(
            summary(&chat),
            "5 tool uses: Bash, Read, Grep and 1 more \u{b7} 1 failed \u{b7} running\u{2026}"
        );
        assert_eq!(summary(&chat[1..2]), "1 tool use: Read");
    }

    #[test]
    fn calm_mode_is_on_until_turned_off_and_reads_back() {
        assert_eq!(Calm::default(), Calm(true));
        for calm in Calm::ALL {
            assert_eq!(Calm::from_json(&calm.to_json()), Ok(calm));
        }
        assert!(Calm::from_json(&json!({"version": store::VERSION})).is_err());
    }
}
