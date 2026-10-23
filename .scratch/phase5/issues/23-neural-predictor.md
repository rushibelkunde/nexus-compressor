# 23: Bit-Level Neural Probability Predictor

**What to build:** Implement the full byte/bit neural predictor in `crates/nexus-core/src/compress/neural/mod.rs`.
- Connects feature extractor, hidden layer(s), and output projection.
- Output: predicts probability $P(\text{bit} = 1) \in [1, 4095]$ using the fixed-point squash lookup table.
- Online training: immediately after each bit is encoded/decoded, computes error gradient and updates network weights via integer backpropagation.
- Determinism guarantee: identical weights and activations across Windows, Linux, macOS, x86_64, and AArch64.

**Blocked by:** 21, 22

**Status:** ready-for-agent

- [ ] `compress/neural/mod.rs`: `NeuralPredictor` struct with `predict(&mut self)` and `update(&mut self, bit: u8)`
- [ ] Connects `FeatureExtractor`, `DenseLayer`, and output projection
- [ ] Unit tests testing learning of repeating and synthetic bit sequences
