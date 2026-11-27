//! Live statistics of a running job, derived from the core's progress
//! updates: monotonic byte counters, a throughput ring buffer, the ETA and
//! the block map.
//!
//! Everything takes the current time as an explicit `now` (seconds since the
//! job started) instead of reading a clock, so the arithmetic is fully
//! unit-tested.
//!
//! Progress updates are emitted from the core's worker threads, one per
//! finished superblock, so they can arrive in bursts and slightly out of
//! order. Counters are therefore merged with `max`, and throughput samples
//! are coalesced over a short window rather than computed per update.

use std::collections::VecDeque;

use nexus_core::pipeline::ProgressUpdate;

use crate::methods::{dominant_method, path_delta, path_max, Method, PathBytes, METHOD_COUNT};
use crate::util::mb_per_sec;

/// Stage string the core sends with its final progress update.
pub const COMPLETE_STAGE: &str = "Complete";

/// Number of throughput samples kept for the sparkline.
pub const SPEED_HISTORY: usize = 120;

/// Minimum spacing of throughput samples in seconds (at most ~10 per second).
pub const SAMPLE_INTERVAL_SECS: f64 = 0.1;

/// Fixed-capacity ring buffer of `f32` samples; pushing onto a full buffer
/// drops the oldest sample.
#[derive(Debug, Clone)]
pub struct SampleRing {
    samples: VecDeque<f32>,
    capacity: usize,
}

impl SampleRing {
    /// Creates an empty ring holding at most `capacity` samples (at least 1).
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            samples: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Appends a sample, evicting the oldest one when full. Non-finite
    /// values are stored as zero so they can never poison a chart scale.
    pub fn push(&mut self, value: f32) {
        let value = if value.is_finite() { value } else { 0.0 };
        if self.samples.len() == self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back(value);
    }

    /// Number of samples currently held.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Whether no sample has been pushed yet.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// The most recent sample.
    pub fn last(&self) -> Option<f32> {
        self.samples.back().copied()
    }

    /// The samples in chronological order (oldest first).
    pub fn to_vec(&self) -> Vec<f32> {
        self.samples.iter().copied().collect()
    }
}

/// How the total number of block-map cells is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellPlan {
    /// Compression: the input is cut into superblocks of this many bytes.
    BySuperblockSize(u64),
    /// Extraction / verification: the archive header states the count.
    Fixed(usize),
    /// Not known; only finished cells are shown.
    Unknown,
}

/// State of one drawn block-map cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellState {
    /// Not finished yet (drawn dimmed).
    Pending,
    /// Finished; coloured by the dominant method.
    Done(Method),
}

/// Remaining seconds given the elapsed time and the completed fraction,
/// assuming progress continues at the average rate so far. `None` when the
/// fraction is not strictly between 0 and 1 or the inputs are not finite.
pub fn remaining_secs(elapsed_secs: f64, fraction: f32) -> Option<f64> {
    let f = f64::from(fraction);
    if !elapsed_secs.is_finite() || !f.is_finite() || elapsed_secs < 0.0 {
        return None;
    }
    if f >= 1.0 {
        return Some(0.0);
    }
    if f <= 0.0 {
        return None;
    }
    Some(elapsed_secs * (1.0 - f) / f)
}

/// Reduces the per-superblock cell list to at most `max_cells` drawn cells.
///
/// When `expected` fits, every superblock gets its own cell. Otherwise
/// consecutive superblocks are grouped; a group is `Done` once all of its
/// superblocks have finished and takes the method most of them used.
/// Returns the cells and the number of superblocks each cell stands for.
pub fn block_cells(done: &[Method], expected: usize, max_cells: usize) -> (Vec<CellState>, usize) {
    let expected = expected.max(done.len());
    if expected == 0 || max_cells == 0 {
        return (Vec::new(), 1);
    }
    let group = expected.div_ceil(max_cells).max(1);
    let cell_count = expected.div_ceil(group);
    let mut cells = Vec::with_capacity(cell_count);
    for cell in 0..cell_count {
        let start = cell * group;
        let end = (start + group).min(expected);
        if end <= done.len() {
            cells.push(CellState::Done(majority_method(&done[start..end])));
        } else {
            cells.push(CellState::Pending);
        }
    }
    (cells, group)
}

// Most frequent method of a non-empty group; ties go to the method listed
// first in `Method::ALL` so the result is deterministic.
fn majority_method(group: &[Method]) -> Method {
    let mut counts = [0usize; METHOD_COUNT];
    for method in group {
        counts[method.index()] += 1;
    }
    let mut best = Method::Lz;
    let mut best_count = 0usize;
    for method in Method::ALL {
        if counts[method.index()] > best_count {
            best = method;
            best_count = counts[method.index()];
        }
    }
    best
}

/// Live statistics of one compression, extraction or verification job.
#[derive(Debug, Clone)]
pub struct JobTelemetry {
    total_bytes: u64,
    bytes_processed: u64,
    compressed_bytes: u64,
    path_bytes: PathBytes,
    fraction: f32,
    stage: &'static str,
    complete: bool,
    cells: Vec<Method>,
    plan: CellPlan,
    speed: SampleRing,
    pending_since: Option<f64>,
    last_sample_time: f64,
    last_sample_bytes: u64,
    eta_anchor: Option<(f64, f64)>,
}

impl JobTelemetry {
    /// Creates telemetry for a job whose total size is `total_bytes` if
    /// already known (pass 0 otherwise; the first update supplies it).
    pub fn new(total_bytes: u64, plan: CellPlan) -> Self {
        Self {
            total_bytes,
            bytes_processed: 0,
            compressed_bytes: 0,
            path_bytes: [0; nexus_core::format::PATH_COUNT],
            fraction: 0.0,
            stage: "Starting",
            complete: false,
            cells: Vec::new(),
            plan,
            speed: SampleRing::new(SPEED_HISTORY),
            pending_since: None,
            last_sample_time: 0.0,
            last_sample_bytes: 0,
            eta_anchor: None,
        }
    }

    /// Folds one progress update into the statistics. `now` is seconds since
    /// the job started.
    pub fn record(&mut self, update: &ProgressUpdate, now: f64) {
        let is_final = update.stage == COMPLETE_STAGE;

        // The core's total is authoritative (for folders it includes the
        // archive manifest, which the pre-scan cannot know).
        if update.total_bytes > 0 {
            self.total_bytes = update.total_bytes;
        }

        let delta = path_delta(&self.path_bytes, &update.path_bytes);
        self.path_bytes = path_max(&self.path_bytes, &update.path_bytes);
        self.bytes_processed = self.bytes_processed.max(update.bytes_processed);
        self.compressed_bytes = self.compressed_bytes.max(update.compressed_bytes);
        if update.fraction.is_finite() {
            self.fraction = self.fraction.max(update.fraction.clamp(0.0, 1.0));
        }

        if is_final {
            self.complete = true;
            self.fraction = 1.0;
            self.stage = COMPLETE_STAGE;
            self.eta_anchor = None;
            // Flush whatever arrived since the last throughput sample.
            if self.bytes_processed > self.last_sample_bytes {
                self.take_sample(now);
            }
            self.pending_since = None;
            return;
        }

        self.stage = update.stage;

        // One non-final update == one finished superblock == one cell. An
        // out-of-order update can carry an empty delta (a later update
        // already reported its bytes); it then inherits the previous colour.
        let method = dominant_method(&delta)
            .or_else(|| self.cells.last().copied())
            .unwrap_or(Method::Lz);
        self.cells.push(method);

        if self.pending_since.is_none() {
            self.pending_since = Some(now);
        }
        self.eta_anchor = remaining_secs(now, self.fraction).map(|left| (now, left));
    }

    /// Advances time-based state; call once per UI frame while the job runs.
    /// Emits a throughput sample once updates have been pending for
    /// [`SAMPLE_INTERVAL_SECS`], so a burst of updates becomes one sample.
    pub fn tick(&mut self, now: f64) {
        if let Some(since) = self.pending_since {
            if now - since >= SAMPLE_INTERVAL_SECS {
                self.take_sample(now);
            }
        }
    }

    fn take_sample(&mut self, now: f64) {
        let dt = now - self.last_sample_time;
        if !dt.is_finite() || dt < 1e-6 {
            return;
        }
        let bytes = self.bytes_processed.saturating_sub(self.last_sample_bytes);
        self.speed.push(mb_per_sec(bytes, dt) as f32);
        self.last_sample_time = now;
        self.last_sample_bytes = self.bytes_processed;
        self.pending_since = None;
    }

    /// Total original bytes of the job (0 while unknown).
    pub fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    /// Original bytes processed so far.
    pub fn bytes_processed(&self) -> u64 {
        self.bytes_processed
    }

    /// Archive bytes produced (compression) or consumed (extraction) so far.
    pub fn compressed_bytes(&self) -> u64 {
        self.compressed_bytes
    }

    /// Cumulative original bytes per compression path.
    pub fn path_bytes(&self) -> &PathBytes {
        &self.path_bytes
    }

    /// Completed fraction in `[0, 1]`.
    pub fn fraction(&self) -> f32 {
        self.fraction
    }

    /// Stage text of the most recent update.
    pub fn stage(&self) -> &'static str {
        self.stage
    }

    /// Whether the core's final update has been received.
    pub fn is_complete(&self) -> bool {
        self.complete
    }

    /// Dominant method of every finished superblock, in arrival order.
    pub fn cells(&self) -> &[Method] {
        &self.cells
    }

    /// Total number of superblocks expected, when it can be known.
    pub fn expected_cells(&self) -> Option<usize> {
        if self.complete {
            return Some(self.cells.len());
        }
        let planned = match self.plan {
            CellPlan::BySuperblockSize(size) => {
                let count = self.total_bytes.div_ceil(size.max(1)).max(1);
                Some(usize::try_from(count).unwrap_or(usize::MAX))
            }
            CellPlan::Fixed(count) => Some(count),
            CellPlan::Unknown => None,
        };
        planned.map(|count| count.max(self.cells.len()))
    }

    /// Recent throughput samples in MB/s.
    pub fn speed(&self) -> &SampleRing {
        &self.speed
    }

    /// Most recent throughput sample in MB/s.
    pub fn current_mb_s(&self) -> Option<f64> {
        self.speed.last().map(f64::from)
    }

    /// Average throughput since the job started, in MB/s.
    pub fn average_mb_s(&self, now: f64) -> f64 {
        mb_per_sec(self.bytes_processed, now)
    }

    /// Estimated seconds remaining at time `now`. The estimate is fixed at
    /// each progress update and counts down between updates.
    pub fn eta_secs(&self, now: f64) -> Option<f64> {
        if self.complete {
            return Some(0.0);
        }
        let (at, left) = self.eta_anchor?;
        Some((left - (now - at).max(0.0)).max(0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::format::PATH_COUNT;

    const MB: u64 = 1024 * 1024;
    const LZ_PATH: usize = 5;
    const BWT_PATH: usize = 1;
    const STORED_PATH: usize = 4;

    fn update(
        processed: u64,
        total: u64,
        compressed: u64,
        paths: &[(usize, u64)],
        stage: &'static str,
    ) -> ProgressUpdate {
        let mut path_bytes = [0u64; PATH_COUNT];
        for (index, bytes) in paths {
            path_bytes[*index] = *bytes;
        }
        ProgressUpdate {
            bytes_processed: processed,
            total_bytes: total,
            compressed_bytes: compressed,
            path_bytes,
            current_target: "test".to_string(),
            stage,
            fraction: if total == 0 {
                1.0
            } else {
                processed as f32 / total as f32
            },
        }
    }

    #[test]
    fn test_ring_keeps_most_recent_samples() {
        let mut ring = SampleRing::new(3);
        assert!(ring.is_empty());
        assert_eq!(ring.last(), None);
        for v in [1.0, 2.0, 3.0, 4.0, 5.0] {
            ring.push(v);
        }
        assert_eq!(ring.len(), 3);
        assert_eq!(ring.to_vec(), vec![3.0, 4.0, 5.0]);
        assert_eq!(ring.last(), Some(5.0));
    }

    #[test]
    fn test_ring_zero_capacity_is_promoted_to_one() {
        let mut ring = SampleRing::new(0);
        ring.push(1.0);
        ring.push(2.0);
        assert_eq!(ring.to_vec(), vec![2.0]);
    }

    #[test]
    fn test_ring_sanitises_non_finite_samples() {
        let mut ring = SampleRing::new(4);
        ring.push(f32::NAN);
        ring.push(f32::INFINITY);
        ring.push(2.0);
        assert_eq!(ring.to_vec(), vec![0.0, 0.0, 2.0]);
    }

    #[test]
    fn test_remaining_secs() {
        assert_eq!(remaining_secs(10.0, 0.0), None);
        assert_eq!(remaining_secs(10.0, 1.0), Some(0.0));
        assert_eq!(remaining_secs(10.0, 0.5), Some(10.0));
        let left = remaining_secs(30.0, 0.25).unwrap();
        assert!((left - 90.0).abs() < 1e-6);
        assert_eq!(remaining_secs(f64::NAN, 0.5), None);
        assert_eq!(remaining_secs(10.0, f32::NAN), None);
        assert_eq!(remaining_secs(-1.0, 0.5), None);
    }

    #[test]
    fn test_block_cells_one_to_one() {
        let done = [Method::Lz, Method::Bwt];
        let (cells, group) = block_cells(&done, 4, 100);
        assert_eq!(group, 1);
        assert_eq!(
            cells,
            vec![
                CellState::Done(Method::Lz),
                CellState::Done(Method::Bwt),
                CellState::Pending,
                CellState::Pending
            ]
        );
    }

    #[test]
    fn test_block_cells_grouped_when_too_many() {
        // 10 superblocks into at most 4 cells -> groups of 3 -> 4 cells.
        let mut done = vec![Method::Lz; 5];
        done[3] = Method::Bwt;
        done[4] = Method::Bwt;
        let (cells, group) = block_cells(&done, 10, 4);
        assert_eq!(group, 3);
        assert_eq!(cells.len(), 4);
        // Group 0 = blocks 0..3 (all LZ, finished).
        assert_eq!(cells[0], CellState::Done(Method::Lz));
        // Group 1 = blocks 3..6, only two finished.
        assert_eq!(cells[1], CellState::Pending);
        assert_eq!(cells[3], CellState::Pending);
    }

    #[test]
    fn test_block_cells_group_takes_majority_and_last_group_may_be_short() {
        let done = [
            Method::Lz,
            Method::Bwt,
            Method::Bwt,
            Method::Stored,
            Method::Stored,
            Method::Lz,
            Method::Neural,
        ];
        let (cells, group) = block_cells(&done, 7, 3);
        assert_eq!(group, 3);
        assert_eq!(
            cells,
            vec![
                CellState::Done(Method::Bwt),
                CellState::Done(Method::Stored),
                CellState::Done(Method::Neural)
            ]
        );
    }

    #[test]
    fn test_block_cells_degenerate_inputs() {
        assert_eq!(block_cells(&[], 0, 10), (Vec::new(), 1));
        assert_eq!(block_cells(&[Method::Lz], 1, 0), (Vec::new(), 1));
        // More finished cells than expected: expected grows to match.
        let (cells, _) = block_cells(&[Method::Lz, Method::Lz], 1, 10);
        assert_eq!(cells.len(), 2);
    }

    #[test]
    fn test_cells_coloured_by_delta_not_by_cumulative_total() {
        let mut t = JobTelemetry::new(4 * MB, CellPlan::BySuperblockSize(MB));
        t.record(
            &update(MB, 4 * MB, 300_000, &[(LZ_PATH, MB)], "Compressing"),
            0.1,
        );
        t.record(
            &update(2 * MB, 4 * MB, 600_000, &[(LZ_PATH, 2 * MB)], "Compressing"),
            0.2,
        );
        // Third block is BWT even though LZ still dominates the totals.
        t.record(
            &update(
                3 * MB,
                4 * MB,
                800_000,
                &[(LZ_PATH, 2 * MB), (BWT_PATH, MB)],
                "Compressing",
            ),
            0.3,
        );
        t.record(
            &update(
                4 * MB,
                4 * MB,
                1_800_000,
                &[(LZ_PATH, 2 * MB), (BWT_PATH, MB), (STORED_PATH, MB)],
                "Compressing",
            ),
            0.4,
        );
        assert_eq!(
            t.cells(),
            &[Method::Lz, Method::Lz, Method::Bwt, Method::Stored]
        );
        assert_eq!(t.expected_cells(), Some(4));
    }

    #[test]
    fn test_out_of_order_updates_keep_counters_monotonic() {
        let mut t = JobTelemetry::new(4 * MB, CellPlan::BySuperblockSize(MB));
        // The later snapshot arrives first.
        t.record(
            &update(
                2 * MB,
                4 * MB,
                500_000,
                &[(BWT_PATH, 2 * MB)],
                "Compressing",
            ),
            0.1,
        );
        t.record(
            &update(MB, 4 * MB, 250_000, &[(BWT_PATH, MB)], "Compressing"),
            0.1,
        );
        assert_eq!(t.bytes_processed(), 2 * MB);
        assert_eq!(t.compressed_bytes(), 500_000);
        assert_eq!(t.path_bytes()[BWT_PATH], 2 * MB);
        assert!((t.fraction() - 0.5).abs() < 1e-6);
        // Both superblocks still get a cell; the stale one inherits the colour.
        assert_eq!(t.cells(), &[Method::Bwt, Method::Bwt]);
    }

    #[test]
    fn test_complete_update_adds_no_cell_and_finishes() {
        let mut t = JobTelemetry::new(MB, CellPlan::BySuperblockSize(MB));
        t.record(&update(MB, MB, 1000, &[(LZ_PATH, MB)], "Compressing"), 0.5);
        t.record(&update(MB, MB, 1000, &[(LZ_PATH, MB)], COMPLETE_STAGE), 0.6);
        assert!(t.is_complete());
        assert_eq!(t.cells().len(), 1);
        assert_eq!(t.fraction(), 1.0);
        assert_eq!(t.eta_secs(0.7), Some(0.0));
        assert_eq!(t.stage(), COMPLETE_STAGE);
        assert_eq!(t.expected_cells(), Some(1));
    }

    #[test]
    fn test_update_total_overrides_prescan_total() {
        let mut t = JobTelemetry::new(1000, CellPlan::BySuperblockSize(MB));
        assert_eq!(t.total_bytes(), 1000);
        t.record(
            &update(MB, 3 * MB, 10, &[(LZ_PATH, MB)], "Compressing"),
            0.1,
        );
        assert_eq!(t.total_bytes(), 3 * MB);
        assert_eq!(t.expected_cells(), Some(3));
    }

    #[test]
    fn test_expected_cells_by_plan() {
        let empty = JobTelemetry::new(0, CellPlan::BySuperblockSize(MB));
        assert_eq!(empty.expected_cells(), Some(1));
        let sized = JobTelemetry::new(5 * MB + 1, CellPlan::BySuperblockSize(MB));
        assert_eq!(sized.expected_cells(), Some(6));
        let zero_block = JobTelemetry::new(5 * MB, CellPlan::BySuperblockSize(0));
        assert!(zero_block.expected_cells().is_some());
        let fixed = JobTelemetry::new(0, CellPlan::Fixed(9));
        assert_eq!(fixed.expected_cells(), Some(9));
        let unknown = JobTelemetry::new(0, CellPlan::Unknown);
        assert_eq!(unknown.expected_cells(), None);
    }

    #[test]
    fn test_throughput_burst_is_coalesced_into_one_sample() {
        let mut t = JobTelemetry::new(64 * MB, CellPlan::BySuperblockSize(8 * MB));
        // Four superblocks finish within a few milliseconds, 2 s into the job.
        for i in 1..=4u64 {
            t.record(
                &update(
                    i * 8 * MB,
                    64 * MB,
                    i * MB,
                    &[(LZ_PATH, i * 8 * MB)],
                    "Compressing",
                ),
                2.0 + i as f64 * 0.001,
            );
            t.tick(2.0 + i as f64 * 0.001);
        }
        assert!(t.speed().is_empty(), "sample must wait for the window");
        t.tick(2.05);
        assert!(t.speed().is_empty());
        t.tick(2.2);
        assert_eq!(t.speed().len(), 1);
        // 32 MB in 2.2 s.
        let sample = t.current_mb_s().unwrap();
        assert!((sample - 32.0 / 2.2).abs() < 1e-3, "got {sample}");
        // No new data: further ticks add nothing.
        t.tick(3.0);
        assert_eq!(t.speed().len(), 1);
    }

    #[test]
    fn test_throughput_sampling_is_rate_limited() {
        let mut t = JobTelemetry::new(1000 * MB, CellPlan::BySuperblockSize(MB));
        // 1000 updates over one second, ticking after each.
        for i in 1..=1000u64 {
            let now = i as f64 * 0.001;
            t.record(
                &update(i * MB, 1000 * MB, i, &[(LZ_PATH, i * MB)], "Compressing"),
                now,
            );
            t.tick(now);
        }
        assert!(t.speed().len() <= 10, "got {} samples", t.speed().len());
        assert!(t.speed().len() >= 8);
        // Steady 1000 MB/s.
        let sample = t.current_mb_s().unwrap();
        assert!((sample - 1000.0).abs() < 20.0, "got {sample}");
    }

    #[test]
    fn test_complete_flushes_pending_throughput() {
        let mut t = JobTelemetry::new(MB, CellPlan::BySuperblockSize(MB));
        t.record(&update(MB, MB, 10, &[(LZ_PATH, MB)], "Compressing"), 0.5);
        t.record(&update(MB, MB, 10, &[(LZ_PATH, MB)], COMPLETE_STAGE), 0.5);
        assert_eq!(t.speed().len(), 1);
        assert!((t.current_mb_s().unwrap() - 2.0).abs() < 1e-6);
    }

    #[test]
    fn test_average_speed() {
        let mut t = JobTelemetry::new(10 * MB, CellPlan::Unknown);
        assert_eq!(t.average_mb_s(0.0), 0.0);
        t.record(
            &update(5 * MB, 10 * MB, 10, &[(LZ_PATH, 5 * MB)], "Extracting"),
            1.0,
        );
        assert!((t.average_mb_s(2.0) - 2.5).abs() < 1e-9);
    }

    #[test]
    fn test_eta_counts_down_between_updates() {
        let mut t = JobTelemetry::new(4 * MB, CellPlan::BySuperblockSize(MB));
        assert_eq!(t.eta_secs(1.0), None);
        // 25 % after 10 s -> 30 s left.
        t.record(
            &update(MB, 4 * MB, 10, &[(LZ_PATH, MB)], "Compressing"),
            10.0,
        );
        assert!((t.eta_secs(10.0).unwrap() - 30.0).abs() < 1e-6);
        assert!((t.eta_secs(25.0).unwrap() - 15.0).abs() < 1e-6);
        // Never negative, even when the job overruns the estimate.
        assert_eq!(t.eta_secs(100.0), Some(0.0));
    }

    #[test]
    fn test_stage_follows_updates() {
        let mut t = JobTelemetry::new(MB, CellPlan::Unknown);
        assert_eq!(t.stage(), "Starting");
        assert!(t.cells().is_empty());
        t.record(&update(MB, MB, 10, &[(LZ_PATH, MB)], "Verifying"), 0.2);
        assert_eq!(t.cells().len(), 1);
        assert_eq!(t.stage(), "Verifying");
    }

    #[test]
    fn test_empty_input_job() {
        // Empty file: one empty superblock, then Complete, total 0.
        let mut t = JobTelemetry::new(0, CellPlan::BySuperblockSize(MB));
        t.record(&update(0, 0, 16, &[], "Compressing"), 0.01);
        t.record(&update(0, 0, 16, &[], COMPLETE_STAGE), 0.02);
        assert!(t.is_complete());
        assert_eq!(t.cells().len(), 1);
        assert_eq!(t.total_bytes(), 0);
        assert_eq!(t.fraction(), 1.0);
    }
}
