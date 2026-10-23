# 05: Block Splitter + Full Pipeline + CLI

**What to build:** Connect LZ77 + Huffman + .nxs format into a working end-to-end compression/decompression pipeline. Implement a fixed-size block splitter (split input into 1MB superblocks, each containing one sub-block). Build the CLI with `compress` and `decompress` subcommands. After this ticket, `nexus compress input.txt output.nxs && nexus decompress output.nxs restored.txt && diff input.txt restored.txt` works.

**Blocked by:** 02 (File Format), 03 (LZ77), 04 (Huffman)

**Status:** done

- [x] `block/splitter.rs`: `split_into_superblocks(data, block_size) -> Vec<Superblock>`
- [x] `pipeline.rs`: `compress(input: &[u8], level: u32) -> Result<Vec<u8>>`
  - Split into superblocks (fixed 1MB)
  - For each sub-block: LZ77 encode → serialize tokens → Huffman encode
  - Write file header, mini-headers, compressed data, block table, footer
- [x] `pipeline.rs`: `decompress(compressed: &[u8]) -> Result<Vec<u8>>`
  - Read file header, validate magic
  - For each sub-block: read mini-header → Huffman decode → deserialize tokens → LZ77 decode
  - Read footer, validate checksum
- [x] XXHash32 per-block checksums (verify on decompress)
- [x] XXHash64 full-file checksum (verify on decompress)
- [x] `nexus-cli/main.rs`: `compress` and `decompress` subcommands with clap
- [x] Integration test: roundtrip empty file
- [x] Integration test: roundtrip 1-byte file
- [x] Integration test: roundtrip 100KB text file
- [x] Integration test: roundtrip 1MB random data
- [x] Integration test: roundtrip file larger than 1 superblock (multi-block)
- [x] Print compression ratio and speed to stderr
