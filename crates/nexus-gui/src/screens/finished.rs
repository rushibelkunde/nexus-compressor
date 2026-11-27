//! Result screen of a job that ran to completion: before/after bars, ratio,
//! bytes saved, average speed and elapsed time — or, for a verification,
//! the "archive is intact" verdict.

use std::path::PathBuf;

use eframe::egui::{self, RichText};

use super::{Action, JobMode, Origin, RAW_DATA_COLOR};
use crate::methods::{method_totals, PathBytes};
use crate::shell::open_in_explorer;
use crate::theme::{mono, sans, SUCCESS, TEXT, TEXT_DIM, TEXT_MUTED};
use crate::util::{
    bytes_saved, format_bytes, format_elapsed, format_ratio, format_speed, mb_per_sec,
};
use crate::widgets::charts::{method_bar, method_legend};
use crate::widgets::icons::{self, Icon};
use crate::widgets::press::{press, PressBar, PressView};
use crate::widgets::{
    elided_label, panel, primary_button, secondary_button, section_label, stat_strip, Stat,
};
use crate::worker::JobSuccess;

/// State of the result screen.
pub struct FinishedJob {
    /// What kind of job finished.
    pub mode: JobMode,
    /// Display name of the source (file, folder or archive name).
    pub source_name: String,
    /// What the job wrote (archive, restored file or folder); the archive
    /// itself for a verification.
    pub location: PathBuf,
    /// Sizes and duration reported by the worker.
    pub result: JobSuccess,
    /// Original bytes handled per compression path.
    pub path_bytes: PathBytes,
    /// The setup screen the job was started from.
    pub origin: Origin,
}

impl FinishedJob {
    fn title(&self) -> &'static str {
        match self.mode {
            JobMode::Compress => "COMPRESSION COMPLETE",
            JobMode::Extract => "EXTRACTION COMPLETE",
            JobMode::Verify => "ARCHIVE IS INTACT",
        }
    }

    fn press_view(&self) -> PressView<'static> {
        let original = self.result.original_bytes;
        let archive = self.result.archive_bytes;
        let accent = self.mode.accent();
        match self.mode {
            JobMode::Compress => PressView {
                top: PressBar {
                    label: "Original",
                    bytes: original,
                    detail: format_bytes(original),
                    color: RAW_DATA_COLOR,
                },
                bottom: PressBar {
                    label: "Compressed",
                    bytes: archive,
                    detail: format_bytes(archive),
                    color: accent,
                },
                scale_bytes: original,
                caption: if archive < original {
                    format!(
                        "{} of original \u{00B7} {} saved",
                        format_ratio(archive, original),
                        format_bytes(bytes_saved(original, archive))
                    )
                } else if original == 0 {
                    "empty input".to_string()
                } else {
                    format!(
                        "{} of original \u{00B7} no savings on this data",
                        format_ratio(archive, original)
                    )
                },
                live: None,
            },
            JobMode::Extract | JobMode::Verify => PressView {
                top: PressBar {
                    label: "Archive",
                    bytes: archive,
                    detail: format_bytes(archive),
                    color: accent,
                },
                bottom: PressBar {
                    label: if self.mode == JobMode::Verify {
                        "Verified"
                    } else {
                        "Restored"
                    },
                    bytes: original,
                    detail: format_bytes(original),
                    color: RAW_DATA_COLOR,
                },
                scale_bytes: original,
                caption: if original == 0 {
                    "empty archive".to_string()
                } else {
                    format!(
                        "archive is {} of the restored size",
                        format_ratio(archive, original)
                    )
                },
                live: None,
            },
        }
    }

    fn stats(&self) -> [Stat<'static>; 4] {
        let original = self.result.original_bytes;
        let archive = self.result.archive_bytes;
        let speed = format_speed(mb_per_sec(original, self.result.elapsed.as_secs_f64()));
        let elapsed = format_elapsed(self.result.elapsed);
        match self.mode {
            JobMode::Compress => [
                Stat::new("Ratio", format_ratio(archive, original)),
                Stat::new("Saved", format_bytes(bytes_saved(original, archive))),
                Stat::new("Avg speed", speed),
                Stat::new("Elapsed", elapsed),
            ],
            JobMode::Extract => [
                Stat::new("Restored", format_bytes(original)),
                Stat::new("Ratio", format_ratio(archive, original)),
                Stat::new("Avg speed", speed),
                Stat::new("Elapsed", elapsed),
            ],
            JobMode::Verify => [
                Stat::new("Decoded", format_bytes(original)),
                Stat::new("Ratio", format_ratio(archive, original)),
                Stat::new("Avg speed", speed),
                Stat::new("Elapsed", elapsed),
            ],
        }
    }

    /// Paints the screen.
    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        ui.horizontal(|ui| {
            let icon = if self.mode == JobMode::Verify {
                Icon::Shield
            } else {
                Icon::Check
            };
            icons::icon(ui, icon, 22.0, SUCCESS);
            ui.label(RichText::new(self.title()).font(mono(15.0)).color(SUCCESS));
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                elided_label(ui, &self.source_name, sans(14.0), TEXT_MUTED);
            });
        });
        if self.mode == JobMode::Verify {
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "The whole archive was decoded. Every block checksum and the \
                         whole-file checksum match; nothing was written to disk.",
                    )
                    .font(sans(12.5))
                    .color(TEXT),
                )
                .wrap(),
            );
        }
        ui.add_space(2.0);

        panel(ui, |ui| press(ui, &self.press_view()));
        ui.add_space(2.0);
        stat_strip(ui, &self.stats());
        ui.add_space(2.0);

        panel(ui, |ui| {
            section_label(ui, "Methods (share of data)");
            let totals = method_totals(&self.path_bytes);
            method_bar(ui, &totals, 12.0);
            method_legend(ui, &totals);
        });
        ui.add_space(2.0);

        if self.mode != JobMode::Verify {
            ui.horizontal(|ui| {
                ui.label(RichText::new("SAVED TO").font(mono(10.5)).color(TEXT_DIM));
                elided_label(
                    ui,
                    &self.location.display().to_string(),
                    mono(11.0),
                    TEXT_MUTED,
                );
            });
        }
        None
    }

    /// Paints the action row pinned to the bottom of the window.
    pub fn footer(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        ui.horizontal(|ui| {
            if secondary_button(ui, "Close").clicked() {
                action = Some(Action::Quit);
            }
            ui.with_layout(
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| match self.mode {
                    JobMode::Compress | JobMode::Extract => {
                        if primary_button(ui, true, "OPEN FOLDER", self.mode.accent()).clicked() {
                            open_in_explorer(&self.location);
                        }
                        let another = if self.mode == JobMode::Compress {
                            "Compress another"
                        } else {
                            "Extract another"
                        };
                        if secondary_button(ui, another).clicked() {
                            action = Some(Action::GoHome);
                        }
                    }
                    JobMode::Verify => {
                        if primary_button(ui, true, "EXTRACT THIS ARCHIVE", self.mode.accent())
                            .clicked()
                        {
                            action = Some(Action::BackToSetup);
                        }
                        if secondary_button(ui, "Test another").clicked() {
                            action = Some(Action::GoHome);
                        }
                        if secondary_button(ui, "Show in folder").clicked() {
                            open_in_explorer(&self.location);
                        }
                    }
                },
            );
        });
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::extract_setup::ExtractSetup;
    use nexus_core::format::PATH_COUNT;
    use std::time::Duration;

    fn job(mode: JobMode, original: u64, archive: u64) -> FinishedJob {
        FinishedJob {
            mode,
            source_name: "data.bin".to_string(),
            location: PathBuf::from("data.bin.nxar"),
            result: JobSuccess {
                original_bytes: original,
                archive_bytes: archive,
                elapsed: Duration::from_secs(2),
            },
            path_bytes: [0; PATH_COUNT],
            origin: Origin::Extract(Box::new(ExtractSetup::new(PathBuf::from("data.bin.nxar")))),
        }
    }

    #[test]
    fn test_compress_result_reports_ratio_and_savings() {
        let finished = job(JobMode::Compress, 1000, 250);
        let view = finished.press_view();
        assert_eq!(view.top.bytes, 1000);
        assert_eq!(view.bottom.bytes, 250);
        assert_eq!(view.scale_bytes, 1000);
        assert!(view.caption.contains("25.0 %"));
        assert!(view.caption.contains("750 B saved"));
        let stats = finished.stats();
        assert_eq!(stats[0].value, "25.0 %");
        assert_eq!(stats[1].value, "750 B");
    }

    #[test]
    fn test_compress_result_without_savings_does_not_claim_any() {
        let finished = job(JobMode::Compress, 1000, 1064);
        let caption = finished.press_view().caption;
        assert!(caption.contains("no savings"));
        assert!(!caption.contains("saved"));
        assert_eq!(finished.stats()[1].value, "0 B");
    }

    #[test]
    fn test_empty_input_results_do_not_divide_by_zero() {
        for mode in [JobMode::Compress, JobMode::Extract, JobMode::Verify] {
            let finished = job(mode, 0, 64);
            assert!(finished.press_view().caption.contains("empty"));
            for stat in finished.stats() {
                assert!(!stat.value.contains("NaN"));
                assert!(!stat.value.contains("inf"));
            }
        }
    }

    #[test]
    fn test_extract_result_puts_archive_on_top() {
        let finished = job(JobMode::Extract, 4096, 1024);
        let view = finished.press_view();
        assert_eq!(view.top.bytes, 1024);
        assert_eq!(view.bottom.bytes, 4096);
        assert_eq!(view.bottom.label, "Restored");
        assert_eq!(
            job(JobMode::Verify, 4096, 1024).press_view().bottom.label,
            "Verified"
        );
        assert_eq!(
            finished.stats()[2].value,
            format_speed(4096.0 / 1048576.0 / 2.0)
        );
    }

    #[test]
    fn test_titles() {
        assert_eq!(job(JobMode::Compress, 1, 1).title(), "COMPRESSION COMPLETE");
        assert_eq!(job(JobMode::Extract, 1, 1).title(), "EXTRACTION COMPLETE");
        assert_eq!(job(JobMode::Verify, 1, 1).title(), "ARCHIVE IS INTACT");
    }
}
