//! Tool approval: which of the chief of staff's tool uses run at once, which wait for the
//! captain, and which are refused. The captain picks a [`Mode`] and may add [`Rule`]s on top of
//! it; the PreToolUse hook in [`crate::bridge`] asks [`Settings::decide`] about every tool use,
//! and every decision lands in the [`Activity`] log.
//!
//! The settings live in the app's own data directory ([`Store`], a [`crate::store::Store`]), never in the captain's Claude
//! settings or in a repository. A fresh install runs without asking, as recommended. A settings
//! file the app cannot read makes every tool use ask until the captain saves new settings, so a
//! damaged file never silently drops a rule.
//!
//! Rules are a convenience filter, not a sandbox. They compare text and see neither what a shell
//! would run nor where a path leads: a deny rule for `git push` misses `git  push` (two spaces),
//! `env git push`, `sudo git push` and `bash -c "git push"`, and a deny rule for `/repo/.env`
//! misses `../repo/.env` or a relative path.
//!
//! The activity log keeps the start of every command, so it can hold whatever the command
//! contained. It is created readable by its owner only.
//!
//! No setting reaches the app's fixed limits: it never reads or stores a credential, it strips
//! the API-billing variables from every `claude` it starts, and it never pushes, publishes or
//! merges anything itself.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rune::TextEdit;
use serde_json::{Value, json};

use crate::store::{self, Saved, Shared, suffixed};

/// The settings file in the data directory.
pub const SETTINGS_FILE: &str = "tool-approval.json";
/// The activity log in the data directory: one JSON object per line.
pub const LOG_FILE: &str = "tool-activity.jsonl";
/// How many log entries the app keeps in memory and shows.
const KEEP: usize = 500;
/// A log larger than this is moved aside to `<log>.1` when the app opens.
const ROTATE_AT: u64 = 1 << 20;
/// The most of a tool use's input the log keeps.
const SUBJECT_MAX: usize = 200;

/// Tools that only read, which "Auto-allow read-only tools" runs without asking. The app's own
/// tools count too: they only add a note in the app.
const READ_ONLY: [&str; 7] = [
    "Read",
    "Glob",
    "Grep",
    "LS",
    "NotebookRead",
    "ToolSearch",
    "TodoWrite",
];

/// How tool uses that no rule decides are handled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Every tool use waits for the captain.
    Ask,
    /// Read-only tools run at once; the rest wait for the captain.
    ReadOnly,
    /// Every tool use runs at once. The recommended default.
    Run,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Run, Mode::ReadOnly, Mode::Ask];
    pub const RECOMMENDED: Mode = Mode::Run;

    pub fn label(self) -> &'static str {
        match self {
            Mode::Ask => "Ask every time",
            Mode::ReadOnly => "Auto-allow read-only tools",
            Mode::Run => "Run without asking",
        }
    }

    /// The trade-off in one sentence.
    pub fn explanation(self) -> &'static str {
        match self {
            Mode::Ask => {
                "Every tool use waits for you in Needs your input, with all of its input shown."
            }
            Mode::ReadOnly => {
                "Tools that only read (Read, Glob, Grep and the like) run at once; editing files, \
                 running commands and every other tool use waits for you."
            }
            Mode::Run => {
                "The chief of staff will edit files, run commands and use tools without stopping \
                 for you, and every tool use is still logged."
            }
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Mode::Ask => "ask-every-time",
            Mode::ReadOnly => "auto-allow-read-only",
            Mode::Run => "run-without-asking",
        }
    }

    fn from_key(key: &str) -> Option<Mode> {
        Mode::ALL.into_iter().find(|m| m.key() == key)
    }
}

/// What a rule, or the mode, does with a tool use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Allow,
    Ask,
    Deny,
}

impl Effect {
    pub const ALL: [Effect; 3] = [Effect::Allow, Effect::Ask, Effect::Deny];

    pub fn key(self) -> &'static str {
        match self {
            Effect::Allow => "allow",
            Effect::Ask => "ask",
            Effect::Deny => "deny",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Effect::Allow => "Allow",
            Effect::Ask => "Ask",
            Effect::Deny => "Deny",
        }
    }

    fn from_key(key: &str) -> Option<Effect> {
        Effect::ALL.into_iter().find(|e| e.key() == key)
    }
}

/// One custom rule: `effect` for uses of `tool` (a name, where `*` matches any text) whose main
/// argument matches `pattern`, or for every use when there is no pattern.
///
/// The main argument is a Bash command, the file path of Read, Edit and Write, and so on (see
/// [`subject`]). A Bash pattern matches a command that is the pattern or starts with it followed
/// by a space, and it is checked against each command of a compound command: an allow rule must
/// match every one of them, an ask or deny rule any one. Other patterns match the whole argument,
/// with `*` matching any text, slashes included.
///
/// A rule is a convenience filter, not a sandbox: it compares text, so extra spaces, an `env` or
/// `sudo` prefix, `bash -c "..."` and relative or `..` paths all get past it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub effect: Effect,
    pub tool: String,
    pub pattern: Option<String>,
}

impl Rule {
    pub fn matches(&self, tool: &str, input: &Value) -> bool {
        if !glob(&self.tool, tool) {
            return false;
        }
        let Some(pattern) = &self.pattern else {
            return true;
        };
        let subject = subject(tool, input);
        if tool != "Bash" {
            return glob(pattern, &subject);
        }
        let hit = |command: &str| glob(pattern, command) || glob(&format!("{pattern} *"), command);
        let commands = commands(&subject);
        match self.effect {
            Effect::Allow => !commands.is_empty() && commands.iter().all(|c| hit(c)),
            Effect::Ask | Effect::Deny => hit(subject.trim()) || commands.iter().any(|c| hit(c)),
        }
    }

    /// The rule in words, such as "Deny Bash matching git push".
    pub fn describe(&self) -> String {
        match &self.pattern {
            Some(pattern) => format!("{} {} matching {pattern}", self.effect.label(), self.tool),
            None => format!("{} {} with any input", self.effect.label(), self.tool),
        }
    }
}

/// The captain's tool-approval settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub mode: Mode,
    pub rules: Vec<Rule>,
}

impl Default for Settings {
    /// The recommended default, which a fresh install uses.
    fn default() -> Self {
        Settings {
            mode: Mode::RECOMMENDED,
            rules: Vec::new(),
        }
    }
}

/// Why a tool use was allowed, asked about or denied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum By {
    Mode(Mode),
    /// The rule at this index (from 0) of the list.
    Rule(usize, Rule),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    pub effect: Effect,
    pub by: By,
}

impl Verdict {
    /// "by the mode Run without asking", or "by rule 2 (Deny Bash matching git push)".
    pub fn why(&self) -> String {
        match &self.by {
            By::Mode(mode) => format!("by the mode {}", mode.label()),
            By::Rule(i, rule) => format!("by rule {} ({})", i + 1, rule.describe()),
        }
    }
}

impl Settings {
    /// What happens to one use of `tool` with `input`. A matching deny rule wins over any other
    /// rule, then an ask rule, then an allow rule; when no rule matches, the mode decides. Among
    /// rules of the same effect the first in the list is named.
    pub fn decide(&self, tool: &str, input: &Value) -> Verdict {
        for effect in [Effect::Deny, Effect::Ask, Effect::Allow] {
            let found = self
                .rules
                .iter()
                .enumerate()
                .find(|(_, r)| r.effect == effect && r.matches(tool, input));
            if let Some((i, rule)) = found {
                return Verdict {
                    effect,
                    by: By::Rule(i, rule.clone()),
                };
            }
        }
        let effect = match self.mode {
            Mode::Run => Effect::Allow,
            Mode::ReadOnly if read_only(tool) => Effect::Allow,
            Mode::ReadOnly | Mode::Ask => Effect::Ask,
        };
        Verdict {
            effect,
            by: By::Mode(self.mode),
        }
    }

    /// Every tool use asks: what the app falls back to when the settings file cannot be read.
    pub fn ask_every_time() -> Self {
        Settings {
            mode: Mode::Ask,
            rules: Vec::new(),
        }
    }
}

fn read_only(tool: &str) -> bool {
    READ_ONLY.contains(&tool) || tool.starts_with(&format!("mcp__{}__", crate::bridge::SERVER))
}

/// The settings shared with the hook, which reads them for every tool use, so a change applies
/// to a session that is already running.
pub type Policy = Shared<Settings>;

/// The settings as saved in the data directory. A file that cannot be read makes every tool use
/// ask.
pub type Store = store::Store<Settings>;

impl Saved for Settings {
    const FILE: &'static str = SETTINGS_FILE;
    const FALLBACK: &'static str = "every tool use asks you";

    fn fallback() -> Self {
        Settings::ask_every_time()
    }

    fn to_json(&self) -> Value {
        let rules: Vec<Value> = self
            .rules
            .iter()
            .map(|r| json!({"effect": r.effect.key(), "tool": r.tool, "pattern": r.pattern}))
            .collect();
        json!({"version": store::VERSION, "mode": self.mode.key(), "rules": rules})
    }

    /// Reads the settings file's JSON. Anything it does not understand fails the whole file, so
    /// a rule is never dropped quietly.
    fn from_json(value: &Value) -> Result<Self, String> {
        store::check_version(value)?;
        let mode = value["mode"]
            .as_str()
            .and_then(Mode::from_key)
            .ok_or("its mode is not one Velvet Hammer knows")?;
        let mut rules = Vec::new();
        for (i, rule) in value["rules"]
            .as_array()
            .ok_or("it has no list of rules")?
            .iter()
            .enumerate()
        {
            let bad = || format!("rule {} is not complete", i + 1);
            let effect = rule["effect"]
                .as_str()
                .and_then(Effect::from_key)
                .ok_or_else(bad)?;
            let tool = rule["tool"]
                .as_str()
                .filter(|t| !t.trim().is_empty())
                .ok_or_else(bad)?;
            let pattern = match &rule["pattern"] {
                Value::Null => None,
                Value::String(p) => Some(p.clone()),
                _ => return Err(bad()),
            };
            rules.push(Rule {
                effect,
                tool: tool.to_string(),
                pattern,
            });
        }
        Ok(Settings { mode, rules })
    }
}

/// Creates the directory, and any missing parents, readable by its owner only.
fn create_private_dir(dir: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir)
}

/// Opens the log for appending, creating it with owner-only permissions and tightening a log an
/// earlier version made.
fn open_private(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let file = options.open(path)?;
    make_private(path)?;
    Ok(file)
}

#[cfg(unix)]
fn make_private(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn make_private(_: &Path) -> io::Result<()> {
    Ok(())
}

/// One tool use and what was decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Seconds since the Unix epoch.
    pub at: u64,
    pub tool: String,
    /// The start of the tool's main argument, such as the command or the file path.
    pub subject: String,
    pub allowed: bool,
    /// How it was decided, such as "by the mode Run without asking".
    pub how: String,
    /// The firstmate rule that decided it, by its key (see [`crate::guard`]).
    pub rule: Option<String>,
}

impl Entry {
    pub fn new(tool: &str, input: &Value, allowed: bool, how: String) -> Entry {
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let subject = subject(tool, input).replace(['\n', '\r'], " ");
        let subject = match subject.char_indices().nth(SUBJECT_MAX) {
            Some((end, _)) => format!("{}\u{2026}", &subject[..end]),
            None => subject,
        };
        Entry {
            at,
            tool: tool.to_string(),
            subject,
            allowed,
            how,
            rule: None,
        }
    }

    /// The same entry, decided by the firstmate rule `key`.
    pub fn by_rule(mut self, key: &str) -> Entry {
        self.rule = Some(key.to_string());
        self
    }

    fn to_json(&self) -> Value {
        let mut out = json!({"at": self.at, "tool": self.tool, "subject": self.subject,
               "decision": if self.allowed { "allow" } else { "deny" }, "how": self.how});
        if let Some(rule) = &self.rule {
            out["rule"] = json!(rule);
        }
        out
    }

    fn from_json(value: &Value) -> Option<Entry> {
        Some(Entry {
            at: value["at"].as_u64()?,
            tool: value["tool"].as_str()?.to_string(),
            subject: value["subject"].as_str()?.to_string(),
            allowed: match value["decision"].as_str()? {
                "allow" => true,
                "deny" => false,
                _ => return None,
            },
            how: value["how"].as_str()?.to_string(),
            rule: value["rule"].as_str().map(str::to_string),
        })
    }
}

/// The log of every tool use and its decision, kept in the data directory so it outlasts a
/// restart. It keeps the start of each tool use's main argument, not all of its input.
pub struct Activity {
    path: PathBuf,
    /// Oldest first.
    entries: Vec<Entry>,
    problem: Option<String>,
}

impl Activity {
    pub fn open(dir: &Path) -> Activity {
        let path = dir.join(LOG_FILE);
        if fs::metadata(&path).is_ok_and(|m| m.len() > ROTATE_AT) {
            let _ = fs::rename(&path, suffixed(&path, ".1"));
            let _ = make_private(&suffixed(&path, ".1"));
        }
        let entries: Vec<Entry> = fs::read_to_string(&path)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| Entry::from_json(&serde_json::from_str(line).ok()?))
            .collect();
        let skip = entries.len().saturating_sub(KEEP);
        Activity {
            path,
            entries: entries.into_iter().skip(skip).collect(),
            problem: None,
        }
    }

    pub fn record(&mut self, entry: Entry) {
        let appended = create_private_dir(self.path.parent().expect("in a directory"))
            .and_then(|()| open_private(&self.path))
            .and_then(|mut file| writeln!(file, "{}", entry.to_json()));
        self.problem = appended.err().map(|e| {
            format!(
                "Velvet Hammer could not write to {}: {e}. Entries since then are shown here \
                 but not kept.",
                self.path.display()
            )
        });
        self.entries.push(entry);
        if self.entries.len() > KEEP {
            self.entries.remove(0);
        }
    }

    /// Oldest first.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }
}

/// The field holding a tool's main argument, for the tools Claude Code ships.
fn subject_field(tool: &str) -> Option<&'static str> {
    Some(match tool {
        "Bash" => "command",
        "Read" | "Edit" | "Write" | "MultiEdit" => "file_path",
        "NotebookEdit" | "NotebookRead" => "notebook_path",
        "Glob" | "Grep" | "LS" => "path",
        "WebFetch" => "url",
        "WebSearch" => "query",
        _ => return None,
    })
}

/// A tool use's main argument, which a rule's pattern is matched against: the command, the
/// file path and so on, or for any other tool all of its input as text.
pub fn subject(tool: &str, input: &Value) -> String {
    subject_field(tool)
        .and_then(|field| input[field].as_str())
        .map_or_else(|| input_text(input), str::to_string)
}

/// The input of a tool use made from a tool name and its main argument, for trying the settings
/// on an example.
pub fn example_input(tool: &str, argument: &str) -> Value {
    match subject_field(tool) {
        Some(field) => json!({field: argument}),
        None => json!({"input": argument}),
    }
}

/// All of a tool's input as text, one `name: value` per field with string values as they are, so
/// a command or file content is shown whole.
pub fn input_text(input: &Value) -> String {
    let Some(fields) = input.as_object() else {
        return input.to_string();
    };
    let lines: Vec<String> = fields
        .iter()
        .map(|(name, value)| match value.as_str() {
            Some(text) => format!("{name}: {text}"),
            None => format!("{name}: {value}"),
        })
        .collect();
    lines.join("\n")
}

/// The commands of a Bash command line, split at `;`, `&`, `|`, newlines, parentheses and
/// backticks. Splitting inside a quoted string only makes allow rules match less and deny and
/// ask rules match more.
pub fn commands(line: &str) -> Vec<&str> {
    line.split([';', '&', '|', '\n', '(', ')', '`'])
        .map(|c| c.trim().trim_end_matches('$').trim())
        .filter(|c| !c.is_empty())
        .collect()
}

/// Whether `text` matches `pattern`, where `*` matches any text, including none.
fn glob(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    let [first, middle @ .., last] = parts.as_slice() else {
        return pattern == text;
    };
    if text.len() < first.len() + last.len() || !text.starts_with(first) || !text.ends_with(last) {
        return false;
    }
    let mut rest = &text[first.len()..text.len() - last.len()];
    for part in middle {
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    true
}

/// `at` (seconds since the Unix epoch) as "2026-11-13 18:21 UTC".
pub fn clock(at: u64) -> String {
    // Howard Hinnant's days-to-civil algorithm.
    let z = (at / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let secs = at % 86_400;
    format!(
        "{year}-{month:02}-{day:02} {:02}:{:02} UTC",
        secs / 3_600,
        secs % 3_600 / 60
    )
}

/// What the settings page is editing: the rule being written, the example being tried, and
/// whether a reset waits for confirmation.
pub struct Form {
    pub effect: Effect,
    pub tool: TextEdit,
    pub pattern: TextEdit,
    /// The rule being changed, by index; None while adding one.
    pub editing: Option<usize>,
    pub problem: Option<String>,
    pub example_tool: TextEdit,
    pub example_argument: TextEdit,
    pub confirm_reset: bool,
}

impl Default for Form {
    fn default() -> Self {
        Form {
            effect: Effect::Deny,
            tool: TextEdit::default(),
            pattern: TextEdit::default(),
            editing: None,
            problem: None,
            example_tool: TextEdit::new("Bash"),
            example_argument: TextEdit::new("git push origin main"),
            confirm_reset: false,
        }
    }
}

impl Form {
    /// Adds the rule in the form, or saves the one being changed.
    pub fn save(&mut self, store: &mut Store) {
        let tool = self.tool.text.trim().to_string();
        if tool.is_empty() {
            self.problem = Some("Name the tool the rule is for, such as Bash or Edit.".into());
            return;
        }
        let pattern = Some(self.pattern.text.trim().to_string()).filter(|p| !p.is_empty());
        let rule = Rule {
            effect: self.effect,
            tool,
            pattern,
        };
        let editing = self.editing;
        store.change(|s| match editing {
            Some(i) if i < s.rules.len() => s.rules[i] = rule,
            _ => s.rules.push(rule),
        });
        if store.problem().is_none() {
            self.clear();
        }
    }

    /// Loads rule `i` into the form for changing.
    pub fn edit(&mut self, store: &Store, i: usize) {
        if let Some(rule) = store.settings().rules.get(i) {
            self.effect = rule.effect;
            self.tool = TextEdit::new(rule.tool.clone());
            self.pattern = TextEdit::new(rule.pattern.clone().unwrap_or_default());
            self.editing = Some(i);
            self.problem = None;
        }
    }

    /// Empties the form and stops changing a rule.
    pub fn clear(&mut self) {
        self.tool.clear();
        self.pattern.clear();
        self.editing = None;
        self.problem = None;
    }

    /// Moves rule `i` up (`-1`) or down (`1`) the list.
    pub fn shift(&mut self, store: &mut Store, i: usize, by: isize) {
        let Some(j) = i.checked_add_signed(by) else {
            return;
        };
        if j < store.settings().rules.len() {
            store.change(|s| s.rules.swap(i, j));
            self.clear();
        }
    }

    pub fn delete(&mut self, store: &mut Store, i: usize) {
        if i < store.settings().rules.len() {
            store.change(|s| {
                s.rules.remove(i);
            });
            self.clear();
        }
    }

    /// Puts back the recommended default: Run without asking and no rules.
    pub fn reset(&mut self, store: &mut Store) {
        store.change(|s| *s = Settings::default());
        self.clear();
        self.confirm_reset = false;
    }

    /// What the settings decide for the example in the form, in words.
    pub fn example(&self, settings: &Settings) -> String {
        let tool = self.example_tool.text.trim();
        if tool.is_empty() {
            return "Name a tool to see what happens to it.".into();
        }
        let input = example_input(tool, &self.example_argument.text);
        let verdict = settings.decide(tool, &input);
        let what = match verdict.effect {
            Effect::Allow => "Runs without asking",
            Effect::Ask => "Waits for you in Needs your input",
            Effect::Deny => "Denied",
        };
        format!("{what}, {}.", verdict.why())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(effect: Effect, tool: &str, pattern: Option<&str>) -> Rule {
        Rule {
            effect,
            tool: tool.into(),
            pattern: pattern.map(str::to_string),
        }
    }

    fn bash(command: &str) -> Value {
        json!({"command": command})
    }

    /// A fresh directory under `target/`.
    fn scratch() -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static RUN: AtomicU32 = AtomicU32::new(0);
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/approval")
            .join(format!(
                "{}-{}",
                std::process::id(),
                RUN.fetch_add(1, Ordering::Relaxed)
            ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn each_mode_decides_what_no_rule_covers() {
        let read = json!({"file_path": "/repo/README.md"});
        let cases = [
            (Mode::Ask, "Read", &read, Effect::Ask),
            (Mode::Ask, "Bash", &bash("ls"), Effect::Ask),
            (
                Mode::Ask,
                "mcp__velvet-hammer__app_note",
                &json!({}),
                Effect::Ask,
            ),
            (Mode::ReadOnly, "Read", &read, Effect::Allow),
            (
                Mode::ReadOnly,
                "Grep",
                &json!({"pattern": "x"}),
                Effect::Allow,
            ),
            (
                Mode::ReadOnly,
                "mcp__velvet-hammer__app_note",
                &json!({}),
                Effect::Allow,
            ),
            (Mode::ReadOnly, "Bash", &bash("ls"), Effect::Ask),
            (
                Mode::ReadOnly,
                "WebFetch",
                &json!({"url": "https://x.test"}),
                Effect::Ask,
            ),
            (Mode::Run, "Bash", &bash("rm -rf build"), Effect::Allow),
            (Mode::Run, "Write", &read, Effect::Allow),
        ];
        for (mode, tool, input, want) in cases {
            let settings = Settings {
                mode,
                rules: vec![],
            };
            let verdict = settings.decide(tool, input);
            assert_eq!(
                (verdict.effect, verdict.by),
                (want, By::Mode(mode)),
                "{mode:?} {tool}"
            );
        }
    }

    #[test]
    fn deny_wins_over_ask_and_ask_over_allow_whatever_the_order() {
        let settings = Settings {
            mode: Mode::Run,
            rules: vec![
                rule(Effect::Allow, "Bash", Some("git")),
                rule(Effect::Ask, "Bash", Some("git push")),
                rule(Effect::Deny, "Bash", Some("git push --force")),
            ],
        };
        let decide = |c: &str| settings.decide("Bash", &bash(c));
        assert_eq!(decide("git status").effect, Effect::Allow);
        assert_eq!(
            decide("git status").by,
            By::Rule(0, settings.rules[0].clone())
        );
        assert_eq!(decide("git push origin main").effect, Effect::Ask);
        let forced = decide("git push --force origin main");
        assert_eq!(forced.effect, Effect::Deny);
        assert_eq!(
            forced.why(),
            "by rule 3 (Deny Bash matching git push --force)"
        );
        // Ask forces a question even when the mode runs everything.
        assert_eq!(
            decide("git push").by,
            By::Rule(1, settings.rules[1].clone())
        );
    }

    #[test]
    fn bash_patterns_match_command_prefixes_in_every_part_of_a_compound_command() {
        let deny = rule(Effect::Deny, "Bash", Some("git push"));
        for command in [
            "git push",
            "git push origin main",
            "cd repo && git push",
            "make; git push",
            "echo $(git push)",
            "echo `git push`",
            "ls | git push",
        ] {
            assert!(deny.matches("Bash", &bash(command)), "{command}");
        }
        for command in ["git pushy", "git status", "echo git push"] {
            assert!(!deny.matches("Bash", &bash(command)), "{command}");
        }
        let allow = rule(Effect::Allow, "Bash", Some("git status"));
        assert!(allow.matches("Bash", &bash("git status --short")));
        assert!(!allow.matches("Bash", &bash("git status && rm -rf ~")));
        let glob_allow = rule(Effect::Allow, "Bash", Some("cargo *"));
        assert!(glob_allow.matches("Bash", &bash("cargo test && cargo build")));
    }

    #[test]
    fn file_patterns_are_globs_and_tool_names_may_be_too() {
        let edit = rule(Effect::Allow, "Edit", Some("/repo/src/*.rs"));
        assert!(edit.matches("Edit", &json!({"file_path": "/repo/src/main.rs"})));
        assert!(edit.matches("Edit", &json!({"file_path": "/repo/src/a/b.rs"})));
        assert!(!edit.matches("Edit", &json!({"file_path": "/repo/README.md"})));
        assert!(!edit.matches("Write", &json!({"file_path": "/repo/src/main.rs"})));
        let writes = rule(Effect::Deny, "*", Some("*/.env"));
        assert!(writes.matches("Write", &json!({"file_path": "/repo/.env"})));
        let any = rule(Effect::Allow, "mcp__github__*", None);
        assert!(any.matches("mcp__github__list_issues", &json!({"repo": "x"})));
        assert!(!any.matches("mcp__slack__post", &json!({})));
        // A tool with no known main argument is matched on all of its input.
        let slack = rule(Effect::Deny, "mcp__slack__post", Some("*#general*"));
        assert!(slack.matches("mcp__slack__post", &json!({"channel": "#general"})));
    }

    #[test]
    fn glob_handles_stars_anywhere() {
        assert!(glob("a*b*c", "a-b-c"));
        assert!(glob("*", ""));
        assert!(glob("ab*", "ab"));
        assert!(!glob("a*a", "a"));
        assert!(!glob("abc", "abcd"));
        assert!(glob("*.rs", "x.rs"));
    }

    #[test]
    fn a_missing_file_gives_the_recommended_default() {
        let store = Store::open(&scratch());
        assert_eq!(store.settings(), &Settings::default());
        assert_eq!(store.settings().mode, Mode::Run);
        assert!(store.problem().is_none());
    }

    #[test]
    fn settings_survive_a_restart() {
        let dir = scratch();
        let mut store = Store::open(&dir);
        store.change(|s| {
            s.mode = Mode::ReadOnly;
            s.rules.push(rule(Effect::Deny, "Bash", Some("git push")));
            s.rules.push(rule(Effect::Allow, "Edit", None));
        });
        assert!(store.problem().is_none());
        assert_eq!(store.shared().read().unwrap().mode, Mode::ReadOnly);
        let again = Store::open(&dir);
        assert_eq!(again.settings(), store.settings());
        assert!(again.problem().is_none());
        // Written whole: no temporary file is left behind.
        let names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, [SETTINGS_FILE]);
    }

    #[test]
    fn an_unreadable_file_asks_every_time_and_is_kept_when_saved_over() {
        for bad in [
            "{not json",
            r#"{"version": 2, "mode": "run-without-asking", "rules": []}"#,
            r#"{"version": 1, "mode": "yolo", "rules": []}"#,
            r#"{"version": 1, "mode": "run-without-asking", "rules": [{"effect": "deny"}]}"#,
        ] {
            let dir = scratch();
            fs::write(dir.join(SETTINGS_FILE), bad).unwrap();
            let mut store = Store::open(&dir);
            assert_eq!(store.settings().mode, Mode::Ask, "{bad}");
            assert!(store.settings().rules.is_empty());
            assert!(
                store.problem().unwrap().contains("every tool use asks you"),
                "{bad}"
            );
            assert_eq!(store.shared().read().unwrap().mode, Mode::Ask);
            store.change(|s| s.mode = Mode::Run);
            assert!(store.problem().is_none());
            assert_eq!(
                fs::read_to_string(dir.join("tool-approval.json.bad")).unwrap(),
                bad
            );
            assert_eq!(Store::open(&dir).settings().mode, Mode::Run);
        }
    }

    #[test]
    #[cfg(unix)]
    fn a_failed_save_changes_nothing() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch();
        let mut store = Store::open(&dir);
        store.change(|s| s.mode = Mode::Ask);
        // A directory the app cannot write to.
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o500)).unwrap();
        store.change(|s| s.mode = Mode::Run);
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(store.settings().mode, Mode::Ask);
        assert_eq!(store.shared().read().unwrap().mode, Mode::Ask);
        assert!(store.problem().unwrap().contains("could not save"));
        assert_eq!(Store::open(&dir).settings().mode, Mode::Ask);
    }

    #[test]
    fn the_form_adds_edits_moves_deletes_and_resets_rules() {
        let mut store = Store::open(&scratch());
        let mut form = Form::default();
        form.save(&mut store);
        assert!(form.problem.is_some(), "a rule needs a tool");
        form.effect = Effect::Deny;
        form.tool.text = "Bash".into();
        form.pattern.text = "git push".into();
        form.save(&mut store);
        form.effect = Effect::Allow;
        form.tool.text = "Edit".into();
        form.save(&mut store);
        let rules = &store.settings().rules;
        assert_eq!(rules[0], rule(Effect::Deny, "Bash", Some("git push")));
        assert_eq!(rules[1], rule(Effect::Allow, "Edit", None));
        form.edit(&store, 0);
        assert_eq!(form.pattern.text, "git push");
        form.effect = Effect::Ask;
        form.save(&mut store);
        assert_eq!(store.settings().rules[0].effect, Effect::Ask);
        form.shift(&mut store, 1, -1);
        assert_eq!(store.settings().rules[0].tool, "Edit");
        form.shift(&mut store, 0, -1);
        assert_eq!(store.settings().rules[0].tool, "Edit");
        form.delete(&mut store, 0);
        assert_eq!(store.settings().rules.len(), 1);
        store.change(|s| s.mode = Mode::Ask);
        form.reset(&mut store);
        assert_eq!(store.settings(), &Settings::default());
    }

    #[test]
    fn the_example_says_what_would_happen() {
        let mut form = Form::default();
        let settings = Settings {
            mode: Mode::Run,
            rules: vec![rule(Effect::Deny, "Bash", Some("git push"))],
        };
        assert_eq!(
            form.example(&settings),
            "Denied, by rule 1 (Deny Bash matching git push)."
        );
        form.example_argument.text = "git status".into();
        assert_eq!(
            form.example(&settings),
            "Runs without asking, by the mode Run without asking."
        );
    }

    #[test]
    fn the_log_keeps_every_decision_across_a_restart() {
        let dir = scratch();
        let mut log = Activity::open(&dir);
        assert!(log.entries().is_empty());
        let long = format!("echo {}", "x".repeat(400));
        log.record(Entry::new(
            "Bash",
            &bash(&long),
            true,
            "by the mode Run without asking".into(),
        ));
        log.record(Entry::new(
            "Bash",
            &bash("git push"),
            false,
            "you denied it".into(),
        ));
        let again = Activity::open(&dir);
        assert_eq!(again.entries(), log.entries());
        assert_eq!(again.entries()[0].subject.chars().count(), SUBJECT_MAX + 1);
        assert!(!again.entries()[1].allowed);
        assert!(again.problem().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn the_log_and_its_new_directory_are_private_even_after_rotation() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let dir = scratch().join("data");
        let mut log = Activity::open(&dir);
        log.record(Entry::new("Bash", &bash("ls"), true, "by the mode".into()));
        assert!(log.problem().is_none());
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join(LOG_FILE)), 0o600);

        let path = dir.join(LOG_FILE);
        fs::write(&path, "x".repeat(ROTATE_AT as usize + 1)).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let mut log = Activity::open(&dir);
        log.record(Entry::new("Bash", &bash("ls"), true, "by the mode".into()));
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(&suffixed(&path, ".1")), 0o600);
    }

    #[test]
    fn clock_reads_utc() {
        assert_eq!(clock(0), "1970-01-01 00:00 UTC");
        assert_eq!(clock(1_794_594_090), "2026-11-13 18:21 UTC");
        assert_eq!(clock(951_782_400), "2000-02-29 00:00 UTC");
    }
}
