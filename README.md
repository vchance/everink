# Everink

An infinite, multi-user drawing surface that runs the same core natively and in the browser (WebAssembly).

Status: design done, spike under way. The first code is `crates/everink-core` (positions, the camera, stroke outlines and the cut) and
`crates/everink-sync` (the object map and the paced connection), with a trial of text libraries in `spike/text-trial`; nothing is released.

## Why

The parts exist: CRDT sync libraries, GPU 2D renderers and stroke modelers all build for both native and wasm.
What nobody ships is the glue: a canvas document model that merges cleanly, sync that only moves the part of an
endless canvas you are looking at, live presence, and one core behind thin native and web shells.
See [docs/vision.md](docs/vision.md).

## Docs

- [docs/vision.md](docs/vision.md): goals, non-goals, the gap Everink fills, candidate stacks, open questions.
- [docs/design.md](docs/design.md): the high-level design: objects, views, sync, presence, server.
- [docs/design/](docs/design/README.md): the detailed design in eight sections, with the questions still open.
- [docs/spike/](docs/spike/README.md): the spike: its questions, the slices, and measured results.
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

## License

Licensed under either of the [Apache License, Version 2.0](LICENSE-APACHE) or the [MIT license](LICENSE-MIT), at
your option.

## Contributing

Everink is not accepting outside contributions yet: pull requests from people who are not collaborators will not
be reviewed or merged. This may change later; it will be announced here.
