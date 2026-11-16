//! What gets the captain's attention ([`crate::attention`]): its settings page, and the question
//! before a quit that would stop work in flight ([`crate::lifecycle`]).

use rune::tok::{self, Theme, radius, space, type_};
use rune::{El, col, el, row, text};

use super::{all_settings, button, choice, note, page, section, small_button, warning};
use crate::attention::{self, Kind, Wait};
use crate::{App, Showing, lifecycle};

/// Each kind with its switch, then how long a question waits.
pub fn attention_page(s: &App, t: Theme, dark: bool) -> El {
    let settings = s.attention_settings.settings();
    let mut body = col().key("attention-body").gap(space::S6);
    if let Some(problem) = s.attention_settings.problem() {
        body = body.child(warning("attention-problem", problem, dark));
    }
    let mut kinds = col().key("attention-kinds").gap(space::S2).child(note(
        "attention-intro",
        "While Velvet Hammer is not the app in front, these post a macOS notification and count \
         on its Dock icon and menu-bar item. Clicking a notification opens the window at the \
         card. Each notifies once, and the notification goes when you answer. A finished turn \
         never notifies.",
        t,
    ));
    for kind in Kind::ALL {
        let on = settings.on(kind);
        let key = kind.key();
        let means = if kind.notifies() {
            "Notification and Dock badge"
        } else {
            "Dock badge only"
        };
        kinds = kinds.child(
            row()
                .key(format!("attention-{key}"))
                .padding(space::S3)
                .gap(space::S3)
                .center_y()
                .radius(radius::MD)
                .fill(t.bg)
                .border(1.0, t.card_border)
                .child(
                    col()
                        .grow(1.0)
                        .gap(2.0)
                        .child(text(kind.label()).font(type_::TITLE).color(t.text).wrap())
                        .child(note(&format!("attention-{key}-means"), means, t)),
                )
                .child(
                    small_button(
                        &format!("attention-{key}-toggle"),
                        if on {
                            "On \u{b7} turn off"
                        } else {
                            "Off \u{b7} turn on"
                        },
                        t,
                        move |s: &mut App| s.attention_settings.change(|x| x.set(kind, !on)),
                    )
                    .role("switch")
                    .checked(on),
                ),
        );
    }
    body = body.child(section("What gets your attention", t).child(kinds));
    let mut waits = col()
        .key("attention-waits")
        .role("radiogroup")
        .gap(space::S2);
    for wait in Wait::ALL {
        waits = waits.child(choice(
            &format!("attention-wait-{}", wait.key()),
            &attention::words(wait.duration()),
            wait == Wait::RECOMMENDED,
            wait.explanation(),
            settings.wait == wait.duration(),
            t,
            move |s: &mut App| s.attention_settings.change(|x| x.wait = wait.duration()),
        ));
    }
    if !Wait::ALL.iter().any(|w| w.duration() == settings.wait) {
        waits = waits.child(note(
            "attention-wait-custom",
            &format!(
                "Set to {} in {}.",
                attention::words(settings.wait),
                attention::SETTINGS_FILE
            ),
            t,
        ));
    }
    body = body.child(
        section("How long a question waits for you", t)
            .child(note(
                "attention-wait-intro",
                "When your chief of staff's hook asks you about a tool use (a merge, a push, a \
                 write into a project, or anything your tool-approval settings ask about), its \
                 card waits this long. Then the tool use is denied, and the card stays, marked \
                 expired, with Ask again. A change applies to the next question, also in a \
                 session that is running.",
                t,
            ))
            .child(waits),
    );
    body = body.child(note(
        "attention-system",
        "Notifications need Velvet Hammer opened as an app (see Install in the README), and your \
         permission, which macOS asks for with the first one. Run from a terminal with cargo, it \
         shows the badge and the menu-bar item but posts no notifications.",
        t,
    ));
    body = body.child(note(
        "attention-kept",
        &format!(
            "These settings are kept in {} in Velvet Hammer's data directory.",
            attention::SETTINGS_FILE
        ),
        t,
    ));
    let actions = row()
        .gap(space::S2)
        .child(all_settings("attention-all-settings", s, t))
        .child(button("attention-close", "Back to chat", t, |s| {
            s.showing = Showing::Chat
        }));
    page("attention-page", "Attention", actions, body, t)
}

/// The question before quitting while work is in flight, over the middle column.
pub fn quit_question(s: &App, t: Theme, dark: bool) -> Option<El> {
    let in_flight = s.quit_ask.as_ref()?;
    Some(
        col()
            .key("quit-question")
            .role("alertdialog")
            .label("Quit Velvet Hammer?")
            .padding(space::S3)
            .gap(space::S2)
            .radius(radius::LG)
            .fill(t.card)
            .border(2.0, tok::accent(3, dark))
            .child(text("Quit Velvet Hammer?").font(type_::TITLE).color(t.text))
            .child(
                text(in_flight.words())
                    .key("quit-in-flight")
                    .font(type_::BODY)
                    .color(t.text)
                    .wrap(),
            )
            .child(note("quit-consequence", lifecycle::CONSEQUENCE, t))
            .child(
                row()
                    .gap(space::S2)
                    .child(button("quit-keep", "Keep running", t, |s| {
                        s.quit_ask = None
                    }))
                    .child(
                        el().key("quit-anyway")
                            .role("button")
                            .label("Quit anyway")
                            .focusable(true)
                            .h(28.0)
                            .px(space::S3)
                            .center()
                            .radius(radius::SM)
                            .fill(t.control_off)
                            .on_press_tx(|s: &mut App, tx| s.quit_anyway(tx))
                            .child(text("Quit anyway").font(type_::LABEL).color(t.text)),
                    ),
            ),
    )
}
