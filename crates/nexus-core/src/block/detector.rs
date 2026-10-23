//! Block content detection and statistical classification (Layer 1).

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

/// Checks if data exhibits source code characteristics (keywords, braces, semicolons).
fn is_source_code(data: &[u8]) -> bool {
    let mut code_symbols = 0;
    for &b in data {
        if b == b'{' || b == b'}' || b == b';' || b == b'(' || b == b')' || b == b'=' {
            code_symbols += 1;
        }
    }
    (code_symbols as f64 / data.len() as f64) > 0.03
}

/// Detects the data classification of a sub-block.
pub fn detect_block_type(data: &[u8]) -> BlockType {
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
    let entropy = compute_shannon_entropy(data);
    if entropy >= 7.85 {
        return BlockType::Incompressible;
    }

    // 3. Floating point array detection
    if is_floating_point(data) {
        return BlockType::FloatingPoint;
    }

    // 4. Text vs SourceCode vs Binary
    let printable = data
        .iter()
        .filter(|&&b| b == 0x09 || b == 0x0A || b == 0x0D || (0x20..=0x7E).contains(&b))
        .count();

    let printable_ratio = printable as f64 / data.len() as f64;
    if printable_ratio > 0.85 {
        if is_source_code(data) {
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
