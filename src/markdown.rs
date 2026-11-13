//! The chief of staff's Markdown, prepared for Rune's Markdown view (`rune_kit::Markdown`, on
//! pulldown-cmark). Two things differ from CommonMark, both for a chat:
//!
//! - **Line breaks show as written.** CommonMark joins a paragraph's lines with a space, so
//!   "**Captain's Call**" on its own line would run into the sentence under it. Each line that
//!   continues a paragraph gets a hard break (`\` at its end).
//! - **HTML is text.** The view draws no HTML and drops HTML blocks, so a `<tag>` the model
//!   writes would vanish. Each `<` that would start HTML is escaped so it shows as written.
//!   Autolinks (`<https://…>`) keep working.
//!
//! Fenced code is left alone, and so is inline code. Lines indented four or more spaces are left
//! alone too, since they may be indented code, where an escape would show.

/// `source` with its line breaks kept and its HTML shown as text.
pub fn prepare(source: &str) -> String {
    let mut out = String::with_capacity(source.len() + source.len() / 16);
    let mut fence: Option<(char, usize)> = None;
    let mut lines = source.split('\n').peekable();
    while let Some(line) = lines.next() {
        let marker = fence_marker(line);
        let in_code = fence.is_some() || marker.is_some();
        match (fence, marker) {
            (None, Some(m)) => fence = Some(m),
            (Some((c, n)), Some((mc, mn))) if c == mc && mn >= n && line.trim().len() == mn => {
                fence = None
            }
            _ => {}
        }
        if in_code || indent(line) >= 4 {
            out.push_str(line);
        } else {
            escape_html(line, &mut out);
        }
        let Some(next) = lines.peek() else { break };
        if fence.is_none() && !in_code && continues(line, next) {
            out.push('\\');
        }
        out.push('\n');
    }
    out
}

/// Whether `next` continues the paragraph `line` is in, so the break between them is one the
/// writer meant to show.
fn continues(line: &str, next: &str) -> bool {
    let (this, next) = (line.trim(), next.trim());
    if this.is_empty() || next.is_empty() || this.ends_with('\\') || line.ends_with("  ") {
        return false;
    }
    // The line is a block of its own: a heading, a table row or a rule.
    if this.starts_with('#') || this.starts_with('|') || rule(this) {
        return false;
    }
    // The next line starts a block of its own, or underlines this one as a heading.
    !(next.starts_with(['#', '>', '|'])
        || fence_marker(next).is_some()
        || list_item(next)
        || rule(next)
        || next.chars().all(|c| c == '='))
}

fn indent(line: &str) -> usize {
    line.chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .map(|c| if c == '\t' { 4 } else { 1 })
        .sum()
}

/// A bullet (`- `, `* `, `+ `) or a numbered item (`1. `, `1) `).
fn list_item(line: &str) -> bool {
    if let Some(rest) = line.strip_prefix(['-', '*', '+']) {
        return rest.is_empty() || rest.starts_with(' ');
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    let rest = &line[digits..];
    (1..=9).contains(&digits) && (rest.starts_with(". ") || rest.starts_with(") "))
}

/// A thematic break, or a setext underline: three or more `-`, `*` or `_`, with spaces.
fn rule(line: &str) -> bool {
    let marks: String = line.chars().filter(|c| *c != ' ').collect();
    marks.len() >= 3
        && ['-', '*', '_']
            .iter()
            .any(|m| marks.chars().all(|c| c == *m))
}

/// The fence character and length a line opens or closes a code fence with: three or more
/// backticks or tildes, indented at most three spaces.
fn fence_marker(line: &str) -> Option<(char, usize)> {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let c = trimmed.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let n = trimmed.chars().take_while(|x| *x == c).count();
    (n >= 3).then_some((c, n))
}

/// Copies `line` to `out` with a `\` before each `<` that would start HTML, outside inline code.
fn escape_html(line: &str, out: &mut String) {
    let mut code: Option<usize> = None;
    let mut rest = line;
    while let Some(c) = rest.chars().next() {
        if c == '`' {
            let run = rest.chars().take_while(|x| *x == '`').count();
            code = match code {
                Some(open) if open == run => None,
                None if rest[run..].contains(&"`".repeat(run)) => Some(run),
                other => other,
            };
            out.push_str(&rest[..run]);
            rest = &rest[run..];
            continue;
        }
        if c == '<' && code.is_none() && starts_html(&rest[1..]) && !autolink(rest) {
            out.push('\\');
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
}

/// Whether the text after a `<` opens a tag, a closing tag, a comment or a declaration.
fn starts_html(after: &str) -> bool {
    after
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, '/' | '!' | '?'))
}

/// Whether `text`, which starts with `<`, starts a CommonMark autolink: `<scheme:…>` or an email
/// address in angle brackets.
fn autolink(text: &str) -> bool {
    let Some(end) = text.find('>') else {
        return false;
    };
    let inner = &text[1..end];
    if inner.is_empty() || inner.contains([' ', '<']) {
        return false;
    }
    if let Some((scheme, _)) = inner.split_once(':') {
        let ok = (2..=32).contains(&scheme.len())
            && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'));
        if ok {
            return true;
        }
    }
    inner
        .split_once('@')
        .is_some_and(|(user, host)| !user.is_empty() && host.contains('.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_paragraph_keeps_its_line_breaks() {
        assert_eq!(
            prepare("**Captain's Call**\nNo decision is recorded."),
            "**Captain's Call**\\\nNo decision is recorded."
        );
        // Blank lines, and lines already ending in a break, are left as they are.
        assert_eq!(prepare("one\n\ntwo"), "one\n\ntwo");
        assert_eq!(prepare("one  \ntwo"), "one  \ntwo");
        assert_eq!(prepare("one\\\ntwo"), "one\\\ntwo");
    }

    #[test]
    fn blocks_of_their_own_get_no_break() {
        for src in [
            "# Heading\ntext",
            "text\n# Heading",
            "- one\n- two",
            "text\n1. one",
            "text\n> quote",
            "| a | b |\n| - | - |",
            "Title\n===",
            "Title\n---",
            "text\n```\ncode\n```",
        ] {
            assert_eq!(prepare(src), src);
        }
        assert_eq!(prepare("- one\n  more"), "- one\\\n  more");
    }

    #[test]
    fn code_is_left_alone() {
        let fenced = "```\na\nb <b>\n```\nafter\nthis";
        assert_eq!(prepare(fenced), "```\na\nb <b>\n```\nafter\\\nthis");
        let tildes = "~~~~\n```\nstill code\n~~~~";
        assert_eq!(prepare(tildes), tildes);
        assert_eq!(prepare("Use `<div>` here"), "Use `<div>` here");
        assert_eq!(prepare("    <indented>"), "    <indented>");
    }

    #[test]
    fn html_shows_as_text_and_autolinks_still_link() {
        assert_eq!(
            prepare("<script>alert(1)</script> and <b>this</b>"),
            "\\<script>alert(1)\\</script> and \\<b>this\\</b>"
        );
        assert_eq!(prepare("<!-- note -->"), "\\<!-- note -->");
        assert_eq!(prepare("a < b and 2<3"), "a < b and 2<3");
        assert_eq!(
            prepare("see <https://example.com> or <me@example.com>"),
            "see <https://example.com> or <me@example.com>"
        );
        // An unclosed backtick is not code, so what follows it is still escaped.
        assert_eq!(prepare("a ` <b>"), "a ` \\<b>");
    }
}
