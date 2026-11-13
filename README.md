# velvet-hammer

Native agentic development environment built with [Rune](https://github.com/anomalyco/rune).

The captain talks to one chief of staff, which delegates to subagents. The window has three
columns:

- **Agents** (left): the subagents the chief of staff manages. Selecting one opens a peek at its
  transcript in place of the chat; **Back to chat** closes it.
- **Chief of staff** (middle): the conversation, with a message field and a **Send** button.
- **Needs your input** and **For you to do** (right): questions the agents are waiting on, each
  with answer buttons, and steps only the captain can take, each with a **Done** button.

This is a skeleton. A stub stands in for the agent layer: it seeds sample data and replies with
canned messages. There is no real agent, networking or persistence yet.

## Layout of the code

- `src/model.rs`: the data model. A `Workspace` holds the chat `Message`s, the `Item`s waiting on
  the captain and the subagent `Agent`s. Everything the captain does is an `Action`
  (`Say`, `Answer`, `Done`), and `Workspace::apply` records it.
- `src/agent.rs`: the agent-layer boundary. The app talks only to the `ChiefOfStaff` trait;
  `Stub` implements it today. A Pi-backed chief of staff replaces `Stub` by implementing the
  same trait.
- `src/view.rs`: the Rune view. Every element a test or script reaches has a key.
- `src/main.rs`: the `App` state (workspace, chief of staff, message draft, peeked agent), the
  headless scenes, and the tests.

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
| `walk-sent.json` | `scripts/walkthrough.json` after typing and sending a message |
| `walk-end.png`, `walk-end.json` | The same script after answering an item, finishing the task and opening a peek |

The `.json` files are Rune element trees. The script then checks them for the keys and text each
state must or must not have (for example `needs-input`, `input-1`, `for-you-empty`, `peek`) and
exits non-zero if any check fails.

The individual commands, if you want to run one by hand:

```sh
B=target/debug/velvet-hammer
$B --list-scenes                                   # chat replied empty peek
$B --scene empty --snap empty.png                  # render a scene to PNG
$B --scene chat --tree chat.json                   # dump a scene's element tree
$B --script scripts/walkthrough.json --at mark:end --snap walk-end.png
$B --script scripts/walkthrough.json --at mark:sent --tree walk-sent.json
cargo test                                         # drives taps through Rune's Runtime
```

Add `--theme light` to render the light theme. A release build (`cargo build --release`) takes
the same arguments because the `tooling` feature is on.
