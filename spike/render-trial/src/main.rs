//! Spike question 10, native part: does the renderer carry the load?
//!
//! `cargo run --release --manifest-path spike/render-trial/Cargo.toml [objects]` (default 10,000).
//! `cargo run --release --manifest-path spike/render-trial/Cargo.toml coverage [strokes]` (default 300) compares
//! what each form of outline covers with what discs and bands cover (slice 3d).
//! `cargo run --release --manifest-path spike/render-trial/Cargo.toml union [strokes]` (default 10,000) replaces
//! each stroke's discs and bands by their true union, made by a polygon-clipping library, and draws that.
//! `cargo run --release --manifest-path spike/render-trial/Cargo.toml picture <file.png> single|runs` draws parts
//! of three rough strokes in five of the forms, side by side: the strokes that are worst for the single path, or
//! for the smoothed runs.
//!
//! A scene of stroke outlines (built by `everink-core`) is spread over a 3840 x 2160 view and drawn the way
//! section 06 has it drawn: in tiles of 512 x 512 pixels, each tile a scene of the objects that touch it, rendered
//! into a texture. The run times one tile and the whole view, with `vello_gpu` on the machine's GPU and with
//! `vello_cpu`, and with the two outline builders of slice 3a.

use std::time::Instant;

use i_overlay::core::fill_rule::FillRule;
use i_overlay::float::simplify::SimplifyShape;
use vello_common::kurbo::{Affine, BezPath, PathEl, Point, Rect, Shape, flatten};
use vello_common::peniko::Color;
use vello_common::pixmap::Pixmap;

mod scene;
use scene::{KINDS, Object, TILE, VIEW, buckets, scene};

/// How often the whole view is rendered; the first is a warm-up and is not counted.
const RUNS: usize = 6;

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

/// One path alone, filled, as the alpha of a pixmap that holds `bounds`.
fn alone(path: &BezPath, bounds: Rect) -> Vec<u8> {
    let (w, h) = (bounds.width() as u16, bounds.height() as u16);
    let mut ctx = vello_cpu::RenderContext::new(w, h);
    let mut resources = vello_cpu::Resources::new();
    let mut pixmap = Pixmap::new(w, h);
    ctx.set_transform(Affine::translate((-bounds.x0, -bounds.y0)));
    ctx.set_paint(Color::from_rgba8(0, 0, 0, 255));
    ctx.fill_path(path);
    ctx.flush();
    ctx.render(&mut pixmap, &mut resources);
    pixmap
        .data_as_u8_slice()
        .chunks_exact(4)
        .map(|p| p[3])
        .collect()
}

/// How many sample rows a pixel has in `rows`.
const ROWS_A_PIXEL: f64 = 8.0;

/// Where the edges of a path cross each sample row: for every row, the crossings as (x, +1 going down or -1
/// going up). The path is flattened to a thousandth of a pixel first. Row `i` is at `y0 + (i + 0.5) / 8`.
fn rows(path: &BezPath, y0: f64, count: usize) -> Vec<Vec<(f64, i32)>> {
    let mut rows = vec![Vec::new(); count];
    let (mut start, mut at) = (Point::ZERO, Point::ZERO);
    let mut edge = |a: Point, b: Point| {
        if a.y == b.y {
            return;
        }
        let (top, bottom, way) = if a.y < b.y { (a, b, 1) } else { (b, a, -1) };
        // The rows whose line is at or below the top end and above the bottom end.
        let first = ((top.y - y0) * ROWS_A_PIXEL - 0.5).ceil().max(0.0) as usize;
        let end = (((bottom.y - y0) * ROWS_A_PIXEL - 0.5).ceil().max(0.0) as usize).min(count);
        for (i, row) in rows.iter_mut().enumerate().take(end).skip(first) {
            let y = y0 + (i as f64 + 0.5) / ROWS_A_PIXEL;
            row.push((
                top.x + (bottom.x - top.x) * (y - top.y) / (bottom.y - top.y),
                way,
            ));
        }
    };
    flatten(path, 0.001, |el| match el {
        PathEl::MoveTo(p) => {
            edge(at, start);
            (start, at) = (p, p);
        }
        PathEl::LineTo(p) => {
            edge(at, p);
            at = p;
        }
        PathEl::ClosePath => {
            edge(at, start);
            at = start;
        }
        _ => unreachable!("a flattened path has only lines"),
    });
    edge(at, start);
    rows
}

/// The areas of two paths along the sample rows, in pixels: of the first, of the second, of what the first
/// covers and the second does not, and of what the second covers and the first does not. A point is covered
/// where the non-zero rule says so; no renderer is used.
fn areas(a: &BezPath, b: &BezPath) -> [f64; 4] {
    let bounds = a
        .bounding_box()
        .union(b.bounding_box())
        .inflate(2.0, 2.0)
        .expand();
    let count = (bounds.height() * ROWS_A_PIXEL) as usize;
    let (ra, rb) = (rows(a, bounds.y0, count), rows(b, bounds.y0, count));
    let mut sum = [0.0f64; 4];
    for (xa, xb) in ra.iter().zip(&rb) {
        // Sweep the row: the winding of each path between one crossing and the next.
        let mut all: Vec<(f64, usize, i32)> = xa
            .iter()
            .map(|c| (c.0, 0, c.1))
            .chain(xb.iter().map(|c| (c.0, 1, c.1)))
            .collect();
        all.sort_by(|p, q| p.0.partial_cmp(&q.0).unwrap());
        let mut w = [0i32; 2];
        let mut from = 0.0f64;
        for (x, which, way) in all {
            let len = (x - from) / ROWS_A_PIXEL;
            for (k, inside) in [
                w[0] != 0,
                w[1] != 0,
                w[0] != 0 && w[1] == 0,
                w[0] == 0 && w[1] != 0,
            ]
            .into_iter()
            .enumerate()
            {
                if inside {
                    sum[k] += len;
                }
            }
            w[which] += way;
            from = x;
        }
        assert_eq!(w, [0, 0], "a row ends outside both paths");
    }
    sum
}

/// What each form of outline covers, against discs and bands, stroke by stroke (`areas`). A form may be a
/// little short of the reference at its edge without having a hole, so each is also held against the reference
/// built a tenth and three tenths thinner: what is missing of those is missing well inside the stroke.
/// Then how heavily `vello_cpu` draws each form: the coverage it gives a stroke drawn alone, over the form's
/// own area.
fn coverage(count: usize) {
    println!(
        "\n== what each form covers, against discs and bands: {count} strokes, one at a time, at 256 pixels a unit =="
    );
    for (label, rough) in [
        ("the scene's strokes", (false, false)),
        ("rough strokes", (true, false)),
        ("rough strokes at a steady pressure", (true, true)),
    ] {
        let reference = scene::lone_strokes(count, KINDS[0].1, 256.0, rough, 100);
        let thinner =
            [90, 70].map(|share| scene::lone_strokes(count, KINDS[0].1, 256.0, rough, share));
        for (name, kind) in KINDS {
            let paths = scene::lone_strokes(count, kind, 256.0, rough, 100);
            let (mut area, mut own, mut missing, mut extra) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
            let mut inside = [(0.0f64, 0.0f64, 0usize); 2];
            let (mut segments, mut drawn, mut drawn_big) = (0usize, 0.0f64, 0.0f64);
            for (i, (a, b)) in reference.iter().zip(&paths).enumerate() {
                segments += b.segments().count();
                let s = areas(a, b);
                area += s[0];
                own += s[1];
                missing += s[2];
                extra += s[3];
                for (thin, sums) in thinner.iter().zip(&mut inside) {
                    let s = areas(&thin[i], b);
                    sums.0 += s[0];
                    sums.1 += s[2];
                    // A thousandth of a square pixel: more than the flattening can account for.
                    sums.2 += usize::from(s[2] > 0.001);
                }
                let sum = |alpha: Vec<u8>| alpha.iter().map(|v| f64::from(*v) / 255.0).sum::<f64>();
                let bounds = b.bounding_box().inflate(2.0, 2.0).expand();
                drawn += sum(alone(b, bounds));
                // The same stroke eight times as large, in pixels of this zoom.
                let big = Affine::scale(8.0) * b.clone();
                let big_bounds = big.bounding_box().inflate(2.0, 2.0).expand();
                drawn_big += sum(alone(&big, big_bounds)) / 64.0;
            }
            println!(
                "{label}; {name}: {} segments a stroke; covers {:.2}% of the reference's area more and {:.2}% less; of the reference a tenth thinner {:.3}% is missing, in {} strokes; of it three tenths thinner {:.3}%, in {} strokes; drawn by vello_cpu at {:.1}% of its own area, and at {:.1}% eight times as large",
                segments / count,
                extra * 100.0 / area,
                missing * 100.0 / area,
                inside[0].1 * 100.0 / inside[0].0,
                inside[0].2,
                inside[1].1 * 100.0 / inside[1].0,
                inside[1].2,
                drawn * 100.0 / own,
                drawn_big * 100.0 / own,
            );
        }
    }
}

/// The true union of a path's parts as one outline: the path flattened to a hundredth of a pixel and its
/// overlaps removed by `i_overlay`. For this trial only, in pixels of one zoom: a figure for what a union costs
/// to make and to draw, not a builder to keep.
fn union_of(path: &BezPath) -> BezPath {
    let mut contours: Vec<Vec<[f64; 2]>> = Vec::new();
    flatten(path, 0.01, |el| match el {
        PathEl::MoveTo(p) => contours.push(vec![[p.x, p.y]]),
        PathEl::LineTo(p) => contours
            .last_mut()
            .expect("a path begins with a move")
            .push([p.x, p.y]),
        PathEl::ClosePath => {}
        _ => unreachable!("a flattened path has only lines"),
    });
    let mut out = BezPath::new();
    for contour in contours.simplify_shape(FillRule::NonZero).iter().flatten() {
        let mut points = contour.iter().map(|p| Point::new(p[0], p[1]));
        if let Some(first) = points.next() {
            out.move_to(first);
            points.for_each(|p| out.line_to(p));
            out.close_path();
        }
    }
    out
}

/// The scene with every stroke's discs and bands replaced by their true union: how long the unions take to
/// make, how many segments they have, whether they cover what discs and bands cover, and how long `vello_cpu`
/// takes to draw the view from them.
fn union(count: usize) {
    let mut objects = scene(count, KINDS[0].1);
    let before: usize = objects.iter().map(|o| o.segments).sum();
    let (mut area, mut missing, mut extra, mut holes) = (0.0f64, 0.0f64, 0.0f64, 0usize);
    let mut took = 0.0f64;
    for (i, object) in objects.iter_mut().enumerate() {
        let t = Instant::now();
        let joined = union_of(&object.path);
        took += t.elapsed().as_secs_f64() * 1e3;
        // The first few hundred are held against what they were made from.
        if i < 300 {
            let s = areas(&object.path, &joined);
            area += s[0];
            missing += s[2];
            extra += s[3];
        }
        let contours = joined
            .elements()
            .iter()
            .filter(|el| matches!(el, PathEl::MoveTo(_)))
            .count();
        holes += contours.saturating_sub(1);
        object.segments = joined.segments().count();
        object.path = joined;
    }
    let segments: usize = objects.iter().map(|o| o.segments).sum();
    println!(
        "\n== {count} strokes, each the true union of its discs and bands (this trial only, by i_overlay) =="
    );
    println!(
        "making the unions: {took:.0} ms in all, {:.0} us a stroke; {} segments a stroke, all straight, from {}; {holes} contours in all beyond one for each stroke",
        took * 1e3 / count as f64,
        segments / count,
        before / count
    );
    println!(
        "against discs and bands, over the first 300 strokes: covers {:.3}% of the area more and {:.3}% less",
        extra * 100.0 / area,
        missing * 100.0 / area
    );
    let tiles = buckets(&objects);
    let check = (0..tiles.len())
        .max_by_key(|i| tiles[*i].1.len())
        .expect("a view has tiles");
    let (runs, alpha) = cpu_tiles(&objects, &tiles, check);
    let total: Vec<f64> = runs[1..].iter().map(|r| r[0] + r[1]).collect();
    let (med, min, max) = spread(total);
    println!(
        "vello_cpu: whole view {med:7.1} ms (min {min:.1}, max {max:.1}); {:.2} ms a tile; {:.0}% of the busiest tile drawn on",
        med / tiles.len() as f64,
        drawn_share(&alpha) * 100.0
    );
}

/// The middle of the longest stretch along a sample row that the first path covers and the second does not.
fn worst_gap(a: &BezPath, b: &BezPath) -> (f64, Point) {
    let bounds = a
        .bounding_box()
        .union(b.bounding_box())
        .inflate(2.0, 2.0)
        .expand();
    let count = (bounds.height() * ROWS_A_PIXEL) as usize;
    let (ra, rb) = (rows(a, bounds.y0, count), rows(b, bounds.y0, count));
    let mut worst = (0.0f64, bounds.center());
    for (row, (xa, xb)) in ra.iter().zip(&rb).enumerate() {
        let mut all: Vec<(f64, usize, i32)> = xa
            .iter()
            .map(|c| (c.0, 0, c.1))
            .chain(xb.iter().map(|c| (c.0, 1, c.1)))
            .collect();
        all.sort_by(|p, q| p.0.partial_cmp(&q.0).unwrap());
        let mut w = [0i32; 2];
        let mut from = 0.0f64;
        for (x, which, way) in all {
            if w[0] != 0 && w[1] == 0 && x - from > worst.0 {
                let y = bounds.y0 + (row as f64 + 0.5) / ROWS_A_PIXEL;
                worst = (x - from, Point::new((x + from) / 2.0, y));
            }
            w[which] += way;
            from = x;
        }
    }
    worst
}

/// The digits 1 to 5, five dots wide and seven high, a row of dots to a byte.
const DIGITS: [[u8; 7]; 5] = [
    [
        0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
    ],
    [
        0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
    ],
    [
        0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
    ],
    [
        0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
    ],
    [
        0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110,
    ],
];

/// A picture of the forms side by side, as a PNG: parts of three rough strokes, a row each, drawn by `vello_cpu`
/// at 8192 or 4096 pixels a unit (thirty-two or sixteen times the scene's zoom). The columns carry their numbers: 1 discs and
/// bands, 2 the single path, 3 runs cut at a quarter turn, 4 runs cut at an eighth, 5 runs cut at an eighth
/// with their sides smoothed.
///
/// `worst` says which strokes, of the first 300. `single`: those where the single path leaves the longest
/// stretch of the stroke's inside (discs and bands built three tenths thinner) uncovered, each row centred on
/// that stretch. `runs`: those where the smoothed runs cover the least of what discs and bands cover, each row
/// centred on the longest stretch they miss.
fn picture(out: &str, worst: &str) {
    // Pixels a unit: thirty-two times the scene's zoom for `single`, whose flaws are small, and sixteen times
    // for `runs`, so that both edges of a wide stroke are in the cell.
    let zoom = if worst == "runs" { 4096.0 } else { 8192.0 };
    const CELL: usize = 300;
    const GAP: usize = 12;
    const ROWS: usize = 3;
    let rough = (true, false);
    let forms = [KINDS[0].1, KINDS[2].1, KINDS[3].1, KINDS[4].1, KINDS[5].1];
    let paths = forms.map(|kind| scene::lone_strokes(300, kind, zoom, rough, 100));
    let mut ranked: Vec<(f64, Point, usize)> = match worst {
        "single" => {
            let inside = scene::lone_strokes(300, KINDS[0].1, zoom, rough, 70);
            (0..300)
                .map(|i| {
                    let (gap, at) = worst_gap(&inside[i], &paths[1][i]);
                    (gap, at, i)
                })
                .collect()
        }
        "runs" => (0..300)
            .map(|i| {
                let s = areas(&paths[0][i], &paths[4][i]);
                (s[2] / s[0], worst_gap(&paths[0][i], &paths[4][i]).1, i)
            })
            .collect(),
        _ => panic!("picture <file.png> single|runs"),
    };
    ranked.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    let (w, h) = (
        forms.len() * CELL + (forms.len() + 1) * GAP,
        ROWS * CELL + (ROWS + 1) * GAP,
    );
    // Light grey between the cells, white in them, and the stroke in one dark colour by its coverage.
    let mut pixels = vec![[226u8, 228, 232, 255]; w * h];
    for (row, (_, at, i)) in ranked.iter().take(ROWS).enumerate() {
        let half = CELL as f64 / 2.0;
        // On whole pixels, so that the window is a cell wide exactly.
        let (cx, cy) = (at.x.floor(), at.y.floor());
        let window = Rect::new(cx - half, cy - half, cx + half, cy + half);
        for (col, form) in paths.iter().enumerate() {
            let alpha = alone(&form[*i], window);
            assert_eq!(alpha.len(), CELL * CELL);
            let (x0, y0) = (GAP + col * (CELL + GAP), GAP + row * (CELL + GAP));
            for (k, a) in alpha.iter().enumerate() {
                let a = u32::from(*a);
                let mix = |ink: u32| ((ink * a + 255 * (255 - a)) / 255) as u8;
                pixels[(y0 + k / CELL) * w + x0 + k % CELL] = [mix(24), mix(40), mix(72), 255];
            }
            // The column's number in the cell's corner, on a white patch so that it reads over ink.
            const DOT: usize = 4;
            for y in 0..9 * DOT {
                for x in 0..7 * DOT {
                    let (dx, dy) = (x / DOT, y / DOT);
                    let on = (1..6).contains(&dx)
                        && (1..8).contains(&dy)
                        && DIGITS[col][dy - 1] >> (5 - dx) & 1 == 1;
                    pixels[(y0 + y) * w + x0 + x] = if on {
                        [176, 32, 32, 255]
                    } else {
                        [255, 255, 255, 255]
                    };
                }
            }
        }
    }
    let file = std::fs::File::create(out).expect("the picture's file can be made");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut writer| writer.write_image_data(pixels.as_flattened()))
        .expect("the picture is written");
    println!(
        "{out}: parts of rough strokes {:?} of the first 300, one a row, chosen as the worst for `{worst}`; columns: 1 discs and bands, 2 the single path, 3 runs cut at a quarter turn, 4 runs cut at an eighth, 5 runs cut at an eighth with their sides smoothed",
        ranked.iter().take(ROWS).map(|r| r.2).collect::<Vec<_>>()
    );
    // What they were ranked by: for `single` the stretch in pixels, for `runs` the share of the area.
    println!(
        "ranked by {:?}",
        ranked.iter().take(ROWS).map(|r| r.0).collect::<Vec<_>>()
    );
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("picture") {
        let args: Vec<String> = std::env::args().skip(2).collect();
        assert_eq!(args.len(), 2, "picture <file.png> single|runs");
        picture(&args[0], &args[1]);
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("union") {
        let count = std::env::args()
            .nth(2)
            .and_then(|s| s.parse().ok())
            .unwrap_or(10_000);
        union(count);
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("coverage") {
        let count = std::env::args()
            .nth(2)
            .and_then(|s| s.parse().ok())
            .unwrap_or(300);
        coverage(count);
        return;
    }
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
    for (name, kind) in KINDS {
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
