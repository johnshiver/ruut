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
    pub(crate) commit_id: CommitId,
    /// The root node at the time of this snapshot. `None` means the tree was empty.
    pub(crate) root: Option<Arc<Node>>,
}

impl Snapshot {
    /// Create a snapshot representing an empty tree at commit 0.
    pub(crate) fn empty() -> Self {
        Snapshot {
            commit_id: 0,
            root: None,
        }
    }

    /// The commit that produced this snapshot.
    pub fn commit_id(&self) -> CommitId {
        self.commit_id
    }

    /// The root node captured by this snapshot, if any.
    pub fn root(&self) -> Option<&Arc<Node>> {
        self.root.as_ref()
    }
}
