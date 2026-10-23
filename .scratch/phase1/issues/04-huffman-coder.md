# 04: Huffman Encoder and Decoder

**What to build:** A canonical Huffman coder that encodes/decodes byte streams. Given byte frequency counts, it builds an optimal prefix code tree. The encoder writes a compact header (code lengths only) followed by the encoded bitstream. The decoder reconstructs the original bytes. Roundtrip tests prove `decode(encode(data)) == data`.

**Blocked by:** 01 (Workspace Setup)

**Status:** done

- [x] `entropy/huffman.rs`: `HuffmanEncoder` struct
- [x] Build Huffman tree from frequency counts (min-heap)
- [x] Generate canonical codes from tree
- [x] `encode(input: &[u8]) -> Vec<u8>` — writes header + bitstream
- [x] `HuffmanDecoder` struct
- [x] `decode(compressed: &[u8]) -> Vec<u8>` — reads header, decodes bitstream
- [x] Bit-level I/O helpers (BitWriter, BitReader)
- [x] Handle edge case: input with only 1 distinct symbol
- [x] Handle edge case: empty input
- [x] Unit test: roundtrip empty
- [x] Unit test: roundtrip single byte repeated
- [x] Unit test: roundtrip all 256 byte values
- [x] Unit test: roundtrip English text
- [x] Unit test: roundtrip random data
