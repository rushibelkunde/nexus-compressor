//! Path classification and default destinations.
//!
//! Decides whether a path handed to the GUI (dropped on the window, passed on
//! the command line or picked in a dialog) should be compressed or extracted,
//! and derives the default output location for either operation.

use std::path::{Path, PathBuf};

/// File extensions (lower-case, without the dot) recognised as NEXUS archives.
pub const ARCHIVE_EXTENSIONS: [&str; 2] = ["nxar", "nxs"];

/// Extension given to newly created archives.
pub const DEFAULT_ARCHIVE_EXTENSION: &str = "nxar";

/// What the GUI should do with a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// Open the compression setup screen.
    Compress,
    /// Open the extraction setup screen.
    Extract,
}

/// Returns `true` when the path has a NEXUS archive extension
/// (case-insensitive).
pub fn has_archive_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            ARCHIVE_EXTENSIONS
                .iter()
                .any(|known| ext.eq_ignore_ascii_case(known))
        })
}

/// Routes a path by its extension: `.nxar` / `.nxs` files are extracted,
/// everything else is compressed. Directories are always compressed, even if
/// their name happens to end in an archive extension.
pub fn route_for(path: &Path, is_dir: bool) -> Route {
    if !is_dir && has_archive_extension(path) {
        Route::Extract
    } else {
        Route::Compress
    }
}

/// Routes a path that exists on disk (queries the filesystem for `is_dir`).
pub fn route_for_existing(path: &Path) -> Route {
    route_for(path, path.is_dir())
}

/// The final component of a path as display text; falls back to the whole
/// path for roots such as `C:\`.
pub fn display_name(path: &Path) -> String {
    match path.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => path.display().to_string(),
    }
}

/// Default archive path for an input: `<input>.nxar` next to the input.
pub fn default_archive_path(input: &Path) -> PathBuf {
    let name = display_name(input);
    input.with_file_name(format!("{name}.{DEFAULT_ARCHIVE_EXTENSION}"))
}

/// Name of the item an archive restores to: the archive's file name with its
/// `.nxar` / `.nxs` extension removed. An archive without a known extension
/// gets an `_extracted` suffix so the output can never be the archive itself.
pub fn restored_name(archive: &Path) -> String {
    let name = display_name(archive);
    if has_archive_extension(archive) {
        if let Some(stem) = archive.file_stem() {
            let stem = stem.to_string_lossy();
            if !stem.is_empty() {
                return stem.into_owned();
            }
        }
    }
    format!("{name}_extracted")
}

/// Default extraction target: [`restored_name`] next to the archive.
pub fn default_extract_path(archive: &Path) -> PathBuf {
    archive.with_file_name(restored_name(archive))
}

/// Extraction target inside a folder the user picked: `<folder>/<restored name>`.
/// Works for both single-file archives (the path becomes the file) and folder
/// archives (the path becomes the directory that is unpacked into).
pub fn extract_path_in(folder: &Path, archive: &Path) -> PathBuf {
    folder.join(restored_name(archive))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_route_archive_extensions_extract() {
        assert_eq!(route_for(Path::new("a/b/data.nxar"), false), Route::Extract);
        assert_eq!(route_for(Path::new("data.nxs"), false), Route::Extract);
    }

    #[test]
    fn test_route_is_case_insensitive() {
        assert_eq!(route_for(Path::new("DATA.NXAR"), false), Route::Extract);
        assert_eq!(route_for(Path::new("data.Nxs"), false), Route::Extract);
    }

    #[test]
    fn test_route_everything_else_compresses() {
        assert_eq!(route_for(Path::new("notes.txt"), false), Route::Compress);
        assert_eq!(route_for(Path::new("archive.zip"), false), Route::Compress);
        assert_eq!(route_for(Path::new("no_extension"), false), Route::Compress);
        assert_eq!(route_for(Path::new("nxar"), false), Route::Compress);
        assert_eq!(route_for(Path::new(".nxar"), false), Route::Compress);
        assert_eq!(
            route_for(Path::new("data.nxar.bak"), false),
            Route::Compress
        );
    }

    #[test]
    fn test_route_directory_always_compresses() {
        assert_eq!(route_for(Path::new("folder.nxar"), true), Route::Compress);
        assert_eq!(route_for(Path::new("folder"), true), Route::Compress);
    }

    #[test]
    fn test_default_archive_path_appends_extension() {
        assert_eq!(
            default_archive_path(Path::new("dir/report.pdf")),
            PathBuf::from("dir/report.pdf.nxar")
        );
        assert_eq!(
            default_archive_path(Path::new("dir/photos")),
            PathBuf::from("dir/photos.nxar")
        );
    }

    #[test]
    fn test_restored_name_strips_known_extension() {
        assert_eq!(restored_name(Path::new("x/report.pdf.nxar")), "report.pdf");
        assert_eq!(restored_name(Path::new("x/photos.NXS")), "photos");
    }

    #[test]
    fn test_restored_name_never_equals_archive_name() {
        assert_eq!(restored_name(Path::new("x/mystery")), "mystery_extracted");
        assert_eq!(restored_name(Path::new("x/data.bin")), "data.bin_extracted");
    }

    #[test]
    fn test_default_extract_path_is_sibling() {
        assert_eq!(
            default_extract_path(Path::new("dir/photos.nxar")),
            PathBuf::from("dir/photos")
        );
    }

    #[test]
    fn test_extract_path_in_folder() {
        assert_eq!(
            extract_path_in(Path::new("out"), Path::new("dir/photos.nxar")),
            PathBuf::from("out").join("photos")
        );
    }

    #[test]
    fn test_display_name_falls_back_to_full_path() {
        assert_eq!(display_name(Path::new("a/b/c.txt")), "c.txt");
        assert!(!display_name(Path::new("/")).is_empty());
    }
}
