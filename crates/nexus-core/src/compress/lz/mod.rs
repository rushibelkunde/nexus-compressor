//! LZ-ANS v2: sequence-based LZ77 with repeat offsets and split entropy
//! streams (Path A2, Layer 3).
//!
//! A sub-block is parsed into *sequences* `(literal_length, match_length,
//! offset)`. Unlike the v1 token format, the four kinds of information are
//! kept in separate streams so each gets its own statistics:
//!
//! | Stream      | Content                                   | Coding            |
//! |-------------|-------------------------------------------|-------------------|
//! | literals    | unmatched bytes, concatenated             | interleaved rANS, |
//! |             |                                           | or context-coded  |
//! | `ll` codes  | literal-length code per sequence          | interleaved rANS  |
//! | `ml` codes  | match-length code per sequence            | interleaved rANS  |
//! | `of` codes  | offset code per sequence                  | interleaved rANS  |
//! | extra bits  | low bits of the three values, in order    | raw bit packing   |
//!
//! Values are split into a *code* and raw extra bits by [`value_code`]:
//! values below 16 are their own code; larger values use two codes per power
//! of two. Offset codes 0..=2 select one of three *repeat offsets* (the most
//! recently used offsets, move-to-front updated), which is what makes
//! structured and tabular data cheap.
//!
//! Matches may reach back to the start of the superblock (the window stays
//! warm across sub-blocks, AGENTS.md rule 4); entropy tables and repeat
//! offsets reset at every sub-block.
//!
//! Sub-block payload layout:
//!
//! ```text
//! varint  sequence_count
//! stream  literals
//! -- only if sequence_count > 0 --
//! stream  ll_codes
//! stream  ml_codes
//! stream  of_codes
//! varint  extra_bits_len, then that many bytes
//! ```
//!
//! Literals left over after the last sequence are copied verbatim at the end.
//!
//! At higher levels the literal stream may instead be a context-coded record
//! (`entropy::ctx_rans`, mode byte 3), where each literal is coded with a
//! table chosen by the byte that precedes it in the output.

pub mod decoder;
pub mod encoder;
mod optimal;

pub use decoder::{decode_block, LzDecodeScratch};
pub use encoder::LzEncoder;

/// Shortest match the format can express.
pub const MIN_MATCH: usize = 3;
/// Number of distinct value codes (covers the full `u32` range).
pub const NUM_VALUE_CODES: usize = 72;
/// Number of repeat-offset codes preceding the explicit offset codes.
pub const NUM_REP_CODES: usize = 3;
/// Repeat offsets at the start of every sub-block.
pub const INITIAL_REPS: [u32; 3] = [1, 4, 8];

/// Base value of each value code.
pub static CODE_BASE: [u32; NUM_VALUE_CODES] = build_code_base();
/// Number of raw extra bits following each value code.
pub static CODE_BITS: [u8; NUM_VALUE_CODES] = build_code_bits();

const fn build_code_base() -> [u32; NUM_VALUE_CODES] {
    let mut t = [0u32; NUM_VALUE_CODES];
    let mut c = 0;
    while c < NUM_VALUE_CODES {
        if c < 16 {
            t[c] = c as u32;
        } else {
            let k = c - 16;
            let hb = 4 + (k / 2) as u32;
            t[c] = (1u32 << hb) + (((k & 1) as u32) << (hb - 1));
        }
        c += 1;
    }
    t
}

const fn build_code_bits() -> [u8; NUM_VALUE_CODES] {
    let mut t = [0u8; NUM_VALUE_CODES];
    let mut c = 16;
    while c < NUM_VALUE_CODES {
        t[c] = (4 + (c - 16) / 2 - 1) as u8;
        c += 1;
    }
    t
}

/// Splits a value into `(code, extra_bit_count, extra_bits)`.
#[inline(always)]
pub fn value_code(v: u32) -> (u8, u32, u32) {
    if v < 16 {
        (v as u8, 0, 0)
    } else {
        let hb = 31 - v.leading_zeros();
        let half = (v >> (hb - 1)) & 1;
        let nbits = hb - 1;
        (
            (16 + (hb - 4) * 2 + half) as u8,
            nbits,
            v & ((1 << nbits) - 1),
        )
    }
}

/// Match-search strategy, in increasing order of effort.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    /// Single hash probe, greedy parse.
    Fast,
    /// Two hash probes (long and short), greedy parse.
    DoubleFast,
    /// Tagged hash rows (16 candidates per row) with lazy evaluation.
    LazyRow,
    /// Hash chains with lazy evaluation.
    Lazy,
    /// Binary-tree match finder feeding a price-based optimal parse.
    Optimal,
}

/// Tunable parameters of the LZ encoder for one compression level.
#[derive(Debug, Clone, Copy)]
pub struct LzParams {
    /// Search/parse strategy.
    pub strategy: Strategy,
    /// log2 of the hash table entry count (for `LazyRow`: of the row count).
    pub hash_log: u32,
    /// Bytes hashed per position by the fast and lazy strategies (4..=8).
    pub hash_bytes: u32,
    /// Maximum hash-chain candidates examined per position.
    pub depth: u32,
    /// Lazy lookahead steps (0 = greedy).
    pub lazy: u32,
    /// Match length at which searching stops early.
    pub nice_len: u32,
    /// Passes of the optimal parser over each sub-block.
    pub opt_passes: u32,
    /// Maximum number of literal context classes (0 = plain order-0 literals).
    pub lit_classes: u32,
}

/// Returns the LZ parameters for a compression level (clamped to 1..=12;
/// higher levels use the level-12 parameters).
pub fn params_for_level(level: u32) -> LzParams {
    use Strategy::*;
    let p =
        |strategy, hash_log, hash_bytes, depth, lazy, nice_len, opt_passes, lit_classes| LzParams {
            strategy,
            hash_log,
            hash_bytes,
            depth,
            lazy,
            nice_len,
            opt_passes,
            lit_classes,
        };
    match level {
        0 | 1 => p(Fast, 14, 6, 1, 0, 0, 0, 0),
        2 => p(Fast, 16, 5, 1, 0, 0, 0, 0),
        3 => p(DoubleFast, 16, 5, 1, 0, 0, 0, 0),
        4 => p(DoubleFast, 18, 4, 1, 0, 0, 0, 0),
        5 => p(LazyRow, 17, 5, 16, 1, 48, 0, 0),
        6 => p(LazyRow, 18, 5, 16, 2, 64, 0, 0),
        7 => p(LazyRow, 19, 4, 16, 2, 96, 0, 8),
        8 => p(Lazy, 21, 4, 48, 2, 128, 0, 8),
        9 => p(Lazy, 21, 4, 128, 2, 256, 0, 8),
        10 => p(Optimal, 20, 4, 12, 0, 48, 1, 12),
        11 => p(Optimal, 21, 4, 32, 0, 128, 1, 12),
        _ => p(Optimal, 22, 4, 96, 0, 273, 2, 16),
    }
}

/// Target sub-block size at a compression level. Levels that model literal
/// contexts use larger sub-blocks so the extra tables are amortised.
pub fn subblock_target_for_level(level: u32) -> usize {
    match level {
        // Block-sorting (levels 13+) gains with every extra byte of context.
        13.. => 512 * 1024,
        7..=12 => 384 * 1024,
        _ => 128 * 1024,
    }
}

/// Returns the superblock size used at a compression level. Larger superblocks
/// give the LZ window more reach; smaller ones give more parallelism.
pub fn superblock_size_for_level(level: u32) -> usize {
    const MB: usize = 1024 * 1024;
    match level {
        0..=2 => 2 * MB,
        3..=4 => 4 * MB,
        5..=7 => 8 * MB,
        _ => 16 * MB,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value_code_tables_are_consistent() {
        let mut probe: Vec<u32> = (0..5000).collect();
        for k in 12..32 {
            let p = 1u32 << k;
            probe.extend_from_slice(&[p - 1, p, p + 1, p + (p >> 1) - 1, p + (p >> 1)]);
        }
        probe.push(u32::MAX);
        for v in probe {
            let (code, nbits, extra) = value_code(v);
            assert!((code as usize) < NUM_VALUE_CODES);
            assert_eq!(CODE_BITS[code as usize] as u32, nbits);
            assert_eq!(CODE_BASE[code as usize] + extra, v);
            assert!(nbits == 0 || extra < (1 << nbits));
        }
    }

    fn lcg(seed: u64) -> impl FnMut() -> u64 {
        let mut s = seed;
        move || {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            s >> 33
        }
    }

    /// Builds inputs that stress different parts of the codec.
    #[allow(clippy::vec_init_then_push)]
    fn corpora() -> Vec<(&'static str, Vec<u8>)> {
        let mut out: Vec<(&'static str, Vec<u8>)> = Vec::new();
        out.push(("empty", Vec::new()));
        out.push(("one", vec![b'x']));
        out.push(("tiny", b"abcabcabcabcabc".to_vec()));
        out.push(("zeros", vec![0u8; 300_000]));

        let mut r = lcg(1);
        out.push(("random", (0..200_000).map(|_| r() as u8).collect()));

        let words: [&[u8]; 8] = [
            b"compression ",
            b"the ",
            b"quick ",
            b"entropy ",
            b"of ",
            b"NEXUS ",
            b"window\n",
            b"match ",
        ];
        let mut text = Vec::new();
        let mut r = lcg(2);
        while text.len() < 400_000 {
            text.extend_from_slice(words[(r() % 8) as usize]);
        }
        out.push(("text", text));

        // Fixed-width records: exercises repeat offsets.
        let mut rec = Vec::new();
        let mut r = lcg(3);
        for i in 0..20_000u32 {
            rec.extend_from_slice(&i.to_le_bytes());
            rec.extend_from_slice(&[0, 0, (r() % 3) as u8, 0xFF]);
            rec.extend_from_slice(&((r() % 1000) as u16).to_le_bytes());
        }
        out.push(("records", rec));

        // A long block repeated far apart: exercises long offsets and
        // very long matches.
        let mut r = lcg(4);
        let unit: Vec<u8> = (0..150_000).map(|_| (r() % 64) as u8).collect();
        let mut far = unit.clone();
        far.extend((0..100_000).map(|_| r() as u8));
        far.extend_from_slice(&unit);
        out.push(("far-repeat", far));

        // Short-period patterns: overlapping copies.
        let mut pat = Vec::new();
        for period in 1..40usize {
            for i in 0..3000 {
                pat.push((i % period) as u8);
            }
        }
        out.push(("periodic", pat));
        out
    }

    fn roundtrip(data: &[u8], level: u32, block: usize) -> usize {
        let mut enc = LzEncoder::new(params_for_level(level), data.len());
        let mut scratch = LzDecodeScratch::default();
        let mut decoded: Vec<u8> = Vec::new();
        let mut total = 0;
        let mut start = 0;
        loop {
            let end = (start + block).min(data.len());
            let mut payload = Vec::new();
            enc.compress_block(data, start, end, &mut payload);
            total += payload.len();
            decode_block(&payload, &mut decoded, end - start, &mut scratch)
                .unwrap_or_else(|e| panic!("level {level} block at {start}: {e}"));
            assert_eq!(&decoded[..], &data[..end], "level {level} block at {start}");
            start = end;
            if start >= data.len() {
                break;
            }
        }
        total
    }

    #[test]
    fn test_lz_roundtrip_all_levels_single_block() {
        for (name, data) in corpora() {
            for level in 1..=12 {
                let size = roundtrip(&data, level, usize::MAX);
                if name == "zeros" {
                    assert!(size < 64, "{name} level {level}: {size}");
                }
                if name == "text" || name == "records" || name == "far-repeat" {
                    assert!(size < data.len() * 6 / 10, "{name} level {level}: {size}");
                }
            }
        }
    }

    #[test]
    fn test_lz_roundtrip_all_levels_warm_window_across_blocks() {
        for (_, data) in corpora() {
            for level in [1, 3, 5, 6, 7, 8, 10, 12] {
                roundtrip(&data, level, 37_123);
            }
        }
    }

    #[test]
    fn test_lz_optimal_tree_survives_block_boundaries() {
        // Regression: the binary-tree match finder once ordered positions
        // near a sub-block end using only the bytes of that sub-block. Text
        // with many shared prefixes cut into small blocks exposes any such
        // inconsistency as a decode mismatch.
        let phrases: [&[u8]; 6] = [
            b"the quick brown fox ",
            b"the quick brown dog ",
            b"the quick red fox ",
            b"the slow brown fox ",
            b"a quick brown fox ",
            b"the quick brown fox jumps over the lazy dog ",
        ];
        let mut r = lcg(31);
        let mut data = Vec::new();
        while data.len() < 600_000 {
            data.extend_from_slice(phrases[(r() % 6) as usize]);
        }
        for level in [10, 11, 12] {
            for block in [997, 4096, 65_537] {
                roundtrip(&data, level, block);
            }
        }
    }

    #[test]
    fn test_lz_window_reaches_previous_blocks() {
        // The second block is an exact copy of the first: with a warm window
        // it must cost almost nothing.
        let mut r = lcg(9);
        let unit: Vec<u8> = (0..100_000).map(|_| r() as u8).collect();
        let mut data = unit.clone();
        data.extend_from_slice(&unit);
        for level in [1, 6, 11] {
            let mut enc = LzEncoder::new(params_for_level(level), data.len());
            let mut first = Vec::new();
            enc.compress_block(&data, 0, unit.len(), &mut first);
            let mut second = Vec::new();
            enc.compress_block(&data, unit.len(), data.len(), &mut second);
            assert!(second.len() < 64, "level {level}: {}", second.len());
        }
    }

    #[test]
    fn test_lz_higher_levels_do_not_regress_on_text() {
        let (_, text) = corpora().into_iter().find(|(n, _)| *n == "text").unwrap();
        let fast = roundtrip(&text, 1, usize::MAX);
        let lazy = roundtrip(&text, 6, usize::MAX);
        let opt = roundtrip(&text, 12, usize::MAX);
        assert!(lazy <= fast, "lazy {lazy} vs fast {fast}");
        assert!(opt <= lazy, "optimal {opt} vs lazy {lazy}");
    }

    #[test]
    fn test_lz_decoder_rejects_corrupt_payloads_without_panicking() {
        let (_, text) = corpora().into_iter().find(|(n, _)| *n == "text").unwrap();
        let data = &text[..50_000];
        let mut enc = LzEncoder::new(params_for_level(5), data.len());
        let mut payload = Vec::new();
        enc.compress_block(data, 0, data.len(), &mut payload);

        let mut scratch = LzDecodeScratch::default();
        let mut r = lcg(77);
        for _ in 0..2000 {
            let mut bad = payload.clone();
            let flips = 1 + (r() % 3) as usize;
            for _ in 0..flips {
                let i = (r() as usize) % bad.len();
                bad[i] ^= 1 << (r() % 8);
            }
            let mut out = Vec::new();
            // Must return (Ok with wrong bytes is caught by block checksums
            // one layer up) and never panic or read out of bounds.
            let _ = decode_block(&bad, &mut out, data.len(), &mut scratch);
        }
        for cut in [0, 1, 2, 10, payload.len() / 2, payload.len() - 1] {
            let mut out = Vec::new();
            assert!(decode_block(&payload[..cut], &mut out, data.len(), &mut scratch).is_err());
        }
    }

    #[test]
    fn test_value_codes_are_monotonic() {
        let mut prev = 0;
        for v in 0..100_000u32 {
            let (code, _, _) = value_code(v);
            assert!(code >= prev);
            prev = code;
        }
    }
}
