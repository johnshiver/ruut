//! Snapshot type: a stable, immutable view of a B+ tree at a given commit.

use std::sync::Arc;

use crate::node::Node;
use crate::types::CommitId;

/// A stable, immutable snapshot of the B+ tree at a specific commit.
///
/// Snapshots are lightweight value types. Holding a `Snapshot` keeps the
/// entire subtree reachable via `Arc` reference counting. Later mutations
/// produce new roots and never alter nodes reachable from this snapshot.
#[derive(Clone, Debug)]
pub struct Snapshot {
    /// The commit that produced this snapshot.
    pub commit_id: CommitId,
    /// The root node at the time of this snapshot. `None` means the tree was empty.
    pub root: Option<Arc<Node>>,
}

impl Snapshot {
    /// Create a snapshot representing an empty tree at commit 0.
    pub fn empty() -> Self {
        Snapshot {
            commit_id: 0,
            root: None,
        }
    }
}
