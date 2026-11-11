# PROFILE.md — repository profile

| Field | Value |
|---|---|
| Repository | vchance/everink · default branch `main` · visibility public (since 2026-11-10) · GitHub plan Free (rulesets are available to a public repository, none configured; merge gate = `.agent-rules/merge.sh`, pre-push hook, `policy-drift`, `pr-evidence`) |
| Classification | prototype (idea and research stage): documents only, no code yet. Everink is a planned infinite, multi-user drawing surface with one core for native and WebAssembly (`docs/vision.md`) |
| Accountable owner | Von Chance (vchance) — owner-reserved decisions (R8); approver of exceptions and D-overrides |
| Supported platforms | none yet (no code). Planned: desktop native and browser (wasm + WebGPU); each will be marked `CI` / `manual` / `nobody` when its code lands |
| Toolchain | none installed yet; the core language is Rust (owner decision 2026-11-10, `AGENTS.md` › Owner decisions). The Rust version, edition and wasm tooling are set by the PR that adds the first code |
| Required services for tests | none (no tests yet) |
| CI runners and storage (D9, D11) | `public: GitHub-hosted` (owner call-out 2026-11-10, "gitrunners will be guthub", linked from `AGENTS.md` › Owner decisions): every job names the standard hosted label `ubuntu-24.04` directly, with no placement gate; no `RUNS_ON_*` variable exists (`RUNS_ON_SMALL__VCHANCE`, which named an Ubicloud label, was removed on 2026-11-10 before the switch); jobs: `policy-drift` and `pr-evidence` only; ci-result: n/a: no product jobs and nothing is placed; scheduled lane: none (no `schedule:` trigger); self-hosted pools reached from here: none; artifact store and caches (D11): none: no job writes artifacts or caches |

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
| `README.md`, `docs/**`, `LICENSE-MIT`, `LICENSE-APACHE` | owner | no | none | people |
| `AGENTS.md`, `CLAUDE.md`, `PROFILE.md`, `.agent-rules/**`, `.claude/**`, `.githooks/**`, `.github/**` | owner | yes (agent policy, R9) | `policy-drift` | agents working here |

## Check selection (impact-based; core D2)
There is no CI build or test: the only workflows are `policy-drift` and `pr-evidence`, so there is no `changes` job
yet. The PR that adds the first code also adds its CI workflow and the rows for it here.

| Changed paths | Checks that run on the PR | Evidence recorded as |
|---|---|---|
| `**/*.md` (`AGENTS.md`/`PROFILE.md`/`CLAUDE.md` excluded, R9), `LICENSE-MIT`, `LICENSE-APACHE` | `pr-evidence`; doc checks: none configured | `docs: n/a: none configured`; build/test `n/a: no code` |
| `AGENTS.md`, `CLAUDE.md`, `PROFILE.md`, `.claude/**`, `.github/**`, `.agent-rules/**`, `.githooks/**` | `policy-drift` + `pr-evidence`; no product job exists, and core 0.1.22 makes `.github/**` and the agent-policy set select none | per check (CI) |
| Uncertain mapping | `policy-drift` + `pr-evidence` | per check (CI) |

Escalation triggers: none until code exists (there is no fuller suite to escalate to).

## Lanes
| Lane | What runs | When | Owner of a failure |
|---|---|---|---|
| PR (fast) | `pr-evidence` (every PR), `policy-drift` (policy paths) on GitHub-hosted `ubuntu-24.04` (D9 public mode; no placement) | every PR commit | author |
| Integration batch | none (no CI on `main`) | — | — |
| Scheduled | none | — | owner |
| Release | none (nothing is released) | — | owner |
| Manual with evidence | none | — | owner |

## Review triggers
| Playbook | Triggered by paths/changes |
|---|---|
| ux-review | n/a until a UI exists |
| technical-design | any decision recorded in `docs/vision.md` (core language, CRDT, document model, sync protocol) |
| architecture | new service, storage or major dependency; the first code layout |
| security | `.github/**`, `.githooks/**`, permissions, anything that adds a network service or a dependency |
| performance | n/a until code exists; budgets will be stated in `docs/vision.md` › Next steps (stroke latency, merge time, memory) |
| observability | n/a (no runtime) |

## Merge authority
- All tiers: agent merges with `.agent-rules/merge.sh <n>` once core R3–R6 hold and the review verdict for the head is recorded (D5).
  No owner acceptance step (owner decision 2026-11-01). Emergency: declared only by Von Chance (vchance); the PR
  names the declarer; follow-up PR within 7 days.
- Whoever merges follows the default branch's push run to its end and owns a failure there (core D5, 0.1.23); here no workflow runs on a push.
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
| Dependency cadence | n/a (no dependencies) |
| Deprecation policy | n/a (no public API) |

## Cleanup (core R10)
| Resource this repo's work creates | Where | Teardown command | Safe to remove when |
|---|---|---|---|
| Build output | none yet | — | — |
| Worktrees | `.claude/worktrees/*` | `git worktree remove <path> && git worktree prune` | branch merged, `git status` clean |
| CI runner artifacts | none (policy workflows only, no artifacts) | — | — |
| Cloud / paid | none: GitHub-hosted minutes are free for a public repository | — | — |

## Budgets
- CI: `pr-evidence` and `policy-drift` only, each under a minute on a GitHub-hosted runner.
- Performance: none until code exists.
