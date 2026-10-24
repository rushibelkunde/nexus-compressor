#![windows_subsystem = "windows"]

use eframe::egui::{self, Color32, CornerRadius, Margin, RichText, Stroke, Vec2};
use nexus_core::pipeline::{
    compress_file_with_progress, decompress_file_with_progress, estimate_compression, sample_path,
    CompressionEstimate, CompressionPreset, ProgressUpdate,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const NEXUS_CYAN: Color32 = Color32::from_rgb(0, 229, 255);
const NEXUS_PURPLE: Color32 = Color32::from_rgb(157, 78, 221);
const BG_DARK: Color32 = Color32::from_rgb(15, 17, 26);
const BG_CARD: Color32 = Color32::from_rgb(24, 27, 40);
const BG_CARD_HOVER: Color32 = Color32::from_rgb(32, 36, 54);
const BORDER_COLOR: Color32 = Color32::from_rgb(45, 50, 75);
const TEXT_MUTED: Color32 = Color32::from_rgb(140, 146, 172);
const SUCCESS_GREEN: Color32 = Color32::from_rgb(76, 217, 100);

#[derive(Clone, Debug)]
struct WorkerSuccess {
    is_compression: bool,
    _source_path: PathBuf,
    output_path: PathBuf,
    original_size: u64,
    final_size: u64,
    elapsed: Duration,
}

enum AppState {
    Home,
    CompressSetup {
        input_path: PathBuf,
        output_path: String,
        preset: CompressionPreset,
        custom_level: u32,
        is_custom_level: bool,
        total_size: u64,
        is_dir: bool,
        sample_buffer: Vec<u8>,
        estimate: CompressionEstimate,
    },
    ExtractSetup {
        input_path: PathBuf,
        output_dir: String,
        archive_size: u64,
    },
    Running {
        is_compressing: bool,
        source_name: String,
        _output_path: PathBuf,
        original_size: u64,
        start_time: Instant,
        progress_rx: mpsc::Receiver<ProgressUpdate>,
        result_rx: mpsc::Receiver<Result<WorkerSuccess, String>>,
        latest_progress: Option<ProgressUpdate>,
    },
    Finished(WorkerSuccess),
    Error(String),
}

struct NexusGuiApp {
    state: AppState,
}

impl NexusGuiApp {
    fn new(cc: &eframe::CreationContext<'_>, initial_target: Option<InitialAction>) -> Self {
        let mut visuals = egui::Visuals::dark();
        visuals.override_text_color = Some(Color32::from_rgb(240, 243, 255));
        visuals.panel_fill = BG_DARK;
        visuals.window_fill = BG_DARK;
        visuals.widgets.noninteractive.bg_fill = BG_CARD;
        visuals.widgets.inactive.bg_fill = BG_CARD;
        visuals.widgets.hovered.bg_fill = BG_CARD_HOVER;
        visuals.widgets.active.bg_fill = BG_CARD_HOVER;
        cc.egui_ctx.set_visuals(visuals);

        let mut app = Self {
            state: AppState::Home,
        };

        match initial_target {
            Some(InitialAction::Compress {
                path,
                preset,
                auto_start,
            }) => {
                let setup = Self::setup_compress(path, preset);
                if auto_start {
                    if let AppState::CompressSetup {
                        input_path,
                        output_path,
                        preset,
                        custom_level,
                        is_custom_level,
                        total_size,
                        ..
                    } = setup
                    {
                        let level = if is_custom_level {
                            custom_level
                        } else {
                            preset.to_level()
                        };
                        app.start_compression(input_path, output_path, level, total_size);
                    }
                } else {
                    app.state = setup;
                }
            }
            Some(InitialAction::Decompress { path, auto_start }) => {
                let setup = Self::setup_decompress(path);
                if auto_start {
                    if let AppState::ExtractSetup {
                        input_path,
                        output_dir,
                        ..
                    } = setup
                    {
                        app.start_decompression(input_path, output_dir);
                    }
                } else {
                    app.state = setup;
                }
            }
            None => {}
        }

        app
    }

    fn setup_compress(input_path: PathBuf, initial_preset: Option<CompressionPreset>) -> AppState {
        let (sample_buffer, total_size, is_dir) = match sample_path(&input_path, 64 * 1024) {
            Ok(res) => res,
            Err(_) => (Vec::new(), 0, input_path.is_dir()),
        };

        let file_name = input_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let default_output = input_path.with_file_name(format!("{}.nxar", file_name));

        let preset = initial_preset.unwrap_or(CompressionPreset::Medium);
        let custom_level = preset.to_level();
        let estimate = estimate_compression(&sample_buffer, total_size, custom_level);

        AppState::CompressSetup {
            input_path,
            output_path: default_output.to_string_lossy().to_string(),
            preset,
            custom_level,
            is_custom_level: false,
            total_size,
            is_dir,
            sample_buffer,
            estimate,
        }
    }

    fn setup_decompress(input_path: PathBuf) -> AppState {
        let archive_size = fs::metadata(&input_path).map(|m| m.len()).unwrap_or(0);
        let file_name = input_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let stripped_name = if let Some(s) = file_name.strip_suffix(".nxar") {
            s
        } else if let Some(s) = file_name.strip_suffix(".nxs") {
            s
        } else {
            &file_name
        };

        let default_output_dir = input_path.with_file_name(stripped_name);

        AppState::ExtractSetup {
            input_path,
            output_dir: default_output_dir.to_string_lossy().to_string(),
            archive_size,
        }
    }

    fn start_compression(
        &mut self,
        input: PathBuf,
        output_str: String,
        level: u32,
        total_size: u64,
    ) {
        let output = PathBuf::from(output_str);
        let (tx_prog, rx_prog) = mpsc::channel();
        let (tx_res, rx_res) = mpsc::channel();

        let source_name = input
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let worker_input = input.clone();
        let worker_output = output.clone();

        let progress_cb = Arc::new(move |p: ProgressUpdate| {
            let _ = tx_prog.send(p);
        });

        thread::spawn(move || {
            let start = Instant::now();
            let res = compress_file_with_progress(
                &worker_input,
                &worker_output,
                level,
                Some(progress_cb),
            );
            match res {
                Ok(()) => {
                    let final_size = fs::metadata(&worker_output)
                        .map(|m| m.len())
                        .unwrap_or(0);
                    let _ = tx_res.send(Ok(WorkerSuccess {
                        is_compression: true,
                        _source_path: worker_input,
                        output_path: worker_output,
                        original_size: total_size,
                        final_size,
                        elapsed: start.elapsed(),
                    }));
                }
                Err(err) => {
                    let _ = tx_res.send(Err(format!("Compression failed: {}", err)));
                }
            }
        });

        self.state = AppState::Running {
            is_compressing: true,
            source_name,
            _output_path: output,
            original_size: total_size,
            start_time: Instant::now(),
            progress_rx: rx_prog,
            result_rx: rx_res,
            latest_progress: None,
        };
    }

    fn start_decompression(&mut self, input: PathBuf, output_dir_str: String) {
        let output_dir = PathBuf::from(output_dir_str);
        let (tx_prog, rx_prog) = mpsc::channel();
        let (tx_res, rx_res) = mpsc::channel();

        let source_name = input
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let worker_input = input.clone();
        let worker_output = output_dir.clone();

        let progress_cb = Arc::new(move |p: ProgressUpdate| {
            let _ = tx_prog.send(p);
        });

        thread::spawn(move || {
            let start = Instant::now();
            let res = decompress_file_with_progress(
                &worker_input,
                &worker_output,
                Some(progress_cb),
            );
            match res {
                Ok(()) => {
                    let _ = tx_res.send(Ok(WorkerSuccess {
                        is_compression: false,
                        _source_path: worker_input,
                        output_path: worker_output,
                        original_size: 0,
                        final_size: 0,
                        elapsed: start.elapsed(),
                    }));
                }
                Err(err) => {
                    let _ = tx_res.send(Err(format!("Decompression failed: {}", err)));
                }
            }
        });

        self.state = AppState::Running {
            is_compressing: false,
            source_name,
            _output_path: output_dir,
            original_size: 0,
            start_time: Instant::now(),
            progress_rx: rx_prog,
            result_rx: rx_res,
            latest_progress: None,
        };
    }
}

impl eframe::App for NexusGuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if matches!(self.state, AppState::Running { .. }) {
            ctx.request_repaint_after(Duration::from_millis(30));
        }

        if let AppState::Running {
            ref progress_rx,
            ref result_rx,
            ref mut latest_progress,
            ..
        } = self.state
        {
            while let Ok(update) = progress_rx.try_recv() {
                *latest_progress = Some(update);
            }

            if let Ok(res) = result_rx.try_recv() {
                match res {
                    Ok(success) => {
                        self.state = AppState::Finished(success);
                    }
                    Err(err) => {
                        self.state = AppState::Error(err);
                    }
                }
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(8.0);
            render_header(ui);
            ui.add_space(14.0);

            match &mut self.state {
                AppState::Home => {
                    self.render_home(ui);
                }
                AppState::CompressSetup {
                    input_path,
                    output_path,
                    preset,
                    custom_level,
                    is_custom_level,
                    total_size,
                    is_dir,
                    sample_buffer,
                    estimate,
                } => {
                    let mut start_requested = false;
                    let input_clone = input_path.clone();
                    let output_clone = output_path.clone();
                    let active_level = if *is_custom_level {
                        *custom_level
                    } else {
                        preset.to_level()
                    };
                    let size_val = *total_size;

                    egui::Frame::new()
                        .fill(BG_CARD)
                        .corner_radius(CornerRadius::same(8))
                        .stroke(Stroke::new(1.0_f32, BORDER_COLOR))
                        .inner_margin(Margin::same(16))
                        .show(ui, |ui| {
                            // Source Info Row
                            ui.horizontal(|ui| {
                                let icon = if *is_dir { "📁" } else { "📄" };
                                ui.label(RichText::new(icon).size(22.0));
                                ui.vertical(|ui| {
                                    ui.label(
                                        RichText::new(format!(
                                            "Source: {}",
                                            input_clone
                                                .file_name()
                                                .unwrap_or_default()
                                                .to_string_lossy()
                                        ))
                                        .size(15.0)
                                        .strong(),
                                    );
                                    ui.label(
                                        RichText::new(format!(
                                            "Path: {} ({})",
                                            input_clone.display(),
                                            format_bytes(size_val)
                                        ))
                                        .size(12.0)
                                        .color(TEXT_MUTED),
                                    );
                                });
                            });

                            ui.add_space(10.0);
                            ui.separator();
                            ui.add_space(10.0);

                            // Output Destination
                            ui.label(
                                RichText::new("Output Archive Destination:").size(13.0).strong(),
                            );
                            ui.horizontal(|ui| {
                                ui.text_edit_singleline(output_path);
                                if ui.button("Browse...").clicked() {
                                    let default_name = input_clone
                                        .file_name()
                                        .unwrap_or_default()
                                        .to_string_lossy();
                                    if let Some(path) = rfd::FileDialog::new()
                                        .set_file_name(format!("{}.nxar", default_name))
                                        .add_filter("NEXUS Archive (*.nxar)", &["nxar"])
                                        .save_file()
                                    {
                                        *output_path = path.to_string_lossy().to_string();
                                    }
                                }
                            });

                            ui.add_space(14.0);
                            ui.label(
                                RichText::new("Select Compression Preset or Custom Level:")
                                    .size(13.0)
                                    .strong(),
                            );
                            ui.add_space(6.0);

                            // 4 Cards: Low, Medium, High, Custom
                            ui.horizontal(|ui| {
                                if preset_card(
                                    ui,
                                    !*is_custom_level && *preset == CompressionPreset::Low,
                                    "⚡ Fast (Low)",
                                    "Level 2",
                                    "~250 MB/s",
                                    "Quick backups",
                                ) {
                                    *is_custom_level = false;
                                    *preset = CompressionPreset::Low;
                                    *custom_level = 2;
                                    *estimate =
                                        estimate_compression(sample_buffer, *total_size, 2);
                                }

                                if preset_card(
                                    ui,
                                    !*is_custom_level && *preset == CompressionPreset::Medium,
                                    "⚖️ Balanced",
                                    "Level 6",
                                    "~80 MB/s",
                                    "Recommended everyday",
                                ) {
                                    *is_custom_level = false;
                                    *preset = CompressionPreset::Medium;
                                    *custom_level = 6;
                                    *estimate =
                                        estimate_compression(sample_buffer, *total_size, 6);
                                }

                                if preset_card(
                                    ui,
                                    !*is_custom_level && *preset == CompressionPreset::High,
                                    "💎 Max (High)",
                                    "Level 9",
                                    "~30 MB/s",
                                    "Maximum savings",
                                ) {
                                    *is_custom_level = false;
                                    *preset = CompressionPreset::High;
                                    *custom_level = 9;
                                    *estimate =
                                        estimate_compression(sample_buffer, *total_size, 9);
                                }

                                if preset_card(
                                    ui,
                                    *is_custom_level,
                                    "🎛️ Custom",
                                    &format!("Level {}", custom_level),
                                    "1 to 19",
                                    "Select exact level",
                                ) {
                                    *is_custom_level = true;
                                    *estimate = estimate_compression(
                                        sample_buffer,
                                        *total_size,
                                        *custom_level,
                                    );
                                }
                            });

                            // Custom Level Slider & Engine Descriptor
                            if *is_custom_level {
                                ui.add_space(8.0);
                                egui::Frame::new()
                                    .fill(BG_CARD_HOVER)
                                    .corner_radius(CornerRadius::same(6))
                                    .stroke(Stroke::new(1.0_f32, NEXUS_CYAN))
                                    .inner_margin(Margin::same(10))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new("Compression Level (1–19):")
                                                    .size(12.0)
                                                    .strong(),
                                            );
                                            let slider = egui::Slider::new(custom_level, 1..=19);
                                            if ui.add(slider).changed() {
                                                *estimate = estimate_compression(
                                                    sample_buffer,
                                                    *total_size,
                                                    *custom_level,
                                                );
                                            }
                                        });

                                        ui.add_space(4.0);
                                        let (algo_title, algo_desc) = match *custom_level {
                                            1..=4 => (
                                                "⚡ Fast LZ77 + Normalized rANS",
                                                "High throughput, low CPU overhead (~120–250 MB/s)",
                                            ),
                                            5..=9 => (
                                                "⚖️ Adaptive Engine (LZ77 + BWT/MTF + Filters + rANS)",
                                                "Adaptive boundary splitting & dynamic multi-transform (~30–80 MB/s)",
                                            ),
                                            10..=12 => (
                                                "💎 Deep LZ77 + Exhaustive Hash Search",
                                                "Maximum dictionary depth & match finder search (~15–30 MB/s)",
                                            ),
                                            13..=16 => (
                                                "🧠 Bit-Level Context Mixing",
                                                "Neural Logistic Mixer blending 5 order models (~1–3 MB/s)",
                                            ),
                                            17..=19 => (
                                                "🧬 Fixed-Point Neural Compression",
                                                "Online integer SGD bit-level neural network predictor (~0.2–0.8 MB/s)",
                                            ),
                                            _ => ("Custom Engine", "NEXUS custom compression pipeline"),
                                        };

                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new(algo_title)
                                                    .color(NEXUS_CYAN)
                                                    .strong()
                                                    .size(11.0),
                                            );
                                            ui.label(
                                                RichText::new(format!("— {}", algo_desc))
                                                    .color(TEXT_MUTED)
                                                    .size(10.5),
                                            );
                                        });
                                    });
                            }

                            // Live Estimated Compression Preview Box
                            ui.add_space(10.0);
                            let est_frame_border = if estimate.is_incompressible {
                                Color32::from_rgb(220, 150, 40)
                            } else {
                                Color32::from_rgb(0, 180, 220)
                            };
                            let est_frame_bg = if estimate.is_incompressible {
                                Color32::from_rgb(32, 24, 14)
                            } else {
                                Color32::from_rgb(18, 26, 38)
                            };

                            egui::Frame::new()
                                .fill(est_frame_bg)
                                .corner_radius(CornerRadius::same(6))
                                .stroke(Stroke::new(1.0_f32, est_frame_border))
                                .inner_margin(Margin::same(12))
                                .show(ui, |ui| {
                                    if estimate.is_incompressible {
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new("⚠️").size(18.0));
                                            ui.vertical(|ui| {
                                                ui.label(
                                                    RichText::new(
                                                        "Content appears already compressed or high-entropy",
                                                    )
                                                    .color(Color32::from_rgb(240, 180, 50))
                                                    .strong()
                                                    .size(13.0),
                                                );
                                                ui.label(
                                                    RichText::new(format!(
                                                        "Estimated size: ~{} (Detected as {:?}, minimal space reduction expected)",
                                                        format_bytes(estimate.estimated_bytes),
                                                        estimate.detected_type
                                                    ))
                                                    .color(TEXT_MUTED)
                                                    .size(11.0),
                                                );
                                            });
                                        });
                                    } else {
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new("📊").size(18.0));
                                            ui.vertical(|ui| {
                                                ui.horizontal(|ui| {
                                                    ui.label(
                                                        RichText::new(format!(
                                                            "Estimated Compressed Size: ~{}",
                                                            format_bytes(estimate.estimated_bytes)
                                                        ))
                                                        .color(NEXUS_CYAN)
                                                        .strong()
                                                        .size(13.5),
                                                    );
                                                    ui.label(
                                                        RichText::new(format!(
                                                            "(~{:.1}% reduction, saves ~{})",
                                                            estimate.savings_percent,
                                                            format_bytes(
                                                                size_val.saturating_sub(
                                                                    estimate.estimated_bytes
                                                                )
                                                            )
                                                        ))
                                                        .color(SUCCESS_GREEN)
                                                        .strong()
                                                        .size(12.0),
                                                    );
                                                });
                                                ui.add_space(2.0);
                                                ui.label(
                                                    RichText::new(format!(
                                                        "Input: {}  ➔  Est. Output: ~{}  |  Level {}  |  Content: {:?}",
                                                        format_bytes(size_val),
                                                        format_bytes(estimate.estimated_bytes),
                                                        active_level,
                                                        estimate.detected_type
                                                    ))
                                                    .color(TEXT_MUTED)
                                                    .size(11.0),
                                                );
                                            });
                                        });
                                    }
                                });

                            ui.add_space(16.0);
                            ui.horizontal(|ui| {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let btn = egui::Button::new(
                                            RichText::new("  Start Compression  ")
                                                .size(14.0)
                                                .color(Color32::BLACK)
                                                .strong(),
                                        )
                                        .fill(NEXUS_CYAN)
                                        .corner_radius(CornerRadius::same(6));

                                        if ui.add(btn).clicked() {
                                            start_requested = true;
                                        }
                                    },
                                );
                            });
                        });

                    if start_requested {
                        self.start_compression(input_clone, output_clone, active_level, size_val);
                    }
                }
                AppState::ExtractSetup {
                    input_path,
                    output_dir,
                    archive_size,
                } => {
                    let mut start_requested = false;
                    let input_clone = input_path.clone();
                    let output_clone = output_dir.clone();

                    egui::Frame::new()
                        .fill(BG_CARD)
                        .corner_radius(CornerRadius::same(8))
                        .stroke(Stroke::new(1.0_f32, BORDER_COLOR))
                        .inner_margin(Margin::same(16))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("📦").size(22.0));
                                ui.vertical(|ui| {
                                    ui.label(
                                        RichText::new(format!(
                                            "Archive: {}",
                                            input_clone
                                                .file_name()
                                                .unwrap_or_default()
                                                .to_string_lossy()
                                        ))
                                        .size(15.0)
                                        .strong(),
                                    );
                                    ui.label(
                                        RichText::new(format!(
                                            "Compressed Size: {}",
                                            format_bytes(*archive_size)
                                        ))
                                        .size(12.0)
                                        .color(TEXT_MUTED),
                                    );
                                });
                            });

                            ui.add_space(12.0);
                            ui.separator();
                            ui.add_space(12.0);

                            ui.label(
                                RichText::new("Extraction Target Directory:").size(13.0).strong(),
                            );
                            ui.horizontal(|ui| {
                                ui.text_edit_singleline(output_dir);
                                if ui.button("Browse...").clicked() {
                                    if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                                        *output_dir = folder.to_string_lossy().to_string();
                                    }
                                }
                            });

                            ui.add_space(24.0);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let btn = egui::Button::new(
                                        RichText::new("  Extract Archive  ")
                                            .size(14.0)
                                            .color(Color32::BLACK)
                                            .strong(),
                                    )
                                    .fill(NEXUS_PURPLE)
                                    .corner_radius(CornerRadius::same(6));

                                    if ui.add(btn).clicked() {
                                        start_requested = true;
                                    }
                                },
                            );
                        });

                    if start_requested {
                        self.start_decompression(input_clone, output_clone);
                    }
                }
                AppState::Running {
                    is_compressing,
                    source_name,
                    start_time,
                    latest_progress,
                    original_size,
                    ..
                } => {
                    egui::Frame::new()
                        .fill(BG_CARD)
                        .corner_radius(CornerRadius::same(8))
                        .stroke(Stroke::new(1.0_f32, BORDER_COLOR))
                        .inner_margin(Margin::same(20))
                        .show(ui, |ui| {
                            let action = if *is_compressing {
                                "Compressing"
                            } else {
                                "Extracting"
                            };
                            ui.label(
                                RichText::new(format!("{} '{}'...", action, source_name))
                                    .size(16.0)
                                    .strong(),
                            );
                            ui.add_space(16.0);

                            let (fraction, bytes_done, total_expected, stage) =
                                if let Some(ref p) = latest_progress {
                                    (p.fraction, p.bytes_processed, p.total_bytes, p.stage)
                                } else {
                                    (0.0, 0, *original_size, "Initializing...")
                                };

                            let progress_bar = egui::ProgressBar::new(fraction)
                                .show_percentage()
                                .animate(true);
                            ui.add(progress_bar);

                            ui.add_space(16.0);

                            let elapsed = start_time.elapsed();
                            let elapsed_secs = elapsed.as_secs_f64().max(0.001);
                            let speed_mb_s =
                                (bytes_done as f64 / (1024.0 * 1024.0)) / elapsed_secs;

                            let eta_str = if fraction > 0.02 && fraction < 0.99 {
                                let total_est_secs = elapsed_secs / fraction as f64;
                                let remaining_secs = (total_est_secs - elapsed_secs).max(0.0);
                                format_duration(Duration::from_secs_f64(remaining_secs))
                            } else {
                                "--:--".to_string()
                            };

                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!("Stage: {}", stage))
                                        .color(TEXT_MUTED)
                                        .size(13.0),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            RichText::new(format!(
                                                "{} / {}",
                                                format_bytes(bytes_done),
                                                format_bytes(total_expected)
                                            ))
                                            .strong()
                                            .size(13.0),
                                        );
                                    },
                                );
                            });

                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!(
                                        "Speed: {:.1} MB/s  |  Elapsed: {}",
                                        speed_mb_s,
                                        format_duration(elapsed)
                                    ))
                                    .color(TEXT_MUTED)
                                    .size(12.0),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            RichText::new(format!("ETA: {}", eta_str))
                                                .color(NEXUS_CYAN)
                                                .size(12.0),
                                        );
                                    },
                                );
                            });
                        });
                }
                AppState::Finished(success) => {
                    egui::Frame::new()
                        .fill(BG_CARD)
                        .corner_radius(CornerRadius::same(8))
                        .stroke(Stroke::new(1.0_f32, SUCCESS_GREEN))
                        .inner_margin(Margin::same(24))
                        .show(ui, |ui| {
                            ui.vertical_centered(|ui| {
                                ui.label(
                                    RichText::new("✨ Operation Complete!")
                                        .size(20.0)
                                        .color(SUCCESS_GREEN)
                                        .strong(),
                                );
                                ui.add_space(16.0);

                                if success.is_compression {
                                    let ratio = if success.original_size > 0 {
                                        (success.final_size as f64
                                            / success.original_size as f64)
                                            * 100.0
                                    } else {
                                        100.0
                                    };
                                    let saved = 100.0 - ratio;
                                    let mb_s = (success.original_size as f64 / (1024.0 * 1024.0))
                                        / success.elapsed.as_secs_f64().max(0.001);

                                    ui.label(
                                        RichText::new(format!(
                                            "Original Size:     {}",
                                            format_bytes(success.original_size)
                                        ))
                                        .size(14.0),
                                    );
                                    ui.label(
                                        RichText::new(format!(
                                            "Compressed Size:   {}",
                                            format_bytes(success.final_size)
                                        ))
                                        .size(14.0)
                                        .strong()
                                        .color(NEXUS_CYAN),
                                    );
                                    ui.label(
                                        RichText::new(format!(
                                            "Space Reduction:   {:.1}% saved",
                                            saved.max(0.0)
                                        ))
                                        .size(14.0)
                                        .color(SUCCESS_GREEN)
                                        .strong(),
                                    );
                                    ui.label(
                                        RichText::new(format!(
                                            "Time Taken:        {:.2?} ({:.1} MB/s)",
                                            success.elapsed, mb_s
                                        ))
                                        .size(13.0)
                                        .color(TEXT_MUTED),
                                    );
                                } else {
                                    ui.label(
                                        RichText::new(format!(
                                            "Archive Extracted Successfully to:\n{}",
                                            success.output_path.display()
                                        ))
                                        .size(14.0),
                                    );
                                    ui.label(
                                        RichText::new(format!(
                                            "Time Taken: {:.2?}",
                                            success.elapsed
                                        ))
                                        .size(13.0)
                                        .color(TEXT_MUTED),
                                    );
                                }

                                ui.add_space(24.0);
                                ui.horizontal(|ui| {
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui.button("  Close  ").clicked() {
                                                std::process::exit(0);
                                            }
                                            if ui.button("  Open Folder  ").clicked() {
                                                open_in_explorer(&success.output_path);
                                            }
                                        },
                                    );
                                });
                            });
                        });
                }
                AppState::Error(err_msg) => {
                    egui::Frame::new()
                        .fill(BG_CARD)
                        .corner_radius(CornerRadius::same(8))
                        .stroke(Stroke::new(1.0_f32, Color32::RED))
                        .inner_margin(Margin::same(24))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new("⚠️ Operation Failed")
                                    .size(18.0)
                                    .color(Color32::RED)
                                    .strong(),
                            );
                            ui.add_space(12.0);
                            ui.label(RichText::new(err_msg.as_str()).size(13.0));
                            ui.add_space(20.0);
                            if ui.button("Close").clicked() {
                                std::process::exit(1);
                            }
                        });
                }
            }
        });
    }
}

impl NexusGuiApp {
    fn render_home(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new("Neural EXtreme Unified compresSion")
                    .size(14.0)
                    .color(TEXT_MUTED),
            );
            ui.add_space(24.0);
        });

        ui.horizontal(|ui| {
            // Card 1: Compress
            egui::Frame::new()
                .fill(BG_CARD)
                .corner_radius(CornerRadius::same(8))
                .stroke(Stroke::new(1.0_f32, BORDER_COLOR))
                .inner_margin(Margin::same(20))
                .show(ui, |ui| {
                    ui.set_width(280.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("🗜️").size(36.0));
                        ui.add_space(8.0);
                        ui.label(RichText::new("Compress Files / Folders").size(15.0).strong());
                        ui.label(
                            RichText::new("Create high-ratio solid .nxar archives")
                                .size(11.0)
                                .color(TEXT_MUTED),
                        );
                        ui.add_space(16.0);
                        if ui.button("Select File...").clicked() {
                            if let Some(file) = rfd::FileDialog::new().pick_file() {
                                self.state = Self::setup_compress(file, None);
                            }
                        }
                        if ui.button("Select Folder...").clicked() {
                            if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                                self.state = Self::setup_compress(folder, None);
                            }
                        }
                    });
                });

            ui.add_space(16.0);

            // Card 2: Decompress
            egui::Frame::new()
                .fill(BG_CARD)
                .corner_radius(CornerRadius::same(8))
                .stroke(Stroke::new(1.0_f32, BORDER_COLOR))
                .inner_margin(Margin::same(20))
                .show(ui, |ui| {
                    ui.set_width(280.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("📦").size(36.0));
                        ui.add_space(8.0);
                        ui.label(RichText::new("Extract Archive").size(15.0).strong());
                        ui.label(
                            RichText::new("Decompress and restore .nxar / .nxs files")
                                .size(11.0)
                                .color(TEXT_MUTED),
                        );
                        ui.add_space(16.0);
                        if ui.button("Select .nxar Archive...").clicked() {
                            if let Some(archive) = rfd::FileDialog::new()
                                .add_filter("NEXUS Archives (*.nxar, *.nxs)", &["nxar", "nxs"])
                                .pick_file()
                            {
                                self.state = Self::setup_decompress(archive);
                            }
                        }
                    });
                });
        });
    }
}

fn render_header(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("NEXUS")
                .size(20.0)
                .color(NEXUS_CYAN)
                .strong(),
        );
        ui.label(
            RichText::new("ARCHIVER")
                .size(20.0)
                .color(NEXUS_PURPLE)
                .strong(),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                RichText::new("v0.1.0")
                    .size(11.0)
                    .color(TEXT_MUTED),
            );
        });
    });
}

fn preset_card(
    ui: &mut egui::Ui,
    is_selected: bool,
    title: &str,
    level_label: &str,
    speed_label: &str,
    desc: &str,
) -> bool {
    let stroke_color = if is_selected {
        NEXUS_CYAN
    } else {
        BORDER_COLOR
    };
    let bg_color = if is_selected {
        BG_CARD_HOVER
    } else {
        BG_CARD
    };

    let frame = egui::Frame::new()
        .fill(bg_color)
        .corner_radius(CornerRadius::same(6))
        .stroke(Stroke::new(
            if is_selected { 2.0_f32 } else { 1.0_f32 },
            stroke_color,
        ))
        .inner_margin(Margin::same(10));

    let response = frame.show(ui, |ui| {
        ui.set_width(135.0);
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                let bullet = if is_selected { "🔘" } else { "⚪" };
                ui.label(RichText::new(bullet).size(11.0));
                ui.label(RichText::new(title).size(12.0).strong());
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!("{} • {}", level_label, speed_label))
                    .size(10.0)
                    .color(if is_selected {
                        NEXUS_CYAN
                    } else {
                        TEXT_MUTED
                    }),
            );
            ui.label(RichText::new(desc).size(9.5).color(TEXT_MUTED));
        });
    });

    response.response.interact(egui::Sense::click()).clicked()
}

fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;

    let b = bytes as f64;
    if b >= GB {
        format!("{:.2} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{} B", bytes)
    }
}

fn format_duration(d: Duration) -> String {
    let total_secs = d.as_secs();
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    format!("{:02}:{:02}", mins, secs)
}

fn open_in_explorer(path: &Path) {
    #[cfg(target_os = "windows")]
    {
        if path.is_file() {
            let _ = std::process::Command::new("explorer")
                .arg(format!("/select,{}", path.display()))
                .spawn();
        } else {
            let _ = std::process::Command::new("explorer")
                .arg(path)
                .spawn();
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let target = if path.is_file() {
            path.parent().unwrap_or(path)
        } else {
            path
        };
        let _ = std::process::Command::new("xdg-open").arg(target).spawn();
    }
}

enum InitialAction {
    Compress {
        path: PathBuf,
        preset: Option<CompressionPreset>,
        auto_start: bool,
    },
    Decompress {
        path: PathBuf,
        auto_start: bool,
    },
}

fn parse_cli_args() -> Option<InitialAction> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        return None;
    }

    if args[1] == "compress" && args.len() >= 3 {
        let path = PathBuf::from(&args[2]);
        let mut preset = None;
        let mut auto_start = false;

        for (i, arg) in args.iter().enumerate() {
            if arg == "--auto-start" {
                auto_start = true;
            }
            if arg == "--preset" && i + 1 < args.len() {
                match args[i + 1].to_lowercase().as_str() {
                    "low" | "lowest" | "fast" => preset = Some(CompressionPreset::Low),
                    "medium" | "med" | "balanced" => preset = Some(CompressionPreset::Medium),
                    "high" | "highest" | "max" => preset = Some(CompressionPreset::High),
                    _ => {}
                }
            }
        }
        Some(InitialAction::Compress {
            path,
            preset,
            auto_start,
        })
    } else if args[1] == "decompress" && args.len() >= 3 {
        let path = PathBuf::from(&args[2]);
        let auto_start = args
            .iter()
            .any(|a| a == "--auto-start" || a == "--extract-here");
        Some(InitialAction::Decompress { path, auto_start })
    } else {
        let path = PathBuf::from(&args[1]);
        if let Some(ext) = path.extension() {
            if ext == "nxar" || ext == "nxs" {
                return Some(InitialAction::Decompress {
                    path,
                    auto_start: false,
                });
            }
        }
        Some(InitialAction::Compress {
            path,
            preset: None,
            auto_start: false,
        })
    }
}

fn main() -> eframe::Result<()> {
    let initial_action = parse_cli_args();

    let icon_data =
        eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/nexus_logo.png")).ok();

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size(Vec2::new(680.0, 580.0))
        .with_min_inner_size(Vec2::new(600.0, 500.0))
        .with_resizable(true)
        .with_title("NEXUS Archiver");

    if let Some(icon) = icon_data {
        viewport = viewport.with_icon(icon);
    }

    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "NEXUS Archiver",
        native_options,
        Box::new(move |cc| Ok(Box::new(NexusGuiApp::new(cc, initial_action)))),
    )
}
