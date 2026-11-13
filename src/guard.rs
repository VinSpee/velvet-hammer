//! Firstmate's rules, enforced at tool-call time. The chief of staff runs as firstmate (see
//! `docs/firstmate.md`), and some of firstmate's hard rules name actions a PreToolUse hook can
//! see: a merge, a forced push, a teardown that discards work, a write into a project. Each
//! [`Rule`] here matches one such tool-call shape and either denies it, with a reason the model
//! can act on, or asks the captain in Needs your input with the exact detail. None ever allows
//! anything: a tool use no rule matches goes on to the tool-approval settings
//! ([`crate::approval`]).
//!
//! The rules apply only to a chief of staff that runs as firstmate, and all are on by default.
//! The captain can turn each off on the Firstmate rules page; the choice is kept in
//! [`SETTINGS_FILE`].
//!
//! Like the approval rules, these read command text. They see each part of a command joined with
//! `&&`, `;`, `|`, `$( )` or backticks, strip `env`, `sudo` and variable assignments in front,
//! follow `cd` and `git -C`, and resolve `~`, `$HOME` and `$FM_HOME`. They do not see inside a
//! script, `bash -c "..."`, `eval` or a path built at run time; `docs/firstmate-rules.md` lists
//! what is not enforceable this way.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde_json::{Value, json};

use crate::approval::{Effect, commands};
use crate::store::{self, Saved};

/// The settings file in the data directory.
pub const SETTINGS_FILE: &str = "firstmate-rules.json";

/// One of firstmate's rules the hook enforces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rule {
    MergeScriptOnly,
    MergeAsks,
    NoAdminMerge,
    NoForcePush,
    DiscardAsks,
    ProjectsReadOnly,
    PublishAsks,
}

impl Rule {
    pub const ALL: [Rule; 7] = [
        Rule::MergeScriptOnly,
        Rule::MergeAsks,
        Rule::NoAdminMerge,
        Rule::NoForcePush,
        Rule::DiscardAsks,
        Rule::ProjectsReadOnly,
        Rule::PublishAsks,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Rule::MergeScriptOnly => "merge-through-script",
            Rule::MergeAsks => "merge-asks-captain",
            Rule::NoAdminMerge => "no-admin-merge",
            Rule::NoForcePush => "no-force-push",
            Rule::DiscardAsks => "discard-asks-captain",
            Rule::ProjectsReadOnly => "projects-read-only",
            Rule::PublishAsks => "publish-asks-captain",
        }
    }

    fn from_key(key: &str) -> Option<Rule> {
        Rule::ALL.into_iter().find(|r| r.key() == key)
    }

    pub fn title(self) -> &'static str {
        match self {
            Rule::MergeScriptOnly => "Merge only through firstmate's merge script",
            Rule::MergeAsks => "Every merge waits for your word",
            Rule::NoAdminMerge => "No admin-bypass merges",
            Rule::NoForcePush => "No force pushes",
            Rule::DiscardAsks => "Discarding unlanded work waits for your word",
            Rule::ProjectsReadOnly => "Projects are read-only to the chief of staff",
            Rule::PublishAsks => "Publishing waits for your word",
        }
    }

    /// The firstmate rule it enforces.
    pub fn source(self) -> &'static str {
        match self {
            Rule::MergeScriptOnly => {
                "AGENTS.md, section 7: use bin/fm-pr-merge.sh for every task PR merge and \
                 bin/fm-merge-local.sh for local-only landing; never call a lower-level merge \
                 command around their guards."
            }
            Rule::MergeAsks => {
                "AGENTS.md, hard rule 2: never merge a PR without the captain's explicit word."
            }
            Rule::NoAdminMerge => {
                "bin/fm-pr-merge.sh refuses --admin unless --attended-override is given for an \
                 explicit captain instruction; this rule refuses it outright."
            }
            Rule::NoForcePush => {
                "AGENTS.md, hard rule 1: no path authorizes forcing. Firstmate itself has no \
                 force-push rule; this one is yours."
            }
            Rule::DiscardAsks => {
                "AGENTS.md, hard rule 3: never tear down unlanded work, and never use --force \
                 unless the captain explicitly authorized discarding it."
            }
            Rule::ProjectsReadOnly => {
                "AGENTS.md, hard rule 1: never edit, commit or run state-changing commands under \
                 projects/ or in a project worktree, unless the captain approves that operation in \
                 the moment."
            }
            Rule::PublishAsks => {
                "AGENTS.md, sections 1 and 6: no push, fork, issue, comment or pull request on \
                 firstmate's own authority, and no remote the captain did not consent to."
            }
        }
    }

    /// The tool calls it matches, in words.
    pub fn matches(self) -> &'static str {
        match self {
            Rule::MergeScriptOnly => {
                "Bash: gh pr merge, glab mr merge, tea pulls merge, or gh api on a …/pulls/N/merge \
                 path."
            }
            Rule::MergeAsks => "Bash: bin/fm-pr-merge.sh or bin/fm-merge-local.sh.",
            Rule::NoAdminMerge => "Bash: --admin given to gh pr merge or bin/fm-pr-merge.sh.",
            Rule::NoForcePush => {
                "Bash: git push with --force, -f, --force-with-lease, --mirror or a +refspec."
            }
            Rule::DiscardAsks => {
                "Bash: bin/fm-teardown.sh --force (asks), or git worktree remove --force (denied)."
            }
            Rule::ProjectsReadOnly => {
                "Edit, Write, MultiEdit and NotebookEdit on a file under projects/ or a task \
                 worktree; Bash git commands that change a repository there (commit, merge, \
                 reset, checkout, stash and the like), and rm, mv, cp, touch, mkdir, sed -i, tee \
                 or a > redirect onto a path there."
            }
            Rule::PublishAsks => {
                "Bash: git push, gh repo create or fork, glab repo create, gh pr create or \
                 comment, gh issue create or comment, gh release create, gh gist create."
            }
        }
    }

    /// What it does with a match, mostly: [`Rule::DiscardAsks`] also denies a forced
    /// `git worktree remove`.
    pub fn effect(self) -> Effect {
        match self {
            Rule::MergeScriptOnly | Rule::NoAdminMerge | Rule::NoForcePush => Effect::Deny,
            Rule::MergeAsks | Rule::DiscardAsks | Rule::ProjectsReadOnly | Rule::PublishAsks => {
                Effect::Ask
            }
        }
    }

    /// What it does with a match, in words.
    pub fn does(self) -> &'static str {
        match self {
            Rule::MergeScriptOnly | Rule::NoAdminMerge | Rule::NoForcePush => {
                "Denies it, telling the chief of staff why and what to do instead."
            }
            Rule::DiscardAsks => {
                "Asks you in Needs your input about a forced teardown, naming the task; denies a \
                 forced worktree removal, pointing at bin/fm-teardown.sh."
            }
            Rule::MergeAsks => {
                "Asks you in Needs your input, naming the pull request. Neither the tool-approval \
                 mode nor an allow rule can answer for you."
            }
            Rule::ProjectsReadOnly => {
                "Asks you in Needs your input, naming the file or repository, so you can approve \
                 a change you asked for."
            }
            Rule::PublishAsks => "Asks you in Needs your input, naming the remote or the command.",
        }
    }
}

/// Which rules the captain turned off. Every rule is on by default.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub off: BTreeSet<Rule>,
}

impl Settings {
    pub fn on(&self, rule: Rule) -> bool {
        !self.off.contains(&rule)
    }

    pub fn set(&mut self, rule: Rule, on: bool) {
        if on {
            self.off.remove(&rule);
        } else {
            self.off.insert(rule);
        }
    }

    /// The rule that decides one tool use, if any. A deny wins over an ask; among rules of the
    /// same effect the first in [`Rule::ALL`] is named. `cwd` is the directory the hook reports
    /// the session in.
    pub fn check(&self, tool: &str, input: &Value, cwd: &Path, home: &Home) -> Option<Hit> {
        let hits: Vec<Hit> = hits(tool, input, cwd, home)
            .into_iter()
            .filter(|h| self.on(h.rule))
            .collect();
        let deny = hits.iter().position(|h| h.effect == Effect::Deny);
        let at = deny.or((!hits.is_empty()).then_some(0))?;
        hits.into_iter().nth(at)
    }
}

impl Saved for Settings {
    const FILE: &'static str = SETTINGS_FILE;
    const FALLBACK: &'static str = "every firstmate rule is on";

    fn fallback() -> Self {
        Settings::default()
    }

    fn to_json(&self) -> Value {
        let off: Vec<&str> = self.off.iter().map(|r| r.key()).collect();
        json!({"version": store::VERSION, "off": off})
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        store::check_version(value)?;
        let mut off = BTreeSet::new();
        for key in value["off"]
            .as_array()
            .ok_or("it has no list of rules turned off")?
        {
            let rule = key.as_str().and_then(Rule::from_key).ok_or_else(|| {
                format!("it turns off a rule Velvet Hammer does not know ({key})")
            })?;
            off.insert(rule);
        }
        Ok(Settings { off })
    }
}

/// A rule matched a tool use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub rule: Rule,
    /// Deny or Ask; a rule never allows.
    pub effect: Effect,
    /// For an ask, the question with the exact detail, such as "Merge https://…/pull/7?".
    pub question: String,
    /// Why, in words the model can act on (a deny) or the captain can decide with (an ask).
    pub why: String,
}

impl Hit {
    fn new(rule: Rule, question: String, why: impl Into<String>) -> Hit {
        Hit {
            rule,
            effect: rule.effect(),
            question,
            why: why.into(),
        }
    }

    fn denied(mut self) -> Hit {
        self.effect = Effect::Deny;
        self
    }
}

/// Firstmate's operational home (`FM_HOME`), whose `projects/` and task worktrees are
/// protected.
#[derive(Clone, Debug)]
pub struct Home {
    pub dir: PathBuf,
}

impl Home {
    pub fn new(dir: impl Into<PathBuf>) -> Home {
        Home { dir: dir.into() }
    }

    /// The directories firstmate's own records name: every `worktree=` and `project=` line of
    /// `state/*.meta`. They are read at each tool use, so a task spawned a moment ago counts.
    fn records(&self) -> Vec<PathBuf> {
        let Ok(entries) = fs::read_dir(self.dir.join("state")) else {
            return Vec::new();
        };
        let mut dirs = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "meta") {
                continue;
            }
            let text = fs::read_to_string(&path).unwrap_or_default();
            for line in text.lines() {
                let value = line
                    .strip_prefix("worktree=")
                    .or_else(|| line.strip_prefix("project="));
                if let Some(dir) = value.filter(|d| d.starts_with('/')) {
                    dirs.push(normal(Path::new(dir)));
                }
            }
        }
        dirs
    }

    /// Whether `path` is in a project or task worktree: `projects/` or anything below it, or a
    /// recorded directory or anything below it. Each is compared as written and as the file system
    /// resolves it, so a symlinked home still counts.
    fn protects(&self, path: &Path, records: &[PathBuf]) -> bool {
        let with_real = |dir: PathBuf| {
            let real = fs::canonicalize(&dir).ok();
            [Some(dir), real].into_iter().flatten()
        };
        let projects = normal(&self.dir.join("projects"));
        with_real(projects).any(|p| path.starts_with(&p))
            || records
                .iter()
                .flat_map(|d| with_real(d.clone()))
                .any(|d| path.starts_with(&d))
    }

    /// `word` as an absolute path, read from `dir`: `~`, `$HOME` and `$FM_HOME` expanded, `.`
    /// and `..` resolved as written. None when it names another variable.
    fn resolve(&self, word: &str, dir: &Path) -> Option<PathBuf> {
        let user = std::env::var_os("HOME").map(PathBuf::from);
        let expanded = if let Some(rest) = strip_var(word, "FM_HOME") {
            self.dir.join(rest)
        } else if let Some(rest) = strip_var(word, "HOME") {
            user?.join(rest)
        } else if word == "~" || word.starts_with("~/") {
            user?.join(word.trim_start_matches('~').trim_start_matches('/'))
        } else if word.contains('$') || word.is_empty() {
            return None;
        } else {
            dir.join(word)
        };
        Some(normal(&expanded))
    }
}

/// `$NAME/rest` or `${NAME}/rest` as `rest`.
fn strip_var<'a>(word: &'a str, name: &str) -> Option<&'a str> {
    let rest = word
        .strip_prefix(&format!("${{{name}}}"))
        .or_else(|| word.strip_prefix(&format!("${name}")))?;
    if rest.is_empty() || rest.starts_with('/') {
        Some(rest.trim_start_matches('/'))
    } else {
        None
    }
}

/// `path` with `.` and `..` resolved as written, without touching the file system.
fn normal(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// Every rule that matches, on or off, in the order of [`Rule::ALL`] within each command part.
fn hits(tool: &str, input: &Value, cwd: &Path, home: &Home) -> Vec<Hit> {
    let records = home.records();
    match tool {
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => {
            let field = if tool == "NotebookEdit" {
                "notebook_path"
            } else {
                "file_path"
            };
            let Some(path) = input[field].as_str().and_then(|p| home.resolve(p, cwd)) else {
                return Vec::new();
            };
            if !home.protects(&path, &records) {
                return Vec::new();
            }
            vec![Hit::new(
                Rule::ProjectsReadOnly,
                format!("Let the chief of staff edit {}?", path.display()),
                "The file is in one of firstmate's projects or task worktrees, which firstmate \
                 only reads: crewmates change projects. Allow it only for a change you asked \
                 for, in this project, now.",
            )]
        }
        "Bash" => {
            let mut dir = cwd.to_path_buf();
            let mut found = Vec::new();
            for part in commands(input["command"].as_str().unwrap_or_default()) {
                let words = program(words(part));
                if let Some(next) = cd(&words, &dir, home) {
                    dir = next;
                    continue;
                }
                found.extend(command_hits(&words, &dir, home, &records));
            }
            found.sort_by_key(|h| Rule::ALL.iter().position(|r| *r == h.rule));
            found
        }
        _ => Vec::new(),
    }
}

/// The directory a `cd` or `pushd` part changes to.
fn cd(words: &[String], dir: &Path, home: &Home) -> Option<PathBuf> {
    match words {
        [cmd] if cmd == "cd" || cmd == "pushd" => {
            std::env::var_os("HOME").map(|h| normal(Path::new(&h)))
        }
        [cmd, to, ..] if cmd == "cd" || cmd == "pushd" => {
            Some(home.resolve(to, dir).unwrap_or_else(|| dir.to_path_buf()))
        }
        _ => None,
    }
}

/// The rules one command part matches. `words` starts with the program.
fn command_hits(words: &[String], dir: &Path, home: &Home, records: &[PathBuf]) -> Vec<Hit> {
    let Some((first, args)) = words.split_first() else {
        return Vec::new();
    };
    let name = Path::new(first)
        .file_name()
        .map_or(first.clone(), |n| n.to_string_lossy().into_owned());
    let mut found = Vec::new();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    let help = has("-h") || has("--help");
    let shown = words.join(" ");
    match name.as_str() {
        "fm-pr-merge.sh" if !help => {
            if has("--admin") {
                found.push(admin(&shown));
            }
            let target = args
                .iter()
                .find(|a| a.contains("://"))
                .or(args.get(1))
                .cloned()
                .unwrap_or_else(|| "a pull request".into());
            found.push(Hit::new(
                Rule::MergeAsks,
                format!("Merge {target}?"),
                "Firstmate's hard rule 2: never merge a pull request without the captain's \
                 explicit word. Allow runs firstmate's guarded merge script for this pull \
                 request; Deny refuses it.",
            ));
        }
        "fm-merge-local.sh" if !help => found.push(Hit::new(
            Rule::MergeAsks,
            format!(
                "Land task {} on its project's default branch?",
                args.first().map_or("?", String::as_str)
            ),
            "Firstmate's hard rule 2: a local-only landing waits for the captain's explicit \
             word. Allow runs firstmate's guarded fast-forward script; Deny refuses it.",
        )),
        "fm-teardown.sh" if has("--force") => found.push(Hit::new(
            Rule::DiscardAsks,
            format!(
                "Discard the unlanded work of task {}?",
                args.iter()
                    .find(|a| !a.starts_with('-'))
                    .map_or("?", String::as_str)
            ),
            "Firstmate's hard rule 3: --force skips the landed-work check and discards whatever \
             the task has not landed. Allow only if you mean to throw that work away.",
        )),
        "gh" | "glab" | "tea" => found.extend(forge_hits(&name, args, &shown)),
        "git" => found.extend(git_hits(args, dir, home, records, &shown)),
        _ => {}
    }
    if let Some(path) = changed_path(&name, args, dir, home, records) {
        found.push(Hit::new(
            Rule::ProjectsReadOnly,
            format!("Let the chief of staff change {}?", path.display()),
            "The path is in one of firstmate's projects or task worktrees, which firstmate only \
             reads: crewmates change projects. Allow it only for a change you asked for, in \
             this project, now.",
        ));
    }
    found
}

fn admin(shown: &str) -> Hit {
    Hit::new(
        Rule::NoAdminMerge,
        String::new(),
        format!(
            "`{shown}` would merge with --admin, which bypasses the project's branch \
             protection. Merge without --admin through bin/fm-pr-merge.sh, or ask the captain; \
             only the captain can turn this rule off, on Velvet Hammer's Firstmate rules page."
        ),
    )
}

/// The rules a `gh`, `glab` or `tea` command matches.
fn forge_hits(name: &str, args: &[String], shown: &str) -> Vec<Hit> {
    // The subcommand words, skipping flags and the value of -R/--repo.
    let mut verbs = Vec::new();
    let mut skip = false;
    for arg in args {
        if std::mem::take(&mut skip) {
            continue;
        }
        if arg == "-R" || arg == "--repo" {
            skip = true;
        } else if !arg.starts_with('-') {
            verbs.push(arg.as_str());
        }
    }
    let mut found = Vec::new();
    let merge = match (name, verbs.as_slice()) {
        ("gh", ["pr", "merge", ..]) | ("glab", ["mr", "merge", ..]) => true,
        ("tea", [noun, "merge", ..]) => ["pulls", "pull", "pr"].contains(noun),
        ("gh", ["api", ..]) => args.iter().any(|a| {
            let a = a.trim_end_matches('/');
            a.ends_with("/merge") && a.contains("/pulls/")
        }),
        _ => false,
    };
    if merge {
        if args.iter().any(|a| a == "--admin") {
            found.push(admin(shown));
        }
        found.push(Hit::new(
            Rule::MergeScriptOnly,
            String::new(),
            format!(
                "`{shown}` merges around firstmate's guards. Merge a task's pull request with \
                 bin/fm-pr-merge.sh <task-id> <pr-url> (or land local-only work with \
                 bin/fm-merge-local.sh <task-id>); Velvet Hammer then asks the captain."
            ),
        ));
    }
    let publish = matches!(
        (name, verbs.as_slice()),
        ("gh", ["repo", "create" | "fork", ..])
            | ("gh", ["pr", "create" | "comment", ..])
            | ("gh", ["issue", "create" | "comment", ..])
            | ("gh", ["release", "create", ..])
            | ("gh", ["gist", "create", ..])
            | ("glab", ["repo", "create" | "fork", ..])
            | ("glab", ["mr" | "issue", "create" | "note", ..])
    );
    if publish {
        found.push(Hit::new(
            Rule::PublishAsks,
            format!("Publish with `{name} {} {}`?", verbs[0], verbs[1]),
            "Firstmate creates, forks or posts to a remote only with the captain's consent to \
             that exact action. Allow runs it as shown; Deny refuses it.",
        ));
    }
    found
}

/// Git subcommands that change a repository's files, history or refs. `remote`, `config`,
/// `fetch`, `clone` and `init` are left out: firstmate's own add-project procedure uses them.
const GIT_WRITES: [&str; 20] = [
    "add",
    "am",
    "apply",
    "checkout",
    "cherry-pick",
    "clean",
    "commit",
    "merge",
    "mv",
    "pull",
    "rebase",
    "reset",
    "restore",
    "revert",
    "rm",
    "stash",
    "switch",
    "tag",
    "worktree",
    "branch",
];

/// The rules a `git` command matches.
fn git_hits(
    args: &[String],
    dir: &Path,
    home: &Home,
    records: &[PathBuf],
    shown: &str,
) -> Vec<Hit> {
    // Global options come before the subcommand; -C changes the directory.
    let mut dir = dir.to_path_buf();
    let mut rest = args;
    while let Some((arg, tail)) = rest.split_first() {
        match arg.as_str() {
            "-C" => {
                if let Some(to) = tail.first() {
                    dir = home.resolve(to, &dir).unwrap_or(dir);
                }
                rest = tail.get(1..).unwrap_or_default();
            }
            "-c" => rest = tail.get(1..).unwrap_or_default(),
            a if a.starts_with('-') => rest = tail,
            _ => break,
        }
    }
    let Some((sub, sub_args)) = rest.split_first() else {
        return Vec::new();
    };
    let flag = |f: &str| sub_args.iter().any(|a| a == f);
    let mut found = Vec::new();
    if sub == "push" {
        let short_force = sub_args
            .iter()
            .any(|a| a.starts_with('-') && !a.starts_with("--") && a.contains('f'));
        let forced = short_force
            || sub_args.iter().any(|a| {
                a == "--force"
                    || a.starts_with("--force-with-lease")
                    || a == "--mirror"
                    || (a.starts_with('+') && a.len() > 1)
            });
        if forced {
            found.push(Hit::new(
                Rule::NoForcePush,
                String::new(),
                format!(
                    "`{shown}` force-pushes, which can discard work on the remote. Push without \
                     forcing; if the remote has moved, investigate and ask the captain."
                ),
            ));
        }
        // A forced push is a push too, so with the force rule off it still asks.
        {
            let remote = sub_args
                .iter()
                .find(|a| !a.starts_with('-'))
                .map_or("the branch's remote", String::as_str);
            found.push(Hit::new(
                Rule::PublishAsks,
                format!("Push to {remote} from {}?", dir.display()),
                "Firstmate pushes to a remote only with the captain's consent: its crewmates \
                 and delivery pipeline push their own work. Allow runs this push as shown; Deny \
                 refuses it.",
            ));
        }
    }
    if sub == "worktree"
        && sub_args.first().is_some_and(|a| a == "remove")
        && (flag("--force") || flag("-f"))
    {
        found.push(
            Hit::new(
                Rule::DiscardAsks,
                String::new(),
                format!(
                    "`{shown}` removes a worktree whatever it holds. Retire a task with \
                 bin/fm-teardown.sh <task-id>, which owns the landed-work check; discarding \
                 unlanded work needs the captain's explicit word."
                ),
            )
            .denied(),
        );
    }
    // `git branch` and `git tag` with no name only list.
    let lists = match sub.as_str() {
        "branch" => !sub_args.iter().any(|a| {
            [
                "-d", "-D", "-m", "-M", "-c", "-C", "-f", "--delete", "--move", "--force",
            ]
            .contains(&a.as_str())
        }),
        "tag" => sub_args.iter().all(|a| a.starts_with('-')) && !flag("-d"),
        "stash" => sub_args.first().is_some_and(|a| a == "list" || a == "show"),
        "worktree" => sub_args.first().is_some_and(|a| a == "list"),
        _ => false,
    };
    if GIT_WRITES.contains(&sub.as_str()) && !lists && home.protects(&dir, records) {
        found.push(Hit::new(
            Rule::ProjectsReadOnly,
            format!("Let the chief of staff run git {sub} in {}?", dir.display()),
            "The repository is one of firstmate's projects or task worktrees, which firstmate \
             only reads: crewmates change projects. Allow it only for an operation you asked \
             for, in this project, now.",
        ));
    }
    found
}

/// The first path a file-changing command writes in a protected place, if any.
fn changed_path(
    name: &str,
    args: &[String],
    dir: &Path,
    home: &Home,
    records: &[PathBuf],
) -> Option<PathBuf> {
    let mut targets: Vec<&String> = Vec::new();
    // A redirect writes whatever command it follows.
    let mut it = args.iter();
    let mut plain = Vec::new();
    while let Some(arg) = it.next() {
        if arg == ">" || arg == ">>" {
            targets.extend(it.next());
        } else if arg != "<" {
            plain.push(arg);
        }
    }
    let operands: Vec<&String> = plain
        .iter()
        .copied()
        .filter(|a| !a.starts_with('-'))
        .collect();
    match name {
        "rm" | "rmdir" | "unlink" | "mv" | "touch" | "mkdir" | "ln" | "chmod" | "chown"
        | "chgrp" | "truncate" | "shred" | "tee" => targets.extend(operands),
        "cp" | "install" | "rsync" | "ditto" => targets.extend(operands.last()),
        "sed" | "gsed" | "perl"
            if plain
                .iter()
                .any(|a| a.starts_with("-i") || a.starts_with("--in-place")) =>
        {
            targets.extend(operands)
        }
        _ => {}
    }
    targets
        .into_iter()
        .filter_map(|t| home.resolve(t, dir))
        .find(|p| home.protects(p, records))
}

/// `words` without what runs in front of the program: variable assignments, and `env`, `sudo`,
/// `command`, `exec`, `nohup`, `time` and `nice` with their flags. A script run with `bash`,
/// `sh` or `zsh` counts as the program.
fn program(mut words: Vec<String>) -> Vec<String> {
    loop {
        let Some(first) = words.first() else {
            return words;
        };
        let assignment = first.split_once('=').is_some_and(|(name, _)| {
            !name.is_empty() && name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())
        });
        let wrapper =
            ["env", "sudo", "command", "exec", "nohup", "time", "nice"].contains(&first.as_str());
        let shell = ["bash", "sh", "zsh"].contains(&first.as_str())
            && words.get(1).is_some_and(|w| !w.starts_with('-'));
        if assignment || shell {
            words.remove(0);
        } else if wrapper {
            words.remove(0);
            while words.first().is_some_and(|w| w.starts_with('-')) {
                words.remove(0);
            }
        } else {
            return words;
        }
    }
}

/// The words of one command part, with quotes and backslashes removed, and `>`, `>>` and `<`
/// as words of their own. A file-descriptor number in front of a redirect is dropped.
fn words(part: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    let mut chars = part.chars().peekable();
    let flush = |out: &mut Vec<String>, word: &mut String, started: &mut bool| {
        if *started || !word.is_empty() {
            out.push(std::mem::take(word));
        }
        *started = false;
    };
    while let Some(c) = chars.next() {
        match quote {
            Some(q) if c == q => quote = None,
            Some('"') if c == '\\' => word.extend(chars.next()),
            Some(_) => word.push(c),
            None => match c {
                '\'' | '"' => {
                    quote = Some(c);
                    started = true;
                }
                '\\' => {
                    word.extend(chars.next());
                    started = true;
                }
                '>' | '<' => {
                    if !word.is_empty() && word.chars().all(|d| d.is_ascii_digit()) {
                        word.clear();
                    }
                    flush(&mut out, &mut word, &mut started);
                    let mut op = c.to_string();
                    if c == '>' && chars.next_if_eq(&'>').is_some() {
                        op.push('>');
                    }
                    out.push(op);
                }
                c if c.is_whitespace() => flush(&mut out, &mut word, &mut started),
                c => word.push(c),
            },
        }
    }
    flush(&mut out, &mut word, &mut started);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A firstmate home under `target/` with one task worktree recorded outside it.
    fn home() -> (Home, PathBuf) {
        use std::sync::atomic::{AtomicU32, Ordering};
        static RUN: AtomicU32 = AtomicU32::new(0);
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/guard")
            .join(format!(
                "{}-{}",
                std::process::id(),
                RUN.fetch_add(1, Ordering::Relaxed)
            ));
        let _ = fs::remove_dir_all(&root);
        let dir = root.join("home");
        let worktree = root.join("pool/1/site");
        fs::create_dir_all(dir.join("state")).unwrap();
        fs::create_dir_all(dir.join("projects/site")).unwrap();
        fs::create_dir_all(&worktree).unwrap();
        fs::write(
            dir.join("state/site-fix.meta"),
            format!(
                "id=site-fix\nworktree={}\nproject={}\n",
                worktree.display(),
                dir.join("projects/site").display()
            ),
        )
        .unwrap();
        (Home::new(dir), worktree)
    }

    fn bash(command: &str) -> Value {
        json!({"command": command})
    }

    /// The rule and effect that decide `command` run from `cwd`, with every rule on.
    fn decide(home: &Home, cwd: &Path, command: &str) -> Option<(Rule, Effect)> {
        Settings::default()
            .check("Bash", &bash(command), cwd, home)
            .map(|h| (h.rule, h.effect))
    }

    #[test]
    fn a_lower_level_merge_is_denied_with_the_script_to_use() {
        let (home, _) = home();
        let cwd = Path::new("/clone");
        for command in [
            "gh pr merge 12 --squash",
            "gh pr merge https://github.com/o/r/pull/12",
            "gh -R o/r pr merge 12",
            "cd /tmp && gh pr merge 12",
            "glab mr merge 3",
            "tea pulls merge 4",
            "gh api -X PUT repos/o/r/pulls/12/merge",
            "GH_TOKEN=x gh pr merge 12",
        ] {
            assert_eq!(
                decide(&home, cwd, command),
                Some((Rule::MergeScriptOnly, Effect::Deny)),
                "{command}"
            );
        }
        let hit = Settings::default()
            .check("Bash", &bash("gh pr merge 12"), cwd, &home)
            .unwrap();
        assert!(hit.why.contains("bin/fm-pr-merge.sh <task-id> <pr-url>"));
        for command in [
            "gh pr view 12",
            "gh pr checks 12",
            "gh api repos/o/r/pulls/12",
        ] {
            assert_eq!(decide(&home, cwd, command), None, "{command}");
        }
    }

    #[test]
    fn the_merge_script_asks_the_captain_naming_the_pull_request() {
        let (home, _) = home();
        let cwd = Path::new("/clone");
        let hit = Settings::default()
            .check(
                "Bash",
                &bash("bin/fm-pr-merge.sh site-fix https://github.com/o/site/pull/7"),
                cwd,
                &home,
            )
            .unwrap();
        assert_eq!(hit.rule, Rule::MergeAsks);
        assert_eq!(hit.question, "Merge https://github.com/o/site/pull/7?");
        for command in [
            "\"$FM_ROOT\"/bin/fm-pr-merge.sh site-fix https://github.com/o/site/pull/7",
            "bash bin/fm-pr-merge.sh site-fix https://github.com/o/site/pull/7",
        ] {
            assert_eq!(
                decide(&home, cwd, command),
                Some((Rule::MergeAsks, Effect::Ask)),
                "{command}"
            );
        }
        let local = Settings::default()
            .check("Bash", &bash("bin/fm-merge-local.sh notes-fix"), cwd, &home)
            .unwrap();
        assert_eq!(
            local.question,
            "Land task notes-fix on its project's default branch?"
        );
        assert_eq!(decide(&home, cwd, "bin/fm-pr-merge.sh --help"), None);
    }

    #[test]
    fn an_admin_merge_is_denied_even_through_the_script() {
        let (home, _) = home();
        let cwd = Path::new("/clone");
        let script =
            "bin/fm-pr-merge.sh t https://github.com/o/r/pull/1 --attended-override -- --admin";
        assert_eq!(
            decide(&home, cwd, script),
            Some((Rule::NoAdminMerge, Effect::Deny))
        );
        // Around the script, the merge rule names the script to use; without it, --admin.
        assert_eq!(
            decide(&home, cwd, "gh pr merge 1 --admin"),
            Some((Rule::MergeScriptOnly, Effect::Deny))
        );
        let mut around = Settings::default();
        around.set(Rule::MergeScriptOnly, false);
        assert_eq!(
            around
                .check("Bash", &bash("gh pr merge 1 --admin"), cwd, &home)
                .map(|h| h.rule),
            Some(Rule::NoAdminMerge)
        );
        // Turned off, the script's ask remains: a merge still never runs unasked.
        let mut settings = Settings::default();
        settings.set(Rule::NoAdminMerge, false);
        let hit = settings
            .check(
                "Bash",
                &bash("bin/fm-pr-merge.sh t https://github.com/o/r/pull/1 --attended-override -- --admin"),
                cwd,
                &home,
            )
            .unwrap();
        assert_eq!(hit.rule, Rule::MergeAsks);
    }

    #[test]
    fn a_forced_push_is_denied_and_a_plain_one_asks() {
        let (home, _) = home();
        let cwd = Path::new("/clone");
        for command in [
            "git push --force origin main",
            "git push -f",
            "git push -uf origin fix",
            "git push --force-with-lease=main:abc origin main",
            "git push origin +fix:fix",
            "git -C /x push --mirror backup",
            "env GIT_SSH=x git push --force",
        ] {
            assert_eq!(
                decide(&home, cwd, command),
                Some((Rule::NoForcePush, Effect::Deny)),
                "{command}"
            );
        }
        let hit = Settings::default()
            .check("Bash", &bash("git push origin fix"), cwd, &home)
            .unwrap();
        assert_eq!(
            (hit.rule, hit.question.as_str()),
            (Rule::PublishAsks, "Push to origin from /clone?")
        );
        assert_eq!(decide(&home, cwd, "git status"), None);
        assert_eq!(decide(&home, cwd, "git log --oneline -f"), None);
    }

    #[test]
    fn a_forced_teardown_asks_and_a_forced_worktree_removal_is_denied() {
        let (home, worktree) = home();
        let cwd = Path::new("/clone");
        let hit = Settings::default()
            .check(
                "Bash",
                &bash("bin/fm-teardown.sh site-fix --force"),
                cwd,
                &home,
            )
            .unwrap();
        assert_eq!(
            (hit.rule, hit.question.as_str()),
            (
                Rule::DiscardAsks,
                "Discard the unlanded work of task site-fix?"
            )
        );
        assert_eq!(decide(&home, cwd, "bin/fm-teardown.sh site-fix"), None);
        let remove = format!("git worktree remove --force {}", worktree.display());
        assert_eq!(
            decide(&home, cwd, &remove),
            Some((Rule::DiscardAsks, Effect::Deny))
        );
    }

    #[test]
    fn writes_into_a_project_or_task_worktree_ask_and_reads_do_not() {
        let (home, worktree) = home();
        let project = home.dir.join("projects/site");
        let clone = Path::new("/clone");
        let edit = |path: &Path| {
            Settings::default()
                .check("Edit", &json!({"file_path": path}), clone, &home)
                .map(|h| h.question)
        };
        assert_eq!(
            edit(&project.join("src/a.rs")),
            Some(format!(
                "Let the chief of staff edit {}?",
                project.join("src/a.rs").display()
            ))
        );
        assert!(edit(&worktree.join("README.md")).is_some());
        assert!(edit(&home.dir.join("projects/../data/projects.md")).is_none());
        assert!(edit(&home.dir.join("data/backlog.md")).is_none());
        let p = project.display();
        let w = worktree.display();
        for command in [
            format!("git -C {p} commit -m x"),
            format!("cd {p} && git reset --hard"),
            "git -C $FM_HOME/projects/site checkout main".to_string(),
            format!("rm -rf {p}"),
            format!("echo hi > {w}/notes.txt"),
            format!("sed -i '' s/a/b/ {p}/README.md"),
            format!("cp /tmp/x {w}/x"),
            format!("git -C {p} branch -D old"),
        ] {
            assert_eq!(
                decide(&home, clone, &command),
                Some((Rule::ProjectsReadOnly, Effect::Ask)),
                "{command}"
            );
        }
        // Run from inside the worktree, a relative path counts too.
        assert_eq!(
            decide(&home, &worktree, "touch new.txt"),
            Some((Rule::ProjectsReadOnly, Effect::Ask))
        );
        for command in [
            format!("git -C {p} status"),
            format!("git -C {p} log --oneline"),
            format!("git -C {p} fetch"),
            format!("git -C {p} branch"),
            format!("git -C {p} remote remove origin"),
            format!("git clone /tmp/src {p}-two"),
            format!("cat {p}/README.md"),
            format!("cp {p}/README.md /tmp/readme"),
            format!("ls {w} 2>/dev/null"),
            format!("grep -r todo {p} > /tmp/todos"),
            "git commit -m 'home records'".to_string(),
        ] {
            assert_eq!(decide(&home, clone, &command), None, "{command}");
        }
    }

    #[test]
    fn publishing_asks_with_the_exact_command() {
        let (home, _) = home();
        let cwd = Path::new("/clone");
        let hit = Settings::default()
            .check("Bash", &bash("gh repo create o/new --public"), cwd, &home)
            .unwrap();
        assert_eq!(
            (hit.rule, hit.question.as_str()),
            (Rule::PublishAsks, "Publish with `gh repo create`?")
        );
        for command in [
            "gh issue comment 3 --body hi",
            "gh pr create --fill",
            "glab mr create",
        ] {
            assert_eq!(
                decide(&home, cwd, command),
                Some((Rule::PublishAsks, Effect::Ask)),
                "{command}"
            );
        }
        for command in ["gh repo view o/r", "gh issue list"] {
            assert_eq!(decide(&home, cwd, command), None, "{command}");
        }
    }

    #[test]
    fn a_rule_turned_off_matches_nothing_and_survives_a_restart() {
        let (home, _) = home();
        let mut settings = Settings::default();
        settings.set(Rule::NoForcePush, false);
        // The forced push is no longer denied, but the push still asks.
        assert_eq!(
            settings
                .check("Bash", &bash("git push -f"), Path::new("/c"), &home)
                .map(|h| h.rule),
            Some(Rule::PublishAsks)
        );
        let dir = home.dir.parent().unwrap().join("data");
        let mut store = store::Store::<Settings>::open(&dir);
        assert!(Rule::ALL.iter().all(|r| store.settings().on(*r)));
        store.change(|s| s.set(Rule::PublishAsks, false));
        let again = store::Store::<Settings>::open(&dir);
        assert!(!again.settings().on(Rule::PublishAsks));
        assert!(again.settings().on(Rule::MergeAsks));
        fs::write(
            dir.join(SETTINGS_FILE),
            r#"{"version": 1, "off": ["no-such-rule"]}"#,
        )
        .unwrap();
        let bad = store::Store::<Settings>::open(&dir);
        assert_eq!(bad.settings(), &Settings::default());
        assert!(
            bad.problem()
                .unwrap()
                .contains("every firstmate rule is on")
        );
    }

    #[test]
    fn words_split_like_a_shell_for_these_purposes() {
        assert_eq!(
            words(r#"git commit -m "two words" 'a b'\ c x>out 2>/dev/null"#),
            [
                "git",
                "commit",
                "-m",
                "two words",
                "a b c",
                "x",
                ">",
                "out",
                ">",
                "/dev/null"
            ]
        );
        assert_eq!(
            program(words("FOO=1 sudo -E env bash bin/fm-teardown.sh t")),
            ["bin/fm-teardown.sh", "t"]
        );
    }
}
