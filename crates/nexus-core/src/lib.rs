//! NEXUS (Neural EXtreme Unified compresSion) core compression library.
//!
//! Provides multi-strategy compression combining LZ77, BWT, Context Mixing,
//! and Neural Prediction with rANS and arithmetic coding.

pub mod block;
pub mod compress;
pub mod entropy;
pub mod error;
pub mod format;
pub mod neural;
pub mod pipeline;
pub mod preprocess;
pub mod vfs;

pub use error::{NexusError, Result};
pub use pipeline::{
    compress, compress_file, compress_file_with_progress, decompress, decompress_file,
    decompress_file_with_progress, estimate_compression, sample_path, CompressionEstimate,
    CompressionPreset, ProgressCallback, ProgressUpdate,
};
