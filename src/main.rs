//! Velvet Hammer: one conversation with a chief of staff, the subagents it manages, and two
//! panels for what waits on the captain. `cargo run` opens the window;
//! `cargo run -- --snap chat.png` renders it headless (see the README for every scene).

mod agent;
mod model;
mod view;

use rune::{AppSpec, TextEdit};

use agent::{ChiefOfStaff, Stub};
use model::{Action, Id, Workspace};

/// All app state: the workspace the agent layer fills, plus what only the UI needs.
pub struct App {
    pub ws: Workspace,
    pub chief: Box<dyn ChiefOfStaff>,
    pub draft: TextEdit,
    /// The subagent whose transcript replaces the chat, if the captain is peeking.
    pub peek: Option<Id>,
}

impl App {
    pub fn new(mut chief: Box<dyn ChiefOfStaff>) -> Self {
        App {
            ws: chief.open(),
            chief,
            draft: TextEdit::default(),
            peek: None,
        }
    }

    pub fn act(&mut self, action: Action) {
        self.ws.apply(&action);
        let replies = self.chief.respond(&self.ws, &action);
        self.ws.chat.extend(replies);
    }

    pub fn send(&mut self) {
        let text = self.draft.text.trim().to_string();
        if text.is_empty() {
            return;
        }
        self.draft.clear();
        self.act(Action::Say(text));
    }
}

fn init() -> App {
    App::new(Box::new(Stub))
}

fn main() {
    rune::run(AppSpec::<App> {
        scenes: &[
            ("chat", |_| {}),
            ("replied", |s| {
                s.draft.text = "Ship it when CI is green.".into();
                s.send();
            }),
            ("empty", |s| s.ws.items.clear()),
            ("peek", |s| s.peek = s.ws.agents.first().map(|a| a.id)),
        ],
        min_size: Some((900.0, 560.0)),
        ..AppSpec::new("Velvet Hammer", (1100.0, 720.0), init, view::view)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use rune::{Env, Runtime};

    fn runtime() -> Runtime<App> {
        let env = Env {
            system_dark: true,
            scale: 2.0,
        };
        let mut rt = Runtime::new(init(), view::view, (1100.0, 720.0), env);
        rt.frame(0.0);
        rt
    }

    fn tap(rt: &mut Runtime<App>, key: &str) {
        let (x, y) = rt
            .find(key)
            .unwrap_or_else(|| panic!("no element keyed {key}"));
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
    }

    #[test]
    fn sending_a_message_adds_it_and_a_reply() {
        let mut rt = runtime();
        let before = rt.state.ws.chat.len();
        rt.state.draft.text = "hello".into();
        rt.frame(1.0 / 60.0);
        tap(&mut rt, "send");
        let chat = &rt.state.ws.chat;
        assert_eq!(chat.len(), before + 2);
        assert_eq!(chat[before].text, "hello");
        assert_eq!(chat[before + 1].author, model::Author::ChiefOfStaff);
        assert!(rt.state.draft.text.is_empty());
    }

    #[test]
    fn an_empty_draft_sends_nothing() {
        let mut rt = runtime();
        let before = rt.state.ws.chat.len();
        rt.state.draft.text = "   ".into();
        tap(&mut rt, "send");
        assert_eq!(rt.state.ws.chat.len(), before);
    }

    #[test]
    fn answering_an_item_removes_it_from_its_panel() {
        let mut rt = runtime();
        assert!(rt.find("input-1").is_some());
        tap(&mut rt, "answer-1-0");
        assert!(rt.state.ws.item(1).is_none());
        assert!(rt.find("input-1").is_none());
        assert!(rt.state.ws.chat.last().unwrap().text.contains("1100×720"));
    }

    #[test]
    fn marking_a_task_done_removes_it_and_shows_the_empty_state() {
        let mut rt = runtime();
        assert!(rt.find("for-you-empty").is_none());
        tap(&mut rt, "done-3");
        assert!(rt.state.ws.item(3).is_none());
        assert!(rt.find("for-you-empty").is_some());
    }

    #[test]
    fn peeking_at_an_agent_replaces_the_chat_until_closed() {
        let mut rt = runtime();
        tap(&mut rt, "agent-2");
        assert_eq!(rt.state.peek, Some(2));
        assert!(rt.find("peek").is_some());
        assert!(rt.find("chat").is_none());
        tap(&mut rt, "peek-close");
        assert_eq!(rt.state.peek, None);
        assert!(rt.find("chat").is_some());
    }
}
