# Architecture — Project NEXUS

> This document explains the system design, data flow, and the reasoning behind every
> architectural decision. Read `CONVENTIONS.md` for coding style; this is for understanding
> the "why" of the system.

---

## 1. System Overview

NEXUS is a 5-layer pipeline where each layer has a single, well-defined responsibility:

```
Layer 0: FRAMING         — Split input into parallelizable superblocks + sub-blocks
Layer 1: DETECTION       — Classify each sub-block by data type
Layer 2: PREPROCESSING   — Apply bijective transforms to expose redundancy
Layer 3: COMPRESSION     — Core compression algorithm (5 possible paths)
Layer 4: ASSEMBLY        — Pack into .nxs binary format with integrity checks
```

### Data Flow (Compression)
```
Raw bytes
  → Layer 0: split into superblocks (4-16MB each)
    → each superblock split into sub-blocks (16-512KB each, by entropy)
      → Layer 1: classify each sub-block → BlockType enum
      → Layer 2: apply type-specific preprocessor → transformed bytes
      → Layer 3: compress via selected path → compressed bytes
      → Layer 4: write mini-header + compressed bytes to output stream
  → Layer 4: write footer (block table + checksum)
```

### Data Flow (Decompression)
```
.nxs file
  → Read footer → block table (random access index)
  → For each sub-block:
    → Read mini-header → path, type, flags
    → Layer 3: decompress via indicated path → transformed bytes
    → Layer 2: reverse preprocess (using flags) → raw bytes
  → Concatenate sub-blocks → original file
```

---

## 2. Architecture Decision Records (ADRs)

### ADR-001: Two-Tier Framing (Superblocks + Sub-blocks)

**Context:** We need to parallelize compression across CPU cores AND split data at entropy
boundaries for optimal algorithm selection.

**Decision:** Use two levels of framing:
- Superblocks (4-16MB): Completely independent units for rayon parallelism
- Sub-blocks (16-512KB): Entropy-based splits within a superblock

**Key rule:** LZ77 sliding window stays warm across sub-block boundaries within a superblock.
Only rANS frequency tables and preprocessor state reset at sub-block boundaries.

**Rationale:** If we used only flat independent blocks split at every entropy change (e.g., every
32KB), the LZ77 window would be wiped constantly. A 50MB binary with alternating code/data
sections would lose all long-range match opportunities, making us worse than 7-Zip and Zstd.

**Rejected alternative:** Single-tier blocks with shared LZ77 window across all blocks.
This prevents parallelism because all blocks share mutable state.

---

### ADR-002: Footer-Based Block Table

**Context:** We need random-access to any block for parallel decompression, but we also
need streaming compression (stdin → stdout).

**Decision:** Store the block table in the file footer, not the header. Each sub-block has
an inline 16-byte mini-header so the stream can be read sequentially.

**Rationale:** Placing the block table in the header requires knowing all block sizes before
writing the first byte of compressed data. This forces either:
1. A two-pass write (compress everything to temp, then write header), or
2. Buffering the entire compressed output in RAM

Both are unacceptable for large files. Footer placement (like ZIP's Central Directory) solves both.

**Trade-off:** Sequential-only readers (like `stdin | nexus decompress`) must read the
entire stream to find the footer. This is acceptable because decompression rarely needs
random access in streaming scenarios.

---

### ADR-003: Fixed-Point Integer Arithmetic for Context Mixing

**Context:** Path C (Context Mixing) and Path D (Neural) require probability predictions that
feed into an arithmetic coder. The encoder and decoder must produce IDENTICAL probabilities.

**Decision:** All probability math uses fixed-point integers. No IEEE 754 floats.

**Rationale:** IEEE 754 float math is non-deterministic across:
- CPU architectures (Intel vs AMD vs ARM)
- SIMD instruction sets (SSE vs AVX2 vs AVX-512)
- Compiler optimization levels (FMA reordering)
- Operating systems (different math library implementations)

A 1-ULP (Unit in Last Place) difference in probability at byte 50,000 will permanently
desynchronize the arithmetic coder's interval [L, R), producing garbage output.

PAQ8 and cmix solved this decades ago by using integer-only math with lookup tables.
We follow their proven approach.

**Implementation:**
- Probabilities: `u16` in `[1, 4095]` (12-bit scale, never 0 or 4096)
- Stretch (logit): `i32[4096]` lookup table mapping `p → ln(p/(1-p)) * 256`
- Squash (sigmoid): `i32[4096]` lookup table mapping `x → 1/(1+exp(-x/256)) * 4096`
- Weights: `i16` (signed, updated by integer SGD)
- Learning rate: integer shift (e.g., `>> 7` = divide by 128)

---

### ADR-004: Bijective-Only Preprocessors

**Context:** Layer 2 transforms data before compression to expose redundancy.

**Decision:** Every preprocessor MUST be strictly bijective (byte-for-byte reversible).

**Rationale:** Lossless compression requires exact byte reconstruction. Transforms like
Unicode normalization (NFC/NFD) are many-to-one mappings that destroy information.
Example: `é` can be encoded as `0xC3 0xA9` (precomposed) or `0x65 0xCC 0x81` (decomposed).
NFC normalization maps both to the same bytes, making it impossible to reconstruct the original.

**Testing:** Every preprocessor module must include a roundtrip test:
`inverse_transform(transform(data)) == data` for all possible byte sequences.

---

### ADR-005: Pilot Sampling for Path Selection

**Context:** At "ultra" compression levels, we need to choose between Path A (LZ-ANS)
and Path B (BWT-ANS). The naive approach is to compress the full block with both and pick
the smaller result.

**Decision:** Use a 16KB pilot sample from the center of the sub-block to estimate which
path will produce better results.

**Rationale:** Full-block trial compression at "ultra" levels would triple CPU time and
memory allocations (compress entire block with Path A, Path B, and Path C, then discard
two results). A 16KB pilot sample provides >95% accurate path prediction at <1% of the cost.

**Heuristics used:**
- If post-MTF zero-run density > 40% → BWT wins (Path B)
- If LZ77 match ratio > 60% → LZ wins (Path A)
- For levels 13+: always use Path C (Context Mixing) — no sampling needed

---

### ADR-006: Rust as Implementation Language

**Context:** Need high performance (within ~3% of C), safety guarantees, and modern tooling.

**Decision:** Rust with stable toolchain.

**Rationale:**
- Zero-cost abstractions matching C++ performance
- Memory safety prevents buffer overflow vulnerabilities in parser code
- `rayon` enables fearless parallelism for superblock processing
- `cargo` provides integrated testing, benchmarking, and dependency management
- Built-in fuzzing support for finding edge-case bugs
- LLVM backend enables SIMD intrinsics when needed

**Rejected alternatives:**
- C++: Same performance but no memory safety guarantees in parser code
- Python: 50-100x too slow for compression algorithm inner loops
- Go: GC pauses unacceptable for low-latency decompression

---

## 3. Layer Responsibilities

### Layer 0: Framing (`block/splitter.rs`)
- **Input:** Raw byte stream
- **Output:** Vector of `Superblock`, each containing vector of `SubBlock`
- **Owns:** Superblock size policy, entropy-based sub-block boundary detection
- **Does NOT:** Classify data types, apply transforms, or compress

### Layer 1: Detection (`block/detector.rs`)
- **Input:** `SubBlock` (raw bytes)
- **Output:** `BlockType` enum tag
- **Owns:** Magic byte tables, entropy calculation, bigram analysis
- **Does NOT:** Transform data or make compression decisions

### Layer 2: Preprocessing (`preprocess/*.rs`)
- **Input:** `SubBlock` bytes + `BlockType` tag
- **Output:** Transformed bytes + `PreprocessFlags` (for reverse transform)
- **Owns:** Delta encoding, MTF, RLE, WRT, x86 filter, float splitting
- **Does NOT:** Compress data. Only restructures it to expose redundancy.
- **INVARIANT:** `inverse(transform(data)) == data` ALWAYS

### Layer 3: Compression (`compress/*.rs` + `entropy/*.rs`)
- **Input:** Preprocessed bytes + compression level
- **Output:** Compressed byte stream
- **Owns:** LZ77, BWT, Context Mixing, Neural Prediction, rANS, Arithmetic Coding
- **Does NOT:** Know about file format, block table, or checksums

### Layer 4: Assembly (`format/*.rs`)
- **Input:** Compressed sub-blocks + metadata
- **Output:** .nxs binary file
- **Owns:** Header/footer/mini-header serialization, checksum computation, block table
- **Does NOT:** Know how compression works internally

---

## 4. Compression Path Decision Matrix

| BlockType | Level 1-6 | Level 7-12 | Level 13-16 | Level 17-19 |
|-----------|-----------|------------|-------------|-------------|
| Text | PATH_B | PATH_B (pilot vs A) | PATH_C | PATH_D |
| SourceCode | PATH_B | PATH_B (pilot vs A) | PATH_C | PATH_D |
| StructuredData | PATH_A | pilot A vs B | PATH_C | PATH_D |
| Binary | PATH_A | PATH_A | PATH_C | PATH_D |
| Executable | PATH_A | PATH_A | PATH_C | PATH_D |
| FloatingPoint | PATH_A | PATH_A | PATH_C | PATH_D |
| Audio | PATH_A | pilot A vs B | PATH_C | PATH_D |
| Image | PATH_A | PATH_A | PATH_C | PATH_D |
| Incompressible | PATH_E | PATH_E | PATH_E | PATH_E |

---

## 5. Performance Targets

| Operation | Target | Measured Against |
|-----------|--------|-----------------|
| Compress (level 1-3) | > 200 MB/s | gzip -1 |
| Compress (level 4-6) | > 50 MB/s | gzip -9 |
| Compress (level 7-12) | > 10 MB/s | 7z ultra |
| Decompress (all levels) | > 300 MB/s | zstd decompress |
| Ratio (level 7-12) | Better than 7z | Silesia corpus |
