//! The "press" gauge: two horizontal bars drawn to the same byte scale — the
//! data going in above, the data coming out below — joined by a funnel, so
//! the squeeze (or, for extraction, the expansion) is visible as geometry.
//!
//! The same widget serves the live view (eased bar lengths, moving activity
//! stripes) and the static before/after display on the finished screen.

use eframe::egui::{
    self, pos2, vec2, Align2, Color32, Id, Pos2, Rect, Sense, Shape, Stroke, StrokeKind,
};

use super::paint_stripes;
use crate::theme::{
    lerp_color, mono, with_alpha, BG_INSET, BG_PANEL, BORDER, BORDER_STRONG, RADIUS, TEXT, TEXT_DIM,
};
use crate::util::fraction_of;

const LABEL_HEIGHT: f32 = 16.0;
const LABEL_GAP: f32 = 3.0;
const BAR_HEIGHT: f32 = 20.0;
const FUNNEL_HEIGHT: f32 = 30.0;
/// Total height of the gauge.
pub const PRESS_HEIGHT: f32 = 2.0 * (LABEL_HEIGHT + LABEL_GAP + BAR_HEIGHT) + FUNNEL_HEIGHT;

/// Smallest drawn length of a bar that stands for a non-zero byte count.
const MIN_BAR: f32 = 2.0;

/// One bar of the press.
pub struct PressBar<'a> {
    /// Caption, e.g. `ORIGINAL`.
    pub label: &'a str,
    /// Bytes this bar stands for.
    pub bytes: u64,
    /// Right-aligned readout, e.g. `123.4 MB / 512.0 MB`.
    pub detail: String,
    /// Fill colour.
    pub color: Color32,
}

/// Everything the press gauge shows.
pub struct PressView<'a> {
    /// Upper bar (input side).
    pub top: PressBar<'a>,
    /// Lower bar (output side).
    pub bottom: PressBar<'a>,
    /// Byte count that maps to the full bar width; both bars share it.
    pub scale_bytes: u64,
    /// Text centred between the bars (ratio and savings).
    pub caption: String,
    /// `Some(id)` for a live gauge: bar lengths ease towards their targets
    /// and unfinished bars show moving stripes. `None` for a static gauge.
    pub live: Option<Id>,
}

/// Drawn length in points of a bar showing `fraction` of a track that is
/// `track_width` wide. Non-zero fractions never collapse below a visible
/// sliver, and nothing exceeds the track.
pub fn bar_length(fraction: f32, track_width: f32) -> f32 {
    if !track_width.is_finite() || track_width <= 0.0 || !fraction.is_finite() || fraction <= 0.0 {
        return 0.0;
    }
    (fraction.min(1.0) * track_width)
        .max(MIN_BAR)
        .min(track_width)
}

/// Shows the gauge across the available width.
pub fn press(ui: &mut egui::Ui, view: &PressView<'_>) {
    let width = ui.available_width().max(160.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, PRESS_HEIGHT), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter().with_clip_rect(rect);
    let time = ui.input(|i| i.time);

    // The shared scale: never smaller than either bar, never zero.
    let scale = view
        .scale_bytes
        .max(view.top.bytes)
        .max(view.bottom.bytes)
        .max(1);
    let mut top_fraction = fraction_of(view.top.bytes, scale);
    let mut bottom_fraction = fraction_of(view.bottom.bytes, scale);
    if let Some(id) = view.live {
        top_fraction = ui
            .ctx()
            .animate_value_with_time(id.with("press_top"), top_fraction, 0.25);
        bottom_fraction =
            ui.ctx()
                .animate_value_with_time(id.with("press_bottom"), bottom_fraction, 0.25);
    }

    let top_label_y = rect.top();
    let top_track = Rect::from_min_size(
        pos2(rect.left(), top_label_y + LABEL_HEIGHT + LABEL_GAP),
        vec2(rect.width(), BAR_HEIGHT),
    );
    let bottom_track = Rect::from_min_size(
        pos2(rect.left(), top_track.bottom() + FUNNEL_HEIGHT),
        vec2(rect.width(), BAR_HEIGHT),
    );
    let bottom_label_y = bottom_track.bottom() + LABEL_GAP;

    label_row(&painter, rect, top_label_y, &view.top);
    label_row(&painter, rect, bottom_label_y, &view.bottom);

    let top_end = bar(
        &painter,
        top_track,
        top_fraction,
        view.top.color,
        view.live.is_some(),
        time,
    );
    let bottom_end = bar(
        &painter,
        bottom_track,
        bottom_fraction,
        view.bottom.color,
        view.live.is_some(),
        time,
    );

    // Graduation ticks facing each other across the funnel.
    let inner_left = top_track.left() + 2.0;
    let inner_width = (top_track.width() - 4.0).max(0.0);
    for i in 0..=10 {
        let x = inner_left + inner_width * i as f32 / 10.0;
        let len = if i % 5 == 0 { 5.0 } else { 3.0 };
        let stroke = Stroke::new(1.0_f32, BORDER_STRONG);
        painter.vline(x, top_track.bottom()..=top_track.bottom() + len, stroke);
        painter.vline(x, bottom_track.top() - len..=bottom_track.top(), stroke);
    }

    // The funnel joins the ends of the two bars.
    if top_end > inner_left + 0.5 || bottom_end > inner_left + 0.5 {
        let funnel_color = lerp_color(view.top.color, view.bottom.color, 0.5);
        let corners: Vec<Pos2> = vec![
            pos2(inner_left, top_track.bottom()),
            pos2(top_end, top_track.bottom()),
            pos2(bottom_end, bottom_track.top()),
            pos2(inner_left, bottom_track.top()),
        ];
        painter.add(Shape::convex_polygon(
            corners,
            with_alpha(funnel_color, 30),
            Stroke::NONE,
        ));
        painter.line_segment(
            [
                pos2(top_end, top_track.bottom()),
                pos2(bottom_end, bottom_track.top()),
            ],
            Stroke::new(1.0_f32, with_alpha(funnel_color, 150)),
        );
    }

    // Caption on a backing plate so it stays readable over the funnel edge.
    if !view.caption.is_empty() {
        let galley = painter.layout_no_wrap(view.caption.clone(), mono(13.0), TEXT);
        let center = pos2(
            rect.center().x,
            (top_track.bottom() + bottom_track.top()) / 2.0,
        );
        let text_rect = Align2::CENTER_CENTER.anchor_size(center, galley.size());
        painter.rect_filled(
            text_rect.expand2(vec2(8.0, 2.0)),
            RADIUS,
            with_alpha(BG_PANEL, 225),
        );
        painter.galley(text_rect.min, galley, TEXT);
    }
}

// Paints one caption row: label on the left, readout on the right. The label
// yields when the two would collide.
fn label_row(painter: &egui::Painter, rect: Rect, y: f32, bar: &PressBar<'_>) {
    let center_y = y + LABEL_HEIGHT / 2.0;
    let detail_rect = painter.text(
        pos2(rect.right(), center_y),
        Align2::RIGHT_CENTER,
        &bar.detail,
        mono(12.0),
        TEXT,
    );
    let label = painter.layout_no_wrap(bar.label.to_uppercase(), mono(10.5), TEXT_DIM);
    // Swatch + label must end before the readout begins.
    let needed = 14.0 + label.size().x + 10.0;
    if rect.left() + needed <= detail_rect.left() {
        let swatch = Rect::from_center_size(pos2(rect.left() + 4.0, center_y), vec2(8.0, 8.0));
        painter.rect_filled(swatch, 1.0, bar.color);
        let pos = pos2(rect.left() + 14.0, center_y - label.size().y / 2.0);
        painter.galley(pos, label, TEXT_DIM);
    }
}

// Paints one track with its fill and returns the x position of the fill's
// end (the "ram face").
fn bar(
    painter: &egui::Painter,
    track: Rect,
    fraction: f32,
    color: Color32,
    live: bool,
    time: f64,
) -> f32 {
    painter.rect(
        track,
        RADIUS,
        BG_INSET,
        Stroke::new(1.0_f32, BORDER),
        StrokeKind::Inside,
    );
    let inner = track.shrink(2.0);
    if inner.width() <= 0.0 || inner.height() <= 0.0 {
        return track.left();
    }
    let length = bar_length(fraction, inner.width());
    if length <= 0.0 {
        return inner.left();
    }
    let fill = Rect::from_min_size(inner.min, vec2(length, inner.height()));
    painter.rect_filled(fill, 0.0, color);
    if live && fraction < 0.999 {
        paint_stripes(painter, fill, time, with_alpha(Color32::BLACK, 46));
    }
    // Bright leading edge: the face of the ram.
    let face = Rect::from_min_max(
        pos2((fill.right() - 2.0).max(fill.left()), fill.top()),
        fill.max,
    );
    painter.rect_filled(face, 0.0, lerp_color(color, Color32::WHITE, 0.55));
    fill.right()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bar_length_proportional() {
        assert_eq!(bar_length(0.5, 400.0), 200.0);
        assert_eq!(bar_length(1.0, 400.0), 400.0);
    }

    #[test]
    fn test_bar_length_zero_and_tiny() {
        assert_eq!(bar_length(0.0, 400.0), 0.0);
        // Non-zero data is always visible.
        assert_eq!(bar_length(0.000_01, 400.0), MIN_BAR);
    }

    #[test]
    fn test_bar_length_never_exceeds_track() {
        assert_eq!(bar_length(7.0, 400.0), 400.0);
        assert_eq!(bar_length(0.5, 1.0), 1.0);
    }

    #[test]
    fn test_bar_length_degenerate_inputs() {
        assert_eq!(bar_length(0.5, 0.0), 0.0);
        assert_eq!(bar_length(0.5, -20.0), 0.0);
        assert_eq!(bar_length(f32::NAN, 400.0), 0.0);
        assert_eq!(bar_length(0.5, f32::NAN), 0.0);
        assert_eq!(bar_length(-0.5, 400.0), 0.0);
    }

    #[test]
    fn test_press_height_matches_layout() {
        assert_eq!(PRESS_HEIGHT, 108.0);
    }
}
