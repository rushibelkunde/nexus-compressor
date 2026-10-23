//! Multi-context neural feature extraction (Layer 3).
//!
//! Extracts byte unigrams, bigrams, bit prefixes, sparse distances, and word
//! tokens into compact sparse feature indices for fixed-point neural prediction.

/// Total number of sparse feature representations extracted per bit.
pub const NUM_FEATURES: usize = 8;

/// Bucket size allocated for each feature partition.
pub const PARTITION_SIZE: usize = 256;

/// Total input dimension across all feature partitions (8 * 256 = 2048).
pub const TOTAL_FEATURES: usize = NUM_FEATURES * PARTITION_SIZE;

/// Multi-context feature extractor tracking data history and bit positions.
#[derive(Debug, Clone)]
pub struct FeatureExtractor {
    /// Rolling history of past bytes: [b_{-1}, b_{-2}, ..., b_{-8}].
    history: [u8; 8],
    /// Current bit position within byte (7 down to 0).
    pub bit_pos: u8,
    /// Tree prefix of bits processed so far in the current byte (1..255).
    pub bit_prefix: u16,
    /// Rolling hash of alphanumeric text words.
    pub word_hash: u32,
}

impl Default for FeatureExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl FeatureExtractor {
    /// Creates a new feature extractor initialized to zero state.
    pub fn new() -> Self {
        Self {
            history: [0u8; 8],
            bit_pos: 7,
            bit_prefix: 1,
            word_hash: 0,
        }
    }

    /// Resets extractor state to initial defaults.
    pub fn reset(&mut self) {
        self.history = [0u8; 8];
        self.bit_pos = 7;
        self.bit_prefix = 1;
        self.word_hash = 0;
    }

    /// Returns previous byte $b_{-1}$.
    #[inline(always)]
    pub fn prev_byte(&self) -> u8 {
        self.history[0]
    }

    /// Returns previous byte $b_{-2}$.
    #[inline(always)]
    pub fn prev_byte2(&self) -> u8 {
        self.history[1]
    }

    /// Extracts active sparse feature indices for the current bit context.
    ///
    /// Each feature maps into its dedicated 256-bucket partition to prevent
    /// collisions between different context types.
    #[inline(always)]
    pub fn extract_features(&self) -> [usize; NUM_FEATURES] {
        let b1 = self.history[0];
        let b2 = self.history[1];
        let b4 = self.history[3];
        let b8 = self.history[7];

        let prefix_byte = (self.bit_prefix & 0xFF) as u8;
        let bpos = self.bit_pos;

        // 0. Bit position and prefix within current byte (0..255)
        let f0 = (prefix_byte as usize) & 0xFF;

        // 1. Order-1 context: (b_{-1}, bit_prefix)
        let f1 = (b1.wrapping_mul(33) ^ prefix_byte) as usize & 0xFF;

        // 2. Order-2 context: (b_{-2}, b_{-1}, bit_prefix)
        let bigram = ((b2 as usize) << 8) | (b1 as usize);
        let f2 = (bigram.wrapping_mul(65599) ^ (prefix_byte as usize)) & 0xFF;

        // 3. Sparse distance 4: (b_{-4}, bit_prefix)
        let f3 = (b4.wrapping_mul(31) ^ prefix_byte) as usize & 0xFF;

        // 4. Sparse distance 8: (b_{-8}, bit_pos)
        let f4 = (b8.wrapping_mul(37) ^ bpos) as usize & 0xFF;

        // 5. Word token hash: (word_hash, bit_prefix)
        let f5 = (self.word_hash.wrapping_mul(31) ^ (prefix_byte as u32)) as usize & 0xFF;

        // 6. Character class: (is_alphanumeric, bit_pos, bit_prefix)
        let is_alpha = (b1.is_ascii_alphanumeric() || b1 == b' ') as usize;
        let f6 =
            ((is_alpha << 6) | ((bpos as usize) << 3) | ((prefix_byte as usize) & 0x07)) & 0xFF;

        // 7. Byte bigram token: (b_{-2}, b_{-1})
        let f7 = ((b2 as usize).wrapping_mul(17) ^ (b1 as usize)) & 0xFF;

        [
            f0,
            PARTITION_SIZE + f1,
            PARTITION_SIZE * 2 + f2,
            PARTITION_SIZE * 3 + f3,
            PARTITION_SIZE * 4 + f4,
            PARTITION_SIZE * 5 + f5,
            PARTITION_SIZE * 6 + f6,
            PARTITION_SIZE * 7 + f7,
        ]
    }

    /// Observes a single bit and updates context state.
    ///
    /// When 8 bits complete a byte, automatically shifts the history window
    /// and updates word tokens.
    #[inline(always)]
    pub fn observe_bit(&mut self, bit: u8) {
        let b = (bit & 1) as u16;
        self.bit_prefix = (self.bit_prefix << 1) | b;

        if self.bit_pos == 0 {
            // Byte finished
            let completed_byte = (self.bit_prefix & 0xFF) as u8;
            self.observe_byte(completed_byte);
            self.bit_prefix = 1;
            self.bit_pos = 7;
        } else {
            self.bit_pos -= 1;
        }
    }

    /// Updates byte history buffer and rolling word hash.
    #[inline(always)]
    pub fn observe_byte(&mut self, byte: u8) {
        // Shift history: [b_{-1}, b_{-2}, ..., b_{-8}]
        self.history[7] = self.history[6];
        self.history[6] = self.history[5];
        self.history[5] = self.history[4];
        self.history[4] = self.history[3];
        self.history[3] = self.history[2];
        self.history[2] = self.history[1];
        self.history[1] = self.history[0];
        self.history[0] = byte;

        // Update word hash
        if byte.is_ascii_alphanumeric() || byte == b'_' {
            self.word_hash = self.word_hash.wrapping_mul(31).wrapping_add(byte as u32);
        } else {
            self.word_hash = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feature_indices_in_bounds() {
        let mut extractor = FeatureExtractor::new();
        let sample = b"NEXUS compression engine 2026";

        for &byte in sample {
            for bit_idx in 0..8 {
                let bit = (byte >> (7 - bit_idx)) & 1;
                let features = extractor.extract_features();

                assert_eq!(features.len(), NUM_FEATURES);
                for (i, &feat) in features.iter().enumerate() {
                    let min_bound = i * PARTITION_SIZE;
                    let max_bound = (i + 1) * PARTITION_SIZE;
                    assert!(
                        feat >= min_bound && feat < max_bound,
                        "Feature {} out of range: {} not in [{}, {})",
                        i,
                        feat,
                        min_bound,
                        max_bound
                    );
                }

                extractor.observe_bit(bit);
            }
        }
    }

    #[test]
    fn test_distinct_contexts_produce_different_features() {
        let mut ext1 = FeatureExtractor::new();
        let mut ext2 = FeatureExtractor::new();

        // ext1 fed ASCII 'A'
        ext1.observe_byte(b'A');

        // ext2 fed binary 0xFF
        ext2.observe_byte(0xFF);

        let f1 = ext1.extract_features();
        let f2 = ext2.extract_features();

        // Features depending on b_{-1} must differ
        assert_ne!(f1[1], f2[1], "Order-1 feature must differ");
        assert_ne!(f1[6], f2[6], "Char class feature must differ");
    }

    #[test]
    fn test_bit_stream_byte_accumulation() {
        let mut extractor = FeatureExtractor::new();
        let byte = 0b1011_0010u8;

        for bit_idx in 0..8 {
            let bit = (byte >> (7 - bit_idx)) & 1;
            extractor.observe_bit(bit);
        }

        assert_eq!(extractor.prev_byte(), byte);
        assert_eq!(extractor.bit_pos, 7);
        assert_eq!(extractor.bit_prefix, 1);
    }
}
