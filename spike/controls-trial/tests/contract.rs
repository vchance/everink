//! The same tests for both candidates: what the toolbar and the field must do, driven through events and read
//! back from commands and the accessibility tree.

use accesskit::{Action, ActionData, ActionRequest, NodeId, Role, TreeId};
use controls_trial::contract::{Tree, faults};
use controls_trial::{
    Command, Controls, InputEvent, Key, TextEvent, TextInputState, Tool, UiFrame, UiModel,
};

const MODEL: UiModel = UiModel {
    tool: Tool::Pen,
    colour: [0x20, 0x20, 0x20],
    can_undo: false,
    can_redo: true,
    zoom_percent: 100,
};

/// A candidate with the model it is shown and the commands it has asked for.
struct Rig {
    controls: Box<dyn Controls>,
    model: UiModel,
    commands: Vec<Command>,
    last: UiFrame,
}

impl Rig {
    fn new(mut controls: Box<dyn Controls>) -> Rig {
        let mut commands = Vec::new();
        // Two frames: an immediate-mode toolkit sizes things in its first.
        controls.frame(&MODEL, &mut commands);
        let last = controls.frame(&MODEL, &mut commands);
        Rig {
            controls,
            model: MODEL,
            commands,
            last,
        }
    }

    /// Runs frames until nothing more comes out: a candidate may answer an event one frame late.
    fn settle(&mut self) {
        for _ in 0..3 {
            self.last = self.controls.frame(&self.model, &mut self.commands);
        }
    }

    fn send(&mut self, event: InputEvent) {
        self.controls
            .handle(&event, &self.model, &mut self.commands);
        self.settle();
    }

    fn take(&mut self) -> Vec<Command> {
        std::mem::take(&mut self.commands)
    }

    fn id(&self, role: Role, text: &str) -> NodeId {
        Tree::new(&self.last.tree)
            .find(role, text)
            .unwrap_or_else(|| panic!("no {role:?} named {text}"))
    }

    fn click(&mut self, role: Role, text: &str) {
        let (x, y) = Tree::new(&self.last.tree).centre(self.id(role, text));
        self.controls.handle(
            &InputEvent::PointerMove { x, y },
            &self.model,
            &mut self.commands,
        );
        self.settle();
        self.controls.handle(
            &InputEvent::PointerDown { x, y },
            &self.model,
            &mut self.commands,
        );
        self.settle();
        self.send(InputEvent::PointerUp { x, y });
    }

    fn act(&mut self, role: Role, text: &str, action: Action, data: Option<ActionData>) {
        let target_node = self.id(role, text);
        self.send(InputEvent::Action(ActionRequest {
            action,
            target_tree: TreeId::ROOT,
            target_node,
            data,
        }));
    }

    fn key(&mut self, key: Key) {
        self.send(InputEvent::Key { key, shift: false });
    }

    fn text(&mut self, event: TextEvent) {
        self.send(InputEvent::Text(event));
    }

    fn focus_field(&mut self) {
        self.act(Role::TextInput, "Hex colour", Action::Focus, None);
        assert_eq!(
            Tree::new(&self.last.tree).focus(),
            self.id(Role::TextInput, "Hex colour"),
            "the field has the focus"
        );
    }

    fn field(&self) -> &TextInputState {
        self.last
            .text_input
            .as_ref()
            .expect("a text field has the focus")
    }

    /// Empties the focused field.
    fn clear(&mut self) {
        self.key(Key::End);
        for _ in 0..6 {
            self.key(Key::Backspace);
        }
        assert_eq!(self.field().value, "");
    }
}

fn the_tree_is_what_a_screen_reader_needs(controls: Box<dyn Controls>) {
    let mut rig = Rig::new(controls);
    assert!(rig.last.painted > 0);
    assert_eq!(faults(&rig.last.tree, &rig.model), Vec::<String>::new());
    // And it follows the model.
    rig.model.tool = Tool::Line;
    rig.model.colour = [0x30, 0x60, 0xd0];
    rig.model.can_undo = true;
    rig.model.zoom_percent = 250;
    rig.settle();
    assert_eq!(faults(&rig.last.tree, &rig.model), Vec::<String>::new());
    let tree = Tree::new(&rig.last.tree);
    assert_eq!(
        tree.node(rig.id(Role::TextInput, "Hex colour")).value(),
        Some("3060D0")
    );
    assert!(rig.take().is_empty(), "showing a model asks for nothing");
}

fn the_pointer_and_assistive_technology_give_the_same_commands(controls: Box<dyn Controls>) {
    let mut rig = Rig::new(controls);
    rig.click(Role::RadioButton, "Eraser");
    assert_eq!(rig.take(), [Command::SetTool(Tool::Eraser)]);
    rig.click(Role::RadioButton, "Green");
    assert_eq!(rig.take(), [Command::SetColour([0x30, 0xa0, 0x50])]);
    rig.click(Role::Button, "Redo");
    assert_eq!(rig.take(), [Command::Redo]);
    rig.click(Role::Button, "Undo");
    assert_eq!(rig.take(), [], "Undo is disabled");

    rig.act(Role::RadioButton, "Line", Action::Click, None);
    assert_eq!(rig.take(), [Command::SetTool(Tool::Line)]);
    rig.act(Role::RadioButton, "Purple", Action::Click, None);
    assert_eq!(rig.take(), [Command::SetColour([0x80, 0x40, 0xb0])]);
    rig.act(Role::Button, "Redo", Action::Click, None);
    assert_eq!(rig.take(), [Command::Redo]);
    // The controls hold no state of their own: the model did not change, so neither did the tree.
    assert_eq!(faults(&rig.last.tree, &rig.model), Vec::<String>::new());
}

fn the_field_takes_typing_and_gives_a_colour_on_enter(controls: Box<dyn Controls>) {
    let mut rig = Rig::new(controls);
    rig.focus_field();
    assert_eq!(rig.field().value, "202020");
    assert_eq!(rig.field().preedit, None);
    rig.clear();
    for c in ["1", "a", "2", "B", "3", "c"] {
        rig.text(TextEvent::Commit(c.to_string()));
    }
    assert_eq!(rig.field().value, "1a2B3c");
    rig.text(TextEvent::Commit("4".to_string()));
    assert_eq!(rig.field().value, "1a2B3c", "six characters at most");
    rig.key(Key::Left);
    rig.key(Key::Backspace);
    rig.text(TextEvent::Commit("F".to_string()));
    assert_eq!(rig.field().value, "1a2BFc");
    assert!(rig.take().is_empty(), "typing alone asks for nothing");
    rig.key(Key::Enter);
    assert_eq!(rig.take(), [Command::SetColour([0x1a, 0x2b, 0xfc])]);

    // Text that is not a colour gives no command.
    rig.focus_field();
    rig.clear();
    rig.text(TextEvent::Commit("xyz".to_string()));
    rig.key(Key::Enter);
    assert_eq!(rig.take(), []);

    // Assistive technology can set the value of the field it is in.
    rig.focus_field();
    rig.act(
        Role::TextInput,
        "Hex colour",
        Action::SetValue,
        Some(ActionData::Value("00FF00".into())),
    );
    let tree = Tree::new(&rig.last.tree);
    assert_eq!(
        tree.node(rig.id(Role::TextInput, "Hex colour")).value(),
        Some("00FF00")
    );
}

fn composition_goes_through_the_field_unharmed(controls: Box<dyn Controls>) {
    let mut rig = Rig::new(controls);
    rig.focus_field();
    rig.clear();
    rig.text(TextEvent::Commit("ab".to_string()));

    // A dead key: the accent is shown as text being composed and is not part of the value yet.
    rig.text(TextEvent::Preedit("´".to_string()));
    assert_eq!(rig.field().preedit.as_deref(), Some("´"));
    assert_eq!(rig.field().value, "ab");
    // While it is being composed nothing rewrites the field: not a new frame, and not a colour that changed
    // under it (another person's edit, say).
    rig.model.colour = [0xd0, 0x30, 0x30];
    rig.settle();
    assert_eq!(rig.field().preedit.as_deref(), Some("´"));
    assert_eq!(rig.field().value, "ab");
    // The letter completes it.
    rig.text(TextEvent::Commit("é".to_string()));
    assert_eq!(rig.field().preedit, None);
    assert_eq!(rig.field().value, "abé");

    // A composition given up leaves nothing behind.
    rig.text(TextEvent::Preedit("`".to_string()));
    assert_eq!(rig.field().preedit.as_deref(), Some("`"));
    rig.text(TextEvent::CompositionCancelled);
    assert_eq!(rig.field().preedit, None);
    assert_eq!(rig.field().value, "abé");

    // An input method that composes in steps (a smoke test; the first version is Latin only).
    rig.text(TextEvent::Preedit("に".to_string()));
    rig.text(TextEvent::Preedit("にほ".to_string()));
    assert_eq!(rig.field().preedit.as_deref(), Some("にほ"));
    assert_eq!(rig.field().value, "abé");
    rig.text(TextEvent::Preedit("日本".to_string()));
    rig.text(TextEvent::Commit("日本".to_string()));
    assert_eq!(rig.field().preedit, None);
    assert_eq!(rig.field().value, "abé日本");

    // The caret is where an input method should put its window: inside the field.
    let tree = Tree::new(&rig.last.tree);
    let bounds = tree
        .node(rig.id(Role::TextInput, "Hex colour"))
        .bounds()
        .unwrap();
    let caret = rig.field().caret_rect;
    assert!(
        f64::from(caret[0]) >= bounds.x0 - 1.0
            && f64::from(caret[2]) <= bounds.x1 + 1.0
            && f64::from(caret[1]) >= bounds.y0 - 1.0
            && f64::from(caret[3]) <= bounds.y1 + 1.0,
        "caret {caret:?} outside the field {bounds:?}"
    );
    assert!(rig.take().is_empty());
}

/// How many presses of Tab take the focus from nothing to the text field, and the names passed on the way.
/// Not a pass or fail: the two candidates differ here, and the difference is a result.
fn tab_stops(controls: Box<dyn Controls>) -> Vec<String> {
    let mut rig = Rig::new(controls);
    let field = rig.id(Role::TextInput, "Hex colour");
    let mut stops = Vec::new();
    for _ in 0..40 {
        rig.key(Key::Tab);
        let tree = Tree::new(&rig.last.tree);
        let focus = tree.focus();
        stops.push(format!(
            "{:?} {}",
            tree.node(focus).role(),
            tree.text(focus)
        ));
        if focus == field {
            return stops;
        }
    }
    panic!("Tab never reached the field: {stops:?}");
}

fn the_keyboard_reaches_everything(controls: Box<dyn Controls>) {
    let name = controls.name();
    let stops = tab_stops(controls);
    println!(
        "{name}: {} presses of Tab to the field: {stops:?}",
        stops.len()
    );
    assert!(
        !stops.iter().any(|s| s.contains("Undo")),
        "a disabled button is not a stop"
    );
}

macro_rules! for_candidate {
    ($module:ident, $make:expr) => {
        mod $module {
            fn make() -> Box<dyn controls_trial::Controls> {
                Box::new($make)
            }
            #[test]
            fn the_tree_is_what_a_screen_reader_needs() {
                super::the_tree_is_what_a_screen_reader_needs(make());
            }
            #[test]
            fn the_pointer_and_assistive_technology_give_the_same_commands() {
                super::the_pointer_and_assistive_technology_give_the_same_commands(make());
            }
            #[test]
            fn the_field_takes_typing_and_gives_a_colour_on_enter() {
                super::the_field_takes_typing_and_gives_a_colour_on_enter(make());
            }
            #[test]
            fn composition_goes_through_the_field_unharmed() {
                super::composition_goes_through_the_field_unharmed(make());
            }
            #[test]
            fn the_keyboard_reaches_everything() {
                super::the_keyboard_reaches_everything(make());
            }
        }
    };
}

#[cfg(feature = "egui")]
for_candidate!(egui, controls_trial::with_egui::EguiControls::default());
#[cfg(feature = "own")]
for_candidate!(own, controls_trial::own::OwnControls::default());
