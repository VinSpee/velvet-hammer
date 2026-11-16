//! The fleet panel on the right of the window (see [`crate::fleet`]): its header with when the
//! snapshot was read, Refresh, Ahoy and Bearings; the Waiting on you strip, pinned at the top,
//! with every card that needs the captain; then Underway, Recently landed and Charted next.
//! Every state is said in words, never by colour alone.

use rune::tok::{self, Theme, radius, space, type_};
use rune::{El, ScrollSpec, col, el, row, text};

use super::{button, heading, note, remind, small_button, warning};
use crate::bearings::{Crew, Gate, Landed};
use crate::fleet::{AHOY, BEARINGS, Section};
use crate::model::{Action, Item, ItemKind};
use crate::reminders::words;
use crate::{App, Showing};

pub fn panel(s: &App, t: Theme, dark: bool) -> El {
    let mut panel = col()
        .key("fleet")
        .role("region")
        .label("Fleet")
        .w(320.0)
        .h_full()
        .min_h(0.0)
        .px(space::S3)
        .pt(space::S6)
        .pb(space::S3)
        .gap(space::S3);
    if !s.has_fleet() {
        // A chief of staff that is not firstmate has no fleet; only what waits on the captain.
        return panel.child(waiting(s, t, dark));
    }
    panel = panel.child(header(s, t, dark)).child(waiting(s, t, dark));
    {
        let mut sections = col().gap(space::S3);
        for section in Section::ALL {
            sections = sections.child(fleet_section(s, section, t, dark));
        }
        panel = panel.child(
            col()
                .key("fleet-sections")
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(sections),
        );
    }
    panel
}

/// The panel's title, when the snapshot shown was read, and its buttons.
fn header(s: &App, t: Theme, dark: bool) -> El {
    let mut header = col()
        .gap(space::S2)
        .child(
            row()
                .gap(space::S2)
                .center_y()
                .child(heading("Fleet", t).grow(1.0))
                .child(
                    small_button("fleet-refresh", "Refresh", t, |s| s.refresh_fleet())
                        .label("Refresh the fleet status"),
                ),
        )
        .child(note("fleet-updated", &updated(s), t));
    if let Some(problem) = &s.captain_problem {
        header = header.child(warning(
            "captain-problem",
            &format!("Could not read firstmate's records: {problem}"),
            dark,
        ));
    }
    let ready = s.chat_ready();
    header.child(
        row()
            .gap(space::S2)
            .flex_wrap()
            .child(ask_button(
                "fleet-ahoy",
                "Ahoy",
                "Catch me up (/ahoy)",
                AHOY,
                ready,
                t,
            ))
            .child(ask_button(
                "fleet-bearings",
                "Bearings",
                "Ask for bearings (/bearings)",
                BEARINGS,
                ready,
                t,
            )),
    )
}

/// How old the snapshot shown is, such as "Updated Today, 2:30 PM".
fn updated(s: &App) -> String {
    let Some(at) = s.fleet.at else {
        return if s.captain_problem.is_some() {
            "No snapshot yet".into()
        } else {
            "Reading firstmate's records\u{2026}".into()
        };
    };
    let when = words(at, &s.reminders.now());
    let read = if s.fleet.kept {
        format!("Saved {when}, before this launch")
    } else {
        format!("Updated {when}")
    };
    if s.fleet.refreshing {
        format!("{read} \u{b7} refreshing\u{2026}")
    } else {
        read
    }
}

/// A button that sends `command` to the chief of staff, as the captain could type it.
fn ask_button(
    key: &str,
    shown: &str,
    label: &str,
    command: &'static str,
    ready: bool,
    t: Theme,
) -> El {
    small_button(key, shown, t, move |s| s.ask(command))
        .label(if ready {
            label.to_string()
        } else {
            format!("{label}: your chief of staff cannot chat yet")
        })
        .opacity(if ready { 1.0 } else { 0.5 })
}

/// Everything that waits on the captain: tool approvals, captain holds, merges waiting for
/// approval, due reminders and the steps the chief of staff left. It stays at the top, and only
/// its own list scrolls.
fn waiting(s: &App, t: Theme, dark: bool) -> El {
    let mut rows: Vec<El> =
        s.ws.needs_input()
            .map(|item| approval(s, item, t))
            .collect();
    rows.extend(expired(s, t, dark));
    rows.extend(holds(s, t));
    rows.extend(merges(s, t));
    rows.extend(remind::panel_rows(s, t, dark));
    rows.extend(s.ws.for_you().map(|item| {
        let id = item.id;
        card(&format!("task-{id}"), item, t).child(row().child(button(
            &format!("done-{id}"),
            "Done",
            t,
            move |s| s.act(Action::Done { item: id }),
        )))
    }));
    let count = rows.len();
    // Reminders still to come wait on the Reminders page, which this opens.
    let all_reminders = (!s.reminders.list().is_empty()).then(|| {
        small_button("waiting-reminders", "All reminders", t, |s| {
            s.showing = Showing::Reminders
        })
    });
    if count == 0 {
        // Nothing waits: the whole strip is one line, the count saying so.
        let mut line = row()
            .gap(space::S2)
            .center_y()
            .child(heading("Waiting on you (0)", t).key("waiting-empty"));
        if let Some(button) = all_reminders {
            line = line.child(el().grow(1.0)).child(button);
        }
        return col()
            .key("waiting")
            .role("region")
            .label("Waiting on you, 0")
            .padding(space::S3)
            .radius(radius::LG)
            .fill(t.card)
            .border(1.0, t.card_border)
            .child(line);
    }
    let mut body = col().gap(space::S2);
    for r in rows {
        body = body.child(r);
    }
    if let Some(button) = all_reminders {
        body = body.child(row().child(button));
    }
    // Beside the fleet it stops growing, so the fleet stays in view; alone it fills the column.
    // Either way only its list scrolls.
    let alone = !s.has_fleet();
    let list = col().min_h(0.0).scroll(ScrollSpec {
        stick_bottom: false,
    });
    let strip = col()
        .key("waiting")
        .role("region")
        .label(format!("Waiting on you, {count}"))
        .padding(space::S3)
        .gap(space::S2)
        .radius(radius::LG)
        .fill(t.card)
        .border(2.0, tok::accent(3, dark))
        .child(heading(&format!("Waiting on you ({count})"), t));
    if alone {
        strip.grow(1.0).min_h(0.0).child(list.grow(1.0).child(body))
    } else {
        strip.max_h(WAITING_MAX_H).child(list.child(body))
    }
}

/// The tallest the Waiting on you strip grows before its list scrolls, so the fleet below it
/// stays in view.
const WAITING_MAX_H: f32 = 380.0;

/// A question from the chief of staff's hook, such as whether a tool may run.
fn approval(s: &App, item: &Item, t: Theme) -> El {
    let ItemKind::NeedsInput { options } = &item.kind else {
        unreachable!()
    };
    let mut answers = row().gap(space::S2).flex_wrap();
    for (n, choice) in options.iter().enumerate() {
        let (id, choice) = (item.id, choice.clone());
        answers = answers.child(button(
            &format!("answer-{id}-{n}"),
            &choice.clone(),
            t,
            move |s| {
                s.act(Action::Answer {
                    item: id,
                    choice: choice.clone(),
                })
            },
        ));
    }
    let key = format!("input-{}", item.id);
    spotlit(s, card(&key, item, t), &key).child(answers)
}

/// Questions nobody answered in time stay, so the captain sees what was missed.
fn expired(s: &App, t: Theme, dark: bool) -> Vec<El> {
    s.ws.expired()
        .map(|item| {
            let ItemKind::Expired { why } = &item.kind else {
                unreachable!()
            };
            let id = item.id;
            words_card(
                &format!("expired-{id}"),
                &format!("Expired: {}", item.title),
                &format!("{why}\n\n{}", item.detail),
                t,
            )
            .border(1.0, tok::accent(3, dark))
            .child(
                row()
                    .gap(space::S2)
                    .center_y()
                    .child(button(
                        &format!("ask-again-{id}"),
                        "Ask again",
                        t,
                        move |s| s.ask_again(id),
                    ))
                    .child(small_button(
                        &format!("dismiss-{id}"),
                        "Dismiss",
                        t,
                        move |s| s.dismiss(id),
                    )),
            )
        })
        .collect()
}

/// Outlines the card keyed `key` while a notification click has it highlighted.
fn spotlit(s: &App, card: El, key: &str) -> El {
    if s.spotlit(key) {
        card.border(2.0, tok::accent(0, true))
    } else {
        card
    }
}

/// The tasks firstmate holds for the captain. A second mate's hold is answered to that mate,
/// which owns it.
fn holds(s: &App, t: Theme) -> Vec<El> {
    let mut rows = Vec::new();
    for hold in &s.captain.holds {
        let key = hold.key.clone();
        if let Some(owner) = hold.owner.clone() {
            let answering = s.mates.answering == Some((owner.clone(), key.clone()));
            let card = format!("hold-{owner}-{key}");
            rows.push(
                spotlit(
                    s,
                    words_card(
                        &card,
                        &format!("Decision \u{b7} {key} \u{b7} from second mate {owner}"),
                        &hold.question,
                        t,
                    ),
                    &card,
                )
                .child(row().child(button(
                    &format!("answer-{owner}-{key}"),
                    if answering {
                        "Answering on its page"
                    } else {
                        "Answer"
                    },
                    t,
                    move |s| s.answer_mate_hold(&owner, &key),
                ))),
            );
            continue;
        }
        let answering = s.answering.as_deref() == Some(&hold.key);
        let card = format!("hold-{key}");
        rows.push(
            spotlit(
                s,
                words_card(&card, &format!("Decision \u{b7} {key}"), &hold.question, t),
                &card,
            )
            .child(row().child(button(
                &format!("answer-{key}"),
                if answering {
                    "Answering below"
                } else {
                    "Answer"
                },
                t,
                move |s| {
                    s.answering = Some(key.clone());
                    s.showing = Showing::Chat;
                },
            ))),
        );
    }
    rows
}

/// Pull requests whose checks are green and which wait for the captain's merge approval.
fn merges(s: &App, t: Theme) -> Vec<El> {
    s.captain
        .merges
        .iter()
        .map(|merge| {
            let (task, url) = (merge.task.clone(), merge.url.clone());
            let card = format!("merge-{task}");
            spotlit(
                s,
                words_card(
                    &card,
                    &format!("Approve the merge for {task}?"),
                    &format!("{}\n{}", merge.url, merge.reason),
                    t,
                ),
                &card,
            )
            .child(
                row()
                    .gap(space::S2)
                    .flex_wrap()
                    .child(open_button(&format!("open-merge-{task}"), &merge.url, t))
                    .child(button(&format!("approve-{task}"), "Approve", t, move |s| {
                        s.approve_merge(&task, &url)
                    })),
            )
        })
        .collect()
}

/// A section of the fleet, with its count, open or closed by its heading.
fn fleet_section(s: &App, section: Section, t: Theme, dark: bool) -> El {
    let key = section.key();
    let open = s.fleet.is_open(section);
    let rows: Vec<El> = match section {
        Section::Underway => s
            .captain
            .crew
            .iter()
            .map(|c| underway(s, c, t, dark))
            .collect(),
        Section::Landed => s.captain.landed.iter().map(|l| landed(l, t)).collect(),
        Section::Charted => s.captain.gates.iter().map(|g| charted(g, t)).collect(),
    };
    let count = rows.len();
    let title = format!("{} ({count})", section.title());
    let toggle = row()
        .key(format!("fleet-{key}-toggle"))
        .role("button")
        .label(format!(
            "{}, {count}, {}",
            section.title(),
            if open { "open" } else { "closed" }
        ))
        .expanded(open)
        .focusable(true)
        .gap(space::S2)
        .center_y()
        .on_press(move |s: &mut App| s.fleet.toggle(section))
        .child(
            text(if open { "\u{25be}" } else { "\u{25b8}" })
                .font(type_::TITLE)
                .color(t.text_secondary),
        )
        .child(heading(&title, t));
    let mut body = col()
        .key(format!("fleet-{key}"))
        .role("region")
        .label(title.clone())
        .gap(space::S2)
        .child(toggle);
    if open {
        if rows.is_empty() {
            body = body.child(note(&format!("fleet-{key}-empty"), empty(section), t));
        }
        for r in rows {
            body = body.child(r);
        }
    }
    body
}

fn empty(section: Section) -> &'static str {
    match section {
        Section::Underway => "No task is in flight.",
        Section::Landed => "Nothing has landed lately.",
        Section::Charted => "Nothing is charted next.",
    }
}

/// A task in flight; pressing it shows its terminal. Its state is the first word of its line.
fn underway(s: &App, crew: &Crew, t: Theme, dark: bool) -> El {
    let id = crew.id.clone();
    let selected = s.showing == Showing::Crew(id.clone());
    let stuck = matches!(crew.state.as_str(), "blocked" | "failed" | "needs-decision");
    col()
        .key(format!("crew-{id}"))
        .role("button")
        .label(format!(
            "{}, {}. Peek at its terminal",
            crew.name, crew.state
        ))
        .focusable(true)
        .padding(space::S2)
        .gap(2.0)
        .radius(radius::SM)
        .fill(if selected { t.control_off } else { t.bg })
        .on_press(move |s: &mut App| s.showing = Showing::Crew(id.clone()))
        .child(
            text(crew.name.clone())
                .font(type_::TITLE)
                .color(t.text)
                .wrap(),
        )
        .child(
            text(format!("{} \u{b7} {}", crew.state, crew.doing))
                .font(type_::LABEL)
                .color(if stuck {
                    tok::accent(3, dark)
                } else {
                    t.text_secondary
                })
                .wrap(),
        )
        .children(
            crew.runs
                .as_ref()
                .map(|runs| note(&format!("crew-{}-runs", crew.id), runs, t).wrap_anywhere()),
        )
}

/// Work that finished, with its pull request or report.
fn landed(l: &Landed, t: Theme) -> El {
    let key = format!("landed-{}", l.id);
    let mut line = l.id.clone();
    if let Some(owner) = &l.owner {
        line.push_str(&format!(" \u{b7} second mate {owner}"));
    }
    let mut card = col()
        .key(key.clone())
        .padding(space::S2)
        .gap(2.0)
        .radius(radius::SM)
        .fill(t.bg)
        .child(text(l.what.clone()).font(type_::TITLE).color(t.text).wrap())
        .child(note(&format!("{key}-id"), &line, t));
    match &l.artifact {
        Some(link) if is_web(link) => {
            card = card.child(row().child(open_button(&format!("{key}-open"), link, t)));
        }
        Some(path) => {
            card = card.child(
                note(&format!("{key}-path"), path, t)
                    .wrap_anywhere()
                    .selectable(true),
            );
        }
        None => {}
    }
    card
}

/// Work charted next, and what it waits for.
fn charted(g: &Gate, t: Theme) -> El {
    let key = format!("gate-{}", g.id);
    let mut why = Vec::new();
    if !g.blocked_by.is_empty() {
        why.push(format!("waits for {}", g.blocked_by));
    }
    if !g.reason.is_empty() {
        why.push(g.reason.clone());
    }
    if let Some(owner) = &g.owner {
        why.push(format!("second mate {owner}"));
    }
    let line = std::iter::once(g.id.clone())
        .chain(why)
        .collect::<Vec<_>>()
        .join(" \u{b7} ");
    col()
        .key(key.clone())
        .padding(space::S2)
        .gap(2.0)
        .radius(radius::SM)
        .fill(t.bg)
        .child(
            text(g.title.clone())
                .font(type_::TITLE)
                .color(t.text)
                .wrap(),
        )
        .child(note(&format!("{key}-why"), &line, t).wrap_anywhere())
}

fn is_web(link: &str) -> bool {
    link.starts_with("https://") || link.starts_with("http://")
}

/// Opens `url` in the browser: "Open pull request" for a pull request's address.
fn open_button(key: &str, url: &str, t: Theme) -> El {
    let shown = if url.contains("/pull/") {
        "Open pull request"
    } else {
        "Open link"
    };
    let url = url.to_string();
    el().key(key)
        .role("link")
        .label(format!("{shown}: {url}"))
        .focusable(true)
        .h(24.0)
        .px(space::S2)
        .center()
        .radius(radius::SM)
        .fill(t.control_off)
        .on_press_tx(move |s: &mut App, tx: &mut rune::Tx| s.open_link(&url, tx))
        .child(text(shown).font(type_::LABEL).color(t.text))
}

/// The tallest the detail of a card grows before it scrolls.
const DETAIL_MAX_H: f32 = 240.0;

fn card(key: &str, item: &Item, t: Theme) -> El {
    words_card(key, &item.title, &item.detail, t)
}

fn words_card(key: &str, title: &str, detail: &str, t: Theme) -> El {
    col()
        .key(key)
        .padding(space::S3)
        .gap(space::S2)
        .radius(radius::MD)
        .fill(t.bg)
        .child(
            text(title.to_string())
                .font(type_::TITLE)
                .color(t.text)
                .wrap(),
        )
        .child(
            col()
                .key(format!("{key}-detail"))
                .max_h(DETAIL_MAX_H)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(
                    text(detail.to_string())
                        .font(type_::LABEL)
                        .color(t.text_secondary)
                        .wrap()
                        .selectable(true),
                ),
        )
}
