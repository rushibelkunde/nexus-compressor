//! Range Asymmetric Numeral Systems (rANS) entropy coder.
//!
//! Implements order-0 byte rANS with 12-bit precision (M = 4096).
//! Provides near-optimal arithmetic-coding compression ratio at high speeds.

use crate::error::{NexusError, Result};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::cmp;
use std::io::Read;

/// Scale bits for rANS probability precision (12 bits).
pub const RANS_SCALE_BITS: u32 = 12;
/// Total frequency scale M = 4096 (1 << 12).
pub const RANS_SCALE: u32 = 1 << RANS_SCALE_BITS;
pub const RANS_SCALE_MASK: u32 = RANS_SCALE - 1;

/// Lower bound of the rANS state (1 << 23).
pub const RANS_L: u32 = 1 << 23;

/// Frequency distribution table for rANS.
#[derive(Clone)]
pub struct RansFreqTable {
    pub freqs: [u16; 256],
    pub cum: [u16; 257],
    pub slot_to_sym: Vec<u8>,
}

impl RansFreqTable {
    /// Builds and normalizes a frequency table from input bytes.
    /// Ensures every symbol present in the input has freq >= 1 and sum == 4096.
    pub fn build(data: &[u8]) -> Self {
        let mut raw_counts = [0u64; 256];
        for &b in data {
            raw_counts[b as usize] += 1;
        }

        let mut distinct = 0u32;
        for &c in &raw_counts {
            if c > 0 {
                distinct += 1;
            }
        }

        let mut freqs = [0u16; 256];
        if distinct == 0 {
            return Self::from_normalized(freqs);
        }

        let total_bytes = data.len() as u64;

        // Reserve 1 for each distinct symbol, distribute remaining 4096 - distinct
        let pool = RANS_SCALE - distinct;
        let mut allocated_sum = 0u32;
        let mut max_sym = 0;
        let mut max_count = 0;

        for (sym, &c) in raw_counts.iter().enumerate() {
            if c > 0 {
                let share = ((c * pool as u64) / total_bytes) as u32;
                let f = (1 + share) as u16;
                freqs[sym] = f;
                allocated_sum += f as u32;

                if c > max_count {
                    max_count = c;
                    max_sym = sym;
                }
            }
        }

        // Adjust rounding errors so sum is exactly RANS_SCALE
        if allocated_sum < RANS_SCALE {
            let diff = (RANS_SCALE - allocated_sum) as u16;
            freqs[max_sym] += diff;
        } else if allocated_sum > RANS_SCALE {
            let mut diff = (allocated_sum - RANS_SCALE) as u16;
            for f in &mut freqs {
                if diff == 0 {
                    break;
                }
                if *f > 1 {
                    let take = cmp::min(*f - 1, diff);
                    *f -= take;
                    diff -= take;
                }
            }
        }

        Self::from_normalized(freqs)
    }

    /// Constructs cumulative table and lookup index from normalized frequencies.
    pub fn from_normalized(freqs: [u16; 256]) -> Self {
        let mut cum = [0u16; 257];
        let mut c = 0u16;
        for i in 0..256 {
            cum[i] = c;
            c += freqs[i];
        }
        cum[256] = c;

        // Build slot lookup table for O(1) decoding
        let mut slot_to_sym = vec![0u8; RANS_SCALE as usize];
        for sym in 0..256 {
            let start = cum[sym] as usize;
            let end = cum[sym + 1] as usize;
            slot_to_sym[start..end].fill(sym as u8);
        }

        Self {
            freqs,
            cum,
            slot_to_sym,
        }
    }

    /// Serializes normalized frequency table into bytes.
    pub fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let active: Vec<u8> = (0..=255)
            .filter(|&s| self.freqs[s as usize] > 0)
            .map(|s| s as u8)
            .collect();

        out.write_u8(active.len() as u8).unwrap();
        for &sym in &active {
            out.write_u8(sym).unwrap();
            out.write_u16::<LittleEndian>(self.freqs[sym as usize])
                .unwrap();
        }
        out
    }

    /// Deserializes normalized frequency table from reader.
    pub fn deserialize<R: Read>(mut reader: R) -> Result<Self> {
        let active_count = reader.read_u8()? as usize;
        let count = if active_count == 0 { 256 } else { active_count };

        let mut freqs = [0u16; 256];
        for _ in 0..count {
            let sym = reader.read_u8()?;
            let f = reader.read_u16::<LittleEndian>()?;
            freqs[sym as usize] = f;
        }

        Ok(Self::from_normalized(freqs))
    }
}

/// Order-0 rANS encoder.
pub struct RansEncoder;

impl Default for RansEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl RansEncoder {
    pub fn new() -> Self {
        Self
    }

    /// Encodes a slice of bytes using rANS into a self-contained byte payload.
    pub fn encode(&self, input: &[u8]) -> Vec<u8> {
        if input.is_empty() {
            let mut empty = Vec::new();
            empty.write_u32::<LittleEndian>(0).unwrap();
            return empty;
        }

        let table = RansFreqTable::build(input);
        let mut state = RANS_L;
        let mut stream = Vec::with_capacity(input.len());

        // Process symbols in REVERSE order
        for &sym in input.iter().rev() {
            let f = table.freqs[sym as usize] as u32;
            let c = table.cum[sym as usize] as u32;

            // Renormalize: while state >= x_max, emit byte
            // x_max = ((RANS_L >> RANS_SCALE_BITS) << 8) * f
            let x_max = ((RANS_L >> RANS_SCALE_BITS) << 8) * f;
            while state >= x_max {
                stream.push((state & 0xFF) as u8);
                state >>= 8;
            }

            // State transition
            state = ((state / f) << RANS_SCALE_BITS) + (state % f) + c;
        }

        // Assemble final payload:
        // [4 bytes: uncompressed length]
        // [header: frequency table]
        // [4 bytes: final rANS state]
        // [rANS stream reversed so decoder reads forward]
        let mut output = Vec::with_capacity(stream.len() + 64);
        output
            .write_u32::<LittleEndian>(input.len() as u32)
            .unwrap();
        output.extend_from_slice(&table.serialize());
        output.write_u32::<LittleEndian>(state).unwrap();

        // Reverse the stream for forward reading by decoder
        stream.reverse();
        output.extend_from_slice(&stream);

        output
    }
}

/// Order-0 rANS decoder.
#[derive(Default)]
pub struct RansDecoder;

impl RansDecoder {
    pub fn new() -> Self {
        Self
    }

    /// Decodes a rANS payload into original uncompressed bytes.
    pub fn decode(&self, compressed: &[u8]) -> Result<Vec<u8>> {
        if compressed.len() < 4 {
            return Err(NexusError::DataCorruption("rANS input too short".into()));
        }

        let mut cursor = compressed;
        let uncompressed_len = cursor.read_u32::<LittleEndian>()? as usize;
        if uncompressed_len == 0 {
            return Ok(Vec::new());
        }

        let table = RansFreqTable::deserialize(&mut cursor)?;
        let mut state = cursor.read_u32::<LittleEndian>()?;

        let mut output = Vec::with_capacity(uncompressed_len);
        let mut byte_idx = 0;
        let stream = cursor;

        for _ in 0..uncompressed_len {
            // 1. Symbol from slot
            let slot = (state & RANS_SCALE_MASK) as usize;
            let sym = table.slot_to_sym[slot];
            output.push(sym);

            // 2. State transition
            let f = table.freqs[sym as usize] as u32;
            let c = table.cum[sym as usize] as u32;
            state = f * (state >> RANS_SCALE_BITS) + (state & RANS_SCALE_MASK) - c;

            // 3. Renormalize: while state < RANS_L, refill byte
            while state < RANS_L && byte_idx < stream.len() {
                state = (state << 8) | (stream[byte_idx] as u32);
                byte_idx += 1;
            }
        }

        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rans_empty() {
        let encoder = RansEncoder::new();
        let decoder = RansDecoder::new();
        let input = b"";
        let compressed = encoder.encode(input);
        let decompressed = decoder.decode(&compressed).unwrap();
        assert_eq!(decompressed, input);
    }

    #[test]
    fn test_rans_single_symbol_repeated() {
        let encoder = RansEncoder::new();
        let decoder = RansDecoder::new();
        let input = vec![b'A'; 2000];
        let compressed = encoder.encode(&input);
        assert!(compressed.len() < input.len() / 5);
        let decompressed = decoder.decode(&compressed).unwrap();
        assert_eq!(decompressed, input);
    }

    #[test]
    fn test_rans_all_256_symbols() {
        let encoder = RansEncoder::new();
        let decoder = RansDecoder::new();
        let input: Vec<u8> = (0..=255).collect();
        let compressed = encoder.encode(&input);
        let decompressed = decoder.decode(&compressed).unwrap();
        assert_eq!(decompressed, input);
    }

    #[test]
    fn test_rans_english_text() {
        let encoder = RansEncoder::new();
        let decoder = RansDecoder::new();
        let input = b"Compression is the science of removing redundancy from data. rANS provides optimal entropy coding.";
        let compressed = encoder.encode(input);
        let decompressed = decoder.decode(&compressed).unwrap();
        assert_eq!(decompressed, input);
    }

    #[test]
    fn test_rans_random_data() {
        let encoder = RansEncoder::new();
        let decoder = RansDecoder::new();
        let mut rng_state: u64 = 0x987654321ABCDEF;
        let mut input = vec![0u8; 16384];
        for b in input.iter_mut() {
            rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
            *b = (rng_state >> 33) as u8;
        }

        let compressed = encoder.encode(&input);
        let decompressed = decoder.decode(&compressed).unwrap();
        assert_eq!(decompressed, input);
    }
}
