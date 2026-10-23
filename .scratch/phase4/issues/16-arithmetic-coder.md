# 16: Bit-Level Integer Arithmetic Coder

**What to build:** Implement a bit-level arithmetic / range coder in `crates/nexus-core/src/entropy/arithmetic.rs`.
- Fixed-point 12-bit probability scale: `p1` in `[1, 4095]`.
- Interval subdivision using pure integer arithmetic (no floats, no ULP divergence across platforms).
- Invariant: `low < high`, range renormalization shifting 8 bits when high bits match.
- Byte-oriented bitstream writer and reader.
- Roundtrip guarantee: `decode(encode(bits)) == bits` for any binary sequence.

**Blocked by:** Phase 3 complete

**Status:** ready-for-agent

- [ ] `entropy/arithmetic.rs`: `ArithmeticEncoder` with `encode_bit(&mut self, bit: u8, p1: u16, output: &mut Vec<u8>)`
- [ ] `entropy/arithmetic.rs`: `ArithmeticDecoder` with `decode_bit(&mut self, p1: u16, input: &mut &[u8]) -> u8`
- [ ] Renormalization and flush on encoder finish
- [ ] Unit tests for uniform bits, skewed probabilities, edge cases (all zeros, all ones, alternating)
