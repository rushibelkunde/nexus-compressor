//! Framing and block splitting logic (Layer 0).

use crate::block::detector::compute_shannon_entropy;

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

/// Dynamically splits a superblock into sub-blocks (16 KB - 512 KB) at entropy transition boundaries.
pub fn split_superblock_into_subblocks(data: &[u8]) -> Vec<SubBlock> {
    if data.is_empty() {
        return vec![SubBlock { data: Vec::new() }];
    }

    if data.len() <= MIN_SUBBLOCK_SIZE {
        return vec![SubBlock {
            data: data.to_vec(),
        }];
    }

    let mut sub_blocks = Vec::new();
    let mut start = 0;
    let mut curr = MIN_SUBBLOCK_SIZE;

    let initial_sample_len = std::cmp::min(ENTROPY_STEP_SIZE, data.len());
    let mut last_window_entropy = compute_shannon_entropy(&data[..initial_sample_len]);

    while curr + ENTROPY_STEP_SIZE <= data.len() {
        let block_len = curr - start;
        let window = &data[curr..curr + ENTROPY_STEP_SIZE];
        let cur_entropy = compute_shannon_entropy(window);
        let entropy_diff = (cur_entropy - last_window_entropy).abs();

        let should_split = block_len >= MAX_SUBBLOCK_SIZE
            || (block_len >= MIN_SUBBLOCK_SIZE && entropy_diff >= ENTROPY_CHANGE_THRESHOLD)
            || (block_len >= DEFAULT_TARGET_SUBBLOCK_SIZE && entropy_diff >= 0.25)
            || (block_len >= DEFAULT_TARGET_SUBBLOCK_SIZE + (64 * 1024));

        if should_split {
            sub_blocks.push(SubBlock {
                data: data[start..curr].to_vec(),
            });
            start = curr;
            last_window_entropy = cur_entropy;
            curr = start + MIN_SUBBLOCK_SIZE;
        } else {
            last_window_entropy = cur_entropy;
            curr += ENTROPY_STEP_SIZE;
        }
    }

    if start < data.len() {
        let tail = &data[start..];
        if tail.len() < MIN_SUBBLOCK_SIZE && !sub_blocks.is_empty() {
            sub_blocks.last_mut().unwrap().data.extend_from_slice(tail);
        } else {
            sub_blocks.push(SubBlock {
                data: tail.to_vec(),
            });
        }
    }

    sub_blocks
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
