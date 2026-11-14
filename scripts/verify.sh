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
  formatted formatted-all formatted-streaming tool-image tool-image-broken tool-video tool-generic tool-picture \
  view-static view-streaming view-state view-action view-unknown view-too-large \
  visuals-settings visuals-enforced visuals-capped rules-settings rules-merge-ask rules-project-ask \
  reminders-list reminder-fired reminder-snooze reminder-settings; do
  headless --scene "$scene" --snap "$out/$scene.png"
  headless --scene "$scene" --tree "$out/$scene.json"
done

# Scripted input on the stub's chat scene: type a message and send it, answer one item, finish
# the task, peek at an agent.
headless --scene chat --script "$walk" --at mark:sent --tree "$out/walk-sent.json"
headless --scene chat --script "$walk" --at mark:end --tree "$out/walk-end.json"
headless --scene chat --script "$walk" --at mark:end --snap "$out/walk-end.png"

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
check chat.json yes '"key": "approvals"'
check chat.json yes '"text": "Run without asking"'
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
# The managed firstmate clone: a card under the agents, and its page in each state.
check chat.json yes '"key": "firstmate"'
check chat.json yes '"text": "up to date"'
check chat.json no  '"key": "firstmate-page"'
check firstmate-missing.json yes '"text": "Firstmate is not installed"'
check firstmate-missing.json yes '"key": "firstmate-install"'
check firstmate-missing.json yes '"text": "not installed"'
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
check firstmate-blocked.json yes '"text": "update blocked"'
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
# output unfolded, and an image enlarged.
check tool-image.json yes '"key": "tool-2-image"'
check tool-image.json yes '"text": "Read"'
check tool-image.json no  '"key": "tool-2-placeholder"'
check tool-image-broken.json yes '"key": "tool-2-placeholder"'
check tool-image-broken.json yes 'broken.png. It could not be decoded'
check tool-video.json yes '"key": "tool-2-video"'
check tool-video.json yes '"key": "tool-2-play"'
check tool-video.json yes '"text": "Open in your video player"'
check tool-generic.json yes '"text": "Bash"'
check tool-generic.json yes '"key": "tool-2-result"'
check tool-generic.json yes '"text": "one\ntwo\nthree"'
check tool-picture.json yes '"key": "picture"'
check tool-picture.json yes '"key": "picture-close"'
check tool-picture.json no  '"key": "chat-input"'
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
# Visual replies: the settings page, a list sent back once and shown as a view, and a reply that
# ignored the feedback, sent back only once.
check chat.json yes '"key": "visuals"'
check chat.json yes '"text": "Enforce"'
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
# Firstmate's rules: the page with what two rules did, a merge question that names the pull
# request, and a write into a project waiting for the captain.
check chat.json yes '"key": "rules"'
check chat.json yes '"text": "7 of 7 on"'
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
