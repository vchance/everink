//! Spike question 10, browser part: does the renderer carry the load?
//!
//! The scene of `spike/render-trial` in a browser: the view's 40 tiles drawn from nothing with `vello_gpu` on
//! WebGPU, with `vello_gpu` on WebGL2 and with `vello_cpu`, and then the part the native trial left out: the tiles
//! kept as textures (the tile cache) and the view composed from them each frame while it pans.
//!
//! `www/index.html` runs it; `README.md` here says how to build and serve it.

use std::cell::RefCell;

use vello_common::kurbo::Affine;
use wasm_bindgen::prelude::*;

#[path = "../../render-trial/src/scene.rs"]
mod scene;
use scene::{KINDS, Object, TILE, buckets, scene};

/// How often the whole view is rendered; the first is a warm-up and is not counted.
const RUNS: usize = 6;

type Tiles = Vec<((u32, u32), Vec<usize>)>;

fn now() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .expect("a window with a clock")
        .now()
}

/// Median, minimum and maximum.
fn spread(mut v: Vec<f64>) -> (f64, f64, f64) {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (v[v.len() / 2], v[0], v[v.len() - 1])
}

fn tile_origin(tx: u32, ty: u32) -> (f64, f64) {
    (
        f64::from(tx * u32::from(TILE)),
        f64::from(ty * u32::from(TILE)),
    )
}

/// The tile the renderers are compared on: the one the most strokes touch.
fn busiest(tiles: &Tiles) -> usize {
    (0..tiles.len())
        .max_by_key(|i| tiles[*i].1.len())
        .expect("a view has tiles")
}

fn heading(out: &mut String, count: usize, name: &str, objects: &[Object], tiles: &Tiles) {
    let segments: usize = objects.iter().map(|o| o.segments).sum();
    let per_tile: usize = tiles.iter().map(|(_, t)| t.len()).sum::<usize>() / tiles.len();
    out.push_str(&format!(
        "\n== {count} strokes, outlines as {name}: {} segments a stroke, {} tiles, {per_tile} strokes a tile on average ==\n",
        segments / count,
        tiles.len()
    ));
}

/// The lines for one renderer and one outline form: the whole view, and the parts named in `names`.
fn lines(
    out: &mut String,
    label: &str,
    runs: &[Vec<f64>],
    names: &[&str],
    tiles: usize,
    drawn: f64,
) {
    let (med, min, max) = spread(runs[1..].iter().map(|r| r.iter().sum()).collect());
    out.push_str(&format!(
        "{label}: whole view {med:7.1} ms (min {min:.1}, max {max:.1}); {:.2} ms a tile; {:.0} tiles a second\n",
        med / tiles as f64,
        tiles as f64 * 1000.0 / med
    ));
    let parts: Vec<String> = names
        .iter()
        .enumerate()
        .map(|(i, n)| {
            format!(
                "{n} {:.1} ms",
                spread(runs[1..].iter().map(|r| r[i]).collect()).0
            )
        })
        .collect();
    out.push_str(&format!(
        "           of which {}; {:.0}% of the busiest tile drawn on\n",
        parts.join(", "),
        drawn * 100.0
    ));
}

/// The share of a tile's pixels that are drawn on.
fn drawn_share(alpha: &[u8]) -> f64 {
    alpha.iter().filter(|a| **a != 0).count() as f64 / alpha.len() as f64
}

thread_local! {
    /// The coverage of the busiest tile as each renderer drew it, by renderer and outline form, for `compare`.
    static ALPHA: RefCell<Vec<(&'static str, &'static str, Vec<u8>)>> = const { RefCell::new(Vec::new()) };
}

fn keep_alpha(renderer: &'static str, kind: &'static str, alpha: Vec<u8>) {
    ALPHA.with_borrow_mut(|a| {
        a.retain(|(r, k, _)| !(*r == renderer && *k == kind));
        a.push((renderer, kind, alpha));
    });
}

/// Compares, pixel by pixel, the busiest tile as each pair of renderers that has run drew it: the pixels where
/// their coverage differs by more than an eighth of full, among the pixels either drew. Returns the lines, and
/// fails if any pair differs on two in a hundred or more.
#[wasm_bindgen]
pub fn compare() -> Result<String, JsValue> {
    let mut out = String::new();
    let mut worst = 0.0f64;
    ALPHA.with_borrow(|all| {
        for (i, (ra, ka, a)) in all.iter().enumerate() {
            for (rb, kb, b) in &all[i + 1..] {
                if ka != kb || a.len() != b.len() {
                    continue;
                }
                let either = a.iter().zip(b).filter(|(x, y)| **x != 0 || **y != 0).count();
                let differ = a.iter().zip(b).filter(|(x, y)| x.abs_diff(**y) > 32).count();
                let share = differ as f64 * 100.0 / either.max(1) as f64;
                worst = worst.max(share);
                out.push_str(&format!(
                    "check, {ka}: {ra} and {rb} differ on {differ} of the {either} pixels either drew ({share:.2}%)\n"
                ));
            }
        }
    });
    if out.is_empty() {
        out.push_str("check: fewer than two renderers have run, so nothing is compared\n");
    }
    if worst >= 2.0 {
        return Err(JsValue::from_str(&format!("{out}the renderers disagree")));
    }
    Ok(out)
}

/// A scene of the size of the view that draws each cached tile as an image, the whole moved by `(dx, dy)`.
/// `flip` is for textures whose first row is the bottom of the image (WebGL's).
#[cfg(any(feature = "webgpu", feature = "webgl"))]
fn compose_scene(scene: &mut vello_gpu::Scene, tiles: &Tiles, dx: f64, dy: f64, flip: bool) {
    use vello_common::TextureId;
    use vello_common::kurbo::Rect;
    use vello_common::paint::{Image, ImageSource, TextureRegion};
    use vello_common::peniko::ImageSampler;
    let side = f64::from(TILE);
    scene.reset();
    scene.set_transform(Affine::translate((dx, dy)));
    for (i, ((tx, ty), _)) in tiles.iter().enumerate() {
        let (x, y) = tile_origin(*tx, *ty);
        scene.set_paint_transform(if flip {
            Affine::translate((x, y + side)) * Affine::scale_non_uniform(1.0, -1.0)
        } else {
            Affine::translate((x, y))
        });
        scene.set_paint(Image {
            image: ImageSource::external_texture(TextureId(i as u64), TextureRegion::Full, true),
            sampler: ImageSampler::default(),
        });
        scene.fill_rect(&Rect::new(x, y, x + side, y + side));
    }
}

#[cfg(feature = "cpu")]
mod cpu {
    use super::*;
    use vello_common::pixmap::Pixmap;

    /// Draws the view in tiles with `vello_cpu`, into a pixel buffer, for the three outline forms.
    #[wasm_bindgen]
    pub fn run_cpu(strokes: u32) -> String {
        let count = strokes as usize;
        let mut out = String::new();
        for (name, kind) in KINDS {
            let objects = scene(count, kind);
            let tiles = buckets(&objects);
            heading(&mut out, count, name, &objects, &tiles);
            let check = busiest(&tiles);
            let mut ctx = vello_cpu::RenderContext::new(TILE, TILE);
            let mut resources = vello_cpu::Resources::new();
            let mut pixmap = Pixmap::new(TILE, TILE);
            let mut runs = Vec::new();
            let mut alpha = Vec::new();
            for run in 0..RUNS {
                let mut ms = vec![0.0f64; 2];
                for (index, ((tx, ty), touching)) in tiles.iter().enumerate() {
                    let t = now();
                    let (x, y) = tile_origin(*tx, *ty);
                    ctx.reset();
                    ctx.set_transform(Affine::translate((-x, -y)));
                    for &i in touching {
                        ctx.set_paint(objects[i].colour);
                        ctx.fill_path(&objects[i].path);
                    }
                    ms[0] += now() - t;
                    let t = now();
                    ctx.flush();
                    ctx.render(&mut pixmap, &mut resources);
                    ms[1] += now() - t;
                    if run + 1 == RUNS && index == check {
                        alpha = pixmap
                            .data_as_u8_slice()
                            .chunks_exact(4)
                            .map(|p| p[3])
                            .collect();
                    }
                }
                runs.push(ms);
            }
            let drawn = drawn_share(&alpha);
            lines(
                &mut out,
                "vello_cpu",
                &runs,
                &["building the scenes", "rasterising"],
                tiles.len(),
                drawn,
            );
            keep_alpha("vello_cpu", name, alpha);
        }
        out
    }
}

#[cfg(feature = "webgl")]
mod webgl {
    use super::scene::VIEW;
    use super::*;
    use vello_gpu::{
        Resources, Scene, WebGlRenderOptions, WebGlRenderTarget, WebGlRenderer,
        WebGlTextureBindings, WebGlTextureTarget,
    };
    use web_sys::{
        HtmlCanvasElement, WebGl2RenderingContext as Gl, WebGlFramebuffer, WebGlTexture,
    };

    struct State {
        gl: Gl,
        renderer: WebGlRenderer,
        resources: Resources,
        tiles: Tiles,
        bindings: WebGlTextureBindings,
        view: Scene,
    }

    thread_local! {
        static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    }

    fn err(e: impl std::fmt::Debug) -> JsValue {
        JsValue::from_str(&format!("{e:?}"))
    }

    /// Reads a tile texture back: the alpha of each pixel, row by row from the top.
    fn read_alpha(
        gl: &Gl,
        frame: &WebGlFramebuffer,
        texture: &WebGlTexture,
    ) -> Result<Vec<u8>, JsValue> {
        let side = i32::from(TILE);
        gl.bind_framebuffer(Gl::FRAMEBUFFER, Some(frame));
        gl.framebuffer_texture_2d(
            Gl::FRAMEBUFFER,
            Gl::COLOR_ATTACHMENT0,
            Gl::TEXTURE_2D,
            Some(texture),
            0,
        );
        let mut data = vec![0u8; usize::from(TILE) * usize::from(TILE) * 4];
        gl.read_pixels_with_opt_u8_array(
            0,
            0,
            side,
            side,
            Gl::RGBA,
            Gl::UNSIGNED_BYTE,
            Some(&mut data),
        )?;
        gl.bind_framebuffer(Gl::FRAMEBUFFER, None);
        // The first row read is the bottom of the image.
        Ok(data
            .chunks_exact(usize::from(TILE) * 4)
            .rev()
            .flat_map(|row| row.chunks_exact(4).map(|p| p[3]))
            .collect())
    }

    /// Draws the view in tiles with `vello_gpu` on WebGL2, each tile into a texture of its own, for the three
    /// outline forms. The textures of the last form are kept as the tile cache for `webgl_frame`.
    #[wasm_bindgen]
    pub fn run_webgl(canvas: HtmlCanvasElement, strokes: u32) -> Result<String, JsValue> {
        let count = strokes as usize;
        let side = i32::from(TILE);
        let (mut renderer, mut resources) = WebGlRenderer::new(&canvas).map_err(err)?;
        let gl: Gl = canvas
            .get_context("webgl2")?
            .ok_or("no WebGL2 context")?
            .dyn_into()?;
        let stats = renderer.shader_compilation_stats();
        let mut out = format!("WebGL2; shaders: {stats:?}\n");

        let depth = gl.create_renderbuffer().ok_or("no renderbuffer")?;
        gl.bind_renderbuffer(Gl::RENDERBUFFER, Some(&depth));
        gl.renderbuffer_storage(Gl::RENDERBUFFER, Gl::DEPTH_COMPONENT24, side, side);
        gl.bind_renderbuffer(Gl::RENDERBUFFER, None);
        let frame = gl.create_framebuffer().ok_or("no framebuffer")?;

        let mut cache: Vec<(WebGlTexture, WebGlTextureTarget)> = Vec::new();
        let mut scene_ = Scene::new(TILE, TILE);
        let mut kept = None;
        for (name, kind) in KINDS {
            let objects = scene(count, kind);
            let tiles = buckets(&objects);
            heading(&mut out, count, name, &objects, &tiles);
            while cache.len() < tiles.len() {
                let texture = gl.create_texture().ok_or("no texture")?;
                gl.bind_texture(Gl::TEXTURE_2D, Some(&texture));
                gl.tex_storage_2d(Gl::TEXTURE_2D, 1, Gl::RGBA8, side, side);
                for (name, value) in [
                    (Gl::TEXTURE_MIN_FILTER, Gl::LINEAR),
                    (Gl::TEXTURE_MAG_FILTER, Gl::LINEAR),
                    (Gl::TEXTURE_WRAP_S, Gl::CLAMP_TO_EDGE),
                    (Gl::TEXTURE_WRAP_T, Gl::CLAMP_TO_EDGE),
                ] {
                    gl.tex_parameteri(Gl::TEXTURE_2D, name, value as i32);
                }
                gl.bind_texture(Gl::TEXTURE_2D, None);
                let target = renderer
                    .create_texture_target(&texture, Some(&depth), TILE, TILE)
                    .map_err(err)?;
                cache.push((texture, target));
            }
            let check = busiest(&tiles);
            let mut runs = Vec::new();
            for _ in 0..RUNS {
                let mut ms = vec![0.0f64; 3];
                for (index, ((tx, ty), touching)) in tiles.iter().enumerate() {
                    let t = now();
                    let (x, y) = tile_origin(*tx, *ty);
                    scene_.reset();
                    scene_.set_transform(Affine::translate((-x, -y)));
                    for &i in touching {
                        scene_.set_paint(objects[i].colour);
                        scene_.fill_path(&objects[i].path);
                    }
                    ms[0] += now() - t;
                    let t = now();
                    renderer
                        .render(
                            &scene_,
                            &mut resources,
                            WebGlRenderTarget::Texture(&cache[index].1),
                            &WebGlTextureBindings::new(),
                            WebGlRenderOptions::default(),
                        )
                        .map_err(err)?;
                    ms[1] += now() - t;
                }
                // One wait for the whole view. Chrome's `finish` returns at once, so the wait is a read of one
                // pixel of the last tile, which cannot return before the GPU has drawn everything sent so far.
                let t = now();
                let last = &cache[tiles.len() - 1].0;
                gl.bind_framebuffer(Gl::FRAMEBUFFER, Some(&frame));
                gl.framebuffer_texture_2d(
                    Gl::FRAMEBUFFER,
                    Gl::COLOR_ATTACHMENT0,
                    Gl::TEXTURE_2D,
                    Some(last),
                    0,
                );
                let mut pixel = [0u8; 4];
                gl.read_pixels_with_opt_u8_array(
                    0,
                    0,
                    1,
                    1,
                    Gl::RGBA,
                    Gl::UNSIGNED_BYTE,
                    Some(&mut pixel),
                )?;
                gl.bind_framebuffer(Gl::FRAMEBUFFER, None);
                ms[2] += now() - t;
                runs.push(ms);
            }
            let alpha = read_alpha(&gl, &frame, &cache[check].0)?;
            lines(
                &mut out,
                "vello_gpu on WebGL2",
                &runs,
                &[
                    "building the scenes",
                    "the renderer on the processor",
                    "waiting for the GPU",
                ],
                tiles.len(),
                drawn_share(&alpha),
            );
            keep_alpha("vello_gpu on WebGL2", name, alpha);
            kept = Some(tiles);
        }

        let tiles = kept.expect("three forms were drawn");
        let mut bindings = WebGlTextureBindings::new();
        for (i, (texture, _)) in cache.iter().enumerate().take(tiles.len()) {
            bindings.insert(
                vello_common::TextureId(i as u64),
                texture.clone(),
                TILE,
                TILE,
            );
        }
        STATE.set(Some(State {
            gl,
            renderer,
            resources,
            tiles,
            bindings,
            view: Scene::new(VIEW.0 as u16, VIEW.1 as u16),
        }));
        Ok(out)
    }

    /// One frame of a steady or panning view: the canvas composed from the cached tiles, moved by `(dx, dy)`.
    /// Returns the milliseconds of processor time it took here.
    #[wasm_bindgen]
    pub fn webgl_frame(dx: f64, dy: f64) -> Result<f64, JsValue> {
        STATE.with_borrow_mut(|state| {
            let s = state.as_mut().ok_or("run_webgl has not run")?;
            let t = now();
            compose_scene(&mut s.view, &s.tiles, dx, dy, true);
            s.renderer
                .render(
                    &s.view,
                    &mut s.resources,
                    WebGlRenderTarget::DrawingBuffer,
                    &s.bindings,
                    WebGlRenderOptions::default(),
                )
                .map_err(err)?;
            Ok(now() - t)
        })
    }

    /// The alpha of the composed canvas over tile number `index` at no offset, row by row from the top, kept for
    /// `compare` under the name of the outline form given. Call it straight after `webgl_frame(0, 0)`.
    #[wasm_bindgen]
    pub fn webgl_composed_check() -> Result<String, JsValue> {
        STATE.with_borrow(|state| {
            let s = state.as_ref().ok_or("run_webgl has not run")?;
            let index = busiest(&s.tiles);
            let (tx, ty) = s.tiles[index].0;
            let side = i32::from(TILE);
            let (x, y) = ((tx * u32::from(TILE)) as i32, (ty * u32::from(TILE)) as i32);
            // The drawing buffer's first row is the bottom of the canvas.
            let from_bottom = VIEW.1 as i32 - y - side;
            let mut data = vec![0u8; usize::from(TILE) * usize::from(TILE) * 4];
            s.gl.read_pixels_with_opt_u8_array(
                x,
                from_bottom,
                side,
                side,
                Gl::RGBA,
                Gl::UNSIGNED_BYTE,
                Some(&mut data),
            )?;
            let alpha: Vec<u8> = data
                .chunks_exact(usize::from(TILE) * 4)
                .rev()
                .flat_map(|row| row.chunks_exact(4).map(|p| p[3]))
                .collect();
            let drawn = drawn_share(&alpha);
            keep_alpha(
                "the canvas composed from the cache on WebGL2",
                KINDS[2].0,
                alpha,
            );
            Ok(format!(
                "composed on WebGL2: {:.0}% of the busiest tile's part of the canvas drawn on\n",
                drawn * 100.0
            ))
        })
    }
}

#[cfg(feature = "webgpu")]
mod webgpu {
    use super::scene::VIEW;
    use super::*;
    use vello_gpu::TextureBindings;
    use vello_gpu::{
        RenderOptions, RenderSize, RenderTarget, RenderTargetConfig, Renderer, Resources, Scene,
    };
    use web_sys::HtmlCanvasElement;

    struct State {
        device: wgpu::Device,
        queue: wgpu::Queue,
        surface: wgpu::Surface<'static>,
        renderer: Renderer,
        resources: Resources,
        depth: wgpu::TextureView,
        tiles: Tiles,
        bindings: TextureBindings,
        view: Scene,
    }

    thread_local! {
        static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    }

    fn err(e: impl std::fmt::Debug) -> JsValue {
        JsValue::from_str(&format!("{e:?}"))
    }

    /// Resolves when the GPU has finished everything submitted so far.
    async fn done(queue: &wgpu::Queue) {
        let (tx, rx) = futures_channel::oneshot::channel();
        queue.on_submitted_work_done(move || {
            let _ = tx.send(());
        });
        let _ = rx.await;
    }

    /// Reads a tile texture back: the alpha of each pixel, row by row.
    async fn read_alpha(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
    ) -> Vec<u8> {
        let row = (u32::from(TILE) * 4).next_multiple_of(256);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(row) * u64::from(TILE),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("readback"),
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
                width: TILE.into(),
                height: TILE.into(),
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        let (tx, rx) = futures_channel::oneshot::channel();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r.is_ok());
        });
        assert_eq!(rx.await, Ok(true), "the buffer maps");
        let data = buffer.slice(..).get_mapped_range().expect("a mapped range");
        data.chunks_exact(row as usize)
            .flat_map(|r| r[..usize::from(TILE) * 4].chunks_exact(4))
            .map(|p| p[3])
            .collect()
    }

    /// Draws the view in tiles with `vello_gpu` on WebGPU, each tile into a texture of its own, for the three
    /// outline forms. The textures of the last form are kept as the tile cache for `webgpu_frame`.
    #[wasm_bindgen]
    pub async fn run_webgpu(canvas: HtmlCanvasElement, strokes: u32) -> Result<String, JsValue> {
        let count = strokes as usize;
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
            .map_err(err)?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .map_err(err)?;
        let info = adapter.get_info();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("render-web"),
                required_features: wgpu::Features::empty(),
                ..Default::default()
            })
            .await
            .map_err(err)?;
        let config = surface
            .get_default_config(&adapter, VIEW.0, VIEW.1)
            .ok_or("the canvas cannot be configured")?;
        surface.configure(&device, &config);
        let mut out = format!(
            "WebGPU; adapter: {} ({:?}, {:?}); canvas format {:?}\n",
            info.name, info.backend, info.device_type, config.format
        );

        let format = wgpu::TextureFormat::Rgba8Unorm;
        let (mut renderer, mut resources) = Renderer::new(
            &device,
            &queue,
            &RenderTargetConfig {
                format,
                width: TILE,
                height: TILE,
            },
        );
        let depth = renderer.create_depth_texture_view(&RenderSize {
            width: TILE,
            height: TILE,
        });
        let mut cache: Vec<(wgpu::Texture, wgpu::TextureView)> = Vec::new();
        let mut scene_ = Scene::new(TILE, TILE);
        let mut kept = None;
        for (name, kind) in KINDS {
            let objects = scene(count, kind);
            let tiles = buckets(&objects);
            heading(&mut out, count, name, &objects, &tiles);
            while cache.len() < tiles.len() {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("tile"),
                    size: wgpu::Extent3d {
                        width: TILE.into(),
                        height: TILE.into(),
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                });
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                cache.push((texture, view));
            }
            let check = busiest(&tiles);
            let mut runs = Vec::new();
            for _ in 0..RUNS {
                let mut ms = vec![0.0f64; 3];
                for (index, ((tx, ty), touching)) in tiles.iter().enumerate() {
                    let t = now();
                    let (x, y) = tile_origin(*tx, *ty);
                    scene_.reset();
                    scene_.set_transform(Affine::translate((-x, -y)));
                    for &i in touching {
                        scene_.set_paint(objects[i].colour);
                        scene_.fill_path(&objects[i].path);
                    }
                    ms[0] += now() - t;
                    let t = now();
                    let mut encoder =
                        device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("tile"),
                        });
                    renderer
                        .render(
                            &scene_,
                            &mut resources,
                            &mut encoder,
                            RenderTarget {
                                view: &cache[index].1,
                                depth_view: Some(&depth),
                            },
                            &TextureBindings::new(),
                            RenderOptions::default(),
                        )
                        .map_err(err)?;
                    queue.submit([encoder.finish()]);
                    ms[1] += now() - t;
                }
                // One wait for the whole view. In a browser the wait ends on a later turn of the event loop, so
                // it includes that turn as well as the GPU's work.
                let t = now();
                done(&queue).await;
                ms[2] += now() - t;
                runs.push(ms);
            }
            let alpha = read_alpha(&device, &queue, &cache[check].0).await;
            lines(
                &mut out,
                "vello_gpu on WebGPU",
                &runs,
                &[
                    "building the scenes",
                    "the renderer on the processor",
                    "waiting for the GPU",
                ],
                tiles.len(),
                drawn_share(&alpha),
            );
            keep_alpha("vello_gpu on WebGPU", name, alpha);
            kept = Some(tiles);
        }

        // The view has a renderer of its own: a renderer is made for one target format and size.
        let tiles = kept.expect("three forms were drawn");
        let (renderer, resources) = Renderer::new(
            &device,
            &queue,
            &RenderTargetConfig {
                format: config.format,
                width: VIEW.0 as u16,
                height: VIEW.1 as u16,
            },
        );
        let depth = renderer.create_depth_texture_view(&RenderSize {
            width: VIEW.0 as u16,
            height: VIEW.1 as u16,
        });
        let mut bindings = TextureBindings::new();
        for (i, (_, view)) in cache.iter().enumerate().take(tiles.len()) {
            bindings.insert(vello_common::TextureId(i as u64), view.clone());
        }
        STATE.set(Some(State {
            device,
            queue,
            surface,
            renderer,
            resources,
            depth,
            tiles,
            bindings,
            view: Scene::new(VIEW.0 as u16, VIEW.1 as u16),
        }));
        Ok(out)
    }

    /// One frame of a steady or panning view: the canvas composed from the cached tiles, moved by `(dx, dy)`.
    /// Returns the milliseconds of processor time it took here.
    #[wasm_bindgen]
    pub fn webgpu_frame(dx: f64, dy: f64) -> Result<f64, JsValue> {
        STATE.with_borrow_mut(|state| {
            let s = state.as_mut().ok_or("run_webgpu has not run")?;
            let t = now();
            compose_scene(&mut s.view, &s.tiles, dx, dy, false);
            let frame = match s.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(f)
                | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
                other => return Err(err(other)),
            };
            let target = frame
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());
            let mut encoder = s
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("view"),
                });
            s.renderer
                .render(
                    &s.view,
                    &mut s.resources,
                    &mut encoder,
                    RenderTarget {
                        view: &target,
                        depth_view: Some(&s.depth),
                    },
                    &s.bindings,
                    RenderOptions::default(),
                )
                .map_err(err)?;
            s.queue.submit([encoder.finish()]);
            s.queue.present(frame);
            Ok(now() - t)
        })
    }
}
