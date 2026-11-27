//! Extraction setup screen: facts read from the archive (format version,
//! sizes, ratio, block count, methods used), the destination, and the
//! "Test archive" action.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Margin, RichText};
use nexus_core::pipeline::{inspect_archive_file, ArchiveInfo};

use super::Action;
use crate::methods::{method_totals, path_bytes_of_blocks, PathBytes};
use crate::paths::{default_extract_path, display_name, extract_path_in};
use crate::theme::{mono, sans, NEXUS_PURPLE, NEXUS_PURPLE_TEXT, TEXT, TEXT_MUTED};
use crate::util::{format_bytes, format_ratio};
use crate::widgets::charts::{method_bar, method_legend};
use crate::widgets::icons::{self, Icon};
use crate::widgets::{
    elided_label, notice, panel, primary_button, secondary_button, section_label, show_notice,
    stat_strip, Notice, NoticeKind, Stat,
};

/// Facts about an archive gathered without decompressing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveSummary {
    /// Container format version.
    pub format_version: u16,
    /// Total uncompressed size in bytes.
    pub original_bytes: u64,
    /// Number of sub-blocks.
    pub block_count: u32,
    /// Number of superblocks (independent parallel units).
    pub superblock_count: u32,
    /// Original bytes handled per compression path.
    pub path_bytes: PathBytes,
}

impl ArchiveSummary {
    /// Condenses the core's block inventory.
    pub fn from_info(info: &ArchiveInfo) -> Self {
        Self {
            format_version: info.format_version,
            original_bytes: info.total_original_size,
            block_count: info.total_blocks,
            superblock_count: info.superblock_count,
            path_bytes: path_bytes_of_blocks(&info.blocks),
        }
    }
}

/// State of the extraction setup screen.
pub struct ExtractSetup {
    /// Archive to extract or test.
    pub input_path: PathBuf,
    /// Destination path as typed by the user.
    pub output_path: String,
    /// Size of the archive file on disk.
    pub archive_bytes: u64,
    /// Archive facts, or why they could not be read.
    pub summary: Result<ArchiveSummary, String>,
    /// Message shown at the top of the screen, if any.
    pub notice: Option<Notice>,
}

impl ExtractSetup {
    /// Creates the setup for `input_path`, reading the archive's block table.
    pub fn new(input_path: PathBuf) -> Self {
        let archive_bytes = std::fs::metadata(&input_path).map(|m| m.len()).unwrap_or(0);
        let summary = inspect_archive_file(&input_path)
            .map(|info| ArchiveSummary::from_info(&info))
            .map_err(|err| err.to_string());
        let output_path = default_extract_path(&input_path)
            .to_string_lossy()
            .into_owned();
        Self {
            input_path,
            output_path,
            archive_bytes,
            summary,
            notice: None,
        }
    }

    /// Destination path with surrounding whitespace removed.
    pub fn output(&self) -> PathBuf {
        PathBuf::from(self.output_path.trim())
    }

    /// Whether extraction can be started.
    pub fn can_start(&self) -> bool {
        !self.output_path.trim().is_empty()
    }

    /// Paints the screen.
    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        show_notice(ui, self.notice.as_ref());

        panel(ui, |ui| {
            ui.horizontal(|ui| {
                icons::icon(ui, Icon::Archive, 28.0, NEXUS_PURPLE_TEXT);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    elided_label(ui, &display_name(&self.input_path), sans(15.0), TEXT);
                    elided_label(
                        ui,
                        &self.input_path.display().to_string(),
                        mono(11.0),
                        TEXT_MUTED,
                    );
                });
            });
        });
        ui.add_space(4.0);

        self.facts(ui);
        ui.add_space(4.0);

        section_label(ui, "Extract to");
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if secondary_button(ui, "Browse\u{2026}").clicked() {
                    if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                        self.output_path = extract_path_in(&folder, &self.input_path)
                            .to_string_lossy()
                            .into_owned();
                    }
                }
                ui.add(
                    egui::TextEdit::singleline(&mut self.output_path)
                        .font(mono(12.0))
                        .margin(Margin::symmetric(8, 8))
                        .desired_width(ui.available_width()),
                );
            });
        });
        let output = self.output_path.trim();
        let hint = if !output.is_empty() && Path::new(output).exists() {
            "This path already exists: a single-file archive replaces it, a folder archive \
             is unpacked into it (existing files with the same names are replaced)."
        } else {
            "A single-file archive is restored to this path; a folder archive is unpacked \
             into a folder at this path."
        };
        ui.add(egui::Label::new(RichText::new(hint).font(sans(11.5)).color(TEXT_MUTED)).wrap());
        None
    }

    /// Paints the action row pinned to the bottom of the window.
    pub fn footer(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        ui.horizontal(|ui| {
            if secondary_button(ui, "Back").clicked() {
                action = Some(Action::GoHome);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if primary_button(ui, self.can_start(), "EXTRACT", NEXUS_PURPLE).clicked() {
                    action = Some(Action::StartExtract);
                }
                if secondary_button(ui, "Test archive")
                    .on_hover_text(
                        "Decode the whole archive and check every block checksum and the \
                         file checksum, without writing anything.",
                    )
                    .clicked()
                {
                    action = Some(Action::StartVerify);
                }
            });
        });
        action
    }

    fn facts(&self, ui: &mut egui::Ui) {
        section_label(ui, "Archive");
        match &self.summary {
            Ok(summary) => {
                stat_strip(
                    ui,
                    &[
                        Stat::new("Format", format!("v{}", summary.format_version)),
                        Stat::new("Original", format_bytes(summary.original_bytes)),
                        Stat::new("Archive", format_bytes(self.archive_bytes)),
                        Stat::new(
                            "Ratio",
                            format_ratio(self.archive_bytes, summary.original_bytes),
                        ),
                        Stat::new("Blocks", summary.block_count.to_string()),
                    ],
                );
                panel(ui, |ui| {
                    section_label(
                        ui,
                        &format!(
                            "Methods (share of data) \u{00B7} {} superblocks",
                            summary.superblock_count
                        ),
                    );
                    let totals = method_totals(&summary.path_bytes);
                    method_bar(ui, &totals, 12.0);
                    method_legend(ui, &totals);
                });
            }
            Err(error) => {
                notice(
                    ui,
                    NoticeKind::Error,
                    &format!(
                        "The archive structure could not be read ({error}). The file may be \
                         damaged or not a NEXUS archive. {} on disk.",
                        format_bytes(self.archive_bytes)
                    ),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::format::PATH_COUNT;

    #[test]
    fn test_setup_for_missing_archive_reports_error_but_keeps_defaults() {
        let path = std::env::temp_dir().join("nexus_gui_test_missing_archive.nxar");
        let setup = ExtractSetup::new(path.clone());
        assert!(setup.summary.is_err());
        assert_eq!(setup.archive_bytes, 0);
        assert!(setup
            .output_path
            .ends_with("nexus_gui_test_missing_archive"));
        assert!(setup.can_start());
        assert_eq!(
            setup.output(),
            path.with_file_name("nexus_gui_test_missing_archive")
        );
    }

    #[test]
    fn test_setup_reads_real_archive_summary() {
        let dir = std::env::temp_dir().join(format!(
            "nexus_gui_test_extract_setup_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("data.txt");
        let archive = dir.join("data.txt.nxar");
        let data = b"abcdefghij".repeat(5000);
        std::fs::write(&input, &data).unwrap();
        nexus_core::compress_file(&input, &archive, 2).unwrap();

        let setup = ExtractSetup::new(archive.clone());
        let summary = setup.summary.clone().expect("archive must be readable");
        assert_eq!(summary.original_bytes, data.len() as u64);
        assert!(summary.block_count >= 1);
        assert!(summary.superblock_count >= 1);
        assert_eq!(
            summary.path_bytes.iter().sum::<u64>(),
            data.len() as u64,
            "every byte is attributed to a path"
        );
        assert_eq!(summary.path_bytes.len(), PATH_COUNT);
        assert_eq!(
            setup.archive_bytes,
            std::fs::metadata(&archive).unwrap().len()
        );
        assert_eq!(setup.output(), dir.join("data.txt"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_blank_output_cannot_start() {
        let mut setup = ExtractSetup::new(PathBuf::from("nothing.nxar"));
        setup.output_path = "  ".to_string();
        assert!(!setup.can_start());
    }
}
