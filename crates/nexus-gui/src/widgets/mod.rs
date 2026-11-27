//! Custom-painted building blocks shared by the screens.
//!
//! This module holds the chrome (panels, labels, buttons, notices, preset
//! cards); the sub-modules hold the instruments:
//!
//! * [`icons`] — vector pictograms
//! * [`level_rail`] — the 1–19 level selector
//! * [`press`] — the original-vs-compressed "press" gauge
//! * [`charts`] — progress gauge, throughput sparkline, block map and
//!   method breakdown

pub mod charts;
pub mod icons;
pub mod level_rail;
pub mod press;

use eframe::egui::{
    self, pos2, vec2, Align2, Color32, FontId, Margin, Rect, RichText, Sense, Stroke, StrokeKind,
    TextWrapMode,
};

use crate::theme::{
    self, mono, sans, BG_DARK, BG_HOVER, BG_PANEL, BG_RAISED, BORDER, BORDER_STRONG, DANGER,
    NEXUS_CYAN, NEXUS_PURPLE, NEXUS_PURPLE_TEXT, RADIUS, SUCCESS, TEXT, TEXT_DIM, TEXT_MUTED,
    WARNING,
};
use crate::util::elide_middle;
use icons::Icon;

/// Height of every action button.
pub const BUTTON_HEIGHT: f32 = 32.0;

/// Severity of a [`notice`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
    /// Neutral information.
    Info,
    /// Something succeeded.
    Success,
    /// Something the user should be aware of before continuing.
    Warning,
    /// Something failed.
    Error,
}

impl NoticeKind {
    fn color(self) -> Color32 {
        match self {
            NoticeKind::Info => NEXUS_CYAN,
            NoticeKind::Success => SUCCESS,
            NoticeKind::Warning => WARNING,
            NoticeKind::Error => DANGER,
        }
    }

    fn icon(self) -> Icon {
        match self {
            NoticeKind::Info => Icon::Info,
            NoticeKind::Success => Icon::Check,
            NoticeKind::Warning => Icon::Warning,
            NoticeKind::Error => Icon::Cross,
        }
    }
}

/// A message shown on a setup screen, e.g. "cancelled" after a cancelled job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// Severity (decides colour and icon).
    pub kind: NoticeKind,
    /// Text to show.
    pub text: String,
}

impl Notice {
    /// Creates a notice.
    pub fn new(kind: NoticeKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
        }
    }
}

/// A bordered panel spanning the available width.
pub fn panel<R>(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(BG_PANEL)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .corner_radius(RADIUS)
        .inner_margin(Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add_contents(ui)
        })
        .inner
}

/// A small upper-case section label, e.g. `THROUGHPUT`.
pub fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.add(
        egui::Label::new(
            RichText::new(text.to_uppercase())
                .font(mono(10.5))
                .color(TEXT_DIM),
        )
        .truncate(),
    );
}

/// Width of `text` when laid out on one line in `font`.
pub fn text_width(ui: &egui::Ui, text: &str, font: &FontId) -> f32 {
    ui.fonts(|f| {
        f.layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE)
            .size()
            .x
    })
}

/// Returns `text`, elided in the middle if needed, so that `measure` reports
/// it as at most `max_width` wide. `measure` must grow with the text length.
/// When not even a lone ellipsis fits, the ellipsis is returned anyway.
pub fn fit_middle_with(text: &str, max_width: f32, measure: impl Fn(&str) -> f32) -> String {
    if !max_width.is_finite() || measure(text) <= max_width {
        return text.to_owned();
    }
    // Binary-search the longest elision that fits.
    let (mut low, mut high) = (1usize, text.chars().count());
    while low < high {
        let mid = (low + high).div_ceil(2);
        if measure(&elide_middle(text, mid)) <= max_width {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    elide_middle(text, low)
}

/// Returns `text`, elided in the middle if needed, so that it is at most
/// `max_width` wide in `font`.
pub fn fit_middle(ui: &egui::Ui, text: &str, font: &FontId, max_width: f32) -> String {
    fit_middle_with(text, max_width, |candidate| text_width(ui, candidate, font))
}

/// A single-line label that elides its middle to fit the available width
/// and shows the full text on hover when it had to.
pub fn elided_label(ui: &mut egui::Ui, text: &str, font: FontId, color: Color32) {
    let fitted = fit_middle(ui, text, &font, ui.available_width());
    let was_elided = fitted != text;
    let response =
        ui.add(egui::Label::new(RichText::new(fitted).font(font).color(color)).truncate());
    if was_elided {
        response.on_hover_text(text);
    }
}

fn accent_text_color(accent: Color32) -> Color32 {
    if accent == NEXUS_PURPLE {
        Color32::WHITE
    } else {
        BG_DARK
    }
}

/// The main call-to-action button of a screen, filled with `accent`.
pub fn primary_button(
    ui: &mut egui::Ui,
    enabled: bool,
    text: &str,
    accent: Color32,
) -> egui::Response {
    let button = egui::Button::new(
        RichText::new(text)
            .font(mono(13.0))
            .color(accent_text_color(accent)),
    )
    .fill(accent)
    .stroke(Stroke::NONE)
    .corner_radius(RADIUS)
    .min_size(vec2(0.0, BUTTON_HEIGHT));
    ui.add_enabled(enabled, button)
}

/// A neutral secondary button.
pub fn secondary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(text).font(sans(13.0)).color(TEXT))
            .corner_radius(RADIUS)
            .min_size(vec2(0.0, BUTTON_HEIGHT)),
    )
}

/// An outlined button for destructive actions such as cancelling a job.
pub fn danger_button(ui: &mut egui::Ui, enabled: bool, text: &str) -> egui::Response {
    let button = egui::Button::new(RichText::new(text).font(mono(13.0)).color(DANGER))
        .fill(BG_PANEL)
        .stroke(Stroke::new(1.0_f32, DANGER))
        .corner_radius(RADIUS)
        .min_size(vec2(0.0, BUTTON_HEIGHT));
    ui.add_enabled(enabled, button)
}

/// One readout of a [`stat_strip`].
pub struct Stat<'a> {
    /// Caption above the value.
    pub label: &'a str,
    /// The value text.
    pub value: String,
    /// Colour of the value text.
    pub color: Color32,
}

impl<'a> Stat<'a> {
    /// A readout in the default text colour.
    pub fn new(label: &'a str, value: impl Into<String>) -> Self {
        Self {
            label,
            value: value.into(),
            color: TEXT,
        }
    }

    /// Sets the colour of the value text.
    pub fn colored(mut self, color: Color32) -> Self {
        self.color = color;
        self
    }
}

/// A panel with equally wide labelled readouts. Texts that do not fit their
/// cell are truncated rather than allowed to overlap the next cell.
pub fn stat_strip(ui: &mut egui::Ui, stats: &[Stat<'_>]) {
    if stats.is_empty() {
        return;
    }
    panel(ui, |ui| {
        ui.columns(stats.len(), |columns| {
            for (column, stat) in columns.iter_mut().zip(stats) {
                column.style_mut().wrap_mode = Some(TextWrapMode::Truncate);
                column.spacing_mut().item_spacing.y = 3.0;
                column.label(
                    RichText::new(stat.label.to_uppercase())
                        .font(mono(10.5))
                        .color(TEXT_DIM),
                );
                column
                    .label(
                        RichText::new(&stat.value)
                            .font(mono(15.0))
                            .color(stat.color),
                    )
                    .on_hover_text(&stat.value);
            }
        });
    });
}

/// A full-width message bar with an icon, e.g. a warning before starting.
pub fn notice(ui: &mut egui::Ui, kind: NoticeKind, text: &str) {
    let color = kind.color();
    let response = egui::Frame::new()
        .fill(theme::lerp_color(BG_PANEL, color, 0.08))
        .stroke(Stroke::new(1.0_f32, theme::lerp_color(BORDER, color, 0.55)))
        .corner_radius(RADIUS)
        .inner_margin(Margin {
            left: 36,
            right: 12,
            top: 8,
            bottom: 8,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(RichText::new(text).font(sans(12.5)).color(TEXT)).wrap());
        })
        .response;
    // The icon sits in the widened left margin so the text can wrap freely.
    let icon_rect = Rect::from_min_size(
        pos2(response.rect.left() + 11.0, response.rect.top() + 8.0),
        vec2(16.0, 16.0),
    );
    if ui.is_rect_visible(icon_rect) {
        icons::paint(ui.painter(), icon_rect, kind.icon(), color);
    }
}

/// Shows a [`Notice`] if there is one, followed by a small gap.
pub fn show_notice(ui: &mut egui::Ui, current: Option<&Notice>) {
    if let Some(n) = current {
        notice(ui, n.kind, &n.text);
        ui.add_space(4.0);
    }
}

/// The application header: brand, version and a ruled line.
pub fn brand_header(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(RichText::new("NEXUS").font(mono(19.0)).color(NEXUS_CYAN));
        ui.label(
            RichText::new("ARCHIVER")
                .font(mono(19.0))
                .color(NEXUS_PURPLE_TEXT),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                RichText::new(concat!("v", env!("CARGO_PKG_VERSION")))
                    .font(mono(11.0))
                    .color(TEXT_DIM),
            );
        });
    });

    // A ruled line with graduation ticks, like the edge of an instrument.
    let width = ui.available_width().max(1.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, 7.0), Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter().with_clip_rect(rect);
        painter.hline(
            rect.x_range(),
            rect.top() + 0.5,
            Stroke::new(1.0_f32, BORDER_STRONG),
        );
        let step = 8.0;
        let count = ((rect.width() / step).floor() as usize).min(1000);
        for i in 0..=count {
            let x = rect.left() + i as f32 * step;
            let len = if i.is_multiple_of(10) { 6.0 } else { 3.0 };
            painter.vline(
                x,
                rect.top()..=rect.top() + len,
                Stroke::new(1.0_f32, BORDER),
            );
        }
    }
}

/// Paints animated diagonal "activity" stripes over `rect`.
pub(crate) fn paint_stripes(painter: &egui::Painter, rect: Rect, time: f64, color: Color32) {
    if !rect.is_finite() || rect.width() <= 1.0 || rect.height() <= 1.0 {
        return;
    }
    const PERIOD: f32 = 14.0;
    const SPEED: f64 = 22.0;
    let painter = painter.with_clip_rect(rect);
    let offset = ((time * SPEED) % f64::from(PERIOD)) as f32;
    let height = rect.height();
    let mut x = rect.left() - height - PERIOD + offset;
    let mut guard = 0;
    while x < rect.right() && guard < 2000 {
        painter.line_segment(
            [pos2(x, rect.bottom()), pos2(x + height, rect.top())],
            Stroke::new(3.0_f32, color),
        );
        x += PERIOD;
        guard += 1;
    }
}

/// Contents of a [`preset_card`].
pub struct PresetCard<'a> {
    /// Pictogram in the top-left corner.
    pub icon: Icon,
    /// Card title, e.g. `Balanced`.
    pub title: &'a str,
    /// Monospaced facts line, e.g. `LEVEL 6 · ~14 MB/s per core`.
    pub facts: &'a str,
    /// One-line description.
    pub tagline: &'a str,
}

/// A selectable preset card filling the available width. Returns the click
/// response.
pub fn preset_card(ui: &mut egui::Ui, selected: bool, card: &PresetCard<'_>) -> egui::Response {
    const HEIGHT: f32 = 70.0;
    let width = ui.available_width().max(60.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, HEIGHT), Sense::click());
    if !ui.is_rect_visible(rect) {
        return response;
    }

    let hovered = response.hovered();
    let fill = if selected {
        BG_RAISED
    } else if hovered {
        BG_HOVER
    } else {
        BG_PANEL
    };
    let border = if selected {
        NEXUS_CYAN
    } else if hovered || response.has_focus() {
        BORDER_STRONG
    } else {
        BORDER
    };
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect(
        rect,
        RADIUS,
        fill,
        Stroke::new(1.0_f32, border),
        StrokeKind::Inside,
    );
    if selected {
        // Accent bar along the left edge marks the active preset without
        // relying on the border colour alone.
        painter.rect_filled(
            Rect::from_min_max(rect.left_top(), pos2(rect.left() + 3.0, rect.bottom())),
            0.0,
            NEXUS_CYAN,
        );
    }

    let accent = if selected { NEXUS_CYAN } else { TEXT_MUTED };
    let left = rect.left() + 12.0;
    let text_clip = painter.with_clip_rect(rect.shrink2(vec2(8.0, 0.0)));
    icons::paint(
        &painter,
        Rect::from_min_size(pos2(left, rect.top() + 9.0), vec2(18.0, 18.0)),
        card.icon,
        accent,
    );
    text_clip.text(
        pos2(left + 26.0, rect.top() + 18.0),
        Align2::LEFT_CENTER,
        card.title,
        sans(14.0),
        TEXT,
    );
    text_clip.text(
        pos2(left, rect.top() + 41.0),
        Align2::LEFT_CENTER,
        card.facts,
        mono(10.5),
        accent,
    );
    text_clip.text(
        pos2(left, rect.top() + 57.0),
        Align2::LEFT_CENTER,
        card.tagline,
        sans(11.0),
        TEXT_MUTED,
    );

    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A fake font in which every character is 7 points wide.
    fn measure(text: &str) -> f32 {
        text.chars().count() as f32 * 7.0
    }

    #[test]
    fn test_fit_middle_with_returns_text_that_fits_unchanged() {
        assert_eq!(fit_middle_with("short.txt", 200.0, measure), "short.txt");
        assert_eq!(fit_middle_with("", 0.0, measure), "");
        assert_eq!(fit_middle_with("exact", 35.0, measure), "exact");
    }

    #[test]
    fn test_fit_middle_with_uses_all_available_width() {
        let text = "C:/Users/someone/Documents/projects/archive-final.nxar";
        let fitted = fit_middle_with(text, 140.0, measure);
        // 140 / 7 = 20 characters fit exactly.
        assert_eq!(fitted.chars().count(), 20);
        assert!(measure(&fitted) <= 140.0);
        assert!(fitted.starts_with("C:/"));
        assert!(fitted.ends_with(".nxar"));
        assert!(fitted.contains('\u{2026}'));
    }

    #[test]
    fn test_fit_middle_with_degenerate_widths() {
        let text = "a-rather-long-file-name.bin";
        assert_eq!(fit_middle_with(text, 0.0, measure), "\u{2026}");
        assert_eq!(fit_middle_with(text, -50.0, measure), "\u{2026}");
        assert_eq!(fit_middle_with(text, f32::INFINITY, measure), text);
        assert_eq!(fit_middle_with(text, f32::NAN, measure), text);
    }

    #[test]
    fn test_notice_kinds_have_distinct_colours() {
        let kinds = [
            NoticeKind::Info,
            NoticeKind::Success,
            NoticeKind::Warning,
            NoticeKind::Error,
        ];
        for (i, a) in kinds.iter().enumerate() {
            for b in &kinds[i + 1..] {
                assert_ne!(a.color(), b.color());
            }
        }
    }
}
