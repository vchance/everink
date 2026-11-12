#!/usr/bin/env bash
# Decides which CI jobs run (core D2). One output per component in PROFILE.md › Components, plus `all`, plus
# `carry_failure` for the scheduled lane. The cases below are in the order of the PROFILE › Check selection rows.
# Selection fails closed: an empty or unreadable file list, or a path that matches no pattern, selects everything.
#
# Usage: ci-changes.sh <event> <before-sha-or-empty> <after-sha-or-empty>
# Writes `rust=<bool>`, `text=<bool>`, `all=<bool>` and `carry_failure=<bool>` to $GITHUB_OUTPUT (or stdout when
# it is unset).
set -euo pipefail

event="${1:-}"
before="${2:-}"
after="${3:-}"
out="${GITHUB_OUTPUT:-/dev/stdout}"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

text=false
emit() {
  printf 'rust=%s\ntext=%s\nall=%s\ncarry_failure=%s\n' "$1" "$text" "$2" "${4:-false}" >>"$out"
  printf 'selection: rust=%s text=%s all=%s carry_failure=%s (%s)\n' "$1" "$text" "$2" "${4:-false}" "$3"
}

list=$(mktemp)
trap 'rm -f "$list"' EXIT

case "$event" in
  pull_request)
    # The checkout is the test merge commit; its first parent is the base, so this is the merge-base diff.
    git diff -z --name-only --no-renames HEAD^1 HEAD >"$list" || { emit true true "file list could not be computed"; exit 0; }
    ;;
  push)
    if [ -z "$before" ] || [ -z "${before//0/}" ]; then emit true true "no previous revision"; exit 0; fi
    git diff -z --name-only --no-renames "$before" "$after" >"$list" || { emit true true "file list could not be computed"; exit 0; }
    ;;
  schedule)
    # The integration lane: every job on the branch head, unless that head is already covered (ci-lane.py).
    decision=$(python3 -I "$here/ci-lane.py") || decision=run
    case "$decision" in
      skip|gate) emit false false "lane: $decision" ;;
      carry-failure) emit false false "lane: carrying an earlier failure on this head" true ;;
      *) emit true true "lane: run every job" ;;
    esac
    exit 0
    ;;
  *)
    emit true true "event $event runs every job"; exit 0 ;;
esac

if [ ! -s "$list" ]; then emit true true "empty file list"; exit 0; fi

rust=false
all=false
while IFS= read -r -d '' f; do
  case "$f" in
    # Row 1. Prose and licenses: no product job.
    README.md|docs/*|LICENSE-MIT|LICENSE-APACHE|.gitignore) ;;
    # Row 2. Agent-policy files and the CI configuration: no product job reads them (core D2). Their checks are
    # policy-drift and pr-evidence.
    AGENTS.md|*/AGENTS.md|CLAUDE.md|*/CLAUDE.md|PROFILE.md|.agent-rules/*|.claude/*|.githooks/*|.github/*) ;;
    # Row 3. The Rust workspace: the crates and everything the build reads.
    crates/*|Cargo.toml|Cargo.lock|tools/*) rust=true ;;
    # Row 4. The text-library trial: a package of its own, outside the workspace.
    spike/text-trial/*) text=true ;;
    # Row 5. The pinned toolchain builds both, and the format, lint and cargo settings at the root are read by
    # both (the tools look in parent directories, and cargo reads `.cargo/` from where it is run).
    rust-toolchain.toml|rustfmt.toml|clippy.toml|.cargo/*) rust=true; text=true ;;
    # Row 6. Anything else is unknown, and unknown paths escalate.
    *) all=true ;;
  esac
done <"$list"

emit "$rust" "$all" "by path"
