//! The boundary between the UI and the agent layer. The app only talks to a [`ChiefOfStaff`];
//! today that is [`Stub`], which simulates replies. A Pi-backed chief of staff replaces it by
//! implementing the same trait.

use crate::model::{Action, Agent, AgentStatus, Author, Id, Item, ItemKind, Message, Workspace};

pub trait ChiefOfStaff {
    /// The workspace as it stands when the app opens.
    fn open(&mut self) -> Workspace;
    /// The chief of staff's replies to an action the captain took. `ws` already reflects it.
    fn respond(&mut self, ws: &Workspace, action: &Action) -> Vec<Message>;
}

/// Simulated chief of staff: fixed starting data and canned replies.
pub struct Stub;

impl ChiefOfStaff for Stub {
    fn open(&mut self) -> Workspace {
        Workspace {
            chat: seed_chat(),
            items: seed_items(),
            agents: seed_agents(),
        }
    }

    fn respond(&mut self, _ws: &Workspace, action: &Action) -> Vec<Message> {
        let reply = match action {
            Action::Say(text) => format!("Got it: \u{201c}{text}\u{201d}. I'll take it from here."),
            Action::Answer { choice, .. } => {
                format!("Thanks. Going with \u{201c}{choice}\u{201d}.")
            }
            Action::Done { .. } => "Thanks, I've let the team know that's done.".to_string(),
        };
        vec![Message::new(Author::ChiefOfStaff, reply)]
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
