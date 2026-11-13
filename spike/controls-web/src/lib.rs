//! Spike question 12, part 4b, the browser: the two controls candidates on a canvas.
//!
//! The page (`www/index.html`) is the web shell. It turns the browser's events into the trial's own and calls in
//! here; each `frame` draws the candidate on the canvas and gives back, as JSON, the accessibility tree and the
//! state of the text field, from which the page keeps its DOM mirror and its hidden textarea. Section 07 has
//! the shell in Rust; for the trial the part that touches the DOM is in the page, where it can be read.

#[path = "../../controls-window/src/paint.rs"]
mod paint;
#[allow(
    dead_code,
    reason = "the page has no platform adapter to scale the tree for"
)]
#[path = "../../controls-window/src/shell.rs"]
mod shell;

use accesskit::{Action, ActionData, ActionRequest, NodeId, Toggled, TreeId};
use controls_trial::{InputEvent, Key, TextEvent};
use paint::{Gpu, physical};
use serde_json::{Value, json};
use shell::{Candidate, Document, Painter};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct Shell {
    candidate: Candidate,
    painter: Painter,
    gpu: Gpu,
    surface: wgpu::Surface<'static>,
    document: Document,
    scale: f32,
    logged: usize,
}

fn err(e: impl std::fmt::Debug) -> JsValue {
    JsValue::from_str(&format!("{e:?}"))
}

#[wasm_bindgen]
impl Shell {
    /// `name` is `own` or `egui`. The canvas must already have the surface's size in physical pixels.
    pub async fn create(
        name: String,
        canvas: web_sys::HtmlCanvasElement,
        scale: f32,
    ) -> Result<Shell, JsValue> {
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
            .map_err(err)?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .map_err(err)?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("controls-web"),
                ..Default::default()
            })
            .await
            .map_err(err)?;
        let (w, h) = physical(scale);
        let mut config = surface
            .get_default_config(&adapter, w, h)
            .ok_or("the canvas cannot be configured")?;
        // `vello_gpu` as used here does not convert to sRGB on the way out.
        if let Some(format) = surface
            .get_capabilities(&adapter)
            .formats
            .into_iter()
            .find(|f| !f.is_srgb())
        {
            config.format = format;
        }
        surface.configure(&device, &config);
        let gpu = Gpu { device, queue };
        let candidate = Candidate::new(&name, scale);
        let painter = candidate.painter(&gpu, config.format, scale);
        Ok(Shell {
            candidate,
            painter,
            gpu,
            surface,
            document: Document::new(),
            scale,
            logged: 0,
        })
    }

    fn send(&mut self, event: InputEvent) {
        let mut commands = Vec::new();
        self.candidate
            .controls()
            .handle(&event, &self.document.model, &mut commands);
        self.document.apply(&mut commands);
    }

    /// `kind`: 0 moved, 1 pressed, 2 released. In CSS pixels from the canvas's corner.
    pub fn pointer(&mut self, kind: u8, x: f32, y: f32) {
        self.send(match kind {
            1 => InputEvent::PointerDown { x, y },
            2 => InputEvent::PointerUp { x, y },
            _ => InputEvent::PointerMove { x, y },
        });
    }

    /// A key by its DOM name (`KeyboardEvent.key`). Returns whether it is one the controls know.
    pub fn key(&mut self, name: &str, shift: bool) -> bool {
        let key = match name {
            "Tab" => Key::Tab,
            "ArrowLeft" => Key::Left,
            "ArrowRight" => Key::Right,
            "Home" => Key::Home,
            "End" => Key::End,
            "Enter" => Key::Enter,
            " " => Key::Space,
            "Escape" => Key::Escape,
            "Backspace" => Key::Backspace,
            "Delete" => Key::Delete,
            _ => return false,
        };
        self.send(InputEvent::Key { key, shift });
        true
    }

    pub fn preedit(&mut self, text: String) {
        self.send(InputEvent::Text(TextEvent::Preedit(text)));
    }

    pub fn commit(&mut self, text: String) {
        self.send(InputEvent::Text(TextEvent::Commit(text)));
    }

    pub fn cancel_composition(&mut self) {
        self.send(InputEvent::Text(TextEvent::CompositionCancelled));
    }

    /// What the mirror was asked for: `click`, `focus`, or `setValue` with a value. The node is the id the
    /// tree gave, as text (the ids do not fit a JavaScript number).
    pub fn action(&mut self, node: &str, kind: &str, value: Option<String>) -> Result<(), JsValue> {
        let target_node = NodeId(node.parse::<u64>().map_err(err)?);
        let (action, data) = match kind {
            "click" => (Action::Click, None),
            "focus" => (Action::Focus, None),
            "setValue" => (
                Action::SetValue,
                Some(ActionData::Value(value.unwrap_or_default().into())),
            ),
            other => return Err(err(format!("no such action: {other}"))),
        };
        self.send(InputEvent::Action(ActionRequest {
            action,
            target_tree: TreeId::ROOT,
            target_node,
            data,
        }));
        Ok(())
    }

    /// Draws a frame on the canvas and returns, as JSON, the accessibility tree, the text field's state while
    /// it has the focus, and the commands given since the last call.
    pub fn frame(&mut self) -> Result<String, JsValue> {
        // Two frames when commands come out: egui answers an event in the frame after it, and the model may
        // change with the answer.
        let mut commands = Vec::new();
        let mut frame = self
            .candidate
            .controls()
            .frame(&self.document.model, &mut commands);
        if !commands.is_empty() {
            self.document.apply(&mut commands);
            frame = self
                .candidate
                .controls()
                .frame(&self.document.model, &mut commands);
            self.document.apply(&mut commands);
        }
        let target = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            other => return Err(err(other)),
        };
        let view = target
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.candidate
            .paint(&mut self.painter, &self.gpu, &view, self.scale, &frame);
        self.gpu.queue.present(target);

        let nodes: Vec<Value> = frame
            .tree
            .nodes
            .iter()
            .map(|(id, node)| {
                let actions: Vec<&str> = [
                    (Action::Click, "click"),
                    (Action::Focus, "focus"),
                    (Action::SetValue, "setValue"),
                ]
                .into_iter()
                .filter(|(a, _)| node.supports_action(*a))
                .map(|(_, n)| n)
                .collect();
                json!({
                    "id": id.0.to_string(),
                    "role": format!("{:?}", node.role()),
                    "label": node.label(),
                    "value": node.value(),
                    "checked": node.toggled().map(|t| t == Toggled::True),
                    "disabled": node.is_disabled(),
                    // In CSS pixels: both candidates give their bounds in logical pixels.
                    "bounds": node.bounds().map(|b| [b.x0, b.y0, b.x1, b.y1]),
                    "children": node.children().iter().map(|c| c.0.to_string()).collect::<Vec<_>>(),
                    "actions": actions,
                })
            })
            .collect();
        let new_commands: Vec<&String> = self.document.log[self.logged..].iter().collect();
        let out = json!({
            "root": frame.tree.tree.as_ref().map(|t| t.root.0.to_string()),
            "focus": frame.tree.focus.0.to_string(),
            "nodes": nodes,
            "textInput": frame.text_input.as_ref().map(|t| json!({
                "value": t.value,
                "preedit": t.preedit,
                "shown": t.shown,
                "caret": t.caret_rect,
            })),
            "commands": new_commands,
            "tool": format!("{:?}", self.document.model.tool),
            "colour": controls_trial::hex(self.document.model.colour),
        })
        .to_string();
        self.logged = self.document.log.len();
        Ok(out)
    }
}
