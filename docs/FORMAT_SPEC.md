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
9       1     u8      compression_path    Path used (see "Compression Path Values")
10      2     u16     preprocess_flags    Bitfield of preprocessing transforms applied:
                                            bit 0: delta encoding
                                            bit 1: MTF transform
                                            bit 2: RLE
                                            bit 3: x86 filter (E8/E9)
                                            bit 4: float byte-plane split
                                            bit 5: WRT (word replacing transform)
                                            bit 6: column transposition
                                            bit 7: x86 E8/E9 branch filter v2
                                            bits 8-15: reserved
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
0 = PATH_A  (v1, read-only) byte-tagged LZ77 tokens + order-0 rANS
1 = PATH_B  (v1, read-only) BWT + MTF + RLE + order-0 rANS
2 = PATH_C  Context Mixing + Arithmetic Coding
3 = PATH_D  Neural Predictor + Arithmetic Coding
4 = PATH_E  Store / no compression
5 = PATH_A2 (v2) sequence LZ, repeat offsets, split interleaved-rANS streams
6 = PATH_B2 (v2) BWT over suffix order + adaptive bit model + range coder
```

Writers of format v2 never emit paths 0 and 1; readers must still decode them.
The payload layouts of paths 5 and 6 are specified in section 10.

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

- `format_version = 1`: Phase 1-6 format (paths 0-4)
- `format_version = 2`: adds paths 5 and 6 and preprocess flag bit 7; a stored
  block (path 4) may now carry preprocess flags, in which case its payload is
  the *preprocessed* bytes. The container (header, mini-headers, block table,
  footer) is byte-for-byte unchanged, so a v2 reader decodes v1 files as-is.
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

---

## 10. Format v2 Payloads

All integers are little-endian. `varint` is LEB128 (7 bits per byte, low
group first, high bit = "more bytes follow").

### 10.1 The preprocessed domain

Within a superblock, sub-blocks are decoded in order into one buffer of
*preprocessed* bytes: the output of each path **before** its inverse
preprocessing is applied. LZ matches (path 5) refer to this buffer, so they
can reach back to the first byte of the superblock regardless of how the
earlier sub-blocks were coded. After a sub-block is decoded, the inverse
transforms named by its `preprocess_flags` produce the original bytes, which
is what `block_checksum` covers.

Preprocess flag bit 7 (E8/E9 v2): for every byte `E8` or `E9` at offset `i`
of the sub-block, the following four bytes are treated as a little-endian
operand and the scan then continues at `i + 5` (whether or not the operand
was changed). The operand is changed only if its top byte is `00` or `FF`:
the encoder replaces `v` with `sext25((v + base + i + 5) mod 2^25)`, the
decoder with `sext25((v - base - i - 5) mod 2^25)`, where `base` is the
offset of the sub-block within its superblock and `sext25` sign-extends bit
24 into bits 25-31.

### 10.2 Entropy stream record

Used for every byte stream in path 5:

```
u8      mode        0 = raw, 1 = single repeated byte, 2 = rANS
varint  length      number of decoded bytes
mode 0: `length` raw bytes
mode 1: u8 value
mode 2: frequency table, varint payload_len, payload
```

**Frequency table.** `u8 n-1` (`n` = highest present symbol + 1), then for
symbols `0..n` in order: a zero byte followed by `u8 extra` meaning
`extra + 1` consecutive absent symbols; or the frequency as one byte
(`1..=127`) or two bytes (`0x80 | (f >> 8)`, `f & 0xFF`). Frequencies sum to
exactly 4096.

**rANS payload.** Eight interleaved states, 12-bit probabilities, 16-bit
renormalisation, state interval `[2^15, 2^31)`. The payload is eight `u32`
initial states (state 0 first) followed by `u16` words. Symbol `i` uses state
`i mod 8`. To decode a symbol from state `x`: `slot = x & 4095`, look up the
symbol `s` whose cumulative range contains `slot`, then
`x = freq[s] * (x >> 12) + slot - cum[s]`; if `x < 2^15` then
`x = (x << 16) | next_u16()`. Symbols are decoded in order `0, 1, 2, ...`
and renormalisation words are consumed in that same order.

### 10.3 PATH_A2 (LZ-ANS v2) payload

```
varint  sequence_count
        literals              entropy stream record, OR a context-coded
                              record (first byte 3, section 10.4)
-- only if sequence_count > 0 --
        ll_codes              entropy stream record, one byte per sequence
        ml_codes              entropy stream record, one byte per sequence
        of_codes              entropy stream record, one byte per sequence
varint  extra_bits_len
        extra bits            LSB-first bit packing
```

**Value codes.** A value `v < 16` is its own code with no extra bits. For
`v >= 16`, let `hb` be the index of its highest set bit: the code is
`16 + 2*(hb - 4) + ((v >> (hb - 1)) & 1)` and the low `hb - 1` bits of `v`
follow as extra bits. Codes above 71 are invalid.

**Sequences.** For each sequence, in order: literal length `ll` (value code
from `ll_codes`), match length minus 3 (value code from `ml_codes`), and an
offset code from `of_codes`. Extra bits are read in that same order.

**Offsets.** Three repeat offsets start at `[1, 4, 8]` in every sub-block.
Offset code 0 uses `rep[0]`; code 1 uses `rep[1]` and swaps it to the front;
code 2 uses `rep[2]` and rotates it to the front; code `c >= 3` is a value
code `c - 3` for `offset - 1`, and the new offset is pushed to the front
(`rep = [offset, rep[0], rep[1]]`). An offset larger than the number of
bytes already in the superblock buffer is an error.

**Execution.** Copy `ll` literals, then copy `match_length` bytes from
`offset` bytes back (byte by byte semantics: overlapping copies replicate).
After the last sequence the remaining literals are appended; the total must
equal `original_size` exactly.

### 10.4 Context-coded literals (first byte 3)

Each literal is coded with a frequency table chosen by the byte that
precedes it in the superblock buffer (0 for the very first byte).

```
u8      3
varint  literal_count
u8      class_count           2..=16
        context map           entropy stream record of 256 class ids
        class_count frequency tables (section 10.2 format)
varint  payload_len
        payload               four u32 rANS states, then u16 words
```

Literal `i` (counting all literals of the sub-block in output order) uses
state `i mod 4` and the table of class `map[previous_byte]`; the decode step
is the same as in section 10.2.

### 10.5 PATH_B2 (BWT + adaptive model) payload

```
varint  primary_index         1..=original_size
        range-coded bytes
```

**Transform.** Let `SA` be the suffix array of the sub-block `T` of length
`n` (a suffix that is a prefix of another sorts first). The transformed
string is `T[n-1]` followed by `T[SA[i] - 1]` for every `i` with
`SA[i] != 0`, in order; `primary_index` is `i + 1` for the `i` with
`SA[i] == 0`.

**Model.** Bytes of the transformed string are coded MSB-first, 8 binary
decisions per byte, with a partial-byte context `ctx` that starts at 1 and
becomes `2*ctx + bit`. All probabilities are 16-bit integers `P(bit = 1)`,
initially 32768, updated by `p += (65535 - p) >> r` for a 1 bit and
`p -= p >> r` for a 0 bit. With `c1` the previous byte, `c2` the byte before
it (both initially 0) and `run` the number of consecutive bytes equal to
their predecessor:

```
p0 = order0[ctx]                      r = 2
p1 = order1[c1][ctx]                  r = 4
p2 = order2[c2][ctx]                  r = 4
m  = ((p0 + p1) * 7 + p2 * 2) >> 4
k  = m >> 12
a  = sse[run > 2][ctx][k],  b = sse[run > 2][ctx][k + 1]      r = 6 (both)
s  = a + (((b - a) * (m & 4095)) >> 12)
p  = clamp((m + 3 * s) >> 2, 1, 65535)
```

`sse[..][..][k]` is initialised to `min(4096 * k, 65535)` for `k = 0..=16`.

**Range coder.** 32-bit carry-less binary coder: `x1 = 0`, `x2 = 2^32 - 1`;
`xmid = x1 + ((x2 - x1) >> 16) * p`; a 1 bit sets `x2 = xmid`, a 0 bit sets
`x1 = xmid + 1`; while the top bytes of `x1` and `x2` are equal, output that
byte and shift both left by 8 (shifting 1-bits into `x2`). The encoder ends
by writing `x1` big-endian.
