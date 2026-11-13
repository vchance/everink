//! Candidate 2: the toolbar and the hex field with egui 0.37, run without a window.
//!
//! egui is immediate-mode: events are queued and the interface is rebuilt and reacts in the next `frame`, so
//! every command comes out of `frame`, one call after the event that caused it.

use egui::{
    Align, Color32, Event, ImeEvent, Layout, Modifiers, PointerButton, Role, Sense, UiBuilder,
};

use crate::{
    Command, Controls, InputEvent, Key, SURFACE, SWATCHES, TextEvent, TextInputState, Tool,
    UiFrame, UiModel, hex, parse_hex,
};

pub struct EguiControls {
    ctx: egui::Context,
    events: Vec<Event>,
    time: f64,
    /// The field's text. egui edits this string in place, and puts the text being composed into it.
    hex: String,
    /// The text being composed, as last sent to egui. egui does not say which part of the string it is.
    preedit: Option<String>,
    /// Physical pixels to a logical one, for a shell that draws on a real display.
    scale: f32,
    /// `Some` once a shell has asked for what was painted: the last frame's triangles, and the texture changes
    /// since the shell last took them.
    kept: Option<(Vec<egui::ClippedPrimitive>, egui::TexturesDelta)>,
}

/// What a shell needs to draw a frame of egui.
pub struct EguiPaint {
    pub primitives: Vec<egui::ClippedPrimitive>,
    pub textures: egui::TexturesDelta,
    pub pixels_per_point: f32,
}

impl Default for EguiControls {
    fn default() -> Self {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        // The light theme, whatever the machine prefers, so that the two candidates are seen alike.
        ctx.set_theme(egui::ThemePreference::Light);
        EguiControls {
            ctx,
            events: Vec::new(),
            time: 0.0,
            hex: String::new(),
            preedit: None,
            scale: 1.0,
            kept: None,
        }
    }
}

fn key(key: Key) -> egui::Key {
    match key {
        Key::Tab => egui::Key::Tab,
        Key::Left => egui::Key::ArrowLeft,
        Key::Right => egui::Key::ArrowRight,
        Key::Home => egui::Key::Home,
        Key::End => egui::Key::End,
        Key::Enter => egui::Key::Enter,
        Key::Space => egui::Key::Space,
        Key::Escape => egui::Key::Escape,
        Key::Backspace => egui::Key::Backspace,
        Key::Delete => egui::Key::Delete,
    }
}

impl EguiControls {
    /// For a shell that draws: the display's scale, and from now on each frame's paint is kept for `take_paint`.
    pub fn keep_paint(&mut self, scale: f32) {
        self.scale = scale;
        if self.kept.is_none() {
            self.kept = Some((Vec::new(), egui::TexturesDelta::default()));
        }
    }

    /// The last frame's triangles and every texture change since this was last called.
    pub fn take_paint(&mut self) -> Option<EguiPaint> {
        let (primitives, textures) = self.kept.as_mut()?;
        Some(EguiPaint {
            primitives: std::mem::take(primitives),
            textures: std::mem::take(textures),
            pixels_per_point: self.scale,
        })
    }
}

impl Drop for EguiControls {
    fn drop(&mut self) {
        // egui asserts that texture changes are not dropped unapplied; a shell that stops has nowhere to
        // apply them.
        if let Some((_, textures)) = &mut self.kept {
            textures.clear();
        }
    }
}

impl Controls for EguiControls {
    fn name(&self) -> &'static str {
        "egui"
    }

    fn handle(&mut self, event: &InputEvent, _model: &UiModel, _commands: &mut Vec<Command>) {
        match event {
            InputEvent::PointerMove { x, y } => {
                self.events.push(Event::PointerMoved(egui::pos2(*x, *y)));
            }
            InputEvent::PointerDown { x, y } | InputEvent::PointerUp { x, y } => {
                self.events.push(Event::PointerMoved(egui::pos2(*x, *y)));
                self.events.push(Event::PointerButton {
                    pos: egui::pos2(*x, *y),
                    button: PointerButton::Primary,
                    pressed: matches!(event, InputEvent::PointerDown { .. }),
                    modifiers: Modifiers::NONE,
                });
            }
            InputEvent::Key { key: k, shift } => {
                let modifiers = if *shift {
                    Modifiers::SHIFT
                } else {
                    Modifiers::NONE
                };
                for pressed in [true, false] {
                    self.events.push(Event::Key {
                        key: key(*k),
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers,
                    });
                }
            }
            InputEvent::Text(TextEvent::Preedit(text)) => {
                self.preedit = (!text.is_empty()).then(|| text.clone());
                self.events.push(Event::Ime(ImeEvent::Preedit {
                    text: text.clone(),
                    active_range_chars: None,
                }));
            }
            InputEvent::Text(TextEvent::Commit(text)) => {
                self.preedit = None;
                self.events.push(Event::Ime(ImeEvent::Commit(text.clone())));
            }
            InputEvent::Text(TextEvent::CompositionCancelled) => {
                // egui has no event for this; an empty preedit takes the composed text out again.
                self.preedit = None;
                self.events.push(Event::Ime(ImeEvent::Preedit {
                    text: String::new(),
                    active_range_chars: None,
                }));
            }
            InputEvent::Action(request) => {
                self.events
                    .push(Event::AccessKitActionRequest(request.clone()));
            }
        }
    }

    fn frame(&mut self, model: &UiModel, commands: &mut Vec<Command>) -> UiFrame {
        self.time += 1.0 / 60.0;
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(SURFACE.0, SURFACE.1),
            )),
            time: Some(self.time),
            focused: true,
            events: std::mem::take(&mut self.events),
            ..Default::default()
        };
        if let Some(viewport) = input.viewports.get_mut(&egui::ViewportId::ROOT) {
            viewport.native_pixels_per_point = Some(self.scale);
        }
        let hex_id = egui::Id::unique("hex");
        let hex_text = &mut self.hex;
        let mut field_has_focus = false;
        let mut output = self.ctx.run_ui(input, |ui| {
            egui::Panel::top("toolbar")
                .exact_size(SURFACE.1)
                .show(ui, |ui| {
                    let row = |role: Role, label: &'static str| {
                        UiBuilder::new()
                            .layout(Layout::left_to_right(Align::Center))
                            .accessibility_role(role)
                            .accessibility_label(label)
                    };
                    ui.scope_builder(row(Role::Toolbar, "Tools"), |ui| {
                        ui.scope_builder(row(Role::RadioGroup, "Tool"), |ui| {
                            for tool in Tool::ALL {
                                if ui.radio(model.tool == tool, tool.name()).clicked() {
                                    commands.push(Command::SetTool(tool));
                                }
                            }
                        });
                        ui.separator();
                        ui.scope_builder(row(Role::RadioGroup, "Colour"), |ui| {
                            for (colour, name) in SWATCHES {
                                let (rect, response) =
                                    ui.allocate_exact_size(egui::vec2(24.0, 24.0), Sense::click());
                                let on = model.colour == colour;
                                ui.painter().rect_filled(
                                    rect,
                                    4.0,
                                    Color32::from_rgb(colour[0], colour[1], colour[2]),
                                );
                                if on || response.has_focus() {
                                    ui.painter().rect_stroke(
                                        rect.expand(2.0),
                                        5.0,
                                        ui.visuals().selection.stroke,
                                        egui::StrokeKind::Outside,
                                    );
                                }
                                response.widget_info(|| {
                                    egui::WidgetInfo::selected(Role::RadioButton, true, on, name)
                                });
                                if response.clicked() {
                                    commands.push(Command::SetColour(colour));
                                }
                            }
                        });
                        ui.separator();
                        if ui
                            .add_enabled(model.can_undo, egui::Button::new("Undo"))
                            .clicked()
                        {
                            commands.push(Command::Undo);
                        }
                        if ui
                            .add_enabled(model.can_redo, egui::Button::new("Redo"))
                            .clicked()
                        {
                            commands.push(Command::Redo);
                        }
                        ui.separator();
                        ui.label(format!("Zoom {}%", model.zoom_percent));
                        ui.separator();
                        // The field shows the model's colour until it has the focus; from then on it is the
                        // user's text, and a colour that changes meanwhile does not rewrite it.
                        if !ui.memory(|m| m.has_focus(hex_id)) {
                            *hex_text = hex(model.colour);
                        }
                        let response = ui.add(
                            egui::TextEdit::singleline(hex_text)
                                .id(hex_id)
                                .char_limit(6)
                                .desired_width(72.0),
                        );
                        ui.ctx()
                            .accesskit_node_builder(hex_id, |node| node.set_label("Hex colour"));
                        if response.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter))
                            && let Some(colour) = parse_hex(hex_text)
                        {
                            commands.push(Command::SetColour(colour));
                        }
                        field_has_focus = response.has_focus();
                    });
                });
        });
        let primitives = self
            .ctx
            .tessellate(std::mem::take(&mut output.shapes), output.pixels_per_point);
        let painted = primitives.len();
        match &mut self.kept {
            Some((kept, textures)) => {
                *kept = primitives;
                textures.append(std::mem::take(&mut output.textures_delta));
            }
            None => output.textures_delta.clear(),
        }
        let tree = output
            .platform_output
            .accesskit_update
            .take()
            .expect("AccessKit is on");
        let text_input = match (&output.platform_output.ime, field_has_focus) {
            (Some(ime), true) => {
                // egui's string holds the preedit too. It is taken out here by its text, which is right as
                // long as the composed text is not also what stands just before it.
                let value = match &self.preedit {
                    Some(p) => match self.hex.rfind(p.as_str()) {
                        Some(at) => format!("{}{}", &self.hex[..at], &self.hex[at + p.len()..]),
                        None => self.hex.clone(),
                    },
                    None => self.hex.clone(),
                };
                let r = ime.cursor_rect;
                Some(TextInputState {
                    value,
                    preedit: self.preedit.clone(),
                    shown: self.hex.clone(),
                    caret_rect: [r.min.x, r.min.y, r.max.x, r.max.y],
                })
            }
            _ => None,
        };
        UiFrame {
            painted,
            tree,
            text_input,
        }
    }
}
