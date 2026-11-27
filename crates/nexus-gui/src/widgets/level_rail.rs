//! The compression-level slider: a segmented rail with one cell per level
//! (1–19), lit up to the selected level like a bar-graph gauge.
//!
//! Click or drag to select; with keyboard focus the left/right arrow keys
//! step by one level. Cell colours run from the brand cyan (fast) to purple
//! (maximum LZ) and switch to the warning and danger colours for the much
//! slower modelling tiers, so the cost of the far end is visible at a glance.

use eframe::egui::{self, pos2, vec2, Align2, Color32, Key, Rect, Sense, Stroke, StrokeKind};

use crate::engine::{clamp_level, MAX_LEVEL, MIN_LEVEL, NEURAL_LEVEL, SLOW_LEVEL};
use crate::theme::{
    lerp_color, mono, BG_RAISED, BORDER, DANGER, NEXUS_CYAN, NEXUS_PURPLE, TEXT, TEXT_DIM, WARNING,
};

const LEVEL_COUNT: u32 = MAX_LEVEL - MIN_LEVEL + 1;
const CELL_HEIGHT: f32 = 22.0;
const NUMERAL_HEIGHT: f32 = 15.0;

/// The level whose cell contains horizontal position `x` on a rail starting
/// at `left` and `width` wide. Positions outside the rail clamp to the
/// first or last level.
pub fn level_at(x: f32, left: f32, width: f32) -> u32 {
    if !width.is_finite() || width <= 0.0 || !x.is_finite() || !left.is_finite() {
        return MIN_LEVEL;
    }
    let t = ((x - left) / width).clamp(0.0, 1.0);
    let index = ((t * LEVEL_COUNT as f32).floor() as u32).min(LEVEL_COUNT - 1);
    MIN_LEVEL + index
}

/// Gauge colour of a level's cell.
pub fn tier_color(level: u32) -> Color32 {
    let level = clamp_level(level);
    if level >= NEURAL_LEVEL {
        DANGER
    } else if level >= SLOW_LEVEL {
        WARNING
    } else {
        let span = (SLOW_LEVEL - 1 - MIN_LEVEL).max(1) as f32;
        lerp_color(NEXUS_CYAN, NEXUS_PURPLE, (level - MIN_LEVEL) as f32 / span)
    }
}

/// Shows the rail and updates `level`. Returns `true` when the level changed.
pub fn level_rail(ui: &mut egui::Ui, level: &mut u32) -> bool {
    let before = clamp_level(*level);
    let mut selected = before;

    let width = ui.available_width().max(120.0);
    let (rect, response) = ui.allocate_exact_size(
        vec2(width, CELL_HEIGHT + NUMERAL_HEIGHT),
        Sense::click_and_drag(),
    );

    if response.is_pointer_button_down_on() || response.clicked() {
        if let Some(pointer) = response.interact_pointer_pos() {
            selected = level_at(pointer.x, rect.left(), rect.width());
        }
        response.request_focus();
    }
    if response.has_focus() {
        // Keep the arrow keys for stepping instead of moving focus away.
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                response.id,
                egui::EventFilter {
                    horizontal_arrows: true,
                    ..Default::default()
                },
            );
        });
        let (down, up) = ui.input(|i| {
            (
                i.key_pressed(Key::ArrowLeft),
                i.key_pressed(Key::ArrowRight),
            )
        });
        if down {
            selected = clamp_level(selected.saturating_sub(1));
        }
        if up {
            selected = clamp_level(selected + 1);
        }
    }

    if ui.is_rect_visible(rect) {
        let painter = ui.painter().with_clip_rect(rect.expand(1.0));
        let cell_width = rect.width() / LEVEL_COUNT as f32;
        let show_all_numerals = cell_width >= 17.0;
        for index in 0..LEVEL_COUNT {
            let cell_level = MIN_LEVEL + index;
            let x0 = rect.left() + index as f32 * cell_width;
            let cell = Rect::from_min_max(
                pos2(x0 + 1.0, rect.top()),
                pos2(x0 + cell_width - 1.0, rect.top() + CELL_HEIGHT),
            );
            if cell.width() < 1.0 {
                continue;
            }
            let lit = cell_level <= selected;
            let fill = if lit {
                tier_color(cell_level)
            } else {
                // Unlit cells keep a hint of their tier colour.
                lerp_color(BG_RAISED, tier_color(cell_level), 0.14)
            };
            painter.rect_filled(cell, 1.0, fill);
            if cell_level == selected {
                painter.rect_stroke(cell, 1.0, Stroke::new(2.0_f32, TEXT), StrokeKind::Inside);
            }

            let is_marker = cell_level == MIN_LEVEL
                || cell_level == MAX_LEVEL
                || cell_level.is_multiple_of(5)
                || cell_level == selected;
            if show_all_numerals || is_marker {
                let color = if cell_level == selected {
                    TEXT
                } else {
                    TEXT_DIM
                };
                painter.text(
                    pos2(cell.center().x, rect.top() + CELL_HEIGHT + 2.0),
                    Align2::CENTER_TOP,
                    cell_level,
                    mono(10.5),
                    color,
                );
            }
        }
        if response.has_focus() {
            painter.rect_stroke(
                Rect::from_min_max(
                    rect.left_top(),
                    pos2(rect.right(), rect.top() + CELL_HEIGHT),
                )
                .expand(1.0),
                1.0,
                Stroke::new(1.0_f32, BORDER),
                StrokeKind::Outside,
            );
        }
    }

    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Click or drag to choose a level. Arrow keys step by one.");

    *level = selected;
    selected != before
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_level_at_cell_centres() {
        let (left, width) = (100.0, 380.0);
        let cell = width / 19.0;
        for level in MIN_LEVEL..=MAX_LEVEL {
            let x = left + (level - MIN_LEVEL) as f32 * cell + cell / 2.0;
            assert_eq!(level_at(x, left, width), level);
        }
    }

    #[test]
    fn test_level_at_clamps_outside_the_rail() {
        assert_eq!(level_at(-500.0, 100.0, 380.0), MIN_LEVEL);
        assert_eq!(level_at(100.0, 100.0, 380.0), MIN_LEVEL);
        assert_eq!(level_at(480.0, 100.0, 380.0), MAX_LEVEL);
        assert_eq!(level_at(9000.0, 100.0, 380.0), MAX_LEVEL);
    }

    #[test]
    fn test_level_at_degenerate_rail() {
        assert_eq!(level_at(50.0, 0.0, 0.0), MIN_LEVEL);
        assert_eq!(level_at(50.0, 0.0, -10.0), MIN_LEVEL);
        assert_eq!(level_at(f32::NAN, 0.0, 100.0), MIN_LEVEL);
        assert_eq!(level_at(50.0, 0.0, f32::NAN), MIN_LEVEL);
    }

    #[test]
    fn test_tier_colors_mark_slow_tiers() {
        assert_eq!(tier_color(1), NEXUS_CYAN);
        assert_eq!(tier_color(12), NEXUS_PURPLE);
        assert_eq!(tier_color(13), WARNING);
        assert_eq!(tier_color(16), WARNING);
        assert_eq!(tier_color(17), DANGER);
        assert_eq!(tier_color(19), DANGER);
        // Out-of-range input clamps instead of panicking.
        assert_eq!(tier_color(0), NEXUS_CYAN);
        assert_eq!(tier_color(99), DANGER);
    }
}
