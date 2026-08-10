# Design Document: Copy-on-Write (CoW) Relational Database in Rust

**Status:** Draft
**Author:** AI Assistant
**Date:** August 2026

---

## 1. Overview

This document outlines the architecture for a single-node, embedded relational database written in Rust. It diverges from traditional Multi-Version Concurrency Control (MVCC) designs (like PostgreSQL), which overwrite pages in place and rely on a Write-Ahead Log (WAL). Instead, it utilizes a pure **Copy-on-Write (CoW) B+Tree** backed by a memory-mapped file.

This append-only architecture naturally provides zero-cost branching, $O(\log N)$ structural auditing, explicit time-travel queries, and native Optimistic Concurrency Control (OCC) by binding row versions directly to transaction IDs.

## 2. Goals & Non-Goals

### Goals

* **ACID Compliance:** Strict guarantees without relying on a WAL.
* **Memory Safety & Concurrency:** Leverage Rust's borrow checker, `memmap2`, and atomic pointer swaps for thread-safe Snapshot Isolation.
* **Native Time-Travel & Auditing:** Retain historical tree roots to allow queries against past database states.
* **First-Class Optimistic Locking (OCC):** Embed versioning into the tuple layer, eliminating application-level `updated_at` schema hacks.
* **Efficient Single-Node Concurrency:** Support multiple concurrent readers with minimal blocking, and optimistic writes via OCC.

### Non-Goals

* **Distributed Consensus:** This is strictly a single-node, embedded/local engine.
* **Full SQL Compliance Initially:** The storage engine, transaction manager, and tuple serializer are the priority. A full SQL parser (like `sqlparser-rs`) can be layered on later.
* **Dynamic Rebalancing:** This document assumes B+Tree nodes are split/merged conservatively; aggressive rebalancing is deferred to Phase 2.

---

## 3. Architecture & Components

The system is composed of four distinct layers, operating from the lowest level of abstraction upward:

1. **Pager (Storage Layer):** Manages disk I/O, file extension, page allocation, and memory mapping via the `memmap2` crate.
2. **B+Tree (Index Layer):** Handles Copy-on-Write path-copying algorithms for `Insert`, `Update`, and `Delete` operations.
3. **Transaction Manager:** Orchestrates atomic root pointer swaps, validates Optimistic Locking criteria, and manages snapshot isolation.
4. **Relational Tuple Serializer:** Maps relational rows and implicit system metadata (`_sys_version`, `_sys_txid`) into flat byte arrays for B+Tree storage.

### 3.1 The Pager and Memory Mapping

The database exists as a single file on disk. The Pager maps this entire file into memory using `memmap2`.

* **Reads:** Handled entirely via pointer arithmetic against the memory map. The OS virtual memory manager handles page caching, prefetching, and swapping.
* **Writes:** Changes are never written in-place. Instead, new pages are appended to the end of the file. The Pager allocates Page IDs sequentially or reuses freed pages from the Free List (§7).
* **Thread Safety:** Read transactions hold an immutable snapshot (via `arc-swap`) to a specific root Page ID. Writes proceed in isolation and atomically swap the root pointer upon commit.

### 3.2 The Copy-on-Write B+Tree

Modifications never overwrite existing nodes. This is the core invariant enabling lock-free readers.

**Modification Flow:**

1. A write transaction begins with the current Root Page ID.
2. When a leaf node is modified (e.g., a row is inserted), a completely new 4 KB leaf page is allocated.
3. A new parent branch node is allocated to point to the new leaf (and unchanged siblings).
4. This cascades upward through the tree. At each level, a new node is created to reflect the modified path.
5. A new Root Page ID is generated for the tree.
6. **Atomic Swap:** The Meta Page's `RootPageID` field is updated atomically (via an `fsync()`). Readers with snapshots pointing to old roots remain unaffected and see the old tree state; new readers pick up the new root.

**Snapshot Isolation Guarantee:**

Readers accessing the old root are entirely unaffected by writers. Multiple concurrent readers can operate on different versions of the tree without locks or latches. Writers block only during the short Meta Page commit window.

---

## 4. Disk & Page Layout

Pages are fixed at **4096 bytes (4 KB)** to align with the OS virtual memory page size. This alignment eliminates translation overhead and naturally maps to modern hardware.

### 4.1 Meta Page (Page 0)

The Meta Page is the durability and consistency anchor. Committing a transaction is fundamentally an atomic write to this page.

**Layout:**

```
Offset  Size  Field
------  ----  -----
0       4     Magic Number (e.g., 0xCAFEBABE)
4       4     Schema Version (incremented on schema changes)
8       8     Current Master Transaction ID (TxID)
16      8     Root Page ID (pointer to active B+Tree root)
24      8     Free List Root Page ID (pointer to GC tree)
32      4     Total pages allocated (for Free List statistics)
36      ...   Reserved for future metadata
```

**Invariants:**

- The Meta Page is the **only page ever overwritten** in-place.
- All other pages are append-only.
- A transaction commit atomically writes the Meta Page + calls `fsync()`. If the system crashes after the first `fsync()` but before the Meta Page sync, the new pages are orphaned and reclaimed by GC.

### 4.2 Node Pages (Branch & Leaf)

Nodes pack headers and serialized KV pairs efficiently into 4 KB blocks.

**Layout:**

```
Offset  Size  Field
------  ----  -----
0       2     Flags (0x0001 = Leaf, 0x0002 = Branch, 0x0004 = Free List Node)
2       2     Item Count (number of keys in this node)
4       2     Free Space Offset (pointer to the start of unused bytes, growing downward)
6       2     Minimum Key Size (optimization hint for binary search)
8       N*2   Cell Pointers (array of offsets, one per key; enables binary search without shifting)
8+N*2   ...   Payload (Keys/Values, growing backward from end of page)
```

**Cell Pointer Strategy:**

Instead of storing keys and values at the head of the payload region and shifting data on insertion, the design uses an indirection layer:

- Each key/value pair is stored at an arbitrary offset in the Payload region.
- A **Cell Pointer** (2 bytes) at the head of the page records the exact byte offset of that key/value.
- On insertion, a new Cell Pointer is added, and the payload grows into free space—no shifting of existing data.
- Binary search iterates the sorted Cell Pointers to locate keys without touching the payload until comparison.

**Example (Leaf Node):**

```
| Flags=0x0001 | Count=3 | FreeOff=3000 | Padding | [200, 150, 450] | ... unused ... | value3 | value2 | value1 |
                                                    ↑ Cell Pointers              ↑ Payload (grows backward)
```

This design minimizes memory copying on insertion and is typical of high-performance embedded databases (e.g., SQLite, LMDB).

---

## 5. Transaction & Concurrency Flow

### 5.1 Read-Only Transactions

Readers are **lock-free** and see a consistent snapshot of the tree.

**Read Transaction Flow:**

1. A read transaction begins by reading the Meta Page and snapping the current `RootPageID`.
2. It acquires a reference to this root via `arc-swap::ArcSwap<Arc<TreeNode>>`. This reference is immutable.
3. The transaction uses this root to navigate the B+Tree, fetching pages via memory mapping.
4. Multiple concurrent readers can hold different roots simultaneously (e.g., one reading an old version, one reading the newest). The Pager ensures their pages remain in the Free List's non-reclamable set as long as readers exist.
5. When the read transaction ends, the reference is dropped.

**Advantages:**

- No latches or locks required during reads.
- Readers do not interfere with writers (or other readers).
- A reader can inspect historical database states if a root ID is provided.

### 5.2 Write Transactions & Atomic Commits

Writes acquire an exclusive lock on the tree to ensure only one writer at a time.

**Write Transaction Flow:**

1. The writer acquires an exclusive lock (e.g., a `Mutex<>` or hand-rolled seqlock). It reads the current Master Root Page ID from the Meta Page.
2. It performs `INSERT`, `UPDATE`, or `DELETE` operations, using the CoW algorithm to generate new pages. All new pages are buffered in memory (or a temporary staging area).
3. **Commit Phase 1 — Durability:**
   - Flush all buffered pages to the end of the file using standard sequential I/O (appends).
   - Call `fsync()` to ensure the kernel has written all bytes to disk. The kernel may still be ordered to crash at this point.
   - Store the new Root Page ID and TxID in memory.
4. **Commit Phase 2 — Consistency:**
   - Atomically overwrite the Meta Page with the new `TxID`, `RootPageID`, and other metadata.
   - Call `fsync()` again.
   - **Post-commit:** If the system crashes before this second `fsync()`, readers will still see the old root (and the new pages are orphaned; the Free List will reclaim them in a later GC pass).
5. Atomically swap the root pointer via `arc-swap`. New readers now pick up the new root.
6. Release the writer lock.

**Crash Recovery:**

Upon restart, the Pager reads the Meta Page. If the Meta Page is consistent (magic number + schema version match), that root is the authoritative version. Any pages beyond the end of the active tree are considered free and marked for reclamation.

### 5.3 Optimistic Concurrency Control (OCC)

The Transaction Manager enforces **OCC at two levels**: stateful and stateless.

#### Stateful OCC (Long-Lived Transactions)

For long-running transactions (e.g., in an application server), the Transaction Manager tracks:

- **ReadSet:** A hash map of `{RowID → TxID}` recorded at the time the row was read.
- **WriteSet:** A hash map of `{RowID → NewValue}` of rows the transaction intends to write.

**Validation at Commit:**

1. Acquire the writer lock.
2. Re-scan the active tree for all rows in the ReadSet.
3. For each row, verify its current `TxID` (stored in the tuple header) matches the recorded TxID at read-time.
4. If any mismatch is found, **abort the transaction** with a conflict error (the application retries).
5. If all rows in the ReadSet are still valid, proceed with commit (Phases 1 & 2 above).

This ensures that if another transaction modified any row the current transaction read, the validation fails, and the application is notified.

#### Stateless OCC (HTTP/REST Clients)

Clients query the `_sys_version` field and return it in `UPDATE` payloads. The execution engine supports:

```sql
UPDATE users SET status = 'active' WHERE id = 10 EXPECTING VERSION 405;
```

The Tuple Serializer verifies that the current tuple's `TxID` equals 405 before allowing the modification. If the version doesn't match, the engine returns a `ConflictError`, and the client can refetch and retry.

---

## 6. Garbage Collection & The Free List

Since data is append-only, the database file grows monotonically. Without GC, the file would eventually exhaust disk space. The Free List manages page reuse.

### 6.1 Free List Structure

The database maintains a dedicated internal B+Tree called the **Free List**. This tree maps freed `PageID` → `PageFreedTxID` (the transaction ID that made the page safe to reuse).

**Why a separate tree?** Queries are fast (`O(log N)` lookups), and it integrates cleanly with the existing B+Tree infrastructure.

### 6.2 Page Reuse Workflow

**When a page is freed:**

1. A writer replaces an old page with a new copy (the CoW path). The old `PageID` is inserted into the Free List with the current `TxID`.

**When pages can be reused:**

2. A background **Reclamation Thread** monitors the set of active read transactions (held in a thread-safe data structure, e.g., a concurrent hash map or an epoch-based tracker).
3. If the oldest active read transaction has a `TxID` older than some freed page's `PageFreedTxID`, that page is **still not safe** (a reader might still be using it).
4. Only when all active transactions are younger than `PageFreedTxID` is the page marked as reusable.
5. The Pager's allocator consults the Free List before extending the file.

### 6.3 Implementation Notes

- **Epoch-Based Reclamation (Advanced):** If seeking very fine-grained reclamation, the system can use epoch-based GC (similar to the `seize` crate): readers "check in" to the current epoch, and the reclamation thread advances the epoch periodically. This avoids querying active readers for every GC pass.
- **Batch Reuse:** Pages are reused lazily, not immediately. The Pager may batch small allocations or defer reuse to off-peak hours to minimize fragmentation.

---

## 7. Schema Versioning & Forward Compatibility

To support schema evolution without full rewrites:

- The Meta Page includes a `SchemaVersion` field (§4.1).
- Each Tuple includes a `SchemaVersion` in its header (§8.1).
- When a schema change is made (e.g., adding a column), the `SchemaVersion` is incremented.
- The Relational Layer includes a schema registry that defines the encoding for each schema version.
- Tuples with older schema versions are decoded according to their recorded version; missing columns are populated with defaults on-the-fly.

This approach allows readers to coexist with tuples of different schema versions and enables gradual migration.

---

## 8. Relational Tuple Serialization

The Relational Tuple Serializer maps rows into flat byte arrays for the B+Tree.

### 8.1 Tuple Layout

Every tuple, regardless of content, is prefixed with a fixed-size header:

```
| TxID (8 bytes) | SchemaVersion (4 bytes) | Reserved (4 bytes) | Column1 | Column2 | ... |
```

- **TxID:** The transaction ID of the last writer. Serves as the implicit `_sys_version` for OCC.
- **SchemaVersion:** Identifies the schema version of the column data that follows.
- **Reserved:** For future metadata (e.g., tombstone flag, compression codec).
- **Columns:** Variable-length encoding of the row's columns (see §8.2).

### 8.2 Column Encoding

Columns are encoded using a compact format:

- **Fixed-size types** (int, bool): Stored as-is.
- **Variable-length types** (strings, blobs): Prefixed with a 2-byte length, followed by raw bytes.
- **NULL:** Encoded as a special 2-byte marker (e.g., `0xFFFF`).

This enables efficient scanning and slicing without full deserialization.

### 8.3 Indexing & Primary Key

- The **Primary Key** is stored as the B+Tree key (encoded and sortable).
- Non-primary-key columns are stored as the B+Tree value (the full tuple).
- Secondary indexes are separate B+Trees, each mapping (indexed_column_value → primary_key). Queries on secondary indexes perform an index scan + lookup in the primary index.

---

## 9. Consistency & Durability Guarantees

### 9.1 ACID Properties

| Property | Guarantee | Implementation |
|----------|-----------|-----------------|
| **Atomicity** | Writes are all-or-nothing. | Meta Page commit is atomic; if a crash occurs before Meta Page is synced, the transaction is entirely rolled back (new pages are orphaned). |
| **Consistency** | All ACID properties are maintained. | Schema versioning, constraint checking in the execution layer. |
| **Isolation** | Snapshot Isolation (readers see a consistent point-in-time). | CoW B+Tree ensures readers and writers never conflict; each reader uses an immutable tree root. OCC ensures stateful writers validate the ReadSet at commit. |
| **Durability** | Writes are persistent after commit returns. | Two-phase commit with fsync; data is on disk before `COMMIT` returns to the application. |

### 9.2 Crash Recovery

1. On startup, read the Meta Page.
2. Validate the magic number and schema version.
3. Use the `RootPageID` as the active tree root.
4. Pages beyond the end of the active tree are considered orphaned and marked for GC.
5. Any in-flight transaction (whose pages were written but Meta Page not synced) are discarded.

---

## 10. Development Milestones

### Phase 1: Pager & I/O (Weeks 1-2)

**Deliverables:**
- `memmap2` integration.
- Page allocation and deallocation.
- Meta Page read/write.
- Free List scaffolding.

**Validation:** Unit tests for basic page I/O, allocation, and Meta Page serialization.

### Phase 2: CoW B+Tree (Weeks 3-4)

**Deliverables:**
- B+Tree node structures (branch, leaf).
- Insert, update, delete with CoW path-copying.
- Tree splitting and merging.
- Binary search on nodes.

**Validation:** Property-based tests (e.g., QuickCheck) verifying tree invariants (all keys sorted, all leaves at same depth, etc.).

### Phase 3: Transaction Manager (Weeks 5-6)

**Deliverables:**
- Read transaction snapshots (`arc-swap`).
- Write transaction lock & OCC validation.
- Atomic root pointer swaps.
- Basic reclamation thread.

**Validation:** Concurrent read/write tests; OCC conflict detection tests.

### Phase 4: Relational Serialization (Weeks 7-8)

**Deliverables:**
- Tuple encoder/decoder.
- Schema versioning logic.
- Primary key extraction.
- `_sys_version` field.

**Validation:** Round-trip serialization tests; schema evolution tests.

### Phase 5: SQL/API Layer & Time-Travel Queries (Weeks 9-10)

**Deliverables:**
- SQL parser (e.g., `sqlparser-rs`).
- Query executor.
- Time-travel query support (e.g., `SELECT ... AS OF TIMESTAMP` or `SELECT ... FOR VERSION`).
- HTTP API (if embedded in a service).

**Validation:** End-to-end integration tests; performance benchmarks.

---

## 11. Performance Considerations

### 11.1 Read Performance

- Memory-mapped I/O is fast; the OS page cache minimizes disk access.
- Binary search on sorted Cell Pointers is efficient.
- No latches or locks; concurrent readers scale linearly with core count.

### 11.2 Write Performance

- CoW causes memory overhead (new nodes on every write path), but batching writes reduces this.
- Append-only I/O is fast and sequential.
- OCC validation is fast for transactions with small ReadSets.

### 11.3 GC Overhead

- Epoch-based reclamation can amortize the cost.
- Fragmentation: Reusing freed pages from the Free List minimizes file growth.

### 11.4 Future Optimizations

- **Compression:** Pages can be compressed if write amplification is a bottleneck.
- **SIMD Scans:** Vectorize column scans for analytical workloads.
- **Multi-version Leaf Nodes:** Store multiple versions of a row in the same leaf to reduce tree height.

---

## 12. Limitations & Future Work

### Known Limitations

1. **Single-node only:** No replication or distribution.
2. **Write serialization:** Only one writer at a time (can be addressed with timestamp ordering in Phase 3+).
3. **No query optimization:** The SQL layer will initially be simple; cost-based optimization is deferred.

### Future Extensions

1. **Multi-table transactions:** Current design assumes single-table operations; cross-table transactions require careful deadlock prevention.
2. **Distributed MVCC:** Export time-travel snapshots to replicas.
3. **Incremental backup:** Leverage the append-only structure for efficient snapshots.

---

## Appendix A: References

- **LMDB** (Howard et al., 2015): Symas' embedded key-value store using CoW B+Trees.
- **SQLite:** Single-threaded, embedded SQL engine with B+Tree indexes.
- **PostgreSQL MVCC:** Multi-version concurrency control via tuple headers and visibility functions.
- **RocksDB:** LSM-tree-based key-value store (contrast: this design favors read performance over write throughput).

---

**Confidence Score:** 9/10

**Blurb:** This document outlines a highly robust, standard approach to building a memory-mapped CoW database in Rust. It aligns closely with real-world architectural patterns found in embedded systems like LMDB and modern Rust implementations, leveraging specific ecosystem strengths (memmap2, arc-swap, atomic pointer swaps) to achieve safety, concurrency, and performance simultaneously. The design prioritizes clarity and correctness over aggressive optimization; future phases can introduce advanced tuning.
