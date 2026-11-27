//! Background analysis for the compression setup screen: scanning the
//! source (size and a content sample) and trial-compressing that sample to
//! estimate the output size at a given level.
//!
//! Both run off the UI thread. Scanning a large folder walks its whole tree,
//! and the estimate runs the real codec, which takes a noticeable moment at
//! the optimal-parse levels.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread;

use eframe::egui;
use nexus_core::pipeline::{estimate_compression, sample_path, CompressionEstimate};

use crate::engine::{clamp_level, ESTIMATE_LEVEL_CAP};

/// Number of bytes sampled from the source for the size estimate.
pub const SAMPLE_BYTES: usize = 64 * 1024;

/// Result of scanning a compression source.
#[derive(Debug, Clone)]
pub struct SourceScan {
    /// Leading bytes of the source (of the first files, for a folder).
    pub sample: Arc<Vec<u8>>,
    /// Total uncompressed size in bytes.
    pub total_bytes: u64,
    /// Why the source could not be read, if it could not.
    pub error: Option<String>,
}

/// A source scan that is either still running or finished.
pub enum ScanState {
    /// The scan thread has not reported yet.
    Pending(Receiver<SourceScan>),
    /// The scan finished.
    Ready(SourceScan),
}

impl ScanState {
    /// Starts scanning `path` on a background thread; `ctx` is repainted
    /// when the result is in.
    pub fn start(path: PathBuf, ctx: egui::Context) -> Self {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let scan = match sample_path(&path, SAMPLE_BYTES) {
                Ok((sample, total_bytes, _is_dir)) => SourceScan {
                    sample: Arc::new(sample),
                    total_bytes,
                    error: None,
                },
                Err(err) => SourceScan {
                    sample: Arc::new(Vec::new()),
                    total_bytes: 0,
                    error: Some(err.to_string()),
                },
            };
            let _ = tx.send(scan);
            ctx.request_repaint();
        });
        ScanState::Pending(rx)
    }

    /// Picks up the scan result if it has arrived. Returns `true` when the
    /// state changed to [`ScanState::Ready`] during this call.
    pub fn poll(&mut self) -> bool {
        let ScanState::Pending(rx) = self else {
            return false;
        };
        let scan = match rx.try_recv() {
            Ok(scan) => scan,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => SourceScan {
                sample: Arc::new(Vec::new()),
                total_bytes: 0,
                error: Some("the source scan stopped unexpectedly".to_string()),
            },
        };
        *self = ScanState::Ready(scan);
        true
    }

    /// The finished scan, if any.
    pub fn ready(&self) -> Option<&SourceScan> {
        match self {
            ScanState::Ready(scan) => Some(scan),
            ScanState::Pending(_) => None,
        }
    }
}

/// The estimate slot used for `level`. Levels above
/// [`ESTIMATE_LEVEL_CAP`] share the cap's slot because the core's trial
/// compression is clamped there.
pub fn estimate_slot(level: u32) -> usize {
    clamp_level(level).min(ESTIMATE_LEVEL_CAP) as usize
}

/// Per-level cache of size estimates with at most one trial compression in
/// flight at a time, so dragging the level slider cannot pile up threads.
pub struct EstimateCache {
    cached: [Option<CompressionEstimate>; ESTIMATE_LEVEL_CAP as usize + 1],
    in_flight: Option<(usize, Receiver<CompressionEstimate>)>,
}

impl Default for EstimateCache {
    fn default() -> Self {
        Self {
            cached: [None; ESTIMATE_LEVEL_CAP as usize + 1],
            in_flight: None,
        }
    }
}

impl EstimateCache {
    /// The cached estimate for `level`, if it has been computed.
    pub fn get(&self, level: u32) -> Option<CompressionEstimate> {
        self.cached[estimate_slot(level)]
    }

    /// Whether a trial compression is currently running.
    #[cfg(test)]
    pub fn is_busy(&self) -> bool {
        self.in_flight.is_some()
    }

    /// Stores a finished estimate directly, bypassing the worker thread.
    #[cfg(test)]
    pub fn insert(&mut self, level: u32, estimate: CompressionEstimate) {
        self.cached[estimate_slot(level)] = Some(estimate);
    }

    /// Collects a finished trial compression, if one has finished.
    pub fn poll(&mut self) {
        let Some((slot, rx)) = &self.in_flight else {
            return;
        };
        match rx.try_recv() {
            Ok(estimate) => {
                self.cached[*slot] = Some(estimate);
                self.in_flight = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => self.in_flight = None,
        }
    }

    /// Makes sure an estimate for `level` is cached or being computed. Does
    /// nothing while another trial compression is in flight; call it again
    /// (every frame) and the newest level is picked up when that finishes.
    pub fn ensure(&mut self, level: u32, scan: &SourceScan, ctx: &egui::Context) {
        let slot = estimate_slot(level);
        if self.cached[slot].is_some() || self.in_flight.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let sample = Arc::clone(&scan.sample);
        let total_bytes = scan.total_bytes;
        let ctx = ctx.clone();
        thread::spawn(move || {
            let estimate = estimate_compression(&sample, total_bytes, slot as u32);
            let _ = tx.send(estimate);
            ctx.request_repaint();
        });
        self.in_flight = Some((slot, rx));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn text_scan() -> SourceScan {
        let mut sample = Vec::new();
        while sample.len() < 32 * 1024 {
            sample.extend_from_slice(b"NEXUS estimates are trial compressions. ");
        }
        SourceScan {
            total_bytes: 10 * sample.len() as u64,
            sample: Arc::new(sample),
            error: None,
        }
    }

    #[test]
    fn test_estimate_slot_clamps_and_caps() {
        assert_eq!(estimate_slot(0), 1);
        assert_eq!(estimate_slot(1), 1);
        assert_eq!(estimate_slot(6), 6);
        assert_eq!(estimate_slot(12), 12);
        assert_eq!(estimate_slot(13), 12);
        assert_eq!(estimate_slot(19), 12);
        assert_eq!(estimate_slot(500), 12);
    }

    #[test]
    fn test_cache_shares_slot_above_cap() {
        let mut cache = EstimateCache::default();
        assert_eq!(cache.get(12), None);
        let estimate = estimate_compression(&text_scan().sample, 1000, 2);
        cache.insert(15, estimate);
        assert_eq!(cache.get(12), Some(estimate));
        assert_eq!(cache.get(19), Some(estimate));
        assert_eq!(cache.get(11), None);
    }

    #[test]
    fn test_ensure_computes_in_background_and_caches() {
        let ctx = egui::Context::default();
        let scan = text_scan();
        let mut cache = EstimateCache::default();
        cache.ensure(2, &scan, &ctx);
        assert!(cache.is_busy());

        // A second request while busy must not replace the running one.
        cache.ensure(6, &scan, &ctx);

        let deadline = Instant::now() + Duration::from_secs(60);
        while cache.get(2).is_none() {
            assert!(Instant::now() < deadline, "estimate did not arrive");
            thread::sleep(Duration::from_millis(5));
            cache.poll();
        }
        assert!(!cache.is_busy());
        assert_eq!(cache.get(6), None);

        let estimate = cache.get(2).unwrap();
        assert!(estimate.estimated_bytes > 0);
        assert!(estimate.estimated_bytes < scan.total_bytes);
        assert!(!estimate.is_incompressible);
    }

    #[test]
    fn test_scan_reports_missing_source() {
        let missing = std::env::temp_dir().join("nexus_gui_test_definitely_missing_source");
        let mut state = ScanState::start(missing, egui::Context::default());
        let deadline = Instant::now() + Duration::from_secs(30);
        while !state.poll() {
            assert!(Instant::now() < deadline, "scan did not finish");
            thread::sleep(Duration::from_millis(5));
        }
        let scan = state.ready().unwrap();
        assert!(scan.error.is_some());
        assert_eq!(scan.total_bytes, 0);
        // Polling a finished scan is a no-op.
        assert!(!state.poll());
    }

    #[test]
    fn test_scan_reads_file_size_and_sample() {
        let path =
            std::env::temp_dir().join(format!("nexus_gui_test_scan_{}.txt", std::process::id()));
        std::fs::write(&path, vec![b'a'; 100_000]).unwrap();
        let mut state = ScanState::start(path.clone(), egui::Context::default());
        let deadline = Instant::now() + Duration::from_secs(30);
        while !state.poll() {
            assert!(Instant::now() < deadline, "scan did not finish");
            thread::sleep(Duration::from_millis(5));
        }
        let scan = state.ready().unwrap().clone();
        let _ = std::fs::remove_file(&path);
        assert_eq!(scan.total_bytes, 100_000);
        assert_eq!(scan.sample.len(), SAMPLE_BYTES);
        assert!(scan.error.is_none());
    }
}
