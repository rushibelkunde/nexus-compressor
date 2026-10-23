//! Framing and block splitting logic (Layer 0).

/// Default superblock size in bytes (1 MB).
pub const DEFAULT_SUPERBLOCK_SIZE: usize = 1024 * 1024;

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

/// Splits a raw byte stream into superblocks of at most `superblock_size` bytes.
///
/// In Phase 1, each superblock contains exactly one sub-block.
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
            sub_blocks: vec![SubBlock {
                data: chunk.to_vec(),
            }],
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
}
