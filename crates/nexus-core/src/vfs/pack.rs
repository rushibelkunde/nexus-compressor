//! VFS directory pack streamer providing bounded-memory solid streaming.

use super::crawler::crawl_directory;
use super::manifest::VfsManifest;
use crate::error::Result;
use std::fs::File;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

/// A streaming reader that yields the VFS manifest followed immediately by the
/// raw bytes of all files in the directory in similarity-sorted order.
pub struct VfsDirectoryReader {
    pub manifest: VfsManifest,
    manifest_cursor: Cursor<Vec<u8>>,
    file_paths: Vec<PathBuf>,
    current_file_idx: usize,
    current_file: Option<File>,
    total_uncompressed_stream_size: u64,
}

impl VfsDirectoryReader {
    /// Opens a directory for solid VFS streaming.
    pub fn open<P: AsRef<Path>>(dir_path: P) -> Result<Self> {
        let (manifest, file_paths) = crawl_directory(dir_path)?;
        let serialized_manifest = manifest.serialize()?;
        let total_uncompressed_stream_size =
            serialized_manifest.len() as u64 + manifest.total_uncompressed_bytes();

        Ok(Self {
            manifest,
            manifest_cursor: Cursor::new(serialized_manifest),
            file_paths,
            current_file_idx: 0,
            current_file: None,
            total_uncompressed_stream_size,
        })
    }

    /// Total number of uncompressed bytes produced by this reader (manifest + files).
    pub fn total_stream_size(&self) -> u64 {
        self.total_uncompressed_stream_size
    }
}

impl Read for VfsDirectoryReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        // 1. Drain manifest first
        if self.manifest_cursor.position() < self.manifest_cursor.get_ref().len() as u64 {
            let n = self.manifest_cursor.read(buf)?;
            if n > 0 {
                return Ok(n);
            }
        }

        // 2. Stream file payloads sequentially
        loop {
            if self.current_file.is_none() {
                if self.current_file_idx >= self.file_paths.len() {
                    return Ok(0); // EOF
                }
                let path = &self.file_paths[self.current_file_idx];
                self.current_file = Some(File::open(path)?);
            }

            if let Some(ref mut file) = self.current_file {
                let n = file.read(buf)?;
                if n > 0 {
                    return Ok(n);
                } else {
                    // Current file reached EOF; advance to next file
                    self.current_file = None;
                    self.current_file_idx += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{create_dir_all, write};

    #[test]
    fn test_vfs_directory_reader_streaming() {
        let temp_dir = std::env::temp_dir().join("nexus_test_vfs_reader");
        let _ = std::fs::remove_dir_all(&temp_dir);
        create_dir_all(&temp_dir).unwrap();

        write(temp_dir.join("f1.txt"), b"Hello World").unwrap();
        write(temp_dir.join("f2.txt"), b"1234567890").unwrap();

        let mut reader = VfsDirectoryReader::open(&temp_dir).unwrap();
        let total_size = reader.total_stream_size();

        let mut stream_bytes = Vec::new();
        reader.read_to_end(&mut stream_bytes).unwrap();

        assert_eq!(stream_bytes.len() as u64, total_size);

        // Verify that stream starts with VFS manifest magic
        assert_eq!(&stream_bytes[..4], b"NXVF");

        // Verify that file contents are present in the stream
        assert!(stream_bytes.windows(11).any(|w| w == b"Hello World"));
        assert!(stream_bytes.windows(10).any(|w| w == b"1234567890"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
