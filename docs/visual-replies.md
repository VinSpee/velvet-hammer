# Visual replies

The chief of staff can show a reply as a view in the chat with the app's `app_ui` tool (see
[Views in the chat](json-render.md)). Visual replies make it do that by default for the replies
that read better as a view, such as a status digest, a list of items that need a decision, a
pull request review or merge ask, a table or progress. A question with a fixed set of answers (yes
or no, pick one, pick several) is asked with a `Choice` selector, so the captain answers with one
click and the chief of staff gets the option's exact value. A plain conversational answer stays
plain text.

Two of Claude Code's hooks carry it, served by the app like its PreToolUse hook (see
[Extending](extending.md)):

- A **standing instruction**, added with each of the captain's messages, so the model reaches
  for the tool by default.
- A **check of each final reply**. When a reply of a qualifying shape arrives with no view in its
  turn, the app sends it back once with feedback asking for a view.

The **Visual replies** section in Settings (Cmd+, on macOS) opens the settings page.

## What Claude Code's hooks offer

Read on 2026-11-13 from the [hooks reference](https://code.claude.com/docs/en/hooks), against
Claude Code 2.1.311:

| Need | Hook | What the page says |
| --- | --- | --- |
| Add context before the model answers a message | `UserPromptSubmit` | Its `hookSpecificOutput.additionalContext` is added "alongside the submitted prompt" as a system reminder. Supports `http` hooks. Default timeout 30 seconds; a hook that fails or times out lets the prompt through without the context unless `onFailure` is `"block"`. |
| Add context at the start of a session | `SessionStart` | Supports only `command` and `mcp_tool` hooks, not `http`, so the app's server cannot answer it. |
| Check a finished reply | `Stop` | Runs "when the main Claude Code agent has finished responding". Its input has `last_assistant_message` (the text of the final response) and `stop_hook_active` (`true` "when Claude Code is already continuing as a result of a stop hook"). `decision: "block"` with a `reason`, or `hookSpecificOutput.additionalContext`, continues the conversation so Claude can act on the feedback; the second is labelled "Stop hook feedback" rather than a hook error. Supports `http` hooks. |
| Not loop | `Stop` | Hooks should check `stop_hook_active`. Claude Code also overrides a block after eight consecutive continuations. `onFailure` has no effect on `Stop`. |

The page also advises phrasing added context as facts rather than as system commands, because
text framed as out-of-band instructions can trigger Claude's prompt-injection defences.

## What the app does

`src/visual.rs` holds the mode, the rules for qualifying shapes, the instruction and the
decision. `src/bridge.rs` serves the two hooks, passed with `--settings` to the chat session
only, next to the PreToolUse hook:

- **`UserPromptSubmit`** (`/hooks/user-prompt-submit`, 30-second timeout, no `onFailure`):
  answers `additionalContext` with the instruction below in **Nudge only** and **Enforce**, and
  nothing in **Off**. It also marks the start of a new reply. If the app cannot answer, the
  message goes through without the instruction.
- **`Stop`** (`/hooks/stop`, 30-second timeout): in **Enforce**, reads `last_assistant_message`
  and, when it has a qualifying shape and no `app_ui` view was shown in the turn (for a question,
  no view with a `Choice`), answers
  `hookSpecificOutput.additionalContext` with the feedback below. Anything else gets an empty
  answer, which lets the turn end. The chat shows a line from Velvet Hammer saying the reply was
  sent back and why, so the captain sees what happened.

The app learns that a view was shown, and whether it had a `Choice`, when the session calls
`app_ui` on the app's MCP server, so it needs no transcript. A question asked in prose next to a
view without a `Choice` is still sent back; any view answers the other shapes.

**The retry cap.** A reply is sent back at most once:

- never while `stop_hook_active` is true;
- never twice in a row, even if Claude Code did not set `stop_hook_active` (the app remembers
  that it just sent one back);
- the next message from the captain, or the next reply after a stop the app let through, is
  checked afresh.

So a model that ignores the feedback ends its turn at the next stop, and nothing can loop or
hang. Claude Code's own cap of eight continuations is a second guard.

### The instruction

> Velvet Hammer shows this chat to the captain and draws views from the
> mcp__velvet-hammer__app_ui tool natively. The captain reads a reply better as a view when it is
> a status digest, a list of items that need a decision, a pull request review or merge ask, a
> table, or progress: such a reply is shown with app_ui (a json-render spec), with only a short
> sentence of text around it. A question with a fixed set of answers (yes or no, pick one of
> several, pick several) is asked with a Choice element in app_ui, with the recommended option
> marked, and is not also asked or answered in text; the captain's pick comes back as the
> option's exact value. A plain conversational answer stays plain text.

### The feedback

For a list, a table, a digest or a pull request ask:

> Velvet Hammer: this reply lists 3 items, which the captain reads better as a view. Show it now
> with the mcp__velvet-hammer__app_ui tool (options as Button elements whose press runs the
> submit action), with at most one short sentence of text, and do not repeat the reply. This is
> asked once; if a view does not fit, end the turn.

For a question (the choices and question shapes):

> Velvet Hammer: this reply asks a question with a fixed set of answers, which the captain
> answers with one click. Ask it now with the mcp__velvet-hammer__app_ui tool as a Choice element
> (the question, each option as a value and a label with a short hint, and the recommended
> option's value), with at most one short sentence of text, and do not ask the question or list
> the options again in text. This is asked once; if the question is open-ended, end the turn.

### Qualifying shapes

`visual::shape` decides, in this order, and nothing else does. Text inside fenced code blocks
does not count.

1. A Markdown table: a line that starts with `|` over a line of only `|`, `-`, `:` and spaces
   with `---` in it.
2. Choices: two or more list items and a line that ends with `?`. Needs a `Choice`.
3. A pull request ask: a link containing `/pull/<number>` and the word merge, review or approve.
4. A question with a fixed set of answers. Needs a `Choice`. Either a line ending with `?` whose
   last sentence starts with a yes-or-no word (do, does, did, should, shall, can, could, would,
   will, is, are, was, were, want, may, might, ok, okay, have, has, am, need, proceed, merge,
   approve, ready) or has ` or ` in it, such as "Do you want it off?" or "Squash it, or rebase it
   first?"; or a line with two or more quoted replies and ` or `, such as "Say 'yolo on' or 'yolo
   off'." An open question (what, how, why, which without options) does not count.
5. A status digest: two or more headings, either `#` to `######` or a line that is all bold, such
   as `**Underway**`.
6. A list: three or more list items (`-`, `*`, `+`, `1.` or `1)`).

These are deliberately plain. A short answer that happens to have two bullets and a question
counts as choices, and a closing "Should I start now?" counts as a question; a long prose answer
with no closed question never counts.

## Settings

The **Visual replies** page offers three modes, with **Enforce** recommended and used by a fresh
install:

| Mode | Instruction | Check |
| --- | --- | --- |
| Enforce (recommended) | Yes | Yes, with the retry cap |
| Nudge only | Yes | No |
| Off | No | No |

The page lists the shapes, shows the instruction and the feedback as sent, and has a **Try a
reply** field that says what the check would do with a reply you type. The mode is kept in
`visual-replies.json` in the app's data directory (`~/.velvet-hammer`, or
`$VELVET_HAMMER_DATA`), written whole like the tool-approval settings, and applies to a running
session at once. If the file cannot be read, the app nudges only and the page says why; the next
change keeps the old file as `visual-replies.json.bad`.

## Verified

- `cargo test` covers the shapes, the retry cap, the instruction per mode and the setting's file
  (`src/visual.rs`), both hooks over HTTP (`src/bridge.rs`), and the whole exchange on the fake
  `claude` (`src/main.rs`): a list sent back once and shown as a view, a reply that ignores the
  feedback sent back only once, and the instruction following the mode.
- `scripts/fake-claude` calls the prompt hook for each message and, for `list:`, `stubborn:` and
  `ask:` messages, the stop hook the way Claude Code does.
- The merge-authority case that prompted the `Choice`: the fake asks "Merge authority for
  velvet-hammer is yolo on. Recommended: yolo off. Do you want it off?" in prose, the stop hook
  sends it back, the fake asks it with a `Choice`, and choosing yolo off sends the fake the value
  `"yolo-off"` (`a_question_asked_in_prose_is_asked_again_with_a_choice_and_its_value_comes_back`
  in `src/main.rs`, and the `choice-enforced` scene).
- With the real `claude` (2.1.311, Haiku), the ignored test
  `real_claude_shows_a_list_as_a_view` was run on 2026-11-13. Asked "Name three fruits I could buy
  today, as a short list.", the model answered with a numbered list as text despite the
  instruction; the stop hook sent it back; the model loaded `app_ui` with `ToolSearch` and showed
  the three fruits as buttons with a submit action, and the turn ended. In an earlier run the
  prompt asked for "no tools", and after the feedback the model said it would leave the list as
  text because the captain asked for no tools, and the turn ended. Both runs confirm that an HTTP
  `Stop` hook's `additionalContext` continues the turn and that the cap ends it.

Not verified: how often the instruction alone is enough with other models (in both Haiku runs
it was not), and the behaviour when firstmate's own `Stop` hook continues the same turn. With the
real `claude`, a question asked in prose being sent back and asked again with a `Choice` has not
been run; the fake covers the exchange.

## Screenshots

| Scene | Screenshot |
| --- | --- |
| The settings page | [visuals-settings.png](screens/hooks/visuals-settings.png) |
| A list sent back once and shown as a view | [visuals-enforced.png](screens/hooks/visuals-enforced.png) |
| A reply that ignored the feedback, sent back only once | [visuals-capped.png](screens/hooks/visuals-capped.png) |
| A question asked in prose, sent back, asked with a `Choice` and answered with yolo off | [choice-enforced.png](screens/choice/choice-enforced.png) |
