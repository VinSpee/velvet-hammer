# The managed firstmate clone

[Firstmate](https://github.com/kunchenguid/firstmate) is the set of instructions, skills and
scripts that the chief of staff runs on. It ships as a git repository, not as a library, so
Velvet Hammer keeps its own git clone of it. This page explains where the clone lives, how it is
installed and updated, how to change it, and how to reset it.

Velvet Hammer installs, updates and reports on the clone today. Running the chief of staff inside
it, and feeding the panels and the agents list from it, are not built yet.

## Where it lives

| Path | Contents |
| --- | --- |
| `~/.velvet-hammer/firstmate` | The clone: `AGENTS.md`, `bin/`, `.agents/skills/`, `docs/`. |

Set `VELVET_HAMMER_DATA` to use a directory other than `~/.velvet-hammer`. The default path has no
spaces because firstmate's shell scripts run from inside it.

The clone belongs to Velvet Hammer. The app does not read or change any other firstmate checkout
on your machine.

## Install

When the app opens and finds no clone, it runs `git clone https://github.com/kunchenguid/firstmate`
into the data directory. The clone is made beside its final place (`firstmate.partial`) and
renamed when it is complete, so an interrupted install is never mistaken for an installed clone.
The next install removes a leftover `firstmate.partial`.

If the install fails, for example because the machine is offline or `git` is not installed, the
Firstmate page shows git's own error and an **Install firstmate** button to try again.

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

## Reset it

Velvet Hammer has no reset button, because a reset discards your changes. To start again from
upstream, quit the app and move the clone away:

```sh
mv ~/.velvet-hammer/firstmate ~/.velvet-hammer/firstmate.old
```

The next time the app opens it installs a fresh clone. Delete `firstmate.old` when you are sure
you do not need anything in it.

## What is not verified

The lifecycle is tested against scratch git repositories on the local disk, and headless renders
cover every state. Installing from GitHub through the app's window has not been tested.
