//! End-to-end compression and decompression pipeline.
//!
//! Integrates Layer 0 (framing & parallel superblocks), Layer 1 (data detection),
//! Layer 2 (bijective MTF, RLE, Delta, x86 filter, Float Split), Layer 3 (Path A: LZ-ANS, Path B: BWT-ANS),
//! and Layer 4 (.nxs container assembly).

use crate::block::detector::detect_block_type;
use crate::block::splitter::{split_into_superblocks, SubBlock, DEFAULT_SUPERBLOCK_SIZE};
use crate::compress::bwt::{bwt_decode, bwt_encode};
use crate::compress::lz77::{
    deserialize_tokens, serialize_tokens, Lz77Decoder, Lz77Encoder, Lz77Token,
};
use crate::compress::mixer::{context_mixing_compress, context_mixing_decompress};
use crate::entropy::rans::{RansDecoder, RansEncoder};
use crate::error::{NexusError, Result};
use crate::format::{
    BlockTableEntry, BlockType, CompressionPath, FileHeader, Footer, PreprocessFlags,
    SubBlockHeader, FILE_HEADER_SIZE, FOOTER_SIZE, MINI_HEADER_SIZE,
};
use crate::preprocess::delta::{delta_decode, delta_encode};
use crate::preprocess::float_split::{float_split_decode, float_split_encode};
use crate::preprocess::mtf::{mtf_decode, mtf_encode};
use crate::preprocess::rle::{zero_rle_decode, zero_rle_encode};
use crate::preprocess::x86::{x86_filter_decode, x86_filter_encode};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use rayon::prelude::*;
use xxhash_rust::xxh32::xxh32;
use xxhash_rust::xxh64::xxh64;

const PILOT_SAMPLE_SIZE: usize = 16 * 1024;

/// Selects between Path A and Path B using a 16KB center pilot sample (ADR-005).
pub fn pilot_sample_path(data: &[u8]) -> CompressionPath {
    let sample = if data.len() <= PILOT_SAMPLE_SIZE {
        data
    } else {
        let center = data.len() / 2;
        let start = center.saturating_sub(PILOT_SAMPLE_SIZE / 2);
        let end = std::cmp::min(start + PILOT_SAMPLE_SIZE, data.len());
        &data[start..end]
    };

    // 1. Post-MTF zero-run density
    let (bwt_sample, _) = bwt_encode(sample);
    let mtf_sample = mtf_encode(&bwt_sample);
    let zero_count = mtf_sample.iter().filter(|&&b| b == 0).count();
    let zero_density = zero_count as f64 / mtf_sample.len() as f64;

    // 2. LZ77 match ratio
    let lz_encoder = Lz77Encoder::new(16 * 1024);
    let tokens = lz_encoder.encode(sample);
    let matched_bytes: usize = tokens
        .iter()
        .map(|t| match t {
            Lz77Token::Match { length, .. } => *length as usize,
            _ => 0,
        })
        .sum();
    let match_ratio = matched_bytes as f64 / sample.len() as f64;

    if zero_density > 0.40 && match_ratio <= 0.60 {
        CompressionPath::PathB
    } else if match_ratio > 0.60 && zero_density <= 0.40 {
        CompressionPath::PathA
    } else if zero_density > 0.35 {
        CompressionPath::PathB
    } else {
        CompressionPath::PathA
    }
}

/// Compresses a single sub-block, choosing the best preprocessing and compression path.
fn compress_sub_block(
    sub_block: &SubBlock,
    level: u32,
    history: &[u8],
) -> (SubBlockHeader, Vec<u8>, Vec<u8>) {
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
        return (mini_header, Vec::new(), Vec::new());
    }

    if block_type == BlockType::Incompressible {
        let mini_header = SubBlockHeader::new(
            orig_len as u32,
            orig_len as u32,
            block_type,
            CompressionPath::PathE,
            PreprocessFlags::empty(),
            block_checksum,
        );
        return (mini_header, sub_block.data.clone(), sub_block.data.clone());
    }

    // Layer 2: Preprocessing
    let mut preprocessed = sub_block.data.clone();
    let mut flags = PreprocessFlags::empty();

    match block_type {
        BlockType::Executable => {
            preprocessed = x86_filter_encode(&sub_block.data);
            flags.insert(PreprocessFlags::X86_FILTER);
        }
        BlockType::FloatingPoint => {
            preprocessed = float_split_encode(&sub_block.data, 4);
            flags.insert(PreprocessFlags::FLOAT_SPLIT);
        }
        BlockType::Audio => {
            preprocessed = delta_encode(&sub_block.data, 2);
            flags.insert(PreprocessFlags::DELTA);
        }
        _ => {}
    }

    let rans_encoder = RansEncoder::new();

    // Layer 3: Algorithm selection
    let can_use_path_b = orig_len <= 131_072;
    let chosen_path = if (13..=16).contains(&level) {
        CompressionPath::PathC
    } else if can_use_path_b {
        match block_type {
            BlockType::Text | BlockType::SourceCode => {
                if level <= 6 {
                    CompressionPath::PathB
                } else {
                    pilot_sample_path(&preprocessed)
                }
            }
            BlockType::StructuredData if level >= 7 => pilot_sample_path(&preprocessed),
            _ => CompressionPath::PathA,
        }
    } else {
        CompressionPath::PathA
    };

    // Execute chosen path
    let (best_path, best_payload, best_flags) = match chosen_path {
        CompressionPath::PathB => {
            let (bwt_bytes, primary_index) = bwt_encode(&preprocessed);
            let mtf_bytes = mtf_encode(&bwt_bytes);
            let rle_bytes = zero_rle_encode(&mtf_bytes);
            let rans_encoded = rans_encoder.encode(&rle_bytes);

            let mut payload = Vec::with_capacity(rans_encoded.len() + 4);
            payload.write_u32::<LittleEndian>(primary_index).unwrap();
            payload.extend_from_slice(&rans_encoded);

            let mut b_flags = flags;
            b_flags.insert(PreprocessFlags::MTF);
            b_flags.insert(PreprocessFlags::RLE);

            if payload.len() < orig_len {
                (CompressionPath::PathB, payload, b_flags)
            } else {
                // Try Path A as fallback
                let lz77_encoder = Lz77Encoder::default();
                let tokens = lz77_encoder.encode_with_history(&preprocessed, history);
                let serialized = serialize_tokens(&tokens);
                let a_encoded = rans_encoder.encode(&serialized);
                if a_encoded.len() < orig_len {
                    (CompressionPath::PathA, a_encoded, flags)
                } else {
                    (
                        CompressionPath::PathE,
                        sub_block.data.clone(),
                        PreprocessFlags::empty(),
                    )
                }
            }
        }
        CompressionPath::PathA => {
            let lz77_encoder = Lz77Encoder::default();
            let tokens = lz77_encoder.encode_with_history(&preprocessed, history);
            let serialized = serialize_tokens(&tokens);
            let rans_encoded = rans_encoder.encode(&serialized);

            if rans_encoded.len() < orig_len {
                (CompressionPath::PathA, rans_encoded, flags)
            } else {
                (
                    CompressionPath::PathE,
                    sub_block.data.clone(),
                    PreprocessFlags::empty(),
                )
            }
        }
        CompressionPath::PathC => {
            let payload = context_mixing_compress(&preprocessed);
            if payload.len() < orig_len {
                (CompressionPath::PathC, payload, flags)
            } else {
                let lz77_encoder = Lz77Encoder::default();
                let tokens = lz77_encoder.encode_with_history(&preprocessed, history);
                let serialized = serialize_tokens(&tokens);
                let a_encoded = rans_encoder.encode(&serialized);
                if a_encoded.len() < orig_len {
                    (CompressionPath::PathA, a_encoded, flags)
                } else {
                    (
                        CompressionPath::PathE,
                        sub_block.data.clone(),
                        PreprocessFlags::empty(),
                    )
                }
            }
        }
        _ => (
            CompressionPath::PathE,
            sub_block.data.clone(),
            PreprocessFlags::empty(),
        ),
    };

    let mini_header = SubBlockHeader::new(
        best_payload.len() as u32,
        orig_len as u32,
        block_type,
        best_path,
        best_flags,
        block_checksum,
    );

    let history_bytes = if best_path == CompressionPath::PathE {
        sub_block.data.clone()
    } else {
        preprocessed
    };

    (mini_header, best_payload, history_bytes)
}

/// Decompresses a single sub-block given its mini-header, compressed payload, and warm LZ77 history.
fn decompress_sub_block(
    mini_header: &SubBlockHeader,
    payload: &[u8],
    history: &[u8],
) -> Result<(Vec<u8>, Vec<u8>)> {
    if mini_header.original_size == 0 {
        return Ok((Vec::new(), Vec::new()));
    }

    let rans_decoder = RansDecoder::new();
    let lz77_decoder = Lz77Decoder::new();

    let preprocessed = match mini_header.compression_path {
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
            lz77_decoder.decode_with_history(&tokens, history)?
        }
        CompressionPath::PathC => {
            context_mixing_decompress(payload, mini_header.original_size as usize)?
        }
        CompressionPath::PathE => payload.to_vec(),
        other => return Err(NexusError::UnsupportedPath(other.to_u8())),
    };

    // Layer 2: Inverse Preprocessing
    let mut raw_bytes = preprocessed.clone();
    let flags = mini_header.preprocess_flags;

    if flags.contains(PreprocessFlags::FLOAT_SPLIT) {
        raw_bytes = float_split_decode(&raw_bytes, 4);
    }
    if flags.contains(PreprocessFlags::DELTA) {
        raw_bytes = delta_decode(&raw_bytes, 2);
    }
    if flags.contains(PreprocessFlags::X86_FILTER) {
        raw_bytes = x86_filter_decode(&raw_bytes);
    }

    // Verify sub-block checksum
    let computed_checksum = xxh32(&raw_bytes, 0);
    if computed_checksum != mini_header.block_checksum {
        return Err(NexusError::BlockChecksumMismatch {
            expected: mini_header.block_checksum,
            actual: computed_checksum,
        });
    }

    if raw_bytes.len() != mini_header.original_size as usize {
        return Err(NexusError::DataCorruption(
            "Sub-block uncompressed size does not match mini-header original size".into(),
        ));
    }

    Ok((raw_bytes, preprocessed))
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
    let mut compressed_superblocks: Vec<SuperblockResult> = superblocks
        .into_par_iter()
        .map(|sb| {
            let mut history = Vec::new();
            let mut compressed_blocks = Vec::with_capacity(sb.sub_blocks.len());
            for sub_block in &sb.sub_blocks {
                let (mini_header, payload, preprocessed) =
                    compress_sub_block(sub_block, level, &history);
                history.extend_from_slice(&preprocessed);
                compressed_blocks.push((mini_header, payload));
            }
            (sb.id, compressed_blocks)
        })
        .collect();

    compressed_superblocks.sort_by_key(|sb| sb.0);

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
/// Validates footer, headers, checksums, and decodes sub-blocks in parallel across superblocks.
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

    // Group block table entries by superblock_id
    let mut superblock_entries: Vec<Vec<BlockTableEntry>> = Vec::new();
    for entry in entries {
        if superblock_entries.is_empty()
            || superblock_entries.last().unwrap()[0].superblock_id != entry.superblock_id
        {
            superblock_entries.push(vec![entry]);
        } else {
            superblock_entries.last_mut().unwrap().push(entry);
        }
    }

    // 4. Parallel superblock decompression using Rayon
    let decompressed_superblocks: Result<Vec<Vec<u8>>> = superblock_entries
        .into_par_iter()
        .map(|sb_entries| {
            let mut sb_bytes = Vec::new();
            let mut history = Vec::new();

            for entry in sb_entries {
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
                let (raw_bytes, preprocessed) =
                    decompress_sub_block(&mini_header, payload, &history)?;
                history.extend_from_slice(&preprocessed);
                sb_bytes.extend_from_slice(&raw_bytes);
            }

            Ok(sb_bytes)
        })
        .collect();

    let sb_results = decompressed_superblocks?;
    let mut decompressed = Vec::with_capacity(header.original_size as usize);
    for sb_data in sb_results {
        decompressed.extend_from_slice(&sb_data);
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

    #[test]
    fn test_pipeline_executable_x86() {
        let mut code = Vec::new();
        code.extend_from_slice(b"\x7FELF\x02\x01\x01\x00"); // ELF header to trigger Executable
        for i in 0..1000 {
            code.push(0xE8); // CALL
            let rel = (i * 16) as i32;
            code.extend_from_slice(&rel.to_le_bytes());
            code.push(0x90); // NOP
        }

        let compressed = compress(&code, 5).unwrap();
        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(decompressed, code);
    }

    #[test]
    fn test_pipeline_floating_point() {
        let mut floats = Vec::new();
        for i in 0..2000 {
            let val = (i as f32) * 1.5;
            floats.extend_from_slice(&val.to_le_bytes());
        }

        let compressed = compress(&floats, 5).unwrap();
        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(decompressed, floats);
    }

    #[test]
    fn test_pipeline_audio_pcm() {
        let mut audio = Vec::new();
        audio.extend_from_slice(b"RIFF");
        audio.extend_from_slice(&[0u8; 4]);
        audio.extend_from_slice(b"WAVE");
        for i in 0..2000 {
            let sample = ((i as f64 * 0.1).sin() * 10000.0) as i16;
            audio.extend_from_slice(&sample.to_le_bytes());
        }

        let compressed = compress(&audio, 5).unwrap();
        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(decompressed, audio);
    }

    #[test]
    fn test_pipeline_pilot_sampling() {
        let text = b"Text that benefits strongly from Burrows-Wheeler Transform and Move-to-Front clustering. ".repeat(200);
        let path = pilot_sample_path(&text);
        assert_eq!(path, CompressionPath::PathB);
    }

    #[test]
    fn test_pipeline_context_mixing_level_13() {
        let input =
            b"Extreme compression powered by Context Mixing and Arithmetic Coding in NEXUS!";
        let compressed = compress(input, 13).unwrap();
        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(decompressed, input);
    }

    #[test]
    fn test_pipeline_context_mixing_repetitive_level_14() {
        let input = b"The quick brown fox jumps over the lazy dog. ".repeat(10);
        let compressed = compress(&input, 14).unwrap();
        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(decompressed, input);
    }
}
