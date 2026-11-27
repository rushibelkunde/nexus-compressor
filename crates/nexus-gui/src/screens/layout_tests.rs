//! Headless layout checks for every screen.
//!
//! The GUI cannot be looked at from a test run, so these tests lay each
//! screen out in an off-screen egui pass at several window sizes and assert
//! the properties a visual review would look for first:
//!
//! * nothing is wider than the space it was given (no horizontal overflow),
//! * no text is cut off by its clip rectangle or the window,
//! * no two pieces of text overlap,
//! * the normal states fit the default window without scrolling.
//!
//! They exercise the real `show` / `footer` functions with realistic and
//! with hostile state (very long names, huge block counts, empty inputs,
//! every level). Set `NEXUS_GUI_PRINT_LAYOUT=1` and run with `--nocapture`
//! to print how much height each screen uses.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, vec2, Pos2, Rect, Shape, Vec2};
use nexus_core::format::{BlockType, PATH_COUNT};
use nexus_core::pipeline::{CompressionEstimate, ProgressUpdate};

use super::compress_setup::CompressSetup;
use super::extract_setup::{ArchiveSummary, ExtractSetup};
use super::failed::FailedJob;
use super::finished::FinishedJob;
use super::home::HomeState;
use super::running::RunningJob;
use super::{drop_overlay, JobMode, Origin};
use crate::analysis::{ScanState, SourceScan};
use crate::app::{footer_panel, main_panel, Screen};
use crate::telemetry::{CellPlan, JobTelemetry, COMPLETE_STAGE};
use crate::theme;
use crate::widgets::{Notice, NoticeKind};
use crate::worker::{JobHandle, JobSuccess};

const MB: u64 = 1024 * 1024;

/// The default window size (see `main.rs`).
const DEFAULT_SIZE: Vec2 = Vec2::new(680.0, 580.0);

/// Window sizes checked: the minimum, the default and a large one.
const SIZES: [Vec2; 3] = [
    Vec2::new(600.0, 500.0),
    DEFAULT_SIZE,
    Vec2::new(1400.0, 900.0),
];

struct PaintedText {
    rect: Rect,
    clip: Rect,
    text: String,
}

struct Layout {
    screen: Rect,
    /// `(used, given)` rectangles of the footer (if any) and the body.
    areas: Vec<(Rect, Rect)>,
    texts: Vec<PaintedText>,
}

impl Layout {
    /// `(used, given)` of the scrolling body, which is always painted last.
    fn body(&self) -> (Rect, Rect) {
        *self.areas.last().expect("the body is always laid out")
    }
}

fn collect_texts(shape: &Shape, clip: Rect, out: &mut Vec<PaintedText>) {
    match shape {
        Shape::Vec(shapes) => {
            for inner in shapes {
                collect_texts(inner, clip, out);
            }
        }
        Shape::Text(text) => {
            let rect = text.galley.rect.translate(text.pos.to_vec2());
            if rect.width() > 0.5 && rect.height() > 0.5 && !text.galley.text().trim().is_empty() {
                out.push(PaintedText {
                    rect,
                    clip,
                    text: text.galley.text().to_owned(),
                });
            }
        }
        _ => {}
    }
}

// Runs `frame` in an off-screen egui pass of the given window size. Three
// passes are run so that layouts which need a frame to settle have settled.
// `frame` returns the `(used, given)` rectangles of the areas it painted.
fn lay_out(size: Vec2, mut frame: impl FnMut(&egui::Context) -> Vec<(Rect, Rect)>) -> Layout {
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let screen = Rect::from_min_size(Pos2::ZERO, size);
    let mut layout = None;
    for pass in 0..3 {
        let input = egui::RawInput {
            screen_rect: Some(screen),
            time: Some(f64::from(pass) * 0.1),
            ..Default::default()
        };
        let mut areas = Vec::new();
        let output = ctx.run(input, |ctx| {
            areas = frame(ctx);
        });
        let mut texts = Vec::new();
        for clipped in &output.shapes {
            collect_texts(&clipped.shape, clipped.clip_rect, &mut texts);
        }
        layout = Some(Layout {
            screen,
            areas,
            texts,
        });
    }
    layout.expect("at least one pass ran")
}

// Paints a screen exactly like `app::show_screen`, recording the extents.
fn show_measured(ctx: &egui::Context, screen: &mut Screen) -> Vec<(Rect, Rect)> {
    let mut areas = Vec::new();
    if screen.has_footer() {
        areas.push(footer_panel(ctx, |ui| {
            screen.footer(ui);
            (ui.min_rect(), ui.max_rect())
        }));
    }
    areas.push(main_panel(ctx, screen.tag(), |ui| {
        screen.show(ui);
        (ui.min_rect(), ui.max_rect())
    }));
    areas
}

fn assert_sound(layout: &Layout, what: &str) {
    let size = layout.screen.size();
    for (used, given) in &layout.areas {
        assert!(
            used.width() <= given.width() + 0.5,
            "{what} @ {size:?}: contents are {} wide but only {} is available",
            used.width(),
            given.width()
        );
    }
    assert!(
        !layout.texts.is_empty(),
        "{what} @ {size:?}: nothing was painted"
    );

    for painted in &layout.texts {
        assert!(
            painted.rect.left() >= painted.clip.left() - 1.0
                && painted.rect.right() <= painted.clip.right() + 1.0,
            "{what} @ {size:?}: text {:?} ({:?}) is cut off by its clip rect {:?}",
            painted.text,
            painted.rect,
            painted.clip
        );
        // A small clip rect belongs to a custom-painted widget or the pinned
        // footer (the scroll area's is much taller); text must fit it
        // vertically too.
        if painted.clip.height() < 200.0 {
            assert!(
                painted.rect.top() >= painted.clip.top() - 1.5
                    && painted.rect.bottom() <= painted.clip.bottom() + 1.5,
                "{what} @ {size:?}: text {:?} ({:?}) is cut off vertically by {:?}",
                painted.text,
                painted.rect,
                painted.clip
            );
        }
        assert!(
            painted.rect.left() >= layout.screen.left() - 0.5
                && painted.rect.right() <= layout.screen.right() + 0.5,
            "{what} @ {size:?}: text {:?} ({:?}) leaves the window",
            painted.text,
            painted.rect
        );
    }

    // Only the visible part of a text can collide with anything: what the
    // scroll area clips away (below the fold) is not on screen.
    let visible = |painted: &PaintedText| painted.rect.intersect(painted.clip).shrink(1.0);
    for (i, a) in layout.texts.iter().enumerate() {
        for b in &layout.texts[i + 1..] {
            let overlap = visible(a).intersect(visible(b));
            assert!(
                !(overlap.width() > 0.5 && overlap.height() > 0.5),
                "{what} @ {size:?}: text {:?} ({:?}) overlaps text {:?} ({:?})",
                a.text,
                a.rect,
                b.text,
                b.rect
            );
        }
    }
}

/// How a screen is expected to fit the default window.
#[derive(Clone, Copy, PartialEq)]
enum Fit {
    /// The body must fit the default window without scrolling.
    NoScroll,
    /// The body may scroll (long warnings, hostile input).
    MayScroll,
}

// Lays the screen built by `build` out at every size in `SIZES` and checks
// it. With `Fit::NoScroll` the body must also fit the default window.
fn check(what: &str, fit: Fit, mut build: impl FnMut(&egui::Context) -> Screen) {
    for size in SIZES {
        let layout = lay_out(size, |ctx| {
            let mut screen = build(ctx);
            show_measured(ctx, &mut screen)
        });
        assert_sound(&layout, what);
        let (used, given) = layout.body();
        if std::env::var_os("NEXUS_GUI_PRINT_LAYOUT").is_some() {
            eprintln!(
                "LAYOUT {what} @ {size:?}: body uses {:.0} of {:.0}",
                used.height(),
                given.height()
            );
        }
        if fit == Fit::NoScroll && size == DEFAULT_SIZE {
            assert!(
                used.height() <= given.height() + 0.5,
                "{what}: body is {} tall but the default window only shows {}",
                used.height(),
                given.height()
            );
        }
    }
}

fn long_name() -> String {
    format!("{}.dat", "a_very_long_file_name_".repeat(14))
}

fn long_notice() -> Notice {
    Notice::new(
        NoticeKind::Info,
        "Extraction cancelled. Partially extracted data may remain at \
         C:/Users/someone/Documents/a/rather/deeply/nested/folder/structure/with/a/long/name.",
    )
}

fn estimate(total: u64, incompressible: bool) -> CompressionEstimate {
    let estimated = if incompressible { total } else { total / 3 };
    CompressionEstimate {
        estimated_bytes: estimated,
        ratio_percent: estimated as f64 / total.max(1) as f64 * 100.0,
        savings_percent: 100.0 - estimated as f64 / total.max(1) as f64 * 100.0,
        detected_type: if incompressible {
            BlockType::Incompressible
        } else {
            BlockType::StructuredData
        },
        is_incompressible: incompressible,
    }
}

fn compress_setup(ctx: &egui::Context, name: &str, level: u32, total: u64) -> CompressSetup {
    let input = PathBuf::from("C:/data/projects/nexus/samples").join(name);
    let mut setup = CompressSetup::new(input, None, false, ctx);
    setup.level = level;
    setup.output_path = "C:/out/sample.nxar".to_string();
    setup.scan = ScanState::Ready(SourceScan {
        sample: Arc::new(vec![0u8; 16]),
        total_bytes: total,
        error: None,
    });
    setup
}

fn extract_setup(name: &str, readable: bool) -> ExtractSetup {
    let mut path_bytes = [0u64; PATH_COUNT];
    path_bytes[5] = 700 * MB;
    path_bytes[1] = 200 * MB;
    path_bytes[4] = 124 * MB;
    ExtractSetup {
        input_path: PathBuf::from("C:/data/archives").join(name),
        output_path: "C:/data/archives/restored".to_string(),
        archive_bytes: 311 * MB,
        summary: if readable {
            Ok(ArchiveSummary {
                format_version: 2,
                original_bytes: 1024 * MB,
                block_count: 8123,
                superblock_count: 128,
                path_bytes,
            })
        } else {
            Err("Invalid magic bytes: expected [0x4E, 0x58, 0x53, 0x01], got [0, 0, 0, 0]".into())
        },
        notice: None,
    }
}

// A running job fed with `finished_blocks` progress updates out of
// `total_blocks`, cycling through every method so all colours appear.
fn running(mode: JobMode, finished_blocks: usize, total_blocks: usize, name: &str) -> RunningJob {
    let (handle, _progress_tx, _outcome_tx) = JobHandle::detached();
    let block = 8 * MB;
    let total = total_blocks as u64 * block;
    let plan = match mode {
        JobMode::Compress => CellPlan::BySuperblockSize(block),
        JobMode::Extract => CellPlan::Fixed(total_blocks),
        JobMode::Verify => CellPlan::Unknown,
    };
    let stage = match mode {
        JobMode::Compress => "Compressing",
        JobMode::Extract => "Extracting",
        JobMode::Verify => "Verifying",
    };
    let mut telemetry = JobTelemetry::new(total, plan);
    let mut path_bytes = [0u64; PATH_COUNT];
    for i in 0..finished_blocks {
        path_bytes[[5, 1, 2, 3, 4][i % 5]] += block;
        let processed = (i as u64 + 1) * block;
        let now = 0.2 + i as f64 * 0.2;
        telemetry.record(
            &ProgressUpdate {
                bytes_processed: processed,
                total_bytes: total,
                compressed_bytes: processed / 3,
                path_bytes,
                current_target: name.to_string(),
                stage,
                fraction: processed as f32 / total.max(1) as f32,
            },
            now,
        );
        telemetry.tick(now + 0.15);
    }
    RunningJob {
        mode,
        source_name: name.to_string(),
        level: (mode == JobMode::Compress).then_some(11),
        archive_bytes: (mode != JobMode::Compress).then_some(total / 3),
        output_path: (mode != JobMode::Verify)
            .then(|| PathBuf::from("C:/data/output/folder").join(name)),
        handle,
        telemetry,
        started: Instant::now(),
        origin: Origin::Extract(Box::new(extract_setup("origin.nxar", true))),
        job_id: 1,
    }
}

fn finished(mode: JobMode, original: u64, archive: u64, name: &str) -> FinishedJob {
    let mut path_bytes = [0u64; PATH_COUNT];
    path_bytes[5] = original / 2;
    path_bytes[1] = original / 4;
    path_bytes[2] = original / 8;
    FinishedJob {
        mode,
        source_name: name.to_string(),
        location: PathBuf::from("C:/data/output/folder").join(name),
        result: JobSuccess {
            original_bytes: original,
            archive_bytes: archive,
            elapsed: Duration::from_millis(83_400),
        },
        path_bytes,
        origin: Origin::Extract(Box::new(extract_setup("origin.nxar", true))),
    }
}

#[test]
fn test_default_size_matches_main() {
    assert_eq!(DEFAULT_SIZE, crate::WINDOW_SIZE);
    assert_eq!(SIZES[0], crate::WINDOW_MIN_SIZE);
}

#[test]
fn test_home_layout() {
    check(
        "home",
        Fit::NoScroll,
        |_| Screen::Home(HomeState::default()),
    );
    check("home with notice", Fit::NoScroll, |_| {
        Screen::Home(HomeState {
            notice: Some(long_notice()),
        })
    });
}

#[test]
fn test_compress_setup_layout_at_every_level() {
    for level in 1..=19 {
        // Levels 13+ add a warning and may scroll; the action row is pinned.
        let fit = if level < 13 {
            Fit::NoScroll
        } else {
            Fit::MayScroll
        };
        check(&format!("compress setup L{level}"), fit, |ctx| {
            let mut setup = compress_setup(ctx, "report.pdf", level, 3 * 1024 * MB);
            setup
                .estimates
                .insert(level, estimate(3 * 1024 * MB, false));
            Screen::CompressSetup(Box::new(setup))
        });
    }
}

#[test]
fn test_compress_setup_layout_edge_states() {
    check("compress setup: still scanning", Fit::NoScroll, |ctx| {
        let input = PathBuf::from("C:/data").join(long_name());
        let mut setup = CompressSetup::new(input, None, false, ctx);
        setup.output_path = "C:/out/sample.nxar".to_string();
        // Hide the background scan's result so the pending state is shown.
        let (_tx, rx) = std::sync::mpsc::channel();
        setup.scan = ScanState::Pending(rx);
        Screen::CompressSetup(Box::new(setup))
    });
    check(
        "compress setup: estimate pending, long name",
        Fit::NoScroll,
        |ctx| {
            let setup = compress_setup(ctx, &long_name(), 12, 40 * 1024 * MB);
            Screen::CompressSetup(Box::new(setup))
        },
    );
    check(
        "compress setup: incompressible at L19 with notice",
        Fit::MayScroll,
        |ctx| {
            let mut setup = compress_setup(ctx, &long_name(), 19, 900 * 1024 * MB);
            setup.notice = Some(long_notice());
            setup.estimates.insert(19, estimate(900 * 1024 * MB, true));
            Screen::CompressSetup(Box::new(setup))
        },
    );
    check("compress setup: empty source", Fit::NoScroll, |ctx| {
        let mut setup = compress_setup(ctx, "empty.bin", 6, 0);
        setup.estimates.insert(6, estimate(0, false));
        Screen::CompressSetup(Box::new(setup))
    });
    check("compress setup: unreadable source", Fit::NoScroll, |ctx| {
        let mut setup = compress_setup(ctx, "gone.bin", 6, 0);
        setup.scan = ScanState::Ready(SourceScan {
            sample: Arc::new(Vec::new()),
            total_bytes: 0,
            error: Some("The system cannot find the file specified. (os error 2)".into()),
        });
        Screen::CompressSetup(Box::new(setup))
    });
}

#[test]
fn test_extract_setup_layout() {
    check("extract setup", Fit::NoScroll, |_| {
        Screen::ExtractSetup(Box::new(extract_setup("backup.nxar", true)))
    });
    check(
        "extract setup: unreadable archive, long name",
        Fit::NoScroll,
        |_| {
            let mut setup = extract_setup(&long_name(), false);
            setup.notice = Some(long_notice());
            Screen::ExtractSetup(Box::new(setup))
        },
    );
    check("extract setup: verified notice", Fit::NoScroll, |_| {
        let mut setup = extract_setup("backup.nxar", true);
        setup.notice = Some(Notice::new(
            NoticeKind::Success,
            "This archive passed verification: every checksum matches.",
        ));
        Screen::ExtractSetup(Box::new(setup))
    });
}

#[test]
fn test_running_layout_all_modes_and_stages() {
    for mode in [JobMode::Compress, JobMode::Extract, JobMode::Verify] {
        check(
            &format!("running {mode:?}: no progress yet"),
            Fit::NoScroll,
            |_| Screen::Running(Box::new(running(mode, 0, 64, "dataset.tar"))),
        );
        check(&format!("running {mode:?}: midway"), Fit::NoScroll, |_| {
            Screen::Running(Box::new(running(mode, 23, 64, "dataset.tar")))
        });
        check(
            &format!("running {mode:?}: long name"),
            Fit::NoScroll,
            |_| Screen::Running(Box::new(running(mode, 5, 9, &long_name()))),
        );
    }
}

#[test]
fn test_running_layout_extremes() {
    check("running: single block", Fit::NoScroll, |_| {
        Screen::Running(Box::new(running(JobMode::Compress, 0, 1, "tiny.txt")))
    });
    check(
        "running: tens of thousands of blocks",
        Fit::NoScroll,
        |_| {
            let job = running(JobMode::Compress, 12_345, 50_000, "huge.img");
            Screen::Running(Box::new(job))
        },
    );
    check("running: cancelling", Fit::NoScroll, |_| {
        let job = running(JobMode::Extract, 10, 64, "dataset.tar");
        job.handle.cancel();
        Screen::Running(Box::new(job))
    });
    check(
        "running: complete, waiting for the worker",
        Fit::NoScroll,
        |_| {
            let mut job = running(JobMode::Compress, 64, 64, "dataset.tar");
            let total = job.telemetry.total_bytes();
            job.telemetry.record(
                &ProgressUpdate {
                    bytes_processed: total,
                    total_bytes: total,
                    compressed_bytes: total / 3,
                    path_bytes: *job.telemetry.path_bytes(),
                    current_target: String::new(),
                    stage: COMPLETE_STAGE,
                    fraction: 1.0,
                },
                20.0,
            );
            Screen::Running(Box::new(job))
        },
    );
    check("running: empty input", Fit::NoScroll, |_| {
        Screen::Running(Box::new(running(JobMode::Compress, 0, 0, "empty.bin")))
    });
}

#[test]
fn test_finished_layout() {
    for mode in [JobMode::Compress, JobMode::Extract, JobMode::Verify] {
        check(&format!("finished {mode:?}"), Fit::NoScroll, |_| {
            let job = finished(mode, 2048 * MB, 611 * MB, "dataset.tar");
            Screen::Finished(Box::new(job))
        });
        check(
            &format!("finished {mode:?}: long name"),
            Fit::NoScroll,
            |_| {
                let job = finished(mode, 2048 * MB, 611 * MB, &long_name());
                Screen::Finished(Box::new(job))
            },
        );
        check(&format!("finished {mode:?}: empty"), Fit::NoScroll, |_| {
            Screen::Finished(Box::new(finished(mode, 0, 64, "empty.bin")))
        });
        check(
            &format!("finished {mode:?}: no savings"),
            Fit::NoScroll,
            |_| {
                let job = finished(mode, 100 * MB, 101 * MB, "random.bin");
                Screen::Finished(Box::new(job))
            },
        );
    }
}

#[test]
fn test_failed_layout() {
    check("failed", Fit::NoScroll, |_| {
        Screen::Failed(Box::new(FailedJob {
            title: "ARCHIVE FAILED VERIFICATION".to_string(),
            message: format!(
                "Verification failed: Block decompression failed at offset 123456789: {}",
                "corrupt token stream ".repeat(30)
            ),
            hint: Some("The archive is damaged or incomplete; try to obtain a fresh copy.".into()),
            origin: Some(Origin::Extract(Box::new(extract_setup("a.nxar", true)))),
        }))
    });
    check("failed without origin", Fit::NoScroll, |_| {
        Screen::Failed(Box::new(FailedJob {
            title: "COMPRESSION FAILED".to_string(),
            message: "Compression failed: I/O error: Access is denied. (os error 5)".to_string(),
            hint: None,
            origin: None,
        }))
    });
}

#[test]
fn test_drop_overlay_layout() {
    let name = long_name();
    let cases: [(Option<PathBuf>, usize, bool); 5] = [
        (Some(PathBuf::from("no_such_dir/photo.png")), 0, false),
        (Some(PathBuf::from("no_such_dir/backup.nxar")), 0, false),
        (Some(PathBuf::from("no_such_dir").join(&name)), 3, false),
        (None, 0, false),
        (Some(PathBuf::from("no_such_dir/photo.png")), 0, true),
    ];
    // Texts that only the overlay paints. The home screen underneath is
    // hidden by the scrim (and has its own test), so it is filtered out.
    let overlay_texts = [
        "DROP TO",
        "A JOB IS RUNNING",
        "photo.png",
        "backup.nxar",
        "Cancel it",
        "a_very",
    ];
    for (path, extra, busy) in cases {
        for size in SIZES {
            let layout = lay_out(size, |ctx| {
                let mut screen = Screen::Home(HomeState::default());
                let areas = show_measured(ctx, &mut screen);
                drop_overlay::show(ctx, path.as_deref(), extra, busy);
                areas
            });
            let overlay = Layout {
                texts: layout
                    .texts
                    .into_iter()
                    .filter(|t| overlay_texts.iter().any(|needle| t.text.contains(needle)))
                    .collect(),
                ..layout
            };
            assert_sound(&overlay, &format!("drop overlay {path:?} busy={busy}"));
            // Headline plus, when a path is known, the detail line.
            let expected = if path.is_some() { 2 } else { 1 };
            assert_eq!(overlay.texts.len(), expected, "{path:?} busy={busy}");
        }
    }
}

#[test]
fn test_block_map_fits_its_panel_for_any_block_count() {
    use crate::methods::Method;
    use crate::widgets::charts::block_map;
    for count in [0usize, 1, 2, 63, 64, 65, 500, 4096, 1_000_000] {
        for size in [vec2(600.0, 500.0), vec2(1400.0, 900.0)] {
            let done: Vec<Method> = Method::ALL.into_iter().cycle().take(count / 2).collect();
            let layout = lay_out(size, |ctx| {
                vec![main_panel(ctx, "block_map_test", |ui| {
                    ui.label("block map");
                    block_map(ui, &done, Some(count), 54.0, true);
                    (ui.min_rect(), ui.max_rect())
                })]
            });
            let (used, given) = layout.body();
            assert!(
                used.width() <= given.width() + 0.5,
                "block map with {count} cells overflows at {size:?}"
            );
        }
    }
}
