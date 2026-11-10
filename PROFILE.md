# PROFILE.md — repository profile

| Field | Value |
|---|---|
| Repository | vchance/everink · default branch `main` · visibility private · GitHub plan Free (no rulesets; merge gate = `.agent-rules/merge.sh`, pre-push hook, `policy-drift`, `pr-evidence`) |
| Classification | prototype (idea and research stage): documents only, no code yet. Everink is a planned infinite, multi-user drawing surface with one core for native and WebAssembly (`docs/vision.md`) |
| Accountable owner | Von Chance (vchance) — owner-reserved decisions (R8); approver of exceptions and D-overrides |
| Supported platforms | none yet (no code). Planned: desktop native and browser (wasm + WebGPU); each will be marked `CI` / `manual` / `nobody` when its code lands |
| Toolchain | none yet; the core language (Rust or C++) is an open owner decision (`docs/vision.md` › Open questions) |
| Required services for tests | none (no tests yet) |
| CI runners and storage (D9, D11) | classes used: `SMALL` (policy-drift, pr-evidence) only — every `runs-on` is `${{ fromJSON(vars[format('RUNS_ON_<CLASS>__{0}', github.triggering_actor)] || '["unplaced"]') }}` and its job starts its `if:` with `${{ vars[format('RUNS_ON_<CLASS>__{0}', github.triggering_actor)] != '' }}`: no placement = the job is skipped, never a GitHub-hosted fallback; no provider or machine label in the repo; key: `github.triggering_actor` (variables `RUNS_ON_<CLASS>__<login>`); placements (per operator, repository variables under the personal account; unset = skipped): vchance: SMALL → Ubicloud; other logins: none yet (their runs are skipped and merge.sh refuses their PRs until set); opt-in classes: none; ci-result fails on an unplaced required class: n/a: no product jobs; scheduled lane: none (no `schedule:` trigger); Ubicloud coverage for vchance (personal account) confirmed by a `policy-drift` run on Ubicloud in vchance/little-readers-vol1, 2026-11-10 (run 42521515026); self-hosted pools reached from here: none; artifact store and caches (D11): none: no job writes artifacts or caches; break-glass: owner changes their variables; known unplaced: none |

## Device testing (core D10; simulators and emulators)
| Surface | Test runner | PR fast lane (smoke) | Integration / release (full) | Where it runs |
|---|---|---|---|---|
| none | — | — | — | repo has no mobile surface (no code yet) |

Web surfaces: none yet. When the browser shell lands, its in-repo e2e suite is the browser evidence (D10).

## Components and critical paths
| Component (path glob) | Owner | Critical? | Tests that cover it (command) | Consumers (transitive) |
|---|---|---|---|---|
| `README.md`, `docs/**` | owner | no | none | people |
| `AGENTS.md`, `CLAUDE.md`, `PROFILE.md`, `.agent-rules/**`, `.claude/**`, `.githooks/**`, `.github/**` | owner | yes (agent policy, R9) | `policy-drift` | agents working here |

## Check selection (impact-based; core D2)
There is no CI build or test: the only workflows are `policy-drift` and `pr-evidence`, so there is no `changes` job
yet. The PR that adds the first code also adds its CI workflow and the rows for it here.

| Changed paths | Checks that run on the PR | Evidence recorded as |
|---|---|---|
| `**/*.md` (`AGENTS.md`/`PROFILE.md`/`CLAUDE.md` excluded, R9) | `pr-evidence`; doc checks: none configured | `docs: n/a: none configured`; build/test `n/a: no code` |
| `AGENTS.md`, `CLAUDE.md`, `PROFILE.md`, `.claude/**`, `.github/**`, `.agent-rules/**`, `.githooks/**` | `policy-drift` + `pr-evidence`; no product job exists, and core 0.1.22 makes `.github/**` and the agent-policy set select none | per check (CI) |
| Uncertain mapping | `policy-drift` + `pr-evidence` | per check (CI) |

Escalation triggers: none until code exists (there is no fuller suite to escalate to).

## Lanes
| Lane | What runs | When | Owner of a failure |
|---|---|---|---|
| PR (fast) | `pr-evidence` (every PR), `policy-drift` (policy paths) on the `SMALL` runner class (D9; vchance's runs on Ubicloud; others with no placement are skipped, and merge.sh refuses their PRs until they have variables) | every PR commit | author |
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
- Server-side enforcement available: none — free plan private repo. The `gh pr merge` guard (agent-rules
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
| Cloud / paid | Ubicloud runner minutes for the two policy jobs | none (runners are discarded per job) | job finished |

## Budgets
- CI: `pr-evidence` and `policy-drift` only, each about a minute on the `SMALL` class — to measure.
- Performance: none until code exists.
