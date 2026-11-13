# controls-web

The two controls candidates of `spike/controls-trial` on a canvas in a browser (spike slice 4b, web part), with
the DOM mirror and the hidden textarea that section 07 of the design describes. The results are in
`docs/spike/results-4b-web.md`.

## Build, check, look

```sh
cargo install wasm-bindgen-cli --version 0.2.130 --locked   # once; the version Cargo.toml pins
sh spike/controls-web/build.sh                              # builds the module into www/pkg
node spike/controls-web/check.mjs own                       # drives the page in Chrome and checks it
node spike/controls-web/check.mjs egui /tmp/egui.png        # the same for egui, and a picture
python3 -m http.server 8766 --bind 127.0.0.1 --directory spike/controls-web/www   # to look yourself
```

Then `http://127.0.0.1:8766/?candidate=own` or `?candidate=egui`, in a browser with WebGPU.

`check.mjs` opens Chrome in a window of its own with an empty profile, which it deletes. Through Chrome's
debugging protocol it sends keys and text by the browser's own input path, runs an input method's composition
and reads the accessibility tree; it exits with 1 if a check fails. No person presses anything. Its `result:` lines are measurements: the candidates differ there.

## What a person should still try

`check.mjs` reads the tree Chrome gives assistive technology. It is not a screen reader. With VoiceOver
(Command-F5) and Safari or Chrome, or NVDA and Chrome or Firefox:

1. Does the screen reader find a toolbar named Tools, and read each tool and colour as a radio button with its
   name and whether it is checked?
2. Does activating a tool from the screen reader (VoiceOver: Control-Option-Space) change the tool? The page
   prints `SetTool(...)` under the toolbar.
3. Is the hex field read as an edit field named "Hex colour" with its value, and are typed characters echoed?
   (It is a textarea, so a reader may call it multi-line.) Is the zoom figure read when you move to it, and
   not after every key?
4. With a touch screen reader (VoiceOver on iOS, TalkBack): can the controls be found by touch? The mirror's
   elements lie over the canvas and take no pointer; whether explore-by-touch reaches them is not known.
5. Press Escape in the controls: the focus goes to nothing until the next Tab. Is that what you expect?
6. In Safari and Firefox: does the page come up at all (it draws with WebGPU), and do a dead key (Option-E then
   E on a Mac) and a Japanese or Chinese input method work in the field?

## What is here

- `src/lib.rs`: the module. It holds a candidate, draws it on the canvas each frame through `wgpu`'s WebGPU
  backend, and gives the page the accessibility tree and the text field's state as JSON. The drawing
  (`paint.rs`) and the candidate and toy document (`shell.rs`) are `spike/controls-window`'s own files.
- `www/index.html`: the web shell, in the page so that it can be read. The mirror: one element for each
  control, with its ARIA role, name and state, placed where the control is drawn, not seen and taking no
  pointer. The text field's element is a real textarea. The DOM's focus follows the controls' focus, and the
  shell lets it out past either end.
- `check.mjs`: the checks. `build.sh`: the build; `www/pkg` is its output and is not kept in git.

CI type-checks and lints the module for wasm32. It cannot run the page: there is no browser with a GPU there.
