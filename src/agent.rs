//! The boundary between the UI and the agent layer. The app only talks to a [`ChiefOfStaff`]:
//! [`Stub`] simulates one for headless scenes and tests, and [`crate::claude::Claude`] runs the
//! captain's own Claude Code.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rune::Waker;

use crate::approval::Entry;
use crate::bridge::Policies;
use crate::model::{
    Action, Agent, AgentStatus, Author, Id, Item, ItemKind, Message, Plan, Status, Workspace,
};
use crate::tools::{ToolResult, ToolUse};
use crate::ui::Sent;

/// Something the agent layer reports. The app drains these each frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Status(Status),
    /// Whether the chief of staff is working: running a turn, or holding a message it has not
    /// taken in yet. Messages queued in the app wait until it is `false`.
    Working(bool),
    /// The chief of staff's reply so far. It replaces the reply in progress or starts one;
    /// `done` ends it.
    Reply {
        text: String,
        done: bool,
    },
    /// A failure to show in the chat, in the agent layer's own words.
    Problem(String),
    /// The chief of staff started a tool use; its result follows as [`Event::ToolResult`].
    Tool(ToolUse),
    /// More of a tool use's input as it streams in, by its id. Only the view tool streams.
    ToolInput {
        id: String,
        input: serde_json::Value,
    },
    /// What a tool use returned, by its id.
    ToolResult {
        id: String,
        result: ToolResult,
    },
    /// A new item for one of the captain's panels.
    Item(Item),
    /// The item no longer needs the captain, such as a question whose session ended.
    Withdrawn(Id),
    /// A tool use and what was decided, for the activity log.
    Activity(Entry),
    /// Something Velvet Hammer itself did in the conversation, shown in the chat in its words.
    Notice(String),
    /// The chief of staff opened the guided flow for adding a second mate, with the scope the
    /// captain described.
    AddMate(String),
}

pub trait ChiefOfStaff {
    /// The workspace as it stands when the app opens. The first [`Event::Status`] follows.
    fn open(&mut self) -> Workspace;
    /// Called once with a waker that background threads call when events are waiting.
    fn connect(&mut self, _wake: Waker) {}
    /// The captain did something; `ws` already reflects it. Replies arrive as events.
    fn act(&mut self, ws: &Workspace, action: &Action);
    /// Everything reported since the last call.
    fn events(&mut self) -> Vec<Event>;
    /// Start the agent layer's own sign-in.
    fn sign_in(&mut self) {}
    /// Stop a sign-in in progress.
    fn cancel_sign_in(&mut self) {}
    /// Ask the agent layer again whether it is signed in.
    fn recheck(&mut self) {}
    /// The managed firstmate clone the chief of staff runs in, if it runs in one.
    fn workplace(&self) -> Option<&Path> {
        None
    }
    /// The captain's tool-approval, visual-replies and firstmate-rule settings, shared so a
    /// change applies at once.
    fn approve_with(&mut self, _policies: Policies) {}
}

/// Simulated chief of staff: fixed starting data and canned replies.
#[derive(Default)]
pub struct Stub {
    queued: Vec<Event>,
    /// Set for a stub whose turns run until the test ends them (see [`Stub::slow`]).
    slow: Option<Finish>,
    /// What the turn that is running was asked, in order, while one runs.
    turn: Vec<String>,
}

/// Ends the turn a slow [`Stub`] is running, as the chief of staff finishing its work would.
#[derive(Clone, Default)]
pub struct Finish(Arc<AtomicBool>);

impl Finish {
    pub fn finish(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

impl Stub {
    /// A stub that keeps working on each message until `Finish::finish` is called. A message
    /// sent while it works is taken into the same turn, as Claude Code takes one in at the
    /// turn's next step, and the reply that ends the turn answers all of them.
    pub fn slow() -> (Stub, Finish) {
        let finish = Finish::default();
        let stub = Stub {
            slow: Some(finish.clone()),
            ..Stub::default()
        };
        (stub, finish)
    }
}

impl ChiefOfStaff for Stub {
    fn open(&mut self) -> Workspace {
        self.queued
            .push(Event::Status(Status::Ready(Plan::Simulated)));
        Workspace {
            chat: seed_chat(),
            items: seed_items(),
            agents: seed_agents(),
            ..Workspace::default()
        }
    }

    fn act(&mut self, _ws: &Workspace, action: &Action) {
        if self.slow.is_some()
            && let Action::Say { text, .. } = action
        {
            // The reply so far ends where the first message sent during the turn arrives, so
            // the rest of the turn shows after it.
            let first = self.turn.first().unwrap_or(text);
            let working = format!("Working on \u{201c}{first}\u{201d}\u{2026}");
            match self.turn.len() {
                0 => {
                    self.queued.push(Event::Working(true));
                    self.queued.push(Event::Reply {
                        text: working,
                        done: false,
                    });
                }
                1 => self.queued.push(Event::Reply {
                    text: working,
                    done: true,
                }),
                _ => {}
            }
            self.turn.push(text.clone());
            return;
        }
        let text = match action {
            Action::Say { text, attachments } if !attachments.is_empty() => format!(
                "Got it: \u{201c}{text}\u{201d}, with {}.",
                crate::attach::count(attachments)
            ),
            Action::Say { text, .. } => {
                format!("Got it: \u{201c}{text}\u{201d}. I'll take it from here.")
            }
            Action::Answer { choice, .. } => {
                format!("Thanks. Going with \u{201c}{choice}\u{201d}.")
            }
            Action::Done { .. } => "Thanks, I've let the team know that's done.".to_string(),
            Action::Ui(ui) => match &ui.sent {
                Sent::Submit { params, .. } => format!("Thanks. I got your submit with {params}."),
                Sent::Answer { answer, .. } => {
                    let values: Vec<&str> =
                        answer.picked.iter().map(|o| o.value.as_str()).collect();
                    format!(
                        "Thanks. I got the values {values:?} and the words \u{201c}{}\u{201d}.",
                        answer.other
                    )
                }
            },
        };
        self.queued.push(Event::Reply { text, done: true });
    }

    fn events(&mut self) -> Vec<Event> {
        if let Some(finish) = &self.slow
            && finish.0.swap(false, Ordering::Relaxed)
            && let Some((first, rest)) = self.turn.split_first()
        {
            let mut text = format!("Done with \u{201c}{first}\u{201d}.");
            for also in rest {
                text.push_str(&format!(" I also took in \u{201c}{also}\u{201d}."));
            }
            self.queued.push(Event::Reply { text, done: true });
            self.queued.push(Event::Working(false));
            self.turn.clear();
        }
        std::mem::take(&mut self.queued)
    }
}

fn seed_chat() -> Vec<Message> {
    vec![
        Message::new(
            Author::Captain,
            "Get the desktop skeleton building and check it headlessly.",
        ),
        Message::new(
            Author::ChiefOfStaff,
            "On it. I've split the work between two crewmates. Two questions need you, and one \
             step is yours to do.",
        ),
    ]
}

fn seed_items() -> Vec<Item> {
    let input = |id: Id, agent, title: &str, detail: &str, options: &[&str]| Item {
        id,
        kind: ItemKind::NeedsInput {
            options: options.iter().map(|o| o.to_string()).collect(),
        },
        title: title.into(),
        detail: detail.into(),
        agent,
    };
    let task = |id: Id, title: &str, detail: &str| Item {
        id,
        kind: ItemKind::ForYou,
        title: title.into(),
        detail: detail.into(),
        agent: None,
    };
    vec![
        input(
            1,
            Some(1),
            "Which window size?",
            "The layout fits both; the larger one shows more chat.",
            &["1100×720", "1280×800"],
        ),
        input(
            2,
            Some(2),
            "Open the PR as a draft?",
            "CI is still running on the branch.",
            &["Draft", "Ready"],
        ),
        task(
            3,
            "Sign in to GitHub",
            "Run `gh auth login` in a terminal; agents cannot sign in for you.",
        ),
    ]
}

fn seed_agents() -> Vec<Agent> {
    vec![
        Agent {
            id: 1,
            name: "layout".into(),
            task: "Build the panel layout".into(),
            status: AgentStatus::Waiting,
            transcript: vec![
                Message::new(
                    Author::ChiefOfStaff,
                    "Lay out the chat, the agents list and both panels.",
                ),
                Message::new(
                    Author::Subagent,
                    "Done with the three columns. Asking the captain about the window size.",
                ),
            ],
        },
        Agent {
            id: 2,
            name: "verify".into(),
            task: "Render every state headlessly".into(),
            status: AgentStatus::Working,
            transcript: vec![
                Message::new(
                    Author::ChiefOfStaff,
                    "Render PNGs of each state and dump the element tree.",
                ),
                Message::new(
                    Author::Subagent,
                    "Rendering chat, empty and peek scenes now.",
                ),
            ],
        },
    ]
}
