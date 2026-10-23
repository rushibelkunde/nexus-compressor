# .nxs Format Specification — Project NEXUS

> Version: 1
> Status: Draft (Phase 1 implementation)

---

## 1. Overview

The `.nxs` file format is a binary container for NEXUS compressed data. It uses a
**footer-based block index** to enable streaming compression while supporting
random-access decompression.

**Byte order:** Little Endian throughout.

---

## 2. File Layout

```
Offset  Size    Description
─────────────────────────────────────────────────────
0       32      FILE HEADER
32      var     SUPERBLOCK DATA (sub-blocks with inline mini-headers)
var     var     BLOCK TABLE (random-access index)
var     24      FOOTER
─────────────────────────────────────────────────────
```

---

## 3. FILE HEADER (32 bytes)

```
Offset  Size  Type    Field               Description
──────────────────────────────────────────────────────────────
0       4     [u8;4]  magic               Magic bytes: [0x4E, 0x58, 0x53, 0x01] = "NXS\x01"
4       2     u16     format_version      Format version (currently 1)
6       2     u16     flags               Bitfield:
                                            bit 0: encryption enabled
                                            bit 1: XXHash64 checksums (vs XXHash32)
                                            bits 2-15: reserved (must be 0)
8       8     u64     original_size       Total uncompressed size in bytes
16      4     u32     superblock_count    Number of superblocks
20      12    [u8;12] reserved            Reserved for future use (must be 0)
──────────────────────────────────────────────────────────────
Total: 32 bytes
```

### Magic Bytes
- Always `[0x4E, 0x58, 0x53, 0x01]` (ASCII "NXS" + version byte 0x01)
- Readers MUST reject files that don't start with these 4 bytes

---

## 4. SUB-BLOCK MINI-HEADER (16 bytes, inline)

Each sub-block within the data section is preceded by its own mini-header:

```
Offset  Size  Type    Field               Description
──────────────────────────────────────────────────────────────
0       4     u32     compressed_size     Size of compressed data following this header
4       4     u32     original_size       Uncompressed size of this sub-block
8       1     u8      block_type          BlockType enum value (0-8)
9       1     u8      compression_path    Path used: 0=A(LZ-ANS), 1=B(BWT), 
                                          2=C(ContextMix), 3=D(Neural), 4=E(Store)
10      2     u16     preprocess_flags    Bitfield of preprocessing transforms applied:
                                            bit 0: delta encoding
                                            bit 1: MTF transform
                                            bit 2: RLE
                                            bit 3: x86 filter (E8/E9)
                                            bit 4: float byte-plane split
                                            bit 5: WRT (word replacing transform)
                                            bit 6: column transposition
                                            bits 7-15: reserved
12      4     u32     block_checksum      XXHash32 of the ORIGINAL (uncompressed) data
──────────────────────────────────────────────────────────────
Total: 16 bytes
```

### BlockType Values
```
0 = Text
1 = SourceCode
2 = StructuredData
3 = Binary
4 = Executable
5 = FloatingPoint
6 = Audio
7 = Image
8 = Incompressible
```

### Compression Path Values
```
0 = PATH_A (LZ77 + rANS)
1 = PATH_B (BWT + MTF + RLE + rANS)
2 = PATH_C (Context Mixing + Arithmetic Coding)
3 = PATH_D (Neural Predictor + Arithmetic Coding)
4 = PATH_E (Store / no compression)
```

### Superblock Boundaries
Superblocks are delimited by a special mini-header with `block_type = 0xFF` and
`compressed_size = 0`. This zero-byte marker indicates "end of superblock, start of next".

---

## 5. BLOCK TABLE (variable length, in footer area)

The block table provides random-access to any sub-block. It is written AFTER all compressed
data, enabling streaming compression.

### Block Table Entry (20 bytes per sub-block)

```
Offset  Size  Type    Field               Description
──────────────────────────────────────────────────────────────
0       4     u32     superblock_id       Which superblock this sub-block belongs to
4       8     u64     file_offset         Absolute byte offset of this sub-block's
                                          mini-header from start of file
12      4     u32     compressed_size     Same as mini-header (redundant for fast seeking)
16      4     u32     original_size       Same as mini-header (redundant for fast seeking)
──────────────────────────────────────────────────────────────
Total: 20 bytes per entry
```

Total block table size = `20 * total_sub_block_count` bytes.

---

## 6. FOOTER (24 bytes)

```
Offset  Size  Type    Field               Description
──────────────────────────────────────────────────────────────
0       8     u64     block_table_offset  Absolute byte offset of the block table
                                          from start of file
8       4     u32     total_block_count   Total number of sub-blocks
12      8     u64     file_checksum       XXHash64 of all bytes from offset 0 to
                                          start of footer
20      4     [u8;4]  end_marker          End marker: [0x4E, 0x58, 0x53, 0x45] = "NXSE"
──────────────────────────────────────────────────────────────
Total: 24 bytes (reduced from 6-byte marker to 4 for alignment)
```

### Reading the Footer (Random Access)
1. Seek to `file_size - 24`
2. Read 24 bytes
3. Verify `end_marker == "NXSE"`
4. Read `block_table_offset` → seek to that offset → read block table
5. Now you can jump to any sub-block by its `file_offset`

### Streaming Read (No Seeking)
Just read sub-blocks sequentially: each mini-header tells you how many bytes to read next.
The block table at the end is optional for sequential reading.

---

## 7. Checksum Policy

| Scope | Algorithm | Size | Where Stored |
|-------|-----------|------|-------------|
| Per sub-block | XXHash32 | 4 bytes | Mini-header `block_checksum` |
| Entire file | XXHash64 | 8 bytes | Footer `file_checksum` |

- Block checksums are computed over the **original (uncompressed)** sub-block data
- File checksum is computed over all bytes from offset 0 to `footer_offset - 1`
- Decompressors SHOULD verify checksums but MAY skip for performance

---

## 8. Versioning

- `format_version = 1`: Phase 1-6 format (this document)
- Future versions MAY add new compression paths, block types, or preprocess flags
- Readers MUST reject files with `format_version > supported_version`
- New block types and paths use reserved enum values (9-255 for types, 5-255 for paths)

---

## 9. Example File (Annotated Hex Dump)

```
# FILE HEADER (32 bytes)
4E 58 53 01    # magic: "NXS\x01"
01 00          # format_version: 1
00 00          # flags: none
00 10 00 00 00 00 00 00  # original_size: 4096 bytes
01 00 00 00    # superblock_count: 1
00 00 00 00 00 00 00 00 00 00 00 00  # reserved

# SUB-BLOCK 0 MINI-HEADER (16 bytes)
80 03 00 00    # compressed_size: 896 bytes
00 10 00 00    # original_size: 4096 bytes
00             # block_type: 0 (Text)
00             # compression_path: 0 (PATH_A = LZ-ANS)
00 00          # preprocess_flags: none
AB CD EF 12    # block_checksum: XXHash32 of original data

# SUB-BLOCK 0 COMPRESSED DATA (896 bytes)
[... 896 bytes of compressed data ...]

# BLOCK TABLE (20 bytes, 1 entry)
00 00 00 00    # superblock_id: 0
20 00 00 00 00 00 00 00  # file_offset: 32 (right after header)
80 03 00 00    # compressed_size: 896
00 10 00 00    # original_size: 4096

# FOOTER (24 bytes)
B0 03 00 00 00 00 00 00  # block_table_offset: 944 (32 + 16 + 896)
01 00 00 00    # total_block_count: 1
[8 bytes]      # file_checksum: XXHash64
4E 58 53 45    # end_marker: "NXSE"
```
