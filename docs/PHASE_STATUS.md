# Phase Status — Project NEXUS

> This file tracks which implementation phase is current and what's been completed.
> Updated after each milestone.

---

## Current Phase: **Phase 4 — Context Mixing (The Core Advantage)**

### Phase 4 Checklist

- [ ] Bitwise rANS / Arithmetic Coder (12-bit fixed-point probabilities)
- [ ] Context Models: Order-0, Order-1, Order-2, Match Model
- [ ] Word-level model for natural text
- [ ] Record-model for structured / tabular binary
- [ ] Integer logistic mixer with precomputed stretch/squash lookup tables
- [ ] Fixed-point gradient descent weight adapter (ADR-003)
- [ ] Path C integration into `pipeline.rs` for Levels 13-16
- [ ] Verification on Silesia / Calgary corpora

---

## Phase Overview

| Phase | Name | Status | Target Ratio |
|-------|------|--------|-------------|
| 1 | Foundation | ✅ Completed | ≈ gzip |
| 2 | Core Optimizations | ✅ Completed | > gzip, ≈ bzip2 |
| 3 | Adaptive Intelligence | ✅ Completed | > 7-Zip, ≈ Zstd |
| 4 | Context Mixing | 🔄 In Progress | > 7-Zip Ultra |
| 5 | Neural Compression | ⏳ Planned | ≈ cmix |
| 6 | Polish & Performance | ⏳ Planned | Production ready |

---

## Completed Milestones

- **Phase 3 — Adaptive Intelligence (Completed 2026-10-24):**
  - Bijective Delta encoding (`preprocess/delta.rs`) supporting generic, audio, and sensor strides.
  - IEEE 754 Float byte-plane splitting (`preprocess/float_split.rs`) with tail alignment handling.
  - Bijective x86 BCJ branch filter (`preprocess/x86.rs`) with relative-to-absolute CALL/JMP translations.
  - Statistical content detector & classifier (`block/detector.rs`) supporting PE, ELF, Mach-O, WAV, IEEE 754 floats, and entropy categorization.
  - Dynamic entropy-based sub-block boundary splitting (`block/splitter.rs`) across 16KB-512KB windows.
  - Warm LZ77 dictionary preserved across sub-block boundaries within independent superblocks (ADR-001).
  - 16KB center pilot sampling for intelligent path routing (ADR-005).
  - Chained Layer 2 preprocessing and inverse preprocessing in `pipeline.rs`.
  - 90/90 tests passing (77 unit tests, 13 integration tests) with zero compiler or clippy warnings.

- **Phase 2 — Core Optimizations (Completed 2026-10-23):**
  - Order-0 rANS entropy coder with 12-bit precision, fast lookup tables, and normalized symbol frequencies.
  - Burrows-Wheeler Transform (BWT) with O(N) LF-mapping inverse transform.
  - Bijective Move-to-Front (MTF) and Zero-Run-Length (Zero-RLE) transforms.
  - Path B text compression pipeline (BWT → MTF → Zero-RLE → rANS).
  - Path A binary compression upgraded to LZ77 + rANS.
  - Multi-threaded parallel superblock processing using `rayon`.
  - 61/61 unit and integration tests passing with zero warnings.

- **Phase 1 — Foundation (Completed 2026-10-23):**
  - Full Cargo workspace setup (`nexus-core`, `nexus-cli`, `nexus-bench`).
  - `.nxs` binary container format with 32-byte header, 16-byte mini-headers, 20-byte block table, and 24-byte footer.
  - Greedy LZ77 encoder with hash-chain matching + lossless LZ77 decoder.
  - Canonical Huffman entropy coder with compact header and bitstream I/O.
  - Sub-block XXHash32 and file-level XXHash64 integrity verification.
  - End-to-end `nexus` CLI supporting `compress` and `decompress`.
  - 39/39 unit & integration tests passing with zero compiler warnings and zero clippy warnings.

