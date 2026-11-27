//! Hand-offs to the operating system's file manager.

use std::path::Path;

/// Reveals `path` in the system file manager: a file is selected inside its
/// folder, a directory is opened. Failures are ignored — there is nothing
/// useful the GUI could do about a missing file manager.
pub fn open_in_explorer(path: &Path) {
    #[cfg(target_os = "windows")]
    {
        if path.is_file() {
            let _ = std::process::Command::new("explorer")
                .arg(format!("/select,{}", path.display()))
                .spawn();
        } else {
            let _ = std::process::Command::new("explorer").arg(path).spawn();
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let target = if path.is_file() {
            path.parent().unwrap_or(path)
        } else {
            path
        };
        let _ = std::process::Command::new("xdg-open").arg(target).spawn();
    }
}
