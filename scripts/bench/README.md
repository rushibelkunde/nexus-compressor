# Benchmarking NEXUS

`nexus-bench` measures compressed size, compression speed, decompression
speed and bit-exact roundtrip for every level over a directory of real files,
and can compare the result with other compressors and with a saved baseline.

## One-time setup

```bash
bash scripts/bench/fetch_corpus.sh      # Silesia corpus + zstd binary -> bench-corpus/
cargo build --release -p nexus-bench
```

`bench-corpus/` is git-ignored. Any directory of files works as a corpus
(`--corpus DIR`); Silesia is the default because published numbers for other
compressors exist for it.

## Everyday use

```bash
# All LZ levels, one thread, so figures are comparable with other tools
target/release/nexus-bench --threads 1

# A few levels plus reference compressors found on this machine
target/release/nexus-bench --threads 1 --levels 1,3,6,9,11,13 --refs

# Per-file sizes and speeds
target/release/nexus-bench --levels 6 --per-file

# One file only (substring match on the file name)
target/release/nexus-bench --levels 1,6,11 --filter dickens

# Where does the time go? Per-stage MB/s for the LZ path
target/release/nexus-bench --profile --levels 1,6 --threads 1
```

Columns: compressed bytes, ratio (compressed / original), compression MB/s,
decompression MB/s. Speeds are the best of up to 3 (compress) or 5
(decompress) runs per file, in-process, on in-memory buffers.

Reference tools are run as child processes writing to a pipe, so their speed
includes process start-up and file reading and they report no decompression
speed. `zstd` is the exception: it is measured with its own in-memory
benchmark mode (`zstd -b`), which is directly comparable.

## Regression gate

Compressed size is deterministic, so any growth is a real regression.

```bash
# After a change you are happy with:
target/release/nexus-bench --threads 1 --save scripts/bench/baseline-silesia.tsv

# Before committing a codec change:
target/release/nexus-bench --threads 1 --check scripts/bench/baseline-silesia.tsv
```

`--check` exits with status 1 if any level's total grew by more than 0.05%.
It also prints the change in compression speed, flagged when more than 25%
slower - but timing is noisy, so re-run before believing a slowdown.

The run also warns whenever a higher level produces a larger total than a
lower one: levels must be monotonic.

## Reading the numbers honestly

* Compare like with like: `--threads 1` against single-threaded tools, or
  give both sides the same thread count (`xz -T`, `zstd -T`).
* A compressor that is best on both size and speed does not exist. Judge each
  level against the tool that occupies the same point of the trade-off:
  levels 1-4 against `zstd -1..-3`, levels 5-9 against `zstd -9..-19` and
  `brotli -5..-9`, levels 10-12 against `xz -9`, levels 13+ against `xz -9`
  and `bzip2 -9`.
* Run on a quiet machine. A background build easily costs 30% of throughput.
