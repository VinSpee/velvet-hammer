//! Velvet Hammer: one conversation with a chief of staff, the subagents it manages, and two
//! panels for what waits on the captain. `cargo run` opens the window;
//! `cargo run -- --snap chat.png` renders it headless (see the README for every scene).

mod agent;
mod bridge;
mod claude;
mod model;
mod view;

use std::time::{Duration, Instant};

use rune::{AppSpec, TextEdit, Tx, Waker};

use agent::{ChiefOfStaff, Event, Stub};
use claude::Claude;
use model::{Action, Author, Id, Message, Status, Workspace};

/// All app state: the workspace the agent layer fills, plus what only the UI needs.
pub struct App {
    pub ws: Workspace,
    pub chief: Box<dyn ChiefOfStaff>,
    pub status: Status,
    pub draft: TextEdit,
    /// The subagent whose transcript replaces the chat, if the captain is peeking.
    pub peek: Option<Id>,
    /// The chat message the chief of staff is still writing.
    pub replying: Option<usize>,
}

impl App {
    pub fn new(mut chief: Box<dyn ChiefOfStaff>) -> Self {
        let mut app = App {
            ws: chief.open(),
            chief,
            status: Status::Checking,
            draft: TextEdit::default(),
            peek: None,
            replying: None,
        };
        app.drain();
        app
    }

    /// Applies what the agent layer reported; true when anything changed.
    pub fn drain(&mut self) -> bool {
        let events = self.chief.events();
        let changed = !events.is_empty();
        for event in events {
            match event {
                Event::Status(status) => self.status = status,
                Event::Reply { text, done } => {
                    match self.replying {
                        Some(i) => self.ws.chat[i].text = text,
                        None => {
                            self.ws.chat.push(Message::new(Author::ChiefOfStaff, text));
                            self.replying = Some(self.ws.chat.len() - 1);
                        }
                    }
                    if done {
                        self.replying = None;
                    }
                }
                Event::Problem(text) => {
                    self.replying = None;
                    self.ws.chat.push(Message::new(Author::Problem, text));
                }
                Event::Item(item) => self.ws.items.push(item),
                Event::Withdrawn(id) => self.ws.items.retain(|i| i.id != id),
            }
        }
        changed
    }

    pub fn act(&mut self, action: Action) {
        self.ws.apply(&action);
        self.chief.act(&self.ws, &action);
        self.drain();
    }

    pub fn send(&mut self) {
        let text = self.draft.text.trim().to_string();
        if text.is_empty() || !matches!(self.status, Status::Ready(_)) {
            return;
        }
        self.draft.clear();
        self.act(Action::Say(text));
    }

    pub fn sign_in(&mut self) {
        self.chief.sign_in();
        self.drain();
    }

    pub fn cancel_sign_in(&mut self) {
        self.chief.cancel_sign_in();
        self.drain();
    }

    pub fn recheck(&mut self) {
        self.chief.recheck();
        self.drain();
    }

    /// Opens the sign-in page Claude Code printed in the system browser. Claude Code opens it
    /// too; this is for when that did not work or the tab was closed.
    pub fn open_sign_in_page(&self) {
        if let Status::SigningIn { url: Some(url) } = &self.status {
            let opener = if cfg!(target_os = "macos") {
                "open"
            } else {
                "xdg-open"
            };
            let _ = std::process::Command::new(opener).arg(url).spawn();
        }
    }

    /// Drains events until `done` holds, for headless scenes that drive a real process.
    fn settle(&mut self, done: impl Fn(&App) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !done(self) {
            assert!(Instant::now() < deadline, "the agent layer never settled");
            std::thread::sleep(Duration::from_millis(20));
            self.drain();
        }
    }
}

fn init() -> App {
    App::new(Box::new(Claude::new("claude")))
}

fn stub() -> App {
    App::new(Box::new(Stub::default()))
}

/// The fake `claude` in `scripts/`, which speaks the real one's protocol without an account.
const FAKE_CLAUDE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/fake-claude");

/// An app on the fake `claude`, starting in `state` (see `scripts/fake-claude`).
fn fake(state: &str, extra_env: &[(&str, &str)]) -> App {
    on_fake(FAKE_CLAUDE, &scratch(state), extra_env)
}

fn fake_program(program: &str, state: &str, extra_env: &[(&str, &str)]) -> App {
    on_fake(program, &scratch(state), extra_env)
}

/// A fresh scratch directory for the fake `claude`, so runs never share sign-in state. They live
/// under `target/`, so `cargo clean` removes them.
fn scratch(state: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static RUN: AtomicU32 = AtomicU32::new(0);
    let home = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/fake-claude")
        .join(format!(
            "{}-{}",
            std::process::id(),
            RUN.fetch_add(1, Ordering::Relaxed)
        ));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("scratch directory");
    std::fs::write(home.join("state"), state).expect("fake claude state");
    home
}

fn on_fake(program: &str, home: &std::path::Path, extra_env: &[(&str, &str)]) -> App {
    let mut claude = Claude::new(program).env("FAKE_CLAUDE_HOME", home.to_string_lossy());
    for (k, v) in extra_env {
        claude = claude.env(k, *v);
    }
    App::new(Box::new(claude))
}

fn settled(s: &App) -> bool {
    s.status != Status::Checking
}

fn replied(s: &App) -> bool {
    s.replying.is_none()
        && s.ws
            .chat
            .last()
            .is_some_and(|m| m.author != Author::Captain)
}

fn poll(s: &mut App, _: f64, _: &mut Tx) -> bool {
    s.drain()
}

fn setup(s: &mut App, wake: Waker) {
    s.chief.connect(wake);
}

fn main() {
    rune::run(AppSpec::<App> {
        scenes: &[
            ("chat", |s| *s = stub()),
            ("replied", |s| {
                *s = stub();
                s.draft.text = "Ship it when CI is green.".into();
                s.send();
            }),
            ("empty", |s| {
                *s = stub();
                s.ws.items.clear()
            }),
            ("peek", |s| {
                *s = stub();
                s.peek = s.ws.agents.first().map(|a| a.id)
            }),
            // The rest drive the fake `claude` for real.
            ("signed-out", |s| {
                *s = fake("signed-out", &[]);
                s.settle(settled);
            }),
            ("signing-in", |s| {
                *s = fake("signed-out", &[]);
                s.settle(settled);
                s.sign_in();
                s.settle(|s| matches!(s.status, Status::SigningIn { url: Some(_) }));
            }),
            ("signed-in", |s| {
                *s = fake("signed-in", &[]);
                s.settle(settled);
                s.draft.text = "What needs me today?".into();
                s.send();
                s.settle(replied);
            }),
            ("expired", |s| {
                *s = fake("expired", &[]);
                s.settle(settled);
                s.draft.text = "What needs me today?".into();
                s.send();
                s.settle(|s| s.status == Status::Expired);
            }),
            ("cli-missing", |s| {
                *s = fake_program("/nonexistent/claude", "signed-out", &[]);
                s.settle(settled);
            }),
            // The fake calls back into the app through the bridge: the app_note tool, and the
            // PreToolUse hook waiting on the captain, then allowed.
            ("noted", |s| {
                *s = fake("signed-in", &[]);
                s.settle(settled);
                s.draft.text = "note: Renew the velvet-hammer.dev domain".into();
                s.send();
                s.settle(replied);
            }),
            ("approval", |s| {
                *s = fake("signed-in", &[]);
                s.settle(settled);
                s.draft.text = "run: git push".into();
                s.send();
                s.settle(|s| s.ws.needs_input().next().is_some());
            }),
            ("approved", |s| {
                *s = fake("signed-in", &[]);
                s.settle(settled);
                s.draft.text = "run: git push".into();
                s.send();
                s.settle(|s| s.ws.needs_input().next().is_some());
                let item = s.ws.needs_input().next().map(|i| i.id).unwrap();
                s.act(Action::Answer {
                    item,
                    choice: "Allow".into(),
                });
                s.settle(replied);
            }),
            ("approval-long", |s| {
                *s = fake("signed-in", &[]);
                s.settle(settled);
                s.draft.text = format!("run: {}", long_command());
                s.send();
                s.settle(|s| s.ws.needs_input().next().is_some());
            }),
            ("claude-error", |s| {
                *s = fake("signed-in", &[]);
                s.settle(settled);
                s.draft.text = "Please fail this one.".into();
                s.send();
                s.settle(replied);
            }),
        ],
        min_size: Some((900.0, 560.0)),
        setup: Some(setup),
        poll: Some(poll),
        ..AppSpec::new("Velvet Hammer", (1100.0, 720.0), init, view::view)
    });
}

/// A command whose dangerous part sits far past where a short preview would have cut it off.
fn long_command() -> String {
    format!("echo {}; rm -rf ~", "a harmless-looking prefix ".repeat(20))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Item;
    use rune::{Env, Runtime};

    fn runtime() -> Runtime<App> {
        runtime_with(stub())
    }

    fn runtime_with(app: App) -> Runtime<App> {
        let env = Env {
            system_dark: true,
            scale: 2.0,
        };
        let mut rt = Runtime::new(app, view::view, (1100.0, 720.0), env);
        rt.poll = Some(poll);
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

    // The rest run the app on `scripts/fake-claude`, a stand-in for Claude Code's `claude`.

    /// Frames until `done` holds, letting the fake's real processes run.
    fn wait(rt: &mut Runtime<App>, done: impl Fn(&App) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !done(&rt.state) {
            assert!(
                Instant::now() < deadline,
                "timed out; status is {:?}",
                rt.state.status
            );
            std::thread::sleep(Duration::from_millis(20));
            rt.frame(1.0 / 60.0);
        }
        rt.frame(1.0 / 60.0);
    }

    #[test]
    fn signed_out_shows_the_sign_in_screen_instead_of_the_chat() {
        let mut rt = runtime_with(fake("signed-out", &[]));
        wait(&mut rt, settled);
        assert_eq!(rt.state.status, Status::SignedOut { problem: None });
        assert!(rt.find("sign-in-button").is_some());
        assert!(rt.find("chat-input").is_none());
    }

    #[test]
    fn signing_in_runs_claude_and_returns_to_the_chat() {
        let home = scratch("signed-out");
        let mut rt = runtime_with(on_fake(FAKE_CLAUDE, &home, &[]));
        wait(&mut rt, settled);
        tap(&mut rt, "sign-in-button");
        wait(&mut rt, |s| {
            matches!(s.status, Status::SigningIn { url: Some(_) })
        });
        assert!(rt.find("sign-in-url").is_some());
        assert!(rt.find("cancel-sign-in").is_some());
        // The captain approves in the browser.
        std::fs::write(home.join("browser-done"), "").unwrap();
        wait(&mut rt, |s| matches!(s.status, Status::Ready(_)));
        assert_eq!(
            rt.state.status,
            Status::Ready(model::Plan::Subscription("max".into()))
        );
        assert!(rt.find("chat-input").is_some());
    }

    #[test]
    fn a_failed_sign_in_says_why_in_claudes_words() {
        let mut rt = runtime_with(fake("signed-out", &[("FAKE_CLAUDE_LOGIN", "fail")]));
        wait(&mut rt, settled);
        tap(&mut rt, "sign-in-button");
        wait(&mut rt, |s| {
            matches!(s.status, Status::SignedOut { problem: Some(_) })
        });
        let Status::SignedOut {
            problem: Some(problem),
        } = &rt.state.status
        else {
            unreachable!()
        };
        assert!(
            problem.contains("the fake sign-in was refused"),
            "{problem}"
        );
        assert!(rt.find("sign-in-problem").is_some());
    }

    #[test]
    fn cancelling_a_sign_in_goes_back_to_signed_out() {
        let mut rt = runtime_with(fake("signed-out", &[]));
        wait(&mut rt, settled);
        tap(&mut rt, "sign-in-button");
        wait(&mut rt, |s| {
            matches!(s.status, Status::SigningIn { url: Some(_) })
        });
        tap(&mut rt, "cancel-sign-in");
        assert_eq!(rt.state.status, Status::SignedOut { problem: None });
    }

    #[test]
    fn a_message_gets_claudes_streamed_reply() {
        let mut rt = runtime_with(fake("signed-in", &[]));
        wait(&mut rt, settled);
        assert!(rt.find("health").is_some());
        rt.state.draft.text = "hello \"there\"".into();
        tap(&mut rt, "send");
        wait(&mut rt, replied);
        let last = rt.state.ws.chat.last().unwrap();
        assert_eq!(last.author, model::Author::ChiefOfStaff);
        assert_eq!(last.text, "You said: hello \"there\"");
    }

    #[test]
    fn a_claude_failure_shows_in_the_chat() {
        let mut rt = runtime_with(fake("signed-in", &[]));
        wait(&mut rt, settled);
        rt.state.draft.text = "please fail".into();
        tap(&mut rt, "send");
        wait(&mut rt, replied);
        let last = rt.state.ws.chat.last().unwrap();
        assert_eq!(last.author, model::Author::Problem);
        assert!(last.text.contains("could not finish this turn"));
    }

    #[test]
    fn a_rejected_sign_in_shows_the_expired_screen() {
        let mut rt = runtime_with(fake("expired", &[]));
        wait(&mut rt, settled);
        assert!(matches!(rt.state.status, Status::Ready(_)));
        rt.state.draft.text = "hello".into();
        tap(&mut rt, "send");
        wait(&mut rt, |s| s.status == Status::Expired);
        assert!(rt.find("sign-in-button").is_some());
        assert!(rt.find("chat-input").is_none());
    }

    #[test]
    fn signing_in_again_after_expiry_returns_to_the_chat() {
        let home = scratch("expired");
        let mut rt = runtime_with(on_fake(FAKE_CLAUDE, &home, &[]));
        wait(&mut rt, settled);
        rt.state.draft.text = "hello".into();
        tap(&mut rt, "send");
        wait(&mut rt, |s| s.status == Status::Expired);
        tap(&mut rt, "sign-in-button");
        wait(&mut rt, |s| {
            matches!(s.status, Status::SigningIn { url: Some(_) })
        });
        std::fs::write(home.join("browser-done"), "").unwrap();
        wait(&mut rt, |s| matches!(s.status, Status::Ready(_)));
        rt.state.draft.text = "hello again".into();
        tap(&mut rt, "send");
        wait(&mut rt, replied);
        assert_eq!(
            rt.state.ws.chat.last().unwrap().text,
            "You said: hello again"
        );
    }

    #[test]
    fn an_api_key_sign_in_is_blocked_and_offers_the_subscription_sign_in() {
        let mut rt = runtime_with(fake("console", &[]));
        wait(&mut rt, settled);
        assert_eq!(
            rt.state.status,
            Status::NotSubscription {
                method: "api_key".into()
            }
        );
        assert!(rt.find("sign-in-button").is_some());
        assert!(rt.find("chat-input").is_none());
    }

    #[test]
    fn api_billing_credentials_never_reach_claude() {
        let mut rt = runtime_with(fake(
            "signed-in",
            &[
                ("ANTHROPIC_API_KEY", "sk-test"),
                ("ANTHROPIC_AUTH_TOKEN", "token-test"),
            ],
        ));
        wait(&mut rt, settled);
        rt.state.draft.text = "envcheck".into();
        tap(&mut rt, "send");
        wait(&mut rt, replied);
        assert_eq!(
            rt.state.ws.chat.last().unwrap().text,
            "You said: api-key: token:"
        );
    }

    #[test]
    fn a_note_from_claudes_tool_lands_in_for_you_to_do() {
        let mut rt = runtime_with(fake("signed-in", &[]));
        wait(&mut rt, settled);
        rt.state.draft.text = "note: Renew the domain".into();
        tap(&mut rt, "send");
        wait(&mut rt, replied);
        assert_eq!(
            rt.state.ws.chat.last().unwrap().text,
            "You said: I noted it for you: Renew the domain"
        );
        let note = rt.state.ws.for_you().next().expect("a note").clone();
        assert_eq!(note.title, "Renew the domain");
        assert!(rt.find(&format!("task-{}", note.id)).is_some());
        // Marking it done tells the chief of staff which note it was.
        let chat = rt.state.ws.chat.len();
        tap(&mut rt, &format!("done-{}", note.id));
        wait(&mut rt, |s| replied(s) && s.ws.chat.len() > chat);
        assert_eq!(
            rt.state.ws.chat.last().unwrap().text,
            "You said: I have done this: Renew the domain"
        );
    }

    /// Sends "run: git push", which makes the fake ask the hook, and answers the question.
    fn answer_tool_use(choice: usize) -> Runtime<App> {
        let mut rt = runtime_with(fake("signed-in", &[]));
        wait(&mut rt, settled);
        rt.state.draft.text = "run: git push".into();
        tap(&mut rt, "send");
        wait(&mut rt, |s| s.ws.needs_input().next().is_some());
        let item = rt.state.ws.needs_input().next().unwrap().clone();
        assert_eq!(item.title, "Allow Bash?");
        assert_eq!(item.detail, "command: git push");
        let chat = rt.state.ws.chat.len();
        tap(&mut rt, &format!("answer-{}-{choice}", item.id));
        wait(&mut rt, replied);
        // The answer went to the hook, not into the chat.
        assert_eq!(rt.state.ws.chat.len(), chat + 1);
        assert!(rt.state.ws.needs_input().next().is_none());
        rt
    }

    #[test]
    fn allowing_a_tool_use_answers_claudes_hook() {
        let rt = answer_tool_use(0);
        assert_eq!(
            rt.state.ws.chat.last().unwrap().text,
            "You said: Bash was allowed to run git push"
        );
    }

    /// Sends `message`, which makes the fake ask the hook, and returns the question shown.
    fn ask_about(message: &str) -> (Runtime<App>, Item) {
        let mut rt = runtime_with(fake("signed-in", &[]));
        wait(&mut rt, settled);
        rt.state.draft.text = message.into();
        tap(&mut rt, "send");
        wait(&mut rt, |s| s.ws.needs_input().next().is_some());
        let item = rt.state.ws.needs_input().next().unwrap().clone();
        (rt, item)
    }

    #[test]
    fn the_question_shows_the_whole_command_not_a_preview() {
        let command = long_command();
        assert!(command.len() > 300);
        let (mut rt, item) = ask_about(&format!("run: {command}"));
        assert_eq!(item.detail, format!("command: {command}"));
        assert!(item.detail.ends_with("; rm -rf ~"));
        assert!(rt.find(&format!("input-{}-detail", item.id)).is_some());
        tap(&mut rt, &format!("answer-{}-1", item.id));
        wait(&mut rt, replied);
    }

    #[test]
    fn the_question_for_a_write_shows_the_path_and_the_content() {
        let (mut rt, item) = ask_about("write: /tmp/notes.txt hello there");
        assert_eq!(item.title, "Allow Write?");
        assert!(item.detail.contains("file_path: /tmp/notes.txt"));
        assert!(item.detail.contains("content: hello there"));
        tap(&mut rt, &format!("answer-{}-0", item.id));
        wait(&mut rt, replied);
        assert_eq!(
            rt.state.ws.chat.last().unwrap().text,
            "You said: Write was allowed to run /tmp/notes.txt"
        );
    }

    #[test]
    fn an_input_too_large_to_show_is_denied_without_asking() {
        let mut rt = runtime_with(fake("signed-in", &[]));
        wait(&mut rt, settled);
        rt.state.draft.text = "big:".into();
        tap(&mut rt, "send");
        wait(&mut rt, replied);
        assert_eq!(
            rt.state.ws.chat.last().unwrap().text,
            "You said: Bash was denied: too large to review"
        );
        assert!(rt.state.ws.needs_input().next().is_none());
    }

    #[test]
    fn a_question_claude_code_stopped_waiting_on_leaves_the_panel() {
        let (mut rt, item) = ask_about("abandon: ls");
        assert_eq!(item.title, "Allow Bash?");
        wait(&mut rt, |s| s.ws.needs_input().next().is_none());
        wait(&mut rt, replied);
        assert_eq!(
            rt.state.ws.chat.last().unwrap().text,
            "You said: the hook did not answer, so Bash was blocked"
        );
        // Nothing is left to click, and no answer went anywhere.
        assert!(rt.find(&format!("answer-{}-0", item.id)).is_none());
    }

    #[test]
    fn denying_a_tool_use_answers_claudes_hook() {
        let rt = answer_tool_use(1);
        assert_eq!(
            rt.state.ws.chat.last().unwrap().text,
            "You said: Bash was denied: git push"
        );
    }

    /// One tiny message on the captain's real Claude Code and the cheapest model, to check that
    /// the installed `claude` accepts the bridge's flags and calls the tool and the hook. It uses
    /// the subscription, so it only runs on request: `cargo test real_claude -- --ignored`.
    #[test]
    #[ignore = "uses the real Claude account"]
    fn real_claude_calls_the_tool_through_the_hook() {
        let claude = Claude::new("claude").env("ANTHROPIC_MODEL", "haiku");
        let mut rt = runtime_with(App::new(Box::new(claude)));
        wait(&mut rt, settled);
        assert!(matches!(rt.state.status, Status::Ready(_)), "not signed in");
        rt.state.draft.text = "Call the app_note tool with the text \"bridge check\", then reply \
                               with only the word done."
            .into();
        tap(&mut rt, "send");
        let deadline = Instant::now() + Duration::from_secs(120);
        while !replied(&rt.state) {
            assert!(Instant::now() < deadline, "no reply");
            // Loading the deferred MCP tool is the only other tool use this needs.
            let question = rt.state.ws.needs_input().next().cloned();
            if let Some(item) = question {
                eprintln!("hook asked: {} {}", item.title, item.detail);
                let choice = if item.title == "Allow ToolSearch?" {
                    0
                } else {
                    1
                };
                tap(&mut rt, &format!("answer-{}-{choice}", item.id));
            }
            std::thread::sleep(Duration::from_millis(50));
            rt.frame(1.0 / 60.0);
        }
        eprintln!("chat: {:?}", rt.state.ws.chat);
        let note = rt.state.ws.for_you().next().expect("app_note added a note");
        assert_eq!(note.title, "bridge check");
    }

    #[test]
    fn a_missing_claude_says_so() {
        let mut rt = runtime_with(fake_program("/nonexistent/claude", "signed-out", &[]));
        wait(&mut rt, settled);
        assert_eq!(rt.state.status, Status::CliMissing);
        assert!(rt.find("recheck").is_some());
        assert!(rt.find("sign-in-button").is_none());
    }

    #[test]
    fn a_broken_claude_shows_its_own_error() {
        let mut rt = runtime_with(fake("broken", &[]));
        wait(&mut rt, settled);
        assert_eq!(
            rt.state.status,
            Status::Failed("fake claude: the settings file is corrupt".into())
        );
        assert!(rt.find("status-detail").is_some());
    }
}
