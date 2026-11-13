//! A program that does one thing with a candidate, to read how large the candidate makes a WebAssembly module:
//! it shows the toolbar, clicks a tool, types into the field, and counts what came out. Built with no
//! candidate it is the baseline. `size.sh` builds it each way.

use controls_trial::{Command, Controls, InputEvent, Key, TextEvent, Tool, UiModel};

fn run(mut controls: Box<dyn Controls>, model: &UiModel) -> usize {
    let mut commands: Vec<Command> = Vec::new();
    controls.frame(model, &mut commands);
    let frame = controls.frame(model, &mut commands);
    let field = frame
        .tree
        .nodes
        .iter()
        .find(|(_, n)| n.role() == accesskit::Role::TextInput)
        .map(|(id, _)| *id);
    for event in [
        InputEvent::PointerDown { x: 30.0, y: 28.0 },
        InputEvent::PointerUp { x: 30.0, y: 28.0 },
        InputEvent::Key {
            key: Key::Tab,
            shift: false,
        },
    ] {
        controls.handle(&event, model, &mut commands);
        controls.frame(model, &mut commands);
    }
    if let Some(target_node) = field {
        controls.handle(
            &InputEvent::Action(accesskit::ActionRequest {
                action: accesskit::Action::Focus,
                target_tree: accesskit::TreeId::ROOT,
                target_node,
                data: None,
            }),
            model,
            &mut commands,
        );
        controls.frame(model, &mut commands);
    }
    for event in [
        InputEvent::Text(TextEvent::Preedit("´".to_string())),
        InputEvent::Text(TextEvent::Commit("é".to_string())),
        InputEvent::Key {
            key: Key::Enter,
            shift: false,
        },
    ] {
        controls.handle(&event, model, &mut commands);
        controls.frame(model, &mut commands);
    }
    let frame = controls.frame(model, &mut commands);
    frame.painted + frame.tree.nodes.len() + commands.len()
}

fn main() {
    // The model comes from outside the program, so that nothing here can be worked out when it is built.
    let n = std::hint::black_box(std::env::args().count() as u32);
    let model = UiModel {
        tool: Tool::ALL[n as usize % 5],
        colour: [n as u8, 0x20, 0x20],
        can_undo: n.is_multiple_of(2),
        can_redo: true,
        zoom_percent: 100 + n,
    };
    #[allow(unused_mut, reason = "with no candidate nothing is added")]
    let mut total = 0usize;
    #[cfg(feature = "own")]
    {
        total += run(
            Box::new(controls_trial::own::OwnControls::default()),
            &model,
        );
    }
    #[cfg(feature = "egui")]
    {
        total += run(
            Box::new(controls_trial::with_egui::EguiControls::default()),
            &model,
        );
    }
    let _ = (&model, run as fn(Box<dyn Controls>, &UiModel) -> usize);
    println!("{total}");
}
