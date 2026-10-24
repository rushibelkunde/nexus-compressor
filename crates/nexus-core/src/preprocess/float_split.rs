//! IEEE 754 Floating-Point Byte-Plane Splitting (Layer 2).
//!
//! De-interleaves 32-bit (f32) or 64-bit (f64) float streams into separate
//! contiguous byte planes (sign/exponent plane, mantissa planes), grouping
//! similar bytes together for dramatic entropy reduction.
//! Strictly bijective: `float_split_decode(float_split_encode(data, width), width) == data`.

/// Splits interleaved float arrays into contiguous byte planes of width `width` (typically 4 or 8).
pub fn float_split_encode(input: &[u8], width: usize) -> Vec<u8> {
    let w = if width == 0 { 4 } else { width };
    let n = input.len();
    if n == 0 || n < w {
        return input.to_vec();
    }

    let m = n / w;
    let aligned_len = m * w;
    let mut output = vec![0u8; n];

    // Transpose aligned blocks into planes
    for p in 0..w {
        for i in 0..m {
            output[p * m + i] = input[i * w + p];
        }
    }

    // Copy any unaligned tail bytes directly
    if aligned_len < n {
        output[aligned_len..n].copy_from_slice(&input[aligned_len..n]);
    }

    output
}

/// Restores contiguous byte planes back to interleaved float bytes.
pub fn float_split_decode(input: &[u8], width: usize) -> Vec<u8> {
    let w = if width == 0 { 4 } else { width };
    let n = input.len();
    if n == 0 || n < w {
        return input.to_vec();
    }

    let m = n / w;
    let aligned_len = m * w;
    let mut output = vec![0u8; n];

    // Invert plane transposition
    for p in 0..w {
        for i in 0..m {
            output[i * w + p] = input[p * m + i];
        }
    }

    // Copy unaligned tail bytes
    if aligned_len < n {
        output[aligned_len..n].copy_from_slice(&input[aligned_len..n]);
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_float_split_empty() {
        assert_eq!(float_split_encode(b"", 4), Vec::<u8>::new());
        assert_eq!(float_split_decode(&[], 4), Vec::<u8>::new());
    }

    #[test]
    fn test_float_split_f32_array() {
        // Simulated array of float values
        let values: [f32; 8] = [1.0, 1.05, 1.10, 1.15, 1.20, 1.25, 1.30, 1.35];
        let mut input = Vec::new();
        for &val in &values {
            input.extend_from_slice(&val.to_le_bytes());
        }

        let encoded = float_split_encode(&input, 4);
        // Exponent bytes (byte 3 in LE) should be grouped together at the end of encoded
        let decoded = float_split_decode(&encoded, 4);
        assert_eq!(decoded, input);
    }

    #[test]
    #[allow(clippy::approx_constant)]
    fn test_float_split_f64_array() {
        let values: [f64; 6] = [3.14159, 2.71828, 1.41421, 1.61803, 0.57721, 4.66920];
        let mut input = Vec::new();
        for &val in &values {
            input.extend_from_slice(&val.to_le_bytes());
        }

        let encoded = float_split_encode(&input, 8);
        let decoded = float_split_decode(&encoded, 8);
        assert_eq!(decoded, input);
    }

    #[test]
    fn test_float_split_with_unaligned_tail() {
        let mut input = vec![1, 2, 3, 4, 5, 6, 7, 8];
        input.extend_from_slice(&[99, 100]); // 2 extra bytes (unaligned)

        let encoded = float_split_encode(&input, 4);
        let decoded = float_split_decode(&encoded, 4);
        assert_eq!(decoded, input);
    }
}
