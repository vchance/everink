//! Spike question 10, native part: does the renderer carry the load?
//!
//! `cargo run --release --manifest-path spike/render-trial/Cargo.toml [objects]` (default 10,000).
//!
//! A scene of stroke outlines (built by `everink-core`) is spread over a 3840 x 2160 view and drawn the way
//! section 06 has it drawn: in tiles of 512 x 512 pixels, each tile a scene of the objects that touch it, rendered
//! into a texture. The run times one tile and the whole view, with `vello_gpu` on the machine's GPU and with
//! `vello_cpu`, and with the two outline builders of slice 3a.

use std::f64::consts::FRAC_1_SQRT_2;
use std::time::Instant;

use everink_core::outline::{InkPoint, Outline, Seg, build, build_single_path};
use vello_common::kurbo::{Affine, BezPath, Point, Rect, Shape};
use vello_common::peniko::Color;
use vello_common::pixmap::Pixmap;

const VIEW: (u32, u32) = (3840, 2160);
const TILE: u16 = 512;
/// How often the whole view is rendered; the first is a warm-up and is not counted.
const RUNS: usize = 6;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// One object of the scene: its path in view pixels, its box, and its colour.
struct Object {
    path: BezPath,
    bounds: Rect,
    colour: Color,
    segments: usize,
}

/// An outline of the 2^-40 lattice as a path in view pixels: `scale` pixels per object unit, moved to `at`.
fn to_path(outline: &Outline, scale: f64, at: (f64, f64)) -> BezPath {
    let px = |p: everink_core::outline::LPt| {
        Point::new(
            p.x as f64 / 1_099_511_627_776.0 * scale + at.0,
            p.y as f64 / 1_099_511_627_776.0 * scale + at.1,
        )
    };
    let mut path = BezPath::new();
    let mut end = None;
    for seg in &outline.segs {
        if end != Some(seg.start()) {
            if end.is_some() {
                path.close_path();
            }
            path.move_to(px(seg.start()));
        }
        match seg {
            Seg::Line(_, b) => path.line_to(px(*b)),
            Seg::Quad(_, c, b) => path.quad_to(px(*c), px(*b)),
        }
        end = Some(seg.end());
    }
    path.close_path();
    path
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// `everink_core::outline::build`: a sixteen-cornered disc of curves at every point and a band between.
    DiscsAndBands,
    /// `everink_core::outline::build_single_path`: the design's one path, which leaves holes.
    SinglePath,
    /// For this trial only: the same union with straight edges, an octagon at every point and a band between.
    /// It is built in plain floats here, so it is a figure for the renderer's cost and not a builder to keep.
    Octagons,
}

/// The union of an octagon at every point and a band along every piece, all anticlockwise, with straight edges.
fn octagons(points: &[InkPoint], width: i32, scale: f64, at: (f64, f64)) -> BezPath {
    const TURN: [(f64, f64); 8] = [
        (1.0, 0.0),
        (FRAC_1_SQRT_2, FRAC_1_SQRT_2),
        (0.0, 1.0),
        (-FRAC_1_SQRT_2, FRAC_1_SQRT_2),
        (-1.0, 0.0),
        (-FRAC_1_SQRT_2, -FRAC_1_SQRT_2),
        (0.0, -1.0),
        (FRAC_1_SQRT_2, -FRAC_1_SQRT_2),
    ];
    let px: Vec<(Point, f64)> = points
        .iter()
        .map(|p| {
            // The same pressure curve as the core, near enough for a timing: a quarter to the whole width.
            let share = 0.25 + 0.75 * f64::from(p.pressure) / 255.0;
            (
                Point::new(
                    f64::from(p.x) / 65536.0 * scale + at.0,
                    f64::from(p.y) / 65536.0 * scale + at.1,
                ),
                f64::from(width) / 65536.0 * scale / 2.0 * share,
            )
        })
        .collect();
    let mut path = BezPath::new();
    for (i, &(p, r)) in px.iter().enumerate() {
        path.move_to(Point::new(p.x + r, p.y));
        for (c, s) in &TURN[1..] {
            path.line_to(Point::new(p.x + r * c, p.y + r * s));
        }
        path.close_path();
        let Some(&(q, s)) = px.get(i + 1) else {
            break;
        };
        let (dx, dy) = (q.x - p.x, q.y - p.y);
        let len = (dx * dx + dy * dy).sqrt();
        if len > 0.0 {
            let (nx, ny) = (-dy / len, dx / len);
            path.move_to(Point::new(p.x - nx * r, p.y - ny * r));
            path.line_to(Point::new(q.x - nx * s, q.y - ny * s));
            path.line_to(Point::new(q.x + nx * s, q.y + ny * s));
            path.line_to(Point::new(p.x + nx * r, p.y + ny * r));
            path.close_path();
        }
    }
    path
}

/// A scene of pen strokes: each a random walk of 30 to 120 points, about 60 to 250 pixels long, 2 to 6 pixels
/// wide, somewhere in the view.
fn scene(objects: usize, kind: Kind) -> Vec<Object> {
    let mut rng = Rng(7);
    (0..objects)
        .map(|_| {
            let n = 30 + rng.below(91) as usize;
            let (mut x, mut y) = (0i32, 0i32);
            let (mut dx, mut dy) = (rng.below(801) as i32 - 400, rng.below(801) as i32 - 400);
            let points: Vec<InkPoint> = (0..n)
                .map(|_| {
                    dx = (dx + rng.below(201) as i32 - 100).clamp(-600, 600);
                    dy = (dy + rng.below(201) as i32 - 100).clamp(-600, 600);
                    x += dx;
                    y += dy;
                    InkPoint {
                        x,
                        y,
                        pressure: 120 + rng.below(100) as u8,
                    }
                })
                .collect();
            // A point lattice unit is 2^-16 of an object unit; 400 of them a sample is 0.006 units. At 256
            // pixels a unit a sample moves about 1.5 pixels and the stroke is 2 to 6 pixels wide.
            let width = ((2 + rng.below(5)) as i32) << 8; // 2 to 6 pixels at 256 pixels a unit
            let at = (
                rng.below(u64::from(VIEW.0)) as f64,
                rng.below(u64::from(VIEW.1)) as f64,
            );
            let path = match kind {
                Kind::DiscsAndBands => {
                    to_path(&build(&points, width).expect("a stroke"), 256.0, at)
                }
                Kind::SinglePath => to_path(
                    &build_single_path(&points, width).expect("a stroke"),
                    256.0,
                    at,
                ),
                Kind::Octagons => octagons(&points, width, 256.0, at),
            };
            let bounds = path.bounding_box();
            let c = rng.next();
            Object {
                segments: path.segments().count(),
                path,
                bounds,
                colour: Color::from_rgba8(
                    c as u8 / 2,
                    (c >> 8) as u8 / 2,
                    (c >> 16) as u8 / 2,
                    255,
                ),
            }
        })
        .collect()
}

/// The objects that touch each tile, in drawing order.
fn buckets(objects: &[Object]) -> Vec<((u32, u32), Vec<usize>)> {
    let (cols, rows) = (
        VIEW.0.div_ceil(u32::from(TILE)),
        VIEW.1.div_ceil(u32::from(TILE)),
    );
    let mut tiles = Vec::new();
    for ty in 0..rows {
        for tx in 0..cols {
            let tile = Rect::new(
                f64::from(tx * u32::from(TILE)),
                f64::from(ty * u32::from(TILE)),
                f64::from((tx + 1) * u32::from(TILE)),
                f64::from((ty + 1) * u32::from(TILE)),
            );
            let touching = objects
                .iter()
                .enumerate()
                .filter(|(_, o)| o.bounds.overlaps(tile))
                .map(|(i, _)| i)
                .collect();
            tiles.push(((tx, ty), touching));
        }
    }
    tiles
}

/// Median, minimum and maximum.
fn spread(mut v: Vec<f64>) -> (f64, f64, f64) {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (v[v.len() / 2], v[0], v[v.len() - 1])
}

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    info: String,
}

fn gpu() -> Option<Gpu> {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
        ..Default::default()
    }))
    .ok()?;
    let info = adapter.get_info();
    // A software device (lavapipe, llvmpipe) is not a GPU: its figures would be the processor's under another
    // name. It is reported and not used.
    if info.device_type == wgpu::DeviceType::Cpu {
        println!("GPU: only a software device ({}); not used", info.name);
        return None;
    }
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("render-trial"),
        required_features: wgpu::Features::empty(),
        ..Default::default()
    }))
    .ok()?;
    Some(Gpu {
        device,
        queue,
        info: format!("{} ({:?}, {:?})", info.name, info.backend, info.device_type),
    })
}

/// Renders the whole view in tiles with `vello_gpu`. Returns, per run, the milliseconds for all tiles split
/// into building the scenes, the renderer's own work on the processor, and waiting for the GPU to finish; and
/// the coverage (alpha) of every pixel of tile number `check` as the last run drew it.
///
/// Each tile is submitted and waited for on its own, so "waiting for the GPU" includes the cost of that
/// synchronisation as well as the GPU's work.
fn gpu_tiles(
    gpu: &Gpu,
    objects: &[Object],
    tiles: &[((u32, u32), Vec<usize>)],
    check: usize,
) -> (Vec<[f64; 3]>, Vec<u8>) {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("tile"),
        size: wgpu::Extent3d {
            width: TILE.into(),
            height: TILE.into(),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let (mut renderer, mut resources) = vello_gpu::Renderer::new(
        &gpu.device,
        &gpu.queue,
        &vello_gpu::RenderTargetConfig {
            format: texture.format(),
            width: TILE,
            height: TILE,
        },
    );
    let depth = renderer.create_depth_texture_view(&vello_gpu::RenderSize {
        width: TILE,
        height: TILE,
    });
    let mut scene = vello_gpu::Scene::new(TILE, TILE);
    let mut runs = Vec::new();
    let mut checked = Vec::new();
    for run in 0..RUNS {
        let mut ms = [0.0f64; 3];
        for (index, ((tx, ty), touching)) in tiles.iter().enumerate() {
            let t = Instant::now();
            scene.reset();
            scene.set_transform(Affine::translate((
                -f64::from(tx * u32::from(TILE)),
                -f64::from(ty * u32::from(TILE)),
            )));
            for &i in touching {
                scene.set_paint(objects[i].colour);
                scene.fill_path(&objects[i].path);
            }
            ms[0] += t.elapsed().as_secs_f64() * 1e3;

            let t = Instant::now();
            let mut encoder = gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("tile"),
                });
            renderer
                .render(
                    &scene,
                    &mut resources,
                    &mut encoder,
                    vello_gpu::RenderTarget {
                        view: &view,
                        depth_view: Some(&depth),
                    },
                    &vello_gpu::TextureBindings::new(),
                    vello_gpu::RenderOptions::default(),
                )
                .expect("a tile renders");
            gpu.queue.submit([encoder.finish()]);
            ms[1] += t.elapsed().as_secs_f64() * 1e3;

            let t = Instant::now();
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .expect("the GPU finishes");
            ms[2] += t.elapsed().as_secs_f64() * 1e3;
            if run + 1 == RUNS && index == check {
                checked = read_alpha(gpu, &texture);
            }
        }
        runs.push(ms);
    }
    (runs, checked)
}

/// Reads the tile texture back and returns the alpha of each pixel, row by row.
fn read_alpha(gpu: &Gpu, texture: &wgpu::Texture) -> Vec<u8> {
    let row = (u32::from(TILE) * 4).next_multiple_of(256);
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(row) * u64::from(TILE),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
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
    gpu.queue.submit([encoder.finish()]);
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, |r| r.expect("the buffer maps"));
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("the GPU finishes");
    let data = buffer.slice(..).get_mapped_range().expect("a mapped range");
    data.chunks_exact(row as usize)
        .flat_map(|r| r[..usize::from(TILE) * 4].chunks_exact(4))
        .map(|p| p[3])
        .collect()
}

/// The same with `vello_cpu`, into a pixel buffer. Returns the milliseconds per run for all tiles, split into
/// building the scenes and rasterising, and the coverage (alpha) of every pixel of tile number `check`.
fn cpu_tiles(
    objects: &[Object],
    tiles: &[((u32, u32), Vec<usize>)],
    check: usize,
) -> (Vec<[f64; 2]>, Vec<u8>) {
    let mut ctx = vello_cpu::RenderContext::new(TILE, TILE);
    let mut resources = vello_cpu::Resources::new();
    let mut pixmap = Pixmap::new(TILE, TILE);
    let mut runs = Vec::new();
    let mut checked = Vec::new();
    for run in 0..RUNS {
        let mut ms = [0.0f64; 2];
        for (index, ((tx, ty), touching)) in tiles.iter().enumerate() {
            let t = Instant::now();
            ctx.reset();
            ctx.set_transform(Affine::translate((
                -f64::from(tx * u32::from(TILE)),
                -f64::from(ty * u32::from(TILE)),
            )));
            for &i in touching {
                ctx.set_paint(objects[i].colour);
                ctx.fill_path(&objects[i].path);
            }
            ms[0] += t.elapsed().as_secs_f64() * 1e3;
            let t = Instant::now();
            ctx.flush();
            ctx.render(&mut pixmap, &mut resources);
            ms[1] += t.elapsed().as_secs_f64() * 1e3;
            if run + 1 == RUNS && index == check {
                checked = pixmap
                    .data_as_u8_slice()
                    .chunks_exact(4)
                    .map(|p| p[3])
                    .collect();
            }
        }
        runs.push(ms);
    }
    (runs, checked)
}

/// The share of a tile's pixels that are drawn on.
fn drawn_share(alpha: &[u8]) -> f64 {
    alpha.iter().filter(|a| **a != 0).count() as f64 / alpha.len() as f64
}

fn main() {
    let count: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000);
    println!(
        "render trial, {} {}; view {} x {}, tiles of {TILE} pixels; {RUNS} runs of the whole view, the first not counted",
        std::env::consts::ARCH,
        std::env::consts::OS,
        VIEW.0,
        VIEW.1
    );
    let gpu = gpu();
    match &gpu {
        Some(g) => println!("GPU: {}", g.info),
        None => println!("GPU: none found; only the processor renderer is run"),
    }
    for (name, kind) in [
        ("discs and bands (slice 3a's builder)", Kind::DiscsAndBands),
        ("octagons and bands (this trial only)", Kind::Octagons),
        (
            "single path (the design's, which leaves holes)",
            Kind::SinglePath,
        ),
    ] {
        let objects = scene(count, kind);
        let tiles = buckets(&objects);
        let segments: usize = objects.iter().map(|o| o.segments).sum();
        let per_tile: usize = tiles.iter().map(|(_, t)| t.len()).sum::<usize>() / tiles.len();
        println!(
            "\n== {count} strokes, outlines as {name}: {} segments a stroke, {} tiles, {per_tile} strokes a tile on average ==",
            segments / count,
            tiles.len()
        );
        // The tile the two renderers are compared on: the one the most strokes touch.
        let check = (0..tiles.len())
            .max_by_key(|i| tiles[*i].1.len())
            .expect("a view has tiles");
        let mut gpu_alpha = None;
        if let Some(g) = &gpu {
            let (runs, alpha) = gpu_tiles(g, &objects, &tiles, check);
            let drawn = drawn_share(&alpha);
            gpu_alpha = Some(alpha);
            let total: Vec<f64> = runs[1..].iter().map(|r| r[0] + r[1] + r[2]).collect();
            let (med, min, max) = spread(total);
            let part = |i: usize| spread(runs[1..].iter().map(|r| r[i]).collect()).0;
            println!(
                "vello_gpu: whole view {med:7.1} ms (min {min:.1}, max {max:.1}); {:.2} ms a tile; {:.0} tiles a second",
                med / tiles.len() as f64,
                tiles.len() as f64 * 1000.0 / med
            );
            println!(
                "           of which building the scenes {:.1} ms, the renderer on the processor {:.1} ms, waiting for the GPU {:.1} ms; {:.0}% of the busiest tile drawn on",
                part(0),
                part(1),
                part(2),
                drawn * 100.0
            );
        }
        // The whole view as one scene, for comparison: no stroke is prepared twice because it crosses a tile edge.
        let whole = [((0u32, 0u32), (0..objects.len()).collect::<Vec<usize>>())];
        let mut one = Vec::new();
        for _ in 0..RUNS {
            let mut ctx = vello_cpu::RenderContext::new(VIEW.0 as u16, VIEW.1 as u16);
            let t = Instant::now();
            for &i in &whole[0].1 {
                ctx.set_paint(objects[i].colour);
                ctx.fill_path(&objects[i].path);
            }
            one.push(t.elapsed().as_secs_f64() * 1e3);
        }
        let (med, min, max) = spread(one[1..].to_vec());
        println!(
            "building one scene of the whole view, not in tiles: {med:.1} ms (min {min:.1}, max {max:.1})"
        );
        let (runs, alpha) = cpu_tiles(&objects, &tiles, check);
        let drawn = drawn_share(&alpha);
        let total: Vec<f64> = runs[1..].iter().map(|r| r[0] + r[1]).collect();
        let (med, min, max) = spread(total);
        let part = |i: usize| spread(runs[1..].iter().map(|r| r[i]).collect()).0;
        println!(
            "vello_cpu: whole view {med:7.1} ms (min {min:.1}, max {max:.1}); {:.2} ms a tile; {:.0} tiles a second",
            med / tiles.len() as f64,
            tiles.len() as f64 * 1000.0 / med
        );
        println!(
            "           of which building the scenes {:.1} ms, rasterising {:.1} ms; {:.0}% of the busiest tile drawn on",
            part(0),
            part(1),
            drawn * 100.0
        );
        // A check that the runs drew something, and the same thing. The busiest tile must be drawn on; and where
        // there is a GPU, the two renderers' coverage of it is compared pixel by pixel: the pixels where their
        // alpha differs by more than an eighth of full must be under two in a hundred of the pixels either drew.
        // (The edges of shapes differ a little between two rasterisers; a missing shape would differ a lot.)
        assert!(
            drawn > 0.05,
            "vello_cpu drew on only {:.2}% of the busiest tile",
            drawn * 100.0
        );
        match &gpu_alpha {
            Some(g) => {
                let either = g
                    .iter()
                    .zip(&alpha)
                    .filter(|(a, b)| **a != 0 || **b != 0)
                    .count();
                let differ = g
                    .iter()
                    .zip(&alpha)
                    .filter(|(a, b)| a.abs_diff(**b) > 32)
                    .count();
                println!(
                    "check: on the busiest tile the two renderers differ on {differ} of the {either} pixels either drew ({:.2}%)",
                    differ as f64 * 100.0 / either as f64
                );
                assert!(
                    differ * 50 < either,
                    "the renderers disagree on {differ} of {either} drawn pixels of the busiest tile"
                );
            }
            None => println!("check: no GPU, so the two renderers are not compared"),
        }
    }
}
