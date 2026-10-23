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

pub use error::{NexusError, Result};
