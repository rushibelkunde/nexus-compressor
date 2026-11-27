//! The application state machine.
//!
//! [`NexusGuiApp`] owns the current [`Screen`], routes dropped files, pumps
//! background results into screen state once per frame, lets the active
//! screen paint itself and applies the [`Action`] it returns.
//!
//! Repaint policy: while a job runs the view is repainted at ~30 fps so the
//! gauges animate; in every other state the app only repaints on input or
//! when a background thread asks for it.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{self, Margin};

use crate::cli_args::InitialAction;
use crate::engine::{clamp_level, superblock_bytes};
use crate::paths::{display_name, route_for_existing, Route};
use crate::screens::compress_setup::CompressSetup;
use crate::screens::drop_overlay;
use crate::screens::extract_setup::ExtractSetup;
use crate::screens::failed::FailedJob;
use crate::screens::finished::FinishedJob;
use crate::screens::home::{self, HomeState};
use crate::screens::running::RunningJob;
use crate::screens::{Action, JobMode, Origin};
use crate::telemetry::{CellPlan, JobTelemetry};
use crate::theme::{self, BG_DARK};
use crate::widgets::{brand_header, Notice, NoticeKind};
use crate::worker::{spawn_job, JobOutcome, JobSpec};

/// Interval between repaints while a job is running (~30 fps).
const RUNNING_REPAINT: Duration = Duration::from_millis(33);

/// The screen currently shown.
pub enum Screen {
    /// Drop zone and pickers.
    Home(HomeState),
    /// Compression setup.
    CompressSetup(Box<CompressSetup>),
    /// Extraction setup.
    ExtractSetup(Box<ExtractSetup>),
    /// A job is running.
    Running(Box<RunningJob>),
    /// A job finished successfully.
    Finished(Box<FinishedJob>),
    /// A job failed.
    Failed(Box<FailedJob>),
}

/// The NEXUS Archiver application.
pub struct NexusGuiApp {
    screen: Screen,
    next_job_id: u64,
}

/// Message shown on the setup screen after a job was cancelled.
pub fn cancel_notice(mode: JobMode, output: Option<&Path>) -> Notice {
    let text = match mode {
        JobMode::Compress => "Compression cancelled. No archive was written.".to_string(),
        JobMode::Extract => match output {
            Some(path) => format!(
                "Extraction cancelled. Partially extracted data may remain at {}.",
                path.display()
            ),
            None => "Extraction cancelled.".to_string(),
        },
        JobMode::Verify => "Verification cancelled. Nothing was written.".to_string(),
    };
    Notice::new(NoticeKind::Info, text)
}

/// Heading and plain-language hint of the error screen for a failed job.
pub fn failure_text(mode: JobMode) -> (&'static str, Option<&'static str>) {
    match mode {
        JobMode::Compress => ("COMPRESSION FAILED", None),
        JobMode::Extract => (
            "EXTRACTION FAILED",
            Some(
                "Files extracted before the error may be incomplete. Use \"Test archive\" \
                 to check whether the archive itself is damaged.",
            ),
        ),
        JobMode::Verify => (
            "ARCHIVE FAILED VERIFICATION",
            Some(
                "The archive could not be decoded completely or a checksum did not match. \
                 It is damaged or incomplete; try to obtain a fresh copy.",
            ),
        ),
    }
}

/// Message shown when more than one item was dropped at once.
pub fn multi_drop_notice(extra_items: usize) -> Option<Notice> {
    if extra_items == 0 {
        return None;
    }
    Some(Notice::new(
        NoticeKind::Info,
        format!(
            "{} items were dropped; only the first one is used. To pack several items into \
             one archive, put them in a folder and drop the folder.",
            extra_items + 1
        ),
    ))
}

/// Paints the window frame shared by every screen — background, brand
/// header and a vertical scroll area — and runs `add_contents` inside it.
/// The scroll area is a safety net: at the minimum window size the taller
/// screens scroll instead of being clipped.
///
/// `scroll_id` keys the scroll position, so each screen keeps its own and a
/// newly opened screen always starts at the top.
pub fn main_panel<R>(
    ctx: &egui::Context,
    scroll_id: &str,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(BG_DARK)
                .inner_margin(Margin::symmetric(16, 10)),
        )
        .show(ctx, |ui| {
            brand_header(ui);
            ui.add_space(4.0);
            egui::ScrollArea::vertical()
                .id_salt(scroll_id)
                .auto_shrink([false, false])
                .show(ui, add_contents)
                .inner
        })
        .inner
}

/// Paints the action row of a screen in a panel pinned to the bottom of
/// the window, so the primary action (start, cancel, open folder) never
/// scrolls out of reach. Must be called before [`main_panel`].
pub fn footer_panel<R>(ctx: &egui::Context, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::TopBottomPanel::bottom("nexus_footer")
        .resizable(false)
        .show_separator_line(true)
        .frame(egui::Frame::new().fill(BG_DARK).inner_margin(Margin {
            left: 16,
            right: 16,
            top: 8,
            bottom: 10,
        }))
        .show(ctx, add_contents)
        .inner
}

impl Screen {
    /// Paints the scrolling body of the screen.
    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        match self {
            Screen::Home(state) => home::show(state, ui),
            Screen::CompressSetup(setup) => setup.show(ui),
            Screen::ExtractSetup(setup) => setup.show(ui),
            Screen::Running(job) => job.show(ui),
            Screen::Finished(finished) => finished.show(ui),
            Screen::Failed(failed) => failed.show(ui),
        }
    }

    /// Short stable name of the screen kind (keys per-screen UI state).
    pub fn tag(&self) -> &'static str {
        match self {
            Screen::Home(_) => "home",
            Screen::CompressSetup(_) => "compress_setup",
            Screen::ExtractSetup(_) => "extract_setup",
            Screen::Running(_) => "running",
            Screen::Finished(_) => "finished",
            Screen::Failed(_) => "failed",
        }
    }

    /// Whether the screen has a pinned action row.
    pub fn has_footer(&self) -> bool {
        !matches!(self, Screen::Home(_))
    }

    /// Paints the pinned action row of the screen.
    pub fn footer(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        match self {
            Screen::Home(_) => None,
            Screen::CompressSetup(setup) => setup.footer(ui),
            Screen::ExtractSetup(setup) => setup.footer(ui),
            Screen::Running(job) => job.footer(ui),
            Screen::Finished(finished) => finished.footer(ui),
            Screen::Failed(failed) => failed.footer(ui),
        }
    }
}

/// Paints one frame of `screen` — pinned action row first, then the body —
/// and returns the action the user triggered, if any.
pub fn show_screen(ctx: &egui::Context, screen: &mut Screen) -> Option<Action> {
    let footer_action = if screen.has_footer() {
        footer_panel(ctx, |ui| screen.footer(ui))
    } else {
        None
    };
    let body_action = main_panel(ctx, screen.tag(), |ui| screen.show(ui));
    footer_action.or(body_action)
}

impl NexusGuiApp {
    /// Creates the application and performs the command-line action, if any.
    pub fn new(cc: &eframe::CreationContext<'_>, initial: Option<InitialAction>) -> Self {
        Self::with_context(&cc.egui_ctx, initial)
    }

    /// Creates the application on an existing egui context. This is all of
    /// [`NexusGuiApp::new`] that does not need a native window.
    pub fn with_context(ctx: &egui::Context, initial: Option<InitialAction>) -> Self {
        theme::apply(ctx);
        let mut app = Self {
            screen: Screen::Home(HomeState::default()),
            next_job_id: 0,
        };
        app.perform_initial(initial, ctx);
        app
    }

    /// Runs one frame: routes dropped files, collects background results,
    /// paints the current screen and applies the action it returned.
    pub fn frame(&mut self, ctx: &egui::Context) {
        self.handle_dropped_files(ctx);
        self.pump(ctx);

        let action = show_screen(ctx, &mut self.screen);

        if let Some(action) = action {
            self.apply(action, ctx);
        }

        // Overlay while files hover the window.
        let hovered: Option<(Option<PathBuf>, usize)> = ctx.input(|i| {
            i.raw
                .hovered_files
                .first()
                .map(|file| (file.path.clone(), i.raw.hovered_files.len() - 1))
        });
        if let Some((path, extra_items)) = hovered {
            drop_overlay::show(ctx, path.as_deref(), extra_items, self.is_running());
        }

        if self.is_running() {
            ctx.request_repaint_after(RUNNING_REPAINT);
        }
    }

    fn perform_initial(&mut self, initial: Option<InitialAction>, ctx: &egui::Context) {
        match initial {
            Some(InitialAction::Compress {
                path,
                preset,
                auto_start,
            }) => {
                // With auto-start the job begins as soon as the background
                // scan has reported the source size (see `pump`).
                self.screen = Screen::CompressSetup(Box::new(CompressSetup::new(
                    path, preset, auto_start, ctx,
                )));
            }
            Some(InitialAction::Decompress { path, auto_start }) => {
                let setup = Box::new(ExtractSetup::new(path));
                if auto_start {
                    self.start_extract(setup, JobMode::Extract, ctx);
                } else {
                    self.screen = Screen::ExtractSetup(setup);
                }
            }
            None => {}
        }
    }

    fn take_screen(&mut self) -> Screen {
        std::mem::replace(&mut self.screen, Screen::Home(HomeState::default()))
    }

    fn next_job_id(&mut self) -> u64 {
        self.next_job_id += 1;
        self.next_job_id
    }

    fn is_running(&self) -> bool {
        matches!(self.screen, Screen::Running(_))
    }

    /// Opens the right setup screen for a path the user dropped.
    fn open_path(&mut self, path: PathBuf, notice: Option<Notice>, ctx: &egui::Context) {
        match route_for_existing(&path) {
            Route::Compress => {
                let mut setup = CompressSetup::new(path, None, false, ctx);
                setup.notice = notice;
                self.screen = Screen::CompressSetup(Box::new(setup));
            }
            Route::Extract => {
                let mut setup = ExtractSetup::new(path);
                setup.notice = notice;
                self.screen = Screen::ExtractSetup(Box::new(setup));
            }
        }
    }

    fn start_compress(&mut self, mut setup: Box<CompressSetup>, ctx: &egui::Context) {
        // Never auto-start again when this setup is restored after a cancel.
        setup.auto_start = false;
        setup.notice = None;
        let level = clamp_level(setup.level);
        let output = setup.output();
        let total = setup.total_bytes().unwrap_or(0);
        let handle = spawn_job(
            JobSpec::Compress {
                input: setup.input_path.clone(),
                output: output.clone(),
                level,
            },
            ctx.clone(),
        );
        let job_id = self.next_job_id();
        self.screen = Screen::Running(Box::new(RunningJob {
            mode: JobMode::Compress,
            source_name: display_name(&setup.input_path),
            level: Some(level),
            archive_bytes: None,
            output_path: Some(output),
            handle,
            telemetry: JobTelemetry::new(
                total,
                CellPlan::BySuperblockSize(superblock_bytes(level)),
            ),
            started: Instant::now(),
            origin: Origin::Compress(setup),
            job_id,
        }));
    }

    // Starts extraction or verification (`mode`) of the setup's archive.
    fn start_extract(&mut self, mut setup: Box<ExtractSetup>, mode: JobMode, ctx: &egui::Context) {
        setup.notice = None;
        let input = setup.input_path.clone();
        let (spec, output_path) = if mode == JobMode::Verify {
            (JobSpec::Verify { input }, None)
        } else {
            let output = setup.output();
            (
                JobSpec::Extract {
                    input,
                    output: output.clone(),
                },
                Some(output),
            )
        };
        let (total, plan) = match &setup.summary {
            Ok(summary) => (
                summary.original_bytes,
                CellPlan::Fixed(summary.superblock_count as usize),
            ),
            Err(_) => (0, CellPlan::Unknown),
        };
        let handle = spawn_job(spec, ctx.clone());
        let job_id = self.next_job_id();
        self.screen = Screen::Running(Box::new(RunningJob {
            mode,
            source_name: display_name(&setup.input_path),
            level: None,
            archive_bytes: Some(setup.archive_bytes),
            output_path,
            handle,
            telemetry: JobTelemetry::new(total, plan),
            started: Instant::now(),
            origin: Origin::Extract(setup),
            job_id,
        }));
    }

    fn restore_origin(&mut self, origin: Origin, notice: Option<Notice>) {
        self.screen = match origin {
            Origin::Compress(mut setup) => {
                setup.notice = notice;
                Screen::CompressSetup(setup)
            }
            Origin::Extract(mut setup) => {
                setup.notice = notice;
                Screen::ExtractSetup(setup)
            }
        };
    }

    // Turns the outcome of the running job into the next screen.
    fn finish_job(&mut self, job: Box<RunningJob>, outcome: JobOutcome) {
        let job = *job;
        match outcome {
            JobOutcome::Success(result) => {
                let location = match (&job.output_path, &job.origin) {
                    (Some(output), _) => output.clone(),
                    (None, Origin::Extract(setup)) => setup.input_path.clone(),
                    (None, Origin::Compress(setup)) => setup.input_path.clone(),
                };
                self.screen = Screen::Finished(Box::new(FinishedJob {
                    mode: job.mode,
                    source_name: job.source_name,
                    location,
                    result,
                    path_bytes: *job.telemetry.path_bytes(),
                    origin: job.origin,
                }));
            }
            JobOutcome::Cancelled => {
                let notice = cancel_notice(job.mode, job.output_path.as_deref());
                self.restore_origin(job.origin, Some(notice));
            }
            JobOutcome::Failed(message) => {
                let (title, hint) = failure_text(job.mode);
                self.screen = Screen::Failed(Box::new(FailedJob {
                    title: title.to_string(),
                    message,
                    hint: hint.map(str::to_string),
                    origin: Some(job.origin),
                }));
            }
        }
    }

    // Routes files dropped onto the window. Drops are ignored while a job runs.
    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|file| file.path.clone())
                .collect()
        });
        if self.is_running() {
            return;
        }
        let extra_items = dropped.len().saturating_sub(1);
        if let Some(first) = dropped.into_iter().next() {
            self.open_path(first, multi_drop_notice(extra_items), ctx);
        }
    }

    // Moves background results into screen state and performs the
    // transitions they trigger.
    fn pump(&mut self, ctx: &egui::Context) {
        let mut auto_start = false;
        let mut outcome = None;
        match &mut self.screen {
            Screen::CompressSetup(setup) => {
                setup.poll(ctx);
                auto_start = setup.auto_start && setup.can_start();
            }
            Screen::Running(job) => outcome = job.pump(),
            _ => {}
        }

        if auto_start {
            self.apply(Action::StartCompress, ctx);
        }
        if let Some(outcome) = outcome {
            match self.take_screen() {
                Screen::Running(job) => self.finish_job(job, outcome),
                other => self.screen = other,
            }
        }
    }

    fn apply(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::GoHome => self.screen = Screen::Home(HomeState::default()),
            Action::OpenSource(path) => {
                self.screen =
                    Screen::CompressSetup(Box::new(CompressSetup::new(path, None, false, ctx)));
            }
            Action::OpenArchive(path) => {
                self.screen = Screen::ExtractSetup(Box::new(ExtractSetup::new(path)));
            }
            Action::StartCompress => match self.take_screen() {
                Screen::CompressSetup(setup) => self.start_compress(setup, ctx),
                other => self.screen = other,
            },
            Action::StartExtract => match self.take_screen() {
                Screen::ExtractSetup(setup) => self.start_extract(setup, JobMode::Extract, ctx),
                other => self.screen = other,
            },
            Action::StartVerify => match self.take_screen() {
                Screen::ExtractSetup(setup) => self.start_extract(setup, JobMode::Verify, ctx),
                other => self.screen = other,
            },
            Action::CancelJob => {
                if let Screen::Running(job) = &self.screen {
                    job.handle.cancel();
                }
            }
            Action::BackToSetup => match self.take_screen() {
                Screen::Finished(finished) => {
                    let notice = (finished.mode == JobMode::Verify).then(|| {
                        Notice::new(
                            NoticeKind::Success,
                            "This archive passed verification: every checksum matches.",
                        )
                    });
                    self.restore_origin(finished.origin, notice);
                }
                Screen::Failed(failed) => match failed.origin {
                    Some(origin) => self.restore_origin(origin, None),
                    None => self.screen = Screen::Home(HomeState::default()),
                },
                other => self.screen = other,
            },
            Action::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Action::QuitAfterFailure => std::process::exit(1),
        }
        // Show the result of the transition without waiting for input.
        ctx.request_repaint();
    }
}

impl eframe::App for NexusGuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame(ctx);
    }
}

#[cfg(test)]
mod flow_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cancel_notice_per_mode() {
        let compress = cancel_notice(JobMode::Compress, Some(Path::new("out.nxar")));
        assert_eq!(compress.kind, NoticeKind::Info);
        assert!(compress.text.contains("cancelled"));
        assert!(compress.text.contains("No archive"));

        let extract = cancel_notice(JobMode::Extract, Some(Path::new("restored_dir")));
        assert!(extract.text.contains("restored_dir"));
        assert!(extract.text.contains("may remain"));
        assert!(cancel_notice(JobMode::Extract, None)
            .text
            .contains("cancelled"));

        let verify = cancel_notice(JobMode::Verify, None);
        assert!(verify.text.contains("Nothing was written"));
    }

    #[test]
    fn test_failure_text_per_mode() {
        assert_eq!(failure_text(JobMode::Compress).0, "COMPRESSION FAILED");
        assert_eq!(failure_text(JobMode::Extract).0, "EXTRACTION FAILED");
        let (title, hint) = failure_text(JobMode::Verify);
        assert!(title.contains("VERIFICATION"));
        assert!(hint.is_some_and(|h| h.contains("damaged")));
    }

    #[test]
    fn test_multi_drop_notice_only_for_several_items() {
        assert_eq!(multi_drop_notice(0), None);
        let notice = multi_drop_notice(2).unwrap();
        assert!(notice.text.starts_with("3 items"));
    }
}
