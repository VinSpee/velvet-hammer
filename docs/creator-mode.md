# Creator mode

Creator mode lets you change Velvet Hammer from inside it and try the change in the same session.
You ask your chief of staff for a change to the app, a worker makes it, and a card for it
appears in **Waiting on you**. You press **Try it**, and within seconds of the build finishing the
window is the new build, in the same place, with the same conversation, and your chief of staff
remembers it. You then keep the change or undo it.

This version is for macOS, changes views and styling only, and needs the app built from source.

## Turn it on

1. Install the app with `scripts/install-app.sh`, which puts it at
   `~/Applications/Velvet Hammer Dev.app` (see [Install the app](../README.md#install-the-app)).
   Creator mode swaps that bundle.
2. Have a Rust toolchain from [rustup](https://rustup.rs), as for **Build and run**. Trial builds
   run with `--offline`, so the crates the app uses must already be in Cargo's cache, as they are
   after the first `scripts/install-app.sh`.
3. Have `velvet-hammer` as a project in your firstmate home, so your chief of staff can send
   workers to it. **Add project** adds it.
4. Open Settings (Cmd+,) and set **Creator mode** to **On**. It is off by default.

## How a change goes

1. **Ask.** Tell your chief of staff what to change, such as "make the message cards wider".
   While creator mode is on, the app tells it once a session how to brief such a change
   (`creator::BRIEF` in `src/creator.rs`): the worker edits only `src/view.rs` and `src/view/`,
   builds, renders the scenes it touched, commits on its branch, pushes nothing, and ends with
   the status line `needs-decision [at=…]: ready to try <commit>: <what changed>`.
2. **Card.** The app reads each task's records in the firstmate home. A task of the
   `velvet-hammer` project whose last status line says `ready to try <commit>` shows as a card:
   what changed, the commit, and **Try it** and **Discard**.
3. **Try it.** The app starts its build step as its own process
   (`velvet-hammer --creator try DATA COMMIT WORKTREE`), because the swap quits the app. The step:
   - fetches the worktree's branch into the app's own checkout, `creator/checkout` in the data
     directory, and checks the commit out there;
   - refuses the change if the branch changed anything but `src/view.rs` and `src/view/` since it
     left the project's `main` (see [What can change](#what-can-change));
   - builds it with `cargo build --locked --offline`, without debug information, in
     `creator/target`, which stays warm, so a view change rebuilds in seconds after the first
     build;
   - makes a bundle from the installed app with the new binary, signed by the same identity;
   - copies the installed app to `creator/known-good/` as the last kept build;
   - swaps the new bundle in with Rune's installer, which waits while the chief of staff is in a
     turn, quits the app, exchanges the bundles atomically, opens the new one, and puts the old
     one back if the new one is not ready within 15 seconds.
   The new build resumes your chief of staff's session and puts your page, draft and queued
   messages back (see [The conversation](conversation.md#the-chief-of-staffs-session)).
   Try it is refused while your chief of staff is working, and while another change is being
   tried: keep or undo that one first, so the installed build is always a kept one.
4. **Trying it.** A banner over the chat says which change you are trying, with **Keep** and
   **Undo**; the card has them too.
5. **Keep.** The app asks your chief of staff to deliver the branch the normal way: the pipeline,
   a pull request, and a merge, which asks you first. The trial build stays meanwhile. After the
   merge, `scripts/install-app.sh` installs it from `main`.
6. **Undo.** The app swaps the last kept build back, which takes as long as a relaunch. The
   branch stays, so the worker can try again.
7. **Discard.** The app undoes the trial if it is running (refused while your chief of staff is working, as Undo is), and asks your chief of staff to have
   the worker drop the branch.

Only your press of **Try it**, **Undo** or **Discard** swaps the app. No tool your chief of
staff can call does, and it is told never to swap, rebuild or restart the app itself.

## When something goes wrong

| What happens | What you see |
| --- | --- |
| The branch changes more than views | The card says which paths, and the app is not changed. Deliver it as a pull request instead. |
| The build fails | The card shows the compiler's last lines, and the app is not changed. The whole output is in `creator/job.log`. |
| The new build does not start, or is not ready in 15 seconds | Rune's installer puts the previous build back and reopens it, and the card says the swap failed. |
| The trial build stops later without your Quit | The build step watches the trial. It puts the last kept build back and opens it, and the card says so. Quit from the menu ends that watch, and opening the app again during the trial starts a new one, so a later crash is still caught. |
| The interface is too broken to reach Undo | Open `creator/known-good/Velvet Hammer Dev.app` from Finder, or run `scripts/install-app.sh`. |

`creator/` is in the app's data directory (`~/.velvet-hammer`, or `$VELVET_HAMMER_DATA`) and is
readable by your user only. `creator/trial.json` records the trial: its state, commit and
worktree, and why it stopped.

## What can change

Only views and styling: `src/view.rs` and `src/view/`. Everything else is delivered as a normal
pull request and picked up by the next `scripts/install-app.sh`, including:

- the hook rules and tool approval (`src/guard.rs`, `src/approval.rs`, `src/bridge.rs`);
- the Claude Code and firstmate boundaries (`src/claude.rs`, `src/firstmate.rs`, `src/home.rs`);
- the stores (`src/store.rs`);
- creator mode itself (`src/creator.rs`, `src/creator/`);
- the build inputs (`Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, any `build.rs`,
  `scripts/`, `.github/`, `.no-mistakes.yaml`).

What `main` gained since your installed build is built along with the change but not checked:
it was reviewed and merged. The check limits accidents; it is not a sandbox, because code in a
view can do anything the app can. The worker is your own agent, and the card shows what it says
it changed.

## Not in this version

- Changes to how the app behaves, beyond views, which would need you to confirm a summary of the
  diff first.
- Colours that change at once without a build.
- Keeping a turn in progress or a waiting approval across the swap: the swap waits until the
  chief of staff is idle.
- Windows and Linux.
