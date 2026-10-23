//! Bijective Delta Encoding (Layer 2).
//!
//! Replaces byte values with the modular difference from previous values
//! at a given stride (e.g. 1 for generic deltas, 2 for 16-bit PCM audio, 4 for 32-bit channels).
//! Strictly bijective: `delta_decode(delta_encode(data, stride), stride) == data`.

/// Encodes input bytes using wrapping modular subtraction at the given stride.
pub fn delta_encode(input: &[u8], stride: usize) -> Vec<u8> {
    let s = if stride == 0 { 1 } else { stride };
    let n = input.len();
    if n == 0 {
        return Vec::new();
    }

    let mut output = Vec::with_capacity(n);

    // Initial prefix shorter than stride is passed unchanged
    let prefix_len = std::cmp::min(s, n);
    output.extend_from_slice(&input[..prefix_len]);

    for i in s..n {
        let diff = input[i].wrapping_sub(input[i - s]);
        output.push(diff);
    }

    output
}

/// Decodes delta-encoded bytes using wrapping modular addition at the given stride.
pub fn delta_decode(input: &[u8], stride: usize) -> Vec<u8> {
    let s = if stride == 0 { 1 } else { stride };
    let n = input.len();
    if n == 0 {
        return Vec::new();
    }

    let mut output = Vec::with_capacity(n);
    let prefix_len = std::cmp::min(s, n);
    output.extend_from_slice(&input[..prefix_len]);

    for i in s..n {
        let original = input[i].wrapping_add(output[i - s]);
        output.push(original);
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_delta_empty() {
        assert_eq!(delta_encode(b"", 1), Vec::<u8>::new());
        assert_eq!(delta_decode(&[], 1), Vec::<u8>::new());
    }

    #[test]
    fn test_delta_shorter_than_stride() {
        let input = b"abc";
        let encoded = delta_encode(input, 4);
        assert_eq!(encoded, input);
        let decoded = delta_decode(&encoded, 4);
        assert_eq!(decoded, input);
    }

    #[test]
    fn test_delta_ramp_stride_1() {
        let input: Vec<u8> = (0..100).collect();
        let encoded = delta_encode(&input, 1);
        // First is 0, rest are 1
        assert_eq!(encoded[0], 0);
        for &b in &encoded[1..] {
            assert_eq!(b, 1);
        }
        let decoded = delta_decode(&encoded, 1);
        assert_eq!(decoded, input);
    }

    #[test]
    fn test_delta_audio_pcm_stride_2() {
        // 16-bit sine wave simulated samples
        let mut input = Vec::new();
        for i in 0..1000 {
            let sample = ((i as f64 * 0.1).sin() * 10000.0) as i16;
            input.extend_from_slice(&sample.to_le_bytes());
        }

        let encoded = delta_encode(&input, 2);
        let decoded = delta_decode(&encoded, 2);
        assert_eq!(decoded, input);
    }

    #[test]
    fn test_delta_random_data() {
        let mut rng: u64 = 0x9988776655443322;
        let mut input = vec![0u8; 8192];
        for b in input.iter_mut() {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            *b = (rng >> 33) as u8;
        }

        for stride in [1, 2, 4, 8] {
            let encoded = delta_encode(&input, stride);
            let decoded = delta_decode(&encoded, stride);
            assert_eq!(decoded, input);
        }
    }
}
