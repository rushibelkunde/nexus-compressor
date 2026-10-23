# 07: Burrows-Wheeler Transform (BWT)

**What to build:** Implement forward and inverse Burrows-Wheeler Transform in `compress/bwt.rs`.
The BWT permutes symbols so identical preceding contexts cluster together, dramatically improving
redundancy for text and source code.
Returns transformed bytes and the primary index (u32).
Inverse BWT uses LF-mapping to restore exact original bytes in O(n) time.

**Blocked by:** Phase 1 complete

**Status:** done

- [x] `compress/bwt.rs`: `bwt_encode(input: &[u8]) -> (Vec<u8>, u32)`
- [x] `bwt_decode(bwt_data: &[u8], primary_index: u32) -> Result<Vec<u8>>`
- [x] O(n) LF-mapping reconstruction
- [x] Unit tests: empty input
- [x] Unit tests: single byte
- [x] Unit tests: repeating pattern ("banana", "abcabc")
- [x] Unit tests: English text paragraph
- [x] Unit tests: random data
