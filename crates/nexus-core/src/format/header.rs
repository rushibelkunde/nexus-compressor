//! .nxs file header serialization and deserialization.

use crate::error::{NexusError, Result};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::io::{Read, Write};

/// Magic bytes for the .nxs container: "NXS\x01".
pub const MAGIC_BYTES: [u8; 4] = [0x4E, 0x58, 0x53, 0x01];

/// The current supported format version.
pub const CURRENT_FORMAT_VERSION: u16 = 1;

/// Fixed size of the file header in bytes.
pub const FILE_HEADER_SIZE: usize = 32;

/// File header for .nxs binary format (32 bytes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileHeader {
    /// Format version (currently 1).
    pub format_version: u16,
    /// Bitfield flags (e.g., bit 0: encryption, bit 1: xxhash64).
    pub flags: u16,
    /// Total uncompressed size in bytes.
    pub original_size: u64,
    /// Number of superblocks in the file.
    pub superblock_count: u32,
    /// Reserved for future expansion (must be 0).
    pub reserved: [u8; 12],
}

impl FileHeader {
    /// Creates a new FileHeader with default flags and reserved zeros.
    pub fn new(original_size: u64, superblock_count: u32) -> Self {
        Self {
            format_version: CURRENT_FORMAT_VERSION,
            flags: 0,
            original_size,
            superblock_count,
            reserved: [0u8; 12],
        }
    }

    /// Serializes the file header into a writer (always exactly 32 bytes, little-endian).
    pub fn write_to<W: Write>(&self, mut writer: W) -> Result<()> {
        writer.write_all(&MAGIC_BYTES)?;
        writer.write_u16::<LittleEndian>(self.format_version)?;
        writer.write_u16::<LittleEndian>(self.flags)?;
        writer.write_u64::<LittleEndian>(self.original_size)?;
        writer.write_u32::<LittleEndian>(self.superblock_count)?;
        writer.write_all(&self.reserved)?;
        Ok(())
    }

    /// Deserializes a file header from a reader and validates magic bytes and format version.
    pub fn read_from<R: Read>(mut reader: R) -> Result<Self> {
        let mut magic = [0u8; 4];
        reader.read_exact(&mut magic)?;
        if magic != MAGIC_BYTES {
            return Err(NexusError::InvalidMagic(magic));
        }

        let format_version = reader.read_u16::<LittleEndian>()?;
        if format_version > CURRENT_FORMAT_VERSION {
            return Err(NexusError::UnsupportedVersion(format_version));
        }

        let flags = reader.read_u16::<LittleEndian>()?;
        let original_size = reader.read_u64::<LittleEndian>()?;
        let superblock_count = reader.read_u32::<LittleEndian>()?;
        let mut reserved = [0u8; 12];
        reader.read_exact(&mut reserved)?;

        Ok(Self {
            format_version,
            flags,
            original_size,
            superblock_count,
            reserved,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_header_roundtrip() {
        let header = FileHeader::new(1048576, 2);
        let mut buffer = Vec::new();
        header.write_to(&mut buffer).unwrap();

        assert_eq!(buffer.len(), FILE_HEADER_SIZE);
        assert_eq!(&buffer[0..4], &MAGIC_BYTES);

        let decoded = FileHeader::read_from(&buffer[..]).unwrap();
        assert_eq!(header, decoded);
    }

    #[test]
    fn test_invalid_magic() {
        let mut buffer = vec![0u8; 32];
        buffer[0..4].copy_from_slice(b"BAD!");
        let err = FileHeader::read_from(&buffer[..]).unwrap_err();
        match err {
            NexusError::InvalidMagic(m) => assert_eq!(m, *b"BAD!"),
            _ => panic!("Expected InvalidMagic error"),
        }
    }

    #[test]
    fn test_unsupported_version() {
        let mut buffer = Vec::new();
        let mut header = FileHeader::new(100, 1);
        header.format_version = 99;
        header.write_to(&mut buffer).unwrap();

        let err = FileHeader::read_from(&buffer[..]).unwrap_err();
        match err {
            NexusError::UnsupportedVersion(v) => assert_eq!(v, 99),
            _ => panic!("Expected UnsupportedVersion error"),
        }
    }
}
