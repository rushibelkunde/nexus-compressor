//! LZ77 sliding-window compression engine.
//!
//! Replaces repeated byte sequences with (offset, length) references using
//! a hash-chain match finder.

use crate::error::{NexusError, Result};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::cmp;

/// Default sliding window size (32 KB).
pub const DEFAULT_WINDOW_SIZE: usize = 32 * 1024;
/// Minimum match length required for an LZ77 reference (3 bytes).
pub const MIN_MATCH_LENGTH: usize = 3;
/// Maximum match length in Phase 1 (258 bytes).
pub const MAX_MATCH_LENGTH: usize = 258;
/// Maximum hash-chain traversal depth for match search.
pub const DEFAULT_CHAIN_LIMIT: usize = 64;
/// Hash table entry count (must be power of two).
pub const HASH_TABLE_SIZE: usize = 1 << 16;
pub const HASH_MASK: usize = HASH_TABLE_SIZE - 1;

/// Represents a single LZ77 output symbol: literal byte or match copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lz77Token {
    /// An unmatched literal byte.
    Literal(u8),
    /// A match reference to previous data: `offset` bytes back, copy `length` bytes.
    Match { offset: u32, length: u16 },
}

/// LZ77 encoder using hash-chain dictionary search.
pub struct Lz77Encoder {
    window_size: usize,
    chain_limit: usize,
    min_match: usize,
    max_match: usize,
}

impl Default for Lz77Encoder {
    fn default() -> Self {
        Self::new(DEFAULT_WINDOW_SIZE)
    }
}

impl Lz77Encoder {
    /// Creates a new encoder with the specified sliding window size.
    pub fn new(window_size: usize) -> Self {
        Self {
            window_size,
            chain_limit: DEFAULT_CHAIN_LIMIT,
            min_match: MIN_MATCH_LENGTH,
            max_match: MAX_MATCH_LENGTH,
        }
    }

    /// Customizes chain limit for speed vs ratio trade-off.
    pub fn with_chain_limit(mut self, limit: usize) -> Self {
        self.chain_limit = limit;
        self
    }

    /// Fast 3-byte hash function.
    #[inline(always)]
    fn hash3(data: &[u8], pos: usize) -> usize {
        let b0 = data[pos] as usize;
        let b1 = data[pos + 1] as usize;
        let b2 = data[pos + 2] as usize;
        ((b0 << 10) ^ (b1 << 5) ^ b2) & HASH_MASK
    }

    /// Encodes input bytes into a vector of LZ77 tokens.
    pub fn encode(&self, input: &[u8]) -> Vec<Lz77Token> {
        self.encode_with_history(input, &[])
    }

    /// Encodes input bytes with a warm sliding-window history from preceding sub-blocks.
    pub fn encode_with_history(&self, input: &[u8], history: &[u8]) -> Vec<Lz77Token> {
        let len = input.len();
        if len == 0 {
            return Vec::new();
        }

        if history.is_empty() {
            let mut tokens = Vec::with_capacity(len / 2 + 16);
            let mut head = vec![usize::MAX; HASH_TABLE_SIZE];
            let mut prev = vec![usize::MAX; len];

            let mut pos = 0;
            while pos < len {
                if pos + self.min_match > len {
                    tokens.push(Lz77Token::Literal(input[pos]));
                    pos += 1;
                    continue;
                }

                let h = Self::hash3(input, pos);
                let mut match_pos = head[h];
                prev[pos] = match_pos;
                head[h] = pos;

                let mut best_len = 0;
                let mut best_offset = 0;
                let mut chain_depth = 0;

                let window_start = pos.saturating_sub(self.window_size);

                while match_pos != usize::MAX
                    && match_pos >= window_start
                    && chain_depth < self.chain_limit
                {
                    chain_depth += 1;
                    let offset = pos - match_pos;
                    let max_len = cmp::min(len - pos, self.max_match);

                    if best_len >= self.min_match
                        && (input[match_pos + best_len - 1] != input[pos + best_len - 1]
                            || input[match_pos] != input[pos])
                    {
                        match_pos = prev[match_pos];
                        continue;
                    }

                    let mut cur_len = 0;
                    while cur_len < max_len && input[match_pos + cur_len] == input[pos + cur_len] {
                        cur_len += 1;
                    }

                    if cur_len > best_len {
                        best_len = cur_len;
                        best_offset = offset;
                        if best_len == max_len {
                            break;
                        }
                    }

                    match_pos = prev[match_pos];
                }

                if best_len >= self.min_match {
                    tokens.push(Lz77Token::Match {
                        offset: best_offset as u32,
                        length: best_len as u16,
                    });
                    #[allow(clippy::needless_range_loop)]
                    for i in (pos + 1)..(pos + best_len) {
                        if i + self.min_match <= len {
                            let hi = Self::hash3(input, i);
                            prev[i] = head[hi];
                            head[hi] = i;
                        }
                    }
                    pos += best_len;
                } else {
                    tokens.push(Lz77Token::Literal(input[pos]));
                    pos += 1;
                }
            }

            return tokens;
        }

        // Warm history path
        let hist = if history.len() > self.window_size {
            &history[history.len() - self.window_size..]
        } else {
            history
        };
        let h_len = hist.len();
        let total_len = h_len + len;

        let mut tokens = Vec::with_capacity(len / 2 + 16);
        let mut head = vec![usize::MAX; HASH_TABLE_SIZE];
        let mut prev = vec![usize::MAX; total_len];

        // Pre-populate hash table with history window
        if h_len >= self.min_match {
            for i in 0..=(h_len - self.min_match) {
                let b0 = hist[i] as usize;
                let b1 = hist[i + 1] as usize;
                let b2 = hist[i + 2] as usize;
                let h = ((b0 << 10) ^ (b1 << 5) ^ b2) & HASH_MASK;
                prev[i] = head[h];
                head[h] = i;
            }
        }
        if h_len >= 2 && len >= 1 {
            let i = h_len - 2;
            let b0 = hist[h_len - 2] as usize;
            let b1 = hist[h_len - 1] as usize;
            let b2 = input[0] as usize;
            let h = ((b0 << 10) ^ (b1 << 5) ^ b2) & HASH_MASK;
            prev[i] = head[h];
            head[h] = i;
        }
        if h_len >= 1 && len >= 2 {
            let i = h_len - 1;
            let b0 = hist[h_len - 1] as usize;
            let b1 = input[0] as usize;
            let b2 = input[1] as usize;
            let h = ((b0 << 10) ^ (b1 << 5) ^ b2) & HASH_MASK;
            prev[i] = head[h];
            head[h] = i;
        }

        let mut pos = h_len;
        while pos < total_len {
            let in_pos = pos - h_len;
            if pos + self.min_match > total_len {
                tokens.push(Lz77Token::Literal(input[in_pos]));
                pos += 1;
                continue;
            }

            let h = Self::hash3(input, in_pos);
            let mut match_pos = head[h];
            prev[pos] = match_pos;
            head[h] = pos;

            let mut best_len = 0;
            let mut best_offset = 0;
            let mut chain_depth = 0;

            let window_start = pos.saturating_sub(self.window_size);

            let get_byte = |idx: usize| -> u8 {
                if idx < h_len {
                    hist[idx]
                } else {
                    input[idx - h_len]
                }
            };

            while match_pos != usize::MAX
                && match_pos >= window_start
                && chain_depth < self.chain_limit
            {
                chain_depth += 1;
                let offset = pos - match_pos;
                let max_len = cmp::min(total_len - pos, self.max_match);

                if best_len >= self.min_match
                    && (get_byte(match_pos + best_len - 1) != input[in_pos + best_len - 1]
                        || get_byte(match_pos) != input[in_pos])
                {
                    match_pos = prev[match_pos];
                    continue;
                }

                let mut cur_len = 0;
                while cur_len < max_len && get_byte(match_pos + cur_len) == input[in_pos + cur_len]
                {
                    cur_len += 1;
                }

                if cur_len > best_len {
                    best_len = cur_len;
                    best_offset = offset;
                    if best_len == max_len {
                        break;
                    }
                }

                match_pos = prev[match_pos];
            }

            if best_len >= self.min_match {
                tokens.push(Lz77Token::Match {
                    offset: best_offset as u32,
                    length: best_len as u16,
                });
                #[allow(clippy::needless_range_loop)]
                for i in (pos + 1)..(pos + best_len) {
                    if i + self.min_match <= total_len {
                        let hi = Self::hash3(input, i - h_len);
                        prev[i] = head[hi];
                        head[hi] = i;
                    }
                }
                pos += best_len;
            } else {
                tokens.push(Lz77Token::Literal(input[in_pos]));
                pos += 1;
            }
        }

        tokens
    }
}

/// LZ77 decoder that reconstructs original bytes from token streams.
#[derive(Default)]
pub struct Lz77Decoder;

impl Lz77Decoder {
    pub fn new() -> Self {
        Self
    }

    /// Decodes a slice of LZ77 tokens into reconstructed raw bytes.
    pub fn decode(&self, tokens: &[Lz77Token]) -> Result<Vec<u8>> {
        self.decode_with_history(tokens, &[])
    }

    /// Decodes LZ77 tokens with warm history context from prior sub-blocks.
    pub fn decode_with_history(&self, tokens: &[Lz77Token], history: &[u8]) -> Result<Vec<u8>> {
        let mut output = Vec::new();
        let h_len = history.len();

        for token in tokens {
            match *token {
                Lz77Token::Literal(b) => {
                    output.push(b);
                }
                Lz77Token::Match { offset, length } => {
                    let offset = offset as usize;
                    let length = length as usize;
                    let total_available = h_len + output.len();

                    if offset == 0 || offset > total_available {
                        return Err(NexusError::DataCorruption(format!(
                            "Invalid LZ77 match offset {} at output len {} (total available: {})",
                            offset,
                            output.len(),
                            total_available
                        )));
                    }

                    let idx = total_available - offset;
                    if idx + length <= h_len {
                        // Match lies entirely within previous history
                        output.extend_from_slice(&history[idx..idx + length]);
                    } else if idx >= h_len && offset >= length {
                        // Match lies entirely within current sub-block without self-overlap
                        let out_start = idx - h_len;
                        output.extend_from_within(out_start..out_start + length);
                    } else {
                        // Overlapping match or crosses history-to-current boundary
                        for _ in 0..length {
                            let cur_idx = (h_len + output.len()) - offset;
                            let b = if cur_idx < h_len {
                                history[cur_idx]
                            } else {
                                output[cur_idx - h_len]
                            };
                            output.push(b);
                        }
                    }
                }
            }
        }

        Ok(output)
    }
}

/// Serializes LZ77 tokens into a compact byte vector.
pub fn serialize_tokens(tokens: &[Lz77Token]) -> Vec<u8> {
    let mut out = Vec::with_capacity(tokens.len() * 3);
    for token in tokens {
        match *token {
            Lz77Token::Literal(b) => {
                out.push(0x00);
                out.push(b);
            }
            Lz77Token::Match { offset, length } => {
                if offset <= 0xFFFF {
                    out.push(0x01);
                    out.write_u16::<LittleEndian>(offset as u16).unwrap();
                    out.write_u16::<LittleEndian>(length).unwrap();
                } else {
                    out.push(0x02);
                    out.write_u32::<LittleEndian>(offset).unwrap();
                    out.write_u16::<LittleEndian>(length).unwrap();
                }
            }
        }
    }
    out
}

/// Deserializes LZ77 tokens from a packed byte slice.
pub fn deserialize_tokens(mut bytes: &[u8]) -> Result<Vec<Lz77Token>> {
    let mut tokens = Vec::new();
    while !bytes.is_empty() {
        let tag = bytes.read_u8()?;
        match tag {
            0x00 => {
                let b = bytes.read_u8()?;
                tokens.push(Lz77Token::Literal(b));
            }
            0x01 => {
                let offset = bytes.read_u16::<LittleEndian>()? as u32;
                let length = bytes.read_u16::<LittleEndian>()?;
                tokens.push(Lz77Token::Match { offset, length });
            }
            0x02 => {
                let offset = bytes.read_u32::<LittleEndian>()?;
                let length = bytes.read_u16::<LittleEndian>()?;
                tokens.push(Lz77Token::Match { offset, length });
            }
            other => {
                return Err(NexusError::DataCorruption(format!(
                    "Invalid LZ77 token tag 0x{:02x}",
                    other
                )));
            }
        }
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lz77_empty() {
        let encoder = Lz77Encoder::default();
        let decoder = Lz77Decoder::new();
        let input = b"";
        let tokens = encoder.encode(input);
        assert!(tokens.is_empty());
        let decompressed = decoder.decode(&tokens).unwrap();
        assert_eq!(decompressed, input);
    }

    #[test]
    fn test_lz77_single_byte() {
        let encoder = Lz77Encoder::default();
        let decoder = Lz77Decoder::new();
        let input = b"x";
        let tokens = encoder.encode(input);
        assert_eq!(tokens.len(), 1);
        let decompressed = decoder.decode(&tokens).unwrap();
        assert_eq!(decompressed, input);
    }

    #[test]
    fn test_lz77_all_same_bytes() {
        let encoder = Lz77Encoder::default();
        let decoder = Lz77Decoder::new();
        let input = vec![b'a'; 10_000];
        let tokens = encoder.encode(&input);
        assert!(tokens.len() < 100); // Massive reduction
        let decompressed = decoder.decode(&tokens).unwrap();
        assert_eq!(decompressed, input);
    }

    #[test]
    fn test_lz77_repeating_pattern() {
        let encoder = Lz77Encoder::default();
        let decoder = Lz77Decoder::new();
        let pattern = b"abcabcabcabc";
        let tokens = encoder.encode(pattern);
        let decompressed = decoder.decode(&tokens).unwrap();
        assert_eq!(decompressed, pattern);
    }

    #[test]
    fn test_lz77_long_match() {
        let encoder = Lz77Encoder::default();
        let decoder = Lz77Decoder::new();
        let mut input = Vec::new();
        for _ in 0..500 {
            input.extend_from_slice(b"12345678");
        }
        let tokens = encoder.encode(&input);
        let decompressed = decoder.decode(&tokens).unwrap();
        assert_eq!(decompressed, input);
    }

    #[test]
    fn test_lz77_random_data() {
        let encoder = Lz77Encoder::default();
        let decoder = Lz77Decoder::new();
        let mut rng_state: u64 = 0x123456789ABCDEF;
        let mut input = vec![0u8; 65536];
        for b in input.iter_mut() {
            rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
            *b = (rng_state >> 33) as u8;
        }

        let tokens = encoder.encode(&input);
        let decompressed = decoder.decode(&tokens).unwrap();
        assert_eq!(decompressed, input);
    }

    #[test]
    fn test_token_serialization_roundtrip() {
        let tokens = vec![
            Lz77Token::Literal(b'h'),
            Lz77Token::Literal(b'i'),
            Lz77Token::Match {
                offset: 2,
                length: 10,
            },
            Lz77Token::Match {
                offset: 70000,
                length: 50,
            },
        ];
        let serialized = serialize_tokens(&tokens);
        let deserialized = deserialize_tokens(&serialized).unwrap();
        assert_eq!(tokens, deserialized);
    }

    #[test]
    fn test_lz77_warm_history_cross_block_match() {
        let encoder = Lz77Encoder::default();
        let decoder = Lz77Decoder::new();

        let block1 = b"Hello, this is a test of the NEXUS warm dictionary architecture!";
        let block2 = b"Another sentence with: NEXUS warm dictionary architecture!";

        // Encode block1 without history
        let tokens1 = encoder.encode(block1);
        let dec1 = decoder.decode(&tokens1).unwrap();
        assert_eq!(dec1, block1);

        // Encode block2 with block1 as warm history
        let tokens2 = encoder.encode_with_history(block2, block1);

        // Verify that a match referencing block1 was found (e.g. offset > block2 position)
        let has_cross_match = tokens2.iter().any(|t| match t {
            Lz77Token::Match { length, .. } => *length >= 10,
            _ => false,
        });
        assert!(has_cross_match, "Should have found long cross-block match");

        // Decode block2 with block1 as history
        let dec2 = decoder.decode_with_history(&tokens2, block1).unwrap();
        assert_eq!(dec2, block2);
    }

    #[test]
    fn test_lz77_warm_history_multi_block_sequence() {
        let encoder = Lz77Encoder::default();
        let decoder = Lz77Decoder::new();

        let chunks = [
            b"First block of data containing unique keywords: ALPHA BETA GAMMA.".as_slice(),
            b"Second block repeating ALPHA BETA GAMMA and introducing DELTA EPSILON.".as_slice(),
            b"Third block repeating DELTA EPSILON and ALPHA BETA GAMMA.".as_slice(),
        ];

        let mut history_enc = Vec::new();
        let mut history_dec = Vec::new();

        for chunk in &chunks {
            let tokens = encoder.encode_with_history(chunk, &history_enc);
            let restored = decoder.decode_with_history(&tokens, &history_dec).unwrap();
            assert_eq!(&restored, chunk);

            history_enc.extend_from_slice(chunk);
            history_dec.extend_from_slice(&restored);
        }
    }
}
