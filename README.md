# ruut

> A single-node, embedded relational database written in Rust, built on a pure **Copy-on-Write (CoW) B+Tree** backed by a memory-mapped file.

## Overview

`ruut` diverges from traditional MVCC designs (e.g., PostgreSQL) that overwrite pages in-place and rely on Write-Ahead Logs. Instead it uses an append-only storage model that naturally provides:

- **Zero-cost branching** — historical tree roots are retained for free.
- **Lock-free readers** — snapshot isolation via immutable root pointers.
- **Native time-travel queries** — `SELECT ... AS OF VERSION <txid>`.
- **First-class Optimistic Concurrency Control (OCC)** — version numbers embedded in every tuple.
- **ACID guarantees** — without a WAL; durability is achieved through a two-phase Meta Page commit + `fsync`.

The full architectural specification lives in [`docs/design.md`](docs/design.md).

## Repository Layout

```
ruut/
├── src/                  # Thin binary entry-point
├── crates/
│   └── pager/            # Phase 1 — storage layer (page I/O, Meta Page, Free List)
├── docs/
│   └── design.md         # Full design document
└── Cargo.toml            # Workspace manifest
```

## Implementation Roadmap

The project is broken into five phases that map directly to the design document.

### Phase 1 — Pager & I/O (`crates/pager`) 🚀 *Completed*

> Storage layer: memory-mapped files, page allocation, and Meta Page management.

- [x] `memmap2` integration for mmap-backed page access
- [x] Fixed 4 KB page allocation and deallocation
- [x] Meta Page (Page 0) read/write: magic number, schema version, TxID, root page ID
- [x] Free List scaffolding

**Key constants:** `PAGE_SIZE = 4096`, `MAGIC = 0xCAFEBABE`

### Phase 2 — CoW B+Tree (`crates/btree`) 🎯 *Next Task*

> Index layer: Copy-on-Write path-copying for Insert, Update, Delete.

- [ ] Branch and leaf node structures with cell-pointer layout
- [ ] CoW insert / update / delete (never overwrite; always allocate new pages)
- [ ] Node splitting and merging
- [ ] Binary search over sorted cell pointers

### Phase 3 — Transaction Manager (`crates/txn`)

> Orchestrates atomic root swaps, snapshot isolation, and OCC validation.

- [ ] Read transactions — immutable snapshots via `arc-swap`
- [ ] Write transactions — exclusive lock + CoW commit
- [ ] Two-phase commit: flush pages → `fsync` → Meta Page swap → `fsync`
- [ ] OCC validation: ReadSet / WriteSet tracking and conflict detection
- [ ] Background reclamation thread for the Free List

### Phase 4 — Relational Tuple Serializer (`crates/tuple`)

> Maps relational rows (with implicit system fields) into flat byte arrays.

- [ ] Tuple header: `TxID (8B)`, `SchemaVersion (4B)`, `Reserved (4B)`
- [ ] Column encoding: fixed-size types, variable-length strings/blobs, NULL marker
- [ ] Schema versioning and forward-compatible decoding
- [ ] Primary key extraction and secondary index support
- [ ] `_sys_version` and `_sys_txid` implicit columns

### Phase 5 — SQL / API Layer (`crates/sql`)

> Query parser, executor, time-travel queries, and optional HTTP API.

- [ ] SQL parser (`sqlparser-rs`)
- [ ] Query executor (table scan, primary-key lookup, secondary-index scan)
- [ ] Time-travel syntax: `SELECT ... AS OF VERSION <txid>`
- [ ] Stateless OCC syntax: `UPDATE ... EXPECTING VERSION <v>`
- [ ] HTTP/REST API (optional)

---

## Getting Started

```bash
# Build the workspace
cargo build

# Run the placeholder binary
cargo run

# Run tests
cargo test
```

## Design Principles

| Principle | Approach |
|-----------|----------|
| **Append-only storage** | New pages are always written at the end of the file; the Meta Page is the only page overwritten in-place. |
| **Lock-free reads** | Readers hold an immutable reference to a root `PageId`; writers never touch pages a reader can see. |
| **Optimistic writes** | Writers validate their ReadSet at commit time and retry on conflict; no reader ever blocks a writer. |
| **Crash safety** | A crash before the second `fsync` leaves the old Meta Page intact; new pages are orphaned and reclaimed by GC. |

## References

- [LMDB](https://www.symas.com/lmdb) — embedded CoW B+Tree key-value store.
- [SQLite B+Tree](https://www.sqlite.org/fileformat.html) — page-based storage with cell-pointer layout.
- [arc-swap](https://docs.rs/arc-swap) — atomic `Arc` swaps for lock-free root pointer updates.
- [memmap2](https://docs.rs/memmap2) — safe Rust bindings for memory-mapped I/O.

## License

See [LICENSE](LICENSE).
