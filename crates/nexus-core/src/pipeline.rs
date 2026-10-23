//! End-to-end compression and decompression pipeline.

use crate::block::detector::detect_block_type;
use crate::block::splitter::{split_into_superblocks, DEFAULT_SUPERBLOCK_SIZE};
use crate::compress::lz77::{deserialize_tokens, serialize_tokens, Lz77Decoder, Lz77Encoder};
use crate::entropy::huffman::{HuffmanDecoder, HuffmanEncoder};
use crate::error::{NexusError, Result};
use crate::format::{
    BlockTableEntry, CompressionPath, FileHeader, Footer, PreprocessFlags, SubBlockHeader,
    FILE_HEADER_SIZE, FOOTER_SIZE,
};
use xxhash_rust::xxh32::xxh32;
use xxhash_rust::xxh64::xxh64;

/// Compresses a raw byte stream into the .nxs container format.
///
/// # Arguments
/// * `input` - Raw uncompressed bytes
/// * `_level` - Compression level (1-19, reserved for tuning in future phases)
pub fn compress(input: &[u8], _level: u32) -> Result<Vec<u8>> {
    let superblocks = split_into_superblocks(input, DEFAULT_SUPERBLOCK_SIZE);
    let mut output = Vec::new();

    // 1. Write file header
    let header = FileHeader::new(input.len() as u64, superblocks.len() as u32);
    header.write_to(&mut output)?;

    let lz77_encoder = Lz77Encoder::default();
    let huffman_encoder = HuffmanEncoder::new();

    let mut block_table = Vec::new();

    for superblock in &superblocks {
        for sub_block in &superblock.sub_blocks {
            let file_offset = output.len() as u64;
            let block_checksum = xxh32(&sub_block.data, 0);
            let block_type = detect_block_type(&sub_block.data);

            let (path, compressed_payload) = if sub_block.data.is_empty() {
                (CompressionPath::PathE, Vec::new())
            } else {
                let tokens = lz77_encoder.encode(&sub_block.data);
                let serialized = serialize_tokens(&tokens);
                let huff = huffman_encoder.encode(&serialized);

                // Fallback to Store if compressed size is not smaller than original
                if huff.len() < sub_block.data.len() {
                    (CompressionPath::PathA, huff)
                } else {
                    (CompressionPath::PathE, sub_block.data.clone())
                }
            };

            let mini_header = SubBlockHeader::new(
                compressed_payload.len() as u32,
                sub_block.data.len() as u32,
                block_type,
                path,
                PreprocessFlags::empty(),
                block_checksum,
            );
            mini_header.write_to(&mut output)?;
            output.extend_from_slice(&compressed_payload);

            block_table.push(BlockTableEntry {
                superblock_id: superblock.id,
                file_offset,
                compressed_size: compressed_payload.len() as u32,
                original_size: sub_block.data.len() as u32,
            });
        }
    }

    // 2. Write block table
    let block_table_offset = output.len() as u64;
    for entry in &block_table {
        entry.write_to(&mut output)?;
    }

    // 3. Write footer with integrity checksum
    let file_checksum = xxh64(&output, 0);
    let footer = Footer::new(block_table_offset, block_table.len() as u32, file_checksum);
    footer.write_to(&mut output)?;

    Ok(output)
}

/// Decompresses a .nxs byte stream back into original raw bytes.
pub fn decompress(compressed: &[u8]) -> Result<Vec<u8>> {
    if compressed.len() < FILE_HEADER_SIZE + FOOTER_SIZE {
        return Err(NexusError::DataCorruption(
            "Input too short to contain valid .nxs header and footer".into(),
        ));
    }

    // 1. Read and validate footer
    let footer_start = compressed.len() - FOOTER_SIZE;
    let footer = Footer::read_from(&compressed[footer_start..])?;

    // Verify full file checksum over bytes [0..footer_start]
    let computed_file_checksum = xxh64(&compressed[..footer_start], 0);
    if computed_file_checksum != footer.file_checksum {
        return Err(NexusError::ChecksumMismatch {
            expected: footer.file_checksum,
            actual: computed_file_checksum,
        });
    }

    // 2. Read and validate file header
    let header = FileHeader::read_from(&compressed[..FILE_HEADER_SIZE])?;

    // 3. Decompress sub-blocks sequentially
    let lz77_decoder = Lz77Decoder::new();
    let huffman_decoder = HuffmanDecoder::new();

    let mut cursor = FILE_HEADER_SIZE;
    let mut decompressed = Vec::with_capacity(header.original_size as usize);

    while cursor < footer.block_table_offset as usize {
        let mini_header = SubBlockHeader::read_from(&compressed[cursor..])?;
        cursor += crate::format::MINI_HEADER_SIZE;

        let payload_len = mini_header.compressed_size as usize;
        if cursor + payload_len > footer.block_table_offset as usize {
            return Err(NexusError::DataCorruption(
                "Sub-block compressed payload exceeds boundary".into(),
            ));
        }

        let payload = &compressed[cursor..cursor + payload_len];
        cursor += payload_len;

        let sub_block_bytes = match mini_header.compression_path {
            CompressionPath::PathA => {
                let serialized_tokens = huffman_decoder.decode(payload)?;
                let tokens = deserialize_tokens(&serialized_tokens)?;
                lz77_decoder.decode(&tokens)?
            }
            CompressionPath::PathE => payload.to_vec(),
            other => return Err(NexusError::UnsupportedPath(other.to_u8())),
        };

        // Verify sub-block checksum
        let computed_checksum = xxh32(&sub_block_bytes, 0);
        if computed_checksum != mini_header.block_checksum {
            return Err(NexusError::BlockChecksumMismatch {
                expected: mini_header.block_checksum,
                actual: computed_checksum,
            });
        }

        if sub_block_bytes.len() != mini_header.original_size as usize {
            return Err(NexusError::DataCorruption(
                "Sub-block uncompressed size does not match mini-header original size".into(),
            ));
        }

        decompressed.extend_from_slice(&sub_block_bytes);
    }

    if decompressed.len() as u64 != header.original_size {
        return Err(NexusError::DataCorruption(format!(
            "Total uncompressed size mismatch: header says {}, got {}",
            header.original_size,
            decompressed.len()
        )));
    }

    Ok(decompressed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipeline_empty() {
        let input = b"";
        let compressed = compress(input, 1).unwrap();
        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(decompressed, input);
    }

    #[test]
    fn test_pipeline_single_byte() {
        let input = b"A";
        let compressed = compress(input, 1).unwrap();
        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(decompressed, input);
    }

    #[test]
    fn test_pipeline_repetitive_text() {
        let input = b"The quick brown fox jumps over the lazy dog. The quick brown fox jumps over the lazy dog.";
        let compressed = compress(input, 1).unwrap();
        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(decompressed, input);
    }

    #[test]
    fn test_pipeline_random_data() {
        let mut rng_state: u64 = 0xFEEDBEEF12345678;
        let mut input = vec![0u8; 32768];
        for b in input.iter_mut() {
            rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
            *b = (rng_state >> 33) as u8;
        }

        let compressed = compress(&input, 1).unwrap();
        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(decompressed, input);
    }

    #[test]
    fn test_pipeline_multi_block() {
        // Multi-block test using small blocks (we can test with 250KB repetitive data)
        let pattern = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
        let mut input = Vec::new();
        while input.len() < 1_200_000 {
            input.extend_from_slice(pattern);
        }

        let compressed = compress(&input, 1).unwrap();
        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(decompressed, input);
    }
}
