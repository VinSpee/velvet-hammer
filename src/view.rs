//! The window: agents on the left, the chief-of-staff chat (or a peek at one subagent, or the
//! Claude sign-in while the chat can't run) in the middle, and the two panels for the captain on
//! the right. Every element a test or a headless script reaches has a key.

use crate::{App, Showing};
use rune::tok::{self, Theme, radius, space, type_};
use rune::{Cx, El, ScrollSpec, TextEdit, TextHighlight, col, el, row, text, text_input};
use rune_kit::{Highlight, Kit, Markdown};

use crate::approval::{self, Effect, Mode};

use crate::find::{self, Hits};
use crate::firstmate::{Checkout, Distro, Firstmate, Standing, Work};
use crate::links;
use crate::model::{Action, Agent, Author, Item, ItemKind, Message, Plan, Status};
use crate::projects::Project;

mod policy;
mod settings;
mod tool;
mod ui;
use policy::{rules_page, visuals_page};
use settings::{all_settings, menu_bar, settings_page};
use tool::{picture_page, tool_use};

pub fn view(s: &App, cx: &mut Cx<App>) -> El {
    let t = tok::theme(cx.system_dark());
    let peeked = match s.showing {
        Showing::Peek(id) => s.ws.agent(id),
        _ => None,
    };
    let main = match (&s.showing, peeked, &s.status) {
        (Showing::Crew(id), ..) => crew_peek(s, id, t, cx.system_dark()),
        (Showing::Settings, ..) => settings_page(s, t, cx.system_dark()),
        (Showing::Firstmate, ..) => firstmate_page(s, t, cx.system_dark()),
        (Showing::Approvals, ..) => approvals_page(s, t, cx.system_dark()),
        (Showing::Activity, ..) => activity_page(s, t, cx.system_dark()),
        (Showing::Visuals, ..) => visuals_page(s, t, cx.system_dark()),
        (Showing::Rules, ..) => rules_page(s, t, cx.system_dark()),
        (Showing::Picture(id), ..) => picture_page(s, id, t, cx.system_dark()),
        (_, Some(agent), _) => peek(s, agent, t, cx.system_dark()),
        // A chief of staff that runs in the managed clone cannot chat until it is installed.
        (_, None, Status::Ready(_)) if !s.chat_ready() => firstmate_page(s, t, cx.system_dark()),
        (_, None, Status::Ready(plan)) => chat(s, plan, t, cx.system_dark()),
        (_, None, status) => sign_in(status, t, cx.system_dark()),
    };
    let window = row()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .fill(t.bg)
        .child(agents(s, t, cx.system_dark()))
        .child(main)
        .child(
            col()
                .key("panels")
                .w(320.0)
                .h_full()
                .min_h(0.0)
                .padding(space::S3)
                .gap(space::S3)
                .child(needs_input(s, t, cx.system_dark()))
                .child(for_you(s, t)),
        );
    col()
        .w_full()
        .h_full()
        .fill(t.bg)
        .children(s.in_window_menus.then(|| menu_bar(s, t)))
        .child(window)
}

fn agents(s: &App, t: Theme, dark: bool) -> El {
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
    // The tasks firstmate has in flight.
    for crew in &s.captain.crew {
        let id = crew.id.clone();
        let selected = s.showing == Showing::Crew(id.clone());
        list = list.child(
            col()
                .key(format!("crew-{id}"))
                .role("button")
                .label(format!("Peek at {}", crew.name))
                .focusable(true)
                .padding(space::S2)
                .gap(2.0)
                .radius(radius::SM)
                .fill(if selected { t.control_off } else { t.card })
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
                        .color(t.text_secondary)
                        .wrap(),
                ),
        );
    }
    if let Some(projects) = &s.projects {
        list = list.child(project_list(s, projects, t, dark));
    }
    list
}

/// The projects in firstmate's records, and the way to add one.
fn project_list(s: &App, projects: &Result<Vec<Project>, String>, t: Theme, dark: bool) -> El {
    let mut list = col()
        .key("projects")
        .role("list")
        .label("Projects")
        .pt(space::S6)
        .gap(space::S1)
        .child(heading("Projects", t));
    match projects {
        Err(problem) => list = list.child(warning("projects-problem", problem, dark)),
        Ok(projects) if projects.is_empty() => {
            list = list.child(note("projects-none", "No projects yet.", t))
        }
        Ok(projects) => {
            for project in projects {
                let mut row = col()
                    .key(format!("project-{}", project.name))
                    .padding(space::S2)
                    .gap(2.0)
                    .child(
                        text(project.name.clone())
                            .font(type_::TITLE)
                            .color(t.text)
                            .wrap(),
                    )
                    .child(
                        text(project.summary())
                            .font(type_::LABEL)
                            .color(t.text_secondary)
                            .wrap(),
                    );
                if !project.about.is_empty() {
                    row = row.child(
                        text(project.about.clone())
                            .font(type_::LABEL)
                            .color(t.text_secondary)
                            .wrap(),
                    );
                }
                list = list.child(row);
            }
        }
    }
    if s.chat_ready() {
        list.child(
            row().child(small_button("add-project", "Add project", t, |s| {
                s.add_project()
            })),
        )
    } else {
        list.child(note(
            "add-project-later",
            "Your chief of staff adds projects once it can chat.",
            t,
        ))
    }
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
fn firstmate_page(s: &App, t: Theme, dark: bool) -> El {
    let fm = &s.firstmate;
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
        buttons
            .child(all_settings("firstmate-all-settings", s, t))
            .child(button("firstmate-close", "Back to chat", t, |s| {
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

/// The tool-approval settings: the mode, the custom rules, an example to try them on, and the
/// limits no setting changes.
fn approvals_page(s: &App, t: Theme, dark: bool) -> El {
    let settings = s.approvals.settings();
    let form = &s.form;
    let mut body = col().key("approvals-body").gap(space::S6);
    if let Some(problem) = s.approvals.problem() {
        body = body.child(warning("approvals-problem", problem, dark));
    }

    let mut modes = col().key("modes").role("radiogroup").gap(space::S2);
    for mode in Mode::ALL {
        modes = modes.child(choice(
            &format!("mode-{}", mode.key()),
            mode.label(),
            mode == Mode::RECOMMENDED,
            mode.explanation(),
            settings.mode == mode,
            t,
            move |s: &mut App| s.approvals.change(|x| x.mode = mode),
        ));
    }
    let reset = if form.confirm_reset {
        row()
            .gap(space::S2)
            .flex_wrap()
            .child(button(
                "approvals-reset-confirm",
                "Reset and delete my rules",
                t,
                |s| s.form.reset(&mut s.approvals),
            ))
            .child(button(
                "approvals-reset-cancel",
                "Keep my settings",
                t,
                |s| s.form.confirm_reset = false,
            ))
    } else {
        row().child(button(
            "approvals-reset",
            "Reset to the recommended default",
            t,
            |s| s.form.confirm_reset = true,
        ))
    };
    body = body.child(section("How tools run", t).child(modes).child(reset));

    let mut rules = col().key("rules").gap(space::S2);
    if settings.rules.is_empty() {
        rules = rules.child(note(
            "rules-empty",
            "No rules. The mode decides every tool use.",
            t,
        ));
    }
    let last = settings.rules.len().saturating_sub(1);
    for (i, rule) in settings.rules.iter().enumerate() {
        let mut buttons = row().gap(space::S1);
        if i > 0 {
            buttons = buttons.child(small_button(&format!("rule-{i}-up"), "Up", t, move |s| {
                s.form.shift(&mut s.approvals, i, -1)
            }));
        }
        if i < last {
            buttons = buttons.child(small_button(
                &format!("rule-{i}-down"),
                "Down",
                t,
                move |s| s.form.shift(&mut s.approvals, i, 1),
            ));
        }
        buttons = buttons
            .child(small_button(
                &format!("rule-{i}-edit"),
                "Edit",
                t,
                move |s| s.form.edit(&s.approvals, i),
            ))
            .child(small_button(
                &format!("rule-{i}-delete"),
                "Delete",
                t,
                move |s| s.form.delete(&mut s.approvals, i),
            ));
        rules = rules.child(
            row()
                .key(format!("rule-{i}"))
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
                    text(format!("{}. {}", i + 1, rule.describe()))
                        .font(type_::LABEL)
                        .color(t.text)
                        .wrap_anywhere()
                        .grow(1.0),
                )
                .child(buttons),
        );
    }

    let mut effects = row().key("rule-effect").role("radiogroup").gap(space::S1);
    for effect in Effect::ALL {
        let selected = form.effect == effect;
        effects = effects.child(
            el().key(format!("rule-effect-{}", effect.key()))
                .role("radio")
                .label(effect.label())
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
                .on_press(move |s: &mut App| s.form.effect = effect)
                .child(text(effect.label()).font(type_::LABEL).color(if selected {
                    t.knob
                } else {
                    t.text
                })),
        );
    }
    let mut editor = col()
        .key("rule-form")
        .gap(space::S2)
        .child(effects)
        .child(field(
            "rule-tool",
            &form.tool,
            "Tool, such as Bash, Edit or mcp__github__*",
            t,
            |s| &mut s.form.tool,
        ))
        .child(field(
            "rule-pattern",
            &form.pattern,
            "Pattern (optional), such as git push or /Users/me/project/*",
            t,
            |s| &mut s.form.pattern,
        ));
    if let Some(problem) = &form.problem {
        editor = editor.child(warning("rule-problem", problem, dark));
    }
    let mut form_buttons = row().gap(space::S2).child(button(
        "rule-save",
        if form.editing.is_some() {
            "Save rule"
        } else {
            "Add rule"
        },
        t,
        |s| s.form.save(&mut s.approvals),
    ));
    if form.editing.is_some() {
        form_buttons = form_buttons.child(button("rule-cancel", "Cancel", t, |s| s.form.clear()));
    }
    editor = editor.child(form_buttons);
    body = body.child(
        section("Rules", t)
            .child(note(
                "rules-help",
                "Rules apply before the mode. Deny wins over ask and allow, and ask wins over \
                 allow, whatever their order; ask brings up the full-input question even when \
                 running without asking. A Bash pattern matches commands that start with it, in \
                 every part of a command joined with && ; or |. Other patterns match the file \
                 path or main argument, and * matches any text. A rule cannot see inside a \
                 script or sh -c. Rules are a convenience filter, not a sandbox: extra spaces, an env or \
                 sudo prefix, bash -c, or a relative or .. path all get past a rule.",
                t,
            ))
            .child(rules)
            .child(editor),
    );

    body = body.child(
        section("Try a tool call", t)
            .child(
                row()
                    .gap(space::S2)
                    .child(
                        field("example-tool", &form.example_tool, "Tool", t, |s| {
                            &mut s.form.example_tool
                        })
                        .w(160.0)
                        .grow(0.0),
                    )
                    .child(field(
                        "example-argument",
                        &form.example_argument,
                        "Command, file path or input",
                        t,
                        |s| &mut s.form.example_argument,
                    )),
            )
            .child(
                text(form.example(settings))
                    .key("example-result")
                    .font(type_::BODY)
                    .color(t.text)
                    .wrap(),
            ),
    );

    body = body.child(note(
        "approvals-limits",
        &format!(
            "No setting changes these: Velvet Hammer never reads or stores your Claude sign-in, \
             never lets ANTHROPIC_API_KEY or ANTHROPIC_AUTH_TOKEN reach Claude Code, and never \
             pushes, publishes or merges anything itself. These settings apply to your chief of \
             staff's tool uses and Velvet Hammer's own tools, take effect at once, and are kept \
             in {}.",
            s.approvals.path().display()
        ),
        t,
    ));
    let actions = row()
        .gap(space::S2)
        .child(all_settings("approvals-all-settings", s, t))
        .child(button("approvals-activity", "Activity log", t, |s| {
            s.showing = Showing::Activity
        }))
        .child(button("approvals-close", "Back to chat", t, |s| {
            s.showing = Showing::Chat
        }));
    page("approvals-page", "Tool approval", actions, body, t)
}

/// Every tool use the chief of staff made and what was decided, newest first.
fn activity_page(s: &App, t: Theme, dark: bool) -> El {
    let mut body = col().key("activity-body").gap(space::S2);
    if let Some(problem) = s.activity.problem() {
        body = body.child(warning("activity-problem", problem, dark));
    }
    if s.activity.entries().is_empty() {
        body = body.child(note("activity-empty", "No tool uses yet.", t));
    }
    for (n, entry) in s.activity.entries().iter().enumerate().rev() {
        let decision = if entry.allowed { "Allowed" } else { "Denied" };
        body = body.child(
            col()
                .key(format!("activity-{n}"))
                .padding(space::S3)
                .gap(2.0)
                .radius(radius::MD)
                .fill(t.bg)
                .child(
                    text(format!("{decision} \u{b7} {}", entry.tool))
                        .font(type_::TITLE)
                        .color(if entry.allowed {
                            t.text
                        } else {
                            tok::accent(3, dark)
                        }),
                )
                .child(
                    text(entry.subject.clone())
                        .font(type_::LABEL.mono())
                        .color(t.text)
                        .wrap_anywhere()
                        .selectable(true),
                )
                .child(note(
                    &format!("activity-{n}-how"),
                    &format!("{} \u{b7} {}", approval::clock(entry.at), entry.how),
                    t,
                )),
        );
    }
    let help = note(
        "activity-help",
        &format!(
            "Every tool use of your chief of staff and how it was decided, newest first. Each \
             entry keeps the start of the command or path, not the whole input, so it can contain \
             command text: avoid putting secrets in commands. The log is kept in {}.",
            s.activity.path().display()
        ),
        t,
    );
    let actions = row()
        .gap(space::S2)
        .child(button("activity-settings", "Tool approval", t, |s| {
            s.showing = Showing::Approvals
        }))
        .child(button("activity-close", "Back to chat", t, |s| {
            s.showing = Showing::Chat
        }));
    page(
        "activity-page",
        "Tool activity",
        actions,
        col().gap(space::S3).child(help).child(body),
        t,
    )
}

/// One choice of a radio group: its label, marked (Recommended) when it is, and why to pick it.
/// Its explanation is keyed `<key>-why`.
fn choice(
    key: &str,
    label: &str,
    recommended: bool,
    why: &str,
    selected: bool,
    t: Theme,
    on_press: impl Fn(&mut App) + 'static,
) -> El {
    let title = if recommended {
        format!("{label} (Recommended)")
    } else {
        label.to_string()
    };
    col()
        .key(key)
        .role("radio")
        .label(title.clone())
        .checked(selected)
        .focusable(true)
        .padding(space::S3)
        .gap(2.0)
        .radius(radius::MD)
        .fill(if selected { t.control_off } else { t.bg })
        .border(
            1.0,
            if selected {
                tok::accent(0, true)
            } else {
                t.card_border
            },
        )
        .on_press(on_press)
        .child(
            text(format!(
                "{} {title}",
                if selected { "\u{25cf}" } else { "\u{25cb}" }
            ))
            .font(type_::TITLE)
            .color(t.text),
        )
        .child(note(&format!("{key}-why"), why, t))
}

/// A page in the middle column: a heading with its buttons over a card that scrolls.
fn page(key: &str, title: &str, actions: El, body: El, t: Theme) -> El {
    col()
        .key(key)
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .padding(space::S6)
        .gap(space::S6)
        .child(
            row()
                .gap(space::S2)
                .center_y()
                .child(heading(title, t).grow(1.0))
                .child(actions),
        )
        .child(
            col()
                .grow(1.0)
                .min_h(0.0)
                .radius(radius::LG)
                .fill(t.card)
                .border(1.0, t.card_border)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(col().padding(space::S6).child(body)),
        )
}

fn section(title: &str, t: Theme) -> El {
    col().gap(space::S2).child(
        text(title.to_string())
            .font(type_::SECTION)
            .color(t.text_secondary),
    )
}

/// A one-line text field.
fn field(
    key: &str,
    edit: &TextEdit,
    placeholder: &str,
    t: Theme,
    on_edit: impl for<'a> Fn(&'a mut App) -> &'a mut TextEdit + 'static,
) -> El {
    row()
        .grow(1.0)
        .h(32.0)
        .px(space::S3)
        .center_y()
        .radius(radius::MD)
        .fill(t.bg)
        .border(1.0, t.card_border)
        .child(
            text_input(edit, type_::BODY)
                .key(key)
                .single_line()
                .grow(1.0)
                .color(t.text)
                .placeholder(placeholder.to_string(), t.text_secondary)
                .on_edit(on_edit),
        )
}

fn small_button(key: &str, label: &str, t: Theme, f: impl Fn(&mut App) + 'static) -> El {
    el().key(key)
        .role("button")
        .label(label.to_string())
        .focusable(true)
        .h(24.0)
        .px(space::S2)
        .center()
        .radius(radius::SM)
        .fill(t.control_off)
        .on_press(f)
        .child(text(label.to_string()).font(type_::LABEL).color(t.text))
}

fn count(n: u32, what: &str) -> String {
    format!("{n} {what}{}", if n == 1 { "" } else { "s" })
}

fn chat(s: &App, plan: &Plan, t: Theme, dark: bool) -> El {
    let input = text_input(&s.draft, type_::BODY)
        .key("chat-input")
        .single_line()
        .grow(1.0)
        .color(t.text)
        .placeholder(
            if s.answering.is_some() {
                "Your answer"
            } else {
                "Message your chief of staff"
            },
            t.text_secondary,
        )
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
    let mut header = col()
        .gap(2.0)
        .child(heading("Chief of staff", t))
        .child(note("health", &health(plan), t));
    if let Some(clone) = s.chief.workplace() {
        header = header.child(note(
            "workplace",
            &format!("Running firstmate from {}", clone.display()),
            t,
        ));
    }
    let hits = if s.find.open {
        find::search(s, &s.find.query.text)
    } else {
        Vec::new()
    };
    col()
        .key("chat")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .padding(space::S6)
        .gap(space::S3)
        .child(header)
        .children(s.find.open.then(|| find_bar(s, &hits, t)))
        .children((s.has_no_projects() && !s.adding).then(|| no_projects(t)))
        .child(transcript(
            s,
            "messages",
            &s.ws.chat,
            s.replying,
            Found {
                chat: true,
                earlier: s.ws.earlier,
                hits: &hits,
                current: s.find.current,
            },
            t,
            dark,
        ))
        .children(s.answering.as_ref().map(|key| {
            row()
                .key("answering")
                .gap(space::S2)
                .center_y()
                .child(
                    text(format!(
                        "Answering the captain hold {key}. Your next message goes to your \
                         chief of staff as your answer, in your own words."
                    ))
                    .font(type_::LABEL)
                    .color(t.text_secondary)
                    .wrap()
                    .grow(1.0),
                )
                .child(button("answer-cancel", "Cancel", t, |s| s.answering = None))
        }))
        .children(s.adding.then(|| adding(s, t, dark)))
        .children(match &s.flash {
            Some((Err(id), _)) => Some(warning(
                "link-missing",
                &format!(
                    "That message is not in this conversation: {}. It may be from history that \
                     was cleared or not kept.",
                    links::url(id)
                ),
                dark,
            )),
            _ => None,
        })
        .child(composer)
}

/// The find bar over the conversation: the query, where the current match is among them all,
/// and stepping through them.
fn find_bar(s: &App, hits: &[Hits], t: Theme) -> El {
    let total = find::total(hits);
    let query = s.find.query.text.trim();
    let count = if query.is_empty() {
        String::new()
    } else if let Some(id) = links::parse(query) {
        if s.ws.message(id).is_some() {
            "Went to the linked message".to_string()
        } else {
            "Message not found".to_string()
        }
    } else if total == 0 {
        "No matches".to_string()
    } else {
        format!("{} of {total}", s.find.current.min(total - 1) + 1)
    };
    let input = text_input(&s.find.query, type_::BODY)
        .key("find-input")
        .single_line()
        .grow(1.0)
        .color(t.text)
        .placeholder("Find in the conversation", t.text_secondary)
        .on_edit(|s: &mut App| &mut s.find.query)
        .on_input_key(
            |s: &mut App, key: &rune::KeyEvent, tx: &mut rune::Tx| match key.key {
                rune::Key::Enter => {
                    s.step_find(if key.mods.shift { -1 } else { 1 });
                    true
                }
                rune::Key::Escape => {
                    s.close_find(tx);
                    true
                }
                _ => false,
            },
        );
    row()
        .key("find-bar")
        .role("search")
        .label("Find in the conversation")
        .gap(space::S2)
        .center_y()
        .child(
            row()
                .grow(1.0)
                .h(32.0)
                .px(space::S3)
                .center_y()
                .radius(radius::MD)
                .fill(t.card)
                .border(1.0, tok::accent(0, true))
                .child(input),
        )
        .child(
            text(count)
                .key("find-count")
                .font(type_::LABEL)
                .color(t.text_secondary),
        )
        .child(small_button("find-previous", "Previous", t, |s| {
            s.step_find(-1)
        }))
        .child(small_button("find-next", "Next", t, |s| s.step_find(1)))
        .child(
            small_button("find-close", "Done", t, |_| {})
                .on_press_tx(|s: &mut App, tx: &mut rune::Tx| s.close_find(tx)),
        )
}

/// The empty state: the app works on projects, and there are none yet.
fn no_projects(t: Theme) -> El {
    col()
        .key("projects-empty")
        .padding(space::S6)
        .gap(space::S2)
        .radius(radius::LG)
        .fill(t.card)
        .border(1.0, t.card_border)
        .child(heading("Add your first project", t))
        .child(
            text(
                "Velvet Hammer works on projects: the repositories your chief of staff and its \
                 crew work in. You have none yet. Your chief of staff asks for the repository (a \
                 git URL or a folder on this computer), a name and how its work should ship, \
                 then shows you what it will clone and register, and where, and waits for your \
                 yes. Nothing remote is created or published unless you agree to it.",
            )
            .key("projects-empty-lede")
            .font(type_::BODY)
            .color(t.text_secondary)
            .wrap(),
        )
        .child(
            row().child(button("add-first-project", "Add a project", t, |s| {
                s.add_project()
            })),
        )
}

/// The small controls that help answer the chief of staff while adding a project. The
/// conversation stays the way the project is added; these only fill in or send an answer.
fn adding(s: &App, t: Theme, dark: bool) -> El {
    let mut buttons = row().gap(space::S2).flex_wrap().child(button(
        "add-pick-folder",
        if s.picking() {
            "Choosing a folder\u{2026}"
        } else {
            "Choose a folder\u{2026}"
        },
        t,
        |s| s.pick_folder(),
    ));
    // A yes or no only once the chief of staff has finished asking.
    let asked = s.replying.is_none()
        && s.ws
            .chat
            .last()
            .is_some_and(|m| m.author == Author::ChiefOfStaff);
    if asked {
        buttons = buttons
            .child(button("add-yes", "Yes", t, |s| s.reply("Yes.")))
            .child(button("add-no", "No", t, |s| s.reply("No.")));
    }
    buttons = buttons.child(button("add-finish", "Done adding", t, |s| {
        s.finish_adding()
    }));
    let mut bar = col()
        .key("adding")
        .gap(space::S2)
        .child(note(
            "adding-help",
            "Adding a project with your chief of staff. Answer in the chat; a chosen folder's \
             path goes into your message for you to send.",
            t,
        ))
        .child(buttons);
    if let Some(problem) = &s.pick_problem {
        bar = bar.child(warning("add-pick-problem", problem, dark));
    }
    bar
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

/// A firstmate crewmate's terminal, as `bin/fm-peek.sh` reads it. Looking only: the captain
/// steers a crewmate through the chief of staff.
fn crew_peek(s: &App, id: &str, t: Theme, dark: bool) -> El {
    let crew = s.captain.crew.iter().find(|c| c.id == id);
    let (name, doing) = match crew {
        Some(crew) => (
            crew.name.as_str(),
            format!("{} \u{b7} {}", crew.state, crew.doing),
        ),
        None => (id, "No longer in flight.".to_string()),
    };
    let header = row()
        .gap(space::S2)
        .center_y()
        .child(
            col()
                .grow(1.0)
                .gap(2.0)
                .child(heading(&format!("Peeking at {name}"), t))
                .child(note("crew-doing", &doing, t)),
        )
        .child(button("peek-close", "Back to chat", t, |s| {
            s.showing = Showing::Chat
        }));
    let pane = match s.peek.as_ref().and_then(|p| p.pane.as_ref()) {
        None => note("crew-peek-reading", "Reading its terminal\u{2026}", t),
        Some(Err(problem)) => warning("crew-peek-problem", problem, dark),
        Some(Ok(lines)) => col()
            .key("crew-pane")
            .grow(1.0)
            .min_h(0.0)
            .padding(space::S3)
            .radius(radius::MD)
            .fill(t.card)
            .scroll(ScrollSpec { stick_bottom: true })
            .child(
                text(lines.clone())
                    .font(type_::LABEL.mono())
                    .color(t.text)
                    .wrap_anywhere()
                    .selectable(true),
            ),
    };
    col()
        .key("crew-peek")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .padding(space::S6)
        .gap(space::S3)
        .child(header)
        .child(pane)
        .child(note(
            "crew-peek-help",
            "The last 40 lines of this crewmate's terminal, read by firstmate's bin/fm-peek.sh. \
             To steer it, tell your chief of staff.",
            t,
        ))
}

fn peek(s: &App, agent: &Agent, t: Theme, dark: bool) -> El {
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
        .child(transcript(
            s,
            "peek-messages",
            &agent.transcript,
            None,
            Found::default(),
            t,
            dark,
        ))
}

/// What a transcript marks: where the messages of this session start, and the matches of
/// the find bar with the current one.
#[derive(Default)]
struct Found<'a> {
    /// The transcript is the chief of staff's chat, whose messages have links.
    chat: bool,
    earlier: usize,
    hits: &'a [Hits],
    current: usize,
}

impl Found<'_> {
    fn of(&self, i: usize) -> Option<&Hits> {
        self.hits.iter().find(|h| h.message == i)
    }
}

/// The colour behind a match, stronger for the current one.
fn match_color(current: bool, dark: bool) -> rune::Color {
    tok::accent(3, dark).alpha(if current { 0.75 } else { 0.3 })
}

/// A conversation. The chief of staff's messages are Markdown, drawn formatted; `replying` is
/// the one still arriving, whose unfinished tail draws as what it is about to become. Everyone
/// else's words show as written, and a tool use shows through its handler. Messages kept from
/// earlier sessions sit above a divider.
fn transcript(
    s: &App,
    key: &str,
    messages: &[Message],
    replying: Option<usize>,
    found: Found,
    t: Theme,
    dark: bool,
) -> El {
    let mut list = col().gap(space::S2);
    for (i, m) in messages.iter().enumerate() {
        if found.earlier > 0 && i == found.earlier {
            list = list.child(earlier_divider(t));
        }
        let hits = found.of(i);
        if let Some(tool) = &m.tool {
            let mut card = tool_use(s, i, tool, t, dark);
            if let Some(hits) = hits {
                let current = hits.holds(found.current);
                card = card.border(if current { 2.0 } else { 1.0 }, match_color(current, dark));
            }
            list = list.child(card);
            continue;
        }
        let who = match m.author {
            Author::Captain => "You",
            Author::ChiefOfStaff => "Chief of staff",
            Author::Subagent => "Subagent",
            Author::Problem => "Claude Code reported a problem",
            Author::Tool => "Tool",
            Author::App => "Velvet Hammer",
        };
        let words = if m.author == Author::ChiefOfStaff {
            let marks = hits.map(|h| {
                h.numbered()
                    .map(|(n, ranges)| {
                        let current = n == found.current;
                        let mark = Highlight::new()
                            .current_match(current)
                            .bg(match_color(current, dark))
                            .key(find::match_key(n));
                        (ranges.clone(), mark)
                    })
                    .collect::<Vec<_>>()
            });
            markdown(
                &links::markdown(&m.text, &s.ws),
                replying == Some(i),
                marks.unwrap_or_default(),
                dark,
            )
            .key(format!("msg-{i}-text"))
        } else {
            let marks = hits
                .into_iter()
                .flat_map(|h| h.numbered())
                .flat_map(|(n, ranges)| {
                    let current = n == found.current;
                    ranges.iter().map(move |r| {
                        TextHighlight::new(r.clone(), match_color(current, dark))
                            .key(find::match_key(n))
                    })
                });
            let (shown, chips) = links::display(&m.text, &s.ws);
            let mut words = text(shown)
                .font(type_::BODY)
                .color(t.text)
                .wrap()
                .selectable(true)
                .text_highlights(marks)
                .pills(
                    chips
                        .iter()
                        .map(|c| (c.range.clone(), chip_style(c.found, t, dark))),
                )
                .on_link(|s: &mut App, url: &str, tx: &mut rune::Tx| s.open_link(url, tx));
            for chip in &chips {
                words = words.link(rune::Link::new(chip.range.clone(), chip.url.clone()));
            }
            words
        };
        let flashed = matches!(&s.flash, Some((Ok(id), _)) if *id == m.id) && found.chat;
        let mut header = row().gap(space::S2).center_y().child(
            text(who)
                .font(type_::SECTION)
                .color(t.text_secondary)
                .grow(1.0),
        );
        if found.chat {
            header = header.child(link_button(s, i, m, t));
        }
        let mut card = col()
            .key(format!("msg-{i}"))
            .padding(space::S3)
            .gap(space::S1)
            .radius(radius::MD)
            .fill(if flashed {
                tok::accent(0, dark).alpha(0.18)
            } else if m.author == Author::Captain {
                t.control_off
            } else {
                t.card
            })
            .child(header)
            .child(words);
        if flashed {
            card = card.border(2.0, tok::accent(0, dark));
        }
        if found.chat {
            card = card.on_hover_change(move |s: &mut App, inside, _| {
                if inside {
                    s.hovered = Some(i);
                } else if s.hovered == Some(i) {
                    s.hovered = None;
                    s.copied = None;
                }
            });
        }
        list = list.child(card);
    }
    col()
        .key(key)
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec { stick_bottom: true })
        .child(list)
}

/// The key of chat message `i`'s element: its card for a tool use, its bubble otherwise.
pub fn message_key(i: usize, m: &Message) -> String {
    if m.tool.is_some() {
        format!("tool-{i}")
    } else {
        format!("msg-{i}")
    }
}

/// How a link chip looks: the accent for a message in the chat, the warning colour for one that
/// is not.
fn chip_style(found: bool, t: Theme, dark: bool) -> rune::PillStyle {
    let tint = if found {
        tok::accent(0, dark)
    } else {
        tok::accent(3, dark)
    };
    rune::PillStyle::new().bg(tint.alpha(0.22)).fg(t.text)
}

/// The small button that copies message `i`'s link. It shows while the pointer is over the
/// message or the button has focus, and stays reachable with Tab.
fn link_button(s: &App, i: usize, m: &Message, t: Theme) -> El {
    let key = format!("msg-{i}-link");
    let copied = s.copied.as_deref() == Some(m.id.as_str());
    let shown = copied || s.hovered == Some(i) || s.focused.as_deref() == Some(key.as_str());
    let id = m.id.clone();
    small_button(
        &key,
        if copied { "Link copied" } else { "Copy link" },
        t,
        |_| {},
    )
    .label(format!(
        "Copy the link to this message, {}",
        links::url(&m.id)
    ))
    .opacity(if shown { 1.0 } else { 0.0 })
    .on_press_tx(move |s: &mut App, tx: &mut rune::Tx| {
        tx.copy(links::url(&id));
        s.copied = Some(id.clone());
    })
}

/// The line between the messages kept from earlier sessions and this session's.
fn earlier_divider(t: Theme) -> El {
    row()
        .key("earlier-divider")
        .gap(space::S2)
        .center_y()
        .py(space::S2)
        .child(el().h(1.0).grow(1.0).fill(t.card_border))
        .child(
            text(
                "Earlier sessions above. Your chief of staff does not remember them unless you \
                 link to one.",
            )
            .font(type_::LABEL)
            .color(t.text_secondary),
        )
        .child(el().h(1.0).grow(1.0).fill(t.card_border))
}

/// Markdown from the chief of staff, formatted (see [`crate::markdown`]), with the find bar's
/// matches marked. Nothing in it runs: HTML shows as text. A message link draws as a chip and
/// goes to its message (see [`crate::links`]); other links open in the default browser only when
/// pressed, and only `http`, `https` and `mailto` links open at all ([`rune::Tx::open_url`]
/// refuses the rest).
fn markdown(
    source: &str,
    streaming: bool,
    marks: Vec<(Vec<std::ops::Range<usize>>, Highlight)>,
    dark: bool,
) -> El {
    Markdown::new(crate::markdown::prepare(source))
        .highlights(marks)
        .streaming(streaming)
        .selectable(true)
        .link_style(move |url: &str| {
            links::parse(url).map(|_| {
                let chip = rune::Span::new(0, 0)
                    .color(tok::theme(dark).text)
                    .bg(tok::accent(0, dark).alpha(0.22), 3.0);
                (
                    chip,
                    rune::LinkPaint::new().bg(tok::accent(0, dark).alpha(0.35)),
                )
            })
        })
        .on_link_tx(|s: &mut App, url: &str, tx: &mut rune::Tx| s.open_link(url, tx))
        .build(&Kit::neutral(dark))
}

fn needs_input(s: &App, t: Theme, dark: bool) -> El {
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
    let mut rows: Vec<El> = rows.collect();
    // From firstmate's records: tasks held for the captain, then merges waiting for approval.
    if let Some(problem) = &s.captain_problem {
        rows.push(warning("captain-problem", problem, dark));
    }
    for hold in &s.captain.holds {
        let key = hold.key.clone();
        let answering = s.answering.as_deref() == Some(&hold.key);
        rows.push(
            words_card(&format!("hold-{key}"), &hold.key, &hold.question, t).child(row().child(
                button(
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
                ),
            )),
        );
    }
    for merge in &s.captain.merges {
        let (task, url) = (merge.task.clone(), merge.url.clone());
        rows.push(
            words_card(
                &format!("merge-{task}"),
                &format!("Approve the merge for {task}?"),
                &format!("{}\n{}", merge.url, merge.reason),
                t,
            )
            .child(row().child(button(
                &format!("approve-{task}"),
                "Approve",
                t,
                move |s| s.approve_merge(&task, &url),
            ))),
        );
    }
    panel(
        "needs-input",
        "Needs your input",
        "Nothing is waiting on you.",
        rows,
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
