//! Bijective x86 BCJ Branch Filter (Layer 2).
//!
//! Converts relative 32-bit jump and call offsets (0xE8 CALL, 0xE9 JMP)
//! into absolute addresses on encode, and back to relative on decode.
//! Aligns function targets across executable binaries for higher dictionary compression.
//! Strictly bijective: `x86_filter_decode(x86_filter_encode(data)) == data`.

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

/// Translates relative 32-bit CALL/JMP offsets to absolute addresses.
pub fn x86_filter_encode(input: &[u8]) -> Vec<u8> {
    let n = input.len();
    if n < 5 {
        return input.to_vec();
    }

    let mut output = input.to_vec();
    let mut i = 0;

    while i + 5 <= n {
        let op = output[i];
        if op == 0xE8 || op == 0xE9 {
            let mut cursor = &output[i + 1..i + 5];
            let rel = cursor.read_i32::<LittleEndian>().unwrap();
            let abs = rel.wrapping_add((i as i32) + 5);

            let mut write_cursor = &mut output[i + 1..i + 5];
            write_cursor.write_i32::<LittleEndian>(abs).unwrap();
            i += 5;
        } else {
            i += 1;
        }
    }

    output
}

/// Translates absolute addresses back to relative 32-bit CALL/JMP offsets.
pub fn x86_filter_decode(input: &[u8]) -> Vec<u8> {
    let n = input.len();
    if n < 5 {
        return input.to_vec();
    }

    let mut output = input.to_vec();
    let mut i = 0;

    while i + 5 <= n {
        let op = output[i];
        if op == 0xE8 || op == 0xE9 {
            let mut cursor = &output[i + 1..i + 5];
            let abs = cursor.read_i32::<LittleEndian>().unwrap();
            let rel = abs.wrapping_sub((i as i32) + 5);

            let mut write_cursor = &mut output[i + 1..i + 5];
            write_cursor.write_i32::<LittleEndian>(rel).unwrap();
            i += 5;
        } else {
            i += 1;
        }
    }

    output
}

/// True if a little-endian rel32 operand looks like a near branch target:
/// its top byte is a pure sign extension (0x00 or 0xFF).
#[inline(always)]
fn is_near(top: u8) -> bool {
    top == 0x00 || top == 0xFF
}

/// Adds (`forward`) or subtracts `delta` from a 25-bit signed value, wrapping
/// within 25 bits so near targets map to near targets.
#[inline(always)]
fn shift25(value: u32, delta: u32, forward: bool) -> u32 {
    let v = if forward {
        value.wrapping_add(delta)
    } else {
        value.wrapping_sub(delta)
    } & 0x01FF_FFFF;
    // Sign-extend bit 24 into the top byte.
    if v & 0x0100_0000 != 0 {
        v | 0xFE00_0000
    } else {
        v
    }
}

#[inline(always)]
fn e8e9_transform(data: &mut [u8], base: u32, forward: bool) {
    let n = data.len();
    if n < 5 {
        return;
    }
    let mut i = 0;
    while i + 5 <= n {
        // 0xE8 = CALL rel32, 0xE9 = JMP rel32.
        if data[i] & 0xFE == 0xE8 {
            if is_near(data[i + 4]) {
                let v = u32::from_le_bytes([data[i + 1], data[i + 2], data[i + 3], data[i + 4]]);
                let pos = base.wrapping_add(i as u32).wrapping_add(5);
                let t = shift25(v, pos, forward);
                data[i + 1..i + 5].copy_from_slice(&t.to_le_bytes());
            }
            // Always step over the operand, converted or not. The decision
            // at `i` then depends only on the opcode byte (never rewritten)
            // and on the operand's own top byte (whose near-ness the
            // transform preserves), so encoder and decoder always agree.
            i += 5;
        } else {
            i += 1;
        }
    }
}

/// E8/E9 branch filter, version 2 (in place).
///
/// Rewrites the rel32 operand of near CALL/JMP instructions as an absolute
/// target (`operand + position`), so every call to the same function becomes
/// the same byte string. Compared with [`x86_filter_encode`] it:
///
/// * only touches operands whose top byte is a sign extension, i.e. real
///   near branches, leaving data bytes that happen to equal `E8` alone;
/// * does the addition modulo 2^25, which maps the set of near operands onto
///   itself and therefore stays bijective for every input;
/// * takes `base`, the position of `data[0]` within the superblock, so
///   targets agree across sub-block boundaries.
pub fn x86_e8e9_encode(data: &mut [u8], base: u32) {
    e8e9_transform(data, base, true);
}

/// Exact inverse of [`x86_e8e9_encode`] with the same `base`.
pub fn x86_e8e9_decode(data: &mut [u8], base: u32) {
    e8e9_transform(data, base, false);
}

/// Heuristic: does this block contain enough near CALL instructions for the
/// branch filter to pay off? Requires roughly one call per 400 bytes.
pub fn looks_like_x86_code(data: &[u8]) -> bool {
    if data.len() < 1024 {
        return false;
    }
    let mut calls = 0usize;
    for w in data.windows(5) {
        if w[0] == 0xE8 && is_near(w[4]) {
            calls += 1;
        }
    }
    calls * 400 > data.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_x86_empty() {
        assert_eq!(x86_filter_encode(b""), Vec::<u8>::new());
        assert_eq!(x86_filter_decode(&[]), Vec::<u8>::new());
    }

    #[test]
    fn test_x86_short() {
        let input = b"\xE8\x01\x02";
        assert_eq!(x86_filter_encode(input), input);
        assert_eq!(x86_filter_decode(input), input);
    }

    #[test]
    fn test_x86_simulated_code() {
        // Two calls to the same target function at offset 0x1000 from different call sites
        let mut code = vec![0x90; 100]; // NOP sled
                                        // Call site 1 at offset 10: target 0x1000 -> rel = 0x1000 - (10 + 5) = 4081
        code[10] = 0xE8;
        (&mut code[11..15]).write_i32::<LittleEndian>(4081).unwrap();

        // Call site 2 at offset 50: target 0x1000 -> rel = 0x1000 - (50 + 5) = 4041
        code[50] = 0xE8;
        (&mut code[51..55]).write_i32::<LittleEndian>(4041).unwrap();

        let encoded = x86_filter_encode(&code);
        // After transform, both calls should encode identical absolute address 0x1000 (4096)
        let addr1 = (&encoded[11..15]).read_i32::<LittleEndian>().unwrap();
        let addr2 = (&encoded[51..55]).read_i32::<LittleEndian>().unwrap();
        assert_eq!(addr1, 4096);
        assert_eq!(addr2, 4096);

        // Decompress must restore exact original relative offsets
        let decoded = x86_filter_decode(&encoded);
        assert_eq!(decoded, code);
    }

    #[test]
    fn test_x86_random_data() {
        let mut rng: u64 = 0xDEADBEEFCAFEBABE;
        let mut input = vec![0u8; 8192];
        for b in input.iter_mut() {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            *b = (rng >> 33) as u8;
        }

        let encoded = x86_filter_encode(&input);
        let decoded = x86_filter_decode(&encoded);
        assert_eq!(decoded, input);
    }

    fn lcg_bytes(n: usize, seed: u64) -> Vec<u8> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s = s
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (s >> 33) as u8
            })
            .collect()
    }

    #[test]
    fn test_e8e9_roundtrip_random_and_adversarial() {
        for seed in 0..40u64 {
            let mut data = lcg_bytes(4000 + seed as usize * 13, seed);
            // Seed plenty of opcodes with every kind of operand top byte.
            for i in (0..data.len().saturating_sub(8)).step_by(7) {
                data[i] = if i % 2 == 0 { 0xE8 } else { 0xE9 };
                data[i + 4] = [0x00, 0xFF, 0x01, 0xFE, 0x7F, 0x80][i % 6];
            }
            let original = data.clone();
            let base = (seed as u32).wrapping_mul(0x0123_4567);
            x86_e8e9_encode(&mut data, base);
            x86_e8e9_decode(&mut data, base);
            assert_eq!(data, original, "seed {seed}");
        }
    }

    #[test]
    fn test_e8e9_roundtrip_all_e8_bytes() {
        let original = vec![0xE8u8; 1000];
        let mut data = original.clone();
        x86_e8e9_encode(&mut data, 12345);
        x86_e8e9_decode(&mut data, 12345);
        assert_eq!(data, original);
    }

    #[test]
    fn test_e8e9_makes_repeated_call_targets_identical() {
        // Two calls to the same absolute target from different positions.
        let mut data = vec![0x90u8; 64];
        let target = 0x400u32;
        for &at in &[8usize, 40] {
            data[at] = 0xE8;
            let rel = target.wrapping_sub(at as u32 + 5);
            data[at + 1..at + 5].copy_from_slice(&rel.to_le_bytes());
        }
        assert_ne!(data[9..13], data[41..45]);
        x86_e8e9_encode(&mut data, 0);
        assert_eq!(data[9..13], data[41..45]);
    }

    #[test]
    fn test_looks_like_x86_code_rejects_text_and_random() {
        let text = b"plain text with no machine code at all. ".repeat(100);
        assert!(!looks_like_x86_code(&text));
        assert!(!looks_like_x86_code(&lcg_bytes(100_000, 7)));
        let mut code = vec![0x90u8; 10_000];
        for i in (0..9_990).step_by(40) {
            code[i] = 0xE8;
            code[i + 1..i + 5].copy_from_slice(&((i as u32) * 3).to_le_bytes());
        }
        assert!(looks_like_x86_code(&code));
    }
}
