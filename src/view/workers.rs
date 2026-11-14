//! The Workers page: the pi runtime for workers on other providers ([`crate::pi`]), the
//! provider key in the keychain, firstmate's pi account setting, and the captain's worker
//! nicknames ([`crate::crew`]).

use rune::tok::{self, Theme, radius, space, type_};
use rune::{El, col, el, row, text};

use super::policy::quote;
use super::{all_settings, button, field, note, page, section, small_button, warning};
use crate::crew::{self, Harness};
use crate::{App, Showing, pi};

/// The Workers section's summary: how many nicknames, and whether pi is ready.
pub fn workers_summary(s: &App) -> String {
    let n = s.nicknames.settings().presets.len();
    let names = format!("{n} nickname{}", if n == 1 { "" } else { "s" });
    let runtime = if s.runtime.installing() {
        "installing pi\u{2026}"
    } else if s.runtime.installed() {
        "pi ready"
    } else {
        "pi not installed"
    };
    format!("{names} \u{b7} {runtime}")
}

pub fn workers_page(s: &App, t: Theme, dark: bool) -> El {
    let mut body = col().key("workers-body").gap(space::S6).max_w(640.0);
    body = body.child(note(
        "workers-lede",
        "Your chief of staff can send crewmates on models other than Claude, such as DeepSeek \
         V4.1 Flash on your opencode Go plan. They run on pi, a coding agent Velvet Hammer \
         installs for them, so you install nothing yourself. Claude workers are unchanged: they \
         run on Claude Code and your Claude subscription, never through pi.",
        t,
    ));
    body = body
        .child(runtime_section(s, t, dark))
        .child(key_section(s, t, dark))
        .child(account_section(s, t, dark))
        .child(nicknames_section(s, t, dark));
    let actions = row()
        .gap(space::S2)
        .child(all_settings("workers-all-settings", s, t))
        .child(button("workers-close", "Back to chat", t, |s| {
            s.showing = Showing::Chat
        }));
    page("workers-page", "Workers", actions, body, t)
}

fn runtime_section(s: &App, t: Theme, dark: bool) -> El {
    let rt = &s.runtime;
    let mut part = section("1. The pi runtime", t);
    let status = if rt.installing() {
        format!("Installing pi {}\u{2026}", pi::VERSION)
    } else if rt.installed() {
        let models = rt.models().read().map(|m| m.len()).unwrap_or(0);
        format!(
            "pi {} is installed in {}. It lists {models} model{} for the keys it has.",
            pi::VERSION,
            rt.bin_dir().display(),
            if models == 1 { "" } else { "s" }
        )
    } else {
        format!("pi {} is not installed.", pi::VERSION)
    };
    part = part.child(
        text(status)
            .key("workers-runtime")
            .font(type_::BODY)
            .color(t.text)
            .wrap_anywhere(),
    );
    if let Some(problem) = &rt.problem {
        part = part.child(warning("workers-runtime-problem", problem, dark));
    }
    if let Some(problem) = &rt.models_problem {
        part = part.child(warning("workers-models-problem", problem, dark));
    }
    let what = match rt.release() {
        Some(release) => format!(
            "Install downloads {} from pi's GitHub release and checks it against the SHA-256 \
             built into Velvet Hammer before unpacking it. pi is MIT licensed. Your chief of \
             staff finds it first on its PATH; firstmate runs it in a terminal pane like any \
             other crewmate.",
            release.asset
        ),
        None => "pi publishes no build for this computer.".into(),
    };
    part = part.child(note("workers-runtime-help", &what, t));
    let mut buttons = row().gap(space::S2);
    if !rt.installing() && rt.release().is_some() {
        let label = if rt.installed() {
            "Install again"
        } else {
            "Install pi"
        };
        buttons = buttons.child(button("workers-install", label, t, |s| s.runtime.install()));
    }
    if rt.installed() {
        buttons = buttons.child(small_button(
            "workers-models",
            "List models again",
            t,
            |s| s.runtime.list_models(),
        ));
    }
    part.child(buttons)
}

fn key_section(s: &App, t: Theme, dark: bool) -> El {
    let mut part = section("2. Your opencode Go key", t);
    let status = match s.key {
        None => "Not checked yet.",
        Some(pi::Key::Present) => "Your opencode Go key is in the keychain.",
        Some(pi::Key::Missing) => "No opencode Go key is in the keychain yet.",
        Some(pi::Key::Unsupported) => "This computer has no macOS keychain to keep the key in.",
    };
    part = part.child(
        text(status)
            .key("workers-key")
            .font(type_::BODY)
            .color(if s.key == Some(pi::Key::Missing) {
                tok::accent(3, dark)
            } else {
                t.text
            })
            .wrap(),
    );
    part.child(note(
        "workers-key-help",
        "Velvet Hammer never reads, shows or stores the key. Copy it from the opencode \
         console, run this in Terminal, and paste the key when security asks for the password; \
         it is not shown and never lands in a file or your shell history. Then press Check \
         again. DeepSeek models also need Global chosen under Settings, Privacy, Regions in the \
         opencode console.",
        t,
    ))
    .child(quote("workers-key-command", pi::ADD_KEY, t))
    .child(
        row().child(small_button("workers-key-check", "Check again", t, |s| {
            s.check_key()
        })),
    )
}

fn account_section(s: &App, t: Theme, dark: bool) -> El {
    let home = s.firstmate.home_dir();
    let path = pi::pin_path(&home);
    let wanted = pi::pin_text(&s.runtime);
    let current = pi::current_pin(&home);
    let mut part = section("3. Firstmate's pi account", t);
    let status = match &current {
        Some(text) if *text == wanted => {
            "Firstmate's pi workers use Velvet Hammer's pi directory and may spend only opencode Go."
                .to_string()
        }
        Some(text) => format!(
            "Firstmate's pi account setting points elsewhere:\n{}",
            text.trim_end()
        ),
        None => "Firstmate's pi workers are not pointed at Velvet Hammer's pi directory yet."
            .to_string(),
    };
    part = part.child(
        text(status)
            .key("workers-account")
            .font(type_::BODY)
            .color(t.text)
            .wrap_anywhere(),
    );
    part = part
        .child(note(
            "workers-account-help",
            &format!(
                "The button writes two files and nothing else: {} in your firstmate home, with \
                 the lines below, and {}, which tells pi to ask the keychain for the key each \
                 time it needs it. Your own ~/.pi, if you have one, and Claude's sign-in are not \
                 touched.",
                path.display(),
                s.runtime.agent_dir().join("auth.json").display()
            ),
            t,
        ))
        .child(quote("workers-account-text", wanted.trim_end(), t));
    if let Some(problem) = &s.pin_problem {
        part = part.child(warning("workers-account-problem", problem, dark));
    }
    if current.as_ref() != Some(&wanted) {
        let label = if current.is_some() {
            "Replace firstmate's pi account setting"
        } else {
            "Point pi workers at Velvet Hammer"
        };
        part =
            part.child(row().child(button("workers-connect", label, t, |s| s.connect_workers())));
    }
    part
}

fn nicknames_section(s: &App, t: Theme, dark: bool) -> El {
    let form = &s.crew_form;
    let mut part = section("4. Nicknames", t).child(note(
        "nicknames-help",
        "Say \u{201c}send a hacker to look into the flaky test\u{201d} and your chief of staff \
         sends that preset. You can also name a provider and model, such as \u{201c}send an \
         opencode go deepseek 4.1 flash worker\u{201d}; it asks you one short question when the \
         words match more than one model.",
        t,
    ));
    if let Some(problem) = s.nicknames.problem() {
        part = part.child(warning("nicknames-problem", problem, dark));
    }
    let presets = &s.nicknames.settings().presets;
    let mut list = col().key("nicknames").gap(space::S2);
    if presets.is_empty() {
        list = list.child(note("nicknames-empty", "No nicknames.", t));
    }
    for (i, preset) in presets.iter().enumerate() {
        list = list.child(
            row()
                .key(format!("nickname-{i}"))
                .gap(space::S2)
                .center_y()
                .padding(space::S2)
                .radius(radius::SM)
                .fill(if form.editing == Some(i) {
                    t.control_off
                } else {
                    t.bg
                })
                .child(
                    col()
                        .grow(1.0)
                        .gap(2.0)
                        .child(
                            text(preset.nickname.clone())
                                .font(type_::TITLE)
                                .color(t.text),
                        )
                        .child(
                            note(&format!("nickname-{i}-runs"), &preset.describe(), t)
                                .wrap_anywhere(),
                        ),
                )
                .child(small_button(
                    &format!("nickname-{i}-edit"),
                    "Edit",
                    t,
                    move |s| s.crew_form.edit(&s.nicknames, i),
                ))
                .child(small_button(
                    &format!("nickname-{i}-delete"),
                    "Delete",
                    t,
                    move |s| s.crew_form.delete(&mut s.nicknames, i),
                )),
        );
    }

    let mut harnesses = row()
        .key("nickname-harness")
        .role("radiogroup")
        .gap(space::S1);
    for harness in Harness::ALL {
        let selected = form.harness == harness;
        harnesses = harnesses.child(
            el().key(format!("nickname-harness-{}", harness.key()))
                .role("radio")
                .label(harness.label())
                .checked(selected)
                .focusable(true)
                .h(28.0)
                .px(space::S3)
                .center()
                .radius(radius::SM)
                .fill(if selected {
                    tok::accent(0, true)
                } else {
                    t.control_off
                })
                .on_press(move |s: &mut App| s.crew_form.harness = harness)
                .child(text(harness.label()).font(type_::LABEL).color(if selected {
                    t.knob
                } else {
                    t.text
                })),
        );
    }
    let model_hint = match form.harness {
        Harness::Pi => format!("Model as pi lists it, such as {}", crew::SEED_MODEL),
        Harness::Claude => "Claude Code model, such as sonnet or opus".to_string(),
    };
    let mut editor = col()
        .key("nickname-form")
        .gap(space::S2)
        .child(field(
            "nickname-name",
            &form.nickname,
            "Nickname, such as hacker",
            t,
            |s| &mut s.crew_form.nickname,
        ))
        .child(harnesses)
        .child(field("nickname-model", &form.model, &model_hint, t, |s| {
            &mut s.crew_form.model
        }))
        .child(field(
            "nickname-effort",
            &form.effort,
            "Effort (optional): low, medium, high, xhigh or max",
            t,
            |s| &mut s.crew_form.effort,
        ));
    if let Some(problem) = &form.problem {
        editor = editor.child(warning("nickname-problem", problem, dark));
    }
    let mut buttons = row().gap(space::S2).child(button(
        "nickname-save",
        if form.editing.is_some() {
            "Save nickname"
        } else {
            "Add nickname"
        },
        t,
        |s| s.crew_form.save(&mut s.nicknames),
    ));
    if form.editing.is_some() {
        buttons = buttons.child(button("nickname-cancel", "Cancel", t, |s| {
            s.crew_form.clear()
        }));
    }
    part.child(list).child(editor.child(buttons)).child(note(
        "nicknames-kept",
        &format!(
            "Kept in {}, which your chief of staff is shown with each of your messages.",
            s.nicknames.path().display()
        ),
        t,
    ))
}
