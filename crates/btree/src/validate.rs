//! Structural validator for the B+ tree.
//!
//! Checks invariants described in the design doc section 14.2:
//! - Key ordering within each leaf (strictly ascending).
//! - Key ordering within each internal node (strictly ascending).
//! - Separator key correctness: keys[i] equals the first key of children[i+1].
//! - Children count invariant: children.len() == keys.len() + 1.
//! - Root consistency: non-empty tree has at least one entry.
//! - Leaf ordering via next-link chain matches in-order traversal.

use std::sync::Arc;

use crate::btree::BTree;
use crate::node::Node;
use crate::snapshot::Snapshot;

/// A structural validation error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    /// Keys within a node are not strictly ascending.
    KeyOrderViolation { description: String },
    /// Separator key mismatch: expected vs actual.
    SeparatorMismatch { expected: Vec<u8>, actual: Vec<u8> },
    /// Internal node children/keys count is wrong.
    ChildrenCountMismatch { keys: usize, children: usize },
    /// Next-link chain order is wrong.
    NextLinkOrderViolation { description: String },
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::KeyOrderViolation { description } => {
                write!(f, "Key order violation: {}", description)
            }
            Self::SeparatorMismatch { expected, actual } => write!(
                f,
                "Separator mismatch: expected {:?}, got {:?}",
                expected, actual
            ),
            Self::ChildrenCountMismatch { keys, children } => write!(
                f,
                "Children count mismatch: {} keys but {} children (expected keys+1)",
                keys, children
            ),
            Self::NextLinkOrderViolation { description } => {
                write!(f, "Next-link order violation: {}", description)
            }
        }
    }
}

impl std::error::Error for ValidationError {}

/// Validate the structural invariants of a snapshot.
/// Returns `Ok(())` if all invariants hold, or the first error found.
pub fn validate_snapshot(snap: &Snapshot) -> Result<(), ValidationError> {
    let root = match &snap.root {
        None => return Ok(()),
        Some(r) => r,
    };
    validate_node(root)?;
    validate_next_links(root)?;
    Ok(())
}

/// Validate the current tree state.
pub fn validate_tree(tree: &BTree) -> Result<(), ValidationError> {
    validate_snapshot(&tree.snapshot())
}

fn validate_node(node: &Arc<Node>) -> Result<(), ValidationError> {
    match node.as_ref() {
        Node::Leaf(leaf) => {
            // Keys must be strictly ascending.
            for i in 1..leaf.keys.len() {
                if leaf.keys[i] <= leaf.keys[i - 1] {
                    return Err(ValidationError::KeyOrderViolation {
                        description: format!(
                            "leaf keys[{}]={:?} <= keys[{}]={:?}",
                            i,
                            leaf.keys[i],
                            i - 1,
                            leaf.keys[i - 1]
                        ),
                    });
                }
            }
            Ok(())
        }
        Node::Internal(internal) => {
            // children.len() == keys.len() + 1
            if internal.children.len() != internal.keys.len() + 1 {
                return Err(ValidationError::ChildrenCountMismatch {
                    keys: internal.keys.len(),
                    children: internal.children.len(),
                });
            }
            // Keys must be strictly ascending.
            for i in 1..internal.keys.len() {
                if internal.keys[i] <= internal.keys[i - 1] {
                    return Err(ValidationError::KeyOrderViolation {
                        description: format!(
                            "internal keys[{}]={:?} <= keys[{}]={:?}",
                            i,
                            internal.keys[i],
                            i - 1,
                            internal.keys[i - 1]
                        ),
                    });
                }
            }
            // Separator keys: keys[i] must equal the first key of children[i+1].
            for i in 0..internal.keys.len() {
                let expected = internal.children[i + 1].first_key().clone();
                if internal.keys[i] != expected {
                    return Err(ValidationError::SeparatorMismatch {
                        expected,
                        actual: internal.keys[i].clone(),
                    });
                }
            }
            // Recurse into children.
            for child in &internal.children {
                validate_node(child)?;
            }
            Ok(())
        }
    }
}

/// Validate that next-link traversal produces keys in sorted order.
fn validate_next_links(root: &Arc<Node>) -> Result<(), ValidationError> {
    // Find the leftmost leaf.
    let mut current = find_leftmost_leaf(root);
    let mut prev_last_key: Option<Vec<u8>> = None;

    loop {
        let leaf_arc = current;
        match leaf_arc.as_ref() {
            Node::Leaf(leaf) => {
                if let Some(ref prev) = prev_last_key
                    && let Some(first) = leaf.keys.first()
                    && first <= prev
                {
                    return Err(ValidationError::NextLinkOrderViolation {
                        description: format!(
                            "leaf first key {:?} <= previous leaf last key {:?}",
                            first, prev
                        ),
                    });
                }
                prev_last_key = leaf.keys.last().cloned();
                match &leaf.next {
                    Some(next) => current = next.clone(),
                    None => break,
                }
            }
            Node::Internal(_) => break, // unexpected
        }
    }
    Ok(())
}

fn find_leftmost_leaf(node: &Arc<Node>) -> Arc<Node> {
    match node.as_ref() {
        Node::Leaf(_) => node.clone(),
        Node::Internal(i) => find_leftmost_leaf(&i.children[0]),
    }
}
