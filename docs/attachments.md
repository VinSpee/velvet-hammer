# Attachments

The captain can attach images, videos and any other file to a message for the chief of staff.

## Attaching

- **Paste.** Command-V in the message field attaches what the clipboard holds when it holds
  files or an image without text: a screenshot copied with Control-Shift-Command-4, an image
  copied from a browser or an editor, or files copied in Finder. A Finder copy also carries each
  file's name and icon; the files attach and neither the names nor the icon do. Text pastes as
  text, including the text a spreadsheet copies beside a picture of its cells. Pasting while no
  field is focused attaches the same way, as long as the chat is showing.
- **Drag and drop.** Files dragged from Finder onto the chat attach. The chat column is outlined
  and says "Drop to attach to your message." while a drag is over it.

Each attachment shows above the message field as a chip: a thumbnail for an image, the first frame
for a video, and the file's extension for anything else, with the file's name and size and a
**×** button that removes it. **Send** sends the message with its attachments; a message may be
attachments alone, with no text.

## Limits

- A file can be up to 50 MB, and a message can carry up to 10 attachments.
- A folder cannot be attached; attach the files in it instead.
- A file that cannot be attached says why above the message field, in words such as
  "“screen-recording.mov” is 200 MB. Attachments can be up to 50 MB each." The other files in
  the same paste or drop still attach.

## Where the files go

When a file is attached, the app copies it into `attachments/` in its data directory
(`~/.velvet-hammer`, or `$VELVET_HAMMER_DATA`), each in a folder of its own that only your user
can read. A pasted image is saved there as `Pasted image.png`. The copy is what is sent and
shown, so moving or deleting the original afterwards changes nothing. Removing a chip before
sending deletes its copy; a sent attachment's copy is kept, so the message keeps showing it.

The chat history is not yet saved across restarts on this branch, so after a restart the
messages, with their attachments, are not shown again. The copies stay in `attachments/`.

## What the chief of staff gets

The message goes to Claude Code on the same `stream-json` session as any other message, as a
list of content blocks instead of plain text:

1. Each image Claude reads directly (PNG, JPEG, GIF or WebP) of up to 3.75 MB, as an `image`
   block with its bytes in base64. The Claude API takes images up to 5 MB in base64, which is
   what 3.75 MB becomes.
2. One `text` block: the captain's words, then "Attached files, saved on this computer:" and a
   line for every attachment with its saved path, name and size. An image sent as a block is
   marked "the image above".

Every other attachment (a larger image, a video, a PDF, any file) reaches the chief of staff as
its path, and it reads the file with its own tools, such as Read. The app sends the attachments
nowhere else, and does not log them.

Pressing an attachment on a sent message opens it in its default app.

## Screens

| State | Screenshot |
| --- | --- |
| A chart, a video and a Markdown file attached to a draft | [draft.png](screens/attachments/draft.png) |
| The message that carried them, and the fake `claude` saying it got one image block and two listed files | [sent.png](screens/attachments/sent.png) |
| A file over the size cap, refused with why | [too-large.png](screens/attachments/too-large.png) |
| A file drag over the chat | [drop.png](screens/attachments/drop.png) |
