//! Spike question 12: which controls toolkit?
//!
//! The same toolbar (a tool palette, colour swatches, undo and redo, a zoom indicator) and the same single-line
//! text field (the colour as six hex digits) built twice: with a small toolkit of Everink's own (`own`) and with
//! egui (`with_egui`). Both sit behind the `Controls` trait below, a cut-down form of the one in section 07 of
//! the design, and both run here without a window: events go in, commands and an accessibility tree come out.
//!
//! This is part 4a of the slice. A window, a screen reader and a real keyboard are part 4b.

pub mod contract;
#[cfg(feature = "own")]
pub mod own;
#[cfg(feature = "egui")]
pub mod with_egui;

/// The tools of the palette, in the order they are shown.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Select,
    Pen,
    Eraser,
    Shape,
    Line,
}

impl Tool {
    pub const ALL: [Tool; 5] = [
        Tool::Select,
        Tool::Pen,
        Tool::Eraser,
        Tool::Shape,
        Tool::Line,
    ];

    /// The name a screen reader says. One table of strings, as the design asks, would hold these.
    pub fn name(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Pen => "Pen",
            Tool::Eraser => "Eraser",
            Tool::Shape => "Shape",
            Tool::Line => "Line",
        }
    }
}

/// A colour as red, green and blue.
pub type Rgb = [u8; 3];

/// The swatches, with the names a screen reader says.
pub const SWATCHES: [(Rgb, &str); 6] = [
    ([0x20, 0x20, 0x20], "Black"),
    ([0xd0, 0x30, 0x30], "Red"),
    ([0xe0, 0x90, 0x20], "Orange"),
    ([0x30, 0xa0, 0x50], "Green"),
    ([0x30, 0x60, 0xd0], "Blue"),
    ([0x80, 0x40, 0xb0], "Purple"),
];

pub fn hex(colour: Rgb) -> String {
    format!("{:02X}{:02X}{:02X}", colour[0], colour[1], colour[2])
}

/// Six hex digits, in either case, as a colour.
pub fn parse_hex(text: &str) -> Option<Rgb> {
    if text.len() != 6 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&text[i..i + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// What the controls show. They hold no document state of their own: this comes in with every frame.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct UiModel {
    pub tool: Tool,
    pub colour: Rgb,
    pub can_undo: bool,
    pub can_redo: bool,
    pub zoom_percent: u32,
}

/// What the controls ask for. They change nothing themselves.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
    SetTool(Tool),
    SetColour(Rgb),
    Undo,
    Redo,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Tab,
    Left,
    Right,
    Home,
    End,
    Enter,
    Space,
    Escape,
    Backspace,
    Delete,
}

/// What a platform's text input sends while a text field has the focus (section 07's `TextEvent`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TextEvent {
    /// The text being composed, not yet part of the field: a dead key's accent, or an input method's
    /// candidate. It replaces the preedit before it.
    Preedit(String),
    /// Text to put in the field for good. It replaces the preedit, if there is one.
    Commit(String),
    /// The composition was given up; the preedit goes and nothing is put in.
    CompositionCancelled,
}

#[derive(Clone, PartialEq, Debug)]
pub enum InputEvent {
    PointerMove {
        x: f32,
        y: f32,
    },
    PointerDown {
        x: f32,
        y: f32,
    },
    PointerUp {
        x: f32,
        y: f32,
    },
    Key {
        key: Key,
        shift: bool,
    },
    Text(TextEvent),
    /// What assistive technology asked for, by way of the accessibility tree.
    Action(accesskit::ActionRequest),
}

/// What the platform's text input needs to know while a text field has the focus.
#[derive(Clone, PartialEq, Debug)]
pub struct TextInputState {
    /// The field's text, without the preedit.
    pub value: String,
    /// The text being composed, if any, as the field shows it.
    pub preedit: Option<String>,
    /// Where the caret is, in surface pixels: left, top, right, bottom. An input method puts its window here.
    pub caret_rect: [f32; 4],
}

/// One frame of the controls.
pub struct UiFrame {
    /// How many things were drawn. Each candidate paints in its own form; the trial does not put pixels on a
    /// screen, so this is only a check that a frame drew something.
    pub painted: usize,
    /// The whole accessibility tree.
    pub tree: accesskit::TreeUpdate,
    /// `Some` while a text field has the focus.
    pub text_input: Option<TextInputState>,
}

/// The surface the controls are laid out on, in logical pixels.
pub const SURFACE: (f32, f32) = (800.0, 56.0);

/// Section 07's `Controls`, cut down to what the trial needs.
pub trait Controls {
    fn name(&self) -> &'static str;
    /// Takes one event. Commands it leads to may come out of this call or of the next `frame`.
    fn handle(&mut self, event: &InputEvent, model: &UiModel, commands: &mut Vec<Command>);
    /// Lays out and paints the controls for `model`.
    fn frame(&mut self, model: &UiModel, commands: &mut Vec<Command>) -> UiFrame;
}
