//! The conversation kept across restarts: each finished chat message as one line of
//! `history.jsonl` in the app's data directory, read back when the app opens so find searches it
//! and its message links still resolve (see [`crate::links`]).
//!
//! The file and its directory are readable by the captain's user only. A message is written once
//! it is finished: the captain's when sent, a reply when it ends, a tool use when its result
//! arrives. A tool use keeps its name, its input and its result as text; image bytes are never
//! written, only the paths a tool named. A file past [`ROTATE_AT`] is moved to
//! `history.jsonl.1` when the app opens, and only the current file is read back.
//!
//! What is kept is never sent to the chief of staff on its own: a message from an earlier session
//! reaches it only when the captain links to it.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::model::{Author, Message};
use crate::store::{
    self, Saved, create_private_dir, make_private, make_private_dir, open_private, suffixed,
};
use crate::tools::{Content, ToolResult, ToolUse};

pub const FILE: &str = "history.jsonl";
/// The settings file for whether history is kept.
pub const SETTINGS_FILE: &str = "history.json";
/// The file is rotated past this size when the app opens.
const ROTATE_AT: u64 = 8 << 20;
/// The most messages read back.
const KEEP: usize = 2000;
/// The most of a text block or a tool input that is written; the rest is cut.
const TEXT_MAX: usize = 64 * 1024;
/// What a restored image says in place of the image.
pub const NO_IMAGE: &str = "Images are not kept in the conversation history.";

/// Whether the conversation is kept across restarts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Keeping(pub bool);

impl Default for Keeping {
    fn default() -> Self {
        Keeping::RECOMMENDED
    }
}

impl Keeping {
    pub const RECOMMENDED: Keeping = Keeping(true);
    pub const ALL: [Keeping; 2] = [Keeping(true), Keeping(false)];

    pub fn label(self) -> &'static str {
        if self.0 {
            "Keep the conversation"
        } else {
            "Do not keep the conversation"
        }
    }

    pub fn explanation(self) -> &'static str {
        if self.0 {
            "Each message is written to history.jsonl in Velvet Hammer's data directory, \
             readable by your user only, so the conversation, find and message links last \
             across restarts. Images are not kept."
        } else {
            "Nothing new is written. What is already kept stays until you clear it, and is not \
             shown the next time Velvet Hammer opens."
        }
    }

    pub fn key(self) -> &'static str {
        if self.0 { "keep" } else { "off" }
    }

    pub fn summary(self) -> &'static str {
        if self.0 { "kept" } else { "not kept" }
    }
}

impl Saved for Keeping {
    const FILE: &'static str = SETTINGS_FILE;
    const FALLBACK: &'static str = "the conversation is not kept";

    fn fallback() -> Self {
        Keeping(false)
    }

    fn to_json(&self) -> Value {
        json!({"version": store::VERSION, "keep": self.0})
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        store::check_version(value)?;
        value["keep"]
            .as_bool()
            .map(Keeping)
            .ok_or_else(|| "it does not say whether to keep the conversation".into())
    }
}

/// The history file.
pub struct History {
    path: PathBuf,
    /// Why the file could not be read or written, for the settings page.
    problem: Option<String>,
}

impl History {
    pub fn new(dir: &Path) -> History {
        History {
            path: dir.join(FILE),
            problem: None,
        }
    }

    /// The messages kept from earlier sessions, oldest first, rotating a file past
    /// [`ROTATE_AT`] first. A line that cannot be read is skipped.
    pub fn load(&mut self) -> Vec<Message> {
        if fs::metadata(&self.path).is_ok_and(|m| m.len() > ROTATE_AT) {
            let old = suffixed(&self.path, ".1");
            let rotated = fs::rename(&self.path, &old).and_then(|()| self.private(&old));
            if let Err(e) = rotated {
                self.problem = Some(format!(
                    "Velvet Hammer could not rotate {}: {e}.",
                    self.path.display()
                ));
            }
        }
        let text = match fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Vec::new(),
            Err(e) => {
                self.problem = Some(format!(
                    "Velvet Hammer could not read {}: {e}.",
                    self.path.display()
                ));
                return Vec::new();
            }
        };
        let messages: Vec<Message> = text
            .lines()
            .filter_map(|line| from_json(&serde_json::from_str(line).ok()?))
            .collect();
        let skip = messages.len().saturating_sub(KEEP);
        messages.into_iter().skip(skip).collect()
    }

    /// Appends one finished message.
    pub fn record(&mut self, message: &Message) {
        let dir = self.path.parent().expect("in a directory");
        let written = create_private_dir(dir)
            .and_then(|()| make_private_dir(dir))
            .and_then(|()| open_private(&self.path))
            .and_then(|mut file| writeln!(file, "{}", to_json(message)));
        self.problem = written.err().map(|e| {
            format!(
                "Velvet Hammer could not write to {}: {e}. Messages since then are not kept.",
                self.path.display()
            )
        });
    }

    /// Deletes the file and its rotated copy.
    pub fn clear(&mut self) {
        let mut problem = None;
        for path in [self.path.clone(), suffixed(&self.path, ".1")] {
            match fs::remove_file(&path) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => {
                    problem = Some(format!(
                        "Velvet Hammer could not delete {}: {e}.",
                        path.display()
                    ))
                }
                _ => {}
            }
        }
        self.problem = problem;
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    /// Makes a rotated file, and the directory, readable by their owner only.
    fn private(&self, file: &Path) -> io::Result<()> {
        make_private(file)?;
        make_private_dir(self.path.parent().expect("in a directory"))
    }
}

fn author_key(author: Author) -> &'static str {
    match author {
        Author::Captain => "captain",
        Author::ChiefOfStaff => "chief-of-staff",
        Author::Subagent => "subagent",
        Author::Problem => "problem",
        Author::Tool => "tool",
        Author::App => "app",
    }
}

fn author_from(key: &str) -> Option<Author> {
    Some(match key {
        "captain" => Author::Captain,
        "chief-of-staff" => Author::ChiefOfStaff,
        "subagent" => Author::Subagent,
        "problem" => Author::Problem,
        "tool" => Author::Tool,
        "app" => Author::App,
        _ => return None,
    })
}

/// `text`, cut to [`TEXT_MAX`] bytes on a character boundary.
fn cut(text: &str) -> String {
    if text.len() <= TEXT_MAX {
        return text.to_string();
    }
    let mut end = TEXT_MAX;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\u{2026} (cut)", &text[..end])
}

fn to_json(message: &Message) -> Value {
    let mut out = json!({
        "v": 1,
        "id": message.id,
        "author": author_key(message.author),
        "text": cut(&message.text),
    });
    if let Some(tool) = &message.tool {
        let input = if tool.input.to_string().len() > TEXT_MAX {
            json!({"kept": cut(&crate::approval::subject(&tool.name, &tool.input))})
        } else {
            tool.input.clone()
        };
        let mut kept = json!({"id": tool.id, "name": tool.name, "input": input});
        if let Some(result) = &tool.result {
            let content: Vec<Value> = result.content.iter().map(content_json).collect();
            kept["result"] = json!({"is_error": result.is_error, "content": content});
        }
        out["tool"] = kept;
    }
    out
}

/// A content block as kept: image data never is.
fn content_json(block: &Content) -> Value {
    match block {
        Content::Text(text) => json!({"type": "text", "text": cut(text)}),
        Content::Image { mime, .. } => json!({"type": "image", "mime": mime}),
        Content::Audio { mime } => json!({"type": "audio", "mime": mime}),
        Content::ResourceLink { uri } => json!({"type": "resource_link", "uri": uri}),
        Content::Resource { uri, text } => {
            json!({"type": "resource", "uri": uri, "text": text.as_deref().map(cut)})
        }
        Content::Other(kind) => json!({"type": "other", "name": kind}),
    }
}

fn content_from(value: &Value) -> Option<Content> {
    let text = |key: &str| value[key].as_str().map(str::to_string);
    Some(match value["type"].as_str()? {
        "text" => Content::Text(text("text")?),
        "image" => Content::Image {
            mime: text("mime")?,
            data: Err(NO_IMAGE.into()),
        },
        "audio" => Content::Audio {
            mime: text("mime")?,
        },
        "resource_link" => Content::ResourceLink { uri: text("uri")? },
        "resource" => Content::Resource {
            uri: text("uri")?,
            text: text("text"),
        },
        "other" => Content::Other(text("name")?),
        _ => return None,
    })
}

fn from_json(value: &Value) -> Option<Message> {
    if value["v"] != 1 {
        return None;
    }
    let tool = match &value["tool"] {
        Value::Null => None,
        tool => Some(ToolUse {
            id: tool["id"].as_str()?.to_string(),
            name: tool["name"].as_str()?.to_string(),
            input: tool["input"].clone(),
            result: match &tool["result"] {
                Value::Null => None,
                result => Some(ToolResult {
                    is_error: result["is_error"].as_bool()?,
                    content: result["content"]
                        .as_array()?
                        .iter()
                        .map(content_from)
                        .collect::<Option<_>>()?,
                }),
            },
        }),
    };
    Some(Message {
        id: value["id"]
            .as_str()
            .filter(|id| !id.is_empty())?
            .to_string(),
        author: author_from(value["author"].as_str()?)?,
        text: value["text"].as_str()?.to_string(),
        tool,
        while_working: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn image(bytes: &[u8]) -> Content {
        Content::Image {
            mime: "image/png".into(),
            data: Ok(Arc::from(bytes)),
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/history-tests")
            .join(format!("{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn messages_come_back_with_their_ids_and_without_image_bytes() {
        let dir = scratch("round-trip");
        let mut history = History::new(&dir);
        let said = Message::new(Author::Captain, "What needs me today?");
        let mut tool = Message::tool(ToolUse {
            id: "toolu_1".into(),
            name: "Read".into(),
            input: json!({"file_path": "/tmp/chart.png"}),
            result: Some(ToolResult {
                is_error: false,
                content: vec![image(b"\x89PNG"), Content::Text("done".into())],
            }),
        });
        tool.text = String::new();
        history.record(&said);
        history.record(&tool);
        let kept = fs::read_to_string(history.path()).unwrap();
        assert!(
            !kept.contains("PNG"),
            "image bytes are never written: {kept}"
        );

        let back = History::new(&dir).load();
        assert_eq!(back.len(), 2);
        assert_eq!(back[0], said);
        let restored = back[1].tool.as_ref().unwrap();
        assert_eq!(back[1].id, tool.id);
        assert_eq!(restored.input, json!({"file_path": "/tmp/chart.png"}));
        let content = &restored.result.as_ref().unwrap().content;
        assert!(matches!(&content[0], Content::Image { data: Err(why), .. } if why == NO_IMAGE));
        assert_eq!(content[1], Content::Text("done".into()));
    }

    #[cfg(unix)]
    #[test]
    fn the_file_and_its_directory_are_readable_by_their_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("private");
        fs::create_dir_all(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        let mut history = History::new(&dir);
        history.record(&Message::new(Author::Captain, "hello"));
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(history.path()), 0o600);
        assert_eq!(mode(&dir), 0o700);
    }

    #[cfg(unix)]
    #[test]
    fn a_large_file_is_rotated_privately_and_not_read_back() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("rotate");
        let mut history = History::new(&dir);
        history.record(&Message::new(Author::Captain, "old"));
        let file = fs::OpenOptions::new()
            .append(true)
            .open(history.path())
            .unwrap();
        file.set_len(ROTATE_AT + 1).unwrap();
        assert!(History::new(&dir).load().is_empty());
        let old = suffixed(history.path(), ".1");
        assert_eq!(
            fs::metadata(&old).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(!history.path().exists());
    }

    #[test]
    fn long_text_is_cut_and_unreadable_lines_are_skipped() {
        let dir = scratch("cut");
        let mut history = History::new(&dir);
        history.record(&Message::new(Author::ChiefOfStaff, "é".repeat(TEXT_MAX)));
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(history.path())
            .unwrap();
        writeln!(file, "not json").unwrap();
        writeln!(
            file,
            "{}",
            json!({"v": 1, "author": "captain", "text": "no id"})
        )
        .unwrap();
        history.record(&Message::new(Author::Captain, "after"));
        let back = History::new(&dir).load();
        assert_eq!(back.len(), 2);
        assert!(back[0].text.ends_with("\u{2026} (cut)"));
        assert!(back[0].text.len() < TEXT_MAX + 16);
        assert_eq!(back[1].text, "after");
    }

    #[test]
    fn clearing_deletes_the_file_and_its_rotated_copy() {
        let dir = scratch("clear");
        let mut history = History::new(&dir);
        history.record(&Message::new(Author::Captain, "hello"));
        fs::write(suffixed(history.path(), ".1"), "old\n").unwrap();
        history.clear();
        assert!(!history.path().exists());
        assert!(!suffixed(history.path(), ".1").exists());
        assert!(history.problem().is_none());
        assert!(History::new(&dir).load().is_empty());
    }
}
