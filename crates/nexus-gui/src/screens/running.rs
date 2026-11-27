//! The live view of a running job — the centrepiece of the GUI.
//!
//! From top to bottom: the press gauge (bytes in above, bytes out below, to
//! one scale), the overall progress gauge with elapsed time and ETA, the
//! throughput sparkline next to the block map, the method breakdown, and the
//! cancel button.

use std::path::PathBuf;
use std::sync::mpsc::TryRecvError;
use std::time::Instant;

use eframe::egui::{self, Id, RichText, TextWrapMode};

use super::{Action, JobMode, Origin, RAW_DATA_COLOR};
use crate::engine::{engine_for_level, superblock_bytes};
use crate::methods::method_totals;
use crate::telemetry::JobTelemetry;
use crate::theme::{mono, sans, TEXT, TEXT_DIM, TEXT_MUTED};
use crate::util::{
    bytes_saved, format_bytes, format_clock, format_clock_opt, format_ratio, format_speed,
};
use crate::widgets::charts::{block_map, method_bar, method_legend, progress_gauge, sparkline};
use crate::widgets::icons;
use crate::widgets::press::{press, PressBar, PressView};
use crate::widgets::{danger_button, elided_label, fit_middle, panel, section_label};
use crate::worker::{JobHandle, JobOutcome};

/// Height of the sparkline and of the block map, which sit side by side.
const INSTRUMENT_HEIGHT: f32 = 54.0;

/// State of a running job.
pub struct RunningJob {
    /// What kind of job this is.
    pub mode: JobMode,
    /// Display name of the source (file, folder or archive name).
    pub source_name: String,
    /// Compression level, for compression jobs.
    pub level: Option<u32>,
    /// Size of the archive on disk, for extraction and verification.
    pub archive_bytes: Option<u64>,
    /// Where the job writes to, if it writes anything.
    pub output_path: Option<PathBuf>,
    /// Channels and cancel flag of the worker thread.
    pub handle: JobHandle,
    /// Live statistics.
    pub telemetry: JobTelemetry,
    /// When the job was started.
    pub started: Instant,
    /// The setup screen to return to on cancel.
    pub origin: Origin,
    /// Unique per job; keeps animation state of one job from leaking into
    /// the next.
    pub job_id: u64,
}

impl RunningJob {
    /// Seconds since the job started.
    pub fn now(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    /// Drains the worker's channels into the telemetry. Returns the outcome
    /// once the job has ended. Call once per frame.
    pub fn pump(&mut self) -> Option<JobOutcome> {
        // Look for the outcome first: every progress update was sent before
        // it, so draining afterwards cannot miss the final ones.
        let outcome = match self.handle.outcome_rx.try_recv() {
            Ok(outcome) => Some(outcome),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(JobOutcome::Failed(
                "The worker thread stopped unexpectedly.".to_string(),
            )),
        };
        let now = self.now();
        while let Ok(update) = self.handle.progress_rx.try_recv() {
            self.telemetry.record(&update, now);
        }
        self.telemetry.tick(now);
        outcome
    }

    /// Paints the screen.
    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let now = self.now();
        let anim = Id::new(("nexus_job", self.job_id));

        self.heading(ui);
        ui.add_space(2.0);

        panel(ui, |ui| press(ui, &self.press_view(anim)));
        ui.add_space(2.0);

        panel(ui, |ui| self.progress_section(ui, anim, now));
        ui.add_space(2.0);

        ui.columns(2, |columns| {
            panel(&mut columns[0], |ui| self.throughput_section(ui, now));
            panel(&mut columns[1], |ui| self.block_map_section(ui));
        });
        ui.add_space(2.0);

        panel(ui, |ui| {
            section_label(ui, "Methods (share of data)");
            let totals = method_totals(self.telemetry.path_bytes());
            method_bar(ui, &totals, 12.0);
            method_legend(ui, &totals);
        });
        None
    }

    /// Paints the action row pinned to the bottom of the window, so the
    /// cancel button is reachable without scrolling.
    pub fn footer(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        let cancelling = self.handle.is_cancel_requested();
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = if cancelling {
                    "CANCELLING\u{2026}"
                } else {
                    "CANCEL"
                };
                if danger_button(ui, !cancelling, label).clicked() {
                    action = Some(Action::CancelJob);
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    if cancelling {
                        ui.add(
                            egui::Label::new(
                                RichText::new("Stopping at the next block boundary\u{2026}")
                                    .font(sans(12.0))
                                    .color(TEXT_MUTED),
                            )
                            .truncate(),
                        );
                    } else if let Some(output) = &self.output_path {
                        ui.label(RichText::new("TO").font(mono(10.5)).color(TEXT_DIM));
                        elided_label(ui, &output.display().to_string(), mono(11.0), TEXT_MUTED);
                    }
                });
            });
        });
        action
    }

    fn heading(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            icons::icon(ui, self.mode.icon(), 20.0, self.mode.accent());
            ui.label(RichText::new(self.mode.verb()).font(mono(14.0)).color(TEXT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(level) = self.level {
                    ui.label(
                        RichText::new(format!(
                            "L{level} \u{00B7} {}",
                            engine_for_level(level).name
                        ))
                        .font(mono(11.0))
                        .color(TEXT_MUTED),
                    );
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    elided_label(ui, &self.source_name, sans(14.0), TEXT_MUTED);
                });
            });
        });
    }

    fn press_view(&self, anim: Id) -> PressView<'static> {
        let t = &self.telemetry;
        let accent = self.mode.accent();
        let total = t.total_bytes();
        let raw_detail = if total > 0 {
            format!(
                "{} / {}",
                format_bytes(t.bytes_processed()),
                format_bytes(total)
            )
        } else {
            format_bytes(t.bytes_processed())
        };

        match self.mode {
            JobMode::Compress => PressView {
                top: PressBar {
                    label: "Original",
                    bytes: t.bytes_processed(),
                    detail: raw_detail,
                    color: RAW_DATA_COLOR,
                },
                bottom: PressBar {
                    label: "Compressed",
                    bytes: t.compressed_bytes(),
                    detail: format_bytes(t.compressed_bytes()),
                    color: accent,
                },
                scale_bytes: total,
                caption: if t.bytes_processed() > 0 {
                    format!(
                        "{} of original \u{00B7} {} saved",
                        format_ratio(t.compressed_bytes(), t.bytes_processed()),
                        format_bytes(bytes_saved(t.bytes_processed(), t.compressed_bytes()))
                    )
                } else {
                    "pressing the first block\u{2026}".to_string()
                },
                live: Some(anim),
            },
            JobMode::Extract | JobMode::Verify => {
                let archive_detail = match self.archive_bytes {
                    Some(size) if size > 0 => format!(
                        "{} / {}",
                        format_bytes(t.compressed_bytes()),
                        format_bytes(size)
                    ),
                    _ => format_bytes(t.compressed_bytes()),
                };
                let restored_label = if self.mode == JobMode::Verify {
                    "Verified"
                } else {
                    "Restored"
                };
                PressView {
                    top: PressBar {
                        label: "Archive read",
                        bytes: t.compressed_bytes(),
                        detail: archive_detail,
                        color: accent,
                    },
                    bottom: PressBar {
                        label: restored_label,
                        bytes: t.bytes_processed(),
                        detail: raw_detail,
                        color: RAW_DATA_COLOR,
                    },
                    scale_bytes: total,
                    caption: if t.bytes_processed() > 0 {
                        format!(
                            "archive is {} of the restored size",
                            format_ratio(t.compressed_bytes(), t.bytes_processed()),
                        )
                    } else {
                        "decoding the first block\u{2026}".to_string()
                    },
                    live: Some(anim),
                }
            }
        }
    }

    fn progress_section(&self, ui: &mut egui::Ui, anim: Id, now: f64) {
        let t = &self.telemetry;
        let accent = self.mode.accent();
        progress_gauge(
            ui,
            t.fraction(),
            (accent, crate::theme::lerp_color(accent, TEXT, 0.35)),
            Some(anim),
        );
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("{:.1} %", t.fraction() * 100.0))
                    .font(mono(20.0))
                    .color(TEXT),
            );
            ui.label(RichText::new(t.stage()).font(sans(12.0)).color(TEXT_MUTED));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let clock = |ui: &mut egui::Ui, label: &str, value: String| {
                    ui.label(RichText::new(value).font(mono(14.0)).color(TEXT));
                    ui.label(RichText::new(label).font(mono(10.5)).color(TEXT_DIM));
                };
                clock(ui, "ETA", format_clock_opt(t.eta_secs(now)));
                ui.add_space(10.0);
                clock(ui, "ELAPSED", format_clock(self.started.elapsed()));
            });
        });
    }

    fn throughput_section(&self, ui: &mut egui::Ui, now: f64) {
        let t = &self.telemetry;
        section_label(ui, "Throughput");
        ui.horizontal(|ui| {
            ui.style_mut().wrap_mode = Some(TextWrapMode::Truncate);
            let current = match t.current_mb_s() {
                Some(speed) => format_speed(speed),
                None => "-- MB/s".to_string(),
            };
            ui.label(RichText::new(current).font(mono(15.0)).color(TEXT));
            ui.label(
                RichText::new(format!("avg {}", format_speed(t.average_mb_s(now))))
                    .font(mono(11.0))
                    .color(TEXT_MUTED),
            );
        });
        sparkline(
            ui,
            &t.speed().to_vec(),
            INSTRUMENT_HEIGHT,
            self.mode.accent(),
        );
    }

    fn block_map_section(&self, ui: &mut egui::Ui) {
        let t = &self.telemetry;
        section_label(ui, "Block map");
        ui.horizontal(|ui| {
            ui.style_mut().wrap_mode = Some(TextWrapMode::Truncate);
            let done = t.cells().len();
            let count = match t.expected_cells() {
                Some(expected) => format!("{done} / {expected}"),
                None => done.to_string(),
            };
            ui.label(RichText::new(count).font(mono(15.0)).color(TEXT));
            let unit = match self.level {
                Some(level) => format!(
                    "superblocks \u{00B7} {} each",
                    format_bytes(superblock_bytes(level))
                ),
                None => "superblocks".to_string(),
            };
            let font = mono(11.0);
            let fitted = fit_middle(ui, &unit, &font, ui.available_width());
            ui.label(RichText::new(fitted).font(font).color(TEXT_MUTED));
        });
        block_map(
            ui,
            t.cells(),
            t.expected_cells(),
            INSTRUMENT_HEIGHT,
            !t.is_complete(),
        );
    }
}
