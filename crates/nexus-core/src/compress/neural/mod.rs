//! Neural network compression engine (Path D, Layer 3).
//!
//! Employs online integer backpropagation and fixed-point neural prediction
//! combined with bit-level arithmetic coding for maximum compression ratio.

pub mod features;
pub mod layer;

use crate::compress::mixer::tables::squash;
use crate::compress::neural::features::{FeatureExtractor, NUM_FEATURES, TOTAL_FEATURES};
use crate::compress::neural::layer::DenseLayer;
use crate::entropy::arithmetic::{ArithmeticDecoder, ArithmeticEncoder};
use crate::error::Result;

/// Hidden dimension for the neural predictor.
const HIDDEN_DIM: usize = 16;

/// Bit-level neural probability predictor with online integer SGD.
pub struct NeuralPredictor {
    extractor: FeatureExtractor,
    hidden: DenseLayer,
    output_weights: [i16; HIDDEN_DIM],
    output_bias: i32,
    last_active_features: [usize; NUM_FEATURES],
    last_p1: u16,
    pub lr_shift: u32,
}

impl Default for NeuralPredictor {
    fn default() -> Self {
        Self::new()
    }
}

impl NeuralPredictor {
    /// Creates a new neural predictor initialized with deterministic weights.
    pub fn new() -> Self {
        let mut output_weights = [0i16; HIDDEN_DIM];
        // Symmetric initial output weights
        for (i, w) in output_weights.iter_mut().enumerate() {
            *w = if i % 2 == 0 { 64 } else { -64 };
        }

        Self {
            extractor: FeatureExtractor::new(),
            hidden: DenseLayer::new(TOTAL_FEATURES, HIDDEN_DIM),
            output_weights,
            output_bias: 0,
            last_active_features: [0; NUM_FEATURES],
            last_p1: 2048,
            lr_shift: 5,
        }
    }

    /// Predicts P(bit = 1) in [1, 4095] for the current context.
    #[inline(always)]
    pub fn predict(&mut self) -> u16 {
        self.last_active_features = self.extractor.extract_features();
        let h = self.hidden.forward_sparse(&self.last_active_features, 256);

        let mut logit_acc: i64 = self.output_bias as i64;
        for (j, &hj) in h.iter().enumerate().take(HIDDEN_DIM) {
            logit_acc += ((hj as i64) * (self.output_weights[j] as i64)) >> 8;
        }

        let logit = logit_acc.clamp(-2048, 2048) as i32;
        let p1 = squash(logit);
        self.last_p1 = p1;
        p1
    }

    /// Online integer backpropagation training step with observed bit.
    #[inline(always)]
    pub fn update(&mut self, bit: u8) {
        let target = if bit == 1 { 4095i32 } else { 0i32 };
        let err = target - (self.last_p1 as i32);

        // 1. Update output bias and weights
        let delta_bias = err >> self.lr_shift;
        self.output_bias = self
            .output_bias
            .saturating_add(delta_bias)
            .clamp(-1_000_000, 1_000_000);

        let mut grad_hidden = [0i32; HIDDEN_DIM];
        for (j, gh) in grad_hidden.iter_mut().enumerate().take(HIDDEN_DIM) {
            let hj = self.hidden.outputs[j] as i64;
            let delta_w = ((err as i64 * hj) >> (self.lr_shift + 8)) as i16;
            self.output_weights[j] = self.output_weights[j]
                .saturating_add(delta_w)
                .clamp(-16384, 16384);

            let w_out = self.output_weights[j] as i64;
            *gh = ((err as i64 * w_out) >> 12) as i32;
        }

        // 2. Backpropagate into hidden layer
        self.hidden
            .backward_sparse(&self.last_active_features, 256, &grad_hidden, self.lr_shift);

        // 3. Update feature extractor context state
        self.extractor.observe_bit(bit);
    }
}

/// Compresses raw input bytes using Path D (Neural Network + Arithmetic Coding).
pub fn neural_compress(data: &[u8]) -> Vec<u8> {
    if data.is_empty() {
        return Vec::new();
    }

    let mut encoder = ArithmeticEncoder::new();
    let mut predictor = NeuralPredictor::new();
    let mut compressed = Vec::with_capacity(data.len() / 2 + 16);

    for &byte in data {
        for bit_idx in 0..8 {
            let bpos = 7 - bit_idx;
            let bit = (byte >> bpos) & 1;
            let p1 = predictor.predict();
            encoder.encode_bit(bit, p1, &mut compressed);
            predictor.update(bit);
        }
    }

    encoder.finish(&mut compressed);
    compressed
}

/// Decompresses Path D byte stream back to original raw bytes.
pub fn neural_decompress(compressed: &[u8], original_size: usize) -> Result<Vec<u8>> {
    if original_size == 0 {
        return Ok(Vec::new());
    }

    let mut decoder = ArithmeticDecoder::new(compressed);
    let mut predictor = NeuralPredictor::new();
    let mut decompressed = Vec::with_capacity(original_size);

    for _ in 0..original_size {
        let mut byte = 0u8;
        for bit_idx in 0..8 {
            let bpos = 7 - bit_idx;
            let p1 = predictor.predict();
            let bit = decoder.decode_bit(p1);
            byte |= bit << bpos;
            predictor.update(bit);
        }
        decompressed.push(byte);
    }

    Ok(decompressed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_neural_predictor_learns_constant_bits() {
        let mut predictor = NeuralPredictor::new();

        // Feed constant 1s
        for _ in 0..100 {
            let _ = predictor.predict();
            predictor.update(1);
        }

        let p1 = predictor.predict();
        assert!(
            p1 > 3500,
            "Predictor should predict high probability for continuous 1s, got {}",
            p1
        );
    }

    #[test]
    fn test_neural_roundtrip_simple() {
        let input = b"The quick brown fox jumps over the lazy dog. 1234567890!";
        let compressed = neural_compress(input);
        let decompressed =
            neural_decompress(&compressed, input.len()).expect("decompression failed");
        assert_eq!(&decompressed[..], input);
    }

    #[test]
    fn test_neural_roundtrip_empty() {
        let input = b"";
        let compressed = neural_compress(input);
        assert!(compressed.is_empty());
        let decompressed = neural_decompress(&compressed, 0).expect("decompression failed");
        assert!(decompressed.is_empty());
    }

    #[test]
    fn test_neural_roundtrip_repeating_patterns() {
        let input = b"NEXUS_NEURAL_COMPRESSION_NEXUS_NEURAL_COMPRESSION_NEXUS";
        let compressed = neural_compress(input);
        let decompressed =
            neural_decompress(&compressed, input.len()).expect("decompression failed");
        assert_eq!(&decompressed[..], input);
    }
}
