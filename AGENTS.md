# Agent instructions — everink

<!-- agent-rules manifest (checked by policy-drift; keep these lines exact) -->
policy: agent-rules/core@0.1.x   ·   profile: PROFILE.md   ·   core-digest: c02ad3091b63

Read, in this order: (1) `.agent-rules/core.md` — the shared standard this repository follows; (2) `PROFILE.md`; (3) the sections below. Then classify the change (core R2) before editing anything.

## This repo
Everink: an infinite, multi-user drawing surface with one core built natively and to WebAssembly. It is at the
idea and research stage: the repository holds only documents (`README.md`, `docs/`), no code, build or tests yet.
`docs/vision.md` lists the open decisions (core language, CRDT library, license); they are the owner's to make.

## Commands
```sh
# build: none yet (no code)
# all tests: none yet
# one test: none yet
# lint: none configured
# format check: none configured
git config core.hooksPath .githooks   # once per clone: enables the pre-push guard (core R3)
```
When code arrives, the PR that adds it also adds the build, test, lint and format commands here and a matching
`Check selection` row in `PROFILE.md`.

## Layout
- `README.md`: what Everink is and the planned packages (`everink-core`, `everink-sync`, `everink-native`, `everink-web`).
- `docs/vision.md`: goals, non-goals, architecture sketch, candidate stacks, open questions, next steps.
- `docs/landscape.md`: existing projects and libraries, with sources; says which claims were not re-verified.
- `docs/naming.md`: why the name, and the availability check.

## Gotchas
- The package names in `README.md` are intentions only. Nothing is published on crates.io or npm; publishing is an
  owner decision.
- `docs/landscape.md` is dated research. Re-check a claim before relying on it, and update the date when you do.

## Repository additions
- none.

## Overrides
- none.

## Owner decisions
<!-- Ask, don't decide: releases, publishing, dependency majors, schema changes -->
None recorded yet. Ask, do not decide: the core language (Rust or C++), the CRDT library, the license, making the
repository public, publishing or reserving package names, and registering domains.
