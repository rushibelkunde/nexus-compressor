//! Block content detection and entropy calculation (Layer 1).

use crate::format::BlockType;

/// Computes the Shannon entropy of a byte slice in bits per byte [0.0, 8.0].
pub fn compute_shannon_entropy(data: &[u8]) -> f64 {
    if data.is_empty() {
        return 0.0;
    }

    let mut freqs = [0u64; 256];
    for &b in data {
        freqs[b as usize] += 1;
    }

    let total = data.len() as f64;
    let mut entropy = 0.0;
    for &count in &freqs {
        if count > 0 {
            let p = count as f64 / total;
            entropy -= p * p.log2();
        }
    }

    entropy
}

/// Detects the data classification of a sub-block.
pub fn detect_block_type(data: &[u8]) -> BlockType {
    if data.is_empty() {
        return BlockType::Text;
    }

    let entropy = compute_shannon_entropy(data);
    if entropy >= 7.8 {
        return BlockType::Incompressible;
    }

    // Check ASCII printable density
    let printable = data
        .iter()
        .filter(|&&b| b == 0x09 || b == 0x0A || b == 0x0D || (0x20..=0x7E).contains(&b))
        .count();

    let printable_ratio = printable as f64 / data.len() as f64;
    if printable_ratio > 0.85 {
        BlockType::Text
    } else {
        BlockType::Binary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shannon_entropy_identical() {
        let data = vec![0x42; 1024];
        let h = compute_shannon_entropy(&data);
        assert_eq!(h, 0.0);
    }

    #[test]
    fn test_shannon_entropy_uniform() {
        let mut data = Vec::new();
        for _ in 0..10 {
            for b in 0..=255 {
                data.push(b);
            }
        }
        let h = compute_shannon_entropy(&data);
        assert!((h - 8.0).abs() < 1e-6);
    }

    #[test]
    fn test_detect_text() {
        let text = b"The quick brown fox jumps over the lazy dog.\n1234567890";
        assert_eq!(detect_block_type(text), BlockType::Text);
    }
}
