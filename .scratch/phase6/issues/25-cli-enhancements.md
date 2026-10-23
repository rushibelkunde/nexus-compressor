# 25: CLI Enhancements & Archive Inspection

**What to build:** Enhance `crates/nexus-cli` with rich flags and inspection capabilities:
- `--threads <N>` / `-t <N>`: Configures Rayon thread pool for compression/decompression.
- `--verbose` / `-v`: Displays detailed per-block telemetry:
  - Superblock ID and sub-block ID
  - Detected block type (Text, Executable, Float, Audio, Structured)
  - Applied preprocessors (x86 filter, Delta, Float split, MTF, RLE)
  - Selected compression path (Path A, B, C, D, E)
  - Compressed vs original size and ratio %
- `nexus list <archive.nxs>` subcommand:
  - Parses header, block table, and footer.
  - Displays summary table of all blocks without decompressing payload.

**Blocked by:** Phase 5 complete

**Status:** done

- [x] `crates/nexus-cli/src/main.rs`: Add `--threads`, `--verbose`, and `list` subcommand
- [x] Integration tests verifying CLI flags and outputs
