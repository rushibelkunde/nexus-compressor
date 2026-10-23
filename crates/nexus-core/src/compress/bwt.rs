//! Burrows-Wheeler Transform (BWT) for text and structured data (Path B).
//!
//! Permutes symbols so identical preceding contexts cluster together,
//! drastically increasing zero-run density after Move-to-Front.
//! Inversion uses the O(N) LF-mapping algorithm.

use crate::error::{NexusError, Result};

/// Computes the forward Burrows-Wheeler Transform.
///
/// Returns a tuple of `(transformed_bytes, primary_index)`.
pub fn bwt_encode(input: &[u8]) -> (Vec<u8>, u32) {
    let n = input.len();
    if n == 0 {
        return (Vec::new(), 0);
    }
    if n == 1 {
        return (input.to_vec(), 0);
    }

    // Create doubled buffer so cyclic comparison does not need modulo division
    let mut double_input = Vec::with_capacity(n * 2);
    double_input.extend_from_slice(input);
    double_input.extend_from_slice(input);

    // Sort cyclic rotation indices with pdqsort (sort_unstable_by)
    let mut indices: Vec<usize> = (0..n).collect();

    indices.sort_unstable_by(|&a, &b| {
        if a == b {
            return std::cmp::Ordering::Equal;
        }

        let slice_a = &double_input[a..a + n];
        let slice_b = &double_input[b..b + n];
        slice_a.cmp(slice_b)
    });

    let mut bwt_output = Vec::with_capacity(n);
    let mut primary_index = 0u32;

    for (row, &idx) in indices.iter().enumerate() {
        if idx == 0 {
            primary_index = row as u32;
            bwt_output.push(input[n - 1]);
        } else {
            bwt_output.push(input[idx - 1]);
        }
    }

    (bwt_output, primary_index)
}

/// Inverts the Burrows-Wheeler Transform using O(N) LF-mapping.
pub fn bwt_decode(bwt_data: &[u8], primary_index: u32) -> Result<Vec<u8>> {
    let n = bwt_data.len();
    if n == 0 {
        return Ok(Vec::new());
    }
    if n == 1 {
        return Ok(bwt_data.to_vec());
    }
    if (primary_index as usize) >= n {
        return Err(NexusError::DataCorruption(format!(
            "BWT primary index {} out of range (length {})",
            primary_index, n
        )));
    }

    // 1. Histogram of byte occurrences
    let mut counts = [0usize; 256];
    for &b in bwt_data {
        counts[b as usize] += 1;
    }

    // 2. Cumulative frequency table C[c] = index of first occurrence of c in first column F
    let mut c = [0usize; 257];
    let mut sum = 0usize;
    for i in 0..256 {
        c[i] = sum;
        sum += counts[i];
    }
    c[256] = sum;

    // 3. Build LF-mapping: LF[i] = C[bwt[i]] + rank(bwt[i], i)
    let mut lf = vec![0usize; n];
    let mut current_counts = [0usize; 256];

    for (i, &byte) in bwt_data.iter().enumerate() {
        let b = byte as usize;
        lf[i] = c[b] + current_counts[b];
        current_counts[b] += 1;
    }

    // 4. Trace back the original string from primary_index in reverse order
    let mut restored = vec![0u8; n];
    let mut curr = primary_index as usize;

    for step in (0..n).rev() {
        restored[step] = bwt_data[curr];
        curr = lf[curr];
    }

    Ok(restored)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bwt_empty() {
        let input = b"";
        let (encoded, idx) = bwt_encode(input);
        let decoded = bwt_decode(&encoded, idx).unwrap();
        assert_eq!(decoded, input);
    }

    #[test]
    fn test_bwt_single_byte() {
        let input = b"A";
        let (encoded, idx) = bwt_encode(input);
        let decoded = bwt_decode(&encoded, idx).unwrap();
        assert_eq!(decoded, input);
    }

    #[test]
    fn test_bwt_banana() {
        let input = b"banana";
        let (encoded, idx) = bwt_encode(input);
        let decoded = bwt_decode(&encoded, idx).unwrap();
        assert_eq!(decoded, input);
    }

    #[test]
    fn test_bwt_text() {
        let input =
            b"The Burrows-Wheeler Transform is a reversible block sorting algorithm used in bzip2.";
        let (encoded, idx) = bwt_encode(input);
        let decoded = bwt_decode(&encoded, idx).unwrap();
        assert_eq!(decoded, input);
    }

    #[test]
    fn test_bwt_repeating_patterns() {
        let input = b"abcabcabcabcabcabcabcabc";
        let (encoded, idx) = bwt_encode(input);
        let decoded = bwt_decode(&encoded, idx).unwrap();
        assert_eq!(decoded, input);
    }

    #[test]
    fn test_bwt_all_same_bytes() {
        let input = vec![0x7F; 500];
        let (encoded, idx) = bwt_encode(&input);
        let decoded = bwt_decode(&encoded, idx).unwrap();
        assert_eq!(decoded, input);
    }

    #[test]
    fn test_bwt_random_data() {
        let mut rng_state: u64 = 0x5432167890ABCDEF;
        let mut input = vec![0u8; 1024];
        for b in input.iter_mut() {
            rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
            *b = (rng_state >> 33) as u8;
        }

        let (encoded, idx) = bwt_encode(&input);
        let decoded = bwt_decode(&encoded, idx).unwrap();
        assert_eq!(decoded, input);
    }
}
