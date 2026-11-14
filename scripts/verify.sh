#!/usr/bin/env bash
# Builds the app, runs its tests, then renders every main state headlessly and checks the element
# trees. Usage: scripts/verify.sh [OUT_DIR]   (default: target/verify)
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
out=${1:-$root/target/verify}
mkdir -p "$out"
app=$root/target/debug/velvet-hammer
walk=$root/scripts/walkthrough.json

# Runs the app headless, printing its output only when it fails.
headless() {
  local log
  if ! log=$("$app" "$@" 2>&1); then
    echo "$log"
    echo "FAILED: velvet-hammer $*"
    exit 1
  fi
}

cargo build --manifest-path "$root/Cargo.toml"
cargo test --manifest-path "$root/Cargo.toml"

# PNGs of each scene (`$app --list-scenes`), and the element tree of each. The sign-in scenes
# run the app on scripts/fake-claude, so they need no Claude account.
# The firstmate scenes run git on a scratch upstream under target/, so they need no network.
for scene in chat replied empty peek signed-out signing-in signed-in expired cli-missing claude-error noted approval approval-long approved \
  approval-settings approval-by-rule approval-activity approval-corrupt \
  firstmate-missing firstmate-installing firstmate-up-to-date firstmate-update-available \
  firstmate-modified firstmate-blocked firstmate-session firstmate-panels firstmate-answering firstmate-crew \
  projects-fresh projects-empty projects-adding projects-listed \
  projects-repos projects-repos-filtered projects-repos-chosen projects-repos-missing projects-repos-signed-out \
  formatted formatted-all formatted-streaming tool-image tool-image-broken tool-video tool-generic tool-picture \
  attach-draft attach-sent attach-too-large attach-adding attach-drop \
  tool-bash tool-bash-open tool-terminal tool-diff tool-files \
  view-static view-streaming view-state view-action view-unknown view-too-large \
  choice-ask choice-answered choice-multiple choice-enforced \
  visuals-settings visuals-enforced visuals-capped rules-settings rules-merge-ask rules-project-ask \
  find find-stepped find-none history link-copied link-opened link-context link-missing \
  settings settings-clear menu-windows menu-linux \
  reminders-list reminder-fired reminder-snooze reminder-settings reminder-link-opened \
  mates-none mates-listed mate-idle mate-working mate-stopped mate-mismatch mate-peek \
  add-mate-scope add-mate-projects add-mate-name add-mate-runtime add-mate-confirm add-mate-created add-mate-refused \
  mate-retire-confirm mate-retire-refused mate-message mate-replied mate-escalation mate-escalation-answered \
  mate-routed mate-from-chat \
  workers-fresh workers-settings workers-claude-refused workers-crew \
  send-while-working send-while-working-answered messages-settings queue-waiting queue-delivering queue-not-ready; do
  headless --scene "$scene" --snap "$out/$scene.png"
  headless --scene "$scene" --tree "$out/$scene.json"
done

# Scripted input on the stub's chat scene: type a message and send it, answer one item, finish
# the task, peek at an agent.
headless --scene chat --script "$walk" --at mark:sent --tree "$out/walk-sent.json"
headless --scene chat --script "$walk" --at mark:end --tree "$out/walk-end.json"
headless --scene chat --script "$walk" --at mark:end --snap "$out/walk-end.png"

# Find through the keyboard on a long chat: Cmd+F (Ctrl+F off macOS), typing, Enter and Cmd+G,
# then Escape back to the message field.
find=$root/scripts/find.json
for mark in found stepped closed; do
  headless --scene long-chat --script "$find" --at "mark:$mark" --tree "$out/find-keys-$mark.json"
done
headless --scene long-chat --script "$find" --at mark:stepped --snap "$out/find-keys.png"

# A message link the system opened with the app (macOS and Windows deliver these to
# App::on_open_url), then the highlight fading.
links=$root/scripts/open-link.json
for mark in opened faded; do
  headless --scene long-chat --script "$links" --at "mark:$mark" --tree "$out/open-link-$mark.json"
done
headless --scene long-chat --script "$links" --at mark:opened --snap "$out/open-link.png"

# The menus: Settings and Find picked from the menu bar (macOS's native items, which headless
# runs keep on every platform), then Settings again with Cmd+, (Ctrl+, off macOS).
menus=$root/scripts/menus.json
for mark in settings find shortcut; do
  headless --scene chat --script "$menus" --at "mark:$mark" --tree "$out/menus-$mark.json"
done

failures=0
check() { # check FILE yes|no PATTERN
  local file=$out/$1 want=$2 pattern=$3
  if grep -qF -- "$pattern" "$file"; then found=yes; else found=no; fi
  if [[ $found != "$want" ]]; then
    echo "FAIL $1: expected $pattern to be $([[ $want == yes ]] && echo present || echo absent)"
    failures=$((failures + 1))
  fi
}

# Chat with both panels populated.
check chat.json yes '"key": "needs-input"'
check chat.json yes '"key": "input-1"'
check chat.json yes '"key": "answer-1-0"'
check chat.json yes '"key": "input-2"'
check chat.json yes '"key": "for-you"'
check chat.json yes '"key": "task-3"'
check chat.json yes '"key": "done-3"'
check chat.json yes '"key": "agent-1"'
check chat.json yes '"key": "chat-input"'
check chat.json no  '"key": "peek"'
# A sent message and the stub's reply.
check replied.json yes '"text": "Ship it when CI is green."'
check replied.json yes 'Got it:'
# Both panels empty.
check empty.json yes '"key": "needs-input-empty"'
check empty.json yes '"key": "for-you-empty"'
check empty.json no  '"key": "input-1"'
check empty.json no  '"key": "task-3"'
# Peek replaces the chat with the subagent's transcript.
check peek.json yes '"key": "peek"'
check peek.json yes '"key": "peek-close"'
check peek.json yes '"text": "Peeking at layout"'
check peek.json no  '"key": "chat"'
# Signed out: the sign-in replaces the chat.
check signed-out.json yes '"key": "sign-in"'
check signed-out.json yes '"key": "sign-in-button"'
check signed-out.json yes '"key": "recheck"'
check signed-out.json yes '"text": "Sign in with Claude"'
check signed-out.json no  '"key": "chat-input"'
# Signing in: `claude auth login` is running and printed its sign-in page.
check signing-in.json yes '"text": "Finish signing in in your browser"'
check signing-in.json yes '"key": "sign-in-url"'
check signing-in.json yes 'https://claude.ai/oauth/authorize'
check signing-in.json yes '"key": "open-sign-in"'
check signing-in.json yes '"key": "cancel-sign-in"'
check signing-in.json no  '"key": "chat-input"'
# Signed in: the chat, its health line, and the fake's streamed reply.
check signed-in.json yes '"key": "chat-input"'
check signed-in.json yes '"text": "Claude Code · signed in with your Claude Max plan"'
check signed-in.json yes '"text": "You said: What needs me today?"'
check signed-in.json no  '"key": "sign-in"'
# Errors: a rejected sign-in, no `claude`, and a failed turn.
check expired.json yes '"text": "Your Claude sign-in has expired"'
check expired.json yes '"key": "sign-in-button"'
check expired.json no  '"key": "chat-input"'
check cli-missing.json yes '"text": "Claude Code isn'"'"'t installed"'
check cli-missing.json yes '"key": "recheck"'
check cli-missing.json no  '"key": "sign-in-button"'
check claude-error.json yes '"text": "Claude Code reported a problem"'
check claude-error.json yes 'could not finish this turn'
check claude-error.json yes '"key": "chat-input"'
# The bridge: a note from the app_note tool, a tool use waiting on the captain, and the reply after
# the captain allowed it.
check noted.json yes '"text": "Renew the velvet-hammer.dev domain"'
check noted.json yes '"text": "Noted by your chief of staff."'
check noted.json yes '"key": "task-1"'
# The reply is formatted, so Rune's direction marks surround the domain name.
check noted.json yes 'I noted it for you: Renew the'
check noted.json yes 'velvet-hammer.dev'
check approval.json yes '"text": "Allow Bash?"'
check approval.json yes '"text": "command: git push"'
check approval.json yes '"key": "answer-1-0"'
check approval.json yes '"key": "answer-1-1"'
check approval.json no  '"key": "needs-input-empty"'
check approval-long.json yes '"key": "input-1-detail"'
check approval-long.json yes '; rm -rf ~'
check approval-long.json yes '"key": "answer-1-0"'
check approved.json yes 'Bash was allowed to run git push'
check approved.json yes '"key": "needs-input-empty"'
# Tool approval: the settings page with the recommended mode, three rules and an example; an ask
# rule bringing up the full-input question while running without asking; the activity log of
# the fake's real tool uses; and a settings file the app could not read.
check settings.json yes '"key": "approvals"'
check settings.json yes '"text": "Run without asking"'
check approval-settings.json yes '"key": "approvals-page"'
check approval-settings.json yes '"text": "● Run without asking (Recommended)"'
check approval-settings.json yes 'will edit files, run commands and use tools without stopping for you, and every tool use is still logged'
check approval-settings.json yes '"key": "mode-ask-every-time"'
check approval-settings.json yes '"key": "mode-auto-allow-read-only"'
check approval-settings.json yes '"text": "2. Deny Bash matching git push --force"'
check approval-settings.json yes '"key": "rule-2-up"'
check approval-settings.json no  '"key": "rule-0-up"'
check approval-settings.json yes '"key": "rule-save"'
check approval-settings.json yes '"key": "approvals-reset"'
check approval-settings.json yes '"text": "Waits for you in Needs your input, by rule 1 (Ask Bash matching git push)."'
check approval-settings.json yes 'never pushes, publishes or merges anything itself'
check approval-settings.json no  '"key": "approvals-problem"'
check approval-by-rule.json yes '"text": "Allow Bash?"'
check approval-by-rule.json yes '"text": "command: git push origin main"'
check approval-activity.json yes '"key": "activity-page"'
check approval-activity.json yes 'UTC · allowed by the mode Run without asking'
check approval-activity.json yes 'denied by rule 2 (Deny Bash matching git push --force)'
check approval-activity.json yes 'asked by rule 1 (Ask Bash matching git push); you allowed it'
check approval-activity.json yes '"text": "Denied · Bash"'
check approval-activity.json no  '"key": "activity-empty"'
check approval-corrupt.json yes '"key": "approvals-problem"'
check approval-corrupt.json yes 'so every tool use asks you'
check approval-corrupt.json yes '"text": "● Ask every time"'
# The managed firstmate clone: a card in Settings, and its page in each state.
check settings.json yes '"key": "firstmate"'
check settings.json yes '"text": "up to date"'
check chat.json no  '"key": "firstmate-page"'
check firstmate-missing.json yes '"text": "Firstmate is not installed"'
check firstmate-missing.json yes '"key": "firstmate-install"'
check firstmate-missing.json no  '"key": "firstmate-check"'
check firstmate-missing.json no  '"key": "chat"'
check firstmate-installing.json yes '"text": "Installing firstmate"'
check firstmate-installing.json yes '"key": "firstmate-progress"'
check firstmate-installing.json no  '"key": "firstmate-install"'
check firstmate-up-to-date.json yes '"text": "Firstmate is up to date"'
check firstmate-up-to-date.json yes '"text": "The clone matches origin/main."'
check firstmate-up-to-date.json yes '"key": "firstmate-check"'
check firstmate-up-to-date.json no  '"key": "firstmate-update"'
check firstmate-update-available.json yes '"text": "A firstmate update is available"'
check firstmate-update-available.json yes '"text": "origin/main has 1 new commit."'
check firstmate-update-available.json yes '"key": "firstmate-update"'
check firstmate-update-available.json yes '"text": "Update"'
check firstmate-modified.json yes '"text": "Firstmate has your changes"'
check firstmate-modified.json yes '"text": "Edited and not committed: AGENTS.md"'
check firstmate-modified.json no  '"key": "firstmate-update"'
check firstmate-blocked.json yes '"text": "The firstmate update is blocked"'
check firstmate-blocked.json yes '"key": "firstmate-conflicts"'
check firstmate-blocked.json yes '"text": "AGENTS.md"'
check firstmate-blocked.json no  '"key": "firstmate-update"'
# The chat running as firstmate: in the managed clone, with its own operational home.
check firstmate-session.json yes '"key": "chat-input"'
check firstmate-session.json yes '"key": "workplace"'
check firstmate-session.json yes 'Running firstmate from'
check firstmate-session.json yes '/data/firstmate'
check firstmate-session.json yes 'FM_HOME='
check firstmate-session.json yes '/data/home'
check firstmate-session.json no  '"key": "firstmate-page"'
# The panel fed from firstmate's records: two captain holds and a merge approval.
check firstmate-panels.json yes '"key": "hold-lab-window-size"'
check firstmate-panels.json yes '1280x800 (shows more chat).'
check firstmate-panels.json yes '"key": "answer-lab-window-size"'
check firstmate-panels.json yes '"key": "hold-lab-gh-login"'
check firstmate-panels.json yes '"key": "merge-lab-panels"'
check firstmate-panels.json yes '"key": "approve-lab-panels"'
check firstmate-panels.json no  '"key": "merge-lab-window-size"'
check firstmate-panels.json no  '"key": "needs-input-empty"'
check firstmate-panels.json no  '"key": "answering"'
check firstmate-answering.json yes '"key": "answering"'
check firstmate-answering.json yes '"key": "answer-cancel"'
check firstmate-answering.json yes '"text": "Answering below"'
# The agents list from firstmate's tasks in flight, and a peek at one crewmate's terminal.
check firstmate-panels.json yes '"key": "crew-lab-verify"'
check firstmate-panels.json yes '"text": "working · rendering chat, empty and peek scenes"'
check firstmate-panels.json yes '"key": "crew-lab-layout"'
check firstmate-panels.json no  '"key": "crew-peek"'
check firstmate-crew.json yes '"key": "crew-peek"'
check firstmate-crew.json yes '"text": "Peeking at Render every state headlessly"'
check firstmate-crew.json yes '"key": "crew-pane"'
check firstmate-crew.json yes 'rendering peek.png'
check firstmate-crew.json yes '"key": "peek-close"'
check firstmate-crew.json no  '"key": "chat-input"'
# Projects from firstmate's records: a fresh install with no home, an installed home with none,
# adding the first one with the chief of staff, and a registry of three.
check projects-fresh.json yes '"key": "projects-none"'
check projects-fresh.json yes '"key": "add-project-later"'
check projects-fresh.json yes '"key": "firstmate-page"'
check projects-fresh.json no  '"key": "add-project"'
check projects-empty.json yes '"key": "projects-empty"'
check projects-empty.json yes '"text": "Add your first project"'
check projects-empty.json yes '"key": "add-first-project"'
check projects-empty.json yes '"key": "add-project"'
check projects-empty.json no  '"key": "adding"'
check projects-adding.json yes '"key": "adding"'
check projects-adding.json yes 'I want to add a project. Follow your project-management procedure'
check projects-adding.json yes 'You said: I want to add a project.'
check projects-adding.json yes '"key": "add-pick-folder"'
check projects-adding.json yes '"key": "add-yes"'
check projects-adding.json yes '"key": "add-finish"'
check projects-adding.json no  '"key": "projects-empty"'
check projects-listed.json yes '"key": "project-velvet-hammer"'
check projects-listed.json yes '"text": "no-mistakes-prod-only"'
check projects-listed.json yes '"key": "project-notes"'
check projects-listed.json yes '"key": "project-site"'
check projects-listed.json yes '"key": "add-project"'
check projects-listed.json no  '"key": "projects-empty"'
check projects-listed.json no  '"key": "projects-none"'
check chat.json no  '"key": "projects"'
# The GitHub repository combobox while adding a project.
check projects-adding.json yes '"key": "repo-search"'
check projects-adding.json no  '"key": "repo-list"'
check projects-repos.json yes '"role": "combobox"'
check projects-repos.json yes '"key": "repo-list"'
check projects-repos.json yes '"role": "listbox"'
check projects-repos.json yes '"key": "repo-option-captain:dotfiles"'
check projects-repos.json yes '"activeDescendant": "repo-option-captain:dotfiles"'
check projects-repos.json yes '"text": "Private"'
check projects-repos.json yes '"text": "Archived"'
check projects-repos.json yes '"key": "add-pick-folder"'
check projects-repos-filtered.json yes '"text": "ham"'
check projects-repos-filtered.json yes '"key": "repo-option-acme:velvet-hammer"'
check projects-repos-filtered.json yes '"key": "repo-option-acme:handbook"'
check projects-repos-filtered.json no  '"key": "repo-option-captain:dotfiles"'
check projects-repos-chosen.json yes 'https://github.com/acme/payments-api'
check projects-repos-chosen.json no  '"key": "repo-list"'
check projects-repos-missing.json yes '"key": "repo-problem"'
check projects-repos-missing.json yes 'https://cli.github.com'
check projects-repos-missing.json yes '"key": "repo-retry"'
check projects-repos-signed-out.json yes 'is not signed in to GitHub'
check projects-repos-signed-out.json yes '"key": "repo-retry"'
# The chief of staff's Markdown, formatted: the screenshot's headings bold on their own lines with
# no asterisks, every construct, HTML shown as text, and a reply still arriving with its bold
# unclosed.
check formatted.json yes '"key": "msg-2-text"'
check formatted.json yes '"text": "Recently Landed\nNo recent completions are in the current baseline."'
check formatted.json no  '**'
check formatted-all.json yes '"text": "Heading one"'
check formatted-all.json yes '"text": "Nested bullet"'
check formatted-all.json yes '"text": "A quoted line."'
check formatted-all.json yes 'println!(\"hello\");'
check formatted-all.json yes 'alert(1)</script>'
check formatted-all.json no  '~~'
check formatted-streaming.json yes '"text": "The fleet is empty: nothing"'
check formatted-streaming.json no  '**'
# Tool uses: an image inline, one that cannot be decoded, a video with its actions, a command's
# output, and an image enlarged.
check tool-image.json yes '"key": "tool-2-image"'
check tool-image.json yes '"text": "Read"'
check tool-image.json no  '"key": "tool-2-placeholder"'
check tool-image-broken.json yes '"key": "tool-2-placeholder"'
check tool-image-broken.json yes 'broken.png. It could not be decoded'
check tool-video.json yes '"key": "tool-2-video"'
check tool-video.json yes '"key": "tool-2-play"'
check tool-video.json yes '"text": "Open in your video player"'
check tool-generic.json yes '"text": "Bash"'
check tool-generic.json yes '"key": "tool-2-output"'
check tool-generic.json yes '"key": "tool-2-command"'
check tool-generic.json yes '"text": "one\ntwo\nthree"'
check tool-picture.json yes '"key": "picture"'
check tool-picture.json yes '"key": "picture-close"'
check tool-picture.json no  '"key": "chat-input"'
# Tool calls as cards: the captain's long test run folded to its first lines and unfolded,
# commands that failed with colours, printed nothing or still run, edits as diffs, and reads,
# searches and a fetch.
check tool-bash.json yes '"key": "tool-2-command"'
check tool-bash.json yes '"text": "Run the test suite"'
check tool-bash.json yes '"text": "42 s"'
check tool-bash.json yes '"text": "Show 20 more lines"'
check tool-bash.json no  'test result: ok'
check tool-bash-open.json yes 'test result: ok. 112 passed'
check tool-bash-open.json yes '"text": "Show fewer lines"'
check tool-terminal.json yes '"text": "Exit code 101"'
check tool-terminal.json yes 'error: unused variable'
check tool-terminal.json no  '\u001b'
check tool-terminal.json yes '"key": "tool-4-empty"'
check tool-terminal.json yes '"text": "Running…"'
check tool-diff.json yes '"key": "tool-2-diff"'
check tool-diff.json yes '"text": "+5"'
check tool-diff.json yes '"key": "tool-5-error"'
check tool-files.json yes '"key": "tool-4-numbers"'
check tool-files.json yes '"text": "Found 2 files"'
check tool-files.json yes 'https://doc.rust-lang.org/book/ch08-02-strings.html'
# json-render views: every component, a spec half streamed, state bound to inputs and a repeat,
# a submit and its reply, an unknown component, and a spec over the limits.
check view-static.json yes '"text": "Chief of staff'"'"'s view"'
check view-static.json yes '"key": "tool-1-ui-review"'
check view-static.json yes '"key": "tool-1-ui-progress"'
check view-static.json yes '"text": "in review"'
check view-static.json yes '"key": "tool-1-ui-tag"'
check view-streaming.json yes '"text": "Chief of staff'"'"'s view · arriving…"'
check view-streaming.json yes '"key": "tool-1-ui-tasks"'
check view-streaming.json no  '"key": "tool-1-ui-alert"'
check view-state.json yes '"key": "tool-1-ui-done.1"'
check view-state.json yes '"key": "tool-1-ui-check.3"'
check view-state.json yes '"text": "Update the README"'
check view-state.json yes '"text": "Announce it"'
check view-state.json no  '"text": "Pressed'
check view-action.json yes '"text": "Pressed “Send the list”."'
check view-action.json yes 'Thanks. I got your submit with'
check view-action.json yes 'Tag the release'
check view-unknown.json yes '"text": "Unknown component “BarChart”"'
check view-unknown.json yes '"text": "Monday was the slowest."'
check view-too-large.json yes 'more than the limit of 200, so it is not shown.'
check view-too-large.json no  '"key": "tool-1-ui-main"'
# A question with a fixed set of answers, asked with a Choice: the recommended option marked, the
# exact value sent and shown as the captain's choice, several picks with their own words, and the
# stop hook sending a question asked in prose back to be asked with a Choice.
check choice-ask.json yes '"text": "Merge authority for velvet-hammer is yolo on. Do you want it off?"'
check choice-ask.json yes '"key": "tool-1-ui-merge-option-0"'
check choice-ask.json yes '"key": "tool-1-ui-merge-recommended"'
check choice-ask.json yes '"key": "tool-1-ui-merge-other"'
check choice-ask.json no  '"key": "tool-1-ui-merge-sent"'
check choice-answered.json yes '"text": "Chose “yolo off”."'
check choice-answered.json yes '"key": "tool-1-ui-merge-sent"'
check choice-answered.json yes 'Thanks. I got the values'
check choice-answered.json yes 'yolo-off'
check choice-answered.json no  '"key": "tool-1-ui-merge-other"'
check choice-multiple.json yes '"key": "tool-1-ui-checks-send"'
check choice-multiple.json yes '"text": "A changelog entry"'
check choice-multiple.json no  '"text": "Chose'
check choice-enforced.json yes 'the reply asks a question with a fixed set of answers.'
check choice-enforced.json yes '"key": "tool-3-ui-merge-sent"'
check choice-enforced.json yes '"text": "Chose “yolo off”."'
check choice-enforced.json yes 'with the option'
check choice-enforced.json yes 'yolo-off'
# Visual replies: the settings page, a list sent back once and shown as a view, and a reply that
# ignored the feedback, sent back only once.
# Messages sent while the chief of staff works: sent at once and marked, the setting with Send
# right away recommended and chosen, the queue with one message being edited, the first queued
# message going out when the turn ends, and the queue kept while Claude Code is not ready.
check send-while-working.json yes '"text": "You · sent while your chief of staff was working"'
check send-while-working.json yes '"key": "working"'
check send-while-working.json no  '"key": "queue"'
check send-while-working-answered.json yes 'I also took in'
check send-while-working-answered.json no  '"key": "working"'
check messages-settings.json yes '"key": "messages-page"'
check messages-settings.json yes '"text": "● Send right away (Recommended)"'
check messages-settings.json yes '"text": "○ Queue new messages until the chief of staff is done"'
check queue-waiting.json yes '"key": "queued-0-edit"'
check queue-waiting.json yes '"key": "queued-1-save"'
check queue-waiting.json yes '"key": "queued-2-delete"'
check queue-waiting.json yes 'Queued until your chief of staff is done'
check queue-delivering.json yes '"text": "Then open the release PR."'
check queue-delivering.json no  '"key": "queued-2"'
check queue-delivering.json yes '"key": "queued-1"'
check queue-not-ready.json yes '"key": "sign-in-queued"'
check settings.json yes '"key": "visuals"'
check settings.json yes '"text": "Enforce"'
check settings.json yes '"key": "messages"'
check settings.json yes '"key": "messages-summary"'
check settings.json yes '"text": "Send right away"'
check visuals-settings.json yes '"key": "visuals-page"'
check visuals-settings.json yes '"text": "● Enforce (Recommended)"'
check visuals-settings.json yes '"key": "visual-mode-nudge"'
check visuals-settings.json yes '"key": "visual-mode-off"'
check visuals-settings.json yes '"key": "visual-shape-4"'
check visuals-settings.json yes '"key": "visual-instruction"'
check visuals-settings.json yes '"text": "Sent back once to be shown as a view: the reply lists 3 items."'
check visuals-enforced.json yes 'Velvet Hammer asked the chief of staff to show this as a view: the reply lists 3 items.'
check visuals-enforced.json yes '"text": "Velvet Hammer"'
check visuals-enforced.json yes '"text": "Chief of staff'"'"'s view"'
check visuals-enforced.json yes '"text": "Start on the first"'
check visuals-enforced.json yes '"text": "Shown above."'
check visuals-capped.json yes 'the reply lists 3 items.'
check visuals-capped.json yes '"text": "Left as text."'
check visuals-capped.json no  'looped'
check visuals-capped.json no  '"text": "Chief of staff'"'"'s view"'
# Settings: every section moved off the main window, the history controls, the confirmation
# before clearing, and the in-window menus.
check settings.json yes '"key": "settings-page"'
check settings.json yes '"key": "approvals"'
check settings.json yes '"key": "visuals"'
check settings.json yes '"key": "rules"'
check settings.json yes '"key": "firstmate"'
check settings.json yes '"key": "history-section"'
check settings.json yes '"text": "● Keep the conversation (Recommended)"'
check settings.json yes '"key": "history-clear"'
check settings.json no  '"key": "history-clear-confirm"'
check settings-clear.json yes '"key": "history-clear-confirm"'
check settings-clear.json yes '"key": "history-clear-cancel"'
check chat.json no  '"key": "approvals"'
check chat.json no  '"key": "history-section"'
check menu-windows.json yes '"key": "menu-bar"'
check menu-windows.json yes '"key": "menu-item-settings"'
check menu-windows.json yes '"text": "Ctrl+,"'
check menu-windows.json yes '"text": "Settings"'
check menu-linux.json yes '"text": "Preferences"'
check menu-linux.json yes '"key": "menu-item-find"'
check menus-settings.json yes '"key": "settings-page"'
check menus-find.json yes '"key": "find-bar"'
check menus-find.json yes '"text": "1 of 1"'
check menus-shortcut.json yes '"key": "settings-page"'
# Firstmate's rules: the page with what two rules did, a merge question that names the pull
# request, and a write into a project waiting for the captain.
check settings.json yes '"key": "rules"'
check settings.json yes '"text": "8 of 8 on"'
check rules-settings.json yes '"key": "rules-page"'
check rules-settings.json yes '"key": "guard-merge-through-script"'
check rules-settings.json yes '"key": "guard-publish-asks-captain-toggle"'
check rules-settings.json yes '"key": "guard-no-force-push-recent-0"'
check rules-settings.json yes 'denied by firstmate rule “No force pushes” · git push --force origin main'
check rules-settings.json yes 'asked by firstmate rule “Every merge waits for your word”; you allowed it'
check rules-settings.json yes '"key": "guard-projects-read-only-quiet"'
check rules-merge-ask.json yes '"text": "Merge https://github.com/captain/site/pull/7?"'
check rules-merge-ask.json yes 'Firstmate'"'"'s hard rule 2: never merge a pull request without the captain'"'"'s explicit word.'
check rules-merge-ask.json yes '"key": "answer-1-0"'
check rules-project-ask.json yes 'Let the chief of staff edit /tmp/velvet-hammer/rules-project-ask/data/home/projects/site/README.md?'
check rules-project-ask.json yes '"key": "answer-1-1"'
# Attachments: chips for an image, a video and a file in the draft; the message that carried
# them, with the image sent as image content and every file by path; a file over the cap; a drag.
check attach-draft.json yes '"key": "attachments"'
check attach-draft.json yes '"label": "Image: chart.png"'
check attach-draft.json yes '"label": "Video: clip.mp4"'
check attach-draft.json yes '"key": "attachment-2-remove"'
check attach-draft.json yes '"text": "136 bytes"'
check attach-draft.json yes '"label": "Remove notes.md"'
check attach-sent.json yes '"key": "msg-0-attachments-1"'
check attach-sent.json yes '"text": "You said: Here is the chart and the notes. [image blocks: 1, files listed: 2]"'
check attach-sent.json no  '"key": "attachments"'
check attach-too-large.json yes '"key": "attach-problem"'
check attach-too-large.json yes 'is 200 MB. Attachments can be up to 50 MB each.'
check attach-too-large.json no  '"key": "attachment-1"'
check attach-adding.json yes '"key": "adding-0-status"'
check attach-adding.json yes '"text": "Adding…"'
check attach-adding.json yes '"key": "adding-1-problem"'
check attach-adding.json yes '"label": "Remove locked.pdf"'
check attach-adding.json yes '"label": "Send, once the attachments are added"'
check attach-drop.json yes '"key": "attach-drop-hint"'
# Workers on other providers: nothing set up, pi installed from a stand-in release with
# firstmate pointed at it, a Claude model refused for pi, and what each worker runs on.
check settings.json yes '"key": "workers"'
check settings.json yes '"text": "1 nickname · pi not installed"'
check workers-fresh.json yes '"key": "workers-page"'
check workers-fresh.json yes '"text": "pi 1.5.2 is not installed."'
check workers-fresh.json yes '"key": "workers-install"'
check workers-fresh.json yes '"text": "No opencode Go key is in the keychain yet."'
check workers-fresh.json yes '"key": "workers-connect"'
check workers-fresh.json yes '"key": "nickname-0"'
check workers-fresh.json yes 'pi · opencode-go/deepseek-v4.1-flash'
check workers-settings.json yes 'It lists 6 models for the keys it has.'
check workers-settings.json yes '"text": "Your opencode Go key is in the keychain."'
check workers-settings.json yes 'may spend only opencode Go.'
check workers-settings.json no  '"key": "workers-connect"'
check workers-settings.json yes '"key": "nickname-1"'
check workers-settings.json yes 'claude · opus · effort high'
check workers-claude-refused.json yes '"key": "nickname-problem"'
check workers-claude-refused.json yes 'never through pi. Choose Claude Code for a Claude model.'
check workers-claude-refused.json no  '"key": "nickname-2"'
check workers-crew.json yes '"key": "crew-lab-verify-runs"'
check workers-crew.json yes '"text": "pi · opencode-go/deepseek-v4.1-flash"'
check workers-crew.json yes '"text": "claude · claude-sonnet-5-5"'
check rules-settings.json yes '"key": "guard-claude-on-subscription"'
# Find in the conversation: the count and the highlighted matches, the current one after
# stepping, no matches, and the history kept from an earlier session above its divider.
check find.json yes '"key": "find-bar"'
check find.json yes '"text": "1 of 4"'
check find.json yes '"key": "find-0"'
check find.json yes '"key": "find-3"'
check find-stepped.json yes '"text": "3 of 4"'
check find-none.json yes '"text": "No matches"'
check find-none.json no  '"key": "find-0"'
check history.json yes '"key": "earlier-divider"'
check history.json yes '"text": "Remember the zeppelin plan for the launch."'
check history.json yes '"text": "1 of 2"'
check chat.json no  '"key": "find-bar"'
check chat.json no  '"key": "earlier-divider"'
check find-keys-found.json yes '"key": "find-bar"'
check find-keys-found.json yes '"text": "release"'
check find-keys-found.json yes '"text": "1 of 4"'
check find-keys-stepped.json yes '"text": "3 of 4"'
check find-keys-closed.json no  '"key": "find-bar"'
check find-keys-closed.json yes '"text": "hi"'
# Message links: the copy button, a link opened and highlighted, a link in the captain's message
# drawn as a chip and quoted to the chief of staff, and a link to a missing message.
check link-copied.json yes '"key": "msg-1-link"'
check link-copied.json yes '"text": "Link copied"'
check chat.json yes '"text": "Copy link"'
check link-opened.json yes '"text": "Reply 5: nothing new."'
check link-context.json yes 'do what this answer says: ↪ Chief of staff: You said: Plan: merge the docs PR'
check link-context.json yes '[Linked messages, quoted by Velvet Hammer]'
check link-context.json yes ', written by you, the chief of staff:'
check link-missing.json yes 'what did ⚠ Message not found say?'
check link-missing.json yes ': message not found. No message in this chat has this link.'
check open-link-opened.json yes '"text": "Reply 5: nothing new."'
check open-link-faded.json yes '"text": "Reply 5: nothing new."'
# Reminders on a fixed clock: the page with each form set, two come due (one overdue) in the
# banner and For you to do, the snooze menu, and the settings with what each keyword means now.
check chat.json yes '"key": "reminders"'
check chat.json yes '"text": "none set"'
check chat.json no  '"key": "reminder-banner"'
check reminders-list.json yes '"key": "reminders-page"'
check reminders-list.json yes '"key": "reminders-coming"'
check reminders-list.json yes '"key": "reminders-later"'
check reminders-list.json yes '"text": "Today, 2:50 PM"'
check reminders-list.json yes '"text": "Today, 5:30 PM"'
check reminders-list.json yes '"text": "Tomorrow, 9:00 AM"'
check reminders-list.json yes '"text": "Monday, Nov 16, 9:00 AM"'
check reminders-list.json yes '"text": "5 coming up · 2 later"'
check reminders-list.json no  '"key": "reminder-banner"'
check reminder-fired.json yes '"key": "reminder-banner"'
check reminder-fired.json yes '"text": "Stretch"'
check reminder-fired.json yes '"text": "Set Today, 2:30 PM · due Today, 2:32 PM"'
check reminder-fired.json yes '"key": "reminder-banner-more"'
check reminder-fired.json yes '"key": "reminder-1"'
check reminder-fired.json yes 'Reminder · overdue · Set Today, 9:00 AM · due Today, 10:00 AM'
check reminder-fired.json yes '"key": "panel-1-done"'
check reminder-fired.json yes '"text": "2 due · 1 coming up · 1 later"'
check reminder-fired.json no  '"key": "banner-2-snooze-menu"'
check reminder-snooze.json yes '"key": "banner-2-snooze-menu"'
check reminder-snooze.json yes '"key": "banner-2-snooze-10m"'
check reminder-snooze.json yes '"key": "banner-2-snooze-tomorrow"'
check reminder-snooze.json yes '"key": "snooze-amount"'
check reminder-settings.json yes '"key": "reminder-settings-page"'
check reminder-settings.json yes '"key": "reminder-setting-evening-up"'
check reminder-settings.json yes '"text": "6:00 PM"'
check reminder-settings.json yes '"text": "Monday"'
check reminder-settings.json yes '→ Today, 5:30 PM'
check reminder-settings.json yes '→ Monday, Nov 16, 9:00 AM'
# A reminder links back to the message it was set in, and its settings are a section of Settings.
check reminder-fired.json yes '"key": "banner-2-message"'
check reminder-fired.json yes '"key": "panel-1-message"'
check reminder-fired.json yes '"text": "↪ Set in this message"'
check reminder-link-opened.json yes '"key": "chat"'
check reminder-link-opened.json no  '"key": "reminders-page"'
check reminder-settings.json yes '"key": "reminder-settings-all-settings"'
check settings.json yes '"key": "reminder-settings"'
check settings.json yes '"text": "later today by 6:00 PM · mornings 9:00 AM"'
# Second mates: none yet, five listed in each state, each one's page, a peek at its terminal,
# the guided add at each step with its outcome and a refusal, a refused retirement, a message
# and its reply, an escalation answered, work the chief of staff routed, and the add opened from
# the chat.
check mates-none.json yes '"key": "mates-none"'
check mates-none.json yes '"key": "add-mate"'
check mates-listed.json yes '"key": "mate-docs"'
check mates-listed.json yes '"text": "running · idle"'
check mates-listed.json yes '"text": "running · working"'
check mates-listed.json yes '"text": "stopped · idle"'
check mates-listed.json yes '"text": "running · waiting on you"'
check mates-listed.json yes '"key": "mate-web-mismatch"'
check mates-listed.json no  '"key": "mate-docs-mismatch"'
check mates-listed.json yes '"key": "hold-support-refund-window"'
check mates-listed.json yes '"key": "answer-support-refund-window"'
check mate-idle.json yes '"key": "mate-page"'
check mate-idle.json yes 'Scope: documentation site work: pages, examples and the docs build'
check mate-idle.json yes '"key": "mate-interrupt"'
check mate-idle.json yes '"key": "mate-relaunch"'
check mate-idle.json yes '"key": "mate-retire"'
check mate-idle.json yes '"key": "mate-children-none"'
check mate-idle.json yes 'The docs build passes on main; the last run took 41 seconds.'
check mate-idle.json yes '"key": "mate-edit-note"'
check mate-working.json yes '"key": "mate-child-invoice-retries"'
check mate-working.json yes '"text": "working · running the retry tests"'
check mate-working.json yes '"key": "mate-child-billing-dashboard"'
check mate-stopped.json yes '"text": "Session: stopped · idle"'
check mate-stopped.json yes '"key": "mate-start"'
check mate-stopped.json no  '"key": "mate-interrupt"'
check mate-mismatch.json yes '"key": "mate-mismatch"'
check mate-mismatch.json yes 'invalid home: not a seeded secondmate home'
check mate-peek.json yes '"text": "Peeking at second mate docs"'
check mate-peek.json yes 'no routed work; waiting quietly'
check mate-peek.json yes '"text": "Back to its page"'
check add-mate-scope.json yes '"key": "add-mate-scope"'
check add-mate-scope.json yes '"text": "Step 1 of 4"'
check add-mate-projects.json yes '"key": "add-mate-project-site"'
check add-mate-projects.json yes 'local-only projects stay with the main firstmate'
check add-mate-projects.json yes '"key": "add-mate-no-projects"'
check add-mate-name.json yes '"key": "add-mate-name"'
check add-mate-name.json yes '/data/secondmates/site.'
check add-mate-runtime.json yes '"key": "add-mate-runtime-0"'
check add-mate-runtime.json yes "codex (firstmate's default for second mates)"
check add-mate-runtime.json yes '"key": "add-mate-runtime-1"'
check add-mate-confirm.json yes '"key": "add-mate-yes"'
check add-mate-confirm.json yes '"text": "Yes, create payments"'
check add-mate-confirm.json yes "1. Run firstmate's bin/fm-home-seed.sh payments"
check add-mate-confirm.json yes 'bin/fm-spawn.sh payments --secondmate --harness claude'
check add-mate-confirm.json yes 'Nothing remote is created, pushed or published'
check add-mate-created.json yes '"key": "mate-payments"'
check add-mate-created.json yes 'Firstmate created payments with its home at'
check add-mate-created.json no  '"key": "mate-mismatch"'
check add-mate-refused.json yes '"key": "add-mate-problem"'
check add-mate-refused.json yes 'Firstmate refused to create payments'
check add-mate-refused.json yes 'is not initialized for no-mistakes; refusing to mutate preexisting clone'
check mate-retire-confirm.json yes '"key": "mate-confirm"'
check mate-retire-confirm.json yes '"text": "Retire billing?"'
check mate-retire-confirm.json yes 'Velvet Hammer never forces a retirement'
check mate-retire-refused.json yes 'Firstmate refused to retire billing, and nothing was changed.'
check mate-retire-refused.json yes 'REFUSED: secondmate billing still has in-flight work'
check mate-retire-refused.json no  'WARNING: watcher'
check mate-message.json yes '"text": "Is the search page live yet?"'
check mate-message.json yes "Delivered to docs's inbox."
check mate-replied.json yes 'waits for its pull request to merge.'
check mate-replied.json no  '**built**'
check mate-escalation.json yes '"key": "mate-hold-refund-window"'
check mate-escalation.json yes '"key": "mate-answering"'
check mate-escalation-answered.json yes "The captain's answer to your captain hold refund-window: 30 days, counted from delivery."
check mate-escalation-answered.json no  '"key": "mate-answering"'
check mate-routed.json yes '"text": "Routed to second mate docs"'
check mate-from-chat.json yes '"key": "add-mate-page"'
check mate-from-chat.json yes 'the billing service, its invoices and payment retries'
# The scripted run.
check walk-sent.json yes '"text": "Ship it when CI is green."'
check walk-sent.json yes 'Got it:'
check walk-end.json no  '"key": "input-1"'
check walk-end.json yes '"key": "input-2"'
check walk-end.json yes '"key": "for-you-empty"'
check walk-end.json yes '"text": "Peeking at verify"'

if (( failures > 0 )); then
  echo "$failures check(s) failed; output in $out"
  exit 1
fi
echo "All checks passed; PNGs and trees in $out"
