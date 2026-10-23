# 08: Move-to-Front (MTF) & Zero-RLE Bijective Transforms

**What to build:** Implement bijective Move-to-Front (`preprocess/mtf.rs`) and Zero-Run-Length
Encoding (`preprocess/rle.rs`) transforms.
MTF converts BWT-clustered sequences into runs of small values (0, 1, 2).
Zero-RLE efficiently compresses consecutive runs of 0s while leaving non-zero values untouched.
Both transforms must be strictly bijective: `decode(encode(data)) == data`.

**Blocked by:** Phase 1 complete

**Status:** done

- [x] `preprocess/mtf.rs`: `mtf_encode(&[u8]) -> Vec<u8>` and `mtf_decode(&[u8]) -> Vec<u8>`
- [x] `preprocess/rle.rs`: `zero_rle_encode(&[u8]) -> Vec<u8>` and `zero_rle_decode(&[u8]) -> Result<Vec<u8>>`
- [x] Unit tests for MTF: roundtrip empty, single byte, repeated, all 256 bytes
- [x] Unit tests for Zero-RLE: roundtrip empty, all zeros, no zeros, mixed runs
