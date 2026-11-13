//! The chief of staff on the captain's own Claude Code. The app runs the `claude` command and
//! nothing else: `claude auth status` says whether it is signed in, `claude auth login` signs in
//! (Claude Code opens the browser and keeps the sign-in), and a long-lived
//! `claude -p --input-format stream-json --output-format stream-json` session carries the chat.
//! The app never reads, stores or forwards a credential; it only reads the fields `claude`
//! prints about the sign-in.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::PathBuf;
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
use crate::model::{Action, Plan, Status, Workspace};

/// How long `claude auth status` may take before the app reports it as not answering.
const CHECK_TIMEOUT: Duration = Duration::from_secs(20);
/// How often the app asks `claude auth status` again while a sign-in runs.
const LOGIN_POLL: Duration = Duration::from_secs(2);

pub struct Claude {
    program: PathBuf,
    env: Vec<(String, String)>,
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
    /// Events for the next [`ChiefOfStaff::events`].
    out: Vec<Event>,
    next_id: u64,
}

impl Claude {
    /// Runs `program` (normally `claude`, found on PATH).
    pub fn new(program: impl Into<PathBuf>) -> Self {
        let (tx, inbox) = channel();
        Claude {
            program: program.into(),
            env: Vec::new(),
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
            out: Vec::new(),
            next_id: 0,
        }
    }

    /// Adds an environment variable for every `claude` the app starts.
    pub fn env(mut self, key: &str, value: impl Into<String>) -> Self {
        self.env.push((key.to_string(), value.into()));
        self
    }

    fn id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
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
                self.session = None;
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
            Msg::SessionEnded(id, stderr) if self.session_is(id) => {
                self.session = None;
                self.finish_reply();
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

    fn login_is(&self, id: u64) -> bool {
        self.login.as_ref().is_some_and(|l| l.id == id)
    }

    fn session_is(&self, id: u64) -> bool {
        self.session.as_ref().is_some_and(|s| s.id == id)
    }

    /// One stream-json record from the chat session.
    fn record(&mut self, record: &Value) {
        match record["type"].as_str() {
            Some("stream_event") => {
                let event = &record["event"];
                match event["type"].as_str() {
                    Some("message_start") if !self.reply.is_empty() => self.reply.push_str("\n\n"),
                    Some("content_block_delta") if event["delta"]["type"] == "text_delta" => {
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
                    if self.reply.trim().is_empty() {
                        self.reply = result_text(record).unwrap_or_default();
                    }
                    self.finish_reply();
                }
            }
            _ => {}
        }
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
                Session::start(id, &self.program, &self.env, &self.outbox)
                    .map_err(|e| format!("Could not start Claude Code: {e}"))?,
            );
        }
        let session = self.session.as_mut().expect("started above");
        let line = json!({"type": "user", "message": {"role": "user", "content": text}});
        writeln!(session.stdin, "{line}")
            .and_then(|()| session.stdin.flush())
            .map_err(|e| format!("Could not send the message to Claude Code: {e}"))
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

    fn act(&mut self, _ws: &Workspace, action: &Action) {
        let text = match action {
            Action::Say(text) => text.clone(),
            Action::Answer { item, choice } => format!("My answer to item {item}: {choice}"),
            Action::Done { item } => format!("I have done item {item}."),
        };
        if let Err(problem) = self.say(&text) {
            self.session = None;
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
    Status::Ready(if method == "claude.ai" {
        Plan::Subscription(
            status["subscriptionType"]
                .as_str()
                .unwrap_or("Claude")
                .to_string(),
        )
    } else {
        Plan::Other(method.to_string())
    })
}

/// Runs a short command to completion, killing it after `timeout`.
fn run(
    program: &PathBuf,
    args: &[&str],
    env: &[(String, String)],
    timeout: Duration,
) -> io::Result<Output> {
    let mut child = Command::new(program)
        .args(args)
        .envs(env.iter().map(|(k, v)| (k, v)))
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

/// The long-lived chat: `claude -p` reading user messages and printing stream-json records.
struct Session {
    id: u64,
    child: Child,
    stdin: ChildStdin,
}

impl Session {
    fn start(
        id: u64,
        program: &PathBuf,
        env: &[(String, String)],
        outbox: &Outbox,
    ) -> io::Result<Session> {
        let mut child = Command::new(program)
            .args([
                "-p",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages",
                "--no-session-persistence",
            ])
            .envs(env.iter().map(|(k, v)| (k, v)))
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
        Ok(Session { id, child, stdin })
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
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
    fn first_https_url_ignores_http() {
        assert_eq!(first_https_url("(http://localhost:1/cb) here"), None);
        assert_eq!(
            first_https_url("visit https://claude.ai/x?y=1 now").as_deref(),
            Some("https://claude.ai/x?y=1")
        );
    }
}
