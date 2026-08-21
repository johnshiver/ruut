# ruut

> A single-node, embedded relational database written in Rust, built on a **Copy-on-Write (CoW) B+ Tree** with page-oriented storage.

## Overview

`ruut` is a storage engine whose fundamental model is an append-only, copy-on-write B+ tree. Every committed database state is represented by an immutable root page and a monotonically increasing commit identifier. Transactions read from a stable snapshot and publish a new root using optimistic concurrency control (OCC).

Key properties:

- **Immutable committed pages** — once written, a page is never modified.
- **Stable snapshots** — identified by `CommitId` + root `PageId`; historical reads are a first-class feature.
- **Crash consistency** — dual superblocks guarantee the engine always recovers to a complete, valid commit.
- **Table-level history retention** — selected tables can retain all historical state for audit and time-travel queries.
- **Explicit on-disk format** — versioned binary codec, not `serde`/`bincode`.
- **Property testing and fuzzing as first-class correctness tools**.

The full technical design lives in [`docs/design.md`](docs/design.md).

## Repository Layout

```
ruut/
├── src/                  # Thin binary entry-point
├── crates/
│   └── pager/            # Existing storage scaffolding (being superseded)
├── docs/
│   └── design.md         # Full technical design and invariants
└── Cargo.toml            # Workspace manifest
```

## Implementation Roadmap

The project is broken into eleven phases. Each phase has a concrete exit criterion; no phase begins until the previous phase's exit criterion is met.

### Phase 0 — Repository Baseline ✅ *Current*

- Document invariants and architecture.
- CI: `cargo fmt`, `clippy`, unit tests, property-test framework.

### Phase 1 — Logical In-Memory CoW B+ Tree

Prove B+ tree algorithms using plain Rust types (`Box`/`Arc`). No page encoding yet.

- [ ] `get`, `put`/`upsert`, `delete`, range scan
- [ ] Splits, root growth
- [ ] Snapshot preservation (mutations never alter old roots)
- [ ] Model-based property tests against `std::collections::BTreeMap`
- [ ] Structural validator

### Phase 2 — Fixed-Size Page Model

Introduce the page abstraction and explicit binary codec.

- [ ] `PageId`, `DirtyPageId`, `PageRef`, `PageBuf` types
- [ ] Page headers and explicit little-endian codec
- [ ] `MemoryPageStore`
- [ ] B+ tree internals converted from Rust references to page references
- [ ] Fuzz page decoding

### Phase 3 — File Persistence

- [ ] `FilePageStore` with positioned I/O (`pread`/`pwrite`)
- [ ] Append-only page allocation
- [ ] Create/open database format
- [ ] Close/reopen integration tests

### Phase 4 — Atomic Commits and Recovery

- [ ] Dual superblocks with generation counters and checksums
- [ ] Dirty-page finalization and root publication protocol
- [ ] Recovery: choose newest valid superblock on open
- [ ] Failpoints and crash/reopen tests

### Phase 5 — Transactions and Coarse OCC

- [ ] `Snapshot` and `Transaction` APIs
- [ ] Private dirty pages invisible before commit
- [ ] Commit rejected when generation differs from base
- [ ] Retryable `Conflict` error and concurrency tests

### Phase 6 — Durable Commit History

- [ ] `CommitRecord` and durable commit-log storage
- [ ] `snapshot(commit_id)` and `snapshot_at(timestamp)`
- [ ] Retention and pinning semantics

### Phase 7 — Relational Catalog and Tables

- [ ] Stable row/key encoding
- [ ] Transactional catalog (table IDs, schemas, history policy)
- [ ] Table primary indexes, schema metadata, basic scalar types
- [ ] Secondary indexes

### Phase 8 — Audited Tables

- [ ] `HistoryPolicy` (`None` / `RetainAll`) on table metadata
- [ ] AS-OF reads through historical snapshots
- [ ] Entity-history API

### Phase 9 — Minimal SQL

- [ ] `CREATE TABLE`, `INSERT`, `SELECT`, `UPDATE`, `DELETE`
- [ ] Transaction statements (`BEGIN` / `COMMIT` / `ROLLBACK`)
- [ ] `AS OF VERSION` / `AS OF SYSTEM TIME` syntax

### Phase 10 — GC, Performance, and mmap

- [ ] Reachability-based page reclamation
- [ ] Offline/stop-the-world compaction
- [ ] Benchmark page sizes and I/O paths
- [ ] Prototype read-only mmap *only after* `FilePageStore` is benchmarked

---

## Getting Started

```bash
# Build the workspace
cargo build --workspace

# Run tests
cargo test --workspace

# Check formatting and lints
cargo fmt --check
cargo clippy --workspace
```

## Design Principles

| Principle | Approach |
|-----------|----------|
| **Staged implementation** | No SQL, mmap, or fine-grained OCC until lower layers are proven correct. |
| **Immutable committed pages** | Every committed page is write-once; new writes allocate new pages. |
| **Explicit binary format** | Disk layout is hand-coded with little-endian routines and format versions. |
| **Dual-superblock crash safety** | A crash mid-commit always leaves the previous valid superblock intact. |
| **Correctness before optimization** | Property tests and fuzzing gate each phase; mmap is a late-phase optimization. |

## References

- [LMDB](https://www.symas.com/lmdb) — embedded CoW B+ tree key-value store.
- [SQLite file format](https://www.sqlite.org/fileformat.html) — page-based storage with slotted pages.

## License

See [LICENSE](LICENSE).
