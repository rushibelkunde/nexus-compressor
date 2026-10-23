# 20: Path C Pipeline Integration (Levels 13-16)

**What to build:** Integrate Path C (Context Mixing + Arithmetic Coding) into `crates/nexus-core/src/pipeline.rs`.
- Compression levels 13 to 16 route sub-blocks to Path C (ADR-005).
- Mini-header flag and `CompressionPath::PathC` serialization/deserialization.
- Encode: iterates byte stream bit-by-bit through `LogisticMixer` and `ArithmeticEncoder`.
- Decode: feeds decoded bits through `ArithmeticDecoder` and identical `LogisticMixer`.
- End-to-end integration roundtrip tests across text, source code, executables, and Silesia benchmarks.

**Blocked by:** 16, 19

**Status:** done

- [x] `compress/mixer/mod.rs`: `context_mixing_compress(data: &[u8]) -> Vec<u8>`
- [x] `compress/mixer/mod.rs`: `context_mixing_decompress(compressed: &[u8], original_size: usize) -> Result<Vec<u8>>`
- [x] Wire `CompressionPath::PathC` into `pipeline.rs` for `compress_sub_block` and `decompress_sub_block`
- [x] Full integration roundtrip tests with levels 13-16
- [x] Verify ratio superiority on repetitive and structured data
