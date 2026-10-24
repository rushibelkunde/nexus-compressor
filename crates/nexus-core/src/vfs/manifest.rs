//! Binary manifest for NEXUS Virtual File System archives.

use super::entry::{VfsEntry, VfsEntryType};
use crate::error::{NexusError, Result};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::io::Read;
use xxhash_rust::xxh32::xxh32;

/// Magic 4-byte header identifying a NEXUS Virtual File System manifest ("NXVF").
pub const VFS_MANIFEST_MAGIC: &[u8; 4] = b"NXVF";
/// Current VFS manifest format version.
pub const VFS_FORMAT_VERSION: u16 = 1;
/// 8-byte end marker separating manifest metadata from file payload streams.
pub const VFS_END_MARKER: u64 = 0xAA55_4E58_5646_4544;

/// Complete catalog of all files and directories packaged in a solid archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VfsManifest {
    pub version: u16,
    pub entries: Vec<VfsEntry>,
}

impl Default for VfsManifest {
    fn default() -> Self {
        Self {
            version: VFS_FORMAT_VERSION,
            entries: Vec::new(),
        }
    }
}

impl VfsManifest {
    pub fn new(entries: Vec<VfsEntry>) -> Self {
        Self {
            version: VFS_FORMAT_VERSION,
            entries,
        }
    }

    /// Total number of regular files in the manifest.
    pub fn file_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.entry_type == VfsEntryType::File)
            .count()
    }

    /// Total number of directories in the manifest.
    pub fn directory_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.entry_type == VfsEntryType::Directory)
            .count()
    }

    /// Sum of uncompressed sizes of all regular files in bytes.
    pub fn total_uncompressed_bytes(&self) -> u64 {
        self.entries.iter().map(|e| e.file_size).sum()
    }

    /// Serializes manifest into binary format.
    pub fn serialize(&self) -> Result<Vec<u8>> {
        let mut buf = Vec::new();
        buf.extend_from_slice(VFS_MANIFEST_MAGIC);
        buf.write_u16::<LittleEndian>(self.version)?;
        buf.write_u32::<LittleEndian>(self.entries.len() as u32)?;

        for entry in &self.entries {
            entry.write_to(&mut buf)?;
        }

        buf.write_u64::<LittleEndian>(VFS_END_MARKER)?;

        let checksum = xxh32(&buf, 0);
        buf.write_u32::<LittleEndian>(checksum)?;

        Ok(buf)
    }

    /// Deserializes manifest from a binary reader.
    pub fn read_from<R: Read>(reader: &mut R) -> Result<Self> {
        let mut magic = [0u8; 4];
        reader.read_exact(&mut magic)?;
        if &magic != VFS_MANIFEST_MAGIC {
            return Err(NexusError::InvalidMagic(magic));
        }

        let version = reader.read_u16::<LittleEndian>()?;
        if version != VFS_FORMAT_VERSION {
            return Err(NexusError::UnsupportedVersion(version));
        }

        let entry_count = reader.read_u32::<LittleEndian>()? as usize;
        let mut entries = Vec::with_capacity(entry_count);

        for _ in 0..entry_count {
            entries.push(VfsEntry::read_from(reader)?);
        }

        let end_marker = reader.read_u64::<LittleEndian>()?;
        if end_marker != VFS_END_MARKER {
            return Err(NexusError::DataCorruption(format!(
                "Invalid VFS end marker: expected 0x{:016x}, got 0x{:016x}",
                VFS_END_MARKER, end_marker
            )));
        }

        let _stored_checksum = reader.read_u32::<LittleEndian>()?;

        Ok(Self { version, entries })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_roundtrip() {
        let entries = vec![
            VfsEntry {
                entry_type: VfsEntryType::Directory,
                path: "src".into(),
                file_size: 0,
                modified_time: 1700000000,
                permissions: 0o755,
                checksum: 0,
            },
            VfsEntry {
                entry_type: VfsEntryType::File,
                path: "src/main.rs".into(),
                file_size: 1024,
                modified_time: 1700000001,
                permissions: 0o644,
                checksum: 0x12345678,
            },
            VfsEntry {
                entry_type: VfsEntryType::File,
                path: "README.md".into(),
                file_size: 512,
                modified_time: 1700000002,
                permissions: 0o644,
                checksum: 0x87654321,
            },
        ];

        let manifest = VfsManifest::new(entries);
        let serialized = manifest.serialize().unwrap();

        let mut cursor = &serialized[..];
        let deserialized = VfsManifest::read_from(&mut cursor).unwrap();

        assert_eq!(manifest, deserialized);
        assert_eq!(deserialized.file_count(), 2);
        assert_eq!(deserialized.directory_count(), 1);
        assert_eq!(deserialized.total_uncompressed_bytes(), 1536);
    }
}
