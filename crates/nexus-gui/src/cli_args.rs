//! Command-line entry points used by the OS shell integration.
//!
//! The Windows context menu, the Linux desktop entry and file associations all
//! launch the GUI with arguments. Supported forms:
//!
//! * `nexus-gui compress <path> [--preset low|medium|high] [--auto-start]`
//! * `nexus-gui decompress <archive> [--auto-start | --extract-here]`
//! * `nexus-gui <path>` — a `.nxar` / `.nxs` file opens the extraction setup
//!   (this is what a double-click on an archive does); anything else opens
//!   the compression setup.

use std::path::PathBuf;

use nexus_core::pipeline::CompressionPreset;

use crate::paths::has_archive_extension;

/// What the GUI should do immediately after start-up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitialAction {
    /// Open (or directly start) compression of `path`.
    Compress {
        /// File or folder to compress.
        path: PathBuf,
        /// Preset chosen in the context menu, if any.
        preset: Option<CompressionPreset>,
        /// Start the job without waiting on the setup screen.
        auto_start: bool,
    },
    /// Open (or directly start) extraction of `path`.
    Decompress {
        /// Archive to extract.
        path: PathBuf,
        /// Start the job without waiting on the setup screen.
        auto_start: bool,
    },
}

/// Parses the process arguments into an [`InitialAction`].
pub fn parse_cli_args() -> Option<InitialAction> {
    let args: Vec<String> = std::env::args().collect();
    parse_args(&args)
}

/// Parses a preset name as accepted after `--preset` (case-insensitive).
pub fn parse_preset(name: &str) -> Option<CompressionPreset> {
    match name.to_lowercase().as_str() {
        "low" | "lowest" | "fast" => Some(CompressionPreset::Low),
        "medium" | "med" | "balanced" => Some(CompressionPreset::Medium),
        "high" | "highest" | "max" => Some(CompressionPreset::High),
        _ => None,
    }
}

/// Parses an argument vector (`args[0]` is the program name).
pub fn parse_args(args: &[String]) -> Option<InitialAction> {
    let first = args.get(1)?;

    if first == "compress" && args.len() >= 3 {
        let path = PathBuf::from(&args[2]);
        let mut preset = None;
        let mut auto_start = false;

        for (i, arg) in args.iter().enumerate() {
            if arg == "--auto-start" {
                auto_start = true;
            }
            if arg == "--preset" {
                if let Some(parsed) = args.get(i + 1).and_then(|name| parse_preset(name)) {
                    preset = Some(parsed);
                }
            }
        }
        Some(InitialAction::Compress {
            path,
            preset,
            auto_start,
        })
    } else if first == "decompress" && args.len() >= 3 {
        let path = PathBuf::from(&args[2]);
        let auto_start = args
            .iter()
            .any(|a| a == "--auto-start" || a == "--extract-here");
        Some(InitialAction::Decompress { path, auto_start })
    } else {
        let path = PathBuf::from(first);
        if has_archive_extension(&path) {
            Some(InitialAction::Decompress {
                path,
                auto_start: false,
            })
        } else {
            Some(InitialAction::Compress {
                path,
                preset: None,
                auto_start: false,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        std::iter::once("nexus-gui")
            .chain(list.iter().copied())
            .map(String::from)
            .collect()
    }

    #[test]
    fn test_no_arguments_opens_home() {
        assert_eq!(parse_args(&args(&[])), None);
        assert_eq!(parse_args(&[]), None);
    }

    #[test]
    fn test_compress_plain() {
        assert_eq!(
            parse_args(&args(&["compress", "C:/data/file.txt"])),
            Some(InitialAction::Compress {
                path: PathBuf::from("C:/data/file.txt"),
                preset: None,
                auto_start: false,
            })
        );
    }

    #[test]
    fn test_compress_with_preset_and_auto_start() {
        assert_eq!(
            parse_args(&args(&[
                "compress",
                "folder",
                "--preset",
                "HIGH",
                "--auto-start"
            ])),
            Some(InitialAction::Compress {
                path: PathBuf::from("folder"),
                preset: Some(CompressionPreset::High),
                auto_start: true,
            })
        );
    }

    #[test]
    fn test_preset_aliases() {
        for name in ["low", "lowest", "fast", "Fast"] {
            assert_eq!(parse_preset(name), Some(CompressionPreset::Low));
        }
        for name in ["medium", "med", "balanced"] {
            assert_eq!(parse_preset(name), Some(CompressionPreset::Medium));
        }
        for name in ["high", "highest", "max"] {
            assert_eq!(parse_preset(name), Some(CompressionPreset::High));
        }
        assert_eq!(parse_preset("ultra"), None);
    }

    #[test]
    fn test_unknown_or_missing_preset_value_is_ignored() {
        let parsed = parse_args(&args(&["compress", "f", "--preset", "ultra"]));
        assert!(matches!(
            parsed,
            Some(InitialAction::Compress { preset: None, .. })
        ));
        let parsed = parse_args(&args(&["compress", "f", "--preset"]));
        assert!(matches!(
            parsed,
            Some(InitialAction::Compress { preset: None, .. })
        ));
    }

    #[test]
    fn test_decompress_variants() {
        assert_eq!(
            parse_args(&args(&["decompress", "a.nxar"])),
            Some(InitialAction::Decompress {
                path: PathBuf::from("a.nxar"),
                auto_start: false,
            })
        );
        for flag in ["--auto-start", "--extract-here"] {
            assert_eq!(
                parse_args(&args(&["decompress", "a.nxar", flag])),
                Some(InitialAction::Decompress {
                    path: PathBuf::from("a.nxar"),
                    auto_start: true,
                })
            );
        }
    }

    #[test]
    fn test_bare_archive_path_opens_extraction() {
        // This is the double-click on a .nxar file.
        assert_eq!(
            parse_args(&args(&["C:/x/backup.nxar"])),
            Some(InitialAction::Decompress {
                path: PathBuf::from("C:/x/backup.nxar"),
                auto_start: false,
            })
        );
        assert!(matches!(
            parse_args(&args(&["old.nxs"])),
            Some(InitialAction::Decompress { .. })
        ));
    }

    #[test]
    fn test_bare_other_path_opens_compression() {
        assert_eq!(
            parse_args(&args(&["C:/x/photo.png"])),
            Some(InitialAction::Compress {
                path: PathBuf::from("C:/x/photo.png"),
                preset: None,
                auto_start: false,
            })
        );
    }

    #[test]
    fn test_subcommand_without_path_is_treated_as_a_path() {
        // Matches the historical behaviour: a lone word is a path to compress.
        assert!(matches!(
            parse_args(&args(&["compress"])),
            Some(InitialAction::Compress { .. })
        ));
    }
}
