//! # pager
//!
//! Storage layer for the ruut database engine.
//!
//! The `pager` crate is responsible for:
//! - Memory-mapped file I/O via `memmap2`.
//! - Fixed-size (4 KB) page allocation and deallocation.
//! - Reading and writing the Meta Page (Page 0).
//! - Maintaining the Free List for page reuse.
//!
//! This crate corresponds to **Phase 1** of the ruut development roadmap.
//! See `docs/design.md` for the full architectural specification.

/// Page size in bytes. Aligned to the OS virtual-memory page size.
pub const PAGE_SIZE: usize = 4096;

/// Magic number stored in the Meta Page to identify a valid ruut database file.
pub const MAGIC: u32 = 0xCAFE_BABE;

/// Placeholder type for a page identifier.
pub type PageId = u64;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_size_is_4kb() {
        assert_eq!(PAGE_SIZE, 4096);
    }

    #[test]
    fn magic_number_matches_spec() {
        assert_eq!(MAGIC, 0xCAFE_BABE);
    }
}
