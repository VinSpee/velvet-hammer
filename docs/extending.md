# Extending the chief of staff

Velvet Hammer can route Claude Code's extension points (tools, hooks, skills, subagents and
permission prompts) through itself without the Claude Agent SDK. It does this by starting the
user's own installed `claude` with command-line flags, so the subscription sign-in keeps working.
This page describes what Claude Code offers, what the app does today, and what Anthropic's current
documentation and terms say about the subscription. It is a summary of Anthropic's pages, read on
2026-11-13 against Claude Code 2.1.311. It is not legal advice.

## What the app does today

Each chat session starts a small HTTP server on `127.0.0.1` (`src/bridge.rs`) and passes it to
that one `claude -p` process with two flags. Nothing is written to the user's Claude settings.

- **A tool.** `--mcp-config` names the app's MCP server, `velvet-hammer`, over streamable HTTP.
  It has five tools. `app_note` adds a note to the **For you to do** panel; marking the note
  done tells the chief of staff which note it was. `app_ui` shows a json-render view in the chat
  (see [Views in the chat](json-render.md)). `app_secondmate` opens the guided flow for adding a
  second mate, with the scope the captain described (see
  [Second mates](firstmate.md#second-mates)). `app_remind` sets a reminder for the captain and
  `app_reminders` lists them (see [Reminders](reminders.md)).
- **Hooks.** `--settings` adds three HTTP hooks. The `UserPromptSubmit` and `Stop` hooks carry
  [visual replies](visual-replies.md): a standing instruction to show digests, lists, choices and
  tables as views, and a check that sends such a reply back once when it arrives as text. The
  `PreToolUse` hook covers every tool, including the app's own. While the chief of staff runs as
  firstmate it first applies [firstmate's rules](firstmate-rules.md), which deny or ask about
  merges, forced pushes, discarding teardowns, writes into projects and publishing. It then
  decides each tool use with the tool-approval settings (`src/approval.rs`, described in
  the README's [Tool approval](../README.md#tool-approval) section): a mode, **Run without
  asking** by default, and custom allow, ask and deny rules on top of it. An allowed or denied
  tool use gets its answer at once, with a reason that names the mode or the rule. A tool use
  the settings leave to you appears in **Needs your input** as "Allow *tool*?" with all
  of its input in a scrollable block you can select from: the whole command, or for a file edit or
  write the path and the full content. Nothing is shortened. Such an input larger than 64 KB is
  denied with "too large to review here", because approving a partial view would approve what
  you never saw. **Allow** or **Deny** goes back to Claude Code as the hook's decision. Every
  decision is added to the activity log. If nobody
  answers within 590 seconds, or Claude Code hangs up on the hook first, the app denies the tool
  use and the question leaves the panel, so a later click cannot look like an approval. The hook
  sets `onFailure: "block"`, so if the app cannot answer at all, Claude Code blocks the tool use.
  When the session ends, its unanswered questions leave the panel and are denied.
- **Access.** Each server makes a random 256-bit token. Both flags carry it in an
  `Authorization` header, and the server refuses requests without it and requests from web pages
  (any request with an `Origin` header).

The sign-in guarantees are unchanged: the app still strips `ANTHROPIC_API_KEY` and
`ANTHROPIC_AUTH_TOKEN` from every `claude` it starts, and it never reads or stores a credential.

`scripts/fake-claude` calls the server the same way the real `claude` does, so `cargo test` and
`scripts/verify.sh` cover both paths, every approval mode, the rules, firstmate's rules and the
visual-replies hooks without an account. The ignored test `real_claude_shows_a_list_as_a_view`
ran on 2026-11-13 against Claude Code 2.1.311: the `UserPromptSubmit` and `Stop` hooks passed with
`--settings` were called, and the `Stop` hook's `additionalContext` continued the turn into a
view (see [Visual replies](visual-replies.md#verified)). One
ignored test, `cargo test real_claude -- --ignored`, sends one short message to the real
`claude` on the Haiku model in **Ask every time**. On Claude Code 2.1.311, before the approval
settings existed, it passed: `claude` accepted both flags, the hook asked about Claude Code's
`ToolSearch` call that loads the deferred `app_note` tool, and the tool added its note. No flag
was rejected. It has not been run against the real `claude` since the settings were added.

## Extension points of the headless `claude`

All of these work with `claude -p` signed in with a subscription. `--bare` does not: in bare mode
Claude Code never reads the OAuth sign-in or the keychain and needs an API key
([headless](https://code.claude.com/docs/en/headless#start-faster-with-bare-mode)). The same page
says `--bare` will become the default for `-p` in a future release, which would affect this app.

| Extension point | How the app routes it | Notes |
| --- | --- | --- |
| Custom tools | Serve an MCP server and pass `--mcp-config` (stdio or `"type": "http"`) | Tools are named `mcp__<server>__<tool>` and need permission, from an allow rule, a hook or a permission prompt ([MCP](https://code.claude.com/docs/en/mcp), [SDK MCP](https://code.claude.com/docs/en/agent-sdk/mcp#allow-mcp-tools)). `--strict-mcp-config` would ignore the user's other servers ([CLI](https://code.claude.com/docs/en/cli-reference)). |
| Hooks | Pass `--settings` with `command` or `http` hooks that call the app | HTTP hooks take `url`, `headers`, `allowedEnvVars`, `timeout` (600 s by default) and `onFailure` (2.1.295+). PreToolUse answers with `hookSpecificOutput.permissionDecision` of `allow`, `deny`, `ask` or `defer`; deny and ask permission rules still win over `allow` ([hooks](https://code.claude.com/docs/en/hooks)). |
| Skills and slash commands | Ship them in a plugin and pass `--plugin-dir`, or use `.claude/skills` in the working directory | In `-p`, a prompt that starts with `/skill-name` runs that skill; terminal-only commands such as `/login` do not work ([skills](https://code.claude.com/docs/en/skills), [headless](https://code.claude.com/docs/en/headless)). Load errors appear in `plugin_errors` of the `system/init` record. |
| Subagents | Pass `--agents` with JSON definitions | Subagent records carry `parent_tool_use_id` in stream-json, which could feed the Agents column ([CLI](https://code.claude.com/docs/en/cli-reference), [headless](https://code.claude.com/docs/en/headless#follow-subagent-messages)). |
| Permission prompts | `--permission-prompt-tool mcp__<server>__<tool>`, or a PreToolUse or PermissionRequest hook | Since 2.1.302, `-p` runs start in Manual mode and deny any tool use that needs approval unless something approves it ([headless](https://code.claude.com/docs/en/headless#auto-approve-tools)). The app uses a PreToolUse hook because its format is documented; the result format of a permission prompt tool and the stream-json `can_use_tool` control messages were not found in the pages read. |

To add another tool, add it to `tools/list` and `tools/call` in `src/bridge.rs` and a `Call` for
the app to handle. To add a skill, put it in a plugin directory and pass `--plugin-dir` in
`Session::start` in `src/claude.rs`. Not tested: whether hooks passed with `--settings` run in
addition to the user's own hooks. The hooks page says hooks from different settings sources
merge, and the settings page says `--settings` overrides only the keys it sets, but neither says
so directly for `--settings`.

## What the Agent SDK adds

The Agent SDK (TypeScript and Python) runs the same Claude Code program and talks to it over the
same stream-json protocol ([overview](https://code.claude.com/docs/en/agent-sdk/overview)). It adds
conveniences inside the app's own process: MCP servers defined as functions, hooks as callbacks,
a `canUseTool` callback for permission prompts, typed messages and session resume
([SDK MCP](https://code.claude.com/docs/en/agent-sdk/mcp),
[SDK hooks](https://code.claude.com/docs/en/agent-sdk/hooks),
[user input](https://code.claude.com/docs/en/agent-sdk/user-input)). It does not add an extension
point that the flags lack, and it does not change how the sign-in works. Its documentation tells
developers to authenticate with an API key or a cloud provider and says that, unless previously
approved, third parties may not offer claude.ai login for products built on it. Velvet Hammer is
written in Rust, and the SDK has no Rust package, so the flags are the simpler route.

## Anthropic's terms on the subscription

**What the pages say.**

- The [legal and compliance page](https://code.claude.com/docs/en/legal-and-compliance) says
  OAuth sign-in is for subscribers and for ordinary use of Claude Code and other Anthropic
  applications. Developers of products, including Agent SDK products, should use API keys.
  Third-party developers may not offer Claude.ai login in their own apps, route requests through
  Free, Pro, Max or Ultra credentials on behalf of their users, or collect, store or intermediate
  Claude.ai credentials or session tokens. The same page says this does not prevent an end user
  from signing in to the unmodified Claude Code program with their own subscription, including
  where a platform hosts Claude Code. Plan limits assume ordinary, individual use of Claude Code
  and the Agent SDK.
- The [Agent SDK overview](https://code.claude.com/docs/en/agent-sdk/overview) carries the
  no-claude.ai-login note for third-party products, "unless previously approved".
- The Help Center article
  [Use the Claude Agent SDK with your Claude plan](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan)
  says, in a note dated 2026-06-15, that announced changes to Agent SDK usage are paused and that
  Agent SDK, `claude -p` and third-party app use signed in with a plan still draw on the plan's
  limits. A note dated 2026-10-07 adds monthly API credits on some plans for runs that use an API
  key.
- The [Consumer Terms](https://www.anthropic.com/legal/consumer-terms) forbid automated access
  except through an API key or where Anthropic explicitly permits it, and forbid sharing
  credentials.

**What changed.** In February 2026 the legal page was reported to say that subscription tokens
may be used only in Claude Code and Claude.ai, not in other products or the Agent SDK, and
Anthropic called it a clarification
([The Register, secondary](https://www.theregister.com/software/2026/02/20/anthropic-clarifies-ban-on-third-party-tool-access-to-claude/5014546)).
The current page instead has the end-user carve-out above. No dated revision history was found.
Reports of an April 2026 change for third-party harnesses come only from secondary sources and
were not confirmed.

**How it applies.**

- *An app that starts the user's own unmodified `claude` on the user's own machine, signed in by
  the user through Anthropic's flow* (Velvet Hammer today). This matches the conditions of the
  carve-out: the program is unmodified, the sign-in is Anthropic's, the app handles no
  credential, and the use is the user's own. MCP servers, hooks, skills and subagents are
  documented settings of that program, not changes to it. What is ambiguous: no page names a
  desktop app that drives `claude -p` as permitted or forbidden, and "ordinary, individual use"
  and the Consumer Terms' clause on automated access are not defined for this case.
- *An app that offers logins to other people or routes requests on their behalf.* The legal page
  forbids this. Such a product should use API keys or a cloud provider under the Commercial Terms.
- *Distributing Velvet Hammer for other people to run on their own machines with their own
  sign-in.* The carve-out reads in favour, but no page says such a launcher is outside the
  restriction on third-party developers. Anthropic's legal page names its sales team as the
  contact for questions on permitted authentication.

**Not verified.** The result format of `--permission-prompt-tool`, the stream-json control
protocol, the end of the very long hooks and MCP pages, a dated history of the legal page, and the
secondary reports above.

**Recommendation.** Keep routing extensions through flags on the user's installed `claude`, with
the app never handling a credential, and keep Velvet Hammer a personal tool for now. Do not
adopt the Agent SDK for this: it adds no extension point the flags lack, and its documentation
points products at API keys. Before distributing the app to other people, ask Anthropic to
confirm that the end-user carve-out covers it, and watch the `--bare` default for `-p`.
