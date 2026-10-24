//! NEXUS Virtual File System (NEXUS-VFS) for solid directory archiving.
//!
//! Replaces legacy TAR with modern type-grouped similarity sorting and
//! compact binary manifest encoding.

pub mod crawler;
pub mod entry;
pub mod manifest;
pub mod pack;
pub mod unpack;

pub use crawler::crawl_directory;
pub use entry::{VfsEntry, VfsEntryType};
pub use manifest::VfsManifest;
pub use pack::VfsDirectoryReader;
pub use unpack::{is_vfs_stream, unpack_vfs_stream};
