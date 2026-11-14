//! Worker nicknames: the captain's presets for the crewmates the chief of staff sends, such as
//! "hacker" for a pi worker on DeepSeek V4.1 Flash through opencode Go. The registry is
//! `config/crew-nicknames.json` in the app's firstmate home, and the chief of staff learns it
//! from the `UserPromptSubmit` hook, together with what the app made of a "send a … worker"
//! request in the captain's message ([`context`]).
//!
//! A preset names one of two harnesses. [`Harness::Pi`] is the pi runtime Velvet Hammer installs
//! ([`crate::pi`]), for models on other providers. [`Harness::Claude`] is Claude Code on the
//! captain's own subscription, exactly as firstmate launches it. A Claude model never runs
//! through pi: [`Preset::check`] refuses one, the resolver never offers one, and the firstmate
//! rule `claude-on-subscription` ([`crate::guard`]) denies a spawn that tries.

use rune::TextEdit;
use serde_json::{Value, json};

use crate::store::{self, Saved, Store};

/// The registry's file, in the firstmate home's `config/` directory.
pub const FILE: &str = "crew-nicknames.json";

/// The effort levels firstmate passes to both harnesses.
pub const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// The seed preset a fresh registry holds, which the captain may keep or delete. The model is
/// listed by pi 1.5.2 as `opencode-go  deepseek-v4.1-flash`.
pub const SEED_MODEL: &str = "opencode-go/deepseek-v4.1-flash";

/// What runs a preset's worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Harness {
    /// The pi runtime Velvet Hammer installs, for models on other providers.
    Pi,
    /// Claude Code on the captain's subscription.
    Claude,
}

impl Harness {
    pub const ALL: [Harness; 2] = [Harness::Pi, Harness::Claude];

    /// Firstmate's name for it, as `bin/fm-spawn.sh --harness` takes it.
    pub fn key(self) -> &'static str {
        match self {
            Harness::Pi => "pi",
            Harness::Claude => "claude",
        }
    }

    fn from_key(key: &str) -> Option<Harness> {
        Harness::ALL.into_iter().find(|h| h.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            Harness::Pi => "pi, another provider",
            Harness::Claude => "Claude Code, your subscription",
        }
    }
}

/// One nickname and the worker it sends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preset {
    pub nickname: String,
    pub harness: Harness,
    /// For pi, `<provider>/<id>` as `pi --list-models` lists it; for Claude, a Claude Code model
    /// such as `sonnet` or `claude-sonnet-5-5`.
    pub model: String,
    pub effort: Option<String>,
}

impl Preset {
    /// Why the preset cannot be used, in words for the settings page.
    pub fn check(&self) -> Result<(), String> {
        let nick = &self.nickname;
        if nick.is_empty()
            || nick.len() > 32
            || !nick
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            || nick.starts_with('-')
        {
            return Err(format!(
                "\u{201c}{nick}\u{201d} is not a nickname Velvet Hammer can use: use 1 to 32 \
                 lowercase letters, digits and hyphens, such as hacker or night-owl."
            ));
        }
        match self.harness {
            Harness::Pi => {
                let (provider, id) = self.model.split_once('/').unwrap_or_default();
                if provider.is_empty() || id.is_empty() || self.model.contains(char::is_whitespace)
                {
                    return Err(format!(
                        "Name the model as pi lists it, provider/model, such as {SEED_MODEL}."
                    ));
                }
                if is_claude(&self.model) {
                    return Err(
                        "Claude runs only through Claude Code on your subscription, never \
                         through pi. Choose Claude Code for a Claude model."
                            .into(),
                    );
                }
            }
            Harness::Claude => {
                if self.model.is_empty()
                    || self.model.contains('/')
                    || self.model.contains(char::is_whitespace)
                {
                    return Err(
                        "Name a Claude Code model, such as sonnet, opus or claude-sonnet-5-5."
                            .into(),
                    );
                }
            }
        }
        if let Some(effort) = &self.effort
            && !EFFORTS.contains(&effort.as_str())
        {
            return Err(format!(
                "Effort is one of {}, or empty for the model's default.",
                EFFORTS.join(", ")
            ));
        }
        Ok(())
    }

    /// The `bin/fm-spawn.sh` flags that send this worker.
    pub fn flags(&self) -> String {
        let mut flags = format!("--harness {} --model {}", self.harness.key(), self.model);
        if let Some(effort) = &self.effort {
            flags.push_str(&format!(" --effort {effort}"));
        }
        flags
    }

    /// "pi · opencode-go/deepseek-v4.1-flash · effort high".
    pub fn describe(&self) -> String {
        let mut words = format!("{} \u{b7} {}", self.harness.key(), self.model);
        if let Some(effort) = &self.effort {
            words.push_str(&format!(" \u{b7} effort {effort}"));
        }
        words
    }

    fn to_json(&self) -> Value {
        let mut row = json!({
            "nickname": self.nickname,
            "harness": self.harness.key(),
            "model": self.model,
        });
        if let Some(effort) = &self.effort {
            row["effort"] = json!(effort);
        }
        row
    }

    fn from_json(row: &Value) -> Result<Preset, String> {
        let word = |field: &str| {
            row[field]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("a nickname has no {field}"))
        };
        let harness = word("harness")?;
        let preset = Preset {
            nickname: word("nickname")?,
            harness: Harness::from_key(&harness)
                .ok_or_else(|| format!("a nickname names the harness {harness}"))?,
            model: word("model")?,
            effort: match &row["effort"] {
                Value::Null => None,
                Value::String(e) => Some(e.clone()),
                _ => return Err("a nickname's effort is not text".into()),
            },
        };
        preset.check()?;
        Ok(preset)
    }
}

/// Whether `model` names a Claude model, which runs only through Claude Code.
pub fn is_claude(model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    model.starts_with("anthropic/") || model.contains("claude")
}

/// The registry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Nicknames {
    pub presets: Vec<Preset>,
}

impl Default for Nicknames {
    /// A fresh registry holds one example.
    fn default() -> Self {
        Nicknames {
            presets: vec![Preset {
                nickname: "hacker".into(),
                harness: Harness::Pi,
                model: SEED_MODEL.into(),
                effort: None,
            }],
        }
    }
}

impl Nicknames {
    pub fn find(&self, nickname: &str) -> Option<&Preset> {
        self.presets.iter().find(|p| p.nickname == nickname)
    }
}

impl Saved for Nicknames {
    const FILE: &'static str = FILE;
    const FALLBACK: &'static str = "no nickname is offered to the chief of staff";

    fn fallback() -> Self {
        Nicknames {
            presets: Vec::new(),
        }
    }

    fn to_json(&self) -> Value {
        let rows: Vec<Value> = self.presets.iter().map(Preset::to_json).collect();
        json!({"version": store::VERSION, "nicknames": rows})
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        store::check_version(value)?;
        let mut presets: Vec<Preset> = Vec::new();
        for row in value["nicknames"]
            .as_array()
            .ok_or("it has no list of nicknames")?
        {
            let preset = Preset::from_json(row)?;
            if presets.iter().any(|p| p.nickname == preset.nickname) {
                return Err(format!("it names {} twice", preset.nickname));
            }
            presets.push(preset);
        }
        Ok(Nicknames { presets })
    }
}

/// The Workers page's nickname editor.
pub struct Form {
    pub nickname: TextEdit,
    pub harness: Harness,
    pub model: TextEdit,
    pub effort: TextEdit,
    /// The nickname being changed, by index; None while adding one.
    pub editing: Option<usize>,
    pub problem: Option<String>,
}

impl Default for Form {
    fn default() -> Self {
        Form {
            nickname: TextEdit::default(),
            harness: Harness::Pi,
            model: TextEdit::default(),
            effort: TextEdit::default(),
            editing: None,
            problem: None,
        }
    }
}

impl Form {
    /// Adds the nickname in the form, or saves the one being changed, if it can be used.
    pub fn save(&mut self, store: &mut Store<Nicknames>) {
        let effort = self.effort.text.trim();
        let preset = Preset {
            nickname: self.nickname.text.trim().to_string(),
            harness: self.harness,
            model: self.model.text.trim().to_string(),
            effort: (!effort.is_empty()).then(|| effort.to_string()),
        };
        if let Err(problem) = preset.check() {
            self.problem = Some(problem);
            return;
        }
        let editing = self.editing;
        let taken = store
            .settings()
            .presets
            .iter()
            .enumerate()
            .any(|(i, p)| p.nickname == preset.nickname && Some(i) != editing);
        if taken {
            self.problem = Some(format!(
                "There is already a nickname {}. Edit that one, or choose another name.",
                preset.nickname
            ));
            return;
        }
        store.change(|n| match editing {
            Some(i) if i < n.presets.len() => n.presets[i] = preset,
            _ => n.presets.push(preset),
        });
        if store.problem().is_none() {
            self.clear();
        }
    }

    /// Loads nickname `i` into the form for changing.
    pub fn edit(&mut self, store: &Store<Nicknames>, i: usize) {
        if let Some(preset) = store.settings().presets.get(i) {
            self.nickname = TextEdit::new(preset.nickname.clone());
            self.harness = preset.harness;
            self.model = TextEdit::new(preset.model.clone());
            self.effort = TextEdit::new(preset.effort.clone().unwrap_or_default());
            self.editing = Some(i);
            self.problem = None;
        }
    }

    pub fn delete(&mut self, store: &mut Store<Nicknames>, i: usize) {
        if i < store.settings().presets.len() {
            store.change(|n| {
                n.presets.remove(i);
            });
            self.clear();
        }
    }

    pub fn clear(&mut self) {
        *self = Form::default();
    }
}

/// What a "send a … worker" phrase names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolved {
    /// A nickname in the registry.
    Preset(Preset),
    /// Exactly one model pi lists, as `<provider>/<id>`.
    Model(String),
    /// A Claude model, which goes through Claude Code as usual.
    Claude,
    /// Several models pi lists, the first few of them.
    Ambiguous(Vec<String>),
    /// Nothing the app knows.
    Unknown,
}

/// A worker request in the captain's message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// The words naming the worker: "send an opencode go deepseek 4.1 flash worker to
    /// investigate" gives "opencode go deepseek 4.1 flash", and "send a hacker to look" gives
    /// "hacker".
    pub phrase: String,
    /// Whether the words ended at "worker", "crewmate" or "agent". Without one, as in "send the
    /// report to Alice", the message may not ask for a worker at all.
    pub worker: bool,
}

/// The worker request in the captain's message, if it reads as one: "send" or "dispatch", then
/// the words up to "worker", "crewmate", "agent", "to" or "and".
pub fn request(message: &str) -> Option<Request> {
    let words: Vec<String> = message
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .collect();
    let send = words.iter().position(|w| w == "send" || w == "dispatch")?;
    let rest = &words[send + 1..];
    let is_worker = |w: &str| {
        matches!(
            w,
            "worker" | "workers" | "crewmate" | "crewmates" | "agent" | "agents"
        )
    };
    let end = rest
        .iter()
        .position(|w| is_worker(w) || w == "to" || w == "and")?;
    let phrase: Vec<&str> = rest[..end]
        .iter()
        .map(String::as_str)
        .filter(|w| !matches!(*w, "a" | "an" | "the" | "one" | "out" | "off" | ""))
        .collect();
    (!phrase.is_empty()).then(|| Request {
        phrase: phrase.join(" "),
        worker: is_worker(&rest[end]),
    })
}

/// The words a phrase or a model id is matched on: lowercase, split at spaces, slashes, hyphens
/// and underscores, with a `v` before a version number dropped, so "v4.1" matches "4.1".
fn tokens(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| c.is_whitespace() || matches!(c, '/' | '-' | '_'))
        .filter(|w| !w.is_empty())
        .map(|w| match w.strip_prefix('v') {
            Some(rest) if rest.starts_with(|c: char| c.is_ascii_digit()) => rest.to_string(),
            _ => w.to_string(),
        })
        .filter(|w| !matches!(w.as_str(), "model" | "via" | "on" | "using" | "with"))
        .collect()
}

/// How many candidates an ambiguous phrase lists.
const SHOWN: usize = 6;

/// Resolves a request phrase: a nickname first, then a Claude model, then the models pi lists
/// (`<provider>/<id>`). Every word of the phrase must appear in a model's id; when several
/// models match, the one whose id has no other words wins.
pub fn resolve(phrase: &str, nicknames: &Nicknames, models: &[String]) -> Resolved {
    let phrase = phrase.trim().to_lowercase();
    let hyphenated = phrase.split_whitespace().collect::<Vec<_>>().join("-");
    if let Some(preset) = nicknames.find(&hyphenated) {
        return Resolved::Preset(preset.clone());
    }
    let wanted = tokens(&phrase);
    if wanted.is_empty() {
        return Resolved::Unknown;
    }
    if wanted.iter().any(|w| {
        matches!(
            w.as_str(),
            "claude" | "anthropic" | "sonnet" | "opus" | "haiku" | "fable"
        )
    }) {
        return Resolved::Claude;
    }
    let matching: Vec<&String> = models
        .iter()
        .filter(|m| !is_claude(m))
        .filter(|m| {
            let have = tokens(m);
            wanted.iter().all(|w| have.contains(w))
        })
        .collect();
    let exact: Vec<&String> = matching
        .iter()
        .copied()
        .filter(|m| tokens(m).len() == wanted.len())
        .collect();
    match (matching.as_slice(), exact.as_slice()) {
        ([], _) => Resolved::Unknown,
        ([one], _) | (_, [one]) => Resolved::Model(one.to_string()),
        (many, _) => Resolved::Ambiguous(many.iter().take(SHOWN).map(|m| m.to_string()).collect()),
    }
}

/// What the `UserPromptSubmit` hook adds for the chief of staff: the presets and how to send a
/// worker on one, and, when `message` asks for a worker, what Velvet Hammer made of it. `models`
/// is what the pi runtime lists, empty while it is not installed or has no key.
pub fn context(message: &str, nicknames: &Nicknames, models: &[String]) -> String {
    let presets = if nicknames.presets.is_empty() {
        "The captain has no worker nicknames.".to_string()
    } else {
        let list: Vec<String> = nicknames
            .presets
            .iter()
            .map(|p| format!("\u{201c}{}\u{201d} is {}", p.nickname, p.flags()))
            .collect();
        format!("The captain's worker nicknames: {}.", list.join("; "))
    };
    let mut text = format!(
        "Velvet Hammer can send crewmates on models other than Claude through pi, a runtime it \
         installs; pi is on your PATH. {presets} When the captain asks for a worker by nickname \
         or by provider and model, spawn it with bin/fm-spawn.sh as usual, adding those flags \
         (for a model, --harness pi --model <provider>/<id> as `pi --list-models` lists it) and \
         --dispatch-override with the captain's request in their words. A Claude model always \
         runs with --harness claude on the captain's subscription, never through pi. If the \
         request is ambiguous or names nothing you can resolve, ask the captain one short \
         question. Never start another harness's command-line tool to judge a candidate, and \
         never edit account pin files."
    );
    let Some(Request { phrase, worker }) = request(message) else {
        return text;
    };
    let resolved = resolve(&phrase, nicknames, models);
    // Without "worker", "crewmate" or "agent", only a nickname or a model pi lists makes the
    // message a worker request; "send the report to Alice" is not one.
    if !worker && !matches!(resolved, Resolved::Preset(_) | Resolved::Model(_)) {
        return text;
    }
    let read = match resolved {
        Resolved::Preset(p) => format!(
            "the nickname {}: send the worker with {}.",
            p.nickname,
            p.flags()
        ),
        Resolved::Model(model) => {
            format!("the model {model}: send the worker with --harness pi --model {model}.")
        }
        Resolved::Claude => "a Claude model: send it with --harness claude as usual, on the \
                             captain's subscription."
            .into(),
        Resolved::Ambiguous(models) => format!(
            "several models: {}. Ask the captain which one, in one short question, before \
             sending anything.",
            models.join(", ")
        ),
        Resolved::Unknown if models.is_empty() => {
            "nothing it can resolve: it has no nickname by that name, and the pi runtime lists no \
             models yet (it is not installed, or has no provider key). Tell the captain so in \
             one short sentence, pointing at the Workers page."
                .into()
        }
        Resolved::Unknown => "nothing it can resolve: no nickname or model pi lists matches. \
                              Ask the captain one short question."
            .into(),
    };
    text.push_str(&format!(
        " Velvet Hammer read the request \u{201c}{phrase}\u{201d} in this message as {read}"
    ));
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Part of what pi 1.5.2 lists for an opencode Go key, plus two opencode Zen entries.
    fn models() -> Vec<String> {
        [
            "opencode-go/claude-haiku-5-5",
            "opencode-go/deepseek-v4-pro",
            "opencode-go/deepseek-v4.1-flash",
            "opencode-go/glm-5.3",
            "opencode-go/glm-5.3-flash",
            "opencode-go/kimi-k3",
            "opencode/deepseek-v4.1-flash",
            "opencode/deepseek-v4.1-flash-free",
        ]
        .map(String::from)
        .to_vec()
    }

    fn preset(nickname: &str, harness: Harness, model: &str) -> Preset {
        Preset {
            nickname: nickname.into(),
            harness,
            model: model.into(),
            effort: None,
        }
    }

    #[test]
    fn a_fresh_registry_holds_the_hacker_example() {
        let fresh = Nicknames::default();
        assert_eq!(
            fresh.presets,
            [preset(
                "hacker",
                Harness::Pi,
                "opencode-go/deepseek-v4.1-flash"
            )]
        );
        assert_eq!(
            fresh.presets[0].flags(),
            "--harness pi --model opencode-go/deepseek-v4.1-flash"
        );
    }

    #[test]
    fn the_registry_round_trips_through_its_documented_shape() {
        let text = r#"{
            "version": 1,
            "nicknames": [
                {"nickname": "hacker", "harness": "pi", "model": "opencode-go/deepseek-v4.1-flash", "effort": "high"},
                {"nickname": "careful", "harness": "claude", "model": "opus"}
            ]
        }"#;
        let parsed = Nicknames::from_json(&serde_json::from_str(text).unwrap()).unwrap();
        assert_eq!(parsed.presets.len(), 2);
        assert_eq!(parsed.presets[0].effort.as_deref(), Some("high"));
        assert_eq!(parsed.presets[1].harness, Harness::Claude);
        assert_eq!(Nicknames::from_json(&parsed.to_json()).unwrap(), parsed);
    }

    #[test]
    fn a_registry_with_anything_wrong_is_refused_whole() {
        let refused =
            |text: &str| Nicknames::from_json(&serde_json::from_str(text).unwrap()).unwrap_err();
        assert!(refused(r#"{"nicknames": []}"#).contains("schema version"));
        assert!(refused(r#"{"version": 1}"#).contains("no list"));
        assert!(
            refused(r#"{"version": 1, "nicknames": [{"nickname": "x", "harness": "codex", "model": "a/b"}]}"#)
                .contains("codex")
        );
        let twice = r#"{"version": 1, "nicknames": [
            {"nickname": "x", "harness": "pi", "model": "a/b"},
            {"nickname": "x", "harness": "pi", "model": "a/c"}]}"#;
        assert!(refused(twice).contains("twice"));
        let claude_via_pi = r#"{"version": 1, "nicknames": [
            {"nickname": "x", "harness": "pi", "model": "opencode-go/claude-haiku-5-5"}]}"#;
        assert!(refused(claude_via_pi).contains("never through pi"));
    }

    #[test]
    fn a_claude_model_is_refused_on_pi_and_needs_a_claude_code_model_on_claude() {
        for model in [
            "anthropic/claude-sonnet-5-5",
            "opencode-go/claude-haiku-5-5",
            "openrouter/anthropic/Claude-opus-5",
        ] {
            let problem = preset("x", Harness::Pi, model).check().unwrap_err();
            assert!(problem.contains("never through pi"), "{model}: {problem}");
        }
        assert!(preset("x", Harness::Claude, "sonnet").check().is_ok());
        assert!(
            preset("x", Harness::Claude, "opencode-go/kimi-k3")
                .check()
                .is_err()
        );
        assert!(preset("x", Harness::Pi, "deepseek").check().is_err());
        assert!(preset("Hacker", Harness::Pi, SEED_MODEL).check().is_err());
        let mut effort = preset("x", Harness::Pi, SEED_MODEL);
        effort.effort = Some("turbo".into());
        assert!(effort.check().unwrap_err().contains("xhigh"));
    }

    #[test]
    fn the_page_adds_edits_and_deletes_nicknames_in_the_registry_file() {
        let dir = std::env::temp_dir().join(format!("vh-crew-form-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut store = Store::<Nicknames>::open(&dir);
        let mut form = Form {
            nickname: TextEdit::new("night-owl"),
            harness: Harness::Claude,
            model: TextEdit::new("opus"),
            effort: TextEdit::new("high"),
            ..Form::default()
        };
        form.save(&mut store);
        assert_eq!(form.problem, None);
        assert_eq!(store.settings().presets.len(), 2);

        form.nickname = TextEdit::new("hacker");
        form.harness = Harness::Claude;
        form.model = TextEdit::new("sonnet");
        form.save(&mut store);
        assert!(
            form.problem
                .as_deref()
                .unwrap()
                .contains("already a nickname hacker")
        );

        form.edit(&store, 0);
        form.model = TextEdit::new("opencode-go/kimi-k3");
        form.save(&mut store);
        assert_eq!(store.settings().presets[0].model, "opencode-go/kimi-k3");

        form.delete(&mut store, 1);
        let reread = Store::<Nicknames>::open(&dir);
        assert_eq!(reread.settings().presets.len(), 1);
        assert_eq!(reread.settings().presets[0].nickname, "hacker");
        let text = std::fs::read_to_string(dir.join(FILE)).unwrap();
        assert!(text.contains("\"version\": 1"), "{text}");
    }

    #[test]
    fn a_request_is_read_from_the_captains_words() {
        let cases = [
            (
                "send an opencode go deepseek 4.1 flash worker to investigate",
                Some("opencode go deepseek 4.1 flash"),
            ),
            ("Send a hacker to look at the flaky test.", Some("hacker")),
            ("please send the night owl crewmate", Some("night owl")),
            ("dispatch a kimi k3 agent and report", Some("kimi k3")),
            ("send a worker to fix it", None),
            ("What needs me today?", None),
            ("send it", None),
        ];
        for (message, want) in cases {
            assert_eq!(
                request(message).map(|r| r.phrase).as_deref(),
                want,
                "{message}"
            );
        }
    }

    #[test]
    fn a_nickname_resolves_to_its_preset() {
        let mut nicknames = Nicknames::default();
        nicknames
            .presets
            .push(preset("night-owl", Harness::Claude, "opus"));
        assert_eq!(
            resolve("hacker", &nicknames, &models()),
            Resolved::Preset(nicknames.presets[0].clone())
        );
        assert_eq!(
            resolve("night owl", &nicknames, &models()),
            Resolved::Preset(nicknames.presets[1].clone())
        );
    }

    #[test]
    fn a_provider_and_model_phrase_resolves_against_what_pi_lists() {
        let none = Nicknames::fallback();
        let model = |phrase: &str| resolve(phrase, &none, &models());
        assert_eq!(
            model("opencode go deepseek 4.1 flash"),
            Resolved::Model("opencode-go/deepseek-v4.1-flash".into())
        );
        assert_eq!(
            model("opencode deepseek v4.1 flash"),
            Resolved::Model("opencode/deepseek-v4.1-flash".into())
        );
        assert_eq!(
            model("kimi k3"),
            Resolved::Model("opencode-go/kimi-k3".into())
        );
        // An exact match wins over a longer id with the same words.
        assert_eq!(
            model("opencode go glm 5.3"),
            Resolved::Model("opencode-go/glm-5.3".into())
        );
        assert_eq!(
            model("deepseek flash"),
            Resolved::Ambiguous(vec![
                "opencode-go/deepseek-v4.1-flash".into(),
                "opencode/deepseek-v4.1-flash".into(),
                "opencode/deepseek-v4.1-flash-free".into(),
            ])
        );
        assert_eq!(model("gpt 9"), Resolved::Unknown);
    }

    #[test]
    fn claude_is_never_resolved_to_pi() {
        let none = Nicknames::fallback();
        assert_eq!(
            resolve("opencode go claude haiku", &none, &models()),
            Resolved::Claude
        );
        assert_eq!(resolve("sonnet", &none, &models()), Resolved::Claude);
        // "haiku 5" alone matches only a Claude id, which pi never offers.
        assert_eq!(resolve("5 5", &none, &models()), Resolved::Unknown);
    }

    #[test]
    fn the_hook_context_names_the_presets_and_what_the_request_resolved_to() {
        let nicknames = Nicknames::default();
        let plain = context("What needs me today?", &nicknames, &models());
        assert!(plain.contains(
            "\u{201c}hacker\u{201d} is --harness pi --model opencode-go/deepseek-v4.1-flash"
        ));
        assert!(plain.contains("never through pi"));
        assert!(!plain.contains("read the request"));

        let sent = context("send a hacker to investigate", &nicknames, &models());
        assert!(sent.contains(
            "read the request \u{201c}hacker\u{201d} in this message as the nickname hacker"
        ));

        let vague = context(
            "send a deepseek flash worker to look",
            &nicknames,
            &models(),
        );
        assert!(vague.contains("several models"), "{vague}");
        assert!(vague.contains("one short question"));

        let bare = context("send a kimi k3 worker to look", &nicknames, &[]);
        assert!(bare.contains("lists no models yet"), "{bare}");

        // Without a worker noun, only a nickname or a model makes it a request.
        for message in [
            "send the report to Alice",
            "send the deepseek flash notes to Bob",
            "send sonnet and the draft",
        ] {
            let text = context(message, &nicknames, &models());
            assert!(!text.contains("read the request"), "{message}: {text}");
        }
        assert!(
            context("send kimi k3 to look at it", &nicknames, &models())
                .contains("as the model opencode-go/kimi-k3")
        );
    }
}
