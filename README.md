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
approval. Which tool uses wait for you is a setting: by default, as recommended, the chief of
staff runs its tools without asking and every tool use is logged (see
[Tool approval](#tool-approval)). Ahead of that setting, the same hook enforces the firstmate rules
a tool call shows: a merge, a forced push, a teardown that discards work, a write into a project
or a publish is denied or waits for your word (see
[Firstmate's rules, enforced by hook](docs/firstmate-rules.md)). The chief of staff shows status
digests, lists, choices and tables as views by default, and a reply of that kind that arrives as
text is sent back once for a view (see [Visual replies](docs/visual-replies.md)). **For you to do** shows the notes Claude Code adds with the app's `app_note` tool. The
agents list shows the tasks firstmate has in flight, and selecting one shows that crewmate's
terminal. The **Projects** list shows the projects in firstmate's records, and **Add project**
starts a guided conversation in which the chief of staff adds one with firstmate's own procedure;
with no projects yet, the chat opens with a prompt to add the first (see
[Projects](docs/firstmate.md#projects)). [Where the integration stands](docs/firstmate.md#where-the-integration-stands) lists
what works end to end and what is not verified. A stub with sample data stands in for the agent layer in headless scenes and tests. The interface
and code will change.

The sign-in flow is tested against a fake `claude`; signing in with a real Claude account has not
been tested yet (see [Trying the real sign-in](#trying-the-real-sign-in)).

## What works today

The window has three columns:

- **Agents** (left): the subagents the chief of staff manages. Selecting one opens a peek at its
  transcript in place of the chat; **Back to chat** closes it. Under them, **Projects** lists the
  projects in firstmate's records with **Add project**. A **Firstmate** card at the bottom
  shows the state of the managed firstmate clone and opens a page for installing and updating it.
  Above it, **Visual replies**, **Firstmate rules** and **Tool approval** cards show each
  setting in a few words and open its page; the tool-approval page links to the activity log.
- **Chief of staff** (middle): the conversation, with a message field and a **Send** button. A
  line under the heading says what runs it, such as "Claude Code · signed in with your Claude Max
  plan". When Claude Code is not ready, a sign-in screen takes the conversation's place. With no
  projects yet, a card above the conversation explains them and offers **Add a project**; while
  you add one, a row above the message field offers **Choose a folder…**, **Yes**, **No** and
  **Done adding**.
  The chief of staff's messages are Markdown, shown formatted: bold, italic, strikethrough,
  inline code and code blocks, headings, lists, block quotes and links. A line break shows as
  written, HTML shows as text, and a reply still arriving never shows a half-written marker. A link
  opens in your default browser when you press it, and only `http`, `https` and `mailto` links
  open. Your own messages and error lines show as written.
  Each tool use the chief of staff makes appears where it happened: an image it read or made
  shows inline (press it to see it larger), a video file plays inline with **Play** and
  **Open in your video player**, and any other tool shows its name, its main argument and its
  result folded behind a button. See [Tool calls in the chat](docs/tool-calls.md).
  The chief of staff can also show you a small interactive view, a json-render spec it passes to
  the app's `app_ui` tool: cards, tables, badges, progress bars, buttons, text fields,
  checkboxes and choice selectors, drawn natively and filling in as the spec streams. Ticking,
  typing and adding stay in the view; a submit comes back to the chief of staff as a message from
  you, with the values you entered. A question with a fixed set of answers, such as yes or no,
  comes as a selector with the recommended option marked, and your click goes back as that
  option's exact value. Nothing in a view runs code, reads files or reaches the network. See
  [Views in the chat](docs/json-render.md).
- **Needs your input** and **For you to do** (right): questions the agents are waiting on, each
  with answer buttons, and steps only you can take, each with a **Done** button.

## Layout of the code

- `src/model.rs`: the data model. A `Workspace` holds the chat `Message`s, the `Item`s waiting on
  the captain and the subagent `Agent`s. Everything the captain does is an `Action`
  (`Say`, `Answer`, `Done`, and `Ui` for a control in a view), and `Workspace::apply` records it.
- `src/agent.rs`: the agent-layer boundary. The app talks only to the `ChiefOfStaff` trait, which
  takes the captain's actions and sign-in requests and reports `Event`s (status changes, streamed
  replies, problems). `Stub` simulates it for headless scenes and tests.
- `src/claude.rs`: `Claude`, the chief of staff on the user's own Claude Code. It runs only the
  `claude` command, as described below.
- `src/bridge.rs`: the server the chat session's `claude` calls back into: two MCP tools and three
  hooks, PreToolUse, UserPromptSubmit and Stop (see [docs/extending.md](docs/extending.md)).
- `src/firstmate.rs`: `Firstmate`, the managed firstmate clone. It runs `git` to install the
  clone, compare it with upstream and update it, and never discards a local change. Its `lab`
  module builds scratch git repositories for tests and scenes.
- `src/approval.rs`: the tool-approval settings (a mode and custom rules), the decision the hook
  makes for each tool use, and the activity log.
- `src/guard.rs`: firstmate's rules the PreToolUse hook enforces, each matching one tool-call
  shape (see [docs/firstmate-rules.md](docs/firstmate-rules.md)).
- `src/visual.rs`: visual replies: the mode, the shapes of reply that read better as a view, the
  standing instruction and the stop check with its retry cap (see
  [docs/visual-replies.md](docs/visual-replies.md)).
- `src/store.rs`: a settings file in the data directory, written whole and shared with the hooks;
  tool approval, visual replies and firstmate's rules each keep one.
- `src/projects.rs`: the projects in firstmate's records (its registry `data/projects.md` and
  the clones under `projects/`), re-read every two seconds, and the system folder dialog the add
  flow uses.
- `src/bearings.rs`: what waits on the captain, read from firstmate's records by running its
  `bin/fm-bearings-snapshot.sh --json` whenever the records change.
- `scripts/fake-claude`: a stand-in for `claude` that speaks the same protocol without an account.
  Tests and the sign-in scenes run the app on it.
- `src/tools.rs`: the chief of staff's tool uses and the registry that picks how each is drawn,
  keyed on the Model Context Protocol's content types and the Agent Client Protocol's tool kinds
  (see [docs/tool-calls.md](docs/tool-calls.md)).
- `src/ui.rs`: json-render views: the catalog, reading a spec (whole or still streaming) within
  its limits, its state, the tree to draw and the actions its controls fire (see
  [docs/json-render.md](docs/json-render.md)).
- `src/markdown.rs`: prepares the chief of staff's Markdown for Rune's Markdown view, keeping
  line breaks and showing HTML as text.
- `src/view.rs`: the Rune view, with the tool-use views in `src/view/tool.rs`, the
  json-render views in `src/view/ui.rs`, and the visual-replies and firstmate-rules pages in
  `src/view/policy.rs`. Every element a
  test or script reaches has a key.
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
  to **For you to do**, its `app_ui` tool shows a view in the chat, and its PreToolUse hook applies
  [firstmate's rules](docs/firstmate-rules.md) and your [tool-approval](#tool-approval) settings
  to every tool use: it runs it, refuses it, or asks you in **Needs your input**. Its
  UserPromptSubmit and Stop hooks carry [visual replies](docs/visual-replies.md). Your Claude
  settings files are not changed. See
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

### Tool approval

Every tool use of the chief of staff goes through the app's PreToolUse hook, which decides it
with settings you choose on the **Tool approval** page (the card above **Firstmate** in the
agents column). While the chief of staff runs as firstmate, [firstmate's rules](docs/firstmate-rules.md)
are checked first; a rule's deny is final, and its question is asked whatever these settings say.

- **Mode.** **Run without asking** (recommended, and what a fresh install uses): the chief of
  staff will edit files, run commands and use tools without stopping for you, and every tool use
  is still logged. **Auto-allow read-only tools**: `Read`, `Glob`, `Grep`, `LS`, `NotebookRead`,
  `ToolSearch`, `TodoWrite` and the app's own `app_note` and `app_ui` run at once, and everything else waits
  for you. **Ask every time**: every tool use waits for you.
- **Rules.** An ordered list of allow, ask and deny rules on top of the mode. A rule names a
  tool (`*` matches any text, so `mcp__github__*` covers a server's tools) and optionally a
  pattern for its main argument. A Bash pattern matches a command that is the pattern or starts
  with it and a space, and it is checked against each part of a command joined with `&&`, `;`,
  `|`, `$( )` or backticks: an allow rule must match every part, an ask or deny rule any part.
  Other patterns are globs on the file path (Read, Edit, Write), URL (WebFetch) or the whole
  input. A matching deny rule always wins, then ask, then allow, whatever their order; ask brings
  up the full-input question even in **Run without asking**. Rules are a convenience filter,
  not a sandbox: they compare text, so extra spaces (`git  push`), an `env` or `sudo` prefix,
  `bash -c "git push"`, a script that runs the command, or a relative or `..` path gets past
  one.
- **Asking.** A tool use that waits for you appears in **Needs your input** with all of its
  input, as before. An input larger than 64 KB that would be asked about is denied, because
  approving a partial view would approve what you never saw.
- **Trying it.** The page shows what the settings decide for an example tool call you type.
  **Reset to the recommended default** asks once before it deletes your rules.
- **Activity log.** Every tool use, with the start of its command or path (200 characters) and
  how it was decided: allowed or denied by the mode or by which rule, or asked and what you
  answered. It is kept in `tool-activity.jsonl` in the app's data directory
  (`~/.velvet-hammer`, or `$VELVET_HAMMER_DATA`), which the app moves to
  `tool-activity.jsonl.1` when it passes 1 MB. It is readable by your user only (mode 0600) but
  holds command text as typed, so avoid putting secrets in commands.
- **Where the settings live.** `tool-approval.json` in the same directory (versioned, written to a
  temporary file and renamed into place), never in your Claude settings or a repository.
  Changes apply to a running session at once. If the file cannot be read, every tool use asks
  you and the page says why; the next change keeps the old file as `tool-approval.json.bad`.

No setting changes the app's fixed limits: it never reads or stores your Claude sign-in, it
always removes `ANTHROPIC_API_KEY` and `ANTHROPIC_AUTH_TOKEN` from `claude`'s environment, and it
never pushes, publishes or merges anything itself. In **Run without asking** the chief of staff
runs any command that no firstmate rule matches; with the firstmate rules on, as they are by
default, a push or a merge through firstmate's script waits for you, and `gh pr merge` or a forced
push is denied. Add an ask or deny rule for any other command you want to see first.

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
| `approval-settings.png`, `approval-settings.json` | The Tool approval page: the recommended mode, three rules and the example |
| `approval-by-rule.png`, `approval-by-rule.json` | An ask rule bringing up the full-input question while running without asking |
| `approval-activity.png`, `approval-activity.json` | The activity log after a command run without asking, one denied by a rule and one you allowed |
| `approval-corrupt.png`, `approval-corrupt.json` | The Tool approval page when its settings file could not be read |
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
| `projects-fresh.png`, `projects-fresh.json` | A fresh install: no home yet, no projects, and adding waits for the chat |
| `projects-empty.png`, `projects-empty.json` | An installed home with no projects: the prompt to add the first one |
| `projects-adding.png`, `projects-adding.json` | After **Add a project**: the request in the chat, the reply and the add controls |
| `projects-listed.png`, `projects-listed.json` | Three projects from firstmate's registry |
| `formatted.png`, `formatted.json` | A reply that once showed raw `**` asterisks, formatted |
| `formatted-all.png`, `formatted-all.json` | Every Markdown construct the chat formats, and HTML shown as text |
| `formatted-streaming.png`, `formatted-streaming.json` | A reply still arriving, with its bold not yet closed |
| `tool-image.png`, `tool-image.json` | An image the Read tool returned, inline |
| `tool-image-broken.png`, `tool-image-broken.json` | An image that cannot be decoded: its path and why |
| `tool-video.png`, `tool-video.json` | A video file, with its poster frame, **Play** and **Open in your video player** |
| `tool-generic.png`, `tool-generic.json` | A Bash command with its output unfolded |
| `tool-picture.png`, `tool-picture.json` | An image from the chat, enlarged |
| `view-static.png`, `view-static.json` | A json-render view: a card with badges, a progress bar, a table, an alert and buttons |
| `view-streaming.png`, `view-streaming.json` | The same view with a little over half of its spec streamed |
| `view-state.png`, `view-state.json` | A checklist after ticking an item, typing and adding one, and typing again: inputs bound to state and a repeat |
| `view-action.png`, `view-action.json` | After **Send the list**: what you did in the chat, and the reply to the submit |
| `view-unknown.png`, `view-unknown.json` | A view naming a component the catalog lacks, shown as a placeholder |
| `view-too-large.png`, `view-too-large.json` | A view over the element limit, refused with why |
| `choice-ask.png`, `choice-ask.json` | A question with a fixed set of answers asked with a Choice: yolo off recommended, yolo on, and an Other field |
| `choice-answered.png`, `choice-answered.json` | After choosing **yolo off**: the answered Choice, your choice in the chat, and the reply to its exact value |
| `choice-multiple.png`, `choice-multiple.json` | A Choice that takes several picks, with two picked and an answer of your own typed |
| `choice-enforced.png`, `choice-enforced.json` | A question the fake asked in prose, sent back by the stop hook, asked again with a Choice and answered |
| `visuals-settings.png`, `visuals-settings.json` | The Visual replies page: Enforce recommended, the shapes, the instruction and an example |
| `visuals-enforced.png`, `visuals-enforced.json` | A list the fake sent as text, sent back once by the stop hook and shown as a view |
| `visuals-capped.png`, `visuals-capped.json` | A reply that ignored the feedback, sent back only once |
| `rules-settings.png`, `rules-settings.json` | The Firstmate rules page after a forced push was denied and a merge was asked about and allowed |
| `rules-merge-ask.png`, `rules-merge-ask.json` | A merge through firstmate's script waiting for you, naming the pull request |
| `rules-project-ask.png`, `rules-project-ask.json` | A write into a project waiting for you, naming the file |
| `walk-sent.json` | `scripts/walkthrough.json` after typing and sending a message |
| `walk-end.png`, `walk-end.json` | The same script after answering an item, finishing the task and opening a peek |

The first four scenes, `approval-settings`, the `formatted` scenes, the `view` scenes and
`visuals-settings` use the stub; the other `visuals` scenes have the fake `claude` call the prompt
and stop hooks; the `rules` scenes run the fake `claude` as firstmate in a scratch clone under
`/tmp/velvet-hammer`; the `tool`
scenes have the fake `claude` use Read and Bash on the files in `scripts/fixtures`. The sign-in scenes run the app on `scripts/fake-claude`, so
they need no Claude account; each waits for the fake's real processes before rendering. The
firstmate and projects scenes run `git` on a scratch upstream repository under `target/`, so
they need no network; the projects scenes write the registry firstmate's procedure would write,
because the fake `claude` cannot run that procedure. The
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

## Pipeline validation

Changes go through the [no-mistakes](https://kunchenguid.github.io/no-mistakes/) pipeline, and
`.no-mistakes.yaml` tells it how to validate this project. Its `commands` fetch dependencies, run
`cargo test`, lint with `cargo fmt --check` and `cargo clippy -D warnings`, and format with
`cargo fmt`, each with rustup's proxies first on `PATH` so the pinned toolchain runs instead of
Homebrew's `rust`. Its `test.instructions` is the runbook the Test step's agent follows: run
`scripts/verify.sh`, require it to exit zero, and open the rendered scenes the change affects.
The runbook excludes what cannot run in the pipeline: a real Claude sign-in, the windowed app,
and a real firstmate home that pushes or merges.

no-mistakes reads `commands` and `test.instructions` from `main`, not from the pushed branch, so
an edit to them applies to the runs after it merges.

## License

Velvet Hammer is released under the [MIT License](LICENSE).
