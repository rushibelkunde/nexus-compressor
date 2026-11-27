//! Background job execution.
//!
//! A job (compress, extract or verify) runs on its own thread so the UI
//! never blocks. The thread reports through two `mpsc` channels — a stream
//! of [`ProgressUpdate`]s and a single [`JobOutcome`] — and can be stopped
//! through a shared cancel flag that the core checks at every superblock
//! boundary.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui;
use nexus_core::pipeline::{
    compress_file_with_control, decompress_file_with_control, verify_file_with_control, JobControl,
    ProgressCallback, ProgressUpdate,
};
use nexus_core::NexusError;

/// What a job does, with everything it needs to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobSpec {
    /// Compress a file or folder into an archive.
    Compress {
        /// File or folder to compress.
        input: PathBuf,
        /// Archive to create.
        output: PathBuf,
        /// Compression level, 1 to 19.
        level: u32,
    },
    /// Extract an archive.
    Extract {
        /// Archive to read.
        input: PathBuf,
        /// File (single-file archive) or directory (folder archive) to write.
        output: PathBuf,
    },
    /// Decode an archive and check every checksum without writing anything.
    Verify {
        /// Archive to read.
        input: PathBuf,
    },
}

/// Figures reported by a job that ran to completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobSuccess {
    /// Uncompressed size in bytes (input of compression, output of
    /// extraction, decoded size of verification).
    pub original_bytes: u64,
    /// Size of the archive file on disk in bytes.
    pub archive_bytes: u64,
    /// Wall-clock duration of the job.
    pub elapsed: Duration,
}

/// How a job ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobOutcome {
    /// Ran to completion.
    Success(JobSuccess),
    /// Stopped because the cancel flag was set.
    Cancelled,
    /// Failed; the message is ready to show to the user.
    Failed(String),
}

/// UI-side handle of a running job.
pub struct JobHandle {
    /// Progress updates, one per finished superblock plus a final one.
    pub progress_rx: Receiver<ProgressUpdate>,
    /// The single final outcome.
    pub outcome_rx: Receiver<JobOutcome>,
    cancel: Arc<AtomicBool>,
}

impl JobHandle {
    /// Asks the job to stop at the next superblock boundary.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// A handle that is not attached to any thread, plus the sending ends
    /// of its channels, for tests that drive a running screen by hand.
    #[cfg(test)]
    pub fn detached() -> (Self, mpsc::Sender<ProgressUpdate>, mpsc::Sender<JobOutcome>) {
        let (progress_tx, progress_rx) = mpsc::channel();
        let (outcome_tx, outcome_rx) = mpsc::channel();
        let handle = Self {
            progress_rx,
            outcome_rx,
            cancel: Arc::new(AtomicBool::new(false)),
        };
        (handle, progress_tx, outcome_tx)
    }

    /// Whether [`JobHandle::cancel`] has been called.
    pub fn is_cancel_requested(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// Maps a core result onto a [`JobOutcome`]: cancellation is its own outcome
/// (it is not an error), every other error becomes a prefixed message.
pub fn outcome_from<T>(
    result: Result<T, NexusError>,
    failure_prefix: &str,
    success: impl FnOnce(T) -> JobSuccess,
) -> JobOutcome {
    match result {
        Ok(value) => JobOutcome::Success(success(value)),
        Err(NexusError::Cancelled) => JobOutcome::Cancelled,
        Err(err) => JobOutcome::Failed(format!("{failure_prefix}: {err}")),
    }
}

fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

// Runs the job on the calling thread. `original_seen` is fed by the progress
// callback with the core's authoritative total size.
fn run_job(spec: &JobSpec, control: &JobControl, original_seen: &AtomicU64) -> JobOutcome {
    let started = Instant::now();
    match spec {
        JobSpec::Compress {
            input,
            output,
            level,
        } => outcome_from(
            compress_file_with_control(input, output, *level, control),
            "Compression failed",
            |()| JobSuccess {
                original_bytes: original_seen.load(Ordering::Relaxed),
                archive_bytes: file_len(output),
                elapsed: started.elapsed(),
            },
        ),
        JobSpec::Extract { input, output } => outcome_from(
            decompress_file_with_control(input, output, control),
            "Extraction failed",
            |()| JobSuccess {
                original_bytes: original_seen.load(Ordering::Relaxed),
                archive_bytes: file_len(input),
                elapsed: started.elapsed(),
            },
        ),
        JobSpec::Verify { input } => outcome_from(
            verify_file_with_control(input, control),
            "Verification failed",
            |decoded| JobSuccess {
                original_bytes: decoded,
                archive_bytes: file_len(input),
                elapsed: started.elapsed(),
            },
        ),
    }
}

/// Starts `spec` on a new thread. `ctx` is woken whenever there is something
/// new to show, so the UI does not have to poll while idle.
pub fn spawn_job(spec: JobSpec, ctx: egui::Context) -> JobHandle {
    let (progress_tx, progress_rx) = mpsc::channel::<ProgressUpdate>();
    let (outcome_tx, outcome_rx) = mpsc::channel::<JobOutcome>();
    let cancel = Arc::new(AtomicBool::new(false));
    let original_seen = Arc::new(AtomicU64::new(0));

    let callback_ctx = ctx.clone();
    let callback_seen = Arc::clone(&original_seen);
    let progress: ProgressCallback = Arc::new(move |update: ProgressUpdate| {
        callback_seen.fetch_max(update.total_bytes, Ordering::Relaxed);
        // The receiver disappears when the UI leaves the running screen;
        // a failed send is harmless.
        let _ = progress_tx.send(update);
        callback_ctx.request_repaint();
    });

    let control = JobControl {
        progress: Some(progress),
        cancel: Some(Arc::clone(&cancel)),
    };

    thread::spawn(move || {
        let outcome = run_job(&spec, &control, &original_seen);
        let _ = outcome_tx.send(outcome);
        ctx.request_repaint();
    });

    JobHandle {
        progress_rx,
        outcome_rx,
        cancel,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::AtomicU32;

    static NEXT_DIR: AtomicU32 = AtomicU32::new(0);

    // A unique, empty scratch directory under the system temp dir.
    fn scratch_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nexus_gui_test_{}_{}_{}",
            std::process::id(),
            tag,
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_data() -> Vec<u8> {
        let mut data = Vec::new();
        for i in 0..4000u32 {
            data.extend_from_slice(format!("line {i}: the quick brown fox\n").as_bytes());
        }
        data
    }

    fn wait(handle: &JobHandle) -> JobOutcome {
        handle
            .outcome_rx
            .recv_timeout(Duration::from_secs(120))
            .expect("job did not finish")
    }

    #[test]
    fn test_outcome_from_maps_cancel_and_errors() {
        let dummy = |()| JobSuccess {
            original_bytes: 1,
            archive_bytes: 2,
            elapsed: Duration::ZERO,
        };
        assert!(matches!(
            outcome_from(Ok(()), "X failed", dummy),
            JobOutcome::Success(JobSuccess {
                original_bytes: 1,
                archive_bytes: 2,
                ..
            })
        ));
        assert_eq!(
            outcome_from(Err(NexusError::Cancelled), "X failed", dummy),
            JobOutcome::Cancelled
        );
        match outcome_from(
            Err(NexusError::DataCorruption("bad block".into())),
            "X failed",
            dummy,
        ) {
            JobOutcome::Failed(message) => {
                assert!(message.starts_with("X failed: "));
                assert!(message.contains("bad block"));
            }
            other => panic!("unexpected outcome {other:?}"),
        }
    }

    #[test]
    fn test_compress_verify_extract_roundtrip_through_workers() {
        let dir = scratch_dir("roundtrip");
        let input = dir.join("input.txt");
        let archive = dir.join("input.txt.nxar");
        let restored = dir.join("restored.txt");
        let data = sample_data();
        fs::write(&input, &data).unwrap();

        let ctx = egui::Context::default();

        let job = spawn_job(
            JobSpec::Compress {
                input: input.clone(),
                output: archive.clone(),
                level: 2,
            },
            ctx.clone(),
        );
        let JobOutcome::Success(compressed) = wait(&job) else {
            panic!("compression did not succeed");
        };
        assert_eq!(compressed.original_bytes, data.len() as u64);
        assert_eq!(
            compressed.archive_bytes,
            fs::metadata(&archive).unwrap().len()
        );
        assert!(compressed.archive_bytes < compressed.original_bytes);
        let updates: Vec<ProgressUpdate> = job.progress_rx.try_iter().collect();
        assert!(
            updates.len() >= 2,
            "expected a block update and a final one"
        );
        assert_eq!(updates.last().unwrap().stage, "Complete");

        let job = spawn_job(
            JobSpec::Verify {
                input: archive.clone(),
            },
            ctx.clone(),
        );
        let JobOutcome::Success(verified) = wait(&job) else {
            panic!("verification did not succeed");
        };
        assert_eq!(verified.original_bytes, data.len() as u64);
        assert_eq!(verified.archive_bytes, compressed.archive_bytes);
        assert!(!restored.exists(), "verification must not write output");

        let job = spawn_job(
            JobSpec::Extract {
                input: archive.clone(),
                output: restored.clone(),
            },
            ctx,
        );
        let JobOutcome::Success(extracted) = wait(&job) else {
            panic!("extraction did not succeed");
        };
        assert_eq!(extracted.original_bytes, data.len() as u64);
        assert_eq!(fs::read(&restored).unwrap(), data);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_cancelled_compression_reports_cancelled_and_leaves_no_archive() {
        let dir = scratch_dir("cancel");
        let input = dir.join("input.txt");
        let archive = dir.join("input.txt.nxar");
        fs::write(&input, sample_data()).unwrap();

        // Run synchronously with the flag already set so the test does not
        // race the worker thread.
        let cancel = Arc::new(AtomicBool::new(true));
        let control = JobControl {
            progress: None,
            cancel: Some(cancel),
        };
        let outcome = run_job(
            &JobSpec::Compress {
                input,
                output: archive.clone(),
                level: 2,
            },
            &control,
            &AtomicU64::new(0),
        );
        assert_eq!(outcome, JobOutcome::Cancelled);
        assert!(!archive.exists(), "partial archive must be removed");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_missing_input_fails_with_message() {
        let dir = scratch_dir("missing");
        let job = spawn_job(
            JobSpec::Verify {
                input: dir.join("does_not_exist.nxar"),
            },
            egui::Context::default(),
        );
        match wait(&job) {
            JobOutcome::Failed(message) => assert!(message.starts_with("Verification failed")),
            other => panic!("unexpected outcome {other:?}"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_cancel_flag_is_shared() {
        let dir = scratch_dir("flag");
        let job = spawn_job(
            JobSpec::Verify {
                input: dir.join("nothing.nxar"),
            },
            egui::Context::default(),
        );
        assert!(!job.is_cancel_requested());
        job.cancel();
        assert!(job.is_cancel_requested());
        let _ = wait(&job);
        let _ = fs::remove_dir_all(&dir);
    }
}
