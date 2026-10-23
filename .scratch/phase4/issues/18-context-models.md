# 18: Context Prediction Models

**What to build:** Implement specialized bit-level context models in `crates/nexus-core/src/compress/mixer/models.rs`.
- Each model maintains context statistics estimating `P(bit = 1)` in `[1, 4095]`.
- Counter representation: 8-bit or 16-bit counts with state saturation (never reaching 0 or 4096).
- Models implemented:
  1. `Order0Model`: Predicts current bit conditioned on already decoded bits of the current byte.
  2. `Order1Model`: Conditioned on (previous byte, current byte prefix).
  3. `Order2Model`: Conditioned on (previous 2 bytes, current byte prefix).
  4. `MatchModel`: Detects longest match in rolling window and predicts next bit matching history.
  5. `WordModel`: Hash of current word token for text.
- Clean trait `ContextModel` with `predict(&self, ctx: &Context) -> u16` and `update(&mut self, ctx: &Context, bit: u8)`.

**Blocked by:** 16, 17

**Status:** ready-for-agent

- [ ] `compress/mixer/models.rs`: `ContextModel` trait and context state struct
- [ ] `Order0Model`, `Order1Model`, `Order2Model`
- [ ] `MatchModel` with rolling buffer lookup
- [ ] `WordModel` with simple fast hashing
- [ ] Unit tests for each model verifying probability shifts toward observed bits
