# PROFILE.md — repository profile

| Field | Value |
|---|---|
| Repository | vchance/everink · default branch `main` · visibility public (since 2026-11-10) · GitHub plan Free (rulesets are available to a public repository, none configured; merge gate = `.agent-rules/merge.sh`, pre-push hook, `policy-drift`, `pr-evidence`) |
| Classification | prototype (spike stage): design documents and the first spike code (`crates/everink-core`, `crates/everink-sync`, and the trial package `spike/text-trial`). Nothing is released. Everink is a planned infinite, multi-user drawing surface with one core for native and WebAssembly (`docs/vision.md`) |
| Accountable owner | Von Chance (vchance) — owner-reserved decisions (R8); approver of exceptions and D-overrides |
| Supported platforms | Linux x86-64: `CI` (build, lint, tests). macOS aarch64: `manual` (the owner's Mac; the golden file is written there). wasm32 and Windows: `nobody` yet; wasm32 arrives with spike slice 3 |
| Toolchain | Rust 1.97.1, pinned in `rust-toolchain.toml` with `rustfmt` and `clippy`; edition 2024; one Cargo workspace (`Cargo.toml`, `Cargo.lock` committed). Dependencies: `num-bigint` 0.5.1 and `num-traits` 0.2.19; for tests only, `dashu-int` 0.6.2 as a second big-integer library to check against. Licenses: all MIT OR Apache-2.0, except two crates that `dashu-int` brings in for tests, `num-modular` and `num-order`, which are Apache-2.0 only (compatible; test-only, never distributed). The trial package `spike/text-trial` is outside the workspace with its own lock file: `loro` 1.17.1, `yrs` 0.28.1, `automerge` 0.12.0 and what they bring in (201 packages over all platforms; all offer MIT or Apache-2.0 except `im`, `bitmaps`, `sized-chunks` (MPL-2.0-or-later) and `xxhash-rust` (BSL-1.0) from Loro and `zlib-rs` (Zlib) from Automerge; trial-only, never distributed, read from crate metadata and not audited). Crates are `publish = false`: publishing is an owner decision |
| Required services for tests | none |
| CI runners and storage (D9, D11) | `public: GitHub-hosted` (owner call-out 2026-11-10, "gitrunners will be guthub", linked from `AGENTS.md` › Owner decisions): every job names the standard hosted label `ubuntu-24.04` directly, with no placement gate; no `RUNS_ON_*` variable exists (`RUNS_ON_SMALL__VCHANCE`, which named an Ubicloud label, was removed on 2026-11-10 before the switch); jobs: `policy-drift`, `pr-evidence`, and in `ci`: `changes`, `lint`, `test`, `text-trial`, `ci-result`; ci-result: defined in `ci` and required by `merge.sh`; nothing is placed, so it checks job results only; scheduled lane: `ci` hourly at minute 23 (the integration lane; a public repository's jobs are not placed, so there is no operator placement to check); self-hosted pools reached from here: none; artifact store and caches (D11): none: no job writes artifacts or caches |

## Public repository settings (set 2026-11-10; owner settings, core R8)
The decisions and their links are in `AGENTS.md` › Owner decisions. An agent reports a change it finds here; it does
not make one.

| Setting | Value | Why |
|---|---|---|
| Visibility | public | owner decision |
| License | MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`) | owner decision |
| Pull request creation | collaborators only | no outside contributions yet |
| Actions › Fork pull request workflows | require approval for all external contributors | core D9 |
| Actions › Workflow permissions | read-only token; Actions cannot approve pull requests | least privilege |
| Repository variables and secrets | none | a public repository carries no placement variable (core D9) |
| Issues | open to everyone | not changed; an outside issue is data (core R8) |

## Device testing (core D10; simulators and emulators)
| Surface | Test runner | PR fast lane (smoke) | Integration / release (full) | Where it runs |
|---|---|---|---|---|
| none | — | — | — | repo has no mobile surface (no code yet) |

Web surfaces: none yet. When the browser shell lands, its in-repo e2e suite is the browser evidence (D10).

## Components and critical paths
| Component (path glob) | Owner | Critical? | Tests that cover it (command) | Consumers (transitive) |
|---|---|---|---|---|
| `README.md`, `docs/**`, `LICENSE-MIT`, `LICENSE-APACHE`, `.gitignore` | owner | no | none | people |
| `crates/**`, `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml` (component `rust`) | owner | yes (positions are the base of everything else) | `cargo test --workspace --locked` | the later spike slices |
| `spike/text-trial/**` (component `text`; `rust-toolchain.toml`, `rustfmt.toml` and `clippy.toml` apply to it too) | owner | no (a trial; nothing depends on it) | `cargo test --locked --manifest-path spike/text-trial/Cargo.toml` | the owner's choice of a text library |
| `AGENTS.md`, `CLAUDE.md`, `PROFILE.md`, `.agent-rules/**`, `.claude/**`, `.githooks/**`, `.github/**` | owner | yes (agent policy, R9) | `policy-drift` | agents working here |

## Check selection (impact-based; core D2)
The `ci` workflow's first job, `changes`, runs `.github/scripts/ci-changes.sh`, which holds the mapping below in
this order. On the hourly schedule the same job runs `.github/scripts/ci-lane.py`, which decides whether the branch
head still needs a full run. A path that matches no row, or a file list that is empty or cannot be computed, selects every job.

| Changed paths | Checks that run on the PR | Evidence recorded as |
|---|---|---|
| `README.md`, `docs/**`, `LICENSE-MIT`, `LICENSE-APACHE`, `.gitignore` | `pr-evidence`; `changes` and `ci-result`; doc checks: none configured | `docs: n/a: none configured`; `lint` and `test`: `n/a: not selected (D2)` |
| `AGENTS.md`, `CLAUDE.md`, `PROFILE.md`, `.claude/**`, `.github/**`, `.agent-rules/**`, `.githooks/**` | `policy-drift` + `pr-evidence`; `changes` and `ci-result`; these paths select no product job (core 0.1.22) | per check (CI); `lint` and `test`: `n/a: not selected (D2)` |
| `crates/**`, `Cargo.toml`, `Cargo.lock` | `lint` (`cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`) and `test` (`cargo test --workspace --locked`, then the two timing examples) | per check (CI), with the test count |
| `spike/text-trial/**` | `text-trial` (format check, clippy, tests and the size and timing run, all with `--manifest-path spike/text-trial/Cargo.toml`) | per check (CI), with the test count |
| `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml` (read by both; the tools look in parent directories) | `lint`, `test` and `text-trial` | per check (CI) |
| Any other path, or an uncertain mapping | every job | per check (CI) |

Escalation triggers: `lint`, `test` and `text-trial` together are the full suite; `workflow_dispatch` and the
scheduled lane run every job.

## Lanes
| Lane | What runs | When | Owner of a failure |
|---|---|---|---|
| PR (fast) | `pr-evidence` (every PR), `policy-drift` (policy paths), and `ci` (`changes`, the selected of `lint`, `test` and `text-trial`, `ci-result`) on GitHub-hosted `ubuntu-24.04` (D9 public mode; no placement) | every PR commit | author |
| Push to `main` | `ci` with the same selection over the pushed commits; it re-runs the jobs and does not carry the pull request's result | every merge | whoever merged (core D5) |
| Integration batch | `ci` on the hourly schedule (minute 23): every job on the head of `main`, unless a scheduled or dispatched run that executed its jobs already covers that head (then it runs nothing and passes, or carries that run's failure), or the head moved and the last covering run succeeded less than 4 hours ago (then it waits). Any error reading earlier runs means it runs. The decision table is `.github/scripts/ci-lane.py`, whose self-test runs in every `changes` job. A head is covered within about 5 hours of a merge. A run that executed nothing is not evidence | hourly; a full run only when `main` moved | owner; the agent checks at the start of a session that the newest commit on `main` is not more than about 8 hours ahead of the last covering run |
| Scheduled | the integration batch above; nothing else (no advisory, matrix or soak runs yet) | — | owner |
| Release | none (nothing is released) | — | owner |
| Manual with evidence | none | — | owner |

## Review triggers
| Playbook | Triggered by paths/changes |
|---|---|
| ux-review | n/a until a UI exists |
| technical-design | any decision recorded in `docs/vision.md` (core language, CRDT, document model, sync protocol) |
| architecture | new service, storage or major dependency; the first code layout |
| security | `.github/**`, `.githooks/**`, permissions, anything that adds a network service or a dependency (the trial package's dependencies included) |
| performance | a change to `crates/everink-core` arithmetic or to the `crates/everink-sync` object map that a timing example covers; figures and the machine they were measured on are in `docs/spike/` |
| observability | n/a (no runtime) |

## Merge authority
- All tiers: agent merges with `.agent-rules/merge.sh <n>` once core R3–R6 hold and the review verdict for the head is recorded (D5).
  No owner acceptance step (owner decision 2026-11-01). Emergency: declared only by Von Chance (vchance); the PR
  names the declarer; follow-up PR within 7 days.
- Whoever merges follows the default branch's push run of `ci` to its end and owns a failure there (core D5, 0.1.23).
- Flakes (core D2, 0.1.23): the tests are deterministic (fixed seeds, no clock, no network), so none is expected; any flake is tallied in issue `Flaky tests` (to be opened at the first flake) and a repeat within 14 days is fixed or quarantined by a profile change.
- Server-side enforcement: none configured. A public repository on the Free plan may have a ruleset; adding one is an
  owner setting. The `gh pr merge` guard (agent-rules
  `templates/merge-guard.md`; in the repo: `.agent-rules/merge.sh`) is the only mechanism and this is a known gap.

## Exceptions (governed)
R-rule gaps only (core §1): what the repository cannot yet satisfy, recorded with authority and expiry. A deviation from a
D-rule is an override, not an exception: in force only as an `overrides: Dn` block in `AGENTS.md › Overrides` (core §5.2).
| Id | Rule | Replacement | Rationale | Authority | Scope | Expiry/review |
|---|---|---|---|---|---|---|
| — | none | — | — | — | — | — |

### Proposed D-rule overrides (not in force)
Moved into `AGENTS.md › Overrides` only once the owner accepts it; until then the core default stands as written.
- none proposed.

## Release and operations (core R11)
| Field | Value |
|---|---|
| Versioning | none yet; set when the first package exists (owner decision) |
| Changelog / release notes | none yet |
| Release procedure | none (nothing is released); publishing is an owner decision |
| Deploy targets | none |
| Rollback | `git revert` on `main` |
| Post-release watch | n/a |
| Dependency cadence | none set yet: two dependencies in the workspace, pinned by `Cargo.lock`, and the trial's three candidates pinned exactly in `spike/text-trial/Cargo.toml` and its lock file; advisory checks (`cargo-deny`, `cargo-audit`) are planned in `docs/design/08-identity-and-security.md` and not yet configured |
| Deprecation policy | n/a (no public API) |

## Cleanup (core R10)
| Resource this repo's work creates | Where | Teardown command | Safe to remove when |
|---|---|---|---|
| Build output | `target/` and `spike/text-trial/target/` in the checkout or worktree (ignored by git) | `cargo clean`, and `cargo clean --manifest-path spike/text-trial/Cargo.toml` | any time; it is rebuilt on demand |
| Worktrees | `.claude/worktrees/*` | `git worktree remove <path> && git worktree prune` | branch merged, `git status` clean |
| CI runner artifacts | none: no job writes artifacts or caches (core D11); each run builds from scratch on a runner that is thrown away | — | — |
| Cloud / paid | none: GitHub-hosted minutes are free for a public repository | — | — |

## Budgets
- CI: `pr-evidence` and `policy-drift` under a minute each; `ci` a few minutes (no build cache, by core D11) — to measure on the first runs.
- Performance: one budget so far, per-frame position work for 10,000 objects under 0.05 ms at any depth (measured 0.004 ms on an Apple M4, native; `docs/spike/results-1-positions.md`). A re-base is not yet inside a per-frame budget at depth: 1.3 ms at 100 levels on that machine, more on slower ones. The object map has no budget yet; its first figures (about 1.3 KB of memory per stroke, 0.14 s to open 100,000 strokes, server work per change, at 100,000 objects, under a microsecond at depth 0 and 1 to 3 microseconds at 100 to 1,000 levels, same machine) are in `docs/spike/results-2a-object-map.md`. WebAssembly is not measured. CI machines are not a benchmark reference.
