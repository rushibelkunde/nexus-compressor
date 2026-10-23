# 06: rANS (Range Asymmetric Numeral Systems) Entropy Coder

**What to build:** Implement an order-0 rANS encoder and decoder in `entropy/rans.rs`.
rANS provides arithmetic-coding-level compression density at significantly higher decoding speeds.
Uses a 12-bit scale (M = 4096), integer renormalization, normalized frequency table, and streaming state.
Roundtrip tests prove `decode(encode(data)) == data` for all byte streams.

**Blocked by:** Phase 1 complete

**Status:** done

- [x] `entropy/rans.rs`: `RansEncoder` and `RansDecoder` structs
- [x] Frequency table normalization (sum to 4096, min frequency 1 for present symbols)
- [x] Compact frequency table serialization/deserialization
- [x] Reverse-order encoding + streaming state serialization
- [x] Fast symbol lookup table for decoder (4096 entries)
- [x] Unit tests: empty input
- [x] Unit tests: single symbol repeated
- [x] Unit tests: uniform 256 symbols
- [x] Unit tests: English text
- [x] Unit tests: random data
