# controls-window

The two controls candidates of `spike/controls-trial` in a native window (spike slice 4b, native part), so that a
person can try each with a screen reader and a real keyboard. What an agent could check is in
`docs/spike/results-4b-window.md`; what is below is the part only a person can do.

## Run

```sh
cargo run --release --locked --manifest-path spike/controls-window/Cargo.toml -- own
cargo run --release --locked --manifest-path spike/controls-window/Cargo.toml -- egui
cargo run --release --locked --manifest-path spike/controls-window/Cargo.toml -- own --shot /tmp/own.png   # a picture, no window
cargo run --release --locked --manifest-path spike/controls-window/Cargo.toml -- own --shot /tmp/own.png --focus Pen   # with the focus on a tool or a colour
```

The window is the toolbar and nothing else: five tools, six colours, Undo and Redo, the zoom figure, and a field
that holds the colour as six hex digits. The terminal prints every command the controls give, every request from
assistive technology, the text of every plain key (`key text:`), and what the platform's text input sends
(`text input:`).

## The checks, for each candidate

Write down what happens; a "no" is a result. The same script for `own` and for `egui`.

**Keyboard** (no screen reader)

1. Press Tab from a fresh window. Where does the focus go, and is there a ring you can see?
2. Keep pressing Tab to the hex field. How many presses? Does Shift-Tab go back?
3. With a tool focused, press the right and left arrows. Does the tool change (the terminal prints `SetTool`)?
4. Tab to Redo after changing something and undoing it; press Space. Does it print `Redo`?
5. In the field: clear it with Backspace, type `00FF00`, then Enter. Does it print `SetColour([0, 255, 0])`?
   (Do not hold Command or Control while typing: the shell follows only Shift.)

**Text composition** (macOS: the U.S. or ABC keyboard)

6. In the field, clear it, then press Option-E and then E. An accent should appear while composing and become
   `é`. The terminal should print `text input: composing "´"` and then `text input: commit "é"`; winit also
   sends an empty `composing ""` just before a commit, which is not a fault. Is the `é` in the field once? If
   a `key text:` line appears as well as the commit, the character came twice: write that down.
7. Press Option-E and then Escape or a click elsewhere. Is the accent gone, and the field as it was?
8. If you have a Japanese or Chinese input source: switch to it and type a syllable. Does the candidate window
   appear at the field, and does the committed text go in?

**Screen reader** (macOS: VoiceOver, Command-F5)

9. Move to the window with VoiceOver on. Does it say there is a toolbar named Tools?
10. Move through it (Control-Option and the arrows). For each tool and colour: is the name read, is it read as
    a radio button, and is "selected" read for the chosen one? Are the groups (Tool, Colour) announced?
11. Is Undo read as dimmed or unavailable when there is nothing to undo?
12. Is the zoom figure read?
13. On the field: is it read as an edit field named "Hex colour" with its value? Type a digit: is it echoed?
14. Activate a tool with VoiceOver (Control-Option-Space). Does the terminal print the request and the command?

Windows (NVDA or Narrator) and Linux (Orca) take the same script; the shell was only built and opened on macOS.

## What is here

- `src/main.rs`: the window. winit's events become the trial's `InputEvent`s, the same for both candidates;
  `accesskit_winit` gets the tree each gives; the platform's text input is switched on while the field has the
  focus and told where the caret is.
- `src/paint.rs`: drawing. The own toolkit's paint list goes through `vello_gpu`, its text shaped by `parley`;
  egui's triangles go through `egui-wgpu`. Both on one `wgpu` surface.

It has no tests. CI type-checks and lints it; it cannot open a window or reach a GPU there.
