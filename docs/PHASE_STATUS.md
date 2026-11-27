# Phase Status — Project NEXUS

> This file tracks which implementation phase is current and what's been completed.
> Updated after each milestone.

---

## Current Status: **Phase 7 in progress — codec rebuild (format v2)**

Phases 1-6 produced a working container, CLI and GUI, but their "production
ready" verdict was based on a benchmark of 32 KB of synthetic repeated text.
Measured on real files, every v1 level was larger than `gzip -6` and the
context-mixing and neural levels were larger than level 2. Phase 7 replaces
the codecs and measures everything against real reference compressors.

### Phase 7 Checklist

- [x] Corpus benchmark harness with reference compressors and a size regression gate (`nexus-bench`, `scripts/bench/`)
- [x] Release profile (LTO, single codegen unit); one buffer per superblock; single-pass block analysis; single-read decompression
- [x] LZ-ANS v2 (Path A2) for levels 1-12: split streams, repeat offsets, interleaved rANS, five match finders, optimal parsing, context-coded literals
- [x] BWT + adaptive model (Path B2) for levels 13+, with linear-time suffix array construction
- [x] Bijective E8/E9 branch filter v2 applied to every block that looks like x86 code
- [x] Format v2 with full v1 read compatibility (verified against archives written by the v0.1.0 release binary)
- [x] Cancellation, verify-only pass (`nexus test`), write-to-temp-then-rename, no partial outputs left behind
- [x] GUI rework: drag and drop, live compression view, block map, level rail with measured estimates, archive test
- [ ] GUI visual review on a real display (it has only been verified by build, lint and off-screen layout tests)
- [ ] Context-mixing rebuild (Paths C/D are unchanged from v1 and rarely win a block)
- [ ] Mid-level speed: levels 1-9 are 2.5-3x slower than zstd at the same ratio
- [ ] Decompression speed: about 3-4x slower than zstd at levels 1-12; BWT blocks are much slower still

### Measured results (Silesia corpus, 211,938,580 bytes, single thread)

Sizes are exact and reproducible. Speeds come from a shared, thermally
limited 4-core laptop where the same run varies by 30-50%, so compare the
speed columns only within one table.

| Compressor | Bytes | Ratio |
|------------|------:|------:|
| nexus -1  | 73,343,688 | 34.61% |
| nexus -3  | 67,261,061 | 31.74% |
| nexus -6  | 60,556,918 | 28.57% |
| nexus -9  | 57,152,757 | 26.97% |
| nexus -11 | 53,843,164 | 25.41% |
| nexus -12 | 53,447,605 | 25.22% |
| nexus -13 | 49,778,677 | 23.49% |
| zstd -1   | 73,411,400 | 34.64% |
| zstd -3   | 66,512,901 | 31.38% |
| zstd -9   | 59,371,345 | 28.01% |
| zstd -19  | 52,924,791 | 24.97% |
| gzip -6   | 68,227,965 | 32.19% |
| brotli -5 | 59,590,609 | 28.12% |
| brotli -11| 49,564,563 | 23.39% |
| bzip2 -9  | 54,506,769 | 25.72% |
| xz -9     | 48,795,480 | 23.02% |

Speed, one run with everything measured back to back (MB/s compress / decompress):

| Compressor | Compress | Decompress |
|------------|---------:|-----------:|
| nexus -1 | 57 | 99 |
| nexus -3 | 43 | 102 |
| nexus -6 | 6.6 | 119 |
| zstd -1  | 141 | 403 |
| zstd -3  | 74 | 345 |
| zstd -9  | 15.6 | 297 |

What this says:

* **Size**: levels 1-12 are within about 1% of zstd at the matching level.
  Level 13 is smaller than `zstd -19` and `bzip2 -9`, equal to `brotli -11`,
  and 2% larger than `xz -9`. On text alone level 13 beats `xz -9`
  (`dickens`: 2,676,326 vs 2,830,604 bytes).
* **Speed**: NEXUS is 2.5-3x slower than zstd to compress and 3-4x slower to
  decompress at equal size. Level 13 decompresses BWT-coded blocks at only a
  few MB/s per thread.
* Before and after on the two files first used to evaluate v1 (a 6 MB
  source-code file and a 14.5 MB executable, 20.5 MB together): level 6 went
  from 7,965,328 to 5,431,001 bytes, and level 13 reaches 4,851,062. For
  reference `gzip -6` gives 6,409,780 and `xz -6` gives 4,703,964.

Regenerate with `target/release/nexus-bench --threads 1 --refs`; gate changes
with `--check scripts/bench/baseline-silesia.tsv`.

---

## Phase Overview

| Phase | Name | Status | Target Ratio |
|-------|------|--------|-------------|
| 1 | Foundation | ✅ Completed | ≈ gzip |
| 2 | Core Optimizations | ✅ Completed | > gzip, ≈ bzip2 |
| 3 | Adaptive Intelligence | ✅ Completed | > 7-Zip, ≈ Zstd (not met; see Phase 7) |
| 4 | Context Mixing | ✅ Completed | > 7-Zip Ultra (not met; see Phase 7) |
| 5 | Neural Compression | ✅ Completed | ≈ cmix (not met; see Phase 7) |
| 6 | Polish & Performance | ✅ Completed | Production ready |
| 7 | Codec rebuild (format v2) | 🚧 In progress | zstd-class at 1-12, > xz at 13+ |

---

## Completed Milestones

- **Desktop GUI, Presets & Shell Integration (Completed 2026-10-24):**
  - **Presets & Custom Levels (1–19)**: Introduced `CompressionPreset` (`Low` = level 2, `Medium` = level 6, `High` = level 9) plus full arbitrary custom level selection (1 through 19) in [nexus-gui](file:///d:/Data/DATA/projects/personal/compression/crates/nexus-gui/src/main.rs) with real-time algorithm engine badges.
  - **Live Size & Ratio Estimation Preview**: Added `estimate_compression` and `sample_path` in [pipeline.rs](file:///d:/Data/DATA/projects/personal/compression/crates/nexus-core/src/pipeline.rs) computing fast (<1ms) trial compression and Shannon entropy to display accurate estimated compressed size, space savings %, and incompressible warnings before compression starts.
  - **Zero-Terminal GUI Decompression**: Wired all Windows Explorer decompression entry points (`Extract with NEXUS...`, `Extract Here` with `--auto-start`, and `.nxar` double-click) directly to `nexus-gui`, completely eliminating console terminal windows while rendering live animated progress bars, MB/s throughput, and ETA.
  - **Custom Logo & Icon**: Designed custom cyberpunk-hexagonal brand icon with multi-resolution `.ico` (16x16, 32x32, 48x48, 256x256) at [assets/nexus.ico](file:///d:/Data/DATA/projects/personal/compression/assets/nexus.ico).
  - **Windows Shell Integration**: Created [install.ps1](file:///d:/Data/DATA/projects/personal/compression/scripts/windows/install.ps1) registering right-click context menus (`Compress with NEXUS...`, quick preset submenu with auto-start, `Extract with NEXUS...`, `Extract Here`), `.nxar` default icon association, and instant shell cache refresh.
  - **130/130 Workspace Tests Passing**: Zero regressions across 112 unit tests and 18 integration tests.

- **Phase 6 — Polish & Performance (Completed 2026-10-24):**
  - Enhanced CLI binary (`nexus-cli`): added `--threads <N>`, `--verbose` block-level telemetry, and `nexus list` subcommand inspecting file headers, superblocks, and block tables without payload decompression.
  - Archive inspection engine (`inspect_archive`) in `pipeline.rs` returning structured `ArchiveInfo` and `BlockInfo`.
  - Comprehensive benchmark suite (`nexus-bench`) evaluating Natural Text, x86 Machine Code, Scientific IEEE 754 Floats, Audio PCM, Structured JSON, and Incompressible Random streams across Levels 1, 5, 7, 13, and 17.
  - Validated memory-bounded multi-superblock streaming up to 4.5+ MB streams with 5+ parallel superblocks.
  - 123/123 workspace tests passing (106 unit tests, 17 integration tests) with zero compiler warnings and zero clippy warnings under `-D warnings`.

- **Phase 5 — Neural Compression (Completed 2026-10-24):**
  - Fixed-point neural network layer (`compress/neural/layer.rs`) with 16-bit integer weights, 32-bit accumulators/biases, integer LeakyReLU activation, sparse forward pass, and online integer backpropagation (ADR-003).
  - Multi-context sparse feature extractor (`compress/neural/features.rs`) extracting byte unigrams/bigrams, bit position/prefix tree states, sparse distance tokens (b_{-4}, b_{-8}), word tokens, and character classes with partitioned hash indexing.
  - Bit-level neural probability predictor (`compress/neural/mod.rs`) connecting feature extractor, hidden layer, and output projection with online integer SGD error adaptation.
  - Path D pipeline integration (`neural_compress` / `neural_decompress`) wired for compression levels 17-19 with arithmetic coding.
  - 121/121 tests passing (105 unit tests, 16 integration tests) with zero compiler or clippy warnings under `-D warnings`.

- **Phase 4 — Context Mixing (Completed 2026-10-24):**
  - Bit-level integer arithmetic/range coder (`entropy/arithmetic.rs`) with Subbotin carry handling and 12-bit fixed-point probabilities.
  - Precomputed fixed-point stretch (logit) and squash (sigmoid) tables (`compress/mixer/tables.rs`) ensuring 100% cross-platform deterministic prediction without IEEE 754 floats.
  - Specialized bit-level predictive context models (`compress/mixer/models.rs`): Order-0, Order-1, Order-2, Word model, and long-range match finder with Krichevsky-Trofimov counters.
  - Fixed-point neural logistic mixer (`compress/mixer/mod.rs`) blending model predictions with online integer SGD error adaptation.
  - Path C end-to-end pipeline integration (`context_mixing_compress` / `context_mixing_decompress`) wired for compression levels 13-16.
  - 107/107 tests passing (93 unit tests, 14 integration tests) with zero compiler or clippy warnings.


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

