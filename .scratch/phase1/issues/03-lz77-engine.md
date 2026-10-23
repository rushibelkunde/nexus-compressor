# 03: LZ77 Encoder and Decoder

**What to build:** A greedy LZ77 encoder with hash-chain match finding and its corresponding decoder. Given raw bytes, the encoder produces a stream of `Lz77Token` (Literal or Match). The decoder reconstructs the original bytes from tokens. Roundtrip tests prove `decode(encode(data)) == data` for empty, single-byte, all-same, repeated patterns, and random data.

**Blocked by:** 01 (Workspace Setup)

**Status:** done

- [x] `compress/lz77.rs`: `Lz77Token` enum (Literal, Match)
- [x] `Lz77Encoder` struct with `window_size` parameter
- [x] Hash-chain match finder (3-byte hash, chain length limit)
- [x] Greedy matching: take first match >= 3 bytes
- [x] `encode(&[u8]) -> Vec<Lz77Token>` method
- [x] Token serialization: pack tokens into bytes (for entropy coding input)
- [x] `Lz77Decoder` struct
- [x] `decode(tokens: &[Lz77Token]) -> Vec<u8>` method
- [x] Token deserialization: unpack tokens from bytes
- [x] Unit test: roundtrip empty input
- [x] Unit test: roundtrip single byte
- [x] Unit test: roundtrip "aaaaaaaaaa" (all same)
- [x] Unit test: roundtrip "abcabcabcabc" (repeating pattern)
- [x] Unit test: roundtrip 64KB random data
- [x] Unit test: roundtrip with long match (> 258 bytes of same pattern)
