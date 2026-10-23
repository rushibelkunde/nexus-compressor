# 14: Dynamic Sub-Block Splitting & Warm LZ77 Dictionary

**What to build:** Implement entropy-based dynamic sub-block splitting within superblocks and maintain
a warm LZ77 dictionary across sub-block boundaries (ADR-001).
- Sub-block size: 16KB to 512KB (default target: 128KB).
- Sliding entropy window: detects transitions between text, code, binary, and compressed segments.
- LZ77 sliding window history remains intact across sub-blocks within the same superblock, preserving
  cross-block references while allowing rANS frequency tables and preprocessor filters to reset.

**Blocked by:** 13 (Content Detector)

**Status:** done

- [x] `block/splitter.rs`: `split_superblock_into_subblocks(data: &[u8]) -> Vec<SubBlock>`
- [x] Entropy change boundary detection (sliding 4096-byte window)
- [x] LZ77 warm window preservation across sub-blocks in `Lz77Encoder`
- [x] Reset rANS state and preprocessors at sub-block boundaries
- [x] Unit tests for multi-subblock superblocks with cross-boundary matching
