//! Creator mode in the window (see [`crate::creator`]): a card in Waiting on you for each change
//! to the app that is ready to try, the banner over the chat while a change is built or tried,
//! and the setting.

use rune::tok::{Theme, radius, space, type_};
use rune::{El, col, row, text};

use super::{button, choice, note, section, small_button, warning};
use crate::App;
use crate::creator::build::State;
use crate::creator::{Change, Mode};

/// A card for each change ready to try, with what can be done with it now.
pub fn rows(s: &App, t: Theme, dark: bool) -> Vec<El> {
    s.creator
        .changes
        .iter()
        .map(|change| card(s, change, t, dark))
        .collect()
}

fn card(s: &App, change: &Change, t: Theme, dark: bool) -> El {
    let key = format!("change-{}", change.task);
    let mut card = col()
        .key(key.clone())
        .padding(space::S3)
        .gap(space::S2)
        .radius(radius::MD)
        .fill(t.bg)
        .child(
            text(format!("Change to Velvet Hammer \u{b7} {}", change.name))
                .font(type_::TITLE)
                .color(t.text)
                .wrap(),
        )
        .child(note(&format!("{key}-summary"), &change.summary, t))
        .child(note(
            &format!("{key}-where"),
            &format!(
                "Commit {} on task {}'s branch. Nothing is pushed until you keep it.",
                change.short(),
                change.task
            ),
            t,
        ));
    let trial = s.creator.trial_of(change);
    if let Some(words) = trial.and_then(|t| said(t.state, t.detail.as_deref())) {
        let failed = matches!(
            trial.map(|t| t.state),
            Some(State::Refused | State::Failed | State::Restored)
        );
        card = card.child(if failed {
            warning(&format!("{key}-state"), &words, dark)
        } else {
            note(&format!("{key}-state"), &words, t)
        });
    }
    let building = s
        .creator
        .trial
        .as_ref()
        .is_some_and(|t| t.state == State::Building);
    let mut buttons = row().gap(space::S2).flex_wrap().center_y();
    if trial.is_some_and(|t| t.state == State::Trying) {
        let (keep, undo) = (change.clone(), key.clone());
        buttons = buttons
            .child(button(&format!("{key}-keep"), "Keep", t, move |s| {
                s.keep_change(&keep)
            }))
            .child(button(&format!("{undo}-undo"), "Undo", t, |s| {
                s.undo_change()
            }));
    } else if !building {
        let tried = change.clone();
        buttons = buttons.child(button(&format!("{key}-try"), "Try it", t, move |s| {
            s.try_change(&tried)
        }));
    }
    let dropped = change.clone();
    buttons = buttons.child(small_button(
        &format!("{key}-discard"),
        "Discard",
        t,
        move |s| s.discard_change(&dropped),
    ));
    if let Some(problem) = &s.creator.problem {
        card = card.child(warning(&format!("{key}-problem"), problem, dark));
    }
    card.child(buttons)
}

/// What a trial's state means for the captain; `None` when there is nothing to add.
fn said(state: State, detail: Option<&str>) -> Option<String> {
    let detail = detail.unwrap_or_default();
    Some(match state {
        State::Building => "Building it on this Mac. The app swaps when it is ready.".into(),
        State::Trying => "You are trying this change now.".into(),
        State::Kept => "Kept. Your chief of staff is delivering it as a pull request.".into(),
        State::Undone => "Undone. The last kept build is back.".into(),
        State::Refused | State::Failed | State::Restored => detail.to_string(),
    })
}

/// Over the chat while a change is built or tried: what is happening, and Keep and Undo while
/// a trial runs, so the captain can end it from wherever they are.
pub fn banner(s: &App, t: Theme) -> Option<El> {
    let trial = s.creator.trial.as_ref()?;
    let change = s.creator.changes.iter().find(|c| c.commit == trial.commit);
    let what = match change {
        Some(c) => format!("{} ({})", c.name, c.short()),
        None => format!("commit {}", &trial.commit[..trial.commit.len().min(7)]),
    };
    let banner = col()
        .key("trial-banner")
        .padding(space::S3)
        .gap(space::S2)
        .radius(radius::MD)
        .fill(t.card)
        .border(1.0, t.card_border);
    match trial.state {
        State::Building => Some(banner.child(note(
            "trial-banner-words",
            &format!("Building {what}. The window swaps to it when it is ready."),
            t,
        ))),
        State::Trying => {
            let mut buttons = row().gap(space::S2);
            if let Some(change) = change.cloned() {
                buttons = buttons.child(button("trial-keep", "Keep", t, move |s| {
                    s.keep_change(&change)
                }));
            }
            buttons = buttons.child(button("trial-undo", "Undo", t, |s| s.undo_change()));
            Some(
                banner
                    .child(note(
                        "trial-banner-words",
                        &format!(
                            "You are trying {what}. Keep delivers it as a pull request; Undo \
                             brings the last kept build back."
                        ),
                        t,
                    ))
                    .child(buttons),
            )
        }
        _ => None,
    }
}

/// The creator mode setting, on the Settings page.
pub fn setting(s: &App, t: Theme, dark: bool) -> El {
    let mode = *s.creator.mode.settings();
    let mut part = section("Creator mode", t).key("creator-section");
    if let Some(problem) = s.creator.mode.problem() {
        part = part.child(warning("creator-problem", problem, dark));
    }
    let mut choices = col().key("creator-modes").role("radiogroup").gap(space::S2);
    for option in Mode::ALL {
        choices = choices.child(choice(
            &format!("creator-{}", option.key()),
            option.label(),
            false,
            option.explanation(),
            mode == option,
            t,
            move |s: &mut App| s.creator.mode.change(|m| *m = option),
        ));
    }
    part.child(choices)
}
