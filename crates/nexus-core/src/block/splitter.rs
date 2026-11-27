//! Framing and block splitting logic (Layer 0).

use crate::block::detector::entropy_from_histogram;
use crate::entropy::rans4::histogram;

/// Default superblock size in bytes (1 MB).
pub const DEFAULT_SUPERBLOCK_SIZE: usize = 1024 * 1024;
/// Minimum sub-block size in bytes (16 KB).
pub const MIN_SUBBLOCK_SIZE: usize = 16 * 1024;
/// Maximum sub-block size in bytes (512 KB).
pub const MAX_SUBBLOCK_SIZE: usize = 512 * 1024;
/// Default target sub-block size in bytes (128 KB).
pub const DEFAULT_TARGET_SUBBLOCK_SIZE: usize = 128 * 1024;
/// Step size for entropy sliding window (4 KB).
pub const ENTROPY_STEP_SIZE: usize = 4096;
/// Entropy difference threshold indicating content type transition.
pub const ENTROPY_CHANGE_THRESHOLD: f64 = 0.75;

/// A sub-block of data within a superblock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubBlock {
    pub data: Vec<u8>,
}

/// An independently compressed superblock unit (parallelizable).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Superblock {
    pub id: u32,
    pub sub_blocks: Vec<SubBlock>,
}

/// A planned sub-block: its byte range within the superblock and the byte
/// histogram of that range (collected for free while looking for boundaries).
#[derive(Debug, Clone)]
pub struct SubBlockPlan {
    /// Byte range within the superblock.
    pub range: std::ops::Range<usize>,
    /// Occurrence count of every byte value in `range`.
    pub hist: [u32; 256],
}

/// Plans the sub-blocks (16 KB - 512 KB) of a superblock, cutting at entropy
/// transitions, in a single pass over the data and without copying it.
///
/// The returned ranges are contiguous and cover `data` exactly (an empty
/// input yields one empty plan).
pub fn plan_subblocks(data: &[u8]) -> Vec<SubBlockPlan> {
    plan_subblocks_with_target(data, DEFAULT_TARGET_SUBBLOCK_SIZE)
}

/// Like [`plan_subblocks`], with an explicit target sub-block size: blocks
/// are cut at mild entropy changes once they reach `target`, and
/// unconditionally 64 KB later (never beyond [`MAX_SUBBLOCK_SIZE`]).
pub fn plan_subblocks_with_target(data: &[u8], target: usize) -> Vec<SubBlockPlan> {
    let target = target.clamp(MIN_SUBBLOCK_SIZE, MAX_SUBBLOCK_SIZE);
    let n = data.len();
    if n <= MIN_SUBBLOCK_SIZE {
        return vec![SubBlockPlan {
            range: 0..n,
            hist: histogram(data),
        }];
    }

    let mut plans: Vec<SubBlockPlan> = Vec::new();
    let mut start = 0usize;
    let mut block_hist = [0u32; 256];
    let mut prev_entropy = 0.0f64;

    let mut offset = 0usize;
    while offset < n {
        let window_end = (offset + ENTROPY_STEP_SIZE).min(n);
        let window_hist = histogram(&data[offset..window_end]);
        let full_window = window_end - offset == ENTROPY_STEP_SIZE;
        let entropy = entropy_from_histogram(&window_hist, window_end - offset);

        let block_len = offset - start;
        if full_window && block_len >= MIN_SUBBLOCK_SIZE {
            let entropy_diff = (entropy - prev_entropy).abs();
            let should_split = block_len >= MAX_SUBBLOCK_SIZE
                || entropy_diff >= ENTROPY_CHANGE_THRESHOLD
                || (block_len >= target && entropy_diff >= 0.25)
                || block_len >= target + (64 * 1024);
            if should_split {
                plans.push(SubBlockPlan {
                    range: start..offset,
                    hist: block_hist,
                });
                start = offset;
                block_hist = [0u32; 256];
            }
        }

        for (acc, &c) in block_hist.iter_mut().zip(&window_hist) {
            *acc += c;
        }
        prev_entropy = entropy;
        offset = window_end;
    }

    // A short tail joins the previous sub-block instead of standing alone.
    match plans.last_mut() {
        Some(last) if n - start < MIN_SUBBLOCK_SIZE => {
            last.range.end = n;
            for (acc, &c) in last.hist.iter_mut().zip(&block_hist) {
                *acc += c;
            }
        }
        _ => plans.push(SubBlockPlan {
            range: start..n,
            hist: block_hist,
        }),
    }

    plans
}

/// Computes sub-block boundaries (16 KB - 512 KB) for a superblock.
/// See [`plan_subblocks`].
pub fn split_subblock_ranges(data: &[u8]) -> Vec<std::ops::Range<usize>> {
    plan_subblocks(data).into_iter().map(|p| p.range).collect()
}

/// Dynamically splits a superblock into sub-blocks (16 KB - 512 KB) at entropy transition boundaries.
pub fn split_superblock_into_subblocks(data: &[u8]) -> Vec<SubBlock> {
    split_subblock_ranges(data)
        .into_iter()
        .map(|r| SubBlock {
            data: data[r].to_vec(),
        })
        .collect()
}

/// Splits a raw byte stream into superblocks of at most `superblock_size` bytes.
///
/// Each superblock is dynamically segmented into sub-blocks based on entropy transitions.
pub fn split_into_superblocks(data: &[u8], superblock_size: usize) -> Vec<Superblock> {
    let size = if superblock_size == 0 {
        DEFAULT_SUPERBLOCK_SIZE
    } else {
        superblock_size
    };

    if data.is_empty() {
        return vec![Superblock {
            id: 0,
            sub_blocks: vec![SubBlock { data: Vec::new() }],
        }];
    }

    let mut superblocks = Vec::new();

    for (id, chunk) in data.chunks(size).enumerate() {
        superblocks.push(Superblock {
            id: id as u32,
            sub_blocks: split_superblock_into_subblocks(chunk),
        });
    }

    superblocks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_empty() {
        let blocks = split_into_superblocks(b"", 1024);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].sub_blocks[0].data.len(), 0);
    }

    #[test]
    fn test_split_single_block() {
        let data = vec![1u8; 500];
        let blocks = split_into_superblocks(&data, 1024);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].sub_blocks[0].data.len(), 500);
    }

    #[test]
    fn test_split_multiple_blocks() {
        let data = vec![2u8; 2500];
        let blocks = split_into_superblocks(&data, 1000);
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].sub_blocks[0].data.len(), 1000);
        assert_eq!(blocks[1].sub_blocks[0].data.len(), 1000);
        assert_eq!(blocks[2].sub_blocks[0].data.len(), 500);
    }

    #[test]
    fn test_split_superblock_entropy_transition() {
        // 32 KB repetitive text followed by 32 KB high-entropy random data
        let mut mixed = Vec::new();
        mixed.extend(vec![b'A'; 32 * 1024]);

        let mut rng: u64 = 0xDEADBEEFCAFE;
        for _ in 0..32 * 1024 {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            mixed.push((rng >> 33) as u8);
        }

        let sub_blocks = split_superblock_into_subblocks(&mixed);
        assert!(
            sub_blocks.len() >= 2,
            "Should have split into at least 2 sub-blocks due to entropy transition, got {}",
            sub_blocks.len()
        );

        let reconstructed: Vec<u8> = sub_blocks.iter().flat_map(|sb| sb.data.clone()).collect();
        assert_eq!(reconstructed, mixed);
    }
}
