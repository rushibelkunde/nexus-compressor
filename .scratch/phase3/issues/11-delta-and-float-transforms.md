# 11: Bijective Delta Encoding & Float Byte-Plane Splitting

**What to build:** Implement bijective Delta Encoding (`preprocess/delta.rs`) and Float Byte-Plane
Splitting (`preprocess/float_split.rs`).
- Delta encoding supports 1, 2, 4, and 8-byte strides for audio, sensor, and time-series data.
- Float byte-plane splitting separates 32-bit (f32) and 64-bit (f64) IEEE 754 arrays into contiguous
  byte streams (exponent/sign plane, high mantissa, low mantissa), grouping similar bytes together.
Both transforms must be strictly bijective: `decode(encode(data)) == data`.

**Blocked by:** Phase 2 complete

**Status:** done

- [x] `preprocess/delta.rs`: `delta_encode(input: &[u8], stride: usize) -> Vec<u8>`
- [x] `preprocess/delta.rs`: `delta_decode(input: &[u8], stride: usize) -> Vec<u8>`
- [x] `preprocess/float_split.rs`: `float_split_encode(input: &[u8], width: usize) -> Vec<u8>`
- [x] `preprocess/float_split.rs`: `float_split_decode(input: &[u8], width: usize) -> Vec<u8>`
- [x] Unit tests for Delta: roundtrip empty, 1-byte, 2-byte, 4-byte stride, arbitrary sequences
- [x] Unit tests for Float Split: roundtrip empty, 4-byte (f32), 8-byte (f64), non-aligned tail
