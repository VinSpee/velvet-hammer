//! Attachments in the chat: the chips above the message field for files the captain pasted or
//! dropped and has not sent yet, and the same files on the message that carried them.

use rune::tok::{self, Theme, radius, space, type_};
use rune::{El, SurfaceFit, col, el, row, text};

use super::{small_button, warning};
use crate::App;
use crate::attach::{Attachment, Kind};

/// The side of a chip's thumbnail.
const THUMB: f32 = 40.0;

/// The draft's attachments, each removable, and why the last one could not be attached.
pub(super) fn tray(s: &App, t: Theme, dark: bool) -> Option<El> {
    if s.attached.is_empty() && s.attach_problem.is_none() && !s.drop_over {
        return None;
    }
    let mut chips = row().key("attachments").gap(space::S2).flex_wrap();
    for (i, a) in s.attached.iter().enumerate() {
        let key = format!("attachment-{i}");
        let remove = small_button(&format!("{key}-remove"), "\u{d7}", t, move |s: &mut App| {
            s.remove_attachment(i)
        })
        .label(format!("Remove {}", a.name));
        chips = chips.child(chip(s, &key, a, t).child(remove));
    }
    Some(
        col()
            .gap(space::S2)
            .children(s.drop_over.then(|| {
                text("Drop to attach to your message.")
                    .key("attach-drop-hint")
                    .font(type_::LABEL)
                    .color(tok::accent(0, true))
            }))
            .children((!s.attached.is_empty()).then_some(chips))
            .children(
                s.attach_problem
                    .as_ref()
                    .map(|why| warning("attach-problem", why, dark)),
            ),
    )
}

/// The attachments a sent message carried. Pressing one opens it in its default app.
pub(super) fn sent(s: &App, key: &str, list: &[Attachment], t: Theme) -> El {
    let mut chips = row().key(key.to_string()).gap(space::S2).flex_wrap();
    for (i, a) in list.iter().enumerate() {
        let path = a.path.clone();
        chips = chips.child(
            chip(s, &format!("{key}-{i}"), a, t)
                .role("button")
                .label(format!("Open {}", a.label()))
                .focusable(true)
                .cursor(rune::Cursor::Pointer)
                .on_press(move |s: &mut App| s.open_file(&path)),
        );
    }
    chips
}

/// A thumbnail, poster frame or file badge, then the file's name and size.
fn chip(s: &App, key: &str, a: &Attachment, t: Theme) -> El {
    row()
        .key(key.to_string())
        .max_w(280.0)
        .gap(space::S2)
        .padding(space::S1)
        .center_y()
        .radius(radius::MD)
        .fill(t.card)
        .border(1.0, t.card_border)
        .child(thumbnail(s, key, a, t))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .child(
                    text(a.name.clone())
                        .key(format!("{key}-name"))
                        .font(type_::LABEL)
                        .color(t.text)
                        .truncate_middle(),
                )
                .child(
                    text(crate::attach::size(a.bytes))
                        .key(format!("{key}-size"))
                        .font(type_::LABEL)
                        .color(t.text_secondary),
                ),
        )
}

fn thumbnail(s: &App, key: &str, a: &Attachment, t: Theme) -> El {
    let frame = el()
        .key(format!("{key}-thumb"))
        .size(THUMB, THUMB)
        .radius(radius::SM)
        .fill(t.bg);
    let media = s.attachment_media.get(&a.path);
    match (a.kind, media) {
        (Kind::Image, Some(m)) => match &m.image {
            Some(Ok(image)) if image.error().is_none() => frame
                .role("image")
                .label(format!("Image: {}", a.name))
                .pixels_fit(image.pixels(), SurfaceFit::Fill),
            _ => badge(frame, "IMG", t),
        },
        (Kind::Video, Some(m)) => match &m.video {
            Some(player) => frame
                .role("image")
                .label(format!("Video: {}", a.name))
                .video(player, SurfaceFit::Fill),
            None => badge(frame, "VID", t),
        },
        _ => {
            let ext = a
                .path
                .extension()
                .map(|e| e.to_string_lossy().to_uppercase())
                .filter(|e| e.len() <= 4)
                .unwrap_or_else(|| "FILE".into());
            badge(frame, &ext, t)
        }
    }
}

/// A file's extension in place of a picture.
fn badge(frame: El, label: &str, t: Theme) -> El {
    frame.center().border(1.0, t.card_border).child(
        text(label.to_string())
            .font(type_::LABEL.mono())
            .color(tok::accent(0, true)),
    )
}
