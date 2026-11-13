//! Spike question 12, the figures an agent can take without a window: what a frame of each candidate costs, and
//! how much code each took.
//!
//! `cargo run --release --locked --manifest-path spike/controls-trial/Cargo.toml --example bench_controls`

use std::time::Instant;

use controls_trial::{Command, Controls, InputEvent, TextEvent, Tool, UiModel};

const FRAMES: usize = 2_000;

fn model(zoom: u32) -> UiModel {
    UiModel {
        tool: Tool::Pen,
        colour: [0x20, 0x20, 0x20],
        can_undo: true,
        can_redo: true,
        zoom_percent: zoom,
    }
}

/// Median and 95th percentile, in microseconds.
fn spread(mut v: Vec<f64>) -> (f64, f64) {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (v[v.len() / 2], v[v.len() * 95 / 100])
}

fn frames(
    controls: &mut dyn Controls,
    mut each: impl FnMut(usize, &mut dyn Controls) -> UiModel,
) -> (f64, f64) {
    let mut commands: Vec<Command> = Vec::new();
    let mut took = Vec::with_capacity(FRAMES);
    for i in 0..FRAMES + 50 {
        let m = each(i, controls);
        let t = Instant::now();
        let frame = controls.frame(&m, &mut commands);
        let us = t.elapsed().as_secs_f64() * 1e6;
        assert!(frame.painted > 0);
        if i >= 50 {
            took.push(us);
        }
    }
    spread(took)
}

fn measure(mut controls: Box<dyn Controls>) {
    let name = controls.name();
    let (med, p95) = frames(controls.as_mut(), |_, _| model(100));
    println!("{name}: a frame with nothing changed: median {med:.1} µs, 95th {p95:.1}");
    let (med, p95) = frames(controls.as_mut(), |i, _| model(100 + (i % 300) as u32));
    println!("{name}: a frame with the zoom figure changed: median {med:.1} µs, 95th {p95:.1}");
    let (med, p95) = frames(controls.as_mut(), |i, c| {
        let mut commands = Vec::new();
        let (x, y) = (20.0 + (i % 700) as f32, 28.0);
        c.handle(
            &InputEvent::PointerMove { x, y },
            &model(100),
            &mut commands,
        );
        model(100)
    });
    println!(
        "{name}: a frame with the pointer moved over the bar: median {med:.1} µs, 95th {p95:.1}"
    );
    // Typing: focus the field by its node, then a character and a backspace in turn.
    let mut commands = Vec::new();
    let frame = controls.frame(&model(100), &mut commands);
    let field = frame
        .tree
        .nodes
        .iter()
        .find(|(_, n)| n.role() == accesskit::Role::TextInput)
        .map(|(id, _)| *id)
        .expect("a text field");
    controls.handle(
        &InputEvent::Action(accesskit::ActionRequest {
            action: accesskit::Action::Focus,
            target_tree: accesskit::TreeId::ROOT,
            target_node: field,
            data: None,
        }),
        &model(100),
        &mut commands,
    );
    controls.handle(
        &InputEvent::Key {
            key: controls_trial::Key::Backspace,
            shift: false,
        },
        &model(100),
        &mut commands,
    );
    let (med, p95) = frames(controls.as_mut(), |i, c| {
        let mut commands = Vec::new();
        let event = if i % 2 == 0 {
            InputEvent::Text(TextEvent::Commit("7".to_string()))
        } else {
            InputEvent::Key {
                key: controls_trial::Key::Backspace,
                shift: false,
            }
        };
        c.handle(&event, &model(100), &mut commands);
        model(100)
    });
    println!("{name}: a frame after a key in the field: median {med:.1} µs, 95th {p95:.1}");
}

/// Lines that are code: not blank and not a comment.
fn code_lines(source: &str) -> usize {
    source
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("//"))
        .count()
}

fn main() {
    println!(
        "controls trial, {} {}; {FRAMES} frames each, after 50 not counted; no window, nothing is rasterised",
        std::env::consts::ARCH,
        std::env::consts::OS
    );
    measure(Box::new(controls_trial::own::OwnControls::default()));
    measure(Box::new(controls_trial::with_egui::EguiControls::default()));
    println!(
        "lines of code: own {}, egui {}; shared by both: the trait and model {}, the contract {}",
        code_lines(include_str!("../src/own.rs")),
        code_lines(include_str!("../src/with_egui.rs")),
        code_lines(include_str!("../src/lib.rs")),
        code_lines(include_str!("../src/contract.rs")),
    );
}
