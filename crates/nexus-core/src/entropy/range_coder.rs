//! Binary range coder with 16-bit probabilities (Layer 3).
//!
//! Codes one bit at a time given an integer probability that the bit is 1.
//! Unlike `arithmetic.rs` (12-bit probabilities), 16 bits of precision let a
//! well-trained model spend as little as ~0.0001 bit on a near-certain
//! symbol, which matters for the highly predictable output of a BWT.
//!
//! The coder keeps a 32-bit interval `[x1, x2]` and emits a byte whenever the
//! two bounds agree in their top byte (carry-less, as popularised by lpaq).
//! All arithmetic is integer; encoder and decoder are bit-exact mirrors.

/// Probability scale: probabilities are in `1..PROB_ONE`.
pub const PROB_BITS: u32 = 16;
/// Exclusive upper bound of a probability value.
pub const PROB_ONE: u32 = 1 << PROB_BITS;

/// Bit-wise range encoder appending to an owned buffer.
pub struct BinEncoder {
    x1: u32,
    x2: u32,
    out: Vec<u8>,
}

impl BinEncoder {
    /// Creates an encoder that appends to `out`.
    pub fn new(out: Vec<u8>) -> Self {
        Self {
            x1: 0,
            x2: u32::MAX,
            out,
        }
    }

    /// Encodes `bit` (0 or 1) given `p1`, the probability that it is 1,
    /// scaled to [`PROB_ONE`]. `p1` must be in `1..PROB_ONE`.
    #[inline(always)]
    pub fn encode(&mut self, bit: u32, p1: u32) {
        debug_assert!(p1 > 0 && p1 < PROB_ONE);
        let xmid = self.x1 + ((self.x2 - self.x1) >> PROB_BITS) * p1;
        if bit != 0 {
            self.x2 = xmid;
        } else {
            self.x1 = xmid + 1;
        }
        while (self.x1 ^ self.x2) & 0xFF00_0000 == 0 {
            self.out.push((self.x2 >> 24) as u8);
            self.x1 <<= 8;
            self.x2 = (self.x2 << 8) | 0xFF;
        }
    }

    /// Flushes the final interval and returns the buffer.
    pub fn finish(mut self) -> Vec<u8> {
        self.out.extend_from_slice(&self.x1.to_be_bytes());
        self.out
    }
}

/// Bit-wise range decoder over a byte slice. Reading past the end supplies
/// zero bytes, so corrupt input yields wrong bits rather than a panic.
pub struct BinDecoder<'a> {
    x1: u32,
    x2: u32,
    x: u32,
    data: &'a [u8],
    pos: usize,
}

impl<'a> BinDecoder<'a> {
    /// Creates a decoder positioned at the start of `data`.
    pub fn new(data: &'a [u8]) -> Self {
        let mut d = Self {
            x1: 0,
            x2: u32::MAX,
            x: 0,
            data,
            pos: 0,
        };
        for _ in 0..4 {
            d.x = (d.x << 8) | d.next_byte();
        }
        d
    }

    #[inline(always)]
    fn next_byte(&mut self) -> u32 {
        let b = self.data.get(self.pos).copied().unwrap_or(0);
        self.pos += 1;
        b as u32
    }

    /// Decodes one bit given `p1` (see [`BinEncoder::encode`]).
    #[inline(always)]
    pub fn decode(&mut self, p1: u32) -> u32 {
        debug_assert!(p1 > 0 && p1 < PROB_ONE);
        let xmid = self.x1 + ((self.x2 - self.x1) >> PROB_BITS) * p1;
        let bit = (self.x <= xmid) as u32;
        if bit != 0 {
            self.x2 = xmid;
        } else {
            self.x1 = xmid + 1;
        }
        while (self.x1 ^ self.x2) & 0xFF00_0000 == 0 {
            self.x1 <<= 8;
            self.x2 = (self.x2 << 8) | 0xFF;
            self.x = (self.x << 8) | self.next_byte();
        }
        bit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_range_coder_roundtrip_varied_probabilities() {
        let mut s = 0x0123_4567_89AB_CDEFu64;
        let mut items = Vec::new();
        for _ in 0..200_000 {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1);
            let p = 1 + ((s >> 20) as u32 % (PROB_ONE - 1));
            // Draw the bit with roughly the stated probability.
            let bit = (((s >> 40) as u32 % PROB_ONE) < p) as u32;
            items.push((bit, p));
        }
        let mut enc = BinEncoder::new(Vec::new());
        for &(bit, p) in &items {
            enc.encode(bit, p);
        }
        let bytes = enc.finish();
        let mut dec = BinDecoder::new(&bytes);
        for &(bit, p) in &items {
            assert_eq!(dec.decode(p), bit);
        }
    }

    #[test]
    fn test_range_coder_extreme_probabilities() {
        // Near-certain bits, including the rare "wrong" outcome.
        let mut enc = BinEncoder::new(Vec::new());
        let mut items = Vec::new();
        for i in 0..50_000u32 {
            let p = if i % 2 == 0 { 1 } else { PROB_ONE - 1 };
            let bit = if i % 997 == 0 {
                i & 1
            } else {
                (i % 2 == 1) as u32
            };
            items.push((bit, p));
            enc.encode(bit, p);
        }
        let bytes = enc.finish();
        // Mostly-predictable input must cost far less than one bit per bit.
        assert!(bytes.len() < 50_000 / 8 / 4);
        let mut dec = BinDecoder::new(&bytes);
        for &(bit, p) in &items {
            assert_eq!(dec.decode(p), bit);
        }
    }

    #[test]
    fn test_range_decoder_survives_truncated_input() {
        let mut dec = BinDecoder::new(&[0x12]);
        for _ in 0..1000 {
            dec.decode(PROB_ONE / 2);
        }
    }
}
