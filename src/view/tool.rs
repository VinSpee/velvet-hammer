//! The chief of staff's tool uses in the chat, each drawn by the handler [`tools::handler`]
//! picked for it, and one image enlarged.

use rune::tok::{self, Theme, radius, space, type_};
use rune::{El, Image, Player, ScrollSpec, SurfaceFit, col, el, row, text};

use super::{DETAIL_MAX_H, button, count, note, small_button, warning};
use crate::tools::{self, Handler, Picture, ToolUse};
use crate::{App, Showing};

/// One tool use, drawn by its handler (see [`tools::handler`]). Audio, diff and terminal have no
/// view of their own yet, so they draw as the generic one.
pub(super) fn tool_use(s: &App, i: usize, tool: &ToolUse, t: Theme, dark: bool) -> El {
    let key = format!("tool-{i}");
    let status = match &tool.result {
        None => " \u{b7} running\u{2026}",
        Some(r) if r.is_error => " \u{b7} failed",
        Some(_) => "",
    };
    let card = col()
        .key(key.clone())
        .padding(space::S3)
        .gap(space::S2)
        .radius(radius::MD)
        .fill(t.card)
        .border(1.0, t.card_border)
        .child(
            row()
                .gap(space::S2)
                .child(
                    text(format!("{}{status}", tool.name))
                        .key(format!("{key}-name"))
                        .font(type_::SECTION)
                        .color(t.text_secondary),
                )
                .child(
                    text(tools::summary(tool))
                        .key(format!("{key}-summary"))
                        .font(type_::LABEL.mono())
                        .color(t.text_secondary)
                        .wrap_anywhere()
                        .grow(1.0),
                ),
        );
    let Some(result) = &tool.result else {
        return card;
    };
    let media = s.media.get(&tool.id);
    let body = match media.map(|m| (&m.handler, m.image.as_ref(), m.video.as_ref())) {
        Some((Handler::Image(picture), image, _)) => image_view(&key, tool, picture, image, t),
        Some((Handler::Video(path), _, Some(player))) => video_view(&key, path, player, t),
        // Audio, diff and terminal are registered for their own views next; until then they,
        // like any other tool, draw as the generic handler.
        _ => folded_result(&key, &tool.id, result, s, t, dark),
    };
    card.child(body)
}

/// A tool's result as text, folded behind a button so a long one does not fill the chat.
fn folded_result(
    key: &str,
    id: &str,
    result: &tools::ToolResult,
    s: &App,
    t: Theme,
    dark: bool,
) -> El {
    let words = tools::result_text(result);
    if words.trim().is_empty() {
        return note(&format!("{key}-empty"), "No output.", t);
    }
    let open = s.unfolded.contains(id);
    let lines = words.lines().count();
    let toggle = {
        let id = id.to_string();
        small_button(
            &format!("{key}-toggle"),
            &if open {
                "Hide the result".to_string()
            } else {
                format!("Show the result ({})", count(lines as u32, "line"))
            },
            t,
            move |s| {
                if !s.unfolded.remove(&id) {
                    s.unfolded.insert(id.clone());
                }
            },
        )
    };
    let mut body = col().gap(space::S1).child(row().child(toggle));
    if open {
        body = body.child(
            col()
                .key(format!("{key}-result"))
                .max_h(DETAIL_MAX_H)
                .padding(space::S2)
                .radius(radius::SM)
                .fill(t.bg)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(
                    text(words)
                        .font(type_::LABEL.mono())
                        .color(if result.is_error {
                            tok::accent(3, dark)
                        } else {
                            t.text
                        })
                        .wrap_anywhere()
                        .selectable(true),
                ),
        );
    }
    body
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
fn picture_el(image: &Image, max_h: f32) -> El {
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
