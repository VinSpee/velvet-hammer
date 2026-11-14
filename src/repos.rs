//! The captain's GitHub repositories, for choosing one to add as a project. The app runs the
//! GitHub CLI the captain already signed in with (`gh api`), never reads or stores its token, and
//! never clones: a chosen repository's address goes into the captain's message, and firstmate's
//! own procedure clones and registers it, as with a chosen folder.
//!
//! The list puts the captain's own private repositories first, then the other private ones they
//! can reach through their organizations or as a collaborator, then public ones, and archived
//! repositories last, each group most recently pushed first. Typing filters it by fuzzy
//! subsequence match over `owner/name` and the description (see [`filter`]).

use std::ops::Range;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, channel};

use rune::{TextEdit, Waker};

/// The most rows the list draws at once; typing narrows the rest.
pub const SHOWN: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    Private,
    /// Visible to every member of the repository's enterprise, which is private to anyone else.
    Internal,
    Public,
}

impl Visibility {
    pub fn word(self) -> &'static str {
        match self {
            Visibility::Private => "Private",
            Visibility::Internal => "Internal",
            Visibility::Public => "Public",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repo {
    pub owner: String,
    pub name: String,
    pub visibility: Visibility,
    /// The repository's description, empty when it has none.
    pub about: String,
    /// When it was last pushed to, as GitHub's ISO 8601 UTC timestamp, which sorts as text;
    /// empty for a repository never pushed to.
    pub pushed: String,
    pub fork: bool,
    pub archived: bool,
}

impl Repo {
    /// `owner/name`, the text the filter matches and the list shows.
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    /// The address the captain sends to the chief of staff.
    pub fn url(&self) -> String {
        format!("https://github.com/{}/{}", self.owner, self.name)
    }

    /// The date of the last push, such as `2026-11-13`.
    pub fn pushed_on(&self) -> &str {
        self.pushed.get(..10).unwrap_or(&self.pushed)
    }
}

/// Why the repositories could not be listed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// No `gh` on `PATH`.
    Missing,
    /// `gh` runs but is not signed in to GitHub.
    SignedOut,
    /// `gh` failed some other way, in its own words.
    Failed(String),
}

impl Problem {
    /// What went wrong and how the captain fixes it.
    pub fn words(&self) -> String {
        match self {
            Problem::Missing => "The GitHub CLI (gh) is not installed, or it is not on this app's \
                PATH. Install it from https://cli.github.com (on a Mac, brew install gh), run gh \
                auth login in a terminal, then choose Try again. You can still choose a folder or \
                type a git URL."
                .into(),
            Problem::SignedOut => "The GitHub CLI (gh) is not signed in to GitHub. Run gh auth \
                login in a terminal, then choose Try again. The app uses gh's own sign-in and \
                never stores a token."
                .into(),
            Problem::Failed(why) => {
                format!("The GitHub CLI (gh) could not list your repositories: {why}")
            }
        }
    }
}

/// The fields `gh api` keeps of each repository, one JSON object per line.
const FIELDS: &str =
    ".[] | {owner: .owner.login, name, visibility, description, pushed_at, fork, archived}";

/// Every repository the account signed in to `gh` can reach as owner, collaborator or
/// organization member, in the order the list shows them.
pub fn list(gh: &Path) -> Result<Vec<Repo>, Problem> {
    let login = run(gh, &["api", "user", "--jq", ".login"])?;
    let lines = run(
        gh,
        &[
            "api",
            "--paginate",
            "user/repos?per_page=100",
            "--jq",
            FIELDS,
        ],
    )?;
    let repos = parse(&lines).map_err(Problem::Failed)?;
    Ok(order(login.trim(), repos))
}

/// Runs `gh` with `args` and returns what it printed.
fn run(gh: &Path, args: &[&str]) -> Result<String, Problem> {
    let output = Command::new(gh)
        .args(args)
        .env("GH_PROMPT_DISABLED", "1")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Problem::Missing,
            _ => Problem::Failed(format!("{}: {e}", gh.display())),
        })?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let said = String::from_utf8_lossy(&output.stderr).trim().to_string();
    // gh exits 4 when a command needs a sign-in it does not have.
    if output.status.code() == Some(4) || said.contains("gh auth login") {
        return Err(Problem::SignedOut);
    }
    Err(Problem::Failed(if said.is_empty() {
        format!("it exited with {}", output.status)
    } else {
        said
    }))
}

/// The repositories in `gh api`'s output after [`FIELDS`], one JSON object per line.
fn parse(lines: &str) -> Result<Vec<Repo>, String> {
    lines
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let v: serde_json::Value = serde_json::from_str(line)
                .map_err(|e| format!("unexpected output from gh ({e}): {line}"))?;
            let text = |key: &str| v[key].as_str().unwrap_or_default().to_string();
            let (owner, name) = (text("owner"), text("name"));
            if owner.is_empty() || name.is_empty() {
                return Err(format!("a repository without an owner or name: {line}"));
            }
            Ok(Repo {
                owner,
                name,
                visibility: match v["visibility"].as_str() {
                    Some("public") => Visibility::Public,
                    Some("internal") => Visibility::Internal,
                    _ => Visibility::Private,
                },
                about: text("description"),
                pushed: text("pushed_at"),
                fork: v["fork"].as_bool().unwrap_or(false),
                archived: v["archived"].as_bool().unwrap_or(false),
            })
        })
        .collect()
}

/// `repos` in the order the list shows them to `login`: their own private repositories, other
/// private ones, public ones, then archived ones, each most recently pushed first.
fn order(login: &str, mut repos: Vec<Repo>) -> Vec<Repo> {
    let group = |r: &Repo| match () {
        _ if r.archived => 3,
        _ if r.visibility == Visibility::Public => 2,
        _ if r.owner.eq_ignore_ascii_case(login) => 0,
        _ => 1,
    };
    repos.sort_by(|a, b| {
        (group(a), &b.pushed, a.full_name().to_lowercase()).cmp(&(
            group(b),
            &a.pushed,
            b.full_name().to_lowercase(),
        ))
    });
    repos
}

/// One repository the query matches, and where, to highlight.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    /// Its position in the ordered list.
    pub index: usize,
    /// The byte ranges of `owner/name` that matched.
    pub in_name: Vec<Range<usize>>,
    /// The byte ranges of the description that matched, when `owner/name` did not.
    pub in_about: Vec<Range<usize>>,
}

/// The repositories `query` matches, best first. Each character of the query, ignoring case and
/// spaces, must appear in order in `owner/name` or in the description. A match in `owner/name`
/// ranks above one only in the description; among those, runs of consecutive characters and
/// characters at the start of a word (after `/`, `-`, `_`, `.` or a space, or a capital) score
/// higher, and a gap between matched characters scores lower. Ties keep the list's order, so the
/// captain's own private repositories still come first. An empty query matches everything in
/// the list's order.
pub fn filter(repos: &[Repo], query: &str) -> Vec<Match> {
    let needle: Vec<char> = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(lower)
        .collect();
    let mut found: Vec<(bool, i32, Match)> = repos
        .iter()
        .enumerate()
        .filter_map(|(index, repo)| {
            if let Some((score, in_name)) = fuzzy(&needle, &repo.full_name()) {
                let m = Match {
                    index,
                    in_name,
                    in_about: vec![],
                };
                return Some((true, score, m));
            }
            let (score, in_about) = fuzzy(&needle, &repo.about)?;
            let m = Match {
                index,
                in_name: vec![],
                in_about,
            };
            Some((false, score, m))
        })
        .collect();
    found.sort_by(|a, b| (b.0, b.1, a.2.index).cmp(&(a.0, a.1, b.2.index)));
    found.into_iter().map(|(.., m)| m).collect()
}

const HIT: i32 = 16;
const RUN: i32 = 16;
const WORD_START: i32 = 12;
/// The most a late first match costs, so a long owner name does not bury a good match.
const MOST_LEAD: i32 = 8;

fn lower(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// The best-scoring way `needle` (lowercase) appears in order in `hay`, as its score and the
/// byte ranges it matched; `None` when it does not appear. Dynamic programming over (query
/// character, position), so the best alignment wins, not the first.
fn fuzzy(needle: &[char], hay: &str) -> Option<(i32, Vec<Range<usize>>)> {
    if needle.is_empty() {
        return Some((0, vec![]));
    }
    let chars: Vec<(usize, char)> = hay.char_indices().collect();
    let (m, n) = (needle.len(), chars.len());
    if m > n {
        return None;
    }
    let word_start = |j: usize| {
        j == 0 || {
            let (before, here) = (chars[j - 1].1, chars[j].1);
            !before.is_alphanumeric() || (before.is_lowercase() && here.is_uppercase())
        }
    };
    const NONE: i32 = i32::MIN / 2;
    // score[i][j]: the best score with needle[i] matched at chars[j]; from[i][j]: where
    // needle[i - 1] was matched for it.
    let mut score = vec![vec![NONE; n]; m];
    let mut from = vec![vec![0; n]; m];
    for i in 0..m {
        // The best score[i - 1][k] + k over k < j - 1, for a match after a gap of j - 1 - k.
        let (mut gapped, mut gapped_at) = (NONE, 0);
        for j in 0..n {
            if i > 0 && j >= 2 && score[i - 1][j - 2] > NONE {
                let k = j - 2;
                if score[i - 1][k] + k as i32 > gapped {
                    (gapped, gapped_at) = (score[i - 1][k] + k as i32, k);
                }
            }
            if lower(chars[j].1) != needle[i] {
                continue;
            }
            let hit = HIT + if word_start(j) { WORD_START } else { 0 };
            if i == 0 {
                score[0][j] = hit - (j as i32).min(MOST_LEAD);
                continue;
            }
            if j >= 1 && score[i - 1][j - 1] > NONE {
                score[i][j] = score[i - 1][j - 1] + hit + RUN;
                from[i][j] = j - 1;
            }
            if gapped > NONE && gapped - (j as i32 - 1) + hit > score[i][j] {
                score[i][j] = gapped - (j as i32 - 1) + hit;
                from[i][j] = gapped_at;
            }
        }
    }
    let (end, best) = (0..n)
        .map(|j| (j, score[m - 1][j]))
        .filter(|(_, s)| *s > NONE)
        .max_by_key(|(j, s)| (*s, std::cmp::Reverse(*j)))?;
    let mut at = vec![end];
    for i in (1..m).rev() {
        at.push(from[i][*at.last().unwrap()]);
    }
    at.reverse();
    // Consecutive characters make one range.
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for j in at {
        let (start, c) = chars[j];
        let end = start + c.len_utf8();
        match ranges.last_mut() {
            Some(last) if last.end == start => last.end = end,
            _ => ranges.push(start..end),
        }
    }
    Some((best, ranges))
}

/// The repository combobox's state: its query, whether its list is open, the highlighted row,
/// and the repositories `gh` listed this session, which each opening refreshes.
pub struct Chooser {
    pub query: TextEdit,
    pub open: bool,
    /// The highlighted repository, by `owner/name`; none highlights the first match.
    active: Option<String>,
    /// The last listing, or why it failed. `None` until the first one finishes.
    listed: Option<Result<Vec<Repo>, Problem>>,
    loading: Option<Receiver<Result<Vec<Repo>, Problem>>>,
    /// Lists the repositories: [`list`] on `gh`, or a stand-in in tests and scenes.
    pub lister: fn() -> Result<Vec<Repo>, Problem>,
}

impl Default for Chooser {
    fn default() -> Self {
        Chooser {
            query: TextEdit::default(),
            open: false,
            active: None,
            listed: None,
            loading: None,
            lister: || list(Path::new("gh")),
        }
    }
}

impl Chooser {
    /// Opens the list and lists the repositories again on a background thread, showing the
    /// session's last listing meanwhile. `wake` wakes the window when it finishes.
    pub fn show(&mut self, wake: Option<Waker>) {
        if self.open {
            return;
        }
        self.open = true;
        self.active = None;
        self.refresh(wake);
    }

    /// Lists the repositories again, unless a listing is already running.
    pub fn refresh(&mut self, wake: Option<Waker>) {
        if self.loading.is_some() {
            return;
        }
        let (tx, rx) = channel();
        let lister = self.lister;
        std::thread::spawn(move || {
            let _ = tx.send(lister());
            if let Some(wake) = wake {
                wake();
            }
        });
        self.loading = Some(rx);
    }

    pub fn hide(&mut self) {
        self.open = false;
        self.active = None;
    }

    /// Takes a finished listing; true when one arrived.
    pub fn poll(&mut self) -> bool {
        let Some(listed) = self.loading.as_ref().and_then(|rx| rx.try_recv().ok()) else {
            return false;
        };
        self.loading = None;
        // A failed refresh does not hide a list the captain is already looking at.
        match (&self.listed, listed) {
            (Some(Ok(_)), Err(_)) => {}
            (_, listed) => self.listed = Some(listed),
        }
        true
    }

    pub fn loading(&self) -> bool {
        self.loading.is_some()
    }

    /// The last listing, or why it failed.
    pub fn listed(&self) -> Option<&Result<Vec<Repo>, Problem>> {
        self.listed.as_ref()
    }

    /// The repositories the query matches, best first, at most [`SHOWN`] of them, and how many
    /// matched in all.
    pub fn matches(&self) -> (Vec<Match>, usize) {
        let Some(Ok(repos)) = &self.listed else {
            return (vec![], 0);
        };
        let mut found = filter(repos, &self.query.text);
        let all = found.len();
        found.truncate(SHOWN);
        (found, all)
    }

    pub fn repo(&self, index: usize) -> Option<&Repo> {
        match &self.listed {
            Some(Ok(repos)) => repos.get(index),
            _ => None,
        }
    }

    /// The position of the repository named `owner/name` in the ordered list.
    pub fn position(&self, full_name: &str) -> Option<usize> {
        match &self.listed {
            Some(Ok(repos)) => repos.iter().position(|r| r.full_name() == full_name),
            _ => None,
        }
    }

    /// The highlighted match: the one the keys or the pointer chose, else the first.
    pub fn active(&self, matches: &[Match]) -> Option<usize> {
        let chosen = self.active.as_ref().and_then(|name| {
            matches
                .iter()
                .find(|m| self.repo(m.index).is_some_and(|r| &r.full_name() == name))
        });
        chosen.or(matches.first()).map(|m| m.index)
    }

    /// Highlights the repository at `index` in the ordered list.
    pub fn highlight(&mut self, index: usize) {
        self.active = self.repo(index).map(Repo::full_name);
    }

    /// Moves the highlight `by` rows, wrapping at the ends. Returns the newly highlighted
    /// repository's position in the ordered list.
    pub fn step(&mut self, by: isize) -> Option<usize> {
        let (matches, _) = self.matches();
        if matches.is_empty() {
            return None;
        }
        let at = self
            .active(&matches)
            .and_then(|index| matches.iter().position(|m| m.index == index))
            .unwrap_or(0);
        let next = matches[(at as isize + by).rem_euclid(matches.len() as isize) as usize].index;
        self.highlight(next);
        Some(next)
    }

    /// The query changed: open the list and highlight its best match again.
    pub fn edited(&mut self, wake: Option<Waker>) {
        self.show(wake);
        self.active = None;
    }
}

/// The element key of a repository's row; `/` separates keys in a path, so it is replaced.
pub fn row_key(repo: &Repo) -> String {
    format!("repo-option-{}:{}", repo.owner, repo.name)
}

/// Repositories in `gh api`'s output after [`FIELDS`], for tests and scenes: the captain's own
/// private ones, an organization's private one, public ones, a fork and an archived one.
pub const SAMPLE: &str = r#"{"owner":"captain","name":"notes","visibility":"private","description":"Meeting notes and plans","pushed_at":"2026-11-10T09:00:00Z","fork":false,"archived":false}
{"owner":"acme","name":"velvet-hammer","visibility":"public","description":"A native desktop app for AI coding agents","pushed_at":"2026-11-13T18:00:00Z","fork":false,"archived":false}
{"owner":"captain","name":"dotfiles","visibility":"private","description":"Shell and editor settings","pushed_at":"2026-11-12T08:30:00Z","fork":false,"archived":false}
{"owner":"acme","name":"payments-api","visibility":"private","description":"Billing and invoices service","pushed_at":"2026-11-13T12:00:00Z","fork":false,"archived":false}
{"owner":"captain","name":"site","visibility":"public","description":"Personal homepage","pushed_at":"2026-11-01T10:00:00Z","fork":false,"archived":false}
{"owner":"captain","name":"rune","visibility":"public","description":"A fork of the Rune UI toolkit","pushed_at":"2026-10-20T10:00:00Z","fork":true,"archived":false}
{"owner":"acme","name":"handbook","visibility":"internal","description":"How the team works","pushed_at":"2026-10-02T10:00:00Z","fork":false,"archived":false}
{"owner":"captain","name":"old-hammer","visibility":"private","description":"The first prototype","pushed_at":"2025-03-01T10:00:00Z","fork":false,"archived":true}
"#;

/// [`SAMPLE`] in the list's order for the account `captain`.
pub fn sample() -> Result<Vec<Repo>, Problem> {
    Ok(order(
        "captain",
        parse(SAMPLE).expect("sample repositories"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(repos: &[Repo]) -> Vec<String> {
        repos.iter().map(Repo::full_name).collect()
    }

    fn found(query: &str) -> Vec<String> {
        let repos = sample().unwrap();
        filter(&repos, query)
            .iter()
            .map(|m| repos[m.index].full_name())
            .collect()
    }

    /// The text of each range in `hay`, to read a highlight.
    fn marked(hay: &str, ranges: &[Range<usize>]) -> Vec<String> {
        ranges.iter().map(|r| hay[r.clone()].to_string()).collect()
    }

    #[test]
    fn own_private_repositories_come_first_then_other_private_then_public_then_archived() {
        assert_eq!(
            names(&sample().unwrap()),
            [
                // The captain's own private ones, most recently pushed first.
                "captain/dotfiles",
                "captain/notes",
                // Private to an organization, internal counting as private.
                "acme/payments-api",
                "acme/handbook",
                // Public, forks included.
                "acme/velvet-hammer",
                "captain/site",
                "captain/rune",
                // Archived, whatever its visibility.
                "captain/old-hammer",
            ]
        );
    }

    #[test]
    fn the_signed_in_login_matches_whatever_its_case() {
        let repos = order("Captain", parse(SAMPLE).unwrap());
        assert_eq!(repos[0].full_name(), "captain/dotfiles");
    }

    #[test]
    fn an_empty_query_keeps_the_order() {
        assert_eq!(found(""), names(&sample().unwrap()));
        assert_eq!(found("   "), names(&sample().unwrap()));
    }

    #[test]
    fn a_query_matches_a_subsequence_ignoring_case_and_spaces() {
        assert_eq!(found("VH"), ["acme/velvet-hammer"]);
        assert_eq!(found("pay api"), ["acme/payments-api"]);
        assert!(found("zzz").is_empty());
        // The characters must appear in that order.
        assert!(found("hv").is_empty());
    }

    #[test]
    fn consecutive_and_word_start_matches_rank_higher() {
        // `hammer` runs whole at a word start in both, so they tie and keep the list's order.
        let ranked = found("hammer");
        assert_eq!(ranked, ["acme/velvet-hammer", "captain/old-hammer"]);
        // A run at a word start beats letters scattered through a name.
        let ranked = found("note");
        assert_eq!(ranked[0], "captain/notes");
        // `site` runs whole in captain/site, and is scattered elsewhere if at all.
        assert_eq!(found("site")[0], "captain/site");
    }

    #[test]
    fn a_name_match_ranks_above_a_description_match() {
        let ranked = found("billing");
        assert_eq!(ranked, ["acme/payments-api"], "matched by its description");
        let ranked = found("hand");
        assert_eq!(ranked[0], "acme/handbook");
    }

    #[test]
    fn ties_keep_the_list_order_so_own_private_repositories_lead() {
        // Every repository's owner/name contains `a`; one letter scores alike at a word start.
        let ranked = found("c");
        assert_eq!(ranked[0], "captain/dotfiles");
        assert_eq!(ranked[1], "captain/notes");
    }

    #[test]
    fn the_best_alignment_is_highlighted_not_the_first() {
        // Taking each character at its first chance would mark the `a` of `acme`, the `p` of
        // `payments` and the `i` of `api`.
        let (_, ranges) = fuzzy(&['a', 'p', 'i'], "acme/payments-api").unwrap();
        assert_eq!(marked("acme/payments-api", &ranges), ["api"]);
        let (_, ranges) = fuzzy(&['v', 'h'], "acme/velvet-hammer").unwrap();
        assert_eq!(marked("acme/velvet-hammer", &ranges), ["v", "h"]);
        assert_eq!(ranges, [5..6, 12..13]);
    }

    #[test]
    fn a_description_match_is_highlighted_in_the_description() {
        let repos = sample().unwrap();
        let m = &filter(&repos, "invoices")[0];
        let repo = &repos[m.index];
        assert_eq!(repo.full_name(), "acme/payments-api");
        assert!(m.in_name.is_empty());
        assert_eq!(marked(&repo.about, &m.in_about), ["invoices"]);
    }

    #[test]
    fn highlights_are_byte_ranges_in_text_that_is_not_ascii() {
        let (_, ranges) = fuzzy(&['é', 't'], "café-tea").unwrap();
        assert_eq!(marked("café-tea", &ranges), ["é", "t"]);
    }

    #[test]
    fn gh_output_is_parsed_with_its_nulls() {
        let repos = parse(
            r#"{"owner":"a","name":"b","visibility":"public","description":null,"pushed_at":null,"fork":false,"archived":false}"#,
        )
        .unwrap();
        assert_eq!(repos[0].about, "");
        assert_eq!(repos[0].pushed, "");
        assert_eq!(repos[0].visibility, Visibility::Public);
        assert!(parse("not json").is_err());
        assert!(parse(r#"{"owner":"a"}"#).is_err());
    }

    /// A stand-in `gh` that runs `body` as a shell script, in a scratch directory.
    fn fake_gh(name: &str, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/repos-lab")
            .join(format!("{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let gh = dir.join("gh");
        std::fs::write(&gh, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
        gh
    }

    #[test]
    fn a_missing_gh_says_how_to_install_it() {
        let problem = list(Path::new("/nonexistent/velvet-hammer/gh")).unwrap_err();
        assert_eq!(problem, Problem::Missing);
        assert!(problem.words().contains("https://cli.github.com"));
        assert!(problem.words().contains("gh auth login"));
    }

    #[test]
    fn a_gh_that_is_not_signed_in_says_how_to_sign_in() {
        let gh = fake_gh(
            "signed-out",
            "echo 'To get started with GitHub CLI, please run:  gh auth login' >&2; exit 4",
        );
        let problem = list(&gh).unwrap_err();
        assert_eq!(problem, Problem::SignedOut);
        assert!(problem.words().contains("gh auth login"));
    }

    #[test]
    fn a_gh_that_fails_otherwise_shows_its_own_words() {
        let gh = fake_gh("failing", "echo 'HTTP 502: Bad Gateway' >&2; exit 1");
        assert_eq!(
            list(&gh).unwrap_err(),
            Problem::Failed("HTTP 502: Bad Gateway".into())
        );
    }

    #[test]
    fn gh_lists_the_signed_in_accounts_repositories_in_order() {
        let lines = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/repos-lab/sample.jsonl");
        std::fs::create_dir_all(lines.parent().unwrap()).unwrap();
        std::fs::write(&lines, SAMPLE).unwrap();
        // It asks for the login, then every page of the repositories with the fields it reads.
        let gh = fake_gh(
            "listing",
            &format!(
                "case \"$*\" in\n\
                 'api user --jq .login') echo captain ;;\n\
                 'api --paginate user/repos?per_page=100 --jq {FIELDS}') cat '{}' ;;\n\
                 *) echo \"unexpected: $*\" >&2; exit 1 ;;\n\
                 esac",
                lines.display()
            ),
        );
        assert_eq!(list(&gh), sample());
    }

    #[test]
    #[ignore = "runs the real gh, which must be installed and signed in"]
    fn the_real_gh_lists_repositories() {
        let repos = list(Path::new("gh")).expect("gh listed");
        eprintln!("{} repositories; first: {:?}", repos.len(), repos.first());
        assert!(!repos.is_empty());
    }

    #[test]
    fn the_highlight_moves_and_wraps_through_the_matches() {
        let mut chooser = Chooser {
            lister: sample,
            ..Chooser::default()
        };
        chooser.show(None);
        while !chooser.poll() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let (matches, all) = chooser.matches();
        assert_eq!(all, 8);
        assert_eq!(chooser.active(&matches), Some(0));
        assert_eq!(chooser.step(1), Some(1));
        assert_eq!(
            chooser.step(-2),
            Some(7),
            "Up from the first wraps to the last"
        );
        // Typing lets go of the highlight: the best match is highlighted again.
        chooser.query.text = "pay".into();
        chooser.edited(None);
        let (matches, _) = chooser.matches();
        assert_eq!(
            chooser
                .repo(chooser.active(&matches).unwrap())
                .unwrap()
                .full_name(),
            "acme/payments-api"
        );
    }

    #[test]
    fn a_failed_refresh_keeps_the_list_already_shown() {
        let mut chooser = Chooser {
            lister: sample,
            ..Chooser::default()
        };
        chooser.refresh(None);
        while !chooser.poll() {}
        chooser.lister = || Err(Problem::SignedOut);
        chooser.refresh(None);
        while !chooser.poll() {}
        assert!(matches!(chooser.listed(), Some(Ok(_))));
    }
}
