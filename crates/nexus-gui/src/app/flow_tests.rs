//! End-to-end tests of the state machine, driven headlessly.
//!
//! The application is run frame by frame on an off-screen egui context — no
//! window is opened — with real worker threads compressing, verifying and
//! extracting real files in a scratch directory. This covers what a manual
//! click-through would: the command-line entry points, drag and drop, the
//! transitions between screens, cancellation and the repaint policy.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use eframe::egui::{self, DroppedFile, HoveredFile, Pos2, Rect, Shape};
use nexus_core::pipeline::CompressionPreset;

use super::*;
use crate::worker::JobHandle;

static NEXT_DIR: AtomicU32 = AtomicU32::new(0);

// A unique, empty scratch directory that is removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "nexus_gui_flow_{}_{}_{}",
            std::process::id(),
            tag,
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn sample_text() -> Vec<u8> {
    let mut data = Vec::new();
    for i in 0..12_000u32 {
        data.extend_from_slice(format!("record {i}: lorem ipsum dolor sit amet\n").as_bytes());
    }
    data
}

fn window_input() -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, crate::WINDOW_SIZE)),
        ..Default::default()
    }
}

fn run_frame(
    app: &mut NexusGuiApp,
    ctx: &egui::Context,
    input: egui::RawInput,
) -> egui::FullOutput {
    ctx.run(input, |ctx| app.frame(ctx))
}

// Runs frames until `done` holds for the current screen.
fn run_until(
    app: &mut NexusGuiApp,
    ctx: &egui::Context,
    what: &str,
    done: impl Fn(&Screen) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        run_frame(app, ctx, window_input());
        if done(&app.screen) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}; still on the {} screen",
            app.screen.tag()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn painted_texts(output: &egui::FullOutput) -> Vec<String> {
    fn walk(shape: &Shape, out: &mut Vec<String>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|inner| walk(inner, out)),
            Shape::Text(text) => out.push(text.galley.text().to_owned()),
            _ => {}
        }
    }
    let mut texts = Vec::new();
    for clipped in &output.shapes {
        walk(&clipped.shape, &mut texts);
    }
    texts
}

fn repaint_delay(output: &egui::FullOutput) -> Duration {
    output
        .viewport_output
        .get(&egui::ViewportId::ROOT)
        .map(|viewport| viewport.repaint_delay)
        .expect("the root viewport always reports")
}

fn dropped(paths: &[&Path]) -> egui::RawInput {
    egui::RawInput {
        dropped_files: paths
            .iter()
            .map(|path| DroppedFile {
                path: Some(path.to_path_buf()),
                ..Default::default()
            })
            .collect(),
        ..window_input()
    }
}

// An application that is "running" a job nobody executes, plus the sending
// ends of its channels so the test can play the worker.
fn app_with_detached_job(
    ctx: &egui::Context,
    origin: Origin,
    mode: JobMode,
) -> (
    NexusGuiApp,
    std::sync::mpsc::Sender<nexus_core::pipeline::ProgressUpdate>,
    std::sync::mpsc::Sender<JobOutcome>,
) {
    let mut app = NexusGuiApp::with_context(ctx, None);
    let (handle, progress_tx, outcome_tx) = JobHandle::detached();
    app.screen = Screen::Running(Box::new(RunningJob {
        mode,
        source_name: "source.bin".to_string(),
        level: (mode == JobMode::Compress).then_some(6),
        archive_bytes: None,
        output_path: Some(PathBuf::from("out/source.bin.nxar")),
        handle,
        telemetry: JobTelemetry::new(0, CellPlan::Unknown),
        started: Instant::now(),
        origin,
        job_id: 7,
    }));
    (app, progress_tx, outcome_tx)
}

fn compress_origin(ctx: &egui::Context) -> Origin {
    let input = std::env::temp_dir().join("nexus_gui_flow_missing_source.bin");
    Origin::Compress(Box::new(CompressSetup::new(input, None, false, ctx)))
}

#[test]
fn test_starts_on_home_without_arguments() {
    let ctx = egui::Context::default();
    let mut app = NexusGuiApp::with_context(&ctx, None);
    let output = run_frame(&mut app, &ctx, window_input());
    assert_eq!(app.screen.tag(), "home");
    let texts = painted_texts(&output);
    assert!(texts.iter().any(|t| t.contains("DROP A FILE OR FOLDER")));
    assert!(texts.iter().any(|t| t == "NEXUS"));
}

#[test]
fn test_cli_compress_with_auto_start_runs_to_the_result_screen() {
    let scratch = Scratch::new("cli_compress");
    let input = scratch.path("notes.txt");
    let data = sample_text();
    fs::write(&input, &data).unwrap();

    let ctx = egui::Context::default();
    let mut app = NexusGuiApp::with_context(
        &ctx,
        Some(InitialAction::Compress {
            path: input.clone(),
            preset: Some(CompressionPreset::Low),
            auto_start: true,
        }),
    );
    // The job must not start before the source has been scanned.
    assert_eq!(app.screen.tag(), "compress_setup");

    run_until(&mut app, &ctx, "compression to finish", |screen| {
        matches!(screen, Screen::Finished(_))
    });
    let Screen::Finished(finished) = &app.screen else {
        unreachable!()
    };
    let archive = scratch.path("notes.txt.nxar");
    assert_eq!(finished.mode, JobMode::Compress);
    assert_eq!(finished.location, archive);
    assert_eq!(finished.result.original_bytes, data.len() as u64);
    assert_eq!(
        finished.result.archive_bytes,
        fs::metadata(&archive).unwrap().len()
    );
    assert!(finished.result.archive_bytes < finished.result.original_bytes);
    assert_eq!(
        finished.path_bytes.iter().sum::<u64>(),
        data.len() as u64,
        "the method breakdown accounts for every input byte"
    );
    let Origin::Compress(setup) = &finished.origin else {
        panic!("compression must remember its setup");
    };
    assert_eq!(setup.level, CompressionPreset::Low.to_level());
    assert!(!setup.auto_start, "a restored setup must not start again");

    // The result screen paints the headline numbers.
    let texts = painted_texts(&run_frame(&mut app, &ctx, window_input()));
    assert!(texts.iter().any(|t| t == "COMPRESSION COMPLETE"));
    assert!(texts.iter().any(|t| t.contains("saved")));

    app.apply(Action::GoHome, &ctx);
    assert_eq!(app.screen.tag(), "home");
}

#[test]
fn test_cli_compress_without_auto_start_waits_on_the_setup_screen() {
    let scratch = Scratch::new("cli_compress_wait");
    let input = scratch.path("notes.txt");
    fs::write(&input, sample_text()).unwrap();

    let ctx = egui::Context::default();
    let mut app = NexusGuiApp::with_context(
        &ctx,
        Some(InitialAction::Compress {
            path: input.clone(),
            preset: Some(CompressionPreset::High),
            auto_start: false,
        }),
    );
    run_until(&mut app, &ctx, "the scan and the estimate", |screen| {
        matches!(screen, Screen::CompressSetup(setup)
            if setup.is_scanned() && setup.estimates.get(setup.level).is_some())
    });
    let Screen::CompressSetup(setup) = &app.screen else {
        unreachable!()
    };
    assert_eq!(setup.level, CompressionPreset::High.to_level());
    assert_eq!(setup.total_bytes(), Some(sample_text().len() as u64));
    assert!(setup.can_start());
    let estimate = setup.estimates.get(setup.level).unwrap();
    assert!(estimate.estimated_bytes < sample_text().len() as u64);
    assert!(!scratch.path("notes.txt.nxar").exists(), "nothing started");

    // Starting from the setup screen works like the START button.
    app.apply(Action::StartCompress, &ctx);
    assert_eq!(app.screen.tag(), "running");
    run_until(&mut app, &ctx, "compression to finish", |screen| {
        matches!(screen, Screen::Finished(_))
    });
    assert!(scratch.path("notes.txt.nxar").exists());
}

#[test]
fn test_cli_decompress_with_auto_start_restores_the_file() {
    let scratch = Scratch::new("cli_extract");
    let input = scratch.path("notes.txt");
    let archive = scratch.path("packed.nxar");
    let data = sample_text();
    fs::write(&input, &data).unwrap();
    nexus_core::compress_file(&input, &archive, 4).unwrap();

    let ctx = egui::Context::default();
    let mut app = NexusGuiApp::with_context(
        &ctx,
        Some(InitialAction::Decompress {
            path: archive.clone(),
            auto_start: true,
        }),
    );
    assert_eq!(app.screen.tag(), "running", "extract-here starts at once");

    run_until(&mut app, &ctx, "extraction to finish", |screen| {
        matches!(screen, Screen::Finished(_))
    });
    let Screen::Finished(finished) = &app.screen else {
        unreachable!()
    };
    let restored = scratch.path("packed");
    assert_eq!(finished.mode, JobMode::Extract);
    assert_eq!(finished.location, restored);
    assert_eq!(finished.result.original_bytes, data.len() as u64);
    assert_eq!(fs::read(&restored).unwrap(), data, "lossless roundtrip");
}

#[test]
fn test_double_clicked_archive_opens_setup_then_test_archive_reports_intact() {
    let scratch = Scratch::new("verify");
    let input = scratch.path("notes.txt");
    let archive = scratch.path("notes.nxar");
    let data = sample_text();
    fs::write(&input, &data).unwrap();
    nexus_core::compress_file(&input, &archive, 2).unwrap();

    // A bare archive path is what a double-click passes.
    let args = vec!["nexus-gui".to_string(), archive.display().to_string()];
    let initial = crate::cli_args::parse_args(&args);
    let ctx = egui::Context::default();
    let mut app = NexusGuiApp::with_context(&ctx, initial);
    let Screen::ExtractSetup(setup) = &app.screen else {
        panic!("a double-clicked archive opens the extraction setup");
    };
    assert_eq!(setup.output(), scratch.path("notes"));
    let summary = setup.summary.as_ref().expect("archive facts are read");
    assert_eq!(summary.original_bytes, data.len() as u64);

    app.apply(Action::StartVerify, &ctx);
    assert_eq!(app.screen.tag(), "running");
    run_until(&mut app, &ctx, "verification to finish", |screen| {
        matches!(screen, Screen::Finished(_))
    });
    let Screen::Finished(finished) = &app.screen else {
        unreachable!()
    };
    assert_eq!(finished.mode, JobMode::Verify);
    assert_eq!(finished.result.original_bytes, data.len() as u64);
    assert!(!scratch.path("notes").exists(), "testing writes nothing");
    let texts = painted_texts(&run_frame(&mut app, &ctx, window_input()));
    assert!(texts.iter().any(|t| t == "ARCHIVE IS INTACT"));

    // "Extract this archive" returns to the setup, which says it passed.
    app.apply(Action::BackToSetup, &ctx);
    let Screen::ExtractSetup(setup) = &app.screen else {
        panic!("back from a verification returns to the extraction setup");
    };
    let notice = setup.notice.as_ref().expect("verified notice");
    assert_eq!(notice.kind, NoticeKind::Success);
}

#[test]
fn test_damaged_archive_fails_verification_with_the_error_screen() {
    let scratch = Scratch::new("damaged");
    let input = scratch.path("notes.txt");
    let archive = scratch.path("notes.nxar");
    fs::write(&input, sample_text()).unwrap();
    nexus_core::compress_file(&input, &archive, 2).unwrap();
    // Flip a byte in the middle of the payload.
    let mut bytes = fs::read(&archive).unwrap();
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0x5A;
    fs::write(&archive, &bytes).unwrap();

    let ctx = egui::Context::default();
    let mut app = NexusGuiApp::with_context(&ctx, None);
    app.apply(Action::OpenArchive(archive), &ctx);
    app.apply(Action::StartVerify, &ctx);
    run_until(&mut app, &ctx, "verification to fail", |screen| {
        !matches!(screen, Screen::Running(_))
    });
    let Screen::Failed(failed) = &app.screen else {
        panic!(
            "a damaged archive must not verify; got the {} screen",
            app.screen.tag()
        );
    };
    assert_eq!(failed.title, "ARCHIVE FAILED VERIFICATION");
    assert!(failed.message.starts_with("Verification failed"));
    assert!(failed.origin.is_some());

    app.apply(Action::BackToSetup, &ctx);
    assert_eq!(app.screen.tag(), "extract_setup");
}

#[test]
fn test_cancelled_job_returns_to_its_setup_with_a_notice_not_the_error_screen() {
    let ctx = egui::Context::default();
    let (mut app, _progress_tx, outcome_tx) =
        app_with_detached_job(&ctx, compress_origin(&ctx), JobMode::Compress);

    // Pressing CANCEL only raises the flag; the screen stays until the
    // worker acknowledges.
    app.apply(Action::CancelJob, &ctx);
    let Screen::Running(job) = &app.screen else {
        panic!("still running until the worker stops");
    };
    assert!(job.handle.is_cancel_requested());
    let texts = painted_texts(&run_frame(&mut app, &ctx, window_input()));
    assert!(texts.iter().any(|t| t.starts_with("CANCELLING")));
    assert_eq!(app.screen.tag(), "running");

    outcome_tx.send(JobOutcome::Cancelled).unwrap();
    run_frame(&mut app, &ctx, window_input());
    let Screen::CompressSetup(setup) = &app.screen else {
        panic!(
            "cancel must return to the setup screen, got {}",
            app.screen.tag()
        );
    };
    let notice = setup.notice.as_ref().expect("a cancelled notice");
    assert_eq!(notice.kind, NoticeKind::Info);
    assert!(notice.text.contains("cancelled"));
}

#[test]
fn test_failed_job_shows_the_error_and_back_restores_the_setup() {
    let ctx = egui::Context::default();
    let (mut app, _progress_tx, outcome_tx) =
        app_with_detached_job(&ctx, compress_origin(&ctx), JobMode::Compress);
    outcome_tx
        .send(JobOutcome::Failed(
            "Compression failed: I/O error: disk full".to_string(),
        ))
        .unwrap();
    run_frame(&mut app, &ctx, window_input());
    let Screen::Failed(failed) = &app.screen else {
        panic!("expected the error screen, got {}", app.screen.tag());
    };
    assert_eq!(failed.title, "COMPRESSION FAILED");
    assert!(failed.message.contains("disk full"));

    app.apply(Action::BackToSetup, &ctx);
    let Screen::CompressSetup(setup) = &app.screen else {
        panic!("back must restore the compression setup");
    };
    assert!(setup.notice.is_none());
}

#[test]
fn test_vanished_worker_is_reported_instead_of_hanging() {
    let ctx = egui::Context::default();
    let (mut app, progress_tx, outcome_tx) =
        app_with_detached_job(&ctx, compress_origin(&ctx), JobMode::Compress);
    drop(progress_tx);
    drop(outcome_tx);
    run_frame(&mut app, &ctx, window_input());
    let Screen::Failed(failed) = &app.screen else {
        panic!("expected the error screen, got {}", app.screen.tag());
    };
    assert!(failed.message.contains("stopped unexpectedly"));
}

#[test]
fn test_progress_updates_reach_the_live_view() {
    use nexus_core::format::PATH_COUNT;
    use nexus_core::pipeline::ProgressUpdate;

    const MB: u64 = 1024 * 1024;
    let ctx = egui::Context::default();
    let (mut app, progress_tx, _outcome_tx) =
        app_with_detached_job(&ctx, compress_origin(&ctx), JobMode::Compress);

    let mut path_bytes = [0u64; PATH_COUNT];
    path_bytes[5] = 8 * MB;
    progress_tx
        .send(ProgressUpdate {
            bytes_processed: 8 * MB,
            total_bytes: 32 * MB,
            compressed_bytes: 2 * MB,
            path_bytes,
            current_target: "source.bin".to_string(),
            stage: "Compressing",
            fraction: 0.25,
        })
        .unwrap();
    let output = run_frame(&mut app, &ctx, window_input());
    let Screen::Running(job) = &app.screen else {
        panic!("still running");
    };
    assert_eq!(job.telemetry.bytes_processed(), 8 * MB);
    assert_eq!(job.telemetry.compressed_bytes(), 2 * MB);
    assert_eq!(job.telemetry.cells(), &[crate::methods::Method::Lz]);

    let texts = painted_texts(&output);
    assert!(texts.iter().any(|t| t == "25.0 %"), "progress percentage");
    assert!(
        texts
            .iter()
            .any(|t| t.contains("25.0 % of original") && t.contains("6.0 MB saved")),
        "live ratio and bytes saved: {texts:?}"
    );
    assert!(texts.iter().any(|t| t == "8.0 MB / 32.0 MB"));
    assert!(texts.iter().any(|t| t == "CANCEL"));
}

#[test]
fn test_dropped_paths_are_routed_by_extension() {
    let scratch = Scratch::new("drop");
    let plain = scratch.path("photo.png");
    let archive = scratch.path("backup.NXAR");
    let folder = scratch.path("folder.nxar");
    fs::write(&plain, b"not really a png").unwrap();
    fs::write(&archive, b"not really an archive").unwrap();
    fs::create_dir_all(&folder).unwrap();

    let ctx = egui::Context::default();
    let mut app = NexusGuiApp::with_context(&ctx, None);

    run_frame(&mut app, &ctx, dropped(&[&plain]));
    let Screen::CompressSetup(setup) = &app.screen else {
        panic!("a plain file opens the compression setup");
    };
    assert_eq!(setup.input_path, plain);
    assert!(setup.notice.is_none());

    // Dropping onto any screen re-routes; extension matching ignores case.
    run_frame(&mut app, &ctx, dropped(&[&archive]));
    let Screen::ExtractSetup(setup) = &app.screen else {
        panic!("an archive opens the extraction setup");
    };
    assert_eq!(setup.input_path, archive);
    assert!(setup.summary.is_err(), "garbage is reported, not trusted");

    // A folder is compressed even if its name ends in .nxar.
    run_frame(&mut app, &ctx, dropped(&[&folder]));
    let Screen::CompressSetup(setup) = &app.screen else {
        panic!("a folder opens the compression setup");
    };
    assert!(setup.is_dir);

    // Several items: the first one is used and the user is told so.
    run_frame(&mut app, &ctx, dropped(&[&archive, &plain, &folder]));
    let Screen::ExtractSetup(setup) = &app.screen else {
        panic!("the first dropped item decides");
    };
    assert!(setup.notice.as_ref().unwrap().text.starts_with("3 items"));
}

#[test]
fn test_drops_are_ignored_while_a_job_runs() {
    let ctx = egui::Context::default();
    let (mut app, _progress_tx, _outcome_tx) =
        app_with_detached_job(&ctx, compress_origin(&ctx), JobMode::Compress);
    run_frame(&mut app, &ctx, dropped(&[Path::new("whatever.nxar")]));
    assert_eq!(app.screen.tag(), "running");
}

#[test]
fn test_hovering_files_shows_the_matching_overlay() {
    let ctx = egui::Context::default();
    let mut app = NexusGuiApp::with_context(&ctx, None);
    let hover = |name: &str| egui::RawInput {
        hovered_files: vec![HoveredFile {
            path: Some(PathBuf::from("no_such_dir").join(name)),
            ..Default::default()
        }],
        ..window_input()
    };

    let texts = painted_texts(&run_frame(&mut app, &ctx, hover("backup.nxar")));
    assert!(texts.iter().any(|t| t == "DROP TO EXTRACT"));
    let texts = painted_texts(&run_frame(&mut app, &ctx, hover("report.pdf")));
    assert!(texts.iter().any(|t| t == "DROP TO COMPRESS"));
    // Hovering alone changes nothing, and the overlay goes away afterwards.
    let texts = painted_texts(&run_frame(&mut app, &ctx, window_input()));
    assert!(!texts.iter().any(|t| t.starts_with("DROP TO")));
    assert_eq!(app.screen.tag(), "home");
}

#[test]
fn test_repaints_continuously_only_while_a_job_runs() {
    let ctx = egui::Context::default();

    // Idle on the home screen: after things settle, no repaint is scheduled.
    let mut app = NexusGuiApp::with_context(&ctx, None);
    let mut output = run_frame(&mut app, &ctx, window_input());
    for _ in 0..5 {
        output = run_frame(&mut app, &ctx, window_input());
    }
    assert!(
        repaint_delay(&output) > Duration::from_secs(3600),
        "the idle home screen keeps repainting (delay {:?})",
        repaint_delay(&output)
    );

    // Running: the next frame is due within the animation interval.
    let (mut app, _progress_tx, _outcome_tx) =
        app_with_detached_job(&ctx, compress_origin(&ctx), JobMode::Compress);
    let mut output = run_frame(&mut app, &ctx, window_input());
    for _ in 0..5 {
        output = run_frame(&mut app, &ctx, window_input());
    }
    assert!(
        repaint_delay(&output) <= RUNNING_REPAINT,
        "a running job must keep animating (delay {:?})",
        repaint_delay(&output)
    );
}

#[test]
fn test_actions_that_do_not_apply_to_the_current_screen_are_ignored() {
    let ctx = egui::Context::default();
    let mut app = NexusGuiApp::with_context(&ctx, None);
    for action in [
        Action::StartCompress,
        Action::StartExtract,
        Action::StartVerify,
        Action::CancelJob,
        Action::BackToSetup,
    ] {
        app.apply(action, &ctx);
        assert_eq!(app.screen.tag(), "home");
    }
}
