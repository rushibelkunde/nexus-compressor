//! Context Mixing compression engine (Path C, Layer 3).
//!
//! Blends predictions from multiple context models using a fixed-point neural mixer
//! and integer arithmetic coding. Outperforms standard dictionary compressors on complex data.

pub mod models;
pub mod tables;

use crate::compress::mixer::models::{
    ContextModel, ContextState, MatchModel, Order0Model, Order1Model, Order2Model, WordModel,
};
use crate::compress::mixer::tables::{squash, stretch};
use crate::entropy::arithmetic::{ArithmeticDecoder, ArithmeticEncoder};
use crate::error::Result;

/// Rolling buffer size for match model lookups (64 KB).
const MATCH_BUFFER_SIZE: usize = 65536;
const MATCH_BUFFER_MASK: usize = MATCH_BUFFER_SIZE - 1;

/// Neural logistic mixer using fixed-point integer arithmetic.
pub struct LogisticMixer {
    m_order0: Order0Model,
    m_order1: Order1Model,
    m_order2: Order2Model,
    m_match: MatchModel,
    m_word: WordModel,
    weights: [i16; 5],
    last_predictions: [u16; 5],
    last_blended_p: u16,

    // Match tracking state
    history: Vec<u8>,
    match_pos: Option<usize>,
    match_len: usize,
    hash_pos: Vec<usize>,
}

impl Default for LogisticMixer {
    fn default() -> Self {
        Self::new()
    }
}

impl LogisticMixer {
    /// Creates a new mixer with equal initial weights.
    pub fn new() -> Self {
        Self {
            m_order0: Order0Model::new(),
            m_order1: Order1Model::new(),
            m_order2: Order2Model::default(),
            m_match: MatchModel::new(),
            m_word: WordModel::new(32768),
            // Initial weights: Order-0: 100, Order-1: 150, Order-2: 200, Match: 300, Word: 150
            weights: [100, 150, 200, 300, 150],
            last_predictions: [2048; 5],
            last_blended_p: 2048,
            history: Vec::with_capacity(MATCH_BUFFER_SIZE),
            match_pos: None,
            match_len: 0,
            hash_pos: vec![usize::MAX; 65536],
        }
    }

    /// Predicts blended P(bit = 1) in [1, 4095] for current context.
    #[inline(always)]
    pub fn predict(&mut self, ctx: &ContextState) -> u16 {
        self.last_predictions[0] = self.m_order0.predict(ctx);
        self.last_predictions[1] = self.m_order1.predict(ctx);
        self.last_predictions[2] = self.m_order2.predict(ctx);
        self.last_predictions[3] = self.m_match.predict(ctx);
        self.last_predictions[4] = self.m_word.predict(ctx);

        let mut sum_logit: i64 = 0;
        let mut total_weight: i64 = 0;

        for i in 0..5 {
            let logit = stretch(self.last_predictions[i]) as i64;
            let w = self.weights[i] as i64;
            sum_logit += w * logit;
            total_weight += w.abs();
        }

        let blended_logit = if total_weight > 0 {
            (sum_logit / total_weight.max(1)).clamp(-2048, 2048) as i32
        } else {
            0
        };

        let p1 = squash(blended_logit);
        self.last_blended_p = p1;
        p1
    }

    /// Updates models and weights with the observed actual bit using integer SGD.
    #[inline(always)]
    pub fn update(&mut self, ctx: &ContextState, bit: u8) {
        // 1. Update sub-models
        self.m_order0.update(ctx, bit);
        self.m_order1.update(ctx, bit);
        self.m_order2.update(ctx, bit);
        self.m_match.update(ctx, bit);
        self.m_word.update(ctx, bit);

        // 2. Integer SGD weight update
        // error in [-4095, 4095]
        let target = if bit == 1 { 4095i32 } else { 1i32 };
        let error = target - (self.last_blended_p as i32);

        for i in 0..5 {
            let logit = stretch(self.last_predictions[i]) as i64;
            // Learning rate shift (>> 14 gives smooth convergence)
            let delta = ((logit * (error as i64)) >> 14) as i16;
            self.weights[i] = (self.weights[i].saturating_add(delta)).clamp(-4000, 4000);
        }

        // 3. Match prediction check
        if let Some(pred) = ctx.match_predicted_bit {
            if pred != bit {
                // Match broken
                self.match_pos = None;
                self.match_len = 0;
            }
        }
    }

    /// Advances context state at the end of a decoded/encoded byte.
    pub fn on_byte_end(&mut self, byte: u8, ctx: &mut ContextState) {
        let n = self.history.len();

        // 1. Update match state
        if let Some(pos) = self.match_pos {
            let next_pos = (pos + 1) & MATCH_BUFFER_MASK;
            if next_pos < n {
                self.match_pos = Some(next_pos);
                self.match_len += 1;
            } else {
                self.match_pos = None;
                self.match_len = 0;
            }
        }

        // If no active match, check 3-byte hash table for new match candidates
        if self.match_pos.is_none() && n >= 2 {
            let b0 = self.history[n - 2] as usize;
            let b1 = self.history[n - 1] as usize;
            let b2 = byte as usize;
            let h = ((b0 << 10) ^ (b1 << 5) ^ b2) & 0xFFFF;
            let prev_match = self.hash_pos[h];
            self.hash_pos[h] = n;

            if prev_match != usize::MAX && prev_match + 1 < n {
                self.match_pos = Some(prev_match + 1);
                self.match_len = 3;
            }
        }

        // 2. Update match predicted bit for the next byte
        ctx.match_predicted_bit = self.match_pos.and_then(|pos| {
            if pos < self.history.len() {
                let candidate_byte = self.history[pos];
                Some((candidate_byte >> ctx.bit_pos) & 1)
            } else {
                None
            }
        });

        // 3. Append byte to history buffer
        if self.history.len() < MATCH_BUFFER_SIZE {
            self.history.push(byte);
        } else {
            let idx = n & MATCH_BUFFER_MASK;
            self.history[idx] = byte;
        }

        // 4. Update word hash for text
        if byte.is_ascii_alphanumeric() || byte == b'_' {
            ctx.word_hash = ctx.word_hash.wrapping_mul(31).wrapping_add(byte as u32);
        } else {
            ctx.word_hash = 0;
        }
    }
}

/// Compresses raw input bytes using Path C (Context Mixing + Arithmetic Coding).
pub fn context_mixing_compress(data: &[u8]) -> Vec<u8> {
    if data.is_empty() {
        return Vec::new();
    }

    let mut encoder = ArithmeticEncoder::new();
    let mut mixer = LogisticMixer::new();
    let mut ctx = ContextState::new();
    let mut compressed = Vec::with_capacity(data.len() / 2 + 16);

    for &byte in data {
        for bit_idx in 0..8 {
            let bpos = 7 - bit_idx;
            ctx.bit_pos = bpos;

            // Update match bit prediction for current bit position
            if let Some(pos) = mixer.match_pos {
                if pos < mixer.history.len() {
                    let cand = mixer.history[pos];
                    ctx.match_predicted_bit = Some((cand >> bpos) & 1);
                } else {
                    ctx.match_predicted_bit = None;
                }
            } else {
                ctx.match_predicted_bit = None;
            }

            let bit = (byte >> bpos) & 1;
            let p1 = mixer.predict(&ctx);
            encoder.encode_bit(bit, p1, &mut compressed);
            mixer.update(&ctx, bit);
            ctx.bit_prefix = (ctx.bit_prefix << 1) | (bit as usize);
        }

        ctx.prev_byte2 = ctx.prev_byte;
        ctx.prev_byte = byte;
        ctx.bit_prefix = 1;
        ctx.bit_pos = 7;
        mixer.on_byte_end(byte, &mut ctx);
    }

    encoder.finish(&mut compressed);
    compressed
}

/// Decompresses Path C byte stream back to original raw bytes.
pub fn context_mixing_decompress(compressed: &[u8], original_size: usize) -> Result<Vec<u8>> {
    if original_size == 0 {
        return Ok(Vec::new());
    }

    let mut decoder = ArithmeticDecoder::new(compressed);
    let mut mixer = LogisticMixer::new();
    let mut ctx = ContextState::new();
    let mut decompressed = Vec::with_capacity(original_size);

    for _ in 0..original_size {
        let mut byte = 0u8;

        for bit_idx in 0..8 {
            let bpos = 7 - bit_idx;
            ctx.bit_pos = bpos;

            if let Some(pos) = mixer.match_pos {
                if pos < mixer.history.len() {
                    let cand = mixer.history[pos];
                    ctx.match_predicted_bit = Some((cand >> bpos) & 1);
                } else {
                    ctx.match_predicted_bit = None;
                }
            } else {
                ctx.match_predicted_bit = None;
            }

            let p1 = mixer.predict(&ctx);
            let bit = decoder.decode_bit(p1);
            byte |= bit << bpos;
            mixer.update(&ctx, bit);
            ctx.bit_prefix = (ctx.bit_prefix << 1) | (bit as usize);
        }

        decompressed.push(byte);
        ctx.prev_byte2 = ctx.prev_byte;
        ctx.prev_byte = byte;
        ctx.bit_prefix = 1;
        ctx.bit_pos = 7;
        mixer.on_byte_end(byte, &mut ctx);
    }

    Ok(decompressed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_mixing_empty() {
        let input = b"";
        let compressed = context_mixing_compress(input);
        assert!(compressed.is_empty());
        let decompressed = context_mixing_decompress(&compressed, 0).unwrap();
        assert_eq!(decompressed, input);
    }

    #[test]
    fn test_context_mixing_single_byte() {
        let input = b"X";
        let compressed = context_mixing_compress(input);
        let decompressed = context_mixing_decompress(&compressed, input.len()).unwrap();
        assert_eq!(decompressed, input);
    }

    #[test]
    fn test_context_mixing_short_text() {
        let input = b"Hello, Project NEXUS Context Mixing Engine!";
        let compressed = context_mixing_compress(input);
        let decompressed = context_mixing_decompress(&compressed, input.len()).unwrap();
        assert_eq!(decompressed, input);
    }

    #[test]
    fn test_context_mixing_repetitive() {
        let input = b"Repeating string pattern ABCDEFG 1234567890. ".repeat(20);
        let compressed = context_mixing_compress(&input);
        assert!(
            compressed.len() < input.len() / 2,
            "Context mixing should compress repetitive patterns well: compressed={}, original={}",
            compressed.len(),
            input.len()
        );
        let decompressed = context_mixing_decompress(&compressed, input.len()).unwrap();
        assert_eq!(decompressed, input);
    }
}
