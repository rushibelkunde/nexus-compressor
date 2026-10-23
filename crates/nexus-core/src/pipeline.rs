//! End-to-end compression and decompression pipeline.
//!
//! Integrates Layer 0 (framing & parallel superblocks), Layer 1 (data detection),
//! Layer 2 (bijective MTF/RLE transforms), Layer 3 (Path A: LZ-ANS, Path B: BWT-ANS),
//! and Layer 4 (.nxs container assembly).

use crate::block::detector::detect_block_type;
use crate::block::splitter::{split_into_superblocks, SubBlock, DEFAULT_SUPERBLOCK_SIZE};
use crate::compress::bwt::{bwt_decode, bwt_encode};
use crate::compress::lz77::{deserialize_tokens, serialize_tokens, Lz77Decoder, Lz77Encoder};
use crate::entropy::rans::{RansDecoder, RansEncoder};
use crate::error::{NexusError, Result};
use crate::format::{
    BlockTableEntry, BlockType, CompressionPath, FileHeader, Footer, PreprocessFlags,
    SubBlockHeader, FILE_HEADER_SIZE, FOOTER_SIZE, MINI_HEADER_SIZE,
};
use crate::preprocess::mtf::{mtf_decode, mtf_encode};
use crate::preprocess::rle::{zero_rle_decode, zero_rle_encode};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use rayon::prelude::*;
use xxhash_rust::xxh32::xxh32;
use xxhash_rust::xxh64::xxh64;

/// Compresses a single sub-block, choosing the best compression path.
fn compress_sub_block(sub_block: &SubBlock, level: u32) -> (SubBlockHeader, Vec<u8>) {
    let block_checksum = xxh32(&sub_block.data, 0);
    let block_type = detect_block_type(&sub_block.data);
    let orig_len = sub_block.data.len();

    if orig_len == 0 {
        let mini_header = SubBlockHeader::new(
            0,
            0,
            block_type,
            CompressionPath::PathE,
            PreprocessFlags::empty(),
            block_checksum,
        );
        return (mini_header, Vec::new());
    }

    let rans_encoder = RansEncoder::new();

    // Strategy 1: Path B (BWT + MTF + Zero-RLE + rANS)
    // Highly effective for natural language text, source code, and structured data (<= 128 KB)
    let try_path_b =
        (block_type == BlockType::Text || block_type == BlockType::SourceCode || level >= 7)
            && orig_len <= 131_072;

    let path_b_result: Option<(Vec<u8>, PreprocessFlags)> = if try_path_b {
        let (bwt_bytes, primary_index) = bwt_encode(&sub_block.data);
        let mtf_bytes = mtf_encode(&bwt_bytes);
        let rle_bytes = zero_rle_encode(&mtf_bytes);
        let rans_encoded = rans_encoder.encode(&rle_bytes);

        let mut payload = Vec::with_capacity(rans_encoded.len() + 4);
        payload.write_u32::<LittleEndian>(primary_index).unwrap();
        payload.extend_from_slice(&rans_encoded);

        let mut flags = PreprocessFlags::empty();
        flags.insert(PreprocessFlags::MTF);
        flags.insert(PreprocessFlags::RLE);
        Some((payload, flags))
    } else {
        None
    };

    // Strategy 2: Path A (LZ77 + rANS)
    let try_path_a = !try_path_b || level >= 7 || block_type == BlockType::Binary;

    let path_a_result: Option<Vec<u8>> = if try_path_a || path_b_result.is_none() {
        let lz77_encoder = Lz77Encoder::default();
        let tokens = lz77_encoder.encode(&sub_block.data);
        let serialized = serialize_tokens(&tokens);
        let rans_encoded = rans_encoder.encode(&serialized);
        Some(rans_encoded)
    } else {
        None
    };

    // Select the path that yields the smallest payload
    let mut best_path = CompressionPath::PathE;
    let mut best_payload = sub_block.data.clone();
    let mut best_flags = PreprocessFlags::empty();
    let mut best_len = orig_len;

    if let Some((payload, flags)) = path_b_result {
        if payload.len() < best_len {
            best_len = payload.len();
            best_path = CompressionPath::PathB;
            best_payload = payload;
            best_flags = flags;
        }
    }

    if let Some(payload) = path_a_result {
        if payload.len() < best_len {
            best_path = CompressionPath::PathA;
            best_payload = payload;
            best_flags = PreprocessFlags::empty();
        }
    }

    let mini_header = SubBlockHeader::new(
        best_payload.len() as u32,
        orig_len as u32,
        block_type,
        best_path,
        best_flags,
        block_checksum,
    );

    (mini_header, best_payload)
}

/// Decompresses a single sub-block given its mini-header and compressed payload.
fn decompress_sub_block(mini_header: &SubBlockHeader, payload: &[u8]) -> Result<Vec<u8>> {
    let rans_decoder = RansDecoder::new();
    let lz77_decoder = Lz77Decoder::new();

    let sub_block_bytes = match mini_header.compression_path {
        CompressionPath::PathB => {
            if payload.len() < 4 {
                return Err(NexusError::DataCorruption(
                    "Path B payload too short for BWT index".into(),
                ));
            }
            let mut cursor = payload;
            let primary_index = cursor.read_u32::<LittleEndian>()?;
            let rle_bytes = rans_decoder.decode(cursor)?;
            let mtf_bytes = zero_rle_decode(&rle_bytes)?;
            let bwt_bytes = mtf_decode(&mtf_bytes);
            bwt_decode(&bwt_bytes, primary_index)?
        }
        CompressionPath::PathA => {
            let serialized_tokens = rans_decoder.decode(payload)?;
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

    Ok(sub_block_bytes)
}

type CompressedBlock = (SubBlockHeader, Vec<u8>);
type SuperblockResult = (u32, Vec<CompressedBlock>);

/// Compresses a raw byte stream into the .nxs container format.
///
/// Uses multi-core parallel superblock compression via Rayon.
///
/// # Arguments
/// * `input` - Raw uncompressed bytes
/// * `level` - Compression level (1-19)
pub fn compress(input: &[u8], level: u32) -> Result<Vec<u8>> {
    let superblocks = split_into_superblocks(input, DEFAULT_SUPERBLOCK_SIZE);
    let mut output = Vec::new();

    // 1. Write file header
    let header = FileHeader::new(input.len() as u64, superblocks.len() as u32);
    header.write_to(&mut output)?;

    // 2. Parallel compression of superblocks via Rayon
    let compressed_superblocks: Vec<SuperblockResult> = superblocks
        .into_par_iter()
        .map(|sb| {
            let compressed_blocks = sb
                .sub_blocks
                .iter()
                .map(|sub_block| compress_sub_block(sub_block, level))
                .collect();
            (sb.id, compressed_blocks)
        })
        .collect();

    // 3. Sequential container assembly
    let mut block_table = Vec::new();

    for (sb_id, blocks) in compressed_superblocks {
        for (mini_header, payload) in blocks {
            let file_offset = output.len() as u64;

            mini_header.write_to(&mut output)?;
            output.extend_from_slice(&payload);

            block_table.push(BlockTableEntry {
                superblock_id: sb_id,
                file_offset,
                compressed_size: payload.len() as u32,
                original_size: mini_header.original_size,
            });
        }
    }

    // 4. Write block table
    let block_table_offset = output.len() as u64;
    for entry in &block_table {
        entry.write_to(&mut output)?;
    }

    // 5. Write footer with XXHash64 integrity checksum
    let file_checksum = xxh64(&output, 0);
    let footer = Footer::new(block_table_offset, block_table.len() as u32, file_checksum);
    footer.write_to(&mut output)?;

    Ok(output)
}

/// Decompresses a .nxs byte stream back into original raw bytes.
///
/// Validates footer, headers, checksums, and decodes sub-blocks.
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

    // 3. Read block table entries from footer area
    let table_start = footer.block_table_offset as usize;
    if table_start > footer_start {
        return Err(NexusError::DataCorruption(
            "Block table offset exceeds footer position".into(),
        ));
    }

    let mut table_cursor = &compressed[table_start..footer_start];
    let mut entries = Vec::with_capacity(footer.total_block_count as usize);
    for _ in 0..footer.total_block_count {
        entries.push(BlockTableEntry::read_from(&mut table_cursor)?);
    }

    // 4. Parallel sub-block decompression using Rayon
    let decompressed_blocks: Result<Vec<Vec<u8>>> = entries
        .into_par_iter()
        .map(|entry| {
            let offset = entry.file_offset as usize;
            if offset + MINI_HEADER_SIZE > compressed.len() {
                return Err(NexusError::DataCorruption(
                    "Block offset exceeds file boundary".into(),
                ));
            }

            let mini_header =
                SubBlockHeader::read_from(&compressed[offset..offset + MINI_HEADER_SIZE])?;
            let payload_start = offset + MINI_HEADER_SIZE;
            let payload_end = payload_start + mini_header.compressed_size as usize;

            if payload_end > compressed.len() {
                return Err(NexusError::DataCorruption(
                    "Payload boundary exceeds file size".into(),
                ));
            }

            let payload = &compressed[payload_start..payload_end];
            decompress_sub_block(&mini_header, payload)
        })
        .collect();

    let blocks = decompressed_blocks?;
    let mut decompressed = Vec::with_capacity(header.original_size as usize);
    for block in blocks {
        decompressed.extend_from_slice(&block);
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
    fn test_pipeline_text_path_b() {
        let input = b"Burrows-Wheeler Transform combined with Move-to-Front and rANS achieves exceptional text compression ratios.";
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
    fn test_pipeline_binary_path_a() {
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
