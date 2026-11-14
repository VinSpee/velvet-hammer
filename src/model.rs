//! The app's data: one chief-of-staff conversation, the items waiting on the captain, and the
//! subagents the chief of staff manages. Everything the view draws comes from a [`Workspace`],
//! and every change the captain makes is an [`Action`] applied to it.

use crate::tools::ToolUse;
use crate::ui::UiAction;

pub type Id = u32;

/// Who wrote a message: the captain, the chief of staff, or a subagent (in a peek transcript).
/// `Problem` is a failure the agent layer reported, shown in the chat in its own words. `Tool` is
/// one of the chief of staff's tool uses. `App` is something Velvet Hammer itself did, such as
/// sending a reply back to be shown as a view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Author {
    Captain,
    ChiefOfStaff,
    Subagent,
    Problem,
    Tool,
    App,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    /// Names the message in its link (see [`crate::links`]). It is random, so it never names
    /// another message, and the history keeps it, so it lasts across restarts.
    pub id: String,
    pub author: Author,
    pub text: String,
    /// The tool use an [`Author::Tool`] message shows.
    pub tool: Option<ToolUse>,
}

impl Message {
    pub fn new(author: Author, text: impl Into<String>) -> Self {
        Message {
            id: new_id(),
            author,
            text: text.into(),
            tool: None,
        }
    }

    pub fn tool(tool: ToolUse) -> Self {
        Message {
            id: new_id(),
            author: Author::Tool,
            text: String::new(),
            tool: Some(tool),
        }
    }
}

/// A new message id: 12 random hexadecimal digits.
pub fn new_id() -> String {
    let mut bytes = [0u8; 6];
    // Without the system's randomness the clock still keeps ids apart.
    if getrandom::fill(&mut bytes).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        bytes.copy_from_slice(&nanos.to_le_bytes()[..6]);
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether the chief of staff can talk, as the agent layer reports it. Claude Code owns the
/// sign-in; the app only shows what `claude` says about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Asking Claude Code whether it is signed in.
    Checking,
    /// Claude Code has no sign-in. `problem` is why the last sign-in attempt ended, if one did.
    SignedOut { problem: Option<String> },
    /// Claude Code's own sign-in is running; `url` is the sign-in page once `claude` prints it.
    SigningIn { url: Option<String> },
    /// Claude Code says it is signed in, but Anthropic rejected the sign-in on the last message.
    Expired,
    /// Claude Code is signed in, but not with a Claude subscription (an Anthropic Console or API
    /// key sign-in, say); `method` is what it reports. The chat stays locked.
    NotSubscription { method: String },
    /// No `claude` on the app's PATH.
    CliMissing,
    /// `claude` is installed but failed in a way signing in cannot fix; the text is its own.
    Failed(String),
    /// Ready to chat.
    Ready(Plan),
}

/// What pays for the chief of staff's messages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plan {
    /// A Claude subscription sign-in; the text is the plan Claude Code reports, such as "max".
    Subscription(String),
    /// The stub: nothing is billed.
    Simulated,
}

/// What an item asks of the captain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemKind {
    /// The agent is blocked until the captain picks one of these answers.
    NeedsInput { options: Vec<String> },
    /// A step only the captain can take; the agent waits for it to be marked done.
    ForYou,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub id: Id,
    pub kind: ItemKind,
    pub title: String,
    pub detail: String,
    /// The subagent that raised it, if any; otherwise the chief of staff did.
    pub agent: Option<Id>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentStatus {
    Working,
    Waiting,
    Done,
}

impl AgentStatus {
    pub fn label(self) -> &'static str {
        match self {
            AgentStatus::Working => "working",
            AgentStatus::Waiting => "waiting",
            AgentStatus::Done => "done",
        }
    }
}

/// A subagent the chief of staff delegated to. The captain can peek at its transcript.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Agent {
    pub id: Id,
    pub name: String,
    pub task: String,
    pub status: AgentStatus,
    pub transcript: Vec<Message>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Workspace {
    /// The conversation, oldest first. The first [`Workspace::earlier`] messages are kept from
    /// earlier sessions (see [`crate::history`]); the chief of staff does not remember them.
    pub chat: Vec<Message>,
    pub earlier: usize,
    pub items: Vec<Item>,
    pub agents: Vec<Agent>,
}

/// Something the captain did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Say(String),
    Answer {
        item: Id,
        choice: String,
    },
    Done {
        item: Id,
    },
    /// A control in a view the chief of staff showed (see [`crate::ui`]).
    Ui(UiAction),
}

impl Workspace {
    pub fn needs_input(&self) -> impl Iterator<Item = &Item> {
        self.items
            .iter()
            .filter(|i| matches!(i.kind, ItemKind::NeedsInput { .. }))
    }

    pub fn for_you(&self) -> impl Iterator<Item = &Item> {
        self.items.iter().filter(|i| i.kind == ItemKind::ForYou)
    }

    pub fn item(&self, id: Id) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }

    /// The chat message with this id, by its index.
    pub fn message(&self, id: &str) -> Option<(usize, &Message)> {
        self.chat.iter().enumerate().find(|(_, m)| m.id == id)
    }

    pub fn agent(&self, id: Id) -> Option<&Agent> {
        self.agents.iter().find(|a| a.id == id)
    }

    /// Records the captain's side of `action`: what they said goes into the chat, and an
    /// answered or finished item leaves its panel. The agent layer replies separately.
    pub fn apply(&mut self, action: &Action) {
        match action {
            Action::Say(text) => self.chat.push(Message::new(Author::Captain, text.clone())),
            Action::Ui(ui) => self
                .chat
                .push(Message::new(Author::Captain, format!("{}.", ui.did))),
            Action::Answer { item, .. } | Action::Done { item } => {
                self.items.retain(|i| i.id != *item)
            }
        }
    }
}
