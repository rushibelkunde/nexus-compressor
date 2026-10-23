# 13: Advanced Content Detector & Statistical Classifier

**What to build:** Upgrade `block/detector.rs` to accurately detect data formats and classify
sub-blocks into `BlockType` variants.
- Magic byte detection: PE (`MZ`), ELF (`\x7FELF`), Mach-O, WAV (`RIFF`), PNG, JPEG, PDF, ZIP.
- Executable code detection (high x86 opcode frequency).
- Float pattern detection (IEEE 754 exponent distributions).
- Periodicity detection for audio and structured/tabular records.
- Entropy profiling (H < 7.0 text/code, H >= 7.8 incompressible).

**Blocked by:** Phase 2 complete

**Status:** done

- [x] `block/detector.rs`: Magic byte signature table
- [x] `detect_executable`: identifies PE / ELF headers and instruction distributions
- [x] `detect_floating_point`: detects IEEE 754 float arrays
- [x] `detect_audio`: detects PCM wave headers and sample autocorrelation
- [x] Classification mapped accurately to `BlockType` enum variants
- [x] Unit tests for all detected types
