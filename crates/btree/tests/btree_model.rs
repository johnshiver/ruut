//! Model-based property tests for the CoW B+ tree.
//!
//! We generate random sequences of insert/delete/get/range operations and
//! compare results against `std::collections::BTreeMap` as the reference model.
//! We also retain random snapshots and verify that later mutations never change
//! earlier snapshot results.

use std::collections::BTreeMap;

use btree::{BTree, Snapshot};
use proptest::prelude::*;

// -------------------------------------------------------------------------
// Helper: convert u64 to a big-endian byte key (so lexicographic == numeric order)
// -------------------------------------------------------------------------
fn key(n: u64) -> Vec<u8> {
    n.to_be_bytes().to_vec()
}

fn val(n: u64) -> Vec<u8> {
    n.to_be_bytes().to_vec()
}

// -------------------------------------------------------------------------
// Operations enum for model-based testing
// -------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Op {
    Insert(u64, u64),
    Delete(u64),
    Get(u64),
    Range(u64, u64), // [lo, hi)
    Snapshot,        // save current snapshot
}

fn arb_key() -> impl Strategy<Value = u64> {
    0u64..64
}

fn arb_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (arb_key(), arb_key()).prop_map(|(k, v)| Op::Insert(k, v)),
        2 => arb_key().prop_map(Op::Delete),
        1 => arb_key().prop_map(Op::Get),
        1 => (arb_key(), arb_key()).prop_map(|(a, b)| {
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            Op::Range(lo, hi)
        }),
        1 => Just(Op::Snapshot),
    ]
}

// -------------------------------------------------------------------------
// Property: results of get/range match BTreeMap reference model
// -------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    #[test]
    fn prop_get_matches_model(ops in prop::collection::vec(arb_op(), 0..100)) {
        let mut tree = BTree::new();
        let mut model: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();

        for op in ops {
            match op {
                Op::Insert(k, v) => {
                    tree.insert(key(k), val(v));
                    model.insert(key(k), val(v));
                }
                Op::Delete(k) => {
                    tree.delete_key(&key(k));
                    model.remove(&key(k));
                }
                Op::Get(k) => {
                    let tree_result = tree.get(&key(k));
                    let model_result = model.get(&key(k)).cloned();
                    prop_assert_eq!(tree_result, model_result);
                }
                Op::Range(lo, hi) => {
                    if lo == hi { continue; }
                    let tree_range = tree.range(Some(&key(lo)), Some(&key(hi)));
                    let model_range: Vec<(Vec<u8>, Vec<u8>)> = model
                        .range(key(lo)..key(hi))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();
                    prop_assert_eq!(tree_range, model_range);
                }
                Op::Snapshot => {} // handled separately
            }
        }
    }

    #[test]
    fn prop_snapshot_immutability(ops in prop::collection::vec(arb_op(), 0..150)) {
        let mut tree = BTree::new();
        let mut model: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
        // Vec of (snapshot, model_copy_at_that_point).
        let mut snapshots: Vec<(Snapshot, BTreeMap<Vec<u8>, Vec<u8>>)> = Vec::new();

        for op in ops {
            match op {
                Op::Insert(k, v) => {
                    tree.insert(key(k), val(v));
                    model.insert(key(k), val(v));
                }
                Op::Delete(k) => {
                    tree.delete_key(&key(k));
                    model.remove(&key(k));
                }
                Op::Snapshot => {
                    snapshots.push((tree.snapshot(), model.clone()));
                }
                _ => {}
            }
        }

        // Verify all retained snapshots still reflect their original state.
        for (snap, snap_model) in &snapshots {
            for (mk, mv) in snap_model {
                let result = BTree::get_in_snapshot(snap, mk);
                prop_assert_eq!(result.as_ref(), Some(mv));
            }
            // Verify that no keys that weren't in the snapshot appear.
            let snap_range = BTree::range_snapshot(snap, None, None);
            let model_range: Vec<(Vec<u8>, Vec<u8>)> = snap_model
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            prop_assert_eq!(snap_range, model_range);
        }
    }

    #[test]
    fn prop_range_unbounded_matches_model(ops in prop::collection::vec(arb_op(), 0..100)) {
        let mut tree = BTree::new();
        let mut model: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();

        for op in &ops {
            match op {
                Op::Insert(k, v) => {
                    tree.insert(key(*k), val(*v));
                    model.insert(key(*k), val(*v));
                }
                Op::Delete(k) => {
                    tree.delete_key(&key(*k));
                    model.remove(&key(*k));
                }
                _ => {}
            }
        }

        // Unbounded full scan.
        let tree_all = tree.range(None, None);
        let model_all: Vec<(Vec<u8>, Vec<u8>)> = model
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        prop_assert_eq!(tree_all, model_all);
    }

    #[test]
    fn prop_upsert_matches_model(initial_keys in prop::collection::vec(arb_key(), 1..50)) {
        let mut tree = BTree::new();
        let mut model: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();

        // Insert initial keys with value 1.
        for &k in &initial_keys {
            tree.insert(key(k), val(1));
            model.insert(key(k), val(1));
        }

        // Update them all with value 2.
        for &k in &initial_keys {
            tree.insert(key(k), val(2));
            model.insert(key(k), val(2));
        }

        // Verify all return updated value.
        for &k in &initial_keys {
            prop_assert_eq!(tree.get(&key(k)), Some(val(2)));
        }

        let tree_all = tree.range(None, None);
        let model_all: Vec<(Vec<u8>, Vec<u8>)> = model
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        prop_assert_eq!(tree_all, model_all);
    }
}

// -------------------------------------------------------------------------
// Structural validator integration tests
// -------------------------------------------------------------------------

#[test]
fn test_validate_empty_tree() {
    let tree = BTree::new();
    btree::validate::validate_tree(&tree).unwrap();
}

#[test]
fn test_validate_single_entry() {
    let mut tree = BTree::new();
    tree.insert(key(1), val(1));
    btree::validate::validate_tree(&tree).unwrap();
}

#[test]
fn test_validate_after_many_inserts() {
    let mut tree = BTree::new();
    for i in 0..200u64 {
        tree.insert(key(i), val(i));
        btree::validate::validate_tree(&tree).unwrap();
    }
}

#[test]
fn test_validate_after_deletes() {
    let mut tree = BTree::new();
    for i in 0..100u64 {
        tree.insert(key(i), val(i));
    }
    for i in (0..100u64).step_by(3) {
        tree.delete_key(&key(i));
        btree::validate::validate_tree(&tree).unwrap();
    }
}

#[test]
fn test_snapshot_not_affected_by_later_inserts() {
    let mut tree = BTree::new();
    tree.insert(key(1), val(10));
    tree.insert(key(2), val(20));
    let snap = tree.snapshot();

    tree.insert(key(3), val(30));
    tree.insert(key(1), val(99)); // upsert key(1)

    // snap should still see val(10) for key(1)
    assert_eq!(BTree::get_in_snapshot(&snap, &key(1)), Some(val(10)));
    assert_eq!(BTree::get_in_snapshot(&snap, &key(2)), Some(val(20)));
    assert_eq!(BTree::get_in_snapshot(&snap, &key(3)), None);
}

#[test]
fn test_snapshot_not_affected_by_later_deletes() {
    let mut tree = BTree::new();
    for i in 0..20u64 {
        tree.insert(key(i), val(i));
    }
    let snap = tree.snapshot();

    for i in 0..20u64 {
        tree.delete_key(&key(i));
    }
    assert!(tree.get(&key(0)).is_none());

    // snap still sees all original values
    for i in 0..20u64 {
        assert_eq!(BTree::get_in_snapshot(&snap, &key(i)), Some(val(i)));
    }
}

#[test]
fn test_range_scan_order() {
    let mut tree = BTree::new();
    // Insert in reverse order.
    for i in (0..50u64).rev() {
        tree.insert(key(i), val(i));
    }
    let all = tree.range(None, None);
    for w in all.windows(2) {
        assert!(w[0].0 < w[1].0, "range scan out of order");
    }
}

#[test]
fn test_range_scan_bounded() {
    let mut tree = BTree::new();
    for i in 0..20u64 {
        tree.insert(key(i), val(i));
    }
    let result = tree.range(Some(&key(5)), Some(&key(10)));
    assert_eq!(result.len(), 5);
    assert_eq!(result[0].0, key(5));
    assert_eq!(result[4].0, key(9));
}

#[test]
fn test_delete_nonexistent_returns_false() {
    let mut tree = BTree::new();
    tree.insert(key(1), val(1));
    let before = tree.commit_id();
    let found = tree.delete_key(&key(99));
    assert!(!found);
    assert_eq!(tree.commit_id(), before, "commit id must not advance on NotFound");
}

#[test]
fn test_commit_id_advances_on_mutations() {
    let mut tree = BTree::new();
    assert_eq!(tree.commit_id(), 0);
    tree.insert(key(1), val(1));
    assert_eq!(tree.commit_id(), 1);
    tree.insert(key(2), val(2));
    assert_eq!(tree.commit_id(), 2);
    tree.delete_key(&key(1));
    assert_eq!(tree.commit_id(), 3);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    #[test]
    fn prop_validate_structural_after_ops(ops in prop::collection::vec(arb_op(), 0..200)) {
        let mut tree = BTree::new();
        for op in ops {
            match op {
                Op::Insert(k, v) => { tree.insert(key(k), val(v)); }
                Op::Delete(k) => { tree.delete_key(&key(k)); }
                _ => {}
            }
            btree::validate::validate_tree(&tree)
                .expect("structural invariant violated");
        }
    }
}
