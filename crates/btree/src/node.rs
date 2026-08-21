//! B+ tree node types: `Leaf` and `Internal`.
//!
//! Nodes are immutable once created. All mutations produce new nodes via
//! clone-on-write semantics. `Arc` sharing means unmodified subtrees are
//! never copied.

use std::sync::Arc;

use crate::types::{Key, Value, ORDER};

/// A B+ tree node, either a leaf or an internal node.
#[derive(Clone, Debug)]
pub enum Node {
    Leaf(Leaf),
    Internal(Internal),
}

impl Node {
    /// Return the smallest key in the leftmost leaf reachable from this node.
    /// Used for separator key computation after splits.
    pub fn first_key(&self) -> &Key {
        match self {
            Node::Leaf(l) => &l.keys[0],
            Node::Internal(i) => i.children[0].first_key(),
        }
    }

    /// Return the largest key directly held by this node (for validation).
    pub fn last_key(&self) -> &Key {
        match self {
            Node::Leaf(l) => l.keys.last().unwrap(),
            Node::Internal(i) => i.keys.last().unwrap(),
        }
    }
}

/// A leaf node holding sorted key-value pairs and a next-leaf link.
///
/// - `keys.len() == values.len()` always.
/// - Keys are sorted in strictly ascending order.
/// - A leaf is "full" when `keys.len() == 2 * ORDER`.
/// - A leaf is "underfull" when `keys.len() < ORDER` (tolerated in Phase 1).
#[derive(Clone, Debug)]
pub struct Leaf {
    pub keys: Vec<Key>,
    pub values: Vec<Value>,
    /// Link to the next leaf in key order (for range scans). `None` for the rightmost leaf.
    pub next: Option<Arc<Node>>,
}

impl Leaf {
    pub fn new() -> Self {
        Leaf {
            keys: Vec::new(),
            values: Vec::new(),
            next: None,
        }
    }

    /// Maximum number of entries before a split is needed.
    pub fn max_entries() -> usize {
        2 * ORDER
    }

    pub fn is_full(&self) -> bool {
        self.keys.len() >= Self::max_entries()
    }
}

impl Default for Leaf {
    fn default() -> Self {
        Self::new()
    }
}

/// An internal node holding separator keys and child pointers.
///
/// Invariant: `children.len() == keys.len() + 1`.
/// - `children[i]` contains all keys `k` such that `keys[i-1] <= k < keys[i]`
///   (with sentinels for the boundaries).
/// - An internal node is "full" when `keys.len() == 2 * ORDER`.
#[derive(Clone, Debug)]
pub struct Internal {
    /// Separator keys. `keys[i]` is the smallest key in `children[i+1]`.
    pub keys: Vec<Key>,
    pub children: Vec<Arc<Node>>,
}

impl Internal {
    pub fn new(keys: Vec<Key>, children: Vec<Arc<Node>>) -> Self {
        debug_assert_eq!(keys.len() + 1, children.len());
        Internal { keys, children }
    }

    /// Maximum number of separator keys before a split is needed.
    pub fn max_keys() -> usize {
        2 * ORDER
    }

    pub fn is_full(&self) -> bool {
        self.keys.len() >= Self::max_keys()
    }
}
