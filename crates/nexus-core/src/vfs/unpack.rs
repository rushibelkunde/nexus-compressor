//! VFS directory unpacker and extractor.

use super::entry::VfsEntryType;
use super::manifest::{VfsManifest, VFS_MANIFEST_MAGIC};
use crate::error::{NexusError, Result};
use std::fs::{create_dir_all, File};
use std::io::{BufWriter, Read, Write};
use std::path::Path;
use xxhash_rust::xxh32::xxh32;

/// Checks if an uncompressed stream begins with the NEXUS-VFS manifest magic header.
pub fn is_vfs_stream(header_bytes: &[u8]) -> bool {
    header_bytes.len() >= 4 && &header_bytes[..4] == VFS_MANIFEST_MAGIC
}

/// Unpacks a solid VFS stream directly into a target destination directory.
///
/// Reconstructs the exact directory hierarchy, writes each file, and validates
/// each file's individual XXHash32 checksum against corruption.
pub fn unpack_vfs_stream<R: Read, P: AsRef<Path>>(
    reader: &mut R,
    dest_dir: P,
) -> Result<VfsManifest> {
    let dest_path = dest_dir.as_ref();
    create_dir_all(dest_path)?;

    // 1. Parse manifest from the front of the stream
    let manifest = VfsManifest::read_from(reader)?;

    // 2. Pre-create all directory nodes
    for entry in &manifest.entries {
        if entry.entry_type == VfsEntryType::Directory {
            let full_dir_path = dest_path.join(&entry.path);
            create_dir_all(full_dir_path)?;
        }
    }

    // 3. Extract all file payloads sequentially
    let mut buffer = [0u8; 65536];

    for entry in &manifest.entries {
        if entry.entry_type == VfsEntryType::File {
            let full_file_path = dest_path.join(&entry.path);
            if let Some(parent) = full_file_path.parent() {
                create_dir_all(parent)?;
            }

            let mut out_file = BufWriter::new(File::create(&full_file_path)?);
            let mut bytes_remaining = entry.file_size;
            let mut running_checksum = 0u32;

            while bytes_remaining > 0 {
                let to_read = std::cmp::min(bytes_remaining, buffer.len() as u64) as usize;
                reader.read_exact(&mut buffer[..to_read])?;
                out_file.write_all(&buffer[..to_read])?;
                running_checksum = xxh32(&buffer[..to_read], running_checksum);
                bytes_remaining -= to_read as u64;
            }

            out_file.flush()?;

            // Verify file checksum
            if running_checksum != entry.checksum {
                return Err(NexusError::BlockChecksumMismatch {
                    expected: entry.checksum,
                    actual: running_checksum,
                });
            }
        }
    }

    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::pack::VfsDirectoryReader;
    use std::fs::{read_to_string, write};

    #[test]
    fn test_vfs_pack_unpack_roundtrip() {
        let temp_src = std::env::temp_dir().join("nexus_test_pack_src");
        let temp_dst = std::env::temp_dir().join("nexus_test_pack_dst");

        let _ = std::fs::remove_dir_all(&temp_src);
        let _ = std::fs::remove_dir_all(&temp_dst);

        create_dir_all(temp_src.join("nested/folder")).unwrap();
        write(temp_src.join("root.txt"), b"Root file content").unwrap();
        write(
            temp_src.join("nested/folder/code.rs"),
            b"pub fn add(a: i32, b: i32) -> i32 { a + b }",
        )
        .unwrap();

        // 1. Pack
        let mut reader = VfsDirectoryReader::open(&temp_src).unwrap();

        // 2. Unpack
        let manifest = unpack_vfs_stream(&mut reader, &temp_dst).unwrap();

        assert_eq!(manifest.file_count(), 2);
        assert_eq!(manifest.directory_count(), 2);

        // 3. Verify content on disk
        let root_content = read_to_string(temp_dst.join("root.txt")).unwrap();
        assert_eq!(root_content, "Root file content");

        let code_content = read_to_string(temp_dst.join("nested/folder/code.rs")).unwrap();
        assert_eq!(code_content, "pub fn add(a: i32, b: i32) -> i32 { a + b }");

        let _ = std::fs::remove_dir_all(&temp_src);
        let _ = std::fs::remove_dir_all(&temp_dst);
    }
}
