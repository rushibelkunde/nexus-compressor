//! Vector icons drawn with the egui `Painter`.
//!
//! Emoji render inconsistently (or not at all) with egui's bundled fonts, so
//! every pictogram in the GUI is a handful of strokes and rectangles laid out
//! on a unit square and scaled into the target rectangle.

use eframe::egui::{self, pos2, Color32, Pos2, Rect, Sense, Shape, Stroke, Vec2};

/// The pictograms used by the GUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    /// A document with a folded corner.
    File,
    /// A folder.
    Folder,
    /// A closed archive box.
    Archive,
    /// A hydraulic press squeezing a block (compression).
    Press,
    /// An open box with an arrow leaving it (extraction).
    Unpack,
    /// A shield with a tick (verification).
    Shield,
    /// A tick.
    Check,
    /// A cross.
    Cross,
    /// A warning triangle.
    Warning,
    /// An information mark.
    Info,
    /// A lightning bolt (fast preset).
    Bolt,
    /// A balance (balanced preset).
    Balance,
    /// A cut gem (maximum preset).
    Gem,
}

/// Paints `icon` into `rect` using `color`. Does nothing for a degenerate
/// rectangle.
pub fn paint(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let size = rect.width().min(rect.height());
    if !size.is_finite() || size < 4.0 {
        return;
    }
    // Work on a centred square so icons never stretch.
    let rect = Rect::from_center_size(rect.center(), Vec2::splat(size));
    let at = |x: f32, y: f32| -> Pos2 { pos2(rect.left() + x * size, rect.top() + y * size) };
    let stroke = Stroke::new((size / 14.0).clamp(1.0, 2.5), color);
    let line = |points: &[(f32, f32)]| {
        painter.add(Shape::line(
            points.iter().map(|(x, y)| at(*x, *y)).collect(),
            stroke,
        ));
    };
    let closed = |points: &[(f32, f32)]| {
        painter.add(Shape::closed_line(
            points.iter().map(|(x, y)| at(*x, *y)).collect(),
            stroke,
        ));
    };
    let fill = |x0: f32, y0: f32, x1: f32, y1: f32| {
        painter.rect_filled(Rect::from_min_max(at(x0, y0), at(x1, y1)), 0.0, color);
    };

    match icon {
        Icon::File => {
            closed(&[
                (0.22, 0.08),
                (0.60, 0.08),
                (0.80, 0.28),
                (0.80, 0.92),
                (0.22, 0.92),
            ]);
            line(&[(0.60, 0.08), (0.60, 0.28), (0.80, 0.28)]);
            line(&[(0.34, 0.52), (0.68, 0.52)]);
            line(&[(0.34, 0.68), (0.68, 0.68)]);
        }
        Icon::Folder => {
            closed(&[
                (0.08, 0.22),
                (0.38, 0.22),
                (0.48, 0.34),
                (0.92, 0.34),
                (0.92, 0.84),
                (0.08, 0.84),
            ]);
            line(&[(0.08, 0.46), (0.92, 0.46)]);
        }
        Icon::Archive => {
            closed(&[(0.08, 0.14), (0.92, 0.14), (0.92, 0.32), (0.08, 0.32)]);
            closed(&[(0.14, 0.32), (0.86, 0.32), (0.86, 0.88), (0.14, 0.88)]);
            line(&[(0.38, 0.52), (0.62, 0.52)]);
        }
        Icon::Press => {
            // Frame: crown, base and two columns.
            fill(0.10, 0.08, 0.90, 0.18);
            fill(0.10, 0.82, 0.90, 0.92);
            line(&[(0.16, 0.18), (0.16, 0.82)]);
            line(&[(0.84, 0.18), (0.84, 0.82)]);
            // Ram and platen.
            line(&[(0.50, 0.18), (0.50, 0.36)]);
            fill(0.26, 0.36, 0.74, 0.44);
            // The squeezed block.
            closed(&[(0.32, 0.60), (0.68, 0.60), (0.68, 0.82), (0.32, 0.82)]);
            line(&[(0.32, 0.71), (0.68, 0.71)]);
        }
        Icon::Unpack => {
            line(&[(0.14, 0.52), (0.14, 0.90), (0.86, 0.90), (0.86, 0.52)]);
            line(&[(0.50, 0.72), (0.50, 0.10)]);
            line(&[(0.32, 0.28), (0.50, 0.10), (0.68, 0.28)]);
        }
        Icon::Shield => {
            closed(&[
                (0.50, 0.06),
                (0.86, 0.20),
                (0.86, 0.52),
                (0.50, 0.94),
                (0.14, 0.52),
                (0.14, 0.20),
            ]);
            line(&[(0.32, 0.46), (0.46, 0.60), (0.70, 0.32)]);
        }
        Icon::Check => {
            line(&[(0.14, 0.54), (0.40, 0.80), (0.86, 0.22)]);
        }
        Icon::Cross => {
            line(&[(0.18, 0.18), (0.82, 0.82)]);
            line(&[(0.82, 0.18), (0.18, 0.82)]);
        }
        Icon::Warning => {
            closed(&[(0.50, 0.08), (0.94, 0.88), (0.06, 0.88)]);
            line(&[(0.50, 0.38), (0.50, 0.62)]);
            painter.circle_filled(at(0.50, 0.75), (size / 18.0).max(1.0), color);
        }
        Icon::Info => {
            painter.circle_stroke(at(0.50, 0.50), size * 0.42, stroke);
            line(&[(0.50, 0.46), (0.50, 0.72)]);
            painter.circle_filled(at(0.50, 0.31), (size / 18.0).max(1.0), color);
        }
        Icon::Bolt => {
            closed(&[
                (0.58, 0.06),
                (0.20, 0.56),
                (0.46, 0.56),
                (0.40, 0.94),
                (0.80, 0.42),
                (0.54, 0.42),
            ]);
        }
        Icon::Balance => {
            line(&[(0.12, 0.26), (0.88, 0.26)]);
            line(&[(0.50, 0.12), (0.50, 0.86)]);
            line(&[(0.30, 0.86), (0.70, 0.86)]);
            line(&[(0.02, 0.58), (0.12, 0.26), (0.22, 0.58), (0.02, 0.58)]);
            line(&[(0.78, 0.58), (0.88, 0.26), (0.98, 0.58), (0.78, 0.58)]);
        }
        Icon::Gem => {
            closed(&[
                (0.28, 0.12),
                (0.72, 0.12),
                (0.94, 0.38),
                (0.50, 0.92),
                (0.06, 0.38),
            ]);
            line(&[(0.06, 0.38), (0.94, 0.38)]);
            line(&[(0.28, 0.12), (0.38, 0.38), (0.50, 0.92)]);
            line(&[(0.72, 0.12), (0.62, 0.38), (0.50, 0.92)]);
        }
    }
}

/// Allocates a `size` x `size` square in the layout and paints `icon` in it.
pub fn icon(ui: &mut egui::Ui, icon: Icon, size: f32, color: Color32) -> egui::Response {
    let size = size.max(4.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    if ui.is_rect_visible(rect) {
        paint(ui.painter(), rect, icon, color);
    }
    response
}
