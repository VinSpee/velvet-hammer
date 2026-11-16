# The conversation: find and history

Velvet Hammer keeps the conversation with the chief of staff across restarts and lets you search
all of it.

## Find

Press Cmd+F (Ctrl+F on Windows and Linux) to open the find bar over the conversation. Type to
search:

- **What it searches.** Every message the chat holds: yours, the chief of staff's, the app's
  notices and problems, each tool use (its name, main argument and result as text) and the words
  in each view the chief of staff showed. The conversation kept from earlier sessions (see
  [History](#history)) is searched too. Subagent transcripts in a peek are not.
- **How it matches.** Case does not matter, and neither do accents you leave out of the query or
  full-width forms (`viet` finds `Việt`), as a browser's find does. The chief of staff's
  messages are searched as they read, not as their Markdown is written, so `**` and link
  addresses never match.
- **Stepping.** The bar shows how many matches there are and which one is current, such as
  "3 of 12", or "No matches". Enter or **Next** goes to the next match and Shift+Enter or
  **Previous** to the one before, wrapping around at either end. Cmd+G and Shift+Cmd+G do the
  same (Ctrl+G off macOS). Typing starts again from the first match.
- **What you see.** Every match is highlighted, the current one more strongly, and the chat
  scrolls to centre the current one. A match in a tool use or a view marks its whole card.
- **Closing.** Esc or **Done** closes the bar, clears the highlights and puts the cursor back in
  the message field. Opening it again keeps the last query.

## History

The conversation is written to `history.jsonl` in the app's data directory (`~/.velvet-hammer`,
or `$VELVET_HAMMER_DATA`), one line per message once it is finished: your message when you send
it, a reply when it ends, a tool use when its result arrives.

- **What is kept.** Each message's id, who wrote it and its text. A tool use keeps its name, its
  input and its result as text. Image data is never kept: an image a tool returned shows
  "Images are not kept in the conversation history." after a restart. Any text over 64 KB is cut.
- **Who can read it.** The file is created readable by your user only (mode 0600), and the data
  directory is made readable by your user only (mode 0700) whenever the app writes to it. It
  holds what you and the chief of staff wrote, as written, so avoid putting secrets in messages.
- **Rotation.** When the file passes 8 MB, the app moves it to `history.jsonl.1` (replacing an
  older one) when it next opens and starts a new file. Only the current file is read back, and at
  most its last 2,000 messages.
- **Turning it off and clearing it.** The **Conversation history** section of Settings (Cmd+,
  on macOS) has **Keep the conversation** (recommended, the default) and **Do not keep the
  conversation**, which writes nothing new and shows nothing kept the next time the app opens.
  **Clear history** asks first, then deletes `history.jsonl` and `history.jsonl.1` and takes the
  earlier sessions off the chat; this session's messages stay on screen until you quit. The
  choice is kept in `history.json`; if that file cannot be read, nothing is kept until you
  choose again.
- **After a restart.** The kept messages show above a divider: "Earlier sessions above. Your
  chief of staff remembers the last one; link to older ones."
  Nothing kept in `history.jsonl` is sent to the chief of staff on its own.

## The chief of staff's session

While the conversation is kept, Claude Code keeps the chief of staff's session on disk too (under
`~/.claude/projects`, as Claude Code keeps any session), and the app keeps the session's id in
`session.json` in its data directory. The next launch resumes that session as soon as Claude Code
is signed in, so the chief of staff remembers the conversation after a restart or after a new
build replaced the old one (see [Install the app](../README.md#install-the-app)).

- **Firstmate's lock.** Run as firstmate, a resumed session runs firstmate's own session start
  (`bin/fm-sessionstart-run.sh`) as a hook, as a new session does, so it takes the home's lock
  and starts the watcher at once. Firstmate alone only asks the model to do that on a resume.
- **A session Claude Code cannot find.** The chat says the chief of staff could not pick up its
  earlier session, the app forgets it, and a new session takes the message that was sent.
- **Turning it off.** **Do not keep the conversation** also stops this: the app forgets the
  kept session, and every session runs with `--no-session-persistence`, so the chief of staff
  starts each one fresh. **Clear history** forgets the kept session as well; the session that is
  running carries on until the app quits.
- **Your place.** The page showing (the chat or a settings page), the draft and the files
  attached to it, and the messages queued for the chief of staff are kept in `place.json` a
  second after they change, and come back when the app opens. Queued messages then go out as
  usual. Pages about something that may be gone by then, such as a worker's terminal or a
  picture, open as the chat, and the chat opens at its newest message.

## Screenshots

| State | Screenshot |
| --- | --- |
| The find bar with four matches, the first current | [find.png](screens/conversation/find.png) |
| The third match current after stepping, scrolled into view | [find-stepped.png](screens/conversation/find-stepped.png) |
| A query with no matches | [find-none.png](screens/conversation/find-none.png) |
| After a restart: the earlier session above its divider, found by find | [history.png](screens/conversation/history.png) |
| Settings, with the conversation history section | [settings.png](screens/settings/settings.png) |
| Clear history asking before it deletes anything | [settings-clear.png](screens/settings/settings-clear.png) |
