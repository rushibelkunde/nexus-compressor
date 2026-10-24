//! Recursive directory crawler with similarity-based file clustering.

use super::entry::{VfsEntry, VfsEntryType};
use super::manifest::VfsManifest;
use crate::error::Result;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use xxhash_rust::xxh32::xxh32;

/// Recursively traverses a root directory and produces an optimized VFS manifest with
/// files clustered by extension for maximum solid-compression dictionary sharing.
pub fn crawl_directory<P: AsRef<Path>>(root: P) -> Result<(VfsManifest, Vec<PathBuf>)> {
    let root_path = root.as_ref();
    let mut dir_entries = Vec::new();
    let mut file_entries = Vec::new();

    crawl_recursive(root_path, root_path, &mut dir_entries, &mut file_entries)?;

    // 1. Sort directories alphabetically by path
    dir_entries.sort_by(|a, b| a.path.cmp(&b.path));

    // 2. Similarity Sort: Cluster files by extension first, then by relative path
    file_entries.sort_by(|(entry_a, _), (entry_b, _)| {
        let ext_a = Path::new(&entry_a.path)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let ext_b = Path::new(&entry_b.path)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("");

        match ext_a.cmp(ext_b) {
            std::cmp::Ordering::Equal => entry_a.path.cmp(&entry_b.path),
            other => other,
        }
    });

    // Combine directories first, followed by similarity-sorted files
    let mut manifest_entries = Vec::with_capacity(dir_entries.len() + file_entries.len());
    manifest_entries.extend(dir_entries);

    let mut disk_paths = Vec::with_capacity(file_entries.len());
    for (entry, disk_path) in file_entries {
        manifest_entries.push(entry);
        disk_paths.push(disk_path);
    }

    Ok((VfsManifest::new(manifest_entries), disk_paths))
}

fn crawl_recursive(
    base: &Path,
    current: &Path,
    dir_entries: &mut Vec<VfsEntry>,
    file_entries: &mut Vec<(VfsEntry, PathBuf)>,
) -> Result<()> {
    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = entry.metadata()?;

        let relative_path = path
            .strip_prefix(base)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");

        let modified_time = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        #[cfg(unix)]
        let permissions = {
            use std::os::unix::fs::PermissionsExt;
            metadata.permissions().mode()
        };
        #[cfg(not(unix))]
        let permissions = if metadata.permissions().readonly() {
            0o444
        } else {
            0o644
        };

        if metadata.is_dir() {
            dir_entries.push(VfsEntry {
                entry_type: VfsEntryType::Directory,
                path: relative_path,
                file_size: 0,
                modified_time,
                permissions,
                checksum: 0,
            });
            crawl_recursive(base, &path, dir_entries, file_entries)?;
        } else if metadata.is_file() {
            let file_size = metadata.len();
            let checksum = compute_file_checksum(&path)?;

            let vfs_entry = VfsEntry {
                entry_type: VfsEntryType::File,
                path: relative_path,
                file_size,
                modified_time,
                permissions,
                checksum,
            };
            file_entries.push((vfs_entry, path));
        }
    }
    Ok(())
}

/// Computes the XXHash32 checksum of a file without loading the entire file into memory.
fn compute_file_checksum(path: &Path) -> Result<u32> {
    let mut file = File::open(path)?;
    let mut buffer = [0u8; 65536];
    let mut running_checksum = 0u32;

    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        running_checksum = xxh32(&buffer[..count], running_checksum);
    }

    Ok(running_checksum)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{create_dir_all, write};

    #[test]
    fn test_crawl_and_similarity_sorting() {
        let temp_dir = std::env::temp_dir().join("nexus_test_crawl");
        let _ = std::fs::remove_dir_all(&temp_dir);
        create_dir_all(temp_dir.join("subdir")).unwrap();

        write(temp_dir.join("b.txt"), b"text b").unwrap();
        write(temp_dir.join("a.rs"), b"fn main() {}").unwrap();
        write(temp_dir.join("subdir/c.rs"), b"struct Foo;").unwrap();
        write(temp_dir.join("a.txt"), b"text a").unwrap();

        let (manifest, paths) = crawl_directory(&temp_dir).unwrap();

        assert_eq!(manifest.directory_count(), 1);
        assert_eq!(manifest.file_count(), 4);
        assert_eq!(paths.len(), 4);

        // Check similarity ordering: .rs files clustered together, .txt files clustered together
        let file_paths: Vec<&str> = manifest
            .entries
            .iter()
            .filter(|e| e.entry_type == VfsEntryType::File)
            .map(|e| e.path.as_str())
            .collect();

        assert_eq!(
            file_paths,
            vec!["a.rs", "subdir/c.rs", "a.txt", "b.txt"]
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
