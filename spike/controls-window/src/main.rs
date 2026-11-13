//! Spike question 12, part 4b: the two controls candidates in a native window.
//!
//!   cargo run --release --locked --manifest-path spike/controls-window/Cargo.toml -- own
//!   cargo run --release --locked --manifest-path spike/controls-window/Cargo.toml -- egui
//!   cargo run ... -- own --shot /tmp/own.png   (no window: draws a few frames and writes the last as a picture)
//!   cargo run ... -- own --shot /tmp/own.png --focus Pen   (the same, with the focus on a tool or a colour)
//!
//! The window is the toolbar and the hex field and nothing else. The shell turns the window's events into the
//! trial's own (`InputEvent`), the same for both candidates, hands the accessibility tree each gives to the
//! platform through `accesskit_winit`, and tells the platform's text input where the caret is while the field
//! has the focus. A screen reader and a keyboard are then a person's to try: see `README.md` here.

mod paint;
#[path = "shell.rs"]
mod shell;

use std::sync::Arc;

use controls_trial::{InputEvent, Key, SURFACE, TextEvent, UiFrame};
use paint::{Gpu, physical};
use shell::{Candidate, Document, Painter, tree_for_platform};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::{ElementState, Ime, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{Key as WinitKey, NamedKey};
use winit::window::{Window, WindowId};

fn gpu(surface: Option<&wgpu::Surface<'_>>, instance: &wgpu::Instance) -> (wgpu::Adapter, Gpu) {
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        compatible_surface: surface,
        ..Default::default()
    }))
    .expect("a GPU");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("controls-window"),
        ..Default::default()
    }))
    .expect("a device");
    (adapter, Gpu { device, queue })
}

// ---------------------------------------------------------------------------------------------------------------
// A picture without a window
// ---------------------------------------------------------------------------------------------------------------

fn shot(name: &str, path: &str, focus: Option<&str>) {
    const SCALE: f32 = 2.0;
    let instance = wgpu::Instance::default();
    let (_, gpu) = gpu(None, &instance);
    let (w, h) = physical(SCALE);
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("shot"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let mut candidate = Candidate::new(name, SCALE);
    let mut painter = candidate.painter(&gpu, format, SCALE);
    let mut document = Document::new();
    let mut commands = Vec::new();

    // A little use, so that the picture shows a checked tool and swatch, the focus ring, and text being
    // composed in the field: click Eraser and Green by their places in the tree, focus the field, type, compose.
    candidate.controls().frame(&document.model, &mut commands);
    let mut frame = candidate.controls().frame(&document.model, &mut commands);
    let step = |candidate: &mut Candidate, document: &mut Document, event: InputEvent| {
        let mut commands = Vec::new();
        candidate
            .controls()
            .handle(&event, &document.model, &mut commands);
        let mut frame = candidate.controls().frame(&document.model, &mut commands);
        document.apply(&mut commands);
        for _ in 0..2 {
            frame = candidate.controls().frame(&document.model, &mut commands);
            document.apply(&mut commands);
        }
        frame
    };
    let request =
        |frame: &UiFrame, role: accesskit::Role, name: &str, action: accesskit::Action| {
            let tree = controls_trial::contract::Tree::new(&frame.tree);
            InputEvent::Action(accesskit::ActionRequest {
                action,
                target_tree: accesskit::TreeId::ROOT,
                target_node: tree.find(role, name).expect("the control is in the tree"),
                data: None,
            })
        };
    use accesskit::{Action, Role};
    for (role, name, action) in [
        (Role::RadioButton, "Eraser", Action::Click),
        (Role::RadioButton, "Green", Action::Click),
        (Role::TextInput, "Hex colour", Action::Focus),
    ] {
        let event = request(&frame, role, name, action);
        frame = step(&mut candidate, &mut document, event);
    }
    if let Some(control) = focus {
        // Another picture: the focus on a tool or a colour, to see its ring beside the chosen one's mark.
        let event = request(&frame, Role::RadioButton, control, Action::Focus);
        frame = step(&mut candidate, &mut document, event);
        candidate.paint(&mut painter, &gpu, &view, SCALE, &frame);
        write_picture(&gpu, &texture, w, h, path);
        println!("{name}: wrote {path} ({w} x {h}) with the focus on {control}");
        return;
    }
    for event in [
        InputEvent::Key {
            key: Key::Backspace,
            shift: false,
        },
        InputEvent::Key {
            key: Key::Backspace,
            shift: false,
        },
        InputEvent::Text(TextEvent::Commit("A".to_string())),
    ] {
        step(&mut candidate, &mut document, event);
    }
    frame = step(
        &mut candidate,
        &mut document,
        InputEvent::Text(TextEvent::Preedit("´".to_string())),
    );
    candidate.paint(&mut painter, &gpu, &view, SCALE, &frame);

    write_picture(&gpu, &texture, w, h, path);
    let input = frame.text_input.expect("the field has the focus");
    println!(
        "{name}: wrote {path} ({w} x {h}); tool {:?}, colour {}; the field shows {:?}, composing {:?}",
        document.model.tool,
        controls_trial::hex(document.model.colour),
        input.shown,
        input.preedit
    );
}

/// Reads the texture back and writes it as a PNG.
fn write_picture(gpu: &Gpu, texture: &wgpu::Texture, w: u32, h: u32, path: &str) {
    let row = (w * 4).next_multiple_of(256);
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("shot"),
        size: u64::from(row) * u64::from(h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("shot"),
        });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit([encoder.finish()]);
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, |r| r.expect("the buffer maps"));
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("the GPU finishes");
    let data = buffer.slice(..).get_mapped_range().expect("a mapped range");
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for line in data.chunks_exact(row as usize) {
        pixels.extend_from_slice(&line[..(w * 4) as usize]);
    }
    let file = std::fs::File::create(path).expect("the picture can be written");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut writer| writer.write_image_data(&pixels))
        .expect("the picture is written");
}

// ---------------------------------------------------------------------------------------------------------------
// The window
// ---------------------------------------------------------------------------------------------------------------

enum UserEvent {
    Accessibility(accesskit_winit::Event),
}

impl From<accesskit_winit::Event> for UserEvent {
    fn from(event: accesskit_winit::Event) -> Self {
        UserEvent::Accessibility(event)
    }
}

struct Shown {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    gpu: Gpu,
    painter: Painter,
    adapter: accesskit_winit::Adapter,
    scale: f32,
}

struct App {
    name: String,
    proxy: EventLoopProxy<UserEvent>,
    candidate: Option<Candidate>,
    document: Document,
    shown: Option<Shown>,
    pointer: (f32, f32),
    shift: bool,
    composing: bool,
    ime_allowed: bool,
    last_tree: Option<accesskit::TreeUpdate>,
}

impl App {
    fn send(&mut self, event: InputEvent) {
        let Some(candidate) = &mut self.candidate else {
            return;
        };
        let mut commands = Vec::new();
        candidate
            .controls()
            .handle(&event, &self.document.model, &mut commands);
        self.document.apply(&mut commands);
        if let Some(shown) = &self.shown {
            shown.window.request_redraw();
        }
    }

    fn redraw(&mut self) {
        let (Some(candidate), Some(shown)) = (&mut self.candidate, &mut self.shown) else {
            return;
        };
        // Two frames: egui answers an event in the frame after it, and the model may change with the answer.
        let mut commands = Vec::new();
        let mut frame = candidate
            .controls()
            .frame(&self.document.model, &mut commands);
        if !commands.is_empty() {
            self.document.apply(&mut commands);
            frame = candidate
                .controls()
                .frame(&self.document.model, &mut commands);
            self.document.apply(&mut commands);
        }

        // The platform's text input: on while the field has the focus, with its window at the caret.
        let wants = frame.text_input.is_some();
        if wants != self.ime_allowed {
            shown.window.set_ime_allowed(wants);
            self.ime_allowed = wants;
        }
        if let Some(input) = &frame.text_input {
            let c = input.caret_rect;
            shown.window.set_ime_cursor_area(
                LogicalPosition::new(c[0], c[1]),
                LogicalSize::new((c[2] - c[0]).max(1.0), c[3] - c[1]),
            );
        }

        let own = matches!(candidate, Candidate::Own(_));
        let tree = tree_for_platform(frame.tree.clone(), own, shown.scale);
        self.last_tree = Some(tree.clone());
        shown.adapter.update_if_active(|| tree);

        let target = match shown.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            _ => {
                shown.surface.configure(&shown.gpu.device, &shown.config);
                return;
            }
        };
        let view = target
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        candidate.paint(&mut shown.painter, &shown.gpu, &view, shown.scale, &frame);
        shown.window.pre_present_notify();
        shown.gpu.queue.present(target);
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.shown.is_some() {
            return;
        }
        // The window is made hidden: the accessibility adapter must be there before it is shown.
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title(format!("Everink controls trial: {}", self.name))
                        .with_inner_size(LogicalSize::new(SURFACE.0, SURFACE.1))
                        .with_resizable(false)
                        .with_visible(false),
                )
                .expect("a window"),
        );
        let adapter = accesskit_winit::Adapter::with_event_loop_proxy(
            event_loop,
            &window,
            self.proxy.clone(),
        );
        let scale = window.scale_factor() as f32;
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window.clone())
            .expect("a surface on the window");
        let (gpu_adapter, gpu) = gpu(Some(&surface), &instance);
        let (w, h) = physical(scale);
        let mut config = surface
            .get_default_config(&gpu_adapter, w, h)
            .expect("the surface can be configured");
        // `vello_gpu` as used here does not convert to sRGB on the way out, so the surface must not be an sRGB
        // format for it; `egui-wgpu` copes with either and prefers this.
        if let Some(format) = surface
            .get_capabilities(&gpu_adapter)
            .formats
            .into_iter()
            .find(|f| !f.is_srgb())
        {
            config.format = format;
        }
        surface.configure(&gpu.device, &config);
        let candidate = Candidate::new(&self.name, scale);
        let painter = candidate.painter(&gpu, config.format, scale);
        self.candidate = Some(candidate);
        window.set_visible(true);
        window.request_redraw();
        self.shown = Some(Shown {
            window,
            surface,
            config,
            gpu,
            painter,
            adapter,
            scale,
        });
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        let UserEvent::Accessibility(event) = event;
        match event.window_event {
            accesskit_winit::WindowEvent::InitialTreeRequested => {
                if let (Some(shown), Some(tree)) = (&mut self.shown, self.last_tree.clone()) {
                    shown.adapter.update_if_active(|| tree);
                }
                println!("assistive technology asked for the tree");
            }
            accesskit_winit::WindowEvent::ActionRequested(request) => {
                println!("assistive technology asked for {:?}", request.action);
                self.send(InputEvent::Action(request));
            }
            accesskit_winit::WindowEvent::AccessibilityDeactivated => {}
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if let Some(shown) = &mut self.shown {
            shown.adapter.process_event(&shown.window, &event);
        }
        let scale = self.shown.as_ref().map_or(1.0, |s| s.scale);
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::CursorMoved { position, .. } => {
                self.pointer = (position.x as f32 / scale, position.y as f32 / scale);
                let (x, y) = self.pointer;
                self.send(InputEvent::PointerMove { x, y });
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                let (x, y) = self.pointer;
                self.send(match state {
                    ElementState::Pressed => InputEvent::PointerDown { x, y },
                    ElementState::Released => InputEvent::PointerUp { x, y },
                });
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.shift = modifiers.state().shift_key();
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key,
                        state: ElementState::Pressed,
                        text,
                        ..
                    },
                ..
            } => {
                let key = match &logical_key {
                    WinitKey::Named(named) => match named {
                        NamedKey::Tab => Some(Key::Tab),
                        NamedKey::ArrowLeft => Some(Key::Left),
                        NamedKey::ArrowRight => Some(Key::Right),
                        NamedKey::Home => Some(Key::Home),
                        NamedKey::End => Some(Key::End),
                        NamedKey::Enter => Some(Key::Enter),
                        NamedKey::Space => Some(Key::Space),
                        NamedKey::Escape => Some(Key::Escape),
                        NamedKey::Backspace => Some(Key::Backspace),
                        NamedKey::Delete => Some(Key::Delete),
                        _ => None,
                    },
                    _ => None,
                };
                if self.composing {
                    // While text is being composed the keys are the input method's; what it makes of them
                    // comes as `Ime` events.
                } else if let Some(key) = key {
                    self.send(InputEvent::Key {
                        key,
                        shift: self.shift,
                    });
                } else if let Some(text) = text.filter(|t| !t.chars().any(char::is_control)) {
                    // A plain key: its text goes in as a commit.
                    println!("key text: {text:?}");
                    self.send(InputEvent::Text(TextEvent::Commit(text.to_string())));
                }
            }
            WindowEvent::Ime(ime) => match ime {
                Ime::Preedit(text, _) => {
                    self.composing = !text.is_empty();
                    println!("text input: composing {text:?}");
                    self.send(InputEvent::Text(TextEvent::Preedit(text)));
                }
                Ime::Commit(text) => {
                    self.composing = false;
                    println!("text input: commit {text:?}");
                    self.send(InputEvent::Text(TextEvent::Commit(text)));
                }
                Ime::Disabled => {
                    if std::mem::take(&mut self.composing) {
                        self.send(InputEvent::Text(TextEvent::CompositionCancelled));
                    }
                }
                Ime::Enabled => {}
            },
            _ => {}
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let name = args.first().map_or("own", String::as_str).to_string();
    if let Some(at) = args.iter().position(|a| a == "--shot") {
        let focus = args
            .iter()
            .position(|a| a == "--focus")
            .and_then(|i| args.get(i + 1));
        shot(
            &name,
            args.get(at + 1).expect("--shot takes a path"),
            focus.map(String::as_str),
        );
        return;
    }
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .expect("an event loop");
    let mut app = App {
        name,
        proxy: event_loop.create_proxy(),
        candidate: None,
        document: Document::new(),
        shown: None,
        pointer: (0.0, 0.0),
        shift: false,
        composing: false,
        ime_allowed: false,
        last_tree: None,
    };
    event_loop.run_app(&mut app).expect("the window runs");
}
