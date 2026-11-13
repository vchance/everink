//! What both candidates must do, read off the accessibility tree: the same tree is what a screen reader gets, so
//! a toolbar that passes here is one a screen reader can be pointed at in part 4b.

use std::collections::HashMap;

use accesskit::{Node, NodeId, Role, Toggled, TreeUpdate};

/// A tree as a candidate gave it, for looking things up.
pub struct Tree<'a> {
    pub update: &'a TreeUpdate,
    nodes: HashMap<NodeId, &'a Node>,
    parent: HashMap<NodeId, NodeId>,
}

impl<'a> Tree<'a> {
    pub fn new(update: &'a TreeUpdate) -> Tree<'a> {
        let nodes: HashMap<NodeId, &Node> = update.nodes.iter().map(|(id, n)| (*id, n)).collect();
        let mut parent = HashMap::new();
        for (id, node) in &update.nodes {
            for child in node.children() {
                parent.insert(*child, *id);
            }
        }
        Tree {
            update,
            nodes,
            parent,
        }
    }

    pub fn node(&self, id: NodeId) -> &'a Node {
        self.nodes[&id]
    }

    /// What a screen reader would say for the node: its label, or for a label its text.
    pub fn text(&self, id: NodeId) -> String {
        let node = self.node(id);
        node.label()
            .or_else(|| node.value())
            .unwrap_or_default()
            .to_string()
    }

    /// The node of this role whose text is `text`.
    pub fn find(&self, role: Role, text: &str) -> Option<NodeId> {
        self.update
            .nodes
            .iter()
            .find(|(id, n)| n.role() == role && self.text(*id) == text)
            .map(|(id, _)| *id)
    }

    /// The node of this role whose text starts with `text`.
    pub fn find_prefix(&self, role: Role, text: &str) -> Option<NodeId> {
        self.update
            .nodes
            .iter()
            .find(|(id, n)| n.role() == role && self.text(*id).starts_with(text))
            .map(|(id, _)| *id)
    }

    /// Whether `id` is inside a node of this role with this text.
    pub fn inside(&self, id: NodeId, role: Role, text: &str) -> bool {
        let mut at = id;
        while let Some(up) = self.parent.get(&at) {
            if self.node(*up).role() == role && self.text(*up) == text {
                return true;
            }
            at = *up;
        }
        false
    }

    pub fn checked(&self, id: NodeId) -> bool {
        self.node(id).toggled() == Some(Toggled::True)
    }

    /// The middle of the node, in surface pixels. Both candidates give bounds in the surface's own pixels.
    pub fn centre(&self, id: NodeId) -> (f32, f32) {
        let b = self.node(id).bounds().expect("a node with bounds");
        (((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32)
    }

    pub fn focus(&self) -> NodeId {
        self.update.focus
    }
}

/// What is wrong with a tree for this model, as a list of sentences; empty when nothing is.
pub fn faults(update: &TreeUpdate, model: &crate::UiModel) -> Vec<String> {
    let tree = Tree::new(update);
    let mut out = Vec::new();
    if tree.find(Role::Toolbar, "Tools").is_none() {
        out.push("no toolbar named Tools".to_string());
    }
    for group in ["Tool", "Colour"] {
        if tree.find(Role::RadioGroup, group).is_none() {
            out.push(format!("no radio group named {group}"));
        }
    }
    let mut radio = |group: &str, name: &str, on: bool| match tree.find(Role::RadioButton, name) {
        None => out.push(format!("no radio button named {name}")),
        Some(id) => {
            if !tree.inside(id, Role::RadioGroup, group) {
                out.push(format!("{name} is not inside the group {group}"));
            }
            if tree.checked(id) != on {
                out.push(format!("{name} is {}checked", if on { "not " } else { "" }));
            }
            if !tree.node(id).supports_action(accesskit::Action::Click) {
                out.push(format!("{name} cannot be clicked"));
            }
            if tree.node(id).bounds().is_none() {
                out.push(format!("{name} has no bounds"));
            }
        }
    };
    for tool in crate::Tool::ALL {
        radio("Tool", tool.name(), tool == model.tool);
    }
    for (colour, name) in crate::SWATCHES {
        radio("Colour", name, colour == model.colour);
    }
    for (name, enabled) in [("Undo", model.can_undo), ("Redo", model.can_redo)] {
        match tree.find(Role::Button, name) {
            None => out.push(format!("no button named {name}")),
            Some(id) => {
                if tree.node(id).is_disabled() == enabled {
                    out.push(format!(
                        "{name} is {}",
                        if enabled { "disabled" } else { "not disabled" }
                    ));
                }
            }
        }
    }
    let zoom = format!("Zoom {}%", model.zoom_percent);
    if tree.find(Role::Label, &zoom).is_none() && tree.find(Role::Status, &zoom).is_none() {
        out.push(format!("nothing says {zoom}"));
    }
    match tree.find(Role::TextInput, "Hex colour") {
        None => out.push("no text field named Hex colour".to_string()),
        Some(id) => {
            if !tree.node(id).supports_action(accesskit::Action::Focus) {
                out.push("the text field cannot be focused".to_string());
            }
        }
    }
    out
}
