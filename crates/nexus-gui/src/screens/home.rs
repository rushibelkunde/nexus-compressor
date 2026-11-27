//! Home screen: a large drop zone plus explicit file pickers.

use eframe::egui::{self, pos2, vec2, Align2, Rect, RichText, Sense, Shape, Stroke};

use super::Action;
use crate::theme::{
    mono, sans, BG_PANEL, BORDER_STRONG, NEXUS_CYAN, NEXUS_PURPLE_TEXT, TEXT, TEXT_MUTED,
};
use crate::widgets::icons::{self, Icon};
use crate::widgets::{panel, secondary_button, show_notice, Notice};

/// State of the home screen.
#[derive(Default)]
pub struct HomeState {
    /// Message shown above the drop zone, if any.
    pub notice: Option<Notice>,
}

/// Paints the home screen.
pub fn show(state: &mut HomeState, ui: &mut egui::Ui) -> Option<Action> {
    let mut action = None;
    show_notice(ui, state.notice.as_ref());

    drop_zone(ui);
    ui.add_space(8.0);

    ui.columns(2, |columns| {
        panel(&mut columns[0], |ui| {
            ui.set_min_height(92.0);
            card_heading(ui, Icon::Press, "COMPRESS", NEXUS_CYAN);
            ui.label(
                RichText::new("Pack a file or a whole folder into a .nxar archive.")
                    .font(sans(12.0))
                    .color(TEXT_MUTED),
            );
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if secondary_button(ui, "Select File\u{2026}").clicked() {
                    if let Some(file) = rfd::FileDialog::new().pick_file() {
                        action = Some(Action::OpenSource(file));
                    }
                }
                if secondary_button(ui, "Select Folder\u{2026}").clicked() {
                    if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                        action = Some(Action::OpenSource(folder));
                    }
                }
            });
        });

        panel(&mut columns[1], |ui| {
            ui.set_min_height(92.0);
            card_heading(ui, Icon::Unpack, "EXTRACT / TEST", NEXUS_PURPLE_TEXT);
            ui.label(
                RichText::new("Restore a .nxar / .nxs archive or check its integrity.")
                    .font(sans(12.0))
                    .color(TEXT_MUTED),
            );
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if secondary_button(ui, "Select Archive\u{2026}").clicked() {
                    if let Some(archive) = rfd::FileDialog::new()
                        .add_filter("NEXUS Archives (*.nxar, *.nxs)", &["nxar", "nxs"])
                        .pick_file()
                    {
                        action = Some(Action::OpenArchive(archive));
                    }
                }
            });
        });
    });

    action
}

fn card_heading(ui: &mut egui::Ui, icon: Icon, title: &str, color: egui::Color32) {
    ui.horizontal(|ui| {
        icons::icon(ui, icon, 20.0, color);
        ui.label(RichText::new(title).font(mono(13.0)).color(TEXT));
    });
}

// The dashed target area. Dropping works anywhere on the window; this zone
// exists to say so.
fn drop_zone(ui: &mut egui::Ui) {
    let width = ui.available_width().max(120.0);
    // Use the spare height of the window, within sensible limits.
    let height = (ui.available_height() - 150.0).clamp(130.0, 260.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 2.0, BG_PANEL);
    let frame = rect.shrink(1.0);
    let outline = [
        frame.left_top(),
        frame.right_top(),
        frame.right_bottom(),
        frame.left_bottom(),
        frame.left_top(),
    ];
    painter.extend(Shape::dashed_line(
        &outline,
        Stroke::new(1.0_f32, BORDER_STRONG),
        8.0,
        6.0,
    ));

    let center = rect.center();
    let icon_size = 44.0;
    icons::paint(
        &painter,
        Rect::from_center_size(pos2(center.x, center.y - 30.0), vec2(icon_size, icon_size)),
        Icon::Press,
        NEXUS_CYAN,
    );
    painter.text(
        pos2(center.x, center.y + 14.0),
        Align2::CENTER_CENTER,
        "DROP A FILE OR FOLDER HERE",
        mono(15.0),
        TEXT,
    );
    painter.text(
        pos2(center.x, center.y + 36.0),
        Align2::CENTER_CENTER,
        "anything to compress  \u{00B7}  a .nxar / .nxs archive to extract or test",
        sans(12.0),
        TEXT_MUTED,
    );
}
