//! The GitHub repository combobox in the add-project controls ([`crate::repos`]): an element with
//! the combobox role holding a search field and, while open, a listbox of the captain's
//! repositories with the matched characters marked and a visibility badge on each. Rune reports
//! every text field to the platform as a text input, so the combobox role sits on the element
//! around it, as in ARIA 1.1's combobox pattern. Focus stays in the field; the arrows move the
//! highlighted row, which the field names as its active descendant, Return chooses it and Escape
//! closes the list.

use std::ops::Range;

use rune::tok::{self, Theme, radius, space, type_};
use rune::{El, Key, KeyEvent, ScrollSpec, Span, Tx, col, el, row, text, text_input};

use super::{note, small_button, warning};
use crate::App;
use crate::repos::{self, Match, Repo};

pub fn repo_picker(s: &App, t: Theme, dark: bool) -> El {
    let chooser = &s.repos;
    let (matches, all) = chooser.matches();
    let active = chooser.active(&matches);
    let mut field = text_input(&chooser.query, type_::BODY)
        .key("repo-search")
        .single_line()
        .grow(1.0)
        .color(t.text)
        .placeholder("Search your GitHub repositories", t.text_secondary)
        .label("Search your GitHub repositories")
        .expanded(chooser.open)
        .on_edit(|s: &mut App| &mut s.repos.query)
        .on_input_change(|s: &mut App, _| s.repos_edited())
        .on_input_key(keys);
    if chooser.open
        && let Some(repo) = active.and_then(|i| chooser.repo(i))
    {
        field = field.active_descendant(repos::row_key(repo));
    }
    let toggle = if chooser.open {
        small_button("repo-toggle", "Hide repositories", t, |s| s.repos.hide())
    } else {
        small_button("repo-toggle", "Show repositories", t, |s| s.show_repos())
    };
    let mut picker = col()
        .key("repo-picker")
        .role("combobox")
        .label("GitHub repository")
        .expanded(chooser.open)
        .gap(space::S2)
        .child(
            row()
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
                        .border(1.0, t.card_border)
                        .focus_within_ring(2.0, 1.0, tok::accent(0, dark))
                        .child(field),
                )
                .child(toggle),
        );
    if !chooser.open {
        return picker;
    }
    // What the list is doing: listing for the first time, refreshing, or why it cannot.
    match (chooser.listed(), chooser.loading()) {
        (None, _) => {
            picker = picker.child(note(
                "repo-loading",
                "Listing your GitHub repositories with gh\u{2026}",
                t,
            ))
        }
        (Some(Err(problem)), loading) => {
            picker = picker
                .child(warning("repo-problem", &problem.words(), dark))
                .child(row().child(small_button(
                    "repo-retry",
                    if loading {
                        "Trying again\u{2026}"
                    } else {
                        "Try again"
                    },
                    t,
                    |s| s.retry_repos(),
                )))
        }
        (Some(Ok(_)), true) => {
            picker = picker.child(note(
                "repo-refreshing",
                "Refreshing the list with gh\u{2026}",
                t,
            ))
        }
        (Some(Ok(_)), false) => {}
    }
    let Some(Ok(_)) = chooser.listed() else {
        return picker;
    };
    if matches.is_empty() {
        return picker.child(note(
            "repo-none",
            "No repository matches. Try fewer letters, or choose a folder or type a git URL.",
            t,
        ));
    }
    let size = matches.len();
    let rows = matches.iter().enumerate().filter_map(|(at, m)| {
        let repo = chooser.repo(m.index)?;
        Some(option(
            repo,
            m,
            active == Some(m.index),
            (at, size),
            t,
            dark,
        ))
    });
    picker = picker.child(
        col()
            .key("repo-list")
            .role("listbox")
            .label("GitHub repositories")
            .max_h(280.0)
            .radius(radius::MD)
            .fill(t.card)
            .border(1.0, t.card_border)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(col().padding(space::S1).children(rows)),
    );
    if all > size {
        picker = picker.child(note(
            "repo-more",
            &format!("Showing {size} of {all}. Type to narrow the list."),
            t,
        ));
    }
    picker
}

/// One repository's row: its name and description with the matched characters marked, and its
/// visibility, fork and archived badges.
fn option(
    repo: &Repo,
    m: &Match,
    active: bool,
    (at, size): (usize, usize),
    t: Theme,
    dark: bool,
) -> El {
    let name = repo.full_name();
    let mark = tok::accent(0, dark);
    let mut words = col().grow(1.0).gap(2.0).child(marked(
        repo.full_name(),
        &m.in_name,
        type_::BODY,
        t.text,
        mark,
    ));
    if !repo.about.is_empty() {
        words = words.child(marked(
            repo.about.clone(),
            &m.in_about,
            type_::LABEL,
            t.text_secondary,
            mark,
        ));
    }
    let mut badges = row().gap(space::S1).center_y().child(badge(
        repo.visibility.word(),
        repo.visibility != repos::Visibility::Public,
        t,
        dark,
    ));
    if repo.fork {
        badges = badges.child(badge("Fork", false, t, dark));
    }
    if repo.archived {
        badges = badges.child(badge("Archived", false, t, dark));
    }
    if !repo.pushed.is_empty() {
        badges = badges.child(
            text(format!("pushed {}", repo.pushed_on()))
                .font(type_::LABEL)
                .color(t.text_secondary),
        );
    }
    let mut row = row()
        .key(repos::row_key(repo))
        .role("option")
        .label(repo.full_name())
        .focus_selected(active)
        .position_in_set(at + 1, size)
        .gap(space::S3)
        .center_y()
        .px(space::S2)
        .py(space::S2)
        .radius(radius::SM)
        .hover(t.hover)
        .preserve_focus_on_press(true)
        // By name, not position: a refresh may reorder the list between drawing and the press.
        .on_press(move |s: &mut App| {
            if let Some(index) = s.repos.position(&name) {
                s.choose_repo(index)
            }
        })
        .child(words)
        .child(badges);
    if active {
        row = row.fill(mark.alpha(0.22));
    }
    row
}

/// `words` with the `hits` byte ranges in bold, in `mark`.
fn marked(
    words: String,
    hits: &[Range<usize>],
    font: rune::TextStyle,
    color: rune::Color,
    mark: rune::Color,
) -> El {
    let spans: Vec<Span> = hits
        .iter()
        .map(|r| Span::new(r.start, r.end).bold().color(mark))
        .collect();
    text(words).font(font).color(color).spans(spans).wrap()
}

/// A visibility, fork or archived badge; private ones are filled so they stand out.
fn badge(label: &str, strong: bool, t: Theme, dark: bool) -> El {
    let badge = el()
        .h(20.0)
        .px(space::S2)
        .center()
        .radius(radius::SM)
        .child(text(label.to_string()).font(type_::LABEL).color(if strong {
            t.knob
        } else {
            t.text
        }));
    if strong {
        badge.fill(tok::accent(1, dark))
    } else {
        badge.fill(t.bg).border(1.0, t.card_border)
    }
}

/// The combobox's keys while its field has focus: Down opens the list or moves the highlight,
/// Up moves it back, Return chooses the highlighted repository, and Escape closes the list, or
/// clears the query when it is already closed. Other keys edit the query.
fn keys(s: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if s.repos.query.preedit().is_some() || ev.mods.cmd || ev.mods.ctrl || ev.mods.alt {
        return false;
    }
    let step = |s: &mut App, by: isize, tx: &mut Tx| {
        if let Some(repo) = s.repos.step(by).and_then(|i| s.repos.repo(i)) {
            tx.scroll_into_view(repos::row_key(repo));
        }
    };
    match ev.key {
        Key::Down if !s.repos.open => s.show_repos(),
        Key::Down => step(s, 1, tx),
        Key::Up if s.repos.open => step(s, -1, tx),
        Key::Enter if s.repos.open => {
            if !tx.key_repeat() {
                let (matches, _) = s.repos.matches();
                if let Some(index) = s.repos.active(&matches) {
                    s.choose_repo(index);
                }
            }
        }
        Key::Escape if s.repos.open => s.repos.hide(),
        Key::Escape if !s.repos.query.text.is_empty() => s.repos.query.clear(),
        _ => return false,
    }
    true
}
