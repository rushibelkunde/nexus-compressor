//! End-to-end integration roundtrip tests for Project NEXUS.

use nexus_core::error::NexusError;
use nexus_core::pipeline::{compress, decompress};

#[test]
fn test_integration_empty() {
    let input = b"";
    let compressed = compress(input, 1).expect("compression failed");
    let restored = decompress(&compressed).expect("decompression failed");
    assert_eq!(restored.as_slice(), input.as_slice());
}

#[test]
fn test_integration_single_byte() {
    let input = b"Z";
    let compressed = compress(input, 1).expect("compression failed");
    let restored = decompress(&compressed).expect("decompression failed");
    assert_eq!(restored.as_slice(), input.as_slice());
}

#[test]
fn test_integration_all_same() {
    let input = vec![0x55u8; 100_000];
    let compressed = compress(&input, 1).expect("compression failed");
    assert!(
        compressed.len() < input.len() / 10,
        "Repetitive data should compress well"
    );
    let restored = decompress(&compressed).expect("decompression failed");
    assert_eq!(restored, input);
}

#[test]
fn test_integration_repetitive_text() {
    let base = b"NEXUS is an extreme multi-strategy compression engine. ";
    let mut input = Vec::new();
    for _ in 0..1000 {
        input.extend_from_slice(base);
    }
    let compressed = compress(&input, 1).expect("compression failed");
    assert!(compressed.len() < input.len());
    let restored = decompress(&compressed).expect("decompression failed");
    assert_eq!(restored, input);
}

#[test]
fn test_integration_binary_random() {
    let mut rng_state: u64 = 0xCAFEBABE12345678;
    let mut input = vec![0u8; 131_072]; // 128 KB
    for b in input.iter_mut() {
        rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
        *b = (rng_state >> 33) as u8;
    }

    let compressed = compress(&input, 1).expect("compression failed");
    let restored = decompress(&compressed).expect("decompression failed");
    assert_eq!(restored, input);
}

#[test]
fn test_integration_multi_superblock() {
    // Superblock is 1MB. 2.5MB ensures 3 superblocks are exercised.
    let pattern = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!@#$%^&*()_+-=[]{}|;':,./<>?";
    let mut input = Vec::with_capacity(2_500_000);
    while input.len() < 2_500_000 {
        input.extend_from_slice(pattern);
    }

    let compressed = compress(&input, 1).expect("compression failed");
    let restored = decompress(&compressed).expect("decompression failed");
    assert_eq!(restored, input);
}

#[test]
fn test_integration_corrupted_magic() {
    let input = b"Hello, world!";
    let mut compressed = compress(input, 1).expect("compression failed");
    compressed[0] = 0x00; // Corrupt magic
    let result = decompress(&compressed);
    assert!(
        matches!(
            result,
            Err(NexusError::InvalidMagic(_)) | Err(NexusError::ChecksumMismatch { .. })
        ),
        "Corrupted magic must be caught by header check or file checksum"
    );
}

#[test]
fn test_integration_corrupted_payload_checksum() {
    let input = vec![0xAB; 2048];
    let mut compressed = compress(&input, 1).expect("compression failed");
    let mid = compressed.len() / 2;
    compressed[mid] ^= 0xFF;
    let result = decompress(&compressed);
    assert!(
        result.is_err(),
        "Corrupted payload must be rejected by checksum or decode"
    );
}

#[test]
fn test_integration_corrupted_end_marker() {
    let input = b"Integrity check test";
    let mut compressed = compress(input, 1).expect("compression failed");
    let last = compressed.len() - 1;
    compressed[last] ^= 0xFF; // Corrupt NXSE marker
    let result = decompress(&compressed);
    assert!(matches!(result, Err(NexusError::InvalidEndMarker(_))));
}
