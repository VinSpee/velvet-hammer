//! The extension seam between the chat session's `claude` and the app: a small HTTP server on
//! 127.0.0.1 that `claude` calls back into. It serves two things, both passed to that one session
//! as command-line flags so the captain's own Claude settings are never written:
//!
//! - an MCP server (`--mcp-config`) with one tool, `app_note`, which puts a note in the captain's
//!   For you to do panel;
//! - a PreToolUse hook (`--settings`), which asks the captain to allow or deny each tool use in
//!   the Needs your input panel. The app's own tools are allowed without asking.
//!
//! Every request must carry the bearer token made for this server, so other programs and web
//! pages cannot reach it. A hook the server cannot answer blocks the tool use (`onFailure`).

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

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

/// Something `claude` asked of the app.
pub enum Call {
    /// `app_note` was called with this text.
    Note(String),
    /// `claude` wants to use `tool` with `input`. Send true to allow it, false to deny it;
    /// dropping `answer` denies it.
    Approve {
        tool: String,
        input: Value,
        answer: Sender<bool>,
    },
}

pub struct Bridge {
    addr: SocketAddr,
    token: String,
    stopped: Arc<AtomicBool>,
}

impl Bridge {
    /// Starts serving on a free port of 127.0.0.1. `on_call` runs on the server's threads.
    pub fn start(on_call: impl Fn(Call) + Send + Sync + 'static) -> io::Result<Bridge> {
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
                let (expected, on_call) = (expected.clone(), on_call.clone());
                thread::spawn(move || {
                    let _ = serve(stream, &expected, &*on_call);
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
fn serve(stream: TcpStream, expected: &str, on_call: &dyn Fn(Call)) -> io::Result<()> {
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
            ("/hooks/pre-tool-use", Ok(input)) => Reply::Json(pre_tool_use(&input, on_call)),
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

/// Decides one PreToolUse hook: the app's own tools are allowed, anything else waits for the
/// captain.
fn pre_tool_use(input: &Value, on_call: &dyn Fn(Call)) -> Value {
    let tool = input["tool_name"].as_str().unwrap_or_default();
    let (allow, reason) = if tool.starts_with(&format!("mcp__{SERVER}__")) {
        (true, "Velvet Hammer's own tool.")
    } else {
        let (answer, decision) = channel();
        on_call(Call::Approve {
            tool: tool.to_string(),
            input: input["tool_input"].clone(),
            answer,
        });
        match decision.recv_timeout(APPROVAL_WAIT) {
            Ok(true) => (true, "The captain allowed this in Velvet Hammer."),
            Ok(false) => (false, "The captain denied this in Velvet Hammer."),
            Err(_) => (false, "Nobody allowed this in Velvet Hammer."),
        }
    };
    json!({"hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": if allow { "allow" } else { "deny" },
        "permissionDecisionReason": reason,
    }})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::mpsc::Receiver;

    /// A bridge whose calls arrive on the returned receiver.
    fn bridge() -> (Bridge, Receiver<Call>) {
        let (tx, rx) = channel();
        let tx = Mutex::new(tx);
        let bridge = Bridge::start(move |call| tx.lock().unwrap().send(call).unwrap()).unwrap();
        (bridge, rx)
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
    fn the_hook_waits_for_the_captain_and_allows_the_apps_own_tools() {
        let (bridge, calls) = bridge();
        let auth = bearer(&bridge);
        let own =
            json!({"hook_event_name": "PreToolUse", "tool_name": "mcp__velvet-hammer__app_note"});
        let (_, out) = post(&bridge, "/hooks/pre-tool-use", Some(&auth), own);
        assert_eq!(out["hookSpecificOutput"]["permissionDecision"], "allow");
        assert!(calls.try_recv().is_err());

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
            let Ok(Call::Approve {
                tool,
                input,
                answer,
            }) = calls.recv_timeout(Duration::from_secs(5))
            else {
                panic!("expected an approval");
            };
            assert_eq!(
                (tool.as_str(), &input["command"]),
                ("Bash", &json!("git push"))
            );
            // `None`: the app dropped the question, as when the session ends.
            match allow {
                Some(allow) => answer.send(allow).unwrap(),
                None => drop(answer),
            }
            let out = hook.join().unwrap();
            assert_eq!(out["hookSpecificOutput"]["permissionDecision"], want);
        }
    }
}
