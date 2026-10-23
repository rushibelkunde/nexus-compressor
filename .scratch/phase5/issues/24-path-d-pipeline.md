# 24: Path D Pipeline Integration (Levels 17-19)

**What to build:** Integrate Path D (Neural Compression + Arithmetic Coding) into `crates/nexus-core/src/pipeline.rs`.
- Compression levels 17 to 19 route to Path D (`CompressionPath::PathD`).
- Encode: `neural_compress(data: &[u8]) -> Vec<u8>` using `NeuralPredictor` and `ArithmeticEncoder`.
- Decode: `neural_decompress(compressed: &[u8], original_size: usize) -> Result<Vec<u8>>` using `NeuralPredictor` and `ArithmeticDecoder`.
- Full integration roundtrip tests across text, binary, and structured files.
- Benchmark validation against existing paths.

**Blocked by:** 23

**Status:** done

- [x] `compress/neural/mod.rs`: `neural_compress` and `neural_decompress`
- [x] Wire `CompressionPath::PathD` into `pipeline.rs` for `compress_sub_block` and `decompress_sub_block`
- [x] Unit and integration tests for levels 17-19
- [x] Update `docs/PHASE_STATUS.md`
