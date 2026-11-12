//! Persisted entities and the split tree.
//!
//! The split tree is a first-class value, not a view artifact: it is what gets
//! persisted, restored, and addressed by the protocol.

use serde::{Deserialize, Serialize};

use crate::agent::{AgentState, Attention};
use crate::ids;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub group_id: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub unread: u32,
    pub created_ms: u64,
    pub root: PaneNode,
    #[serde(default)]
    pub active_surface: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub collapsed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    /// Children side by side (split vertically down the middle).
    Horizontal,
    /// Children stacked (split horizontally across the middle).
    Vertical,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PaneNode {
    Split {
        id: String,
        orientation: Orientation,
        children: Vec<PaneNode>,
        weights: Vec<f32>,
    },
    Leaf {
        pane_id: String,
    },
}

impl PaneNode {
    pub fn leaf(pane_id: impl Into<String>) -> Self {
        Self::Leaf {
            pane_id: pane_id.into(),
        }
    }

    pub fn leaves(&self) -> Vec<&str> {
        match self {
            Self::Leaf { pane_id } => vec![pane_id.as_str()],
            Self::Split { children, .. } => children.iter().flat_map(|c| c.leaves()).collect(),
        }
    }

    pub fn contains(&self, pane_id: &str) -> bool {
        self.leaves().contains(&pane_id)
    }

    /// Split `target` in place, inserting `new_pane` on the trailing side.
    pub fn split(&mut self, target: &str, new_pane: &str, orientation: Orientation) -> bool {
        self.split_side(target, new_pane, orientation, false)
    }

    /// Split `target`, inserting `new_pane` before or after it. `left`/`up`
    /// need the leading side so the new terminal lands where the user asked.
    pub fn split_side(
        &mut self,
        target: &str,
        new_pane: &str,
        orientation: Orientation,
        before: bool,
    ) -> bool {
        match self {
            Self::Leaf { pane_id } if pane_id == target => {
                let existing = Self::leaf(target);
                let new = Self::leaf(new_pane);
                let children = if before {
                    vec![new, existing]
                } else {
                    vec![existing, new]
                };
                *self = Self::Split {
                    id: ids::mint(ids::PANE),
                    orientation,
                    children,
                    weights: vec![0.5, 0.5],
                };
                true
            }
            Self::Leaf { .. } => false,
            Self::Split { children, .. } => children
                .iter_mut()
                .any(|c| c.split_side(target, new_pane, orientation, before)),
        }
    }

    /// Remove a leaf. Returns true when the target was found somewhere in
    /// this subtree (the caller prunes empty children itself).
    pub fn remove(&mut self, target: &str) -> bool {
        match self {
            Self::Leaf { pane_id } => pane_id == target,
            Self::Split {
                children, weights, ..
            } => {
                let mut removed = false;
                let mut i = 0;
                while i < children.len() {
                    let hit = children[i].remove(target);
                    let is_target_leaf = matches!(
                        &children[i],
                        Self::Leaf { pane_id } if pane_id.as_str() == target
                    );
                    if hit && (is_target_leaf || children[i].is_empty()) {
                        removed = true;
                        children.remove(i);
                        if i < weights.len() {
                            weights.remove(i);
                        }
                    } else {
                        removed |= hit;
                        i += 1;
                    }
                }
                if removed {
                    self.normalize();
                }
                removed
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.leaves().is_empty()
    }

    /// Collapse one-child splits and renormalize weights. Call after mutation.
    pub fn normalize(&mut self) {
        if let Self::Split {
            children, weights, ..
        } = self
        {
            for c in children.iter_mut() {
                c.normalize();
            }
            if children.len() == 1 {
                let only = children.remove(0);
                *self = only;
                return;
            }
            while weights.len() < children.len() {
                weights.push(1.0);
            }
            weights.truncate(children.len());
            let sum: f32 = weights.iter().sum();
            if sum > 0.0 {
                for w in weights.iter_mut() {
                    *w /= sum;
                }
            } else if !weights.is_empty() {
                let even = 1.0 / weights.len() as f32;
                for w in weights.iter_mut() {
                    *w = even;
                }
            }
        }
    }

    pub fn set_weight(&mut self, pane_id: &str, weight: f32) -> bool {
        match self {
            Self::Leaf { .. } => false,
            Self::Split {
                orientation: _,
                children,
                weights,
                id: _,
            } => {
                for (i, child) in children.iter_mut().enumerate() {
                    if child.contains(pane_id) {
                        if let Self::Leaf { pane_id: p } = child {
                            if p == pane_id && i < weights.len() {
                                weights[i] = weight.clamp(0.05, 0.95);
                                self.normalize();
                                return true;
                            }
                        }
                        return child.set_weight(pane_id, weight);
                    }
                }
                false
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pane {
    pub id: String,
    pub workspace_id: String,
    pub surfaces: Vec<String>,
    #[serde(default)]
    pub active: usize,
}

impl Pane {
    pub fn active_surface(&self) -> Option<&str> {
        self.surfaces.get(self.active).map(String::as_str)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceKind {
    #[default]
    Terminal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Surface {
    pub id: String,
    pub pane_id: String,
    pub workspace_id: String,
    #[serde(default)]
    pub kind: SurfaceKind,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub pid: Option<u32>,
    #[serde(default)]
    pub tty: Option<String>,
    #[serde(default)]
    pub cols: u16,
    #[serde(default)]
    pub rows: u16,
    /// OSC 9;4 progress, 0..=1.
    #[serde(default)]
    pub progress: Option<f32>,
    #[serde(default)]
    pub bell_count: u32,
    #[serde(default)]
    pub agent_id: Option<String>,
    pub created_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Resume {
    pub argv: Vec<String>,
    pub cwd: String,
    #[serde(default)]
    pub session_ref: Option<String>,
    /// Environment captured when the agent was launched. Ghostty runs the
    /// command through a login shell with a minimal environment, so PATH (and
    /// interpreter locations) must survive a restore.
    #[serde(default)]
    pub env: Vec<(String, String)>,
}

/// A typed agent session bound to exactly one surface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentSurface {
    pub id: String,
    pub surface_id: String,
    pub workspace_id: String,
    pub provider: String,
    pub name: String,
    pub state: AgentState,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub progress: Option<f32>,
    pub attention: Attention,
    pub since_ms: u64,
    pub last_seen_ms: u64,
    #[serde(default)]
    pub tokens_in: u64,
    #[serde(default)]
    pub tokens_out: u64,
    #[serde(default)]
    pub cost_usd: f64,
    #[serde(default)]
    pub resume: Option<Resume>,
    #[serde(default)]
    pub meta: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotifLevel {
    Info,
    NeedsInput,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    pub id: String,
    pub level: NotifLevel,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub surface_id: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    pub created_ms: u64,
    #[serde(default)]
    pub acked: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> PaneNode {
        let mut root = PaneNode::leaf("p1");
        assert!(root.split("p1", "p2", Orientation::Horizontal));
        assert!(root.split("p2", "p3", Orientation::Vertical));
        root
    }

    #[test]
    fn nested_split_and_leaves() {
        let root = tree();
        assert_eq!(root.leaves(), vec!["p1", "p2", "p3"]);
        assert!(root.contains("p3"));
        assert!(!root.contains("p9"));
    }

    #[test]
    fn remove_collapses_single_child_splits() {
        let mut root = tree();
        assert!(root.remove("p2"));
        assert_eq!(root.leaves(), vec!["p1", "p3"]);
        // p1 and the (split) p3 now must be children of one split, not nested
        match &root {
            PaneNode::Split { children, .. } => assert_eq!(children.len(), 2),
            PaneNode::Leaf { .. } => panic!("expected split"),
        }
    }

    #[test]
    fn removing_the_only_leaf_is_reported() {
        let mut root = PaneNode::leaf("solo");
        assert!(!root.remove("other"));
        assert!(root.remove("solo"));
        assert_eq!(root.leaves(), vec!["solo"]);
    }

    #[test]
    fn weights_renormalize() {
        let mut root = tree();
        root.normalize();
        if let PaneNode::Split { weights, .. } = &root {
            let sum: f32 = weights.iter().sum();
            assert!((sum - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn split_side_controls_leaf_order() {
        let mut leading = PaneNode::leaf("p1");
        assert!(leading.split_side("p1", "new", Orientation::Horizontal, true));
        assert_eq!(leading.leaves(), vec!["new", "p1"]);

        let mut trailing = PaneNode::leaf("p1");
        assert!(trailing.split_side("p1", "new", Orientation::Vertical, false));
        assert_eq!(trailing.leaves(), vec!["p1", "new"]);
    }
}
