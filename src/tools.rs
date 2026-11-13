//! The chief of staff's tool uses as the chat shows them, and the handler registry that picks how
//! each one is drawn. The keys are the open standards' types, not a list of our own (see
//! [docs/tool-calls.md](../docs/tool-calls.md)): a result's content blocks are the Model Context
//! Protocol's (text, image, audio, resource link, embedded resource), and a tool's kind is the
//! Agent Client Protocol's `ToolKind` (read, edit, execute and so on). Claude Code's tool names
//! map onto those kinds. Video is not a standard content type; it is a file or resource link whose
//! media type is video. Anything the registry does not know falls back to the generic handler,
//! so no tool use disappears from the chat.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine;
use serde_json::Value;

/// The largest image or video file the chat reads, and the largest image a result may carry.
/// Rune's decoder caps the pixel size on top of this.
pub const MAX_MEDIA_BYTES: u64 = 32 * 1024 * 1024;

/// One tool use of the chief of staff: what it called, and what came back once it has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolUse {
    /// Claude Code's `tool_use` id, which its result names.
    pub id: String,
    pub name: String,
    pub input: Value,
    pub result: Option<ToolResult>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolResult {
    pub is_error: bool,
    pub content: Vec<Content>,
}

/// A content block of a tool result, by the Model Context Protocol's content types (the Agent
/// Client Protocol uses the same blocks). Claude Code reports Anthropic's shapes (an image's
/// `source.media_type`); MCP's (`mimeType`) are read too.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Content {
    Text(String),
    /// The decoded bytes, or why they could not be used.
    Image {
        mime: String,
        data: Result<Arc<[u8]>, String>,
    },
    Audio {
        mime: String,
    },
    ResourceLink {
        uri: String,
        mime: Option<String>,
    },
    /// An embedded resource, with its text when it has text rather than a blob.
    Resource {
        uri: String,
        mime: Option<String>,
        text: Option<String>,
    },
    /// A type none of the standards above define, by its `type`.
    Other(String),
}

/// What a tool does, as the Agent Client Protocol's `ToolKind` names it. ACP also defines
/// `delete` and `move`, which none of Claude Code's tools are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Read,
    Edit,
    Search,
    Execute,
    Think,
    Fetch,
    SwitchMode,
    Other,
}

impl Kind {
    /// The kind of one of Claude Code's tools. Tools it does not know, such as MCP tools, are
    /// [`Kind::Other`].
    pub fn of(tool: &str) -> Kind {
        match tool {
            "Read" | "NotebookRead" => Kind::Read,
            "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => Kind::Edit,
            "Bash" | "BashOutput" | "KillShell" | "KillBash" => Kind::Execute,
            "Grep" | "Glob" | "LS" | "ToolSearch" => Kind::Search,
            "WebFetch" | "WebSearch" => Kind::Fetch,
            "TodoWrite" => Kind::Think,
            "ExitPlanMode" | "EnterPlanMode" => Kind::SwitchMode,
            _ => Kind::Other,
        }
    }
}

/// How the chat draws a tool use. Image and video have their own views; audio, diff and terminal
/// are registered for the next handlers and draw as [`Handler::Generic`] until they have theirs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Handler {
    /// An image to show inline: a result's image block, or an image file the tool named.
    Image(Picture),
    /// A video file the tool named or linked.
    Video(PathBuf),
    /// MCP audio content.
    Audio,
    /// An ACP `diff`: an edit tool's change.
    Diff,
    /// An ACP `terminal`: a command's output.
    Terminal,
    /// The tool's name, a short summary of its input and its result, folded.
    Generic,
}

/// Where an image's pixels come from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Picture {
    /// An image block in the result: its decoded bytes, or why they could not be used.
    Block {
        mime: String,
        data: Result<Arc<[u8]>, String>,
    },
    /// An image file the tool use named.
    File(PathBuf),
}

/// The registry: picks the handler for `tool` by its result's content types, the media type of
/// a file it named, and its kind, in that order. A tool use still running is generic.
pub fn handler(tool: &ToolUse) -> Handler {
    let Some(result) = &tool.result else {
        return Handler::Generic;
    };
    for block in &result.content {
        match block {
            Content::Image { mime, data } => {
                return Handler::Image(Picture::Block {
                    mime: mime.clone(),
                    data: data.clone(),
                });
            }
            Content::Audio { .. } => return Handler::Audio,
            Content::ResourceLink { uri, mime } | Content::Resource { uri, mime, .. } => {
                if let Some(handler) = file_uri(uri).and_then(|p| media_file(p, mime.as_deref())) {
                    return handler;
                }
            }
            Content::Text(_) | Content::Other(_) => {}
        }
    }
    if let Some(handler) = named_path(&tool.input).and_then(|p| media_file(p, None)) {
        return handler;
    }
    match Kind::of(&tool.name) {
        Kind::Edit => Handler::Diff,
        Kind::Execute => Handler::Terminal,
        _ => Handler::Generic,
    }
}

/// The handler for a media file the tool use named, when it is an image or a video file that
/// exists and is not too large to read.
fn media_file(path: PathBuf, mime: Option<&str>) -> Option<Handler> {
    let media = mime
        .map(str::to_ascii_lowercase)
        .or_else(|| extension_type(&path).map(str::to_string))?;
    let handler = if media.starts_with("image/") {
        Handler::Image(Picture::File(path.clone()))
    } else if media.starts_with("video/") {
        Handler::Video(path.clone())
    } else {
        return None;
    };
    // A missing file, or one that is not a regular file, is not media to show.
    std::fs::metadata(&path)
        .is_ok_and(|m| m.is_file())
        .then_some(handler)
}

/// The media type of an image or video file, by its extension.
fn extension_type(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" | "heif" => "image/heic",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        _ => return None,
    })
}

/// The absolute file path a tool use's input names, in the fields Claude Code's file tools use.
/// A relative path is left alone: the app does not guess what it is relative to.
fn named_path(input: &Value) -> Option<PathBuf> {
    ["file_path", "path", "notebook_path"]
        .iter()
        .find_map(|field| input[field].as_str())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

/// The path of a `file://` URI.
fn file_uri(uri: &str) -> Option<PathBuf> {
    let path = uri.strip_prefix("file://")?;
    Some(PathBuf::from(percent_decode(path))).filter(|p| p.is_absolute())
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (bytes[i], bytes.get(i + 1), bytes.get(i + 2)) {
            (b'%', Some(&a), Some(&b)) if hex(a).is_some() && hex(b).is_some() => {
                out.push((hex(a).unwrap() * 16 + hex(b).unwrap()) as u8);
                i += 3;
            }
            (b, ..) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A tool's input in a few words: its main argument (the command, file path and so on), cut to
/// one line.
pub fn summary(tool: &ToolUse) -> String {
    const MAX: usize = 120;
    let subject = crate::approval::subject(&tool.name, &tool.input);
    let line = subject.lines().next().unwrap_or_default();
    let cut = line.char_indices().nth(MAX).map(|(at, _)| at);
    match cut {
        Some(at) => format!("{}\u{2026}", &line[..at]),
        None if subject.lines().nth(1).is_some() => format!("{line}\u{2026}"),
        None => line.to_string(),
    }
}

/// A result as text, for the generic handler: each text block, and a line naming each other one.
pub fn result_text(result: &ToolResult) -> String {
    let lines: Vec<String> = result
        .content
        .iter()
        .map(|block| match block {
            Content::Text(text) => text.clone(),
            Content::Image { mime, .. } => format!("[image, {mime}]"),
            Content::Audio { mime } => format!("[audio, {mime}]"),
            Content::ResourceLink { uri, .. } => format!("[link to {uri}]"),
            Content::Resource {
                text: Some(text), ..
            } => text.clone(),
            Content::Resource { uri, .. } => format!("[resource {uri}]"),
            Content::Other(kind) => format!("[{kind} content]"),
        })
        .collect();
    lines.join("\n")
}

/// The `tool_use` blocks of an `assistant` message's content.
pub fn uses(content: &Value) -> Vec<ToolUse> {
    blocks(content)
        .filter(|b| b["type"] == "tool_use")
        .filter_map(|b| {
            Some(ToolUse {
                id: b["id"].as_str()?.to_string(),
                name: b["name"].as_str()?.to_string(),
                input: b["input"].clone(),
                result: None,
            })
        })
        .collect()
}

/// The `tool_result` blocks of a `user` message's content, by the id of the tool use each
/// answers.
pub fn results(content: &Value) -> Vec<(String, ToolResult)> {
    blocks(content)
        .filter(|b| b["type"] == "tool_result")
        .filter_map(|b| {
            let content = match &b["content"] {
                Value::String(text) => vec![Content::Text(text.clone())],
                Value::Array(items) => items.iter().map(content_block).collect(),
                _ => Vec::new(),
            };
            Some((
                b["tool_use_id"].as_str()?.to_string(),
                ToolResult {
                    is_error: b["is_error"] == true,
                    content,
                },
            ))
        })
        .collect()
}

fn blocks(content: &Value) -> impl Iterator<Item = &Value> {
    content.as_array().into_iter().flatten()
}

/// One content block, in Anthropic's shape or MCP's.
fn content_block(block: &Value) -> Content {
    let text = |field: &str| block[field].as_str().map(str::to_string);
    match block["type"].as_str().unwrap_or_default() {
        "text" => Content::Text(text("text").unwrap_or_default()),
        "image" => {
            let source = &block["source"];
            let mime = source["media_type"]
                .as_str()
                .or(block["mimeType"].as_str())
                .unwrap_or("image")
                .to_string();
            let data = match source["data"].as_str().or(block["data"].as_str()) {
                Some(data) => decode_image(data),
                None => Err("The image block has no data.".into()),
            };
            Content::Image { mime, data }
        }
        "audio" => Content::Audio {
            mime: text("mimeType").unwrap_or_else(|| "audio".into()),
        },
        "resource_link" => Content::ResourceLink {
            uri: text("uri").unwrap_or_default(),
            mime: text("mimeType"),
        },
        "resource" => {
            let resource = &block["resource"];
            Content::Resource {
                uri: resource["uri"].as_str().unwrap_or_default().to_string(),
                mime: resource["mimeType"].as_str().map(str::to_string),
                text: resource["text"].as_str().map(str::to_string),
            }
        }
        other => Content::Other(other.to_string()),
    }
}

/// An image block's base64 data, refused past [`MAX_MEDIA_BYTES`] before it is decoded.
fn decode_image(data: &str) -> Result<Arc<[u8]>, String> {
    if data.len() as u64 / 4 * 3 > MAX_MEDIA_BYTES {
        return Err(format!(
            "The image is larger than {} MB, so it is not shown.",
            MAX_MEDIA_BYTES / (1024 * 1024)
        ));
    }
    base64::engine::general_purpose::STANDARD
        .decode(data.trim())
        .map(Arc::from)
        .map_err(|e| format!("The image data could not be read: {e}."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A 1x1 PNG.
    const PIXEL: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

    fn used(name: &str, input: Value, content: Value) -> ToolUse {
        let mut tool =
            uses(&json!([{"type": "tool_use", "id": "t1", "name": name, "input": input}]))
                .pop()
                .unwrap();
        let (id, result) =
            results(&json!([{"type": "tool_result", "tool_use_id": "t1", "content": content}]))
                .pop()
                .unwrap();
        assert_eq!(id, tool.id);
        tool.result = Some(result);
        tool
    }

    fn scratch(name: &str, bytes: &[u8]) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/tools-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn an_image_block_in_anthropic_or_mcp_shape_is_an_image() {
        let anthropic = json!([{"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": PIXEL}}]);
        let mcp = json!([{"type": "image", "mimeType": "image/png", "data": PIXEL}]);
        for content in [anthropic, mcp] {
            let Handler::Image(Picture::Block { mime, data }) =
                handler(&used("Read", json!({"file_path": "/x.png"}), content))
            else {
                panic!("not an image");
            };
            assert_eq!(mime, "image/png");
            assert!(data.unwrap().starts_with(b"\x89PNG"));
        }
    }

    #[test]
    fn a_bad_or_huge_image_block_is_still_an_image_with_a_reason() {
        let bad = json!([{"type": "image", "source": {"media_type": "image/png", "data": "!!"}}]);
        let Handler::Image(Picture::Block { data, .. }) = handler(&used("Read", json!({}), bad))
        else {
            panic!("not an image");
        };
        assert!(data.is_err());
        let huge = "A".repeat(MAX_MEDIA_BYTES as usize / 3 * 4 + 8);
        assert!(decode_image(&huge).unwrap_err().contains("larger than"));
    }

    #[test]
    fn a_named_image_or_video_file_that_exists_is_media() {
        let png = scratch("shot.png", b"not really a png");
        let mov = scratch("clip.MOV", b"not really a movie");
        let tool = used("Write", json!({"file_path": png}), json!("written"));
        assert_eq!(handler(&tool), Handler::Image(Picture::File(png)));
        let tool = used("Read", json!({"file_path": mov}), json!("binary file"));
        assert_eq!(handler(&tool), Handler::Video(mov));
        let link = json!([{"type": "resource_link", "uri": format!("file://{}", scratch("a b.webm", b"x").display()).replace(' ', "%20"), "name": "a b.webm"}]);
        assert!(matches!(
            handler(&used("mcp__x__record", json!({}), link)),
            Handler::Video(_)
        ));
    }

    #[test]
    fn missing_relative_or_unknown_files_are_not_media() {
        let missing = used(
            "Read",
            json!({"file_path": "/nonexistent/a.png"}),
            json!("no"),
        );
        assert_eq!(handler(&missing), Handler::Generic);
        let relative = used("Read", json!({"file_path": "a.png"}), json!("ok"));
        assert_eq!(handler(&relative), Handler::Generic);
        let text = used(
            "Read",
            json!({"file_path": scratch("notes.txt", b"hi")}),
            json!("hi"),
        );
        assert_eq!(handler(&text), Handler::Generic);
    }

    #[test]
    fn kinds_and_other_content_pick_the_registered_handlers() {
        assert_eq!(
            handler(&used("Edit", json!({}), json!("ok"))),
            Handler::Diff
        );
        assert_eq!(
            handler(&used("Bash", json!({"command": "ls"}), json!("a"))),
            Handler::Terminal
        );
        let audio = json!([{"type": "audio", "mimeType": "audio/wav", "data": ""}]);
        assert_eq!(
            handler(&used("mcp__x__say", json!({}), audio)),
            Handler::Audio
        );
        let odd = used("mcp__x__y", json!({}), json!([{"type": "hologram"}]));
        assert_eq!(handler(&odd), Handler::Generic);
        assert_eq!(
            result_text(odd.result.as_ref().unwrap()),
            "[hologram content]"
        );
        let running = ToolUse {
            result: None,
            ..odd
        };
        assert_eq!(handler(&running), Handler::Generic);
        assert_eq!(Kind::of("Grep"), Kind::Search);
        assert_eq!(Kind::of("mcp__velvet-hammer__app_note"), Kind::Other);
    }

    #[test]
    fn the_summary_is_one_short_line() {
        let tool = used("Bash", json!({"command": "echo one\necho two"}), json!(""));
        assert_eq!(summary(&tool), "echo one\u{2026}");
        let long = used("Bash", json!({"command": "x".repeat(300)}), json!(""));
        assert_eq!(summary(&long).chars().count(), 121);
    }
}
