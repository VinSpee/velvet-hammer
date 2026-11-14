# Message links

Every message in the chat has a link, and a link in a message takes you to the message it names.
When you put links in a message to the chief of staff, it gets the linked messages quoted, so
"do what this answer says: <link>" works.

## The link form

```
velvet-hammer://message/<id>
```

`<id>` is the message's id: 12 lowercase hexadecimal digits, chosen at random when the message is
made, such as `velvet-hammer://message/3f9a01bc22de`. The id is kept with the message in the
conversation history (see [docs/conversation.md](conversation.md)), so a link keeps working after
a restart for as long as the message is kept. The app reads any id of 1 to 64 letters, digits,
`-` and `_`, so the form can grow without breaking old links.

## Copying a link

Point at a message, or give its button keyboard focus (Tab reaches buttons when the system's
keyboard navigation is on), and **Copy link** appears in its top corner. It puts
the message's link on the clipboard and says **Link copied**.

## Opening a link

- **In a message.** A link in any message, yours or the chief of staff's, shows as a chip with
  who wrote the message it names and the start of its first line, such as
  "↪ Chief of staff: Plan: merge the docs PR first…". Press it to go there: the chat scrolls to
  the message and highlights it for a moment. A link to a message that is not in the chat shows
  as "⚠ Message not found", and pressing it says so above the message field.
- **Pasted into find.** Paste a link into the find bar (Cmd+F) to go to its message.
- **From outside the app.** The app takes links the system opens with it (Rune's
  `App::on_open_url`) and goes to the message the same way. This needs the operating system to
  know that Velvet Hammer opens `velvet-hammer://` links, which it does not yet: see
  [Registering the scheme](#registering-the-scheme).

A link from outside the app is only parsed: anything that is not a message link opens in the
browser if it is an `http`, `https` or `mailto` link, and does nothing otherwise.

## Links in messages to the chief of staff

When your message contains message links, the chief of staff receives your words as you wrote
them, then each linked message quoted:

```
do what this answer says: velvet-hammer://message/3f9a01bc22de

[Linked messages, quoted by Velvet Hammer]
<<< velvet-hammer://message/3f9a01bc22de, written by you, the chief of staff:
Plan: merge the docs PR first, then tag the release.
>>>
```

- Each quote names who wrote the message: the captain, the chief of staff, Velvet Hammer, Claude
  Code reporting a problem, or one of the chief of staff's tool uses (its name, main argument and
  result as text).
- A quote is cut at 4,000 characters, and at most five links in one message are quoted; the rest
  are counted.
- A link to a message that is not in the chat is passed on as
  `<<< velvet-hammer://message/<id>: message not found. No message in this chat has this link. >>>`.
- Each of your messages ends with a line giving its own link and the link of the chief of
  staff's last reply in this session, so the chief of staff can link to them:
  `[Velvet Hammer: this message's link is velvet-hammer://message/… . Your last reply's link is …]`.
- Once per session, with your first message, the app's prompt hook tells the chief of staff how
  links work ([`links::INSTRUCTION`](../src/links.rs)), the same way it adds the
  [visual replies](visual-replies.md) instruction.

Nothing else from the conversation is sent. A message kept from an earlier session reaches the
chief of staff only when you link to it.

The chief of staff's own replies can contain links too. A bare link in a reply draws as the same
chip and goes to its message; a link it writes inside code stays as written.

## Registering the scheme

Not done yet, because the repository builds a plain binary with `cargo build` and has no app
bundle or installer. What each platform needs:

- **macOS.** An app bundle (`Velvet Hammer.app`) whose `Info.plist` declares the scheme, and
  Launch Services registering the bundle (it does when the app is first opened from
  `/Applications`, or with `lsregister`). Rune then delivers each link to `App::on_open_url`,
  which the app already handles, including the link that launched it:

  ```xml
  <key>CFBundleURLTypes</key>
  <array>
    <dict>
      <key>CFBundleURLName</key>
      <string>Velvet Hammer message link</string>
      <key>CFBundleURLSchemes</key>
      <array>
        <string>velvet-hammer</string>
      </array>
    </dict>
  </array>
  ```

- **Windows.** An installer that writes
  `HKEY_CURRENT_USER\Software\Classes\velvet-hammer` with a `URL Protocol` value and
  `shell\open\command` set to `"<path>\velvet-hammer.exe" "%1"`, and the app calling
  `rune::single_instance` at start, so a second launch hands its link to the running app instead
  of opening another window. Rune then delivers the link to `App::on_open_url`.
- **Linux.** A `.desktop` file with `MimeType=x-scheme-handler/velvet-hammer;`, registered with
  `xdg-mime default velvet-hammer.desktop x-scheme-handler/velvet-hammer`. Rune passes a link on
  Linux to a new process as an argument rather than to `on_open_url`, so the app would also need
  to read it from its arguments and hand it to a running instance.

## Screenshots

| State | Screenshot |
| --- | --- |
| A message's link copied | [link-copied.png](screens/links/link-copied.png) |
| A link opened: the chat scrolled to the message, highlighted | [link-opened.png](screens/links/link-opened.png) |
| A link in your message as a chip, and the chief of staff given the message quoted | [link-context.png](screens/links/link-context.png) |
| A link to a message that is not in the chat | [link-missing.png](screens/links/link-missing.png) |
