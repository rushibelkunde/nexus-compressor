# 02: .nxs File Format — Header, Footer, Mini-Header Serialization

**What to build:** Implement the .nxs binary format structs and their read/write methods. A file header (32 bytes) can be serialized to and deserialized from bytes. A sub-block mini-header (16 bytes) can be serialized/deserialized. A footer (24 bytes) with block table can be serialized/deserialized. Roundtrip tests prove write → read → equals original for all three structures.

**Blocked by:** 01 (Workspace Setup)

**Status:** done

- [x] `format/header.rs`: `FileHeader` struct with serialize/deserialize methods
- [x] `format/mini_header.rs`: `SubBlockHeader` struct with serialize/deserialize
- [x] `format/footer.rs`: `Footer` and `BlockTableEntry` structs with serialize/deserialize
- [x] `format/mod.rs`: re-exports, `BlockType` enum, `CompressionPath` enum, `PreprocessFlags` bitfield
- [x] Magic byte validation on read
- [x] End marker validation on read
- [x] Unit tests: roundtrip for each struct
- [x] Unit tests: reject invalid magic bytes
- [x] Unit tests: reject invalid end marker
