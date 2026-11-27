//! Block content detection and statistical classification (Layer 1).

use crate::entropy::rans4::histogram;
use crate::format::BlockType;
use std::sync::OnceLock;

/// Computes the Shannon entropy of a byte slice in bits per byte [0.0, 8.0].
pub fn compute_shannon_entropy(data: &[u8]) -> f64 {
    entropy_from_histogram(&histogram(data), data.len())
}

/// Computes Shannon entropy in bits per byte from a byte histogram whose
/// counts sum to `total`.
///
/// Uses the identity `H = log2(N) - sum(c * log2(c)) / N`, so small counts
/// can be served from a lookup table instead of 256 `log2` calls.
pub fn entropy_from_histogram(hist: &[u32; 256], total: usize) -> f64 {
    if total == 0 {
        return 0.0;
    }
    let table = c_log2_c_table();
    let mut sum = 0.0f64;
    for &c in hist {
        let c = c as usize;
        sum += if c < table.len() {
            table[c]
        } else {
            c as f64 * (c as f64).log2()
        };
    }
    let n = total as f64;
    (n.log2() - sum / n).max(0.0)
}

/// `c * log2(c)` for every count that can occur in one splitter window.
fn c_log2_c_table() -> &'static [f64] {
    static TABLE: OnceLock<Vec<f64>> = OnceLock::new();
    TABLE.get_or_init(|| {
        (0..=4096usize)
            .map(|c| {
                if c < 2 {
                    0.0
                } else {
                    c as f64 * (c as f64).log2()
                }
            })
            .collect()
    })
}

/// Checks if data starts with known executable magic signatures (PE, ELF, Mach-O).
fn is_executable_magic(data: &[u8]) -> bool {
    if data.len() >= 2 && &data[0..2] == b"MZ" {
        return true;
    }
    if data.len() >= 4 && &data[0..4] == b"\x7FELF" {
        return true;
    }
    if data.len() >= 4 {
        let m = &data[0..4];
        if m == [0xFE, 0xED, 0xFA, 0xCE]
            || m == [0xFE, 0xED, 0xFA, 0xCF]
            || m == [0xCE, 0xFA, 0xED, 0xFE]
            || m == [0xCF, 0xFA, 0xED, 0xFE]
        {
            return true;
        }
    }
    false
}

/// Checks if data starts with known archive/compressed signatures.
fn is_incompressible_magic(data: &[u8]) -> bool {
    if data.len() >= 4 && &data[0..4] == b"PK\x03\x04" {
        return true;
    }
    if data.len() >= 6 && &data[0..6] == b"7z\xBC\xAF\x27\x1C" {
        return true;
    }
    if data.len() >= 6 && &data[0..6] == b"\xFD7zXZ\x00" {
        return true;
    }
    if data.len() >= 4 && &data[0..4] == b"\x28\xB5\x2F\xFD" {
        return true;
    }
    false
}

/// Checks if data exhibits typical IEEE 754 32-bit floating point exponent patterns.
fn is_floating_point(data: &[u8]) -> bool {
    if data.len() < 32 {
        return false;
    }

    let chunks = data.len() / 4;
    let mut float_count = 0;

    for i in 0..chunks {
        // In little-endian f32, byte 3 contains sign bit and 7 bits of exponent
        let b3 = data[i * 4 + 3];
        let exp = b3 & 0x7F;
        // Typical scientific / normalized floats have exponents roughly between 2^-20 and 2^20
        // corresponding to exp byte between 0x35 and 0x45 (107..137)
        if (0x30..=0x48).contains(&exp) {
            float_count += 1;
        }
    }

    (float_count as f64 / chunks as f64) > 0.75
}

/// Detects the data classification of a sub-block.
pub fn detect_block_type(data: &[u8]) -> BlockType {
    classify_block(data, &histogram(data))
}

/// Classifies a sub-block given its precomputed byte histogram, so callers
/// that already counted the bytes do not scan the data again.
pub fn classify_block(data: &[u8], hist: &[u32; 256]) -> BlockType {
    if data.is_empty() {
        return BlockType::Text;
    }

    // 1. Magic bytes
    if is_executable_magic(data) {
        return BlockType::Executable;
    }
    if is_incompressible_magic(data) {
        return BlockType::Incompressible;
    }
    if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WAVE" {
        return BlockType::Audio;
    }

    // 2. High entropy check
    if entropy_from_histogram(hist, data.len()) >= 7.85 {
        return BlockType::Incompressible;
    }

    // 3. Floating point array detection
    if is_floating_point(data) {
        return BlockType::FloatingPoint;
    }

    // 4. Text vs SourceCode vs Binary, straight from the histogram
    let count = |b: u8| hist[b as usize] as u64;
    let printable: u64 = count(0x09)
        + count(0x0A)
        + count(0x0D)
        + hist[0x20..=0x7E].iter().map(|&c| c as u64).sum::<u64>();

    let printable_ratio = printable as f64 / data.len() as f64;
    if printable_ratio > 0.85 {
        let code_symbols =
            count(b'{') + count(b'}') + count(b';') + count(b'(') + count(b')') + count(b'=');
        if (code_symbols as f64 / data.len() as f64) > 0.03 {
            BlockType::SourceCode
        } else {
            BlockType::Text
        }
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

    #[test]
    fn test_detect_source_code() {
        let code = b"fn main() { let x = 42; println!(\"{}\", x); }";
        assert_eq!(detect_block_type(code), BlockType::SourceCode);
    }

    #[test]
    fn test_detect_executable_pe() {
        let mut pe = vec![0u8; 100];
        pe[0] = b'M';
        pe[1] = b'Z';
        assert_eq!(detect_block_type(&pe), BlockType::Executable);
    }

    #[test]
    fn test_detect_executable_elf() {
        let mut elf = vec![0u8; 100];
        elf[0..4].copy_from_slice(b"\x7FELF");
        assert_eq!(detect_block_type(&elf), BlockType::Executable);
    }

    #[test]
    fn test_detect_floating_point() {
        let mut floats = Vec::new();
        for i in 1..=100 {
            let val = (i as f32) * 0.25;
            floats.extend_from_slice(&val.to_le_bytes());
        }
        assert_eq!(detect_block_type(&floats), BlockType::FloatingPoint);
    }

    #[test]
    fn test_detect_incompressible_archive() {
        let mut zip = vec![0u8; 50];
        zip[0..4].copy_from_slice(b"PK\x03\x04");
        assert_eq!(detect_block_type(&zip), BlockType::Incompressible);
    }
}
