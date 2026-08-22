//! Copy-on-Write B+ tree implementation.
//!
//! The `BTree` holds a current `Snapshot` (commit_id + optional root `Arc<Node>`).
//! Every mutating operation (`insert`, `delete_key`) produces a new root without
//! altering any node reachable from a previously obtained `Snapshot`.
//!
//! ## Snapshot semantics
//!
//! After calling `insert`/`delete_key`, the internal snapshot is advanced. Callers
//! that want to retain the previous state should call `snapshot()` **before**
//! the mutation.
//!
//! ## Deletion rebalancing
//!
//! Phase 1 accepts underfull leaves (no rebalancing / merging). Keys are
//! removed from leaves directly; empty internal nodes are pruned.

use std::sync::Arc;

use crate::node::{Internal, Leaf, Node};
use crate::snapshot::Snapshot;
use crate::types::{CommitId, Key, Value};

// -------------------------------------------------------------------------
// Internal split/insert/delete result types
// -------------------------------------------------------------------------

enum InsertResult {
    Fit(Arc<Node>),
    Split(Arc<Node>, Key, Arc<Node>),
}

enum DeleteResult {
    Removed(Arc<Node>),
    NotFound,
}

// -------------------------------------------------------------------------
// BTree
// -------------------------------------------------------------------------

/// A CoW B+ tree.
///
/// The tree tracks the current committed snapshot internally. Callers may
/// obtain immutable snapshots at any time via [`BTree::snapshot`].
pub struct BTree {
    snapshot: Snapshot,
}

impl Default for BTree {
    fn default() -> Self {
        Self::new()
    }
}

impl BTree {
    /// Create a new, empty B+ tree.
    pub fn new() -> Self {
        BTree {
            snapshot: Snapshot::empty(),
        }
    }

    /// Return the current snapshot (immutable view of the tree).
    pub fn snapshot(&self) -> Snapshot {
        self.snapshot.clone()
    }

    /// Return the current commit id.
    pub fn commit_id(&self) -> CommitId {
        self.snapshot.commit_id()
    }

    // -------------------------------------------------------------------------
    // get
    // -------------------------------------------------------------------------

    /// Look up `key` in the current tree. Returns `None` if absent.
    pub fn get(&self, key: &[u8]) -> Option<Value> {
        Self::get_in(self.snapshot.root.as_deref()?, key)
    }

    /// Look up `key` in an arbitrary snapshot.
    pub fn get_in_snapshot(snap: &Snapshot, key: &[u8]) -> Option<Value> {
        Self::get_in(snap.root().map(Arc::as_ref)?, key)
    }

    fn get_in(node: &Node, key: &[u8]) -> Option<Value> {
        match node {
            Node::Leaf(leaf) => {
                let idx = leaf.keys.binary_search_by(|k| k.as_slice().cmp(key));
                idx.ok().map(|i| leaf.values[i].clone())
            }
            Node::Internal(internal) => {
                let child_idx = Self::find_child(internal, key);
                Self::get_in(&internal.children[child_idx], key)
            }
        }
    }

    // -------------------------------------------------------------------------
    // insert / upsert
    // -------------------------------------------------------------------------

    /// Insert or update `key` → `value`. Advances the commit id.
    pub fn insert(&mut self, key: Key, value: Value) {
        let new_root = match self.snapshot.root.take() {
            None => {
                let mut leaf = Leaf::new();
                leaf.keys.push(key);
                leaf.values.push(value);
                Arc::new(Node::Leaf(leaf))
            }
            Some(root) => match Self::insert_node(root, key, value) {
                InsertResult::Fit(new_root) => new_root,
                InsertResult::Split(left, sep, right) => {
                    Arc::new(Node::Internal(Internal::new(vec![sep], vec![left, right])))
                }
            },
        };
        let threaded = Self::thread_leaves(new_root);
        self.advance(Some(threaded));
    }

    fn insert_node(node: Arc<Node>, key: Key, value: Value) -> InsertResult {
        match Arc::unwrap_or_clone(node) {
            Node::Leaf(mut leaf) => match leaf.keys.binary_search_by(|k| k.as_slice().cmp(&key)) {
                Ok(i) => {
                    leaf.values[i] = value;
                    InsertResult::Fit(Arc::new(Node::Leaf(leaf)))
                }
                Err(i) => {
                    leaf.keys.insert(i, key);
                    leaf.values.insert(i, value);
                    if leaf.is_full() {
                        let (left, sep, right) = Self::split_leaf(leaf);
                        InsertResult::Split(
                            Arc::new(Node::Leaf(left)),
                            sep,
                            Arc::new(Node::Leaf(right)),
                        )
                    } else {
                        InsertResult::Fit(Arc::new(Node::Leaf(leaf)))
                    }
                }
            },
            Node::Internal(mut internal) => {
                let child_idx = Self::find_child(&internal, &key);
                let child = internal.children.remove(child_idx);
                match Self::insert_node(child, key, value) {
                    InsertResult::Fit(new_child) => {
                        internal.children.insert(child_idx, new_child);
                        InsertResult::Fit(Arc::new(Node::Internal(internal)))
                    }
                    InsertResult::Split(left, sep, right) => {
                        internal.children.insert(child_idx, left);
                        internal.children.insert(child_idx + 1, right);
                        internal.keys.insert(child_idx, sep);
                        if internal.is_full() {
                            let (l, s, r) = Self::split_internal(internal);
                            InsertResult::Split(
                                Arc::new(Node::Internal(l)),
                                s,
                                Arc::new(Node::Internal(r)),
                            )
                        } else {
                            InsertResult::Fit(Arc::new(Node::Internal(internal)))
                        }
                    }
                }
            }
        }
    }

    // -------------------------------------------------------------------------
    // delete
    // -------------------------------------------------------------------------

    /// Remove `key` from the tree. Returns `true` if the key was present.
    /// Advances the commit id only when the key was found.
    pub fn delete_key(&mut self, key: &[u8]) -> bool {
        let root = match self.snapshot.root.take() {
            None => {
                self.snapshot.root = None;
                return false;
            }
            Some(r) => r,
        };
        match Self::remove_node(root.clone(), key) {
            DeleteResult::NotFound => {
                self.snapshot.root = Some(root);
                false
            }
            DeleteResult::Removed(new_root) => {
                let collapsed = Self::collapse_root(new_root);
                // If the tree is now empty, store None.
                let final_root = match collapsed.as_ref() {
                    Node::Leaf(l) if l.keys.is_empty() => None,
                    Node::Internal(i) if i.children.is_empty() => None,
                    _ => {
                        let threaded = Self::thread_leaves(collapsed);
                        Some(threaded)
                    }
                };
                self.advance(final_root);
                true
            }
        }
    }

    fn remove_node(node: Arc<Node>, key: &[u8]) -> DeleteResult {
        match Arc::unwrap_or_clone(node) {
            Node::Leaf(mut leaf) => match leaf.keys.binary_search_by(|k| k.as_slice().cmp(key)) {
                Ok(i) => {
                    leaf.keys.remove(i);
                    leaf.values.remove(i);
                    DeleteResult::Removed(Arc::new(Node::Leaf(leaf)))
                }
                Err(_) => DeleteResult::NotFound,
            },
            Node::Internal(mut internal) => {
                let child_idx = Self::find_child(&internal, key);
                let child = internal.children.remove(child_idx);
                match Self::remove_node(child.clone(), key) {
                    DeleteResult::NotFound => {
                        internal.children.insert(child_idx, child);
                        DeleteResult::NotFound
                    }
                    DeleteResult::Removed(new_child) => {
                        let child_is_empty = match new_child.as_ref() {
                            Node::Leaf(l) => l.keys.is_empty(),
                            Node::Internal(i) => i.children.is_empty(),
                        };
                        if child_is_empty {
                            // Remove the separator key that pointed to this child.
                            // keys[i] is the first key of children[i+1].
                            // So child at index 0 has no separator (it's the leftmost).
                            // child at index j > 0 has separator keys[j-1].
                            if !internal.keys.is_empty() {
                                if child_idx == 0 {
                                    // Remove keys[0] (separator to children[1], now to promote).
                                    internal.keys.remove(0);
                                } else {
                                    internal.keys.remove(child_idx - 1);
                                }
                            }
                            // child already removed from children above; don't re-insert.
                        } else {
                            internal.children.insert(child_idx, new_child.clone());
                            // The separator keys[i] == first key of children[i+1].
                            // We modified children[child_idx]. Update keys[child_idx - 1]
                            // which is the separator pointing to children[child_idx], i.e.,
                            // keys[child_idx - 1] = first key of children[child_idx].
                            if child_idx > 0 {
                                internal.keys[child_idx - 1] = new_child.first_key().clone();
                            }
                        }
                        DeleteResult::Removed(Arc::new(Node::Internal(internal)))
                    }
                }
            }
        }
    }

    /// Collapse a root internal node with a single child.
    fn collapse_root(node: Arc<Node>) -> Arc<Node> {
        match node.as_ref() {
            Node::Internal(internal) if internal.children.len() == 1 => {
                internal.children[0].clone()
            }
            _ => node,
        }
    }

    // -------------------------------------------------------------------------
    // range scan
    // -------------------------------------------------------------------------

    /// Return all key-value pairs with keys in `[start, end)` in ascending order.
    /// `start = None` means unbounded lower bound; `end = None` means unbounded upper.
    pub fn range(&self, start: Option<&[u8]>, end: Option<&[u8]>) -> Vec<(Key, Value)> {
        match &self.snapshot.root {
            None => vec![],
            Some(root) => Self::range_in(root, start, end),
        }
    }

    /// Range scan against an arbitrary snapshot.
    pub fn range_snapshot(
        snap: &Snapshot,
        start: Option<&[u8]>,
        end: Option<&[u8]>,
    ) -> Vec<(Key, Value)> {
        match &snap.root {
            None => vec![],
            Some(root) => Self::range_in(root, start, end),
        }
    }

    fn range_in(root: &Arc<Node>, start: Option<&[u8]>, end: Option<&[u8]>) -> Vec<(Key, Value)> {
        let first_leaf = Self::find_first_leaf(root, start);
        let mut result = Vec::new();
        let mut current: Option<Arc<Node>> = Some(first_leaf);

        'outer: while let Some(leaf_arc) = current {
            match leaf_arc.as_ref() {
                Node::Leaf(leaf) => {
                    for (k, v) in leaf.keys.iter().zip(leaf.values.iter()) {
                        if let Some(s) = start
                            && k.as_slice() < s
                        {
                            continue;
                        }
                        if let Some(e) = end
                            && k.as_slice() >= e
                        {
                            break 'outer;
                        }
                        result.push((k.clone(), v.clone()));
                    }
                    current = leaf.next.clone();
                }
                Node::Internal(_) => break,
            }
        }
        result
    }

    fn find_first_leaf(node: &Arc<Node>, start: Option<&[u8]>) -> Arc<Node> {
        match node.as_ref() {
            Node::Leaf(_) => node.clone(),
            Node::Internal(internal) => {
                let child_idx = match start {
                    None => 0,
                    Some(key) => Self::find_child(internal, key),
                };
                Self::find_first_leaf(&internal.children[child_idx], start)
            }
        }
    }

    // -------------------------------------------------------------------------
    // Split helpers
    // -------------------------------------------------------------------------

    fn split_leaf(mut leaf: Leaf) -> (Leaf, Key, Leaf) {
        let mid = leaf.keys.len() / 2;
        let right_keys = leaf.keys.split_off(mid);
        let right_values = leaf.values.split_off(mid);
        let sep = right_keys[0].clone();
        let right = Leaf {
            keys: right_keys,
            values: right_values,
            next: leaf.next.take(),
        };
        (leaf, sep, right)
    }

    fn split_internal(mut internal: Internal) -> (Internal, Key, Internal) {
        let mid = internal.keys.len() / 2;
        let sep = internal.keys.remove(mid);
        let right_keys = internal.keys.split_off(mid);
        let right_children = internal.children.split_off(mid + 1);
        let right = Internal::new(right_keys, right_children);
        (internal, sep, right)
    }

    // -------------------------------------------------------------------------
    // Internal helpers
    // -------------------------------------------------------------------------

    fn find_child(internal: &Internal, key: &[u8]) -> usize {
        match internal.keys.binary_search_by(|k| k.as_slice().cmp(key)) {
            Ok(i) => i + 1,
            Err(i) => i,
        }
    }

    fn advance(&mut self, root: Option<Arc<Node>>) {
        self.snapshot = Snapshot {
            commit_id: self.snapshot.commit_id + 1,
            root,
        };
    }

    // -------------------------------------------------------------------------
    // Leaf next-pointer threading
    // -------------------------------------------------------------------------

    /// Rebuild all leaf `next` pointers in-order. Called after each mutation.
    /// O(n) but simple and correct for Phase 1.
    fn thread_leaves(root: Arc<Node>) -> Arc<Node> {
        // Collect all leaf data in order.
        let mut leaf_data: Vec<Leaf> = Vec::new();
        Self::collect_leaf_data(&root, &mut leaf_data);
        let n = leaf_data.len();
        if n == 0 {
            return root;
        }

        // Build threaded leaf arcs right-to-left.
        let mut threaded: Vec<Arc<Node>> = Vec::with_capacity(n);
        for i in (0..n).rev() {
            let next = if i + 1 < n {
                // threaded is built in reverse; threaded[0] is the rightmost
                let pos = n - 1 - (i + 1); // index into threaded for leaf i+1
                Some(threaded[pos].clone())
            } else {
                None
            };
            let mut l = leaf_data[i].clone();
            l.next = next;
            threaded.push(Arc::new(Node::Leaf(l)));
        }
        threaded.reverse(); // now threaded[i] is the Arc for leaf at position i

        let mut iter = threaded.into_iter();
        Self::rebuild_with_leaves(root, &mut iter)
    }

    fn rebuild_with_leaves(
        node: Arc<Node>,
        leaves: &mut impl Iterator<Item = Arc<Node>>,
    ) -> Arc<Node> {
        match Arc::unwrap_or_clone(node) {
            Node::Leaf(_) => leaves.next().expect("leaf count mismatch"),
            Node::Internal(internal) => {
                let new_children: Vec<Arc<Node>> = internal
                    .children
                    .into_iter()
                    .map(|c| Self::rebuild_with_leaves(c, leaves))
                    .collect();
                Arc::new(Node::Internal(Internal::new(internal.keys, new_children)))
            }
        }
    }

    fn collect_leaf_data(node: &Arc<Node>, out: &mut Vec<Leaf>) {
        match node.as_ref() {
            Node::Leaf(l) => out.push(l.clone()),
            Node::Internal(i) => {
                for child in &i.children {
                    Self::collect_leaf_data(child, out);
                }
            }
        }
    }
}
