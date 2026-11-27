//! NEXUS Archiver — the desktop GUI of the NEXUS compressor.
//!
//! The binary is a thin shell around `nexus-core`: it parses the arguments
//! passed by the OS shell integration, opens the window and hands control to
//! the state machine in [`app`].
//!
//! Module map:
//!
//! * [`app`] — state machine, drag-and-drop routing, repaint policy
//! * [`screens`] — one module per screen
//! * [`widgets`] — custom-painted instruments and chrome
//! * [`theme`] — colours, fonts, egui visuals
//! * [`worker`] — job threads, progress channels, cancellation
//! * [`analysis`] — background source scan and size estimates
//! * [`telemetry`] — live statistics derived from progress updates
//! * [`methods`] — compression-method families and their colours
//! * [`engine`] — what each level means (engine, speed, block size)
//! * [`cli_args`] — command-line entry points
//! * [`paths`] — routing by extension and default destinations
//! * [`shell`] — file-manager hand-off
//! * [`util`] — formatting helpers

#![windows_subsystem = "windows"]

mod analysis;
mod app;
mod cli_args;
mod engine;
mod methods;
mod paths;
mod screens;
mod shell;
mod telemetry;
mod theme;
mod util;
mod widgets;
mod worker;

use eframe::egui::{self, Vec2};

use app::NexusGuiApp;

/// Default inner size of the window in points.
pub(crate) const WINDOW_SIZE: Vec2 = Vec2::new(680.0, 580.0);
/// Smallest inner size the window can be resized to.
pub(crate) const WINDOW_MIN_SIZE: Vec2 = Vec2::new(600.0, 500.0);

fn main() -> eframe::Result<()> {
    let initial_action = cli_args::parse_cli_args();

    let icon_data =
        eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/nexus_logo.png")).ok();

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size(WINDOW_SIZE)
        .with_min_inner_size(WINDOW_MIN_SIZE)
        .with_resizable(true)
        .with_drag_and_drop(true)
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
