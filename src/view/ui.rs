//! A json-render view in the chat (see [`crate::ui`]): each node of the drawn tree as one of the
//! app's own views. Every element is keyed `tool-N-ui-ID`, with `.I` after the id for each
//! repeat copy.

use rune::tok::{self, TextStyle, Theme, radius, space, type_};
use rune::{El, TextEdit, col, el, row, text, text_input};

use super::note;
use super::tool::picture_el;
use crate::App;
use crate::tools::{self, ToolUse};
use crate::ui::{self, Node, Tree, What};

/// The tallest an image in a view draws.
const IMAGE_MAX_H: f32 = 240.0;

/// The view a tool use carries, as far as it has arrived.
pub(super) fn view(s: &App, key: &str, tool: &ToolUse, t: Theme, dark: bool) -> El {
    let running = tool.result.is_none();
    // The tool refused the spec: its reason is the model's to fix and the captain's to see.
    if let Some(result) = tool.result.as_ref().filter(|r| r.is_error) {
        return note(&format!("{key}-refused"), &tools::result_text(result), t);
    }
    let spec = match ui::parse(&tool.input) {
        Ok((spec, _)) => spec,
        Err(why) => return note(&format!("{key}-refused"), &why, t),
    };
    let state =
        s.ui.get(&tool.id)
            .map_or_else(|| spec.state.clone(), |live| live.state(&spec));
    let Tree { root, cut, .. } = ui::tree(&spec, &state);
    let Some(root) = root else {
        let words = if running {
            "The view is arriving\u{2026}"
        } else {
            "The view has nothing to show."
        };
        return note(&format!("{key}-waiting"), words, t);
    };
    let draw = Draw {
        s,
        key,
        id: &tool.id,
        t,
        dark,
    };
    let mut body = col()
        .key(format!("{key}-ui"))
        .gap(space::S2)
        .child(draw.node(&root));
    if cut {
        body = body.child(note(
            &format!("{key}-cut"),
            &format!(
                "Only the first {} parts of this view are shown.",
                ui::MAX_NODES
            ),
            t,
        ));
    }
    body
}

struct Draw<'a> {
    s: &'a App,
    key: &'a str,
    /// The view's tool use.
    id: &'a str,
    t: Theme,
    dark: bool,
}

impl Draw<'_> {
    fn key(&self, node: &Node) -> String {
        format!("{}-ui-{}", self.key, node.key)
    }

    fn node(&self, node: &Node) -> El {
        let (t, key) = (self.t, self.key(node));
        let children = || node.children.iter().map(|c| self.node(c));
        match &node.what {
            What::Stack { horizontal, gap } => {
                let gap = match gap.as_str() {
                    "none" => 0.0,
                    "sm" => space::S1,
                    "lg" => space::S3,
                    _ => space::S2,
                };
                let stack = if *horizontal {
                    row().flex_wrap().center_y()
                } else {
                    col()
                };
                stack.key(key).gap(gap).children(children())
            }
            What::Card { title, description } => {
                let mut card = col()
                    .key(key)
                    .padding(space::S3)
                    .gap(space::S2)
                    .radius(radius::MD)
                    .fill(t.bg)
                    .border(1.0, t.card_border);
                if !title.is_empty() {
                    card = card.child(words(title, type_::TITLE, t.text));
                }
                if !description.is_empty() {
                    card = card.child(words(description, type_::LABEL, t.text_secondary));
                }
                card.children(children())
            }
            What::Separator => el().key(key).h(1.0).w_full().fill(t.separator),
            What::Heading { text, level } => {
                let style = match level.as_str() {
                    "h1" => TextStyle::new(20.0, 650.0),
                    "h2" => TextStyle::new(16.0, 600.0),
                    "h3" => TextStyle::new(14.0, 600.0),
                    _ => type_::TITLE,
                };
                words(text, style, t.text).key(key)
            }
            What::Text { text, variant } => {
                let (style, color) = match variant.as_str() {
                    "caption" => (type_::LABEL, t.text_secondary),
                    "muted" => (type_::BODY, t.text_secondary),
                    "code" => (type_::LABEL.mono(), t.text),
                    _ => (type_::BODY, t.text),
                };
                words(text, style, color).key(key).selectable(true)
            }
            What::Badge {
                text: label,
                variant,
            } => {
                let (fill, color) = match variant.as_str() {
                    "secondary" | "outline" => (t.control_off, t.text),
                    "destructive" => (tok::accent(2, self.dark), t.knob),
                    _ => (tok::accent(0, true), t.knob),
                };
                let mut badge = el()
                    .key(key)
                    .h(20.0)
                    .px(space::S2)
                    .center()
                    .radius(radius::SM)
                    .fill(fill)
                    .child(text(label.clone()).font(type_::LABEL).color(color));
                if variant == "outline" {
                    badge = badge.fill(t.bg).border(1.0, t.card_border);
                }
                row().child(badge)
            }
            What::Alert {
                title,
                message,
                tone,
            } => {
                let accent = match tone.as_str() {
                    "success" => 4,
                    "warning" => 3,
                    "error" => 2,
                    _ => 0,
                };
                let mut alert = col()
                    .key(key)
                    .role("alert")
                    .padding(space::S3)
                    .gap(space::S1)
                    .radius(radius::MD)
                    .fill(t.bg)
                    .border(1.0, tok::accent(accent, self.dark))
                    .child(words(title, type_::TITLE, t.text));
                if !message.is_empty() {
                    alert = alert.child(words(message, type_::BODY, t.text));
                }
                alert
            }
            What::Progress { fraction, label } => {
                let mut bar = col().key(key).gap(space::S1);
                if !label.is_empty() {
                    bar = bar.child(words(label, type_::LABEL, t.text_secondary));
                }
                bar.child(
                    row()
                        .label(format!("{label} {:.0}%", fraction * 100.0))
                        .h(6.0)
                        .w_full()
                        .radius(3.0)
                        .fill(t.control_track)
                        .child(
                            el().h(6.0)
                                .w_frac(*fraction)
                                .radius(3.0)
                                .fill(tok::accent(0, true)),
                        ),
                )
            }
            What::Image { src, alt } => match self.s.ui_images.get(src) {
                Some(Ok(image)) if image.error().is_none() => picture_el(image, IMAGE_MAX_H)
                    .key(key)
                    .role("image")
                    .label(if alt.is_empty() {
                        src.clone()
                    } else {
                        alt.clone()
                    }),
                Some(Ok(image)) => {
                    let why = image.error().map(|e| e.0.clone()).unwrap_or_default();
                    note(
                        &key,
                        &format!("Image {src}: it could not be decoded ({why})."),
                        t,
                    )
                }
                Some(Err(why)) => note(&key, &format!("Image {src} is not shown. {why}"), t),
                None => note(&key, &format!("Image {src}\u{2026}"), t),
            },
            What::Table {
                columns,
                rows,
                caption,
            } => {
                let line = |cells: &[String], style: TextStyle, color| {
                    let mut line = row().gap(space::S2);
                    for i in 0..columns.len().max(cells.len()) {
                        let cell = cells.get(i).cloned().unwrap_or_default();
                        line = line.child(
                            col()
                                .grow(1.0)
                                .w(0.0)
                                .child(text(cell).font(style).color(color).wrap_anywhere()),
                        );
                    }
                    line
                };
                let mut table = col()
                    .key(key)
                    .role("table")
                    .gap(space::S1)
                    .child(line(columns, type_::SECTION, t.text_secondary))
                    .child(el().h(1.0).w_full().fill(t.separator));
                for r in rows {
                    table = table.child(line(r, type_::BODY, t.text));
                }
                if !caption.is_empty() {
                    table = table.child(words(caption, type_::LABEL, t.text_secondary));
                }
                table
            }
            What::Button {
                label,
                variant,
                disabled,
                press,
            } => {
                let (fill, color) = match variant.as_str() {
                    "secondary" => (t.control_off, t.text),
                    "danger" => (tok::accent(2, self.dark), t.knob),
                    _ => (tok::accent(0, true), t.knob),
                };
                let mut button = el()
                    .key(key)
                    .role("button")
                    .label(label.clone())
                    .h(28.0)
                    .px(space::S3)
                    .center()
                    .radius(radius::SM)
                    .fill(fill)
                    .child(text(label.clone()).font(type_::LABEL).color(color));
                if *disabled {
                    button = button.disabled(true).opacity(0.5);
                } else {
                    button = button.focusable(true);
                    if let Some(press) = press.clone() {
                        let (id, did) = (
                            self.id.to_string(),
                            format!("Pressed \u{201c}{label}\u{201d}"),
                        );
                        button =
                            button.on_press(move |s: &mut App| s.fire(&id, &press, did.clone()));
                    }
                }
                row().child(button)
            }
            What::Input {
                label,
                placeholder,
                path,
                value,
                submit,
            } => {
                // An input bound to nothing still takes typing; it keeps its text to itself.
                let path = path.clone().unwrap_or_else(|| format!("#{}", node.key));
                let shown = self
                    .s
                    .ui
                    .get(self.id)
                    .and_then(|live| live.edits.get(&path))
                    .cloned()
                    .unwrap_or_else(|| {
                        let mut edit = TextEdit::default();
                        edit.text = value.clone();
                        edit
                    });
                let (id, seed) = (self.id.to_string(), value.clone());
                let mut input = text_input(&shown, type_::BODY)
                    .key(key.clone())
                    .label(label.clone())
                    .single_line()
                    .grow(1.0)
                    .color(t.text)
                    .placeholder(placeholder.clone(), t.text_secondary)
                    .on_edit(move |s: &mut App| s.view_edit(&id, &path, &seed));
                if let Some(submit) = submit.clone() {
                    let (id, did) = (
                        self.id.to_string(),
                        format!("Submitted \u{201c}{label}\u{201d}"),
                    );
                    input =
                        input.on_submit(move |s: &mut App, _| s.fire(&id, &submit, did.clone()));
                }
                col()
                    .gap(space::S1)
                    .child(words(label, type_::LABEL, t.text_secondary))
                    .child(
                        row()
                            .h(32.0)
                            .px(space::S3)
                            .center_y()
                            .radius(radius::MD)
                            .fill(t.bg)
                            .border(1.0, t.card_border)
                            .child(input),
                    )
            }
            What::Checkbox {
                label,
                path,
                checked,
                change,
            } => {
                let (id, path, change, on) =
                    (self.id.to_string(), path.clone(), change.clone(), !checked);
                let did = format!(
                    "{} \u{201c}{label}\u{201d}",
                    if on { "Checked" } else { "Unchecked" }
                );
                let mut tick = el()
                    .w(16.0)
                    .h(16.0)
                    .center()
                    .radius(4.0)
                    .border(1.0, t.card_border);
                if *checked {
                    tick = tick
                        .fill(tok::accent(0, true))
                        .child(text("\u{2713}").font(type_::LABEL).color(t.knob));
                }
                row()
                    .key(key)
                    .role("checkbox")
                    .label(label.clone())
                    .checked(*checked)
                    .focusable(true)
                    .gap(space::S2)
                    .center_y()
                    .on_press(move |s: &mut App| {
                        s.tick(&id, path.as_deref(), on, change.as_ref(), did.clone())
                    })
                    .child(tick)
                    .child(words(label, type_::BODY, t.text))
            }
            What::Choice {
                question,
                options,
                recommended,
                multiple,
                other,
            } => self.choice(node, question, options, recommended, *multiple, *other),
            What::Unknown(kind) => el()
                .key(key)
                .padding(space::S2)
                .radius(radius::SM)
                .border(1.0, t.card_border)
                .child(
                    text(format!("Unknown component \u{201c}{kind}\u{201d}"))
                        .font(type_::LABEL)
                        .color(t.text_secondary),
                ),
        }
    }
}

impl Draw<'_> {
    /// A `Choice`: the question, one row per option with the recommended one marked, an Other
    /// field, and for several picks a Send button. A press on an option of a single choice sends
    /// it at once. Once answered, it shows the answer and takes no more presses.
    fn choice(
        &self,
        node: &Node,
        question: &str,
        options: &[ui::Opt],
        recommended: &str,
        multiple: bool,
        other: bool,
    ) -> El {
        let (t, key) = (self.t, self.key(node));
        let live = self.s.ui.get(self.id);
        let answered = live.and_then(|l| l.answered.get(&node.key));
        let picks = live.and_then(|l| l.picks.get(&node.key));
        let on = |value: &str| match answered {
            Some(answer) => answer.picked.iter().any(|o| o.value == value),
            None => picks.is_some_and(|p| p.iter().any(|v| v == value)),
        };
        let mut choice = col()
            .key(key.clone())
            .role(if multiple { "group" } else { "radiogroup" })
            .label(question.to_string())
            .padding(space::S3)
            .gap(space::S2)
            .radius(radius::MD)
            .fill(t.bg)
            .border(1.0, t.card_border)
            .child(words(question, type_::TITLE, t.text));
        for (i, option) in options.iter().enumerate() {
            let picked = on(&option.value);
            let best = !recommended.is_empty() && option.value == recommended;
            let mut mark = el()
                .w(16.0)
                .h(16.0)
                .center()
                .radius(if multiple { 4.0 } else { 8.0 })
                .border(1.0, t.card_border);
            if picked {
                let inner = if multiple {
                    text("\u{2713}").font(type_::LABEL).color(t.knob)
                } else {
                    el().w(6.0).h(6.0).radius(3.0).fill(t.knob)
                };
                mark = mark.fill(tok::accent(0, true)).child(inner);
            }
            let mut label =
                row()
                    .gap(space::S2)
                    .center_y()
                    .child(words(&option.label, type_::BODY, t.text));
            if best {
                label = label.child(
                    el().key(format!("{key}-recommended"))
                        .h(18.0)
                        .px(space::S2)
                        .center()
                        .radius(radius::SM)
                        .fill(tok::accent(0, true))
                        .child(text("Recommended").font(type_::LABEL).color(t.knob)),
                );
            }
            let mut words_col = col().grow(1.0).w(0.0).gap(2.0).child(label);
            if !option.hint.is_empty() {
                words_col = words_col.child(words(&option.hint, type_::LABEL, t.text_secondary));
            }
            let mut row_el = row()
                .key(format!("{key}-option-{i}"))
                .role(if multiple { "checkbox" } else { "radio" })
                .label(if best {
                    format!("{} (recommended)", option.label)
                } else {
                    option.label.clone()
                })
                .description(option.hint.clone())
                .checked(picked)
                .gap(space::S2)
                .center_y()
                .padding(space::S2)
                .radius(radius::SM)
                .border(
                    1.0,
                    if best {
                        tok::accent(0, true)
                    } else {
                        t.card_border
                    },
                )
                .child(mark)
                .child(words_col);
            if answered.is_some() {
                row_el = row_el.disabled(true);
                if !picked {
                    row_el = row_el.opacity(0.5);
                }
            } else {
                let (id, node_key, value) =
                    (self.id.to_string(), node.key.clone(), option.value.clone());
                row_el = row_el.focusable(true).on_press(move |s: &mut App| {
                    if multiple {
                        s.pick(&id, &node_key, &value)
                    } else {
                        s.answer(&id, &node_key, std::slice::from_ref(&value))
                    }
                });
            }
            choice = choice.child(row_el);
        }
        match answered {
            Some(answer) => {
                if !answer.other.is_empty() {
                    choice = choice.child(
                        words(
                            &format!("Your answer: {}", answer.other),
                            type_::BODY,
                            t.text,
                        )
                        .key(format!("{key}-other-sent")),
                    );
                }
                choice.child(
                    words("Sent.", type_::LABEL, t.text_secondary).key(format!("{key}-sent")),
                )
            }
            None => {
                let path = ui::other_field(&node.key);
                if other {
                    let shown = live
                        .and_then(|l| l.edits.get(&path))
                        .cloned()
                        .unwrap_or_default();
                    let (id, node_key, field) =
                        (self.id.to_string(), node.key.clone(), path.clone());
                    let picked: Vec<String> = picks.cloned().unwrap_or_default();
                    let input = text_input(&shown, type_::BODY)
                        .key(format!("{key}-other"))
                        .label("Other answer")
                        .single_line()
                        .grow(1.0)
                        .color(t.text)
                        .placeholder("Other\u{2026} (Return sends it)", t.text_secondary)
                        .on_edit(move |s: &mut App| s.view_edit(&id, &field, ""));
                    let id = self.id.to_string();
                    let input = input.on_submit(move |s: &mut App, _| {
                        let typed =
                            s.ui.get(&id)
                                .and_then(|l| l.edits.get(&ui::other_field(&node_key)))
                                .is_some_and(|e| !e.text.trim().is_empty());
                        if typed {
                            s.answer(&id, &node_key, &picked)
                        }
                    });
                    choice = choice.child(
                        row()
                            .h(32.0)
                            .px(space::S3)
                            .center_y()
                            .radius(radius::MD)
                            .fill(t.bg)
                            .border(1.0, t.card_border)
                            .child(input),
                    );
                }
                if multiple {
                    let (id, node_key) = (self.id.to_string(), node.key.clone());
                    let picked: Vec<String> = picks.cloned().unwrap_or_default();
                    let typed = live
                        .and_then(|l| l.edits.get(&path))
                        .is_some_and(|e| !e.text.trim().is_empty());
                    let mut send = el()
                        .key(format!("{key}-send"))
                        .role("button")
                        .label("Send")
                        .h(28.0)
                        .px(space::S3)
                        .center()
                        .radius(radius::SM)
                        .fill(tok::accent(0, true))
                        .child(text("Send").font(type_::LABEL).color(t.knob));
                    // Nothing picked and nothing typed is not an answer yet.
                    send = if picked.is_empty() && !typed {
                        send.disabled(true).opacity(0.5)
                    } else {
                        send.focusable(true)
                            .on_press(move |s: &mut App| s.answer(&id, &node_key, &picked))
                    };
                    choice = choice.child(row().child(send));
                }
                choice
            }
        }
    }
}

fn words(words: &str, style: TextStyle, color: rune::Color) -> El {
    text(words.to_string()).font(style).color(color).wrap()
}
