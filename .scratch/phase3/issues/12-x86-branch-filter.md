# 12: Bijective x86 BCJ Branch Filter

**What to build:** Implement a bijective x86 branch filter in `preprocess/x86.rs`.
x86 machine code contains relative jump and call targets (`0xE8` for CALL, `0xE9` for JMP).
The filter translates relative jump offsets into absolute addresses on encode, which aligns identical
function targets across the binary, dramatically increasing redundancy for LZ77 and rANS.
The decode step maps absolute addresses back to original relative offsets.
Must be strictly bijective for all binary sequences: `decode(encode(data)) == data`.

**Blocked by:** Phase 2 complete

**Status:** done

- [x] `preprocess/x86.rs`: `x86_filter_encode(input: &[u8]) -> Vec<u8>`
- [x] `preprocess/x86.rs`: `x86_filter_decode(input: &[u8]) -> Vec<u8>`
- [x] Correct relative-to-absolute address translation (taking current offset into account)
- [x] Boundary safety (handling instructions near end of block)
- [x] Unit tests: empty input
- [x] Unit tests: simulated x86 code with repeated CALL/JMP targets
- [x] Unit tests: arbitrary binary data and edge-case offsets
