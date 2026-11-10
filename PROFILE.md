# PROFILE.md — repository profile (template; a documented table, not YAML — automation can come later if it earns it)

| Field | Value |
|---|---|
| Repository | {{slug}} · default branch `{{main}}` · visibility {{private/public}} · GitHub plan {{free/pro/team}} (determines whether rulesets exist) |
| Classification | {{active product / library / infrastructure / prototype / archived / docs}} |
| Accountable owner | {{name}} — owner-reserved decisions (R8); approver of exceptions and D-overrides |
| Supported platforms | {{e.g. x86_64 Linux (CI), aarch64 Linux (manual), Windows (nobody), iOS 17+ (manual on owner Mac)}} — mark each `CI` / `manual` / `nobody` |
| Toolchain | {{rust 1.xx (rust-toolchain.toml) / node 22 / cmake 3.28 ...}} |
| Required services for tests | {{Postgres w/ CREATEDB, MinIO, none ...}} and how CI provides them |
| CI runners and storage (D9, D11) | classes used: {{`SMALL` (policy-drift, pr-evidence, lint), `MEDIUM` (`test`, `build`), `LARGE` (`docker`), `MACOS` (`ios-build` — Xcode)}} — every `runs-on` is `${{ fromJSON(vars[format('RUNS_ON_<CLASS>__{0}', github.triggering_actor)] || '["unplaced"]') }}` and its job starts its `if:` with `${{ vars[format('RUNS_ON_<CLASS>__{0}', github.triggering_actor)] != '' }}`: no placement = the job is skipped, never a GitHub-hosted fallback; no provider or machine label in the repo; key: {{`github.triggering_actor` (variables `RUNS_ON_<CLASS>__<login>`) / `github.actor_id` (variables `RUNS_ON_<CLASS>__ID_<id>`, for a login containing `-`; a re-run by someone else then keeps the original actor)}}, one choice per repo; placements (per operator, {{repository variables / organization variables with access "Private repositories" where the plan shares them with private repos}}; unset = skipped): {{vchance: SMALL, MEDIUM → synerlytics self-hosted; LARGE, ARM → Ubicloud; MACOS, WINDOWS → `["macos-15"]` / `["windows-2025"]` hosted, explicit (owner call-out, date, link) or unset (skipped)}}; {{<other login>: own variables / none yet (their runs are skipped and merge.sh refuses their PRs until set)}}; opt-in classes (skipped when unplaced, `# d9-opt-in` on the job's `if:`; covered another way): {{`MACOS` — manual simulator run / none}}; ci-result fails on an unplaced required class: {{yes / n/a: no product jobs}}; scheduled lane runs as {{the last modifier of the cron schedule in ci.yml / pinned to <login>, checked on run <id>}}, and a scheduled or push run whose jobs were all skipped for lack of placement is not integration evidence (core D2, D9); Ubicloud coverage for {{org}} confirmed by {{owner, date, link}} or `n/a: no operator places a class on Ubicloud`; self-hosted pools reached from here: {{`synerlytics` — runner group `<name>`, selected private repos, public off, ephemeral / re-imaged between jobs, no `pull_request_target`/`workflow_run` jobs / none}}; artifact store and caches (D11, declared by the store host's runners as `CI_ARTIFACT_STORE` / `CI_LOCAL_CACHE` in their `.env`): {{`synerlytics:/data/ci-artifacts/<org>/<repo>/<run_id>-<run_attempt>/<name>/`, 14 days; box caches under `/data/sccache/<org>/<repo>/`; `actions/cache` only on Ubicloud (redirect verified <date>, <run>) / none: no job writes artifacts or caches}}; break-glass: {{owner changes their variables / runner group `<name>`}}; known unplaced: {{`ARM` (a person with no ARM runner: `ci-result` warns, jobs recorded `skipped: no placement`) / none}}. **Public repository:** replace this cell with `public: GitHub-hosted` (core D9, owner decision 2026-11-05): every job names one hosted label, no gate, no `RUNS_ON_*` |

## Device testing (core D10; simulators and emulators)
| Surface | Test runner | PR fast lane (smoke) | Integration / release (full) | Where it runs |
|---|---|---|---|---|
| {{mobile: apps/ios (SwiftUI)}} | {{XCUITest}} | {{iPhone 17 / iOS 26 (the runtime `macos-15` ships)}} | {{current iOS + minimum deployment target on current iPhone; iPad if supported}} | {{`macos-15` job on main/release; PR when D2 selects; else owner Mac, `manual (simulator, …)`}} |
| {{mobile: apps/android (Kotlin) / apps/mobile (Expo)}} | {{Espresso / Maestro}} | {{Pixel 8 / API 36 AVD}} | {{API 36 + minSdk API on Pixel-class AVD}} | {{Linux runner with KVM (a runner class whose placement is confirmed to have KVM)}} |
| none | — | — | — | {{repo has no mobile surface}} |

Web surfaces: {{the in-repo e2e suite (`e2e` job, Playwright/Chromium) is the browser evidence; no multi-browser matrix (D10)}}.

## Components and critical paths
| Component (path glob) | Owner | Critical? | Tests that cover it (command) | Consumers (transitive) |
|---|---|---|---|---|
| `{{crates/foo/**}}` | {{owner}} | yes/no | `{{cargo test -p foo}}` | `{{bar, baz}}` |

## Check selection (impact-based; core D2)
The CI workflow's `changes` job implements this table (`playbooks/testing-and-ci.md › Selection in the workflow`); keep the
two in step. Jobs not selected are recorded `n/a: not selected (D2)`; pushes to `{{DEFAULT_BRANCH}}` run the selected jobs; the full suite runs on the scheduled integration lane (core D2). A repository that deploys records here which it chose (owner decision): deploy on merge after the selected jobs, gate the deploy on the covering scheduled run, or deploy only when the owner says so.

| Changed paths | Checks that run on the PR | Evidence recorded as |
|---|---|---|
| `**/*.md` not under `docs/requirements/`, the agent-policy row's files excluded (R9) | doc checks (links, lint) | `docs: pass`; build/test `n/a` |
| `**/AGENTS.md`, `**/CLAUDE.md`, `PROFILE.md`, `.agent-rules/**`, `.claude/**`, `.githooks/**`, `.github/pull_request_template.md` | `policy-drift` + `pr-evidence`; **no product job** (core D2 since 0.1.18; high tier by R9) — or only `{{the job that reads them}}` (trace: `{{script:line}}`) | per check; build/test `n/a: not selected (D2)` |
| `.github/**` (workflows, actions, the selection script, toolchain pins set there) | `policy-drift` (lints placement and cache rules) + `pr-evidence`, with the `changes` job and `ci-result`; **no product job** (core D2 since 0.1.22; high tier by R9); the scheduled integration run first exercises the changed workflow after merge, and a `workflow_dispatch` run on the branch does before it when wanted | per check; build/test `n/a: not selected (D2)` |
| `{{crates/foo/**}}` | `{{cargo test -p foo -p bar}}` + lint + format | per check |
| `Cargo.lock`, `rust-toolchain.toml`, `package-lock.json`, build scripts outside `.github/**`, shared crates `{{...}}` | **escalation: full suite** | per check |
| Uncertain mapping / `{{>N}}` files touched / cross-component refactor | **escalation: full suite** | per check |

Escalation triggers (any one → full component suite; `{{...}}` → whole repository): shared or widely-consumed code,
toolchain/build-system change, broad refactor, uncertain impact, release candidate, a regression that escaped selection.

## Lanes
| Lane | What runs | When | Owner of a failure |
|---|---|---|---|
| PR (fast) | selected checks above | every PR commit | author |
| Integration batch | full suite on `{{DEFAULT_BRANCH}}` | {{hourly scheduled run, runs only when the default branch's head is uncovered and the last covering run started at least 4 hours earlier (core D2, 0.1.23) / <another cadence form, stated>}}; a push runs the selected jobs, or none when it carries its PR's `ci-result` on an identical tree (core D2, 0.1.23) | merger of the PRs since the last green run (bisect) |
| Scheduled | `{{audit, dependency advisories, platform matrix, soak}}` | `{{weekly Mon 06:00}}` | owner |
| Release | `{{full matrix + packaging + oracles}}` | tag / release PR | owner |
| Manual with evidence | `{{S7/BACnet oracles, iOS on owner Mac}}` | before release; result pasted in PR/release notes with date and revision | owner |

## Review triggers
| Playbook | Triggered by paths/changes |
|---|---|
| ux-review | `{{ui/**, styles/**, templates/**, rendered output fixtures}}` |
| technical-design | new public API, algorithm, data model, migration |
| architecture | new service/storage/major dependency, cross-repo boundary |
| security | `{{auth/**, api/**, uploads, deps, .github/**, permissions}}` |
| performance | `{{hot paths}}`; budgets: {{p95 latency, startup, binary size, memory}} measured by `{{bench cmd}}` on `{{hardware}}` |
| observability | runtime behavior or failure-mode changes in `{{services}}` |

## Merge authority
- All tiers: agent merges with `.agent-rules/merge.sh <n>` once core R3–R6 hold and the review verdict for the head is recorded (D5).
  No owner acceptance step (owner decision 2026-11-01). Emergency: declared only by `{{owner}}` (or `{{named human}}`); the PR
  names the declarer; follow-up PR within `{{N}}` days.
- Whoever merges follows the default branch's push run to its end and owns a failure there (core D5, 0.1.23).
- Flakes (core D2, 0.1.23): tallied in {{issue `Flaky tests` / `docs/flakes.md`}}; a test that flakes again within
  {{14}} days is fixed or quarantined by a profile change.
- Server-side enforcement available: {{none — free plan private repo | rulesets: required checks X, Y}}. If none, the
  `gh pr merge` guard (agent-rules `templates/merge-guard.md`; in the repo: `.agent-rules/merge.sh`) is the only mechanism and this is a known gap.

## Exceptions (governed)
R-rule gaps only (core §1): what the repository cannot yet satisfy, recorded with authority and expiry. A deviation from a
D-rule is an override, not an exception: in force only as an `overrides: Dn` block in `AGENTS.md › Overrides` (core §5.2).
| Id | Rule | Replacement | Rationale | Authority | Scope | Expiry/review |
|---|---|---|---|---|---|---|
| X1 | {{R5}} | {{...}} | {{...}} | {{owner, date}} — or `proposed — owner decision` | {{paths}} | {{date}} |

### Proposed D-rule overrides (not in force)
Moved into `AGENTS.md › Overrides` only once the owner accepts it; until then the core default stands as written.
- proposed `overrides: {{Dn}}` — {{replacement}} — why: {{...}} — authority: owner decision pending — scope: {{paths}} — review: {{date}}

## Release and operations (core R11)
| Field | Value |
|---|---|
| Versioning | {{semver via Cargo.toml / tags / marketing version}} and who bumps it |
| Changelog / release notes | {{CHANGELOG.md kept per PR / generated from PR titles / none for internal tools}} |
| Release procedure | {{tag → release lane → artifacts → deploy command or store submission}}; owner-only steps marked |
| Deploy targets | {{synerlytics docker compose / App Store / crates.io / none}} |
| Rollback | {{revert + redeploy / previous tag / migration down path}}; tested how often |
| Post-release watch | {{signals to watch, for how long, who}} |
| Dependency cadence | {{weekly audit; Dependabot grouped; majors are owner decisions}} |
| Deprecation policy | {{how long a deprecated API/flag stays, how announced}} |

## Cleanup (core R10)
| Resource this repo's work creates | Where | Teardown command | Safe to remove when |
|---|---|---|---|
| Build output | `{{target/, build/, node_modules/, .build/}}` | `{{cargo clean -p <crate> / rm -rf build/<preset>}}` | always (rebuilds) |
| Test services | `{{docker compose -f docker-compose.test.yml}}` / `{{Postgres db per test run}}` | `{{docker compose ... down -v}}` | tests finished |
| Simulators / emulators | `{{xcrun simctl list booted}}` | `{{xcrun simctl shutdown <udid>}}` | capture done |
| Worktrees | `.claude/worktrees/*` | `git worktree remove <path> && git worktree prune` | branch merged, `git status` clean |
| CI runner artifacts | `{{/data/build/<repo>, sccache}}` | `{{owner runbook}}`; agents only report sizes | owner decision |
| Cloud / paid | `{{Ubicloud runner minutes, k3d clusters, buckets}}` | `{{k3d cluster delete <name>}}` | job finished |

## Budgets
- CI: `{{≤ N runner-minutes per PR commit; full suite ≤ M/week}}` — measured from `gh run list` job durations (no report tool yet; say `to measure` until there is a baseline).
- Device tests: macOS hosted minutes `{{≤ N/week}}`; emulator job `{{≤ N min per run}}`.
- Performance: see Review triggers › performance.
