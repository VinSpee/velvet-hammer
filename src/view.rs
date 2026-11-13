//! The window: agents on the left, the chief-of-staff chat (or a peek at one subagent, or the
//! Claude sign-in while the chat can't run) in the middle, and the two panels for the captain on
//! the right. Every element a test or a headless script reaches has a key.

use crate::{App, Showing};
use rune::tok::{self, Theme, radius, space, type_};
use rune::{Cx, El, ScrollSpec, col, el, row, text, text_input};

use crate::firstmate::{Checkout, Distro, Firstmate, Standing, Work};
use crate::model::{Action, Agent, Author, Item, ItemKind, Message, Plan, Status};

pub fn view(s: &App, cx: &mut Cx<App>) -> El {
    let t = tok::theme(cx.system_dark());
    let peeked = match s.showing {
        Showing::Peek(id) => s.ws.agent(id),
        _ => None,
    };
    let main = match (s.showing, peeked, &s.status) {
        (Showing::Firstmate, ..) => firstmate_page(&s.firstmate, t, cx.system_dark()),
        (_, Some(agent), _) => peek(agent, t),
        (_, None, Status::Ready(plan)) => chat(s, plan, t),
        (_, None, status) => sign_in(status, t, cx.system_dark()),
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
        let selected = s.showing == Showing::Peek(id);
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
                .on_press(move |s: &mut App| s.showing = Showing::Peek(id))
                .child(text(a.name.clone()).font(type_::TITLE).color(t.text))
                .child(
                    text(format!("{} · {}", a.status.label(), a.task))
                        .font(type_::LABEL)
                        .color(t.text_secondary)
                        .wrap(),
                ),
        );
    }
    list.child(el().grow(1.0)).child(
        col()
            .key("firstmate")
            .role("button")
            .label("Firstmate")
            .focusable(true)
            .padding(space::S2)
            .gap(2.0)
            .radius(radius::SM)
            .fill(if s.showing == Showing::Firstmate {
                t.control_off
            } else {
                t.card
            })
            .on_press(|s: &mut App| s.showing = Showing::Firstmate)
            .child(text("Firstmate").font(type_::TITLE).color(t.text))
            .child(note(
                "firstmate-summary",
                firstmate_summary(&s.firstmate),
                t,
            )),
    )
}

/// The managed clone in a few words, for the card under the agents.
fn firstmate_summary(fm: &Firstmate) -> &'static str {
    match (fm.work(), fm.distro()) {
        (Some(Work::Installing), _) => "installing\u{2026}",
        (Some(Work::Checking), _) => "checking for updates\u{2026}",
        (Some(Work::Updating), _) => "updating\u{2026}",
        (None, Distro::Missing) => "not installed",
        (None, Distro::Installed(c)) => match c.standing() {
            Standing::UpToDate => "up to date",
            Standing::UpdateAvailable => "update available",
            Standing::Modified => "has your changes",
            Standing::Blocked => "update blocked",
            Standing::NotTracking => "not tracking upstream",
        },
    }
}

/// The managed firstmate clone: how it stands against upstream, and installing and updating it.
fn firstmate_page(fm: &Firstmate, t: Theme, dark: bool) -> El {
    let path = fm.clone_dir().display().to_string();
    let mut body = col().key("firstmate-status").gap(space::S3).max_w(560.0);
    let mut buttons = row().gap(space::S2).flex_wrap();
    let (title, lede) = match fm.distro() {
        Distro::Missing if fm.work().is_some() => (
            "Installing firstmate",
            format!("Cloning {} into {path}.", fm.url()),
        ),
        Distro::Missing => (
            "Firstmate is not installed",
            format!(
                "Firstmate is the instructions, skills and scripts your chief of staff runs on. \
                 Velvet Hammer keeps its own copy, a git clone of {} at {path}, and touches no \
                 other firstmate you have.",
                fm.url()
            ),
        ),
        Distro::Installed(c) => standing_words(c),
    };
    body = body
        .child(
            text(title)
                .key("firstmate-title")
                .font(type_::TITLE)
                .color(t.text),
        )
        .child(
            text(lede)
                .key("firstmate-lede")
                .font(type_::BODY)
                .color(t.text_secondary)
                .wrap(),
        );
    if let Distro::Installed(c) = fm.distro() {
        if !c.conflicts.is_empty() {
            body = body.child(warning(
                "firstmate-conflicts",
                &c.conflicts.join("\n"),
                dark,
            ));
        }
        if !c.edited.is_empty() {
            body = body.child(note(
                "firstmate-edited",
                &format!("Edited and not committed: {}", c.edited.join(", ")),
                t,
            ));
        }
        let tracking = match &c.upstream {
            Some(upstream) => format!("tracking {upstream}"),
            None => "not tracking a branch".into(),
        };
        body = body.child(note(
            "firstmate-version",
            &format!("Commit {} \u{b7} {tracking} \u{b7} {path}", c.head),
            t,
        ));
    }
    if let Some(problem) = fm.problem() {
        body = body.child(warning("firstmate-problem", problem, dark));
    }
    match (fm.work(), fm.distro()) {
        (Some(work), _) => {
            let words = match work {
                Work::Installing => "This takes a moment\u{2026}",
                Work::Checking => "Checking for updates\u{2026}",
                Work::Updating => "Updating\u{2026}",
            };
            body = body.child(note("firstmate-progress", words, t));
        }
        (None, Distro::Missing) => {
            buttons = buttons.child(button("firstmate-install", "Install firstmate", t, |s| {
                s.firstmate.install()
            }));
        }
        (None, Distro::Installed(c)) => {
            if c.standing() == Standing::UpdateAvailable {
                let label = if c.edited.is_empty() {
                    "Update"
                } else {
                    "Save my changes and update"
                };
                buttons = buttons.child(button("firstmate-update", label, t, |s| {
                    s.firstmate.update()
                }));
            }
            buttons = buttons.child(button("firstmate-check", "Check for updates", t, |s| {
                s.firstmate.check()
            }));
        }
    }
    body = body.child(
        buttons.child(button("firstmate-close", "Back to chat", t, |s| {
            s.showing = Showing::Chat
        })),
    );
    if matches!(fm.distro(), Distro::Installed(_)) {
        body = body.child(note(
            "firstmate-help",
            "This clone is yours to change: edit AGENTS.md, the skills under .agents/skills or \
             the scripts under bin, and updates keep your changes. Velvet Hammer never discards \
             an edit and never pushes.",
            t,
        ));
    }
    col()
        .key("firstmate-page")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .padding(space::S6)
        .gap(space::S6)
        .child(heading("Firstmate", t))
        .child(
            col()
                .padding(space::S6)
                .radius(radius::LG)
                .fill(t.card)
                .border(1.0, t.card_border)
                .child(body),
        )
}

/// The title and explanation for an installed clone.
fn standing_words(c: &Checkout) -> (&'static str, String) {
    let upstream = c.upstream.as_deref().unwrap_or("upstream");
    match c.standing() {
        Standing::UpToDate => (
            "Firstmate is up to date",
            format!("The clone matches {upstream}."),
        ),
        Standing::UpdateAvailable => (
            "A firstmate update is available",
            format!(
                "{upstream} has {}.{}",
                count(c.behind, "new commit"),
                if c.modified() {
                    " Your changes are kept: the update saves any edited files as a local \
                     commit and merges upstream into it."
                } else {
                    ""
                }
            ),
        ),
        Standing::Modified => (
            "Firstmate has your changes",
            format!(
                "Nothing new in {upstream}. The clone has {} and {}, and updates keep them.",
                count(c.ahead, "local commit"),
                count(c.edited.len() as u32, "edited file"),
            ),
        ),
        Standing::Blocked if c.merging => (
            "The firstmate update is blocked",
            "A merge is unfinished in the clone. Finish it in a terminal (resolve the \
             conflicts, then `git commit`) and choose Check for updates."
                .into(),
        ),
        Standing::Blocked => (
            "The firstmate update is blocked",
            format!(
                "Your changes and {upstream} both change the files below, and Velvet Hammer \
                 will not choose between them. Nothing has been changed. In a terminal in the \
                 clone, commit your changes, run `git merge {upstream}`, resolve the conflicts \
                 and commit. Then choose Check for updates."
            ),
        ),
        Standing::NotTracking => (
            "Firstmate is not on its upstream branch",
            "The clone is not on a branch that tracks upstream, so there is nothing to update \
             against. Run `git switch main` in the clone to get updates again."
                .into(),
        ),
    }
}

fn count(n: u32, what: &str) -> String {
    format!("{n} {what}{}", if n == 1 { "" } else { "s" })
}

fn chat(s: &App, plan: &Plan, t: Theme) -> El {
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
        .child(
            col()
                .gap(2.0)
                .child(heading("Chief of staff", t))
                .child(note("health", &health(plan), t)),
        )
        .child(transcript("messages", &s.ws.chat, t))
        .child(composer)
}

/// One line on what runs the chief of staff and what pays for it.
fn health(plan: &Plan) -> String {
    match plan {
        Plan::Subscription(kind) => {
            format!(
                "Claude Code \u{b7} signed in with your Claude {} plan",
                capitalized(kind)
            )
        }
        Plan::Simulated => "Simulated chief of staff \u{b7} no Claude Code".into(),
    }
}

fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// What the middle column shows while the chat can't run: the sign-in, its progress, or why
/// Claude Code isn't usable. Claude Code does the signing in; this only starts it and reports.
fn sign_in(status: &Status, t: Theme, dark: bool) -> El {
    let mut body = col().key("sign-in").gap(space::S3).max_w(520.0);
    let mut buttons = row().gap(space::S2);
    let (title, lede) = match status {
        Status::Checking => (
            "Checking Claude Code",
            "Asking `claude auth status` whether Claude Code is signed in.",
        ),
        Status::SignedOut { .. } => (
            "Sign in with Claude",
            "Your chief of staff runs on your own Claude Code, so its messages count against your \
             Claude subscription. Claude Code signs you in and keeps the sign-in; Velvet Hammer \
             never sees it.",
        ),
        Status::NotSubscription { .. } => (
            "Sign in with a Claude subscription",
            "Your chief of staff only runs on a Claude subscription (Pro or Max). Claude Code is \
             signed in another way, so the chat is locked. Run `claude auth login --claudeai`, or \
             choose Sign in with Claude, to switch.",
        ),
        Status::SigningIn { .. } => (
            "Finish signing in in your browser",
            "Claude Code opened its sign-in page. Approve it there and this window returns to the \
             chat.",
        ),
        Status::Expired => (
            "Your Claude sign-in has expired",
            "Claude Code still has a sign-in, but Anthropic did not accept it for your last \
             message. Sign in again to keep going.",
        ),
        Status::CliMissing => (
            "Claude Code isn't installed",
            "Velvet Hammer needs the `claude` command on its PATH. Install Claude Code from \
             https://code.claude.com, then choose Check again.",
        ),
        Status::Failed(_) => (
            "Claude Code isn't working",
            "Claude Code is installed, but asking it whether it is signed in failed:",
        ),
        Status::Ready(_) => unreachable!("the chat shows instead"),
    };
    body = body
        .child(
            text(title)
                .key("sign-in-title")
                .font(type_::TITLE)
                .color(t.text),
        )
        .child(
            text(lede)
                .key("sign-in-lede")
                .font(type_::BODY)
                .color(t.text_secondary)
                .wrap(),
        );
    match status {
        Status::SignedOut { problem } => {
            if let Some(problem) = problem {
                body = body.child(warning("sign-in-problem", problem, dark));
            }
            buttons = buttons
                .child(button("sign-in-button", "Sign in with Claude", t, |s| {
                    s.sign_in()
                }))
                .child(button("recheck", "Check again", t, |s| s.recheck()));
            body = body.child(buttons).child(note(
                "sign-in-help",
                "This runs `claude auth login`, which opens your browser. You can also run it in \
                 a terminal yourself, then choose Check again.",
                t,
            ));
        }
        Status::SigningIn { url } => {
            body = body.child(note(
                "sign-in-progress",
                if url.is_some() {
                    "Waiting for you to approve the sign-in in your browser\u{2026}"
                } else {
                    "Starting Claude Code's sign-in\u{2026}"
                },
                t,
            ));
            if let Some(url) = url {
                body = body.child(
                    text(url.clone())
                        .key("sign-in-url")
                        .font(type_::LABEL)
                        .color(t.text_secondary)
                        .wrap_anywhere(),
                );
                buttons = buttons.child(button("open-sign-in", "Open the sign-in page", t, |s| {
                    s.open_sign_in_page()
                }));
            }
            body = body.child(buttons.child(button("cancel-sign-in", "Cancel", t, |s| {
                s.cancel_sign_in()
            })));
        }
        Status::NotSubscription { method } => {
            body = body
                .child(warning(
                    "status-detail",
                    &format!("Claude Code reports its sign-in method as {method}."),
                    dark,
                ))
                .child(
                    buttons
                        .child(button("sign-in-button", "Sign in with Claude", t, |s| {
                            s.sign_in()
                        }))
                        .child(button("recheck", "Check again", t, |s| s.recheck())),
                );
        }
        Status::Expired => {
            body = body.child(
                buttons
                    .child(button("sign-in-button", "Sign in again", t, |s| {
                        s.sign_in()
                    }))
                    .child(button("recheck", "Check again", t, |s| s.recheck())),
            );
        }
        Status::Failed(detail) => {
            body = body
                .child(warning("status-detail", detail, dark))
                .child(buttons.child(button("recheck", "Check again", t, |s| s.recheck())));
        }
        Status::CliMissing => {
            body = body.child(buttons.child(button("recheck", "Check again", t, |s| s.recheck())));
        }
        Status::Checking | Status::Ready(_) => {}
    }
    col()
        .key("sign-in-screen")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .padding(space::S6)
        .gap(space::S6)
        .child(heading("Chief of staff", t))
        .child(
            col()
                .padding(space::S6)
                .radius(radius::LG)
                .fill(t.card)
                .border(1.0, t.card_border)
                .child(body),
        )
}

fn note(key: &str, words: &str, t: Theme) -> El {
    text(words.to_string())
        .key(key)
        .font(type_::LABEL)
        .color(t.text_secondary)
        .wrap()
}

/// A problem in the agent layer's own words.
fn warning(key: &str, words: &str, dark: bool) -> El {
    text(words.to_string())
        .key(key)
        .font(type_::BODY)
        .color(tok::accent(3, dark))
        .wrap()
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
        .child(button("peek-close", "Back to chat", t, |s| {
            s.showing = Showing::Chat
        }));
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
            Author::Problem => "Claude Code reported a problem",
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

/// The tallest the detail of a card grows before it scrolls.
const DETAIL_MAX_H: f32 = 240.0;

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
            col()
                .key(format!("{key}-detail"))
                .max_h(DETAIL_MAX_H)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(
                    text(item.detail.clone())
                        .font(type_::LABEL)
                        .color(t.text_secondary)
                        .wrap()
                        .selectable(true),
                ),
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
