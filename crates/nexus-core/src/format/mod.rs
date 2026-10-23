//! Layer 4: .nxs binary format container, headers, footer, and block table.

pub mod footer;
pub mod header;
pub mod mini_header;

pub use footer::{BlockTableEntry, Footer, BLOCK_TABLE_ENTRY_SIZE, END_MARKER, FOOTER_SIZE};
pub use header::{FileHeader, CURRENT_FORMAT_VERSION, FILE_HEADER_SIZE, MAGIC_BYTES};
pub use mini_header::{SubBlockHeader, MINI_HEADER_SIZE, SUPERBLOCK_BOUNDARY_MARKER};

use crate::error::{NexusError, Result};

/// Classification of sub-block data type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BlockType {
    Text = 0,
    SourceCode = 1,
    StructuredData = 2,
    Binary = 3,
    Executable = 4,
    FloatingPoint = 5,
    Audio = 6,
    Image = 7,
    Incompressible = 8,
    SuperblockBoundary = 0xFF,
}

impl BlockType {
    /// Converts a byte into a BlockType enum variant.
    pub fn from_u8(value: u8) -> Result<Self> {
        match value {
            0 => Ok(Self::Text),
            1 => Ok(Self::SourceCode),
            2 => Ok(Self::StructuredData),
            3 => Ok(Self::Binary),
            4 => Ok(Self::Executable),
            5 => Ok(Self::FloatingPoint),
            6 => Ok(Self::Audio),
            7 => Ok(Self::Image),
            8 => Ok(Self::Incompressible),
            0xFF => Ok(Self::SuperblockBoundary),
            other => Err(NexusError::UnsupportedBlockType(other)),
        }
    }

    /// Converts BlockType to its byte representation.
    pub fn to_u8(self) -> u8 {
        self as u8
    }
}

/// Compression algorithm path selected for a sub-block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CompressionPath {
    /// Path A: LZ77 + rANS (or Huffman in Phase 1)
    PathA = 0,
    /// Path B: BWT + MTF + RLE + rANS
    PathB = 1,
    /// Path C: Context Mixing + Arithmetic Coding
    PathC = 2,
    /// Path D: Neural Predictor + Arithmetic Coding
    PathD = 3,
    /// Path E: Store (uncompressed fallback)
    PathE = 4,
}

impl CompressionPath {
    /// Converts a byte into a CompressionPath enum variant.
    pub fn from_u8(value: u8) -> Result<Self> {
        match value {
            0 => Ok(Self::PathA),
            1 => Ok(Self::PathB),
            2 => Ok(Self::PathC),
            3 => Ok(Self::PathD),
            4 => Ok(Self::PathE),
            other => Err(NexusError::UnsupportedPath(other)),
        }
    }

    /// Converts CompressionPath to its byte representation.
    pub fn to_u8(self) -> u8 {
        self as u8
    }
}

/// Bitfield of preprocessing transforms applied to a sub-block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PreprocessFlags(u16);

impl PreprocessFlags {
    pub const NONE: Self = Self(0);
    pub const DELTA: Self = Self(1 << 0);
    pub const MTF: Self = Self(1 << 1);
    pub const RLE: Self = Self(1 << 2);
    pub const X86_FILTER: Self = Self(1 << 3);
    pub const FLOAT_SPLIT: Self = Self(1 << 4);
    pub const WRT: Self = Self(1 << 5);
    pub const COL_TRANSPOSE: Self = Self(1 << 6);

    pub fn empty() -> Self {
        Self(0)
    }

    pub fn from_bits_truncate(bits: u16) -> Self {
        Self(bits)
    }

    pub fn bits(&self) -> u16 {
        self.0
    }

    pub fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }
}
