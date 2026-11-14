//! The chief of staff's tool uses in the chat, each a card drawn by the handler
//! [`tools::handler`] picked for it, and one image enlarged.
//!
//! A card's header says what ran and how it went: a status dot, the tool, its main argument
//! (a command coloured as shell, a file, a pattern or an address), how long it took, a status
//! in words and a copy button. Its body is the handler's: a command's output as a terminal
//! shows it ([`output`]), an edit as a diff ([`diff`]), a file's numbered lines, a search's
//! matches, a fetched page's summary, an image or a video.

mod diff;
mod output;

use std::time::Duration;

use rune::tok::{self, Family, TextStyle, Theme, radius, space, type_};
use rune::{Color, El, Image, Player, SurfaceFit, col, el, rich, row, text};
use rune_kit::Kit;
use rune_kit::kit::StateColors;
use rune_kit::syntax::{self, Syntax, SyntaxTheme};

use super::{button, note, small_button, warning};
use crate::tools::{self, Handler, Picture, ToolUse};
use crate::{App, Clock, Showing};

use output::Look;

/// Code, commands and their output, in the system's monospaced font (SF Mono on macOS), which
/// unlike the app's mono slot is there without bundling a font.
const CODE: TextStyle = TextStyle::new(12.0, 400.0)
    .family(Family::SystemMono)
    .lh(18.0);

/// The colours a card draws with beyond the theme's: the terminal block's background, and the
/// success and danger states.
struct Palette {
    term: Color,
    success: StateColors,
    danger: StateColors,
}

fn palette(t: Theme, dark: bool) -> Palette {
    let kit = Kit::neutral(dark);
    Palette {
        term: if dark { t.bg } else { t.bg.mix(t.card, 0.4) },
        success: kit.c.success,
        danger: kit.c.danger,
    }
}

/// Code colours for the theme. Rune's neutral theme draws a shell word or a variable a step
/// lighter than the text, which on the light terminal block falls under 4.5:1; they draw as text.
fn code_theme(t: Theme, dark: bool) -> SyntaxTheme {
    SyntaxTheme::neutral(dark).with(Syntax::Variable, t.text)
}

/// How a tool use went, as far as the chat knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Running,
    Done,
    /// Failed, with the command's exit code when it was a command.
    Failed(Option<i32>),
}

fn state(tool: &ToolUse, handler: &Handler) -> State {
    match &tool.result {
        None => State::Running,
        Some(r) if r.is_error => State::Failed(
            (*handler == Handler::Terminal)
                .then(|| tools::output(r).code)
                .flatten(),
        ),
        Some(_) => State::Done,
    }
}

/// How long a tool took, in words, when it took long enough to matter.
fn took(d: Duration) -> Option<String> {
    let secs = d.as_secs_f64();
    if secs < 0.1 {
        return None;
    }
    Some(if secs < 10.0 {
        format!("{secs:.1} s")
    } else if secs < 60.0 {
        format!("{secs:.0} s")
    } else {
        let whole = secs.round() as u64;
        format!("{} min {} s", whole / 60, whole % 60)
    })
}

/// One tool use, drawn by its handler (see [`tools::handler`]).
pub(super) fn tool_use(s: &App, i: usize, tool: &ToolUse, t: Theme, dark: bool) -> El {
    let key = format!("tool-{i}");
    let handler = tools::handler(tool);
    if handler == Handler::Ui {
        return view_card(s, &key, tool, t, dark);
    }
    let p = palette(t, dark);
    let state = state(tool, &handler);
    let failed = matches!(state, State::Failed(_));
    let body = tool.result.as_ref().map(|result| {
        let media = s.media.get(&tool.id);
        match (
            &handler,
            media.map(|m| (m.image.as_ref(), m.video.as_ref())),
        ) {
            (Handler::Image(picture), Some((image, _))) => {
                image_view(&key, tool, picture, image, t)
            }
            (Handler::Video(path), Some((_, Some(player)))) => video_view(&key, path, player, t),
            (Handler::Terminal, _) => {
                let out = tools::output(result);
                output::block(
                    &key,
                    &tool.id,
                    &out.text,
                    Look::Terminal { failed },
                    s,
                    t,
                    dark,
                )
            }
            (Handler::Diff, _) => {
                let mut body = col().gap(space::S2);
                if failed {
                    body = body.child(problem(&key, &tools::result_text(result), dark));
                }
                body.child(diff::view(&key, tool, &path_of(tool), s, t, dark))
            }
            (Handler::File, _) if !failed => {
                let words = tools::result_text(result);
                let path = path_of(tool);
                let (code, look) = match tools::numbered(&words) {
                    Some((first, code)) => {
                        let lang = syntax::lang_for_path(&path);
                        (code, Look::Code { lang, first })
                    }
                    None => (words, Look::Plain),
                };
                output::block(&key, &tool.id, &code, look, s, t, dark)
            }
            (Handler::Search, _) if !failed => {
                let (_, matches) = found(result);
                output::block(&key, &tool.id, &matches, Look::Plain, s, t, dark)
            }
            (Handler::Fetch, _) if !failed => {
                let words = tools::result_text(result);
                output::block(&key, &tool.id, &words, Look::Prose, s, t, dark)
            }
            _ if failed => problem(&key, &tools::result_text(result), dark),
            // Audio is registered for its own view next; until then it, like any other tool,
            // draws its result as text.
            _ => output::block(
                &key,
                &tool.id,
                &tools::result_text(result),
                Look::Plain,
                s,
                t,
                dark,
            ),
        }
    });
    col()
        .key(key.clone())
        .padding(space::S3)
        .gap(space::S2)
        .radius(radius::MD)
        .fill(t.card)
        .border(
            1.0,
            if failed {
                p.danger.border
            } else {
                t.card_border
            },
        )
        .child(header(s, &key, tool, &handler, state, &p, t, dark))
        .children(command(s, &key, tool, &handler, t, dark))
        .children(body)
}

/// A json-render view the chief of staff showed with `app_ui`, under its own heading.
fn view_card(s: &App, key: &str, tool: &ToolUse, t: Theme, dark: bool) -> El {
    let status = if tool.result.is_none() {
        " \u{b7} arriving\u{2026}"
    } else {
        ""
    };
    // The tool's result only says whether the model's spec was accepted; a refusal also shows
    // as the view's own note, so the result adds nothing here.
    col()
        .key(key.to_string())
        .padding(space::S3)
        .gap(space::S2)
        .radius(radius::MD)
        .fill(t.card)
        .border(1.0, t.card_border)
        .child(
            text(format!("Chief of staff's view{status}"))
                .key(format!("{key}-name"))
                .font(type_::SECTION)
                .color(t.text_secondary),
        )
        .child(super::ui::view(s, key, tool, t, dark))
}

/// A tool's error, in its own words.
fn problem(key: &str, words: &str, dark: bool) -> El {
    warning(&format!("{key}-error"), &tools::error_text(words), dark)
}

/// The file an edit or read tool named, as given.
fn path_of(tool: &ToolUse) -> String {
    ["file_path", "notebook_path", "path"]
        .iter()
        .find_map(|f| tool.input[*f].as_str())
        .unwrap_or_default()
        .to_string()
}

/// What a search found: its own count line when it gave one (`Found 3 files`), and the matches.
fn found(result: &tools::ToolResult) -> (Option<String>, String) {
    let words = tools::result_text(result);
    match words.split_once('\n') {
        Some((first, rest)) if first.starts_with("Found ") => {
            (Some(first.trim().to_string()), rest.to_string())
        }
        _ => (None, words),
    }
}

/// The card's header: status dot, tool, main argument, time, status and copy.
#[allow(clippy::too_many_arguments)]
fn header(
    s: &App,
    key: &str,
    tool: &ToolUse,
    handler: &Handler,
    state: State,
    p: &Palette,
    t: Theme,
    dark: bool,
) -> El {
    let (dot, word, ink) = match state {
        State::Running => (
            tok::accent(0, dark),
            "Running\u{2026}".to_string(),
            t.text_secondary,
        ),
        State::Done => (p.success.icon, "Done".to_string(), t.text_secondary),
        State::Failed(Some(code)) => (p.danger.icon, format!("Exit code {code}"), p.danger.fg),
        State::Failed(None) => (p.danger.icon, "Failed".to_string(), p.danger.fg),
    };
    let quiet = |words: String| text(words).font(type_::LABEL).color(t.text_secondary);
    let mut subject = row().gap(space::S2).center_y().grow(1.0).min_w(0.0);
    let mut copy = None;
    match handler {
        Handler::Terminal => {
            if let Some(what) = tool.input["description"].as_str() {
                subject = subject.child(
                    text(what.to_string())
                        .key(format!("{key}-description"))
                        .font(type_::LABEL)
                        .color(t.text)
                        .truncate(),
                );
            }
            copy = command_text(tool).map(|c| ("Copy the command", c));
        }
        Handler::Diff | Handler::File | Handler::Image(_) | Handler::Video(_) => {
            let path = path_of(tool);
            subject = subject.child(file_name(key, &path, t));
            if *handler == Handler::Diff {
                let (add, del) = diff::counts(tool);
                let counts = [(add, "+", p.success.fg), (del, "\u{2212}", p.danger.fg)]
                    .into_iter()
                    .filter(|(n, ..)| *n > 0)
                    .map(|(n, sign, ink)| text(format!("{sign}{n}")).font(type_::LABEL).color(ink));
                subject = subject.child(
                    row()
                        .key(format!("{key}-counts"))
                        .gap(space::S1)
                        .children(counts),
                );
            }
            if !path.is_empty() {
                copy = Some(("Copy the path", path));
            }
        }
        Handler::Search => {
            let pattern = tool.input["pattern"].as_str().unwrap_or_default();
            subject = subject.child(
                text(pattern.to_string())
                    .key(format!("{key}-pattern"))
                    .font(CODE)
                    .color(t.text)
                    .truncate(),
            );
            if let Some(dir) = tool.input["path"].as_str() {
                subject = subject.child(quiet(format!("in {dir}")).truncate_head());
            }
            if let Some(result) = &tool.result {
                let (count, matches) = found(result);
                let lines = matches.lines().filter(|l| !l.trim().is_empty()).count();
                let what = count.unwrap_or_else(|| super::count(lines as u32, "result"));
                subject = subject.child(quiet(what).key(format!("{key}-found")));
            }
            copy = Some(("Copy the pattern", pattern.to_string()));
        }
        Handler::Fetch => {
            let what = crate::approval::subject(&tool.name, &tool.input);
            subject = subject.child(
                text(what.clone())
                    .key(format!("{key}-summary"))
                    .font(type_::LABEL)
                    .color(t.text)
                    .truncate_middle(),
            );
            copy = Some(("Copy the address", what));
        }
        Handler::Audio | Handler::Generic | Handler::Ui => {
            subject = subject.child(
                text(tools::summary(tool))
                    .key(format!("{key}-summary"))
                    .font(CODE)
                    .color(t.text_secondary)
                    .truncate(),
            );
            copy = Some((
                "Copy the input",
                crate::approval::subject(&tool.name, &tool.input),
            ));
        }
    }
    let took = match s.clock.get(&tool.id) {
        Some(Clock::Took(d)) => took(*d),
        _ => None,
    };
    row()
        .key(format!("{key}-header"))
        .gap(space::S2)
        .center_y()
        .child(
            el().key(format!("{key}-dot"))
                .w(8.0)
                .h(8.0)
                .radius(4.0)
                .fill(dot),
        )
        .child(
            text(tool.name.clone())
                .key(format!("{key}-name"))
                .font(type_::SECTION)
                .color(t.text_secondary),
        )
        .child(subject)
        .children(took.map(|d| quiet(d).key(format!("{key}-took"))))
        .child(
            text(word)
                .key(format!("{key}-status"))
                .font(type_::LABEL)
                .color(ink),
        )
        .children(copy.map(|(label, what)| copy_button(s, &format!("{key}-copy"), label, what, t)))
}

/// A file's name, then its folder, which gives way first when the row is narrow.
fn file_name(key: &str, path: &str, t: Theme) -> El {
    let (dir, name) = match path.rsplit_once('/') {
        Some((dir, name)) => (format!("{dir}/"), name.to_string()),
        None => (String::new(), path.to_string()),
    };
    row()
        .key(format!("{key}-path"))
        .gap(space::S1)
        .center_y()
        .min_w(0.0)
        .shrink()
        .child(text(name).font(type_::TITLE).color(t.text))
        .child(
            text(dir)
                .font(type_::LABEL)
                .color(t.text_secondary)
                .truncate_head(),
        )
}

/// A command tool's command, when its input has one.
fn command_text(tool: &ToolUse) -> Option<String> {
    tool.input["command"].as_str().map(str::to_string)
}

/// A command over the output, coloured as shell after a `$` prompt: its first line, cut to the
/// width, until the captain shows all of it.
fn command(
    s: &App,
    key: &str,
    tool: &ToolUse,
    handler: &Handler,
    t: Theme,
    dark: bool,
) -> Option<El> {
    if *handler != Handler::Terminal {
        return None;
    }
    let p = palette(t, dark);
    let Some(full) = command_text(tool) else {
        // BashOutput and KillShell name a shell, not a command.
        return Some(
            text(tools::summary(tool))
                .key(format!("{key}-summary"))
                .font(CODE)
                .color(t.text_secondary)
                .truncate(),
        );
    };
    let open = s.cards.get(&tool.id).is_some_and(|c| c.full_input);
    let lines = full.lines().count();
    let long = lines > 1 || full.chars().count() > COMMAND_PREVIEW;
    let shown = if open {
        full.clone()
    } else {
        full.lines().next().unwrap_or_default().to_string()
    };
    let found = syntax::highlight(Some("bash"), &shown);
    let spans = code_theme(t, dark).spans(&found, 0);
    let code = rich(shown, spans)
        .key(format!("{key}-command"))
        .font(CODE)
        .color(t.text)
        .grow(1.0)
        .min_w(0.0);
    let code = if open {
        code.wrap_anywhere().selectable(true)
    } else {
        code.truncate()
    };
    let mut line = row()
        .gap(space::S2)
        .min_w(0.0)
        .padding(space::S2)
        .radius(radius::SM)
        .fill(p.term)
        .child(text("$").font(CODE).color(t.text_secondary))
        .child(code);
    if long {
        let id = tool.id.clone();
        let label = match (open, lines > 1) {
            (true, _) => "Show less".to_string(),
            (false, true) => format!("Show all {lines} lines"),
            (false, false) => "Show all".to_string(),
        };
        line = line.child(
            small_button(&format!("{key}-command-toggle"), &label, t, move |s| {
                let card = s.cards.entry(id.clone()).or_default();
                card.full_input = !card.full_input;
            })
            .expanded(open),
        );
    }
    Some(line)
}

/// A command longer than this many characters may not fit the header's width, so it can be
/// shown whole.
const COMMAND_PREVIEW: usize = 60;

/// A small button that copies `what` and says so until another copy button is pressed. `label`
/// is what it copies, for assistive technology.
fn copy_button(s: &App, key: &str, label: &str, what: String, t: Theme) -> El {
    let copied = s.copied.as_deref() == Some(key);
    let id = key.to_string();
    el().key(key)
        .role("button")
        .label(if copied {
            format!("{label}: copied")
        } else {
            label.to_string()
        })
        .focusable(true)
        .h(24.0)
        .px(space::S2)
        .center()
        .radius(radius::SM)
        .fill(t.control_off)
        .on_press_tx(move |s: &mut App, tx: &mut rune::Tx| {
            tx.copy(what.clone());
            s.copied = Some(id.clone());
        })
        .child(
            text(if copied { "Copied" } else { "Copy" })
                .font(type_::LABEL)
                .color(t.text),
        )
}

/// The tallest an image or video draws in the chat; it fits the column's width below that.
const MEDIA_MAX_H: f32 = 360.0;

/// An image a tool use returned or named, fitted to the column and opened larger on a press, or
/// a plain line saying why it cannot be shown.
fn image_view(
    key: &str,
    tool: &ToolUse,
    picture: &Picture,
    image: Option<&Result<Image, String>>,
    t: Theme,
) -> El {
    let what = match picture {
        Picture::File(path) => path.display().to_string(),
        Picture::Block { .. } => tools::summary(tool),
    };
    let image = match image {
        Some(Ok(image)) => image,
        Some(Err(why)) => return placeholder(key, &what, why, t),
        None => return placeholder(key, &what, "There is no image to show.", t),
    };
    if let Some(e) = image.error() {
        let why = format!("It could not be decoded: {}.", e.0.trim_end_matches('.'));
        return placeholder(key, &what, &why, t);
    }
    let id = tool.id.clone();
    picture_el(image, MEDIA_MAX_H)
        .key(format!("{key}-image"))
        .role("button")
        .label(format!("Enlarge {what}"))
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .on_press(move |s: &mut App| s.showing = Showing::Picture(id.clone()))
}

/// An image as wide as its column allows, no wider than its own pixels and no taller than
/// `max_h`. Its height follows its shape once it decodes.
pub(super) fn picture_el(image: &Image, max_h: f32) -> El {
    let mut e = el().w_full().max_h(max_h).radius(radius::SM);
    if let Some([w, _]) = image.size() {
        e = e.max_w(w as f32);
    }
    e.pixels_fit(image.pixels(), SurfaceFit::Fit)
}

fn placeholder(key: &str, what: &str, why: &str, t: Theme) -> El {
    note(
        &format!("{key}-placeholder"),
        &format!("Image: {what}. {why}"),
        t,
    )
}

/// A video file a tool use named: Rune plays it inline (AVFoundation on macOS, Media Foundation
/// on Windows), showing its first frame until it plays. Where Rune has no player the frame stays
/// a placeholder and the system's player is one press away.
fn video_view(key: &str, path: &std::path::Path, player: &Player, t: Theme) -> El {
    let (w, h) = player
        .natural_size()
        .map_or((16.0, 9.0), |(w, h)| (w.max(1) as f32, h.max(1) as f32));
    let playing = player.is_playing();
    let toggle = {
        let player = player.clone();
        small_button(
            &format!("{key}-play"),
            if playing { "Pause" } else { "Play" },
            t,
            move |_| {
                if player.is_playing() {
                    player.pause()
                } else {
                    player.play()
                }
            },
        )
    };
    let open = {
        let path = path.to_path_buf();
        small_button(
            &format!("{key}-open"),
            "Open in your video player",
            t,
            move |s| s.open_file(&path),
        )
    };
    let mut view = col()
        .gap(space::S2)
        .child(
            el().key(format!("{key}-video"))
                .role("image")
                .label(format!("Video: {}", path.display()))
                .w_full()
                .max_h(MEDIA_MAX_H)
                .aspect_ratio(w / h)
                .radius(radius::SM)
                .fill(t.bg)
                .video(player, SurfaceFit::Fit),
        )
        .child(row().gap(space::S2).child(toggle).child(open));
    if let rune::PlayerState::Failed(why) = player.state() {
        view = view.child(note(
            &format!("{key}-problem"),
            &format!("This video cannot play here ({why}). Open it in your video player."),
            t,
        ));
    }
    view.child(note(&format!("{key}-path"), &path.display().to_string(), t))
}

/// One image from the chat, as large as the middle column allows.
pub(super) fn picture_page(s: &App, id: &str, t: Theme, dark: bool) -> El {
    let tool = s.tool(id);
    let title = tool.map_or_else(|| "Image".to_string(), tools::summary);
    let image = s.media.get(id).and_then(|m| m.image.as_ref());
    let body = match image {
        Some(Ok(image)) => el()
            .key("picture")
            .grow(1.0)
            .w_full()
            .min_h(0.0)
            .pixels_fit(image.pixels(), SurfaceFit::Fit),
        _ => warning(
            "picture-missing",
            "This image is no longer available.",
            dark,
        ),
    };
    col()
        .key("picture-page")
        .grow(1.0)
        .h_full()
        .min_h(0.0)
        .padding(space::S6)
        .gap(space::S3)
        .child(
            row()
                .gap(space::S2)
                .center_y()
                .child(
                    text(title)
                        .key("picture-title")
                        .font(type_::TITLE)
                        .color(t.text)
                        .wrap_anywhere()
                        .grow(1.0),
                )
                .child(button("picture-close", "Back to chat", t, |s| {
                    s.showing = Showing::Chat
                })),
        )
        .child(body)
}
