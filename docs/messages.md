# Messages sent while the chief of staff works

By default, a message the captain sends goes to the chief of staff at once, even while it is
working, so the captain can steer it or add to what it is doing without waiting. The message
shows in the conversation at once, in the order it was sent, marked "sent while your chief of
staff was working". A line over the message field says when the chief of staff is working.

The **Messages** card in the agents column opens the setting. Its other mode, **Queue new
messages until the chief of staff is done**, is off by default.

## What Claude Code does with a message sent mid-turn

The chat session is one `claude -p --input-format stream-json --output-format stream-json`
process, and the app writes each message to its input as the captain sends it. Two pages of the
documentation, read on 2026-11-13, describe the input side:

- The [CLI reference](https://code.claude.com/docs/en/cli-reference) says `--replay-user-messages`
  "Re-emit[s] user messages from stdin back on stdout for acknowledgment", and that with
  `--input-format stream-json` "a message still queued when the limit ends a turn stays queued
  and starts a new turn".
- The [headless page](https://code.claude.com/docs/en/headless) names an interrupt as a separate
  act: SIGINT, or the Agent SDK's `interrupt()`.

Neither page says when a queued message is read, so it was tested against Claude Code 2.1.311
with a small script (stream-json in and out, `--replay-user-messages`):

| Turn | Message sent | What happened |
| --- | --- | --- |
| Runs `sleep 6; echo one`, then `sleep 6; echo two`, then replies DONE | "add the word BANANA on its own line", while the first command ran | The message was echoed back right after the first command's result, before the second command. The turn went on, ended with "DONE / BANANA", and reported one `result`. |
| Writes a 400-word story with no tools | The same, as the story started streaming | The story finished unchanged and reported its `result`. The message was then echoed back and started a second turn. |

So Claude Code takes in a message at the turn's next step, after the tool call that is running,
and starts the next turn with it when the turn has no step left. It never stops what is running.
The app relies on exactly that and adds no interruption. Claude Code does offer a distinct
interrupt; the app does not use it, and stopping a turn would be a separate, explicit action.

## How the app knows the chief of staff is working

The session runs with `--replay-user-messages`. `src/claude.rs` counts the messages it wrote that
Claude Code has not echoed back yet, and notes when a turn runs: from an echoed message or the
session starting a turn on its own (a firstmate hook can wake it) to the turn's `result`. The
chief of staff is working while a turn runs or a message has not been taken in. This matters
because one turn can answer two messages (the first case above) or two turns can follow each
other with no pause (the second), so counting `result` records alone would be wrong. The end of a turn is still the last word: a
`result` clears the count of messages awaiting an echo, so an echo that never comes (Claude Code
may merge a held message into another's turn) cannot leave the app working, or a queue stuck.

## Queue mode

In queue mode, a message sent while the chief of staff works waits in a list under the message
field. Each can be edited (**Edit**, then **Save** or **Cancel**; saving an empty message deletes
it) or deleted. When the chief of staff is done, the first goes out; the chief of staff is
working again until it has answered that one, and then the next goes out, so they go out one at
a time, in order. Reordering is not offered.

- A message being edited holds the ones after it, so a message never goes out half edited or out
  of order. A message sent while the queue is not empty joins the end of it, in either mode.
- While Claude Code is not ready (not signed in, starting, or after it stopped), queued messages
  wait, and the sign-in screen says how many. They go out once it is ready again. If the session
  stops in the middle of a turn, the next queued message starts a new session.
- Queued messages are kept in memory only, because the app keeps no conversation history yet.
  Quitting drops them, as it drops the conversation. The setting itself is kept in
  `messages.json` in the data directory.
- Each queued message leaves the queue as it is sent, so it cannot go out twice.

## Messages that are not ready to go

With **Send right away**, the behaviour is as before: while Claude Code is not ready the sign-in
screen replaces the chat, and a message already typed stays in the field. Pressing Enter twice
sends once: the field is emptied before the message goes out, so the second Enter finds nothing
to send. A very long message, or one with links, takes the same path as any other. The add-project
quick replies and the merge approval are messages too and follow the setting; answers to the
panels' questions and view submits answer something already in progress and go out at once.

## Screenshots

| Scene | Screenshot |
| --- | --- |
| A message sent while the chief of staff works, marked, with the working line | [send-while-working.png](screens/messages/send-while-working.png) |
| The end of that turn, answering both messages | [send-while-working-answered.png](screens/messages/send-while-working-answered.png) |
| The setting, with Send right away recommended and chosen | [messages-settings.png](screens/messages/messages-settings.png) |
| Queue mode: three messages waiting, one being edited | [queue-waiting.png](screens/messages/queue-waiting.png) |
| The turn ended: the first queued message went out, two still wait | [queue-delivering.png](screens/messages/queue-delivering.png) |
| Queued messages kept while Claude Code is signed out | [queue-not-ready.png](screens/messages/queue-not-ready.png) |
