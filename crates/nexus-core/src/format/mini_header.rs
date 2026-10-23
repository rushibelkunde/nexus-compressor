//! Sub-block mini-header serialization and deserialization.

use crate::error::Result;
use crate::format::{BlockType, CompressionPath, PreprocessFlags};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::io::{Read, Write};

/// Fixed size of a sub-block mini-header in bytes.
pub const MINI_HEADER_SIZE: usize = 16;

/// Special block type marker indicating end of a superblock.
pub const SUPERBLOCK_BOUNDARY_MARKER: u8 = 0xFF;

/// 16-byte mini-header preceding each sub-block in the stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubBlockHeader {
    /// Compressed size in bytes following this header.
    pub compressed_size: u32,
    /// Uncompressed size of this sub-block.
    pub original_size: u32,
    /// Content type classification.
    pub block_type: BlockType,
    /// Compression path used.
    pub compression_path: CompressionPath,
    /// Bijective preprocessing transforms applied.
    pub preprocess_flags: PreprocessFlags,
    /// XXHash32 checksum of uncompressed original data.
    pub block_checksum: u32,
}

impl SubBlockHeader {
    /// Creates a new sub-block mini-header.
    pub fn new(
        compressed_size: u32,
        original_size: u32,
        block_type: BlockType,
        compression_path: CompressionPath,
        preprocess_flags: PreprocessFlags,
        block_checksum: u32,
    ) -> Self {
        Self {
            compressed_size,
            original_size,
            block_type,
            compression_path,
            preprocess_flags,
            block_checksum,
        }
    }

    /// Serializes the mini-header to a writer (exactly 16 bytes, little-endian).
    pub fn write_to<W: Write>(&self, mut writer: W) -> Result<()> {
        writer.write_u32::<LittleEndian>(self.compressed_size)?;
        writer.write_u32::<LittleEndian>(self.original_size)?;
        writer.write_u8(self.block_type.to_u8())?;
        writer.write_u8(self.compression_path.to_u8())?;
        writer.write_u16::<LittleEndian>(self.preprocess_flags.bits())?;
        writer.write_u32::<LittleEndian>(self.block_checksum)?;
        Ok(())
    }

    /// Deserializes a mini-header from a reader.
    pub fn read_from<R: Read>(mut reader: R) -> Result<Self> {
        let compressed_size = reader.read_u32::<LittleEndian>()?;
        let original_size = reader.read_u32::<LittleEndian>()?;
        let block_type_byte = reader.read_u8()?;
        let block_type = BlockType::from_u8(block_type_byte)?;
        let path_byte = reader.read_u8()?;
        let compression_path = CompressionPath::from_u8(path_byte)?;
        let preprocess_flags =
            PreprocessFlags::from_bits_truncate(reader.read_u16::<LittleEndian>()?);
        let block_checksum = reader.read_u32::<LittleEndian>()?;

        Ok(Self {
            compressed_size,
            original_size,
            block_type,
            compression_path,
            preprocess_flags,
            block_checksum,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mini_header_roundtrip() {
        let header = SubBlockHeader::new(
            896,
            4096,
            BlockType::Text,
            CompressionPath::PathA,
            PreprocessFlags::MTF,
            0x12EFCDAB,
        );
        let mut buffer = Vec::new();
        header.write_to(&mut buffer).unwrap();

        assert_eq!(buffer.len(), MINI_HEADER_SIZE);

        let decoded = SubBlockHeader::read_from(&buffer[..]).unwrap();
        assert_eq!(header, decoded);
    }
}
