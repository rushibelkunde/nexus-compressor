//! Context prediction models for Context Mixing (Layer 3).
//!
//! Provides bit-level predictive statistics for order-0, order-1, order-2,
//! word models, and long-range match finding.

/// Bit-level context state passed to models for prediction and update.
#[derive(Debug, Clone, Copy)]
pub struct ContextState {
    pub prev_byte: u8,
    pub prev_byte2: u8,
    pub bit_pos: usize,
    pub bit_prefix: usize,
    pub word_hash: u32,
    pub match_predicted_bit: Option<u8>,
}

impl Default for ContextState {
    fn default() -> Self {
        Self::new()
    }
}

impl ContextState {
    pub fn new() -> Self {
        Self {
            prev_byte: 0,
            prev_byte2: 0,
            bit_pos: 7,
            bit_prefix: 1,
            word_hash: 0,
            match_predicted_bit: None,
        }
    }
}

/// A two-counter state representing zero and one counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BitCounter {
    pub n0: u8,
    pub n1: u8,
}

impl Default for BitCounter {
    fn default() -> Self {
        Self::new()
    }
}

impl BitCounter {
    pub const fn new() -> Self {
        Self { n0: 0, n1: 0 }
    }

    /// Computes P(bit = 1) in [1, 4095] using Krichevsky-Trofimov estimator.
    #[inline(always)]
    pub fn predict(&self) -> u16 {
        let total = self.n0 as u32 + self.n1 as u32;
        if total == 0 {
            return 2048;
        }
        let p1 = ((2 * self.n1 as u32 + 1) * 2048) / (total + 1);
        p1.clamp(1, 4095) as u16
    }

    /// Updates counter with observed bit, with half-decay saturation at 240.
    #[inline(always)]
    pub fn update(&mut self, bit: u8) {
        if bit == 0 {
            if self.n0 < 240 {
                self.n0 += 1;
            } else {
                self.n0 = (self.n0 >> 1) + 1;
                self.n1 >>= 1;
            }
        } else if self.n1 < 240 {
            self.n1 += 1;
        } else {
            self.n1 = (self.n1 >> 1) + 1;
            self.n0 >>= 1;
        }
    }
}

/// Common trait for all context prediction models.
pub trait ContextModel {
    /// Predicts P(bit = 1) in [1, 4095].
    fn predict(&self, ctx: &ContextState) -> u16;
    /// Updates model parameters with observed actual bit.
    fn update(&mut self, ctx: &ContextState, bit: u8);
}

/// Order-0 model: conditioned only on the current byte's bit-prefix.
pub struct Order0Model {
    counters: [BitCounter; 256],
}

impl Default for Order0Model {
    fn default() -> Self {
        Self::new()
    }
}

impl Order0Model {
    pub fn new() -> Self {
        Self {
            counters: [BitCounter::new(); 256],
        }
    }
}

impl ContextModel for Order0Model {
    #[inline(always)]
    fn predict(&self, ctx: &ContextState) -> u16 {
        let idx = ctx.bit_prefix.min(255);
        self.counters[idx].predict()
    }

    #[inline(always)]
    fn update(&mut self, ctx: &ContextState, bit: u8) {
        let idx = ctx.bit_prefix.min(255);
        self.counters[idx].update(bit);
    }
}

/// Order-1 model: conditioned on (previous_byte, bit-prefix).
pub struct Order1Model {
    counters: Vec<BitCounter>,
}

impl Default for Order1Model {
    fn default() -> Self {
        Self::new()
    }
}

impl Order1Model {
    pub fn new() -> Self {
        Self {
            counters: vec![BitCounter::new(); 256 * 256],
        }
    }
}

impl ContextModel for Order1Model {
    #[inline(always)]
    fn predict(&self, ctx: &ContextState) -> u16 {
        let idx = ((ctx.prev_byte as usize) << 8) | ctx.bit_prefix.min(255);
        self.counters[idx].predict()
    }

    #[inline(always)]
    fn update(&mut self, ctx: &ContextState, bit: u8) {
        let idx = ((ctx.prev_byte as usize) << 8) | ctx.bit_prefix.min(255);
        self.counters[idx].update(bit);
    }
}

/// Order-2 model: conditioned on hash(prev_byte2, prev_byte, bit-prefix).
pub struct Order2Model {
    counters: Vec<BitCounter>,
    mask: usize,
}

impl Default for Order2Model {
    fn default() -> Self {
        Self::new(65536)
    }
}

impl Order2Model {
    pub fn new(size: usize) -> Self {
        let cap = size.next_power_of_two();
        Self {
            counters: vec![BitCounter::new(); cap],
            mask: cap - 1,
        }
    }

    #[inline(always)]
    fn hash_ctx(&self, ctx: &ContextState) -> usize {
        let h = ((ctx.prev_byte2 as usize) << 12)
            ^ ((ctx.prev_byte as usize) << 8)
            ^ ctx.bit_prefix.min(255);
        h & self.mask
    }
}

impl ContextModel for Order2Model {
    #[inline(always)]
    fn predict(&self, ctx: &ContextState) -> u16 {
        let idx = self.hash_ctx(ctx);
        self.counters[idx].predict()
    }

    #[inline(always)]
    fn update(&mut self, ctx: &ContextState, bit: u8) {
        let idx = self.hash_ctx(ctx);
        self.counters[idx].update(bit);
    }
}

/// Match model: predicts the next bit from the longest matching history sequence.
#[derive(Default)]
pub struct MatchModel;

impl MatchModel {
    pub fn new() -> Self {
        Self
    }
}

impl ContextModel for MatchModel {
    #[inline(always)]
    fn predict(&self, ctx: &ContextState) -> u16 {
        match ctx.match_predicted_bit {
            Some(0) => 400,  // ~90% confidence 0
            Some(1) => 3696, // ~90% confidence 1
            _ => 2048,       // neutral 50%
        }
    }

    #[inline(always)]
    fn update(&mut self, _ctx: &ContextState, _bit: u8) {
        // Match prediction is derived dynamically from buffer
    }
}

/// Word model: conditioned on word token hash for natural text.
pub struct WordModel {
    counters: Vec<BitCounter>,
    mask: usize,
}

impl Default for WordModel {
    fn default() -> Self {
        Self::new(32768)
    }
}

impl WordModel {
    pub fn new(size: usize) -> Self {
        let cap = size.next_power_of_two();
        Self {
            counters: vec![BitCounter::new(); cap],
            mask: cap - 1,
        }
    }
}

impl ContextModel for WordModel {
    #[inline(always)]
    fn predict(&self, ctx: &ContextState) -> u16 {
        let idx = ((ctx.word_hash as usize) ^ ctx.bit_prefix.min(255)) & self.mask;
        self.counters[idx].predict()
    }

    #[inline(always)]
    fn update(&mut self, ctx: &ContextState, bit: u8) {
        let idx = ((ctx.word_hash as usize) ^ ctx.bit_prefix.min(255)) & self.mask;
        self.counters[idx].update(bit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bit_counter_adaptation() {
        let mut counter = BitCounter::new();
        assert_eq!(counter.predict(), 2048); // initial neutral

        // Feed 10 consecutive ones
        for _ in 0..10 {
            counter.update(1);
        }
        let p = counter.predict();
        assert!(p > 3500, "Probability of 1 should be very high, got {}", p);

        // Feed 20 zeros -> probability should swing to low
        for _ in 0..20 {
            counter.update(0);
        }
        let p2 = counter.predict();
        assert!(p2 < 1500, "Probability of 1 should decrease, got {}", p2);
    }

    #[test]
    fn test_order0_model() {
        let mut model = Order0Model::new();
        let mut ctx = ContextState::new();
        ctx.bit_prefix = 1;

        assert_eq!(model.predict(&ctx), 2048);
        model.update(&ctx, 1);
        model.update(&ctx, 1);
        assert!(model.predict(&ctx) > 2500);
    }

    #[test]
    fn test_order1_model() {
        let mut model = Order1Model::new();
        let mut ctx = ContextState::new();
        ctx.prev_byte = b'e';
        ctx.bit_prefix = 3;

        assert_eq!(model.predict(&ctx), 2048);
        model.update(&ctx, 0);
        model.update(&ctx, 0);
        assert!(model.predict(&ctx) < 1800);
    }
}
