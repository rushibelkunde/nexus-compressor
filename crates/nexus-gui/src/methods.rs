//! Compression-method families as shown in the block map and the method
//! breakdown bar.
//!
//! The core reports work per [`CompressionPath`](nexus_core::format::CompressionPath)
//! index (`path_bytes[i]`). The GUI folds those paths into five families a
//! user can tell apart — LZ, BWT, context mixing, neural, stored — and gives
//! each a fixed colour. Everything here is sized by
//! [`PATH_COUNT`], never by a literal.
//!
//! The four chromatic colours were checked for colour-vision-deficiency
//! separation on the panel background in every pairing (cells of any two
//! methods can be neighbours in the block map). `Stored` is deliberately a
//! neutral grey and is additionally drawn hollow, so it never relies on hue.

use eframe::egui::Color32;
use nexus_core::format::PATH_COUNT;
use nexus_core::pipeline::BlockInfo;

/// Number of method families.
pub const METHOD_COUNT: usize = 5;

/// Cumulative original bytes handled per compression path.
pub type PathBytes = [u64; PATH_COUNT];

/// Cumulative original bytes handled per method family, indexed by
/// [`Method::index`].
pub type MethodBytes = [u64; METHOD_COUNT];

/// A family of compression paths that is shown as one colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    /// LZ with ANS entropy coding (legacy path A and the current path A2).
    Lz,
    /// Burrows-Wheeler based coding (legacy path B and the adaptive path B2).
    Bwt,
    /// Bit-level context mixing (path C).
    ContextMix,
    /// Neural predictor (path D).
    Neural,
    /// Stored without compression (path E).
    Stored,
}

impl Method {
    /// Every method in display order (stack order of the breakdown bar).
    pub const ALL: [Method; METHOD_COUNT] = [
        Method::Lz,
        Method::Bwt,
        Method::ContextMix,
        Method::Neural,
        Method::Stored,
    ];

    /// Maps a `CompressionPath::to_u8()` index to its family. Returns `None`
    /// for an index this GUI does not know (a path added to the core later).
    pub fn from_path_index(index: usize) -> Option<Method> {
        match index {
            0 | 5 => Some(Method::Lz),
            1 | 6 => Some(Method::Bwt),
            2 => Some(Method::ContextMix),
            3 => Some(Method::Neural),
            4 => Some(Method::Stored),
            _ => None,
        }
    }

    /// Position of this method in [`Method::ALL`] and in [`MethodBytes`].
    pub fn index(self) -> usize {
        match self {
            Method::Lz => 0,
            Method::Bwt => 1,
            Method::ContextMix => 2,
            Method::Neural => 3,
            Method::Stored => 4,
        }
    }

    /// Short legend label.
    pub fn label(self) -> &'static str {
        match self {
            Method::Lz => "LZ",
            Method::Bwt => "BWT",
            Method::ContextMix => "Context mixing",
            Method::Neural => "Neural",
            Method::Stored => "Stored",
        }
    }

    /// Fixed chart colour of this method.
    pub fn color(self) -> Color32 {
        match self {
            Method::Lz => Color32::from_rgb(0x10, 0xB0, 0xD0),
            Method::Bwt => Color32::from_rgb(0xD2, 0x8E, 0x05),
            Method::ContextMix => Color32::from_rgb(0x8F, 0x7C, 0xF0),
            Method::Neural => Color32::from_rgb(0xE2, 0x4D, 0x8B),
            Method::Stored => Color32::from_rgb(0x8A, 0x93, 0xA8),
        }
    }

    /// Whether marks of this method are drawn as an outline instead of a
    /// solid fill (secondary, non-colour encoding for "not compressed").
    pub fn is_hollow(self) -> bool {
        matches!(self, Method::Stored)
    }
}

/// Element-wise `current - previous`, saturating at zero.
pub fn path_delta(previous: &PathBytes, current: &PathBytes) -> PathBytes {
    let mut delta = [0u64; PATH_COUNT];
    for ((d, cur), prev) in delta.iter_mut().zip(current).zip(previous) {
        *d = cur.saturating_sub(*prev);
    }
    delta
}

/// Element-wise maximum, used to keep cumulative counters monotonic when
/// progress updates arrive out of order.
pub fn path_max(a: &PathBytes, b: &PathBytes) -> PathBytes {
    let mut out = [0u64; PATH_COUNT];
    for ((o, x), y) in out.iter_mut().zip(a).zip(b) {
        *o = (*x).max(*y);
    }
    out
}

/// Folds per-path byte counts into per-method byte counts. Bytes of paths
/// this GUI does not know are left out.
pub fn method_totals(path_bytes: &PathBytes) -> MethodBytes {
    let mut totals = [0u64; METHOD_COUNT];
    for (index, bytes) in path_bytes.iter().enumerate() {
        if let Some(method) = Method::from_path_index(index) {
            totals[method.index()] = totals[method.index()].saturating_add(*bytes);
        }
    }
    totals
}

/// The method that handled the most bytes, or `None` when no bytes were
/// handled at all. Ties go to the method listed first in [`Method::ALL`].
pub fn dominant_of(totals: &MethodBytes) -> Option<Method> {
    let mut best: Option<(Method, u64)> = None;
    for method in Method::ALL {
        let bytes = totals[method.index()];
        if bytes > 0 && best.is_none_or(|(_, top)| bytes > top) {
            best = Some((method, bytes));
        }
    }
    best.map(|(method, _)| method)
}

/// The dominant method in a per-path delta (the bytes one finished
/// superblock added). This picks the colour of a block-map cell.
pub fn dominant_method(delta: &PathBytes) -> Option<Method> {
    dominant_of(&method_totals(delta))
}

/// Share of each method in `[0, 1]`; all zero when nothing was handled.
pub fn method_shares(totals: &MethodBytes) -> [f32; METHOD_COUNT] {
    let sum: u64 = totals.iter().fold(0u64, |acc, b| acc.saturating_add(*b));
    let mut shares = [0.0f32; METHOD_COUNT];
    if sum == 0 {
        return shares;
    }
    for (share, bytes) in shares.iter_mut().zip(totals) {
        *share = (*bytes as f64 / sum as f64) as f32;
    }
    shares
}

/// Formats a share as a percentage for legends, e.g. `82%`, `4.5%`, `<0.1%`.
pub fn format_share(share: f32) -> String {
    let pct = f64::from(share) * 100.0;
    if !pct.is_finite() || pct <= 0.0 {
        "0%".to_string()
    } else if pct < 0.1 {
        "<0.1%".to_string()
    } else if pct < 10.0 {
        format!("{pct:.1}%")
    } else {
        format!("{pct:.0}%")
    }
}

/// Sums the original bytes of an archive's blocks per compression path.
pub fn path_bytes_of_blocks(blocks: &[BlockInfo]) -> PathBytes {
    let mut totals = [0u64; PATH_COUNT];
    for block in blocks {
        let index = usize::from(block.compression_path.to_u8());
        if let Some(slot) = totals.get_mut(index) {
            *slot = slot.saturating_add(u64::from(block.original_size));
        }
    }
    totals
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(pairs: &[(usize, u64)]) -> PathBytes {
        let mut out = [0u64; PATH_COUNT];
        for (index, bytes) in pairs {
            out[*index] = *bytes;
        }
        out
    }

    #[test]
    fn test_every_core_path_maps_to_a_method() {
        for index in 0..PATH_COUNT {
            assert!(
                Method::from_path_index(index).is_some(),
                "path index {index} has no method family"
            );
        }
        assert_eq!(Method::from_path_index(PATH_COUNT + 50), None);
    }

    #[test]
    fn test_path_families() {
        assert_eq!(Method::from_path_index(0), Some(Method::Lz));
        assert_eq!(Method::from_path_index(5), Some(Method::Lz));
        assert_eq!(Method::from_path_index(1), Some(Method::Bwt));
        assert_eq!(Method::from_path_index(6), Some(Method::Bwt));
        assert_eq!(Method::from_path_index(2), Some(Method::ContextMix));
        assert_eq!(Method::from_path_index(3), Some(Method::Neural));
        assert_eq!(Method::from_path_index(4), Some(Method::Stored));
    }

    #[test]
    fn test_method_index_matches_all_order() {
        for (position, method) in Method::ALL.into_iter().enumerate() {
            assert_eq!(method.index(), position);
            assert!(!method.label().is_empty());
        }
    }

    #[test]
    fn test_method_colours_are_distinct() {
        for a in Method::ALL {
            for b in Method::ALL {
                if a != b {
                    assert_ne!(a.color(), b.color());
                }
            }
        }
    }

    #[test]
    fn test_path_delta_saturates() {
        let prev = paths(&[(5, 100), (1, 50)]);
        let cur = paths(&[(5, 160), (1, 40), (4, 7)]);
        let delta = path_delta(&prev, &cur);
        assert_eq!(delta[5], 60);
        assert_eq!(delta[1], 0);
        assert_eq!(delta[4], 7);
    }

    #[test]
    fn test_path_max_is_elementwise() {
        let a = paths(&[(5, 100), (1, 50)]);
        let b = paths(&[(5, 80), (1, 70), (2, 1)]);
        let m = path_max(&a, &b);
        assert_eq!(m[5], 100);
        assert_eq!(m[1], 70);
        assert_eq!(m[2], 1);
    }

    #[test]
    fn test_method_totals_merge_families() {
        let totals = method_totals(&paths(&[(0, 10), (5, 30), (1, 5), (4, 2)]));
        assert_eq!(totals[Method::Lz.index()], 40);
        assert_eq!(totals[Method::Bwt.index()], 5);
        assert_eq!(totals[Method::Stored.index()], 2);
        assert_eq!(totals[Method::Neural.index()], 0);
    }

    #[test]
    fn test_method_totals_merge_both_bwt_paths() {
        if PATH_COUNT > 6 {
            let totals = method_totals(&paths(&[(1, 5), (6, 9)]));
            assert_eq!(totals[Method::Bwt.index()], 14);
        }
    }

    #[test]
    fn test_dominant_method_picks_largest_family() {
        assert_eq!(
            dominant_method(&paths(&[(5, 900), (1, 100)])),
            Some(Method::Lz)
        );
        assert_eq!(
            dominant_method(&paths(&[(5, 100), (1, 900)])),
            Some(Method::Bwt)
        );
        assert_eq!(
            dominant_method(&paths(&[(2, 3), (3, 2)])),
            Some(Method::ContextMix)
        );
        assert_eq!(dominant_method(&paths(&[(3, 8)])), Some(Method::Neural));
        assert_eq!(dominant_method(&paths(&[(4, 1)])), Some(Method::Stored));
    }

    #[test]
    fn test_dominant_method_sums_family_members() {
        // Legacy LZ (0) + LZ v2 (5) together outweigh BWT.
        assert_eq!(
            dominant_method(&paths(&[(0, 40), (5, 40), (1, 60)])),
            Some(Method::Lz)
        );
    }

    #[test]
    fn test_dominant_method_empty_delta_is_none() {
        assert_eq!(dominant_method(&[0u64; PATH_COUNT]), None);
    }

    #[test]
    fn test_dominant_method_tie_goes_to_first_listed() {
        assert_eq!(
            dominant_method(&paths(&[(5, 50), (1, 50)])),
            Some(Method::Lz)
        );
    }

    #[test]
    fn test_method_shares_sum_to_one() {
        let shares = method_shares(&[60, 20, 10, 0, 10]);
        assert!((shares.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        assert!((shares[0] - 0.6).abs() < 1e-6);
        assert_eq!(method_shares(&[0; METHOD_COUNT]), [0.0; METHOD_COUNT]);
    }

    #[test]
    fn test_format_share() {
        assert_eq!(format_share(0.0), "0%");
        assert_eq!(format_share(0.0005), "<0.1%");
        assert_eq!(format_share(0.045), "4.5%");
        assert_eq!(format_share(0.82), "82%");
        assert_eq!(format_share(1.0), "100%");
        assert_eq!(format_share(f32::NAN), "0%");
    }
}
