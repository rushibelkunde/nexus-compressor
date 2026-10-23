# 17: Fixed-Point Stretch & Squash Lookup Tables

**What to build:** Implement precomputed fixed-point math lookup tables in `crates/nexus-core/src/compress/mixer/tables.rs`.
- `STRETCH_TABLE`: `i32[4096]` mapping probability `p ∈ [1, 4095]` to log-odds `round(ln(p / (4096 - p)) * 256)`.
- `SQUASH_TABLE`: `u16[4096]` mapping log-odds `x ∈ [-2048, 2047]` back to probability in `[1, 4095]`.
- Clamping and indexing utilities ensuring zero bounds check failure or panic.
- Strictly deterministic across architectures (ADR-003).

**Blocked by:** Phase 3 complete

**Status:** done

- [x] `compress/mixer/tables.rs`: Precomputed `STRETCH_TABLE` and `SQUASH_TABLE`
- [x] Fixed-point conversion helpers `stretch(p: u16) -> i32` and `squash(x: i32) -> u16`
- [x] Inversion accuracy unit tests verifying `squash(stretch(p)) ≈ p` within integer precision
