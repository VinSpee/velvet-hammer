//! The pages for the hooks that steer the chief of staff beyond tool approval: visual replies
//! ([`crate::visual`]) and firstmate's rules ([`crate::guard`]).

use rune::tok::{Theme, radius, space, type_};
use rune::{El, col, row, text, text_input};

use super::{button, choice, note, page, section, small_button, warning};
use crate::guard::{self, Rule};
use crate::visual::{self, Mode};
use crate::{App, Showing, approval};

/// The visual-replies setting: the mode, the shapes that qualify, what the chief of staff is told,
/// and a reply to try them on.
pub fn visuals_page(s: &App, t: Theme, dark: bool) -> El {
    let mode = *s.visuals.settings();
    let mut body = col().key("visuals-body").gap(space::S6);
    if let Some(problem) = s.visuals.problem() {
        body = body.child(warning("visuals-problem", problem, dark));
    }
    let mut modes = col().key("visual-modes").role("radiogroup").gap(space::S2);
    for choice_mode in Mode::ALL {
        modes = modes.child(choice(
            &format!("visual-mode-{}", choice_mode.key()),
            choice_mode.label(),
            choice_mode == Mode::RECOMMENDED,
            choice_mode.explanation(),
            mode == choice_mode,
            t,
            move |s: &mut App| s.visuals.change(|m| *m = choice_mode),
        ));
    }
    body = body.child(section("How replies are shown", t).child(modes));

    let mut shapes = col().key("visual-shapes").gap(space::S1);
    for (i, rule) in visual::RULES.iter().enumerate() {
        shapes = shapes.child(
            text(format!("{}. {rule}", i + 1))
                .key(format!("visual-shape-{i}"))
                .font(type_::LABEL)
                .color(t.text)
                .wrap(),
        );
    }
    body = body.child(
        section("Replies that read better as a view", t)
            .child(shapes)
            .child(note(
                "visual-shapes-help",
                "Anything else, such as a plain answer or a question back to you, stays plain \
                 text. Text inside code blocks does not count.",
                t,
            )),
    );

    body = body.child(
        section("What the chief of staff is told", t)
            .child(note(
                "visual-instruction-help",
                "Added with each of your messages, in Nudge only and Enforce:",
                t,
            ))
            .child(quote("visual-instruction", visual::INSTRUCTION, t))
            .child(note(
                "visual-retry",
                "In Enforce, a reply of one of the shapes above with no view in its turn is sent \
                 back once with the feedback below, and the chat says so. A reply is never sent \
                 back twice in a row, nor while Claude Code is already continuing because of a \
                 stop hook, so it cannot loop or hang.",
                t,
            ))
            .child(quote(
                "visual-feedback",
                &visual::feedback(&visual::Shape::List(3)),
                t,
            )),
    );

    let verdict = match (visual::shape(&s.visual_example.text), mode) {
        (None, _) => "Stays plain text.".to_string(),
        (Some(shape), Mode::Enforce) => format!(
            "Sent back once to be shown as a view: the reply {}.",
            shape.describe()
        ),
        (Some(shape), _) => format!(
            "Not checked in {}; in Enforce it would be sent back, because the reply {}.",
            mode.label(),
            shape.describe()
        ),
    };
    body = body.child(
        section("Try a reply", t)
            .child(
                row()
                    .h(96.0)
                    .padding(space::S2)
                    .radius(radius::MD)
                    .fill(t.bg)
                    .border(1.0, t.card_border)
                    .child(
                        text_input(&s.visual_example, type_::BODY)
                            .key("visual-example")
                            .grow(1.0)
                            .color(t.text)
                            .on_edit(|s: &mut App| &mut s.visual_example),
                    ),
            )
            .child(
                text(verdict)
                    .key("visual-example-result")
                    .font(type_::BODY)
                    .color(t.text)
                    .wrap(),
            ),
    );
    body = body.child(note(
        "visuals-kept",
        &format!(
            "Changes apply to the running session at once and are kept in {} in Velvet \
             Hammer's data directory.",
            visual::SETTINGS_FILE
        ),
        t,
    ));
    let actions = row()
        .gap(space::S2)
        .child(button("visuals-close", "Back to chat", t, |s| {
            s.showing = Showing::Chat
        }));
    page("visuals-page", "Visual replies", actions, body, t)
}

/// Firstmate's rules: each with what it enforces, what it matches, what it does, a switch, and
/// what it decided lately.
pub fn rules_page(s: &App, t: Theme, dark: bool) -> El {
    let settings = s.rules.settings();
    let mut body = col().key("rules-body").gap(space::S6);
    if let Some(problem) = s.rules.problem() {
        body = body.child(warning("rules-problem", problem, dark));
    }
    body = body.child(note(
        "rules-intro",
        "Your chief of staff runs as firstmate, and these are firstmate's own rules for the \
         actions a hook can see. Velvet Hammer checks each before every tool use, ahead of your \
         tool-approval settings: a rule only denies or asks you, never allows, and a deny rule \
         of yours still wins over its question. They read command text, so a script, bash -c or \
         a path built at run time gets past them; docs/firstmate-rules.md lists what no hook \
         can enforce. All are on by default.",
        t,
    ));
    for rule in Rule::ALL {
        let key = rule.key();
        let on = settings.on(rule);
        let recent: Vec<&approval::Entry> = s
            .activity
            .entries()
            .iter()
            .rev()
            .filter(|e| e.rule.as_deref() == Some(key))
            .take(3)
            .collect();
        let mut lately = col().key(format!("guard-{key}-recent")).gap(2.0);
        if recent.is_empty() {
            lately = lately.child(note(&format!("guard-{key}-quiet"), "Nothing yet.", t));
        }
        for (n, entry) in recent.into_iter().enumerate() {
            lately = lately.child(
                text(format!(
                    "{} \u{b7} {} \u{b7} {}",
                    approval::clock(entry.at),
                    entry.how,
                    entry.subject
                ))
                .key(format!("guard-{key}-recent-{n}"))
                .font(type_::LABEL.mono())
                .color(t.text)
                .wrap_anywhere()
                .selectable(true),
            );
        }
        let switch = small_button(
            &format!("guard-{key}-toggle"),
            if on {
                "On \u{b7} turn off"
            } else {
                "Off \u{b7} turn on"
            },
            t,
            move |s: &mut App| s.rules.change(|x| x.set(rule, !on)),
        )
        .role("switch")
        .checked(on);
        body = body.child(
            col()
                .key(format!("guard-{key}"))
                .padding(space::S3)
                .gap(space::S1)
                .radius(radius::MD)
                .fill(t.bg)
                .border(1.0, t.card_border)
                .child(
                    row()
                        .gap(space::S2)
                        .center_y()
                        .child(
                            text(rule.title())
                                .font(type_::TITLE)
                                .color(if on { t.text } else { t.text_secondary })
                                .wrap()
                                .grow(1.0),
                        )
                        .child(switch),
                )
                .child(note(
                    &format!("guard-{key}-source"),
                    &format!("Enforces: {}", rule.source()),
                    t,
                ))
                .child(note(
                    &format!("guard-{key}-matches"),
                    &format!("Matches: {}", rule.matches()),
                    t,
                ))
                .child(note(
                    &format!("guard-{key}-does"),
                    &format!("Then: {}", rule.does()),
                    t,
                ))
                .child(text("Lately").font(type_::LABEL).color(t.text_secondary))
                .child(lately),
        );
    }
    body = body.child(note(
        "rules-kept",
        &format!(
            "Changes apply to the running session at once and are kept in {} in Velvet \
             Hammer's data directory. Every decision is also in the activity log.",
            guard::SETTINGS_FILE
        ),
        t,
    ));
    let actions = row()
        .gap(space::S2)
        .child(button("rules-activity", "Activity log", t, |s| {
            s.showing = Showing::Activity
        }))
        .child(button("rules-close", "Back to chat", t, |s| {
            s.showing = Showing::Chat
        }));
    page("rules-page", "Firstmate rules", actions, body, t)
}

/// Text the app sends the model, shown as written.
fn quote(key: &str, words: &str, t: Theme) -> El {
    col()
        .padding(space::S3)
        .radius(radius::MD)
        .fill(t.bg)
        .border(1.0, t.card_border)
        .child(
            text(words.to_string())
                .key(key)
                .font(type_::LABEL.mono())
                .color(t.text)
                .wrap()
                .selectable(true),
        )
}
