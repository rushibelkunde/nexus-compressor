# Coding Conventions — Project NEXUS

> This document defines the coding style, patterns, and practices for NEXUS.
> All contributors (human and AI) MUST follow these conventions.

---

## 1. Language & Toolchain

- **Language:** Rust (stable toolchain)
- **Edition:** 2021
- **Formatting:** `rustfmt` defaults (run `cargo fmt` before every commit)
- **Linting:** `cargo clippy -- -W clippy::all` must pass with zero warnings
- **Testing:** `cargo test` must pass before every commit

---

## 2. Naming Conventions

| Element | Convention | Example |
|---------|-----------|---------|
| Types / Structs / Enums | `PascalCase` | `BlockType`, `RansEncoder`, `NexusError` |
| Functions / Methods | `snake_case` | `compress_block()`, `detect_type()` |
| Local variables | `snake_case` | `block_size`, `match_len` |
| Constants | `SCREAMING_SNAKE_CASE` | `MAX_WINDOW_SIZE`, `MAGIC_BYTES` |
| Modules | `snake_case` | `context_mix`, `float_split` |
| Type parameters | Single uppercase letter or short PascalCase | `T`, `W: Write` |
| Lifetimes | Short lowercase | `'a`, `'buf` |

### Specific Domain Terms

| Term | Meaning | Where Used |
|------|---------|-----------|
| `superblock` | 4-16MB independent parallel unit | `block/splitter.rs` |
| `sub_block` | 16-512KB entropy-split unit within a superblock | `block/splitter.rs` |
| `path` | Compression algorithm choice (A/B/C/D/E) | `pipeline.rs` |
| `preprocess` | Bijective transform applied before compression | `preprocess/*.rs` |
| `entropy` | Shannon entropy in bits per byte [0.0, 8.0] | `block/detector.rs` |
| `stretch` | logit function (prob → log-odds), integer LUT | `compress/context_mix.rs` |
| `squash` | sigmoid function (log-odds → prob), integer LUT | `compress/context_mix.rs` |

---

## 3. Module Organization

### One Concept Per Module
Each `.rs` file should contain ONE clearly defined concept:
- `lz77.rs` — LZ77 encoder and decoder
- `rans.rs` — rANS entropy coder
- `bwt.rs` — Burrows-Wheeler Transform

### Module Layout Pattern
```rust
//! Module-level documentation explaining what this module does,
//! which layer of the NEXUS pipeline it belongs to, and key
//! algorithmic references.

// Standard library imports
use std::io::{Read, Write};

// External crate imports
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

// Internal crate imports
use crate::entropy::rans::RansEncoder;

// Constants (module-scoped)
const MAX_MATCH_LENGTH: usize = 258;

// Types
pub struct Lz77Encoder { ... }

// Implementation
impl Lz77Encoder { ... }

// Unit tests
#[cfg(test)]
mod tests { ... }
```

---

## 4. Error Handling

### Use a single error enum for the entire crate:

```rust
// In nexus-core/src/error.rs
#[derive(Debug, thiserror::Error)]
pub enum NexusError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid magic bytes: expected NXS\\x01, got {0:?}")]
    InvalidMagic([u8; 4]),

    #[error("Checksum mismatch: expected {expected:#x}, got {actual:#x}")]
    ChecksumMismatch { expected: u64, actual: u64 },

    #[error("Unsupported compression path: {0}")]
    UnsupportedPath(u8),

    #[error("Block decompression failed at offset {offset}: {reason}")]
    DecompressFailed { offset: u64, reason: String },

    #[error("Data corruption: {0}")]
    DataCorruption(String),
}

pub type Result<T> = std::result::Result<T, NexusError>;
```

### Rules:
- **Never use `.unwrap()` or `.expect()` in library code** (nexus-core)
- `.unwrap()` is allowed in test code and CLI (nexus-cli) for known-safe operations
- Use `?` operator for propagation
- Add context to errors with descriptive messages

---

## 5. Documentation

### Every public item MUST have a doc comment:

```rust
/// Compresses a sub-block using the LZ77 algorithm with hash-chain matching.
///
/// # Algorithm
/// Uses a sliding window of `window_size` bytes. Matches are found via
/// 3-byte hash chains. Greedy matching in Phase 1; optimal parsing
/// (price-based DP) added in Phase 2.
///
/// # Arguments
/// * `input` - Raw bytes to compress
/// * `window_size` - Sliding window size in bytes (default: 32768)
///
/// # Returns
/// A vector of LZ77 tokens (literals and match references).
///
/// # Errors
/// Returns `NexusError::Io` if reading input fails.
pub fn compress_lz77(input: &[u8], window_size: usize) -> Result<Vec<Lz77Token>> {
    // ...
}
```

### Internal functions SHOULD have brief comments explaining WHY, not WHAT:

```rust
// Hash the next 3 bytes for match-finder lookup.
// Using a simple shift-xor hash because MurmurHash is overkill
// for 3-byte keys and this is in the hot loop.
fn hash3(data: &[u8], pos: usize) -> u32 {
    // ...
}
```

---

## 6. Testing Patterns

### Unit Tests (inside each module):
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip_simple() {
        let input = b"hello hello hello world";
        let compressed = compress(input).unwrap();
        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(input.as_slice(), decompressed.as_slice());
    }

    #[test]
    fn test_roundtrip_empty() {
        let input = b"";
        let compressed = compress(input).unwrap();
        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(input.as_slice(), decompressed.as_slice());
    }

    #[test]
    fn test_roundtrip_single_byte() {
        let input = b"x";
        let compressed = compress(input).unwrap();
        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(input.as_slice(), decompressed.as_slice());
    }

    #[test]
    fn test_roundtrip_all_same_bytes() {
        let input = vec![0xAA; 100_000];
        let compressed = compress(&input).unwrap();
        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(input, decompressed);
    }

    #[test]
    fn test_roundtrip_random_data() {
        // Pseudo-random but deterministic
        let mut input = vec![0u8; 65536];
        let mut state: u32 = 12345;
        for byte in input.iter_mut() {
            state = state.wrapping_mul(1103515245).wrapping_add(12345);
            *byte = (state >> 16) as u8;
        }
        let compressed = compress(&input).unwrap();
        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(input, decompressed);
    }
}
```

### Integration Tests (in `tests/` directory):
- `roundtrip.rs` — Full pipeline compress→decompress for various file types
- Always test: empty, 1 byte, all-same, random, text, binary

### Property: EVERY module that transforms data must have roundtrip tests.

---

## 7. Performance Conventions

### Hot Path Rules:
- **No allocations in inner loops.** Pre-allocate buffers and reuse them.
- **No bounds checks in hot loops** when safety is proven. Use `unsafe { data.get_unchecked(i) }` with a comment explaining why it's safe.
- **Prefer `&[u8]` over `Vec<u8>`** for read-only data.
- **Use `byteorder` crate** for endian-safe reads/writes (always Little Endian in .nxs format).

### Integer Arithmetic Rules (for Path C and D):
- **NO `f32` or `f64`** in any code path that affects encoded output
- Probabilities: `u16` in range `[1, 4095]` (12-bit, never zero)
- Weights: `i16` (signed, for SGD updates)
- Stretch/squash: precomputed `[i32; 4096]` lookup tables
- All divisions: integer division with explicit rounding direction

---

## 8. Git Conventions

### Commit Messages:
```
<phase>(<module>): <short description>

Examples:
phase1(lz77): implement greedy match finder with hash chains
phase1(format): add .nxs header and footer serialization
phase2(rans): replace Huffman with rANS entropy coder
phase3(detector): add entropy-based data type classification
fix(bwt): correct off-by-one in suffix array construction
test(roundtrip): add random data roundtrip for all paths
```

### Branching:
- `main` — always compiles, all tests pass
- `phase-N` — work-in-progress for phase N
- Feature branches: `feat/<description>`

---

## 9. Dependency Policy

- **Minimize dependencies.** Every dependency is a security and maintenance risk.
- **Prefer `no_std` compatible crates** where possible.
- **Core algorithms (LZ77, BWT, rANS, arithmetic coding, context mixing) are implemented from scratch** — this is the entire point of the project.
- Dependencies are for infrastructure only: I/O, CLI, hashing, parallelism, testing.

### Approved Dependencies (nexus-core):
| Crate | Purpose | Justification |
|-------|---------|--------------|
| `byteorder` | Endian-safe byte I/O | Standard, tiny, no alternatives |
| `xxhash-rust` | Fast checksums | Industry standard, 10x faster than CRC32 |
| `thiserror` | Error derive macros | Reduces boilerplate |
| `rayon` | Parallel iteration | De facto standard for data parallelism |

### Approved Dependencies (nexus-cli only):
| Crate | Purpose |
|-------|---------|
| `clap` | CLI argument parsing |
| `indicatif` | Progress bars |

### DO NOT ADD without discussion:
- `serde` (we use custom binary serialization)
- `tokio` / `async-std` (no async needed — we use rayon for parallelism)
- Any compression crate (we're building our own!)

---

## 10. Binary Format Conventions

- **Byte order:** Always **Little Endian** (matches x86/ARM, no conversion needed)
- **Alignment:** No alignment requirements (tightly packed structures)
- **Magic bytes:** `NXS\x01` = `[0x4E, 0x58, 0x53, 0x01]`
- **End marker:** `NXSEND` = `[0x4E, 0x58, 0x53, 0x45, 0x4E, 0x44]`
- **Checksums:** XXHash32 for blocks, XXHash64 for full file
- **Version:** Format version as `u16`, starting at `1`

---

## 11. Phase-Aware Development

Each phase builds on the previous. When working on Phase N:
1. **Don't break Phase N-1 functionality.** All existing tests must pass.
2. **Use feature flags sparingly.** Prefer runtime compression levels over compile-time features.
3. **Update `docs/PHASE_STATUS.md`** when completing a milestone.
4. **Benchmark against the previous phase** to prove improvement.
