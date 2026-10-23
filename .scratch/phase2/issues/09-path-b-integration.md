# 09: Path B Integration & rANS Pipeline Upgrade

**What to build:** Integrate Path B (BWT → MTF → Zero-RLE → rANS) and upgrade Path A to use rANS.
The pipeline automatically routes Text and SourceCode sub-blocks to Path B, and Binary to Path A.
Decompressor reads `mini_header.compression_path` and decodes the appropriate path.
Fallback to Path E (Store) if compressed size >= original size.

**Blocked by:** 06 (rANS), 07 (BWT), 08 (MTF/RLE)

**Status:** done

- [x] Assemble Path B encoding in `pipeline.rs`: `bwt -> mtf -> zero_rle -> rans`
- [x] Assemble Path B decoding in `pipeline.rs`: `rans -> zero_rle -> mtf -> bwt`
- [x] Upgrade Path A in `pipeline.rs`: `lz77 -> serialize_tokens -> rans`
- [x] Routing logic: Text/SourceCode -> Path B; Binary -> Path A; fallback to Path E
- [x] Mini-header preprocess flags set correctly (e.g. MTF, RLE)
- [x] Unit & integration tests for Path B on natural text and source code
- [x] Unit & integration tests for Path A on binary data
