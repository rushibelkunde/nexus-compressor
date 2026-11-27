//! End-to-end integration roundtrip tests for Project NEXUS.

use nexus_core::error::NexusError;
use nexus_core::pipeline::{compress, decompress, inspect_archive};

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
fn test_integration_large_streaming_stream() {
    let mut input = Vec::with_capacity(4_500_000);
    let chunk_text = b"Adaptive Multi-Strategy Lossless Compression with Project NEXUS. ";
    let chunk_binary = [0xAA, 0x55, 0x00, 0xFF, 0x12, 0x34, 0x56, 0x78];

    while input.len() < 4_500_000 {
        for _ in 0..100 {
            input.extend_from_slice(chunk_text);
        }
        for _ in 0..100 {
            input.extend_from_slice(&chunk_binary);
        }
    }

    let compressed = compress(&input, 1).expect("compression failed");
    let restored = decompress(&compressed).expect("decompression failed");
    assert_eq!(restored, input);

    let info = inspect_archive(&compressed).expect("inspect failed");
    // The stream must span several independent superblocks at this level.
    let expected = input
        .len()
        .div_ceil(nexus_core::compress::lz::superblock_size_for_level(1));
    assert!(expected >= 3, "test input should cover several superblocks");
    assert_eq!(info.superblock_count as usize, expected);
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
    for i in 0..5000i32 {
        exe_data.push(0xE8); // CALL
        let rel = i * 32;
        exe_data.extend_from_slice(&rel.to_le_bytes());
        exe_data.push(0xE9); // JMP
        let rel_jmp = -(i * 8);
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

#[test]
fn test_integration_neural_level_17() {
    let input = b"NEXUS Level 17 Neural Compression test with online integer backpropagation and fixed-point arithmetic coding!";
    let compressed = compress(input, 17).expect("compression failed");
    let restored = decompress(&compressed).expect("decompression failed");
    assert_eq!(restored.as_slice(), input.as_slice());
}

#[test]
fn test_integration_neural_level_18_repetitive() {
    let base = b"Project NEXUS: Neural EXtreme Unified compresSion. ";
    let mut input = Vec::new();
    for _ in 0..50 {
        input.extend_from_slice(base);
    }
    let compressed = compress(&input, 18).expect("compression failed");
    let restored = decompress(&compressed).expect("decompression failed");
    assert_eq!(restored, input);
}

#[test]
fn test_integration_directory_vfs_roundtrip() {
    use nexus_core::pipeline::{compress_file, decompress_file};
    use std::fs::{create_dir_all, read_to_string, remove_dir_all, write};

    let temp_base = std::env::temp_dir().join("nexus_roundtrip_vfs_test");
    let src_dir = temp_base.join("source");
    let extracted_dir = temp_base.join("extracted");
    let archive_path = temp_base.join("test_archive.nxar");

    let _ = remove_dir_all(&temp_base);
    create_dir_all(src_dir.join("code/src")).unwrap();
    create_dir_all(src_dir.join("docs")).unwrap();

    write(
        src_dir.join("code/src/main.rs"),
        b"fn main() { println!(\"NEXUS-VFS\"); }",
    )
    .unwrap();
    write(
        src_dir.join("code/src/lib.rs"),
        b"pub fn add(a: i32, b: i32) -> i32 { a + b }",
    )
    .unwrap();
    write(
        src_dir.join("docs/README.md"),
        b"# NEXUS Solid Directory Archive Test",
    )
    .unwrap();
    write(
        src_dir.join("metadata.json"),
        b"{\"format\":\"nxar\",\"version\":1}",
    )
    .unwrap();

    // 1. Compress directory to .nxar
    compress_file(&src_dir, &archive_path, 5).expect("directory compression failed");
    assert!(archive_path.exists());

    // 2. Decompress .nxar directly into extracted directory
    decompress_file(&archive_path, &extracted_dir).expect("directory decompression failed");

    // 3. Verify all files and contents
    assert_eq!(
        read_to_string(extracted_dir.join("code/src/main.rs")).unwrap(),
        "fn main() { println!(\"NEXUS-VFS\"); }"
    );
    assert_eq!(
        read_to_string(extracted_dir.join("code/src/lib.rs")).unwrap(),
        "pub fn add(a: i32, b: i32) -> i32 { a + b }"
    );
    assert_eq!(
        read_to_string(extracted_dir.join("docs/README.md")).unwrap(),
        "# NEXUS Solid Directory Archive Test"
    );
    assert_eq!(
        read_to_string(extracted_dir.join("metadata.json")).unwrap(),
        "{\"format\":\"nxar\",\"version\":1}"
    );

    let _ = remove_dir_all(&temp_base);
}

#[test]
fn test_integration_verify_and_cancel_file_api() {
    use nexus_core::pipeline::{
        compress_file_with_control, decompress_file, verify_file_with_control, JobControl,
    };
    use nexus_core::NexusError;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    let dir = std::env::temp_dir().join(format!("nexus-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input_path = dir.join("input.bin");
    let archive_path = dir.join("input.nxar");
    let restored_path = dir.join("restored.bin");

    let mut input = Vec::new();
    let mut state: u64 = 42;
    while input.len() < 5_000_000 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let word = format!(
            "record-{} value-{}\n",
            (state >> 40) % 5000,
            (state >> 20) % 97
        );
        input.extend_from_slice(word.as_bytes());
    }
    std::fs::write(&input_path, &input).unwrap();

    // Normal run, then verify without extracting, then extract and compare.
    compress_file_with_control(&input_path, &archive_path, 3, &JobControl::default()).unwrap();
    let verified = verify_file_with_control(&archive_path, &JobControl::default()).unwrap();
    assert_eq!(verified, input.len() as u64);
    decompress_file(&archive_path, &restored_path).unwrap();
    assert_eq!(std::fs::read(&restored_path).unwrap(), input);

    // Verification must notice a flipped payload byte.
    let mut damaged = std::fs::read(&archive_path).unwrap();
    let mid = damaged.len() / 2;
    damaged[mid] ^= 0x10;
    let damaged_path = dir.join("damaged.nxar");
    std::fs::write(&damaged_path, &damaged).unwrap();
    assert!(verify_file_with_control(&damaged_path, &JobControl::default()).is_err());

    // A pre-set cancel flag aborts and leaves no partial archive behind.
    let cancelled_path = dir.join("cancelled.nxar");
    let control = JobControl {
        progress: None,
        cancel: Some(Arc::new(AtomicBool::new(true))),
    };
    let result = compress_file_with_control(&input_path, &cancelled_path, 3, &control);
    assert!(matches!(result, Err(NexusError::Cancelled)));
    assert!(!cancelled_path.exists());

    // Cancelling an overwrite must leave the existing archive untouched and
    // no temporary file behind.
    let before = std::fs::read(&archive_path).unwrap();
    let result = compress_file_with_control(&input_path, &archive_path, 3, &control);
    assert!(matches!(result, Err(NexusError::Cancelled)));
    assert_eq!(std::fs::read(&archive_path).unwrap(), before);
    assert!(!dir.join("input.nxar.part").exists());

    // Extracting a corrupt archive must not leave a partial output file, and
    // an invalid archive must not touch a file already at the destination.
    let partial_out = dir.join("partial.bin");
    assert!(decompress_file(&damaged_path, &partial_out).is_err());
    assert!(!partial_out.exists());
    let precious = dir.join("precious.bin");
    std::fs::write(&precious, b"keep me").unwrap();
    let not_an_archive = dir.join("garbage.nxar");
    std::fs::write(&not_an_archive, vec![0x55u8; 4096]).unwrap();
    assert!(decompress_file(&not_an_archive, &precious).is_err());
    assert_eq!(std::fs::read(&precious).unwrap(), b"keep me");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_integration_level_13_uses_bwt_for_text_and_is_smaller() {
    use nexus_core::format::CompressionPath;

    let words: [&[u8]; 12] = [
        b"compression ",
        b"entropy ",
        b"window ",
        b"the ",
        b"of ",
        b"and ",
        b"match ",
        b"literal ",
        b"suffix ",
        b"array ",
        b"model ",
        b"context.\n",
    ];
    let mut state: u64 = 7;
    let mut input = Vec::new();
    while input.len() < 1_500_000 {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        input.extend_from_slice(words[((state >> 33) % 12) as usize]);
    }

    let lz_only = compress(&input, 12).expect("level 12 failed");
    let with_bwt = compress(&input, 13).expect("level 13 failed");
    assert_eq!(decompress(&with_bwt).expect("decode failed"), input);
    assert!(
        with_bwt.len() < lz_only.len(),
        "level 13 ({}) should beat level 12 ({}) on text",
        with_bwt.len(),
        lz_only.len()
    );

    let info = inspect_archive(&with_bwt).expect("inspect failed");
    assert!(info
        .blocks
        .iter()
        .any(|b| b.compression_path == CompressionPath::PathB2));
}
