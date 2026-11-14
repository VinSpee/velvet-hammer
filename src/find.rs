//! Find in the conversation (Cmd+F, Ctrl+F off macOS): every message the chat holds, including
//! the history kept from earlier sessions, tool uses and the text of the views the chief of staff
//! showed. Matching folds case, width and accents the way a person expects ([`Fold`]); the chief
//! of staff's Markdown is searched as it reads, not as it is written.
//!
//! A message's text is searched exactly as the chat draws it, so each match is a range the view
//! can highlight. A tool use or a view is searched as the words on its card and highlighted as a
//! whole card.

use std::ops::Range;

use rune::TextEdit;
use rune_kit::markdown::{self, Case};
use rune_kit::search::Fold;

use crate::model::Author;
use crate::tools::{self, Handler};
use crate::{App, links, ui};

/// The find bar.
#[derive(Default)]
pub struct Find {
    pub open: bool,
    pub query: TextEdit,
    /// The current match, counted across the whole conversation.
    pub current: usize,
    /// The query the current match was chosen for.
    pub searched: String,
    /// The current match should be scrolled to on the next frame.
    pub reveal: bool,
}

/// What of a message find searches, as the chat draws it.
pub enum Searched {
    /// The chief of staff's Markdown, prepared for the view.
    Markdown(String),
    /// Words drawn as written.
    Text(String),
    /// The words on a tool use's or a view's card.
    Card(String),
}

impl Searched {
    /// The matches of `query`, each as the byte ranges it covers.
    fn find(&self, query: &str) -> Vec<Vec<Range<usize>>> {
        let fold = Fold::default();
        match self {
            Searched::Markdown(source) => markdown::find(source, query, Case::Folded(fold)),
            Searched::Text(text) | Searched::Card(text) => fold
                .find(text, query)
                .into_iter()
                .map(|r| vec![r])
                .collect(),
        }
    }
}

/// The matches in one message.
pub struct Hits {
    pub message: usize,
    pub ranges: Vec<Vec<Range<usize>>>,
    /// The overall number of this message's first match.
    pub first: usize,
    /// The message is a tool use or a view, highlighted as a whole card.
    pub card: bool,
}

impl Hits {
    /// The overall number of each match, with its ranges.
    pub fn numbered(&self) -> impl Iterator<Item = (usize, &Vec<Range<usize>>)> {
        self.ranges
            .iter()
            .enumerate()
            .map(|(n, r)| (self.first + n, r))
    }

    pub fn holds(&self, overall: usize) -> bool {
        (self.first..self.first + self.ranges.len()).contains(&overall)
    }
}

/// The key the view gives match `overall`, for highlights and scrolling.
pub fn match_key(overall: usize) -> String {
    format!("find-{overall}")
}

/// What find searches in chat message `i`.
pub fn searched(s: &App, i: usize) -> Searched {
    let m = &s.ws.chat[i];
    match (&m.tool, m.author) {
        (Some(tool), _) => Searched::Card(card_words(s, tool)),
        (None, Author::ChiefOfStaff) => {
            Searched::Markdown(crate::markdown::prepare(&links::markdown(&m.text, &s.ws)))
        }
        (None, _) => Searched::Text(links::display(&m.text, &s.ws).0),
    }
}

/// Every match of `query` in the conversation, message by message.
pub fn search(s: &App, query: &str) -> Vec<Hits> {
    let query = query.trim();
    let mut out = Vec::new();
    // A message link goes to its message instead (see [`App::follow_find`]).
    if query.is_empty() || links::parse(query).is_some() {
        return out;
    }
    let mut first = 0;
    for i in 0..s.ws.chat.len() {
        let searched = searched(s, i);
        let ranges = searched.find(query);
        if ranges.is_empty() {
            continue;
        }
        let count = ranges.len();
        out.push(Hits {
            message: i,
            ranges,
            first,
            card: matches!(searched, Searched::Card(_)),
        });
        first += count;
    }
    out
}

pub fn total(hits: &[Hits]) -> usize {
    hits.last().map_or(0, |h| h.first + h.ranges.len())
}

/// The element to scroll to for match `overall`: its highlight, or the card it is on.
pub fn target(hits: &[Hits], overall: usize) -> Option<String> {
    let hit = hits.iter().find(|h| h.holds(overall))?;
    Some(if hit.card {
        format!("tool-{}", hit.message)
    } else {
        match_key(overall)
    })
}

/// The words a tool use's card shows: its name and main argument, its result as text, and for a
/// view, every word drawn in it.
fn card_words(s: &App, tool: &tools::ToolUse) -> String {
    let mut words = vec![tool.name.clone(), tools::summary(tool)];
    if tools::handler(tool) == Handler::Ui {
        if let Ok(spec) = s.view_spec(&tool.id) {
            let state =
                s.ui.get(&tool.id)
                    .map_or(spec.state.clone(), |l| l.state(&spec));
            view_words(ui::tree(&spec, &state).root.as_ref(), &mut words);
        }
    } else if let Some(result) = &tool.result {
        words.push(tools::result_text(result));
    }
    words.retain(|w| !w.is_empty());
    words.join("\n")
}

fn view_words(node: Option<&ui::Node>, out: &mut Vec<String>) {
    let Some(node) = node else { return };
    use ui::What;
    match &node.what {
        What::Card { title, description } => out.extend([title.clone(), description.clone()]),
        What::Heading { text, .. } | What::Text { text, .. } | What::Badge { text, .. } => {
            out.push(text.clone())
        }
        What::Alert { title, message, .. } => out.extend([title.clone(), message.clone()]),
        What::Progress { label, .. } => out.push(label.clone()),
        What::Image { alt, .. } => out.push(alt.clone()),
        What::Table {
            columns,
            rows,
            caption,
        } => {
            out.extend(columns.iter().cloned());
            out.extend(rows.iter().flatten().cloned());
            out.push(caption.clone());
        }
        What::Button { label, .. } | What::Checkbox { label, .. } => out.push(label.clone()),
        What::Input { label, value, .. } => out.extend([label.clone(), value.clone()]),
        What::Unknown(name) => out.push(name.clone()),
        What::Stack { .. } | What::Separator => {}
    }
    for child in &node.children {
        view_words(Some(child), out);
    }
}

impl App {
    /// Opens the find bar over the chat and puts the cursor in it.
    pub fn open_find(&mut self, tx: &mut rune::Tx) {
        self.showing = crate::Showing::Chat;
        self.find.open = true;
        self.find.reveal = true;
        tx.focus("find-input", true);
    }

    /// Closes the find bar, clearing its highlights, and returns to the message field.
    pub fn close_find(&mut self, tx: &mut rune::Tx) {
        self.find.open = false;
        tx.focus("chat-input", true);
    }

    /// Moves to the next match (`by` 1) or the previous one (-1), wrapping around.
    pub fn step_find(&mut self, by: isize) {
        let total = total(&search(self, &self.find.query.text)) as isize;
        if total == 0 {
            return;
        }
        self.find.current = (self.find.current as isize + by).rem_euclid(total) as usize;
        self.find.reveal = true;
    }

    /// On each frame: a changed query starts again from its first match, and the current match
    /// is scrolled to when it moved. True when anything changed.
    pub fn follow_find(&mut self, tx: &mut rune::Tx) -> bool {
        if !self.find.open {
            return false;
        }
        let mut changed = false;
        if self.find.query.text != self.find.searched {
            self.find.searched = self.find.query.text.clone();
            self.find.current = 0;
            self.find.reveal = true;
            changed = true;
            // A message link pasted into the bar goes to its message.
            if let Some(id) = links::parse(self.find.searched.trim()) {
                let url = links::url(id);
                self.open_link(&url, tx);
            }
        }
        if std::mem::take(&mut self.find.reveal) {
            let hits = search(self, &self.find.query.text);
            if let Some(key) = target(&hits, self.find.current) {
                tx.scroll_center(key);
            }
        }
        changed
    }
}
