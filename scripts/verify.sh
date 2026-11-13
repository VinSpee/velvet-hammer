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
  firstmate-missing firstmate-installing firstmate-up-to-date firstmate-update-available \
  firstmate-modified firstmate-blocked firstmate-session; do
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
check noted.json yes 'I noted it for you: Renew the velvet-hammer.dev domain'
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
check firstmate-session.json yes '/data/firstmate FM_HOME='
check firstmate-session.json yes '/data/home"'
check firstmate-session.json no  '"key": "firstmate-page"'
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
