//! NEXUS Command-Line Interface.
//!
//! Provides `compress`, `decompress`, and `list` subcommands with multi-threading,
//! verbose block inspection, and throughput metrics.

use clap::{Parser, Subcommand, ValueEnum};
use nexus_core::pipeline::{
    compress_file_with_progress, decompress_file_with_progress, inspect_archive_file,
    CompressionPreset, ProgressUpdate,
};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process;
use std::sync::Arc;
use std::time::Instant;

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresetArg {
    /// Fast mode: Level 2, ~200+ MB/s, lower CPU usage
    Low,
    /// Balanced mode: Level 6, default general-purpose compression
    Medium,
    /// Maximum mode: Level 9, deep LZ match search & maximum space savings
    High,
}

impl PresetArg {
    pub fn to_preset(self) -> CompressionPreset {
        match self {
            Self::Low => CompressionPreset::Low,
            Self::Medium => CompressionPreset::Medium,
            Self::High => CompressionPreset::High,
        }
    }
}

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

    /// Wait for keypress before exiting (recommended for File Explorer context menu)
    #[arg(short = 'p', long, global = true)]
    pause: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Compresses a file or folder into a NEXUS (.nxar) archive
    Compress {
        /// Path to input file or directory to compress
        input: PathBuf,
        /// Path to output .nxar archive (defaults to <input>.nxar)
        output: Option<PathBuf>,
        /// Compression preset (low, medium, high - overrides numeric level)
        #[arg(long, value_enum)]
        preset: Option<PresetArg>,
        /// Compression level (1-19, default: 6 if preset not specified)
        #[arg(short, long)]
        level: Option<u32>,
    },
    /// Decompresses a .nxar archive (restoring file or directory)
    Decompress {
        /// Path to .nxar or .nxs input archive
        input: PathBuf,
        /// Destination path for restored file or directory (defaults to archive base name)
        output: Option<PathBuf>,
    },
    /// Inspects archive metadata and block table without decompressing
    List {
        /// Path to .nxar or .nxs input archive
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
            preset,
            level,
        } => {
            let start = Instant::now();
            let effective_level = match (preset, level) {
                (Some(p), _) => p.to_preset().to_level(),
                (None, Some(l)) => l,
                (None, None) => CompressionPreset::Medium.to_level(),
            };

            let metadata = match fs::metadata(&input) {
                Ok(meta) => meta,
                Err(err) => {
                    exit_err(format!("Error reading input path '{}': {}", input.display(), err), cli.pause);
                }
            };

            let output = match output {
                Some(p) => p,
                None => {
                    let mut p = input.clone();
                    let name = p
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    p.set_file_name(format!("{}.nxar", name));
                    p
                }
            };

            let original_size = if metadata.is_dir() {
                let (manifest, _) = match nexus_core::vfs::crawl_directory(&input) {
                    Ok(m) => m,
                    Err(err) => {
                        exit_err(format!("Error scanning directory '{}': {}", input.display(), err), cli.pause);
                    }
                };
                eprintln!(
                    "Compressing folder '{}' ({} files, {} dirs, {} bytes, level {})...",
                    input.display(),
                    manifest.file_count(),
                    manifest.directory_count(),
                    manifest.total_uncompressed_bytes(),
                    effective_level
                );
                manifest.total_uncompressed_bytes()
            } else {
                let size = metadata.len();
                eprintln!(
                    "Compressing '{}' ({} bytes, level {})...",
                    input.display(),
                    size,
                    effective_level
                );
                size
            };

            let progress = Arc::new(|update: ProgressUpdate| {
                eprint!(
                    "\r\x1B[2K[+] {:<12} [{:>3.0}%] ({:.1} MB / {:.1} MB)",
                    update.stage,
                    update.fraction * 100.0,
                    update.bytes_processed as f64 / (1024.0 * 1024.0),
                    update.total_bytes as f64 / (1024.0 * 1024.0)
                );
                let _ = std::io::stderr().flush();
            });

            if let Err(err) = compress_file_with_progress(&input, &output, effective_level, Some(progress)) {
                eprintln!();
                exit_err(format!("Compression failed: {}", err), cli.pause);
            }
            eprintln!();

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
                "Done in {:.2?}. Output: '{}', Size: {} bytes ({:.2}% of original), Throughput: {:.2} MB/s",
                elapsed,
                output.display(),
                compressed_size,
                ratio,
                mb_per_sec
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
                    exit_err(format!("Error reading archive '{}': {}", input.display(), err), cli.pause);
                }
            };

            let output = match output {
                Some(p) => p,
                None => {
                    let file_name = input.file_name().unwrap_or_default().to_string_lossy();
                    let stripped = if let Some(s) = file_name.strip_suffix(".nxar") {
                        s
                    } else if let Some(s) = file_name.strip_suffix(".nxs") {
                        s
                    } else {
                        &file_name
                    };
                    input.with_file_name(stripped)
                }
            };

            eprintln!(
                "Decompressing '{}' ({} bytes) -> '{}'...",
                input.display(),
                compressed_size,
                output.display()
            );

            if cli.verbose {
                if let Ok(info) = inspect_archive_file(&input) {
                    print_archive_details(&info, &input);
                }
            }

            let progress = Arc::new(|update: ProgressUpdate| {
                eprint!(
                    "\r\x1B[2K[+] {:<12} [{:>3.0}%] ({:.1} MB / {:.1} MB)",
                    update.stage,
                    update.fraction * 100.0,
                    update.bytes_processed as f64 / (1024.0 * 1024.0),
                    update.total_bytes as f64 / (1024.0 * 1024.0)
                );
                let _ = std::io::stderr().flush();
            });

            if let Err(err) = decompress_file_with_progress(&input, &output, Some(progress)) {
                eprintln!();
                exit_err(format!("Decompression failed: {}", err), cli.pause);
            }
            eprintln!();

            let restored_size = match fs::metadata(&output) {
                Ok(meta) => {
                    if meta.is_dir() {
                        let (m, _) = nexus_core::vfs::crawl_directory(&output).unwrap_or_default();
                        m.total_uncompressed_bytes()
                    } else {
                        meta.len()
                    }
                }
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
                exit_err(format!("Failed to inspect archive '{}': {}", input.display(), err), cli.pause);
            }
        },
    }

    if cli.pause {
        eprintln!("\nPress Enter to exit...");
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
    }
}

fn exit_err(msg: impl std::fmt::Display, pause: bool) -> ! {
    eprintln!("{}", msg);
    if pause {
        eprintln!("\nPress Enter to exit...");
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
    }
    process::exit(1);
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
