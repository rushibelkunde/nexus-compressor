# AGENTS.md — AI Agent Instructions for Project NEXUS

> **READ THIS FIRST.** This file is the entry point for any AI assistant working on this codebase.

## What is NEXUS?

NEXUS (Neural EXtreme Unified compresSion) is a custom lossless compression format and tool implemented in Rust.
It aims to outperform ZIP, RAR, 7z, Brotli, and Zstandard by combining multiple compression strategies
(LZ77, BWT, Context Mixing, Neural Prediction) into an adaptive pipeline that selects the best algorithm
per data block.

## Key Documentation

Read these before making changes:

| Document | What It Covers |
|----------|---------------|
| `docs/CONVENTIONS.md` | Coding style, naming, error handling, testing patterns |
| `docs/ARCHITECTURE.md` | System design, layer responsibilities, data flow |
| `docs/FORMAT_SPEC.md` | .nxs binary file format specification |
| `docs/ALGORITHMS.md` | Mathematical details of each compression algorithm |
| `docs/PHASE_STATUS.md` | Current implementation phase and what's done/pending |

## Critical Rules (NEVER VIOLATE)

### 1. Lossless Correctness is Sacred
Every code change MUST pass the roundtrip test: `compress(decompress(data)) == data` for ALL inputs.
Run `cargo test` before every commit. If a roundtrip test fails, fix it before doing anything else.

### 2. No Floating-Point Math in Encoder/Decoder Paths
Paths C (Context Mixing) and D (Neural) use **fixed-point integer arithmetic ONLY**.
IEEE 754 floats cause encoder/decoder desync across platforms (1-ULP rounding differences).
- Weights: `i16` or `i32`
- Probabilities: integers in `[1, 4095]` (12-bit scale)
- Stretch/squash: precomputed integer lookup tables
- See `docs/ALGORITHMS.md` for the fixed-point math spec

### 3. All Preprocessors Must Be Bijective
Every transform in Layer 2 must be **byte-for-byte reversible**.
`preprocess(inverse_preprocess(data)) == data` must hold for ALL byte sequences.
No many-to-one mappings (e.g., NO Unicode normalization).

### 4. Two-Tier Framing Architecture
- **Superblocks** (4-16MB): Independent. Parallel via rayon. LZ77 window does NOT cross boundaries.
- **Sub-blocks** (16-512KB): Entropy-split within a superblock. LZ77 window STAYS WARM across sub-blocks.
  rANS tables and preprocessor state RESET at sub-block boundaries.

### 5. Footer-Based Block Table
The block table lives in the FOOTER (not header). Each sub-block has an inline 16-byte mini-header.
This enables streaming compression. Never move the block table to the header.

## Project Structure

```
compression/
├── Cargo.toml              # Workspace: members = ["crates/*"]
├── crates/
│   ├── nexus-core/         # Core compression library (ALL algorithms)
│   ├── nexus-cli/          # CLI binary (depends on nexus-core)
│   └── nexus-bench/        # Benchmarks (depends on nexus-core)
├── tests/                  # Integration tests
└── docs/                   # Architecture, conventions, specs
```

## How to Build & Test

```bash
cargo build                 # Build all crates
cargo test                  # Run all tests (MUST pass before any commit)
cargo run -p nexus-cli -- compress input.txt output.nxs
cargo run -p nexus-cli -- decompress output.nxs restored.txt
cargo bench                 # Run benchmarks
```

## Current Phase

Check `docs/PHASE_STATUS.md` for what phase we're in and what's been implemented.

## Implementation Order

When adding a new feature, follow this checklist:
1. Write the module with thorough doc comments
2. Write unit tests alongside the code
3. Add integration roundtrip tests in `tests/`
4. Update `docs/PHASE_STATUS.md`
5. Run `cargo test` and `cargo clippy`

## Naming Conventions

See `docs/CONVENTIONS.md` for complete rules. Quick reference:
- Types: `PascalCase` (e.g., `BlockType`, `RansEncoder`)
- Functions: `snake_case` (e.g., `compress_block`, `detect_type`)
- Constants: `SCREAMING_SNAKE` (e.g., `MAX_WINDOW_SIZE`, `MAGIC_BYTES`)
- Modules: `snake_case`, one concept per module
- Error types: `NexusError` enum with descriptive variants
