# Technical Design: Copy-on-Write B+ Tree Relational Database in Rust

**Status:** Initial Architecture / Coding-LLM Implementation Specification
**Date:** August 2026

---

## 1. Executive Summary

Build a single-node database engine in Rust whose storage model is a page-oriented copy-on-write (CoW) B+ tree. Every committed database state is represented by an immutable root page and monotonically increasing commit identifier. Transactions read from a stable snapshot and publish a new root using optimistic concurrency control (OCC). Historical retention is a table-level policy, allowing selected tables to expose durable audit history and time-travel queries while ordinary tables may garbage-collect unreachable historical pages.

The implementation must be staged. Do not begin with SQL, mmap, query optimization, or fine-grained OCC. First establish B+ tree correctness and immutable snapshots; then introduce a page abstraction, explicit binary encoding, durable file storage, crash-safe root publication, coarse OCC, relational tables, history retention, and finally SQL. mmap is an optional read-path optimization after the ordinary file-backed implementation is correct and benchmarked.

---

## 2. Goals and Non-Goals

### 2.1 Primary Goals

- Single-node embedded database written in Rust.
- B+ tree as the fundamental ordered storage structure.
- Copy-on-write page updates: committed pages are immutable.
- Stable snapshots identified by `CommitId`/root `PageId`.
- Optimistic concurrency control as the default transaction model.
- Table-level history retention (e.g. `HISTORY = RETAIN_ALL`).
- Time-travel reads of historical committed table state.
- Crash consistency: after restart, expose either the previous complete commit or the new complete commit, never a partially published tree.
- Explicit, versioned on-disk format independent of Rust struct layout.
- Property testing, fuzzing, and fault injection as first-class correctness mechanisms.
- A path to a useful subset of relational SQL without requiring PostgreSQL feature completeness.

### 2.2 Initial Non-Goals

- Distributed consensus, replication, or multi-node operation.
- Full PostgreSQL compatibility.
- Serializable Snapshot Isolation in the first transaction implementation.
- Fine-grained concurrent writers in the first OCC implementation.
- Online compaction/vacuum in the first durable milestone.
- mmap as a correctness dependency.
- A sophisticated cost-based query optimizer.
- Foreign keys, triggers, stored procedures, or advanced SQL initially.

---

## 3. Core Invariants

These invariants are architectural requirements. Violations are correctness bugs, not implementation choices.

- Committed pages are immutable.
- A committed root identifies a complete, internally consistent database snapshot.
- A transaction never mutates pages reachable from its base committed root.
- A failed or aborted transaction cannot make its private pages reachable from the committed root.
- Root publication occurs only after every page reachable exclusively through the new root has been durably written.
- On restart, the engine chooses the newest valid committed superblock/root and ignores incomplete newer state.
- Page references persisted on disk are stable `PageId`s, never process pointers or Rust references.
- Dirty transaction-local page identifiers cannot be confused with committed `PageId`s.
- Historical snapshots retained by policy remain reachable and cannot be reclaimed.
- Every decode operation validates enough metadata/bounds to reject malformed or corrupt pages safely.
- The logical result of B+ tree operations must match a trusted reference model such as `std::collections::BTreeMap`.

---

## 4. High-Level Architecture

```
SQL Parser / Binder / Planner / Executor       [late phase]
                   |
            Relational Catalog
         Tables / Indexes / Schemas
                   |
            Transaction Manager
         Snapshots + OCC + Commits
                   |
             CoW B+ Tree
      get / put / delete / range scan
                   |
               PageStore
         /           |           \
MemoryPageStore  FilePageStore  MmapReadStore
     tests          baseline       optional
                   |
           database file(s)
```

### 4.1 Layering Rule

Higher layers may depend on lower layers; lower layers must not know about SQL or relational semantics. The B+ tree operates on encoded byte keys/values and page references. `PageStore` does not know about B+ tree nodes. This separation permits the same tree implementation to run against an in-memory page store and a file-backed store.

---

## 5. Fundamental Types

```rust
pub type CommitId = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PageId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DirtyPageId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PageRef {
    Committed(PageId),
    Dirty(DirtyPageId),
}

#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
    pub commit_id: CommitId,
    pub root: PageId,
}
```

`PageId` and `DirtyPageId` are kept distinct as deliberate type-level protection against accidentally serializing an uncommitted reference into durable state.

---

## 6. B+ Tree Design

### 6.1 Initial Logical Tree

The first milestone may use convenient Rust-owned nodes (`Box`/`Arc`) solely to prove algorithms. Required operations: `get`, `insert`/`upsert`, `delete`, ordered range scan, leaf split, internal split, root split, and snapshot preservation. Deletion rebalancing may be deferred if tombstones or underfull pages are explicitly accepted for the first milestone.

### 6.2 Page-Oriented Tree

After logical correctness, replace pointer-to-node relationships with `PageRef`/`PageId` relationships and encode nodes into fixed-size pages. This is the decisive transition toward a storage engine.

### 6.3 Page Size

Make page size a format/configuration parameter during development. Benchmark 4 KiB, 8 KiB, and 16 KiB later. Do not scatter a magic page size throughout algorithms. A default of 8 KiB or 16 KiB is reasonable for development.

### 6.4 Slotted Page Format

**Leaf page (conceptual)**

```
+------------------------------+
| Page header                  |
| magic / format version       |
| page type                    |
| entry count                  |
| page id                      |
| creation commit (optional)   |
| next leaf PageId             |
| free_start / free_end        |
| checksum                     |
+------------------------------+
| slot directory               |
| (offset, length) ...         |
+------------------------------+
| free space                   |
+------------------------------+
| variable-length records      |
+------------------------------+
```

Use explicit little-endian encode/decode routines. Do not persist Rust structs directly and do not make `serde`/`bincode` the canonical database format. The disk format must be deliberately versioned and independently testable.

---

## 7. PageStore

```rust
pub trait PageStore {
    fn read_page(&self, id: PageId) -> Result<Box<[u8]>>;
    fn write_page(&mut self, id: DirtyPageId, data: Box<[u8]>) -> Result<()>;
    fn allocate(&mut self) -> DirtyPageId;
    fn commit(&mut self, root: DirtyPageId) -> Result<Snapshot>;
}
```

### 7.1 MemoryPageStore

Used in tests and Phase 1/2. Stores pages in a `HashMap`. Supports full snapshot semantics without disk I/O.

### 7.2 FilePageStore

Baseline durable implementation. Uses positioned I/O (`pread`/`pwrite`). Append-only page allocation. Atomic root publication via dual superblocks.

### 7.3 MmapReadStore (optional, Phase 10)

Read-only `mmap` overlay over `FilePageStore`. Introduced only after `FilePageStore` is benchmarked and proven correct.

---

## 8. Storage Format

### 8.1 Superblock

The database maintains two superblock slots (A and B). Each superblock contains:

| Offset | Size | Field |
|--------|------|-------|
| 0 | 8 | Magic (`0x52555554_42545245` — "RUUTBTRE") |
| 8 | 4 | Format version |
| 12 | 4 | Superblock slot (0 or 1) |
| 16 | 8 | Generation counter (monotonically increasing) |
| 24 | 8 | `CommitId` |
| 32 | 8 | Root `PageId` |
| 40 | 8 | Total pages allocated |
| 48 | 8 | Commit log tail `PageId` |
| 56 | 8 | Checksum (CRC32 or xxHash over bytes 0–55) |

On restart, read both superblocks, validate checksums, and choose the one with the higher generation counter. The other slot (or one with invalid checksum) is ignored.

### 8.2 Page Header

Every page begins with a fixed-size header:

| Offset | Size | Field |
|--------|------|-------|
| 0 | 4 | Page magic |
| 4 | 2 | Format version |
| 6 | 2 | Page type (leaf / internal / overflow / free) |
| 8 | 8 | `PageId` (self-identifying) |
| 16 | 8 | Creation `CommitId` |
| 24 | 4 | Entry count |
| 28 | 4 | Checksum |

---

## 9. Transaction Manager

### 9.1 Snapshots

A `Snapshot` pairs a `CommitId` with the root `PageId` at that commit. Read transactions hold a `Snapshot` and may not observe later writes. Snapshots are lightweight — they are plain value types, not reference-counted objects.

### 9.2 Write Transactions

A write transaction begins from a base `Snapshot`, accumulates dirty pages in a private `DirtyPageId`-indexed buffer, and publishes a new root atomically at commit. The `DirtyPageId` namespace is transaction-local and cannot be confused with committed `PageId`s.

### 9.3 Coarse OCC

The first OCC implementation uses a single generation counter. A commit is rejected if the current generation differs from the transaction's base generation. Applications catch the `Conflict` error and retry. Finer-grained OCC (read-set tracking) is deferred until benchmarks justify it.

### 9.4 Commit Protocol

1. Finalize all dirty pages: encode, checksum, write to `FilePageStore`.
2. `fsync` data pages.
3. Write the new superblock to the alternate slot with an incremented generation counter.
4. `fsync` superblock.
5. Increment in-memory generation; make the new `Snapshot` visible.

A crash at any point before step 4 leaves the previous superblock intact. A crash between steps 4 and 5 is safe: the new superblock is valid on the next open.

---

## 10. Durable Commit History

```rust
pub struct CommitRecord {
    pub commit_id: CommitId,
    pub root: PageId,
    pub timestamp: Timestamp,
    // later: transaction/user metadata
}
```

This provides an ordered database timeline and supports `snapshot(commit_id)` and `snapshot_at(timestamp)`. Initially the commit history is linear.

**Important:** Snapshot capability and retention policy are separate. MVCC only requires old versions while readers may need them. Audit history intentionally pins historical state beyond normal MVCC lifetime.

---

## 11. Relational Layer

### 11.1 Catalog

Once the storage/transaction layer is durable, add a catalog containing table IDs, names, schema versions, column definitions, primary indexes, secondary indexes, and history policy. Catalog changes are themselves transactional.

### 11.2 Tables

Represent each table primarily as a B+ tree keyed by encoded primary key. Secondary indexes are additional B+ trees whose updates occur in the same transaction.

```rust
struct TableDescriptor {
    table_id: TableId,
    name: String,
    schema: Schema,
    primary_root: PageId,
    secondary_indexes: Vec<IndexDescriptor>,
    history_policy: HistoryPolicy,
}
```

### 11.3 History Policy

```rust
pub enum HistoryPolicy {
    None,
    RetainAll,
    // later:
    RetainFor(Duration),
}
```

For the first history implementation, exploit retained snapshots rather than duplicating every row version into a separate temporal table. Add specialized per-entity historical indexes only if benchmarks show that queries such as "all versions of entity X" are too expensive when resolved through commit snapshots.

---

## 12. SQL Scope

SQL comes after durable tables. Start with a deliberately small grammar and grow it.

- `CREATE TABLE` with `PRIMARY KEY` and optional history policy.
- `INSERT`.
- `SELECT` by primary key.
- `UPDATE` by primary key.
- `DELETE` by primary key.
- Simple `WHERE` predicates.
- Ordered range scans.
- Basic secondary indexes.
- `AS OF VERSION` / `AS OF SYSTEM TIME`.
- Transactions: `BEGIN` / `COMMIT` / `ROLLBACK`.

Defer joins, aggregates, complex expressions, optimizer statistics, and PostgreSQL compatibility until the storage and transaction semantics are proven.

---

## 13. Garbage Collection / Vacuum

Append-only CoW storage eventually accumulates unreachable pages. GC is required for a long-running production engine but should not block the first durable prototype.

A future collector computes reachability from all roots that must remain live: current root, active transaction snapshots, retained historical commits, and any administrative pins. Unreachable pages may then be reclaimed or compacted into a new file. `RetainAll` tables constrain reclamation because pages needed to reconstruct their historical state remain live.

Do not implement page reuse before crash recovery is solid; append-only allocation makes early correctness reasoning substantially easier.

---

## 14. Testing Strategy

### 14.1 Model-Based Property Tests

Generate random sequences of insert/update/delete/get/range operations and compare logical results with `std::collections::BTreeMap`. Retain random snapshots and verify that later mutations never change earlier snapshot results.

### 14.2 Structural Validation

Implement a debug validator that recursively checks B+ tree invariants: key ordering, separator correctness, legal occupancy, child count, leaf ordering, next-leaf links, reachable `PageId`s, absence of cycles, and root consistency.

### 14.3 Persistence Tests

- Create → write → close → reopen → read.
- Multiple commits → reopen → read current state.
- Multiple commits → reopen → read retained historical snapshots.
- Large split-heavy workloads.
- Delete-heavy workloads.
- Corrupt checksum/header detection.
- Truncated-file handling.

### 14.4 Crash Tests

Run transactions in a child process, kill the process at randomized commit failpoints, reopen, validate the tree, and compare the visible state against the set of legally committed outcomes.

### 14.5 Fuzzing

Fuzz page decoders independently; arbitrary bytes must produce either a validated node or a controlled error, never panic/UB. Fuzz operation sequences and reopen cycles as separate targets.

---

## 15. Observability and Diagnostics

- Expose current `CommitId`/root `PageId`.
- Tree validator command/API.
- Page dump/decoder tool.
- Commit-log inspection tool.
- Tree statistics: height, page count, fill factor, key count.
- Transaction conflict counters.
- Bytes/pages written per commit.
- Optional tracing spans around reads, splits, commits, syncs, and recovery.

---

## 16. Module Layout

```
src/
 lib.rs
 error.rs

 page/
   mod.rs
   id.rs
   buf.rs
   codec.rs
   store.rs
   memory.rs
   file.rs
   checksum.rs

 btree/
   mod.rs
   node.rs
   leaf.rs
   internal.rs
   cursor.rs
   split.rs
   delete.rs
   validate.rs

 txn/
   mod.rs
   snapshot.rs
   transaction.rs
   dirty.rs
   commit.rs
   occ.rs

 storage/
   mod.rs
   superblock.rs
   recovery.rs
   commit_log.rs
   allocator.rs

 catalog/
   mod.rs
   schema.rs
   table.rs
   index.rs
   history.rs

 sql/                 # late phase
   parser.rs
   binder.rs
   plan.rs
   executor.rs

tests/
 btree_model.rs
 snapshots.rs
 page_codec.rs
 persistence.rs
 recovery.rs
 occ.rs
 history.rs
 crash.rs

fuzz/
 fuzz_targets/
   page_decode.rs
   operation_sequence.rs
```

---

## 17. Implementation Roadmap

### Phase 0 — Repository Baseline ✅

- Document invariants and current architecture.
- Add `cargo fmt`, `clippy`, unit tests, property-test framework, and CI.
- Keep dependencies minimal and justify storage-format dependencies.

**Exit criterion:** Clean CI and an architecture/invariants document checked into the repo.

### Phase 1 — Logical In-Memory CoW B+ Tree

- Implement `get`, `put`/`upsert`, `delete`, range scan.
- Implement splits/root growth.
- Ensure a mutation creates a new root and does not alter old snapshots.
- Add model-based property tests against `BTreeMap`.
- Add structural validator.

**Exit criterion:** Randomized operation sequences and retained snapshots pass reliably.

### Phase 2 — Fixed-Size Page Model

- Introduce `PageId`, `DirtyPageId`, `PageRef`, `PageBuf`.
- Design page headers and explicit binary codec.
- Implement `MemoryPageStore`.
- Convert B+ tree internals from Rust object references to page references.
- Fuzz page decoding.

**Exit criterion:** The same logical/property tests pass against encoded in-memory pages.

### Phase 3 — File Persistence

- Implement `FilePageStore` with positioned IO.
- Append-only page allocation.
- Create/open database format.
- Persist and validate metadata.
- Add close/reopen integration tests.

**Exit criterion:** Data survives process restart and the tree validator passes after reopen.

### Phase 4 — Atomic Commits and Recovery

- Implement dual superblocks with generation counters and checksums.
- Implement dirty-page finalization and root publication.
- Implement recovery choosing the newest valid superblock.
- Add failpoints and crash/reopen tests.

**Exit criterion:** Arbitrary simulated crashes during commit yield either the previous or new valid commit, never torn logical state.

### Phase 5 — Transactions and Coarse OCC

- Add `Snapshot` and `Transaction` APIs.
- Transactions read from immutable base roots.
- Private dirty pages remain invisible before commit.
- Reject commit when current generation differs from base generation.
- Add retryable `Conflict` error and concurrency tests.

**Exit criterion:** Concurrent writers have deterministic, tested conflict behavior; readers remain snapshot-consistent.

### Phase 6 — Durable Commit History

- Add `CommitRecord` and durable commit-log storage.
- Support `snapshot(commit_id)`.
- Support `snapshot_at(timestamp)` if timestamps are reliable.
- Define retention/pinning semantics.

**Exit criterion:** Historical committed roots survive restart and can be opened as read-only snapshots.

### Phase 7 — Relational Catalog and Tables

- Define stable row/key encoding.
- Add transactional catalog.
- Implement table primary indexes.
- Implement schema metadata and basic scalar types.
- Add secondary indexes after primary-table semantics are stable.

**Exit criterion:** Typed/internal relational API supports transactional CRUD and index maintenance.

### Phase 8 — Audited Tables

- Add `HistoryPolicy` to table metadata.
- Pin/reclaim historical state according to policy.
- Implement AS-OF table reads through historical snapshots.
- Add entity-history API; initially permit snapshot traversal if necessary.
- Benchmark and decide whether per-entity version indexes are warranted.

**Exit criterion:** A history-enabled table can return old entity/table states after later updates and process restart.

### Phase 9 — Minimal SQL

- Choose or implement parser strategy.
- Implement `CREATE TABLE`, `INSERT`, `SELECT`, `UPDATE`, `DELETE`.
- Add transaction statements.
- Add `AS OF VERSION` / `SYSTEM TIME` syntax.
- Keep planner simple and deterministic.

**Exit criterion:** End-to-end SQL demonstrates the core differentiators: relational CRUD, OCC conflicts, and native historical queries.

### Phase 10 — GC, Performance, and mmap

- Implement reachability accounting/pinning.
- Implement offline or stop-the-world compaction first.
- Benchmark page sizes and cache behavior.
- Profile syscall/copy overhead of `FilePageStore`.
- Only then prototype read-only mmap or segmented mmap.
- Benchmark mmap against positioned IO plus an explicit page cache.
- Add finer-grained OCC only after contention benchmarks justify it.

**Exit criterion:** Optimizations are supported by benchmarks and preserve all crash/property tests.

---

## 18. Coding-LLM Operating Instructions

Use the following constraints when asking an LLM to modify the repository.

- Work one roadmap phase or narrowly scoped issue at a time.
- Before editing, inspect the existing repository and summarize relevant modules/invariants.
- Do not replace working architecture wholesale unless explicitly requested.
- For each change, state which invariants are affected.
- Prefer small reviewable commits/diffs.
- Add or update tests in the same change as implementation.
- Never claim persistence/crash safety without a test demonstrating the relevant failure boundary.
- Do not introduce mmap, unsafe code, SQL, GC, or fine-grained OCC ahead of the roadmap merely because it appears more advanced.
- Avoid `serde`/`bincode` as the canonical page format; use explicit codecs.
- Treat `clippy` warnings, panics on corrupt disk input, unchecked offsets, and integer-overflow risks as correctness issues.
- When uncertain about a database invariant, stop and explain the ambiguity instead of guessing.
- After implementation, run formatting, tests, `clippy`, and relevant property/fuzz tests; report exact results.

### Recommended Task Prompt Template

```
You are modifying an existing Rust database repository.

Current roadmap phase: <phase number and name>
```
