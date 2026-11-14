//! The chief of staff on the captain's own Claude Code. The app runs the `claude` command and
//! nothing else: `claude auth status` says whether it is signed in, `claude auth login` signs in
//! (Claude Code opens the browser and keeps the sign-in), and a long-lived
//! `claude -p --input-format stream-json --output-format stream-json` session carries the chat.
//! The app never reads, stores or forwards a credential; it only reads the fields `claude`
//! prints about the sign-in. The session calls back into the app through a [`Bridge`]: one tool
//! and hooks, passed as flags to that session only. The hooks apply firstmate's rules
//! ([`crate::guard`]), the captain's tool-approval settings ([`crate::approval`]) and the
//! visual-replies mode ([`crate::visual`]).

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use rune::Waker;
use serde_json::{Value, json};

use crate::agent::{ChiefOfStaff, Event};
use crate::bridge::{Bridge, Call, Policies};
use crate::guard::Home;
use crate::model::{Action, Id, Item, ItemKind, Plan, Status, Workspace};
use crate::tools::{self, ToolUse};
use crate::ui;

/// How long `claude auth status` may take before the app reports it as not answering.
const CHECK_TIMEOUT: Duration = Duration::from_secs(20);
/// How often the app asks `claude auth status` again while a sign-in runs.
const LOGIN_POLL: Duration = Duration::from_secs(2);
/// The answers to a tool-use question; the first allows it.
const ALLOW: &str = "Allow";
const DENY: &str = "Deny";

pub struct Claude {
    program: PathBuf,
    env: Vec<(String, String)>,
    workplace: Option<Workplace>,
    outbox: Outbox,
    inbox: Receiver<Msg>,
    status: Status,
    checking: bool,
    /// A message was rejected for its sign-in; the next check decides Expired or SignedOut.
    rejected: bool,
    /// Why the last sign-in stopped, shown if the check after it finds no sign-in.
    login_problem: Option<String>,
    login: Option<Login>,
    session: Option<Session>,
    /// The reply in progress, built from the session's text deltas.
    reply: String,
    /// The turn has shown some of its text, so its `result` record's text is not new.
    replied: bool,
    /// The view tool use whose input is streaming: its content block index, id and the JSON
    /// so far.
    streaming: Option<(u64, String, String)>,
    /// Events for the next [`ChiefOfStaff::events`].
    out: Vec<Event>,
    next_id: u64,
    next_item: Id,
    /// Tool uses waiting on the captain, by the item that asks about them.
    approvals: HashMap<Id, Approval>,
    /// Notes `app_note` added, by item, so marking one done can say which.
    notes: HashMap<Id, String>,
    /// The settings the session's hooks decide with.
    policies: Policies,
    /// Messages written to the session that Claude Code has not taken in yet. It echoes each one
    /// back as it takes it in (`--replay-user-messages`): at the start of a turn, or at the next
    /// step of the turn that is running when the message arrived mid-turn.
    unread: usize,
    /// A turn is running: from a message or the session's own start of one to its `result`.
    turn: bool,
    /// What [`Event::Working`] last said.
    working: bool,
}

/// Where a firstmate chief of staff runs: the managed clone is the working directory, so
/// Claude Code loads its `AGENTS.md`, skills and hooks, and `home` is the `FM_HOME` its scripts
/// keep their records in.
#[derive(Clone)]
struct Workplace {
    clone: PathBuf,
    home: PathBuf,
}

/// A tool use waiting on the captain.
struct Approval {
    ticket: u64,
    answer: Sender<bool>,
}

impl Claude {
    /// Runs `program` (normally `claude`, found on PATH).
    pub fn new(program: impl Into<PathBuf>) -> Self {
        let (tx, inbox) = channel();
        Claude {
            program: program.into(),
            env: Vec::new(),
            workplace: None,
            outbox: Outbox {
                tx,
                wake: Arc::new(OnceLock::new()),
            },
            inbox,
            status: Status::Checking,
            checking: false,
            rejected: false,
            login_problem: None,
            login: None,
            session: None,
            reply: String::new(),
            replied: false,
            streaming: None,
            out: Vec::new(),
            next_id: 0,
            next_item: 0,
            approvals: HashMap::new(),
            notes: HashMap::new(),
            // Asks about everything until the app hands over the captain's settings.
            policies: Policies::ask_every_time(),
            unread: 0,
            turn: false,
            working: false,
        }
    }

    /// Adds an environment variable for every `claude` the app starts.
    pub fn env(mut self, key: &str, value: impl Into<String>) -> Self {
        self.env.push((key.to_string(), value.into()));
        self
    }

    /// Runs the chat as firstmate: in the managed clone at `clone`, with `home` as firstmate's
    /// operational home.
    pub fn in_firstmate(mut self, clone: impl Into<PathBuf>, home: impl Into<PathBuf>) -> Self {
        self.workplace = Some(Workplace {
            clone: clone.into(),
            home: home.into(),
        });
        self
    }

    fn id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    fn item_id(&mut self) -> Id {
        self.next_item += 1;
        self.next_item
    }

    /// Stops the chat session. Its unanswered tool-use questions leave the panel and are denied.
    fn end_session(&mut self) {
        self.session = None;
        for (id, _) in self.approvals.drain() {
            self.out.push(Event::Withdrawn(id));
        }
        self.unread = 0;
        self.turn = false;
        self.report_working();
    }

    /// Reports whether the chief of staff is working, when that changed.
    fn report_working(&mut self) {
        let working = self.turn || self.unread > 0;
        if working != self.working {
            self.working = working;
            self.out.push(Event::Working(working));
        }
    }

    fn check(&mut self) {
        if self.checking {
            return;
        }
        self.checking = true;
        let (program, env, outbox) = (self.program.clone(), self.env.clone(), self.outbox.clone());
        thread::spawn(move || outbox.send(Msg::Checked(auth_status(&program, &env))));
    }

    fn set_status(&mut self, status: Status) {
        if status != self.status {
            if !matches!(status, Status::Ready(_)) {
                self.end_session();
            }
            self.status = status.clone();
            self.out.push(Event::Status(status));
        }
    }

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Checked(status) => {
                self.checking = false;
                let rejected = std::mem::take(&mut self.rejected);
                let status = match status {
                    Status::Ready(_) if rejected => Status::Expired,
                    Status::SignedOut { problem: None } => Status::SignedOut {
                        problem: self.login_problem.take(),
                    },
                    other => other,
                };
                // A sign-in in progress decides for itself when it is finished.
                if self.login.is_none() {
                    self.set_status(status);
                }
            }
            Msg::LoginUrl(id, url) if self.login_is(id) => {
                self.set_status(Status::SigningIn { url: Some(url) })
            }
            Msg::LoginSignedIn(id, status) if self.login_is(id) => {
                self.login = None;
                self.set_status(status);
            }
            Msg::LoginEnded(id, problem) if self.login_is(id) => {
                self.login = None;
                self.login_problem = problem;
                self.set_status(Status::Checking);
                self.check();
            }
            Msg::Record(id, record) if self.session_is(id) => self.record(&record),
            Msg::Call(id, call) if self.session_is(id) => self.call(call),
            Msg::SessionEnded(id, stderr) if self.session_is(id) => {
                self.end_session();
                self.finish_reply();
                self.replied = false;
                let detail = stderr.trim();
                self.out.push(Event::Problem(if detail.is_empty() {
                    "Claude Code stopped unexpectedly.".to_string()
                } else {
                    format!("Claude Code stopped: {detail}")
                }));
                self.check();
            }
            // From a sign-in or session the app already stopped.
            _ => {}
        }
    }

    /// Something the session asked of the app through the bridge.
    fn call(&mut self, call: Call) {
        if let Call::Expired(ticket) = call {
            if let Some(id) = self
                .approvals
                .iter()
                .find_map(|(id, a)| (a.ticket == ticket).then_some(*id))
            {
                self.approvals.remove(&id);
                self.out.push(Event::Withdrawn(id));
            }
            return;
        }
        if let Call::Logged(entry) = call {
            self.out.push(Event::Activity(entry));
            return;
        }
        if let Call::AddMate(scope) = call {
            self.out.push(Event::AddMate(scope));
            return;
        }
        // The reply so far ends here, so the captain sees it was sent back before the view.
        if let Call::Nudged(why) = call {
            self.finish_reply();
            self.out.push(Event::Notice(format!(
                "Velvet Hammer asked the chief of staff to show this as a view: the reply {why}."
            )));
            return;
        }
        // The wake that brought this call makes the app look for the next reminder due.
        if let Call::Reminded = call {
            return;
        }
        let id = self.item_id();
        let item = match call {
            Call::Expired(_)
            | Call::Logged(_)
            | Call::Nudged(_)
            | Call::AddMate(_)
            | Call::Reminded => return,
            Call::Note(text) => {
                self.notes.insert(id, text.clone());
                Item {
                    id,
                    kind: ItemKind::ForYou,
                    title: text,
                    detail: "Noted by your chief of staff.".into(),
                    agent: None,
                }
            }
            Call::Approve {
                ticket,
                title,
                input,
                answer,
                ..
            } => {
                self.approvals.insert(id, Approval { ticket, answer });
                Item {
                    id,
                    kind: ItemKind::NeedsInput {
                        options: vec![ALLOW.into(), DENY.into()],
                    },
                    title,
                    detail: input,
                    agent: None,
                }
            }
        };
        self.out.push(Event::Item(item));
    }

    fn login_is(&self, id: u64) -> bool {
        self.login.as_ref().is_some_and(|l| l.id == id)
    }

    fn session_is(&self, id: u64) -> bool {
        self.session.as_ref().is_some_and(|s| s.id == id)
    }

    /// One stream-json record from the chat session. A subagent's own records (those with a
    /// `parent_tool_use_id`) are its work, not the chief of staff's tool uses, and are left out.
    fn record(&mut self, record: &Value) {
        let subagent = !record["parent_tool_use_id"].is_null();
        match record["type"].as_str() {
            // One of the captain's messages, echoed as Claude Code takes it in.
            Some("user") if record["isReplay"] == true => {
                self.unread = self.unread.saturating_sub(1);
                self.turn = true;
            }
            // A turn the session started on its own, such as one a hook woke, is work too.
            Some("stream_event" | "assistant") => self.turn = true,
            _ => {}
        }
        match record["type"].as_str() {
            Some("user") if record["isReplay"] == true => {}
            Some("stream_event") => {
                let event = &record["event"];
                match event["type"].as_str() {
                    Some("message_start") if !self.reply.is_empty() => self.reply.push_str("\n\n"),
                    // A view tool use shows as soon as it starts, and fills in as its input
                    // streams, so the captain sees the view build up.
                    Some("content_block_start")
                        if !subagent
                            && event["content_block"]["type"] == "tool_use"
                            && event["content_block"]["name"] == ui::tool_name().as_str() =>
                    {
                        let block = &event["content_block"];
                        let id = block["id"].as_str().unwrap_or_default().to_string();
                        self.finish_reply();
                        self.out.push(Event::Tool(ToolUse {
                            id: id.clone(),
                            name: ui::tool_name(),
                            input: json!({}),
                            result: None,
                        }));
                        let index = event["index"].as_u64().unwrap_or_default();
                        self.streaming = Some((index, id, String::new()));
                    }
                    Some("content_block_delta") if event["delta"]["type"] == "input_json_delta" => {
                        let Some((index, id, json)) = &mut self.streaming else {
                            return;
                        };
                        // Past the limit the view is refused anyway, once the input is whole.
                        if event["index"].as_u64() == Some(*index) && json.len() <= ui::MAX_BYTES {
                            json.push_str(
                                event["delta"]["partial_json"].as_str().unwrap_or_default(),
                            );
                            self.out.push(Event::ToolInput {
                                id: id.clone(),
                                input: ui::partial(json),
                            });
                        }
                    }
                    Some("content_block_stop") => self.streaming = None,
                    Some("content_block_delta") if event["delta"]["type"] == "text_delta" => {
                        self.replied = true;
                        self.reply
                            .push_str(event["delta"]["text"].as_str().unwrap_or_default());
                        self.out.push(Event::Reply {
                            text: self.reply.trim().to_string(),
                            done: false,
                        });
                    }
                    _ => {}
                }
            }
            Some("assistant") if record["error"] == "authentication_failed" => {
                self.rejected = true;
            }
            // A tool use ends the text before it, so the chat shows them in the order they
            // happened; the text after it starts a new message.
            Some("assistant") if !subagent => {
                for tool in tools::uses(&record["message"]["content"]) {
                    self.finish_reply();
                    self.out.push(Event::Tool(tool));
                }
            }
            Some("user") if !subagent => {
                for (id, result) in tools::results(&record["message"]["content"]) {
                    self.out.push(Event::ToolResult { id, result });
                }
            }
            Some("result") => {
                let failed = record["is_error"] == true || record["subtype"] != "success";
                if self.rejected {
                    self.finish_reply();
                    self.out.push(Event::Problem(
                        "Anthropic did not accept Claude Code's sign-in for this message.".into(),
                    ));
                    self.set_status(Status::Checking);
                    self.check();
                } else if failed || record["api_error_status"] == 403 {
                    self.finish_reply();
                    self.out
                        .push(Event::Problem(result_text(record).unwrap_or_else(|| {
                            format!(
                                "Claude Code reported an error ({}).",
                                record["subtype"].as_str().unwrap_or("unknown")
                            )
                        })));
                } else {
                    if !self.replied {
                        self.reply = result_text(record).unwrap_or_default();
                    }
                    self.finish_reply();
                }
                self.replied = false;
                self.turn = false;
            }
            _ => {}
        }
        self.report_working();
    }

    fn finish_reply(&mut self) {
        let text = std::mem::take(&mut self.reply).trim().to_string();
        if !text.is_empty() {
            self.out.push(Event::Reply { text, done: true });
        }
    }

    fn say(&mut self, text: &str) -> Result<(), String> {
        if self.session.is_none() {
            let id = self.id();
            self.session = Some(
                Session::start(
                    id,
                    &self.program,
                    &self.env,
                    self.workplace.as_ref(),
                    &self.policies,
                    &self.outbox,
                )
                .map_err(|e| format!("Could not start Claude Code: {e}"))?,
            );
        }
        let stdin = &mut self.session.as_mut().expect("started above").stdin;
        let stdin = stdin.as_mut().expect("open until the session is dropped");
        let line = json!({"type": "user", "message": {"role": "user", "content": text}});
        writeln!(stdin, "{line}")
            .and_then(|()| stdin.flush())
            .map_err(|e| format!("Could not send the message to Claude Code: {e}"))?;
        self.unread += 1;
        self.report_working();
        Ok(())
    }
}

impl ChiefOfStaff for Claude {
    fn open(&mut self) -> Workspace {
        self.check();
        Workspace::default()
    }

    fn connect(&mut self, wake: Waker) {
        let _ = self.outbox.wake.set(wake);
    }

    fn workplace(&self) -> Option<&Path> {
        self.workplace.as_ref().map(|w| w.clone.as_path())
    }

    fn approve_with(&mut self, policies: Policies) {
        self.policies = policies;
    }

    fn act(&mut self, ws: &Workspace, action: &Action) {
        // A tool-use answer goes back to the hook that asked, not into the chat.
        if let Action::Answer { item, choice } = action
            && let Some(approval) = self.approvals.remove(item)
        {
            if approval.answer.send(choice == ALLOW).is_err() {
                self.out.push(Event::Problem(
                    "That question had already expired, so your answer was not used.".into(),
                ));
            }
            return;
        }
        let text = match action {
            // The captain's words, with any message they link to quoted (see crate::links).
            Action::Say(text) => crate::links::outgoing(ws, text),
            Action::Answer { item, choice } => format!("My answer to item {item}: {choice}"),
            Action::Done { item } => match self.notes.remove(item) {
                Some(note) => format!("I have done this: {note}"),
                None => format!("I have done item {item}."),
            },
            Action::Ui(ui) => crate::links::outgoing(ws, &ui.message()),
        };
        if let Err(problem) = self.say(&text) {
            self.end_session();
            self.out.push(Event::Problem(problem));
        }
    }

    fn events(&mut self) -> Vec<Event> {
        while let Ok(msg) = self.inbox.try_recv() {
            self.handle(msg);
        }
        std::mem::take(&mut self.out)
    }

    fn sign_in(&mut self) {
        if self.login.is_some() {
            return;
        }
        let id = self.id();
        match Login::start(id, &self.program, &self.env, &self.outbox) {
            Ok(login) => {
                self.login = Some(login);
                self.login_problem = None;
                self.set_status(Status::SigningIn { url: None });
            }
            Err(e) => self.set_status(Status::SignedOut {
                problem: Some(format!("Could not start `claude auth login`: {e}")),
            }),
        }
    }

    fn cancel_sign_in(&mut self) {
        if self.login.take().is_some() {
            self.set_status(Status::SignedOut { problem: None });
        }
    }

    fn recheck(&mut self) {
        if self.login.is_none() {
            self.set_status(Status::Checking);
            self.check();
        }
    }
}

/// What the background threads report to the UI thread. Ids tie a report to the sign-in or
/// session that made it, so reports from one the app already stopped are dropped.
enum Msg {
    Checked(Status),
    LoginUrl(u64, String),
    LoginSignedIn(u64, Status),
    LoginEnded(u64, Option<String>),
    Record(u64, Value),
    Call(u64, Call),
    SessionEnded(u64, String),
}

#[derive(Clone)]
struct Outbox {
    tx: Sender<Msg>,
    wake: Arc<OnceLock<Waker>>,
}

impl Outbox {
    fn send(&self, msg: Msg) {
        if self.tx.send(msg).is_ok()
            && let Some(wake) = self.wake.get()
        {
            wake();
        }
    }
}

/// Asks `claude auth status --json` whether Claude Code is signed in. Only `loggedIn`,
/// `authMethod` and `subscriptionType` are read; the rest, such as the account email, is not.
fn auth_status(program: &PathBuf, env: &[(String, String)]) -> Status {
    let output = match run(program, &["auth", "status", "--json"], env, CHECK_TIMEOUT) {
        Ok(output) => output,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Status::CliMissing,
        Err(e) if e.kind() == io::ErrorKind::TimedOut => {
            return Status::Failed(format!(
                "`claude auth status` did not answer within {} seconds.",
                CHECK_TIMEOUT.as_secs()
            ));
        }
        Err(e) => return Status::Failed(format!("Could not run `claude auth status`: {e}")),
    };
    let Ok(status) = serde_json::from_slice::<Value>(&output.stdout) else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Status::Failed(if stderr.is_empty() {
            "`claude auth status` printed something Velvet Hammer could not read.".into()
        } else {
            stderr
        });
    };
    if status["loggedIn"] != true {
        return Status::SignedOut { problem: None };
    }
    let method = status["authMethod"].as_str().unwrap_or("an unknown method");
    if method != "claude.ai" {
        return Status::NotSubscription {
            method: method.to_string(),
        };
    }
    Status::Ready(Plan::Subscription(
        status["subscriptionType"]
            .as_str()
            .unwrap_or("Claude")
            .to_string(),
    ))
}

/// Credentials that make Claude Code bill an API account instead of the subscription.
const BILLING_ENV: [&str; 2] = ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"];

fn subscription_only(cmd: &mut Command) -> &mut Command {
    for key in BILLING_ENV {
        cmd.env_remove(key);
    }
    cmd
}

/// Runs a short command to completion, killing it after `timeout`.
fn run(
    program: &PathBuf,
    args: &[&str],
    env: &[(String, String)],
    timeout: Duration,
) -> io::Result<Output> {
    let mut child = subscription_only(
        Command::new(program)
            .args(args)
            .envs(env.iter().map(|(k, v)| (k, v))),
    )
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()?;
    let deadline = Instant::now() + timeout;
    while child.try_wait()?.is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::ErrorKind::TimedOut.into());
        }
        thread::sleep(Duration::from_millis(25));
    }
    child.wait_with_output()
}

/// `claude auth login --claudeai`, run in a pseudo-terminal because it is an interactive
/// command. Claude Code opens the browser and keeps the sign-in; the app reads its output only
/// for the sign-in page's address, and asks `claude auth status` to learn when it is done.
struct Login {
    id: u64,
    killer: Box<dyn ChildKiller + Send + Sync>,
    stopped: Arc<AtomicBool>,
    // Closing the terminal hangs up on `claude`, so it lives as long as the sign-in.
    _master: Box<dyn MasterPty + Send>,
}

impl Login {
    fn start(
        id: u64,
        program: &PathBuf,
        env: &[(String, String)],
        outbox: &Outbox,
    ) -> Result<Login, String> {
        let pty = native_pty_system()
            .openpty(PtySize {
                rows: 40,
                // Wide enough that the sign-in address is printed on one line.
                cols: 2000,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| e.to_string())?;
        let mut cmd = CommandBuilder::new(program);
        cmd.args(["auth", "login", "--claudeai"]);
        for (k, v) in env {
            cmd.env(k, v);
        }
        for key in BILLING_ENV {
            cmd.env_remove(key);
        }
        let mut child = pty.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
        drop(pty.slave);
        let killer = child.clone_killer();
        let reader = pty.master.try_clone_reader().map_err(|e| e.to_string())?;
        let stopped = Arc::new(AtomicBool::new(false));

        let out = outbox.clone();
        thread::spawn(move || {
            let last = watch_login_output(reader, |url| out.send(Msg::LoginUrl(id, url)));
            let ok = child.wait().is_ok_and(|status| status.success());
            let ended = if ok { "" } else { " before it finished" };
            out.send(Msg::LoginEnded(
                id,
                Some(match last {
                    Some(line) => format!("Claude Code's sign-in stopped{ended}. It said: {line}"),
                    None => format!("Claude Code's sign-in stopped{ended}."),
                }),
            ));
        });

        let (program, env, out, done) = (
            program.clone(),
            env.to_vec(),
            outbox.clone(),
            stopped.clone(),
        );
        thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                thread::sleep(LOGIN_POLL);
                let status = auth_status(&program, &env);
                if matches!(status, Status::Ready(_)) && !done.load(Ordering::Relaxed) {
                    out.send(Msg::LoginSignedIn(id, status));
                    return;
                }
            }
        });

        Ok(Login {
            id,
            killer,
            stopped,
            _master: pty.master,
        })
    }
}

impl Drop for Login {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        let _ = self.killer.kill();
    }
}

/// Reads the sign-in's terminal output, reporting the first `https://` address once, and
/// returns the last line it printed. Nothing it reads is logged.
fn watch_login_output(
    mut reader: Box<dyn Read + Send>,
    mut on_url: impl FnMut(String),
) -> Option<String> {
    let mut pending = Vec::new();
    let mut buf = [0u8; 4096];
    let mut url_seen = false;
    let mut last = None;
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        pending.extend_from_slice(&buf[..n]);
        while let Some(end) = pending.iter().position(|&b| b == b'\n') {
            let raw: Vec<u8> = pending.drain(..=end).collect();
            let line = plain_text(&String::from_utf8_lossy(&raw));
            if !url_seen && let Some(url) = first_https_url(&line) {
                url_seen = true;
                on_url(url);
            } else if !line.is_empty() {
                last = Some(line);
            }
        }
    }
    last
}

/// `text` without terminal escape sequences or control characters, trimmed.
fn plain_text(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            match chars.next() {
                // CSI: parameters, then one final byte in @..~.
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                // OSC: up to BEL or ESC \.
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' || (c == '\u{1b}' && chars.next_if_eq(&'\\').is_some()) {
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else if !c.is_control() {
            out.push(c);
        }
    }
    out.trim().to_string()
}

fn first_https_url(line: &str) -> Option<String> {
    line.split_whitespace()
        .find(|word| word.starts_with("https://"))
        .map(str::to_string)
}

/// The long-lived chat: `claude -p` reading user messages and printing stream-json records,
/// and calling back into the app through its bridge.
struct Session {
    id: u64,
    // Both are taken when the session is dropped.
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    _bridge: Bridge,
}

impl Session {
    fn start(
        id: u64,
        program: &PathBuf,
        env: &[(String, String)],
        workplace: Option<&Workplace>,
        policies: &Policies,
        outbox: &Outbox,
    ) -> io::Result<Session> {
        let calls = outbox.clone();
        let home = workplace.map(|w| Home::new(&w.home));
        let bridge = Bridge::start(policies.clone(), home, move |call| {
            calls.send(Msg::Call(id, call))
        })?;
        let mut command = Command::new(program);
        if let Some(Workplace { clone, home }) = workplace {
            crate::firstmate::prepare_home(home)?;
            command.current_dir(clone).env("FM_HOME", home);
        }
        let mut child = subscription_only(
            command
                .args([
                    "-p",
                    "--input-format",
                    "stream-json",
                    "--output-format",
                    "stream-json",
                    "--verbose",
                    "--include-partial-messages",
                    "--replay-user-messages",
                    "--no-session-persistence",
                    "--mcp-config",
                    &bridge.mcp_config(),
                    "--settings",
                    &bridge.settings(),
                ])
                .envs(env.iter().map(|(k, v)| (k, v))),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
        let stdin = child.stdin.take().expect("piped");
        let stdout = child.stdout.take().expect("piped");
        let mut stderr = child.stderr.take().expect("piped");
        let stderr = thread::spawn(move || {
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text);
            text
        });
        let out = outbox.clone();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut line = Vec::new();
            while reader.read_until(b'\n', &mut line).is_ok_and(|n| n > 0) {
                if let Ok(record) = serde_json::from_slice::<Value>(&line) {
                    out.send(Msg::Record(id, record));
                }
                line.clear();
            }
            out.send(Msg::SessionEnded(id, stderr.join().unwrap_or_default()));
        });
        Ok(Session {
            id,
            child: Some(child),
            stdin: Some(stdin),
            _bridge: bridge,
        })
    }
}

/// How long a session may take to exit after its input closes before the app kills it.
const EXIT_GRACE: Duration = Duration::from_secs(5);

impl Drop for Session {
    /// Closes the session's input and, at once, asks `claude` to stop (SIGTERM on unix), so no
    /// session outlives its owner, even when the app quits before the reaper below runs. `claude`
    /// stops its hooks' processes (for firstmate, the watcher) on the way out, which a kill would
    /// orphan. The reaper kills it only if it has not exited after [`EXIT_GRACE`].
    fn drop(&mut self) {
        drop(self.stdin.take());
        let Some(mut child) = self.child.take() else {
            return;
        };
        #[cfg(unix)]
        // SAFETY: `kill` only signals the pid of a child this session has not yet reaped.
        unsafe {
            libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
        }
        #[cfg(not(unix))]
        let _ = child.kill();
        thread::spawn(move || {
            let deadline = Instant::now() + EXIT_GRACE;
            while matches!(child.try_wait(), Ok(None)) {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return;
                }
                thread::sleep(Duration::from_millis(25));
            }
        });
    }
}

/// The text of a `result` record: its `result`, or its `errors` joined.
fn result_text(record: &Value) -> Option<String> {
    if let Some(text) = record["result"].as_str().filter(|t| !t.trim().is_empty()) {
        return Some(text.trim().to_string());
    }
    let errors: Vec<&str> = record["errors"]
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    (!errors.is_empty()).then(|| errors.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_drops_terminal_escapes() {
        let line =
            "\u{1b}[2K\u{1b}[1G\u{1b}[36mhttps://claude.ai/oauth/authorize?a=1\u{1b}[39m\r\n";
        assert_eq!(plain_text(line), "https://claude.ai/oauth/authorize?a=1");
        let link = "\u{1b}]8;;https://x.test\u{7}Open\u{1b}]8;;\u{1b}\\ now";
        assert_eq!(plain_text(link), "Open now");
    }

    #[test]
    fn tool_uses_split_the_reply_and_a_subagents_are_left_out() {
        let mut claude = Claude::new("unused");
        let delta = |text: &str| {
            json!({"type": "stream_event", "event": {"type": "content_block_delta",
                "delta": {"type": "text_delta", "text": text}}})
        };
        let tool_use = |parent: Value| {
            json!({"type": "assistant", "parent_tool_use_id": parent, "message": {"content": [
                {"type": "tool_use", "id": "t1", "name": "Read", "input": {"file_path": "/a.png"}}]}})
        };
        let tool_result = |parent: Value| {
            json!({"type": "user", "parent_tool_use_id": parent, "message": {"content": [
                {"type": "tool_result", "tool_use_id": "t1", "content": "ok"}]}})
        };
        for record in [
            delta("Before."),
            tool_use(json!("toolu_parent")),
            tool_result(json!("toolu_parent")),
            tool_use(Value::Null),
            tool_result(Value::Null),
            json!({"type": "stream_event", "event": {"type": "message_start"}}),
            delta("After."),
            json!({"type": "result", "subtype": "success", "result": "After."}),
        ] {
            claude.record(&record);
        }
        let events = std::mem::take(&mut claude.out);
        let finished: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                Event::Reply { text, done: true } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(finished, ["Before.", "After."]);
        let tools: Vec<&Event> = events
            .iter()
            .filter(|e| matches!(e, Event::Tool(_) | Event::ToolResult { .. }))
            .collect();
        assert!(
            matches!(tools.as_slice(), [Event::Tool(t), Event::ToolResult { id, .. }]
            if t.name == "Read" && id == "t1")
        );
        // The tool use comes between the two replies.
        let at = |want: fn(&Event) -> bool| events.iter().position(want).unwrap();
        assert!(
            at(|e| matches!(e, Event::Reply { done: true, .. }))
                < at(|e| matches!(e, Event::Tool(_)))
        );
    }

    #[test]
    fn a_view_tool_use_shows_at_its_start_and_fills_in_as_its_input_streams() {
        let mut claude = Claude::new("unused");
        let name = ui::tool_name();
        let event = |event: Value| json!({"type": "stream_event", "event": event});
        let delta = |json: &str| {
            event(json!({"type": "content_block_delta", "index": 1,
                "delta": {"type": "input_json_delta", "partial_json": json}}))
        };
        for record in [
            event(json!({"type": "content_block_delta", "index": 0,
                "delta": {"type": "text_delta", "text": "Here it is."}})),
            event(json!({"type": "content_block_start", "index": 1,
                "content_block": {"type": "tool_use", "id": "v1", "name": name, "input": {}}})),
            delta(r#"{"root": "t", "elem"#),
            delta(r#"ents": {"t": {"type": "Text", "props": {"text": "Hi"}}}}"#),
            event(json!({"type": "content_block_stop", "index": 1})),
            delta("ignored"),
        ] {
            claude.record(&record);
        }
        let events: Vec<Event> = std::mem::take(&mut claude.out)
            .into_iter()
            .filter(|e| !matches!(e, Event::Working(_)))
            .collect();
        // The text before the view ends as it starts.
        assert!(matches!(&events[1], Event::Reply { text, done: true } if text == "Here it is."));
        assert!(matches!(&events[2], Event::Tool(t) if t.id == "v1" && t.name == name));
        let inputs: Vec<&Value> = events
            .iter()
            .filter_map(|e| match e {
                Event::ToolInput { id, input } if id == "v1" => Some(input),
                _ => None,
            })
            .collect();
        assert_eq!(
            inputs,
            [
                &json!({"root": "t"}),
                &json!({"root": "t", "elements":
            {"t": {"type": "Text", "props": {"text": "Hi"}}}})
            ]
        );
    }

    #[test]
    fn a_turn_that_only_used_tools_still_shows_its_result_text() {
        let mut claude = Claude::new("unused");
        claude.record(&json!({"type": "assistant", "message": {"content": [
            {"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "ls"}}]}}));
        claude.record(&json!({"type": "result", "subtype": "success", "result": "Listed."}));
        assert!(claude.out.iter().any(|e| *e
            == Event::Reply {
                text: "Listed.".into(),
                done: true
            }));
    }

    #[test]
    fn it_works_until_every_message_written_was_taken_in_and_answered() {
        let mut claude = Claude::new("unused");
        // Two messages written: the second arrived during the first's turn.
        claude.unread = 2;
        claude.report_working();
        let replay = |text: &str| {
            json!({"type": "user", "isReplay": true, "parent_tool_use_id": null,
                "message": {"role": "user", "content": text}})
        };
        let result = json!({"type": "result", "subtype": "success", "result": "ok"});
        claude.record(&replay("first"));
        claude.record(&result);
        // The second was held for the next turn, so the first turn's end is not the end.
        claude.record(&replay("second"));
        claude.record(&result);
        // A turn the session starts on its own is work too.
        claude.record(&json!({"type": "stream_event", "event": {"type": "message_start"}}));
        claude.record(&result);
        let working: Vec<bool> = claude
            .out
            .iter()
            .filter_map(|e| match e {
                Event::Working(w) => Some(*w),
                _ => None,
            })
            .collect();
        assert_eq!(working, [true, false, true, false]);
        assert!(
            !claude
                .out
                .iter()
                .any(|e| matches!(e, Event::Tool(_) | Event::ToolResult { .. })),
            "a replayed message is not a tool result"
        );
    }

    #[test]
    fn first_https_url_ignores_http() {
        assert_eq!(first_https_url("(http://localhost:1/cb) here"), None);
        assert_eq!(
            first_https_url("visit https://claude.ai/x?y=1 now").as_deref(),
            Some("https://claude.ai/x?y=1")
        );
    }
}
