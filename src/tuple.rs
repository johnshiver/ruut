use zerocopy::little_endian::{U32, U64};
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout, Unaligned};

pub const TUPLE_HEADER_LEN: usize = core::mem::size_of::<TupleHeader>();

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TupleCodecError {
    #[error(
        "tuple buffer is too small for a tuple header: expected at least {expected} bytes, got {actual}"
    )]
    BufferTooSmall { expected: usize, actual: usize },
}

/// Zero-copy relational tuple header stored ahead of every tuple payload.
///
/// Logical layout:
/// - `tx_id_created: u64`
/// - `tx_id_expired: u64` (`0` means active)
/// - `sys_version: u32`
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, FromBytes, IntoBytes, KnownLayout, Immutable, Unaligned)]
pub struct TupleHeader {
    pub tx_id_created: U64,
    pub tx_id_expired: U64,
    pub sys_version: U32,
}

impl TupleHeader {
    pub const fn new(tx_id_created: u64, tx_id_expired: u64, sys_version: u32) -> Self {
        Self {
            tx_id_created: U64::new(tx_id_created),
            tx_id_expired: U64::new(tx_id_expired),
            sys_version: U32::new(sys_version),
        }
    }

    pub const fn tx_id_created(self) -> u64 {
        self.tx_id_created.get()
    }

    pub const fn tx_id_expired(self) -> u64 {
        self.tx_id_expired.get()
    }

    pub const fn sys_version(self) -> u32 {
        self.sys_version.get()
    }

    pub fn encode(self) -> [u8; TUPLE_HEADER_LEN] {
        let mut bytes = [0u8; TUPLE_HEADER_LEN];
        bytes.copy_from_slice(self.as_bytes());
        bytes
    }

    pub fn decode_prefix(bytes: &[u8]) -> Result<(&Self, &[u8]), TupleCodecError> {
        if bytes.len() < TUPLE_HEADER_LEN {
            return Err(TupleCodecError::BufferTooSmall {
                expected: TUPLE_HEADER_LEN,
                actual: bytes.len(),
            });
        }

        Ok(Self::ref_from_prefix(bytes).expect("TupleHeader is unaligned-safe"))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BorrowedTuple<'a> {
    header: &'a TupleHeader,
    column_bytes: &'a [u8],
}

impl<'a> BorrowedTuple<'a> {
    pub fn decode(bytes: &'a [u8]) -> Result<Self, TupleCodecError> {
        let (header, column_bytes) = TupleHeader::decode_prefix(bytes)?;
        Ok(Self {
            header,
            column_bytes,
        })
    }

    pub fn header(&self) -> &'a TupleHeader {
        self.header
    }

    pub fn column_bytes(&self) -> &'a [u8] {
        self.column_bytes
    }
}

pub fn serialize_tuple(header: TupleHeader, column_bytes: &[u8]) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(TUPLE_HEADER_LEN + column_bytes.len());
    encoded.extend_from_slice(header.as_bytes());
    encoded.extend_from_slice(column_bytes);
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tuple_header_has_expected_wire_size() {
        assert_eq!(TUPLE_HEADER_LEN, 20);
    }

    #[test]
    fn tuple_header_round_trip_is_zero_copy() {
        let encoded = serialize_tuple(TupleHeader::new(11, 0, 7), b"payload");

        let tuple = BorrowedTuple::decode(&encoded).unwrap();

        assert_eq!(tuple.header().tx_id_created(), 11);
        assert_eq!(tuple.header().tx_id_expired(), 0);
        assert_eq!(tuple.header().sys_version(), 7);
        assert_eq!(tuple.column_bytes(), b"payload");
    }
}
