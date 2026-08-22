//! Fundamental types used throughout the btree crate.

/// Monotonically increasing commit identifier.
pub type CommitId = u64;

/// Key type: an owned byte vector. Keys are compared lexicographically.
pub type Key = Vec<u8>;

/// Value type: an owned byte vector.
pub type Value = Vec<u8>;

/// The branching factor (order) of the B+ tree.
/// Each internal node holds at most `2 * ORDER` keys and `2 * ORDER + 1` children.
/// Each leaf node holds at most `2 * ORDER` key-value pairs.
/// This must be >= 2.
pub const ORDER: usize = 4;
