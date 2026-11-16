//! The Firstmate page's choice of home, and what the app shows about a home's session lock,
//! there and under the chat heading.

use rune::tok::{Theme, radius, space, type_};
use rune::{El, col, row, text_input};

use super::{button, note, section, small_button, warning};
use crate::App;
use crate::home::{Checked, Choice, Lock};

/// Which home is in use, and choosing another: a folder typed or picked, checked at once, then
/// used on the captain's word.
pub fn home_section(s: &App, t: Theme, dark: bool) -> El {
    let in_use = match s.firstmate.custom() {
        Some(dir) => format!("In use: your firstmate home at {}.", dir.display()),
        None => format!(
            "In use: the managed home, Velvet Hammer's own clone at {} and the home beside it.",
            s.firstmate.clone_dir().display()
        ),
    };
    let mut part = section("Firstmate home", t)
        .key("firstmate-home")
        .child(note("firstmate-home-in-use", &in_use, t))
        .child(note(
            "firstmate-home-help",
            "Choose an existing firstmate, such as the ~/firstmate you run in a terminal, to see \
             and steer its fleet here. It must be a clone of firstmate that firstmate has run \
             in. Velvet Hammer never installs, updates, resets or deletes anything in a home you \
             choose, and never takes its session lock from another session.",
            t,
        ));
    for problem in [s.home_problem.as_deref(), s.home_choice.problem()]
        .into_iter()
        .flatten()
    {
        part = part.child(warning("firstmate-home-saved-problem", problem, dark));
    }
    let input = text_input(&s.home_form.path, type_::BODY)
        .key("firstmate-home-path")
        .single_line()
        .grow(1.0)
        .color(t.text)
        .placeholder("/Users/you/firstmate", t.text_secondary)
        .on_edit(|s: &mut App| &mut s.home_form.path)
        .on_submit(|s: &mut App, _| s.check_home());
    let picking = if s.picking_home() {
        small_button("firstmate-home-pick", "Choosing\u{2026}", t, |_| {})
    } else {
        small_button("firstmate-home-pick", "Choose folder\u{2026}", t, |s| {
            s.pick_home()
        })
    };
    // The buttons go under the field, so a long path never pushes them out of the card.
    part = part
        .child(
            row()
                .w_full()
                .min_w(0.0)
                .h(32.0)
                .px(space::S3)
                .center_y()
                .radius(radius::MD)
                .fill(t.bg)
                .border(1.0, t.card_border)
                .child(input),
        )
        .child(row().gap(space::S2).child(picking).child(small_button(
            "firstmate-home-check",
            "Check",
            t,
            |s| s.check_home(),
        )));
    match &s.home_form.checked {
        Some(Ok(checked)) => part = part.child(checked_home(s, checked, t, dark)),
        Some(Err(why)) => part = part.child(warning("firstmate-home-refused", why, dark)),
        None => {}
    }
    if let Some(choice) = &s.home_form.confirming {
        part = part.child(confirm_switch(s, choice, t, dark));
    }
    if let Some(problem) = &s.home_form.problem {
        part = part.child(warning("firstmate-home-problem", problem, dark));
    }
    if s.firstmate.custom().is_some() && s.home_form.confirming.is_none() {
        part = part.child(row().child(button(
            "firstmate-home-managed",
            "Use the managed home",
            t,
            |s| s.use_home(Choice(None)),
        )));
    }
    part
}

/// What the check found in a folder that can be used, who holds its lock, and the button that
/// uses it.
fn checked_home(s: &App, checked: &Checked, t: Theme, dark: bool) -> El {
    let mut found = col().key("firstmate-home-found").gap(2.0);
    for (i, line) in checked.found.iter().enumerate() {
        found = found.child(note(
            &format!("firstmate-home-found-{i}"),
            &format!("\u{2713} {line}"),
            t,
        ));
    }
    match &s.home_form.lock {
        Some(Ok(lock)) => {
            let words = match &lock.holder {
                Some(holder) => format!(
                    "Another firstmate session holds this home's lock now: {}. If you use this \
                     home, firstmate runs your chief of staff read-only until that session \
                     ends. Velvet Hammer will not take the lock.",
                    holder_words(holder.pid, &holder.command)
                ),
                None => format!("Its session lock is not held ({}).", lock.said),
            };
            found = found.child(if lock.holder.is_some() {
                warning("firstmate-home-lock", &words, dark)
            } else {
                note("firstmate-home-lock", &words, t)
            });
        }
        Some(Err(why)) => found = found.child(warning("firstmate-home-lock", why, dark)),
        None => {}
    }
    if s.firstmate.custom() == Some(checked.path.as_path()) {
        found.child(note("firstmate-home-same", "This is the home in use.", t))
    } else if s.home_form.confirming.is_none() {
        let choice = Choice(Some(checked.path.clone()));
        found.child(row().pt(space::S1).child(button(
            "firstmate-home-use",
            "Use this home",
            t,
            move |s| s.use_home(choice.clone()),
        )))
    } else {
        found
    }
}

/// The captain's yes before a switch that interrupts work in flight.
fn confirm_switch(s: &App, choice: &Choice, t: Theme, dark: bool) -> El {
    let to = match &choice.0 {
        Some(dir) => format!("Switch to your home at {}?", dir.display()),
        None => "Switch to the managed home?".into(),
    };
    let choice = choice.clone();
    col()
        .key("firstmate-home-confirm")
        .gap(space::S2)
        .padding(space::S3)
        .radius(radius::MD)
        .fill(t.bg)
        .border(1.0, t.card_border)
        .child(warning("firstmate-home-confirm-title", &to, dark))
        .child(note(
            "firstmate-home-in-flight",
            &s.work_in_flight().join(" "),
            t,
        ))
        .child(
            row()
                .gap(space::S2)
                .flex_wrap()
                .child(button(
                    "firstmate-home-switch",
                    "Switch anyway",
                    t,
                    move |s| s.switch_home(choice.clone()),
                ))
                .child(button("firstmate-home-cancel", "Cancel", t, |s| {
                    s.home_form.confirming = None
                })),
        )
}

/// A process in a few words: its pid, and its command when `ps` gave one.
fn holder_words(pid: u32, command: &str) -> String {
    if command.is_empty() {
        format!("pid {pid}")
    } else {
        format!("pid {pid}, {command}")
    }
}

/// What the captain must know about the home's lock: another session holds it, so the chief
/// of staff runs read-only; or that session ended, and a restart takes the lock. `None` when
/// there is nothing to say.
pub fn lock_notice(s: &App, t: Theme, dark: bool) -> Option<El> {
    let Some(Ok(lock)) = &s.lock else {
        return match &s.lock {
            Some(Err(why)) => Some(warning("lock-problem", why, dark)),
            _ => None,
        };
    };
    if let Some(holder) = s.lock_holder() {
        return Some(held(
            lock,
            &holder_words(holder.pid, &holder.command),
            t,
            dark,
        ));
    }
    s.read_only_session.then(|| {
        col()
            .key("lock-free")
            .gap(space::S1)
            .child(note(
                "lock-free-words",
                &format!(
                    "The other session let go of this home's lock ({}). Your chief of staff's \
                     session started while it was held, so firstmate still runs it read-only. \
                     Restart it to take the lock.",
                    lock.said
                ),
                t,
            ))
            .child(row().child(small_button(
                "lock-restart",
                "Restart chief of staff",
                t,
                |s| s.restart_chief(),
            )))
    })
}

/// The lock is held by another session: who, firstmate's exact words, and what follows.
fn held(lock: &Lock, holder: &str, t: Theme, dark: bool) -> El {
    col()
        .key("lock-held")
        .gap(space::S1)
        .child(warning(
            "lock-held-title",
            "Another firstmate session holds this home's lock, so your chief of staff runs \
             read-only.",
            dark,
        ))
        .child(note(
            "lock-held-holder",
            &format!("Holder: {holder}. Firstmate says: {}", lock.said),
            t,
        ))
        .child(note(
            "lock-held-help",
            "Firstmate allows one session per home. Velvet Hammer will not take the lock, stop \
             that session or force anything. Until it ends, firstmate keeps your chief of staff \
             from spawning, steering or merging, and Velvet Hammer makes no change to second \
             mates. You can still read the fleet and talk with your chief of staff here.",
            t,
        ))
}
