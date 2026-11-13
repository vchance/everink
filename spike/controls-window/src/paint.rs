//! Drawing a frame of either candidate into a texture: the own toolkit's paint list through `vello_gpu`, with
//! its text shaped by `parley`; egui's triangles through `egui-wgpu`.

use controls_trial::own::Paint;
use controls_trial::with_egui::EguiPaint;
use controls_trial::{SURFACE, TextInputState};
use parley::{FontContext, GenericFamily, LayoutContext, PositionedLayoutItem, StyleProperty};
use vello_common::kurbo::{Affine, Rect, RoundedRect, Shape, Stroke};
use vello_common::peniko::Color;

pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

/// The surface in physical pixels for a display scale.
pub fn physical(scale: f32) -> (u32, u32) {
    (
        (SURFACE.0 * scale).round() as u32,
        (SURFACE.1 * scale).round() as u32,
    )
}

fn colour(c: [u8; 4]) -> Color {
    Color::from_rgba8(c[0], c[1], c[2], c[3])
}

/// Draws the own toolkit's paint list.
pub struct OwnPainter {
    renderer: vello_gpu::Renderer,
    resources: vello_gpu::Resources,
    scene: vello_gpu::Scene,
    fonts: FontContext,
    layouts: LayoutContext<[u8; 4]>,
}

impl OwnPainter {
    pub fn new(gpu: &Gpu, format: wgpu::TextureFormat, scale: f32) -> OwnPainter {
        let (w, h) = physical(scale);
        let (renderer, resources) = vello_gpu::Renderer::new(
            &gpu.device,
            &gpu.queue,
            &vello_gpu::RenderTargetConfig {
                format,
                width: w as u16,
                height: h as u16,
            },
        );
        // The same one font the toolkit measured its text with.
        let mut fonts = FontContext::new();
        let families = fonts.collection.register_fonts(
            parley::fontique::Blob::from(epaint_default_fonts::UBUNTU_LIGHT.to_vec()),
            None,
        );
        fonts
            .collection
            .set_generic_families(GenericFamily::SansSerif, families.iter().map(|(id, _)| *id));
        OwnPainter {
            renderer,
            resources,
            scene: vello_gpu::Scene::new(w as u16, h as u16),
            fonts,
            layouts: LayoutContext::new(),
        }
    }

    fn text(&mut self, at: [f32; 2], text: &str, rgba: [u8; 4]) {
        let mut builder = self
            .layouts
            .ranged_builder(&mut self.fonts, text, 1.0, true);
        builder.push_default(StyleProperty::FontSize(14.0));
        builder.push_default(GenericFamily::SansSerif);
        let mut layout: parley::Layout<[u8; 4]> = builder.build(text);
        layout.break_all_lines(None);
        self.scene.set_paint(colour(rgba));
        for line in layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else {
                    continue;
                };
                let coords: Vec<i16> = run
                    .run()
                    .normalized_coords()
                    .iter()
                    .map(|c| c.to_bits())
                    .collect();
                let glyphs = run.positioned_glyphs().map(|g| glifo::Glyph {
                    id: g.id,
                    x: at[0] + g.x,
                    y: at[1] + g.y,
                });
                let _ = self
                    .scene
                    .glyph_run(&mut self.resources, run.run().font())
                    .font_size(run.run().font_size())
                    .hint(true)
                    .normalized_coords(&coords)
                    .fill_glyphs(glyphs);
            }
        }
    }

    pub fn paint(
        &mut self,
        gpu: &Gpu,
        view: &wgpu::TextureView,
        scale: f32,
        list: &[Paint],
        text_input: Option<&TextInputState>,
    ) {
        self.scene.reset();
        self.scene.set_transform(Affine::scale(f64::from(scale)));
        let rounded = |r: &[f32; 4], radius: f32| {
            RoundedRect::new(
                r[0].into(),
                r[1].into(),
                r[2].into(),
                r[3].into(),
                f64::from(radius),
            )
            .to_path(0.05)
        };
        for item in list {
            match item {
                Paint::Fill { rect, radius, rgba } => {
                    self.scene.set_paint(colour(*rgba));
                    self.scene.fill_path(&rounded(rect, *radius));
                }
                Paint::Outline { rect, radius, rgba } => {
                    self.scene.set_paint(colour(*rgba));
                    self.scene.set_stroke(Stroke::new(1.5));
                    self.scene.stroke_path(&rounded(rect, *radius));
                }
                Paint::Text { at, text, rgba } => self.text(*at, text, *rgba),
            }
        }
        if let Some(input) = text_input.filter(|i| i.preedit.is_none()) {
            // The caret, where the toolkit says it is. While text is being composed there is none: that text is
            // drawn with the rest of the field's, without its underline.
            let c = input.caret_rect;
            self.scene.set_paint(colour([0x10, 0x50, 0xe0, 0xff]));
            self.scene.fill_rect(&Rect::new(
                c[0].into(),
                c[1].into(),
                (c[0] + 1.5).into(),
                c[3].into(),
            ));
        }
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("own") });
        self.renderer
            .render(
                &self.scene,
                &mut self.resources,
                &mut encoder,
                vello_gpu::RenderTarget {
                    view,
                    depth_view: None,
                },
                &vello_gpu::TextureBindings::new(),
                vello_gpu::RenderOptions::default(),
            )
            .expect("the controls render");
        gpu.queue.submit([encoder.finish()]);
    }
}

/// Draws egui's triangles.
pub struct EguiPainter {
    renderer: egui_wgpu::Renderer,
}

impl EguiPainter {
    pub fn new(gpu: &Gpu, format: wgpu::TextureFormat) -> EguiPainter {
        EguiPainter {
            renderer: egui_wgpu::Renderer::new(
                &gpu.device,
                format,
                egui_wgpu::RendererOptions::default(),
            ),
        }
    }

    pub fn paint(&mut self, gpu: &Gpu, view: &wgpu::TextureView, mut paint: EguiPaint) {
        let (w, h) = physical(paint.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [w, h],
            pixels_per_point: paint.pixels_per_point,
        };
        for (id, deltas) in paint.textures.set.drain() {
            for delta in deltas {
                self.renderer
                    .update_texture(&gpu.device, &gpu.queue, id, &delta);
            }
        }
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("egui"),
            });
        let buffers = self.renderer.update_buffers(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            &paint.primitives,
            &screen,
        );
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            self.renderer
                .render(&mut pass.forget_lifetime(), &paint.primitives, &screen);
        }
        gpu.queue
            .submit(buffers.into_iter().chain([encoder.finish()]));
        for id in paint.textures.free.drain() {
            self.renderer.free_texture(&id);
        }
    }
}
