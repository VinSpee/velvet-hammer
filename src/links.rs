//! Message links: every chat message has a link, `velvet-hammer://message/<id>`, where `<id>` is
//! the message's id (see [`crate::model::Message::id`]). A link in a message draws as a chip
//! naming the message it points to, and pressing it scrolls the chat there. When the captain's
//! message holds links, the chief of staff gets each linked message quoted after the captain's
//! words, so "do what this answer says: <link>" works (see [docs/links.md](../docs/links.md)).

use std::ops::Range;

use crate::model::{Author, Message, Workspace};
use crate::tools;

/// What every message link starts with.
pub const PREFIX: &str = "velvet-hammer://message/";
/// The longest id a link may carry.
const ID_MAX: usize = 64;
/// The most of one linked message quoted to the chief of staff.
const QUOTE_MAX: usize = 4000;
/// The most links in one message quoted to the chief of staff.
const QUOTES_MAX: usize = 5;
/// The most of a message's first line a chip shows.
const CHIP_MAX: usize = 60;

/// What the chief of staff is told once per session, with its first message (see
/// [`crate::bridge`]). It is phrased as facts about the chat, as Claude Code's hooks documentation
/// advises for added context.
pub const INSTRUCTION: &str = "Velvet Hammer gives every message in this chat a link of the form \
    velvet-hammer://message/<id>. Each of the captain's messages ends with a line from Velvet \
    Hammer giving that message's link and the link of your last reply. When the captain's message \
    contains links, Velvet Hammer quotes each linked message after the captain's words, between \
    <<< and >>>, saying who wrote it, or says that no message has that link. To point the captain \
    at an earlier message, write its link as plain text: the chat shows it as a chip the captain \
    can press to go to that message. Only links Velvet Hammer has given you are valid.";

/// A message's link.
pub fn url(id: &str) -> String {
    format!("{PREFIX}{id}")
}

fn id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

/// The id a link names, when `text` is exactly one message link.
pub fn parse(text: &str) -> Option<&str> {
    let id = text.strip_prefix(PREFIX)?;
    (!id.is_empty() && id.len() <= ID_MAX && id.chars().all(id_char)).then_some(id)
}

/// Each message link in `text`: where it is, and the id it names.
pub fn find(text: &str) -> Vec<(Range<usize>, &str)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(at) = text[from..].find(PREFIX).map(|at| from + at) {
        let start = at + PREFIX.len();
        let len = text[start..]
            .find(|c: char| !id_char(c))
            .unwrap_or(text.len() - start);
        from = start + len;
        if (1..=ID_MAX).contains(&len) {
            out.push((at..from, &text[start..from]));
        }
    }
    out
}

/// Who wrote a message, as a chip names them.
fn who(m: &Message) -> String {
    match (m.author, &m.tool) {
        (_, Some(tool)) => tool.name.clone(),
        (Author::Captain, _) => "You".into(),
        (Author::ChiefOfStaff, _) => "Chief of staff".into(),
        (Author::Subagent, _) => "Subagent".into(),
        (Author::Problem, _) => "Problem".into(),
        (Author::App, _) => "Velvet Hammer".into(),
        (Author::Tool, None) => "Tool".into(),
    }
}

/// A message as text: its words, or a tool use's name, main argument and result.
fn words(m: &Message) -> String {
    match &m.tool {
        Some(tool) => {
            let result = tool.result.as_ref().map(tools::result_text);
            [tool.name.clone(), tools::summary(tool)]
                .into_iter()
                .chain(result)
                .filter(|w| !w.is_empty())
                .collect::<Vec<_>>()
                .join("\n")
        }
        None => m.text.clone(),
    }
}

/// `text` cut to `max` characters, with an ellipsis when it was.
fn cut(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((end, _)) => format!("{}\u{2026}", text[..end].trim_end()),
        None => text.to_string(),
    }
}

/// What a chip for a link to `id` says: who wrote the message and its first line, or that it is
/// not in the chat.
pub fn chip(ws: &Workspace, id: &str) -> String {
    match ws.message(id) {
        Some((_, m)) => {
            let words = words(m);
            let first = words.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
            // Markdown markers and the chip's own arrows would only clutter a chip.
            let first: String = first
                .chars()
                .filter(|c| !matches!(c, '*' | '`' | '#'))
                .collect();
            format!("\u{21aa} {}: {}", who(m), cut(first.trim(), CHIP_MAX))
        }
        None => "\u{26a0} Message not found".into(),
    }
}

/// A link drawn as a chip in a message shown as written.
pub struct Chip {
    /// The chip's text in the drawn message.
    pub range: Range<usize>,
    pub url: String,
    pub found: bool,
}

/// A message shown as written, with each link replaced by its chip's words.
pub fn display(text: &str, ws: &Workspace) -> (String, Vec<Chip>) {
    let mut out = String::with_capacity(text.len());
    let mut chips = Vec::new();
    let mut from = 0;
    for (range, id) in find(text) {
        out.push_str(&text[from..range.start]);
        let words = chip(ws, id);
        let start = out.len();
        out.push_str(&words);
        chips.push(Chip {
            range: start..out.len(),
            url: url(id),
            found: ws.message(id).is_some(),
        });
        from = range.end;
    }
    out.push_str(&text[from..]);
    (out, chips)
}

/// The chief of staff's Markdown with each bare link written as a Markdown link whose text is
/// its chip's words, so it draws as a chip. Links already in a Markdown link or in code are left
/// as written.
pub fn markdown(source: &str, ws: &Workspace) -> String {
    if !source.contains(PREFIX) {
        return source.to_string();
    }
    let mut out = String::with_capacity(source.len());
    let mut fenced = false;
    for (n, line) in source.split('\n').enumerate() {
        if n > 0 {
            out.push('\n');
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
        }
        if fenced || trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            out.push_str(line);
            continue;
        }
        let mut from = 0;
        for (range, id) in find(line) {
            let before = &line[..range.start];
            let in_code = before.matches('`').count() % 2 == 1;
            let in_link = before.ends_with('(') || before.ends_with('<');
            if in_code || in_link {
                continue;
            }
            out.push_str(&line[from..range.start]);
            out.push('[');
            out.push_str(&escape(&chip(ws, id)));
            out.push_str("](");
            out.push_str(&url(id));
            out.push(')');
            from = range.end;
        }
        out.push_str(&line[from..]);
    }
    out
}

/// `text` with the characters Markdown would read as markup escaped.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(
            c,
            '\\' | '[' | ']' | '*' | '_' | '`' | '<' | '>' | '~' | '|' | '#'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Who wrote a message, as the chief of staff is told.
fn writer(m: &Message) -> String {
    match (m.author, &m.tool) {
        (_, Some(tool)) => format!("your tool use {}", tool.name),
        (Author::Captain, _) => "the captain".into(),
        (Author::ChiefOfStaff, _) => "you, the chief of staff".into(),
        (Author::Subagent, _) => "a subagent".into(),
        (Author::Problem, _) => "Claude Code, reporting a problem".into(),
        (Author::App, _) => "Velvet Hammer".into(),
        (Author::Tool, None) => "a tool use".into(),
    }
}

/// Each message `text` links to, quoted for the chief of staff, or `None` when it links to none.
/// Only the linked messages are quoted, each cut to [`QUOTE_MAX`] characters, and at most
/// [`QUOTES_MAX`] of them.
pub fn quotes(ws: &Workspace, text: &str) -> Option<String> {
    let mut ids: Vec<&str> = Vec::new();
    for (_, id) in find(text) {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    if ids.is_empty() {
        return None;
    }
    let mut out = String::from("[Linked messages, quoted by Velvet Hammer]");
    for id in ids.iter().take(QUOTES_MAX) {
        let link = url(id);
        match ws.message(id) {
            Some((_, m)) => out.push_str(&format!(
                "\n<<< {link}, written by {}:\n{}\n>>>",
                writer(m),
                cut(&words(m), QUOTE_MAX)
            )),
            None => out.push_str(&format!(
                "\n<<< {link}: message not found. No message in this chat has this link. >>>"
            )),
        }
    }
    if ids.len() > QUOTES_MAX {
        out.push_str(&format!(
            "\n({} more links are not quoted.)",
            ids.len() - QUOTES_MAX
        ));
    }
    Some(out)
}

/// What the chief of staff is sent for the captain's newest message, `text`: the captain's words,
/// the messages they link to, and a line giving the message's own link and the link of the chief
/// of staff's last reply in this session. Nothing else from the conversation is added.
pub fn outgoing(ws: &Workspace, text: &str) -> String {
    let mut out = text.to_string();
    if let Some(quotes) = quotes(ws, text) {
        out.push_str("\n\n");
        out.push_str(&quotes);
    }
    let mine = ws
        .chat
        .last()
        .filter(|m| m.author == Author::Captain)
        .map(|m| url(&m.id));
    let reply = ws.chat[ws.earlier.min(ws.chat.len())..]
        .iter()
        .rev()
        .find(|m| m.author == Author::ChiefOfStaff)
        .map(|m| url(&m.id));
    if let Some(mine) = mine {
        out.push_str(&format!(
            "\n\n[Velvet Hammer: this message's link is {mine}."
        ));
        if let Some(reply) = reply {
            out.push_str(&format!(" Your last reply's link is {reply}."));
        }
        out.push(']');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws(messages: Vec<Message>) -> Workspace {
        Workspace {
            chat: messages,
            ..Workspace::default()
        }
    }

    #[test]
    fn a_link_names_one_message_by_its_id() {
        assert_eq!(
            parse("velvet-hammer://message/3f9a01bc22de"),
            Some("3f9a01bc22de")
        );
        assert_eq!(parse("velvet-hammer://message/"), None);
        assert_eq!(parse("velvet-hammer://message/a b"), None);
        assert_eq!(parse("https://example.com"), None);
        let text = "see velvet-hammer://message/abc, then velvet-hammer://message/def.";
        let found: Vec<_> = find(text).into_iter().map(|(_, id)| id).collect();
        assert_eq!(found, ["abc", "def"]);
        assert_eq!(
            &text[find(text)[0].0.clone()],
            "velvet-hammer://message/abc"
        );
    }

    #[test]
    fn a_link_shows_as_a_chip_naming_its_message_or_saying_it_is_missing() {
        let answer = Message::new(
            Author::ChiefOfStaff,
            "**Plan:** merge the docs PR, then tag the release.\nMore below.",
        );
        let ws = ws(vec![answer.clone()]);
        let text = format!("do what this says: {} and {}", url(&answer.id), url("gone"));
        let (shown, chips) = display(&text, &ws);
        assert_eq!(
            shown,
            "do what this says: \u{21aa} Chief of staff: Plan: merge the docs PR, then tag the \
             release. and \u{26a0} Message not found"
        );
        assert!(chips[0].found && !chips[1].found);
        assert_eq!(&shown[chips[1].range.clone()], "\u{26a0} Message not found");
        assert_eq!(chips[0].url, url(&answer.id));
    }

    #[test]
    fn a_bare_link_in_markdown_becomes_a_link_but_code_and_links_stay() {
        let answer = Message::new(Author::Captain, "Ship [it] now");
        let ws = ws(vec![answer.clone()]);
        let link = url(&answer.id);
        let source = format!("As you asked in {link}.\n`{link}`\n[here]({link})\n```\n{link}\n```");
        assert_eq!(
            markdown(&source, &ws),
            format!(
                "As you asked in [\u{21aa} You: Ship \\[it\\] now]({link}).\n`{link}`\n\
                 [here]({link})\n```\n{link}\n```"
            )
        );
    }

    #[test]
    fn the_chief_of_staff_gets_only_the_linked_messages_quoted_and_bounded() {
        let earlier = Message::new(Author::ChiefOfStaff, "Earlier: the secret plan.");
        let answer = Message::new(
            Author::ChiefOfStaff,
            format!("Step one.\n{}", "x".repeat(5000)),
        );
        let other = Message::new(Author::ChiefOfStaff, "Unrelated reply.");
        let text = format!(
            "do what this answer says: {} {}",
            url(&answer.id),
            url("nope")
        );
        let said = Message::new(Author::Captain, text.clone());
        let ws = Workspace {
            chat: vec![earlier, answer.clone(), other.clone(), said.clone()],
            earlier: 1,
            ..Workspace::default()
        };
        let sent = outgoing(&ws, &text);
        assert!(sent.starts_with(&text));
        assert!(sent.contains(&format!(
            "<<< {}, written by you, the chief of staff:\nStep one.",
            url(&answer.id)
        )));
        assert!(sent.contains(&format!("<<< {}: message not found.", url("nope"))));
        assert!(
            sent.len() < text.len() + QUOTE_MAX + 600,
            "bounded: {}",
            sent.len()
        );
        assert!(
            !sent.contains("secret plan"),
            "nothing unlinked from history"
        );
        assert!(!sent.contains("Unrelated reply"));
        assert!(sent.ends_with(&format!(
            "[Velvet Hammer: this message's link is {}. Your last reply's link is {}.]",
            url(&said.id),
            url(&other.id)
        )));
    }

    #[test]
    fn a_message_without_links_gets_only_its_own_link_and_never_an_earlier_sessions_reply() {
        let earlier = Message::new(Author::ChiefOfStaff, "From yesterday.");
        let said = Message::new(Author::Captain, "Hello");
        let ws = Workspace {
            chat: vec![earlier, said.clone()],
            earlier: 1,
            ..Workspace::default()
        };
        assert_eq!(
            outgoing(&ws, "Hello"),
            format!(
                "Hello\n\n[Velvet Hammer: this message's link is {}.]",
                url(&said.id)
            )
        );
        assert_eq!(quotes(&ws, "Hello"), None);
    }

    #[test]
    fn at_most_five_links_are_quoted() {
        let messages: Vec<_> = (0..7)
            .map(|i| Message::new(Author::Captain, format!("m{i}")))
            .collect();
        let text: Vec<_> = messages.iter().map(|m| url(&m.id)).collect();
        let quoted = quotes(&ws(messages), &text.join(" ")).unwrap();
        assert_eq!(quoted.matches("<<<").count(), 5);
        assert!(quoted.ends_with("(2 more links are not quoted.)"));
    }
}
