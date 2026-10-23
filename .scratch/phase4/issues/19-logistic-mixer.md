# 19: Neural Logistic Mixer & Integer SGD

**What to build:** Implement fixed-point neural logistic mixer in `crates/nexus-core/src/compress/mixer/mod.rs`.
- Blends predictions from multiple context models using fixed-point weights `w[i]: i16`.
- Logit domain accumulation: `sum = Σ (w[i] * stretch(p[i])) >> shift`.
- Squashes blended log-odds back to `P(bit=1) ∈ [1, 4095]`.
- Online integer SGD weight adaptation:
  `error = (y * 4095) - blended_p`
  `Δw[i] = (stretch(p[i]) * error) >> (lr_shift + 8)`
- Fixed-point math ONLY (no IEEE 754 floats) ensuring cross-platform determinism (ADR-003).

**Blocked by:** 17, 18

**Status:** done

- [x] `compress/mixer/mod.rs`: `LogisticMixer` struct holding models and integer weights
- [x] Forward pass: `predict(&mut self, ctx: &ContextState) -> u16`
- [x] Backward pass: `update(&mut self, ctx: &ContextState, bit: u8)`
- [x] Roundtrip tests verifying bit-by-bit encode and decode stay 100% synchronized
