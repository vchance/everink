# render-web

The renderer trial in a browser (spike slice 3c, question 10). It draws the scene of `spike/render-trial` with
`vello_gpu` on WebGPU, with `vello_gpu` on WebGL2 and with `vello_cpu`, keeps the tiles as textures, and composes
the view from them on every frame while it pans. The results are in `docs/spike/results-3c-browser.md`.

## Build and run

```sh
cargo install wasm-bindgen-cli --version 0.2.130 --locked   # once; the version Cargo.toml pins
sh spike/render-web/build.sh                                # builds into www/ and prints the module sizes
python3 spike/render-web/serve.py                           # serves www/ at http://127.0.0.1:8765/
```

Then open `http://127.0.0.1:8765/` in a browser, with its window in front and not covered. The page runs by itself
and writes what it measured; its title ends in `done` or `failed`. Options, all in the address:

- `strokes=10000`: how many strokes the scene has.
- `run=cpu,webgl,webgpu`: which renderers to run.
- `frames=300`: how many frames the pan test draws. `pan=no` leaves it out.
- `pkg=pkg-simd`: the build with WebAssembly SIMD. The default, `pkg`, is without.

## What is here

- `src/lib.rs`: the three runs, the pan frame for WebGPU and for WebGL2, and the comparison of what the renderers
  drew. The scene comes from `../render-trial/src/scene.rs`, so both trials draw the same strokes.
- `www/index.html`: the page. It has no dependencies.
- `build.sh`: the builds. `www/pkg*` and `www/size-*` are its output and are not kept in git.
- `serve.py`: a static server for `www/` that sends the two headers that make the page cross-origin isolated.
  Chrome rounds an ordinary page's clock to 0.1 ms; an isolated page's is fine enough for the smaller figures.
  The page prints which clock it has.

It has no tests. The page checks itself: every pair of renderers must agree on the busiest tile pixel by pixel, in colour
and coverage, and so must the canvas composed from the cached tiles. CI builds and lints it and does not run it: a hosted runner has
neither a GPU nor a browser to drive.
