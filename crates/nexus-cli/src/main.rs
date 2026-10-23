//! NEXUS Command-Line Interface.
//!
//! Provides `compress` and `decompress` subcommands for files.

use clap::{Parser, Subcommand};
use std::fs;
use std::path::PathBuf;
use std::process;
use std::time::Instant;

#[derive(Parser)]
#[command(name = "nexus")]
#[command(about = "NEXUS: Neural EXtreme Unified compresSion", long_about = None)]
#[command(version)]
struct Cli {
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
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Compress {
            input,
            output,
            level,
        } => {
            let start = Instant::now();
            let raw_data = match fs::read(&input) {
                Ok(bytes) => bytes,
                Err(err) => {
                    eprintln!("Error reading input file '{}': {}", input.display(), err);
                    process::exit(1);
                }
            };

            let original_size = raw_data.len();
            eprintln!(
                "Compressing '{}' ({} bytes, level {})...",
                input.display(),
                original_size,
                level
            );

            let compressed = match nexus_core::pipeline::compress(&raw_data, level) {
                Ok(bytes) => bytes,
                Err(err) => {
                    eprintln!("Compression failed: {}", err);
                    process::exit(1);
                }
            };

            let compressed_size = compressed.len();
            if let Err(err) = fs::write(&output, &compressed) {
                eprintln!("Error writing output file '{}': {}", output.display(), err);
                process::exit(1);
            }

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
        }
        Commands::Decompress { input, output } => {
            let start = Instant::now();
            let compressed = match fs::read(&input) {
                Ok(bytes) => bytes,
                Err(err) => {
                    eprintln!("Error reading input file '{}': {}", input.display(), err);
                    process::exit(1);
                }
            };

            eprintln!(
                "Decompressing '{}' ({} bytes)...",
                input.display(),
                compressed.len()
            );

            let restored = match nexus_core::pipeline::decompress(&compressed) {
                Ok(bytes) => bytes,
                Err(err) => {
                    eprintln!("Decompression failed: {}", err);
                    process::exit(1);
                }
            };

            let restored_size = restored.len();
            if let Err(err) = fs::write(&output, &restored) {
                eprintln!("Error writing output file '{}': {}", output.display(), err);
                process::exit(1);
            }

            let elapsed = start.elapsed();
            let mb_per_sec =
                (restored_size as f64 / (1024.0 * 1024.0)) / elapsed.as_secs_f64().max(0.000001);

            eprintln!(
                "Done in {:.2?}. Restored: {} bytes, Throughput: {:.2} MB/s",
                elapsed, restored_size, mb_per_sec
            );
        }
    }
}
