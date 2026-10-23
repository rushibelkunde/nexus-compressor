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

#[test]
fn test_integration_executable_binary() {
    let mut exe_data = Vec::new();
    exe_data.extend_from_slice(b"MZ\x90\x00\x03\x00\x00\x00\x04\x00\x00\x00\xFF\xFF\x00\x00");
    for i in 0..5000 {
        exe_data.push(0xE8); // CALL
        let rel = (i * 32) as i32;
        exe_data.extend_from_slice(&rel.to_le_bytes());
        exe_data.push(0xE9); // JMP
        let rel_jmp = -(i as i32 * 8);
        exe_data.extend_from_slice(&rel_jmp.to_le_bytes());
        exe_data.extend_from_slice(&[0x90, 0x55, 0x89, 0xE5]);
    }

    let compressed = compress(&exe_data, 7).expect("compression failed");
    let restored = decompress(&compressed).expect("decompression failed");
    assert_eq!(restored, exe_data);
}

#[test]
fn test_integration_float_stream() {
    let mut float_data = Vec::new();
    for i in 0..10_000 {
        let f = (i as f32) * 0.125 + 1.0;
        float_data.extend_from_slice(&f.to_le_bytes());
    }

    let compressed = compress(&float_data, 7).expect("compression failed");
    let restored = decompress(&compressed).expect("decompression failed");
    assert_eq!(restored, float_data);
}

#[test]
fn test_integration_audio_pcm() {
    let mut wav_data = Vec::new();
    wav_data.extend_from_slice(b"RIFF\x24\x08\x00\x00WAVEfmt \x10\x00\x00\x00\x01\x00\x01\x00");
    for i in 0..10_000 {
        let sample = ((i as f64 * 0.05).sin() * 8000.0) as i16;
        wav_data.extend_from_slice(&sample.to_le_bytes());
    }

    let compressed = compress(&wav_data, 7).expect("compression failed");
    let restored = decompress(&compressed).expect("decompression failed");
    assert_eq!(restored, wav_data);
}

#[test]
fn test_integration_dynamic_subblocks_mixed() {
    let mut composite = Vec::new();
    for _ in 0..10 {
        composite.extend_from_slice(
            b"Natural language text block demonstrating BWT and MTF text compression capabilities.\n"
                .repeat(200)
                .as_slice(),
        );
        for i in 0..1000 {
            let f = (i as f32) * 2.5;
            composite.extend_from_slice(&f.to_le_bytes());
        }
    }

    let compressed = compress(&composite, 7).expect("compression failed");
    let restored = decompress(&compressed).expect("decompression failed");
    assert_eq!(restored, composite);
}

#[test]
fn test_integration_context_mixing_level_13() {
    let input = b"NEXUS Level 13 Context Mixing test with neural probability blending and arithmetic range coder!";
    let compressed = compress(input, 13).expect("compression failed");
    let restored = decompress(&compressed).expect("decompression failed");
    assert_eq!(restored.as_slice(), input.as_slice());
}
