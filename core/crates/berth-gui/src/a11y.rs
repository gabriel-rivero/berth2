//! Accessibility tree for the portable window.
//!
//! One window node with one terminal node carrying the visible text of the
//! focused pane. That is what a screen reader needs to read a terminal and to
//! type into it; per-cell structure would add nothing over the text itself.

use accesskit::{Node, NodeId, Role, Tree, TreeUpdate};

pub const WINDOW_ID: NodeId = NodeId(0);
pub const TERMINAL_ID: NodeId = NodeId(1);

/// Build the tree for a window showing `text` in the focused pane.
pub fn tree(title: &str, text: &str) -> TreeUpdate {
    let mut window = Node::new(Role::Window);
    window.set_label(title);
    window.set_children(vec![TERMINAL_ID]);

    let mut terminal = Node::new(Role::Terminal);
    terminal.set_label("Terminal");
    terminal.set_value(text);
    // Focusability and the type-into-it path come from supporting actions.
    terminal.add_action(accesskit::Action::Focus);
    terminal.add_action(accesskit::Action::Click);
    terminal.add_action(accesskit::Action::ReplaceSelectedText);

    TreeUpdate {
        nodes: vec![(WINDOW_ID, window), (TERMINAL_ID, terminal)],
        tree: Some(Tree::new(WINDOW_ID)),
        focus: TERMINAL_ID,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(update: &TreeUpdate, id: NodeId) -> &Node {
        update
            .nodes
            .iter()
            .find(|(node_id, _)| *node_id == id)
            .map(|(_, node)| node)
            .expect("node present")
    }

    #[test]
    fn given_a_screen_when_built_then_the_window_owns_one_terminal_with_its_text() {
        let update = tree("Berth — work", "hello\nworld");
        assert_eq!(node(&update, WINDOW_ID).role(), Role::Window);
        assert_eq!(node(&update, WINDOW_ID).label(), Some("Berth — work"));
        assert_eq!(node(&update, TERMINAL_ID).role(), Role::Terminal);
        assert_eq!(node(&update, TERMINAL_ID).value(), Some("hello\nworld"));
        assert_eq!(update.focus, TERMINAL_ID);
        assert_eq!(update.tree.as_ref().map(|tree| tree.root), Some(WINDOW_ID));
    }
}
