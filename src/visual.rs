//! Visual replies: the chief of staff shows a reply that benefits from a picture as a json-render
//! view (the app's `app_ui` tool, see [`crate::ui`]) instead of as text. Two hooks carry it, both
//! served by [`crate::bridge`]:
//!
//! - a `UserPromptSubmit` hook adds [`INSTRUCTION`] to the session with each of the captain's
//!   messages, so the model reaches for the tool by default ([`Mode::Nudge`] and
//!   [`Mode::Enforce`]);
//! - a `Stop` hook checks each final reply with [`shape`] and, when a reply of a qualifying shape
//!   arrives with no view in its turn (for a question with a fixed set of answers, no `Choice`),
//!   sends the model feedback asking for one
//!   ([`Mode::Enforce`] only). It asks at most once per reply: never while Claude Code is
//!   already continuing because of a stop hook (`stop_hook_active`), and never twice in a row
//!   ([`Turn`]), so it cannot loop or hang.
//!
//! Which replies qualify is decided here, by the plain rules in [`shape`] and nowhere else.

use serde_json::{Value, json};

use crate::store::{self, Saved};

/// The settings file in the data directory.
pub const SETTINGS_FILE: &str = "visual-replies.json";

/// How the app steers the chief of staff towards views.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// Nothing is added and nothing is checked.
    Off,
    /// The standing instruction only.
    Nudge,
    /// The standing instruction, and a reply of a qualifying shape without a view is sent back
    /// once for one. The recommended default.
    #[default]
    Enforce,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Enforce, Mode::Nudge, Mode::Off];
    pub const RECOMMENDED: Mode = Mode::Enforce;

    pub fn label(self) -> &'static str {
        match self {
            Mode::Off => "Off",
            Mode::Nudge => "Nudge only",
            Mode::Enforce => "Enforce",
        }
    }

    /// The trade-off in one sentence.
    pub fn explanation(self) -> &'static str {
        match self {
            Mode::Off => "The chief of staff decides for itself when to show a view.",
            Mode::Nudge => {
                "Each of your messages reminds the chief of staff to show digests, lists, \
                 choices and tables as views. Nothing checks its replies."
            }
            Mode::Enforce => {
                "The same reminder, and a reply of one of the shapes below that arrives as text \
                 only is sent back once to be shown as a view. A plain answer stays plain text."
            }
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Mode::Off => "off",
            Mode::Nudge => "nudge",
            Mode::Enforce => "enforce",
        }
    }

    fn from_key(key: &str) -> Option<Mode> {
        Mode::ALL.into_iter().find(|m| m.key() == key)
    }

    /// The standing instruction the `UserPromptSubmit` hook adds, if any.
    pub fn instruction(self) -> Option<&'static str> {
        (self != Mode::Off).then_some(INSTRUCTION)
    }
}

impl Saved for Mode {
    const FILE: &'static str = SETTINGS_FILE;
    const FALLBACK: &'static str = "replies are only nudged towards views, not checked";

    fn fallback() -> Self {
        Mode::Nudge
    }

    fn to_json(&self) -> Value {
        json!({"version": store::VERSION, "mode": self.key()})
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        store::check_version(value)?;
        value["mode"]
            .as_str()
            .and_then(Mode::from_key)
            .ok_or_else(|| "its mode is not one Velvet Hammer knows".into())
    }
}

/// The standing instruction. It is phrased as facts about the chat, as Claude Code's hooks
/// documentation advises for added context.
pub const INSTRUCTION: &str = "Velvet Hammer shows this chat to the captain and draws views \
    from the mcp__velvet-hammer__app_ui tool natively. The captain reads a reply better as a \
    view when it is a status digest, a list of items that need a decision, a pull request review \
    or merge ask, a table, or progress: such a reply is shown with app_ui (a json-render spec), \
    with only a short sentence of text around it. A question with a fixed set of answers (yes or \
    no, pick one of several, pick several) is asked with a Choice element in app_ui, with the \
    recommended option marked, and is not also asked or answered in text; the captain's pick \
    comes back as the option's exact value. A plain conversational answer stays plain text.";

/// A reply's shape that reads better as a view. [`shape`] finds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shape {
    /// A Markdown table.
    Table,
    /// Two or more list items and a question: options to choose from.
    Choices(usize),
    /// A question with a fixed set of answers: yes or no, this or that, or a reply to say in
    /// quotes.
    Question,
    /// A pull request link with the words merge, review or approve.
    PullRequest,
    /// Two or more headings, as a status digest has.
    Sections(usize),
    /// Three or more list items.
    List(usize),
}

impl Shape {
    /// What the reply has, in words: "lists 4 items".
    pub fn describe(&self) -> String {
        match self {
            Shape::Table => "has a table".into(),
            Shape::Choices(n) => format!("offers {n} options to choose from"),
            Shape::Question => "asks a question with a fixed set of answers".into(),
            Shape::PullRequest => "asks about a pull request".into(),
            Shape::Sections(n) => format!("has {n} headed sections, like a status digest"),
            Shape::List(n) => format!("lists {n} items"),
        }
    }

    /// Whether only a `Choice` answers this shape, rather than any view.
    pub fn asks(&self) -> bool {
        matches!(self, Shape::Choices(_) | Shape::Question)
    }
}

/// The reply the settings page starts its example with.
pub const EXAMPLE: &str =
    "Open items:\n- Merge the docs PR\n- Renew the domain\n- Answer the window-size question";

/// The rules [`shape`] applies, in the order it applies them, for the settings page.
pub const RULES: [&str; 6] = [
    "A Markdown table (a row of | cells over a row of dashes).",
    "Two or more list items and a question: options to choose from, asked with a Choice.",
    "A pull request link (…/pull/123) with the word merge, review or approve.",
    "A question with a fixed set of answers, asked with a Choice: one that starts like Do, \
     Should or Want, one with or in it, or two quoted replies joined by or (say \"on\" or \"off\").",
    "Two or more headings, # or a line in bold on its own, as a status digest has.",
    "Three or more list items (-, *, + or 1.).",
];

/// The shape of a reply that reads better as a view, or None for a plain answer. Text inside
/// fenced code blocks does not count. These rules are the only place this is decided.
pub fn shape(reply: &str) -> Option<Shape> {
    let mut lines = Vec::new();
    let mut fenced = false;
    for line in reply.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if !fenced {
            lines.push(trimmed);
        }
    }
    let table = lines.windows(2).any(|pair| {
        pair[0].starts_with('|')
            && pair[1].contains("---")
            && pair[1].chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
    });
    if table {
        return Some(Shape::Table);
    }
    let items = lines.iter().filter(|l| list_item(l)).count();
    let asks = lines.iter().any(|l| l.ends_with('?'));
    if items >= 2 && asks {
        return Some(Shape::Choices(items));
    }
    let lower = reply.to_lowercase();
    let pull = lower
        .split("/pull/")
        .skip(1)
        .any(|rest| rest.chars().next().is_some_and(|c| c.is_ascii_digit()));
    if pull
        && ["merge", "review", "approve"]
            .iter()
            .any(|w| lower.contains(w))
    {
        return Some(Shape::PullRequest);
    }
    if lines
        .iter()
        .any(|l| closed_question(l) || quoted_options(l))
    {
        return Some(Shape::Question);
    }
    let headings = lines.iter().filter(|l| heading(l)).count();
    if headings >= 2 {
        return Some(Shape::Sections(headings));
    }
    (items >= 3).then_some(Shape::List(items))
}

/// `- x`, `* x`, `+ x`, `1. x` or `1) x`, already trimmed.
fn list_item(line: &str) -> bool {
    if ["- ", "* ", "+ "].iter().any(|m| line.starts_with(m)) {
        return true;
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && (line[digits..].starts_with(". ") || line[digits..].starts_with(") "))
}

/// The words a yes-or-no question starts with.
const YES_OR_NO: [&str; 26] = [
    "do", "does", "did", "should", "shall", "can", "could", "would", "will", "is", "are", "was",
    "were", "want", "may", "might", "ok", "okay", "have", "has", "am", "need", "proceed", "merge",
    "approve", "ready",
];

/// A line ending in a question that has a fixed set of answers: its last sentence starts like
/// a yes-or-no question, or offers this `or` that. An open question (what, how, why) does not.
fn closed_question(line: &str) -> bool {
    let Some(asked) = line.strip_suffix('?') else {
        return false;
    };
    let sentence = asked
        .rsplit(['.', '!', '?', ':'])
        .next()
        .unwrap_or(asked)
        .to_lowercase();
    let first = sentence
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_string();
    let first = first.split('\'').next().unwrap_or_default();
    YES_OR_NO.contains(&first) || sentence.contains(" or ")
}

/// A line offering two or more quoted replies joined by `or`, such as
/// `Say "yolo on" or "yolo off".`
fn quoted_options(line: &str) -> bool {
    let mut quoted = 0;
    let mut open: Option<char> = None;
    let mut before = ' ';
    for c in line.chars() {
        match open {
            Some(close) if c == close => {
                quoted += 1;
                open = None;
            }
            Some(_) => {}
            None => {
                open = match c {
                    '"' | '`' => Some(c),
                    '\u{201c}' => Some('\u{201d}'),
                    '\u{2018}' => Some('\u{2019}'),
                    // An apostrophe inside a word does not open a quote.
                    '\'' if !before.is_alphanumeric() => Some('\''),
                    _ => None,
                };
            }
        }
        before = c;
    }
    quoted >= 2 && line.to_lowercase().contains(" or ")
}

/// `# x` to `###### x`, or a line that is all bold, such as `**Underway**`.
fn heading(line: &str) -> bool {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) && line[hashes..].starts_with(' ') {
        return true;
    }
    line.len() > 4
        && line.starts_with("**")
        && line.ends_with("**")
        && !line[2..line.len() - 2].contains("**")
}

/// What the stop check knows about the turn so far. The bridge keeps one per session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Turn {
    /// The chief of staff showed a view in this turn.
    pub viewed: bool,
    /// The chief of staff asked with a `Choice` in this turn.
    pub asked: bool,
    /// The last stop was sent back for a view, so the next one is not.
    pub nudged: bool,
}

impl Turn {
    /// A new message from the captain starts a new reply.
    pub fn prompt(&mut self) {
        *self = Turn::default();
    }

    /// Decides one `Stop` hook: the shape of the reply that was sent back for a view, or None
    /// to let the turn end. `continuing` is Claude Code's `stop_hook_active`. Either way the
    /// next reply starts fresh, except that a reply sent back once is never sent back again.
    pub fn stop(&mut self, mode: Mode, reply: &str, continuing: bool) -> Option<Shape> {
        let ask = mode == Mode::Enforce && !self.nudged && !continuing;
        // A question needs a Choice; any other shape is answered by any view.
        let found = ask
            .then(|| shape(reply))
            .flatten()
            .filter(|s| if s.asks() { !self.asked } else { !self.viewed });
        *self = Turn {
            nudged: found.is_some(),
            ..Turn::default()
        };
        found
    }
}

/// The feedback a reply of `shape` is sent back with.
pub fn feedback(shape: &Shape) -> String {
    if shape.asks() {
        return format!(
            "Velvet Hammer: this reply {}, which the captain answers with one click. Ask it now \
             with the mcp__velvet-hammer__app_ui tool as a Choice element (the question, each \
             option as a value and a label with a short hint, and the recommended option's value), \
             with at most one short sentence of text, and do not ask the question or list the \
             options again in text. This is asked once; if the question is open-ended, end the \
             turn.",
            shape.describe()
        );
    }
    format!(
        "Velvet Hammer: this reply {}, which the captain reads better as a view. Show it now with \
         the mcp__velvet-hammer__app_ui tool (options as Button elements whose press runs the \
         submit action), with at most one short sentence of text, and do not repeat the reply. \
         This is asked once; if a view does not fit, end the turn.",
        shape.describe()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_answers_stay_plain() {
        for reply in [
            "Done. The build is green.",
            "What should the release notes say?",
            "How did it go? Tell me about the deploy.",
            "Anything else on your mind?",
            "I'd rather not, so don't worry about it.",
            "- one item\n- two items",
            "# Only one heading\nand text",
            "Here is code:\n```\n| a | b |\n|---|---|\n- x\n- y\n- z\n```",
            "See https://github.com/o/r/pull/12 when you have time.",
            "**Bold** text in a sentence, **twice**.",
        ] {
            assert_eq!(shape(reply), None, "{reply}");
        }
    }

    #[test]
    fn each_qualifying_shape_is_found() {
        let cases = [
            ("| Task | State |\n| --- | :-: |\n| a | ok |", Shape::Table),
            (
                "Two ways:\n1. Squash\n2. Rebase\nWhich do you want?",
                Shape::Choices(2),
            ),
            (
                "https://github.com/o/r/pull/42 is green. Approve the merge?",
                Shape::PullRequest,
            ),
            (
                "**Underway**\nTwo tasks.\n\n**Charted Next**\nNothing.",
                Shape::Sections(2),
            ),
            ("## Done\nx\n## Next\ny", Shape::Sections(2)),
            ("Open items:\n- a\n- b\n* c\n+ d", Shape::List(4)),
            ("Steps:\n1) a\n2) b\n3) c", Shape::List(3)),
            ("Yes, I can do that. Should I start now?", Shape::Question),
            (
                "Merge authority is **yolo on** for velvet-hammer. Recommended: yolo off. Do you \
                 want it off?",
                Shape::Question,
            ),
            (
                "Say \u{2018}yolo on\u{2019} or \u{2018}yolo off\u{2019}.",
                Shape::Question,
            ),
            ("Say 'yolo on' or 'yolo off'.", Shape::Question),
            ("Squash it, or rebase it first?", Shape::Question),
            ("Ready to ship?\n\n## Done\nx\n## Next\ny", Shape::Question),
        ];
        for (reply, want) in cases {
            assert_eq!(shape(reply), Some(want), "{reply}");
        }
    }

    #[test]
    fn a_reply_is_sent_back_at_most_once() {
        let list = "- a\n- b\n- c";
        let mut turn = Turn::default();
        assert_eq!(turn.stop(Mode::Enforce, list, false), Some(Shape::List(3)));
        // The retry ignored the feedback; Claude Code says it is continuing.
        assert_eq!(turn.stop(Mode::Enforce, list, true), None);
        // Even if Claude Code did not say so, a second send-back in a row never happens.
        let mut turn = Turn::default();
        turn.stop(Mode::Enforce, list, false);
        assert_eq!(turn.stop(Mode::Enforce, list, false), None);
        // The next reply is checked again.
        assert_eq!(turn.stop(Mode::Enforce, list, false), Some(Shape::List(3)));
    }

    #[test]
    fn a_view_in_the_turn_or_another_mode_lets_the_reply_through() {
        let list = "- a\n- b\n- c";
        let mut turn = Turn {
            viewed: true,
            ..Turn::default()
        };
        assert_eq!(turn.stop(Mode::Enforce, list, false), None);
        assert!(!turn.viewed, "the next reply starts fresh");
        for mode in [Mode::Off, Mode::Nudge] {
            assert_eq!(Turn::default().stop(mode, list, false), None);
        }
        let mut turn = Turn {
            nudged: true,
            ..Turn::default()
        };
        turn.prompt();
        assert_eq!(turn, Turn::default());
    }

    #[test]
    fn a_question_needs_a_choice_and_any_view_will_not_do() {
        let question = "Merge authority is yolo on. Do you want it off?";
        let mut turn = Turn {
            viewed: true,
            ..Turn::default()
        };
        assert_eq!(
            turn.stop(Mode::Enforce, question, false),
            Some(Shape::Question)
        );
        let mut turn = Turn {
            viewed: true,
            asked: true,
            ..Turn::default()
        };
        assert_eq!(turn.stop(Mode::Enforce, question, false), None);
        assert_eq!(turn, Turn::default(), "the next reply starts fresh");
        let feedback = feedback(&Shape::Question);
        assert!(feedback.contains("as a Choice element"), "{feedback}");
        assert!(feedback.contains("do not ask the question"), "{feedback}");
    }

    #[test]
    fn the_instruction_follows_the_mode() {
        assert_eq!(Mode::Off.instruction(), None);
        assert!(Mode::Nudge.instruction().unwrap().contains("app_ui"));
        assert!(
            Mode::Enforce
                .instruction()
                .unwrap()
                .contains("Choice element")
        );
        assert_eq!(Mode::default(), Mode::RECOMMENDED);
        assert!(feedback(&Shape::List(3)).contains("lists 3 items"));
    }

    #[test]
    fn the_setting_survives_a_restart_and_a_bad_file_only_nudges() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/visual")
            .join(std::process::id().to_string());
        let _ = std::fs::remove_dir_all(&dir);
        let mut store = store::Store::<Mode>::open(&dir);
        assert_eq!(*store.settings(), Mode::Enforce);
        store.change(|m| *m = Mode::Off);
        assert_eq!(*store::Store::<Mode>::open(&dir).settings(), Mode::Off);
        std::fs::write(
            dir.join(SETTINGS_FILE),
            "{\"version\": 1, \"mode\": \"loud\"}",
        )
        .unwrap();
        let store = store::Store::<Mode>::open(&dir);
        assert_eq!(*store.settings(), Mode::Nudge);
        assert!(store.problem().unwrap().contains("only nudged"));
    }
}
