# The managed firstmate clone

[Firstmate](https://github.com/kunchenguid/firstmate) is the set of instructions, skills and
scripts that the chief of staff runs on. It ships as a git repository, not as a library, so
Velvet Hammer keeps its own git clone of it. This page explains where the clone lives, how it is
installed and updated, how to change it, and how to reset it.

Velvet Hammer installs and updates the clone, runs the chief of staff inside it, fills the
**Needs your input** panel and the agents list from firstmate's records, and lets you peek at a
crewmate's terminal. [Where the integration stands](#where-the-integration-stands) lists what
works end to end and what is not verified.

## Where it lives

| Path | Contents |
| --- | --- |
| `~/.velvet-hammer/firstmate` | The clone: `AGENTS.md`, `bin/`, `.agents/skills/`, `docs/`. |
| `~/.velvet-hammer/home` | Firstmate's operational home (`FM_HOME`): `state/`, `data/`, `config/`, `projects/`. |

Set `VELVET_HAMMER_DATA` to use a directory other than `~/.velvet-hammer`. The default path has no
spaces because firstmate's shell scripts run from inside it.

The clone and the home belong to Velvet Hammer. The app does not read or change any other
firstmate checkout or home on your machine, and it sets `FM_HOME` itself for every session, so a
`FM_HOME` in your shell is not used.

## Install

When the app opens and finds no clone, it runs `git clone https://github.com/kunchenguid/firstmate`
into the data directory. The clone is made beside its final place (`firstmate.partial`) and
renamed when it is complete, so an interrupted install is never mistaken for an installed clone.
The next install removes a leftover `firstmate.partial`.

If the install fails, for example because the machine is offline or `git` is not installed, the
Firstmate page shows git's own error and an **Install firstmate** button to try again.

If `~/.velvet-hammer/firstmate` already exists but is not a git clone, the install stops and
reports that. The app leaves the directory untouched; move it away and install again.

## The Firstmate page

The card at the bottom of the agents column shows the clone's state in a few words. Select it to
open the Firstmate page, which shows one of these states:

| State | Meaning | What you can do |
| --- | --- | --- |
| Not installed | There is no clone. | **Install firstmate** |
| Installing | `git clone` is running. | Wait |
| Up to date | The clone matches its upstream branch. | **Check for updates** |
| Update available | Upstream has commits the clone lacks, and merging them is clean. | **Update** |
| Has your changes | You edited or committed in the clone, and upstream has nothing new. | **Check for updates** |
| Update blocked | An update would conflict with your changes, or a merge is unfinished. | Resolve it in a terminal |
| Not on its upstream branch | The clone is on a detached commit or a branch that tracks nothing. | `git switch main` |

The app learns about new upstream commits only when it fetches. It fetches when it opens and when
you choose **Check for updates**. It never updates the clone without you choosing **Update**.

## Update

**Update** runs these steps in the clone and stops at the first one that fails:

1. `git fetch`.
2. Ask git whether merging upstream would conflict with your changes. This uses
   `git merge-tree`, which works in git's object store and does not touch your files. If it would
   conflict, the update stops here and nothing is changed.
3. If you have uncommitted edits, save them: `git add -A`, then
   `git commit -m "Local changes saved by Velvet Hammer"`. The button reads
   **Save my changes and update** when this step applies. The commit is local and is authored as
   "Velvet Hammer".
4. Check for conflicts again, because step 3 may have added new files to the history.
5. `git merge` the upstream branch. With no local commits this is a fast-forward. With local
   commits it creates a merge commit.

Velvet Hammer runs only `git clone`, `fetch`, `add`, `commit` and `merge` in the clone, plus
read-only queries. It never runs `reset`, `clean`, `checkout` over your files, a forced operation
or `push`.

## Change it

The clone is yours to change. Edit `AGENTS.md`, the skills under `.agents/skills/` or the scripts
under `bin/` in `~/.velvet-hammer/firstmate` with any editor. You may leave edits uncommitted or
commit them yourself on the `main` branch.

Your changes stay on the clone's `main` branch, with upstream merged into them at each update.
Velvet Hammer uses a merge and not a rebase because a merge never rewrites your commits and git
can predict its outcome exactly before it starts.

To follow a fork of firstmate, point the clone at it with
`git -C ~/.velvet-hammer/firstmate remote set-url origin <your fork>`. The app updates from
whatever branch the clone's current branch tracks.

## When an update is blocked

If your changes and upstream change the same lines, the page shows **The firstmate update is
blocked** and lists the files. Velvet Hammer does not choose between the two versions. Resolve it
yourself:

```sh
cd ~/.velvet-hammer/firstmate
git add -A && git commit -m "My changes"   # if you have uncommitted edits
git merge origin/main                      # git marks the conflicts in the files
# edit the files to resolve the conflicts
git add -A && git commit
```

Then choose **Check for updates**. The page also shows this state while a merge you started is
unfinished.

## The chief of staff runs as firstmate

The chat is one long-lived `claude -p` process (see the README). Velvet Hammer starts it with the
clone as its working directory and `FM_HOME` set to `~/.velvet-hammer/home`, which it creates
with the four directories firstmate expects. Claude Code then loads the clone's `AGENTS.md`,
skills and `.claude/settings.json` hooks the way it does in a terminal, so firstmate's own
contract governs the session. The line under the chat heading says "Running firstmate from" and
the clone's path.

The chat needs the clone. Until it is installed, the Firstmate page takes the chat's place.

What happens in the session, and what was measured on 2026-11-13 with Claude Code 2.1.311 and
firstmate `427056c4`:

| Firstmate mechanism | Under the app's headless session |
| --- | --- |
| Session-start digest (`SessionStart` hook) | Works. The hook ran, took the session lock for the home and put the digest in the model's context. Measured through the app with the real `claude`. |
| Firstmate's command guards (`PreToolUse` hooks) | Run as in a terminal. Not measured separately. |
| Wakes while idle (`Stop` hook with `asyncRewake`, which exits 2 when the watcher has something) | The transport works: a `Stop` hook that exits 2 started a new turn in an idle headless session, and the app shows a turn nobody asked for as a chief-of-staff message. Measured with a test hook, not with a real crewmate in flight. |
| Wake drain (`bin/fm-wake-drain.sh`) | The chief of staff runs it itself when woken. The app never runs it, because a drain claims the queued wakes for the session that runs it. |
| Supervision host | On by default for Claude. It runs inside the `Stop` hook and needs no terminal. Not measured. |

Limits of the headless session:

- **Nothing watches until the first turn ends.** Firstmate arms its watcher from the `Stop` hook,
  and the app starts `claude` when you send the first message. After you reopen the app, a
  crewmate's status change is not noticed until you have sent one message.
- **An idle home is not woken.** With no task in flight firstmate runs no watcher, so a note
  queued with `bin/fm-inbox.sh note` waits for your next message. This is firstmate's design, and
  the chat is the way to reach the chief of staff.
- **Features that type into a terminal pane do not work.** These are the acknowledged hand-back
  retries (`config/supervision-handoff-ack`), warm-cache compaction (`config/warm-compact`) and
  the away daemon's injection. They need a tmux or herdr pane, and the session has none. Leave
  them off. Away mode on a home with the supervision host does not use a pane.
- **Tool uses run without asking by default.** Firstmate runs many scripts, so the app's hook
  (see [Extending](extending.md)) uses the tool-approval mode **Run without asking** unless you
  choose otherwise on the **Tool approval** page, and logs every tool use. Firstmate's own
  `PreToolUse` command guards are separate hooks, and this setting does not change them. Choosing **Ask every time** puts each of firstmate's
  script runs in **Needs your input**; see the README's
  [Tool approval](../README.md#tool-approval) section for the modes and rules.
- **Firstmate's companion tools are yours to install.** The digest lists any that are missing
  (`tasks-axi`, `treehouse`, `no-mistakes`, `gh`, `tmux` and others), and the chief of staff will
  tell you.

Updating the clone while a session runs changes the scripts and hooks at once, because they are
read from disk when used. A changed `AGENTS.md` is read when the next session starts, which is
the next time you open the app.

When a session ends, or the app quits, the app closes its input and sends `claude` a termination
signal at once, so that Claude Code stops firstmate's watcher instead of leaving it running. If
`claude` has not exited after five seconds, the app kills it.

## The panels and firstmate's records

Firstmate keeps what waits on the captain in its own records and publishes them as one
structured document, `bin/fm-bearings-snapshot.sh --json` (schema `fm-bearings.v1`). Its own rule
is that a view renders that document and does not parse the record files. Velvet Hammer follows
it: `src/bearings.rs` runs the script from the clone with `FM_HOME` set to the app's home and
`FM_CREW_STATE_NO_FORGE=1`, so it makes no network request and writes nothing. It runs once when
the app opens and again whenever a file in the home's `state/` or `data/` directory changes,
which the app checks every two seconds. If a run fails, for example while the clone is being
updated, the app runs it again every 30 seconds until it succeeds, even when nothing in the home
changes. A change in the home still runs it at once, and the same error is shown only once.

| Panel | Source in the snapshot | What you see |
| --- | --- | --- |
| **Needs your input** | `decisions_open[]`: tasks held for the captain | The task id and firstmate's whole question (`question`, never the shortened `summary`), with **Answer** |
| **Needs your input** | `contributions.captain[]` rows with no `hold`: pull requests whose checks are green and which wait for a merge approval | The pull request, firstmate's reason, and **Approve** |
| **For you to do** | No firstmate source (see below) | The notes the chief of staff adds with the app's `app_note` tool |

**Why "For you to do" has no firstmate source.** Firstmate has one kind of captain call: a
task held for the captain. Its records do not say whether the captain must answer a question or
do something himself, such as signing in. Firstmate's own guidance says that distinction is the
agent's judgment and that nothing may classify a hold by reading its wording. So every hold
appears under **Needs your input**, and the app does not guess. A structured marker on a hold
that says "answer" or "act" would let the app split them; that is a change to firstmate and is
proposed there, not made here. Until then the chief of staff can put a step that only you can
take into **For you to do** with `app_note`.

**Answering.** **Answer** on a hold turns your next chat message into your answer: the app sends
"My answer to the captain hold *task-id*: *your words*" to the chief of staff. Firstmate then
records your exact words with its own command, `bin/fm-captain-hold.sh answer`, and the hold
leaves the panel when firstmate's records say it is closed. **Approve** on a merge sends "I
approve merging *url* (task *task-id*)." the same way, and firstmate checks the pull request
again before it merges.

The app does not call `fm-captain-hold.sh answer` or the keyed-answer intake
(`fm-captain-hold.sh answers`) itself, for two reasons taken from that script and firstmate's
`captain-hold-lifecycle` skill:

- An answer either closes the task (a question) or releases its hold so held work resumes, and
  the caller must say which. Firstmate treats that choice as the agent's judgment. The snapshot
  does not carry it, and the wrong choice marks unfinished work done or leaves a question open.
- Recording an answer does not tell the chief of staff. It has to learn of the answer to act on
  it, and the chat is how the captain's words reach it.

If the snapshot exposed the close mode of each hold, the app could feed the intake directly with
`--source "Velvet Hammer"` and still tell the chief of staff. That is the second proposed
firstmate change.

If the script fails, for example because `jq` or `tasks-axi` is missing or the home is in away
mode, the panel shows firstmate's own error above the last items it read.

## The agents list and peek

The agents column lists firstmate's tasks in flight, from `in_flight[]` in the same snapshot
that fills the panel: each row shows the task's title, firstmate's word for its state (such as
`working`, `parked` or `blocked`) and one line on what it is doing. The list changes when
firstmate's records change. You never have to open a row.

Selecting a row opens a peek in place of the chat. While it is open, the app runs firstmate's
`bin/fm-peek.sh <task-id> 40` every two seconds and shows the last 40 lines of that crewmate's
terminal as plain text. The read stops when you choose **Back to chat**. If firstmate cannot
read the terminal, the peek shows firstmate's own reason.

Peek is for looking only. The app sends nothing to a crewmate: firstmate's rule is that the
captain talks to the first mate, and the first mate steers the crew. To change what a crewmate
is doing, tell your chief of staff.

`bin/fm-peek.sh` runs firstmate's supervision guard before it reads, and the guard may write a
warning marker in the home's `state/` directory when no watcher is running. That is the only
thing a peek can write.

## Reset it

Velvet Hammer has no reset button, because a reset discards your changes. To start again from
upstream, quit the app and move the clone away:

```sh
mv ~/.velvet-hammer/firstmate ~/.velvet-hammer/firstmate.old
```

The next time the app opens it installs a fresh clone. Delete `firstmate.old` when you are sure
you do not need anything in it.

The operational home is separate. Moving `~/.velvet-hammer/home` away in the same way gives the
chief of staff an empty backlog and no task records, and loses nothing in the clone.

## Where the integration stands

**Works end to end**, measured with the real firstmate (`427056c4`) and the real `claude`
(2.1.311), always in scratch clones and scratch homes:

- Installing the clone from GitHub, and updating it while keeping local edits. A conflicting
  update stops and names the files.
- Starting the chief of staff as firstmate: the session-start digest reaches the model and the
  session takes the home's lock.
- Reading a task held for the captain through firstmate's snapshot script into **Needs your
  input**.
- Reading a crewmate's terminal through `bin/fm-peek.sh` from a real tmux pane. This one was run
  by hand against the scripts, not through the app's code.

**Works against stand-ins only** (the fake `claude` and stand-in scripts in a scratch upstream):
the answer flow, merge approvals, the agents list and the peek view.

**Not verified:**

- Answering a hold end to end with the real `claude`: the answer reaches the chat, but whether
  firstmate then records it was not run.
- A merge approval row from a real pull request. The fixture's rows are written by hand in the
  shape firstmate's script produces.
- A real crewmate in flight under the app: the watcher arming at turn end, a status change
  waking the session, the supervision host, and the agents list and peek fed by it.
- Whether Claude Code stops firstmate's hook processes when the app ends a session.
- The app's real window. Everything above ran headless.

**Decided: configurable, recommended run without asking.** The chief of staff's tool uses are
governed by a persistent, customizable tool-approval setting. A fresh install runs tools without
asking and logs every one; **Auto-allow read-only tools**, **Ask every time** and custom allow,
ask and deny rules are on the **Tool approval** page. Whether this changes how firstmate
approves its own crew was not in scope and is unchanged.

**Proposed firstmate changes** (none is made by this app):

1. A structured marker on a captain hold that says whether the captain must answer or act, so
   **For you to do** can be filled from firstmate's records.
2. The close mode of each hold (close the task, or release held work) in the bearings snapshot,
   so the app can feed the keyed-answer intake directly and still tell the chief of staff.
3. A way to start supervision when a session opens, so that a crewmate's status change is
   noticed before the captain's first message after reopening the app.
