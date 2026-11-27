//! The screens of the application and the vocabulary they share.
//!
//! Each screen owns its state and has a `show` function that paints it and
//! returns at most one [`Action`]. The state machine in [`crate::app`]
//! applies the action; screens never switch screens themselves.
//!
//! * [`home`] — drop zone and file pickers
//! * [`compress_setup`] — destination, preset / level, estimates
//! * [`extract_setup`] — archive facts, destination, test archive
//! * [`running`] — the live press view of a running job
//! * [`finished`] — result of a completed job
//! * [`failed`] — error report
//! * [`drop_overlay`] — full-window overlay while files hover the window

pub mod compress_setup;
pub mod drop_overlay;
pub mod extract_setup;
pub mod failed;
pub mod finished;
pub mod home;
pub mod running;

#[cfg(test)]
mod layout_tests;

use std::path::PathBuf;

use eframe::egui::Color32;

use crate::theme::{NEXUS_CYAN, NEXUS_PURPLE};
use crate::widgets::icons::Icon;
use compress_setup::CompressSetup;
use extract_setup::ExtractSetup;

/// Fill colour of bars that stand for raw (uncompressed) data.
pub const RAW_DATA_COLOR: Color32 = Color32::from_rgb(124, 136, 166);

/// A request from a screen to the application state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Return to the home screen.
    GoHome,
    /// Open the compression setup for this file or folder.
    OpenSource(PathBuf),
    /// Open the extraction setup for this archive.
    OpenArchive(PathBuf),
    /// Start compressing with the current compression setup.
    StartCompress,
    /// Start extracting with the current extraction setup.
    StartExtract,
    /// Start verifying the archive of the current extraction setup.
    StartVerify,
    /// Ask the running job to stop.
    CancelJob,
    /// Return from a result screen to the setup the job was started from.
    BackToSetup,
    /// Close the application.
    Quit,
    /// Close the application after a failure (non-zero exit code).
    QuitAfterFailure,
}

/// The kind of job a running / finished screen is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobMode {
    /// Creating an archive.
    Compress,
    /// Restoring an archive's contents.
    Extract,
    /// Checking an archive without writing anything.
    Verify,
}

impl JobMode {
    /// Present-participle heading, e.g. `COMPRESSING`.
    pub fn verb(self) -> &'static str {
        match self {
            JobMode::Compress => "COMPRESSING",
            JobMode::Extract => "EXTRACTING",
            JobMode::Verify => "VERIFYING",
        }
    }

    /// Brand accent of the mode: cyan for compression, purple otherwise.
    pub fn accent(self) -> Color32 {
        match self {
            JobMode::Compress => NEXUS_CYAN,
            JobMode::Extract | JobMode::Verify => NEXUS_PURPLE,
        }
    }

    /// Pictogram of the mode.
    pub fn icon(self) -> Icon {
        match self {
            JobMode::Compress => Icon::Press,
            JobMode::Extract => Icon::Unpack,
            JobMode::Verify => Icon::Shield,
        }
    }
}

/// The setup screen a job was started from, kept so that cancelling (or
/// going back from a result) restores it exactly as the user left it.
pub enum Origin {
    /// Started from the compression setup.
    Compress(Box<CompressSetup>),
    /// Started from the extraction setup.
    Extract(Box<ExtractSetup>),
}
