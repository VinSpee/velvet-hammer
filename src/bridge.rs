//! The extension seam between the chat session's `claude` and the app: a small HTTP server on
//! 127.0.0.1 that `claude` calls back into. It serves two things, both passed to that one session
//! as command-line flags so the captain's own Claude settings are never written:
//!
//! - an MCP server (`--mcp-config`) with one tool, `app_note`, which puts a note in the captain's
//!   For you to do panel;
//! - a PreToolUse hook (`--settings`), which applies the captain's tool-approval settings (see
//!   [`crate::approval`]) to each tool use: it allows it, denies it, or asks the captain to allow
//!   or deny it in the Needs your input panel. Every decision is reported for the activity log.
//!
//! Every request must carry the bearer token made for this server, so other programs and web
//! pages cannot reach it. A hook the server cannot answer blocks the tool use (`onFailure`).

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::approval::{self, Effect, Entry, Policy};

/// The MCP server's name; Claude Code calls its tools `mcp__velvet-hammer__<tool>`.
pub const SERVER: &str = "velvet-hammer";
const NOTE_TOOL: &str = "app_note";
/// Claude Code's timeout for the hook, in seconds (its default).
const HOOK_TIMEOUT_SECS: u64 = 600;
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
    /// `claude` wants to use `tool`; `input` is all of what it would do, as text. Send true to
    /// allow it, false to deny it; dropping `answer` denies it.
    Approve {
        ticket: u64,
        tool: String,
        input: String,
        answer: Sender<bool>,
    },
    /// The question with this ticket can no longer be answered: the wait ran out or Claude Code
    /// stopped waiting. Whatever answer comes later is not used.
    Expired(u64),
    /// The hook decided a tool use, for the activity log.
    Logged(Entry),
}

pub struct Bridge {
    addr: SocketAddr,
    token: String,
    stopped: Arc<AtomicBool>,
}

impl Bridge {
    /// Starts serving on a free port of 127.0.0.1. The hook decides with `policy` as it stands at
    /// each tool use. `on_call` runs on the server's threads.
    pub fn start(
        policy: Policy,
        on_call: impl Fn(Call) + Send + Sync + 'static,
    ) -> io::Result<Bridge> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let addr = listener.local_addr()?;
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|e| io::Error::other(e.to_string()))?;
        let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let stopped = Arc::new(AtomicBool::new(false));
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
                let (expected, policy, on_call) =
                    (expected.clone(), policy.clone(), on_call.clone());
                thread::spawn(move || {
                    let _ = serve(stream, &expected, &policy, &*on_call);
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

    /// The `--settings` value adding the PreToolUse hook for this session only.
    pub fn settings(&self) -> String {
        json!({"hooks": {"PreToolUse": [{"matcher": "*", "hooks": [{
            "type": "http",
            "url": self.url("/hooks/pre-tool-use"),
            "headers": {"Authorization": format!("Bearer {}", self.token)},
            "timeout": HOOK_TIMEOUT_SECS,
            "onFailure": "block",
        }]}]}})
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
    policy: &Policy,
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
        let mut body = vec![0; length];
        reader.read_exact(&mut body)?;
        match (path.as_str(), serde_json::from_slice::<Value>(&body)) {
            ("/mcp", Ok(request)) => mcp(&request, on_call).map_or(Reply::Accepted, Reply::Json),
            ("/hooks/pre-tool-use", Ok(input)) => {
                Reply::Json(pre_tool_use(&input, &stream, policy, on_call))
            }
            ("/mcp" | "/hooks/pre-tool-use", Err(_)) => Reply::Status(400, "Bad Request"),
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
fn mcp(request: &Value, on_call: &dyn Fn(Call)) -> Option<Value> {
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

/// Decides one PreToolUse hook with the captain's settings. A tool use they leave to the
/// captain waits in Needs your input with all of its input shown or, when it is too large to
/// show, is denied rather than approved from a partial view.
fn pre_tool_use(
    input: &Value,
    stream: &TcpStream,
    policy: &Policy,
    on_call: &dyn Fn(Call),
) -> Value {
    let tool = input["tool_name"].as_str().unwrap_or_default();
    let tool_input = &input["tool_input"];
    let verdict = policy
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .decide(tool, tool_input);
    let why = verdict.why();
    let review = approval::input_text(tool_input);
    let (allow, how) = match verdict.effect {
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
                tool: tool.to_string(),
                input: review,
                answer,
            });
            match wait_for(stream, ticket, &decision, on_call) {
                Some(true) => (true, format!("asked {why}; you allowed it")),
                Some(false) => (false, format!("asked {why}; you denied it")),
                None => (false, format!("asked {why}; nobody answered, so denied")),
            }
        }
    };
    let reason = format!("Velvet Hammer: {how}.");
    on_call(Call::Logged(Entry::new(tool, tool_input, allow, how)));
    json!({"hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": if allow { "allow" } else { "deny" },
        "permissionDecisionReason": reason,
    }})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approval::{Mode, Rule, Settings};
    use std::sync::{Mutex, RwLock};

    /// A bridge in "Ask every time" mode whose calls arrive on the returned receiver.
    fn bridge() -> (Bridge, Receiver<Call>) {
        let (bridge, calls, _) = bridge_with(Settings {
            mode: Mode::Ask,
            rules: vec![],
        });
        (bridge, calls)
    }

    fn bridge_with(settings: Settings) -> (Bridge, Receiver<Call>, Policy) {
        let (tx, rx) = channel();
        let tx = Mutex::new(tx);
        let policy = Arc::new(RwLock::new(settings));
        let bridge = Bridge::start(policy.clone(), move |call| {
            tx.lock().unwrap().send(call).unwrap()
        })
        .unwrap();
        (bridge, rx, policy)
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
                tool,
                input,
                answer,
                ..
            }) = next_call(&calls)
            else {
                panic!("expected an approval");
            };
            assert_eq!(
                (tool.as_str(), input.as_str()),
                ("Bash", "command: git push")
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
}
