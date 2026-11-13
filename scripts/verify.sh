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
for scene in chat replied empty peek signed-out signing-in signed-in expired cli-missing claude-error; do
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
