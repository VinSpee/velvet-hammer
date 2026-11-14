//! A tool's text result in a block that folds: a command's output as a terminal shows it, a
//! file's lines with their numbers, a search's matches, or prose. A long result shows its first
//! lines and a control for the rest; a very long one stops at [`MAX_LINES`] so the chat stays
//! quick, and its copy button still copies all of it.

use rune::tok::{Theme, radius, space, type_};
use rune::{Color, El, ScrollSpec, Span, col, rich, row, text};
use rune_kit::syntax;

use super::{CODE, code_theme, copy_button, palette};
use crate::ansi;
use crate::view::{count, note, small_button};
use crate::{App, Card};

/// The lines a folded result shows.
const PREVIEW_LINES: usize = 8;
/// A result at most this many lines longer than the preview shows whole: folding away a line or
/// two saves nothing.
const FOLD_SLACK: usize = 3;
/// The most lines a result shows unfolded; copying it gets the rest.
const MAX_LINES: usize = 2_000;
/// The most bytes a result shows unfolded, for results of a few enormous lines.
const MAX_BYTES: usize = 256 * 1024;
/// The tallest an unfolded result grows before it scrolls.
const MAX_H: f32 = 360.0;
/// A line longer than this many characters may wrap, so the block offers to stop wrapping.
const LONG_LINE: usize = 72;

/// How a block draws its text.
pub(super) enum Look<'a> {
    /// A command's output, with the colours its escape sequences ask for; `failed` when the
    /// command failed, which the block's border marks.
    Terminal { failed: bool },
    /// A file's code from line `first`, coloured as `lang` when the highlighter knows it.
    Code { lang: Option<&'a str>, first: u32 },
    /// Monospaced text as written: search results, any other tool's result.
    Plain,
    /// Markdown, as the chief of staff's messages draw it: a fetched page's summary.
    Prose,
}

impl Look<'_> {
    fn mono(&self) -> bool {
        !matches!(self, Look::Prose)
    }

    /// Whether long lines wrap. Code never does, so its lines stay beside their numbers;
    /// output and search results do until the captain turns it off.
    fn wraps(&self, card: &Card) -> bool {
        match self {
            Look::Code { .. } => false,
            Look::Prose => true,
            Look::Terminal { .. } | Look::Plain => !card.no_wrap,
        }
    }
}

/// The part of `text` to draw: up to `limit` lines and [`MAX_BYTES`].
fn head(text: &str, limit: usize) -> &str {
    let mut end = text
        .match_indices('\n')
        .nth(limit.saturating_sub(1))
        .map_or(text.len(), |(at, _)| at);
    if end > MAX_BYTES {
        end = MAX_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
    }
    &text[..end]
}

/// Whether a `total`-line result folds to its first lines.
pub(super) fn folds(total: usize) -> bool {
    total > PREVIEW_LINES + FOLD_SLACK
}

/// The button that shows the `hidden` lines of a folded result, or folds it again when `all`
/// show, keyed `<key>-more`.
pub(super) fn fold_button(key: &str, id: &str, all: bool, hidden: usize, t: Theme) -> El {
    let id = id.to_string();
    let label = if all {
        "Show fewer lines".to_string()
    } else if hidden == 1 {
        "Show 1 more line".to_string()
    } else {
        format!("Show {hidden} more lines")
    };
    small_button(&format!("{key}-more"), &label, t, move |s| {
        let card = s.cards.entry(id.clone()).or_default();
        card.all_lines = !card.all_lines;
    })
    .expanded(all)
}

/// How many lines of a `total`-line result a block shows: all of them up to [`MAX_LINES`]
/// when the captain unfolded it or it is short, else [`PREVIEW_LINES`].
pub(super) fn shown_lines(total: usize, all: bool) -> usize {
    if all || !folds(total) {
        total.min(MAX_LINES)
    } else {
        PREVIEW_LINES
    }
}

/// `text`, a tool use's result, in a block keyed `<key>-output`, with its controls under it.
pub(super) fn block(
    key: &str,
    id: &str,
    result: &str,
    look: Look,
    s: &App,
    t: Theme,
    dark: bool,
) -> El {
    let result = result.trim_end_matches(['\n', '\r']);
    if result.trim().is_empty() {
        return note(&format!("{key}-empty"), "No output.", t);
    }
    let card = s.cards.get(id).copied().unwrap_or_default();
    let total = result.lines().count();
    let shown = shown_lines(total, card.all_lines);
    let part = head(result, shown);
    let wrap = look.wraps(&card);
    let p = palette(t, dark);
    let body = match look {
        Look::Terminal { .. } => {
            let styled = ansi::parse(part);
            let spans = ansi_spans(&styled.runs, t, dark);
            words(styled.text, spans, wrap, t)
        }
        Look::Code { lang, first } => {
            let found = syntax::highlight(lang, part);
            let spans = code_theme(t, dark).spans(&found, 0);
            let last = first as usize + part.lines().count().max(1) - 1;
            let numbers: Vec<String> = (first as usize..=last).map(|n| n.to_string()).collect();
            row()
                .gap(space::S3)
                .child(
                    text(numbers.join("\n"))
                        .key(format!("{key}-numbers"))
                        .font(CODE)
                        .color(t.text_secondary.mix(p.term, 0.35)),
                )
                .child(words(part.to_string(), spans, wrap, t).grow(1.0).min_w(0.0))
        }
        Look::Plain => words(part.to_string(), Vec::new(), wrap, t),
        Look::Prose => super::super::markdown(part, false, dark),
    };
    let mut frame = col()
        .key(format!("{key}-output"))
        .padding(space::S2)
        .radius(radius::SM)
        .fill(if look.mono() { p.term } else { t.card })
        .border(
            1.0,
            if matches!(look, Look::Terminal { failed: true }) {
                p.danger.border
            } else {
                t.card_border
            },
        );
    if card.all_lines && shown > PREVIEW_LINES {
        frame = frame.max_h(MAX_H).scroll(ScrollSpec {
            stick_bottom: false,
        });
    }
    frame = if wrap {
        frame.child(body)
    } else {
        frame.child(col().scroll_x().child(body))
    };
    col()
        .gap(space::S1)
        .child(frame)
        .child(controls(s, key, id, &look, result, part, t))
}

/// The row under a block whose result is `result`, `part` of it drawn: how long it is, the
/// fold, wrapping and copying.
fn controls(s: &App, key: &str, id: &str, look: &Look, result: &str, part: &str, t: Theme) -> El {
    let card = s.cards.get(id).copied().unwrap_or_default();
    let total = result.lines().count();
    let shown = shown_lines(total, card.all_lines);
    let length = if card.all_lines && shown < total {
        format!("Showing {shown} of {total} lines. Copy the output for all of it.")
    } else if look.mono() {
        count(total as u32, "line")
    } else {
        String::new()
    };
    let mut bar = row().gap(space::S2).center_y().child(
        text(length)
            .key(format!("{key}-lines"))
            .font(type_::LABEL)
            .color(t.text_secondary)
            .grow(1.0)
            .wrap(),
    );
    if folds(total) {
        bar = bar.child(fold_button(key, id, card.all_lines, total - shown, t));
    }
    if matches!(look, Look::Terminal { .. } | Look::Plain)
        && part.lines().any(|l| l.chars().count() > LONG_LINE)
    {
        let id = id.to_string();
        bar = bar.child(
            small_button(
                &format!("{key}-wrap"),
                if card.no_wrap {
                    "Wrap lines"
                } else {
                    "Don't wrap lines"
                },
                t,
                move |s| {
                    let card = s.cards.entry(id.clone()).or_default();
                    card.no_wrap = !card.no_wrap;
                },
            )
            .checked(!card.no_wrap),
        );
    }
    let plain = match look {
        Look::Terminal { .. } => ansi::parse(result).text,
        _ => result.to_string(),
    };
    bar.child(copy_button(
        s,
        &format!("{key}-copy-output"),
        "Copy the output",
        plain,
        t,
    ))
}

/// Monospaced text with `spans`, wrapping anywhere or running on, and selectable.
fn words(content: String, spans: Vec<Span>, wrap: bool, t: Theme) -> El {
    let e = if wrap {
        rich(content, spans).wrap_anywhere()
    } else {
        text(content).spans(spans)
    };
    e.font(CODE).color(t.text).selectable(true)
}

/// Spans drawing ANSI runs in the theme's terminal colours.
fn ansi_spans(runs: &[(std::ops::Range<usize>, ansi::Style)], t: Theme, dark: bool) -> Vec<Span> {
    let p = palette(t, dark);
    runs.iter()
        .map(|(range, style)| {
            let mut fg = style.fg.map(|c| ansi_color(c, dark));
            let mut bg = style.bg.map(|c| ansi_color(c, dark));
            if style.inverse {
                (fg, bg) = (Some(bg.unwrap_or(p.term)), Some(fg.unwrap_or(t.text)));
            }
            if style.dim {
                fg = Some(fg.unwrap_or(t.text).mix(p.term, 0.4));
            }
            let mut span = Span::new(range.start, range.end);
            span.color = fg;
            if let Some(bg) = bg {
                span = span.bg(bg, 0.0);
            }
            if style.bold {
                span = span.bold();
            }
            if style.italic {
                span = span.italic();
            }
            if style.underline {
                span = span.underline(fg.unwrap_or(t.text));
            }
            span
        })
        .collect()
}

/// An ANSI colour for the theme. The sixteen named ones keep their hues at a lightness that
/// reads on the terminal block in either theme; the cube, the greys and true colours are what
/// the program asked for.
fn ansi_color(color: ansi::Color, dark: bool) -> Color {
    let rgb = |r: u8, g: u8, b: u8| {
        let c = |v: u8| rune::color::srgb_decode(v as f32 / 255.0);
        Color {
            r: c(r),
            g: c(g),
            b: c(b),
            a: 1.0,
        }
    };
    match color {
        ansi::Color::Rgb(r, g, b) => rgb(r, g, b),
        ansi::Color::Indexed(n @ 0..=15) => {
            let bright = n >= 8;
            let (l, grey) = match (dark, bright) {
                (true, false) => (0.72, [0.5, 0.86]),
                (true, true) => (0.82, [0.62, 0.97]),
                (false, false) => (0.5, [0.25, 0.55]),
                (false, true) => (0.44, [0.4, 0.35]),
            };
            match n % 8 {
                0 => rune::oklch(grey[0], 0.0, 0.0),
                7 => rune::oklch(grey[1], 0.0, 0.0),
                hue => {
                    let h = [0.0, 25.0, 145.0, 85.0, 255.0, 330.0, 200.0][hue as usize];
                    let chroma = if hue == 3 && !dark { 0.12 } else { 0.15 };
                    rune::oklch(l, chroma, h)
                }
            }
        }
        ansi::Color::Indexed(n) => {
            let (r, g, b) = ansi::cube(n).unwrap_or((128, 128, 128));
            rgb(r, g, b)
        }
    }
}
