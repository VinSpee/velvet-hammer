//! Firstmate's second mates: crewmates with their own persistent firstmate home, a charter, a
//! project list, a backlog and a session, idle until the main firstmate routes work to them.
//! The app shows exactly what firstmate's records and scripts define, and changes nothing by
//! hand:
//!
//! - who the mates are comes from firstmate's registry, `data/secondmates.md`, whose line format
//!   the header of its `bin/fm-secondmate-registry-lib.sh` owns;
//! - how each one stands, and whether its session is up, comes from the bearings snapshot
//!   (see [`crate::bearings`]);
//! - what was sent to a mate comes from its steering inbox, `state/<id>.inbox/`, whose record
//!   format `bin/fm-task-inbox-lib.sh` owns, and what came back from its parent channel,
//!   `state/<id>.status`, which `docs/secondmate-parent-channel.md` describes;
//! - every change runs one of firstmate's guarded scripts: `bin/fm-home-seed.sh` creates a mate,
//!   `bin/fm-spawn.sh --secondmate` starts one, `bin/fm-control.sh` interrupts or relaunches
//!   one, `bin/fm-teardown.sh` retires one (never with `--force`), and `bin/fm-send.sh` delivers
//!   a message the way firstmate delivers a routed request, so the reply returns on the parent
//!   channel.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::Duration;

use rune::{TextEdit, Waker};

use crate::bearings::Bearings;
use crate::model::Action;
use crate::projects::Project;
use crate::{App, Showing};

/// How often the watcher reads the records again.
const POLL: Duration = Duration::from_secs(2);

/// The marker firstmate puts before a routed request (`FM_FROMFIRST_MARK` in
/// `bin/fm-operational-input.sh`): a label, then an invisible separator.
const FROM_FIRSTMATE: &str = "[fm-from-firstmate]\u{2063}";

/// The note `bin/fm-control.sh relaunch` requires, saying why the agent was replaced.
const RELAUNCH_NOTE: &str = "Relaunched by the captain from Velvet Hammer.";

/// Where the app records which messages the captain sent, by correlation id, so the
/// conversation can tell them from what firstmate sent.
pub const SENT_FILE: &str = "secondmate-sent.jsonl";

/// One second mate in firstmate's registry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mate {
    pub id: String,
    /// The one-sentence charter summary.
    pub summary: String,
    /// The natural-language responsibility firstmate routes work by.
    pub scope: String,
    pub projects: Vec<String>,
    /// The mate's own firstmate home.
    pub home: String,
    /// The SSH host of a mate whose home is on another machine.
    pub host: Option<String>,
    pub added: String,
}

/// The second mates in firstmate home `home`'s registry, in its order. A home or registry that
/// does not exist yet has none.
pub fn registry(home: &Path) -> Result<Vec<Mate>, String> {
    let path = home.join("data/secondmates.md");
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(text.lines().filter_map(entry).collect()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("Could not read {}: {e}", path.display())),
    }
}

/// One registry line, or `None` for a line firstmate's parser would not accept. The summary and
/// scope may hold parentheses and semicolons, so, as in firstmate's parser, the fields are found
/// from the generated suffix's markers.
fn entry(line: &str) -> Option<Mate> {
    let line = line.trim_end();
    let rest = line.strip_prefix("- ")?;
    let (id, rest) = rest.split_once(" - ")?;
    let valid_id = |id: &str| {
        !id.is_empty()
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    };
    if !valid_id(id) {
        return None;
    }
    let body = rest.strip_suffix(')')?;
    let (added_at, added) = body.rsplit_once("; added ")?;
    let (scoped, projects) = added_at.rsplit_once("; projects:")?;
    let (placed, scope) = scoped.rsplit_once("; scope:")?;
    let (summary, host, home) = match placed.rsplit_once(" (home:") {
        Some((summary, home)) if !summary.contains(" (host:") => (summary, None, home),
        _ => {
            let (summary, remote) = placed.rsplit_once(" (host:")?;
            let (host, rest) = remote.split_once("; root:")?;
            let (_, home) = rest.split_once("; home:")?;
            (summary, Some(host.trim().to_string()), home)
        }
    };
    let projects = projects
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect();
    Some(Mate {
        id: id.to_string(),
        summary: summary.trim().to_string(),
        scope: scope.trim().to_string(),
        projects,
        home: home.trim().to_string(),
        host,
        added: added.trim().to_string(),
    })
}

/// One line on a mate's parent channel: something it, or one of its home's scripts, returned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// Such as `done`, `needs-decision`, `blocked` or `failed`.
    pub verb: String,
    pub at: Option<u64>,
    /// The correlation id of the request this answers.
    pub corr: Option<String>,
    pub note: String,
}

/// A status line: `<verb> [<field>=<value>]...: <note>`.
fn status_line(text: &str) -> Option<Line> {
    let (head, note) = text.split_once(": ")?;
    let mut words = head.split(' ');
    let verb = words.next()?.to_string();
    if verb.is_empty() {
        return None;
    }
    let (mut at, mut corr) = (None, None);
    for field in words {
        let field = field.strip_prefix('[')?.strip_suffix(']')?;
        match field.split_once('=') {
            Some(("at", value)) => at = value.parse().ok(),
            Some(("corr", value)) => corr = Some(value.to_string()),
            _ => {}
        }
    }
    let note = note.trim();
    let note = note.strip_suffix(" (via-helper)").unwrap_or(note);
    Some(Line {
        verb,
        at,
        corr,
        note: note.to_string(),
    })
}

/// One message delivered to a mate's steering inbox.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub seq: u32,
    pub at: Option<u64>,
    pub corr: Option<String>,
    /// The words, without firstmate's marker and correlation token.
    pub text: String,
    /// The mate moved it to its inbox's `handled/`: it read it.
    pub read: bool,
}

/// An inbox record: `key=value` lines, `--`, then the body. With `body-terminator=1` the writer
/// added one final newline that is not part of the body.
fn inbox_record(seq: u32, record: &str, read: bool) -> Option<Request> {
    let (header, body) = record.split_once("\n--\n")?;
    let mut at = None;
    let mut terminated = false;
    for line in header.lines() {
        match line.split_once('=') {
            Some(("at", value)) => at = iso_epoch(value),
            Some(("body-terminator", "1")) => terminated = true,
            _ => {}
        }
    }
    let body = if terminated {
        body.strip_suffix('\n').unwrap_or(body)
    } else {
        body
    };
    let mut text = body.strip_prefix(FROM_FIRSTMATE).unwrap_or(body);
    let mut corr = None;
    while let Some((token, rest)) = text.split_once(' ') {
        match token.split_once('=') {
            Some(("corr", id)) => corr = Some(id.to_string()),
            Some(("delivery", _)) => {}
            _ => break,
        }
        text = rest;
    }
    Some(Request {
        seq,
        at,
        corr,
        text: text.trim().to_string(),
        read,
    })
}

/// Seconds since 1970 for a UTC time written `YYYY-MM-DDTHH:MM:SSZ`.
fn iso_epoch(text: &str) -> Option<u64> {
    let text = text.strip_suffix('Z')?;
    let (date, time) = text.split_once('T')?;
    let mut date = date.split('-').map(|n| n.parse::<i64>());
    let (y, m, d) = (date.next()?.ok()?, date.next()?.ok()?, date.next()?.ok()?);
    let mut time = time.split(':').map(|n| n.parse::<i64>());
    let (hh, mm, ss) = (time.next()?.ok()?, time.next()?.ok()?, time.next()?.ok()?);
    // Howard Hinnant's days-from-civil algorithm.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + hh * 3_600 + mm * 60 + ss).ok()
}

/// What was sent to one mate and what came back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Channel {
    pub sent: Vec<Request>,
    pub back: Vec<Line>,
}

/// One entry of a conversation, in time order.
pub enum Said<'a> {
    Sent(&'a Request),
    Back(&'a Line),
}

impl Channel {
    /// The messages sent and the replies to them, oldest first. A reply carries the correlation
    /// id of the request it answers.
    pub fn conversation(&self) -> Vec<Said<'_>> {
        let mut all: Vec<(u64, Said)> = self
            .sent
            .iter()
            .map(|r| (r.at.unwrap_or(u64::MAX), Said::Sent(r)))
            .chain(
                self.back
                    .iter()
                    .filter(|l| l.corr.is_some())
                    .map(|l| (l.at.unwrap_or(u64::MAX), Said::Back(l))),
            )
            .collect();
        all.sort_by_key(|(at, _)| *at);
        all.into_iter().map(|(_, said)| said).collect()
    }

    /// The newest lines that answer no request, such as a child's pull request or a held
    /// decision: what the mate's work returned. Newest first.
    pub fn outcomes(&self, n: usize) -> Vec<&Line> {
        self.back
            .iter()
            .rev()
            .filter(|l| l.corr.is_none())
            .take(n)
            .collect()
    }

    /// The request with this correlation id.
    pub fn request(&self, corr: &str) -> Option<&Request> {
        self.sent.iter().find(|r| r.corr.as_deref() == Some(corr))
    }
}

/// Mate `id`'s channel, from the parent home `home`. A file that is missing reads as empty.
pub fn read_channel(home: &Path, id: &str) -> Channel {
    let state = home.join("state");
    let back = std::fs::read_to_string(state.join(format!("{id}.status")))
        .unwrap_or_default()
        .lines()
        .filter_map(status_line)
        .collect();
    let inbox = state.join(format!("{id}.inbox"));
    let mut sent = Vec::new();
    for (dir, read) in [(inbox.join("handled"), true), (inbox, false)] {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(seq) = name.strip_suffix(".msg").and_then(|n| n.parse().ok()) else {
                continue;
            };
            if let Ok(record) = std::fs::read_to_string(entry.path())
                && let Some(request) = inbox_record(seq, &record, read)
            {
                sent.push(request);
            }
        }
    }
    sent.sort_by_key(|r| r.seq);
    Channel { sent, back }
}

/// Everything the app reads about second mates from one home's records.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Records {
    pub mates: Result<Vec<Mate>, String>,
    pub channels: BTreeMap<String, Channel>,
}

pub fn read(home: &Path) -> Records {
    let mates = registry(home);
    let channels = mates
        .iter()
        .flatten()
        .map(|m| (m.id.clone(), read_channel(home, &m.id)))
        .collect();
    Records { mates, channels }
}

/// Keeps one home's [`Records`] current until dropped.
pub struct Watcher {
    rx: Receiver<Records>,
    wake: Arc<OnceLock<Waker>>,
    stop: Arc<AtomicBool>,
}

impl Watcher {
    pub fn start(home: &Path) -> Watcher {
        let (tx, rx) = mpsc::channel();
        let wake: Arc<OnceLock<Waker>> = Arc::new(OnceLock::new());
        let stop = Arc::new(AtomicBool::new(false));
        let home = home.to_path_buf();
        let (waker, stopped) = (wake.clone(), stop.clone());
        thread::spawn(move || {
            let mut shown = None;
            while !stopped.load(Ordering::Relaxed) {
                let records = read(&home);
                if shown.as_ref() != Some(&records) {
                    shown = Some(records.clone());
                    if tx.send(records).is_err() {
                        return;
                    }
                    if let Some(wake) = waker.get() {
                        wake();
                    }
                }
                thread::sleep(POLL);
            }
        });
        Watcher { rx, wake, stop }
    }

    pub fn connect(&mut self, wake: Waker) {
        let _ = self.wake.set(wake);
        if let Some(wake) = self.wake.get() {
            wake();
        }
    }

    fn poll(&mut self) -> Option<Records> {
        self.rx.try_iter().last()
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Whether a mate's session is up, from its endpoint in the bearings snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Session {
    /// Its terminal exists and firstmate does not read its agent as dead.
    Up,
    /// Its terminal is gone or its agent has exited.
    Stopped,
    /// Firstmate has no task record for it: it was never started, or its record was removed.
    NotStarted,
}

impl Session {
    pub fn of(bearings: &Bearings, id: &str) -> Session {
        match bearings.endpoint(id) {
            None => Session::NotStarted,
            Some(e) if e.exists && e.agent != "dead" => Session::Up,
            Some(_) => Session::Stopped,
        }
    }

    pub fn words(self) -> &'static str {
        match self {
            Session::Up => "running",
            Session::Stopped => "stopped",
            Session::NotStarted => "not started",
        }
    }
}

/// Firstmate's word for how a mate's work stands, in plain words.
pub fn work_words(state: &str) -> &str {
    match state {
        "no_active_work" => "idle",
        "active_child_work" => "working",
        "captain_decision" => "waiting on you",
        "externally_held" => "held",
        "unknown" => "state unknown",
        other => other,
    }
}

/// Where firstmate's records disagree about mate `id`, in firstmate's words where it gave them;
/// `None` when they agree.
pub fn disagreement(bearings: &Bearings, mates: &[Mate], id: &str) -> Option<String> {
    if !bearings.read {
        return None;
    }
    let registered = mates.iter().any(|m| m.id == id);
    match bearings.mate(id) {
        None if registered => Some(
            "It is in the registry, data/secondmates.md, but firstmate's bearings snapshot does \
             not report it."
                .into(),
        ),
        None => None,
        Some(_) if !registered => Some(
            "Firstmate has a task record for it, but it is not in the registry, \
             data/secondmates.md."
                .into(),
        ),
        Some(m) if m.contradiction => Some(format!(
            "What it reported on the parent channel contradicts its own home's records.{}",
            if m.reason != "-" && !m.reason.is_empty() {
                format!(" Firstmate says: {}.", m.reason)
            } else {
                String::new()
            }
        )),
        Some(m) if m.state == "unknown" => Some(format!(
            "Firstmate cannot read its home: {}.",
            if m.reason != "-" && !m.reason.is_empty() {
                &m.reason
            } else {
                &m.doing
            }
        )),
        Some(_) => None,
    }
}

/// The second mate a chief-of-staff command routes work to: `bin/fm-send.sh <id>` or
/// `bin/fm-backlog-handoff.sh <id>` naming a registered mate.
pub fn routed_to<'a>(command: &str, mates: &'a [Mate]) -> Option<&'a Mate> {
    let words: Vec<&str> = command
        .split_whitespace()
        .map(|w| w.trim_matches(|c| c == '\'' || c == '"'))
        .collect();
    words.iter().enumerate().find_map(|(i, word)| {
        if !(word.ends_with("fm-send.sh") || word.ends_with("fm-backlog-handoff.sh")) {
            return None;
        }
        let target = words[i + 1..].iter().find(|w| !w.starts_with('-'))?;
        mates.iter().find(|m| m.id == *target)
    })
}

/// A new second mate, as the captain described it in the guided flow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub id: String,
    pub charter: String,
    /// The mate's new home: a local clone of the managed firstmate clone.
    pub home: PathBuf,
    /// Empty for a mate with no projects, seeded with `--no-projects`.
    pub projects: Vec<String>,
    /// A runtime the captain chose over firstmate's default, passed to `fm-spawn.sh`.
    pub harness: Option<String>,
    /// The runtime it starts with, in words.
    pub runtime: String,
}

impl Plan {
    /// Exactly what Velvet Hammer will run and what that creates, for the confirmation.
    pub fn steps(&self, clone: &Path, home: &Path) -> Vec<String> {
        let projects = if self.projects.is_empty() {
            "--no-projects".to_string()
        } else {
            self.projects.join(" ")
        };
        let mut steps = vec![format!(
            "Run firstmate's bin/fm-home-seed.sh {} {} {projects}, with your words as its charter. \
             It writes the charter to {}, creates the mate's home at {} as a local git clone of \
             {}, and adds a line for {} to {}.",
            self.id,
            self.home.display(),
            home.join("data").join(&self.id).join("brief.md").display(),
            self.home.display(),
            clone.display(),
            self.id,
            home.join("data/secondmates.md").display(),
        )];
        if self.projects.is_empty() {
            steps.push(
                "It clones no projects: a mate with no projects works on firstmate itself.".into(),
            );
        } else {
            steps.push(format!(
                "It clones {} from {} into {}, and sets up no-mistakes in each clone whose \
                 project ships with it. If any step fails, firstmate undoes the others.",
                self.projects.join(", "),
                home.join("projects").display(),
                self.home.join("projects").display(),
            ));
        }
        steps.push(format!(
            "Then run bin/fm-spawn.sh {} --secondmate{}, which starts its session in a terminal \
             firstmate manages, running {}. It stays idle until work is routed to it.",
            self.id,
            self.harness
                .as_ref()
                .map(|h| format!(" --harness {h}"))
                .unwrap_or_default(),
            self.runtime,
        ));
        steps.push(
            "Then ask your chief of staff, in the chat, to hand it the queued work that fits its \
             scope."
                .into(),
        );
        steps.push(
            "Nothing remote is created, pushed or published: every clone is made on this \
             computer from copies already here."
                .into(),
        );
        steps
    }
}

/// Something the captain asked of firstmate's scripts about a second mate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Create(Plan),
    Start(String),
    Interrupt(String),
    Relaunch(String),
    Retire(String),
    /// A message, delivered as a routed request. `hold` is the captain hold it answers.
    Send {
        id: String,
        text: String,
        hold: Option<String>,
    },
}

impl Op {
    pub fn id(&self) -> &str {
        match self {
            Op::Create(plan) => &plan.id,
            Op::Start(id) | Op::Interrupt(id) | Op::Relaunch(id) | Op::Retire(id) => id,
            Op::Send { id, .. } => id,
        }
    }

    /// The question that confirms it, what it does, and the yes button's words.
    pub fn confirmation(&self, mate: Option<&Mate>) -> (String, String, String) {
        let home = mate.map_or("its home".to_string(), |m| m.home.clone());
        match self {
            Op::Start(id) => (
                format!("Start {id}?"),
                format!(
                    "Firstmate's bin/fm-spawn.sh {id} --secondmate starts its session in its home \
                     at {home}, with the runtime firstmate's configuration names. It picks up \
                     only work already its own, then waits for routed work."
                ),
                format!("Start {id}"),
            ),
            Op::Interrupt(id) => (
                format!("Interrupt {id}?"),
                format!(
                    "Firstmate's bin/fm-control.sh {id} interrupt stops the turn it is taking \
                     now. Its session keeps running and nothing is discarded."
                ),
                format!("Interrupt {id}"),
            ),
            Op::Relaunch(id) => (
                format!("Relaunch {id}?"),
                format!(
                    "Firstmate's bin/fm-control.sh {id} relaunch stops its agent and starts a new \
                     one in the same home, with the runtime firstmate's configuration names. Its \
                     work, backlog and uncommitted changes stay where they are."
                ),
                format!("Relaunch {id}"),
            ),
            Op::Retire(id) => (
                format!("Retire {id}?"),
                format!(
                    "Firstmate's bin/fm-teardown.sh {id} stops its session, removes its line from \
                     data/secondmates.md and deletes its home at {home}. Firstmate refuses, and \
                     changes nothing, while the mate has work in its home or owes a reply. \
                     Velvet Hammer never forces a retirement, so no work is discarded."
                ),
                format!("Retire {id}"),
            ),
            Op::Create(plan) => (
                format!("Create {}?", plan.id),
                String::new(),
                format!("Yes, create {}", plan.id),
            ),
            Op::Send { id, .. } => (format!("Send to {id}?"), String::new(), "Send".into()),
        }
    }
}

/// What came of an [`Op`], in plain words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub op: Op,
    /// Firstmate did what was asked. For a new mate: it was created, even if it did not start.
    pub ok: bool,
    pub words: String,
    /// The correlation id of a message that was sent.
    pub corr: Option<String>,
}

/// Runs `op` with the scripts of the firstmate clone `clone` against home `home`. It blocks:
/// call it off the UI thread.
pub fn perform(clone: &Path, home: &Path, op: Op) -> Outcome {
    let run = |script: &str, args: &[&str], env: &[(&str, &str)]| {
        run_script(clone, home, script, args, env)
    };
    let outcome = |ok: bool, words: String| Outcome {
        op: op.clone(),
        ok,
        words,
        corr: None,
    };
    match &op {
        Op::Create(plan) => {
            let home_arg = plan.home.to_string_lossy().into_owned();
            let mut args = vec![plan.id.as_str(), home_arg.as_str()];
            if plan.projects.is_empty() {
                args.push("--no-projects");
            } else {
                args.extend(plan.projects.iter().map(String::as_str));
            }
            if let Err(why) = run(
                "fm-home-seed.sh",
                &args,
                &[("FM_SECONDMATE_CHARTER", &plan.charter)],
            ) {
                return outcome(
                    false,
                    format!(
                        "Firstmate refused to create {}, and undid what it had started, so \
                         nothing was created. It said: {why}",
                        plan.id
                    ),
                );
            }
            let mut spawn = vec![plan.id.as_str(), "--secondmate"];
            if let Some(harness) = &plan.harness {
                spawn.extend(["--harness", harness.as_str()]);
            }
            match run("fm-spawn.sh", &spawn, &[]) {
                Ok(_) => outcome(
                    true,
                    format!(
                        "Firstmate created {} with its home at {} and started its session. It \
                         is idle until work is routed to it.",
                        plan.id,
                        plan.home.display()
                    ),
                ),
                Err(why) => outcome(
                    true,
                    format!(
                        "Firstmate created {} with its home at {}, but its session did not \
                         start. Firstmate said: {why} Choose Start to try again.",
                        plan.id,
                        plan.home.display()
                    ),
                ),
            }
        }
        Op::Start(id) => match run("fm-spawn.sh", &[id, "--secondmate"], &[]) {
            Ok(_) => outcome(true, format!("Firstmate started {id}.")),
            Err(why) => outcome(
                false,
                format!("Firstmate did not start {id}. It said: {why}"),
            ),
        },
        Op::Interrupt(id) => match run("fm-control.sh", &[id, "interrupt"], &[]) {
            Ok(_) => outcome(true, format!("Firstmate interrupted {id}'s turn.")),
            Err(why) => outcome(
                false,
                format!("Firstmate could not interrupt {id}. It said: {why}"),
            ),
        },
        Op::Relaunch(id) => match run(
            "fm-control.sh",
            &[id, "relaunch", "--note", RELAUNCH_NOTE],
            &[],
        ) {
            Ok(_) => outcome(true, format!("Firstmate relaunched {id}.")),
            Err(why) => outcome(
                false,
                format!("Firstmate could not relaunch {id}. It said: {why}"),
            ),
        },
        Op::Retire(id) => match run("fm-teardown.sh", &[id], &[]) {
            Ok(_) => outcome(
                true,
                format!("Firstmate retired {id}: its session, registry line and home are gone."),
            ),
            Err(why) => outcome(
                false,
                format!(
                    "Firstmate refused to retire {id}, and nothing was changed. It said: {why} \
                     Velvet Hammer never forces a retirement; let its work finish, or ask your \
                     chief of staff."
                ),
            ),
        },
        Op::Send { id, text, .. } => {
            let before = read_channel(home, id).sent.iter().map(|r| r.seq).max();
            match run("fm-send.sh", &[id, text], &[]) {
                Ok(_) => {
                    // fm-send prints nothing on success; the new inbox record carries the
                    // correlation id its reply will name.
                    let corr = read_channel(home, id)
                        .sent
                        .into_iter()
                        .filter(|r| Some(r.seq) > before)
                        .find(|r| r.text == text.trim())
                        .and_then(|r| r.corr);
                    Outcome {
                        corr,
                        ..outcome(
                            true,
                            format!(
                                "Delivered to {id}'s inbox. Its reply comes back on the parent \
                                 channel and shows here."
                            ),
                        )
                    }
                }
                Err(why) => outcome(
                    false,
                    format!("Firstmate did not deliver the message to {id}. It said: {why}"),
                ),
            }
        }
    }
}

/// Runs `bin/<script>` from the clone with `FM_HOME` set to `home`; its output on success, or
/// firstmate's reason when it fails.
fn run_script(
    clone: &Path,
    home: &Path,
    script: &str,
    args: &[&str],
    env: &[(&str, &str)],
) -> Result<String, String> {
    let mut command = Command::new(clone.join("bin").join(script));
    command
        .args(args)
        .current_dir(clone)
        .env("FM_HOME", home)
        .stdin(Stdio::null());
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command
        .output()
        .map_err(|e| format!("Could not run bin/{script}: {e}."))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if output.status.success() {
        return Ok(stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(reason(&stderr, &stdout)
        .unwrap_or_else(|| format!("bin/{script} failed ({}).", output.status)))
}

/// Firstmate's reason in a failed script's output: from its first `error:` or `REFUSED:` line
/// to the end, else its last line. Supervision banners and warnings are left out.
fn reason(stderr: &str, stdout: &str) -> Option<String> {
    let lines: Vec<&str> = stderr
        .lines()
        .chain(stdout.lines())
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('\u{25cf}') && !l.starts_with("WARNING:"))
        .collect();
    let first = lines
        .iter()
        .position(|l| l.starts_with("error:") || l.starts_with("REFUSED:"));
    match first {
        Some(at) => Some(lines[at..].join(" ")),
        None => lines.last().map(|l| l.to_string()),
    }
}

/// The runtimes firstmate's configuration names for launching mate `id`: the one it resolves
/// for a second mate first, then the crewmate one when it differs. One entry means there is no
/// choice to make.
pub fn runtimes(clone: &Path, home: &Path, id: &str) -> Result<Vec<String>, String> {
    let mut found: Vec<String> = Vec::new();
    for args in [vec!["secondmate", id], vec!["crew"]] {
        let named = run_script(clone, home, "fm-harness.sh", &args, &[])?;
        let named = named.trim().to_string();
        if !named.is_empty() && named != "unknown" && !found.contains(&named) {
            found.push(named);
        }
    }
    if found.is_empty() {
        return Err("Firstmate's configuration names no runtime.".into());
    }
    Ok(found)
}

/// A step of the guided flow for adding a second mate, one question each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Scope,
    Projects,
    Name,
    Runtime,
    Confirm,
}

/// The guided flow for adding a second mate, as far as the captain has answered.
#[derive(Default)]
pub struct AddMate {
    pub step: Option<Step>,
    /// What the mate is responsible for, in the captain's words: its charter and routing scope.
    pub charter: TextEdit,
    /// The chosen projects, by name.
    pub projects: Vec<String>,
    /// The captain chose no projects.
    pub no_projects: bool,
    pub name: TextEdit,
    /// The runtimes firstmate's configuration offers, or why it could not say.
    pub runtimes: Option<Result<Vec<String>, String>>,
    /// The chosen runtime, by its index in `runtimes`.
    pub runtime: usize,
    /// Why the answer cannot be taken, or firstmate's refusal.
    pub problem: Option<String>,
}

impl AddMate {
    pub fn step(&self) -> Step {
        self.step.unwrap_or(Step::Scope)
    }

    /// Whether firstmate's configuration offers a choice of runtime.
    pub fn runtime_choice(&self) -> bool {
        matches!(&self.runtimes, Some(Ok(r)) if r.len() > 1)
    }

    /// The number of this step and how many there are.
    pub fn progress(&self) -> (usize, usize) {
        let total = if self.runtime_choice() { 5 } else { 4 };
        let n = match self.step() {
            Step::Scope => 1,
            Step::Projects => 2,
            Step::Name => 3,
            Step::Runtime => 4,
            Step::Confirm => total,
        };
        (n, total)
    }

    pub fn back(&mut self) {
        self.problem = None;
        self.step = Some(match self.step() {
            Step::Scope | Step::Projects => Step::Scope,
            Step::Name => Step::Projects,
            Step::Runtime => Step::Name,
            Step::Confirm if self.runtime_choice() => Step::Runtime,
            Step::Confirm => Step::Name,
        });
    }

    /// Why `name` cannot name a new second mate, if it cannot.
    pub fn name_problem(name: &str, mates: &[Mate]) -> Option<String> {
        if name.is_empty() {
            return Some("Give it a name.".into());
        }
        if name == "." || name == ".." || name.starts_with('-') {
            return Some(format!("{name} cannot be a name."));
        }
        if !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        {
            return Some(
                "Use only letters, digits, dots, hyphens and underscores, as firstmate's \
                 registry requires."
                    .into(),
            );
        }
        if mates.iter().any(|m| m.id == name) {
            return Some(format!("There is already a second mate named {name}."));
        }
        None
    }

    /// A name to start from: the first chosen project, else the scope's first word.
    fn suggested_name(&self) -> String {
        let source = self.projects.first().cloned().unwrap_or_else(|| {
            self.charter
                .text
                .split_whitespace()
                .next()
                .unwrap_or("")
                .into()
        });
        source
            .to_lowercase()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
            .collect()
    }

    /// The plan the answers describe, with the mate's home under `mates_dir`.
    pub fn plan(&self, mates_dir: &Path) -> Plan {
        let id = self.name.text.trim().to_string();
        let (harness, runtime) = match &self.runtimes {
            Some(Ok(found)) if self.runtime > 0 => (
                found.get(self.runtime).cloned(),
                format!(
                    "{}, as you chose for this launch",
                    found.get(self.runtime).cloned().unwrap_or_default()
                ),
            ),
            Some(Ok(found)) => (
                None,
                format!("{}, which firstmate's configuration names", found[0]),
            ),
            _ => (None, "the runtime firstmate's configuration names".into()),
        };
        Plan {
            home: mates_dir.join(&id),
            id,
            charter: self.charter.text.trim().to_string(),
            projects: if self.no_projects {
                Vec::new()
            } else {
                self.projects.clone()
            },
            harness,
            runtime,
        }
    }
}

/// Whether a project can go to a second mate, and why not when it cannot: firstmate keeps
/// `local-only` projects with the main firstmate and seeds only registered projects.
pub fn eligible(project: &Project) -> Result<(), &'static str> {
    match &project.posture {
        None => Err("not in firstmate's project registry yet"),
        Some(posture) if posture.split_whitespace().any(|w| w == "local-only") => {
            Err("local-only projects stay with the main firstmate")
        }
        Some(_) => Ok(()),
    }
}

/// What the app keeps about second mates besides firstmate's records.
#[derive(Default)]
pub struct Mates {
    watcher: Option<Watcher>,
    /// The registry and channels, or `None` until the first read and always for a chief of
    /// staff that is not firstmate.
    pub records: Option<Records>,
    /// The guided flow for adding one, while it is open.
    pub adding: AddMate,
    /// An action waiting for the captain's yes.
    pub confirming: Option<Op>,
    /// The action firstmate's scripts are running, and where its outcome arrives.
    running: Option<(Op, Receiver<Outcome>)>,
    /// The last outcome for each mate, by id.
    pub outcomes: HashMap<String, Outcome>,
    /// The message being written to each mate.
    pub drafts: HashMap<String, TextEdit>,
    /// The captain hold the next message to a mate answers: (mate, hold key).
    pub answering: Option<(String, String)>,
    /// Correlation ids of the messages the captain sent.
    pub sent: HashSet<String>,
    sent_file: Option<PathBuf>,
}

impl Mates {
    /// Watches the second mates of firstmate home `home`, when the chief of staff runs as
    /// firstmate, recalling the captain's sent messages from `data`.
    pub fn new(home: Option<&Path>, data: &Path) -> Mates {
        let sent_file = data.join(SENT_FILE);
        let sent = std::fs::read_to_string(&sent_file)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter_map(|v| v["corr"].as_str().map(str::to_string))
            .collect();
        Mates {
            watcher: home.map(Watcher::start),
            sent,
            sent_file: Some(sent_file),
            ..Mates::default()
        }
    }

    pub fn connect(&mut self, wake: Waker) {
        if let Some(watcher) = &mut self.watcher {
            watcher.connect(wake);
        }
    }

    /// The registered mates, empty until they are read.
    pub fn list(&self) -> &[Mate] {
        match &self.records {
            Some(Records {
                mates: Ok(list), ..
            }) => list,
            _ => &[],
        }
    }

    pub fn get(&self, id: &str) -> Option<&Mate> {
        self.list().iter().find(|m| m.id == id)
    }

    pub fn channel(&self, id: &str) -> Option<&Channel> {
        self.records.as_ref()?.channels.get(id)
    }

    /// The action running now.
    pub fn running(&self) -> Option<&Op> {
        self.running.as_ref().map(|(op, _)| op)
    }

    pub fn draft(&mut self, id: &str) -> &mut TextEdit {
        self.drafts.entry(id.to_string()).or_default()
    }

    fn remember_sent(&mut self, id: &str, corr: &str) {
        self.sent.insert(corr.to_string());
        let Some(path) = &self.sent_file else { return };
        let line = serde_json::json!({"mate": id, "corr": corr}).to_string();
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(file, "{line}");
        }
    }
}

/// What the captain says to the chief of staff after a new mate is created: firstmate's
/// procedure leaves the record intake and the backlog handoff to its judgment.
pub fn handoff(plan: &Plan, outcome: &str) -> String {
    format!(
        "I added the second mate {} through Velvet Hammer, which ran firstmate's \
         bin/fm-home-seed.sh with the charter \"{}\" and {}. {} Continue your \
         secondmate-provisioning procedure from after seeding: if this is an existing or \
         inherited domain, do its record intake first; then hand it the queued backlog items \
         that fit its scope with bin/fm-backlog-handoff.sh. Tell me in plain words which items \
         you moved, or that none fit.",
        plan.id,
        plan.charter,
        if plan.projects.is_empty() {
            "no projects".to_string()
        } else {
            format!("the projects {}", plan.projects.join(", "))
        },
        outcome,
    )
}

impl App {
    /// Takes in new records and a finished action; true when anything changed.
    pub fn drain_mates(&mut self) -> bool {
        let mut changed = false;
        if let Some(records) = self.mates.watcher.as_mut().and_then(|w| w.poll()) {
            self.mates.records = Some(records);
            changed = true;
        }
        let done = match &self.mates.running {
            Some((_, rx)) => rx.try_recv().ok(),
            None => None,
        };
        if let Some(outcome) = done {
            self.mates.running = None;
            self.mate_done(outcome);
            changed = true;
        }
        changed
    }

    fn mate_done(&mut self, mut outcome: Outcome) {
        match outcome.op.clone() {
            Op::Create(plan) if outcome.ok => {
                self.mates.adding = AddMate::default();
                self.showing = Showing::Mate(plan.id.clone());
                if self.chat_ready() {
                    self.act(Action::Say(handoff(&plan, &outcome.words)));
                } else {
                    outcome.words.push_str(
                        " Your chief of staff cannot chat yet; once it can, ask it to hand the \
                         new mate the queued work that fits its scope.",
                    );
                }
            }
            Op::Create(_) => {
                self.mates.adding.problem = Some(outcome.words);
                return;
            }
            Op::Send { id, hold, .. } if outcome.ok => {
                if let Some(corr) = &outcome.corr {
                    self.mates.remember_sent(&id, corr);
                }
                if hold.is_some() {
                    self.mates.answering = None;
                }
            }
            _ => {}
        }
        self.mates
            .outcomes
            .insert(outcome.op.id().to_string(), outcome);
    }

    /// Opens a mate's page.
    pub fn open_mate(&mut self, id: &str) {
        self.mates.confirming = None;
        self.showing = Showing::Mate(id.to_string());
    }

    /// Opens the guided flow for adding a second mate, with the scope filled in when the chief
    /// of staff opened it from the chat.
    pub fn add_mate(&mut self, scope: Option<String>) {
        if self.mates.running().is_some() {
            return;
        }
        let mut flow = AddMate::default();
        if let Some(scope) = scope {
            flow.charter.text = scope.trim().to_string();
        }
        self.mates.adding = flow;
        self.showing = Showing::AddMate;
    }

    /// Takes the answer to the current question and moves to the next one.
    pub fn add_mate_next(&mut self) {
        let mates = self.mates.list().to_vec();
        let flow = &mut self.mates.adding;
        flow.problem = None;
        match flow.step() {
            Step::Scope => {
                let charter = flow.charter.text.trim();
                if charter.is_empty() {
                    flow.problem = Some("Say what it should be responsible for.".into());
                } else if charter.contains("{TASK}") {
                    flow.problem =
                        Some("Firstmate refuses a charter that still says {TASK}.".into());
                } else {
                    flow.step = Some(Step::Projects);
                }
            }
            Step::Projects => {
                if flow.projects.is_empty() && !flow.no_projects {
                    flow.problem = Some("Choose its projects, or choose No projects.".into());
                } else {
                    if flow.name.text.trim().is_empty() {
                        flow.name.text = flow.suggested_name();
                    }
                    flow.step = Some(Step::Name);
                }
            }
            Step::Name => {
                let name = flow.name.text.trim().to_string();
                if let Some(problem) = AddMate::name_problem(&name, &mates) {
                    flow.problem = Some(problem);
                    return;
                }
                let found = match self.chief.workplace() {
                    Some(clone) => runtimes(clone, &self.firstmate.home_dir(), &name),
                    None => Err("The chief of staff does not run as firstmate.".into()),
                };
                let flow = &mut self.mates.adding;
                flow.runtime = 0;
                flow.runtimes = Some(found);
                flow.step = Some(if flow.runtime_choice() {
                    Step::Runtime
                } else {
                    Step::Confirm
                });
            }
            Step::Runtime => flow.step = Some(Step::Confirm),
            Step::Confirm => {}
        }
    }

    /// Chooses project `name` for the new mate, or unchooses it.
    pub fn toggle_mate_project(&mut self, name: &str) {
        let flow = &mut self.mates.adding;
        flow.no_projects = false;
        match flow.projects.iter().position(|p| p == name) {
            Some(at) => {
                flow.projects.remove(at);
            }
            None => flow.projects.push(name.to_string()),
        }
    }

    /// Where a new mate's home goes: beside the app's firstmate home.
    pub fn mates_dir(&self) -> PathBuf {
        self.firstmate.home_dir().with_file_name("secondmates")
    }

    /// The captain said yes to the plan: firstmate's scripts create the mate.
    pub fn create_mate(&mut self) {
        let plan = self.mates.adding.plan(&self.mates_dir());
        self.mates.adding.problem = None;
        self.perform_mate(Op::Create(plan));
    }

    /// Asks the captain to confirm `op` before it runs.
    pub fn confirm_mate(&mut self, op: Op) {
        self.mates.confirming = Some(op);
    }

    /// Runs the confirmed action.
    pub fn confirmed_mate(&mut self) {
        if let Some(op) = self.mates.confirming.take() {
            self.perform_mate(op);
        }
    }

    /// Sends the draft to mate `id`, as an answer to its captain hold when one is being
    /// answered.
    pub fn send_to_mate(&mut self, id: &str) {
        let words = self.mates.draft(id).text.trim().to_string();
        if words.is_empty() {
            return;
        }
        let hold = match &self.mates.answering {
            Some((mate, key)) if mate == id => Some(key.clone()),
            _ => None,
        };
        let text = match &hold {
            Some(key) => format!("The captain's answer to your captain hold {key}: {words}"),
            None => words,
        };
        if self.perform_mate(Op::Send {
            id: id.to_string(),
            text,
            hold,
        }) {
            self.mates.draft(id).clear();
        }
    }

    /// Starts answering mate `id`'s captain hold `key` from its page.
    pub fn answer_mate_hold(&mut self, id: &str, key: &str) {
        self.mates.answering = Some((id.to_string(), key.to_string()));
        self.open_mate(id);
    }

    /// Runs `op` on a background thread; false when one is already running or firstmate is
    /// not where the scripts are.
    fn perform_mate(&mut self, op: Op) -> bool {
        let Some(clone) = self.chief.workplace().map(Path::to_path_buf) else {
            return false;
        };
        if self.mates.running.is_some() {
            return false;
        }
        let home = self.firstmate.home_dir();
        let (tx, rx) = mpsc::channel();
        let wake = self.wake.clone();
        let work = op.clone();
        thread::spawn(move || {
            let _ = tx.send(perform(&clone, &home, work));
            if let Some(wake) = wake {
                wake();
            }
        });
        self.mates.outcomes.remove(op.id());
        self.mates.running = Some((op, rx));
        true
    }

    /// Drains until no action is running, for headless scenes and tests.
    pub fn settle_mates(&mut self) {
        self.settle(|s| s.mates.running().is_none());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The registry line `bin/fm-home-seed.sh` wrote for a scratch home (firstmate 427056c4).
    const SEEDED: &str = "- lab-docs - Keep the documentation site accurate and current. (home: \
        /tmp/lab/mates/lab-docs; scope: documentation site work: pages, examples and the docs \
        build; projects: ; added 2026-11-13)";

    #[test]
    fn registry_lines_are_read_like_firstmates_parser() {
        let mate = entry(SEEDED).unwrap();
        assert_eq!(mate.id, "lab-docs");
        assert_eq!(
            mate.summary,
            "Keep the documentation site accurate and current."
        );
        assert_eq!(
            mate.scope,
            "documentation site work: pages, examples and the docs build"
        );
        assert!(mate.projects.is_empty());
        assert_eq!(mate.home, "/tmp/lab/mates/lab-docs");
        assert_eq!(mate.host, None);
        assert_eq!(mate.added, "2026-11-13");

        // Parentheses and semicolons in the summary and scope, and a project list.
        let busy = entry(
            "- billing - Owns billing (invoices; retries). (home: /h/billing; scope: billing \
             work; payments (cards); projects: billing, site; added 2026-11-12)",
        )
        .unwrap();
        assert_eq!(busy.summary, "Owns billing (invoices; retries).");
        assert_eq!(busy.scope, "billing work; payments (cards)");
        assert_eq!(busy.projects, ["billing", "site"]);

        let remote = entry(
            "- far - Far away. (host: box; root: /srv/fm; home: /srv/homes/far; scope: far \
             work; projects: api; added 2026-11-12)",
        )
        .unwrap();
        assert_eq!(remote.host.as_deref(), Some("box"));
        assert_eq!(remote.home, "/srv/homes/far");

        assert!(entry("# Second mates").is_none());
        assert!(entry("- bad id - x (home: /h; scope: s; projects: ; added 2026-11-12)").is_none());
    }

    #[test]
    fn parent_channel_lines_and_inbox_records_are_read() {
        // Lines firstmate's scripts appended on a scratch parent channel.
        let hold = status_line(
            "needs-decision [key=captain-hold-docs-domain-1] [at=1794617854]: captain hold \
             docs-domain: Publish the docs at docs.example.com or under example.com/docs?",
        )
        .unwrap();
        assert_eq!(hold.verb, "needs-decision");
        assert_eq!(hold.at, Some(1794617854));
        assert_eq!(hold.corr, None);
        let reply = status_line(
            "done [corr=ebbe292e407ab6f6] [at=1794617925]: The docs build passes on main. \
             (via-helper)",
        )
        .unwrap();
        assert_eq!(reply.corr.as_deref(), Some("ebbe292e407ab6f6"));
        assert_eq!(reply.note, "The docs build passes on main.");

        // The record bin/fm-send.sh wrote.
        let record = "schema=fm-task-inbox.v1\nat=2026-11-14T00:58:32Z\nbody-terminator=1\n--\n\
            [fm-from-firstmate]\u{2063}corr=ebbe292e407ab6f6 Please check the docs build.\n";
        let request = inbox_record(1, record, false).unwrap();
        assert_eq!(request.text, "Please check the docs build.");
        assert_eq!(request.corr.as_deref(), Some("ebbe292e407ab6f6"));
        assert_eq!(request.at, Some(1794617912));
    }

    #[test]
    fn a_conversation_pairs_requests_with_their_replies_in_time_order() {
        let channel = Channel {
            sent: vec![Request {
                seq: 1,
                at: Some(100),
                corr: Some("a".into()),
                text: "Check the build.".into(),
                read: true,
            }],
            back: vec![
                Line {
                    verb: "done".into(),
                    at: Some(50),
                    corr: None,
                    note: "PR ready".into(),
                },
                Line {
                    verb: "done".into(),
                    at: Some(200),
                    corr: Some("a".into()),
                    note: "It passes.".into(),
                },
            ],
        };
        let said = channel.conversation();
        assert_eq!(said.len(), 2);
        assert!(matches!(said[0], Said::Sent(r) if r.text == "Check the build."));
        assert!(matches!(said[1], Said::Back(l) if l.note == "It passes."));
        assert_eq!(channel.outcomes(5)[0].note, "PR ready");
    }

    #[test]
    fn a_routed_request_names_the_mate_that_took_it() {
        let mates = vec![entry(SEEDED).unwrap()];
        let to = |c: &str| routed_to(c, &mates).map(|m| m.id.as_str());
        assert_eq!(to("bin/fm-send.sh lab-docs 'Add search'"), Some("lab-docs"));
        assert_eq!(
            to("FM_HOME=/h bin/fm-backlog-handoff.sh lab-docs docs-search"),
            Some("lab-docs")
        );
        assert_eq!(to("bin/fm-send.sh lab-docs --key Enter"), Some("lab-docs"));
        assert_eq!(to("bin/fm-send.sh crew-1 hello"), None);
        assert_eq!(to("git status"), None);
    }

    #[test]
    fn a_refusal_is_firstmates_own_words_without_its_banners() {
        let stderr = "\u{25cf}\u{2501}\u{2501}\n\u{25cf}  WATCHER DOWN - SUPERVISION IS OFF\n\
            WARNING: watcher still down\nREFUSED: secondmate lab-docs still has in-flight work \
            in /h/state.\nFound docs-search.meta. Let that home finish or explicitly discard \
            with --force.\n";
        assert_eq!(
            reason(stderr, "").unwrap(),
            "REFUSED: secondmate lab-docs still has in-flight work in /h/state. Found \
             docs-search.meta. Let that home finish or explicitly discard with --force."
        );
        assert_eq!(reason("noise\nlast words\n", "").unwrap(), "last words");
    }

    #[test]
    fn names_follow_the_registry_rules() {
        let mates = vec![entry(SEEDED).unwrap()];
        assert!(AddMate::name_problem("docs", &mates).is_none());
        assert!(AddMate::name_problem("lab-docs", &mates).is_some());
        assert!(AddMate::name_problem("two words", &mates).is_some());
        assert!(AddMate::name_problem("", &mates).is_some());
        assert!(AddMate::name_problem("-x", &mates).is_some());
    }

    #[test]
    fn only_registered_projects_that_are_not_local_only_go_to_a_mate() {
        let project = |posture: Option<&str>| Project {
            name: "p".into(),
            posture: posture.map(str::to_string),
            about: String::new(),
        };
        assert!(eligible(&project(Some("no-mistakes-prod-only"))).is_ok());
        assert!(eligible(&project(Some(""))).is_ok());
        assert!(eligible(&project(Some("local-only +yolo"))).is_err());
        assert!(eligible(&project(None)).is_err());
    }
}
