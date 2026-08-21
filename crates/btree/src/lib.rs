//! # btree
//!
//! Logical in-memory Copy-on-Write B+ tree for the ruut database engine.
//!
//! This crate implements Phase 1 of the technical roadmap: a fully correct
//! in-memory CoW B+ tree that preserves snapshot immutability. Every mutation
//! produces a new root; old snapshots are never altered.
//!
//! ## Invariants
//! - Committed nodes are immutable (enforced via `Arc` sharing).
//! - A mutation always returns a new root, never mutating the prior root's subtree.
//! - Snapshots refer to a root node and remain valid regardless of later mutations.
//! - Range scans return keys in sorted order.

pub mod btree;
pub mod node;
pub mod snapshot;
pub mod types;
pub mod validate;

pub use btree::BTree;
pub use snapshot::Snapshot;
pub use types::{CommitId, Key, Value};
