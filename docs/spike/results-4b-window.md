# Spike slice 4, part b (native): the two controls toolkits in a window

Date: 2026-11-12. Code: `spike/controls-window`, and small changes to `spike/controls-trial`. This is the second
part of question 12 of [`README.md`](README.md): **which controls toolkit?** Part 4a
([`results-4a-controls.md`](results-4a-controls.md)) compared the two candidates without a window. This part
puts each in a native window, so that the checks only a person can make become possible. The browser, with the
DOM mirror, is a further part and is not here. The choice is the owner's; nothing here makes it.

## What an agent could and could not check

| | Checked here | How |
|---|---|---|
| Both candidates draw the same toolbar on a GPU surface | Yes | A frame of each rendered to a picture by the same painters the window uses (below) |
| Each window opens and runs | Yes | Opened for six seconds each on an Apple M4, macOS; neither failed |
| The platform is handed the accessibility tree | Built, not observed | `accesskit_winit` 0.34.2 gets every frame's tree; nothing here reads it back from macOS |
| A screen reader reads it rightly | No | The owner's, with the script in `spike/controls-window/README.md` |
| A real keyboard and dead keys | No | The same: the shell maps winit's events as winit documents them, and no key was pressed |
| An input method's window at the caret | No | The shell calls `set_ime_cursor_area` with the caret each frame; not seen |
| Windows and Linux | No | Built on macOS only; CI builds it on Linux |

So this part's result is a tool and its first pictures, not a verdict. The script has fourteen steps for each
candidate.

## The pictures

Each after the same use: Eraser and Green chosen, the field focused, two characters deleted, `A` typed, and a
dead-key accent being composed. 1600 by 112 pixels (800 by 56 at a scale of 2).

The own toolkit (`vello_gpu`, text by `parley`):

![The own toolkit's toolbar](images/4b-own.png)

egui 0.37 (`egui-wgpu`), light theme:

![egui's toolbar](images/4b-egui.png)

What they show:

- **Both are the toolbar the trial specifies**, with the chosen tool and swatch marked, Redo dimmed, and the
  accent in the field.
- **egui draws more of a text field**: the text being composed is underlined. The own toolkit's is not: its
  shell lays the field's text out again plainly, and the underline Parley's editor asks for is lost on the way.
  While composing it draws no caret.
- **The own toolkit's focus ring is its own** (blue, outside the field); egui's field shows focus by its
  border.
- **egui's tools are radio buttons with a dot; the own toolkit's are buttons with a filled face.** Neither is
  the design's look; there is none yet.
- **Same type in both**: Ubuntu Light, from the fonts egui ships, shaped by the same shaper.

Three faults in the own toolkit were found by looking and fixed before these pictures: its row was wider than
the surface, so the field ran off the right edge; its caret went to the field's edge while composing (Parley's
editor hides its caret then, and the fallback was wrong); and egui came up in the machine's dark theme, in
which the black swatch cannot be seen. None of the three was caught by part 4a's tests, which read the
accessibility tree and not the pixels.

## What was built

- **`spike/controls-window`**: a package of its own, outside the workspace, with its own lock file. One winit
  0.30.13 window with one `wgpu` 30.0.2 surface. winit's events are turned into the trial's `InputEvent`s, the
  same code for both candidates: pointer, keys, and the platform's text input (`Ime::Preedit` and `Commit`
  become section 07's `Preedit` and `Commit`). The text input is allowed only while the field has the focus,
  and is told the caret's place each frame, as section 07 has it. The tree each candidate gives goes to
  `accesskit_winit`; requests come back as `InputEvent::Action`.
- **Drawing**: the own toolkit's paint list through `vello_gpu` 0.4.0, its text laid out by `parley` and drawn
  as glyph runs; egui's triangles through `egui-wgpu` 0.37.0. One `wgpu` serves both.
- **In `spike/controls-trial`**: the egui adapter can keep each frame's triangles and texture changes for a
  shell, and takes the display's scale; the own toolkit's row is narrower and its caret is right while
  composing; egui is set to its light theme.

## Findings

1. **One shell serves both candidates.** Neither `eframe` nor `egui-winit` is used: egui is driven through the
   trial's own events, as section 07 intends for whichever toolkit wins. The shell is about 860 lines for both, a third of it the mode that writes a picture.
2. **The accessibility tree needs the display's scale from the shell for the own toolkit**; egui puts it on its
   root node itself. A detail for section 07's `AccessibilityHost`.
3. **The surface must not be an sRGB format** for either painter as used here: neither converts on the way out.
4. **Tests on the accessibility tree do not see layout faults.** Three were found only by rendering. A product
   needs picture tests of the controls as well as tree tests; section 07's test list has the second and
   should have the first.
5. **`accesskit_winit` is Apache-2.0 only**, as section 07 notes, and so are `winit` and a few others here: a
   native binary is distributed under Apache-2.0 terms, which the project's licence (MIT OR Apache-2.0) allows.

## What this part does not show

Everything in the right-hand "No" rows above, and:

- **The browser**: the DOM mirror, the hidden textarea, egui's web text agent. Not built.
- **Contrast and target size by measure.** Part 4a measured sizes from the tree (egui's buttons are 20 high);
  contrast is not measured.
- **Resizing, a second display scale, dark theme, high contrast.** The window is one size and not resizable.
- **Frame cost with drawing.** Not measured; part 4a's figures are without it.

## Where the code departs from the design

- No `Shell` or `AccessibilityHost` trait: one program, written out.
- The own toolkit's text is laid out twice, once by the toolkit to measure and once by the shell to draw.
- The shell applies commands to a toy document (a tool, a colour, and an undo list) so that Undo and Redo do
  something.

## What the design should change

With part 4a's corrections, in a separate change:

1. Section 07, tests: add picture tests of the controls beside the tree tests.
2. Section 07, accessibility: say who applies the display's scale to the tree.

## Kept afterwards

`spike/controls-window` stays with `spike/controls-trial` until the owner chooses, and is what the owner's
checks are run with.
