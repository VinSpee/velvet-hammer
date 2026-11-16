# What gets your attention

Velvet Hammer supervises while you work in other apps. This page covers how it reaches you when
something waits on you, how long a question from the chief of staff's hook waits for your answer,
and what closing the window and quitting do. The settings are on the **Attention** page in
**Settings**, and are kept in `attention.json` in the app's data directory.

## Notifications and the Dock badge

Four kinds of thing wait on you:

| Kind | Where it comes from | Notification | Dock badge and menu-bar count |
| --- | --- | --- | --- |
| A tool use waits for your approval | The chief of staff's PreToolUse hook (merges, pushes, project writes, and anything your [tool-approval settings](../README.md#tool-approval) ask about) | Yes | Yes |
| A merge is ready for your approval | Firstmate's records: a green pull request waiting for your approval | Yes | Yes |
| A decision is held for you | Firstmate's records: a task held for you, in this home or a second mate's | Yes | Yes |
| A worker is blocked or failed | Firstmate's records: a task in flight whose state is `blocked` or `failed` | No | Yes |

A finished turn is not one of them and never notifies.

- **Once each.** Each thing notifies once while it waits, however often the window redraws or
  firstmate's records are read again. When you answer it, or it otherwise stops waiting, its
  notification is taken back from Notification Center and the count drops. A worker that is
  blocked again after it worked on counts as a new thing.
- **Not while you look.** A thing that arrives while Velvet Hammer is the app in front shows in
  the window and is not notified, now or later. The count still includes it.
- **The count.** The Dock icon and the menu-bar item show how many things wait on you, of the
  kinds that are on.
- **Clicking a notification** opens the window, shows it if the app was hidden, scrolls to the
  thing's card and outlines it for four seconds. A card for something already answered is not
  there, and nothing is outlined.
- **Each kind can be turned off** on the **Attention** page. A kind turned off leaves the count
  and takes back its notifications; turned on again, what still waits notifies.
- **Restarting** the app notifies again for what still waits, unless the window is in front.

Notifications need the app opened from its bundle (see [Install the app](../README.md#install-the-app))
and your permission, which macOS asks for with the first notification. Run with `cargo run`, the
app shows the badge and the menu-bar item but posts no notifications. A refused permission, Focus
or Do Not Disturb hide the notifications; the badge, the menu-bar count and the window still show
everything.

## How long a question waits

When the hook asks you about a tool use, its card in **Needs your input** waits for your answer
for the time set on the **Attention** page: 10 minutes, 1 hour, 8 hours (recommended, the default)
or 24 hours. The chief of staff waits with it. A change applies to the next question, also in a
session that is already running, because the hook reads the setting at every question.

If nobody answers in that time, the tool use is denied, and the card stays, marked **Expired**,
with what happened and the question's full input:

- **Ask again** sends the chief of staff a message saying that the question expired and asking it
  to try the tool use again, with the question's title and up to 2,000 characters of its input.
  Trying again asks you again. The expired card goes.
- **Dismiss** takes the card away and sends nothing.

The same card appears when Claude Code stops waiting before you answered, for example because its
session ended, and the chief of staff's reply says the tool use was blocked.

### What was checked

The waits rely on Claude Code honoring the hook's `timeout`. The app sets it to 24 hours and ten
minutes, longer than the longest wait, so the app always answers first, with a deny it chose. With
Claude Code 2.1.311 and an HTTP PreToolUse hook like the app's (`onFailure: "block"`), in a
scratch directory:

| Hook timeout | The hook answered after | Outcome |
| --- | --- | --- |
| 7,200 s | 330 s | Allowed; the command ran (past the 300 s many HTTP clients wait by default) |
| 7,200 s | 660 s | Allowed; the command ran (past Claude Code's default of 600 s) |
| 7,200 s | 1,301 s | Allowed; the command ran |
| 87,000 s | 3 s | Allowed: a timeout of 24 hours and ten minutes is accepted |
| 20 s | 40 s | Claude Code hung up at 20 s and blocked the tool use, so the timeout is in seconds and enforced |

Nothing on the app's side cuts a wait short: the hook's connection has no read timeout once the
request is read, and the hook only checks every quarter second whether Claude Code hung up.
**Not verified:** a wait of several hours, and a wait across the Mac sleeping. Both clocks
involved (the app's and Claude Code's) may not count sleep, in which case a question waits longer
than set, not shorter.

## Closing the window and quitting

On macOS, closing the window (the close button or Cmd+W) only hides it. The app keeps running,
with the chief of staff, firstmate's watcher and any question that waits on you, and a hammer in
the menu bar keeps it reachable. Its menu says how things stand, then offers its two commands:

```
Chief of staff: idle
8 things wait on you
3 workers in flight
────────────────────
Open Velvet Hammer
Quit Velvet Hammer
```

**Open Velvet Hammer**, clicking the Dock icon, or opening the app again from Finder or Spotlight
brings the window back as you left it.

**Quit** (Cmd+Q, **Quit Velvet Hammer** in the menu-bar item, the Dock's Quit, or logging out)
asks first when it would stop work: while the chief of staff is in the middle of a turn, or while
firstmate has workers in flight (any task in flight that is not `done` or `failed`). The question
opens the window and says what is in flight, by name, and what quitting does: it stops the chief
of staff and firstmate's watcher, and the workers keep running in their terminals with nothing
supervising them until you open the app again and send a message. **Keep running** cancels the
quit; **Quit anyway** quits. With nothing in flight, Quit quits at once. A logout or restart
asks AppKit the same way, so while work is in flight the question holds it too (not tried).

On Windows and Linux there is no menu-bar item yet, so closing the window quits the app as before,
without asking.

## Screenshots

The first four are headless scenes (`velvet-hammer --scene NAME --snap FILE`); the rest are from
the installed app in a real window, on the fake `claude` and a scratch firstmate home, except the
first launch, which used the real `claude`.

| State | Screenshot |
| --- | --- |
| The **Attention** page | [attention-settings.png](screens/attention/attention-settings.png) |
| A question nobody answered in time, with **Ask again** | [approval-expired.png](screens/attention/approval-expired.png) |
| The card a notification click revealed, outlined | [attention-revealed.png](screens/attention/attention-revealed.png) |
| Quit with work in flight | [quit-question.png](screens/attention/quit-question.png) |
| The installed app opened with Finder's `PATH`, finding Claude Code | [real-finder-launch.png](screens/attention/real-finder-launch.png) |
| A notification for a held decision | [real-notification.png](screens/attention/real-notification.png) |
| The window after clicking a merge's notification | [real-notification-click.png](screens/attention/real-notification-click.png) |
| The menu-bar item with eight things waiting | [real-menu-bar-item.png](screens/attention/real-menu-bar-item.png) |
| Cmd+Q from AppleScript with three workers in flight | [real-quit-question.png](screens/attention/real-quit-question.png) |
