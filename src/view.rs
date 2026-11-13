//! The window: agents on the left, the chief-of-staff chat (or a peek at one subagent) in the
//! middle, and the two panels for the captain on the right. Every element a test or a headless
//! script reaches has a key.

use rune::tok::{self, Theme, radius, space, type_};
use rune::{Cx, El, ScrollSpec, col, el, row, text, text_input};

use crate::App;
use crate::model::{Action, Agent, Author, Item, ItemKind, Message};

pub fn view(s: &App, cx: &mut Cx<App>) -> El {
    let t = tok::theme(cx.system_dark());
    let main = match s.peek.and_then(|id| s.ws.agent(id)) {
        Some(agent) => peek(agent, t),
        None => chat(s, t),
    };
    row()
        .w_full()
        .h_full()
        .fill(t.bg)
        .child(agents(s, t))
        .child(main)
        .child(
            col()
                .key("panels")
                .w(320.0)
                .h_full()
                .min_h(0.0)
                .padding(space::S3)
                .gap(space::S3)
                .child(needs_input(s, t))
                .child(for_you(s, t)),
        )
}

fn agents(s: &App, t: Theme) -> El {
    let mut list = col()
        .key("agents")
        .role("list")
        .label("Agents")
        .w(200.0)
        .h_full()
        .px(space::S3)
        .py(space::S6)
        .gap(space::S1)
        .fill(t.card)
        .child(heading("Agents", t));
    for a in &s.ws.agents {
        let id = a.id;
        let selected = s.peek == Some(id);
        list = list.child(
            col()
                .key(format!("agent-{id}"))
                .role("button")
                .label(format!("Peek at {}", a.name))
                .focusable(true)
                .padding(space::S2)
                .gap(2.0)
                .radius(radius::SM)
                .fill(if selected { t.control_off } else { t.card })
                .on_press(move |s: &mut App| s.peek = Some(id))
                .child(text(a.name.clone()).font(type_::TITLE).color(t.text))
                .child(
                    text(format!("{} · {}", a.status.label(), a.task))
                        .font(type_::LABEL)
                        .color(t.text_secondary)
                        .wrap(),
                ),
        );
    }
    list
}

fn chat(s: &App, t: Theme) -> El {
    let input = text_input(&s.draft, type_::BODY)
        .key("chat-input")
        .single_line()
        .grow(1.0)
        .color(t.text)
        .placeholder("Message your chief of staff", t.text_secondary)
        .on_edit(|s: &mut App| &mut s.draft)
        .on_submit(|s: &mut App, _| s.send());
    let composer = row()
        .key("composer")
        .gap(space::S2)
        .center_y()
        .child(
            row()
                .grow(1.0)
                .h(36.0)
                .px(space::S3)
                .center_y()
                .radius(radius::MD)
                .fill(t.card)
                .border(1.0, t.card_border)
                .child(input),
        )
        .child(button("send", "Send", t, |s| s.send()));
    col()
        .key("chat")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .padding(space::S6)
        .gap(space::S3)
        .child(heading("Chief of staff", t))
        .child(transcript("messages", &s.ws.chat, t))
        .child(composer)
}

fn peek(agent: &Agent, t: Theme) -> El {
    let header = row()
        .gap(space::S2)
        .center_y()
        .child(
            col()
                .grow(1.0)
                .gap(2.0)
                .child(heading(&format!("Peeking at {}", agent.name), t))
                .child(
                    text(agent.task.clone())
                        .font(type_::LABEL)
                        .color(t.text_secondary),
                ),
        )
        .child(button("peek-close", "Back to chat", t, |s| s.peek = None));
    col()
        .key("peek")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .padding(space::S6)
        .gap(space::S3)
        .child(header)
        .child(transcript("peek-messages", &agent.transcript, t))
}

fn transcript(key: &str, messages: &[Message], t: Theme) -> El {
    let mut list = col().gap(space::S2);
    for (i, m) in messages.iter().enumerate() {
        let who = match m.author {
            Author::Captain => "You",
            Author::ChiefOfStaff => "Chief of staff",
            Author::Subagent => "Subagent",
        };
        list = list.child(
            col()
                .key(format!("msg-{i}"))
                .padding(space::S3)
                .gap(space::S1)
                .radius(radius::MD)
                .fill(if m.author == Author::Captain {
                    t.control_off
                } else {
                    t.card
                })
                .child(text(who).font(type_::SECTION).color(t.text_secondary))
                .child(text(m.text.clone()).font(type_::BODY).color(t.text).wrap()),
        );
    }
    col()
        .key(key)
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec { stick_bottom: true })
        .child(list)
}

fn needs_input(s: &App, t: Theme) -> El {
    let rows = s.ws.needs_input().map(|item| {
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
        card(&format!("input-{}", item.id), item, t).child(answers)
    });
    panel(
        "needs-input",
        "Needs your input",
        "Nothing is waiting on you.",
        rows.collect(),
        t,
    )
}

fn for_you(s: &App, t: Theme) -> El {
    let rows = s.ws.for_you().map(|item| {
        let id = item.id;
        card(&format!("task-{id}"), item, t).child(row().child(button(
            &format!("done-{id}"),
            "Done",
            t,
            move |s| s.act(Action::Done { item: id }),
        )))
    });
    panel(
        "for-you",
        "For you to do",
        "Nothing for you to do.",
        rows.collect(),
        t,
    )
}

fn panel(key: &str, title: &str, empty: &str, rows: Vec<El>, t: Theme) -> El {
    let mut body = col().gap(space::S2);
    if rows.is_empty() {
        body = body.child(
            text(empty)
                .key(format!("{key}-empty"))
                .font(type_::BODY)
                .color(t.text_secondary),
        );
    }
    for r in rows {
        body = body.child(r);
    }
    col()
        .key(key)
        .role("region")
        .label(title)
        .grow(1.0)
        .min_h(0.0)
        .padding(space::S3)
        .gap(space::S2)
        .radius(radius::LG)
        .fill(t.card)
        .border(1.0, t.card_border)
        .child(heading(title, t))
        .child(
            col()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(body),
        )
}

fn card(key: &str, item: &Item, t: Theme) -> El {
    col()
        .key(key)
        .padding(space::S3)
        .gap(space::S2)
        .radius(radius::MD)
        .fill(t.bg)
        .child(
            text(item.title.clone())
                .font(type_::TITLE)
                .color(t.text)
                .wrap(),
        )
        .child(
            text(item.detail.clone())
                .font(type_::LABEL)
                .color(t.text_secondary)
                .wrap(),
        )
}

fn heading(title: &str, t: Theme) -> El {
    text(title.to_string()).font(type_::TITLE).color(t.text)
}

fn button(key: &str, label: &str, t: Theme, f: impl Fn(&mut App) + 'static) -> El {
    el().key(key)
        .role("button")
        .label(label.to_string())
        .focusable(true)
        .h(28.0)
        .px(space::S3)
        .center()
        .radius(radius::SM)
        .fill(tok::accent(0, true))
        .on_press(f)
        .child(text(label.to_string()).font(type_::LABEL).color(t.knob))
}
