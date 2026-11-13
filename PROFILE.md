# PROFILE.md — repository profile

| Field | Value |
|---|---|
| Repository | vchance/everink · default branch `main` · visibility public (since 2026-11-10) · GitHub plan Free (rulesets are available to a public repository, none configured; merge gate = `.agent-rules/merge.sh`, pre-push hook, `policy-drift`, `pr-evidence`) |
| Classification | prototype (spike stage): design documents and the first spike code (`crates/everink-core`, `crates/everink-sync`, and the trial packages `spike/text-trial`, `spike/render-trial`, `spike/render-web`, `spike/store-trial`, `spike/controls-trial`, `spike/controls-window` and `spike/controls-web`). Nothing is released. Everink is a planned infinite, multi-user drawing surface with one core for native and WebAssembly (`docs/vision.md`) |
| Accountable owner | Von Chance (vchance) — owner-reserved decisions (R8); approver of exceptions and D-overrides |
| Supported platforms | Linux x86-64: `CI` (build, lint, tests). macOS aarch64: `manual` (the owner's Mac; the golden file is written there; the controls window is opened and its screen-reader and keyboard checks are made there). wasm32 (`wasm32-wasip1`, under Node): `CI` for `everink-core` only (tests and both golden files). A browser (`wasm32-unknown-unknown`): `CI` builds and lints `spike/render-web` and lints `spike/controls-web`; `manual` for running them (Chrome on the owner's Mac; the renderer page checks itself, and `spike/controls-web/check.mjs` drives the controls page). Windows: `nobody` |
| Toolchain | Rust 1.97.1, pinned in `rust-toolchain.toml` with `rustfmt`, `clippy` and the `wasm32-wasip1` and `wasm32-unknown-unknown` targets; Node (as found on the machine: 26 on the owner's Mac, the runner's own on CI) runs the wasm32 tests through `tools/wasi-run.mjs`; edition 2024; one Cargo workspace (`Cargo.toml`, `Cargo.lock` committed). Dependencies: `num-bigint` 0.5.1 and `num-traits` 0.2.19; for tests only, `dashu-int` 0.6.2 as a second big-integer library to check against; for one example only, `socket2` 0.6.5 (with `libc`, and on Windows targets `windows-sys` and `windows-link`), all MIT OR Apache-2.0. Licenses: all MIT OR Apache-2.0, except two crates that `dashu-int` brings in for tests, `num-modular` and `num-order`, which are Apache-2.0 only (compatible; test-only, never distributed). The trial package `spike/text-trial` is outside the workspace with its own lock file: `loro` 1.17.1, `yrs` 0.28.1, `automerge` 0.12.0 and what they bring in (201 packages over all platforms; all offer MIT or Apache-2.0 except `im`, `bitmaps`, `sized-chunks` (MPL-2.0-or-later) and `xxhash-rust` (BSL-1.0) from Loro and `zlib-rs` (Zlib) from Automerge; trial-only, never distributed, read from crate metadata and not audited). The trial package `spike/render-trial` is outside the workspace with its own lock file: `vello_gpu` 0.4.0, `vello_cpu` 0.4.0, `vello_common` 0.4.0, `wgpu` 30.0.2, `pollster` 1.0.1 and what they bring in (214 packages over all platforms; all offer MIT or Apache-2.0 except `foldhash`, `slotmap` and `zlib-rs` (Zlib), `libloading` (ISC) and `yoke`, `yoke-derive`, `zerofrom`, `zerofrom-derive` (Unicode-3.0); trial-only, never distributed, read from crate metadata and not audited). The trial package `spike/render-web` is outside the workspace with its own lock file: the same renderer packages for `wasm32-unknown-unknown`, with `wasm-bindgen` 0.2.130, `wasm-bindgen-futures` 0.4.80, `js-sys` and `web-sys` 0.3.107 and `futures-channel` 0.3.34 (168 packages over all platforms; all offer MIT or Apache-2.0 except `foldhash` and `zlib-rs` (Zlib), `libloading` (ISC) and `yoke`, `yoke-derive`, `zerofrom`, `zerofrom-derive` (Unicode-3.0); trial-only, never distributed, read from crate metadata). `cargo audit` on 2026-11-12 found no advisory for the workspace's lock file or the two renderer trials'; for `spike/text-trial` it gave seven warnings and no vulnerability (unmaintained: `atomic-polyfill`, `bitmaps`, `im`, `sized-chunks`, `smallstr`; unsound: `im`, `sized-chunks`; all brought in by Loro). The trial package `spike/store-trial` is outside the workspace with its own lock file: `rusqlite` 0.42.0 with its `bundled` feature (SQLite 3.54.1, built from source, public domain), `lz4_flex` 0.14.0, `blake3` 1.8.7, `crc32c` 0.6.9 and what they bring in (37 packages over all platforms; all offer MIT or Apache-2.0 except `foldhash` (Zlib); trial-only, never distributed, read from crate metadata; no advisory on 2026-11-12). The trial package `spike/controls-trial` is outside the workspace with its own lock file: `egui` 0.37.0, `taffy` 0.15.1, `parley` 0.12.0, `accesskit` 0.25.2 and what they bring in (120 packages over all platforms; all offer MIT or Apache-2.0 except `foldhash` and `slotmap` (Zlib), `libloading` (ISC) and 22 packages of ICU4X and its helpers (Unicode-3.0), which `parley` brings; `unicode-ident` and `unicode-properties` are (MIT OR Apache-2.0) AND Unicode-3.0; `epaint_default_fonts` is MIT OR Apache-2.0 for its code and the trial builds three of its font files in: Ubuntu Light (Ubuntu Font Licence 1.0; used by both candidates), Hack (MIT and the Bitstream Vera licence, which reserves the font names) and a small icon set (MIT); its fonts under OFL-1.1 are not built in; trial-only, never distributed, read from crate metadata; no advisory on 2026-11-12). The trial package `spike/controls-window` is outside the workspace with its own lock file: `winit` 0.30.13, `accesskit_winit` 0.34.2, `wgpu` 30.0.2, `egui-wgpu` 0.37.0, `vello_gpu` 0.4.0, `glifo` 0.5.0, `png` 0.18.1 and what they bring in (440 packages over all platforms; all offer MIT or Apache-2.0, twelve of them Apache-2.0 only, among them `winit` and `accesskit_winit`, and `dpi` under both at once (Apache-2.0 AND MIT), except `arrayref` (BSD-2-Clause), `tiny-skia` and `tiny-skia-path` (BSD-3-Clause), `libloading` (ISC), `foldhash`, `slotmap` and `zlib-rs` (Zlib), the 22 of ICU4X (Unicode-3.0) and the font files noted for the controls trial; trial-only, never distributed, read from crate metadata; `cargo audit` on 2026-11-12: no vulnerability, four warnings of unmaintained packages (`ttf-parser`, `zbus_names`, `zvariant`, `zvariant_derive`, the last three from the Linux accessibility adapter)). The trial package `spike/controls-web` is outside the workspace with its own lock file: the controls window's packages for `wasm32-unknown-unknown` without winit and the platform adapters, with `wasm-bindgen` 0.2.130, `wasm-bindgen-futures` 0.4.80, `web-sys` 0.3.107 and `serde_json` (221 packages over all platforms; all offer MIT or Apache-2.0, two of them Apache-2.0 only, except `libloading` (ISC), `foldhash`, `slotmap` and `zlib-rs` (Zlib), the 22 of ICU4X (Unicode-3.0) and the font files noted for the controls trial; trial-only, never distributed, read from crate metadata; no advisory on 2026-11-12). The browser trial is built with the `wasm-bindgen` command line tool 0.2.130, installed by hand (`cargo install wasm-bindgen-cli --version 0.2.130 --locked`); CI does not need it. Crates are `publish = false`: publishing is an owner decision |
| Required services for tests | none |
| CI runners and storage (D9, D11) | `public: GitHub-hosted` (owner call-out 2026-11-10, "gitrunners will be guthub", linked from `AGENTS.md` › Owner decisions): every job names the standard hosted label `ubuntu-24.04` directly, with no placement gate; no `RUNS_ON_*` variable exists (`RUNS_ON_SMALL__VCHANCE`, which named an Ubicloud label, was removed on 2026-11-10 before the switch); jobs: `policy-drift`, `pr-evidence`, and in `ci`: `changes`, `lint`, `test`, `text-trial`, `render-trial`, `store-trial`, `controls-trial`, `ci-result`; ci-result: defined in `ci` and required by `merge.sh`; nothing is placed, so it checks job results only; scheduled lane: `ci` hourly at minute 23 (the integration lane; a public repository's jobs are not placed, so there is no operator placement to check); self-hosted pools reached from here: none; artifact store and caches (D11): none: no job writes artifacts or caches |

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
| `crates/**`, `Cargo.toml`, `Cargo.lock`, `.cargo/**`, `tools/**`, `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml` (component `rust`) | owner | yes (positions are the base of everything else) | `cargo test --workspace --locked` | the later spike slices |
| `spike/render-trial/**`, `spike/render-web/**` (component `render`; the browser trial builds the native trial's `src/scene.rs` in, and each builds `crates/everink-core` in, which reads the root `Cargo.toml`, and `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml` and `.cargo/**` apply to it too) | owner | no (a trial; nothing depends on it) | `cargo run --release --locked --manifest-path spike/render-trial/Cargo.toml` (it checks itself; no tests); for the browser trial, `sh spike/render-web/build.sh` and its page opened in a browser (the page checks itself) | the choice of renderer and of outline form |
| `spike/controls-trial/**`, `spike/controls-window/**`, `spike/controls-web/**` (component `controls`; the window and the page build the trial in, and the page builds two of the window's files in; `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml` and `.cargo/**` apply to it too) | owner | no (a trial; nothing depends on it) | `cargo test --locked --manifest-path spike/controls-trial/Cargo.toml`; for the window, its `--shot` pictures and the checks in `spike/controls-window/README.md`, which are a person's; for the page, `node spike/controls-web/check.mjs own` and `egui` (Chrome and a GPU) | the choice of controls toolkit (owner-reserved) |
| `spike/store-trial/**` (component `store`; it builds `crates/everink-core` and `crates/everink-sync` in, and `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml` and `.cargo/**` apply to it too) | owner | no (a trial; nothing depends on it) | `cargo test --locked --manifest-path spike/store-trial/Cargo.toml` | the choice of storage engine |
| `spike/text-trial/**` (component `text`; `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml` and `.cargo/**` apply to it too) | owner | no (a trial; nothing depends on it) | `cargo test --locked --manifest-path spike/text-trial/Cargo.toml` | the owner's choice of a text library |
| `AGENTS.md`, `CLAUDE.md`, `PROFILE.md`, `.agent-rules/**`, `.claude/**`, `.githooks/**`, `.github/**` | owner | yes (agent policy, R9) | `policy-drift` | agents working here |

## Check selection (impact-based; core D2)
The `ci` workflow's first job, `changes`, runs `.github/scripts/ci-changes.sh`, which holds the mapping below in
this order. On the hourly schedule the same job runs `.github/scripts/ci-lane.py`, which decides whether the branch
head still needs a full run. A path that matches no row, or a file list that is empty or cannot be computed, selects every job.

| Changed paths | Checks that run on the PR | Evidence recorded as |
|---|---|---|
| `README.md`, `docs/**`, `LICENSE-MIT`, `LICENSE-APACHE`, `.gitignore` | `pr-evidence`; `changes` and `ci-result`; doc checks: none configured | `docs: n/a: none configured`; `lint` and `test`: `n/a: not selected (D2)` |
| `AGENTS.md`, `CLAUDE.md`, `PROFILE.md`, `.claude/**`, `.github/**`, `.agent-rules/**`, `.githooks/**` | `policy-drift` + `pr-evidence`; `changes` and `ci-result`; these paths select no product job (core 0.1.22) | per check (CI); `lint` and `test`: `n/a: not selected (D2)` |
| `crates/everink-core/**`, the root `Cargo.toml` | `lint`, `test` (as in the next row), `render-trial` and `store-trial` (the trials build the crate in, and the crate takes its dependencies, edition and lints from the root manifest) | per check (CI), with the test count |
| `crates/everink-sync/**` | `lint`, `test` (as in the next row) and `store-trial` (the trial builds the crate in) | per check (CI), with the test count |
| the rest of `crates/**`, `Cargo.lock`, `tools/**` | `lint` (`cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`) and `test` (`cargo test --workspace --locked`, the core's tests on wasm32, the two golden files again with fused multiply-add switched on and in optimised builds, then the timing examples and the paced-link run) | per check (CI), with the test count |
| `spike/render-trial/**`, `spike/render-web/**` | `render-trial` (for the native trial: format check, clippy, and a run at 2,000 strokes with the processor renderer; a hosted runner has no GPU, so the comparison of the two renderers does not run there. For the browser trial: format check, clippy on wasm32 for each renderer alone and all three, and release builds without and with WebAssembly SIMD; it is not run there) | per check (CI) |
| `spike/controls-trial/**`, `spike/controls-window/**`, `spike/controls-web/**` | `controls-trial` (for the window, and for the page's module on wasm32: format check and clippy; neither is run there. For the trial: format check, clippy for each set of candidates except `system-fonts`, which needs the fontconfig library and is not checked, tests, the frame-cost run, which asserts nothing about time, and the wasm size builds) | per check (CI), with the test count |
| `spike/store-trial/**` | `store-trial` (format check, clippy, tests, and the timing run at 10,000 and 100,000 strokes, which asserts nothing about time, all with `--manifest-path spike/store-trial/Cargo.toml`) | per check (CI), with the test count |
| `spike/text-trial/**` | `text-trial` (format check, clippy, tests and the size and timing run, all with `--manifest-path spike/text-trial/Cargo.toml`) | per check (CI), with the test count |
| `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml`, `.cargo/**` (read by all; the tools look in parent directories, and cargo reads `.cargo/` from where it is run) | `lint`, `test`, `text-trial`, `render-trial`, `store-trial` and `controls-trial` | per check (CI) |
| Any other path, or an uncertain mapping | every job | per check (CI) |

Escalation triggers: `lint`, `test`, `text-trial`, `render-trial`, `store-trial` and `controls-trial` together are the full suite; `workflow_dispatch` and the
scheduled lane run every job.

## Lanes
| Lane | What runs | When | Owner of a failure |
|---|---|---|---|
| PR (fast) | `pr-evidence` (every PR), `policy-drift` (policy paths), and `ci` (`changes`, the selected of `lint`, `test`, `text-trial`, `render-trial`, `store-trial` and `controls-trial`, `ci-result`) on GitHub-hosted `ubuntu-24.04` (D9 public mode; no placement) | every PR commit | author |
| Push to `main` | `ci` with the same selection over the pushed commits; it re-runs the jobs and does not carry the pull request's result | every merge | whoever merged (core D5) |
| Integration batch | (Not keeping its hour: the schedule reached `main` on 2026-11-12 at 07:27 UTC and has fired once since, at 16:54 UTC, run 43263766909, every job passed on `6f28f05`; the owner's answer that day was to leave it for now. A head of `main` may therefore wait much longer than the five hours below for a full run; the push run after each merge still runs the selected jobs.) `ci` on the hourly schedule (minute 23): every job on the head of `main`, unless a scheduled or dispatched run that executed its jobs already covers that head (then it runs nothing and passes, or carries that run's failure), or the head moved and the last covering run succeeded less than 4 hours ago (then it waits). Any error reading earlier runs means it runs. The decision table is `.github/scripts/ci-lane.py`, whose self-test runs in every `changes` job. A head is covered within about 5 hours of a merge. A run that executed nothing is not evidence | hourly; a full run only when `main` moved | owner; the agent checks at the start of a session that the newest commit on `main` is not more than about 8 hours ahead of the last covering run |
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
| performance | a change to `crates/everink-core` arithmetic, to the `crates/everink-sync` object map or to its pacing rule that a timing example covers; figures and the machine they were measured on are in `docs/spike/` |
| observability | n/a (no runtime) |

## Merge authority
- All tiers: agent merges with `.agent-rules/merge.sh <n>` once core R3–R6 hold and the review verdict for the head is recorded (D5).
  No owner acceptance step (owner decision 2026-11-01). Emergency: declared only by Von Chance (vchance); the PR
  names the declarer; follow-up PR within 7 days.
- Whoever merges follows the default branch's push run of `ci` to its end and owns a failure there (core D5, 0.1.23).
- Flakes (core D2, 0.1.23): the tests are meant to be deterministic (fixed seeds, no clock, no network). One flake so far: `without_keeping_writes_during_the_fill_a_late_joiner_loses_them` (`everink-sync`) depended on the order of a hash map and failed once locally on 2026-11-12; fixed the same day by sending the objects in a fixed order. Any flake is tallied in issue #15, `Flaky tests` (https://github.com/vchance/everink/issues/15, opened for this one), and a repeat within 14 days is fixed or quarantined by a profile change. A test must not let a hash map's order decide its outcome.
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
| Dependency cadence | none set yet: two dependencies in the workspace (and two more used by tests or one example), pinned by `Cargo.lock`, and the trial's three candidates pinned exactly in `spike/text-trial/Cargo.toml` and its lock file; advisory checks (`cargo-deny`, `cargo-audit`) are planned in `docs/design/08-identity-and-security.md` and not yet configured |
| Deprecation policy | n/a (no public API) |

## Cleanup (core R10)
| Resource this repo's work creates | Where | Teardown command | Safe to remove when |
|---|---|---|---|
| Build output | `target/`, `spike/text-trial/target/`, `spike/render-trial/target/`, `spike/render-web/target/`, `spike/store-trial/target/` (its timing run's files are under it, in `data/`), `spike/controls-trial/target/`, `spike/controls-window/target/`, `spike/controls-web/target/` and `spike/controls-web/www/pkg` and what `spike/render-web/build.sh` writes (`spike/render-web/www/pkg*`, `www/size-*`) in the checkout or worktree (ignored by git) | `cargo clean`, and the same with `--manifest-path` for each trial; `rm -r spike/render-web/target spike/render-web/www/pkg* spike/render-web/www/size-*` | any time; it is rebuilt on demand |
| Worktrees | `.claude/worktrees/*` | `git worktree remove <path> && git worktree prune` | branch merged, `git status` clean |
| CI runner artifacts | none: no job writes artifacts or caches (core D11); each run builds from scratch on a runner that is thrown away | — | — |
| Cloud / paid | none: GitHub-hosted minutes are free for a public repository | — | — |

## Budgets
- CI: `pr-evidence` and `policy-drift` under a minute each; `ci` a few minutes (no build cache, by core D11) — to measure on the first runs.
- Performance: a full redraw of a 4K view of 10,000 strokes takes 61 to 400 ms of processor time on an Apple M4 depending on the outline's form (more with the GPU renderer), against the design's reading that 60 ms is acceptable and 400 ms is not (`docs/spike/results-3b-renderer.md`). In a browser (Chrome on the same machine) the same redraw takes 80 to 540 ms with WebAssembly SIMD and 185 to 1,340 ms without, the same on WebGPU, WebGL2 and the processor renderer; a view composed from cached tiles holds 60 frames a second with a median of under a millisecond of the page's thread a frame (`docs/spike/results-3c-browser.md`). The cut has the design's budget of 2 ms a frame and does not meet it for a single curve from about 300 levels above the view (0.37 ms at 100 levels, 11 ms at 1,000, Apple M4; three to four times that on wasm32; `docs/spike/results-3a-wasm-outlines-cut.md`). Storage: a durable commit of the room's log takes about 4 ms on the same machine's SSD (inside the design's 1 to 5 ms) and a room opens from its file in 0.29 s at 100,000 strokes and 4 to 6 s at 1,000,000 (`docs/spike/results-5-storage.md`). One measured budget so far, per-frame position work for 10,000 objects under 0.05 ms at any depth (measured 0.004 ms on an Apple M4, native; `docs/spike/results-1-positions.md`). A re-base is not yet inside a per-frame budget at depth: 1.3 ms at 100 levels on that machine, more on slower ones. The object map has no budget yet; its first figures (about 1.3 KB of memory per stroke, 0.14 s to open 100,000 strokes, server work per change, at 100,000 objects, under a microsecond at depth 0 and 1 to 3 microseconds at 100 to 1,000 levels, same machine) are in `docs/spike/results-2a-object-map.md`. On wasm32 only the cut and the outline are measured; slice 1's re-base and frame figures are not. CI machines are not a benchmark reference.
