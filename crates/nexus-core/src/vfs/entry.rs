//! VFS file and directory entry representations.

use crate::error::{NexusError, Result};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::io::{Read, Write};

/// Entry classification within a NEXUS Virtual File System archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum VfsEntryType {
    /// Regular file with binary or text payload.
    File = 0x01,
    /// Directory hierarchy node.
    Directory = 0x02,
    /// Symbolic link (preserved on Unix).
    Symlink = 0x03,
}

impl VfsEntryType {
    pub fn from_u8(val: u8) -> Result<Self> {
        match val {
            0x01 => Ok(Self::File),
            0x02 => Ok(Self::Directory),
            0x03 => Ok(Self::Symlink),
            other => Err(NexusError::DataCorruption(format!(
                "Invalid VFS entry type tag 0x{:02x}",
                other
            ))),
        }
    }
}

/// Metadata describing a single file, directory, or symlink in the archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VfsEntry {
    /// Type of this filesystem node.
    pub entry_type: VfsEntryType,
    /// Normalized relative path (using forward slashes, e.g. "src/engine/lz77.rs").
    pub path: String,
    /// Uncompressed file payload size in bytes (0 for directories).
    pub file_size: u64,
    /// Last modified timestamp in seconds since Unix epoch.
    pub modified_time: u64,
    /// File mode/permissions (POSIX mode bits or Windows attribute flags).
    pub permissions: u32,
    /// XXHash32 checksum of the raw uncompressed file contents (0 for directories).
    pub checksum: u32,
}

impl VfsEntry {
    /// Serializes entry metadata to a binary writer.
    pub fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        writer.write_u8(self.entry_type as u8)?;
        let path_bytes = self.path.as_bytes();
        if path_bytes.len() > u16::MAX as usize {
            return Err(NexusError::DataCorruption(format!(
                "VFS entry path length {} exceeds maximum 65535 bytes",
                path_bytes.len()
            )));
        }
        writer.write_u16::<LittleEndian>(path_bytes.len() as u16)?;
        writer.write_all(path_bytes)?;
        writer.write_u64::<LittleEndian>(self.file_size)?;
        writer.write_u64::<LittleEndian>(self.modified_time)?;
        writer.write_u32::<LittleEndian>(self.permissions)?;
        writer.write_u32::<LittleEndian>(self.checksum)?;
        Ok(())
    }

    /// Deserializes entry metadata from a binary reader.
    pub fn read_from<R: Read>(reader: &mut R) -> Result<Self> {
        let entry_type = VfsEntryType::from_u8(reader.read_u8()?)?;
        let path_len = reader.read_u16::<LittleEndian>()? as usize;
        let mut path_buf = vec![0u8; path_len];
        reader.read_exact(&mut path_buf)?;
        let path = String::from_utf8(path_buf).map_err(|e| {
            NexusError::DataCorruption(format!("VFS entry path is not valid UTF-8: {}", e))
        })?;
        let file_size = reader.read_u64::<LittleEndian>()?;
        let modified_time = reader.read_u64::<LittleEndian>()?;
        let permissions = reader.read_u32::<LittleEndian>()?;
        let checksum = reader.read_u32::<LittleEndian>()?;

        Ok(Self {
            entry_type,
            path,
            file_size,
            modified_time,
            permissions,
            checksum,
        })
    }
}
