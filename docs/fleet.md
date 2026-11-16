# The fleet panel

The column on the right of the window is the fleet panel. It shows what waits on you and how
firstmate's fleet stands, from firstmate's own records, and costs no model use. It replaced the
two panels **Needs your input** and **For you to do**; everything they showed is now in
**Waiting on you** at the top of the panel.

From the top, the panel has:

- **Fleet**, with when the snapshot shown was read ("Updated Today, 2:30 PM"), **Refresh**, and
  the buttons **Ahoy** and **Bearings**.
- **Waiting on you (*n*)**, pinned at the top: every tool use waiting for your approval, every
  task firstmate holds for your decision, every pull request waiting for your merge approval,
  every reminder that came due and every note the chief of staff left you. Each card keeps its
  actions: **Allow** or **Deny** (or the question's own answers), **Answer**, **Open pull
  request** and **Approve**, **Done**, **Snooze** and **Delete**. With nothing waiting it shrinks to
  one line, the heading "Waiting on you (0)", with **All reminders** at its right when a reminder
  is set. Its border is thicker and coloured while something waits, and the count in its heading
  says the same in words.
- **Underway (*n*)**: firstmate's tasks in flight, each with its state as a word (`working`,
  `parked`, `blocked`), what it is doing and what it runs on. Pressing one shows its terminal
  (see [The managed firstmate clone](firstmate.md#underway-and-peek)).
- **Recently landed (*n*)**: work that finished lately, with **Open pull request** for a merged
  pull request, or the path of a scout's report.
- **Charted next (*n*)**: queued work and what each task waits for. It starts closed.

Each section opens and closes from its heading, which says whether it is open, for the
keyboard and for assistive technology as well as with its arrow. While the chief of staff is not
firstmate, as in the simulated chief of staff, the panel shows only **Waiting on you**.

## Where the facts come from

The panel draws firstmate's bearings snapshot, `bin/fm-bearings-snapshot.sh --json --fields
endpoints --all-secondmates` (schema `fm-bearings.v1`), which reads firstmate's records without a
model, a network request or a write. The app runs it from its firstmate clone with `FM_HOME` set
to its home and `FM_CREW_STATE_NO_FORGE=1`.

| Section | Snapshot field |
| --- | --- |
| Waiting on you: decisions | `decisions_open[]`, firstmate's whole `question` |
| Waiting on you: merges | `contributions.captain[]` rows with no `hold` |
| Waiting on you: tool approvals, due reminders, notes | The app itself (the hook, `reminders.json`, `app_note`) |
| Underway | `in_flight[]`, with the harness and model from `state/<id>.meta` |
| Recently landed | `landed[]`: `id`, `what`, `artifact` |
| Charted next | `gates[]`: `id`, `title`, `blocked_by`, `reason` |

The snapshot on an empty home took 0.36 s (measured once on firstmate `427056c4`). The time on a
large home was not measured.

## When it refreshes

The snapshot runs only when something says the last one may be out of date, never on a timer:

- a status log or task record in firstmate's home changed (`state/*.status`, `state/*.meta`, a
  second mate's `state/home-summary.json`, or anything directly in `data/`, such as the backlog
  and reports). The app compares those files' change times every two seconds, which costs a
  directory listing. The rest of `state/`, such as locks and counters, changes far more often and
  never changes the snapshot, so it is not watched;
- a turn of the chief of staff finished;
- you answered a card: a tool approval, a hold, a merge approval or a note;
- you pressed **Refresh**.

Changes that arrive together share one run, which starts 0.3 s after the first. Runs that changes
ask for are at least five seconds apart, and the last change always gets its run, so a busy
fleet costs at most one run every five seconds. **Refresh** runs at once. While the clone has no
snapshot script yet, for example during its install, the app only checks again every two
seconds and keeps the pending run until the script appears.

A run happens on a background thread. While it runs, the panel keeps showing the last snapshot
and adds "refreshing…" to its time. If a run fails, the panel keeps the last snapshot, shows
firstmate's own error under the time, and the app tries again 30 seconds later.

The last snapshot is kept in the app's data directory, `bearings-snapshot.json`, with the time it
was read and the home it was read from. When the app opens, the panel shows it at once, marked
"Saved *time*, before this launch", until the first run of the launch finishes. A snapshot kept
for another home is not shown.

## Ahoy and Bearings

These two buttons ask the chief of staff, which costs a model turn, so the app presses them for
you at most once a day:

- **Ahoy** sends `/ahoy`: firstmate recaps what happened in this session since your last message
  and walks you through open decisions one at a time. A new session starts at every launch, so
  right after a launch firstmate gives its bearings instead.
- **Bearings** sends `/bearings`: firstmate runs the same snapshot and writes its digest of the
  fleet.

Each goes to the chief of staff like a message you typed, and the reply arrives in the chat like
any other. Neither runs on a timer. The first time the chat can run on a new day, the app sends
`/bearings` once by itself, which also starts the session so that firstmate supervises the fleet
from then on. The day is kept in `bearings-day` in the app's data directory, so opening the app
again that day sends nothing.

A message that starts with a skill's name runs that skill in the app's kind of session: with
Claude Code 2.1.311 in `-p` mode with `--input-format stream-json`, a user message of
`/probe-digest` in a scratch project ran a scratch skill of that name (checked once by hand on
2026-11-15). Firstmate's own `/ahoy` and `/bearings` were not run through the app.

## How it is tested

- `refresh::tests` checks when runs happen: changes made together share one, a status line every
  second for twelve seconds costs four runs, and **Refresh** runs inside the gap.
- `bearings::tests` reads each section from fixture snapshots, and runs the watcher on a scratch
  clone whose script fails once: the failure is retried, a kept snapshot arrives before any run
  and stands while the run fails, a kept snapshot for another home is not shown, and a finished
  turn or **Refresh** runs the snapshot with nothing changed on disk.
- `fleet::tests` checks which sections start open and that a day has its digest once across
  launches.
- The app's own tests draw the panel on the fake `claude` with the fixtures
  `scripts/fixtures/bearings-fleet.json` (a decision, a merge approval, a blocked worker, landed
  work and charted work) and `bearings-empty.json` (an empty fleet), both printed by the real
  script for a seeded scratch home (the workers' states and the merge row are written by hand,
  because firstmate reads those only from a live terminal and a real pull request). They also cover
  a kept snapshot with a failing run, **Ahoy** and **Bearings** in the chat, and `/bearings` on
  the first launch of a day only.

Not verified: the panel on a real fleet, how long the snapshot takes on a large home, and
firstmate's own `/ahoy` and `/bearings` replies in the app.

## Screens

| State | Screenshot |
| --- | --- |
| Before: the two panels this replaced | [before-two-panels.png](screens/fleet/before-two-panels.png) |
| Every section filled: a decision, a merge, a blocked worker, landed work | [fleet.png](screens/fleet/fleet.png) |
| Underway and Recently landed closed, Charted next open | [fleet-charted.png](screens/fleet/fleet-charted.png) |
| An empty fleet | [fleet-empty.png](screens/fleet/fleet-empty.png) |
| A snapshot kept from an earlier launch while this launch's run fails | [fleet-failed.png](screens/fleet/fleet-failed.png) |
| Waiting on you alone, with the simulated chief of staff | [waiting-only.png](screens/fleet/waiting-only.png) |
