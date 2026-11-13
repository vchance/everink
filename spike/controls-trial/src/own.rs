//! Candidate 1: the toolbar and the hex field with a small toolkit of Everink's own, as section 07 sketches it:
//! a retained set of widgets laid out by `taffy`, text measured and edited with `parley`, and an AccessKit tree
//! written by hand. It reacts to an event at once, and lays out and paints again only when something changed.
//!
//! What it paints is a list of rectangles and text runs for a renderer to draw; drawing them is part 4b.

use std::collections::HashMap;

use accesskit::{Action, ActionData, Node, NodeId, Role, Toggled, TreeId, TreeInfo, TreeUpdate};
use parley::{FontContext, GenericFamily, LayoutContext, PlainEditor, StyleProperty};
use taffy::prelude::*;

use crate::{
    Command, Controls, InputEvent, Key, Rgb, SURFACE, SWATCHES, TextEvent, TextInputState, Tool,
    UiFrame, UiModel, hex, parse_hex,
};

const FONT_SIZE: f32 = 14.0;
const FIELD_CHARS: usize = 6;
const FIELD_PAD: f32 = 6.0;

/// A control that can be pointed at, focused and acted on.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Item {
    Tool(usize),
    Swatch(usize),
    Undo,
    Redo,
    Field,
}

const ROOT: NodeId = NodeId(1);
const TOOLBAR: NodeId = NodeId(2);
const TOOL_GROUP: NodeId = NodeId(3);
const COLOUR_GROUP: NodeId = NodeId(4);
const ZOOM: NodeId = NodeId(5);

impl Item {
    fn id(self) -> NodeId {
        NodeId(match self {
            Item::Tool(i) => 10 + i as u64,
            Item::Swatch(i) => 20 + i as u64,
            Item::Undo => 30,
            Item::Redo => 31,
            Item::Field => 32,
        })
    }

    fn from_id(id: NodeId) -> Option<Item> {
        Some(match id.0 {
            n @ 10..=14 => Item::Tool(n as usize - 10),
            n @ 20..=25 => Item::Swatch(n as usize - 20),
            30 => Item::Undo,
            31 => Item::Redo,
            32 => Item::Field,
            _ => return None,
        })
    }
}

/// Left, top, right, bottom in surface pixels.
type Rect = [f32; 4];

fn contains(r: &Rect, x: f32, y: f32) -> bool {
    x >= r[0] && x < r[2] && y >= r[1] && y < r[3]
}

fn ak(r: &Rect) -> accesskit::Rect {
    accesskit::Rect::new(r[0].into(), r[1].into(), r[2].into(), r[3].into())
}

/// One thing to draw.
#[derive(Clone, PartialEq, Debug)]
pub enum Paint {
    Fill {
        rect: Rect,
        radius: f32,
        rgba: [u8; 4],
    },
    Outline {
        rect: Rect,
        radius: f32,
        rgba: [u8; 4],
    },
    Text {
        at: [f32; 2],
        text: String,
        rgba: [u8; 4],
    },
}

/// What was laid out: where everything is.
#[derive(Clone, Default)]
struct Placed {
    items: Vec<(Item, Rect)>,
    toolbar: Rect,
    tool_group: Rect,
    colour_group: Rect,
    zoom: Rect,
}

impl Placed {
    fn rect(&self, item: Item) -> Rect {
        self.items
            .iter()
            .find(|(i, _)| *i == item)
            .map(|(_, r)| *r)
            .unwrap_or_default()
    }

    fn at(&self, x: f32, y: f32) -> Option<Item> {
        self.items
            .iter()
            .find(|(_, r)| contains(r, x, y))
            .map(|(i, _)| *i)
    }
}

pub struct OwnControls {
    fonts: FontContext,
    layouts: LayoutContext<[u8; 4]>,
    editor: PlainEditor<[u8; 4]>,
    widths: HashMap<String, f32>,
    focus: Option<Item>,
    pressed: Option<Item>,
    placed: Placed,
    paint: Vec<Paint>,
    /// What the last layout and paint were made for; while it is the same they are kept.
    made_for: Option<(UiModel, Option<Item>, Option<Item>, parley::Generation)>,
    /// How often the controls were laid out and painted, for the timing run.
    pub rebuilt: usize,
}

impl Default for OwnControls {
    fn default() -> Self {
        let mut editor = PlainEditor::new(FONT_SIZE);
        editor.set_width(None);
        editor
            .edit_styles()
            .insert(StyleProperty::from(GenericFamily::SansSerif));
        #[allow(unused_mut, reason = "without a font nothing is registered")]
        let mut fonts = FontContext::new();
        #[cfg(feature = "own-fonts")]
        {
            // The one font the controls set their text in. The product would register the document's fonts
            // here; the trial borrows one of egui's, so that both candidates set the same type.
            let families = fonts.collection.register_fonts(
                parley::fontique::Blob::from(epaint_default_fonts::UBUNTU_LIGHT.to_vec()),
                None,
            );
            fonts
                .collection
                .set_generic_families(GenericFamily::SansSerif, families.iter().map(|(id, _)| *id));
        }
        OwnControls {
            fonts,
            layouts: LayoutContext::new(),
            editor,
            widths: HashMap::new(),
            focus: None,
            pressed: None,
            placed: Placed::default(),
            paint: Vec::new(),
            made_for: None,
            rebuilt: 0,
        }
    }
}

impl OwnControls {
    /// The list of things to draw, as the last `frame` made it.
    pub fn paint(&self) -> &[Paint] {
        &self.paint
    }

    fn text_width(&mut self, text: &str) -> f32 {
        if let Some(w) = self.widths.get(text) {
            return *w;
        }
        let mut builder = self
            .layouts
            .ranged_builder(&mut self.fonts, text, 1.0, true);
        builder.push_default(StyleProperty::FontSize(FONT_SIZE));
        builder.push_default(GenericFamily::SansSerif);
        let mut layout: parley::Layout<[u8; 4]> = builder.build(text);
        layout.break_all_lines(None);
        // A machine with no fonts measures nothing; the controls still need room for their names.
        let width = layout
            .width()
            .max(text.chars().count() as f32 * FONT_SIZE * 0.5);
        self.widths.insert(text.to_string(), width);
        width
    }

    fn enabled(item: Item, model: &UiModel) -> bool {
        match item {
            Item::Undo => model.can_undo,
            Item::Redo => model.can_redo,
            _ => true,
        }
    }

    /// The places Tab stops at: one for each group (the checked button of it), each enabled button, the field.
    fn stops(model: &UiModel) -> Vec<Item> {
        let tool = Tool::ALL.iter().position(|t| *t == model.tool).unwrap_or(0);
        let swatch = SWATCHES
            .iter()
            .position(|(c, _)| *c == model.colour)
            .unwrap_or(0);
        [
            Item::Tool(tool),
            Item::Swatch(swatch),
            Item::Undo,
            Item::Redo,
            Item::Field,
        ]
        .into_iter()
        .filter(|i| Self::enabled(*i, model))
        .collect()
    }

    fn group(item: Item) -> Option<usize> {
        match item {
            Item::Tool(_) => Some(0),
            Item::Swatch(_) => Some(1),
            Item::Undo => Some(2),
            Item::Redo => Some(3),
            Item::Field => Some(4),
        }
    }

    fn set_focus(&mut self, to: Option<Item>, model: &UiModel) {
        if self.focus == to {
            return;
        }
        if self.focus == Some(Item::Field) {
            // Leaving the field gives up what was being composed and what was typed.
            let mut driver = self.editor.driver(&mut self.fonts, &mut self.layouts);
            driver.clear_compose();
        }
        self.focus = to;
        if to == Some(Item::Field) {
            self.editor.set_text(&hex(model.colour));
            self.editor
                .driver(&mut self.fonts, &mut self.layouts)
                .move_to_text_end();
        }
    }

    fn activate(&mut self, item: Item, model: &UiModel, commands: &mut Vec<Command>) {
        if !Self::enabled(item, model) {
            return;
        }
        match item {
            Item::Tool(i) => commands.push(Command::SetTool(Tool::ALL[i])),
            Item::Swatch(i) => commands.push(Command::SetColour(SWATCHES[i].0)),
            Item::Undo => commands.push(Command::Undo),
            Item::Redo => commands.push(Command::Redo),
            Item::Field => {}
        }
    }

    fn key(&mut self, key: Key, shift: bool, model: &UiModel, commands: &mut Vec<Command>) {
        if key == Key::Tab {
            let stops = Self::stops(model);
            let here = self
                .focus
                .and_then(|f| stops.iter().position(|s| Self::group(*s) == Self::group(f)));
            let next = match (here, shift) {
                (None, false) => stops.first().copied(),
                (None, true) => stops.last().copied(),
                (Some(i), false) => stops.get(i + 1).copied(),
                (Some(i), true) => i.checked_sub(1).map(|j| stops[j]),
            };
            // Past either end the focus leaves the controls, for the canvas to take.
            self.set_focus(next, model);
            return;
        }
        match self.focus {
            None => {}
            Some(Item::Field) => self.field_key(key, model, commands),
            Some(item) => {
                // In a group the arrows move to the next button and check it, as a radio group does.
                let step = |i: usize, n: usize| match key {
                    Key::Right => Some((i + 1) % n),
                    Key::Left => Some((i + n - 1) % n),
                    Key::Home => Some(0),
                    Key::End => Some(n - 1),
                    _ => None,
                };
                let moved = match item {
                    Item::Tool(i) => step(i, Tool::ALL.len()).map(Item::Tool),
                    Item::Swatch(i) => step(i, SWATCHES.len()).map(Item::Swatch),
                    _ => None,
                };
                if let Some(to) = moved {
                    self.focus = Some(to);
                    self.activate(to, model, commands);
                } else if matches!(key, Key::Space | Key::Enter) {
                    self.activate(item, model, commands);
                } else if key == Key::Escape {
                    self.set_focus(None, model);
                }
            }
        }
    }

    fn field_key(&mut self, key: Key, model: &UiModel, commands: &mut Vec<Command>) {
        if self.editor.is_composing() {
            // While text is being composed the keys belong to the input method.
            return;
        }
        let mut driver = self.editor.driver(&mut self.fonts, &mut self.layouts);
        match key {
            Key::Left => driver.move_left(),
            Key::Right => driver.move_right(),
            Key::Home => driver.move_to_text_start(),
            Key::End => driver.move_to_text_end(),
            Key::Backspace => driver.backdelete(),
            Key::Delete => driver.delete(),
            Key::Enter => {
                if let Some(colour) = parse_hex(&self.editor.text().to_string()) {
                    commands.push(Command::SetColour(colour));
                }
            }
            Key::Escape => self.set_focus(None, model),
            Key::Space | Key::Tab => {}
        }
    }

    fn field_text(&mut self, event: &TextEvent) {
        let mut driver = self.editor.driver(&mut self.fonts, &mut self.layouts);
        match event {
            TextEvent::Preedit(text) if text.is_empty() => driver.clear_compose(),
            TextEvent::Preedit(text) => driver.set_compose(text, None),
            TextEvent::CompositionCancelled => driver.clear_compose(),
            TextEvent::Commit(text) => {
                driver.clear_compose();
                let have = driver.editor.text().to_string().chars().count();
                let replaced = driver
                    .editor
                    .selected_text()
                    .map_or(0, |s| s.chars().count());
                if have - replaced + text.chars().count() <= FIELD_CHARS {
                    driver.insert_or_replace_selection(text);
                }
            }
        }
    }

    /// Where everything goes, by `taffy`.
    fn lay_out(&mut self, model: &UiModel) -> Placed {
        let mut tree: TaffyTree<()> = TaffyTree::new();
        let fixed = |w: f32, h: f32| Style {
            size: Size {
                width: length(w),
                height: length(h),
            },
            flex_shrink: 0.0,
            ..Default::default()
        };
        let row = |gap: f32| Style {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::CENTER,
            gap: Size {
                width: length(gap),
                height: zero(),
            },
            ..Default::default()
        };
        let mut leaves: Vec<(Option<Item>, taffy::NodeId)> = Vec::new();
        let mut leaf = |tree: &mut TaffyTree<()>, item: Option<Item>, w: f32, h: f32| {
            let node = tree.new_leaf(fixed(w, h)).expect("a leaf");
            leaves.push((item, node));
            node
        };
        let tools: Vec<taffy::NodeId> = Tool::ALL
            .iter()
            .enumerate()
            .map(|(i, tool)| {
                let w = self.text_width(tool.name()) + 16.0;
                leaf(&mut tree, Some(Item::Tool(i)), w.max(24.0), 28.0)
            })
            .collect();
        let swatches: Vec<taffy::NodeId> = (0..SWATCHES.len())
            .map(|i| leaf(&mut tree, Some(Item::Swatch(i)), 24.0, 24.0))
            .collect();
        let undo_w = self.text_width("Undo") + 16.0;
        let redo_w = self.text_width("Redo") + 16.0;
        let zoom_w = self.text_width(&format!("Zoom {}%", model.zoom_percent));
        let undo = leaf(&mut tree, Some(Item::Undo), undo_w, 28.0);
        let redo = leaf(&mut tree, Some(Item::Redo), redo_w, 28.0);
        let zoom = leaf(&mut tree, None, zoom_w, 20.0);
        let field = leaf(&mut tree, Some(Item::Field), 76.0, 28.0);
        let tool_group = tree.new_with_children(row(2.0), &tools).expect("a row");
        let colour_group = tree.new_with_children(row(6.0), &swatches).expect("a row");
        let root = tree
            .new_with_children(
                Style {
                    size: Size {
                        width: length(SURFACE.0),
                        height: length(SURFACE.1),
                    },
                    padding: length(8.0),
                    ..row(10.0)
                },
                &[tool_group, colour_group, undo, redo, zoom, field],
            )
            .expect("a row");
        tree.compute_layout(
            root,
            Size {
                width: AvailableSpace::Definite(SURFACE.0),
                height: AvailableSpace::Definite(SURFACE.1),
            },
        )
        .expect("a layout");
        let rect = |node: taffy::NodeId, origin: (f32, f32)| -> Rect {
            let l = tree.layout(node).expect("a laid out node");
            let (x, y) = (origin.0 + l.location.x, origin.1 + l.location.y);
            [x, y, x + l.size.width, y + l.size.height]
        };
        let tool_rect = rect(tool_group, (0.0, 0.0));
        let colour_rect = rect(colour_group, (0.0, 0.0));
        let mut placed = Placed {
            toolbar: [0.0, 0.0, SURFACE.0, SURFACE.1],
            tool_group: tool_rect,
            colour_group: colour_rect,
            zoom: rect(zoom, (0.0, 0.0)),
            items: Vec::new(),
        };
        for (item, node) in leaves {
            let origin = match item {
                Some(Item::Tool(_)) => (tool_rect[0], tool_rect[1]),
                Some(Item::Swatch(_)) => (colour_rect[0], colour_rect[1]),
                _ => (0.0, 0.0),
            };
            if let Some(item) = item {
                placed.items.push((item, rect(node, origin)));
            }
        }
        placed
    }

    fn paint_all(&self, model: &UiModel) -> Vec<Paint> {
        const INK: [u8; 4] = [0x20, 0x20, 0x20, 0xff];
        const DIM: [u8; 4] = [0x90, 0x90, 0x90, 0xff];
        const FACE: [u8; 4] = [0xee, 0xee, 0xee, 0xff];
        const ON: [u8; 4] = [0xc8, 0xdc, 0xff, 0xff];
        const RING: [u8; 4] = [0x10, 0x50, 0xe0, 0xff];
        let mut out = vec![Paint::Fill {
            rect: self.placed.toolbar,
            radius: 0.0,
            rgba: [0xfa, 0xfa, 0xfa, 0xff],
        }];
        let label = |out: &mut Vec<Paint>, rect: &Rect, text: &str, rgba: [u8; 4]| {
            out.push(Paint::Text {
                at: [rect[0] + 8.0, rect[1] + 6.0],
                text: text.to_string(),
                rgba,
            });
        };
        for (item, rect) in &self.placed.items {
            match *item {
                Item::Tool(i) => {
                    let on = Tool::ALL[i] == model.tool;
                    out.push(Paint::Fill {
                        rect: *rect,
                        radius: 6.0,
                        rgba: if on { ON } else { FACE },
                    });
                    if on {
                        // Not by colour alone: the chosen tool has a dark outline too.
                        out.push(Paint::Outline {
                            rect: [rect[0] + 1.0, rect[1] + 1.0, rect[2] - 1.0, rect[3] - 1.0],
                            radius: 5.0,
                            rgba: INK,
                        });
                    }
                    label(&mut out, rect, Tool::ALL[i].name(), INK);
                }
                Item::Swatch(i) => {
                    let c: Rgb = SWATCHES[i].0;
                    out.push(Paint::Fill {
                        rect: *rect,
                        radius: 4.0,
                        rgba: [c[0], c[1], c[2], 0xff],
                    });
                    // An outline, so that a swatch near the background's colour is still seen.
                    out.push(Paint::Outline {
                        rect: [rect[0] + 0.5, rect[1] + 0.5, rect[2] - 0.5, rect[3] - 0.5],
                        radius: 4.0,
                        rgba: [0x60, 0x60, 0x60, 0xff],
                    });
                    if c == model.colour {
                        out.push(Paint::Outline {
                            rect: [rect[0] - 2.0, rect[1] - 2.0, rect[2] + 2.0, rect[3] + 2.0],
                            radius: 6.0,
                            rgba: INK,
                        });
                    }
                }
                Item::Undo | Item::Redo => {
                    out.push(Paint::Fill {
                        rect: *rect,
                        radius: 6.0,
                        rgba: FACE,
                    });
                    let ink = if Self::enabled(*item, model) {
                        INK
                    } else {
                        DIM
                    };
                    label(
                        &mut out,
                        rect,
                        if *item == Item::Undo { "Undo" } else { "Redo" },
                        ink,
                    );
                }
                Item::Field => {
                    out.push(Paint::Fill {
                        rect: *rect,
                        radius: 4.0,
                        rgba: [0xff, 0xff, 0xff, 0xff],
                    });
                    out.push(Paint::Outline {
                        rect: *rect,
                        radius: 4.0,
                        rgba: DIM,
                    });
                    out.push(Paint::Text {
                        at: [rect[0] + FIELD_PAD, rect[1] + 6.0],
                        text: self.editor.raw_text().to_string(),
                        rgba: INK,
                    });
                }
            }
            if self.pressed == Some(*item) {
                out.push(Paint::Fill {
                    rect: *rect,
                    radius: 6.0,
                    rgba: [0, 0, 0, 0x20],
                });
            }
        }
        if let Some(focus) = self.focus {
            // The controls draw their own focus ring: on the web the browser's is hidden under the canvas. It is
            // drawn after everything else, so that the control beside the focused one does not cover it.
            let rect = self.placed.rect(focus);
            out.push(Paint::Outline {
                rect: [rect[0] - 3.0, rect[1] - 3.0, rect[2] + 3.0, rect[3] + 3.0],
                radius: 8.0,
                rgba: RING,
            });
        }
        out.push(Paint::Text {
            at: [self.placed.zoom[0], self.placed.zoom[1]],
            text: format!("Zoom {}%", model.zoom_percent),
            rgba: INK,
        });
        out
    }

    fn tree(&self, model: &UiModel) -> TreeUpdate {
        let mut nodes: Vec<(NodeId, Node)> = Vec::new();
        let mut root = Node::new(Role::Window);
        root.set_children(vec![TOOLBAR]);
        nodes.push((ROOT, root));
        let mut toolbar = Node::new(Role::Toolbar);
        toolbar.set_label("Tools");
        toolbar.set_bounds(ak(&self.placed.toolbar));
        toolbar.set_children(vec![
            TOOL_GROUP,
            COLOUR_GROUP,
            Item::Undo.id(),
            Item::Redo.id(),
            ZOOM,
            Item::Field.id(),
        ]);
        nodes.push((TOOLBAR, toolbar));
        let group = |id: NodeId, name: &str, rect: &Rect, children: Vec<NodeId>| {
            let mut node = Node::new(Role::RadioGroup);
            node.set_label(name);
            node.set_bounds(ak(rect));
            node.set_children(children);
            (id, node)
        };
        nodes.push(group(
            TOOL_GROUP,
            "Tool",
            &self.placed.tool_group,
            (0..Tool::ALL.len()).map(|i| Item::Tool(i).id()).collect(),
        ));
        nodes.push(group(
            COLOUR_GROUP,
            "Colour",
            &self.placed.colour_group,
            (0..SWATCHES.len()).map(|i| Item::Swatch(i).id()).collect(),
        ));
        for (item, rect) in &self.placed.items {
            let mut node = match *item {
                Item::Tool(i) => {
                    let mut n = Node::new(Role::RadioButton);
                    n.set_label(Tool::ALL[i].name());
                    n.set_toggled(Toggled::from(Tool::ALL[i] == model.tool));
                    n
                }
                Item::Swatch(i) => {
                    let mut n = Node::new(Role::RadioButton);
                    n.set_label(SWATCHES[i].1);
                    n.set_toggled(Toggled::from(SWATCHES[i].0 == model.colour));
                    n
                }
                Item::Undo | Item::Redo => {
                    let mut n = Node::new(Role::Button);
                    n.set_label(if *item == Item::Undo { "Undo" } else { "Redo" });
                    n
                }
                Item::Field => {
                    let mut n = Node::new(Role::TextInput);
                    n.set_label("Hex colour");
                    // The value is the text without what is being composed.
                    n.set_value(self.editor.text().to_string());
                    n.add_action(Action::SetValue);
                    n
                }
            };
            node.set_bounds(ak(rect));
            if Self::enabled(*item, model) {
                node.add_action(Action::Focus);
                node.add_action(Action::Click);
            } else {
                node.set_disabled();
            }
            nodes.push((item.id(), node));
        }
        let mut zoom = Node::new(Role::Label);
        zoom.set_value(format!("Zoom {}%", model.zoom_percent));
        zoom.set_bounds(ak(&self.placed.zoom));
        nodes.push((ZOOM, zoom));
        TreeUpdate {
            nodes,
            tree: Some(TreeInfo::new(ROOT)),
            tree_id: TreeId::ROOT,
            focus: self.focus.map_or(ROOT, Item::id),
        }
    }
}

impl Controls for OwnControls {
    fn name(&self) -> &'static str {
        "own"
    }

    fn handle(&mut self, event: &InputEvent, model: &UiModel, commands: &mut Vec<Command>) {
        match event {
            InputEvent::PointerMove { .. } => {}
            InputEvent::PointerDown { x, y } => {
                self.pressed = self.placed.at(*x, *y);
            }
            InputEvent::PointerUp { x, y } => {
                let over = self.placed.at(*x, *y);
                if let (Some(item), true) = (over, over == self.pressed)
                    && Self::enabled(item, model)
                {
                    self.set_focus(Some(item), model);
                    self.activate(item, model, commands);
                }
                self.pressed = None;
            }
            InputEvent::Key { key, shift } => self.key(*key, *shift, model, commands),
            InputEvent::Text(text) => {
                if self.focus == Some(Item::Field) {
                    self.field_text(text);
                }
            }
            InputEvent::Action(request) => {
                let Some(item) = Item::from_id(request.target_node) else {
                    return;
                };
                if !Self::enabled(item, model) {
                    return;
                }
                match (&request.action, &request.data) {
                    (Action::Focus, _) => self.set_focus(Some(item), model),
                    (Action::Click, _) => {
                        self.set_focus(Some(item), model);
                        self.activate(item, model, commands);
                    }
                    (Action::SetValue, Some(ActionData::Value(value))) if item == Item::Field => {
                        self.set_focus(Some(item), model);
                        let text: String = value.chars().take(FIELD_CHARS).collect();
                        self.editor.set_text(&text);
                    }
                    _ => {}
                }
            }
        }
    }

    fn frame(&mut self, model: &UiModel, _commands: &mut Vec<Command>) -> UiFrame {
        if self.focus != Some(Item::Field) && self.editor.text() != hex(model.colour).as_str() {
            // Out of focus the field shows the model's colour. In focus it is the user's text, and a colour
            // that changes meanwhile does not rewrite it.
            self.editor.set_text(&hex(model.colour));
        }
        if self.focus.is_some_and(|f| !Self::enabled(f, model)) {
            self.focus = None;
        }
        let key = (*model, self.focus, self.pressed, self.editor.generation());
        if self.made_for != Some(key) {
            self.placed = self.lay_out(model);
            self.paint = self.paint_all(model);
            self.made_for = Some(key);
            self.rebuilt += 1;
        }
        let text_input = (self.focus == Some(Item::Field)).then(|| {
            let field = self.placed.rect(Item::Field);
            self.editor.layout(&mut self.fonts, &mut self.layouts);
            // While text is being composed the editor hides its caret; the place given then is the start of
            // what is being composed, which is where an input method puts its window.
            let caret = self.editor.cursor_geometry(1.5).or_else(|| {
                let area = self.editor.ime_cursor_area();
                Some(parley::BoundingBox {
                    x1: area.x0 + 1.5,
                    ..area
                })
            });
            let (x0, x1) = caret.map_or((0.0, 1.5), |c: parley::BoundingBox| {
                (c.x0 as f32, c.x1 as f32)
            });
            let left = (field[0] + FIELD_PAD + x0).min(field[2] - 1.5);
            let compose = self.editor.raw_compose().clone();
            TextInputState {
                value: self.editor.text().to_string(),
                preedit: compose.map(|r| self.editor.raw_text()[r].to_string()),
                shown: self.editor.raw_text().to_string(),
                caret_rect: [
                    left,
                    field[1] + 4.0,
                    (left + (x1 - x0)).min(field[2]),
                    field[3] - 4.0,
                ],
            }
        });
        UiFrame {
            painted: self.paint.len(),
            tree: self.tree(model),
            text_input,
        }
    }
}
