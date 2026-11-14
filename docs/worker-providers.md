# Workers on other providers

The chief of staff can send crewmates on models other than Claude, such as DeepSeek V4.1 Flash on
an opencode Go subscription, without the captain installing any agent command-line tool for them.
Those workers run on [pi](https://github.com/earendil-works/pi), a coding agent that Velvet
Hammer downloads and manages itself, and firstmate launches them through its existing, verified
`pi` harness. Claude workers are unchanged: they run on Claude Code and the captain's Claude
subscription, never through pi or an API key.

The captain can also give a worker setting a nickname. "Send a hacker to look into the flaky
test" sends the `hacker` preset; "send an opencode go deepseek 4.1 flash worker to investigate"
names a provider and model directly.

The **Workers** card in the agents column opens the page for all of this.

## How it fits together

| Piece | What it does | Where |
| --- | --- | --- |
| The pi runtime | **Install pi** downloads pi's official release for this computer from GitHub with `curl`, checks it against the SHA-256 built into the app, and unpacks it into `~/.velvet-hammer/runtime/pi-1.5.2/`. A download that does not match is not installed. | `src/pi.rs` |
| The chief of staff's `PATH` | The runtime's directory goes first on the `PATH` of the chief of staff's `claude` process. Firstmate's `bin/fm-spawn.sh` resolves `pi` from that `PATH` and writes its absolute path into each worker's launch, so a worker's terminal pane needs no `PATH` of its own. Claude Code is found and signed in as before. | `src/main.rs` (`init`) |
| The key | The opencode Go key stays in the macOS keychain, as the generic password `velvet-hammer-opencode-go`. The captain adds it with `security add-generic-password`, which prompts for it, so it never appears in a command line, a file or the shell history. The app never reads it; it only asks the keychain whether the entry exists. | `src/pi.rs` (`key_state`) |
| Firstmate's pi account | **Point pi workers at Velvet Hammer** writes two files and nothing else: `config/pi-account` in the app's firstmate home, naming the app's pi directory `~/.velvet-hammer/pi-agent` and the one provider pi workers may spend (`opencode-go`), and that directory's `auth.json`, whose `opencode-go` entry is the command `!security find-generic-password -s velvet-hammer-opencode-go -w`. pi runs that command when it needs the key. The app writes these only when the captain presses the button; the captain's own `~/.pi`, any other account file and Claude's sign-in are not touched. | `src/pi.rs` (`connect`) |
| Nicknames | `config/crew-nicknames.json` in the app's firstmate home (gitignored with the rest of firstmate's `config/`). With no file, the app offers one example, `hacker`, from memory; the file is written only when the captain adds, edits or deletes a nickname on the Workers page. | `src/crew.rs` |
| Request resolution | With each of the captain's messages, the app's `UserPromptSubmit` hook tells the chief of staff the nicknames and how to send a worker, and, when the message asks for one, what the app made of it. | `src/crew.rs` (`request`, `resolve`, `context`), `src/bridge.rs` |
| Claude stays on the subscription | A Claude model never runs through pi. The registry refuses one, the resolver never offers one, and the firstmate rule **Claude workers run only on your subscription** denies a spawn that tries (see [Firstmate's rules](firstmate-rules.md)). | `src/crew.rs`, `src/guard.rs` |
| The fleet view | Each worker in the agents list shows what it runs on, such as `pi · opencode-go/deepseek-v4.1-flash`, from the `harness=` and `model=` lines of firstmate's `state/<id>.meta`. | `src/bearings.rs` (`runs`) |

### The registry's shape

```json
{
  "version": 1,
  "nicknames": [
    {"nickname": "hacker", "harness": "pi", "model": "opencode-go/deepseek-v4.1-flash"},
    {"nickname": "night-owl", "harness": "claude", "model": "opus", "effort": "high"}
  ]
}
```

- `nickname`: 1 to 32 lowercase letters, digits and hyphens, unique. "Send the night owl" matches
  `night-owl`.
- `harness`: `pi` for another provider, or `claude` for Claude Code on the captain's
  subscription. These are firstmate's own harness names.
- `model`: for `pi`, `<provider>/<id>` exactly as `pi --list-models` lists it; a Claude model is
  refused. For `claude`, a Claude Code model such as `sonnet` or `claude-sonnet-5-5`.
- `effort` (optional): `low`, `medium`, `high`, `xhigh` or `max`, passed as `--effort`.

A file with any entry the app cannot use is not read at all: no nickname is offered to the chief
of staff, the Workers page says why, and the next change keeps the old file as
`crew-nicknames.json.bad`.

### What the chief of staff is told

With every message, while it runs as firstmate, the chief of staff learns the nicknames as
`bin/fm-spawn.sh` flags, and these instructions: spawn a worker with the normal firstmate script,
add the preset's or model's flags and a `--dispatch-override` reason that quotes the captain's
request, run a Claude model only with `--harness claude`, ask the captain one short question when
the request is ambiguous or unknown, never start another harness's command-line tool to judge a
candidate, and never edit account pin files.

When the message asks for a worker ("send", or "dispatch", then the words up to "worker",
"crewmate", "agent", "to" or "and"), the app resolves those words and adds what it found:

1. A nickname, matched with spaces read as hyphens.
2. A Claude model, for any of the words claude, anthropic, sonnet, opus, haiku or fable: send it
   with `--harness claude` as usual.
3. The models pi lists. Every word must appear in a model's id, read with `/`, `-` and `_` as
   spaces and a `v` before a version number dropped, so "opencode go deepseek 4.1 flash" matches
   `opencode-go/deepseek-v4.1-flash`. One match is sent; among several, the one with no other
   words wins; otherwise the chief of staff is told to ask which, and is given up to six
   candidates.
4. Nothing: the chief of staff asks one short question, or, while pi lists no models, tells the
   captain to finish setting up on the Workers page.

## Setting it up

1. Open **Workers** and press **Install pi**.
2. Copy your API key from the [opencode console](https://opencode.ai/console), then run this in
   Terminal and paste the key when `security` asks for the password:

   ```sh
   security add-generic-password -U -a "$USER" -s velvet-hammer-opencode-go -w
   ```

   Press **Check again**. DeepSeek models also need **Global** chosen under Settings, Privacy,
   Regions in the opencode console.
3. Press **Point pi workers at Velvet Hammer**. The page then lists how many models pi can use.
4. Ask your chief of staff: "send a hacker to investigate the flaky test".

## Screenshots

| Scene | Screenshot |
| --- | --- |
| Nothing set up yet | [workers-fresh.png](screens/workers/workers-fresh.png) |
| pi installed from a stand-in release, the key found, firstmate pointed at the runtime, and a second nickname | [workers-settings.png](screens/workers/workers-settings.png) |
| A Claude model refused for pi | [workers-claude-refused.png](screens/workers/workers-claude-refused.png) |
| The agents list showing what each worker runs on | [workers-crew.png](screens/workers/workers-crew.png) |

## What was verified

- **Host tests:** the registry's parsing, validation and round trip, and the page's add, edit and
  delete (`crew::tests`); request extraction and resolution, including ambiguity and Claude
  (`crew::tests`); the hook context with and without firstmate (`bridge::tests`); the guard rule
  (`guard::a_claude_model_is_never_spawned_through_pi`); installing a stand-in release, refusing
  a tampered one and a failed download, the `PATH` order, the model-listing parser, and
  `connect` writing only `config/pi-account` and `auth.json` with mode 0600 (`pi::tests`); and
  reading a worker's harness and model from its meta record (`bearings::tests`); and opening the
  app leaving a firstmate home untouched (`opening_the_app_writes_nothing_into_the_firstmate_home`
  in `main.rs`). All of them run
  in scratch directories, never in the captain's firstmate home.
- **Headless scenes:** the four screenshots above, checked by `scripts/verify.sh`.
- **The real pi 1.5.2 release**, by hand on macOS (arm64): the archive matched the checksum in
  the release's `SHA256SUMS`; it holds one executable, `pi/pi`, whose process name `pi` is the one
  firstmate's process detection accepts; `--help` offers the `--tui-mode`, `--approve` and
  `--thinking` flags firstmate probes for; and `pi --list-models` with an app-style
  `PI_CODING_AGENT_DIR` lists the opencode Go models (32 of them, including
  `opencode-go/deepseek-v4.1-flash`) once the key command resolves, and none while it does not.
- **opencode Go:** its endpoint for DeepSeek is OpenAI Chat Completions at
  `https://opencode.ai/inference/go/openai/v1`, and a request without an `x-opencode-session`
  header is refused (`MissingSessionID`), which pi sends itself. The opencode Go documentation
  lists pi among the clients set up for Go.

**Not verified:**

- A real request to opencode Go: no valid key was available, and no stored key was used.
- A real pi worker spawned by firstmate under the app, through the account setting.
- Firstmate's `pi` adapter is recorded as verified with pi 0.82.0; the app installs 1.5.2.
- Windows: pi's Windows builds are zip files, which the app does not install.

### The captain's live check

1. Complete [Setting it up](#setting-it-up).
2. In Terminal, check that pi reaches opencode Go with the app's directory:

   ```sh
   PI_CODING_AGENT_DIR=~/.velvet-hammer/pi-agent \
     ~/.velvet-hammer/runtime/pi-1.5.2/pi/pi -p --model opencode-go/deepseek-v4.1-flash "Say hello"
   ```

3. Ask the chief of staff "send a hacker to list the files in the README's directory" on a
   scratch project, and watch the worker appear in the agents list as
   `pi · opencode-go/deepseek-v4.1-flash`.

## Why pi, and not a worker built into the app

Three ways were compared before building. Firstmate runs every crewmate in a terminal pane and
refuses a harness it has not verified, so the choice was mostly about firstmate's contract.

| | Chosen: an app-managed pi runtime | A native Rust worker in the app | Workers run by the app outside firstmate |
| --- | --- | --- | --- |
| Firstmate changes | None: firstmate's verified `pi` harness launches it. | A new harness adapter across about eight scripts, a harness reference and live verification, upstream in firstmate. | None, but firstmate's watcher, control, recovery and delivery would not apply. |
| Effort | Days. | Weeks: an agent loop, an interactive terminal prompt, interrupt handling and busy reporting. | Rejected. |
| Tool use | pi's coding tools and system prompt. | Starts below pi. | — |
| Size | About 48 MB, downloaded on first use. | A few MB inside the app. | — |

## Follow-ups for firstmate

- Re-verify the `pi` adapter against a current pi release (its record is for 0.82.0).
- Firstmate's bearings snapshot does not carry a worker's harness and model, so the app reads
  them from `state/<id>.meta`. Carrying them in the snapshot would let the app render
  firstmate's document only.
