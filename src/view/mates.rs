//! Firstmate's second mates: the list in the agents column, each mate's page, and the guided
//! flow that adds one (see [`crate::secondmates`]).

use rune::tok::{self, Theme, radius, space, type_};
use rune::{El, TextEdit, col, row, text, text_input};

use super::{button, field, heading, markdown, note, page, section, small_button, warning};
use crate::secondmates::{self, AddMate, Op, Said, Session, Step};
use crate::{App, Showing};

/// The mates in the registry, then any the bearings snapshot reports that the registry lacks.
fn ids(s: &App) -> Vec<String> {
    let mut ids: Vec<String> = s.mates.list().iter().map(|m| m.id.clone()).collect();
    for m in &s.captain.mates {
        if !ids.contains(&m.id) {
            ids.push(m.id.clone());
        }
    }
    ids
}

/// One line on how a mate stands: its session, then its work.
fn standing(s: &App, id: &str) -> String {
    if !s.captain.read {
        return "reading firstmate's records\u{2026}".into();
    }
    let session = Session::of(&s.captain, id).words();
    match s.captain.mate(id) {
        Some(m) => format!("{session} \u{b7} {}", secondmates::work_words(&m.state)),
        None => session.to_string(),
    }
}

/// The second mates in the agents column, and the way to add one.
pub(super) fn mate_list(s: &App, t: Theme, dark: bool) -> El {
    let mut list = col()
        .key("mates")
        .role("list")
        .label("Second mates")
        .pt(space::S6)
        .gap(space::S1)
        .child(heading("Second mates", t));
    if let Some(secondmates::Records {
        mates: Err(problem),
        ..
    }) = &s.mates.records
    {
        list = list.child(warning("mates-problem", problem, dark));
    }
    let ids = ids(s);
    if ids.is_empty() {
        list = list.child(note("mates-none", "No second mates yet.", t));
    }
    for id in ids {
        let selected = s.showing == Showing::Mate(id.clone());
        let open = id.clone();
        let mut row = col()
            .key(format!("mate-{id}"))
            .role("button")
            .label(format!("Open second mate {id}"))
            .focusable(true)
            .padding(space::S2)
            .gap(2.0)
            .radius(radius::SM)
            .fill(if selected { t.control_off } else { t.card })
            .on_press(move |s: &mut App| s.open_mate(&open))
            .child(text(id.clone()).font(type_::TITLE).color(t.text).wrap())
            .child(
                text(standing(s, &id))
                    .key(format!("mate-{id}-standing"))
                    .font(type_::LABEL)
                    .color(t.text_secondary)
                    .wrap(),
            );
        if secondmates::disagreement(&s.captain, s.mates.list(), &id).is_some() {
            row = row.child(
                text("records disagree")
                    .key(format!("mate-{id}-mismatch"))
                    .font(type_::LABEL)
                    .color(tok::accent(3, dark)),
            );
        }
        list = list.child(row);
    }
    if s.firstmate.installed() && s.chief.workplace().is_some() {
        list = list.child(
            row().child(small_button("add-mate", "Add second mate", t, |s| {
                s.add_mate(None);
            })),
        );
    }
    list
}

/// One second mate: what firstmate's records say about it, what it is doing and has returned,
/// what it waits on the captain for, the conversation with it, and the actions on it.
pub(super) fn mate_page(s: &App, id: &str, t: Theme, dark: bool) -> El {
    let mate = s.mates.get(id);
    let session = Session::of(&s.captain, id);
    let state = s.captain.mate(id);
    let mut body = col().key("mate-body").gap(space::S6);

    let mut about = col().gap(space::S1);
    match mate {
        Some(m) => {
            about = about
                .child(
                    text(m.summary.clone())
                        .key("mate-summary")
                        .font(type_::BODY)
                        .color(t.text)
                        .wrap(),
                )
                .child(note("mate-scope", &format!("Scope: {}", m.scope), t))
                .child(note(
                    "mate-projects",
                    &format!(
                        "Projects: {}",
                        if m.projects.is_empty() {
                            "none".to_string()
                        } else {
                            m.projects.join(", ")
                        }
                    ),
                    t,
                ))
                .child(note(
                    "mate-home",
                    &match &m.host {
                        Some(host) => {
                            format!("Home: {} on {host} \u{b7} added {}", m.home, m.added)
                        }
                        None => format!("Home: {} \u{b7} added {}", m.home, m.added),
                    },
                    t,
                ));
        }
        None => {
            about = about.child(note(
                "mate-unregistered",
                "Firstmate's registry, data/secondmates.md, does not list this mate.",
                t,
            ))
        }
    }
    let mut standing_words = if s.captain.read {
        format!("Session: {}", session.words())
    } else {
        "Session: reading firstmate's records\u{2026}".to_string()
    };
    if let Some(state) = state {
        standing_words.push_str(&format!(
            " \u{b7} {}",
            secondmates::work_words(&state.state)
        ));
        // Its children are listed below and an unknown state's reason under it; a held or
        // waiting mate's line says what it waits on.
        if matches!(state.state.as_str(), "captain_decision" | "externally_held") {
            standing_words.push_str(&format!(" \u{b7} {}", state.doing));
        }
    }
    about = about.child(
        text(standing_words)
            .key("mate-standing")
            .font(type_::LABEL)
            .color(t.text)
            .wrap(),
    );
    if let Some(problem) = secondmates::disagreement(&s.captain, s.mates.list(), id) {
        about = about.child(warning(
            "mate-mismatch",
            &format!("Firstmate's records disagree about {id}. {problem}"),
            dark,
        ));
    }
    body = body
        .child(about)
        .child(actions(s, id, mate, session, t, dark));

    // What it is working on now: its home's tasks in flight, as firstmate reports them.
    let prefix = format!("{id}/");
    let mut children = col().key("mate-children").gap(space::S1);
    let mut any = false;
    for child in s.captain.crew.iter().filter(|c| c.id.starts_with(&prefix)) {
        any = true;
        children = children.child(
            col()
                .key(format!("mate-child-{}", &child.id[prefix.len()..]))
                .padding(space::S2)
                .gap(2.0)
                .radius(radius::SM)
                .fill(t.bg)
                .child(
                    text(child.name.clone())
                        .font(type_::TITLE)
                        .color(t.text)
                        .wrap(),
                )
                .child(
                    text(format!("{} \u{b7} {}", child.state, child.doing))
                        .font(type_::LABEL)
                        .color(t.text_secondary)
                        .wrap(),
                ),
        );
    }
    if !any {
        children = children.child(note("mate-children-none", "Nothing in flight.", t));
    }
    body = body.child(section("Working on now", t).child(children));

    // What it escalated to the captain: tasks held for the captain in its own home.
    let holds: Vec<_> = s
        .captain
        .holds
        .iter()
        .filter(|h| h.owner.as_deref() == Some(id))
        .collect();
    if !holds.is_empty() {
        let mut list = col().key("mate-holds").gap(space::S2);
        for hold in holds {
            let (mate_id, key) = (id.to_string(), hold.key.clone());
            let answering = s.mates.answering.as_ref() == Some(&(id.to_string(), hold.key.clone()));
            list = list.child(
                col()
                    .key(format!("mate-hold-{}", hold.key))
                    .padding(space::S3)
                    .gap(space::S2)
                    .radius(radius::MD)
                    .fill(t.bg)
                    .child(
                        text(hold.question.clone())
                            .font(type_::BODY)
                            .color(t.text)
                            .wrap()
                            .selectable(true),
                    )
                    .child(row().child(button(
                        &format!("mate-answer-{}", hold.key),
                        if answering {
                            "Answering below"
                        } else {
                            "Answer"
                        },
                        t,
                        move |s| s.answer_mate_hold(&mate_id, &key),
                    ))),
            );
        }
        body = body.child(section("Waiting on you", t).child(list));
    }

    body = body.child(conversation(s, id, t, dark));

    // What its work returned on the parent channel, besides replies.
    let outcomes = s
        .mates
        .channel(id)
        .map(|c| c.outcomes(5))
        .unwrap_or_default();
    let mut returned = col().key("mate-outcomes").gap(space::S1);
    if outcomes.is_empty() {
        returned = returned.child(note("mate-outcomes-none", "Nothing returned yet.", t));
    }
    for (n, line) in outcomes.iter().enumerate() {
        returned = returned.child(
            col()
                .key(format!("mate-outcome-{n}"))
                .gap(2.0)
                .child(
                    text(format!("{}{}", line.verb, when(line.at)))
                        .font(type_::SECTION)
                        .color(t.text_secondary),
                )
                .child(
                    text(line.note.clone())
                        .font(type_::LABEL)
                        .color(t.text)
                        .wrap()
                        .selectable(true),
                ),
        );
    }
    body = body
        .child(section("Last outcomes", t).child(returned))
        .child(note(
            "mate-edit-note",
            "Changing a second mate's charter or projects is not offered: firstmate's \
             secondmate-provisioning procedure has no step for it. To change them, retire this \
             mate and add a new one.",
            t,
        ));

    let id_owned = id.to_string();
    let top = row()
        .gap(space::S2)
        .child(button("mate-peek", "Peek at its terminal", t, move |s| {
            s.showing = Showing::Crew(id_owned.clone())
        }))
        .child(button("mate-close", "Back to chat", t, |s| {
            s.showing = Showing::Chat
        }));
    page("mate-page", &format!("Second mate {id}"), top, body, t)
}

/// The actions on a mate, each through one of firstmate's scripts after the captain's yes.
fn actions(
    s: &App,
    id: &str,
    mate: Option<&secondmates::Mate>,
    session: Session,
    t: Theme,
    dark: bool,
) -> El {
    let mut bar = col().key("mate-actions").gap(space::S2);
    if let Some(outcome) = s
        .mates
        .outcomes
        .get(id)
        .filter(|o| !matches!(o.op, Op::Send { .. }))
    {
        bar = bar.child(if outcome.ok {
            note("mate-outcome", &outcome.words, t)
        } else {
            warning("mate-outcome", &outcome.words, dark)
        });
    }
    // Firstmate's scripts run one at a time.
    if let Some(op) = s.mates.running() {
        let doing = match op {
            Op::Send { id: to, .. } => format!("Delivering your message to {to}\u{2026}"),
            op => format!("Firstmate is working on {}\u{2026}", op.id()),
        };
        return bar.child(note("mate-running", &doing, t));
    }
    if let Some(op) = s.mates.confirming.as_ref().filter(|op| op.id() == id) {
        let (title, words, yes) = op.confirmation(mate);
        return bar.child(
            col()
                .key("mate-confirm")
                .padding(space::S3)
                .gap(space::S2)
                .radius(radius::MD)
                .fill(t.bg)
                .border(1.0, t.card_border)
                .child(text(title).font(type_::TITLE).color(t.text))
                .child(note("mate-confirm-words", &words, t))
                .child(
                    row()
                        .gap(space::S2)
                        .child(button("mate-confirm-yes", &yes, t, |s| s.confirmed_mate()))
                        .child(button("mate-confirm-cancel", "Cancel", t, |s| {
                            s.mates.confirming = None
                        })),
                ),
        );
    }
    let remote = mate.is_some_and(|m| m.host.is_some());
    let mut buttons = row().gap(space::S2).flex_wrap();
    let op = |make: fn(String) -> Op| {
        let id = id.to_string();
        move |s: &mut App| s.confirm_mate(make(id.clone()))
    };
    match session {
        Session::Up => {
            if !remote {
                buttons = buttons
                    .child(button("mate-interrupt", "Interrupt", t, op(Op::Interrupt)))
                    .child(button("mate-relaunch", "Relaunch", t, op(Op::Relaunch)));
            }
        }
        Session::Stopped | Session::NotStarted => {
            buttons = buttons.child(button("mate-start", "Start", t, op(Op::Start)));
        }
    }
    if mate.is_some() {
        buttons = buttons.child(button("mate-retire", "Retire", t, op(Op::Retire)));
    }
    bar = bar.child(buttons);
    if remote {
        bar = bar.child(note(
            "mate-remote",
            "This mate's home is on another machine. Firstmate's control plane refuses to \
             interrupt or relaunch it from here, so those actions are not offered.",
            t,
        ));
    }
    bar
}

/// The time of a line, after a separator, or nothing.
fn when(at: Option<u64>) -> String {
    at.map(|at| format!(" \u{b7} {}", crate::approval::clock(at)))
        .unwrap_or_default()
}

/// The messages sent to a mate and its replies, and the field to write the next one.
fn conversation(s: &App, id: &str, t: Theme, dark: bool) -> El {
    let channel = s.mates.channel(id);
    let mut list = col().key("mate-conversation").gap(space::S2);
    let said = channel.map(|c| c.conversation()).unwrap_or_default();
    if said.is_empty() {
        list = list.child(note(
            "mate-conversation-empty",
            "No messages yet. A message goes to its inbox the way firstmate routes a request, \
             and its reply comes back on the parent channel.",
            t,
        ));
    }
    for (n, entry) in said.iter().enumerate() {
        let (who, words, mine) = match entry {
            Said::Sent(request) => {
                let mine = request
                    .corr
                    .as_ref()
                    .is_some_and(|c| s.mates.sent.contains(c));
                let who = format!(
                    "{} \u{b7} to {id}{}{}",
                    if mine { "You" } else { "Firstmate" },
                    when(request.at),
                    if request.read {
                        " \u{b7} read"
                    } else {
                        " \u{b7} not read yet"
                    }
                );
                (
                    who,
                    text(request.text.clone())
                        .font(type_::BODY)
                        .color(t.text)
                        .wrap()
                        .selectable(true),
                    true,
                )
            }
            Said::Back(line) => (
                format!("{id} \u{b7} {}{}", line.verb, when(line.at)),
                markdown(&line.note, false, dark),
                false,
            ),
        };
        list = list.child(
            col()
                .key(format!("mate-msg-{n}"))
                .padding(space::S3)
                .gap(space::S1)
                .radius(radius::MD)
                .fill(if mine { t.control_off } else { t.bg })
                .child(text(who).font(type_::SECTION).color(t.text_secondary))
                .child(words.key(format!("mate-msg-{n}-text"))),
        );
    }
    if let Some(outcome) = s
        .mates
        .outcomes
        .get(id)
        .filter(|o| matches!(o.op, Op::Send { .. }))
    {
        list = list.child(if outcome.ok {
            note("mate-sent", &outcome.words, t)
        } else {
            warning("mate-sent", &outcome.words, dark)
        });
    }
    let answering = s
        .mates
        .answering
        .as_ref()
        .filter(|(mate, _)| mate == id)
        .map(|(_, key)| key.clone());
    if let Some(key) = &answering {
        list = list.child(
            row()
                .key("mate-answering")
                .gap(space::S2)
                .center_y()
                .child(
                    text(format!(
                        "Answering its captain hold {key}. Your next message goes to {id} as \
                         your answer, in your own words."
                    ))
                    .font(type_::LABEL)
                    .color(t.text_secondary)
                    .wrap()
                    .grow(1.0),
                )
                .child(button("mate-answer-cancel", "Cancel", t, |s| {
                    s.mates.answering = None
                })),
        );
    }
    let empty = TextEdit::default();
    let draft = s.mates.drafts.get(id).unwrap_or(&empty);
    let (edit_id, submit_id, send_id) = (id.to_string(), id.to_string(), id.to_string());
    let input = text_input(draft, type_::BODY)
        .key("mate-message")
        .single_line()
        .grow(1.0)
        .color(t.text)
        .placeholder(
            if answering.is_some() {
                "Your answer".to_string()
            } else {
                format!("Message {id}")
            },
            t.text_secondary,
        )
        .on_edit(move |s: &mut App| s.mates.draft(&edit_id))
        .on_submit(move |s: &mut App, _| s.send_to_mate(&submit_id));
    let composer = row()
        .gap(space::S2)
        .center_y()
        .child(
            row()
                .grow(1.0)
                .h(36.0)
                .px(space::S3)
                .center_y()
                .radius(radius::MD)
                .fill(t.bg)
                .border(1.0, t.card_border)
                .child(input),
        )
        .child(button("mate-send", "Send", t, move |s| {
            s.send_to_mate(&send_id)
        }));
    section("Conversation", t).child(list).child(composer)
}

/// The guided flow for adding a second mate: one question at a time, then exactly what will be
/// created and where, then the captain's yes.
pub(super) fn add_page(s: &App, t: Theme, dark: bool) -> El {
    let flow = &s.mates.adding;
    let (n, total) = flow.progress();
    let mut body = col().key("add-mate-body").gap(space::S3).child(note(
        "add-mate-progress",
        &format!("Step {n} of {total}"),
        t,
    ));
    let creating = matches!(s.mates.running(), Some(Op::Create(_)));
    let mut buttons = row().gap(space::S2).flex_wrap();
    let (question, lede) = match flow.step() {
        Step::Scope => (
            "What should this second mate be responsible for?".to_string(),
            "Say it in plain words, in one line: firstmate keeps it as the mate's charter and \
             routes work to it when work fits. For example: the billing service, its invoices \
             and payment retries."
                .to_string(),
        ),
        Step::Projects => (
            "Which projects should it hold?".to_string(),
            "It gets its own clone of each one. A project can be with more than one mate."
                .to_string(),
        ),
        Step::Name => (
            "What should it be called?".to_string(),
            format!(
                "A short name of letters, digits, dots, hyphens or underscores. Its home will \
                 be {}.",
                s.mates_dir()
                    .join(if flow.name.text.trim().is_empty() {
                        "<name>"
                    } else {
                        flow.name.text.trim()
                    })
                    .display()
            ),
        ),
        Step::Runtime => (
            "Which runtime should it run on?".to_string(),
            "Firstmate's configuration names more than one. A runtime you choose here applies \
             to this launch; when firstmate starts it again later, its configuration decides."
                .to_string(),
        ),
        Step::Confirm => (
            "Create this second mate?".to_string(),
            "Velvet Hammer will run these steps, and nothing else:".to_string(),
        ),
    };
    body = body
        .child(
            text(question)
                .key("add-mate-question")
                .font(type_::TITLE)
                .color(t.text)
                .wrap(),
        )
        .child(note("add-mate-lede", &lede, t));
    match flow.step() {
        Step::Scope => {
            body = body.child(field(
                "add-mate-scope",
                &flow.charter,
                "What it is responsible for",
                t,
                |s| &mut s.mates.adding.charter,
            ));
        }
        Step::Projects => body = body.child(projects(s, flow, t)),
        Step::Name => {
            body = body.child(field("add-mate-name", &flow.name, "Its name", t, |s| {
                &mut s.mates.adding.name
            }));
        }
        Step::Runtime => {
            let mut choices = col()
                .key("add-mate-runtimes")
                .role("radiogroup")
                .gap(space::S2);
            if let Some(Ok(found)) = &flow.runtimes {
                for (i, runtime) in found.iter().enumerate() {
                    let label = if i == 0 {
                        format!("{runtime} (firstmate's default for second mates)")
                    } else {
                        runtime.clone()
                    };
                    choices = choices.child(toggle(
                        &format!("add-mate-runtime-{i}"),
                        &label,
                        None,
                        flow.runtime == i,
                        t,
                        move |s| s.mates.adding.runtime = i,
                    ));
                }
            }
            body = body.child(choices);
        }
        Step::Confirm => body = body.child(confirm(s, flow, t, dark)),
    }
    if let Some(problem) = &flow.problem {
        body = body.child(warning("add-mate-problem", problem, dark));
    }
    if creating {
        body = body.child(note(
            "add-mate-creating",
            "Firstmate is creating it\u{2026}",
            t,
        ));
    } else {
        if flow.step() != Step::Scope {
            buttons = buttons.child(button("add-mate-back", "Back", t, |s| {
                s.mates.adding.back()
            }));
        }
        buttons = match flow.step() {
            Step::Confirm => {
                let yes = format!("Yes, create {}", flow.name.text.trim());
                buttons.child(button("add-mate-yes", &yes, t, |s| s.create_mate()))
            }
            _ => buttons.child(button("add-mate-next", "Next", t, |s| s.add_mate_next())),
        };
        body = body.child(buttons);
    }
    let cancel = row().child(button("add-mate-cancel", "Cancel", t, |s| {
        if s.mates.running().is_none() {
            s.mates.adding = AddMate::default();
            s.showing = Showing::Chat;
        }
    }));
    page("add-mate-page", "Add a second mate", cancel, body, t)
}

/// The projects firstmate has, to choose from, and the choice of none.
fn projects(s: &App, flow: &AddMate, t: Theme) -> El {
    let mut list = col().key("add-mate-projects").gap(space::S2);
    match &s.projects {
        Some(Ok(projects)) if !projects.is_empty() => {
            for project in projects {
                let name = project.name.clone();
                let why = secondmates::eligible(project).err();
                let chosen = flow.projects.contains(&project.name);
                list = list.child(toggle(
                    &format!("add-mate-project-{}", project.name),
                    &project.name,
                    Some(why.unwrap_or(&project.summary()).to_string()),
                    chosen,
                    t,
                    move |s| {
                        let eligible = s
                            .projects
                            .as_ref()
                            .and_then(|p| p.as_ref().ok())
                            .and_then(|p| p.iter().find(|p| p.name == name))
                            .is_some_and(|p| secondmates::eligible(p).is_ok());
                        if eligible {
                            s.toggle_mate_project(&name)
                        }
                    },
                ));
            }
        }
        Some(Err(problem)) => list = list.child(note("add-mate-projects-problem", problem, t)),
        _ => {
            list = list.child(note(
                "add-mate-projects-none",
                "Firstmate has no projects yet. Add one first, or choose No projects.",
                t,
            ))
        }
    }
    list.child(toggle(
        "add-mate-no-projects",
        "No projects",
        Some("For a mate that works on firstmate itself.".into()),
        flow.no_projects,
        t,
        |s| {
            let flow = &mut s.mates.adding;
            flow.no_projects = !flow.no_projects;
            if flow.no_projects {
                flow.projects.clear();
            }
        },
    ))
}

/// The plan, step by step, and what the captain answered.
fn confirm(s: &App, flow: &AddMate, t: Theme, dark: bool) -> El {
    let plan = flow.plan(&s.mates_dir());
    let clone = s
        .chief
        .workplace()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| s.firstmate.clone_dir().to_path_buf());
    let mut list = col()
        .key("add-mate-plan")
        .gap(space::S2)
        .child(note(
            "add-mate-charter",
            &format!("Charter: {}", plan.charter),
            t,
        ))
        .child(note(
            "add-mate-chosen",
            &format!(
                "Name: {} \u{b7} Projects: {}",
                plan.id,
                if plan.projects.is_empty() {
                    "none".to_string()
                } else {
                    plan.projects.join(", ")
                }
            ),
            t,
        ));
    if let Some(Err(problem)) = &flow.runtimes {
        list = list.child(warning(
            "add-mate-runtime-problem",
            &format!("Firstmate did not say which runtime it will use: {problem}"),
            dark,
        ));
    }
    for (n, step) in plan
        .steps(&clone, &s.firstmate.home_dir())
        .iter()
        .enumerate()
    {
        list = list.child(
            text(format!("{}. {step}", n + 1))
                .key(format!("add-mate-step-{n}"))
                .font(type_::BODY)
                .color(t.text)
                .wrap()
                .selectable(true),
        );
    }
    list
}

/// A choice that is on or off, with a line under it.
fn toggle(
    key: &str,
    label: &str,
    why: Option<String>,
    on: bool,
    t: Theme,
    f: impl Fn(&mut App) + 'static,
) -> El {
    let mut choice = col()
        .key(key)
        .role("checkbox")
        .label(label.to_string())
        .checked(on)
        .focusable(true)
        .padding(space::S3)
        .gap(2.0)
        .radius(radius::MD)
        .fill(if on { t.control_off } else { t.bg })
        .border(
            1.0,
            if on {
                tok::accent(0, true)
            } else {
                t.card_border
            },
        )
        .on_press(f)
        .child(
            text(format!(
                "{} {label}",
                if on { "\u{25cf}" } else { "\u{25cb}" }
            ))
            .font(type_::TITLE)
            .color(t.text),
        );
    if let Some(why) = why {
        choice = choice.child(note(&format!("{key}-why"), &why, t));
    }
    choice
}
