//! End-to-end compression and decompression pipeline.
//!
//! Integrates Layer 0 (framing & parallel superblocks), Layer 1 (data detection),
//! Layer 2 (bijective MTF, RLE, Delta, x86 filter, Float Split), Layer 3 (Path A: LZ-ANS, Path B: BWT-ANS),
//! and Layer 4 (.nxs container assembly).

use crate::block::detector::{classify_block, compute_shannon_entropy, detect_block_type};
use crate::block::splitter::plan_subblocks_with_target;
use crate::compress::bwt::{bwt_decode, bwt_encode};
use crate::compress::bwt_cm::{bwt_cm_compress, bwt_cm_decompress};
use crate::compress::lz::{
    decode_block, params_for_level, subblock_target_for_level, superblock_size_for_level,
    LzDecodeScratch, LzEncoder,
};
use crate::compress::lz77::{deserialize_tokens, Lz77Decoder, Lz77Encoder, Lz77Token};
use crate::compress::mixer::{context_mixing_compress, context_mixing_decompress};
use crate::compress::neural::{neural_compress, neural_decompress};
use crate::entropy::rans::RansDecoder;
use crate::error::{NexusError, Result};
use crate::format::{
    BlockTableEntry, BlockType, CompressionPath, FileHeader, Footer, PreprocessFlags,
    SubBlockHeader, BLOCK_TABLE_ENTRY_SIZE, FILE_HEADER_SIZE, FOOTER_SIZE, MINI_HEADER_SIZE,
    PATH_COUNT,
};
use crate::preprocess::delta::{delta_decode, delta_encode};
use crate::preprocess::float_split::{float_split_decode, float_split_encode};
use crate::preprocess::mtf::{mtf_decode, mtf_encode};
use crate::preprocess::rle::zero_rle_decode;
use crate::preprocess::x86::{
    looks_like_x86_code, x86_e8e9_decode, x86_e8e9_encode, x86_filter_decode,
};
use byteorder::{LittleEndian, ReadBytesExt};
use rayon::prelude::*;
use std::sync::atomic::{AtomicU64, Ordering};
use xxhash_rust::xxh32::xxh32;
use xxhash_rust::xxh64::xxh64;

/// Curated compression presets offering intuitive tradeoffs between speed and ratio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompressionPreset {
    /// Fast mode: Level 2, single-probe LZ, highest throughput.
    Low,
    /// Balanced mode: Level 6, lazy LZ over a 4 MB window.
    #[default]
    Medium,
    /// Maximum mode: Level 11, optimal-parse LZ over a 16 MB window.
    High,
}

impl CompressionPreset {
    /// Maps preset to corresponding pipeline compression level.
    pub fn to_level(self) -> u32 {
        match self {
            Self::Low => 2,
            Self::Medium => 6,
            Self::High => 11,
        }
    }
}

/// Real-time progress update event emitted during compression or decompression.
#[derive(Debug, Clone)]
pub struct ProgressUpdate {
    /// Bytes processed so far
    pub bytes_processed: u64,
    /// Total bytes expected
    pub total_bytes: u64,
    /// Archive bytes produced (compression) or consumed (extraction) so far
    pub compressed_bytes: u64,
    /// Original bytes handled so far by each compression path, indexed by
    /// `CompressionPath::to_u8()`
    pub path_bytes: [u64; PATH_COUNT],
    /// Name or relative path of current target item
    pub current_target: String,
    /// Current pipeline stage description
    pub stage: &'static str,
    /// Completion fraction between 0.0 and 1.0
    pub fraction: f32,
}

/// Thread-safe callback hook for progress updates.
pub type ProgressCallback = std::sync::Arc<dyn Fn(ProgressUpdate) + Send + Sync + 'static>;

/// Optional hooks for a long-running file operation.
#[derive(Clone, Default)]
pub struct JobControl {
    /// Called after every finished superblock (from worker threads) and once
    /// more with stage `"Complete"`.
    pub progress: Option<ProgressCallback>,
    /// Set to `true` to abort the operation. It stops at the next superblock
    /// boundary and returns [`NexusError::Cancelled`].
    pub cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

impl JobControl {
    /// Control block with only a progress callback.
    pub fn with_progress(progress: Option<ProgressCallback>) -> Self {
        Self {
            progress,
            cancel: None,
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(|c| c.load(Ordering::Relaxed))
    }
}

/// Estimated compression metrics based on fast sample analysis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompressionEstimate {
    /// Approximate output size in bytes
    pub estimated_bytes: u64,
    /// Estimated compression ratio (compressed / original * 100.0)
    pub ratio_percent: f64,
    /// Estimated space savings percentage (100.0 - ratio)
    pub savings_percent: f64,
    /// Primary detected data type
    pub detected_type: BlockType,
    /// Whether data is deemed largely incompressible
    pub is_incompressible: bool,
}

/// Computes a fast, lightweight compression estimate for an input of `total_bytes`
/// using a representative `sample` (typically 16KB - 64KB) at a given `level` (1 to 19).
pub fn estimate_compression(sample: &[u8], total_bytes: u64, level: u32) -> CompressionEstimate {
    if sample.is_empty() || total_bytes == 0 {
        return CompressionEstimate {
            estimated_bytes: 0,
            ratio_percent: 100.0,
            savings_percent: 0.0,
            detected_type: BlockType::Binary,
            is_incompressible: false,
        };
    }

    let detected_type = detect_block_type(sample);
    let entropy = compute_shannon_entropy(sample);

    if detected_type == BlockType::Incompressible || entropy >= 7.85 {
        let est_bytes = (total_bytes as f64 * 0.995) as u64 + 64;
        return CompressionEstimate {
            estimated_bytes: est_bytes.min(total_bytes + 64),
            ratio_percent: 99.5,
            savings_percent: 0.5,
            detected_type,
            is_incompressible: true,
        };
    }

    // Trial-compress the sample with the real codec. Levels above 12 add
    // modelling paths that are only kept when they beat LZ, so the level-12
    // result is a safe upper bound for them.
    let test_slice = &sample[..sample.len().min(64 * 1024)];
    let payload_bytes: usize = compress_superblock(test_slice, level.clamp(1, 12))
        .iter()
        .map(|(_, payload)| payload.len() + MINI_HEADER_SIZE)
        .sum();
    let scaled_ratio = (payload_bytes as f64 / test_slice.len() as f64).clamp(0.01, 1.0);

    let frame_overhead = 64u64;
    let est_bytes = ((total_bytes as f64 * scaled_ratio) as u64).saturating_add(frame_overhead);
    let final_bytes = est_bytes.min(total_bytes);
    let ratio_percent = if total_bytes > 0 {
        (final_bytes as f64 / total_bytes as f64 * 100.0).clamp(0.0, 100.0)
    } else {
        100.0
    };
    let savings_percent = (100.0 - ratio_percent).max(0.0);

    CompressionEstimate {
        estimated_bytes: final_bytes,
        ratio_percent,
        savings_percent,
        detected_type,
        is_incompressible: false,
    }
}

/// Reads a representative sample (up to `max_bytes`) and total size from a file or directory.
pub fn sample_path<P: AsRef<std::path::Path>>(
    path: P,
    max_bytes: usize,
) -> std::io::Result<(Vec<u8>, u64, bool)> {
    use std::fs::File;
    use std::io::Read;

    let p = path.as_ref();
    if p.is_dir() {
        let (manifest, _) =
            crate::vfs::crawl_directory(p).map_err(|e| std::io::Error::other(e.to_string()))?;
        let total_size = manifest.total_uncompressed_bytes();
        let mut sample_buf = Vec::with_capacity(max_bytes);
        for entry in &manifest.entries {
            if sample_buf.len() >= max_bytes {
                break;
            }
            let full_path = p.join(&entry.path);
            if full_path.is_file() {
                if let Ok(mut f) = File::open(&full_path) {
                    let to_read = max_bytes - sample_buf.len();
                    let mut chunk = vec![0u8; to_read.min(entry.file_size as usize)];
                    if let Ok(n) = f.read(&mut chunk) {
                        chunk.truncate(n);
                        sample_buf.extend_from_slice(&chunk);
                    }
                }
            }
        }
        Ok((sample_buf, total_size, true))
    } else {
        let mut file = File::open(p)?;
        let total_size = file.metadata()?.len();
        let mut buf = vec![0u8; max_bytes.min(total_size as usize)];
        let n = file.read(&mut buf)?;
        buf.truncate(n);
        Ok((buf, total_size, false))
    }
}

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

/// One compressed sub-block: its mini-header and payload.
type CompressedBlock = (SubBlockHeader, Vec<u8>);

/// Largest sub-block the decoder will allocate for; guards against corrupt
/// mini-headers requesting absurd sizes.
const MAX_DECODED_SUBBLOCK: usize = 64 * 1024 * 1024;

/// Applies the Layer 2 transform suited to `block_type`, appending the result
/// to `pre`. Returns the flags describing what was applied.
///
/// Transforms are only kept when they preserve length, so the preprocessed
/// domain and the raw domain always share sub-block boundaries.
fn preprocess_into(raw: &[u8], block_type: BlockType, pre: &mut Vec<u8>) -> PreprocessFlags {
    let start = pre.len();
    let mut flags = PreprocessFlags::empty();
    let transformed = match block_type {
        BlockType::Executable | BlockType::Binary => {
            pre.extend_from_slice(raw);
            if looks_like_x86_code(raw) {
                x86_e8e9_encode(&mut pre[start..], start as u32);
                flags.insert(PreprocessFlags::X86_E8E9);
            }
            return flags;
        }
        BlockType::FloatingPoint => {
            flags.insert(PreprocessFlags::FLOAT_SPLIT);
            float_split_encode(raw, 4)
        }
        BlockType::Audio => {
            flags.insert(PreprocessFlags::DELTA);
            delta_encode(raw, 2)
        }
        _ => {
            pre.extend_from_slice(raw);
            return flags;
        }
    };
    if transformed.len() == raw.len() {
        pre.extend_from_slice(&transformed);
        flags
    } else {
        pre.extend_from_slice(raw);
        PreprocessFlags::empty()
    }
}

/// Compresses one superblock into its sequence of sub-blocks.
///
/// The superblock is independent of all others (parallel unit). Within it,
/// one LZ match finder spans every sub-block so the window stays warm, while
/// entropy tables reset per sub-block.
fn compress_superblock(data: &[u8], level: u32) -> Vec<CompressedBlock> {
    if data.is_empty() {
        let header = SubBlockHeader::new(
            0,
            0,
            BlockType::Text,
            CompressionPath::PathE,
            PreprocessFlags::empty(),
            xxh32(&[], 0),
        );
        return vec![(header, Vec::new())];
    }

    let plans = plan_subblocks_with_target(data, subblock_target_for_level(level));

    // Phase 1: classify and preprocess every sub-block, building the
    // preprocessed-domain image of the whole superblock (the LZ window).
    // Doing this up front lets the match finder see the bytes that follow
    // the sub-block it is working on.
    struct Prepared {
        range: std::ops::Range<usize>,
        block_type: BlockType,
        flags: PreprocessFlags,
        checksum: u32,
    }
    let mut pre: Vec<u8> = Vec::with_capacity(data.len());
    let mut prepared = Vec::with_capacity(plans.len());
    for plan in plans {
        let raw = &data[plan.range.clone()];
        let block_type = classify_block(raw, &plan.hist);
        let flags = if block_type == BlockType::Incompressible {
            pre.extend_from_slice(raw);
            PreprocessFlags::empty()
        } else {
            preprocess_into(raw, block_type, &mut pre)
        };
        prepared.push(Prepared {
            range: plan.range,
            block_type,
            flags,
            checksum: xxh32(raw, 0),
        });
    }
    debug_assert_eq!(pre.len(), data.len());

    // Phase 2: compress the sub-blocks in order.
    let mut lz = LzEncoder::new(params_for_level(level), data.len());
    let mut blocks = Vec::with_capacity(prepared.len());
    for block in prepared {
        let (start, end) = (block.range.start, block.range.end);
        let len = end - start;

        if block.block_type == BlockType::Incompressible {
            lz.skip_block(end);
            let header = SubBlockHeader::new(
                len as u32,
                len as u32,
                block.block_type,
                CompressionPath::PathE,
                PreprocessFlags::empty(),
                block.checksum,
            );
            blocks.push((header, pre[start..end].to_vec()));
            continue;
        }

        let mut payload = Vec::with_capacity(len / 2 + 64);
        lz.compress_block(&pre, start, end, &mut payload);
        let mut path = CompressionPath::PathA2;

        // Modelling paths compete with the LZ result; the smaller one wins,
        // so a higher level can never produce a larger block.
        if level >= 13 {
            if let Some(alt) = bwt_cm_compress(&pre[start..end]) {
                if alt.len() < payload.len() {
                    payload = alt;
                    path = CompressionPath::PathB2;
                }
            }
        }
        if level >= 17 {
            let alt = neural_compress(&pre[start..end]);
            if alt.len() < payload.len() {
                payload = alt;
                path = CompressionPath::PathD;
            }
        } else if level == 16 {
            let alt = context_mixing_compress(&pre[start..end]);
            if alt.len() < payload.len() {
                payload = alt;
                path = CompressionPath::PathC;
            }
        }

        if payload.len() >= len {
            // Store the preprocessed bytes so the window stays consistent.
            payload.clear();
            payload.extend_from_slice(&pre[start..end]);
            path = CompressionPath::PathE;
        }

        let header = SubBlockHeader::new(
            payload.len() as u32,
            len as u32,
            block.block_type,
            path,
            block.flags,
            block.checksum,
        );
        blocks.push((header, payload));
    }
    blocks
}

/// Decompresses all sub-blocks of one superblock, verifying each block
/// checksum, and returns the original bytes.
///
/// Handles both format v2 (Path A2) and legacy format v1 blocks.
fn decompress_superblock<'a, I>(blocks: I) -> Result<Vec<u8>>
where
    I: IntoIterator<Item = (SubBlockHeader, &'a [u8])>,
{
    // Preprocessed-domain image (the LZ window) and, once a block needed an
    // inverse transform, the separate raw-domain output.
    let mut pre: Vec<u8> = Vec::new();
    let mut out: Option<Vec<u8>> = None;
    let mut scratch = LzDecodeScratch::default();

    for (header, payload) in blocks {
        let start = pre.len();
        let n = header.original_size as usize;
        if n > MAX_DECODED_SUBBLOCK {
            return Err(NexusError::DataCorruption(
                "Sub-block original size exceeds format limit".into(),
            ));
        }
        if n == 0 {
            continue;
        }

        match header.compression_path {
            CompressionPath::PathA2 => decode_block(payload, &mut pre, n, &mut scratch)?,
            CompressionPath::PathB2 => {
                pre.extend_from_slice(&bwt_cm_decompress(payload, n)?);
            }
            CompressionPath::PathA => {
                let serialized = RansDecoder::new().decode(payload)?;
                let tokens = deserialize_tokens(&serialized)?;
                let decoded = Lz77Decoder::new().decode_with_history(&tokens, &pre)?;
                pre.extend_from_slice(&decoded);
            }
            CompressionPath::PathB => {
                if payload.len() < 4 {
                    return Err(NexusError::DataCorruption(
                        "Path B payload too short for BWT index".into(),
                    ));
                }
                let mut cursor = payload;
                let primary_index = cursor.read_u32::<LittleEndian>()?;
                let rle_bytes = RansDecoder::new().decode(cursor)?;
                let mtf_bytes = zero_rle_decode(&rle_bytes)?;
                let bwt_bytes = mtf_decode(&mtf_bytes);
                pre.extend_from_slice(&bwt_decode(&bwt_bytes, primary_index)?);
            }
            CompressionPath::PathC => {
                pre.extend_from_slice(&context_mixing_decompress(payload, n)?);
            }
            CompressionPath::PathD => {
                pre.extend_from_slice(&neural_decompress(payload, n)?);
            }
            CompressionPath::PathE => pre.extend_from_slice(payload),
        }

        let flags = header.preprocess_flags;
        let needs_inverse = flags.contains(PreprocessFlags::FLOAT_SPLIT)
            || flags.contains(PreprocessFlags::DELTA)
            || flags.contains(PreprocessFlags::X86_FILTER)
            || flags.contains(PreprocessFlags::X86_E8E9);

        let restored;
        let raw: &[u8] = if needs_inverse {
            let mut bytes = pre[start..].to_vec();
            if flags.contains(PreprocessFlags::FLOAT_SPLIT) {
                bytes = float_split_decode(&bytes, 4);
            }
            if flags.contains(PreprocessFlags::DELTA) {
                bytes = delta_decode(&bytes, 2);
            }
            if flags.contains(PreprocessFlags::X86_FILTER) {
                bytes = x86_filter_decode(&bytes);
            }
            if flags.contains(PreprocessFlags::X86_E8E9) {
                x86_e8e9_decode(&mut bytes, start as u32);
            }
            restored = bytes;
            &restored
        } else {
            &pre[start..]
        };

        if raw.len() != n {
            return Err(NexusError::DataCorruption(
                "Sub-block uncompressed size does not match mini-header original size".into(),
            ));
        }
        let computed = xxh32(raw, 0);
        if computed != header.block_checksum {
            return Err(NexusError::BlockChecksumMismatch {
                expected: header.block_checksum,
                actual: computed,
            });
        }

        if needs_inverse {
            out.get_or_insert_with(|| pre[..start].to_vec())
                .extend_from_slice(raw);
        } else if let Some(o) = out.as_mut() {
            o.extend_from_slice(raw);
        }
    }

    Ok(out.unwrap_or(pre))
}

/// Groups block-table entries into runs sharing a superblock id.
fn group_by_superblock(entries: Vec<BlockTableEntry>) -> Vec<Vec<BlockTableEntry>> {
    let mut groups: Vec<Vec<BlockTableEntry>> = Vec::new();
    for entry in entries {
        match groups.last_mut() {
            Some(g) if g[0].superblock_id == entry.superblock_id => g.push(entry),
            _ => groups.push(vec![entry]),
        }
    }
    groups
}

/// Compresses a raw byte stream into the .nxs container format.
///
/// Uses multi-core parallel superblock compression via Rayon.
///
/// # Arguments
/// * `input` - Raw uncompressed bytes
/// * `level` - Compression level (1-19)
pub fn compress(input: &[u8], level: u32) -> Result<Vec<u8>> {
    let sb_size = superblock_size_for_level(level);
    let chunks: Vec<&[u8]> = if input.is_empty() {
        vec![input]
    } else {
        input.chunks(sb_size).collect()
    };
    let mut output = Vec::with_capacity(input.len() / 2 + 256);

    // 1. Write file header
    let header = FileHeader::new(input.len() as u64, chunks.len() as u32);
    header.write_to(&mut output)?;

    // 2. Parallel compression of superblocks via Rayon (order-preserving)
    let compressed_superblocks: Vec<Vec<CompressedBlock>> = chunks
        .into_par_iter()
        .map(|chunk| compress_superblock(chunk, level))
        .collect();

    // 3. Sequential container assembly
    let mut block_table = Vec::new();
    for (sb_id, blocks) in compressed_superblocks.into_iter().enumerate() {
        for (mini_header, payload) in blocks {
            let file_offset = output.len() as u64;
            mini_header.write_to(&mut output)?;
            output.extend_from_slice(&payload);
            block_table.push(BlockTableEntry {
                superblock_id: sb_id as u32,
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
    let superblock_entries = group_by_superblock(entries);

    // 4. Parallel superblock decompression using Rayon
    let decompressed_superblocks: Result<Vec<Vec<u8>>> = superblock_entries
        .into_par_iter()
        .map(|sb_entries| {
            let mut blocks = Vec::with_capacity(sb_entries.len());
            for entry in sb_entries {
                let offset = entry.file_offset as usize;
                let payload_start = offset
                    .checked_add(MINI_HEADER_SIZE)
                    .filter(|&p| p <= compressed.len())
                    .ok_or_else(|| {
                        NexusError::DataCorruption("Block offset exceeds file boundary".into())
                    })?;
                let mini_header = SubBlockHeader::read_from(&compressed[offset..payload_start])?;
                let payload_end = payload_start
                    .checked_add(mini_header.compressed_size as usize)
                    .filter(|&p| p <= compressed.len())
                    .ok_or_else(|| {
                        NexusError::DataCorruption("Payload boundary exceeds file size".into())
                    })?;
                blocks.push((mini_header, &compressed[payload_start..payload_end]));
            }
            decompress_superblock(blocks)
        })
        .collect();

    let mut sb_results = decompressed_superblocks?;
    let decompressed = if sb_results.len() == 1 {
        sb_results.pop().unwrap_or_default()
    } else {
        let mut all = Vec::with_capacity(header.original_size as usize);
        for sb_data in sb_results {
            all.extend_from_slice(&sb_data);
        }
        all
    };

    if decompressed.len() as u64 != header.original_size {
        return Err(NexusError::DataCorruption(format!(
            "Total uncompressed size mismatch: header says {}, got {}",
            header.original_size,
            decompressed.len()
        )));
    }

    Ok(decompressed)
}

/// Compresses an input file into an output .nxs file with bounded streaming memory.
///
/// Superblocks are read in batches of one per worker thread and compressed in
/// parallel, so memory use is bounded by `threads x superblock size` (plus the
/// match-finder tables of the chosen level) regardless of file size.
pub fn compress_file<P: AsRef<std::path::Path>, Q: AsRef<std::path::Path>>(
    input_path: P,
    output_path: Q,
    level: u32,
) -> Result<()> {
    compress_file_with_progress(input_path, output_path, level, None)
}

/// Shared counters behind the progress callbacks of one file operation.
struct ProgressTracker {
    callback: Option<ProgressCallback>,
    target: String,
    stage: &'static str,
    total: u64,
    processed: AtomicU64,
    compressed: AtomicU64,
    path_bytes: [AtomicU64; PATH_COUNT],
}

impl ProgressTracker {
    fn new(
        callback: Option<ProgressCallback>,
        target: String,
        stage: &'static str,
        total: u64,
    ) -> Self {
        Self {
            callback,
            target,
            stage,
            total,
            processed: AtomicU64::new(0),
            compressed: AtomicU64::new(0),
            path_bytes: Default::default(),
        }
    }

    /// Records one finished superblock and notifies the callback. Safe to
    /// call from worker threads.
    fn superblock_done<'a, I>(&self, headers: I)
    where
        I: IntoIterator<Item = &'a SubBlockHeader>,
    {
        let mut original = 0u64;
        let mut compressed = 0u64;
        for h in headers {
            original += h.original_size as u64;
            compressed += (h.compressed_size as usize + MINI_HEADER_SIZE) as u64;
            self.path_bytes[h.compression_path.to_u8() as usize % PATH_COUNT]
                .fetch_add(h.original_size as u64, Ordering::Relaxed);
        }
        self.processed.fetch_add(original, Ordering::Relaxed);
        self.compressed.fetch_add(compressed, Ordering::Relaxed);
        self.emit(self.stage);
    }

    fn emit(&self, stage: &'static str) {
        let Some(cb) = &self.callback else {
            return;
        };
        let done = stage == "Complete";
        let processed = if done {
            self.total
        } else {
            self.processed.load(Ordering::Relaxed).min(self.total)
        };
        let fraction = if done || self.total == 0 {
            1.0
        } else {
            (processed as f32 / self.total as f32).min(1.0)
        };
        let mut path_bytes = [0u64; PATH_COUNT];
        for (dst, src) in path_bytes.iter_mut().zip(&self.path_bytes) {
            *dst = src.load(Ordering::Relaxed);
        }
        cb(ProgressUpdate {
            bytes_processed: processed,
            total_bytes: self.total,
            compressed_bytes: self.compressed.load(Ordering::Relaxed),
            path_bytes,
            current_target: self.target.clone(),
            stage,
            fraction,
        });
    }
}

/// Streams superblocks in batches with real-time progress callbacks.
pub fn compress_file_with_progress<P: AsRef<std::path::Path>, Q: AsRef<std::path::Path>>(
    input_path: P,
    output_path: Q,
    level: u32,
    progress: Option<ProgressCallback>,
) -> Result<()> {
    compress_file_with_control(
        input_path,
        output_path,
        level,
        &JobControl::with_progress(progress),
    )
}

/// Compresses a file or directory with progress reporting and cancellation.
///
/// The archive is written to a temporary sibling file (`<output>.part`) and
/// renamed into place only once it is complete. On cancellation or error the
/// temporary file is removed, and an archive that already existed at
/// `output_path` is left untouched.
pub fn compress_file_with_control<P: AsRef<std::path::Path>, Q: AsRef<std::path::Path>>(
    input_path: P,
    output_path: Q,
    level: u32,
    control: &JobControl,
) -> Result<()> {
    let output_path = output_path.as_ref();
    let mut partial = output_path.as_os_str().to_owned();
    partial.push(".part");
    let partial = std::path::PathBuf::from(partial);

    let result = compress_file_impl(input_path.as_ref(), &partial, level, control)
        .and_then(|()| std::fs::rename(&partial, output_path).map_err(NexusError::from));
    if result.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    result
}

fn compress_file_impl(
    input_path: &std::path::Path,
    output_path: &std::path::Path,
    level: u32,
    control: &JobControl,
) -> Result<()> {
    use std::fs::File;
    use std::io::{BufWriter, Read, Write};
    use xxhash_rust::xxh64::Xxh64;

    let target_name = input_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let (mut input_reader, original_size): (Box<dyn Read>, u64) = if input_path.is_dir() {
        let vfs_reader = crate::vfs::pack::VfsDirectoryReader::open(input_path)?;
        let size = vfs_reader.total_stream_size();
        (Box::new(vfs_reader), size)
    } else {
        let file = File::open(input_path)?;
        let size = file.metadata()?.len();
        (Box::new(file), size)
    };

    let mut output_file = BufWriter::with_capacity(1 << 20, File::create(output_path)?);
    let mut file_hasher = Xxh64::new(0);

    let sb_size = superblock_size_for_level(level);
    let total_superblocks = if original_size == 0 {
        1
    } else {
        original_size.div_ceil(sb_size as u64).max(1) as u32
    };

    let header = FileHeader::new(original_size, total_superblocks);
    let mut header_buf = Vec::with_capacity(FILE_HEADER_SIZE);
    header.write_to(&mut header_buf)?;
    output_file.write_all(&header_buf)?;
    file_hasher.update(&header_buf);

    let tracker = ProgressTracker::new(
        control.progress.clone(),
        target_name,
        "Compressing",
        original_size,
    );
    let mut block_table: Vec<BlockTableEntry> = Vec::new();
    let mut current_offset = FILE_HEADER_SIZE as u64;
    let mut sb_id = 0u32;
    let batch_superblocks = rayon::current_num_threads().max(1);

    loop {
        if control.is_cancelled() {
            return Err(NexusError::Cancelled);
        }
        let mut batch_chunks: Vec<Vec<u8>> = Vec::with_capacity(batch_superblocks);
        for _ in 0..batch_superblocks {
            let mut chunk = vec![0u8; sb_size];
            let mut read_bytes = 0;
            while read_bytes < sb_size {
                let n = input_reader.read(&mut chunk[read_bytes..])?;
                if n == 0 {
                    break;
                }
                read_bytes += n;
            }
            if read_bytes == 0 {
                break;
            }
            chunk.truncate(read_bytes);
            batch_chunks.push(chunk);
            if read_bytes < sb_size {
                break;
            }
        }

        let mut last_batch = false;
        if batch_chunks.is_empty() {
            if sb_id > 0 {
                break;
            }
            // Empty input still gets one (empty) superblock.
            batch_chunks.push(Vec::new());
            last_batch = true;
        }

        let compressed_batch: Vec<Vec<CompressedBlock>> = batch_chunks
            .par_iter()
            .map(|chunk| {
                if control.is_cancelled() {
                    return Vec::new();
                }
                let blocks = compress_superblock(chunk, level);
                tracker.superblock_done(blocks.iter().map(|(h, _)| h));
                blocks
            })
            .collect();
        if control.is_cancelled() {
            return Err(NexusError::Cancelled);
        }

        for blocks in compressed_batch {
            let mut mini_buf = Vec::with_capacity(MINI_HEADER_SIZE);
            for (mini_header, payload) in blocks {
                mini_buf.clear();
                mini_header.write_to(&mut mini_buf)?;

                output_file.write_all(&mini_buf)?;
                file_hasher.update(&mini_buf);
                output_file.write_all(&payload)?;
                file_hasher.update(&payload);

                block_table.push(BlockTableEntry {
                    superblock_id: sb_id,
                    file_offset: current_offset,
                    compressed_size: payload.len() as u32,
                    original_size: mini_header.original_size,
                });
                current_offset += (mini_buf.len() + payload.len()) as u64;
            }
            sb_id += 1;
        }

        if last_batch {
            break;
        }
    }

    let block_table_offset = current_offset;
    let mut entry_buf = Vec::with_capacity(BLOCK_TABLE_ENTRY_SIZE);
    for entry in &block_table {
        entry_buf.clear();
        entry.write_to(&mut entry_buf)?;
        output_file.write_all(&entry_buf)?;
        file_hasher.update(&entry_buf);
    }

    let file_checksum = file_hasher.digest();
    let footer = Footer::new(block_table_offset, block_table.len() as u32, file_checksum);
    footer.write_to(&mut output_file)?;
    output_file.flush()?;

    tracker.emit("Complete");
    Ok(())
}

/// Decompresses an input .nxs or .nxar file into an output file with bounded streaming memory.
pub fn decompress_file<P: AsRef<std::path::Path>, Q: AsRef<std::path::Path>>(
    input_path: P,
    output_path: Q,
) -> Result<()> {
    decompress_file_with_progress(input_path, output_path, None)
}

/// Decompresses an input archive with real-time progress callbacks.
///
/// The archive is read once, front to back: every sub-block is verified
/// against its own checksum before its bytes are written, and the whole-file
/// checksum is accumulated on the way and checked at the end.
pub fn decompress_file_with_progress<P: AsRef<std::path::Path>, Q: AsRef<std::path::Path>>(
    input_path: P,
    output_path: Q,
    progress: Option<ProgressCallback>,
) -> Result<()> {
    decompress_file_with_control(
        input_path,
        output_path,
        &JobControl::with_progress(progress),
    )
}

/// Decompresses an archive with progress reporting and cancellation.
pub fn decompress_file_with_control<P: AsRef<std::path::Path>, Q: AsRef<std::path::Path>>(
    input_path: P,
    output_path: Q,
    control: &JobControl,
) -> Result<()> {
    let mut created_file = false;
    let result = decompress_file_impl(
        input_path.as_ref(),
        Some(output_path.as_ref()),
        control,
        &mut created_file,
    );
    if result.is_err() && created_file {
        // Only a file this call created is removed; a destination that was
        // never opened (for example because the archive header was invalid)
        // is left exactly as it was. Extracted directory trees are kept.
        let _ = std::fs::remove_file(output_path.as_ref());
    }
    result.map(|_| ())
}

/// Fully decodes an archive and verifies every block checksum and the file
/// checksum without writing any output. Returns the original (uncompressed)
/// size on success.
pub fn verify_file_with_control<P: AsRef<std::path::Path>>(
    input_path: P,
    control: &JobControl,
) -> Result<u64> {
    decompress_file_impl(input_path.as_ref(), None, control, &mut false)
}

/// Shared implementation of extraction (`output_path = Some`) and
/// verification (`None`). Returns the number of original bytes decoded.
/// `created_file` is set once a single-file output has been created.
fn decompress_file_impl(
    input_path: &std::path::Path,
    output_path: Option<&std::path::Path>,
    control: &JobControl,
    created_file: &mut bool,
) -> Result<u64> {
    use std::fs::File;
    use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
    use xxhash_rust::xxh64::Xxh64;

    let target_name = input_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let mut input_file = File::open(input_path)?;
    let file_len = input_file.metadata()?.len();

    if file_len < (FILE_HEADER_SIZE + FOOTER_SIZE) as u64 {
        return Err(NexusError::DataCorruption(
            "Input file too small to contain valid .nxs header and footer".into(),
        ));
    }

    // 1. Read and validate footer
    let footer_start = file_len - FOOTER_SIZE as u64;
    input_file.seek(SeekFrom::Start(footer_start))?;
    let mut footer_buf = [0u8; FOOTER_SIZE];
    input_file.read_exact(&mut footer_buf)?;
    let footer = Footer::read_from(&footer_buf[..])?;

    let table_len = footer.total_block_count as u64 * BLOCK_TABLE_ENTRY_SIZE as u64;
    if footer.block_table_offset < FILE_HEADER_SIZE as u64
        || footer.block_table_offset.checked_add(table_len) != Some(footer_start)
    {
        return Err(NexusError::DataCorruption(
            "Block table does not fit between payload and footer".into(),
        ));
    }

    // 2. Read block table entries
    input_file.seek(SeekFrom::Start(footer.block_table_offset))?;
    let mut table_bytes = vec![0u8; table_len as usize];
    input_file.read_exact(&mut table_bytes)?;
    let mut table_cursor = &table_bytes[..];
    let mut entries = Vec::with_capacity(footer.total_block_count as usize);
    for _ in 0..footer.total_block_count {
        entries.push(BlockTableEntry::read_from(&mut table_cursor)?);
    }
    let superblock_entries = group_by_superblock(entries);

    // 3. Read and validate file header; start the streaming file checksum
    input_file.seek(SeekFrom::Start(0))?;
    let mut header_buf = [0u8; FILE_HEADER_SIZE];
    input_file.read_exact(&mut header_buf)?;
    let header = FileHeader::read_from(&header_buf[..])?;
    let mut hasher = Xxh64::new(0);
    hasher.update(&header_buf);
    let mut expected_offset = FILE_HEADER_SIZE as u64;

    // 4. Stream decompress batch by batch directly to output file or VFS unpacker
    let mut total_written: u64 = 0;
    let batch_superblocks = rayon::current_num_threads().max(1);
    let stage = if output_path.is_some() {
        "Extracting"
    } else {
        "Verifying"
    };
    let tracker = ProgressTracker::new(
        control.progress.clone(),
        target_name,
        stage,
        header.original_size,
    );

    enum DecompressTarget {
        File(BufWriter<File>),
        Vfs(
            std::sync::mpsc::Sender<Vec<u8>>,
            std::thread::JoinHandle<Result<()>>,
        ),
    }

    struct ChannelReader {
        receiver: std::sync::mpsc::Receiver<Vec<u8>>,
        current: std::io::Cursor<Vec<u8>>,
    }

    impl Read for ChannelReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            loop {
                if self.current.position() < self.current.get_ref().len() as u64 {
                    let n = self.current.read(buf)?;
                    if n > 0 {
                        return Ok(n);
                    }
                }
                match self.receiver.recv() {
                    Ok(chunk) => {
                        self.current = std::io::Cursor::new(chunk);
                    }
                    Err(_) => return Ok(0),
                }
            }
        }
    }

    let mut target: Option<DecompressTarget> = None;

    for chunk_sb_entries in superblock_entries.chunks(batch_superblocks) {
        if control.is_cancelled() {
            return Err(NexusError::Cancelled);
        }
        // Each superblock occupies one contiguous span of the file: read it whole.
        let mut batch_spans: Vec<(Vec<u8>, &[BlockTableEntry])> =
            Vec::with_capacity(chunk_sb_entries.len());
        for sb_entries in chunk_sb_entries {
            let span_start = sb_entries[0].file_offset;
            let mut span_end = span_start;
            for entry in sb_entries {
                if entry.file_offset != span_end {
                    return Err(NexusError::DataCorruption(
                        "Block table entries are not contiguous".into(),
                    ));
                }
                span_end += MINI_HEADER_SIZE as u64 + entry.compressed_size as u64;
            }
            if span_start != expected_offset || span_end > footer.block_table_offset {
                return Err(NexusError::DataCorruption(
                    "Superblock span does not follow the previous one".into(),
                ));
            }
            let mut span = vec![0u8; (span_end - span_start) as usize];
            input_file.seek(SeekFrom::Start(span_start))?;
            input_file.read_exact(&mut span)?;
            hasher.update(&span);
            expected_offset = span_end;
            batch_spans.push((span, sb_entries.as_slice()));
        }

        let decompressed_batch: Result<Vec<Vec<u8>>> = batch_spans
            .par_iter()
            .map(|(span, sb_entries)| {
                let mut blocks = Vec::with_capacity(sb_entries.len());
                let mut pos = 0usize;
                for entry in sb_entries.iter() {
                    let mini_header =
                        SubBlockHeader::read_from(&span[pos..pos + MINI_HEADER_SIZE])?;
                    if mini_header.compressed_size != entry.compressed_size {
                        return Err(NexusError::DataCorruption(
                            "Mini-header size disagrees with block table".into(),
                        ));
                    }
                    let payload_start = pos + MINI_HEADER_SIZE;
                    let payload_end = payload_start + entry.compressed_size as usize;
                    blocks.push((mini_header, &span[payload_start..payload_end]));
                    pos = payload_end;
                }
                let bytes = decompress_superblock(blocks.iter().map(|(h, p)| (h.clone(), *p)))?;
                tracker.superblock_done(blocks.iter().map(|(h, _)| h));
                Ok(bytes)
            })
            .collect();

        let decompressed_batch = decompressed_batch?;

        if let (None, Some(output_path)) = (&target, output_path) {
            let is_vfs = decompressed_batch
                .first()
                .map(|chunk| crate::vfs::is_vfs_stream(chunk))
                .unwrap_or(false);

            if is_vfs {
                let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
                let dest_dir = output_path.to_path_buf();
                let handle = std::thread::spawn(move || {
                    let mut chan_reader = ChannelReader {
                        receiver: rx,
                        current: std::io::Cursor::new(Vec::new()),
                    };
                    crate::vfs::unpack_vfs_stream(&mut chan_reader, &dest_dir)?;
                    Ok(())
                });
                target = Some(DecompressTarget::Vfs(tx, handle));
            } else {
                let out_file = BufWriter::with_capacity(1 << 20, File::create(output_path)?);
                *created_file = true;
                target = Some(DecompressTarget::File(out_file));
            }
        }

        for sb_bytes in decompressed_batch {
            let sb_len = sb_bytes.len() as u64;
            match target.as_mut() {
                Some(DecompressTarget::File(f)) => {
                    f.write_all(&sb_bytes)?;
                }
                Some(DecompressTarget::Vfs(tx, _)) => {
                    tx.send(sb_bytes).map_err(|e| {
                        NexusError::DataCorruption(format!("VFS unpacker channel error: {}", e))
                    })?;
                }
                None => {}
            }
            total_written += sb_len;
        }
    }

    match target {
        Some(DecompressTarget::File(mut f)) => {
            f.flush()?;
        }
        Some(DecompressTarget::Vfs(tx, handle)) => {
            drop(tx);
            handle
                .join()
                .map_err(|_| NexusError::DataCorruption("VFS unpacker thread panicked".into()))??;
        }
        None => {
            // Archive of an empty input: still produce the (empty) output file.
            if let Some(output_path) = output_path {
                File::create(output_path)?;
            }
        }
    }

    if expected_offset != footer.block_table_offset {
        return Err(NexusError::DataCorruption(
            "Payload does not end at the block table".into(),
        ));
    }
    hasher.update(&table_bytes);
    let computed_checksum = hasher.digest();
    if computed_checksum != footer.file_checksum {
        return Err(NexusError::ChecksumMismatch {
            expected: footer.file_checksum,
            actual: computed_checksum,
        });
    }

    if total_written != header.original_size {
        return Err(NexusError::DataCorruption(format!(
            "Total restored size mismatch: header says {}, got {}",
            header.original_size, total_written
        )));
    }

    tracker.emit("Complete");
    Ok(total_written)
}

/// Information about an individual compressed block within an archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockInfo {
    pub superblock_id: u32,
    pub file_offset: u64,
    pub compressed_size: u32,
    pub original_size: u32,
    pub block_type: BlockType,
    pub compression_path: CompressionPath,
    pub preprocess_flags: PreprocessFlags,
    pub block_checksum: u32,
}

/// Metadata and block inventory for a .nxs archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveInfo {
    pub format_version: u16,
    pub superblock_count: u32,
    pub total_blocks: u32,
    pub total_original_size: u64,
    pub total_compressed_size: u64,
    pub file_checksum: u64,
    pub blocks: Vec<BlockInfo>,
}

/// Inspects a .nxs archive without decompressing payload blocks.
pub fn inspect_archive(compressed: &[u8]) -> Result<ArchiveInfo> {
    if compressed.len() < FILE_HEADER_SIZE + FOOTER_SIZE {
        return Err(NexusError::DataCorruption(
            "Input too short to contain valid .nxs header and footer".into(),
        ));
    }

    let footer_start = compressed.len() - FOOTER_SIZE;
    let footer = Footer::read_from(&compressed[footer_start..])?;

    let header = FileHeader::read_from(&compressed[..FILE_HEADER_SIZE])?;

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

    let mut blocks = Vec::with_capacity(entries.len());
    let mut total_original: u64 = 0;
    let mut total_compressed: u64 = 0;

    for entry in entries {
        let offset = entry.file_offset as usize;
        if offset + MINI_HEADER_SIZE > compressed.len() {
            return Err(NexusError::DataCorruption(
                "Block offset exceeds file boundary".into(),
            ));
        }

        let mini_header =
            SubBlockHeader::read_from(&compressed[offset..offset + MINI_HEADER_SIZE])?;
        total_original += entry.original_size as u64;
        total_compressed += entry.compressed_size as u64;

        blocks.push(BlockInfo {
            superblock_id: entry.superblock_id,
            file_offset: entry.file_offset,
            compressed_size: entry.compressed_size,
            original_size: entry.original_size,
            block_type: mini_header.block_type,
            compression_path: mini_header.compression_path,
            preprocess_flags: mini_header.preprocess_flags,
            block_checksum: mini_header.block_checksum,
        });
    }

    Ok(ArchiveInfo {
        format_version: header.format_version,
        superblock_count: header.superblock_count,
        total_blocks: footer.total_block_count,
        total_original_size: total_original,
        total_compressed_size: total_compressed,
        file_checksum: footer.file_checksum,
        blocks,
    })
}

/// Inspects a .nxs archive directly from disk without decompressing payload blocks.
pub fn inspect_archive_file<P: AsRef<std::path::Path>>(path: P) -> Result<ArchiveInfo> {
    use std::fs::File;
    use std::io::{Read, Seek, SeekFrom};

    let mut file = File::open(path.as_ref())?;
    let file_len = file.metadata()?.len();

    if file_len < (FILE_HEADER_SIZE + FOOTER_SIZE) as u64 {
        return Err(NexusError::DataCorruption(
            "Input file too small to contain valid .nxs header and footer".into(),
        ));
    }

    // Read footer
    file.seek(SeekFrom::End(-(FOOTER_SIZE as i64)))?;
    let mut footer_buf = [0u8; FOOTER_SIZE];
    file.read_exact(&mut footer_buf)?;
    let footer = Footer::read_from(&footer_buf[..])?;

    // Read header
    file.seek(SeekFrom::Start(0))?;
    let mut header_buf = [0u8; FILE_HEADER_SIZE];
    file.read_exact(&mut header_buf)?;
    let header = FileHeader::read_from(&header_buf[..])?;

    // Read block table
    file.seek(SeekFrom::Start(footer.block_table_offset))?;
    let mut entries = Vec::with_capacity(footer.total_block_count as usize);
    for _ in 0..footer.total_block_count {
        let mut entry_buf = [0u8; 20];
        file.read_exact(&mut entry_buf)?;
        entries.push(BlockTableEntry::read_from(&entry_buf[..])?);
    }

    let mut blocks = Vec::with_capacity(entries.len());
    let mut total_original: u64 = 0;
    let mut total_compressed: u64 = 0;

    for entry in entries {
        file.seek(SeekFrom::Start(entry.file_offset))?;
        let mut mini_buf = [0u8; MINI_HEADER_SIZE];
        file.read_exact(&mut mini_buf)?;
        let mini_header = SubBlockHeader::read_from(&mini_buf[..])?;

        total_original += entry.original_size as u64;
        total_compressed += entry.compressed_size as u64;

        blocks.push(BlockInfo {
            superblock_id: entry.superblock_id,
            file_offset: entry.file_offset,
            compressed_size: entry.compressed_size,
            original_size: entry.original_size,
            block_type: mini_header.block_type,
            compression_path: mini_header.compression_path,
            preprocess_flags: mini_header.preprocess_flags,
            block_checksum: mini_header.block_checksum,
        });
    }

    Ok(ArchiveInfo {
        format_version: header.format_version,
        superblock_count: header.superblock_count,
        total_blocks: footer.total_block_count,
        total_original_size: total_original,
        total_compressed_size: total_compressed,
        file_checksum: footer.file_checksum,
        blocks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipeline_inspect_archive() {
        let input =
            b"Inspection verification sample for Project NEXUS compression archive.".repeat(10);
        let compressed = compress(&input, 5).unwrap();
        let info = inspect_archive(&compressed).unwrap();

        assert_eq!(info.format_version, 2);
        assert_eq!(info.total_original_size, input.len() as u64);
        assert_eq!(info.blocks.len(), info.total_blocks as usize);
        assert!(!info.blocks.is_empty());
    }

    /// Builds a format-v1 archive by hand from the legacy encoders and checks
    /// that the current decoder still reads it, including an LZ block whose
    /// matches reach into the previous (BWT-coded) sub-block.
    #[test]
    fn test_pipeline_decodes_format_v1_archive() {
        use crate::compress::lz77::serialize_tokens;
        use crate::entropy::rans::RansEncoder;
        use crate::preprocess::rle::zero_rle_encode;
        use byteorder::WriteBytesExt;

        let part_b = b"Legacy path B block: Burrows-Wheeler, move-to-front, zero runs. ".repeat(40);
        let mut part_a =
            b"Legacy path A block reusing: Burrows-Wheeler, move-to-front. ".repeat(30);
        part_a.extend_from_slice(&part_b[..500]);
        let part_e: Vec<u8> = (0..300u32).map(|i| (i * 131 % 251) as u8).collect();

        // Path B payload: primary index + rANS(zero-RLE(MTF(BWT))).
        let (bwt, primary) = bwt_encode(&part_b);
        let rle = zero_rle_encode(&mtf_encode(&bwt));
        let mut payload_b = Vec::new();
        payload_b.write_u32::<LittleEndian>(primary).unwrap();
        payload_b.extend_from_slice(&RansEncoder::new().encode(&rle));

        // Path A payload: rANS(serialised tokens), with part_b as warm history.
        let tokens = Lz77Encoder::new(64 * 1024).encode_with_history(&part_a, &part_b);
        assert!(tokens.iter().any(|t| matches!(
            t,
            Lz77Token::Match { offset, .. } if *offset as usize > part_a.len()
        )));
        let payload_a = RansEncoder::new().encode(&serialize_tokens(&tokens));

        let mut mtf_flags = PreprocessFlags::empty();
        mtf_flags.insert(PreprocessFlags::MTF);
        mtf_flags.insert(PreprocessFlags::RLE);
        let blocks: Vec<(&[u8], Vec<u8>, CompressionPath, PreprocessFlags)> = vec![
            (&part_b, payload_b, CompressionPath::PathB, mtf_flags),
            (
                &part_a,
                payload_a,
                CompressionPath::PathA,
                PreprocessFlags::empty(),
            ),
            (
                &part_e,
                part_e.clone(),
                CompressionPath::PathE,
                PreprocessFlags::empty(),
            ),
        ];

        let total: usize = blocks.iter().map(|b| b.0.len()).sum();
        let mut header = FileHeader::new(total as u64, 1);
        header.format_version = 1;
        let mut file = Vec::new();
        header.write_to(&mut file).unwrap();
        let mut table = Vec::new();
        for (raw, payload, path, flags) in &blocks {
            let mini = SubBlockHeader::new(
                payload.len() as u32,
                raw.len() as u32,
                BlockType::Text,
                *path,
                *flags,
                xxh32(raw, 0),
            );
            table.push(BlockTableEntry {
                superblock_id: 0,
                file_offset: file.len() as u64,
                compressed_size: payload.len() as u32,
                original_size: raw.len() as u32,
            });
            mini.write_to(&mut file).unwrap();
            file.extend_from_slice(payload);
        }
        let table_offset = file.len() as u64;
        for entry in &table {
            entry.write_to(&mut file).unwrap();
        }
        let checksum = xxh64(&file, 0);
        Footer::new(table_offset, table.len() as u32, checksum)
            .write_to(&mut file)
            .unwrap();

        let mut expected = part_b.clone();
        expected.extend_from_slice(&part_a);
        expected.extend_from_slice(&part_e);
        assert_eq!(decompress(&file).unwrap(), expected);
        assert_eq!(inspect_archive(&file).unwrap().format_version, 1);
    }

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
        for i in 0..1000i32 {
            code.push(0xE8); // CALL
            let rel = i * 16;
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

    #[test]
    fn test_pipeline_neural_level_17() {
        let input = b"Extreme neural compression with fixed-point LeakyReLU and arithmetic coding in NEXUS!";
        let compressed = compress(input, 17).unwrap();
        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(decompressed, input);
    }

    #[test]
    fn test_pipeline_neural_repetitive_level_18() {
        let input = b"The quick brown fox jumps over the lazy dog. ".repeat(10);
        let compressed = compress(&input, 18).unwrap();
        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(decompressed, input);
    }

    #[test]
    fn test_estimate_compression_text() {
        let text = b"The quick brown fox jumps over the lazy dog. ".repeat(100);
        let est_low = estimate_compression(&text, text.len() as u64, 2);
        let est_med = estimate_compression(&text, text.len() as u64, 6);
        let est_high = estimate_compression(&text, text.len() as u64, 9);
        let est_neural = estimate_compression(&text, text.len() as u64, 18);

        assert!(!est_low.is_incompressible);
        assert!(est_low.savings_percent > 40.0);
        assert!(est_med.savings_percent >= est_low.savings_percent - 5.0);
        assert!(est_neural.estimated_bytes <= est_high.estimated_bytes);
    }

    #[test]
    fn test_estimate_compression_incompressible() {
        let mut pseudo_random = vec![0u8; 10000];
        for (i, b) in pseudo_random.iter_mut().enumerate() {
            *b = ((i * 101 + 37) % 256) as u8;
        }
        let est = estimate_compression(&pseudo_random, pseudo_random.len() as u64, 6);
        assert!(est.is_incompressible || est.savings_percent < 5.0);
    }
}
