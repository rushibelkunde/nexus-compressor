//! Error types and result definitions for Project NEXUS.

#[derive(Debug, thiserror::Error)]
pub enum NexusError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid magic bytes: expected [0x4E, 0x58, 0x53, 0x01], got {0:?}")]
    InvalidMagic([u8; 4]),

    #[error("Invalid end marker: expected [0x4E, 0x58, 0x53, 0x45], got {0:?}")]
    InvalidEndMarker([u8; 4]),

    #[error("Unsupported format version: {0}")]
    UnsupportedVersion(u16),

    #[error("Checksum mismatch: expected {expected:#x}, got {actual:#x}")]
    ChecksumMismatch { expected: u64, actual: u64 },

    #[error("Block checksum mismatch: expected {expected:#010x}, got {actual:#010x}")]
    BlockChecksumMismatch { expected: u32, actual: u32 },

    #[error("Unsupported compression path: {0}")]
    UnsupportedPath(u8),

    #[error("Unsupported block type: {0}")]
    UnsupportedBlockType(u8),

    #[error("Block decompression failed at offset {offset}: {reason}")]
    DecompressFailed { offset: u64, reason: String },

    #[error("Data corruption: {0}")]
    DataCorruption(String),
}

/// Convenience alias for results within NEXUS.
pub type Result<T> = std::result::Result<T, NexusError>;
