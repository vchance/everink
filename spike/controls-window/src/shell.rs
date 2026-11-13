//! What the native window and the browser page share: one of the two candidates with what draws it, and the toy
//! document the controls act on.

use controls_trial::own::OwnControls;
use controls_trial::with_egui::EguiControls;
use controls_trial::{Command, Controls, Tool, UiFrame, UiModel};

use crate::paint::{EguiPainter, Gpu, OwnPainter};

/// One of the two, with what draws it.
#[allow(
    clippy::large_enum_variant,
    reason = "there is one of these in the program"
)]
pub enum Candidate {
    Own(OwnControls),
    Egui(EguiControls),
}

#[allow(
    clippy::large_enum_variant,
    reason = "there is one of these in the program"
)]
pub enum Painter {
    Own(OwnPainter),
    Egui(EguiPainter),
}

impl Candidate {
    pub fn new(name: &str, scale: f32) -> Candidate {
        match name {
            "own" => Candidate::Own(OwnControls::default()),
            "egui" => {
                let mut controls = EguiControls::default();
                controls.keep_paint(scale);
                Candidate::Egui(controls)
            }
            other => panic!("the candidate is `own` or `egui`, not {other}"),
        }
    }

    pub fn controls(&mut self) -> &mut dyn Controls {
        match self {
            Candidate::Own(c) => c,
            Candidate::Egui(c) => c,
        }
    }

    pub fn painter(&self, gpu: &Gpu, format: wgpu::TextureFormat, scale: f32) -> Painter {
        match self {
            Candidate::Own(_) => Painter::Own(OwnPainter::new(gpu, format, scale)),
            Candidate::Egui(_) => Painter::Egui(EguiPainter::new(gpu, format)),
        }
    }

    pub fn paint(
        &mut self,
        painter: &mut Painter,
        gpu: &Gpu,
        view: &wgpu::TextureView,
        scale: f32,
        frame: &UiFrame,
    ) {
        match (self, painter) {
            (Candidate::Own(c), Painter::Own(p)) => {
                p.paint(gpu, view, scale, c.paint(), frame.text_input.as_ref())
            }
            (Candidate::Egui(c), Painter::Egui(p)) => {
                if let Some(paint) = c.take_paint() {
                    p.paint(gpu, view, paint);
                }
            }
            _ => unreachable!("a candidate is drawn by its own painter"),
        }
    }
}

/// The document the controls act on, as far as the trial has one: a tool, a colour, and what can be undone.
pub struct Document {
    pub model: UiModel,
    /// Every command the controls gave, as text, for a shell that shows them.
    pub log: Vec<String>,
    past: Vec<(Tool, controls_trial::Rgb)>,
    future: Vec<(Tool, controls_trial::Rgb)>,
}

impl Document {
    pub fn new() -> Document {
        Document {
            model: UiModel {
                tool: Tool::Pen,
                colour: [0x20, 0x20, 0x20],
                can_undo: false,
                can_redo: false,
                zoom_percent: 100,
            },
            log: Vec::new(),
            past: Vec::new(),
            future: Vec::new(),
        }
    }

    pub fn apply(&mut self, commands: &mut Vec<Command>) {
        for command in commands.drain(..) {
            let now = (self.model.tool, self.model.colour);
            match command {
                Command::SetTool(tool) if tool != self.model.tool => {
                    self.past.push(now);
                    self.future.clear();
                    self.model.tool = tool;
                }
                Command::SetColour(colour) if colour != self.model.colour => {
                    self.past.push(now);
                    self.future.clear();
                    self.model.colour = colour;
                }
                Command::Undo => {
                    if let Some((tool, colour)) = self.past.pop() {
                        self.future.push(now);
                        (self.model.tool, self.model.colour) = (tool, colour);
                    }
                }
                Command::Redo => {
                    if let Some((tool, colour)) = self.future.pop() {
                        self.past.push(now);
                        (self.model.tool, self.model.colour) = (tool, colour);
                    }
                }
                _ => {}
            }
            println!("{command:?}");
            self.log.push(format!("{command:?}"));
        }
        self.model.can_undo = !self.past.is_empty();
        self.model.can_redo = !self.future.is_empty();
    }
}

/// The accessibility tree in the window's physical pixels. egui's root node already scales; the own toolkit's
/// gives logical pixels, so the shell puts the scale on its root.
pub fn tree_for_platform(
    mut tree: accesskit::TreeUpdate,
    own: bool,
    scale: f32,
) -> accesskit::TreeUpdate {
    if let (true, Some(info)) = (own, &tree.tree) {
        let root = info.root;
        if let Some((_, node)) = tree.nodes.iter_mut().find(|(id, _)| *id == root) {
            node.set_transform(accesskit::Affine::scale(f64::from(scale)));
        }
    }
    tree
}
