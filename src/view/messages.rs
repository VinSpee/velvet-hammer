//! When the captain's messages reach the chief of staff ([`crate::messages`]): the settings page,
//! the line over the message field while the chief of staff works, and the queued messages under
//! it.

use rune::tok::{Theme, radius, space, type_};
use rune::{El, col, row, text};

use super::{all_settings, button, choice, field, note, page, section, small_button, warning};
use crate::messages::{self, Delivery};
use crate::{App, Showing};

/// The setting: send right away, or queue until the chief of staff is done.
pub fn messages_page(s: &App, t: Theme, dark: bool) -> El {
    let delivery = *s.delivery.settings();
    let mut body = col().key("messages-body").gap(space::S6);
    if let Some(problem) = s.delivery.problem() {
        body = body.child(warning("messages-problem", problem, dark));
    }
    let mut modes = col().key("message-modes").role("radiogroup").gap(space::S2);
    for mode in Delivery::ALL {
        modes = modes.child(choice(
            &format!("message-mode-{}", mode.key()),
            mode.label(),
            mode == Delivery::RECOMMENDED,
            mode.explanation(),
            delivery == mode,
            t,
            move |s: &mut App| s.delivery.change(|d| *d = mode),
        ));
    }
    body = body
        .child(section("Messages you send while your chief of staff is working", t).child(modes));
    body = body.child(
        section("How your chief of staff takes in a message", t)
            .child(note(
                "messages-steps",
                "Claude Code reads a message that arrives during a turn at the turn's next step, \
                 after the tool call that is running, and carries on with it in mind. When the \
                 turn has no step left, the message starts the next turn instead. Either way, \
                 nothing it is running is stopped.",
                t,
            ))
            .child(note(
                "messages-order",
                "A sent message shows in the conversation at once, in the order you sent it, \
                 marked when it went out while your chief of staff was working. A queued message \
                 shows there when it goes out.",
                t,
            )),
    );
    body = body.child(
        section("Queued messages", t)
            .child(note(
                "messages-queue-kept",
                "Queued messages are kept in memory only. While Claude Code is not ready (not \
                 signed in, starting, or after it stopped), they wait and go out once it is. \
                 Quitting Velvet Hammer drops them, as it drops the conversation.",
                t,
            ))
            .child(note(
                "messages-switch",
                "Messages already queued when you switch to Send right away still go out first, \
                 in order, when your chief of staff is done.",
                t,
            )),
    );
    body = body.child(note(
        "messages-kept",
        &format!(
            "This setting is kept in {} in Velvet Hammer's data directory.",
            messages::SETTINGS_FILE
        ),
        t,
    ));
    let actions = row()
        .gap(space::S2)
        .child(all_settings("messages-all-settings", s, t))
        .child(button("messages-close", "Back to chat", t, |s| {
            s.showing = Showing::Chat
        }));
    page("messages-page", "Messages", actions, body, t)
}

/// How much of a queued message its row shows. The whole message goes out, and Edit shows it all.
const SHOWN_CHARS: usize = 280;

fn shortened(message: &str) -> String {
    match message.char_indices().nth(SHOWN_CHARS) {
        Some((at, _)) => format!("{}\u{2026}", &message[..at]),
        None => message.to_string(),
    }
}

/// The line over the message field while the chief of staff works.
pub fn working(s: &App, t: Theme) -> Option<El> {
    if !s.working {
        return None;
    }
    let words = match s.delivery.settings() {
        Delivery::Now => {
            "Your chief of staff is working. A message you send now reaches it at its next step."
        }
        Delivery::Queue => {
            "Your chief of staff is working. A message you send now waits below and goes out \
             when it is done."
        }
    };
    Some(note("working", words, t))
}

/// The messages waiting until the chief of staff is done, each with a way to edit or delete it.
pub fn queued(s: &App, t: Theme) -> Option<El> {
    if s.queue.is_empty() {
        return None;
    }
    let waiting = if s.working {
        "until your chief of staff is done"
    } else if s.editing.is_some() {
        "until you finish editing"
    } else {
        "until Claude Code is ready"
    };
    let mut list = col().key("queue").gap(space::S2).child(note(
        "queue-title",
        &format!("Queued {waiting}. They go out one at a time, in this order."),
        t,
    ));
    for (i, message) in s.queue.iter().enumerate() {
        let key = format!("queued-{i}");
        let item = match &s.editing {
            Some((at, edit)) if *at == i => row()
                .gap(space::S2)
                .center_y()
                .child(field(
                    &format!("{key}-field"),
                    edit,
                    "Message",
                    t,
                    |s: &mut App| &mut s.editing.as_mut().expect("being edited").1,
                ))
                .child(small_button(&format!("{key}-save"), "Save", t, |s| {
                    s.save_queued()
                }))
                .child(small_button(&format!("{key}-cancel"), "Cancel", t, |s| {
                    s.cancel_edit()
                })),
            _ => row()
                .gap(space::S2)
                .center_y()
                .child(
                    text(format!("{}. {}", i + 1, shortened(message)))
                        .key(format!("{key}-text"))
                        .font(type_::BODY)
                        .color(t.text)
                        .wrap()
                        .grow(1.0),
                )
                .child(small_button(&format!("{key}-edit"), "Edit", t, move |s| {
                    s.edit_queued(i)
                }))
                .child(small_button(
                    &format!("{key}-delete"),
                    "Delete",
                    t,
                    move |s| s.delete_queued(i),
                )),
        };
        list = list.child(
            item.key(key)
                .padding(space::S2)
                .radius(radius::MD)
                .fill(t.card)
                .border(1.0, t.card_border),
        );
    }
    Some(list)
}
