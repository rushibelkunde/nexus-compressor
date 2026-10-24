//! .nxs file footer and block table serialization and deserialization.

use crate::error::{NexusError, Result};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::io::{Read, Write};

/// Fixed size of a single block table entry in bytes.
pub const BLOCK_TABLE_ENTRY_SIZE: usize = 20;

/// Fixed size of the file footer in bytes.
pub const FOOTER_SIZE: usize = 24;

/// End marker bytes: "NXSE".
pub const END_MARKER: [u8; 4] = [0x4E, 0x58, 0x53, 0x45];

/// An entry in the footer's random-access block table (20 bytes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockTableEntry {
    /// Superblock index this sub-block belongs to.
    pub superblock_id: u32,
    /// Absolute byte offset of this sub-block's mini-header in the file.
    pub file_offset: u64,
    /// Size of compressed data.
    pub compressed_size: u32,
    /// Size of uncompressed data.
    pub original_size: u32,
}

impl BlockTableEntry {
    /// Serializes the block table entry to a writer (20 bytes, little-endian).
    pub fn write_to<W: Write>(&self, mut writer: W) -> Result<()> {
        writer.write_u32::<LittleEndian>(self.superblock_id)?;
        writer.write_u64::<LittleEndian>(self.file_offset)?;
        writer.write_u32::<LittleEndian>(self.compressed_size)?;
        writer.write_u32::<LittleEndian>(self.original_size)?;
        Ok(())
    }

    /// Deserializes a block table entry from a reader.
    pub fn read_from<R: Read>(mut reader: R) -> Result<Self> {
        let superblock_id = reader.read_u32::<LittleEndian>()?;
        let file_offset = reader.read_u64::<LittleEndian>()?;
        let compressed_size = reader.read_u32::<LittleEndian>()?;
        let original_size = reader.read_u32::<LittleEndian>()?;

        Ok(Self {
            superblock_id,
            file_offset,
            compressed_size,
            original_size,
        })
    }
}

/// The file footer at the very end of a .nxs file (24 bytes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Footer {
    /// Absolute byte offset of the block table from start of file.
    pub block_table_offset: u64,
    /// Total count of sub-blocks in the file.
    pub total_block_count: u32,
    /// XXHash64 of all bytes from offset 0 to start of footer.
    pub file_checksum: u64,
}

impl Footer {
    /// Creates a new Footer struct.
    pub fn new(block_table_offset: u64, total_block_count: u32, file_checksum: u64) -> Self {
        Self {
            block_table_offset,
            total_block_count,
            file_checksum,
        }
    }

    /// Serializes the 24-byte footer to a writer.
    pub fn write_to<W: Write>(&self, mut writer: W) -> Result<()> {
        writer.write_u64::<LittleEndian>(self.block_table_offset)?;
        writer.write_u32::<LittleEndian>(self.total_block_count)?;
        writer.write_u64::<LittleEndian>(self.file_checksum)?;
        writer.write_all(&END_MARKER)?;
        Ok(())
    }

    /// Deserializes the 24-byte footer from a reader, validating the end marker.
    pub fn read_from<R: Read>(mut reader: R) -> Result<Self> {
        let block_table_offset = reader.read_u64::<LittleEndian>()?;
        let total_block_count = reader.read_u32::<LittleEndian>()?;
        let file_checksum = reader.read_u64::<LittleEndian>()?;
        let mut marker = [0u8; 4];
        reader.read_exact(&mut marker)?;

        if marker != END_MARKER {
            return Err(NexusError::InvalidEndMarker(marker));
        }

        Ok(Self {
            block_table_offset,
            total_block_count,
            file_checksum,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_block_table_entry_roundtrip() {
        let entry = BlockTableEntry {
            superblock_id: 1,
            file_offset: 1024,
            compressed_size: 512,
            original_size: 4096,
        };
        let mut buffer = Vec::new();
        entry.write_to(&mut buffer).unwrap();
        assert_eq!(buffer.len(), BLOCK_TABLE_ENTRY_SIZE);

        let decoded = BlockTableEntry::read_from(&buffer[..]).unwrap();
        assert_eq!(entry, decoded);
    }

    #[test]
    fn test_footer_roundtrip() {
        let footer = Footer::new(944, 1, 0x0123456789ABCDEF);
        let mut buffer = Vec::new();
        footer.write_to(&mut buffer).unwrap();
        assert_eq!(buffer.len(), FOOTER_SIZE);

        let decoded = Footer::read_from(&buffer[..]).unwrap();
        assert_eq!(footer, decoded);
    }

    #[test]
    fn test_invalid_end_marker() {
        let mut buffer = [0u8; 24];
        buffer[20..24].copy_from_slice(b"FAIL");
        let err = Footer::read_from(&buffer[..]).unwrap_err();
        match err {
            NexusError::InvalidEndMarker(m) => assert_eq!(m, *b"FAIL"),
            _ => panic!("Expected InvalidEndMarker"),
        }
    }
}
