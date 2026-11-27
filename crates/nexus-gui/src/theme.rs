//! Visual identity of the NEXUS desktop GUI: colour tokens, font helpers and
//! the egui `Visuals` shared by every screen.
//!
//! The look is deliberately instrument-like: near-black wells, thin 1 px
//! borders, square corners and monospaced numerals. Chart colours for the
//! compression methods live in [`crate::methods`]; this module only holds
//! chrome, text and status colours.

use eframe::egui::{self, Color32, CornerRadius, FontId, Stroke};

/// Primary brand accent (compression, primary actions).
pub const NEXUS_CYAN: Color32 = Color32::from_rgb(0, 229, 255);
/// Secondary brand accent (extraction). Use [`NEXUS_PURPLE_TEXT`] for text.
pub const NEXUS_PURPLE: Color32 = Color32::from_rgb(157, 78, 221);
/// Lightened purple that keeps readable contrast when used for small text.
pub const NEXUS_PURPLE_TEXT: Color32 = Color32::from_rgb(190, 140, 242);

/// Window background.
pub const BG_DARK: Color32 = Color32::from_rgb(11, 13, 18);
/// Background of panels that sit on the window.
pub const BG_PANEL: Color32 = Color32::from_rgb(19, 22, 30);
/// Background of raised or selected elements inside a panel.
pub const BG_RAISED: Color32 = Color32::from_rgb(29, 33, 46);
/// Background of hovered interactive elements.
pub const BG_HOVER: Color32 = Color32::from_rgb(38, 43, 60);
/// Background of recessed wells: gauge tracks, text fields, chart areas.
pub const BG_INSET: Color32 = Color32::from_rgb(7, 8, 12);

/// Default 1 px border.
pub const BORDER: Color32 = Color32::from_rgb(46, 52, 70);
/// Emphasised border and tick marks.
pub const BORDER_STRONG: Color32 = Color32::from_rgb(78, 86, 110);

/// Primary text.
pub const TEXT: Color32 = Color32::from_rgb(232, 236, 245);
/// Secondary text (captions, descriptions).
pub const TEXT_MUTED: Color32 = Color32::from_rgb(156, 164, 186);
/// Tertiary text (section labels, scale numerals).
pub const TEXT_DIM: Color32 = Color32::from_rgb(118, 126, 150);

/// Status: success.
pub const SUCCESS: Color32 = Color32::from_rgb(76, 217, 100);
/// Status: warning.
pub const WARNING: Color32 = Color32::from_rgb(240, 180, 50);
/// Status: failure / destructive action.
pub const DANGER: Color32 = Color32::from_rgb(255, 99, 99);

/// Corner radius used by every panel, button and gauge.
pub const RADIUS: CornerRadius = CornerRadius::same(2);

/// Monospaced font, used for every number and section label.
pub fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

/// Proportional font, used for names and prose.
pub fn sans(size: f32) -> FontId {
    FontId::proportional(size)
}

/// Returns `color` with its alpha replaced by `alpha` (0 = transparent).
pub fn with_alpha(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

/// Linear blend from `a` (`t = 0`) to `b` (`t = 1`); `t` is clamped.
pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = if t.is_finite() {
        t.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let mix = |x: u8, y: u8| -> u8 {
        (f32::from(x) + (f32::from(y) - f32::from(x)) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

/// Installs the NEXUS dark visuals on the egui context.
pub fn apply(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.override_text_color = Some(TEXT);
    visuals.panel_fill = BG_DARK;
    visuals.window_fill = BG_PANEL;
    visuals.window_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.extreme_bg_color = BG_INSET;
    visuals.faint_bg_color = BG_PANEL;
    visuals.hyperlink_color = NEXUS_CYAN;
    visuals.selection.bg_fill = with_alpha(NEXUS_CYAN, 70);
    visuals.selection.stroke = Stroke::new(1.0_f32, NEXUS_CYAN);

    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = BG_PANEL;
    widgets.noninteractive.weak_bg_fill = BG_PANEL;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT_MUTED);
    widgets.noninteractive.corner_radius = RADIUS;

    widgets.inactive.bg_fill = BG_RAISED;
    widgets.inactive.weak_bg_fill = BG_RAISED;
    widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    widgets.inactive.corner_radius = RADIUS;

    widgets.hovered.bg_fill = BG_HOVER;
    widgets.hovered.weak_bg_fill = BG_HOVER;
    widgets.hovered.bg_stroke = Stroke::new(1.0_f32, BORDER_STRONG);
    widgets.hovered.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
    widgets.hovered.corner_radius = RADIUS;
    widgets.hovered.expansion = 0.0;

    widgets.active.bg_fill = BG_HOVER;
    widgets.active.weak_bg_fill = BG_HOVER;
    widgets.active.bg_stroke = Stroke::new(1.0_f32, NEXUS_CYAN);
    widgets.active.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
    widgets.active.corner_radius = RADIUS;
    widgets.active.expansion = 0.0;

    widgets.open.bg_fill = BG_RAISED;
    widgets.open.weak_bg_fill = BG_RAISED;
    widgets.open.corner_radius = RADIUS;

    ctx.set_visuals(visuals);

    ctx.style_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lerp_color_endpoints_and_midpoint() {
        let a = Color32::from_rgb(0, 100, 200);
        let b = Color32::from_rgb(100, 200, 0);
        assert_eq!(lerp_color(a, b, 0.0), a);
        assert_eq!(lerp_color(a, b, 1.0), b);
        assert_eq!(lerp_color(a, b, 0.5), Color32::from_rgb(50, 150, 100));
    }

    #[test]
    fn test_lerp_color_clamps_out_of_range_and_nan() {
        let a = Color32::from_rgb(10, 20, 30);
        let b = Color32::from_rgb(200, 210, 220);
        assert_eq!(lerp_color(a, b, -3.0), a);
        assert_eq!(lerp_color(a, b, 7.0), b);
        assert_eq!(lerp_color(a, b, f32::NAN), a);
    }

    #[test]
    fn test_with_alpha_keeps_opaque_colour_at_full_alpha() {
        assert_eq!(with_alpha(NEXUS_CYAN, 255), NEXUS_CYAN);
        assert_eq!(with_alpha(NEXUS_CYAN, 0).a(), 0);
    }
}
