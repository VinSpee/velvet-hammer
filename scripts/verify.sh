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

# PNGs of each scene (`$app --list-scenes`), and the element tree of each.
for scene in chat replied empty peek; do
  headless --scene "$scene" --snap "$out/$scene.png"
  headless --scene "$scene" --tree "$out/$scene.json"
done

# Scripted input: type a message and send it, answer one item, finish the task, peek at an agent.
headless --script "$walk" --at mark:sent --tree "$out/walk-sent.json"
headless --script "$walk" --at mark:end --tree "$out/walk-end.json"
headless --script "$walk" --at mark:end --snap "$out/walk-end.png"

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
