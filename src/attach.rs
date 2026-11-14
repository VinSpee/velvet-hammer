//! Files the captain attaches to a message: images, videos and any other file, pasted or dropped
//! on the chat. Each is copied into `attachments/` in the app's data directory the moment it is
//! attached, so a pasted screenshot has a file at all and a dropped file stays readable after the
//! original moves. Removing an attachment before sending deletes its copy; a sent one is kept
//! with the message.
//!
//! The chief of staff gets them the ways Claude Code takes them: an image small enough for the
//! model goes as image content, and every other attachment goes as its path, which the chief of
//! staff reads with its own tools. Nothing is uploaded or logged anywhere else.

use std::fs;
use std::path::{Path, PathBuf};

use crate::approval::create_private_dir;
use crate::tools;

/// The folder in the data directory that holds the copies.
pub const DIR: &str = "attachments";
/// The largest file the captain can attach.
pub const MAX_BYTES: u64 = 50 * 1024 * 1024;
/// The most attachments one message carries.
pub const MAX_PER_MESSAGE: usize = 10;
/// The largest image sent as image content. The Claude API takes an image up to 5 MB as base64,
/// which this many bytes become; a larger image goes as its path instead.
pub const MAX_INLINE_IMAGE_BYTES: u64 = 3_750_000;
/// What a pasted image is called; the clipboard gives no name.
pub const PASTED_IMAGE: &str = "Pasted image.png";

/// What an attachment is, by its file's extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Image,
    Video,
    File,
}

/// One attached file: the app's own copy of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    /// The copy in the data directory.
    pub path: PathBuf,
    /// The name the captain knows it by.
    pub name: String,
    pub bytes: u64,
    pub kind: Kind,
}

impl Attachment {
    /// The media type of an image or video, by its extension.
    pub fn media_type(&self) -> Option<&'static str> {
        tools::extension_type(&self.path)
    }

    /// The media type to send this image as image content, when it is one the model reads
    /// and small enough; `None` when it goes as its path.
    pub fn inline_type(&self) -> Option<&'static str> {
        let media = self.media_type()?;
        let readable = matches!(
            media,
            "image/png" | "image/jpeg" | "image/gif" | "image/webp"
        );
        (readable && self.bytes <= MAX_INLINE_IMAGE_BYTES).then_some(media)
    }

    /// The name and size, such as "notes.txt, 12 KB".
    pub fn label(&self) -> String {
        format!("{}, {}", self.name, size(self.bytes))
    }
}

/// Copies the file at `source` into `data`'s attachments folder.
pub fn file(data: &Path, source: &Path) -> Result<Attachment, String> {
    let name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| source.display().to_string());
    let meta = fs::metadata(source)
        .map_err(|e| format!("\u{201c}{name}\u{201d} could not be read: {e}."))?;
    if meta.is_dir() {
        return Err(format!(
            "\u{201c}{name}\u{201d} is a folder. Attach the files in it instead."
        ));
    }
    if !meta.is_file() {
        return Err(format!("\u{201c}{name}\u{201d} is not a file."));
    }
    within_cap(&name, meta.len())?;
    let path = slot(data, &name)?;
    let bytes = fs::copy(source, &path)
        .map_err(|e| format!("\u{201c}{name}\u{201d} could not be copied: {e}."))?;
    Ok(Attachment {
        kind: kind(&path),
        path,
        name,
        bytes,
    })
}

/// Saves a pasted image's PNG bytes into `data`'s attachments folder.
pub fn png(data: &Path, png: &[u8]) -> Result<Attachment, String> {
    within_cap(PASTED_IMAGE, png.len() as u64)?;
    let path = slot(data, PASTED_IMAGE)?;
    fs::write(&path, png).map_err(|e| format!("The pasted image could not be saved: {e}."))?;
    Ok(Attachment {
        path,
        name: PASTED_IMAGE.into(),
        bytes: png.len() as u64,
        kind: Kind::Image,
    })
}

/// Deletes the copy of an attachment the captain removed before sending.
pub fn discard(attachment: &Attachment) {
    let _ = fs::remove_file(&attachment.path);
    if let Some(folder) = attachment.path.parent() {
        let _ = fs::remove_dir(folder);
    }
}

/// How many attachments, such as "1 attachment" or "3 attachments".
pub fn count(list: &[Attachment]) -> String {
    match list.len() {
        1 => "1 attachment".into(),
        n => format!("{n} attachments"),
    }
}

/// A size in bytes as the captain reads it: "512 bytes", "12 KB", "3.4 MB".
pub fn size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} bytes")
    } else if b < KB * KB {
        format!("{:.0} KB", b / KB)
    } else {
        let mb = format!("{:.1}", b / (KB * KB));
        format!("{} MB", mb.trim_end_matches(".0"))
    }
}

fn within_cap(name: &str, bytes: u64) -> Result<(), String> {
    if bytes > MAX_BYTES {
        return Err(format!(
            "\u{201c}{name}\u{201d} is {}. Attachments can be up to {} each.",
            size(bytes),
            size(MAX_BYTES)
        ));
    }
    Ok(())
}

fn kind(path: &Path) -> Kind {
    match tools::extension_type(path) {
        Some(m) if m.starts_with("image/") => Kind::Image,
        Some(m) if m.starts_with("video/") => Kind::Video,
        _ => Kind::File,
    }
}

/// A new folder of its own for one attachment, so two files with the same name never meet, and
/// the path the copy goes to in it. The folders are readable by the captain only.
fn slot(data: &Path, name: &str) -> Result<PathBuf, String> {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).map_err(|e| format!("No folder could be made for it: {e}."))?;
    let id: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let folder = data.join(DIR).join(id);
    create_private_dir(&folder).map_err(|e| format!("No folder could be made for it: {e}."))?;
    // Only the last part of the name, so a name can never point outside the folder.
    let name = Path::new(name)
        .file_name()
        .map_or_else(|| "attachment".into(), |n| n.to_os_string());
    Ok(folder.join(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static RUN: AtomicU32 = AtomicU32::new(0);
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "target/attach-tests/{}-{}",
            std::process::id(),
            RUN.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_file_is_copied_into_the_data_directory_and_named_by_kind() {
        let data = data();
        let source = data.join("clip.MOV");
        fs::write(&source, b"not really a movie").unwrap();
        let a = file(&data, &source).unwrap();
        assert_eq!(a.kind, Kind::Video);
        assert_eq!(a.name, "clip.MOV");
        assert_eq!(a.bytes, 18);
        assert!(a.path.starts_with(data.join(DIR)));
        assert_eq!(fs::read(&a.path).unwrap(), b"not really a movie");
        fs::remove_file(&source).unwrap();
        assert!(a.path.exists(), "the copy outlives the original");
        discard(&a);
        assert!(!a.path.exists());
        assert!(!a.path.parent().unwrap().exists());
    }

    #[test]
    fn folders_and_files_over_the_cap_are_refused_with_why() {
        let data = data();
        let folder = data.join("photos");
        fs::create_dir(&folder).unwrap();
        assert!(file(&data, &folder).unwrap_err().contains("is a folder"));
        let big = data.join("huge.bin");
        fs::File::create(&big)
            .unwrap()
            .set_len(MAX_BYTES + 1024 * 1024)
            .unwrap();
        let why = file(&data, &big).unwrap_err();
        assert_eq!(
            why,
            "\u{201c}huge.bin\u{201d} is 51 MB. Attachments can be up to 50 MB each."
        );
        assert!(!data.join(DIR).exists() || fs::read_dir(data.join(DIR)).unwrap().count() == 0);
    }

    #[test]
    fn only_small_images_the_model_reads_go_as_image_content() {
        let shot = |name: &str, bytes| Attachment {
            path: PathBuf::from(format!("/a/{name}")),
            name: name.into(),
            bytes,
            kind: kind(Path::new(name)),
        };
        assert_eq!(shot("a.png", 1000).inline_type(), Some("image/png"));
        assert_eq!(shot("a.JPG", 1000).inline_type(), Some("image/jpeg"));
        assert_eq!(
            shot("a.png", MAX_INLINE_IMAGE_BYTES + 1).inline_type(),
            None
        );
        assert_eq!(shot("a.heic", 1000).inline_type(), None);
        assert_eq!(shot("a.mp4", 1000).inline_type(), None);
        assert_eq!(shot("a.txt", 1000).kind, Kind::File);
    }

    #[test]
    fn sizes_read_plainly() {
        assert_eq!(size(512), "512 bytes");
        assert_eq!(size(12 * 1024), "12 KB");
        assert_eq!(size(3_565_158), "3.4 MB");
        assert_eq!(size(MAX_BYTES), "50 MB");
    }
}
