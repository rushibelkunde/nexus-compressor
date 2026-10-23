# 27: Cross-Corpus Benchmark Suite

**What to build:** Build a full-featured benchmark runner in `crates/nexus-bench`:
- Benchmark datasets:
  - Natural Language Text (English books / Wikipedia / Canterbury text)
  - Executable Machine Code (x86-64 / PE / ELF patterns)
  - Scientific IEEE 754 Floating-Point Array
  - 16-bit PCM Audio Waveform
  - Structured Data (JSON / CSV with recurring records)
  - Incompressible Uniform Random Stream
- Test each dataset across:
  - Level 1 (Fast LZ-rANS)
  - Level 5 (Balanced LZ-rANS)
  - Level 7 (Adaptive BWT / MTF)
  - Level 13 (Context Mixing)
  - Level 17 (Neural Prediction)
- Measure:
  - Original size vs compressed size
  - Compression ratio (% and bpc - bits per character)
  - Compression speed (MB/s)
  - Decompression speed (MB/s)
  - Verification check: lossless roundtrip bit-exact match

**Blocked by:** 25, 26

**Status:** done

- [x] `crates/nexus-bench/src/main.rs`: Multi-dataset benchmark suite with formatted terminal table output
- [x] Benchmarks runnable via `cargo run -p nexus-bench`
