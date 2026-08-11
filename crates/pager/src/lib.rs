//! # pager
//!
//! Storage layer for the ruut database engine.
//!
//! The `pager` crate is responsible for:
//! - Memory-mapped file I/O via `memmap2`.
//! - Fixed-size (4 KB) page allocation and deallocation.
//! - Reading and writing the Meta Page (Page 0).
//! - Maintaining the Free List for page reuse.

use std::fs::File;
use std::path::Path;
use memmap2::MmapMut;

/// Page size in bytes. Aligned to the OS virtual-memory page size.
pub const PAGE_SIZE: usize = 4096;

/// Magic number stored in the Meta Page to identify a valid ruut database file.
pub const MAGIC: u32 = 0xCAFE_BABE;

/// Page identifier.
pub type PageId = u64;

/// Number of pages to reserve ahead when growing the file.
const GROWTH_EXTENT_PAGES: u32 = 8;

/// Errors that can occur in Pager operations.
#[derive(Debug)]
pub enum PagerError {
    /// Standard I/O errors
    Io(std::io::Error),
    /// File starts with an invalid magic number
    InvalidMagic(u32),
    /// Requested PageId is out of bounds
    InvalidPageId(PageId),
    /// Attempted to free the Meta Page (Page 0)
    CannotFreeMetaPage,
    /// Attempted to call get_page_mut on Page 0
    CannotMutateMetaPage,
    /// Attempted to free a page that is already free
    PageAlreadyFree(PageId),
    /// Opened file is too small to contain a valid Meta Page
    FileTooSmall(u64),
    /// The total page count has reached the maximum representable value
    PageCountOverflow,
}

impl std::fmt::Display for PagerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PagerError::Io(e) => write!(f, "I/O error: {}", e),
            PagerError::InvalidMagic(m) => write!(f, "Invalid magic number: {:#010X}", m),
            PagerError::InvalidPageId(id) => write!(f, "Invalid page ID: {}", id),
            PagerError::CannotFreeMetaPage => write!(f, "Cannot free the Meta Page (Page 0)"),
            PagerError::CannotMutateMetaPage => write!(
                f,
                "Cannot directly mutate the Meta Page (Page 0) via get_page_mut; use update_meta instead"
            ),
            PagerError::PageAlreadyFree(id) => write!(f, "Page {} is already free", id),
            PagerError::FileTooSmall(size) => {
                write!(f, "File is too small to be a valid database: {} bytes", size)
            }
            PagerError::PageCountOverflow => {
                write!(f, "Page count has reached the maximum representable value")
            }
        }
    }
}

impl std::error::Error for PagerError {}

impl From<std::io::Error> for PagerError {
    fn from(err: std::io::Error) -> Self {
        PagerError::Io(err)
    }
}

/// The metadata master block stored at Page 0 of the database file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetaPage {
    pub magic: u32,
    pub schema_version: u32,
    pub tx_id: u64,
    pub root_page_id: PageId,
    pub free_list_root_page_id: PageId,
    pub total_pages: u32,
}

impl MetaPage {
    /// Deserializes a MetaPage struct from a 4 KB raw buffer.
    pub fn deserialize(buf: &[u8; PAGE_SIZE]) -> Result<Self, PagerError> {
        let magic = u32::from_le_bytes(buf[0..4].try_into().unwrap());
        if magic != MAGIC {
            return Err(PagerError::InvalidMagic(magic));
        }
        let schema_version = u32::from_le_bytes(buf[4..8].try_into().unwrap());
        let tx_id = u64::from_le_bytes(buf[8..16].try_into().unwrap());
        let root_page_id = u64::from_le_bytes(buf[16..24].try_into().unwrap());
        let free_list_root_page_id = u64::from_le_bytes(buf[24..32].try_into().unwrap());
        let total_pages = u32::from_le_bytes(buf[32..36].try_into().unwrap());

        Ok(MetaPage {
            magic,
            schema_version,
            tx_id,
            root_page_id,
            free_list_root_page_id,
            total_pages,
        })
    }

    /// Serializes a MetaPage struct into a 4 KB raw buffer.
    pub fn serialize(&self, buf: &mut [u8; PAGE_SIZE]) {
        buf[0..4].copy_from_slice(&self.magic.to_le_bytes());
        buf[4..8].copy_from_slice(&self.schema_version.to_le_bytes());
        buf[8..16].copy_from_slice(&self.tx_id.to_le_bytes());
        buf[16..24].copy_from_slice(&self.root_page_id.to_le_bytes());
        buf[24..32].copy_from_slice(&self.free_list_root_page_id.to_le_bytes());
        buf[32..36].copy_from_slice(&self.total_pages.to_le_bytes());
        buf[36..PAGE_SIZE].fill(0); // Clear reserved space
    }
}

/// Manages database file layout, page-level I/O, and memory mapping.
pub struct Pager {
    file: File,
    mmap: MmapMut,
    meta: MetaPage,
    free_list: Vec<PageId>, // Phase 1 in-memory Free List scaffolding
    /// Number of pages currently covered by the memory map (may exceed `meta.total_pages`
    /// because the file is grown in extents to amortise remap overhead).
    mapped_pages: u32,
}

impl Pager {
    /// Opens an existing database file or creates and initializes a new one.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, PagerError> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)?;

        let len = file.metadata()?.len();

        if len == 0 {
            // New database: initialize with Page 0
            file.set_len(PAGE_SIZE as u64)?;
            let mut mmap = unsafe { MmapMut::map_mut(&file)? };

            let meta = MetaPage {
                magic: MAGIC,
                schema_version: 1,
                tx_id: 0,
                root_page_id: 0,
                free_list_root_page_id: 0,
                total_pages: 1,
            };

            let mut buf = [0u8; PAGE_SIZE];
            meta.serialize(&mut buf);
            mmap[0..PAGE_SIZE].copy_from_slice(&buf);
            mmap.flush()?;
            file.sync_all()?;

            Ok(Pager {
                file,
                mmap,
                meta,
                free_list: Vec::new(),
                mapped_pages: 1,
            })
        } else {
            if len < PAGE_SIZE as u64 {
                return Err(PagerError::FileTooSmall(len));
            }

            let mmap = unsafe { MmapMut::map_mut(&file)? };

            let mut buf = [0u8; PAGE_SIZE];
            buf.copy_from_slice(&mmap[0..PAGE_SIZE]);
            let meta = MetaPage::deserialize(&buf)?;

            let expected_size = (meta.total_pages as u64) * (PAGE_SIZE as u64);
            if len < expected_size {
                return Err(PagerError::FileTooSmall(len));
            }

            let mapped_pages = meta.total_pages;
            Ok(Pager {
                file,
                mmap,
                meta,
                free_list: Vec::new(),
                mapped_pages,
            })
        }
    }

    /// Retrieves an immutable copy of the Meta Page.
    pub fn get_meta(&self) -> MetaPage {
        self.meta
    }

    /// Overwrites the Meta Page on the memory map.
    /// Callers must invoke `flush()` and/or `sync()` after this method
    /// to propagate changes to the OS page cache and to durable storage.
    pub fn update_meta(&mut self, meta: MetaPage) -> Result<(), PagerError> {
        if meta.magic != MAGIC {
            return Err(PagerError::InvalidMagic(meta.magic));
        }
        self.meta = meta;
        self.write_meta_to_mmap()?;
        Ok(())
    }

    /// Allocates a new page ID, reclaiming from the Free List if available,
    /// or extending the file size and memory map sequentially.
    /// The file is grown in extents of `GROWTH_EXTENT_PAGES` pages to reduce
    /// remap overhead as the database grows.
    pub fn allocate_page(&mut self) -> Result<PageId, PagerError> {
        if let Some(page_id) = self.free_list.pop() {
            Ok(page_id)
        } else {
            let new_page_id = self.meta.total_pages as PageId;
            self.meta.total_pages = self
                .meta
                .total_pages
                .checked_add(1)
                .ok_or(PagerError::PageCountOverflow)?;

            // Only extend the file (and remap) when we exceed the already-mapped region.
            if self.meta.total_pages > self.mapped_pages {
                let new_mapped = self
                    .mapped_pages
                    .checked_add(GROWTH_EXTENT_PAGES)
                    .ok_or(PagerError::PageCountOverflow)?;
                let new_len = (new_mapped as u64) * (PAGE_SIZE as u64);
                self.file.set_len(new_len)?;
                self.mmap = unsafe { MmapMut::map_mut(&self.file)? };
                self.mapped_pages = new_mapped;
            }

            // Persist the updated count in Page 0 immediately
            self.write_meta_to_mmap()?;

            Ok(new_page_id)
        }
    }

    /// Frees a Page ID to the in-memory scaffold Free List for reuse.
    pub fn free_page(&mut self, page_id: PageId) -> Result<(), PagerError> {
        if page_id == 0 {
            return Err(PagerError::CannotFreeMetaPage);
        }
        if page_id >= self.meta.total_pages as PageId {
            return Err(PagerError::InvalidPageId(page_id));
        }
        if self.free_list.contains(&page_id) {
            return Err(PagerError::PageAlreadyFree(page_id));
        }
        self.free_list.push(page_id);
        Ok(())
    }

    /// Returns a reference to the 4 KB page data for reading.
    pub fn get_page(&self, page_id: PageId) -> Result<&[u8; PAGE_SIZE], PagerError> {
        if page_id >= self.meta.total_pages as PageId {
            return Err(PagerError::InvalidPageId(page_id));
        }
        let start = (page_id as usize)
            .checked_mul(PAGE_SIZE)
            .ok_or(PagerError::InvalidPageId(page_id))?;
        let end = start
            .checked_add(PAGE_SIZE)
            .ok_or(PagerError::InvalidPageId(page_id))?;
        let slice = self
            .mmap
            .get(start..end)
            .ok_or(PagerError::InvalidPageId(page_id))?;
        Ok(slice.try_into().unwrap())
    }

    /// Returns a mutable reference to the 4 KB page data for writing.
    /// Accessing Page 0 is prevented; use `update_meta` instead.
    pub fn get_page_mut(&mut self, page_id: PageId) -> Result<&mut [u8; PAGE_SIZE], PagerError> {
        if page_id == 0 {
            return Err(PagerError::CannotMutateMetaPage);
        }
        if page_id >= self.meta.total_pages as PageId {
            return Err(PagerError::InvalidPageId(page_id));
        }
        let start = (page_id as usize)
            .checked_mul(PAGE_SIZE)
            .ok_or(PagerError::InvalidPageId(page_id))?;
        let end = start
            .checked_add(PAGE_SIZE)
            .ok_or(PagerError::InvalidPageId(page_id))?;
        let slice = self
            .mmap
            .get_mut(start..end)
            .ok_or(PagerError::InvalidPageId(page_id))?;
        Ok(slice.try_into().unwrap())
    }

    /// Flush the memory map changes to the OS page cache.
    pub fn flush(&self) -> Result<(), PagerError> {
        self.mmap.flush()?;
        Ok(())
    }

    /// Fsync the underlying file to guarantee physical durability on disk.
    pub fn sync(&self) -> Result<(), PagerError> {
        self.file.sync_all()?;
        Ok(())
    }

    /// Helper to write the current meta struct directly to Page 0 of the mmap.
    fn write_meta_to_mmap(&mut self) -> Result<(), PagerError> {
        let mut buf = [0u8; PAGE_SIZE];
        self.meta.serialize(&mut buf);
        self.mmap[0..PAGE_SIZE].copy_from_slice(&buf);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn page_size_is_4kb() {
        assert_eq!(PAGE_SIZE, 4096);
    }

    #[test]
    fn magic_number_matches_spec() {
        assert_eq!(MAGIC, 0xCAFE_BABE);
    }

    #[test]
    fn test_meta_page_serialization_roundtrip() {
        let original = MetaPage {
            magic: MAGIC,
            schema_version: 42,
            tx_id: 101,
            root_page_id: 7,
            free_list_root_page_id: 9,
            total_pages: 15,
        };

        let mut buf = [0u8; PAGE_SIZE];
        original.serialize(&mut buf);

        let deserialized = MetaPage::deserialize(&buf).unwrap();
        assert_eq!(original, deserialized);
    }

    #[test]
    fn test_meta_page_invalid_magic() {
        let mut buf = [0u8; PAGE_SIZE];
        buf[0..4].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());

        let result = MetaPage::deserialize(&buf);
        assert!(result.is_err());
        match result.unwrap_err() {
            PagerError::InvalidMagic(m) => assert_eq!(m, 0xDEAD_BEEF),
            other => panic!("expected InvalidMagic error, got {:?}", other),
        }
    }

    #[test]
    fn test_pager_new_initialization() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        let pager = Pager::open(&db_path).unwrap();
        let meta = pager.get_meta();

        assert_eq!(meta.magic, MAGIC);
        assert_eq!(meta.schema_version, 1);
        assert_eq!(meta.tx_id, 0);
        assert_eq!(meta.root_page_id, 0);
        assert_eq!(meta.free_list_root_page_id, 0);
        assert_eq!(meta.total_pages, 1);

        // Check file length is 4096
        assert_eq!(db_path.metadata().unwrap().len(), 4096);

        // Try reading Page 0
        let page0 = pager.get_page(0).unwrap();
        assert_eq!(u32::from_le_bytes(page0[0..4].try_into().unwrap()), MAGIC);
    }

    #[test]
    fn test_pager_page_allocations_and_growth() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        let mut pager = Pager::open(&db_path).unwrap();

        // Allocate a page
        let p1 = pager.allocate_page().unwrap();
        assert_eq!(p1, 1);
        assert_eq!(pager.get_meta().total_pages, 2);
        // File is grown in extents; the on-disk size will be at least 2 pages.
        assert!(db_path.metadata().unwrap().len() >= 2 * PAGE_SIZE as u64);

        // Write some data to p1
        {
            let p1_mut = pager.get_page_mut(p1).unwrap();
            p1_mut[0..11].copy_from_slice(b"hello world");
        }

        // Flush and close
        pager.flush().unwrap();
        pager.sync().unwrap();
        drop(pager);

        // Reopen pager and verify the content of p1
        let reopen_pager = Pager::open(&db_path).unwrap();
        assert_eq!(reopen_pager.get_meta().total_pages, 2);

        let p1_read = reopen_pager.get_page(1).unwrap();
        assert_eq!(&p1_read[0..11], b"hello world");
    }

    #[test]
    fn test_pager_safety_guards() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        let mut pager = Pager::open(&db_path).unwrap();

        // Attempting to mutate Page 0 directly should fail
        let mut_result = pager.get_page_mut(0);
        assert!(mut_result.is_err());
        match mut_result.unwrap_err() {
            PagerError::CannotMutateMetaPage => {}
            other => panic!("expected CannotMutateMetaPage, got {:?}", other),
        }

        // Attempting to get out-of-bounds page should fail
        let out_of_bounds = pager.get_page(1);
        assert!(out_of_bounds.is_err());
        match out_of_bounds.unwrap_err() {
            PagerError::InvalidPageId(1) => {}
            other => panic!("expected InvalidPageId, got {:?}", other),
        }
    }

    #[test]
    fn test_pager_free_list_scaffolding() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        let mut pager = Pager::open(&db_path).unwrap();

        // Allocate two pages
        let p1 = pager.allocate_page().unwrap();
        let p2 = pager.allocate_page().unwrap();
        assert_eq!(p1, 1);
        assert_eq!(p2, 2);
        assert_eq!(pager.get_meta().total_pages, 3);

        // Free p1
        pager.free_page(p1).unwrap();

        // Double free should fail
        let double_free = pager.free_page(p1);
        assert!(double_free.is_err());
        match double_free.unwrap_err() {
            PagerError::PageAlreadyFree(1) => {}
            other => panic!("expected PageAlreadyFree, got {:?}", other),
        }

        // Allocate a page again; it should reuse p1
        let reused = pager.allocate_page().unwrap();
        assert_eq!(reused, 1);
        // Total pages should not have increased
        assert_eq!(pager.get_meta().total_pages, 3);

        // Next allocation should create a new page
        let p3 = pager.allocate_page().unwrap();
        assert_eq!(p3, 3);
        assert_eq!(pager.get_meta().total_pages, 4);
    }
}
