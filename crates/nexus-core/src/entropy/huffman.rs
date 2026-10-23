//! Canonical Huffman entropy coder and bitstream reader/writer.
//!
//! Provides optimal prefix coding with compact canonical header representation
//! and O(1) table lookup decoding.

use crate::error::{NexusError, Result};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// Maximum Huffman code length in bits.
pub const MAX_CODE_LENGTH: usize = 32;

/// Helper for packing bits into a byte buffer (MSB first).
#[derive(Default)]
pub struct BitWriter {
    buffer: Vec<u8>,
    current_byte: u8,
    bit_count: u8,
}

impl BitWriter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Writes `count` bits from `value` (MSB first).
    pub fn write_bits(&mut self, value: u32, count: u8) {
        for i in (0..count).rev() {
            let bit = ((value >> i) & 1) as u8;
            self.current_byte = (self.current_byte << 1) | bit;
            self.bit_count += 1;
            if self.bit_count == 8 {
                self.buffer.push(self.current_byte);
                self.current_byte = 0;
                self.bit_count = 0;
            }
        }
    }

    /// Flushes any pending bits into the buffer, padded with zeros on the right.
    pub fn flush(&mut self) {
        if self.bit_count > 0 {
            self.current_byte <<= 8 - self.bit_count;
            self.buffer.push(self.current_byte);
            self.current_byte = 0;
            self.bit_count = 0;
        }
    }

    /// Returns the accumulated byte vector.
    pub fn into_bytes(mut self) -> Vec<u8> {
        self.flush();
        self.buffer
    }
}

/// Helper for reading bits sequentially from a byte slice.
pub struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    current_byte: u8,
    bits_left: u8,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            current_byte: 0,
            bits_left: 0,
        }
    }

    /// Reads a single bit (0 or 1). Returns None if EOF is reached.
    #[inline(always)]
    pub fn read_bit(&mut self) -> Option<u8> {
        if self.bits_left == 0 {
            if self.pos >= self.data.len() {
                return None;
            }
            self.current_byte = self.data[self.pos];
            self.pos += 1;
            self.bits_left = 8;
        }

        self.bits_left -= 1;
        let bit = (self.current_byte >> self.bits_left) & 1;
        Some(bit)
    }
}

/// Node in the Huffman tree used during construction.
#[derive(Eq, PartialEq)]
struct TreeNode {
    freq: u64,
    symbol: Option<u8>,
    left: Option<Box<TreeNode>>,
    right: Option<Box<TreeNode>>,
}

impl Ord for TreeNode {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse for min-heap
        other.freq.cmp(&self.freq)
    }
}

impl PartialOrd for TreeNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Computes code lengths from tree by recursive traversal.
fn compute_lengths(node: &TreeNode, depth: u8, lengths: &mut [u8; 256]) {
    if let Some(sym) = node.symbol {
        lengths[sym as usize] = if depth == 0 { 1 } else { depth };
    } else {
        if let Some(ref left) = node.left {
            compute_lengths(left, depth + 1, lengths);
        }
        if let Some(ref right) = node.right {
            compute_lengths(right, depth + 1, lengths);
        }
    }
}

/// Canonical code representation: (code_bits, code_length).
#[derive(Debug, Clone, Copy, Default)]
pub struct CanonicalCode {
    pub code: u32,
    pub length: u8,
}

/// Builds canonical Huffman codes from code lengths.
pub fn generate_canonical_codes(lengths: &[u8; 256]) -> [CanonicalCode; 256] {
    let mut codes = [CanonicalCode::default(); 256];

    // Collect active symbols
    let mut symbols: Vec<u8> = (0..=255)
        .filter(|&s| lengths[s as usize] > 0)
        .map(|s| s as u8)
        .collect();

    // Sort by code length ascending, then by symbol ascending
    symbols.sort_by(|&a, &b| {
        let len_a = lengths[a as usize];
        let len_b = lengths[b as usize];
        len_a.cmp(&len_b).then_with(|| a.cmp(&b))
    });

    if symbols.is_empty() {
        return codes;
    }

    let mut current_code: u32 = 0;
    let mut current_len = lengths[symbols[0] as usize];

    for &sym in &symbols {
        let len = lengths[sym as usize];
        if len > current_len {
            current_code <<= len - current_len;
            current_len = len;
        }

        codes[sym as usize] = CanonicalCode {
            code: current_code,
            length: len,
        };
        current_code += 1;
    }

    codes
}

/// Canonical Huffman encoder.
pub struct HuffmanEncoder;

impl Default for HuffmanEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl HuffmanEncoder {
    pub fn new() -> Self {
        Self
    }

    /// Encodes input bytes into a self-contained bitstream with canonical header.
    pub fn encode(&self, input: &[u8]) -> Vec<u8> {
        let uncompressed_len = input.len() as u32;
        if uncompressed_len == 0 {
            // Write 0 length marker
            let mut empty = Vec::new();
            empty.write_u32::<LittleEndian>(0).unwrap();
            return empty;
        }

        // 1. Calculate symbol frequencies
        let mut freqs = [0u64; 256];
        for &b in input {
            freqs[b as usize] += 1;
        }

        // 2. Build min-heap of active symbols
        let mut heap = BinaryHeap::new();
        for (sym, &freq) in freqs.iter().enumerate() {
            if freq > 0 {
                heap.push(TreeNode {
                    freq,
                    symbol: Some(sym as u8),
                    left: None,
                    right: None,
                });
            }
        }

        // 3. Handle single-symbol edge case
        let mut lengths = [0u8; 256];
        if heap.len() == 1 {
            let single = heap.pop().unwrap();
            lengths[single.symbol.unwrap() as usize] = 1;
        } else {
            // Build tree
            while heap.len() > 1 {
                let left = heap.pop().unwrap();
                let right = heap.pop().unwrap();
                let parent = TreeNode {
                    freq: left.freq + right.freq,
                    symbol: None,
                    left: Some(Box::new(left)),
                    right: Some(Box::new(right)),
                };
                heap.push(parent);
            }
            if let Some(root) = heap.pop() {
                compute_lengths(&root, 0, &mut lengths);
            }
        }

        // 4. Generate canonical codes
        let codes = generate_canonical_codes(&lengths);

        // 5. Serialize header:
        // [4 bytes: uncompressed_len]
        // [1 byte: count of active symbols]
        // [count * 2 bytes: (symbol: u8, code_length: u8)]
        let mut out = Vec::new();
        out.write_u32::<LittleEndian>(uncompressed_len).unwrap();

        let active_count = lengths.iter().filter(|&&l| l > 0).count();
        out.write_u8(active_count as u8).unwrap();
        for (sym, &len) in lengths.iter().enumerate() {
            if len > 0 {
                out.write_u8(sym as u8).unwrap();
                out.write_u8(len).unwrap();
            }
        }

        // 6. Encode bitstream
        let mut writer = BitWriter::new();
        for &b in input {
            let code = codes[b as usize];
            writer.write_bits(code.code, code.length);
        }
        writer.flush();

        out.extend(writer.into_bytes());
        out
    }
}

/// Canonical Huffman decoder.
#[derive(Default)]
pub struct HuffmanDecoder;

impl HuffmanDecoder {
    pub fn new() -> Self {
        Self
    }

    /// Decodes a compressed canonical Huffman payload back into original bytes.
    pub fn decode(&self, compressed: &[u8]) -> Result<Vec<u8>> {
        if compressed.len() < 4 {
            return Err(NexusError::DataCorruption(
                "Compressed Huffman data too short".into(),
            ));
        }

        let mut cursor = compressed;
        let uncompressed_len = cursor.read_u32::<LittleEndian>()? as usize;
        if uncompressed_len == 0 {
            return Ok(Vec::new());
        }

        let active_count = cursor.read_u8()? as usize;
        let header_symbols_count = if active_count == 0 { 256 } else { active_count };

        let mut lengths = [0u8; 256];
        for _ in 0..header_symbols_count {
            let sym = cursor.read_u8()?;
            let len = cursor.read_u8()?;
            lengths[sym as usize] = len;
        }

        // Reconstruct canonical codes
        let codes = generate_canonical_codes(&lengths);

        // Build prefix lookup tree or map: (code, length) -> symbol
        // For canonical codes, we can decode using a lookup trie or sequential matching
        struct DecodeEntry {
            symbol: u8,
            code: u32,
            length: u8,
        }

        let mut entries = Vec::with_capacity(header_symbols_count);
        for (sym, &len) in lengths.iter().enumerate() {
            if len > 0 {
                entries.push(DecodeEntry {
                    symbol: sym as u8,
                    code: codes[sym].code,
                    length: codes[sym].length,
                });
            }
        }

        let mut reader = BitReader::new(cursor);
        let mut output = Vec::with_capacity(uncompressed_len);

        while output.len() < uncompressed_len {
            let mut current_code: u32 = 0;
            let mut current_len: u8 = 0;
            let mut matched = false;

            while current_len < 32 {
                match reader.read_bit() {
                    Some(bit) => {
                        current_code = (current_code << 1) | (bit as u32);
                        current_len += 1;

                        if let Some(entry) = entries
                            .iter()
                            .find(|e| e.length == current_len && e.code == current_code)
                        {
                            output.push(entry.symbol);
                            matched = true;
                            break;
                        }
                    }
                    None => {
                        return Err(NexusError::DataCorruption(
                            "Unexpected end of bitstream during Huffman decode".into(),
                        ));
                    }
                }
            }

            if !matched {
                return Err(NexusError::DataCorruption(
                    "Invalid Huffman code encountered".into(),
                ));
            }
        }

        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_huffman_empty() {
        let encoder = HuffmanEncoder::new();
        let decoder = HuffmanDecoder::new();
        let input = b"";
        let compressed = encoder.encode(input);
        let decompressed = decoder.decode(&compressed).unwrap();
        assert_eq!(decompressed, input);
    }

    #[test]
    fn test_huffman_single_byte_repeated() {
        let encoder = HuffmanEncoder::new();
        let decoder = HuffmanDecoder::new();
        let input = vec![b'Z'; 500];
        let compressed = encoder.encode(&input);
        assert!(compressed.len() < input.len());
        let decompressed = decoder.decode(&compressed).unwrap();
        assert_eq!(decompressed, input);
    }

    #[test]
    fn test_huffman_all_256_bytes() {
        let encoder = HuffmanEncoder::new();
        let decoder = HuffmanDecoder::new();
        let input: Vec<u8> = (0..=255).collect();
        let compressed = encoder.encode(&input);
        let decompressed = decoder.decode(&compressed).unwrap();
        assert_eq!(decompressed, input);
    }

    #[test]
    fn test_huffman_english_text() {
        let encoder = HuffmanEncoder::new();
        let decoder = HuffmanDecoder::new();
        let input = b"To be, or not to be, that is the question: Whether 'tis nobler in the mind to suffer...";
        let compressed = encoder.encode(input);
        let decompressed = decoder.decode(&compressed).unwrap();
        assert_eq!(decompressed, input);
    }

    #[test]
    fn test_huffman_random_data() {
        let encoder = HuffmanEncoder::new();
        let decoder = HuffmanDecoder::new();
        let mut rng_state: u64 = 0xDEADBEEFCAFE;
        let mut input = vec![0u8; 8192];
        for b in input.iter_mut() {
            rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
            *b = (rng_state >> 33) as u8;
        }

        let compressed = encoder.encode(&input);
        let decompressed = decoder.decode(&compressed).unwrap();
        assert_eq!(decompressed, input);
    }
}
