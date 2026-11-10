# Landscape

Researched 2026-11-10. No existing project is infinite, multi-user, native and web together.

## Complete apps and SDKs

| Project | Infinite | Multi-user | Native | Web/wasm | Notes |
|---|---|---|---|---|---|
| Drawpile (C++/Qt) | Probably not (docs mention sessions running out of space) | Yes, its core purpose | Yes | Yes, whole app as wasm | Raster painting. A full app, not a library. |
| tldraw (TypeScript) | Yes | Yes (tldraw sync) | No | Yes | Best whiteboard SDK. Own license; commercial use needs a paid key. |
| Excalidraw (TypeScript, MIT) | Yes | Yes (collab server) | No | Yes | Hand-drawn style. Easy to embed. |
| BlockSuite / AFFiNE (TypeScript) | Yes ("edgeless" mode) | Yes (Yjs) | Electron only | Yes | Canvas plus documents. |
| Graphite (Rust) | Yes | Not yet | Yes | Yes (wasm) | Vector and node-based editor. Proves the Rust + wasm + GPU route. |

## Building blocks that work natively and in wasm

### Sync (CRDTs)

- **Loro**: Rust core with a wasm build. Fast; handles movable trees and reordering well.
- **Yjs / yrs**: Yjs in JavaScript, yrs in Rust with C bindings. Largest ecosystem, including servers such
  as Hocuspocus, y-sweet and Liveblocks.
- **Automerge**: Rust core with wasm and C bindings.

### Rendering

- **Vello**: Rust GPU 2D renderer on wgpu, so native and WebGPU.
- **Skia**: C++, with CanvasKit as its wasm build.
- **ThorVG**, **Blend2D**, **Lyon** (path tessellation).

### Ink

- **google/ink-stroke-modeler**: C++17 library that smooths stylus input and predicts its motion to cut
  latency. Depends only on the standard library and Abseil.
- **google/ink**: Apache-2.0 cross-platform C++ core behind Android's Jetpack Ink API.
- **perfect-freehand**: pressure-style stroke outlines, ported to several languages.

### Infinite-canvas basics

- R-tree or quadtree spatial index (Rust `rstar`, Boost.Geometry) to find what is on screen.
- Tiled or level-of-detail rendering for zoom.
- 64-bit world coordinates or per-tile origins to avoid float precision loss.

## Sources

- [Drawpile in the Browser](https://drawpile.net/news/browser-enabled/)
- [Drawpile dev update, week 51 of 2023](https://docs.drawpile.net/devblog/2023/12/23/dev-update.html)
- [Drawpile dev update, week 10 of 2024](https://docs.drawpile.net/devblog/2024/03/09/dev-update.html)
- [Drawpile download](https://drawpile.net/download)
- [Ink Stroke Modeler](https://android-git.linaro.org/platform/external/ink-stroke-modeler.git/about/)
- [Introducing the Ink API Jetpack library](https://android-developers.googleblog.com/2024/10/introducing-ink-api-jetpack-library.html)
- [Chromium third_party/ink README](https://chromium.googlesource.com/chromium/src/+/b99d73dcb0f3e8a25893a502f654993282d6e2ca/third_party/ink/README.chromium)
- [miskeenity canvas (WebGPU, wasm)](https://github.com/forkrp/miskeenity-canvas)

Details on tldraw, Excalidraw, BlockSuite, Graphite, Loro, Yjs, Automerge, Vello and Skia come from general
knowledge as of 2026 and were not re-verified for this note.
