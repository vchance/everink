# Everink

An infinite, multi-user drawing surface that runs the same core natively and in the browser (WebAssembly).

Status: idea and research. No code yet.

## Why

The parts exist: CRDT sync libraries, GPU 2D renderers and stroke modelers all build for both native and wasm.
What nobody ships is the glue: a canvas document model that merges cleanly, sync that only moves the part of an
endless canvas you are looking at, live presence, and one core behind thin native and web shells.
See [docs/vision.md](docs/vision.md).

## Docs

- [docs/vision.md](docs/vision.md): goals, non-goals, the gap Everink fills, candidate stacks, open questions.
- [docs/landscape.md](docs/landscape.md): existing apps, SDKs and libraries, and what each covers.
- [docs/naming.md](docs/naming.md): why "Everink", and the names that were checked.

## Planned packages

| Package | Role |
|---|---|
| `everink-core` | Document model, spatial index, stroke geometry. No I/O. |
| `everink-sync` | CRDT binding, viewport-scoped sync, presence. |
| `everink-native` | Desktop shell. |
| `everink-web` | Browser shell (wasm + WebGPU). |

Names are reserved in intent only; nothing is published.
