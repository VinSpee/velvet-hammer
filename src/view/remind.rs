//! The captain's reminders (see [`crate::reminders`]): the banner over the middle column while
//! one is due, its card at the top of For you to do, the Reminders page and its settings. A
//! reminder's text is the model's words, so it is only ever drawn as plain text.

use jiff::Zoned;
use rune::tok::{self, Theme, radius, space, type_};
use rune::{El, col, row, text, text_input};

use super::{button, note, page, section, small_button, warning};
use crate::reminders::{Due, Reminder, Setting, Unit, When, clock_words, words};
use crate::{App, Showing};

/// The banner over the middle column while a reminder is due: the latest to come due, with
/// how many more wait.
pub fn banner(s: &App, t: Theme, dark: bool) -> Option<El> {
    let due: Vec<Reminder> = s
        .reminders
        .list()
        .into_iter()
        .filter(|r| r.fired.is_some())
        .collect();
    let latest = due.last()?;
    let now = s.reminders.now();
    let mut body = col()
        .key("reminder-banner")
        .role("alert")
        .label(format!("Reminder: {}", latest.text))
        .padding(space::S3)
        .gap(space::S2)
        .radius(radius::LG)
        .fill(t.card)
        .border(2.0, tok::accent(3, dark))
        .child(
            text(if latest.overdue() {
                "Reminder \u{b7} overdue"
            } else {
                "Reminder"
            })
            .font(type_::SECTION)
            .color(tok::accent(3, dark)),
        )
        .child(
            text(latest.text.clone())
                .key("reminder-banner-text")
                .font(type_::TITLE)
                .color(t.text)
                .wrap()
                .selectable(true),
        )
        .child(note("reminder-banner-when", &when_line(latest, &now), t))
        .child(actions(s, "banner", latest, t, dark));
    if due.len() > 1 {
        body = body.child(row().child(small_button(
            "reminder-banner-more",
            &format!("{} more due \u{b7} All reminders", due.len() - 1),
            t,
            |s| s.showing = Showing::Reminders,
        )));
    }
    Some(body)
}

/// When it was set and when it came due, such as "Set Today, 2:30 PM · due Today, 2:32 PM".
fn when_line(r: &Reminder, now: &Zoned) -> String {
    let set = format!("Set {}", words(r.set_at, now));
    match (r.due, r.fired) {
        (Due::At(at), _) => format!("{set} \u{b7} due {}", words(at, now)),
        (Due::Later, Some(fired)) => {
            format!(
                "{set} \u{b7} back from the Later list {}",
                words(fired, now)
            )
        }
        (Due::Later, None) => format!("{set} \u{b7} on the Later list"),
    }
}

/// Done, Snooze and Delete for one reminder, keyed `<place>-<id>-<action>`, with the snooze menu
/// when it is open here.
fn actions(s: &App, place: &'static str, r: &Reminder, t: Theme, dark: bool) -> El {
    let id = r.id;
    let key = |action: &str| format!("{place}-{id}-{action}");
    let open = s.snoozing == Some((place, id));
    let mut buttons = row().gap(space::S2).flex_wrap();
    if r.fired.is_some() {
        buttons = buttons.child(button(&key("done"), "Done", t, move |s| {
            s.reminder_done(id)
        }));
    }
    buttons = buttons
        .child(small_button(&key("snooze"), "Snooze", t, move |s| {
            s.snoozing = if s.snoozing == Some((place, id)) {
                None
            } else {
                Some((place, id))
            };
            s.snooze_problem = None;
        }))
        .child(small_button(&key("delete"), "Delete", t, move |s| {
            s.delete_reminder(id)
        }));
    let mut out = col().gap(space::S2).child(buttons);
    if open {
        out = out.child(snooze_menu(s, place, id, t, dark));
    }
    out
}

/// The snooze choices: a few fixed times, and a number of minutes, hours or days.
fn snooze_menu(s: &App, place: &'static str, id: u64, t: Theme, dark: bool) -> El {
    let key = |what: &str| format!("{place}-{id}-snooze-{what}");
    let mut fixed = row().gap(space::S1).flex_wrap();
    for (what, label, when) in When::SNOOZES {
        fixed = fixed.child(small_button(&key(what), label, t, move |s| {
            s.snooze(id, when)
        }));
    }
    let mut custom = row().gap(space::S1).center_y().child(
        row()
            .w(64.0)
            .h(24.0)
            .px(space::S2)
            .center_y()
            .radius(radius::SM)
            .fill(t.bg)
            .border(1.0, t.card_border)
            .child(
                text_input(&s.snooze_amount, type_::LABEL)
                    .key("snooze-amount")
                    .single_line()
                    .grow(1.0)
                    .color(t.text)
                    .placeholder("In\u{2026}", t.text_secondary)
                    .on_edit(|s: &mut App| &mut s.snooze_amount),
            ),
    );
    for unit in Unit::ALL {
        custom = custom.child(small_button(&key(unit.key()), unit.key(), t, move |s| {
            s.snooze_for(id, unit)
        }));
    }
    let mut menu = col()
        .key(key("menu"))
        .role("menu")
        .label("Snooze")
        .padding(space::S2)
        .gap(space::S2)
        .radius(radius::MD)
        .fill(t.control_off)
        .child(fixed)
        .child(custom.child(small_button(&key("cancel"), "Cancel", t, |s| {
            s.snoozing = None
        })));
    if let Some(problem) = &s.snooze_problem {
        menu = menu.child(warning("snooze-problem", problem, dark));
    }
    menu
}

/// The reminders that came due, for the top of For you to do.
pub fn panel_rows(s: &App, t: Theme, dark: bool) -> Vec<El> {
    let now = s.reminders.now();
    s.reminders
        .list()
        .iter()
        .filter(|r| r.fired.is_some())
        .map(|r| {
            col()
                .key(format!("reminder-{}", r.id))
                .padding(space::S3)
                .gap(space::S2)
                .radius(radius::MD)
                .fill(t.bg)
                .border(1.0, tok::accent(3, dark))
                .child(
                    text(r.text.clone())
                        .font(type_::TITLE)
                        .color(t.text)
                        .wrap()
                        .selectable(true),
                )
                .child(note(
                    &format!("reminder-{}-when", r.id),
                    &format!(
                        "Reminder{} \u{b7} {}",
                        if r.overdue() { " \u{b7} overdue" } else { "" },
                        when_line(r, &now)
                    ),
                    t,
                ))
                .child(actions(s, "panel", r, t, dark))
        })
        .collect()
}

/// The groups the Reminders page shows, by key and title.
const GROUPS: [(&str, &str); 3] = [
    ("due", "Due now"),
    ("coming", "Coming up"),
    ("later", "Later list"),
];

/// The key of the group a reminder is in.
fn group(r: &Reminder) -> &'static str {
    match (r.fired, r.due) {
        (Some(_), _) => "due",
        (None, Due::At(_)) => "coming",
        (None, Due::Later) => "later",
    }
}

/// The card summary under the agents, such as "1 due · 3 coming up · 2 later".
pub fn summary(s: &App) -> String {
    let list = s.reminders.list();
    let parts: Vec<String> = [("due", "due"), ("coming", "coming up"), ("later", "later")]
        .into_iter()
        .map(|(key, what)| (list.iter().filter(|r| group(r) == key).count(), what))
        .filter(|(n, _)| *n > 0)
        .map(|(n, what)| format!("{n} {what}"))
        .collect();
    if parts.is_empty() {
        "none set".into()
    } else {
        parts.join(" \u{b7} ")
    }
}

/// Every reminder: due now, coming up by time, and the Later list.
pub fn reminders_page(s: &App, t: Theme, dark: bool) -> El {
    let now = s.reminders.now();
    let list = s.reminders.list();
    let mut body = col().key("reminders-body").gap(space::S6);
    if let Some(problem) = s.reminders.problem() {
        body = body.child(warning("reminders-problem", &problem, dark));
    }
    if list.is_empty() {
        body = body.child(note(
            "reminders-empty",
            "No reminders. Tell your chief of staff \u{201c}remind me in 20 minutes to check the \
             deploy\u{201d}, \u{201c}remind me tomorrow morning\u{201d} or \u{201c}remind me \
             later\u{201d}.",
            t,
        ));
    }
    for (key, title) in GROUPS {
        let rows: Vec<&Reminder> = list.iter().filter(|r| group(r) == key).collect();
        if rows.is_empty() {
            continue;
        }
        let mut group = section(title, t).key(format!("reminders-{key}"));
        if key == "later" {
            group = group.child(note(
                "reminders-later-help",
                &format!(
                    "Each comes back once a day at {}, or when Velvet Hammer next opens if that \
                     time has passed, until you mark it done or delete it.",
                    clock_words(s.reminder_settings.settings().later_time)
                ),
                t,
            ));
        }
        for r in rows {
            let due = match r.due {
                Due::At(at) => words(at, &now),
                Due::Later => "Later list".into(),
            };
            group = group.child(
                col()
                    .key(format!("reminders-{}", r.id))
                    .padding(space::S3)
                    .gap(space::S1)
                    .radius(radius::MD)
                    .fill(t.bg)
                    .child(
                        text(due)
                            .key(format!("reminders-{}-due", r.id))
                            .font(type_::SECTION)
                            .color(if r.overdue() {
                                tok::accent(3, dark)
                            } else {
                                t.text_secondary
                            }),
                    )
                    .child(
                        text(r.text.clone())
                            .font(type_::BODY)
                            .color(t.text)
                            .wrap()
                            .selectable(true),
                    )
                    .child(note(
                        &format!("reminders-{}-set", r.id),
                        &format!("Set {}", words(r.set_at, &now)),
                        t,
                    ))
                    .child(actions(s, "list", r, t, dark)),
            );
        }
        body = body.child(group);
    }
    body = body.child(note(
        "reminders-kept",
        &format!(
            "These are your own reminders in Velvet Hammer, not firstmate tasks. Each keeps its \
             text, when it was set, when it is due and whether it has come due, in {} in \
             Velvet Hammer's data directory, readable by your user only. A reminder that comes \
             due shows here, in a banner and in For you to do, and in your system's \
             notifications where Velvet Hammer may post them.",
            crate::reminders::FILE
        ),
        t,
    ));
    let actions = row()
        .gap(space::S2)
        .child(button("reminders-settings", "Settings", t, |s| {
            s.showing = Showing::ReminderSettings
        }))
        .child(button("reminders-close", "Back to chat", t, |s| {
            s.showing = Showing::Chat
        }));
    page("reminders-page", "Reminders", actions, body, t)
}

/// The times the keywords stand for, each with a step down and up, and what each keyword would
/// mean said now.
pub fn settings_page(s: &App, t: Theme, dark: bool) -> El {
    let settings = s.reminder_settings.settings();
    let mut body = col().key("reminder-settings-body").gap(space::S6);
    if let Some(problem) = s.reminder_settings.problem() {
        body = body.child(warning("reminder-settings-problem", problem, dark));
    }
    let mut rows = col().gap(space::S2);
    for setting in Setting::ALL {
        let key = format!("reminder-setting-{}", setting.key());
        rows = rows.child(
            row()
                .key(key.clone())
                .gap(space::S2)
                .center_y()
                .padding(space::S2)
                .radius(radius::SM)
                .fill(t.bg)
                .child(
                    text(setting.label())
                        .font(type_::LABEL)
                        .color(t.text)
                        .wrap()
                        .grow(1.0),
                )
                .child(small_button(
                    &format!("{key}-down"),
                    "\u{2212}",
                    t,
                    move |s| s.reminder_settings.change(|x| x.step(setting, false)),
                ))
                .child(
                    text(settings.shown(setting))
                        .key(format!("{key}-value"))
                        .font(type_::TITLE)
                        .color(t.text)
                        .w(96.0),
                )
                .child(small_button(&format!("{key}-up"), "+", t, move |s| {
                    s.reminder_settings.change(|x| x.step(setting, true))
                })),
        );
    }
    body = body.child(
        section("What the words mean", t)
            .child(note(
                "reminder-settings-help",
                "In n minutes and in n hours are exact. In n days keeps the time of day. Later \
                 today is the first number of hours from now, but no later than the evening \
                 time; once the evening time has passed, it is the second number of hours from \
                 now. Times are your computer's local time and follow daylight saving changes.",
                t,
            ))
            .child(rows)
            .child(row().child(small_button(
                "reminder-settings-reset",
                "Reset to the defaults",
                t,
                |s| s.reminder_settings.change(|x| *x = Default::default()),
            ))),
    );
    let now = s.reminders.now();
    let mut examples = col().gap(space::S1);
    for (key, said, when) in [
        ("later-today", "later today", When::LaterToday),
        ("tomorrow", "tomorrow morning", When::TomorrowMorning),
        ("next-week", "next week", When::NextWeek),
    ] {
        let meaning = match settings.resolve(when, &now) {
            Ok(Due::At(at)) => words(at, &now),
            Ok(Due::Later) => "the Later list".into(),
            Err(problem) => problem,
        };
        examples = examples.child(
            text(format!(
                "\u{201c}Remind me {said}\u{201d} \u{2192} {meaning}"
            ))
            .key(format!("reminder-example-{key}"))
            .font(type_::BODY)
            .color(t.text)
            .wrap(),
        );
    }
    body = body
        .child(section(&format!("Said now, {}", words(now.timestamp(), &now)), t).child(examples));
    body = body.child(note(
        "reminder-settings-kept",
        &format!(
            "Changes apply at once, to reminders set from now on, and are kept in {} in Velvet \
             Hammer's data directory.",
            crate::reminders::SETTINGS_FILE
        ),
        t,
    ));
    let actions = row()
        .gap(space::S2)
        .child(button("reminder-settings-back", "Reminders", t, |s| {
            s.showing = Showing::Reminders
        }))
        .child(button("reminder-settings-close", "Back to chat", t, |s| {
            s.showing = Showing::Chat
        }));
    page(
        "reminder-settings-page",
        "Reminder settings",
        actions,
        body,
        t,
    )
}
