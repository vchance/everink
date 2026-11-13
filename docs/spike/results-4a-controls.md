# Spike slice 4, part a: the two controls toolkits, without a window

Date: 2026-11-12. Code: `spike/controls-trial`. This is the first part of question 12 of
[`README.md`](README.md): **which controls toolkit?** The design (section 07) has the same toolbar and the same
single-line text field built twice, with a small toolkit of Everink's own and with egui, and the choice made on
the result. The choice is the owner's; nothing here makes it.

Slice 4 lands in three parts. This one, 4a, is what can be measured without a window: both candidates behind one
trait, the accessibility tree each gives, text composition through the field, what a frame costs, how large
each makes a WebAssembly module, and how much code each took. Part 4b puts both in a window, natively and in a
browser, for the checks only a person can make: a screen reader, a real keyboard and input method. Part 4c is the
owner's further candidate, whose details are kept outside this repository.

## Answer so far

Both candidates do everything the trial asks of them: the same twelve tests pass for both. They differ in what
they cost and in what they leave to be written.

| | Everink's own (`taffy`, `parley`, AccessKit by hand) | egui 0.37.0 |
|---|---|---|
| The accessibility tree a screen reader needs | Yes | Yes |
| Text composition through the field (dead key, cancel, an input method in steps) | Yes | Yes |
| A frame with nothing changed | 1 µs | 18 µs |
| A frame with something changed | 7 to 9 µs | 19 to 21 µs |
| Added to a WebAssembly module (raw; Brotli), no font | 1.24 MB; 366 KB | 2.42 MB; 616 KB |
| The same with a font | 1.64 MB; 513 KB (one font) | 3.09 MB; 886 KB (the three it ships) |
| Lines of code for this toolbar and field | 669 | 232 |
| Smallest control, in pixels | 24 by 24 | 24 wide, 20 high (its default button height) |
| Presses of Tab from the start to the field | 4 (one stop a group) | 13 (every button) |
| Commands come out | With the event | One frame later |
| Text stack | Parley, the canvas's own | Its own layout and line breaking, over the same shaper and font reader Parley uses (`harfrust`, `skrifa`) |

- **Either is far inside the design's planning figure of 1 ms a frame**, by about 45 times or more. Frame cost does
  not separate them.
- **egui costs about twice the bytes**, with or without fonts; the design's whole budget for the first version
  is 2.5 MB with Brotli, fonts excluded.
- **The own toolkit costs about three times the code** for this much, and this much is little: see "What each
  still lacks".
- **egui stops Tab at every radio button**; the design asks for one focus stop a group with arrows inside it
  (WCAG 2.4.3 as the WAI-ARIA practices read it). The own toolkit does that because it was written to.
- **Neither works without a font it is given.** egui draws no text without one, and the own toolkit's field
  cannot be edited: Parley's editor needs glyphs to move the caret over. Neither uses the machine's fonts here,
  as the design asks.
- **egui keeps the text being composed inside the application's string.** It worked in every test, but the
  application cannot tell the value from the preedit without keeping track itself.

## What was built

`spike/controls-trial` is a package of its own, outside the workspace, with its own lock file.

- **The trait and the model** (`src/lib.rs`): a cut-down form of section 07's `Controls`. The controls are
  shown a `UiModel` (tool, colour, whether undo and redo can be used, the zoom figure) every frame and hold no
  document state. Events go in (`InputEvent`: pointer, keys, text events with `Preedit`, `Commit` and
  `CompositionCancelled` as in section 07, and requests from assistive technology). Out come `Command`s, an
  AccessKit `TreeUpdate`, and, while the field has the focus, what a platform's text input needs: the value, the
  text being composed and where the caret is.
- **The toolbar**: a palette of five tools and six colour swatches, each a radio group; undo and redo; the zoom
  figure; and the hex field, six characters, which gives a colour on Enter.
- **Candidate 1** (`src/own.rs`): widgets placed by `taffy` 0.15.1, text measured by `parley` 0.12.0 and edited
  by its `PlainEditor`, a list of rectangles and text runs to draw, and an AccessKit 0.25.2 tree written by
  hand. `parley` is built without its `system` feature: no system fonts, as section 06 has it. Its one font is
  Ubuntu Light, borrowed from the fonts egui ships, so that both candidates set the same type. It lays out and paints again only when the model, the focus, the pressed control or the field's text
  changed.
- **Candidate 2** (`src/with_egui.rs`): egui 0.37.0 driven without `eframe`: `Context::run_ui` with events
  made by hand, its radio buttons, buttons, label and `TextEdit`, a painted swatch given a role and a name, and
  the AccessKit tree egui makes. Each frame is tessellated, as it would be before drawing.
- **The contract** (`src/contract.rs`, `tests/contract.rs`): the same five tests run on each candidate, driven
  through events and read back from the commands and the tree.

Nothing is drawn on a screen. Neither candidate is seen, only checked.

## What the tests hold both to

1. **The tree.** A toolbar named Tools; radio groups named Tool and Colour, with every button inside its group,
   named, checked or not as the model says, clickable and with bounds; Undo and Redo, disabled when the model
   says so; the zoom figure as text; a text field named "Hex colour" that can be focused and holds the colour.
   The tree follows a changed model, and showing a model asks for nothing.
2. **Pointer and assistive technology.** A click at the middle of a control's bounds and a `Click` request on
   its node give the same command. A disabled button gives none.
3. **The field.** Typing, the six-character limit, the caret keys and backspace, Enter giving the colour,
   text that is not a colour giving nothing, and `SetValue` from assistive technology.
4. **Composition.** A dead key's accent is shown as text being composed and is not in the value; a new frame
   and a colour changed under it (another person's edit) do not rewrite it; the letter completes it; a
   composition given up leaves nothing; an input method that composes in steps (a Japanese smoke test) ends with
   its text in the field; the caret reported for the input method's window is inside the field.
5. **Tab.** Tab reaches the field, stops at Redo and never stops on a disabled button. The stops are printed:
   they differ, and the difference is a result and not a failure.
6. **The keyboard.** Space presses the button that has the focus; Shift and Tab go back. What the arrow keys do
   in the tool group is printed for each, and so is the smallest control.

A thirteenth test is the own toolkit's alone and holds it to what section 07 asks: one stop a group, the arrows,
Home and End moving the check inside it and wrapping, Escape letting go of the focus, and every control at
least 24 by 24. egui is not held to it: in egui the right arrow moves the focus to the next radio button and
gives no command, and Tab stops at each.

In the composition test the text on show is read from each toolkit itself (egui's string, Parley's editor), so
a toolkit that ignored the preedit would fail.

## Measurements

Machine: Apple M4 (aarch64, macOS), native, release build, Rust 1.97.1. Three runs; a range is over them. Another
job was using the machine during them (load average about 11); an earlier three runs on a quieter machine, with
the system's fonts, gave the same figures to within a microsecond.

### A frame

`cargo run --release --locked --manifest-path spike/controls-trial/Cargo.toml --example bench_controls`.
2,000 frames each after 50 not counted; median, with the 95th percentile in brackets.

| A frame in which | Own | egui |
|---|---|---|
| nothing changed | 0.9 to 1.1 µs (1.0 to 1.2) | 17.8 to 18.2 µs (19 to 21) |
| the zoom figure changed | 7.9 to 8.8 µs (11 to 13) | 20.9 to 21.4 µs (22 to 23) |
| the pointer moved over the bar | 0.7 to 0.8 µs (0.8 to 0.9) | 18.6 to 18.7 µs (19) |
| a key was typed in the field | 7.3 to 7.6 µs (7.6 to 8.0) | 19.4 to 19.6 µs (20 to 21) |

- egui builds its interface again every frame, so its frames cost the same whatever happened; that includes
  laying its text out to glyphs and tessellating its shapes into triangles. The own toolkit's paint list
  carries its text as strings: shaping them to glyphs for drawing is the renderer's and is not in its figure
  (measuring them for layout is, the first time each is seen). The own toolkit's unchanged frame is the cost of writing the
  accessibility tree, and its changed frame adds a `taffy` layout and the paint list.
- Neither figure includes drawing. The own toolkit's list would go to the canvas's renderer; egui's triangles
  to `egui-wgpu`. Section 06 budgets 0.5 ms of processor and 0.3 ms of GPU a frame for the controls; the
  processor part measured here is 0.001 to 0.02 ms.
- The own toolkit does not react to the pointer moving (it has no hover state), which is why that row costs it
  nothing.

### Size in a WebAssembly module

`sh spike/controls-trial/size.sh` builds a small program (`examples/size_probe.rs`) that shows the toolbar,
clicks, types and composes, for `wasm32-unknown-unknown`, built for size (`opt-level = "s"`, link-time
optimisation, one codegen unit, `panic = "abort"`), with no candidate, with each with and without a font, and with both. No
further size tool was run.

| Probe | Bytes | gzip -9 | Brotli | Added over the baseline (raw; Brotli) |
|---|---|---|---|---|
| No candidate | 21,940 | 9,356 | 7,903 | |
| Own, no font | 1,265,889 | 472,895 | 374,075 | 1.24 MB; 366 KB |
| Own with one font (Ubuntu Light, 0.36 MB) | 1,659,289 | 653,035 | 520,646 | 1.64 MB; 513 KB |
| egui, no fonts | 2,439,814 | 842,430 | 623,452 | 2.42 MB; 616 KB |
| egui with its three fonts (0.68 MB) | 3,116,536 | 1,164,122 | 894,165 | 3.09 MB; 886 KB |
| Both, with fonts | 3,823,281 | 1,424,163 | 1,089,576 | |

- The two are not the same kind of thing. The own toolkit's bytes include `parley` and the Unicode data it
  brings, which the canvas needs for text objects whichever toolkit wins, and it draws nothing itself. egui's
  bytes are its own layout, widgets, text layout and tessellator; drawing it needs `egui-wgpu` as well, which
  is not in the figure. How much of the 1.24 MB is Parley was not measured apart.
- **They share more than the design supposed.** egui 0.37 shapes text with `harfrust` 0.12.0 and reads fonts
  with `skrifa` 0.44.0, the same libraries at the same versions as Parley. The last row shows it: the two
  together are 0.93 MB smaller than their sum. 0.36 MB of that is the one font both hold; the other 0.57 MB is
  code they share. So in a module that already has Parley, egui adds about 1.85 MB without fonts and not 2.42.
  egui also brings `vello_cpu` 0.2.0, another version than the renderer's 0.4.0.
- So the reading is: with the own toolkit the controls add `taffy` and their own code to a module that already
  has Parley; with egui they add egui less what it shares with Parley.
- egui draws no text without fonts. Its own are 0.68 MB raw (three files: two text fonts, Hack and Ubuntu Light, and a 3.5 KB set of icons). It
  can be given the document's fonts instead; that was not tried. The rows without a font are there to show
  the code alone: without one the own toolkit's field cannot be edited either.
- Section 07's budget for the whole first version is 8 MB raw and 2.5 MB with Brotli, fonts excluded; the
  renderer alone measured 0.36 to 0.52 MB raw (part 3c).

### Code and effort

| | Own | egui |
|---|---|---|
| Lines of code (not blank, not comments) | 669 | 232 |
| Shared by both: the trait and model, the contract | 123, 154 | |
| Time to first passing all tests | one sitting, after egui | one sitting, first |

Both were written by an agent in one session, egui first and the tests with it, so "effort" in person-days is
not a figure this trial can give. What it can say: egui passed four of the first five tests on its first build and the
fifth after a change to the test (the test had asked assistive technology to set the value of a field that did
not have the focus, which the field then overwrote from the model; it now focuses the field first, for both); the own toolkit passed all five on its first build, with the tests already
there to write against, and then failed two of them on a machine without system fonts until it was given a
font of its own.

## What each still lacks

Section 07's criteria ask for "the widget behaviors still missing". For this toolbar and field:

| | Own | egui |
|---|---|---|
| Hover and pressed looks | Pressed only | Yes |
| Selecting text in the field with the pointer or with Shift | No | Yes |
| Copy, cut and paste | No | Yes (through its platform output) |
| The caret and selection in the accessibility tree (text runs) | No: `parley` 0.12 has no AccessKit support, so they must be written | No: its `TextEdit` gives the value only |
| One focus stop a group, arrows inside | Yes (tested) | No: Tab stops at every button, and an arrow moves the focus without checking; would need writing against egui's focus handling |
| Controls at least 24 by 24 | Yes | Not at its default style (20 high) |
| A focus ring of its own | Yes | Yes |
| Scroll, popovers, menus, sliders, tooltips, a modal with a focus trap | None written | All there |
| A container's role and name (toolbar, radio group) | By hand | Yes, through `UiBuilder` |
| The same text layout as the canvas | Yes (Parley) | No (the same shaper underneath; its own layout and line breaking) |
| A name for a text field | By hand | Needed a low-level call (`accesskit_node_builder`) |

The own toolkit has what the trial needed and nothing else; every further widget is to be written. egui has
the widgets and lacks two things the design asks for by name: the canvas's text layout and the radio group's
keyboard pattern.

## Findings

1. **Both candidates can meet the accessibility tree the design needs.** egui 0.37 lets a container carry a
   role and a name, which the design did not know it could; the tree it gives passed the same checks as the
   one written by hand.
2. **Composition works in both, headless.** The design's worry about egui was its web text agent, which this
   part does not touch. What did show: egui inserts the preedit into the application's `String`. The
   application sees composed text as value unless it remembers what it sent, as the trial's adapter does, and
   that adapter's way of taking it out again is not exact.
3. **egui answers in its next frame.** An event is queued and its command comes out of the next `frame` call;
   the own toolkit answers in the call that takes the event. Whether that costs a frame on screen depends on
   whether the application runs the controls before the canvas in the same tick; it was not measured.
4. **Frame cost is not a reason to choose.** Size may be: egui is about twice the bytes (1.7 to 1.95 times,
   measured each alone), and about 1.85 MB more in a module that already has Parley.
5. **The own toolkit's cost is everything not yet written**, and the text field shows it: three of the
   behaviours a person expects of a field are missing after 669 lines.
6. **Parley's editor does nothing without a font.** With no font registered, the caret cannot move and typed
   text does not go in. The trial met this on CI, where there are no system fonts to fall back on. The controls
   must have their font before the first text field is shown; section 07 should say so.
7. **AccessKit has no role for a toggle button** in 0.25.2 and reports "checked" and "pressed" through one
   property; both candidates use `RadioButton` with it. `parley` 0.12 has no AccessKit support at all. Section
   07 assumes neither.

## What this part does not show

- **A screen reader.** The tree is checked by tests, not heard. VoiceOver, NVDA, Orca and the others are the
  owner's to run, and need a window: part 4b.
- **A real keyboard and input method.** The events are made by hand in the form section 07 gives them. Whether
  winit and a browser's textarea produce them so, and whether egui's web text agent copes, is part 4b.
- **Pixels.** Neither candidate is drawn. Contrast and the focus ring are not checked by eye or by measure.
  Target size is measured from the bounds in the accessibility tree: the own toolkit's smallest control is 24
  by 24; egui's radio buttons and buttons are 20 high at its default style, under the 24 of WCAG 2.5.8. egui's
  style can be changed (`interact_size`); that was not tried.
- **The web at all**, beyond that both compile for it. The DOM mirror is not written.
- **Scale, themes, right-to-left, a narrow screen.**
- **Other machines.** One Apple M4.
- **Maintenance**: egui's release cadence and breaking changes are in section 07, not re-checked here. Writing
  against egui 0.37 from memory of older versions fails at once: `Context::run`, `TopBottomPanel` and
  `WidgetType` are gone or renamed.

## Where the code departs from the design

- The trait is smaller than section 07's `Controls`: no `Instant`, no surface information, no `SceneFragment`
  (each candidate paints in its own form and only a count comes out), no cursor icon, no announcements.
- Commands may come out of `frame` as well as `handle`, because egui's do.
- The hex field accepts any six characters and is checked on Enter, so that composition has something to
  compose into.

## What the design should change

These go into `docs/design/` in a separate change.

1. Section 07, the candidates: egui's AccessKit output covers container roles and names; its `TextEdit` holds
   the preedit in the application's string; its commands arrive a frame late; it does not give one focus stop a
   group.
2. Section 07, accessibility: AccessKit 0.25.2 has no toggle-button role; "pressed" and "checked" are one
   property. `parley` 0.12 has no AccessKit support, so the text field's caret and selection for a screen
   reader are Everink's to write with either candidate.
3. Section 07, the options table: the measured sizes and frame costs above, in place of its estimates.
4. `docs/design/README.md`, the cross-section table, says the controls are "Everink's own small toolkit"; the
   owner's decision is that the toolkit is chosen after the spike. The table should say so.

## Kept afterwards

`spike/controls-trial` stays until the owner chooses. Part 4b builds on it.
