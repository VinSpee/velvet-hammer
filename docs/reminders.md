# Reminders

Tell the chief of staff "remind me ..." and Velvet Hammer keeps the reminder and shows it when it
comes due. The chief of staff calls the app's `app_remind` tool with the reminder's text and one
of a few fixed forms of *when*; the app, not the model, works out the time, so the same words
always mean the same time. The tool answers with that time in plain words, such as "Reminder set
for Tomorrow, 9:00 AM: the standup", and the chief of staff passes it on. Asked "what reminders do
I have?", the chief of staff reads them with the read-only `app_reminders` tool.

These are the captain's own reminders inside the app. They are not firstmate tasks: the tool's
description tells the chief of staff never to create a task for one, and the app never touches
firstmate's records. A reminder about a firstmate task or pull request just says so in its text.

The code is in `src/reminders.rs` (the forms, the times, the file and the tools' answers) and
`src/view/remind.rs` (the banner, the panel cards, the Reminders page and its settings).

## What each form means

| The captain says | The tool's `when` | When it comes due |
| --- | --- | --- |
| "in 20 minutes", "in 2 hours" | `in` with `amount` and `unit` `minutes` or `hours` | Exactly that long from now. |
| "in 3 days" | `in` with `unit` `days` | The same time of day, that many calendar days on. |
| "later today" | `later_today` | 3 hours from now, but no later than 6:00 PM. If 6:00 PM has already passed, 2 hours from now. |
| "tomorrow morning" | `tomorrow_morning` | 9:00 AM tomorrow. |
| "next week" | `next_week` | 9:00 AM on the next Monday. On a Monday, that is the Monday a week on. |
| "later" | `later` | No time: the reminder goes on the **Later list**. |
| a clock time or date none of the above say, such as "at 3:15 PM" | `at` with `at` as local `YYYY-MM-DDTHH:MM` | That local time. It must be in the future. |

Spelled out, `later_today` said at a time *now* is: if *now* is at or after the evening time, *now*
plus the second number of hours; otherwise *now* plus the first number of hours, or the evening
time today if that comes sooner. With the defaults, at 10:00 AM that is 1:00 PM, at 4:00 PM it is
6:00 PM, and at 7:00 PM it is 9:00 PM.

The **Later list** comes back once a day at 9:00 AM: each reminder on it shows as due, until the
captain marks it done, deletes it, or snoozes it. If Velvet Hammer is not running at 9:00 AM, the
list comes back when it next opens that day. A reminder put on the list after that day's 9:00 AM
first comes back the next morning; one put on it earlier comes back that same morning.

Every number and time above except the exact minutes, hours and days is a setting on the
**Reminder settings** page (see below). The page also shows what "later today", "tomorrow morning"
and "next week" would mean if said now.

**Local time and daylight saving.** Times are the computer's local wall-clock times in its time
zone, worked out with [jiff](https://docs.rs/jiff). Minutes and hours are exact durations, so "in
6 hours" at 9:00 PM on the night clocks go forward is 4:00 AM. Days and the keywords keep the
clock time, so "tomorrow morning" said that same evening is 9:00 AM, eleven real hours later. A
time a change skips (2:30 AM as clocks go forward) moves forward by the gap; a time a change
repeats is the first of the two. The tests check these on both changes of 2026 in US Eastern
time.

**Limits.** A reminder's text is at most 200 characters, and it is kept and shown as one line of
plain text: line breaks become spaces, and control and direction-changing characters are removed.
Nothing in it is run, opened or formatted. At most 100 reminders may wait at once, and a reminder
can be at most 366 days ahead. The tool says what is wrong with a call it refuses, such as "Today,
9:00 AM has already passed: it is now Today, 2:30 PM.", so the chief of staff can try again.

## When a reminder comes due

While the app runs, it checks at the next reminder's time, and at least every 30 seconds while any
reminder waits, so a computer that slept through the time still shows it. A reminder that comes
due shows in three places:

- a **banner** over the middle column, outlined in orange, with the reminder's text, when it was
  set and when it was due, and **Done**, **Snooze** and **Delete**. If more than one is due, it
  shows the latest and a button to all reminders;
- a card at the **top of For you to do**, with the same actions;
- a **system notification**, where the platform and Rune allow it: on macOS, Notification Center,
  through Rune's `rune-notify`. Only an app run from a `.app` bundle may post there, and macOS asks
  the captain's permission the first time. Velvet Hammer run with `cargo run` is not bundled, and a
  captain who refuses permission sees no notification; in both cases the banner and the card still
  show it. A notification is not posted while the window is in front, where the banner shows the
  reminder, and it is taken back when the captain deals with the reminder. On Windows Rune posts
  a toast only for a registered app, which Velvet Hammer does not do yet; elsewhere there are none.

A reminder that came due while the app was not running shows as due, marked **overdue**, as soon
as the app opens.

**Set in this message.** A reminder keeps the id of the captain's message the chief of staff was
answering when it set it, and shows **↪ Set in this message** in the banner, its For you to do
card and the Reminders page. Pressing it goes to that message the way a
[message link](links.md) does: the chat scrolls to it and highlights it briefly, or says it is
not in the chat when the conversation history no longer holds it. `app_reminders` gives the
chief of staff the same link, as "(set Today, 2:30 PM in velvet-hammer://message/…)".

**Snooze** opens a small menu: **10 minutes**, **1 hour**, **Later today**, **Tomorrow morning**,
**Next week** and **Later list**, and a field for a number of minutes, hours or days. A snoozed
reminder is set again from now, by the same rules as the tool. **Done** and **Delete** both remove
the reminder. A reminder that has not come due yet has **Snooze** and **Delete** on the Reminders
page.

## The Reminders page

The **Reminders** card under the agents, which says how many are due, coming up and later, opens
the **Reminders** page: **Due now**, **Coming up** in time order, and the **Later list**, each
reminder with its time, its text, when it was set, and its actions. **All reminders** at the
bottom of For you to do opens it too. **Reminder settings** is the **Reminders** section of
**Settings** (Cmd+, on macOS), whose card sums them up ("later today by 6:00 PM · mornings 9:00
AM"); **Settings** on the Reminders page opens it too. It has each
setting with **−** and **+** (hours one at a time, times by half an hour, the day of the week
round the week), **Reset to the defaults**, and what the keywords mean if said now. The settings
apply at once, to reminders set from then on. Its buttons go back to the Reminders page, to all
settings, and to the chat.

## What is stored

Both files are in the app's data directory (`~/.velvet-hammer`, or `$VELVET_HAMMER_DATA`), never in
a repository or in firstmate's home.

- `reminders.json` keeps, for each reminder, a number, its text, when it was set, when it is due
  (or that it is on the Later list), when it came due if it has, for the Later list the day it
  last came back, and the id of the message it was set in. Nothing else from the conversation is stored. The file is written whole (a
  temporary file, synced, then renamed into place) and readable by the captain only (mode
  0600). If it cannot be read, the page says why and shows no reminders,
  and the next change keeps the old file as `reminders.json.bad`.
- `reminder-settings.json` keeps the settings. If it cannot be read, the defaults are used and the
  settings page says why.

## The tools

`app_remind` takes `text` and `when`, plus `amount` and `unit` for `in` and `at` for `at`. Its
description gives the forms and three examples ("remind me in 20 minutes to check the deploy",
"remind me tomorrow morning about the standup" and "remind me later") so the model needs no
other prompting. `app_reminders` takes nothing and answers with the time now and every reminder,
due ones first and the Later list last. Both count as the app's own tools for tool approval: in
**Auto-allow read-only tools** they run without asking, like `app_note`.

**Verified.** On 2026-11-13, against Claude Code 2.1.311 on the Haiku model, the ignored test
`real_claude_sets_a_reminder_that_fires` (`cargo test real_claude -- --ignored`) sent "remind me
in 2 minutes to stretch". Claude Code loaded the deferred tool with `ToolSearch`, called
`app_remind` with `{"text": "stretch", "when": "in", "amount": 2, "unit": "minutes"}`, got "Reminder
set for Today, 8:16 PM: stretch" and passed it on; with the app's clock moved two minutes on, the
banner showed. The fake `claude` in `scripts/` calls both tools the same way for the other tests
(`remind: in 2 minutes | Stretch`, `reminders?`).

Not in this version: recurring reminders, syncing across devices, and sending reminders anywhere
outside the app.

## Screens

| State | Screenshot |
| --- | --- |
| The Reminders page with a reminder of each form | [reminders-list.png](screens/reminders/reminders-list.png) |
| Two reminders due, one overdue: the banner and For you to do | [reminder-fired.png](screens/reminders/reminder-fired.png) |
| The snooze menu open on the banner | [reminder-snooze.png](screens/reminders/reminder-snooze.png) |
| Reminder settings, with what each keyword means now | [reminder-settings.png](screens/reminders/reminder-settings.png) |
| **Set in this message** pressed: the chat at the captain's message, highlighted | [reminder-link-opened.png](screens/reminders/reminder-link-opened.png) |
| Settings, with its Reminders section | [settings.png](screens/settings/settings.png) |
