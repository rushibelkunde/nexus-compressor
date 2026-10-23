//! Comprehensive cross-corpus benchmark suite for Project NEXUS.
//!
//! Evaluates compression ratio, bits-per-character (bpc), compression throughput,
//! decompression throughput, and bit-exact lossless roundtrip verification
//! across heterogeneous data types and compression paths.

use nexus_core::pipeline::{compress, decompress, inspect_archive};
use std::time::Instant;

struct BenchmarkDataset {
    name: &'static str,
    data: Vec<u8>,
}

struct BenchmarkResult {
    dataset_name: &'static str,
    level: u32,
    path_name: String,
    orig_bytes: usize,
    comp_bytes: usize,
    ratio_pct: f64,
    bpc: f64,
    comp_speed_mbs: f64,
    decomp_speed_mbs: f64,
    verified: bool,
}

fn generate_natural_text(size: usize) -> Vec<u8> {
    let prose =
        b"Project NEXUS (Neural EXtreme Unified compresSion) is a state-of-the-art lossless \
compression engine combining LZ77 dictionary matching, Burrows-Wheeler Transform (BWT), \
Move-To-Front (MTF) clustering, Run-Length Encoding (RLE), asymmetric numeral systems (rANS), \
bijective branch filters, multi-context mixing, and online fixed-point neural prediction. \
Every sub-block is adaptively analyzed and routed to the mathematically optimal pipeline. ";

    let mut out = Vec::with_capacity(size);
    while out.len() < size {
        out.extend_from_slice(prose);
    }
    out.truncate(size);
    out
}

fn generate_executable_code(size: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(size);
    // Simulated PE header
    out.extend_from_slice(b"MZ\x90\x00\x03\x00\x00\x00\x04\x00\x00\x00\xFF\xFF\x00\x00");
    out.extend_from_slice(b"PE\x00\x00\x64\x86\x03\x00\x00\x00\x00\x00\x00\x00\x00\x00");

    let mut i = 0usize;
    while out.len() < size {
        // Function prologue: push rbp; mov rbp, rsp
        out.extend_from_slice(&[0x55, 0x48, 0x89, 0xE5]);
        // CALL with relative displacement
        out.push(0xE8);
        let rel_call = ((i * 32) as i32).to_le_bytes();
        out.extend_from_slice(&rel_call);
        // JMP with relative displacement
        out.push(0xE9);
        let rel_jmp = (-(i as i32 * 16)).to_le_bytes();
        out.extend_from_slice(&rel_jmp);
        // Function epilogue: pop rbp; ret
        out.extend_from_slice(&[0x5D, 0xC3]);
        i += 1;
    }
    out.truncate(size);
    out
}

fn generate_floating_point(size: usize) -> Vec<u8> {
    let num_floats = size / 4;
    let mut out = Vec::with_capacity(size);
    for i in 0..num_floats {
        let angle = (i as f32) * 0.025;
        let val = angle.sin() * 100.0 + (angle * 0.5).cos() * 25.0;
        out.extend_from_slice(&val.to_le_bytes());
    }
    out.truncate(size);
    out
}

fn generate_audio_pcm(size: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(size);
    // RIFF WAVE header
    out.extend_from_slice(b"RIFF\x24\x08\x00\x00WAVEfmt \x10\x00\x00\x00\x01\x00\x01\x00");
    let num_samples = (size - out.len()) / 2;
    for i in 0..num_samples {
        let sample = (((i as f64 * 0.05).sin() + (i as f64 * 0.02).cos()) * 8000.0) as i16;
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out.truncate(size);
    out
}

fn generate_structured_json(size: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(size);
    let mut id = 1000;
    while out.len() < size {
        let record = format!(
            r#"{{"id":{},"status":"active","timestamp":1700000000,"metrics":{{"lat":37.77,"lon":-122.41,"cpu":42.5}},"tags":["nexus","rust","ai"]}}"#,
            id
        );
        out.extend_from_slice(record.as_bytes());
        out.push(b'\n');
        id += 1;
    }
    out.truncate(size);
    out
}

fn generate_random_bytes(size: usize) -> Vec<u8> {
    let mut out = vec![0u8; size];
    let mut rng: u64 = 0xDEADBEEFCAFE1234;
    for b in out.iter_mut() {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
        *b = (rng >> 33) as u8;
    }
    out
}

fn run_single_benchmark(dataset: &BenchmarkDataset, level: u32) -> BenchmarkResult {
    let orig_bytes = dataset.data.len();

    // Compression
    let t_comp_start = Instant::now();
    let compressed = compress(&dataset.data, level).expect("compression failed");
    let comp_duration = t_comp_start.elapsed();

    let comp_bytes = compressed.len();

    // Extract path name from archive
    let path_name = if let Ok(info) = inspect_archive(&compressed) {
        if let Some(first_block) = info.blocks.first() {
            format!("{:?}", first_block.compression_path)
        } else {
            "Unknown".into()
        }
    } else {
        "Unknown".into()
    };

    // Decompression
    let t_decomp_start = Instant::now();
    let restored = decompress(&compressed).expect("decompression failed");
    let decomp_duration = t_decomp_start.elapsed();

    // Verify lossless roundtrip
    let verified = restored == dataset.data;

    let ratio_pct = if orig_bytes > 0 {
        (comp_bytes as f64 / orig_bytes as f64) * 100.0
    } else {
        100.0
    };

    let bpc = if orig_bytes > 0 {
        (comp_bytes as f64 * 8.0) / orig_bytes as f64
    } else {
        8.0
    };

    let comp_sec = comp_duration.as_secs_f64().max(0.000001);
    let decomp_sec = decomp_duration.as_secs_f64().max(0.000001);

    let comp_speed_mbs = (orig_bytes as f64 / (1024.0 * 1024.0)) / comp_sec;
    let decomp_speed_mbs = (orig_bytes as f64 / (1024.0 * 1024.0)) / decomp_sec;

    BenchmarkResult {
        dataset_name: dataset.name,
        level,
        path_name,
        orig_bytes,
        comp_bytes,
        ratio_pct,
        bpc,
        comp_speed_mbs,
        decomp_speed_mbs,
        verified,
    }
}

fn main() {
    println!("=========================================================================================================");
    println!("                                PROJECT NEXUS COMPRESSION BENCHMARK SUITE                                ");
    println!("=========================================================================================================");

    let bench_size = 32_768; // 32 KB per dataset for thorough multi-algorithm benchmarking

    let datasets = vec![
        BenchmarkDataset {
            name: "Text (Natural)",
            data: generate_natural_text(bench_size),
        },
        BenchmarkDataset {
            name: "Executable (x86)",
            data: generate_executable_code(bench_size),
        },
        BenchmarkDataset {
            name: "Scientific Floats",
            data: generate_floating_point(bench_size),
        },
        BenchmarkDataset {
            name: "Audio (PCM Wave)",
            data: generate_audio_pcm(bench_size),
        },
        BenchmarkDataset {
            name: "Structured (JSON)",
            data: generate_structured_json(bench_size),
        },
        BenchmarkDataset {
            name: "Random Stream",
            data: generate_random_bytes(bench_size),
        },
    ];

    let levels = [1, 5, 7, 13, 17];

    println!(
        "{:<18} {:<5} {:<8} {:>10} {:>10} {:>9} {:>7} {:>13} {:>13} {:>8}",
        "Dataset",
        "Level",
        "Path",
        "OrigBytes",
        "CompBytes",
        "Ratio %",
        "BPC",
        "Comp (MB/s)",
        "Decomp (MB/s)",
        "Status"
    );
    println!("---------------------------------------------------------------------------------------------------------");

    let mut total_orig = 0usize;
    let mut total_comp = 0usize;
    let mut all_passed = true;

    for dataset in &datasets {
        for &lvl in &levels {
            let res = run_single_benchmark(dataset, lvl);

            total_orig += res.orig_bytes;
            total_comp += res.comp_bytes;
            if !res.verified {
                all_passed = false;
            }

            let status_str = if res.verified { "PASS" } else { "FAIL" };

            println!(
                "{:<18} {:<5} {:<8} {:>10} {:>10} {:>8.2}% {:>7.2} {:>13.2} {:>13.2} {:>8}",
                res.dataset_name,
                res.level,
                res.path_name,
                res.orig_bytes,
                res.comp_bytes,
                res.ratio_pct,
                res.bpc,
                res.comp_speed_mbs,
                res.decomp_speed_mbs,
                status_str
            );
        }
        println!("- - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - -");
    }

    println!("=========================================================================================================");
    let overall_ratio = if total_orig > 0 {
        (total_comp as f64 / total_orig as f64) * 100.0
    } else {
        100.0
    };
    println!(
        "Overall Summary: {} total bytes -> {} compressed bytes ({:.2}% average ratio)",
        total_orig, total_comp, overall_ratio
    );
    println!(
        "Lossless Roundtrip Integrity: {}",
        if all_passed {
            "100% VERIFIED (ALL BIT-EXACT MATCHES)"
        } else {
            "FAILED (MISMATCH DETECTED)"
        }
    );
    println!("=========================================================================================================");
}
