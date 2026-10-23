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
}
