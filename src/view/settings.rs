//! Settings: every setting the app has, by section, reached from the Settings menu item (Cmd+,)
//! rather than from the main window. Each section opens its own page, except the conversation
//! history's, which is small enough to set here. Also the in-window menu bar that stands in for
//! the native one where Rune has none (see [`crate::menus`]).

use rune::tok::{Theme, radius, space, type_};
use rune::{El, col, el, row, text};

use super::{button, choice, firstmate_summary, note, page, section, warning};
use crate::history::Keeping;
use crate::menus;
use crate::{App, Showing, guard};

/// The settings, by section.
pub fn settings_page(s: &App, t: Theme, dark: bool) -> El {
    let on = guard::Rule::ALL
        .iter()
        .filter(|r| s.rules.settings().on(**r))
        .count();
    let sections = col()
        .key("settings-sections")
        .gap(space::S2)
        .child(section_card(
            "approvals",
            "Tool approval",
            s.approvals.settings().mode.label().to_string(),
            Showing::Approvals,
            t,
        ))
        .child(section_card(
            "visuals",
            "Visual replies",
            s.visuals.settings().label().to_string(),
            Showing::Visuals,
            t,
        ))
        .child(section_card(
            "rules",
            "Firstmate rules",
            format!("{on} of {} on", guard::Rule::ALL.len()),
            Showing::Rules,
            t,
        ))
        .child(section_card(
            "firstmate",
            "Firstmate",
            firstmate_summary(&s.firstmate).to_string(),
            Showing::Firstmate,
            t,
        ));
    let body = col()
        .key("settings-body")
        .gap(space::S6)
        .child(
            section("Your chief of staff", t)
                .child(note(
                    "settings-help",
                    "How your chief of staff's tool uses are decided, how its replies are shown, \
                     the firstmate rules its hook enforces, and the firstmate clone it runs on. \
                     Each opens its page.",
                    t,
                ))
                .child(sections),
        )
        .child(history_section(s, t, dark));
    let actions = row()
        .gap(space::S2)
        .child(button("settings-close", "Back to chat", t, |s| {
            s.showing = Showing::Chat
        }));
    page(
        "settings-page",
        s.platform.settings_name(),
        actions,
        body,
        t,
    )
}

/// A section of the settings that opens its page, with its setting in a few words keyed
/// `<key>-summary`.
fn section_card(key: &str, title: &str, summary: String, page: Showing, t: Theme) -> El {
    col()
        .key(key)
        .role("button")
        .label(title.to_string())
        .focusable(true)
        .padding(space::S3)
        .gap(2.0)
        .radius(radius::MD)
        .fill(t.bg)
        .border(1.0, t.card_border)
        .hover(t.control_off)
        .on_press(move |s: &mut App| s.showing = page.clone())
        .child(text(title.to_string()).font(type_::TITLE).color(t.text))
        .child(note(&format!("{key}-summary"), &summary, t))
}

/// Whether the conversation is kept across restarts, and clearing what is kept.
fn history_section(s: &App, t: Theme, dark: bool) -> El {
    let keeping = *s.keeping.settings();
    let mut part = section("Conversation history", t).key("history-section");
    for problem in [s.keeping.problem(), s.history.problem()]
        .into_iter()
        .flatten()
    {
        part = part.child(warning("history-problem", problem, dark));
    }
    let mut choices = col().key("history-modes").role("radiogroup").gap(space::S2);
    for option in Keeping::ALL {
        choices = choices.child(choice(
            &format!("history-{}", option.key()),
            option.label(),
            option == Keeping::RECOMMENDED,
            option.explanation(),
            keeping == option,
            t,
            move |s: &mut App| s.keeping.change(|k| *k = option),
        ));
    }
    let clear = if s.confirm_clear {
        row()
            .gap(space::S2)
            .flex_wrap()
            .child(button(
                "history-clear-confirm",
                "Delete the kept conversation",
                t,
                |s| s.clear_history(),
            ))
            .child(button("history-clear-cancel", "Keep it", t, |s| {
                s.confirm_clear = false
            }))
    } else {
        row().child(button("history-clear", "Clear history", t, |s| {
            s.confirm_clear = true
        }))
    };
    let mut part = part.child(choices).child(clear);
    if s.confirm_clear {
        part = part.child(note(
            "history-clear-ask",
            "This deletes the kept conversation and its rotated copy, and takes the earlier \
             sessions off the chat. Links to those messages stop working. This session's \
             messages stay on screen until you quit.",
            t,
        ));
    }
    part.child(note(
        "history-kept",
        &format!(
            "Kept in {}, readable by your user only. It holds what you and your chief of staff \
             wrote, as written, so avoid putting secrets in messages. Nothing kept is sent to \
             your chief of staff unless you link to it.",
            s.history.path().display()
        ),
        t,
    ))
}

/// The menus along the top of the window, where there is no native menu bar: each menu's title,
/// and the open one's items with their shortcuts.
pub fn menu_bar(s: &App, t: Theme) -> El {
    let menus = menus::in_window(s.platform);
    let mut bar = row()
        .key("menu-bar")
        .role("menubar")
        .label("Menu bar")
        .w_full()
        .h(28.0)
        .px(space::S2)
        .gap(space::S1)
        .center_y()
        .fill(t.card)
        .border(1.0, t.card_border);
    let mut open = None;
    for (title, items) in menus {
        let shown = s.open_menu == Some(title);
        bar = bar.child(
            el().key(format!("menu-{}", title.to_lowercase()))
                .role("menuitem")
                .label(title)
                .focusable(true)
                .h(24.0)
                .px(space::S2)
                .center()
                .radius(radius::SM)
                .fill(if shown { t.control_off } else { t.card })
                .on_press(move |s: &mut App| {
                    s.open_menu = if s.open_menu == Some(title) {
                        None
                    } else {
                        Some(title)
                    }
                })
                .child(text(title).font(type_::LABEL).color(t.text)),
        );
        if shown {
            open = Some(items);
        }
    }
    let Some(items) = open else {
        return bar;
    };
    let mut list = col()
        .key("menu-open")
        .role("menu")
        .padding(space::S1)
        .gap(2.0)
        .radius(radius::MD)
        .fill(t.card)
        .border(1.0, t.card_border);
    for item in items {
        let command = item.command;
        list = list.child(
            row()
                .key(format!("menu-item-{}", item.id))
                .role("menuitem")
                .label(item.title)
                .focusable(true)
                .w(260.0)
                .h(26.0)
                .px(space::S2)
                .gap(space::S3)
                .center_y()
                .radius(radius::SM)
                .hover(t.control_off)
                .on_press_tx(move |s: &mut App, tx: &mut rune::Tx| s.command(command, tx))
                .child(text(item.title).font(type_::LABEL).color(t.text).grow(1.0))
                .child(
                    text(item.keys.unwrap_or_default())
                        .font(type_::LABEL)
                        .color(t.text_secondary),
                ),
        );
    }
    col()
        .w_full()
        .child(bar)
        .child(row().px(space::S2).child(list))
}

/// The page buttons that return to Settings, on each settings page.
pub fn all_settings(key: &str, s: &App, t: Theme) -> El {
    button(
        key,
        &format!("All {}", s.platform.settings_name().to_lowercase()),
        t,
        |s| s.showing = Showing::Settings,
    )
}
