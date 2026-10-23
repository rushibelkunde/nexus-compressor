//! Move-to-Front (MTF) transform (Layer 2).
//!
//! Converts repetitive context symbols into small-integer-dominated streams.
//! Strictly bijective: `mtf_decode(mtf_encode(data)) == data`.

/// Encodes a byte slice using the Move-to-Front transform.
pub fn mtf_encode(input: &[u8]) -> Vec<u8> {
    if input.is_empty() {
        return Vec::new();
    }

    let mut list = [0u8; 256];
    for (i, item) in list.iter_mut().enumerate() {
        *item = i as u8;
    }

    let mut output = Vec::with_capacity(input.len());

    for &byte in input {
        // Find position of byte in list
        let mut pos = 0;
        while pos < 256 && list[pos] != byte {
            pos += 1;
        }

        output.push(pos as u8);

        // Move to front: shift elements [0..pos] right by 1
        let val = list[pos];
        for i in (1..=pos).rev() {
            list[i] = list[i - 1];
        }
        list[0] = val;
    }

    output
}

/// Decodes an MTF-transformed slice back to the original byte sequence.
pub fn mtf_decode(input: &[u8]) -> Vec<u8> {
    if input.is_empty() {
        return Vec::new();
    }

    let mut list = [0u8; 256];
    for (i, item) in list.iter_mut().enumerate() {
        *item = i as u8;
    }

    let mut output = Vec::with_capacity(input.len());

    for &idx in input {
        let pos = idx as usize;
        let val = list[pos];
        output.push(val);

        // Move to front
        for i in (1..=pos).rev() {
            list[i] = list[i - 1];
        }
        list[0] = val;
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mtf_empty() {
        assert_eq!(mtf_encode(b""), Vec::<u8>::new());
        assert_eq!(mtf_decode(&[]), Vec::<u8>::new());
    }

    #[test]
    fn test_mtf_single_byte() {
        let input = b"A";
        let encoded = mtf_encode(input);
        assert_eq!(encoded, vec![b'A']);
        let decoded = mtf_decode(&encoded);
        assert_eq!(decoded, input);
    }

    #[test]
    fn test_mtf_repeating_symbols() {
        let input = b"aaaaabbbbb";
        let encoded = mtf_encode(input);
        // After first 'a', subsequent 'a's will be 0
        assert_eq!(encoded[0], b'a');
        assert_eq!(encoded[1], 0);
        assert_eq!(encoded[2], 0);
        let decoded = mtf_decode(&encoded);
        assert_eq!(decoded, input);
    }

    #[test]
    fn test_mtf_all_256_bytes() {
        let input: Vec<u8> = (0..=255).collect();
        let encoded = mtf_encode(&input);
        let decoded = mtf_decode(&encoded);
        assert_eq!(decoded, input);
    }

    #[test]
    fn test_mtf_random_data() {
        let mut rng_state: u64 = 0x1122334455667788;
        let mut input = vec![0u8; 4096];
        for b in input.iter_mut() {
            rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
            *b = (rng_state >> 33) as u8;
        }

        let encoded = mtf_encode(&input);
        let decoded = mtf_decode(&encoded);
        assert_eq!(decoded, input);
    }
}
