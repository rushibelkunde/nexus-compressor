//! Compression setup screen: source summary, destination, preset cards and
//! level rail (kept in sync), and what the selected level means — engine,
//! estimated output size and savings, and an estimated duration.

use std::path::{Path, PathBuf};

use eframe::egui::{self, vec2, Margin, Rect, RichText, Sense};
use nexus_core::format::BlockType;
use nexus_core::pipeline::{CompressionEstimate, CompressionPreset};

use super::Action;
use crate::analysis::{EstimateCache, ScanState, SAMPLE_BYTES};
use crate::engine::{
    available_threads, clamp_level, engine_for_level, estimate_compress_seconds, is_slow_level,
    parallel_units, preset_for_level, preset_tagline, preset_title, reference_throughput_mb_s,
    superblock_bytes, ESTIMATE_LEVEL_CAP, MAX_LEVEL, NEURAL_LEVEL, PRESETS, SLOW_LEVEL,
};
use crate::paths::{default_archive_path, display_name, DEFAULT_ARCHIVE_EXTENSION};
use crate::theme::{mono, sans, NEXUS_CYAN, TEXT, TEXT_DIM, TEXT_MUTED};
use crate::util::{format_bytes, format_estimate};
use crate::widgets::icons::{self, Icon};
use crate::widgets::level_rail::{level_rail, tier_color};
use crate::widgets::{
    elided_label, notice, panel, preset_card, primary_button, secondary_button, section_label,
    show_notice, stat_strip, Notice, NoticeKind, PresetCard, Stat,
};

/// State of the compression setup screen.
pub struct CompressSetup {
    /// File or folder to compress.
    pub input_path: PathBuf,
    /// Whether the input is a folder.
    pub is_dir: bool,
    /// Destination archive path as typed by the user.
    pub output_path: String,
    /// Selected compression level, 1 to 19.
    pub level: u32,
    /// Background scan of the source (size and content sample).
    pub scan: ScanState,
    /// Size estimates per level.
    pub estimates: EstimateCache,
    /// Start the job as soon as the scan is in (shell "compress here" entry).
    pub auto_start: bool,
    /// Message shown at the top of the screen, if any.
    pub notice: Option<Notice>,
    threads: usize,
}

/// Short display label of a detected content type.
pub fn block_type_label(block_type: BlockType) -> &'static str {
    match block_type {
        BlockType::Text => "Text",
        BlockType::SourceCode => "Source code",
        BlockType::StructuredData => "Structured data",
        BlockType::Binary => "Binary",
        BlockType::Executable => "Executable code",
        BlockType::FloatingPoint => "Floating-point data",
        BlockType::Audio => "Audio",
        BlockType::Image => "Image",
        BlockType::Incompressible => "Already compressed / random",
        _ => "Unknown",
    }
}

fn preset_icon(preset: CompressionPreset) -> Icon {
    match preset {
        CompressionPreset::Low => Icon::Bolt,
        CompressionPreset::Medium => Icon::Balance,
        CompressionPreset::High => Icon::Gem,
    }
}

impl CompressSetup {
    /// Creates the setup for `input_path` and starts scanning it in the
    /// background. `preset` pre-selects a level (default: balanced).
    pub fn new(
        input_path: PathBuf,
        preset: Option<CompressionPreset>,
        auto_start: bool,
        ctx: &egui::Context,
    ) -> Self {
        let is_dir = input_path.is_dir();
        let output_path = default_archive_path(&input_path)
            .to_string_lossy()
            .into_owned();
        let level = preset.unwrap_or_default().to_level();
        let scan = ScanState::start(input_path.clone(), ctx.clone());
        Self {
            input_path,
            is_dir,
            output_path,
            level,
            scan,
            estimates: EstimateCache::default(),
            auto_start,
            notice: None,
            threads: available_threads(),
        }
    }

    /// Collects background results and keeps the estimate for the selected
    /// level cached or in progress. Call once per frame.
    pub fn poll(&mut self, ctx: &egui::Context) {
        self.scan.poll();
        self.estimates.poll();
        if let Some(scan) = self.scan.ready() {
            if scan.error.is_none() {
                self.estimates.ensure(self.level, scan, ctx);
            }
        }
    }

    /// Total source size once the scan has finished.
    pub fn total_bytes(&self) -> Option<u64> {
        self.scan.ready().map(|scan| scan.total_bytes)
    }

    /// Whether the scan has finished (successfully or not).
    pub fn is_scanned(&self) -> bool {
        self.scan.ready().is_some()
    }

    /// Destination path with surrounding whitespace removed.
    pub fn output(&self) -> PathBuf {
        PathBuf::from(self.output_path.trim())
    }

    /// Whether the job can be started.
    pub fn can_start(&self) -> bool {
        self.is_scanned() && !self.output_path.trim().is_empty()
    }

    /// Paints the screen.
    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        self.level = clamp_level(self.level);
        show_notice(ui, self.notice.as_ref());

        self.source_panel(ui);
        ui.add_space(4.0);
        self.destination_row(ui);
        ui.add_space(4.0);
        self.level_controls(ui);
        ui.add_space(4.0);
        self.readout(ui);
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
                if primary_button(ui, self.can_start(), "START COMPRESSION", NEXUS_CYAN).clicked() {
                    action = Some(Action::StartCompress);
                }
                if !self.is_scanned() {
                    ui.label(
                        RichText::new("Scanning source\u{2026}")
                            .font(sans(12.0))
                            .color(TEXT_MUTED),
                    );
                    ui.spinner();
                }
            });
        });
        action
    }

    fn source_panel(&self, ui: &mut egui::Ui) {
        panel(ui, |ui| {
            ui.horizontal(|ui| {
                let icon = if self.is_dir {
                    Icon::Folder
                } else {
                    Icon::File
                };
                icons::icon(ui, icon, 28.0, NEXUS_CYAN);
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
    }

    fn destination_row(&mut self, ui: &mut egui::Ui) {
        section_label(ui, "Output archive");
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if secondary_button(ui, "Browse\u{2026}").clicked() {
                    let default_name = format!(
                        "{}.{DEFAULT_ARCHIVE_EXTENSION}",
                        display_name(&self.input_path)
                    );
                    if let Some(path) = rfd::FileDialog::new()
                        .set_file_name(default_name)
                        .add_filter("NEXUS Archive (*.nxar)", &[DEFAULT_ARCHIVE_EXTENSION])
                        .save_file()
                    {
                        self.output_path = path.to_string_lossy().into_owned();
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
        if !output.is_empty() && Path::new(output).exists() {
            ui.label(
                RichText::new("A file already exists at this path and will be replaced.")
                    .font(sans(11.5))
                    .color(TEXT_MUTED),
            );
        }
    }

    fn level_controls(&mut self, ui: &mut egui::Ui) {
        section_label(ui, "Compression");
        ui.columns(PRESETS.len(), |columns| {
            for (column, preset) in columns.iter_mut().zip(PRESETS) {
                let preset_level = preset.to_level();
                let facts = format!(
                    "LEVEL {preset_level} \u{00B7} ~{} MB/s/core",
                    reference_throughput_mb_s(preset_level)
                );
                let card = PresetCard {
                    icon: preset_icon(preset),
                    title: preset_title(preset),
                    facts: &facts,
                    tagline: preset_tagline(preset),
                };
                let selected = preset_for_level(self.level) == Some(preset);
                if preset_card(column, selected, &card).clicked() {
                    self.level = preset_level;
                    // Repaint so the cards drawn before this one, and the
                    // estimate for the new level, update without more input.
                    column.ctx().request_repaint();
                }
            }
        });
        ui.add_space(2.0);

        let engine = engine_for_level(self.level);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("LEVEL {:>2} / {MAX_LEVEL}", self.level))
                    .font(mono(13.0))
                    .color(TEXT),
            );
            let preset_name = match preset_for_level(self.level) {
                Some(preset) => preset_title(preset).to_uppercase(),
                None => "CUSTOM".to_string(),
            };
            ui.label(RichText::new(preset_name).font(mono(10.5)).color(TEXT_DIM));
            // Swatch in the rail's colour for this level; the name itself
            // stays in the text colour for contrast.
            let (swatch, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
            ui.painter()
                .rect_filled(swatch, 1.0, tier_color(self.level));
            ui.add(
                egui::Label::new(RichText::new(engine.name).font(sans(13.0)).color(TEXT))
                    .truncate(),
            );
        });
        if level_rail(ui, &mut self.level) {
            // Describe the level that was just picked, not the previous one.
            ui.ctx().request_repaint();
        }
        ui.add(
            egui::Label::new(
                RichText::new(engine.detail)
                    .font(sans(12.0))
                    .color(TEXT_MUTED),
            )
            .wrap(),
        );
        // The warning sits right under the rail so it cannot be missed.
        if let Some(warning) = slow_level_warning(self.level) {
            ui.add_space(2.0);
            notice(ui, NoticeKind::Warning, &warning);
        }
    }

    fn readout(&self, ui: &mut egui::Ui) {
        let level = self.level;
        let Some(scan) = self.scan.ready() else {
            stat_strip(
                ui,
                &[
                    Stat::new("Source", "scanning\u{2026}").colored(TEXT_DIM),
                    Stat::new("Est. output", "--").colored(TEXT_DIM),
                    Stat::new("Est. savings", "--").colored(TEXT_DIM),
                    Stat::new("Est. time", "--").colored(TEXT_DIM),
                ],
            );
            return;
        };
        if let Some(error) = &scan.error {
            notice(
                ui,
                NoticeKind::Error,
                &format!("The source could not be read: {error}"),
            );
            return;
        }

        let total = scan.total_bytes;
        let estimate: Option<CompressionEstimate> = self.estimates.get(level);
        let capped = level > ESTIMATE_LEVEL_CAP;
        let pending = || Stat::new("", "calculating\u{2026}").colored(TEXT_DIM);

        let output_label = if capped {
            "Est. output (max)"
        } else {
            "Est. output"
        };
        let savings_label = if capped {
            "Est. savings (min)"
        } else {
            "Est. savings"
        };
        let (output_stat, savings_stat) = match estimate {
            Some(est) => (
                Stat::new(
                    output_label,
                    format!("~{}", format_bytes(est.estimated_bytes)),
                ),
                Stat::new(savings_label, format!("~{:.1} %", est.savings_percent)),
            ),
            None => (
                Stat {
                    label: output_label,
                    ..pending()
                },
                Stat {
                    label: savings_label,
                    ..pending()
                },
            ),
        };
        let seconds = estimate_compress_seconds(total, level, self.threads);
        stat_strip(
            ui,
            &[
                Stat::new("Source", format_bytes(total)),
                output_stat,
                savings_stat,
                Stat::new("Est. time", format_estimate(seconds)),
            ],
        );

        // Miniature before/after bars make the estimate tangible.
        if let Some(est) = estimate {
            estimate_bars(ui, total, est.estimated_bytes);
        }

        let units = parallel_units(total, level, self.threads);
        let block_size = format_bytes(superblock_bytes(level));
        let parallelism = if units == 1 {
            format!("one {block_size} block at a time")
        } else {
            format!("{units} blocks of {block_size} in parallel")
        };
        let sample = format_bytes(SAMPLE_BYTES as u64);
        let size_basis = if capped {
            format!(
                "Size is an upper bound from a level-{ESTIMATE_LEVEL_CAP} trial compression of \
                 the first {sample} (the extra models only ever shrink blocks)"
            )
        } else {
            format!("Size from a trial compression of the first {sample}, scaled up")
        };
        let content = match estimate {
            Some(est) => format!(
                "Detected content: {}. ",
                block_type_label(est.detected_type)
            ),
            None => String::new(),
        };
        let caption = format!(
            "{content}{size_basis}; time from reference speeds (~{} MB/s per core, \
             {parallelism}). Both are estimates.",
            reference_throughput_mb_s(level),
        );
        ui.add(egui::Label::new(RichText::new(caption).font(sans(11.5)).color(TEXT_MUTED)).wrap());

        if estimate.is_some_and(|est| est.is_incompressible) {
            ui.add_space(2.0);
            notice(
                ui,
                NoticeKind::Warning,
                "This data looks already compressed or encrypted. Expect little or no size \
                 reduction at any level; a fast level will finish sooner for the same result.",
            );
        }
    }
}

/// Warning shown for the much slower modelling levels, or `None` below them.
pub fn slow_level_warning(level: u32) -> Option<String> {
    if !is_slow_level(level) {
        return None;
    }
    let high = CompressionPreset::High.to_level();
    let medium = CompressionPreset::Medium.to_level();
    let mut warning = format!(
        "Levels {SLOW_LEVEL}+ are much slower: about {} MB/s per core at level {level}, \
         against ~{} MB/s at level {high} and ~{} MB/s at level {medium}. Expect long run \
         times on large inputs.",
        reference_throughput_mb_s(level),
        reference_throughput_mb_s(high),
        reference_throughput_mb_s(medium),
    );
    if level >= NEURAL_LEVEL {
        warning.push_str(&format!(
            " Levels {NEURAL_LEVEL}\u{2013}{MAX_LEVEL} add the neural model and are extremely slow."
        ));
    }
    Some(warning)
}

// Two thin bars on a common scale: the source and the estimated archive.
fn estimate_bars(ui: &mut egui::Ui, total: u64, estimated: u64) {
    use crate::theme::{BG_INSET, RADIUS};
    use crate::widgets::press::bar_length;

    let width = ui.available_width().max(60.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, 16.0), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter().with_clip_rect(rect);
    let scale = total.max(estimated).max(1);
    let rows = [
        (total, super::RAW_DATA_COLOR, rect.top()),
        (estimated, NEXUS_CYAN, rect.top() + 9.0),
    ];
    for (bytes, color, top) in rows {
        let track = Rect::from_min_size(egui::pos2(rect.left(), top), vec2(rect.width(), 7.0));
        painter.rect_filled(track, RADIUS, BG_INSET);
        let length = bar_length(crate::util::fraction_of(bytes, scale), track.width());
        if length > 0.0 {
            painter.rect_filled(
                Rect::from_min_size(track.min, vec2(length, track.height())),
                RADIUS,
                color,
            );
        }
    }
    response.on_hover_text(format!(
        "Source {}\nEstimated archive ~{}",
        format_bytes(total),
        format_bytes(estimated)
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_block_type_labels_are_distinct_and_non_empty() {
        let types = [
            BlockType::Text,
            BlockType::SourceCode,
            BlockType::StructuredData,
            BlockType::Binary,
            BlockType::Executable,
            BlockType::FloatingPoint,
            BlockType::Audio,
            BlockType::Image,
            BlockType::Incompressible,
        ];
        let labels: Vec<&str> = types.iter().map(|t| block_type_label(*t)).collect();
        for (i, label) in labels.iter().enumerate() {
            assert!(!label.is_empty());
            assert_ne!(*label, "Unknown");
            assert!(!labels[..i].contains(label), "duplicate label {label}");
        }
        assert_eq!(block_type_label(BlockType::SuperblockBoundary), "Unknown");
    }

    #[test]
    fn test_slow_level_warning_only_from_level_13() {
        for level in 1..SLOW_LEVEL {
            assert_eq!(slow_level_warning(level), None);
        }
        for level in SLOW_LEVEL..=MAX_LEVEL {
            let warning = slow_level_warning(level).expect("slow levels warn");
            assert!(warning.contains("much slower"));
            assert_eq!(
                warning.contains("neural"),
                level >= NEURAL_LEVEL,
                "level {level}"
            );
        }
    }

    #[test]
    fn test_new_setup_defaults() {
        let ctx = egui::Context::default();
        let input = std::env::temp_dir().join("nexus_gui_test_setup_missing.bin");
        let setup = CompressSetup::new(input.clone(), None, false, &ctx);
        assert_eq!(setup.level, CompressionPreset::Medium.to_level());
        assert!(setup
            .output_path
            .ends_with("nexus_gui_test_setup_missing.bin.nxar"));
        assert!(!setup.is_dir);
        assert!(!setup.auto_start);

        let fast = CompressSetup::new(input, Some(CompressionPreset::Low), true, &ctx);
        assert_eq!(fast.level, CompressionPreset::Low.to_level());
        assert!(fast.auto_start);
    }

    #[test]
    fn test_cannot_start_before_scan_or_without_output() {
        let ctx = egui::Context::default();
        let input = std::env::temp_dir().join("nexus_gui_test_setup_missing2.bin");
        let mut setup = CompressSetup::new(input, None, false, &ctx);
        // Wait for the (failing) scan of the missing file.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !setup.is_scanned() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
            setup.poll(&ctx);
        }
        assert!(setup.can_start());
        assert_eq!(setup.total_bytes(), Some(0));
        setup.output_path = "   ".to_string();
        assert!(!setup.can_start());
        setup.output_path = "  out.nxar ".to_string();
        assert_eq!(setup.output(), PathBuf::from("out.nxar"));
    }
}
