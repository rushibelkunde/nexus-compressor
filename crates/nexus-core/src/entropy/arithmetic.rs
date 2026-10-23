//! Bit-level integer range/arithmetic coder (Layer 3).
//!
//! Uses a 12-bit fixed-point probability scale [1, 4095] with pure integer arithmetic
//! (Subbotin range coder). Zero floating-point math, 100% deterministic across all platforms.

/// Default top range value.
const TOP_RANGE: u32 = 0xFFFF_FFFF;
/// Threshold below which range must be normalized (2^24).
const NORM_THRESHOLD: u32 = 0x0100_0000;

/// Bit-level arithmetic / range encoder.
pub struct ArithmeticEncoder {
    low: u64,
    range: u32,
    cache: u8,
    cache_size: u64,
}

impl Default for ArithmeticEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl ArithmeticEncoder {
    /// Creates a new arithmetic encoder.
    pub fn new() -> Self {
        Self {
            low: 0,
            range: TOP_RANGE,
            cache: 0,
            cache_size: 1,
        }
    }

    /// Encodes a single bit given P(bit = 1) in [1, 4095].
    #[inline(always)]
    pub fn encode_bit(&mut self, bit: u8, p1: u16, output: &mut Vec<u8>) {
        let p0 = 4096 - p1 as u32;
        let bound = ((self.range as u64 * p0 as u64) >> 12) as u32;

        if bit == 0 {
            self.range = bound;
        } else {
            self.low += bound as u64;
            self.range -= bound;
        }

        while self.range < NORM_THRESHOLD {
            self.shift_low(output);
            self.range <<= 8;
        }
    }

    #[inline(always)]
    fn shift_low(&mut self, output: &mut Vec<u8>) {
        let carry = (self.low >> 32) as u32;
        let lead = (self.low >> 24) as u8;

        if lead != 0xFF || carry > 0 {
            let out_byte = self.cache.wrapping_add(carry as u8);
            output.push(out_byte);
            let fill = if carry > 0 { 0x00 } else { 0xFF };
            for _ in 0..(self.cache_size - 1) {
                output.push(fill);
            }
            self.cache = lead;
            self.cache_size = 1;
        } else {
            self.cache_size += 1;
        }

        self.low = (self.low & 0x00FF_FFFF) << 8;
    }

    /// Flushes remaining bits into the output buffer.
    pub fn finish(&mut self, output: &mut Vec<u8>) {
        for _ in 0..5 {
            self.shift_low(output);
        }
    }
}

/// Bit-level arithmetic / range decoder.
pub struct ArithmeticDecoder<'a> {
    code: u32,
    range: u32,
    input: &'a [u8],
    pos: usize,
}

impl<'a> ArithmeticDecoder<'a> {
    /// Initializes decoder from a compressed byte slice.
    pub fn new(input: &'a [u8]) -> Self {
        let mut decoder = Self {
            code: 0,
            range: TOP_RANGE,
            input,
            pos: 0,
        };

        // Skip leading dummy byte from encoder initialization
        if decoder.pos < decoder.input.len() {
            decoder.pos += 1;
        }

        for _ in 0..4 {
            let b = decoder.read_byte();
            decoder.code = (decoder.code << 8) | (b as u32);
        }

        decoder
    }

    #[inline(always)]
    fn read_byte(&mut self) -> u8 {
        if self.pos < self.input.len() {
            let b = self.input[self.pos];
            self.pos += 1;
            b
        } else {
            0
        }
    }

    /// Decodes a single bit given P(bit = 1) in [1, 4095].
    #[inline(always)]
    pub fn decode_bit(&mut self, p1: u16) -> u8 {
        let p0 = 4096 - p1 as u32;
        let bound = ((self.range as u64 * p0 as u64) >> 12) as u32;

        let bit = if self.code < bound {
            self.range = bound;
            0
        } else {
            self.code -= bound;
            self.range -= bound;
            1
        };

        while self.range < NORM_THRESHOLD {
            self.code = (self.code << 8) | (self.read_byte() as u32);
            self.range <<= 8;
        }

        bit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_arithmetic_single_bit() {
        let mut encoder = ArithmeticEncoder::new();
        let mut buffer = Vec::new();
        encoder.encode_bit(1, 2048, &mut buffer);
        encoder.finish(&mut buffer);

        let mut decoder = ArithmeticDecoder::new(&buffer);
        let bit = decoder.decode_bit(2048);
        assert_eq!(bit, 1);
    }

    #[test]
    fn test_arithmetic_all_zeros() {
        let count = 1000;
        let p1 = 100; // 0 is very likely
        let mut encoder = ArithmeticEncoder::new();
        let mut buffer = Vec::new();

        for _ in 0..count {
            encoder.encode_bit(0, p1, &mut buffer);
        }
        encoder.finish(&mut buffer);

        assert!(
            buffer.len() < 50,
            "1000 highly predictable zeros should compress to few bytes"
        );

        let mut decoder = ArithmeticDecoder::new(&buffer);
        for _ in 0..count {
            let bit = decoder.decode_bit(p1);
            assert_eq!(bit, 0);
        }
    }

    #[test]
    fn test_arithmetic_all_ones() {
        let count = 1000;
        let p1 = 3995; // 1 is very likely
        let mut encoder = ArithmeticEncoder::new();
        let mut buffer = Vec::new();

        for _ in 0..count {
            encoder.encode_bit(1, p1, &mut buffer);
        }
        encoder.finish(&mut buffer);

        assert!(
            buffer.len() < 50,
            "1000 highly predictable ones should compress to few bytes"
        );

        let mut decoder = ArithmeticDecoder::new(&buffer);
        for _ in 0..count {
            let bit = decoder.decode_bit(p1);
            assert_eq!(bit, 1);
        }
    }

    #[test]
    fn test_arithmetic_alternating_bits_varying_probabilities() {
        let bits = [0, 1, 0, 0, 1, 1, 1, 0, 1, 0, 1, 1, 0, 0, 0, 1];
        let probs = [
            500, 3500, 1000, 2048, 2048, 3000, 4000, 200, 2500, 1800, 3100, 2200, 800, 400, 1500,
            2900,
        ];

        let mut encoder = ArithmeticEncoder::new();
        let mut buffer = Vec::new();

        for (&bit, &p1) in bits.iter().zip(probs.iter()) {
            encoder.encode_bit(bit, p1, &mut buffer);
        }
        encoder.finish(&mut buffer);

        let mut decoder = ArithmeticDecoder::new(&buffer);
        for (&expected, &p1) in bits.iter().zip(probs.iter()) {
            let actual = decoder.decode_bit(p1);
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn test_arithmetic_pseudo_random_stream() {
        let mut rng: u64 = 0x123456789ABCDE;
        let mut bits = Vec::with_capacity(10_000);
        let mut probs = Vec::with_capacity(10_000);

        for _ in 0..10_000 {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            let p1 = ((rng >> 32) % 4094 + 1) as u16; // [1, 4094]
            let bit = ((rng >> 16) & 1) as u8;
            bits.push(bit);
            probs.push(p1);
        }

        let mut encoder = ArithmeticEncoder::new();
        let mut buffer = Vec::new();

        for i in 0..bits.len() {
            encoder.encode_bit(bits[i], probs[i], &mut buffer);
        }
        encoder.finish(&mut buffer);

        let mut decoder = ArithmeticDecoder::new(&buffer);
        for i in 0..bits.len() {
            let actual = decoder.decode_bit(probs[i]);
            assert_eq!(actual, bits[i], "Mismatch at bit index {}", i);
        }
    }
}
