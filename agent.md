# Agent Guidelines for `ruut`

This file describes conventions, architecture context, and workflows to help AI agents work productively on this codebase.

---

## Project Summary

`ruut` is a single-node, embedded relational database written in Rust. It uses an **append-only, Copy-on-Write (CoW) B+Tree** backed by a memory-mapped file (`memmap2`) for ACID guarantees — no Write-Ahead Log required. The full architectural specification is in [`docs/design.md`](docs/design.md).

---

## Repository Layout

```
ruut/
├── src/                  # Thin binary entry-point (main.rs)
├── crates/
│   └── pager/            # Phase 1 — storage layer (COMPLETE)
│       └── src/          # Page I/O, Meta Page, Free List
├── docs/
│   └── design.md         # Authoritative architecture document
├── Cargo.toml            # Workspace manifest (also root package)
└── agent.md              # This file
```

---

## Build, Test & Lint Commands

```bash
# Build the entire workspace
cargo build --workspace

# Run all tests
cargo test --workspace

# Check without producing a binary (faster)
cargo check --workspace

# Lint (Clippy)
cargo clippy --workspace -- -D warnings

# Format
cargo fmt --all

# Format check (CI-style)
cargo fmt --all -- --check
```

All CI checks (build + test) are defined in `.github/workflows/ci.yml` and run on every push/PR.

---

## Implementation Roadmap

Work progresses in strict phases. **Do not implement a later phase before the prior one is complete and tested.**

| Phase | Crate | Status | Description |
|-------|-------|--------|-------------|
| 1 | `crates/pager` | ✅ Complete | Storage layer: mmap, page allocation, Meta Page |
| 2 | `crates/btree` | 🎯 Next | CoW B+Tree: insert, update, delete, split/merge |
| 3 | `crates/txn` | ⬜ Pending | Transaction manager: OCC, snapshot isolation, commit |
| 4 | `crates/tuple` | ⬜ Pending | Relational tuple serializer, system columns |
| 5 | `crates/sql` | ⬜ Pending | SQL parser, executor, time-travel queries |

When starting a new phase, create its crate under `crates/` and add it to the `[workspace]` members list in the root `Cargo.toml`.

---

## Core Architectural Invariants

These must **never** be violated:

1. **Append-only pages** — only the Meta Page (Page 0) is ever overwritten in-place. All other writes allocate new pages at the end of the file.
2. **Lock-free reads** — readers hold an immutable snapshot root `PageId`; writers must not modify pages a reader can see.
3. **Atomic commit** — a transaction commit = flush new pages → `fsync` → overwrite Meta Page → `fsync`. A crash between the two `fsync`s leaves orphaned pages that the Free List GC reclaims.
4. **PAGE_SIZE = 4096** and **MAGIC = 0xCAFEBABE** are fixed constants in `crates/pager`.

---

## Coding Conventions

- **Rust edition 2024** across all crates.
- Prefer `thiserror` for library error types; use `anyhow` only in binary entry-points if needed.
- All public API items must have doc comments (`///`).
- Write unit tests in the same file under `#[cfg(test)]`; use `tempfile` for any file-system tests.
- No `unwrap()` or `expect()` in library code — propagate errors with `?`.
- Keep `unsafe` blocks minimal, well-commented, and only where required by `memmap2` or low-level page arithmetic.
- Run `cargo clippy --workspace -- -D warnings` before submitting; fix all warnings.
- Run `cargo fmt --all` before submitting.

---

## Key Types & Concepts (Phase 1 — Pager)

| Type / Constant | Location | Purpose |
|-----------------|----------|---------|
| `PAGE_SIZE` | `crates/pager/src/` | 4096-byte fixed page size |
| `MAGIC` | `crates/pager/src/` | `0xCAFEBABE` file identity sentinel |
| `MetaPage` | `crates/pager/src/` | Page 0: magic, schema version, TxID, root PageId, free-list root |
| `Pager` | `crates/pager/src/` | mmap handle, page allocation/deallocation, read/write accessors |
| `PageId` | `crates/pager/src/` | Newtype `u64` page address |

---

## Adding a New Crate

1. `cargo new --lib crates/<name>`
2. Add `"crates/<name>"` to `[workspace].members` in the root `Cargo.toml`.
3. Add a `[package]` description matching the style in `crates/pager/Cargo.toml`.
4. Add a dependency on `pager` (or other prerequisite crates) via `{ path = "crates/pager" }`.

---

## References

- [`docs/design.md`](docs/design.md) — full specification (read this before making architectural decisions)
- [LMDB](https://www.symas.com/lmdb) — reference CoW B+Tree design
- [SQLite file format](https://www.sqlite.org/fileformat.html) — cell-pointer page layout
- [memmap2 docs](https://docs.rs/memmap2)
- [arc-swap docs](https://docs.rs/arc-swap)
