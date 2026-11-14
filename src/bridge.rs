//! The extension seam between the chat session's `claude` and the app: a small HTTP server on
//! 127.0.0.1 that `claude` calls back into. It serves two things, both passed to that one session
//! as command-line flags so the captain's own Claude settings are never written:
//!
//! - an MCP server (`--mcp-config`) with four tools: `app_note`, which puts a note in the
//!   captain's For you to do panel, `app_ui`, which shows a json-render view in the chat
//!   (see [`crate::ui`]), and `app_remind` and `app_reminders`, which set and list the captain's
//!   reminders (see [`crate::reminders`]);
//! - a PreToolUse hook (`--settings`), which applies firstmate's rules (see [`crate::guard`])
//!   and the captain's tool-approval settings (see [`crate::approval`]) to each tool use: it
//!   allows it, denies it, or asks the captain to allow or deny it in the Needs your input panel.
//!   Every decision is reported for the activity log;
//! - a UserPromptSubmit hook and a Stop hook (also `--settings`), which steer the chief of staff
//!   towards views for the replies that read better as one (see [`crate::visual`]).
//!
//! Every request must carry the bearer token made for this server, so other programs and web
//! pages cannot reach it. A hook the server cannot answer blocks the tool use (`onFailure`).

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::approval::{self, Effect, Entry, Policy, Settings};
use crate::guard::{self, Home};
use crate::reminders::{self, Reminders};
use crate::store::Shared;
use crate::ui;
use crate::visual::{self, Turn};

/// The MCP server's name; Claude Code calls its tools `mcp__velvet-hammer__<tool>`.
pub const SERVER: &str = "velvet-hammer";
const NOTE_TOOL: &str = "app_note";
/// Claude Code's timeout for the hook, in seconds (its default).
const HOOK_TIMEOUT_SECS: u64 = 600;
/// Claude Code's timeout for the prompt and stop hooks, which answer at once.
const QUICK_TIMEOUT_SECS: u64 = 30;
/// How long the hook waits for the captain: a little less than Claude Code waits for the hook,
/// so an unanswered question becomes a deny the app chose rather than a timeout.
const APPROVAL_WAIT: Duration = Duration::from_secs(HOOK_TIMEOUT_SECS - 10);
/// The largest request body the server reads.
const MAX_BODY: usize = 1 << 20;
/// The most of a tool's input the captain is shown. A larger input is denied, never approved
/// from a partial view.
const MAX_REVIEW: usize = 64 * 1024;
/// How often a waiting hook checks that Claude Code is still waiting for it.
const POLL: Duration = Duration::from_millis(250);
static NEXT_TICKET: AtomicU64 = AtomicU64::new(1);

/// Something `claude` asked of the app.
pub enum Call {
    /// `app_note` was called with this text.
    Note(String),
    /// `claude` wants to use a tool; `title` is the question for the captain, such as "Allow
    /// Bash?", and `input` is all of what it would do, as text, after any firstmate rule's
    /// reason. Send true to allow it,
    /// false to deny it; dropping `answer` denies it.
    Approve {
        ticket: u64,
        title: String,
        input: String,
        answer: Sender<bool>,
    },
    /// The question with this ticket can no longer be answered: the wait ran out or Claude Code
    /// stopped waiting. Whatever answer comes later is not used.
    Expired(u64),
    /// The hook decided a tool use, for the activity log.
    Logged(Entry),
    /// The stop hook sent the reply back to be shown as a view, because it has this shape (in
    /// words, such as "lists 4 items").
    Nudged(String),
    /// `app_remind` set a reminder, so the app looks again for the next one due.
    Reminded,
}

/// The settings the hooks decide with, each read at every call so a change applies at once,
/// and the captain's reminders, which the reminder tools set and list.
#[derive(Clone)]
pub struct Policies {
    pub approval: Policy,
    pub visual: Shared<visual::Mode>,
    pub rules: Shared<guard::Settings>,
    pub reminders: Reminders,
}

impl Policies {
    /// Asks about every tool use and steers nothing: what a session runs on until the app hands
    /// over the captain's settings.
    pub fn ask_every_time() -> Policies {
        Policies {
            approval: Arc::new(RwLock::new(Settings::ask_every_time())),
            visual: Arc::new(RwLock::new(visual::Mode::Off)),
            rules: Arc::new(RwLock::new(guard::Settings::default())),
            reminders: Reminders::in_memory(reminders::system_clock()),
        }
    }
}

/// What the server's threads share.
struct Serving {
    policies: Policies,
    /// Firstmate's home, when the chief of staff runs as firstmate; its rules apply then.
    home: Option<Home>,
    /// The reply in progress, for the stop hook.
    turn: Mutex<Turn>,
    /// The session has been told about message links (see [`crate::links::INSTRUCTION`]).
    told_links: AtomicBool,
}

fn read<T: Clone>(shared: &Shared<T>) -> T {
    shared.read().unwrap_or_else(|e| e.into_inner()).clone()
}

pub struct Bridge {
    addr: SocketAddr,
    token: String,
    stopped: Arc<AtomicBool>,
}

impl Bridge {
    /// Starts serving on a free port of 127.0.0.1. The hooks decide with `policies` as they
    /// stand at each call, and firstmate's rules apply when `home` is firstmate's home.
    /// `on_call` runs on the server's threads.
    pub fn start(
        policies: Policies,
        home: Option<Home>,
        on_call: impl Fn(Call) + Send + Sync + 'static,
    ) -> io::Result<Bridge> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let addr = listener.local_addr()?;
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|e| io::Error::other(e.to_string()))?;
        let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let stopped = Arc::new(AtomicBool::new(false));
        let serving = Arc::new(Serving {
            policies,
            home,
            turn: Mutex::new(Turn::default()),
            told_links: AtomicBool::new(false),
        });
        let (expected, done, on_call) = (
            format!("Bearer {token}"),
            stopped.clone(),
            Arc::new(on_call),
        );
        thread::spawn(move || {
            for stream in listener.incoming() {
                if done.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                let (expected, serving, on_call) =
                    (expected.clone(), serving.clone(), on_call.clone());
                thread::spawn(move || {
                    let _ = serve(stream, &expected, &serving, &*on_call);
                });
            }
        });
        Ok(Bridge {
            addr,
            token,
            stopped,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    /// The `--mcp-config` value naming this server.
    pub fn mcp_config(&self) -> String {
        json!({"mcpServers": {SERVER: {
            "type": "http",
            "url": self.url("/mcp"),
            "headers": {"Authorization": format!("Bearer {}", self.token)},
        }}})
        .to_string()
    }

    /// The `--settings` value adding the hooks for this session only. A PreToolUse hook the app
    /// cannot answer blocks the tool use; a prompt or stop hook it cannot answer changes nothing.
    pub fn settings(&self) -> String {
        let hook = |path: &str, timeout: u64| {
            json!({
                "type": "http",
                "url": self.url(path),
                "headers": {"Authorization": format!("Bearer {}", self.token)},
                "timeout": timeout,
            })
        };
        let mut pre = hook("/hooks/pre-tool-use", HOOK_TIMEOUT_SECS);
        pre["onFailure"] = json!("block");
        json!({"hooks": {
            "PreToolUse": [{"matcher": "*", "hooks": [pre]}],
            "UserPromptSubmit": [{"hooks": [hook("/hooks/user-prompt-submit", QUICK_TIMEOUT_SECS)]}],
            "Stop": [{"hooks": [hook("/hooks/stop", QUICK_TIMEOUT_SECS)]}],
        }})
        .to_string()
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        // Wakes the accept loop so it sees `stopped`.
        let _ = TcpStream::connect(self.addr);
    }
}

/// What the server answers a request with.
enum Reply {
    Json(Value),
    /// 202 with no body, for an MCP notification.
    Accepted,
    Status(u16, &'static str),
}

/// Reads one request from `stream`, answers it and closes the connection.
fn serve(
    stream: TcpStream,
    expected: &str,
    serving: &Serving,
    on_call: &dyn Fn(Call),
) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut words = line.split_whitespace();
    let (method, path) = (
        words.next().unwrap_or_default().to_string(),
        words.next().unwrap_or_default().to_string(),
    );
    let (mut length, mut authorized, mut origin) = (0, false, false);
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 || line.trim().is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => length = value.parse().unwrap_or(usize::MAX),
            "authorization" => authorized = value == expected,
            "origin" => origin = true,
            _ => {}
        }
    }
    // Read the body before any reply: closing with unread bytes resets the connection and can
    // discard the response.
    let mut body = vec![0; if length <= MAX_BODY { length } else { 0 }];
    reader.read_exact(&mut body)?;
    let reply = if !authorized {
        Reply::Status(401, "Unauthorized")
    } else if origin {
        // Only `claude` calls this server; a request from a web page carries an Origin.
        Reply::Status(403, "Forbidden")
    } else if method != "POST" {
        Reply::Status(405, "Method Not Allowed")
    } else if length > MAX_BODY {
        Reply::Status(413, "Content Too Large")
    } else {
        match (path.as_str(), serde_json::from_slice::<Value>(&body)) {
            ("/mcp", Ok(request)) => {
                mcp(&request, serving, on_call).map_or(Reply::Accepted, Reply::Json)
            }
            ("/hooks/pre-tool-use", Ok(input)) => {
                Reply::Json(pre_tool_use(&input, &stream, serving, on_call))
            }
            ("/hooks/user-prompt-submit", Ok(_)) => Reply::Json(user_prompt_submit(serving)),
            ("/hooks/stop", Ok(input)) => Reply::Json(stop(&input, serving, on_call)),
            (
                "/mcp" | "/hooks/pre-tool-use" | "/hooks/user-prompt-submit" | "/hooks/stop",
                Err(_),
            ) => Reply::Status(400, "Bad Request"),
            _ => Reply::Status(404, "Not Found"),
        }
    };
    respond(stream, reply)
}

fn respond(mut stream: TcpStream, reply: Reply) -> io::Result<()> {
    let (status, reason, body) = match reply {
        Reply::Json(value) => (200, "OK", value.to_string()),
        Reply::Accepted => (202, "Accepted", String::new()),
        Reply::Status(status, reason) => (status, reason, String::new()),
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    )?;
    stream.flush()
}

/// Answers one MCP JSON-RPC message; None for a notification, which gets no answer.
fn mcp(request: &Value, serving: &Serving, on_call: &dyn Fn(Call)) -> Option<Value> {
    let id = request.get("id")?.clone();
    let params = &request["params"];
    let outcome = match request["method"].as_str().unwrap_or_default() {
        "initialize" => Ok(json!({
            "protocolVersion": params["protocolVersion"].as_str().unwrap_or("2025-06-18"),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": SERVER, "version": env!("CARGO_PKG_VERSION")},
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": [{
            "name": NOTE_TOOL,
            "description": "Add a note to the captain's \"For you to do\" panel in Velvet Hammer. \
                            Use it for a step only the captain can take.",
            "inputSchema": {
                "type": "object",
                "properties": {"text": {"type": "string", "description": "The note, in one short sentence."}},
                "required": ["text"],
            },
        }, {
            "name": ui::TOOL,
            "description": ui::description(),
            "inputSchema": ui::input_schema(),
        }, {
            "name": reminders::TOOL,
            "description": reminders::description(),
            "inputSchema": reminders::input_schema(),
        }, {
            "name": reminders::LIST_TOOL,
            "description": reminders::list_description(),
            "inputSchema": {"type": "object", "properties": {}},
        }]})),
        "tools/call" if params["name"] == NOTE_TOOL => {
            let text = params["arguments"]["text"]
                .as_str()
                .unwrap_or_default()
                .trim();
            Ok(if text.is_empty() {
                json!({"isError": true, "content": [{"type": "text", "text": "The note is empty."}]})
            } else {
                on_call(Call::Note(text.to_string()));
                json!({"content": [{"type": "text", "text": "Added to the captain's For you to do panel."}]})
            })
        }
        // The chat draws the view from the tool use itself; the call only checks it, so the
        // model learns what was refused or left out.
        "tools/call" if params["name"] == ui::TOOL => Ok(match ui::answer(&params["arguments"]) {
            Ok(text) => {
                lock(&serving.turn).viewed = true;
                json!({"content": [{"type": "text", "text": text}]})
            }
            Err(problem) => {
                json!({"isError": true, "content": [{"type": "text", "text": problem}]})
            }
        }),
        // The app works out the time, so the answer tells the model what was set.
        "tools/call" if params["name"] == reminders::TOOL => Ok(
            match serving.policies.reminders.answer(&params["arguments"]) {
                Ok(text) => {
                    on_call(Call::Reminded);
                    json!({"content": [{"type": "text", "text": text}]})
                }
                Err(problem) => {
                    json!({"isError": true, "content": [{"type": "text", "text": problem}]})
                }
            },
        ),
        "tools/call" if params["name"] == reminders::LIST_TOOL => Ok(
            json!({"content": [{"type": "text", "text": serving.policies.reminders.describe()}]}),
        ),
        "tools/call" => Err((-32602, "Unknown tool")),
        _ => Err((-32601, "Method not found")),
    };
    Some(match outcome {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err((code, message)) => {
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
        }
    })
}

/// Whether Claude Code has hung up on this request, which it does when it stops waiting.
fn hung_up(stream: &TcpStream) -> bool {
    if stream.set_nonblocking(true).is_err() {
        return false;
    }
    let gone = matches!(stream.peek(&mut [0u8; 1]), Ok(0));
    let _ = stream.set_nonblocking(false);
    gone
}

/// Waits for the captain's answer to `ticket`; None when the wait ended without one.
fn wait_for(
    stream: &TcpStream,
    ticket: u64,
    decision: &Receiver<bool>,
    on_call: &dyn Fn(Call),
) -> Option<bool> {
    let deadline = Instant::now() + APPROVAL_WAIT;
    loop {
        match decision.recv_timeout(POLL) {
            Ok(allow) => return Some(allow),
            Err(RecvTimeoutError::Disconnected) => return None,
            Err(RecvTimeoutError::Timeout) => {}
        }
        if Instant::now() >= deadline || hung_up(stream) {
            on_call(Call::Expired(ticket));
            return None;
        }
    }
}

fn lock(turn: &Mutex<Turn>) -> std::sync::MutexGuard<'_, Turn> {
    turn.lock().unwrap_or_else(|e| e.into_inner())
}

/// A message from the captain starts a new reply, and carries the standing instruction to use
/// views when the visual-replies mode asks for one. The session's first message also says how
/// message links work, once.
fn user_prompt_submit(serving: &Serving) -> Value {
    lock(&serving.turn).prompt();
    let links =
        (!serving.told_links.swap(true, Ordering::SeqCst)).then_some(crate::links::INSTRUCTION);
    let context: Vec<&str> = links
        .into_iter()
        .chain(read(&serving.policies.visual).instruction())
        .collect();
    if context.is_empty() {
        return json!({});
    }
    json!({"hookSpecificOutput": {
        "hookEventName": "UserPromptSubmit",
        "additionalContext": context.join("\n\n"),
    }})
}

/// Decides one Stop hook: a final reply of a qualifying shape with no view in its turn is sent
/// back once to be shown as one (see [`crate::visual`]). Anything else lets the turn end.
fn stop(input: &Value, serving: &Serving, on_call: &dyn Fn(Call)) -> Value {
    let mode = read(&serving.policies.visual);
    let reply = input["last_assistant_message"].as_str().unwrap_or_default();
    let continuing = input["stop_hook_active"] == true;
    let Some(shape) = lock(&serving.turn).stop(mode, reply, continuing) else {
        return json!({});
    };
    on_call(Call::Nudged(shape.describe()));
    json!({"hookSpecificOutput": {
        "hookEventName": "Stop",
        "additionalContext": visual::feedback(&shape),
    }})
}

/// Decides one PreToolUse hook. A firstmate rule decides first, when the chief of staff runs as
/// firstmate, and the captain's tool-approval settings decide the rest; they can still deny what
/// a rule would ask about. A tool use left to the captain waits in Needs your input with all of
/// its input shown or, when it is too large to show, is denied rather than approved from a
/// partial view.
fn pre_tool_use(
    input: &Value,
    stream: &TcpStream,
    serving: &Serving,
    on_call: &dyn Fn(Call),
) -> Value {
    let tool = input["tool_name"].as_str().unwrap_or_default();
    let tool_input = &input["tool_input"];
    let cwd = Path::new(input["cwd"].as_str().unwrap_or("/"));
    let verdict = read(&serving.policies.approval).decide(tool, tool_input);
    let review = approval::input_text(tool_input);
    let hit = serving
        .home
        .as_ref()
        .and_then(|home| read(&serving.policies.rules).check(tool, tool_input, cwd, home))
        .filter(|hit| hit.effect == Effect::Deny || verdict.effect != Effect::Deny);
    let (effect, why, title, shown) = match &hit {
        Some(hit) => (
            hit.effect,
            format!("by firstmate rule \u{201c}{}\u{201d}", hit.rule.title()),
            hit.question.clone(),
            format!("{}\n\n{review}", hit.why),
        ),
        None => (
            verdict.effect,
            verdict.why(),
            format!("Allow {tool}?"),
            review.clone(),
        ),
    };
    let (allow, how) = match effect {
        Effect::Allow => (true, format!("allowed {why}")),
        Effect::Deny => (false, format!("denied {why}")),
        Effect::Ask if review.len() > MAX_REVIEW => (
            false,
            format!("asked {why}, but too large to review here, so denied"),
        ),
        Effect::Ask => {
            let (answer, decision) = channel();
            let ticket = NEXT_TICKET.fetch_add(1, Ordering::Relaxed);
            on_call(Call::Approve {
                ticket,
                title,
                input: shown,
                answer,
            });
            match wait_for(stream, ticket, &decision, on_call) {
                Some(true) => (true, format!("asked {why}; you allowed it")),
                Some(false) => (false, format!("asked {why}; you denied it")),
                None => (false, format!("asked {why}; nobody answered, so denied")),
            }
        }
    };
    // A rule's deny says what to do instead, so the model can act on it.
    let reason = match &hit {
        Some(hit) if effect == Effect::Deny => format!("Velvet Hammer: {how}. {}", hit.why),
        _ => format!("Velvet Hammer: {how}."),
    };
    let mut entry = Entry::new(tool, tool_input, allow, how);
    if let Some(hit) = &hit {
        entry = entry.by_rule(hit.rule.key());
    }
    on_call(Call::Logged(entry));
    json!({"hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": if allow { "allow" } else { "deny" },
        "permissionDecisionReason": reason,
    }})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approval::{Mode, Rule};

    /// A bridge in "Ask every time" mode whose calls arrive on the returned receiver.
    fn bridge() -> (Bridge, Receiver<Call>) {
        let (bridge, calls, _) = bridge_with(Settings {
            mode: Mode::Ask,
            rules: vec![],
        });
        (bridge, calls)
    }

    fn bridge_with(settings: Settings) -> (Bridge, Receiver<Call>, Policy) {
        let (bridge, calls, policies) = bridge_on(settings, None);
        (bridge, calls, policies.approval)
    }

    /// A bridge deciding with `settings`, visual replies enforced, every firstmate rule on, and
    /// firstmate's home at `home` if given.
    fn bridge_on(settings: Settings, home: Option<Home>) -> (Bridge, Receiver<Call>, Policies) {
        let (tx, rx) = channel();
        let tx = Mutex::new(tx);
        let policies = Policies {
            approval: Arc::new(RwLock::new(settings)),
            visual: Arc::new(RwLock::new(visual::Mode::Enforce)),
            rules: Arc::new(RwLock::new(guard::Settings::default())),
            reminders: Reminders::in_memory(reminders::system_clock()),
        };
        let bridge = Bridge::start(policies.clone(), home, move |call| {
            tx.lock().unwrap().send(call).unwrap()
        })
        .unwrap();
        (bridge, rx, policies)
    }

    /// The next call that is not an activity log entry.
    fn next_call(calls: &Receiver<Call>) -> Option<Call> {
        loop {
            match calls.recv_timeout(Duration::from_secs(5)) {
                Ok(Call::Logged(_)) => continue,
                other => return other.ok(),
            }
        }
    }

    /// The activity log entries reported so far.
    fn logged(calls: &Receiver<Call>) -> Vec<Entry> {
        calls
            .try_iter()
            .filter_map(|c| match c {
                Call::Logged(entry) => Some(entry),
                _ => None,
            })
            .collect()
    }

    fn decision(out: &Value) -> (&str, &str) {
        let out = &out["hookSpecificOutput"];
        (
            out["permissionDecision"].as_str().unwrap(),
            out["permissionDecisionReason"].as_str().unwrap(),
        )
    }

    /// POSTs `body` and returns the status and the parsed body (Null when empty).
    fn post(bridge: &Bridge, path: &str, auth: Option<&str>, body: Value) -> (u16, Value) {
        let mut stream = TcpStream::connect(bridge.addr).unwrap();
        let body = body.to_string();
        let auth = auth
            .map(|a| format!("Authorization: {a}\r\n"))
            .unwrap_or_default();
        write!(
            stream,
            "POST {path} HTTP/1.1\r\nHost: x\r\n{auth}Content-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let status = response[9..12].parse().unwrap();
        let body = response.split_once("\r\n\r\n").unwrap().1;
        (status, serde_json::from_str(body).unwrap_or(Value::Null))
    }

    fn bearer(bridge: &Bridge) -> String {
        format!("Bearer {}", bridge.token)
    }

    #[test]
    fn requests_without_the_token_are_refused() {
        let (bridge, calls) = bridge();
        let note = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                          "params": {"name": "app_note", "arguments": {"text": "hi"}}});
        assert_eq!(post(&bridge, "/mcp", None, note.clone()).0, 401);
        assert_eq!(post(&bridge, "/mcp", Some("Bearer wrong"), note).0, 401);
        assert!(calls.try_recv().is_err());
    }

    #[test]
    fn the_mcp_server_lists_and_runs_app_note() {
        let (bridge, calls) = bridge();
        let auth = bearer(&bridge);
        let (status, init) = post(
            &bridge,
            "/mcp",
            Some(&auth),
            json!({"jsonrpc": "2.0", "id": 0,
            "method": "initialize", "params": {"protocolVersion": "2025-06-18"}}),
        );
        assert_eq!(status, 200);
        assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
        let notified = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        assert_eq!(post(&bridge, "/mcp", Some(&auth), notified).0, 202);
        let (_, list) = post(
            &bridge,
            "/mcp",
            Some(&auth),
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        );
        assert_eq!(list["result"]["tools"][0]["name"], "app_note");
        let (_, called) = post(
            &bridge,
            "/mcp",
            Some(&auth),
            json!({"jsonrpc": "2.0", "id": 2,
            "method": "tools/call", "params": {"name": "app_note", "arguments": {"text": "Renew the domain"}}}),
        );
        assert_eq!(called["result"]["isError"], Value::Null);
        assert!(matches!(calls.try_recv(), Ok(Call::Note(text)) if text == "Renew the domain"));
    }

    #[test]
    fn app_ui_lists_the_catalog_and_checks_a_spec() {
        let (bridge, calls) = bridge();
        let auth = bearer(&bridge);
        let (_, list) = post(
            &bridge,
            "/mcp",
            Some(&auth),
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        );
        let tool = &list["result"]["tools"][1];
        assert_eq!(tool["name"], "app_ui");
        assert_eq!(tool["inputSchema"]["required"], json!(["root", "elements"]));
        let description = tool["description"].as_str().unwrap();
        assert!(
            description.contains("- Button: {label: string") && description.contains("- submit")
        );
        let call = |arguments: Value| {
            post(
                &bridge,
                "/mcp",
                Some(&auth),
                json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                       "params": {"name": "app_ui", "arguments": arguments}}),
            )
            .1["result"]
                .clone()
        };
        let shown = call(json!({"root": "t", "elements": {"t": {"type": "Text",
            "props": {"text": "Hi", "size": 9}}}}));
        assert_eq!(shown["isError"], Value::Null);
        let text = shown["content"][0]["text"].as_str().unwrap();
        assert!(text.starts_with("The view is shown") && text.contains("no prop \"size\""));
        let refused = call(json!({"root": "missing", "elements": {}}));
        assert_eq!(refused["isError"], true);
        // Checking a spec asks nothing of the app: the chat draws it from the tool use.
        assert!(calls.try_recv().is_err());
    }

    #[test]
    fn app_remind_sets_a_reminder_at_the_apps_time_and_app_reminders_lists_it() {
        let (tx, calls) = channel();
        let tx = Mutex::new(tx);
        let eastern = jiff::tz::TimeZone::posix("EST5EDT,M3.2.0,M11.1.0").unwrap();
        let now = jiff::civil::date(2026, 11, 13)
            .at(14, 30, 0, 0)
            .to_zoned(eastern)
            .unwrap();
        let mut policies = Policies::ask_every_time();
        policies.reminders = Reminders::in_memory(Arc::new(move || now.clone()));
        let bridge = Bridge::start(policies.clone(), None, move |call| {
            tx.lock().unwrap().send(call).unwrap()
        })
        .unwrap();
        let auth = bearer(&bridge);
        let (_, list) = post(
            &bridge,
            "/mcp",
            Some(&auth),
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        );
        let names: Vec<&str> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["app_note", "app_ui", "app_remind", "app_reminders"]);
        let call = |name: &str, arguments: Value| {
            post(
                &bridge,
                "/mcp",
                Some(&auth),
                json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                       "params": {"name": name, "arguments": arguments}}),
            )
            .1["result"]
                .clone()
        };
        let set = call(
            "app_remind",
            json!({"text": "stretch", "when": "in", "amount": 2, "unit": "minutes"}),
        );
        assert_eq!(
            set["content"][0]["text"],
            "Reminder set for Today, 2:32 PM: stretch"
        );
        assert!(matches!(calls.try_recv(), Ok(Call::Reminded)));
        let refused = call("app_remind", json!({"text": "stretch", "when": "soon"}));
        assert_eq!(refused["isError"], true);
        assert!(calls.try_recv().is_err());
        let listed = call("app_reminders", json!({}));
        assert_eq!(
            listed["content"][0]["text"],
            "It is now Today, 2:30 PM.\n- Today, 2:32 PM: stretch (set Today, 2:30 PM)"
        );
        assert_eq!(policies.reminders.list().len(), 1);
    }

    #[test]
    fn in_ask_mode_the_hook_waits_for_the_captain() {
        let (bridge, calls) = bridge();
        let auth = bearer(&bridge);
        let bridge = Arc::new(bridge);
        for (allow, want) in [(Some(true), "allow"), (Some(false), "deny"), (None, "deny")] {
            let hook = thread::spawn({
                let (bridge, auth) = (bridge.clone(), auth.clone());
                move || {
                    let input = json!({"hook_event_name": "PreToolUse", "tool_name": "Bash",
                                       "tool_input": {"command": "git push"}});
                    post(&bridge, "/hooks/pre-tool-use", Some(&auth), input).1
                }
            });
            let Some(Call::Approve {
                title,
                input,
                answer,
                ..
            }) = next_call(&calls)
            else {
                panic!("expected an approval");
            };
            assert_eq!(
                (title.as_str(), input.as_str()),
                ("Allow Bash?", "command: git push")
            );
            // `None`: the app dropped the question, as when the session ends.
            match allow {
                Some(allow) => answer.send(allow).unwrap(),
                None => drop(answer),
            }
            let out = hook.join().unwrap();
            assert_eq!(decision(&out).0, want);
            let entry = next_logged(&calls);
            assert_eq!(
                (entry.allowed, entry.subject.as_str()),
                (want == "allow", "git push")
            );
            assert!(entry.how.starts_with("asked by the mode Ask every time; "));
        }
    }

    fn next_logged(calls: &Receiver<Call>) -> Entry {
        match calls.recv_timeout(Duration::from_secs(5)) {
            Ok(Call::Logged(entry)) => entry,
            _ => panic!("expected a log entry"),
        }
    }

    #[test]
    fn each_mode_allows_or_asks_without_a_rule() {
        let read = json!({"tool_name": "Read", "tool_input": {"file_path": "/repo/a.rs"}});
        let note =
            json!({"tool_name": "mcp__velvet-hammer__app_note", "tool_input": {"text": "x"}});
        let bash = json!({"tool_name": "Bash", "tool_input": {"command": "rm -rf build"}});
        for (mode, input, asks) in [
            (Mode::Run, &bash, false),
            (Mode::Run, &read, false),
            (Mode::ReadOnly, &read, false),
            (Mode::ReadOnly, &note, false),
            (Mode::ReadOnly, &bash, true),
            (Mode::Ask, &read, true),
            (Mode::Ask, &note, true),
        ] {
            let (bridge, calls, _) = bridge_with(Settings {
                mode,
                rules: vec![],
            });
            let auth = bearer(&bridge);
            let bridge = Arc::new(bridge);
            let hook = thread::spawn({
                let (bridge, auth, input) = (bridge.clone(), auth.clone(), input.clone());
                move || post(&bridge, "/hooks/pre-tool-use", Some(&auth), input).1
            });
            if asks {
                let Some(Call::Approve { answer, .. }) = next_call(&calls) else {
                    panic!("{mode:?} should ask about {input}");
                };
                answer.send(true).unwrap();
            }
            let out = hook.join().unwrap();
            assert_eq!(decision(&out).0, "allow", "{mode:?} {input}");
            let entries = logged(&calls);
            assert_eq!(entries.len(), 1, "{mode:?} {input}");
            let how = &entries[0].how;
            let want = if asks {
                "asked by the mode"
            } else {
                "allowed by the mode"
            };
            assert!(how.starts_with(want), "{mode:?}: {how}");
        }
    }

    #[test]
    fn rules_deny_ask_and_allow_with_deny_first() {
        let (bridge, calls, policy) = bridge_with(Settings {
            mode: Mode::Run,
            rules: vec![
                Rule {
                    effect: Effect::Allow,
                    tool: "Bash".into(),
                    pattern: Some("git".into()),
                },
                Rule {
                    effect: Effect::Deny,
                    tool: "Bash".into(),
                    pattern: Some("git push".into()),
                },
                Rule {
                    effect: Effect::Ask,
                    tool: "Write".into(),
                    pattern: None,
                },
            ],
        });
        let auth = bearer(&bridge);
        let bash = |c: &str| json!({"tool_name": "Bash", "tool_input": {"command": c}});
        // Deny wins over the allow rule that also matches, without asking.
        let (_, out) = post(
            &bridge,
            "/hooks/pre-tool-use",
            Some(&auth),
            bash("git push"),
        );
        assert_eq!(
            decision(&out),
            (
                "deny",
                "Velvet Hammer: denied by rule 2 (Deny Bash matching git push)."
            )
        );
        let (_, out) = post(
            &bridge,
            "/hooks/pre-tool-use",
            Some(&auth),
            bash("git status"),
        );
        assert_eq!(decision(&out).0, "allow");
        let entries = logged(&calls);
        assert_eq!(entries.len(), 2);
        assert!(!entries[0].allowed && entries[1].allowed);
        // Ask forces the full-input question in Run without asking.
        let bridge = Arc::new(bridge);
        let hook = thread::spawn({
            let (bridge, auth) = (bridge.clone(), auth.clone());
            move || {
                let input = json!({"tool_name": "Write", "tool_input":
                    {"file_path": "/repo/a.txt", "content": "line one\nline two"}});
                post(&bridge, "/hooks/pre-tool-use", Some(&auth), input).1
            }
        });
        let Some(Call::Approve { input, answer, .. }) = next_call(&calls) else {
            panic!("an ask rule should ask");
        };
        assert_eq!(input, "content: line one\nline two\nfile_path: /repo/a.txt");
        answer.send(false).unwrap();
        assert_eq!(decision(&hook.join().unwrap()).0, "deny");
        // A change applies to the next tool use of the running session.
        policy.write().unwrap().rules.clear();
        let (_, out) = post(
            &bridge,
            "/hooks/pre-tool-use",
            Some(&auth),
            bash("git push"),
        );
        assert_eq!(decision(&out).0, "allow");
    }

    #[test]
    fn too_large_an_input_is_denied_when_it_would_be_asked_about() {
        let (bridge, calls, policy) = bridge_with(Settings {
            mode: Mode::Run,
            rules: vec![Rule {
                effect: Effect::Ask,
                tool: "Bash".into(),
                pattern: None,
            }],
        });
        let auth = bearer(&bridge);
        let big = json!({"tool_name": "Bash", "tool_input": {"command": "x".repeat(MAX_REVIEW)}});
        let (_, out) = post(&bridge, "/hooks/pre-tool-use", Some(&auth), big.clone());
        assert_eq!(decision(&out).0, "deny");
        assert!(decision(&out).1.contains("too large to review"));
        assert!(matches!(calls.try_recv(), Ok(Call::Logged(e)) if !e.allowed));
        assert!(calls.try_recv().is_err(), "nothing was asked");
        // Nothing is approved from a partial view; with no question there is no view.
        policy.write().unwrap().rules.clear();
        let (_, out) = post(&bridge, "/hooks/pre-tool-use", Some(&auth), big);
        assert_eq!(decision(&out).0, "allow");
    }

    #[test]
    fn the_captain_is_shown_the_whole_input_and_too_large_an_input_is_denied() {
        let (bridge, calls) = bridge();
        let auth = bearer(&bridge);
        let bridge = Arc::new(bridge);
        let command = format!("echo {} ; rm -rf ~", "a".repeat(500));
        let hook = thread::spawn({
            let (bridge, auth) = (bridge.clone(), auth.clone());
            move || {
                let input = json!({"tool_name": "Write", "tool_input":
                    {"file_path": "/tmp/x", "content": "one\ntwo", "command": command}});
                post(&bridge, "/hooks/pre-tool-use", Some(&auth), input).1
            }
        });
        let Some(Call::Approve { input, answer, .. }) = next_call(&calls) else {
            panic!("expected an approval");
        };
        for part in ["file_path: /tmp/x", "content: one\ntwo", "; rm -rf ~"] {
            assert!(input.contains(part), "{part} missing from {input}");
        }
        answer.send(true).unwrap();
        hook.join().unwrap();

        let big = json!({"tool_name": "Bash", "tool_input": {"command": "x".repeat(MAX_REVIEW)}});
        let (_, out) = post(&bridge, "/hooks/pre-tool-use", Some(&auth), big);
        assert_eq!(out["hookSpecificOutput"]["permissionDecision"], "deny");
        assert!(
            out["hookSpecificOutput"]["permissionDecisionReason"]
                .as_str()
                .unwrap()
                .contains("too large to review")
        );
        assert!(!calls.try_iter().any(|c| matches!(c, Call::Approve { .. })));
    }

    #[test]
    fn a_hook_claude_code_hung_up_on_expires_its_question() {
        let (bridge, calls) = bridge();
        let auth = bearer(&bridge);
        let mut stream = TcpStream::connect(bridge.addr).unwrap();
        let body = json!({"tool_name": "Bash", "tool_input": {"command": "ls"}}).to_string();
        write!(
            stream,
            "POST /hooks/pre-tool-use HTTP/1.1\r\nAuthorization: {auth}\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let Ok(Call::Approve { ticket, answer, .. }) = calls.recv_timeout(Duration::from_secs(5))
        else {
            panic!("expected an approval");
        };
        drop(stream);
        assert!(matches!(
            calls.recv_timeout(Duration::from_secs(5)),
            Ok(Call::Expired(t)) if t == ticket
        ));
        drop(answer);
    }

    #[test]
    fn the_settings_add_three_hooks_and_only_the_tool_hook_blocks_on_failure() {
        let (bridge, _) = bridge();
        let settings: Value = serde_json::from_str(&bridge.settings()).unwrap();
        let hooks = &settings["hooks"];
        assert_eq!(hooks["PreToolUse"][0]["hooks"][0]["onFailure"], "block");
        for (event, path) in [
            ("UserPromptSubmit", "/hooks/user-prompt-submit"),
            ("Stop", "/hooks/stop"),
        ] {
            let hook = &hooks[event][0]["hooks"][0];
            assert!(hook["url"].as_str().unwrap().ends_with(path), "{event}");
            assert_eq!(hook["onFailure"], Value::Null, "{event}");
            assert_eq!(hook["timeout"], QUICK_TIMEOUT_SECS, "{event}");
        }
    }

    #[test]
    fn each_message_carries_the_standing_instruction_unless_visuals_are_off() {
        let (bridge, _, policies) = bridge_on(Settings::default(), None);
        let auth = bearer(&bridge);
        let prompt = json!({"hook_event_name": "UserPromptSubmit", "prompt": "status?"});
        for (mode, wants) in [
            (visual::Mode::Enforce, true),
            (visual::Mode::Nudge, true),
            (visual::Mode::Off, false),
        ] {
            *policies.visual.write().unwrap() = mode;
            let (status, out) = post(
                &bridge,
                "/hooks/user-prompt-submit",
                Some(&auth),
                prompt.clone(),
            );
            assert_eq!(status, 200);
            let context = out["hookSpecificOutput"]["additionalContext"].as_str();
            assert_eq!(
                context.is_some_and(|c| c.contains("app_ui")),
                wants,
                "{mode:?}"
            );
        }
    }

    #[test]
    fn the_first_message_of_a_session_says_how_message_links_work_once() {
        let (bridge, _, policies) = bridge_on(Settings::default(), None);
        *policies.visual.write().unwrap() = visual::Mode::Off;
        let auth = bearer(&bridge);
        let prompt = json!({"hook_event_name": "UserPromptSubmit", "prompt": "hello"});
        let (_, first) = post(
            &bridge,
            "/hooks/user-prompt-submit",
            Some(&auth),
            prompt.clone(),
        );
        assert_eq!(
            first["hookSpecificOutput"]["additionalContext"],
            crate::links::INSTRUCTION
        );
        let (_, second) = post(&bridge, "/hooks/user-prompt-submit", Some(&auth), prompt);
        assert_eq!(second, json!({}));
    }

    #[test]
    fn a_list_without_a_view_is_sent_back_once_and_a_view_lets_it_through() {
        let (bridge, calls, policies) = bridge_on(Settings::default(), None);
        let auth = bearer(&bridge);
        let stop = |active: bool| {
            post(&bridge, "/hooks/stop", Some(&auth), json!({"hook_event_name": "Stop",
                "stop_hook_active": active, "last_assistant_message": "Open items:\n- a\n- b\n- c"}))
            .1
        };
        let out = stop(false);
        assert_eq!(out["hookSpecificOutput"]["hookEventName"], "Stop");
        assert!(
            out["hookSpecificOutput"]["additionalContext"]
                .as_str()
                .unwrap()
                .contains("lists 3 items")
        );
        assert!(matches!(calls.try_recv(), Ok(Call::Nudged(why)) if why == "lists 3 items"));
        // The retry cap: the next stop ends the turn, whatever Claude Code reports.
        assert_eq!(stop(false), json!({}));
        assert_eq!(stop(false)["hookSpecificOutput"]["hookEventName"], "Stop");
        assert_eq!(stop(true), json!({}));
        // A view shown in the turn satisfies it.
        let (_, shown) = post(
            &bridge,
            "/mcp",
            Some(&auth),
            json!({"jsonrpc": "2.0", "id": 1,
            "method": "tools/call", "params": {"name": "app_ui", "arguments":
            {"root": "t", "elements": {"t": {"type": "Text", "props": {"text": "Hi"}}}}}}),
        );
        assert_eq!(shown["result"]["isError"], Value::Null);
        assert_eq!(stop(false), json!({}));
        *policies.visual.write().unwrap() = visual::Mode::Nudge;
        assert_eq!(stop(false), json!({}));
    }

    /// A firstmate home under `target/` with no tasks.
    fn firstmate_home() -> Home {
        use std::sync::atomic::{AtomicU32, Ordering};
        static RUN: AtomicU32 = AtomicU32::new(0);
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/bridge-home")
            .join(format!(
                "{}-{}",
                std::process::id(),
                RUN.fetch_add(1, Ordering::Relaxed)
            ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("state")).unwrap();
        Home::new(dir)
    }

    #[test]
    fn a_firstmate_rule_denies_with_what_to_do_whatever_the_mode() {
        let (bridge, calls, _) = bridge_on(Settings::default(), Some(firstmate_home()));
        let auth = bearer(&bridge);
        let (_, out) = post(
            &bridge,
            "/hooks/pre-tool-use",
            Some(&auth),
            json!({"tool_name": "Bash",
            "cwd": "/clone", "tool_input": {"command": "git push --force origin main"}}),
        );
        let (decision, reason) = decision(&out);
        assert_eq!(decision, "deny");
        assert!(reason.starts_with(
            "Velvet Hammer: denied by firstmate rule \u{201c}No force pushes\u{201d}. "
        ));
        assert!(reason.contains("Push without forcing"));
        let entry = next_logged(&calls);
        assert_eq!(entry.rule.as_deref(), Some("no-force-push"));
        assert!(!entry.allowed);
    }

    #[test]
    fn a_merge_asks_the_captain_by_name_and_no_allow_rule_answers_for_them() {
        let allow_all = Settings {
            mode: Mode::Run,
            rules: vec![Rule {
                effect: Effect::Allow,
                tool: "Bash".into(),
                pattern: None,
            }],
        };
        let (bridge, calls, policies) = bridge_on(allow_all, Some(firstmate_home()));
        let auth = bearer(&bridge);
        let bridge = Arc::new(bridge);
        let merge = json!({"tool_name": "Bash", "cwd": "/clone", "tool_input":
            {"command": "bin/fm-pr-merge.sh site-fix https://github.com/o/site/pull/7"}});
        for allow in [true, false] {
            let hook = thread::spawn({
                let (bridge, auth, merge) = (bridge.clone(), auth.clone(), merge.clone());
                move || post(&bridge, "/hooks/pre-tool-use", Some(&auth), merge).1
            });
            let Some(Call::Approve {
                title,
                input,
                answer,
                ..
            }) = next_call(&calls)
            else {
                panic!("a merge should ask");
            };
            assert_eq!(title, "Merge https://github.com/o/site/pull/7?");
            assert!(input.starts_with("Firstmate's hard rule 2"));
            assert!(input.ends_with(
                "command: bin/fm-pr-merge.sh site-fix https://github.com/o/site/pull/7"
            ));
            answer.send(allow).unwrap();
            let out = hook.join().unwrap();
            assert_eq!(decision(&out).0, if allow { "allow" } else { "deny" });
            let entry = next_logged(&calls);
            assert_eq!(entry.rule.as_deref(), Some("merge-asks-captain"));
        }
        // A deny rule of the captain's still wins over the rule's question.
        policies.approval.write().unwrap().rules.push(Rule {
            effect: Effect::Deny,
            tool: "Bash".into(),
            pattern: Some("bin/fm-pr-merge.sh".into()),
        });
        let (_, out) = post(&bridge, "/hooks/pre-tool-use", Some(&auth), merge.clone());
        assert_eq!(decision(&out).0, "deny");
        assert!(next_logged(&calls).rule.is_none());
        // Turned off, the rule leaves the merge to the approval settings.
        policies.approval.write().unwrap().rules.pop();
        policies
            .rules
            .write()
            .unwrap()
            .set(guard::Rule::MergeAsks, false);
        let (_, out) = post(&bridge, "/hooks/pre-tool-use", Some(&auth), merge);
        assert_eq!(decision(&out).0, "allow");
    }

    #[test]
    fn firstmate_rules_apply_only_to_a_firstmate_session() {
        let (bridge, calls, _) = bridge_on(Settings::default(), None);
        let auth = bearer(&bridge);
        let (_, out) = post(
            &bridge,
            "/hooks/pre-tool-use",
            Some(&auth),
            json!({"tool_name": "Bash",
            "tool_input": {"command": "git push --force"}}),
        );
        assert_eq!(decision(&out).0, "allow");
        assert!(next_logged(&calls).rule.is_none());
    }

    #[test]
    fn every_rule_denies_or_asks_and_the_captains_answer_decides() {
        let home = firstmate_home();
        let project = home.dir.join("projects/site");
        let (bridge, calls, _) = bridge_on(Settings::default(), Some(home));
        let auth = bearer(&bridge);
        let bridge = Arc::new(bridge);
        let bash = |command: &str| {
            json!({"tool_name": "Bash", "cwd": "/clone",
            "tool_input": {"command": command}})
        };
        let hook = |input: Value| {
            let (bridge, auth) = (bridge.clone(), auth.clone());
            thread::spawn(move || post(&bridge, "/hooks/pre-tool-use", Some(&auth), input).1)
        };
        let denied = [
            ("merge-through-script", bash("gh pr merge 7 --squash")),
            (
                "no-admin-merge",
                bash("bin/fm-pr-merge.sh t https://x.test/o/r/pull/7 -- --admin"),
            ),
            ("no-force-push", bash("git push --force origin main")),
            (
                "discard-asks-captain",
                bash("git worktree remove --force /pool/1/site"),
            ),
        ];
        for (rule, input) in denied {
            let out = hook(input).join().unwrap();
            assert_eq!(decision(&out).0, "deny", "{rule}");
            let entry = next_logged(&calls);
            assert_eq!(entry.rule.as_deref(), Some(rule));
        }
        let asked = [
            (
                "merge-asks-captain",
                bash("bin/fm-pr-merge.sh t https://x.test/o/r/pull/7"),
            ),
            ("merge-asks-captain", bash("bin/fm-merge-local.sh t")),
            ("discard-asks-captain", bash("bin/fm-teardown.sh t --force")),
            (
                "projects-read-only",
                json!({"tool_name": "Edit", "cwd": "/clone", "tool_input":
                    {"file_path": project.join("a.rs"), "old_string": "a", "new_string": "b"}}),
            ),
            (
                "projects-read-only",
                bash(&format!("git -C {} commit -am x", project.display())),
            ),
            ("publish-asks-captain", bash("git push origin fix")),
            (
                "publish-asks-captain",
                bash("gh repo create o/new --private"),
            ),
        ];
        for (rule, input) in asked {
            for allow in [true, false] {
                let waiting = hook(input.clone());
                let Some(Call::Approve { answer, .. }) = next_call(&calls) else {
                    panic!("{rule} should ask about {input}");
                };
                answer.send(allow).unwrap();
                let out = waiting.join().unwrap();
                assert_eq!(
                    decision(&out).0,
                    if allow { "allow" } else { "deny" },
                    "{rule}"
                );
                assert_eq!(next_logged(&calls).rule.as_deref(), Some(rule));
            }
        }
        // What no rule matches goes on to the mode, which runs it.
        for command in [
            "gh pr view 7",
            "git status",
            "bin/fm-teardown.sh t",
            "ls projects",
        ] {
            let out = hook(bash(command)).join().unwrap();
            assert_eq!(decision(&out).0, "allow", "{command}");
            assert!(next_logged(&calls).rule.is_none());
        }
    }
}
