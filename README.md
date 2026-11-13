# Velvet Hammer

Velvet Hammer is a native desktop app for working with AI coding agents, built with
[Rune](https://github.com/anomalyco/rune). The idea is that you talk to one chief of staff, which
delegates work to subagents.

## Status

Velvet Hammer is early and experimental. The chief of staff runs on your own installed Claude Code
and your Claude subscription (see [Claude Code and your subscription](#claude-code-and-your-subscription)).
The app installs and updates its own clone of
[firstmate](https://github.com/kunchenguid/firstmate), and the chief of staff runs inside it as
firstmate, with its own operational home (see
[The managed firstmate clone](docs/firstmate.md)).
**Needs your input** shows the tasks firstmate holds for you and the merges waiting for your
approval, read from firstmate's own records, together with the tool uses waiting for your
approval. **For you to do** shows the notes Claude Code adds with the app's `app_note` tool. The
agents list shows the tasks firstmate has in flight, and selecting one shows that crewmate's
terminal. [Where the integration stands](docs/firstmate.md#where-the-integration-stands) lists
what works end to end and what is not verified. A stub with sample data stands in for the agent layer in headless scenes and tests. The interface
and code will change.

The sign-in flow is tested against a fake `claude`; signing in with a real Claude account has not
been tested yet (see [Trying the real sign-in](#trying-the-real-sign-in)).

## What works today

The window has three columns:

- **Agents** (left): the subagents the chief of staff manages. Selecting one opens a peek at its
  transcript in place of the chat; **Back to chat** closes it. A **Firstmate** card at the bottom
  shows the state of the managed firstmate clone and opens a page for installing and updating it.
- **Chief of staff** (middle): the conversation, with a message field and a **Send** button. A
  line under the heading says what runs it, such as "Claude Code · signed in with your Claude Max
  plan". When Claude Code is not ready, a sign-in screen takes the conversation's place.
- **Needs your input** and **For you to do** (right): questions the agents are waiting on, each
  with answer buttons, and steps only you can take, each with a **Done** button.

## Layout of the code

- `src/model.rs`: the data model. A `Workspace` holds the chat `Message`s, the `Item`s waiting on
  the captain and the subagent `Agent`s. Everything the captain does is an `Action`
  (`Say`, `Answer`, `Done`), and `Workspace::apply` records it.
- `src/agent.rs`: the agent-layer boundary. The app talks only to the `ChiefOfStaff` trait, which
  takes the captain's actions and sign-in requests and reports `Event`s (status changes, streamed
  replies, problems). `Stub` simulates it for headless scenes and tests.
- `src/claude.rs`: `Claude`, the chief of staff on the user's own Claude Code. It runs only the
  `claude` command, as described below.
- `src/bridge.rs`: the server the chat session's `claude` calls back into: one MCP tool and one
  PreToolUse hook (see [docs/extending.md](docs/extending.md)).
- `src/firstmate.rs`: `Firstmate`, the managed firstmate clone. It runs `git` to install the
  clone, compare it with upstream and update it, and never discards a local change. Its `lab`
  module builds scratch git repositories for tests and scenes.
- `src/bearings.rs`: what waits on the captain, read from firstmate's records by running its
  `bin/fm-bearings-snapshot.sh --json` whenever the records change.
- `scripts/fake-claude`: a stand-in for `claude` that speaks the same protocol without an account.
  Tests and the sign-in scenes run the app on it.
- `src/view.rs`: the Rune view. Every element a test or script reaches has a key.
- `src/main.rs`: the `App` state (workspace, chief of staff, sign-in status, firstmate clone,
  message draft, what the middle column shows), the headless scenes, and the tests.

## Claude Code and your subscription

Velvet Hammer does not sign in to Anthropic itself and never handles a credential. It starts the
unmodified `claude` program that you installed, and Claude Code signs in, stores the sign-in and
talks to Anthropic:

- **Is it signed in?** The app runs `claude auth status --json` when it opens, when you choose
  **Check again**, and every two seconds while a sign-in runs. It reads only `loggedIn`,
  `authMethod` and `subscriptionType` from the output.
- **Signing in.** **Sign in with Claude** runs `claude auth login --claudeai` in a pseudo-terminal
  (it is an interactive command). Claude Code opens its sign-in page in your browser, and the app
  shows that page's address with an **Open the sign-in page** button in case the browser did not
  open. When `claude auth status` reports a sign-in, the app stops the login command and opens the
  chat. You can also run `claude auth login` in a terminal yourself and choose **Check again**.
- **Chatting.** The first message starts one long-lived
  `claude -p --input-format stream-json --output-format stream-json --verbose
  --include-partial-messages --no-session-persistence` process. Each message is written to its
  standard input, and the reply streams into the chat as Claude Code produces it. The process
  runs in the managed firstmate clone with `FM_HOME` set to the app's own firstmate home, so it
  starts as firstmate (see [The managed firstmate clone](docs/firstmate.md)).
- **Tools and hooks.** The chat session also gets `--mcp-config` and `--settings` flags that point
  at a server the app runs on `127.0.0.1` for that session only. Its `app_note` tool adds a note
  to **For you to do**, and its PreToolUse hook asks you in **Needs your input** before Claude
  Code uses any other tool. Your Claude settings files are not changed. See
  [docs/extending.md](docs/extending.md), which also covers skills, subagents, the Agent SDK and
  what Anthropic's terms say.
- **Subscription only.** If `claude auth status` reports a sign-in method other than `claude.ai`
  (an Anthropic Console sign-in, an API key or anything else), the chat stays locked and the
  sign-in screen asks for a Claude subscription sign-in: run `claude auth login --claudeai`, or
  choose **Sign in with Claude**. The app also removes `ANTHROPIC_API_KEY` and
  `ANTHROPIC_AUTH_TOKEN` from the environment of every `claude` process it starts, so a key set in
  your shell cannot make the chat bill an API account.
- **When something is wrong.** The sign-in screen says which case applies: not signed in, signing
  in, the sign-in expired (Claude Code reports a sign-in but Anthropic rejected it for a message,
  which Claude Code reports as `authentication_failed`), `claude` not on `PATH`, or `claude`
  failing to run (its own error text is shown). A failed message shows Claude Code's own error in
  the chat.

The app looks for `claude` on its `PATH`. When you start it from a terminal with `cargo run`, that
is your shell's `PATH`.

**How this fits Anthropic's terms.** Anthropic's Claude Code legal and compliance page
(https://code.claude.com/docs/en/legal-and-compliance) says that third-party developers may not
offer Claude.ai login inside their own applications or route requests through Free, Pro, Max or
Ultra credentials on behalf of their users, and that nothing prevents an end user from signing in
to the unmodified Claude Code program with their own subscription, including where another product
hosts Claude Code. Velvet Hammer takes the second route. It is a personal tool that starts your own
unmodified `claude`; the sign-in happens in Claude Code and your browser, and the app never sees,
stores or forwards a token. Messages are billed the way Claude Code bills them. This reading of the
terms has not been confirmed by Anthropic. If Velvet Hammer is ever distributed to other people,
check the terms again for that case.

### Trying the real sign-in

This has not been run against a real account. To try it on a Mac where Claude Code is installed:

1. Sign Claude Code out so the app starts signed out: run `claude auth logout`. (Skip this to try
   the signed-in path only; your existing sign-in will be used.)
2. Run `cargo run` from a terminal whose `PATH` includes `claude` (`which claude` prints it).
3. Check that the middle column shows **Sign in with Claude**.
4. Choose **Sign in with Claude**. Check that a browser tab opens on claude.ai and that the app
   shows **Finish signing in in your browser** with the sign-in address. If no tab opened, choose
   **Open the sign-in page**.
5. Sign in with your Claude Pro or Max account in the browser and approve the request.
6. Within a few seconds the app should show the chat, with "Claude Code · signed in with your
   Claude Max plan" (or Pro) under the heading. In a terminal, `claude auth status --text` should
   agree.
7. Send a short message, such as "Reply with one word." Check that the reply streams in.
8. Check the usage at https://claude.ai/settings/usage to confirm the message counted against
   your subscription.

If step 4 shows a problem instead, or the browser page asks you to paste a code into the terminal,
choose **Cancel**, run `claude auth login` in a terminal, finish there, and choose **Check again**.
Please report what the app showed.

## Build and run

Rune pins its Rust toolchain, and this repository repeats the pin in `rust-toolchain.toml`, so
build with [rustup](https://rustup.rs). If another `cargo` comes first on your `PATH` (for
example Homebrew's `rust`), put rustup's first.

```sh
cargo build
cargo run            # opens the window
```

Rune is pinned to one rev of its `dev` branch in `Cargo.toml`, together with the
`[patch.crates-io]` entries Rune's README asks dependent apps to repeat. Bump all of them
together.

## Verify headlessly

Rune renders the same frames without a window, on a fixed virtual clock. One command builds the
app, runs the tests, renders every main state to PNG, dumps the element trees, and checks them:

```sh
scripts/verify.sh [OUT_DIR]     # default OUT_DIR: target/verify
```

It writes these files to `OUT_DIR`:

| File | State |
| --- | --- |
| `chat.png`, `chat.json` | The chat with both panels populated |
| `replied.png`, `replied.json` | After the captain sends a message and the stub replies |
| `empty.png`, `empty.json` | Both panels empty |
| `peek.png`, `peek.json` | Peeking at a subagent's transcript |
| `signed-out.png`, `signed-out.json` | Claude Code is not signed in: the sign-in screen |
| `signing-in.png`, `signing-in.json` | `claude auth login` is running and printed its sign-in page |
| `signed-in.png`, `signed-in.json` | Signed in with a subscription, after a message and its streamed reply |
| `expired.png`, `expired.json` | Anthropic rejected the sign-in for a message |
| `cli-missing.png`, `cli-missing.json` | No `claude` on `PATH` |
| `claude-error.png`, `claude-error.json` | A message failed, with Claude Code's error in the chat |
| `noted.png`, `noted.json` | Claude Code called the `app_note` tool: a note in For you to do |
| `approval.png`, `approval.json` | The PreToolUse hook is waiting for you to allow or deny a Bash command |
| `approval-long.png`, `approval-long.json` | The same question for a very long command, shown whole in a scrollable block |
| `approved.png`, `approved.json` | After you allowed it, with the reply |
| `firstmate-missing.png`, `firstmate-missing.json` | The Firstmate page with no clone installed |
| `firstmate-installing.png`, `firstmate-installing.json` | The clone is being installed |
| `firstmate-up-to-date.png`, `firstmate-up-to-date.json` | The clone matches upstream |
| `firstmate-update-available.png`, `firstmate-update-available.json` | Upstream has a new commit |
| `firstmate-modified.png`, `firstmate-modified.json` | A file in the clone was edited |
| `firstmate-blocked.png`, `firstmate-blocked.json` | An edit conflicts with the update, which is blocked |
| `firstmate-session.png`, `firstmate-session.json` | The chat running as firstmate in the managed clone |
| `firstmate-panels.png`, `firstmate-panels.json` | **Needs your input** filled from firstmate's records: two held tasks and a merge approval |
| `firstmate-crew.png`, `firstmate-crew.json` | Peeking at a crewmate's terminal from the agents list |
| `firstmate-answering.png`, `firstmate-answering.json` | After choosing **Answer** on a held task |
| `walk-sent.json` | `scripts/walkthrough.json` after typing and sending a message |
| `walk-end.png`, `walk-end.json` | The same script after answering an item, finishing the task and opening a peek |

The first four scenes use the stub. The sign-in scenes run the app on `scripts/fake-claude`, so
they need no Claude account; each waits for the fake's real processes before rendering. The
firstmate scenes run `git` on a scratch upstream repository under `target/`, so they need no
network. The
walkthrough runs on the stub's `chat` scene. The `.json` files are Rune element trees. The script then checks them for the keys and text each
state must or must not have (for example `needs-input`, `input-1`, `for-you-empty`, `peek`) and
exits non-zero if any check fails.

The individual commands, if you want to run one by hand:

```sh
B=target/debug/velvet-hammer
$B --list-scenes                                   # chat replied empty peek signed-out ...
$B --scene empty --snap empty.png                  # render a scene to PNG
$B --scene chat --tree chat.json                   # dump a scene's element tree
$B --scene chat --script scripts/walkthrough.json --at mark:end --snap walk-end.png
$B --scene chat --script scripts/walkthrough.json --at mark:sent --tree walk-sent.json
cargo test                                         # drives taps through Rune's Runtime
```

Add `--theme light` to render the light theme. A release build (`cargo build --release`) takes
the same arguments because the `tooling` feature is on.

## License

Velvet Hammer is released under the [MIT License](LICENSE).
