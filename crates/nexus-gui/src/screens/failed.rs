//! Error screen shown when a job fails (including a failed verification,
//! which means the archive is damaged).

use eframe::egui::{self, RichText};

use super::{Action, Origin};
use crate::theme::{mono, sans, DANGER, TEXT, TEXT_MUTED};
use crate::widgets::icons::{self, Icon};
use crate::widgets::{panel, primary_button, secondary_button};

/// State of the error screen.
pub struct FailedJob {
    /// Heading, e.g. `COMPRESSION FAILED`.
    pub title: String,
    /// The error message from the core.
    pub message: String,
    /// Optional plain-language explanation shown under the message.
    pub hint: Option<String>,
    /// The setup screen the job was started from, if it can be returned to.
    pub origin: Option<Origin>,
}

impl FailedJob {
    /// Paints the screen.
    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        panel(ui, |ui| {
            ui.horizontal(|ui| {
                icons::icon(ui, Icon::Cross, 22.0, DANGER);
                ui.label(RichText::new(&self.title).font(mono(15.0)).color(DANGER));
            });
            ui.add_space(6.0);
            ui.add(
                egui::Label::new(RichText::new(&self.message).font(sans(13.0)).color(TEXT))
                    .wrap()
                    .selectable(true),
            );
            if let Some(hint) = &self.hint {
                ui.add_space(4.0);
                ui.add(
                    egui::Label::new(RichText::new(hint).font(sans(12.0)).color(TEXT_MUTED)).wrap(),
                );
            }
        });
        None
    }

    /// Paints the action row pinned to the bottom of the window.
    pub fn footer(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        ui.horizontal(|ui| {
            if secondary_button(ui, "Close").clicked() {
                action = Some(Action::QuitAfterFailure);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.origin.is_some()
                    && primary_button(ui, true, "BACK TO SETUP", crate::theme::NEXUS_CYAN).clicked()
                {
                    action = Some(Action::BackToSetup);
                }
                if secondary_button(ui, "Home").clicked() {
                    action = Some(Action::GoHome);
                }
            });
        });
        action
    }
}
