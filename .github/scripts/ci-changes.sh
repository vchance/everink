#!/usr/bin/env bash
# Decides which CI jobs run, from the paths a change touches (core D2). One output per component in
# PROFILE.md › Components, plus `all`. The order of the cases is the order of the PROFILE › Check selection rows.
# Selection fails closed: an empty or unreadable file list, or a path that matches no pattern, selects everything.
#
# Usage: ci-changes.sh <event> <before-sha-or-empty> <after-sha-or-empty>
# Writes `rust=<bool>` and `all=<bool>` to $GITHUB_OUTPUT (or stdout when it is unset).
set -euo pipefail

event="${1:-}"
before="${2:-}"
after="${3:-}"
out="${GITHUB_OUTPUT:-/dev/stdout}"

emit() { printf 'rust=%s\nall=%s\n' "$1" "$2" >>"$out"; printf 'selection: rust=%s all=%s (%s)\n' "$1" "$2" "$3"; }

case "$event" in
  pull_request)
    # The checkout is the test merge commit; its first parent is the base, so this is the merge-base diff.
    files=$(git diff --name-only --no-renames HEAD^1 HEAD) || { emit true true "file list could not be computed"; exit 0; }
    ;;
  push)
    if [ -z "$before" ] || [ -z "${before//0/}" ]; then emit true true "no previous revision"; exit 0; fi
    files=$(git diff --name-only --no-renames "$before" "$after") || { emit true true "file list could not be computed"; exit 0; }
    ;;
  *)
    emit true true "event $event runs every job"; exit 0 ;;
esac

if [ -z "$files" ]; then emit true true "empty file list"; exit 0; fi

rust=false
all=false
while IFS= read -r f; do
  case "$f" in
    # Agent-policy files and the CI configuration: no product job reads them (core D2). Their checks are
    # policy-drift and pr-evidence.
    AGENTS.md|*/AGENTS.md|CLAUDE.md|*/CLAUDE.md|PROFILE.md|.agent-rules/*|.claude/*|.githooks/*|.github/*) ;;
    # Prose and licenses: no product job.
    README.md|docs/*|LICENSE-MIT|LICENSE-APACHE|.gitignore) ;;
    # The Rust workspace: the crates and everything the build reads.
    crates/*|Cargo.toml|Cargo.lock|rust-toolchain.toml|rustfmt.toml|clippy.toml) rust=true ;;
    # Anything else is unknown, and unknown paths escalate.
    *) all=true ;;
  esac
done <<<"$files"

emit "$rust" "$all" "by path"
