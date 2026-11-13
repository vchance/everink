# Controls, platform API, shells and embedding

Status: detailed design, 2026-11-11, proposal.

## Scope

This section covers everything between the core and the person (or the embedding app) that is not the drawing of the
surface itself:

- the shared controls (toolbars, pickers, dialogs, menus) in `everink-ui`, and the two toolkit candidates the spike
  compares;
- the platform API, the contract between the core and a shell, and how the core stays free of input and output;
- the two shells, native desktop (`everink-native`) and web (`everink-web`): windowing, event loop, input,
  clipboard, drag-and-drop, local storage, packaging;
- text entry on a canvas (input method composition, on-screen keyboards);
- accessibility of the controls and of the surface;
- the embedding API: the forms in which Everink is offered, its commands and events, theming;
- what must not be ruled out for tablets and phones.

It leaves to other sections: positions, camera and zoom arithmetic (01); the document model, merge and undo (02);
the sync protocol, server and server storage (03); presence and cursors (04); media decoding, fonts and the blob
store (05); drawing, ink, text layout and the GPU renderer (06); tokens, roles, invite links, limits and the threat
model (08). Where this section needs something from them it says so under "What this assumes of other sections".

Terms. A **shell** is the platform-specific program around the core: it owns the window or canvas element and talks
to the operating system or browser. An **effect** is a request the core makes of the shell ("send these bytes",
"write this to storage"). **Sans-I/O** means the core never does input or output itself: events go in, effects come
out. **IME** (input method editor) is the operating-system tool for typing characters a keyboard has no key for
(Chinese, Japanese, Korean, accent composition); the provisional text shown while composing is the **preedit**.
**AccessKit** is a Rust library that describes a user interface as a tree of accessible nodes and passes it to each
operating system's accessibility API.

## The problem

Owner decision 6 puts every control on the same GPU surface as the canvas. That removes what a native or DOM control
gets for free, and Everink must rebuild each item:

1. **Screen readers see nothing.** A canvas is one picture to a screen reader. Windows, macOS and Linux each have an
   accessibility API, and the web has a fourth (ARIA in the DOM). Without an explicit description every control and
   object is invisible to people who use one.
2. **Text entry is not free.** Only a real text field gets IME, spell-check, autofill, the emoji picker, handwriting
   and the on-screen keyboard. Text objects and every single-line field (hex color, version name, library search)
   depend on one bridge.
3. **Many input devices.** Pointer input is mouse, touch or pen. Pens carry pressure (0 to 1) and tilt, and a
   browser may merge several samples into one event (a 240 Hz pen into a 60 Hz event loop). A trackpad pinch is a
   wheel event with a modifier flag in Chromium and Firefox but separate gesture events in Safari (unverified,
   checked in the spike).
4. **The browser is single-threaded and refuses to block.** Anything that waits must be asynchronous, and some
   actions only work inside a user-gesture handler (showing the on-screen keyboard, reading the clipboard, starting
   sound).
5. **Graphics support varies by browser and changes by release.** This section cites no browser's WebGPU status.
   Owner decision, 2026-11-12: the WebGPU and WebGL2 paths are supported and tested; the processor-only path is best
   effort (it must open and work, slowly, and is not tuned or tested as a full experience). 06 provides the fallback
   chain (`vello_gpu` on WebGPU, then on WebGL2, then `vello_cpu`, behind its `Backend` trait), so the shell never
   assumes WebGPU: it asks the renderer which backend it got.
6. **Embedding.** Goal 4 means an embedder has its own page, headers, content security policy and accessibility
   story. Requiring special headers or a whole window would make Everink hard to embed.

Numbers that drive the design: pointer event to GPU submit under 4 ms and pen to visible ink as 06 measures it, so
live ink cannot wait for async work; at 60 Hz a frame is 16.7 ms, so the controls may spend about 1 ms per frame on
layout, painting and accessibility updates at ten visible widgets (a starting value); the first visit should
download in about two seconds on a 10 Mbit/s link, which is 2.5 MB compressed (10 Mbit/s is 1.25 MB/s).

## What others do

**egui** (Rerun and many tools) is immediate-mode: the whole UI is rebuilt each frame. Its README says it runs on
web and native; its AccessKit support "currently implements the native accessibility APIs on Windows and macOS", and
on other platforms, web included, there is "an experimental built-in screen reader" [2]. Its web backend takes text
and IME input through a hidden `<input>` called the text agent [9]; open issues include janky web focus (#4569),
mobile web text editing (#4527) and high CPU with AccessKit (search result, issues not opened). Release 0.37.0, MIT
OR Apache-2.0 [1].

**Flutter web** draws to a canvas and builds a parallel DOM of `<flt-semantics>` elements with ARIA roles and
labels, absolutely positioned over the canvas. It is off until someone activates a visually hidden "Enable
accessibility" button, because the cost is noticeable; text fields are real `<input>` or `<textarea>` elements;
listboxes expose only visible items and a few neighbors [10]. The closest precedent for Everink.

**Google Docs** moved to canvas rendering in 2021 and kept a hidden DOM for assistive technology [11, search
results, not opened]. HTML-in-canvas, a 2026 proposal that keeps real DOM in a canvas in the accessibility tree, is
an origin trial [24, search result] and no basis for a design today. Figma and Miro are reported to mirror into
hidden DOM [24, secondary, unverified].

**Linebender (Xilem, Masonry)**, the family of Vello and Parley: "experimental", Apache-2.0, built on Parley,
Fontique and AccessKit; its web backend `xilem_web` is DOM-based, not Masonry on a canvas [3]. Release 0.4.0 of
2025-10-29 [1]; roadmap notes list Parley IME as to-do [14, 2024, may be out of date]. **GPUI** 0.2.2, Apache-2.0,
"pre-1.0. There will often be breaking changes"; README covers macOS, Linux, Windows [13]; a web platform exists in
Zed's repository (search results, not opened). **Floem** 0.2.0 MIT, desktop only in its README [15]. **Makepad**
1.0.0 supports web, but a community thread says no IME and no assistive technology there [22, unverified]. **Iced**
0.14.0 MIT; accessibility on a fork only [23, unverified now]. **Blitz** (Dioxus Native) is beta, "many bugs and
missing features" [15]. **Slint** 1.18.1 is `GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR
LicenseRef-Slint-Software-3.0` [1]; the royalty-free terms require disclosing the use of Slint [17, snippet].

**tldraw and Excalidraw** are the embedding comparison. tldraw exposes everything through an `Editor` instance and
needs a license key in production [20]; Excalidraw gives the host an imperative API through an `excalidrawAPI`
callback and props such as `initialData` and `onChange` [20]. Both are React components, so a host can replace their
UI with its own. Everink cannot: its controls are pixels on a GPU surface. It can offer hide-and-command.

**Browser primitives.** EditContext lets a canvas page receive IME input; MDN: "Limited availability", experimental
[5]. Caniuse, read 2026-11-11: Chrome and Edge 121 and later, no Firefox or Safari [6]; Mozilla plans only the
DOM-based form [8]. A hidden textarea becomes the accessible surface of the edited content [21]. OPFS is Baseline,
its synchronous handle is worker-only, and it is deleted with site data [16]. The async Clipboard API needs a secure
context and user activation or a permission; custom types need a `web ` prefix [16]. AccessKit has released adapters
for Windows, macOS, Unix, Android and iOS, without rich text; the web adapter is only planned [4]. winit is
0.31.0-beta.4 (stable 0.30.13) [12, 1]. `wasm-pack` moved out of the archived rustwasm organization [18].

## Options considered

### Toolkit for the shared controls

"Share the surface" means drawing through the renderer Everink owns, in the same frame as the canvas. "Share the
text stack" means controls and canvas use one shaper and font set (Parley, 06's choice).

| Option | Version, license | wgpu native + browser | Shares surface and text stack | Accessibility | Text input and IME | Verdict |
|---|---|---|---|---|---|---|
| egui | 0.37.0, MIT OR Apache-2.0 | Yes both; `egui-wgpu` draws into a host pass | Surface yes; text stack no (own, not Parley) | AccessKit on Windows and macOS per README [2]; none on web but an experimental reader | Hidden `<input>` on web [9]; known web and mobile issues | **Candidate 2**, carried into the spike |
| Xilem / Masonry | 0.4.0, Apache-2.0 | Native yes; browser only through DOM backend [3] | Same family as Vello and Parley, but owns its window and loop | AccessKit, native | Parley IME to-do [14] | Fits the stack best; too early, no canvas-web path |
| Iced | 0.14.0, MIT | wgpu yes; web path uncertain [23] | Own renderer and text | AccessKit on a fork | Unverified on web | Reject |
| Floem | 0.2.0, MIT | Native only [15] | No | Not stated | Not stated | Reject (no web) |
| Makepad | 1.0.0, MIT OR Apache-2.0 | Both, own renderer | No | Web invisible [22, unverified] | No web IME [22] | Reject |
| GPUI | 0.2.2, Apache-2.0 | Own renderer; web new [13] | Owns its renderer | Not read | Unverified | Reject |
| Dioxus Native / Blitz | 0.3.0-beta.4, MIT OR Apache-2.0 | Vello based; web unclear [15] | Vello and Parley yes; large CSS engine | AccessKit a goal | Unclear | Reject for now; heavy for ten dialogs |
| Slint | 1.18.1, GPL-3.0 OR royalty-free (disclosure) OR commercial | Yes | Own renderer | AccessKit | Yes | Reject: not MIT OR Apache-2.0 and passes conditions to embedders |
| Purpose-built retained UI on Everink's renderer | ours, MIT OR Apache-2.0 | Whatever 06 gives | Yes, both, by construction | Emits `accesskit::TreeUpdate` | Reuses the canvas text bridge | **Candidate 1**, carried into the spike |

**The toolkit is not chosen yet** (owner decision, 2026-11-12). The step-0 spike builds the same toolbar (tool
palette, color swatches, undo and redo, zoom indicator) and the same single-line text field (the hex field) twice,
and the choice is made on the result. Both sit behind the `Controls` trait (below), so the rest of the system does
not change with the outcome. The other toolkits in the table are rejected for the reasons shown: Xilem/Masonry has
no canvas-web path, Iced, Floem, Makepad and GPUI fail on web or on their own renderer, Blitz is heavy for about a
dozen widgets, and Slint's license is not MIT OR Apache-2.0 and would pass its conditions to every embedder.

- **Candidate 1: Everink's own small toolkit.** A retained widget tree laid out by `taffy` (MIT, 0.15.1 [1]),
  painted through 06's scene API and shaped with 06's text service, emitting `accesskit::TreeUpdate`, reusing the
  canvas text bridge for its text fields. The control set is small (button, toggle, slider, swatch grid, color area,
  list, tab strip, popover, modal, tooltip, single-line text field, menu). Cost: Everink writes focus handling,
  scroll containers and popover placement itself.
- **Candidate 2: egui** (0.37.0, MIT OR Apache-2.0 [1]) with `egui-wgpu` drawing into a host render pass, behind
  `Controls`. Cost: its own text stack (not Parley), AccessKit coverage that its README lists as native Windows and
  macOS only [2], and a hidden `<input>` text agent on the web with open issues [9]. The spike qualified three of
  these (below; on the web through Everink's mirror, and natively without a screen reader): egui lays text out itself but shapes it with the libraries Parley uses; its AccessKit output carries
  container roles and names; and the text agent is not used when egui is driven through Everink's shell.

What each must show in the spike, on the same fixed scope, measured and written into this section:

| Criterion | What must be shown |
|---|---|
| Accessibility on desktop | The toolbar and the field reach UI Automation, NSAccessibility and AT-SPI with correct roles, names and states, checked with a screen reader on each |
| Accessibility on the web | The tree reaches the DOM mirror (Everink writes the mirror for either candidate, since AccessKit has no web adapter [4]) with the same roles and names; keyboard operation and visible focus work |
| Text composition | Latin accent composition (dead keys) works in the field on macOS, Windows and the web without the preedit being rewritten; a CJK preedit smoke test is recorded for later work |
| One text stack or two | Whether the controls shape text with the same shaper and fonts as canvas text; if two stacks, the measured difference in glyph shapes and metrics |
| Wasm size | The brotli and raw size added by the toolbar and field over the baseline build, against the budget under Packaging |
| Effort | Person-days to reach the fixed scope, lines of code, and the widget behaviors still missing (focus trap, scroll, popover placement) |
| Frame cost | Layout, painting and accessibility update within the 1 ms per frame planning figure |
| Maintenance and license | Both are MIT OR Apache-2.0 today; release cadence and breaking-change history over the spike period |

**What the spike measured** (`../spike/results-4a-controls.md`, `../spike/results-4b-window.md`,
`../spike/results-4b-web.md`; Apple M4, release build, Chrome 157; the same toolbar and hex field built twice, behind
one cut-down `Controls` trait). **The choice is not made** (to be decided; see [the index](README.md#still-open)).
No person pressed a key and no screen reader was heard: the tree is checked by tests and, on the web, read from
Chrome's accessibility tree; keys and compositions come from tests and from Chrome's debugging protocol.

| | Own (`taffy`, `parley`, AccessKit by hand) | egui 0.37.0 |
|---|---|---|
| The tree a screen reader needs (toolbar, radio groups, names, states, field) | Yes | Yes |
| Text composition (dead key, cancel, an input method in steps) | Yes | Yes |
| A frame with nothing changed; something changed (median, processor only, no drawing) | 1 µs; 7 to 9 µs | 18 µs; 19 to 21 µs |
| Added to a WebAssembly module, raw; Brotli; no font | 1.24 MB; 366 KB | 2.42 MB; 616 KB |
| The same with a font | 1.64 MB; 513 KB (one font) | 3.09 MB; 886 KB (its three) |
| Lines of code for this toolbar and field | 669 | 232 |
| Smallest control | 24 by 24 | 24 wide, 20 high at its default style |
| Presses of Tab from the start to the field | 4 (3 in the web page) | 13 (12 in the web page) |
| Commands come out | With the event | One frame later (the web shell runs two frames an event) |
| Arrow key on a focused tool | Focuses the next and chooses it | Focuses the next; chooses nothing |
| Text stack | Parley, the canvas's own | Its own layout and line breaking, over the shaper and font reader Parley uses (`harfrust`, `skrifa`) |
| Checks in the web page (54, five runs each) | All pass | All pass |

- **Frame cost does not separate them.** Both are at least 45 times inside the 1 ms planning figure; the own toolkit's
  figure leaves out shaping its text for drawing (the renderer's), egui's includes laying out text and tessellating.
  Section 06 budgets 0.5 ms of processor for the controls; measured here, 0.001 to 0.02 ms.
- **Size may.** egui is 1.7 to 1.95 times the bytes measured each alone, with or without fonts. The two share more than
  the design supposed: egui 0.37 shapes with `harfrust` 0.12.0 and reads fonts with `skrifa` 0.44.0, the same versions
  as Parley, so in a module that already has Parley egui adds about 1.85 MB raw without fonts, not 2.42. How much of
  the own toolkit's 1.24 MB is Parley was not measured apart. egui also brings `vello_cpu` 0.2.0, another version than
  the renderer's. Section 07's whole first version is budgeted at 8 MB raw and 2.5 MB with Brotli, fonts excluded.
- **Effort.** The own toolkit has what the trial needed and nothing else: no selecting text with the pointer or Shift,
  no copy, cut or paste, no hover look, no scroll, popovers, menus, sliders, tooltips or modal focus trap, and roles
  and names written by hand. egui has those, and lacks two things the design asks for by name: the canvas's own text
  layout, and one focus stop for a radio group with arrows inside it (it stops Tab at every button, and its arrows
  move the focus without checking). A test holds the own toolkit to a 24 by 24 target; egui is not held to it, and its
  default style is 20 high (`interact_size` can change it; not tried). Neither writes the caret and selection into the accessibility tree.
  "Person-days" is not a figure the trial can give: an agent wrote both in one session each, egui first.
- **egui's `TextEdit` puts the text being composed inside the application's string.** It worked in every test, but the
  application cannot tell the value from the preedit unless it keeps track itself, and egui counts composed text
  against the field's six-character limit (a seventh composed character is not shown).
- **Neither works without a font it is given.** Parley's editor cannot move the caret or take typed text with no font
  registered, so the controls must have their font before the first text field is shown. Neither uses the machine's
  fonts, as the design asks.
- **Pictures** (the same painters draw both, natively through `wgpu` and in Chrome through WebGPU): five faults were
  found only by looking at rendered output, none visible to tests of the tree. Neither candidate's unchosen tools
  have a boundary at 3:1 against the toolbar (WCAG 1.4.11): the own toolkit's by the trial's styling, egui's by its
  default style; the disabled Redo is exempt. These are of a trial's styling and egui's defaults, not of a theme made
  to meet WCAG 2.2 AA.
- **Not shown.** A screen reader (the owner's, with `spike/controls-window/README.md` and `spike/controls-web/README.md`);
  the platform adapters (`accesskit_winit` was handed the tree and nothing read it back from the system); a real
  keyboard's dead keys and a real input method; Windows and Linux; Safari, Firefox and a phone; explore by touch
  (the design's own open question); the mirror switched on by a button; a value written into the textarea by
  assistive technology or dictation; scale, themes, right-to-left and a narrow screen; egui's animations;
  maintenance and release cadence (not re-checked); and other machines.

The owner also asked that a project of theirs be considered where it makes sense; it is evaluated in the spike, its
details are kept outside this repository, and nothing here depends on it until then.

### Windowing and event loop

| Option | Native | Web | Notes |
|---|---|---|---|
| winit everywhere | yes, 0.31 beta | `winit-web` exists [12] | One loop type; web IME, pen tilt and DOM access limited and unverified |
| winit native, direct `web-sys` on the web | yes | own DOM code | More code; full control of IME, pointers, clipboard, mirror and the container |
| A toolkit's loop (eframe, Masonry) | per toolkit | per toolkit | Ties the shell to a toolkit: not used, whichever toolkit wins |

Chosen: winit natively with `accesskit_winit`, direct `web-sys` on the web. The web shell must mount into an element
the embedder supplies, and must own the textarea, the mirror, `getCoalescedEvents` and the clipboard events; winit's
web layer would sit between Everink and each. Natively the shell pins winit 0.30.13 until 0.31 is stable.

### Text-entry bridge on the web

| Option | Support | Accessibility | Cost |
|---|---|---|---|
| Hidden `<textarea>` at the caret | All browsers, mobile keyboards included | The textarea is a real text field for screen readers | Must be kept in step with the core; candidate-window placement approximate |
| EditContext | Chromium only [6] | Canvas form not planned in Firefox [8]; page supplies caret bounds | Two paths; no keyboard raising where absent |
| `keydown` only | all | none | No IME: unacceptable (the owner accepted the cost of composition) |

Chosen: the textarea as the only path first; EditContext later behind the same trait, if candidate placement proves
poor.

### Storage on the web

| Option | Good for | Limits |
|---|---|---|
| IndexedDB | Small and medium records, transactions, main thread | Async only; shares eviction rules |
| OPFS, async handles (main thread) | Large files, simple | No synchronous reads; slow for many small writes |
| OPFS sync access handle in a worker | Fast file-like access, can host a database | Worker only [16] |

Chosen: IndexedDB for records; OPFS in a dedicated worker for blobs; one `Storage` trait over both.

### Forms of the embedding API

| Form | For whom | Chosen |
|---|---|---|
| Rust crate `everink` (facade over core, ui, native shell) | Rust apps | Yes |
| npm package `@everink/web`: wasm, TypeScript API, `<everink-surface>` element | Web apps, any framework | Yes |
| C API over commands and notifications | C, C++, Swift, Kotlin hosts | Not now |
| Iframe-hosted build with a postMessage API | Hosts that want isolation | Yes, later, same command surface |

Package names are intentions only; publishing them is the owner's decision (AGENTS.md).

## Design

### Layers

```
 shell / embedder                      everink (one instance per surface view)
 ----------------                      ---------------------------------------
 winit or DOM events  --Input-->  [ everink-runtime ] --Event--> Core (sans-I/O state machine)
 Platform traits  <---Effect---        |                          |-- Controls (everink-ui)
 (storage, net, clipboard,             |                          |-- sync, render (scene)
  surface, text, a11y, media)          +-- runs effects, feeds Completed back
 Embedder Command --> Core;  Core --Notification--> Embedder
```

Packages: `everink-core`, `everink-render`, `everink-ui`, `everink-sync`, `everink-media` (05: the media worker and
blob store code), `everink-runtime` (new here: the loop that feeds events in and runs the core's effects through the
`Platform` traits; used by both shells), `everink-server`, `everink-native`, `everink-web`. Core, sync and ui know
nothing of any platform and never call async code.

### Sans-I/O: events in, effects out

```rust
pub struct Core { /* single-threaded, !Sync */ }

impl Core {
    pub fn new(config: Config, seed: [u8; 32], now: Instant) -> Core;
    /// Feed one event. Never blocks, never does I/O. Time is an argument, never read from a clock.
    pub fn handle(&mut self, now: Instant, event: Event);
    /// Drain what the core wants done; the runtime calls this after every handle().
    pub fn next_effect(&mut self) -> Option<Effect>;
    /// Earliest time the core wants waking (animation, smoothing, jittered reconnect and retry timers).
    pub fn next_wake(&self) -> Option<Instant>;
    /// Build the frame: scene for the GPU, accessibility update, cursor icon. Pure given the state.
    pub fn frame(&mut self, now: Instant, view: &SurfaceInfo) -> Frame;
}

pub enum Event {
    Input(InputEvent),                       // pointer, wheel, key, text, gesture, focus, drop
    Command(Command),                        // from the embedder, or from Controls
    Completed(RequestId, Completion),        // answer to an earlier effect
    Net(NetEvent),                           // Opened, Frame(Vec<u8>), Closed, BufferedAmount(u64)
    Surface(SurfaceEvent),                   // Resized, ScaleChanged, Lost, Restored, Hidden, Visible, Suspended, Resumed
    Appearance(Appearance),                  // dark or light, reduced motion, high contrast, UI scale
    Tick,                                    // the wake requested by next_wake
}

pub enum Effect {
    Net(NetRequest),                         // Connect, Send, Close (one WebSocket)
    Http(RequestId, HttpRequest),            // blob upload and download (03, 05)
    Storage(RequestId, StorageOp),
    Clipboard(ClipboardOp),                  // Write, or Read(RequestId)
    DecodeImage(RequestId, DecodeRequest),   // defined by 05
    Video(VideoOp),                          // open, seek, play, volume, mute (05)
    TextInput(TextInputState),               // show, move, or hide the text bridge
    SetCursorIcon(CursorIcon),
    RequestFrame,
    Announce(String, Politeness),            // screen-reader announcement, e.g. "Alice joined"
    OpenUrl(String),
    GetToken(RequestId),                     // ask the embedder for a fresh token (08)
    Notify(Notification),                    // to the embedder
}
```

Rules that make this testable:

1. **No clock, no randomness, no threads inside the core.** `now` is passed in and the id seed comes from the shell
   once, so a recorded sequence of `(now, Event)` replays to the same effects. Reconnect timers are the core's own,
   jittered with its seeded generator and delivered through `next_wake` (03 owns the schedule). The same core runs
   unchanged in the server and the overview renderer.
2. **Effects run before the event handler returns.** After `handle` the runtime drains `next_effect()` in the same
   call stack as the browser or OS event. A browser raises the on-screen keyboard and allows a clipboard read only
   inside a user-gesture handler, so a tap on a text object must focus the textarea before the handler ends. Effects
   that finish later come back as `Completed`.
3. **Every asynchronous effect has a `RequestId` and exactly one `Completed`** (success, failure or cancelled). The
   core keeps its own timeouts.
4. **No ordering between kinds of effect**, but `Net::Send` keeps order, and so do `Storage` writes. The outbox rule
   (03) relies on this.

### The platform traits

Shells implement these; the runtime uses them to run effects and turn results into `Completed`. They use `async fn`
in traits (stable since Rust 1.75). The browser has no threads, so futures are not `Send` there; one marker handles
both:

```rust
/// Send on native, nothing on wasm32. Futures and callbacks in the traits are bounded by it.
#[cfg(not(target_arch = "wasm32"))] pub trait MaybeSend: Send {}
#[cfg(target_arch = "wasm32")]      pub trait MaybeSend {}

pub trait Platform: MaybeSend + 'static {
    type Surface: SurfaceHost;  type Net: Transport;   type Http: HttpClient;  type Store: Storage;
    type Clip: Clipboard;       type Text: TextBridge; type A11y: AccessibilityHost;
    type Video: PlatformVideoPlayer /* defined in 05 */;  type Media: MediaHost;  type Clock: Clock;  type Host: HostServices;
    fn parts(&mut self) -> Parts<'_, Self>;
}

pub trait SurfaceHost {
    /// wgpu surface target: a window handle natively, the main canvas on the web. The renderer (06) makes the device.
    fn create_target(&mut self, instance: &wgpu::Instance) -> Result<wgpu::Surface<'static>, SurfaceError>;
    /// Web only: the `desynchronized` 2D canvas stacked above the main one, for local live ink (06). None natively.
    fn live_ink_target(&mut self) -> Option<LiveInkTarget>;
    fn info(&self) -> SurfaceInfo;            // physical size, scale, safe-area and keyboard insets, refresh rate hint
    fn request_frame(&mut self);              // ask for one redraw callback; repeated calls coalesce
    fn set_cursor(&mut self, icon: CursorIcon);
    fn set_fullscreen(&mut self, on: bool);
}

pub trait Transport {                          // the one WebSocket per surface (03 owns the protocol)
    /// Opens the socket; `url` carries no secret. The first frame the core sends is only `auth { token }`, as 08
    /// defines it; the shell never puts the token in the address.
    async fn connect(&mut self, url: &str, subprotocol: &str) -> Result<(), NetError>;
    fn send(&mut self, frame: &[u8]) -> Result<(), NetError>;   // binary frame; never blocks
    fn buffered_amount(&self) -> u64;          // bytes queued, not yet on the wire
    fn close(&mut self, code: u16, reason: &str);
    // Incoming frames and closure arrive as Event::Net, pushed by the shell's reader task.
}

pub trait HttpClient {                          // blob transfer and token exchange (03, 05, 08)
    /// Headers include `Range`; the response body is a chunk stream.
    async fn request(&self, req: HttpRequest) -> Result<HttpResponse, HttpError>;
    /// Streaming `PATCH` upload. Where the platform cannot stream a request body, the shell sends sequential
    /// chunk requests with the same observable result.
    async fn upload(&self, req: HttpRequest, body: UploadBody) -> Result<HttpResponse, HttpError>;
}

pub trait Storage {                             // see "Local storage"
    async fn get(&self, ns: Namespace, key: &[u8]) -> Result<Option<Vec<u8>>, StorageError>;
    async fn batch(&self, ops: Vec<StorageOp>) -> Result<(), StorageError>;   // atomic: all or none
    async fn scan(&self, ns: Namespace, prefix: &[u8], limit: usize) -> Result<Vec<(Vec<u8>, Vec<u8>)>, StorageError>;
    async fn blob_put(&self, hash: BlobHash, bytes: BlobSource) -> Result<(), StorageError>;   // BlobSource may stream
    async fn blob_get(&self, hash: BlobHash, range: Option<Range<u64>>) -> Result<Option<Vec<u8>>, StorageError>;
    async fn usage(&self) -> StorageUsage;     // used bytes, quota if known, whether persistence was granted
    async fn request_persistence(&self) -> bool; // web: navigator.storage.persist(); native: true
    async fn clear_all(&self, identity: IdentityScope) -> Result<(), StorageError>;   // sign-out
}

pub trait Clipboard {
    async fn write(&self, items: ClipboardItems) -> Result<(), ClipboardError>;
    async fn read(&self) -> Result<ClipboardItems, ClipboardError>;  // may fail: no user activation, denied
}

pub trait TextBridge {                          // see "Text entry"
    fn set_state(&mut self, state: Option<TextInputState>);   // None hides the bridge and the keyboard
    // Preedit, commit and delete-surrounding arrive as InputEvent::Text.
}

pub trait AccessibilityHost {                   // see "Accessibility"
    fn set_enabled_hint(&mut self) -> bool;     // has assistive technology asked for the tree?
    fn update(&mut self, update: accesskit::TreeUpdate);
    fn announce(&mut self, text: &str, politeness: Politeness);
}

pub trait MediaHost {                           // still images; detail in 05
    async fn decode_image(&self, bytes: Vec<u8>, max_dim: u32) -> Result<DecodedImage, MediaError>;
}

pub trait Clock {
    fn now(&self) -> Instant;                   // monotonic; web: performance.now()
    fn wall_ms(&self) -> i64;                   // wall time for display only
    fn wake_at(&mut self, at: Option<Instant>); // schedule Event::Tick
}

pub trait HostServices {                        // files, drag-and-drop, URLs, activation, randomness
    async fn read_dropped(&self, file: DroppedFileId, max: u64) -> Result<FileData, FileError>;
    async fn pick_files(&self, accept: &[Mime]) -> Result<Vec<DroppedFileId>, FileError>;
    fn open_url(&self, url: &str);
    fn user_activation(&self) -> Activation;    // web: navigator.userActivation (active now, sticky); native: always active
    fn entropy(&self) -> [u8; 32];              // crypto.getRandomValues or OS random, used once for the seed
}
```

`Transport::buffered_amount` implements 03's adaptive pacing. The client keeps unsent bytes below what the
connection drains in about 20 ms, clamped to between 4 KiB and 64 KiB, and writes bulk data in chunks no larger than
that limit and never over 16 KiB; each side measures its own drain rate: the client, through the shell, from how
fast `buffered_amount` falls for what it sends, and the server from how fast its writes complete for what it sends.
A cursor update then waits about 40 ms at most behind bulk data; the 4 KiB floor takes over below about 1.6 Mbit/s,
where cursors can lag during bulk loads and the server sends fewer objects first by raising `eff_min_px`. 20 ms is
the first value the spike tries. The client applies the same rule to its own uploads of large changes (a big paste,
a long offline backlog), so its cursor update is not stuck behind them: presence is written first and dropped
newest-wins when the queue is over the limit. A browser WebSocket has no other backpressure signal
(`WebSocketStream` is experimental, unverified).

### Threading and async

| | Native | Web |
|---|---|---|
| Core, controls, scene building | One thread (winit's main thread) | The main thread |
| Executor | `tokio` multi-thread runtime for net, storage, decoding; results post an `Event` through a channel and wake the loop (`EventLoopProxy`) | `wasm-bindgen-futures::spawn_local`; results call `handle` or a queue |
| Storage | `redb` on a blocking pool | IndexedDB on the main thread; OPFS in one dedicated worker |
| Heavy work (decode, hashing, large merges) | Runtime thread pool | Browser decoders (`createImageBitmap`); one worker for hashing, started on demand |
| Threads inside wasm | n/a | **Not used.** They need `SharedArrayBuffer`, hence cross-origin isolation (COOP `same-origin`, COEP `require-corp`) for the whole page, which an embedder cannot accept in general |

**Measured** (`../spike/results-3a-wasm-outlines-cut.md`, `../spike/results-3c-browser.md`; Apple M4): WebAssembly on
one thread ran section 01's cut 3 to 4 times slower than native and the stroke outline 2.7 times slower (under Node
26, Chrome's engine), and the renderer's redraw of 10,000 strokes 1.3 to 1.4 times slower than native with SIMD and 3.0 to 3.4
times without. A worker that prepares tiles was not tried.

The core is single-threaded on purpose: the document, camera and controls change only inside `handle`, so there are
no locks and replay is deterministic. Anything slow is an effect. Re-entrancy: a DOM event can fire while an async
continuation runs, so `handle` is never called re-entrantly; the runtime queues events that arrive during a `handle`
and runs them in order after it returns, still inside the original handler where one exists. Browsers throttle
timers in hidden tabs, so reconnect and smoothing timers run slower there.

### Windowing and the event loop

Native: winit `ApplicationHandler`, `ControlFlow::Wait`. The loop wakes on input, on `EventLoopProxy` messages from
tokio, on `next_wake()` (`WaitUntil`) and on `RequestFrame` (`request_redraw`). While a stroke or animation runs the
core asks for a frame at every display refresh; idle, the process sleeps. IME is allowed (`set_ime_allowed`) only
while a `TextInputState` is active, and `set_ime_cursor_area` follows the caret (names from winit 0.30; unverified
for 0.31).

Web: `mount(container, options)` builds, inside a shadow root in the container, a stack from bottom to top:

1. the **accessibility mirror root** and the **hidden `<textarea>`**, lowest, absolutely positioned;
2. the **main `<canvas>`** (the renderer's target);
3. a **`desynchronized` 2D canvas** for the local user's live ink, topmost; the shell creates this element and its
   stacking and hands it to the renderer through `SurfaceHost::live_ink_target()`, and 06 draws into what it is
   given, because nothing may be placed above a desynchronized canvas without losing its low-latency path.

Pointer listeners are on the topmost canvas (`touch-action: none`, pointer capture); the other canvas has
`pointer-events: none`. What this means for the layer underneath, checked by reasoning (a browser run was not
possible here; each item has a test below):

- *Pointer events:* mouse, pen and touch hit the topmost canvas, which is what is wanted; the core hit-tests its own
  controls from coordinates. The mirror never receives them.
- *Focus:* DOM `focus()` does not depend on stacking, so the textarea and mirror elements take focus while beneath
  the canvases. Their native focus outlines are therefore invisible, so the controls draw their own focus ring from
  the same tree; the textarea's native caret and iOS selection handles are not seen either, and Everink draws its
  own caret and menus.
- *Screen-reader activation:* desktop screen readers act through the accessibility tree (focus and "click" actions
  sent to the element), not coordinates, so they are unaffected. Explore-by-touch (VoiceOver on iOS, TalkBack) finds
  an element by hit testing, which lands on the topmost canvas; that is why each mirror element is also positioned
  over its control, so a double-tap at that spot reaches the control through the canvas, and why the canvas carries
  `role="application"` with a label that tells the reader to swipe. Linear swipe navigation walks the DOM and is
  unaffected. Whether explore-by-touch can reach mirror elements under a canvas is **unverified**: it is the first
  item on the manual mobile screen-reader list.
- *IME and on-screen keyboard:* the candidate window follows the textarea's position, which is at the caret
  rectangle even when covered, and `focus()` called inside the tap handler raises the keyboard regardless of
  z-order.

The canvases are sized by one `ResizeObserver` (`devicePixelContentBoxSize` where available, else CSS size times
`devicePixelRatio`); `devicePixelRatio` changes are tracked with a `matchMedia("(resolution: Xdppx)")` listener and
sent as `ScaleChanged`. Frames use `requestAnimationFrame` only while something asks. `visibilitychange` becomes
`Hidden` and `Visible` (04 uses them to stop sending cursors from a hidden tab), `pagehide` becomes `Suspended`; the
outbox is written on every acknowledged local change, not only at exit, because the browser may kill the page before
asynchronous writes finish. The whole shell runs under a content security policy with `'wasm-unsafe-eval'` as the
only relaxation (see Packaging), so it uses no inline script, no `eval`, and builds its styles with constructable
stylesheets (their acceptance under a strict `style-src` is unverified and is in the browser tests).


**View freshness.** Whenever the camera changes, the core queues a `ViewMsg` (holding the exact, unpadded `ViewRect`
from `Camera::view_rect`) for the sync layer. While the view keeps moving (a pan, a zoom, an animation to a saved
place) it keeps a timer in `next_wake` so one is sent at least every 100 ms, however small the move. The shell only
runs the frame loop and delivers `Tick`; a hidden tab (`Hidden`) stops the stream.

### Input

One event model for every device:

```rust
pub enum InputEvent {
    Pointer(PointerEvent), Wheel(WheelEvent), Gesture(GestureEvent),
    Key(KeyEvent), Text(TextEvent), Focus(bool), Drop(DropEvent), FilesHovered(Vec<FileHint>),
}
pub struct PointerEvent {
    pub id: PointerId, pub kind: PointerKind,        // Mouse, Touch, Pen
    pub phase: Phase,                                // Down, Move, Up, Cancel, Enter, Leave (hover included)
    pub samples: SmallVec<[Sample; 8]>,              // coalesced samples, oldest first, never empty
    pub predicted: SmallVec<[Sample; 4]>,            // platform prediction if offered; the core may ignore it
    pub buttons: Buttons,                            // primary, secondary, middle, barrel, eraser-tip
    pub modifiers: Modifiers,
    pub is_primary: bool,                            // the first finger or the pen
}
pub struct Sample {
    pub pos: PhysicalPoint, pub t: Instant,          // device pixels; high-resolution timestamp
    pub pressure: Option<f32>,                       // 0..=1; None when the device has none (mouse)
    pub tilt: Option<(f32, f32)>,                    // radians, x and y
    pub twist: Option<f32>, pub contact_size: Option<(f32, f32)>,
}
```

- **Pointers and pen.** Web: `pointerdown/move/up/cancel` with `setPointerCapture`; `getCoalescedEvents()` on every
  move so the sample rate is the device's, not the frame rate, and `getPredictedEvents()` where present (Safari
  support for either unverified; MDN lists both as not Baseline [16]). All samples of an event reach the core in the
  same call and feed 06's live-ink path directly, with no queue between. A mouse reports `pressure: None`, not the
  browser's 0.5. The eraser end and barrel button are in `Buttons`. Native: winit touch and pen events with force
  and tilt where the OS supplies them (what winit reports for tilt per OS is unverified; the spike checks, and a
  per-OS adapter is allowed). Pointer entry, exit and hover movement are delivered too: 04 needs them to know when a
  cursor is active or has left the surface.
- **Palm rejection** is core policy: while a pen is in contact or hovering, touches pan and zoom and do not draw.
  Without a pen a single finger draws with a pen tool and pans otherwise (a setting).
- **Touch gestures.** The core's recognizer turns two touches into pan and pinch from raw pointers, so behavior is
  the same on every target. Safari's `gesturestart/change/end` and `ctrlKey` + wheel in other browsers map to
  `GestureEvent::Pinch { scale_delta, center }`.
- **Wheel versus trackpad.** The shell classifies each event stream. Line `deltaMode`, or pixel `deltaY` a multiple
  of 100 or 120 with `deltaX == 0` and gaps over 50 ms, is a mouse wheel; many small fractional deltas under 20 ms
  apart are a trackpad. The class is sticky for 500 ms. Mouse wheel zooms about the cursor (one notch is a factor of
  1.25; Shift pans sideways); trackpad scroll pans and pinch zooms. The thresholds are starting values; the spike
  records traces from a Magic Mouse, a Logitech MX wheel, a Windows precision touchpad and a Linux libinput
  touchpad. Natively `LineDelta` and `PixelDelta` and the OS pinch event (winit exposes pinch on macOS and touch
  platforms; Windows and Linux touchpad pinch is unverified).
- **Keyboard.** The shell sends the physical `code` (for position-based shortcuts), the logical `key`, `repeat` and
  `is_composing`; a composing key event is never a shortcut. The primary modifier is Cmd on macOS and Ctrl
  elsewhere; single-letter tool shortcuts fire only when no text field has focus; one binding table in `everink-ui`
  feeds tooltips, menus and the help dialog. Browsers reserve some combinations (Ctrl/Cmd+T, N, W, Q, tab switching)
  which a page cannot intercept, and the table avoids them; the Keyboard Lock API (Chromium, fullscreen, unverified)
  is not relied on. The embedder can disable any binding by id or all of them, and Everink handles keys only while
  the canvas or one of its controls has focus.
- **Focus.** On the web Everink's focus is DOM focus on the canvas or textarea. Focus leaving the container is
  `Focus(false)`, and the core cancels in-progress gestures.

### Text entry on a canvas

The core owns all text: layout, caret, selection and characters (the text CRDT of 02). The shell supplies the
platform hookup, told what to expose:

```rust
pub struct TextInputState {
    pub field: FieldId,                  // which text object or control field has focus
    pub surrounding: String,             // a window of the text around the caret, at most 512 chars
    pub window_start: usize,             // offset of `surrounding` in the whole text, in UTF-16 units
    pub selection: Range<usize>,         // within `surrounding`
    pub composition: Option<Range<usize>>,
    pub caret_rect: Rect,                // device pixels; where IME candidates should appear
    pub multiline: bool, pub input_mode: InputMode,    // text, numeric, url, search, ...
    pub enter_key: EnterKeyHint, pub autocapitalize: bool, pub spellcheck: bool, pub password: bool,
}
pub enum TextEvent {
    Preedit { text: String, cursor: Option<(usize, usize)> },  // provisional; replaces the previous preedit
    Commit(String),                                            // final text, replaces preedit
    DeleteSurrounding { before: usize, after: usize },         // unit fixed per target in the shell
    Key(KeyEvent),                                             // Backspace, arrows, Enter when not composing
    CompositionCancelled,
}
```

Web algorithm (the hidden textarea):

1. On `TextInput(Some(state))` the runtime, still inside the gesture's call stack (rule 2), sets the textarea value
   to `surrounding`, its selection, places it at `caret_rect` (so candidate windows and the touch keyboard appear at
   the caret), sets `inputmode`, `enterkeyhint`, `autocapitalize`, `spellcheck`, and calls `focus({preventScroll:
   true})`, which raises the on-screen keyboard on iOS and Android.
2. `compositionstart/update/end` become `Preedit` and `Commit`. `beforeinput` events (`deleteContentBackward` and
   similar) become `Key` or `DeleteSurrounding` and are cancelled, so the core stays the source of truth. A
   `keydown` with `isComposing` or key code 229 is not forwarded.
3. After each event the core applies it and answers with a new state; the runtime rewrites the textarea only if text
   or selection differ and never during a composition (rewriting cancels it in several IMEs, a common bug in canvas
   editors). A remote edit inside the window during a composition is held until `compositionend`.
4. The textarea is labelled and is also the screen reader's text field. It is transparent and caret-sized, never
   `display: none` or `visibility: hidden`, because a hidden element cannot take focus.
5. Only a 512-character window is mirrored, so a text object of any length costs the same.

On desktop the same state goes to winit (`set_ime_allowed`, `set_ime_cursor_area`) and `WindowEvent::Ime` events
become the same `TextEvent`s. macOS and Windows deliver IME well; Linux depends on IBus, Fcitx 5 or Wayland
text-input and on winit's coverage, which is unverified and on the manual list. EditContext later is a Chromium-only
second `TextBridge` (attach to the canvas, answer `characterboundsupdate`, apply `textformatupdate`), falling back
to the textarea.

**Scripts (owner decision, 2026-11-12).** Text on the surface supports Latin script only at first release; other
scripts and emoji show as missing-glyph boxes until support is added, and no CJK or emoji fonts are shipped or
fetched. The bridge still carries Latin accent composition (dead keys, which produce a preedit). Composition for
other scripts (Chinese, Japanese, Korean, and the Windows and Linux input method frameworks) is kept in this design
as later work: the `TextInputState` and `TextEvent` shapes already support it, and only its test matrix and fonts
are deferred. The interface is English with every string in one table (`Strings`, keyed by stable ids, from the
first commit); right-to-left layout is designed for (layout uses logical start and end, and a mirroring flag exists)
and not shipped.

**The bridge is needed in build step 1**, not step 4: the hex field of the color picker is a text field. The version
name and library search fields (later) reuse it; the text-formatting bar does not need it.

### Controls: the toolkit

The first app is a whiteboard with about 50 active people per surface as the planning figure (owner decision,
2026-11-12), and the control list is written for one. Whichever candidate wins, `everink-ui` meets the system
through the `Controls` trait below and paints through **06's scene API into the same frame as the canvas**.
Candidate 1 also shapes text with **06's shared text service (Parley)**, so controls and canvas share one renderer
and one text stack, and builds a retained widget tree in an arena laid out by `taffy`; every widget produces an
`accesskit::Node`. Candidate 2 produces the same `UiFrame` from egui's output.

```rust
pub trait Controls {
    /// Offer an input event. Returns whether the controls consumed it (hit a control, or hold a pointer capture).
    fn handle(&mut self, now: Instant, ev: &InputEvent, ctx: &mut UiContext) -> Consumed;
    /// Lay out for a surface size and scale; build the scene fragment and the accessibility changes since last call.
    fn frame(&mut self, now: Instant, info: &SurfaceInfo, model: &UiModel) -> UiFrame;
}
pub struct UiFrame { pub scene: SceneFragment, pub tree: Option<accesskit::TreeUpdate>,
                     pub cursor: Option<CursorIcon>, pub text_input: Option<TextInputState>, pub next_wake: Option<Instant> }
pub struct UiContext<'a> { pub commands: &'a mut Vec<Command>, pub announce: &'a mut Vec<String> }
```

`UiFrame.scene` as written is the own toolkit's form. egui cannot fill it: it gives triangles for `egui-wgpu`, so
with egui the frame would carry those and the compose pass a host render pass in place of a `SceneFragment`. The
spike's own trait left the scene out for this reason (`../spike/results-4a-controls.md`). The trait's output is
settled with the choice of toolkit.

- **Controls only issue `Command`s**, the same enum as the embedding API, so the built-in UI is the first user of
  the public surface. They read a `UiModel`, a read-only snapshot (tool, color, can-undo, peers, connection state,
  role capabilities, camera description); they hold no document state.
- **Input order.** Controls see pointers first; a hit or pointer capture keeps the canvas from seeing the event.
  Keyboard events go to the focused control, else the canvas. Controls live in screen pixels and know nothing of the
  position system (01).
- **Layout.** Docked panels (tool palette at an edge, top bar), popovers (pickers, menus), modals (invite,
  versions). Under 600 CSS px (a starting value) the palette becomes a bottom bar and modals go full width. UI scale
  is the OS or browser scale times a user setting (80 to 200 percent). Targets are at least 24 by 24 CSS px, 44 by
  44 on touch.
- **Theme** is a struct of tokens (below), defaulting to the OS appearance, reduced motion and forced colors from
  `Event::Appearance`. No font is fetched for the interface; it is English, and right-to-left layout is designed for
  and not shipped.
- **Focus.** One focus order per panel; `Tab` moves between panels and controls; arrow keys move inside a group
  (palette, swatches, menus) as the WAI-ARIA authoring practices describe (unverified here, confirmed per widget);
  `Escape` closes the topmost popover or modal and returns focus; modals trap focus. The canvas is one tab stop and
  `Tab` leaves it (no keyboard trap, WCAG 2.1.2).

**The controls the first versions need, and the step that introduces each:**

| Control | Step | Notes |
|---|---|---|
| Tool palette (select, pen, eraser, shapes, line/arrow; text tool with text) | 1 | Radio group; shortcuts in tooltips |
| Color picker (swatch grid, hue and saturation/value area, hex field, recent colors) and stroke picker (width slider, preview) | 1 | The hex field needs the text bridge in step 1 |
| Undo, redo | 1 | |
| Zoom and level indicator, zoom in/out, reset to home, "fit content" | 1 | "Level 3, 12x" (wording from 01); announced on change, rate-limited; tells a person where they are in an unbounded space |
| Context menu (copy, paste, delete, duplicate, order, group) | 1, grows | Right click, long press or the `Menu` key |
| Connection status | 2 | Online, reconnecting, offline with N unsent changes |
| Invite dialog (owner): make an invite link with a role (viewer or editor) and an expiry (30 days by default), see who came through it, turn it off, suspend its people | 2 | Content from 08; the link address is handed to the embedding app, never opened by Everink |
| People list (name, color), "jump to", the opt-out of being located, and who follows whom | 2; jump and follow 3 | Panel fed by 04's people data |
| Saved places list; "more here if you zoom in" markers | 3 | The markers are overlay (06) |
| Text formatting bar (font, size, weight, color, alignment) | 4 | Follows the selected text object |
| Library browser (built-in and user libraries; search; drag out; save selection as template) | 4 | Thumbnails from 06 |
| Upload and download progress, "not yet uploaded" marks | 4 | |
| Command palette (`Ctrl/Cmd+K`) | 4 | Lists every command; needs a search field |
| Versions dialog (list, name, open read-only, copy from, restore with confirmation) | 5 | Who may create or restore is 08's rule |
| Role-aware disabled states, bounded-surface indicator | 5 | Disabled controls stay in focus order and say why |
| Video controls (play, pause, seek, mute) and the unmute control | 6 | Overlay on the selected video; the unmute control appears when sound is blocked |

The step-0 spike includes the palette, the zoom indicator and a hex field, to test the widget kit, the text bridge,
the mirror and the web shell together.

### Accessibility

**One source.** Every widget yields an `accesskit::Node` (role, name, value, state, bounds, children, actions).
`accesskit` (0.25.2, MIT OR Apache-2.0 [1]) is the schema crate; its types are the core's own output on every
target.

**What the spike found of AccessKit** (`../spike/results-4a-controls.md`, `../spike/results-4b-window.md`).
AccessKit 0.25.2 has no role for a toggle button and reports "checked" and "pressed" through one property; both
candidates used `RadioButton` with it. `parley` 0.12 has no AccessKit support, so the text field's caret and selection
for a screen reader are Everink's to write with either candidate (egui's `TextEdit` gives the value only). egui 0.37
lets a container carry a role and a name, and the tree it gave passed the same checks as the one written by hand. The
display's scale has to reach the tree. The shell applies it, on the tree's root node, unless the toolkit's root
already carries it: the own toolkit's does not, and egui's does.

**Native.** `accesskit_winit` (0.34.2, Apache-2.0 only [1]) connects to UI Automation, NSAccessibility and AT-SPI.
Adapters lack rich text [4], so text objects are a plain string value with caret and selection. The tree is built
only when assistive technology asks. In the spike the adapter was offered every frame's tree and took it only while
assistive technology was active, which it never was, so nothing was read back from the system.

**Web.** `everink-web` translates `TreeUpdate` to DOM, after Flutter [10]:

1. A visually hidden "Enable accessibility" button is the first focusable element. Activating it turns the mirror
   on; until then no mirror nodes exist. The embedder can force it on.
2. The mirror root is `<div role="application" aria-label="Drawing surface">` (a deliberate choice: screen readers
   then pass keystrokes to the page; the embedder can override). Nodes become elements with ARIA roles,
   `aria-label`, `aria-pressed/checked/expanded/valuenow/disabled`, and one element in the tab order (the one
   that has the controls' focus; see "Which side moves the focus" below). **Every string is set
   with `textContent` or an attribute setter; the shell never uses `innerHTML`**, because names include
   user-supplied text (people, text objects, file names).
3. Elements are positioned over their controls (see the stack under "Windowing" for what that does underneath the
   canvases). Only controls and the objects-in-view list are mirrored.
4. DOM focus is the single focus: a control gets real focus when it has accessibility focus, and `focus`, `click`
   and arrow events on mirror elements return as `Command`s or `Key` events. A text field is the hidden textarea
   itself.
5. Two `aria-live` regions (polite, assertive) carry announcements, at most one per second, identical messages
   coalesced.

**What the spike's web page found** (`../spike/results-4b-web.md`; Chrome 157; the mirror was always on, and the two
live regions and the objects list were not built). One mirror kept from the AccessKit tree, a textarea as the text
field and the DOM's focus as the single focus worked with both candidates: Chrome's accessibility tree was right, and
typing and composition were right. AccessKit's role, label, value, toggled, disabled, bounds, children and the three
actions were all the page needed. What the design did not say:
- **Which side moves the focus.** The controls do, and the DOM's focus follows: the shell gives Tab and the arrows to
  the controls and then puts the DOM's focus on the element of the control they name. (This section had the browser
  moving the focus over a roving `tabindex`; left to the browser, the first Tab went to the first element in the
  mirror and not to the control the toolkit starts at.) The controls choose their entry
  whichever way and from wherever Tab comes, and let go of their focus when the DOM's focus goes elsewhere in the page,
  but keep it when only the window loses the focus. A toolkit that cycles its focus (egui does) would trap the keyboard
  (WCAG 2.1.2), so the shell makes it drop the focus and lets the browser take the key.
- **A node may have no bounds.** egui gives none for its containers (the toolbar, the groups); the mirror must not size
  or clip by them.
- **A toolkit that answers late needs a second frame.** egui's commands, and its focus after Shift and Tab, appear in
  the frame after the event; a shell that draws once an event shows it one key behind.
- **A value that does not change is not rewritten.** Attributes and text are written only when they change, `tabindex`
  included (writing it "only when it changes" at first never wrote it). The zoom figure is plain text and not a live
  region, because as one it was rewritten on every frame. Pasted, dropped and replacement text become a commit, and
  every other kind of edit (deleting a word, undo) is cancelled.
- **Not shown:** explore by touch over a canvas (still the design's open question), whether a screen reader says the
  textarea is multi-line of a one-line field, and the cost of keeping the mirror (not measured).

**What the surface exposes.**

- **Controls: all of them**, from the first version that has them.
- **Selection summary: yes.** An always-present element reading "3 objects selected: 2 strokes, 1 text".
- **Objects in view as a navigable list: yes, bounded.** The nearest at most 50 objects (a starting value that keeps
  the mirror to a few hundred nodes), in reading order on the view's level, each with kind, author, text content
  (first 100 characters; alt text or file name for images) and rough position ("upper left"). Objects at other sizes
  are a count ("12 more objects at finer scale"), because the surface is unbounded. Nearest is by exact distance
  from the center of `Camera::view_rect` (01).
- **Rich-text structure:** not in the first version, matching the native adapters' limit [4].
- **Ink:** listed as "stroke by Alice"; its shape cannot be heard and no description is generated. Authors can set
  alt text on images and groups (a text property, 02).
- **Other people's cursors:** not mirrored. The people list is, with polite, rate-limited join and leave
  announcements.

**Keyboard-only operation (WCAG 2.1.1).**

- `Tab` into the canvas; arrows pan by a tenth of the view, `Shift+arrows` by a view, `+`/`-` zoom about the center,
  `0` goes home.
- `F6` or `Ctrl+Alt+L` enters the objects list; `Up`/`Down` move, `Enter` selects (the camera moves the minimum if
  the object is off screen). On a selection, arrows nudge by a grid step, `Shift` by ten, `Alt+arrows` resize,
  `Delete` removes, `Ctrl/Cmd+C/V/X/D/Z` as usual. This is the alternative to dragging (WCAG 2.5.7).
- Shapes, text and library items can be placed at the view center from the keyboard. Freehand ink has no keyboard
  equivalent; it is a pointing activity and is documented as such.

**WCAG 2.2 criteria that bind the controls** (numbers from the Recommendation, not re-fetched, unverified): 1.1.1
names for icon buttons; 1.3.1 and 4.1.2 roles, names, states; 1.4.3 and 1.4.11 contrast (4.5:1 text, 3:1 borders and
states, per theme); 1.4.4 and 1.4.10 UI scale to 200 percent; 1.4.13 dismissible tooltips; 2.1.1 and 2.1.2 keyboard,
no trap; 2.4.3 and 2.4.7 focus order and visible focus; 2.4.11 focus not obscured (the camera keeps a selection out
from under panels); 2.5.1 pinch has a single-pointer alternative (zoom buttons); 2.5.7 dragging alternatives; 2.5.8
target size 24 by 24 CSS px; 3.3.7 no redundant entry; 3.3.8 no password or puzzle (Everink asks for none; sign-in
belongs to the embedding app); no flashing, and animation respects reduced motion. **Commitment (owner decision,
2026-11-12).** The built-in controls meet WCAG 2.2 AA: keyboard operation, screen-reader labels, contrast and focus.
The canvas exposes a selection summary and a list of objects in view, and objects can be moved and resized from the
keyboard. Freehand drawing has no keyboard equivalent, and a published statement says so. The criteria above are
what this commitment binds the controls to; the statement is published after the first screen-reader pass.

### Clipboard and drag-and-drop

```rust
pub struct ClipboardItems { pub text: Option<String>, pub html: Option<String>, pub images: Vec<EncodedImage>,
                            pub everink: Option<EverinkClip> }
pub struct EverinkClip { pub version: u32, pub payload: Vec<u8> /* postcard or CBOR bytes of objects */ }
```

- **Copy.** The core builds an `EverinkClip` of the selection (new ids on paste, relative positions, styles, text;
  for images and videos the blob hash and tiny preview, not the bytes), a `text/plain` fallback, and for a single
  image or a rendered selection an `image/png` (capped, rendered by 06).
- **Carrying the custom format.** The payload goes (a) as the custom type `web application/x-everink` in the async
  API (feature-detected with `ClipboardItem.supports` [16]) and (b) inside `text/html` as a base64 `data-everink`
  attribute on a wrapper element, the path every browser and native clipboard keeps. On paste the core looks for
  (a), then (b), then `image/*`, then `text/plain`.
- **Paste rule (08 owns it; this section implements it).** Foreign HTML on the clipboard is never parsed, inserted
  or rendered; only its plain-text sibling and images are used. Everink's own objects travel in a custom clipboard
  type and in a `data-everink` attribute inside the HTML item, found by a bounded byte scan, never by an HTML
  parser. The payload is untrusted data: its size limit is the clipboard-payload row of section 08's limits table,
  it is parsed by the same validating decoder as network data, and it is rejected on version mismatch.
- **Web.** Copy and cut run inside the `copy` and `cut` events with `clipboardData.setData` (synchronous, no
  permission); the menu and toolbar Copy use `navigator.clipboard.write` inside a click. Paste runs inside the
  `paste` event (`clipboardData.getData` and `.files`, synchronous). Only the menu Paste uses the async
  `navigator.clipboard.read()`: Chromium prompts once, Firefox and Safari show a "Paste" bubble [16]; if denied the
  UI says "Use Ctrl/Cmd+V". In an embedder's iframe the host must allow `clipboard-read; clipboard-write` [16].
- **Native.** `arboard` 3.7.0 [1] for text, images and HTML; custom-format support is unverified, so the payload
  travels in the HTML item as on the web, which makes copy between desktop and web builds work.
- **Files.** Dropped or pasted files arrive as `DropEvent { files, pos }` (web `File` objects, native paths via
  winit `DroppedFile`/`HoveredFile`). The core calls `read_dropped` with the size cap from 08, then 05 takes over.
  `FilesHovered` drives a drop-target highlight. Dragging out of Everink is not in the first versions.

### Local storage

Namespaces: `doc` (change-log chunks and snapshots per surface, 03), `outbox` (changes not yet acknowledged),
`cache` (other users' objects, safe to lose), `meta` (surface list, settings, saved places), `blob`
(content-addressed files), `uploads` (pending uploads with their progress, 05) and `tiles` (the overview-tile cache,
05). `uploads` is durable like `outbox`; `cache`, `blob` and `tiles` are evictable.

- **Native.** `redb` 4.5.0 [1]: embedded, transactional, pure Rust, one file in the OS data directory; `batch` is
  one write transaction. Blobs are files named by hash in a sibling directory, written to a temporary name and
  renamed. Throughput at 100,000 strokes is unmeasured and goes into the spike.
- **Web.** Records in IndexedDB (one object store per namespace; `batch` is one `readwrite` transaction, atomic);
  blobs and pending uploads in OPFS through a worker using `createSyncAccessHandle` (write to a temporary file,
  flush, rename). A thin `web-sys` wrapper is preferred to a third-party IndexedDB crate; none was evaluated.
- **Quota and eviction.** Web storage is best effort until `navigator.storage.persist()` is granted; users can clear
  it, and clearing deletes OPFS [16]. `outbox` and `uploads` are the irreplaceable data. Rules: (1) call
  `request_persistence()` the first time the outbox is non-empty and show the result in the connection status; (2)
  keep `cache`, `blob` and `tiles` under a budget (256 MB web, 2 GB native, starting values) by least-recently-used
  eviction at startup and after each blob write; (3) check `usage()` before a large write; (4) on
  `StorageError::QuotaExceeded` evict evictable namespaces first, never `outbox` or `uploads`; if the outbox itself
  cannot fit, the UI says "Unsaved changes cannot be kept offline; stay connected"; (5) Safari's cap on
  script-written storage for sites not visited for a while (7 days) is unverified and on the test list, as it
  affects offline claims.
- **A stroke in progress when the connection or the app dies** (owner decision, 2026-11-12: nothing is lost unless
  the device itself is lost). Drawing never needs the network: the stroke carries on locally and the preview fades
  on other screens (04). While a stroke is being drawn, online or offline, the points so far are saved to the
  durable `outbox` at most every 250 ms (a starting value), as an unfinished-stroke record. When the pen lifts the
  core replaces that record with the finished stroke as a normal `Change`, which is sent as usual, or on reconnect.
  If the app or tab dies first, the next start finds the unfinished record, finishes the stroke as if the pen had
  lifted, and sends it once (the record and the change share one `ClientChangeId`, so a repeat is refused by 03's
  per-client resubmission check). At most the last 250 ms of a stroke can be lost. The server never commits a stroke
  on someone's behalf.
- **Contract.** All implementations (memory, redb, IndexedDB, OPFS) pass one test suite; memory is also what the
  server's headless core uses.
- **Sign-out.** `Command::SignOut { force }` first tries to send the outbox. If changes remain unsent, and the
  connection is down or refused, the core emits `Notification::UnsentChanges(count)` and does nothing more until the
  embedder or person confirms with `force: true` (a warning in the UI; an embedder decides in its own). After that,
  or when the outbox is empty, it calls `clear_all` for that identity: every namespace and every blob file, plus any
  in-memory token.

### Network and token supply

The shell implements `Transport` (browser `WebSocket`; native `tokio-tungstenite` 0.30.0 [1] with rustls) and
`HttpClient` (`fetch`; native `reqwest` or `hyper`, not evaluated). It reports `Opened` and `Closed(code)`;
reconnect timing is the core's, jittered and persisted across restarts as needed by 03. Tokens (08): the embedder
supplies an async `TokenProvider`, called at connect and when a token is about to expire (effect `GetToken`). There
are no anonymous guests (owner decision, 2026-11-12): everyone is identified by the embedding app, and the first
frame on the socket is only `auth { token }`. An invite address, if the page address carries one, is not used by
Everink: **`take_invite_secret()` (section 08's name) removes the fragment from the visible address
(`history.replaceState`) and returns the secret**. A page usually reads the secret before it has a token, so the
embedding app can call `take_invite_secret()` itself before Everink is mounted; the shell also calls it at mount,
before any `await`, as a safeguard. `Notification::InviteReceived` is the convenience for a page that mounts Everink
first: the shell hands it the secret. In both cases the app signs the person in, redeems the invite with Everink
(08) and issues connect tokens that carry the granted role and the link's id. The embedding app has two further
duties when it supplies tokens, which 08 defines: when it redeems an invite it tells Everink whether the person
already has access it granted directly (`has_direct`), and the first time it grants direct access to a person who
came only through a link, the token it issues carries `direct: true`; otherwise Everink refuses that token. Size and
rate limits (message sizes, upload sizes, clipboard payload) are in 08's limits table; this section names them and
carries no numbers of its own.

### The embedding API

Commands in, notifications out, the same shape in Rust and TypeScript. The enums are the stable contract, versioned
(`api_version`) and `#[non_exhaustive]`.

```rust
#[non_exhaustive]
pub enum Command {
    SetTool(Tool), SetColor(Color), SetStrokeWidth(f32), SetFill(Option<Color>),
    Undo, Redo,
    Camera(CameraCommand),                   // ZoomBy{factor, anchor} -> Camera::zoom_about, PanBy{dx, dy} -> pan_by_px, FitContent -> zoom_to_fit, GoHome/GoToPlace(SavedPlace)/GoToPerson(UserId) -> Camera::set
    Selection(SelectionCommand),             // SelectAll, Clear, Delete, Duplicate, Group, Ungroup, Order(..)
    Copy, Cut, Paste,
    PlaceTemplate { library: LibraryId, template: TemplateId },
    InsertText(String), InsertImage(ImageSource),
    SaveVersion { name: String }, OpenVersion(VersionId), RestoreVersion(VersionId), CloseVersion,
    SetReadOnly(bool),                       // embedder-side only; can only lower what the token allows
    SetUiVisibility(UiVisibility), SetTheme(Theme), SetLocale(String) /* "en" only at first */, SetShortcuts(ShortcutMap),
    Connect, Disconnect, SignOut { force: bool },
    Invite(InviteCommand),                   // Create{role, expires, max_redemptions, label}, TurnOff(LinkId),
                                             // ChangeLink{link, label, role, max_redemptions, expires}, ListPeople(LinkId),
                                             // SuspendPeople(LinkId), ResumePeople(LinkId), RemovePerson{link: LinkId, user: UserId},
                                             // DeleteBy(DeleteFilter)  // DeleteFilter::{ByUser(UserId), ByLink(LinkId)}
    Export(ExportRequest),                   // image of the view or selection, via 06
}
#[non_exhaustive]
pub enum Notification {
    Ready, ToolChanged(Tool), ColorChanged(Color), SelectionChanged(SelectionSummary),
    ViewChanged(ViewSummary),                // level, zoom, rough position text; not raw positions
    HistoryChanged { can_undo: bool, can_redo: bool },
    Connection(ConnectionState), Peers(Vec<PeerSummary>), SyncStatus { unsent: usize }, UnsentChanges(usize),
    TokenNeeded, Error(EverinkError), ExportReady(ExportId, Vec<u8>),
    InviteReceived(String), InviteCreated(String),  // the address is for the embedding app
    LinkOpened,                              // a link in a text object was activated; the embedder decides
}
```

- **Rust crate `everink`:** `Everink::new(config, platform)`, `fn command(&mut self, Command)`, `fn subscribe(&mut
  self) -> Receiver<Notification>`, and for hosting inside an existing winit or egui app `fn render(&mut self,
  device, queue, target_view)` (the host keeps its loop and forwards `InputEvent`s). Only the types above are
  re-exported.
- **npm package `@everink/web`:** the `.wasm` loaded with `WebAssembly.instantiateStreaming` and a hand-written,
  versioned TypeScript wrapper over a tiny wasm-bindgen boundary, `dispatch(commandJson)` and a notification
  callback. Hand-written `.d.ts` types, checked against the Rust enums in CI, keep the public API stable when
  internals change.

```ts
const ink = await Everink.mount(containerElement, {
  server: "wss://draw.example.com",                  // no default
  surface: "surface-id",
  getToken: async () => fetchTokenFromMyBackend(),   // 08 defines the token; the embedder signs it
  ui: { controls: "full" | "minimal" | "none", hide: ["invite", "versions"], accessibility: "auto" | "on" },
  theme: { mode: "auto" | "light" | "dark", tokens: {...} },
  locale: "en",
});
ink.command({ type: "setTool", tool: "pen" });
ink.on("selectionChanged", s => ...);
ink.destroy();
```

  Also `<everink-surface server=... surface=...>` (the `getToken` function is set as a property). Mounting uses a
  shadow root so host CSS cannot reach the hidden elements.
- **Hiding controls.** `none` creates no widgets at all; the mirror then holds only the canvas and the objects list,
  and the embedder owns the accessibility of its own controls. `minimal` keeps tool palette, color and undo; `hide`
  removes controls by stable id (`tools`, `color`, `zoom`, `people`, `invite`, `library`, `versions`, `format`,
  `status`, `menu`). Shortcuts keep working unless `SetShortcuts` removes them. Unlike tldraw and Excalidraw [20], a
  host cannot replace one control with its own component; it can hide all and draw its own.
- **Theming.** `Theme` tokens: color roles (surface, panel, text, muted text, accent, danger, focus ring, canvas
  background and grid), corner radius, density, UI font family (a shipped or blob-store font), UI scale, motion.
  Defaults follow the platform appearance. The canvas background is a theme token, not document content.
- **Server and tokens.** `server` has no default (Everink hosts no service). `getToken` is the only way a token
  enters; Everink never writes a token to disk or `localStorage`.
- **Not in the first release:** custom buttons in the toolbar, plug-in tools, replacing a control with host UI
  (questions). No C API now: the enums are serde-based, so a later C API is a thin `everink_command(json) -> json`
  layer made by `cbindgen`; building it before the command set is stable would freeze the wrong thing.

### Packaging

**Web build.** `cargo build --target wasm32-unknown-unknown --release`, then `wasm-bindgen-cli` (pinned to the crate
version, as the two must match) and `wasm-opt -Oz`, driven by an `xtask`. `trunk` (0.22.1 [1]) serves the demo and
development only. `wasm-pack` is not relied on: its home moved when the rustwasm organization was archived [18] and
it builds only libraries. Release profile: `opt-level = "s"` for ui and shell (`3` for the renderer if the spike
shows it matters), `lto = "fat"`, `codegen-units = 1`, `panic = "abort"`. Budget for the step-1 build: `.wasm` at
most 8 MB raw and 2.5 MB brotli (the two seconds on 10 Mbit/s computed above; the raw figure is a guess to replace
by measurement), fonts and shader data excluded and loaded separately. CI fails a build 5 percent over the recorded
size. The browser compiles while downloading when the file is served as `application/wasm`.

**Measured** (built for size, `opt-level = "s"`, link-time optimisation, one codegen unit, `panic = "abort"`, no
`wasm-opt`; `../spike/results-3c-browser.md`, `../spike/results-4a-controls.md`). The renderer for paths and images,
one backend: 0.36 to 0.52 MB raw and 122 to 155 KB with Brotli; all three backends 0.91 MB and 262 KB; no text and
none of the optional shaders, so a product build is larger. The controls: 1.24 MB raw and 366 KB with Brotli for the
own toolkit, 2.42 MB and 616 KB for egui, each without a font; with one font (the own) 1.64 MB and 513 KB, with egui's
three 3.09 MB and 886 KB. The controls' and the renderer's figures are not to be added: the controls' leave out what draws
them (`egui-wgpu` for egui), and a product renderer with text carries Parley again, which the own toolkit's figure
already includes. Loading and compiling the speed build took 45 to 68 ms from a local server.
**SIMD.** The renderer's speed in a browser depends on WebAssembly SIMD (`-C target-feature=+simd128`, not the
`relaxed-simd` feature that section 01 bans): 2.3 to 2.6 times faster, and a module built with it does not load in a
browser without it. A speed build with SIMD is 1.35 MB (290 KB Brotli) for all three backends; without, 1.46 MB (306
KB). **Whether to require SIMD or ship two builds is not decided** (to be decided; see [the
index](README.md#still-open)).

| Need | Header or setting |
|---|---|
| Streaming compile | `Content-Type: application/wasm`; brotli or gzip `Content-Encoding` |
| Content security policy | Section 08's policy, which this shell runs under: `script-src 'self' 'wasm-unsafe-eval'` and nothing looser (no `'unsafe-eval'`, no `'unsafe-inline'`); `worker-src 'self'` (the storage worker is a file of the package); `connect-src 'self'` plus the server's `wss:` and `https:` origins and the blob origin, because the wasm code fetches and decodes images itself; `img-src 'self' blob: data:`; `media-src` the blob origin and `blob:` (for video elements) |
| Clipboard inside an iframe | `Permissions-Policy` and iframe `allow="clipboard-read; clipboard-write"` |
| Cross-origin isolation | **Not required.** Only a future threaded build would need COOP `same-origin` and COEP `require-corp` |
| Secure context | HTTPS (clipboard, OPFS, service worker, WebGPU require it) [16] |
| Caching | content-hashed names, `Cache-Control: immutable`; no service worker at first |

At mount the shell asks the renderer for a backend; only if all of 06's backends fail does it send
`Notification::Error(Unsupported)` with the reason, so the embedder can show its own message.

**Desktop.** One binary per platform: macOS (arm64, x86-64), Windows (x86-64, arm64), Linux (x86-64, arm64; X11 and
Wayland via winit), graphics through wgpu's Metal, Direct3D 12 and Vulkan. Packaging formats (`.app` in a `.dmg`,
MSI or MSIX, AppImage or Flatpak), signing, notarization and store listing are the owner's; the build leaves one
file for version and bundle id and keeps secrets out of the repository.

### Mobile and tablets (later; what must not be ruled out now)

Pens live on tablets, so these constrain the first design:

1. `PointerKind::Pen` is first class (pressure, tilt, twist, hover, barrel button, eraser end); no path assumes a
   mouse. Touch targets are 44 by 44 points and hover-only affordances have a tap or long-press equivalent.
2. `SurfaceEvent::Suspended`, `Resumed`, `Lost`, `Restored` exist from step 0: mobile systems destroy the surface in
   the background and a browser can lose a GPU context. The renderer rebuilds everything from core state.
3. Safe-area and keyboard insets are in `SurfaceInfo` (web: `visualViewport`); platform decoders and text services
   sit behind the traits; the core calls no OS API.
4. The outbox is written at every acknowledged change, which covers an app killed in the background.
5. AccessKit has released Android and iOS adapters [4]. Ink prediction is the core's (06), fed timestamped samples;
   platform prediction is optional. winit has Android and iOS backends [12]; whether they suffice for stylus and
   text services is for the mobile step.

## Failure modes

| Failure | What happens |
|---|---|
| All of 06's renderer backends fail in a browser | `Unsupported` notification with the reason; the embedder shows its message |
| GPU context lost, tab backgrounded, surface dropped | `Lost` then `Restored`; the renderer rebuilds from state; no document data lives only on the GPU |
| The browser blocks the on-screen keyboard (focus not in a gesture) | Prevented by rule 2 and tested in a real browser; a "tap to type" button in the formatting bar focuses from a real click |
| IME composition interrupted by a rewrite, or a remote edit lands in the window | Never rewrite while composing; hold the edit until `compositionend`; map the caret through the change (02) |
| Mirror elements unreachable by explore-by-touch under the canvases (**unverified risk**) | Linear swipe navigation still works; activation by coordinates reaches the controls; checked by the manual mobile screen-reader pass |
| Screen reader turned on mid-session | The mirror is built from a full `TreeUpdate` on activation, not from history |
| Clipboard payload hostile or from another version | Rejected by the validating decoder: "Cannot paste this"; nothing is partly applied |
| Quota exceeded or browser evicts storage | Evict `cache`, `blob`, `tiles` first, never `outbox` or `uploads`; if the outbox is lost the status says what |
| IndexedDB or OPFS unavailable (private window, blocked data) | `Storage` falls back to memory; the status says changes will not be kept if the tab closes |
| Wheel misclassified, or a browser reserves a shortcut | Sticky class and a visible "scroll wheel: zoom or pan" setting; the shortcut table avoids reserved keys and the embedder can disable bindings |
| Two Everink instances in one page, or an embedder that hides all controls | Each instance has its own shadow root, textarea, mirror and storage scope; with controls hidden, shortcuts and the objects list still work and the embedder owns its controls' accessibility |
| A stroke is in progress when the connection or the app dies | The stroke continues locally and its preview fades on other screens. Its points are saved to the `outbox` every 250 ms, online or offline; after an app death the next start finishes it as if the pen had lifted and sends it once. At most the last 250 ms is lost; the server never commits a stroke for the author |
| Sign-out with unsent changes | `UnsentChanges(count)`; nothing is cleared until `force: true` |
| Token provider fails or is slow | `TokenNeeded`; the surface stays usable offline; sync reconnects when a token arrives |

## How it is tested

| Rule | Proved by |
|---|---|
| The core does no I/O and is deterministic | A recorded `(now, Event)` session is replayed twice, native and wasm, with byte-identical `Effect` logs; a lint denies `std::time`, `std::fs`, `std::net` and `rand` in core crates |
| Effects run before the handler returns | Browser test (headless Chromium and Firefox via WebDriver, WebKit nightly): tap a text object and assert in the same event that `document.activeElement` is the textarea and no effect is queued |
| Every effect completes exactly once | Property test of the runtime with a fault-injecting `Platform` (delays, failures, duplicates, reordering between kinds) |
| Stacking works | Browser test with the two canvases over the mirror: pointer events reach the top canvas, textarea and mirror take focus, the controls draw their own focus ring, and `focus()` in a tap raises the keyboard flag; manual VoiceOver (iOS) and TalkBack check of explore-by-touch, the one **unverified** risk (mirror elements under a canvas) and a release blocker for the mobile step |
| Pointer, pen and touch | Eight coalesced samples arrive as eight with pressure, tilt and timestamps unchanged; a mouse has `pressure: None`; hover and leave arrive; scripted pen hover plus touch pans and does not draw |
| Wheel classification | Recorded traces from the listed devices are replayed; 99 percent classified correctly (a starting threshold) |
| IME on the web | Drive composition through Chromium's remote debugging interface (`Input.imeSetComposition`; **works** in Chrome 157, where it raises `compositionupdate` and `compositionend` on the textarea, and the spike's check does this): preedit then commit, a cancelled composition, a remote edit during composition; assert the document text and that the textarea is not rewritten mid-composition. The accessibility tree is read with `Accessibility.getFullAXTree`; no person is needed. These are Chrome's compositions and not a real keyboard's or input method's |
| The web mirror and text bridge, in Chrome | The spike's script (54 checks per candidate) is the model. It must hit-test every mirror element at its control's middle (boxes that are right can still be unreachable), press Tab both ways from nothing and from the elements before and after the controls, check the DOM's focus and the controls' focus are on the same control at every stop, watch which attributes and text are rewritten (keys that change no text rewrite none), and check that its keys arrived (Chrome does not deliver keys to a window that is not in front) |
| Picture tests of the controls | Render the controls to a picture by the painters the window uses and check it, beside the tree tests: five faults of the spike's own were found only by looking (a row wider than the surface, a caret in the wrong place while composing, an unreadable swatch in a dark theme, a focus ring painted over, a focused item drawn as a chosen one), and none shows in a tree. Contrast and focus rings are measured from the pixels |
| IME on desktop and phones | Manual matrix per release for Latin accent composition (dead keys) on macOS, Windows (US-International), Linux (IBus, Fcitx 5; X11 and Wayland), iOS Safari and Android Chrome keyboards; the rows for Japanese, Pinyin and Korean are kept in the list and run when other scripts are added |
| Toolkit comparison | The spike table under Options is filled for both candidates on the same toolbar and hex field; the choice is recorded with the numbers |
| Renderer backends | Browser tests run on the WebGPU and WebGL2 paths; the processor-only path has a smoke test that it opens and a stroke can be drawn, with no performance claim |
| Stroke in progress when the connection or the app dies | Scripted: cut the connection mid-stroke, finish the stroke, restore the connection, and assert one finished stroke arrives once; kill the app mid-stroke while connected, restart, and assert the stroke appears exactly once; kill it offline mid-stroke and assert the journaled points are recovered as a finished stroke |
| Accessibility tree is complete | Property test over the widget arena: every actionable widget has a role and a non-empty name; every `TreeUpdate` is consistent (no node without parent, stable ids) |
| Mirror equals the tree and is safe | Snapshot test of `TreeUpdate` to DOM; `axe-core` over the mirror (unverified fit); a fuzz test with names containing markup asserts they appear as text and no element or attribute is injected (the mirror uses `textContent` only) |
| Screen readers work | Manual pass per release with NVDA, VoiceOver (macOS, iOS), Orca, TalkBack over a fixed script: find tool, change color, hear the objects list, edit text |
| Keyboard-only operation | A table of every command with its key path, each performed by keys alone; a tab-through asserts no trap and a visible focus indicator at every stop |
| Clipboard round trip and paste limits | Copy a mixed selection, paste, compare (ids excepted); paste with the custom type missing, text only, image only; foreign HTML pastes as its text only and is never parsed; fuzz the payload decoder (size, truncation, version, hostile counts) |
| Storage contract | One suite on memory, redb, IndexedDB and OPFS: atomic `batch`, `scan` order, crash between write and rename keeps the old value, quota error leaves `outbox` and `uploads` intact |
| Sign-out | With unsent changes and no connection nothing is cleared until `force`; afterwards every namespace, blob and token is gone |
| Mount-time and delivery rules | The address has no fragment before the first `await` of mount (read `location` from a microtask queued at mount); the demo runs under `script-src 'self' 'wasm-unsafe-eval'` with no violation reports, styles included; it works with only the headers in the Packaging table (no COOP or COEP); the brotli size fails CI above budget plus 5 percent |
| Controls quality | Automated checks over the widget arena: size at least 24 by 24 CSS px (44 by 44 touch), contrast per theme token pair, no two bindings collide per platform and none is browser-reserved |
| Embedding API | TypeScript compile test of the example against the shipped `.d.ts`; `controls: "none"` yields no UI nodes; each `Command` yields its documented `Notification`; the token never reaches storage |

## Dependencies

Latest on crates.io on 2026-11-11 [1] unless noted.

| Crate | Version | License | Maintenance | Why |
|---|---|---|---|---|
| `accesskit` | 0.25.2 | MIT OR Apache-2.0 | Active (2026-10-25) | Accessibility tree types for core, ui and shells |
| `accesskit_winit` | 0.34.2 | Apache-2.0 only | Active | Native adapter, `everink-native` only. Compatible with MIT OR Apache-2.0, but the native binary is then distributed under Apache-2.0 terms; library crates must not depend on it |
| `winit` | 0.31.0-beta.4 (stable 0.30.13) | Apache-2.0 | Active | Native windowing and input; stay on 0.30 until 0.31 is stable |
| `wgpu` | 30.0.1 | MIT OR Apache-2.0 | Active | GPU access (through 06) |
| `taffy` | 0.15.1 | MIT | Active | Layout for candidate 1 |
| `egui`, `egui-wgpu` | 0.37.0 (`egui-wgpu` unchecked) | MIT OR Apache-2.0 | Active; breaking changes each release [2] | Candidate 2, spike only until the choice |
| `web-sys`, `js-sys` | 0.3.107 | MIT OR Apache-2.0 | Active | Direct DOM access |
| `wasm-bindgen`, `wasm-bindgen-cli` | 0.2.130 | MIT OR Apache-2.0 | Active; moved out of rustwasm [18] | Rust to JS boundary and build step; pinned |
| `wasm-bindgen-futures` | 0.4.80 | MIT OR Apache-2.0 | Active | Futures on the browser's single thread |
| `trunk` | 0.22.1 | MIT/Apache-2.0 | Active | Demo and dev server only |
| `tokio` | 1.54.1 | MIT | Active | Native async runtime |
| `tokio-tungstenite` | 0.30.0 | MIT | Active | Native WebSocket |
| `arboard` | 3.7.0 | MIT OR Apache-2.0 | Active | Native clipboard |
| `redb` | 4.5.0 | MIT OR Apache-2.0 | Active | Native embedded store |
| `vello_gpu`, `vello_cpu` | 0.4.0 | Apache-2.0 OR MIT (unverified for these crates) | Active | Via 06: scene API behind its `Backend` trait |
| `parley` | 0.12.0 | Apache-2.0 OR MIT | Active | Via 06: the one text stack |
| `serde`, `postcard` or `ciborium`, `smallvec` | not checked | MIT OR Apache-2.0 each (unverified) | | Command and clipboard encoding |

Considered and not used: xilem and masonry 0.4.0 (Apache-2.0), iced 0.14.0 (MIT), floem 0.2.0 (MIT), gpui 0.2.2
(Apache-2.0), makepad-widgets 1.0.0 (MIT OR Apache-2.0), blitz-dom 0.3.0-beta.4 (MIT OR Apache-2.0), slint 1.18.1
(GPL-3.0-only OR Slint royalty-free OR Slint commercial; **incompatible with an MIT OR Apache-2.0 library** because
embedders would inherit its conditions). Test-only tools such as `axe-core` (MPL-2.0, unverified) are not shipped.

## What this assumes of other sections

- **01 (camera):** 01's names, as in its "API by name" table: `Camera::pan_by_px`, `zoom_about`, `zoom_to_fit`,
  `set` (the embedder commands for home, a saved place and a person all end in `set` with a `Position` or
  `SavedPlace`), `clamp`, `normalize`, `Camera::view_rect` giving the `ViewRect` the sync layer sends, and
  `Camera::scale`. The level and zoom text ("Level 3, 12x") with a home-direction hint is built from `Camera::scale`
  and `SavedPlace`; the objects list is ordered with `Position::sub` and `cmp_axis`, and all use `Position`.
- **02 (document, undo):** every editing `Command` becomes a `Change` of `Op`s (`Op::Create`, `Set`, `SetSlot`,
  `Text`, `DeleteBy`), the same path tools use; a move or resize writes the `Xform` register; per-user undo behind
  `Undo` and `Redo` (`UndoEntry`); a text object gives `surrounding(window)` and a caret mapping through changes via
  `TextDoc`; alt text is a property; an `EverinkClip` can be built from, and applied as, a `Change`. Pending changes
  survive restarts in the `outbox` namespace.
- **03 (sync, server):** one binary WebSocket with the adaptive pacing above; the client sends the exact, unpadded
  view (`ViewMsg` holding a `ViewRect`) and, while the view moves, at least every 100 ms however small the move, so
  presence routing never works from a stale view (the core asks for this timer through `next_wake`); reconnect,
  jitter and resend are the core's, with the outbox persisted through `Storage::batch`; version commands map to its
  API.
- **04 (presence):** cursors, selections and live ink are drawn by the core overlay; the shell supplies pointer,
  hover and leave activity and `Hidden`/`Visible`; this section supplies the people-list panel from 04's data (name,
  color, whether the person can be located, who follows them, view summary).
- **05 (media):** `HttpClient` with `Range` and streaming `PATCH`, the shell-side implementation of 05's
  `PlatformVideoPlayer` (`open`, `set_rate`, `seek`, `position_us`, `output_latency_us`, `next_frame`; sound blocked
  for lack of user activation is reported as `Completed::AudioBlocked` and the controls then show the unmute
  control), the `uploads` and `tiles` stores, `user_activation()` and the unmute control are provided here; 05
  defines `DecodeRequest`, `VideoOp`, and pasted-media rules (no interface fonts are fetched).
- **06 (rendering):** a scene API for `SceneFragment` painted in the same frame as the canvas; the shared text
  service (Parley); the wgpu device; `request_frame()` and the pen-sample stream (coalesced samples, timestamps)
  delivered by this section; the `desynchronized` canvas it asks for through `live_ink_target`; selection-to-PNG;
  and the `Backend` fallback chain (`vello_gpu` on WebGPU, then WebGL2, then `vello_cpu`).
- **08 (identity):** the token is an opaque string through `TokenProvider`; the invite address and its redemption by
  the embedding app, with connect tokens that carry the link id; `Capabilities` per role for disabling controls;
  every limit (media, clipboard, message sizes) in its table; the rules for who may create, delete or restore
  versions.

## Questions for the owner

Answered by the owner on 2026-11-11 and 2026-11-12 and now stated in the Design: browser support levels, the first
app (a whiteboard), the accessibility commitment, interface language and scripts, and the identity model without
guests. Still open:

1. **How far may embedders customize the controls?** First release: hide-and-command only. Custom toolbar buttons,
   plug-in tools or replaced controls cost a large API surface. Recommendation: wait for the first embedder's
   request.
2. **Desktop platforms and signing.** Platforms first, signing identities, store or direct download. Recommendation:
   macOS and Windows first (pen tablets), Linux as a tarball.
3. **Mobile and tablet timing.** When, and native apps or the web build only? Nothing here prevents either.
4. **Package names and the npm scope** (`@everink/web`, `everink`, `everink-runtime`): the owner's; placeholders
   here.

Open because of the spike (the choices are the owner's and are not made here):

5. **Which controls toolkit.** The spike has built both and measured them (see "Toolkit for the shared controls"); the
   screen-reader and real-keyboard checks are the owner's to run.
6. **WebAssembly SIMD: required, or two builds.** See "Packaging" and section 06.

## Sources

Fetched or searched on 2026-11-11. "Search result" means I saw the entry and a summary but did not open the page;
claims resting on those are marked.

[1] crates.io API, `https://crates.io/api/v1/crates/<name>` for egui, xilem, masonry, iced, accesskit, accesskit_winit,
    winit, wgpu, slint, floem, gpui, makepad-widgets, blitz-dom, vello, parley, arboard, wasm-bindgen, web-sys,
    wasm-bindgen-futures, taffy, redb, tokio, tokio-tungstenite, trunk: versions, licenses, update dates.
[2] egui README, https://github.com/emilk/egui
[3] Xilem README, https://github.com/linebender/xilem
[4] AccessKit README, https://github.com/AccessKit/accesskit
[5] MDN, EditContext API, https://developer.mozilla.org/en-US/docs/Web/API/EditContext_API
[6] Can I use, EditContext, https://caniuse.com/mdn-api_editcontext (read 2026-11-11)
[8] Mozilla Bugzilla 2046107, "Ship DOM-based EditContext", https://bugzilla.mozilla.org/show_bug.cgi?id=2046107
[9] eframe web text agent source (search result), https://docs.rs/crate/eframe/latest/source/src/web/text_agent.rs
[10] Flutter, "Accessibility in Flutter on the Web", https://flutter.dev/blog/accessibility-in-flutter-on-the-web
[11] Google Docs canvas rendering (search results, not opened):
    https://workspaceupdates.googleblog.com/2021/05/Google-Docs-Canvas-Based-Rendering-Update.html ,
    https://webaim.org/blog/seismic-change-to-docs/
[12] winit README, https://github.com/rust-windowing/winit
[13] GPUI README, https://github.com/zed-industries/zed/tree/main/crates/gpui
[14] Linebender post-July-2024 roadmap notes (search result), https://linebender.org/wiki/long-term-roadmap/
[15] Blitz, https://github.com/DioxusLabs/blitz ; Floem, https://github.com/lapce/floem
[16] MDN: Origin private file system,
    https://developer.mozilla.org/en-US/docs/Web/API/File_System_API/Origin_private_file_system ; Clipboard API,
    https://developer.mozilla.org/en-US/docs/Web/API/Clipboard_API ; PointerEvent.getCoalescedEvents,
    https://developer.mozilla.org/en-US/docs/Web/API/PointerEvent/getCoalescedEvents
[17] Slint license (search result), https://components.espressif.com/components/slint/slint/versions/1.17.0/license?language=en
[18] Rust blog, "Sunsetting the rustwasm GitHub org" (search result),
    https://blog.rust-lang.org/inside-rust/2025/07/21/sunsetting-the-rustwasm-github-org
[20] tldraw license key, https://tldraw.dev/sdk-features/license-key ; Excalidraw props,
    https://docs.excalidraw.com/docs/@excalidraw/excalidraw/api/props (search results; method lists not read)
[21] W3C EditContext explainer (search result), https://w3c.github.io/editing/docs/EditContext/explainer.html
[22] Makepad 1.0 discussion (community comments, unverified),
    https://axebps-redlib.hf.space/r/rust/comments/1kllldg/makepad_10_rust_ui_framework/ms8gmz6/
[23] Iced forum, https://discourse.iced.rs/t/allowing-users-to-compile-wgpu-without-the-webgl-flag-in-wasm-builds/59
[24] Chrome, HTML-in-canvas origin trial (search result), https://developer.chrome.com/blog/html-in-canvas-origin-trial

Unverified: WCAG 2.2 criterion numbers and the WAI-ARIA authoring patterns; winit 0.31 IME API names, pen tilt per
OS and Windows/Linux touchpad pinch; Safari support for coalesced and predicted pointer events and its gesture
events; Safari storage eviction; `WebSocketStream`; the Keyboard Lock API; `arboard`
custom formats; constructable stylesheets under a strict `style-src`; whether explore-by-touch reaches mirror
elements under a canvas; `redb` at 100,000 strokes; Figma and Miro mirrors; Linux IME
framework coverage; `axe-core` on mirrored DOM; licenses of serde, postcard, ciborium, smallvec and axe-core;
streaming request bodies in `fetch` on each browser; the egui issues listed (not opened; the text agent is not used
when egui is driven through Everink's shell, see "Toolkit for the shared controls"); Makepad, Iced and GPUI web state.

**Settled by the spike** (`../spike/results-3c-browser.md`, `../spike/results-4a-controls.md`,
`../spike/results-4b-window.md`, `../spike/results-4b-web.md`; one Apple M4, Chrome 157).
- `Input.imeSetComposition` drives a real composition in Chrome 157 (it was unverified).
- The size of WebAssembly modules for the renderer (wgpu plus Vello, paths and images only) and for each controls
  candidate; frame cost of each candidate; that egui 0.37 can give containers roles and names.
- That a mirror built from the AccessKit tree, a textarea and the DOM's focus work for both candidates in Chrome.
- Still unverified: a screen reader, a real keyboard and input method, other browsers, explore by touch, the
  platform adapters on Windows and Linux, and the maintenance claims.
