# Workers on other providers: design note

Status: proposal, waiting for a decision. Nothing in this note is built yet.

The captain wants crewmates that run on models other than Claude, for example DeepSeek V4.1
Flash on an opencode Go subscription, without the user having any agent command-line tool
(`claude`, `opencode`, `codex` or `pi`) installed for those workers. The captain also wants
nicknames for provider and model settings ("send a hacker to investigate") and wants the chief of
staff to understand requests such as "send an opencode go deepseek 4.1 flash worker to
investigate".

**Recommendation: option B, a pi runtime that Velvet Hammer downloads and manages itself, launched
through firstmate's existing, verified `pi` harness.** It needs no change to firstmate, keeps every
firstmate construct (task id, worktree, status file, steering inbox, no-mistakes delivery), keeps
the API key in the macOS keychain, and is days of work rather than weeks. Option A, a native Rust
worker inside Velvet Hammer, is the right long-term shape only if bundling pi proves unworkable,
because it needs a new firstmate harness adapter and its own verification.

Research was done on 2026-11-13 against firstmate `bb3b2d69` in the captain's home, pi 1.5.2
and the opencode Go documentation of that date.

## What opencode Go offers a client

Verified from `opencode models --verbose` on this machine and from the
[Go documentation](https://opencode.ai/v2/docs/console/go/):

| Fact | Value |
| --- | --- |
| Base URL | `https://opencode.ai/inference/go` |
| DeepSeek V4.1 Flash | Wire id `deepseek-v4.1-flash`; CLI id `opencode-go/deepseek-v4.1-flash`. Native API is OpenAI Chat Completions at `openai/v1/chat/completions`. Tool calling and reasoning are listed as supported. |
| Other API shapes | Anthropic Messages, OpenAI Responses and Gemini endpoints exist under the same base for other models; the console translates between them. |
| Authentication | `Authorization: Bearer <key>`, a key copied from the opencode console. |
| Required headers | A stable `x-opencode-session` per conversation, and the client's own `User-Agent` (not a generic HTTP library name). A request without the session header fails with `MissingSessionID`; this was reproduced. |
| Third-party use | Allowed for "OpenCode and other coding agents that produce similar types of requests". The docs list pi, Claude Code, Codex and Hermes as set up clients and say pi sends the session header. |
| Account setting | DeepSeek models are blocked until a workspace admin chooses Global under Settings, Privacy, Regions. |

A live tool-call request was not completed: the opencode Go key stored on this machine is
expired (`api_key_expired`). The header requirement was confirmed against the live endpoint.

## How a worker must fit firstmate

Firstmate launches every crewmate in a terminal pane (tmux, herdr or rex) through
`bin/fm-spawn.sh`. A harness adapter must provide all of the following, and firstmate's router
forbids dispatching on an adapter that has not been verified live:

- A launch command that takes the whole brief as one argument and starts the first turn at once.
- An interactive prompt that stays open after the first turn and accepts typed lines, because
  firstmate types the inbox doorbell, replies and the exit command into the pane.
- A fixed process name and a launch marker variable, so `bin/fm-harness.sh` and
  `bin/fm-agent-process-lib.sh` can recognise it.
- Interrupt and exit keys in `bin/fm-control-lib.sh`, an input-box layout in
  `bin/fm-composer-lib.sh`, and turn-start and turn-end reports through `bin/fm-busy-event.sh`
  and `state/<id>.turn-ended`.
- A shell tool that can append to the status file and move inbox records, because the worker
  carries out the status and steering protocol itself.

A space in `--harness` turns it into a raw launch command, but that form exists only to verify a
new adapter: it cannot be set in `config/crew-harness`, `crew-dispatch.json` refuses it without an
override, and interrupt, exit, relaunch and busy tracking do not work for it.

## The options

| | A. Native Rust worker in Velvet Hammer | B. Managed pi runtime (recommended) | C. Workers run by the app outside firstmate |
| --- | --- | --- | --- |
| What it is | A `velvet-hammer worker` binary with its own agent loop over OpenAI-compatible chat completions and shell, read, write and edit tools, registered as a new firstmate harness. | The app downloads pi's official standalone release, puts it on the chief of staff's `PATH`, and the chief of staff spawns `--harness pi --model opencode-go/<id>`. | The app calls the provider directly and writes firstmate-shaped records itself. |
| Installed tools the user needs | None besides firstmate's existing companions (tmux, git, no-mistakes). | Same. The app installs pi the way it installs the firstmate clone. | Same. |
| Firstmate changes | Required: a new adapter in about eight scripts, a harness reference, a verification record and live tests, upstream in firstmate. | None. `fm-spawn.sh` resolves `pi` from the caller's `PATH` and writes its absolute path into the launch, so the pane's own `PATH` does not matter. | None, but it bypasses `fm-spawn.sh`, so watcher, control, recovery and delivery do not apply. |
| Effort | Two to three weeks: agent loop, streaming, interactive terminal prompt, interrupt handling, busy reporting, the adapter, and verification. | Two to three days: download and checksum, a keychain-backed provider key, nickname registry and page, chief-of-staff instructions, one live crewmate run. | Rejected: it reimplements the fleet instead of using firstmate's constructs. |
| Tool-use quality | Starts below pi; prompt and tool design would need tuning per model family. | pi's mature coding tools (`read`, `bash`, `edit`, `write`) and system prompt; opencode lists pi as a set up client. | n/a |
| Licence | Ours (MIT). Candidate crates: `async-openai` (MIT) or a small `reqwest` client to control the session header. | pi is MIT. | n/a |
| Size | A few MB inside the app. | About 48 MB per platform (`pi-darwin-arm64.tar.gz` in v1.5.2), downloaded on first use, with a published `SHA256SUMS`. | n/a |
| Main risk | Large surface to build and keep verified against firstmate's control protocol. | Firstmate's pi adapter was last verified with pi 0.82.0; the app must pin a pi version that is checked live with firstmate, and pi updates move under us. | n/a |

## How keys are stored

The key never goes in the repository, the firstmate home's config, or a plain file.

- **Option B.** The app stores the key in the macOS keychain (Security framework through the
  `keyring` or `security-framework` crate, or the `security` command). It gives pi its own agent
  directory, `~/.velvet-hammer/pi-agent`, whose `auth.json` holds only a command that pi runs when
  it needs the key: `"key": "!security find-generic-password -ws velvet-hammer-opencode-go"`.
  pi documents this form and caches the output only in memory. Firstmate points pi workers at
  that directory through its worker account pin, `config/pi-account`, which sets
  `PI_CODING_AGENT_DIR` for every pi launch from the app's home and limits it to the providers
  named on its second line (for example `opencode-go`). The user's own `~/.pi`, if any, is not
  touched.
- **Option A.** The worker reads the key from the keychain at the start of each request.

## What stays the same in either option

- **Nickname registry.** `config/crew-nicknames.json` in the app's firstmate home, gitignored with
  the rest of `config/`, maps a nickname to a harness, a `provider/model` id and an optional
  effort. For option B the seed entry is `"hacker"` with harness `pi`, model
  `opencode-go/deepseek-v4.1-flash`, verified in `pi --list-models deepseek`.
- **Phrase resolution.** The chief of staff matches a nickname first, then a provider and model
  phrase against the runtime's own model list (`pi --list-models` for option B), and asks one
  short question when the phrase is ambiguous or unknown. It spawns with `bin/fm-spawn.sh
  --harness pi --model <provider/id>` and a `--dispatch-override` reason that quotes the
  captain's request. It never edits pin files and never starts another harness's command-line
  tool to judge a candidate.
- **The settings page** lists, adds, edits and deletes nicknames, and the agents list shows each
  worker's provider and model.

## Decisions needed

1. **Choose B (managed pi runtime) or A (native Rust worker).** Recommended: B.
2. **For B, may the app write `config/pi-account` in its own firstmate home?** This is the
   firstmate-native way to give pi workers a keychain-backed account without touching `~/.pi`.
   The earlier spec said never to edit account pin files; that rule was written for the chief of
   staff. Recommended: yes, written once by the app when the captain adds a provider key, never by
   the chief of staff.
3. **A fresh opencode Go key is needed for live verification,** and DeepSeek must be enabled in
   the opencode console's region setting. The key on this machine is expired.

## Follow-ups for firstmate

- Re-verify the `pi` adapter against a current pi release (the recorded verification is 0.82.0).
- For option A only: the new harness adapter described above.
