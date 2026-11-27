//! Corpus benchmark and regression gate for Project NEXUS.
//!
//! Measures every requested compression level over a directory of real files
//! (e.g. the Silesia corpus): compressed size, compression and decompression
//! throughput, and bit-exact roundtrip. Optionally runs reference compressors
//! found on the machine for comparison, and compares the results against a
//! saved baseline so that size regressions fail the run.
//!
//! ```text
//! nexus-bench --corpus bench-corpus/silesia --levels 1,3,6,9,11 --refs
//! nexus-bench --corpus bench-corpus/silesia --save bench-baseline.tsv
//! nexus-bench --corpus bench-corpus/silesia --check bench-baseline.tsv
//! ```
//!
//! NEXUS is timed in-process on in-memory buffers. Reference tools are timed
//! as child processes writing to a null device, so their figures include
//! process start-up and file reading; `zstd` is the exception, measured with
//! its own in-memory benchmark mode.

use nexus_core::pipeline::{compress, decompress};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const MB: f64 = 1024.0 * 1024.0;

struct Options {
    corpus: PathBuf,
    levels: Vec<u32>,
    threads: usize,
    refs: bool,
    per_file: bool,
    save: Option<PathBuf>,
    check: Option<PathBuf>,
    /// Only benchmark files whose name contains this text.
    filter: Option<String>,
    /// Print a per-stage timing breakdown instead of the benchmark table.
    profile: bool,
}

fn parse_args() -> Result<Options, String> {
    let mut opts = Options {
        corpus: PathBuf::from("bench-corpus/silesia"),
        levels: vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
        threads: 0,
        refs: false,
        per_file: false,
        save: None,
        check: None,
        filter: None,
        profile: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or(format!("{name} needs a value"));
        match arg.as_str() {
            "--corpus" => opts.corpus = PathBuf::from(value("--corpus")?),
            "--levels" => {
                opts.levels = value("--levels")?
                    .split(',')
                    .map(|s| s.trim().parse::<u32>().map_err(|e| e.to_string()))
                    .collect::<Result<_, _>>()?;
            }
            "--threads" => {
                opts.threads = value("--threads")?.parse().map_err(|_| "bad --threads")?;
            }
            "--refs" => opts.refs = true,
            "--per-file" => opts.per_file = true,
            "--save" => opts.save = Some(PathBuf::from(value("--save")?)),
            "--check" => opts.check = Some(PathBuf::from(value("--check")?)),
            "--filter" => opts.filter = Some(value("--filter")?),
            "--profile" => opts.profile = true,
            "-h" | "--help" => {
                println!(
                    "nexus-bench [--corpus DIR] [--levels 1,6,11] [--threads N] [--filter TEXT]\n\
                     \x20           [--refs] [--per-file] [--save FILE] [--check FILE]"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(opts)
}

struct CorpusFile {
    name: String,
    path: PathBuf,
    data: Vec<u8>,
}

fn load_corpus(dir: &Path, filter: Option<&str>) -> Result<Vec<CorpusFile>, String> {
    let mut files = Vec::new();
    let entries = std::fs::read_dir(dir).map_err(|e| {
        format!(
            "cannot read corpus directory {}: {e}\n\
             Download a corpus first (see scripts/bench/README.md).",
            dir.display()
        )
    })?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if filter.is_some_and(|f| !name.contains(f)) {
            continue;
        }
        let data = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        files.push(CorpusFile { name, path, data });
    }
    files.sort_by(|a, b| a.name.cmp(&b.name));
    if files.is_empty() {
        return Err(format!("no files found in {}", dir.display()));
    }
    Ok(files)
}

/// Totals for one compressor configuration across the corpus.
#[derive(Default, Clone)]
struct Totals {
    original: u64,
    compressed: u64,
    comp_time: Duration,
    decomp_time: Duration,
    /// False when decompression speed could not be measured.
    has_decomp: bool,
    /// `(name, compressed bytes, compress MB/s, decompress MB/s)`.
    per_file: Vec<(String, u64, f64, f64)>,
}

impl Totals {
    fn ratio_pct(&self) -> f64 {
        self.compressed as f64 * 100.0 / self.original.max(1) as f64
    }
    fn comp_mbs(&self) -> f64 {
        self.original as f64 / MB / self.comp_time.as_secs_f64().max(1e-9)
    }
    fn decomp_mbs(&self) -> f64 {
        self.original as f64 / MB / self.decomp_time.as_secs_f64().max(1e-9)
    }
}

/// Runs `f` repeatedly (up to `max_runs`, stopping once `budget` is spent)
/// and returns the fastest time together with the last result.
fn best_of<T>(max_runs: u32, budget: Duration, mut f: impl FnMut() -> T) -> (Duration, T) {
    let started = Instant::now();
    let t = Instant::now();
    let mut result = f();
    let mut best = t.elapsed();
    for _ in 1..max_runs {
        if started.elapsed() >= budget {
            break;
        }
        let t = Instant::now();
        result = f();
        best = best.min(t.elapsed());
    }
    (best, result)
}

fn bench_nexus(files: &[CorpusFile], level: u32) -> Result<Totals, String> {
    let mut totals = Totals {
        has_decomp: true,
        ..Totals::default()
    };
    for file in files {
        let (ct, compressed) = best_of(3, Duration::from_millis(1500), || {
            compress(&file.data, level)
        });
        let compressed = compressed.map_err(|e| format!("{} L{level}: {e}", file.name))?;
        let (dt, restored) = best_of(5, Duration::from_millis(1000), || decompress(&compressed));
        let restored = restored.map_err(|e| format!("{} L{level} decode: {e}", file.name))?;
        if restored != file.data {
            return Err(format!(
                "ROUNDTRIP MISMATCH: {} at level {level}",
                file.name
            ));
        }
        totals.original += file.data.len() as u64;
        totals.compressed += compressed.len() as u64;
        totals.comp_time += ct;
        totals.decomp_time += dt;
        let mb = file.data.len() as f64 / MB;
        totals.per_file.push((
            file.name.clone(),
            compressed.len() as u64,
            mb / ct.as_secs_f64().max(1e-9),
            mb / dt.as_secs_f64().max(1e-9),
        ));
    }
    Ok(totals)
}

/// Locates an executable by name on PATH or under `bench-corpus/tools`.
fn find_tool(name: &str) -> Option<PathBuf> {
    let exe = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    fn search(dir: &Path, exe: &str, depth: u32) -> Option<PathBuf> {
        let direct = dir.join(exe);
        if direct.is_file() {
            return Some(direct);
        }
        if depth == 0 {
            return None;
        }
        std::fs::read_dir(dir)
            .ok()?
            .flatten()
            .filter(|e| e.path().is_dir())
            .find_map(|e| search(&e.path(), exe, depth - 1))
    }
    if let Some(p) = search(Path::new("bench-corpus/tools"), &exe, 2) {
        return Some(p);
    }
    let path_var = std::env::var_os("PATH")?;
    let mut dirs: Vec<PathBuf> = std::env::split_paths(&path_var).collect();
    // Git for Windows ships these tools outside the default PATH.
    dirs.push(PathBuf::from(r"C:\Program Files\Git\usr\bin"));
    dirs.push(PathBuf::from(r"C:\Program Files\Git\mingw64\bin"));
    dirs.into_iter().map(|d| d.join(&exe)).find(|p| p.is_file())
}

/// Times a tool that writes its compressed output to stdout.
fn bench_stdout_tool(tool: &Path, args: &[&str], files: &[CorpusFile]) -> Option<Totals> {
    let mut totals = Totals::default();
    for file in files {
        let t = Instant::now();
        let out = Command::new(tool)
            .args(args)
            .arg(&file.path)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        let elapsed = t.elapsed();
        if !out.status.success() {
            return None;
        }
        totals.original += file.data.len() as u64;
        totals.compressed += out.stdout.len() as u64;
        totals.comp_time += elapsed;
        totals.per_file.push((
            file.name.clone(),
            out.stdout.len() as u64,
            file.data.len() as f64 / MB / elapsed.as_secs_f64().max(1e-9),
            0.0,
        ));
    }
    Some(totals)
}

/// Uses zstd's built-in in-memory benchmark (`-b<level>`), parsing lines like
/// `  3#dickens : 10192446 -> 3667613 (x2.779),  230.1 MB/s,  871.0 MB/s`.
fn bench_zstd(tool: &Path, level: u32, threads: usize, files: &[CorpusFile]) -> Option<Totals> {
    let mut totals = Totals {
        has_decomp: true,
        ..Totals::default()
    };
    for file in files {
        let mut cmd = Command::new(tool);
        cmd.arg(format!("-b{level}"))
            .arg("-i1")
            .arg(format!("-T{threads}"));
        if level > 19 {
            cmd.arg("--ultra");
        }
        let out = cmd.arg(&file.path).stdin(Stdio::null()).output().ok()?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        // Progress updates are separated by carriage returns; use the last
        // line that carries all three figures.
        let parse = |line: &str| -> Option<(u64, f64, f64)> {
            let after = line.split("->").nth(1)?;
            let size: u64 = after.split('(').next()?.trim().parse().ok()?;
            let mut speeds = after
                .split(',')
                .skip(1)
                .filter_map(|s| s.trim().split(' ').next()?.parse::<f64>().ok());
            Some((size, speeds.next()?, speeds.next()?))
        };
        let (size, comp, decomp) = text.split(['\r', '\n']).rev().find_map(parse)?;
        let megabytes = file.data.len() as f64 / 1e6; // zstd reports MB = 10^6 bytes
        totals.original += file.data.len() as u64;
        totals.compressed += size;
        totals.comp_time += Duration::from_secs_f64(megabytes / comp.max(1e-9));
        totals.decomp_time += Duration::from_secs_f64(megabytes / decomp.max(1e-9));
        totals
            .per_file
            .push((file.name.clone(), size, comp / 1.048576, decomp / 1.048576));
    }
    Some(totals)
}

fn print_row(label: &str, t: &Totals) {
    let decomp = if t.has_decomp {
        format!("{:>9.1}", t.decomp_mbs())
    } else {
        format!("{:>9}", "-")
    };
    println!(
        "{:<16} {:>13} {:>7.2}% {:>9.1} {}",
        label,
        t.compressed,
        t.ratio_pct(),
        t.comp_mbs(),
        decomp
    );
}

fn print_per_file(t: &Totals) {
    for (name, size, comp, decomp) in &t.per_file {
        println!(
            "    {:<12} {:>13} {:>8} {:>9.1} {:>9.1}",
            name, size, "", comp, decomp
        );
    }
}

/// Times the individual stages of the LZ path on one file (single thread),
/// to show where compression and decompression time goes.
fn profile(files: &[CorpusFile], level: u32) {
    use nexus_core::block::detector::detect_block_type;
    use nexus_core::block::splitter::split_subblock_ranges;
    use nexus_core::compress::lz::{
        decode_block, params_for_level, superblock_size_for_level, LzDecodeScratch, LzEncoder,
    };
    let sb_size = superblock_size_for_level(level);
    println!(
        "{:<10} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8}   (MB/s per stage, level {level})",
        "file", "split", "detect", "xxh32", "parse", "entropy", "decode"
    );
    for file in files {
        let mut t = [Duration::ZERO; 6];
        for sb in file.data.chunks(sb_size) {
            let t0 = Instant::now();
            let ranges = split_subblock_ranges(sb);
            t[0] += t0.elapsed();
            let t0 = Instant::now();
            for r in &ranges {
                std::hint::black_box(detect_block_type(&sb[r.clone()]));
            }
            t[1] += t0.elapsed();
            let t0 = Instant::now();
            for r in &ranges {
                std::hint::black_box(xxhash(&sb[r.clone()]));
            }
            t[2] += t0.elapsed();

            let mut enc = LzEncoder::new(params_for_level(level), sb.len());
            let t0 = Instant::now();
            for r in &ranges {
                std::hint::black_box(enc.parse_only(sb, r.start, r.end));
            }
            t[3] += t0.elapsed();

            let mut enc = LzEncoder::new(params_for_level(level), sb.len());
            let mut payloads = Vec::new();
            let t0 = Instant::now();
            for r in &ranges {
                let mut p = Vec::new();
                enc.compress_block(sb, r.start, r.end, &mut p);
                payloads.push(p);
            }
            t[4] += t0.elapsed();

            let mut scratch = LzDecodeScratch::default();
            let mut out = Vec::with_capacity(sb.len());
            let t0 = Instant::now();
            for (r, p) in ranges.iter().zip(&payloads) {
                decode_block(p, &mut out, r.len(), &mut scratch).expect("decode");
            }
            t[5] += t0.elapsed();
            assert_eq!(out, sb);
        }
        // Entropy coding time = full compress minus parse.
        t[4] = t[4].saturating_sub(t[3]);
        let mb = file.data.len() as f64 / MB;
        let s = |d: Duration| mb / d.as_secs_f64().max(1e-9);
        println!(
            "{:<10} {:>8.0} {:>8.0} {:>8.0} {:>8.1} {:>8.1} {:>8.1}",
            file.name,
            s(t[0]),
            s(t[1]),
            s(t[2]),
            s(t[3]),
            s(t[4]),
            s(t[5])
        );
    }
}

/// Stand-in for the block checksum cost (FNV-style pass over the data).
fn xxhash(data: &[u8]) -> u32 {
    data.chunks(4).fold(0x811C_9DC5u32, |h, c| {
        (h ^ c.iter().fold(0u32, |a, &b| (a << 8) | b as u32)).wrapping_mul(0x0100_0193)
    })
}

fn run() -> Result<bool, String> {
    let opts = parse_args()?;
    if opts.threads > 0 {
        rayon::ThreadPoolBuilder::new()
            .num_threads(opts.threads)
            .build_global()
            .map_err(|e| e.to_string())?;
    }
    let threads = rayon::current_num_threads();
    let files = load_corpus(&opts.corpus, opts.filter.as_deref())?;
    let total: u64 = files.iter().map(|f| f.data.len() as u64).sum();

    println!(
        "Corpus: {} ({} files, {:.1} MB)   NEXUS threads: {}",
        opts.corpus.display(),
        files.len(),
        total as f64 / MB,
        threads
    );
    if opts.profile {
        for &level in &opts.levels {
            profile(&files, level);
        }
        return Ok(true);
    }
    println!(
        "{:<16} {:>13} {:>8} {:>9} {:>9}",
        "compressor", "bytes", "ratio", "comp MB/s", "dec MB/s"
    );
    println!("{}", "-".repeat(60));

    let mut ok = true;
    let mut results: BTreeMap<u32, Totals> = BTreeMap::new();
    for &level in &opts.levels {
        let t = bench_nexus(&files, level)?;
        print_row(&format!("nexus -{level}"), &t);
        if opts.per_file {
            print_per_file(&t);
        }
        results.insert(level, t);
    }

    // Every level must be at least as small as the level below it.
    let mut prev: Option<(u32, u64)> = None;
    for (&level, t) in &results {
        if let Some((pl, ps)) = prev {
            if t.compressed > ps {
                println!(
                    "WARNING: level {level} ({}) is larger than level {pl} ({ps})",
                    t.compressed
                );
            }
        }
        prev = Some((level, t.compressed));
    }

    if opts.refs {
        println!("{}", "-".repeat(60));
        if let Some(zstd) = find_tool("zstd") {
            for level in [1, 3, 9, 19] {
                match bench_zstd(&zstd, level, threads, &files) {
                    Some(t) => {
                        print_row(&format!("zstd -{level}"), &t);
                        if opts.per_file {
                            print_per_file(&t);
                        }
                    }
                    None => println!("zstd -{level}: failed to run"),
                }
            }
        } else {
            println!("zstd: not found (see scripts/bench/README.md)");
        }
        let thread_arg = format!("-T{threads}");
        let stdout_tools: [(&str, &str, Vec<&str>); 7] = [
            ("gzip -6", "gzip", vec!["-6", "-c"]),
            ("brotli -5", "brotli", vec!["-q", "5", "-c"]),
            ("brotli -9", "brotli", vec!["-q", "9", "-c"]),
            ("brotli -11", "brotli", vec!["-q", "11", "-c"]),
            ("bzip2 -9", "bzip2", vec!["-9", "-c"]),
            ("xz -6", "xz", vec!["-6", &thread_arg, "-c"]),
            ("xz -9", "xz", vec!["-9", &thread_arg, "-c"]),
        ];
        for (label, tool, args) in &stdout_tools {
            match find_tool(tool).and_then(|p| bench_stdout_tool(&p, args, &files)) {
                Some(t) => {
                    print_row(label, &t);
                    if opts.per_file {
                        print_per_file(&t);
                    }
                }
                None => println!("{label}: not available"),
            }
        }
        println!("(reference tools other than zstd include process start-up and file I/O)");
    }

    if let Some(path) = &opts.check {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        println!("{}", "-".repeat(60));
        for line in text.lines().filter(|l| !l.starts_with('#')) {
            let mut cols = line.split('\t');
            let (Some(level), Some(size), Some(comp)) = (cols.next(), cols.next(), cols.next())
            else {
                continue;
            };
            let (Ok(level), Ok(size), Ok(comp)) = (
                level.parse::<u32>(),
                size.parse::<u64>(),
                comp.parse::<f64>(),
            ) else {
                continue;
            };
            let Some(now) = results.get(&level) else {
                continue;
            };
            let delta = now.compressed as f64 * 100.0 / size as f64 - 100.0;
            let speed = now.comp_mbs() * 100.0 / comp.max(1e-9) - 100.0;
            // Size is deterministic: any growth beyond rounding is a regression.
            let verdict = if now.compressed as f64 > size as f64 * 1.0005 {
                ok = false;
                "SIZE REGRESSION"
            } else if speed < -25.0 {
                "slower (timing is noisy; re-run to confirm)"
            } else {
                "ok"
            };
            println!("level {level:>2}: size {delta:+.2}%  compress speed {speed:+.0}%  {verdict}");
        }
    }

    if let Some(path) = &opts.save {
        let mut text = String::from("# level\tbytes\tcomp_mbs\tdecomp_mbs\n");
        for (level, t) in &results {
            text.push_str(&format!(
                "{level}\t{}\t{:.1}\t{:.1}\n",
                t.compressed,
                t.comp_mbs(),
                t.decomp_mbs()
            ));
        }
        std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))?;
        println!("Saved baseline to {}", path.display());
    }

    println!("Roundtrip: all levels bit-exact on all files");
    Ok(ok)
}

fn main() {
    match run() {
        Ok(true) => {}
        Ok(false) => std::process::exit(1),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(2);
        }
    }
}
