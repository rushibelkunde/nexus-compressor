# 15: Pilot Sampling & Phase 3 Pipeline Integration

**What to build:** Integrate preprocessors (Delta, x86 filter, Float Split), content-aware routing,
and 16KB center pilot sampling into the full compression pipeline (ADR-005).
- Pilot sampling: tests a 16KB center sample to predict whether Path A or Path B wins without full-block trial overhead.
- Preprocessors automatically chained based on `BlockType`:
  - `Executable`: x86 filter → Path A
  - `FloatingPoint`: Float byte-plane split → Path A
  - `Audio`: Delta filter → Path A
  - `Text` / `SourceCode`: Path B (or Path A if pilot sampling indicates LZ77 dominates)
- Mini-header flags updated with `DELTA`, `X86_FILTER`, `FLOAT_SPLIT`, `MTF`, `RLE`.
- Integration tests verifying all preprocessors and data types roundtrip cleanly.

**Blocked by:** 11, 12, 13, 14

**Status:** done

- [x] `pipeline.rs`: Pilot sampling function (16KB sample)
- [x] Chaining of Layer 2 preprocessors before Layer 3 compression
- [x] Inverse preprocessing on decode based on mini-header `preprocess_flags`
- [x] Full end-to-end integration tests for executable binaries, float arrays, audio data, and text
