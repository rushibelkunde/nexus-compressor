//! Zero-Run-Length Encoding (Zero-RLE) transform (Layer 2).
//!
//! Efficiently compresses runs of zeros produced by BWT+MTF.
//! Strictly bijective: `zero_rle_decode(zero_rle_encode(data)) == data`.

use crate::error::{NexusError, Result};
use std::cmp;

/// Encodes consecutive runs of zero bytes while preserving non-zero bytes verbatim.
pub fn zero_rle_encode(input: &[u8]) -> Vec<u8> {
    if input.is_empty() {
        return Vec::new();
    }

    let mut output = Vec::with_capacity(input.len());
    let mut i = 0;

    while i < input.len() {
        if input[i] != 0 {
            output.push(input[i]);
            i += 1;
        } else {
            // Count run of zeros
            let start = i;
            while i < input.len() && input[i] == 0 {
                i += 1;
            }
            let mut run_len = i - start;

            // Emit in chunks of up to 255 zeros: [0x00, count - 1]
            while run_len > 0 {
                let chunk = cmp::min(run_len, 255);
                output.push(0x00);
                output.push((chunk - 1) as u8);
                run_len -= chunk;
            }
        }
    }

    output
}

/// Decodes a Zero-RLE encoded stream back to original bytes.
pub fn zero_rle_decode(input: &[u8]) -> Result<Vec<u8>> {
    let mut output = Vec::with_capacity(input.len());
    let mut i = 0;

    while i < input.len() {
        let b = input[i];
        if b != 0 {
            output.push(b);
            i += 1;
        } else {
            i += 1;
            if i >= input.len() {
                return Err(NexusError::DataCorruption(
                    "Zero-RLE stream terminated unexpectedly with solitary zero byte".into(),
                ));
            }
            let count = (input[i] as usize) + 1;
            i += 1;
            output.resize(output.len() + count, 0);
        }
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zero_rle_empty() {
        assert_eq!(zero_rle_encode(b""), Vec::<u8>::new());
        assert_eq!(zero_rle_decode(&[]).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn test_zero_rle_no_zeros() {
        let input = b"abcdef123456";
        let encoded = zero_rle_encode(input);
        assert_eq!(encoded, input);
        let decoded = zero_rle_decode(&encoded).unwrap();
        assert_eq!(decoded, input);
    }

    #[test]
    fn test_zero_rle_heavy_zeros() {
        let mut input = vec![0u8; 1000];
        input.push(42);
        input.extend_from_slice(&[0u8; 500]);

        let encoded = zero_rle_encode(&input);
        assert!(encoded.len() < 20); // 1500 zeros reduced to ~12 bytes
        let decoded = zero_rle_decode(&encoded).unwrap();
        assert_eq!(decoded, input);
    }

    #[test]
    fn test_zero_rle_mixed_data() {
        let input = vec![0, 1, 0, 0, 2, 0, 0, 0, 3, 4, 5, 0];
        let encoded = zero_rle_encode(&input);
        let decoded = zero_rle_decode(&encoded).unwrap();
        assert_eq!(decoded, input);
    }
}
