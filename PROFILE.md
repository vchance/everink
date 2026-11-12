# PROFILE.md — repository profile

| Field | Value |
|---|---|
| Repository | vchance/everink · default branch `main` · visibility public (since 2026-11-10) · GitHub plan Free (rulesets are available to a public repository, none configured; merge gate = `.agent-rules/merge.sh`, pre-push hook, `policy-drift`, `pr-evidence`) |
| Classification | prototype (spike stage): design documents and the first spike code (`crates/everink-core`). Nothing is released. Everink is a planned infinite, multi-user drawing surface with one core for native and WebAssembly (`docs/vision.md`) |
| Accountable owner | Von Chance (vchance) — owner-reserved decisions (R8); approver of exceptions and D-overrides |
| Supported platforms | Linux x86-64: `CI` (build, lint, tests). macOS aarch64: `manual` (the owner's Mac; the golden file is written there). wasm32 and Windows: `nobody` yet; wasm32 arrives with spike slice 3 |
| Toolchain | Rust 1.97.1, pinned in `rust-toolchain.toml` with `rustfmt` and `clippy`; edition 2024; one Cargo workspace (`Cargo.toml`, `Cargo.lock` committed). Dependencies: `num-bigint` 0.5.1 and `num-traits` 0.2.19 (both MIT OR Apache-2.0). Crates are `publish = false`: publishing is an owner decision |
| Required services for tests | none |
| CI runners and storage (D9, D11) | `public: GitHub-hosted` (owner call-out 2026-11-10, "gitrunners will be guthub", linked from `AGENTS.md` › Owner decisions): every job names the standard hosted label `ubuntu-24.04` directly, with no placement gate; no `RUNS_ON_*` variable exists (`RUNS_ON_SMALL__VCHANCE`, which named an Ubicloud label, was removed on 2026-11-10 before the switch); jobs: `policy-drift`, `pr-evidence`, and in `ci`: `changes`, `lint`, `test`, `ci-result`; ci-result: defined in `ci` and required by `merge.sh`; nothing is placed, so it checks job results only; scheduled lane: none (no `schedule:` trigger); self-hosted pools reached from here: none; artifact store and caches (D11): none: no job writes artifacts or caches |

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
| `AGENTS.md`, `CLAUDE.md`, `PROFILE.md`, `.agent-rules/**`, `.claude/**`, `.githooks/**`, `.github/**` | owner | yes (agent policy, R9) | `policy-drift` | agents working here |

## Check selection (impact-based; core D2)
The `ci` workflow's first job, `changes`, runs `.github/scripts/ci-changes.sh`, which holds the mapping below in
this order. A path that matches no row, or a file list that is empty or cannot be computed, selects every job.

| Changed paths | Checks that run on the PR | Evidence recorded as |
|---|---|---|
| `README.md`, `docs/**`, `LICENSE-MIT`, `LICENSE-APACHE`, `.gitignore` | `pr-evidence`; `changes` and `ci-result`; doc checks: none configured | `docs: n/a: none configured`; `lint` and `test`: `n/a: not selected (D2)` |
| `AGENTS.md`, `CLAUDE.md`, `PROFILE.md`, `.claude/**`, `.github/**`, `.agent-rules/**`, `.githooks/**` | `policy-drift` + `pr-evidence`; `changes` and `ci-result`; these paths select no product job (core 0.1.22) | per check (CI); `lint` and `test`: `n/a: not selected (D2)` |
| `crates/**`, `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml` | `lint` (`cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`) and `test` (`cargo test --workspace --locked`, then the timing example) | per check (CI), with the test count |
| Any other path, or an uncertain mapping | every job | per check (CI) |

Escalation triggers: with one component, `lint` and `test` are already the full suite; `workflow_dispatch` runs every job.

## Lanes
| Lane | What runs | When | Owner of a failure |
|---|---|---|---|
| PR (fast) | `pr-evidence` (every PR), `policy-drift` (policy paths), and `ci` (`changes`, the selected of `lint` and `test`, `ci-result`) on GitHub-hosted `ubuntu-24.04` (D9 public mode; no placement) | every PR commit | author |
| Integration batch | `ci` on each push to `main`, with the same selection over the pushed commits; it re-runs the jobs and does not carry the pull request's result | every merge | whoever merged (core D5) |
| Scheduled | none: `ci` has no `schedule` trigger. With one component every Rust change already runs every job; a change confined to `.github/**` is first exercised by the next Rust change or a `workflow_dispatch` (core D2) | — | owner |
| Release | none (nothing is released) | — | owner |
| Manual with evidence | none | — | owner |

## Review triggers
| Playbook | Triggered by paths/changes |
|---|---|
| ux-review | n/a until a UI exists |
| technical-design | any decision recorded in `docs/vision.md` (core language, CRDT, document model, sync protocol) |
| architecture | new service, storage or major dependency; the first code layout |
| security | `.github/**`, `.githooks/**`, permissions, anything that adds a network service or a dependency |
| performance | a change to `crates/everink-core` arithmetic that the timing example covers; figures and the machine they were measured on are in `docs/spike/` |
| observability | n/a (no runtime) |

## Merge authority
- All tiers: agent merges with `.agent-rules/merge.sh <n>` once core R3–R6 hold and the review verdict for the head is recorded (D5).
  No owner acceptance step (owner decision 2026-11-01). Emergency: declared only by Von Chance (vchance); the PR
  names the declarer; follow-up PR within 7 days.
- Whoever merges follows the default branch's push run of `ci` to its end and owns a failure there (core D5, 0.1.23).
- Flakes (core D2, 0.1.23): no check here is a test, so none is expected; any flake is tallied in issue `Flaky tests` (to be opened at the first flake) and a repeat within 14 days is fixed or quarantined by a profile change.
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
| Dependency cadence | none set yet: two dependencies, pinned by `Cargo.lock`; advisory checks (`cargo-deny`, `cargo-audit`) are planned in `docs/design/08-identity-and-security.md` and not yet configured |
| Deprecation policy | n/a (no public API) |

## Cleanup (core R10)
| Resource this repo's work creates | Where | Teardown command | Safe to remove when |
|---|---|---|---|
| Build output | `target/` in the checkout or worktree (ignored by git) | `cargo clean` | any time; it is rebuilt on demand |
| Worktrees | `.claude/worktrees/*` | `git worktree remove <path> && git worktree prune` | branch merged, `git status` clean |
| CI runner artifacts | none (policy workflows only, no artifacts) | — | — |
| Cloud / paid | none: GitHub-hosted minutes are free for a public repository | — | — |

## Budgets
- CI: `pr-evidence` and `policy-drift` under a minute each; `ci` a few minutes (no build cache, by core D11) — to measure on the first runs.
- Performance: per-frame position work for 10,000 objects under 0.05 ms, and a re-base inside 2 ms per frame up to depth 100 (measured: `docs/spike/results-1-positions.md`). Measured on the owner's Mac (Apple M4); CI machines are not a benchmark reference.
