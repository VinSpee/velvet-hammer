//! An edit tool's change as a diff: each line it removes and adds, coloured as code, with the
//! words that changed marked, folded like any long result.

use rune::tok::{Theme, radius, space};
use rune::{El, Span, col, rich, row, text};
use rune_kit::diff::{FileDiff, LineKind};
use rune_kit::syntax::{self, SyntaxTheme};

use super::output::{fold_button, folds, shown_lines};
use super::{CODE, Palette, code_theme, palette};
use crate::App;
use crate::tools::{self, Change, ToolUse};

/// Every line of a change shows: the input holds only the text around it, so there is nothing
/// to fold away between hunks.
const CONTEXT: u32 = u32::MAX / 2;

/// One line of the diff, ready to draw.
struct Row {
    kind: LineKind,
    text: String,
    spans: Vec<Span>,
}

/// The lines each change removes and adds, in order, with a gap between changes.
fn rows(path: &str, changes: &[Change], p: &Palette, theme: &SyntaxTheme) -> Vec<Option<Row>> {
    let lang = syntax::lang_for_path(path);
    let mut rows = Vec::new();
    for (n, change) in changes.iter().enumerate() {
        if n > 0 {
            rows.push(None);
        }
        let diff = FileDiff::from_texts_with_context(path, &change.old, &change.new, CONTEXT);
        let sides = [Side::new(&change.old, lang), Side::new(&change.new, lang)];
        for line in diff.hunks.iter().flat_map(|h| &h.lines) {
            let (side, number) = match line.kind {
                LineKind::Removed => (&sides[0], line.old),
                _ => (&sides[1], line.new),
            };
            let mut spans = number.map_or_else(Vec::new, |n| side.spans(n, theme));
            let mark = match line.kind {
                LineKind::Added => Some(p.success.bg.mix(p.success.border, 0.6)),
                LineKind::Removed => Some(p.danger.bg.mix(p.danger.border, 0.6)),
                LineKind::Context => None,
            };
            if let Some(mark) = mark {
                spans.extend(
                    line.changes
                        .iter()
                        .map(|r| Span::new(r.start, r.end).bg(mark, 0.0)),
                );
            }
            rows.push(Some(Row {
                kind: line.kind,
                text: line.text.clone(),
                spans,
            }));
        }
    }
    rows
}

/// One side of a change, highlighted whole so a line inside a string or comment colours as one.
struct Side {
    starts: Vec<usize>,
    text_len: usize,
    found: std::rc::Rc<[syntax::Highlight]>,
}

impl Side {
    fn new(text: &str, lang: Option<&str>) -> Side {
        let starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(at, _)| at + 1))
            .collect();
        Side {
            starts,
            text_len: text.len(),
            found: syntax::highlight(lang, text),
        }
    }

    /// The colour spans of 1-based line `n`, moved to start at the line.
    fn spans(&self, n: u32, theme: &SyntaxTheme) -> Vec<Span> {
        let i = n as usize - 1;
        let Some(&start) = self.starts.get(i) else {
            return Vec::new();
        };
        let end = self.starts.get(i + 1).map_or(self.text_len, |&e| e - 1);
        let inside: Vec<syntax::Highlight> = self
            .found
            .iter()
            .filter(|(r, _)| r.start < end && r.end > start)
            .map(|(r, paint)| (r.start.max(start) - start..r.end.min(end) - start, *paint))
            .collect();
        theme.spans(&inside, 0)
    }
}

/// How many lines the changes add and remove.
pub(super) fn counts(tool: &ToolUse) -> (usize, usize) {
    let path = tools::named_files(tool)
        .first()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    tools::changes(tool).iter().fold((0, 0), |(add, del), c| {
        let diff = FileDiff::from_texts(&path, &c.old, &c.new);
        (add + diff.additions(), del + diff.deletions())
    })
}

/// The diff of an edit tool's changes, keyed `<key>-diff`.
pub(super) fn view(key: &str, tool: &ToolUse, path: &str, s: &App, t: Theme, dark: bool) -> El {
    let p = palette(t, dark);
    let rows = rows(path, &tools::changes(tool), &p, &code_theme(t, dark));
    let card = s.cards.get(&tool.id).copied().unwrap_or_default();
    let total = rows.len();
    let shown = shown_lines(total, card.all_lines);
    let mut lines = col().key(format!("{key}-diff")).py(space::S1);
    for (n, line) in rows.iter().take(shown).enumerate() {
        let Some(line) = line else {
            lines = lines.child(
                text("\u{22ef}")
                    .font(CODE)
                    .color(t.text_secondary)
                    .px(space::S2),
            );
            continue;
        };
        let (sign, fill, ink) = match line.kind {
            LineKind::Added => ("+", Some(p.success.bg), p.success.fg),
            LineKind::Removed => ("\u{2212}", Some(p.danger.bg), p.danger.fg),
            LineKind::Context => (" ", None, t.text_secondary),
        };
        let mut r = row()
            .key(format!("{key}-diff-{n}"))
            .px(space::S2)
            .gap(space::S2)
            .child(text(sign).font(CODE).color(ink).w(10.0))
            .child(
                rich(line.text.clone(), line.spans.clone())
                    .font(CODE)
                    .color(t.text)
                    .wrap_anywhere()
                    .selectable(true)
                    .grow(1.0)
                    .min_w(0.0),
            );
        if let Some(fill) = fill {
            r = r.fill(fill);
        }
        lines = lines.child(r);
    }
    let mut view = col().gap(space::S1).child(
        col()
            .radius(radius::SM)
            .clip_rounded(radius::SM)
            .fill(p.term)
            .border(1.0, t.card_border)
            .child(lines),
    );
    if folds(total) {
        view =
            view.child(row().child(fold_button(key, &tool.id, card.all_lines, total - shown, t)));
    }
    view
}
