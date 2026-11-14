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

The copy runs on a background thread, one per paste or drop, so a large video does not freeze the
window. Until its copy finishes, a file shows as a chip with its name and "Adding…" and no
thumbnail, and **Send** does nothing (it is dimmed). If the copy fails, the chip says why, in
words such as "Permission denied", and its **×** removes it; a failed chip does not hold up
**Send**, and sending clears it. Removing a chip that is still being added deletes the copy when
it finishes.

## Limits

- A file can be up to 50 MB, and a message can carry up to 10 attachments.
- A folder cannot be attached; attach the files in it instead.
- The size, folder and attachment-count limits are checked before any copy starts, and an
  attachment still being added counts toward the 10.
- A file refused by those limits says why above the message field, in words such as
  "“screen-recording.mov” is 200 MB. Attachments can be up to 50 MB each." The other files in
  the same paste or drop still attach.

## Where the files go

When a file is attached, the app copies it into `attachments/` in its data directory
(`~/.velvet-hammer`, or `$VELVET_HAMMER_DATA`), each in a folder of its own that only your user
can read. A pasted image is saved there as `Pasted image.png`. The copy is what is sent and
shown, so moving or deleting the original afterwards changes nothing. Removing a chip before
sending deletes its copy; a sent attachment's copy is kept, so the message keeps showing it.

When the conversation is kept across restarts (see [The conversation](conversation.md)), each
message keeps its attachments: `history.jsonl` records the path of each copy with its name, size
and kind, never the file's bytes, and after a restart the message shows its chips again from the
copies in `attachments/`. **Clear history** also deletes the copies attached to the messages it
clears. A draft's attachments, like a queued message's, are not kept: after quitting, their
copies stay in `attachments/` unused.

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
| A file still being added, one whose copy failed, and Send waiting | [adding.png](screens/attachments/adding.png) |
| A file drag over the chat | [drop.png](screens/attachments/drop.png) |
