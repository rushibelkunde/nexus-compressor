//! NEXUS Command-Line Interface.
//!
//! Provides `compress`, `decompress`, and `list` subcommands with multi-threading,
//! verbose block inspection, and throughput metrics.

use clap::{Parser, Subcommand};
use nexus_core::pipeline::{compress_file, decompress_file, inspect_archive_file};
use std::fs;
use std::path::PathBuf;
use std::process;
use std::time::Instant;

#[derive(Parser)]
#[command(name = "nexus")]
#[command(about = "NEXUS: Neural EXtreme Unified compresSion", long_about = None)]
#[command(version)]
struct Cli {
    /// Number of worker threads (default: logical CPU count)
    #[arg(short = 't', long, global = true)]
    threads: Option<usize>,

    /// Enable verbose per-block telemetry and detailed statistics
    #[arg(short = 'v', long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Compresses a file using NEXUS
    Compress {
        /// Path to input file to compress
        input: PathBuf,
        /// Path to output .nxs file
        output: PathBuf,
        /// Compression level (1-19, default: 1)
        #[arg(short, long, default_value_t = 1)]
        level: u32,
    },
    /// Decompresses a .nxs file
    Decompress {
        /// Path to .nxs input file
        input: PathBuf,
        /// Path to write decompressed output
        output: PathBuf,
    },
    /// Inspects archive metadata and block table without decompressing
    List {
        /// Path to .nxs input file
        input: PathBuf,
    },
}

fn main() {
    let cli = Cli::parse();

    // Configure Rayon global thread pool if specified
    if let Some(num_threads) = cli.threads {
        if let Err(err) = rayon::ThreadPoolBuilder::new()
            .num_threads(num_threads)
            .build_global()
        {
            eprintln!(
                "Warning: Failed to set thread count to {}: {}",
                num_threads, err
            );
        }
    }

    match cli.command {
        Commands::Compress {
            input,
            output,
            level,
        } => {
            let start = Instant::now();
            let metadata = match fs::metadata(&input) {
                Ok(meta) => meta,
                Err(err) => {
                    eprintln!("Error reading input file '{}': {}", input.display(), err);
                    process::exit(1);
                }
            };

            let original_size = metadata.len();
            eprintln!(
                "Compressing '{}' ({} bytes, level {})...",
                input.display(),
                original_size,
                level
            );

            if let Err(err) = compress_file(&input, &output, level) {
                eprintln!("Compression failed: {}", err);
                process::exit(1);
            }

            let compressed_size = match fs::metadata(&output) {
                Ok(meta) => meta.len(),
                Err(_) => 0,
            };

            let elapsed = start.elapsed();
            let ratio = if original_size > 0 {
                (compressed_size as f64 / original_size as f64) * 100.0
            } else {
                100.0
            };
            let mb_per_sec =
                (original_size as f64 / (1024.0 * 1024.0)) / elapsed.as_secs_f64().max(0.000001);

            eprintln!(
                "Done in {:.2?}. Compressed: {} bytes ({:.2}% of original), Throughput: {:.2} MB/s",
                elapsed, compressed_size, ratio, mb_per_sec
            );

            if cli.verbose {
                if let Ok(info) = inspect_archive_file(&output) {
                    print_archive_details(&info, &output);
                }
            }
        }
        Commands::Decompress { input, output } => {
            let start = Instant::now();
            let compressed_size = match fs::metadata(&input) {
                Ok(meta) => meta.len(),
                Err(err) => {
                    eprintln!("Error reading input file '{}': {}", input.display(), err);
                    process::exit(1);
                }
            };

            eprintln!(
                "Decompressing '{}' ({} bytes)...",
                input.display(),
                compressed_size
            );

            if cli.verbose {
                if let Ok(info) = inspect_archive_file(&input) {
                    print_archive_details(&info, &input);
                }
            }

            if let Err(err) = decompress_file(&input, &output) {
                eprintln!("Decompression failed: {}", err);
                process::exit(1);
            }

            let restored_size = match fs::metadata(&output) {
                Ok(meta) => meta.len(),
                Err(_) => 0,
            };

            let elapsed = start.elapsed();
            let mb_per_sec =
                (restored_size as f64 / (1024.0 * 1024.0)) / elapsed.as_secs_f64().max(0.000001);

            eprintln!(
                "Done in {:.2?}. Restored: {} bytes, Throughput: {:.2} MB/s",
                elapsed, restored_size, mb_per_sec
            );
        }
        Commands::List { input } => match inspect_archive_file(&input) {
            Ok(info) => print_archive_details(&info, &input),
            Err(err) => {
                eprintln!("Failed to inspect archive '{}': {}", input.display(), err);
                process::exit(1);
            }
        },
    }
}

fn print_archive_details(info: &nexus_core::pipeline::ArchiveInfo, path: &std::path::Path) {
    println!("================================================================================");
    println!("NEXUS Archive Information: {}", path.display());
    println!("Format Version    : {}", info.format_version);
    println!("Superblock Count  : {}", info.superblock_count);
    println!("Total Blocks      : {}", info.total_blocks);
    println!(
        "Original Size     : {} bytes ({:.2} KB)",
        info.total_original_size,
        info.total_original_size as f64 / 1024.0
    );
    println!(
        "Compressed Size   : {} bytes ({:.2} KB)",
        info.total_compressed_size,
        info.total_compressed_size as f64 / 1024.0
    );
    let overall_ratio = if info.total_original_size > 0 {
        (info.total_compressed_size as f64 / info.total_original_size as f64) * 100.0
    } else {
        100.0
    };
    println!(
        "Ratio             : {:.2}% of original (saving {:.2}%)",
        overall_ratio,
        100.0 - overall_ratio
    );
    println!(
        "File Checksum     : 0x{:016X} (XXHash64)",
        info.file_checksum
    );
    println!("--------------------------------------------------------------------------------");
    println!(
        "{:<6} {:<6} {:<14} {:<8} {:<10} {:>10} {:>10} {:>8}",
        "Block", "SB_ID", "Type", "Path", "Flags", "OrigBytes", "CompBytes", "Ratio"
    );
    println!("--------------------------------------------------------------------------------");
    for (i, b) in info.blocks.iter().enumerate() {
        let ratio = if b.original_size > 0 {
            (b.compressed_size as f64 / b.original_size as f64) * 100.0
        } else {
            100.0
        };
        println!(
            "{:<6} {:<6} {:<14?} {:<8?} {:<10?} {:>10} {:>10} {:>7.2}%",
            i,
            b.superblock_id,
            b.block_type,
            b.compression_path,
            b.preprocess_flags,
            b.original_size,
            b.compressed_size,
            ratio
        );
    }
    println!("================================================================================");
}
