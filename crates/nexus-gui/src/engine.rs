//! What each compression level means to a user: the engine behind it, its
//! reference throughput, its superblock (parallel unit) size and a rough
//! time estimate.
//!
//! The throughput table holds single-thread figures measured on the
//! development machine. Anything derived from it is an *estimate* and must be
//! labelled as such wherever it is shown.

use nexus_core::compress::lz::superblock_size_for_level;
use nexus_core::pipeline::CompressionPreset;

/// Lowest selectable compression level.
pub const MIN_LEVEL: u32 = 1;
/// Highest selectable compression level.
pub const MAX_LEVEL: u32 = 19;
/// First level of the "much slower" modelling tiers.
pub const SLOW_LEVEL: u32 = 13;
/// First level that also tries the neural model.
pub const NEURAL_LEVEL: u32 = 17;
/// The trial compression behind the size estimate is capped at this level;
/// above it the estimate is an upper bound.
pub const ESTIMATE_LEVEL_CAP: u32 = 12;

/// The presets offered as cards, fastest first.
pub const PRESETS: [CompressionPreset; 3] = [
    CompressionPreset::Low,
    CompressionPreset::Medium,
    CompressionPreset::High,
];

/// Human-readable description of the engine used at a level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineInfo {
    /// Short engine name, e.g. `Lazy LZ`.
    pub name: &'static str,
    /// One-sentence description of what the engine does.
    pub detail: &'static str,
}

/// Clamps a level into the selectable range.
pub fn clamp_level(level: u32) -> u32 {
    level.clamp(MIN_LEVEL, MAX_LEVEL)
}

/// Describes the engine used at `level`.
pub fn engine_for_level(level: u32) -> EngineInfo {
    match clamp_level(level) {
        1..=2 => EngineInfo {
            name: "Fast LZ",
            detail: "Single hash probe per position. Highest throughput, lightest on CPU.",
        },
        3..=4 => EngineInfo {
            name: "Double-hash LZ",
            detail: "Two hash probes per position for better matches at high speed.",
        },
        5..=7 => EngineInfo {
            name: "Lazy LZ",
            detail: "Lazy matching with a row-hash match finder. Balanced speed and ratio.",
        },
        8..=9 => EngineInfo {
            name: "Deep lazy LZ",
            detail: "Lazy matching over hash chains; searches much deeper for long matches.",
        },
        10..=12 => EngineInfo {
            name: "Optimal-parse LZ",
            detail: "Binary-tree match finder, optimal parsing and context-modelled literals.",
        },
        13..=15 => EngineInfo {
            name: "Optimal LZ + BWT modelling",
            detail: "Also tries BWT with adaptive modelling on every block and keeps whichever \
                     is smaller. BWT wins on text; those blocks decompress slowly (~15 MB/s).",
        },
        16 => EngineInfo {
            name: "Optimal LZ + BWT + context mixing",
            detail: "Additionally tries the context-mixing model on every block and keeps \
                     the smallest result.",
        },
        _ => EngineInfo {
            name: "Optimal LZ + BWT + CM + neural",
            detail: "Additionally tries the neural model on every block. Extremely slow.",
        },
    }
}

/// Reference single-thread compression throughput in MB/s at `level`.
pub fn reference_throughput_mb_s(level: u32) -> f64 {
    match clamp_level(level) {
        1 => 105.0,
        2 => 100.0,
        3 => 85.0,
        4 => 50.0,
        5 => 24.0,
        6 => 14.0,
        7 => 10.0,
        8 => 1.6,
        9 => 1.0,
        10 => 1.5,
        11 => 1.0,
        12 => 0.7,
        13..=16 => 0.5,
        _ => 0.15,
    }
}

/// Superblock (independent parallel unit) size in bytes at `level`.
pub fn superblock_bytes(level: u32) -> u64 {
    (superblock_size_for_level(level) as u64).max(1)
}

/// Number of superblocks an input of `total_bytes` is split into at `level`.
/// An empty input still produces one (empty) superblock.
pub fn superblock_count(total_bytes: u64, level: u32) -> u64 {
    total_bytes.div_ceil(superblock_bytes(level)).max(1)
}

/// Number of logical CPUs available for parallel superblocks.
pub fn available_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

/// Number of superblocks that can actually run in parallel.
pub fn parallel_units(total_bytes: u64, level: u32, threads: usize) -> u64 {
    superblock_count(total_bytes, level).min(threads.max(1) as u64)
}

/// Rough compression time in seconds: reference throughput multiplied by the
/// number of superblocks that can run in parallel. This is an estimate, not a
/// promise — actual speed depends on the data and the machine.
pub fn estimate_compress_seconds(total_bytes: u64, level: u32, threads: usize) -> f64 {
    let mb = total_bytes as f64 / (1024.0 * 1024.0);
    let speed =
        reference_throughput_mb_s(level) * parallel_units(total_bytes, level, threads) as f64;
    if speed <= 0.0 {
        return f64::INFINITY;
    }
    mb / speed
}

/// Whether `level` belongs to the much slower modelling tiers.
pub fn is_slow_level(level: u32) -> bool {
    level >= SLOW_LEVEL
}

/// The preset whose level is exactly `level`, if any.
pub fn preset_for_level(level: u32) -> Option<CompressionPreset> {
    PRESETS.into_iter().find(|p| p.to_level() == level)
}

/// Card title of a preset.
pub fn preset_title(preset: CompressionPreset) -> &'static str {
    match preset {
        CompressionPreset::Low => "Fast",
        CompressionPreset::Medium => "Balanced",
        CompressionPreset::High => "Maximum",
    }
}

/// Card tagline of a preset.
pub fn preset_tagline(preset: CompressionPreset) -> &'static str {
    match preset {
        CompressionPreset::Low => "Quick backups, big files",
        CompressionPreset::Medium => "Recommended for everyday use",
        CompressionPreset::High => "Smallest archive, takes time",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1024 * 1024;

    #[test]
    fn test_every_level_has_an_engine_and_positive_throughput() {
        for level in MIN_LEVEL..=MAX_LEVEL {
            let info = engine_for_level(level);
            assert!(!info.name.is_empty());
            assert!(!info.detail.is_empty());
            assert!(reference_throughput_mb_s(level) > 0.0);
            assert!(superblock_bytes(level) >= MB);
        }
    }

    #[test]
    fn test_out_of_range_levels_are_clamped() {
        assert_eq!(engine_for_level(0), engine_for_level(1));
        assert_eq!(engine_for_level(99), engine_for_level(19));
        assert_eq!(reference_throughput_mb_s(0), reference_throughput_mb_s(1));
        assert_eq!(clamp_level(0), 1);
        assert_eq!(clamp_level(40), 19);
    }

    #[test]
    fn test_engine_tiers_change_at_documented_levels() {
        assert_eq!(engine_for_level(1), engine_for_level(2));
        assert_ne!(engine_for_level(2), engine_for_level(3));
        assert_ne!(engine_for_level(4), engine_for_level(5));
        assert_eq!(engine_for_level(5), engine_for_level(7));
        assert_ne!(engine_for_level(9), engine_for_level(10));
        assert_ne!(engine_for_level(12), engine_for_level(13));
        assert_eq!(engine_for_level(13), engine_for_level(15));
        assert_ne!(engine_for_level(15), engine_for_level(16));
        assert_ne!(engine_for_level(16), engine_for_level(17));
        assert_eq!(engine_for_level(17), engine_for_level(19));
    }

    #[test]
    fn test_presets_map_to_their_levels() {
        for preset in PRESETS {
            assert_eq!(preset_for_level(preset.to_level()), Some(preset));
            assert!(!preset_title(preset).is_empty());
            assert!(!preset_tagline(preset).is_empty());
        }
        assert_eq!(preset_for_level(7), None);
        assert_eq!(preset_for_level(19), None);
    }

    #[test]
    fn test_superblock_count_rounds_up_and_is_never_zero() {
        let sb = superblock_bytes(6);
        assert_eq!(superblock_count(0, 6), 1);
        assert_eq!(superblock_count(1, 6), 1);
        assert_eq!(superblock_count(sb, 6), 1);
        assert_eq!(superblock_count(sb + 1, 6), 2);
        assert_eq!(superblock_count(10 * sb, 6), 10);
    }

    #[test]
    fn test_parallel_units_limited_by_threads_and_blocks() {
        let sb = superblock_bytes(6);
        assert_eq!(parallel_units(sb / 2, 6, 16), 1);
        assert_eq!(parallel_units(4 * sb, 6, 16), 4);
        assert_eq!(parallel_units(100 * sb, 6, 8), 8);
        assert_eq!(parallel_units(100 * sb, 6, 0), 1);
    }

    #[test]
    fn test_estimate_seconds_single_block() {
        // 1 MB at level 2 (100 MB/s, one superblock) takes ~10 ms.
        let secs = estimate_compress_seconds(MB, 2, 8);
        assert!((secs - 0.01).abs() < 1e-9);
        assert_eq!(estimate_compress_seconds(0, 6, 8), 0.0);
    }

    #[test]
    fn test_estimate_seconds_scales_with_threads() {
        let total = 64 * superblock_bytes(6);
        let one = estimate_compress_seconds(total, 6, 1);
        let eight = estimate_compress_seconds(total, 6, 8);
        assert!((one / eight - 8.0).abs() < 1e-9);
    }

    #[test]
    fn test_slow_levels() {
        assert!(!is_slow_level(12));
        assert!(is_slow_level(13));
        assert!(is_slow_level(19));
    }
}
