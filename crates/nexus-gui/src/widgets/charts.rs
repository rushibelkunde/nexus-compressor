//! Instruments of the live view: the overall progress gauge, the throughput
//! sparkline, the block map and the method breakdown bar with its legend.
//!
//! All geometry is computed by small pure functions (tested below) and then
//! painted with the egui `Painter`; no plotting crate is involved.

use eframe::egui::{
    self, pos2, vec2, Align2, Color32, Id, Mesh, Pos2, Rect, RichText, Sense, Shape, Stroke,
    StrokeKind,
};

use super::paint_stripes;
use crate::methods::{format_share, method_shares, Method, MethodBytes};
use crate::telemetry::{block_cells, CellState};
use crate::theme::{
    lerp_color, mono, with_alpha, BG_INSET, BG_PANEL, BG_RAISED, BORDER, BORDER_STRONG, RADIUS,
    TEXT, TEXT_DIM, TEXT_MUTED,
};
use crate::util::{format_bytes, format_speed};

// ---------------------------------------------------------------------------
// Progress gauge
// ---------------------------------------------------------------------------

const GAUGE_BAR_HEIGHT: f32 = 14.0;
const GAUGE_TICK_HEIGHT: f32 = 6.0;

/// Overall progress bar with a graduated scale underneath. `colors` is the
/// gradient from the start to the end of the full track. `live` enables the
/// eased fill and the moving activity stripes.
pub fn progress_gauge(
    ui: &mut egui::Ui,
    fraction: f32,
    colors: (Color32, Color32),
    live: Option<Id>,
) {
    let width = ui.available_width().max(80.0);
    let (rect, _) = ui.allocate_exact_size(
        vec2(width, GAUGE_BAR_HEIGHT + GAUGE_TICK_HEIGHT),
        Sense::hover(),
    );
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter().with_clip_rect(rect);
    let mut fraction = if fraction.is_finite() {
        fraction.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if let Some(id) = live {
        fraction = ui
            .ctx()
            .animate_value_with_time(id.with("progress_gauge"), fraction, 0.25)
            .clamp(0.0, 1.0);
    }

    let track = Rect::from_min_size(rect.min, vec2(rect.width(), GAUGE_BAR_HEIGHT));
    painter.rect(
        track,
        RADIUS,
        BG_INSET,
        Stroke::new(1.0_f32, BORDER),
        StrokeKind::Inside,
    );
    let inner = track.shrink(2.0);
    if inner.width() > 0.0 && inner.height() > 0.0 && fraction > 0.0 {
        let fill = Rect::from_min_size(
            inner.min,
            vec2((inner.width() * fraction).max(1.0), inner.height()),
        );
        // The gradient is anchored to the full track, so the colour at a
        // given position does not shift while the bar grows.
        let end_color = lerp_color(colors.0, colors.1, fraction);
        let mut mesh = Mesh::default();
        mesh.colored_vertex(fill.left_top(), colors.0);
        mesh.colored_vertex(fill.right_top(), end_color);
        mesh.colored_vertex(fill.right_bottom(), end_color);
        mesh.colored_vertex(fill.left_bottom(), colors.0);
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        painter.add(Shape::mesh(mesh));
        if live.is_some() && fraction < 0.999 {
            let time = ui.input(|i| i.time);
            paint_stripes(&painter, fill, time, with_alpha(Color32::BLACK, 46));
        }
    }

    for i in 0..=20 {
        let x = inner.left() + inner.width().max(0.0) * i as f32 / 20.0;
        let len = if i % 10 == 0 {
            GAUGE_TICK_HEIGHT
        } else if i % 2 == 0 {
            4.0
        } else {
            2.0
        };
        painter.vline(
            x,
            track.bottom()..=track.bottom() + len,
            Stroke::new(1.0_f32, BORDER_STRONG),
        );
    }
}

// ---------------------------------------------------------------------------
// Throughput sparkline
// ---------------------------------------------------------------------------

/// Number of sample slots the sparkline is laid out for at minimum: with
/// fewer samples the line grows in from the right like a strip chart.
const SPARK_MIN_SLOTS: usize = 30;

/// Headroom above the peak sample.
const SPARK_HEADROOM: f32 = 1.15;

/// Computes the polyline of a sparkline inside `plot`. Samples run oldest to
/// newest; the newest always sits on the right edge and zero on the bottom
/// edge. A single sample yields a short flat stub so it is visible.
pub fn sparkline_points(samples: &[f32], plot: Rect) -> Vec<Pos2> {
    if samples.is_empty() || !plot.is_finite() || plot.width() <= 0.0 || plot.height() <= 0.0 {
        return Vec::new();
    }
    let peak = samples
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .fold(0.0f32, f32::max);
    let scale = (peak * SPARK_HEADROOM).max(1e-6);
    let slots = samples.len().max(SPARK_MIN_SLOTS);
    let step = plot.width() / (slots - 1) as f32;
    let y_of = |value: f32| -> f32 {
        let value = if value.is_finite() {
            value.max(0.0)
        } else {
            0.0
        };
        plot.bottom() - (value / scale).clamp(0.0, 1.0) * plot.height()
    };

    let last = samples.len() - 1;
    let mut points = Vec::with_capacity(samples.len() + 1);
    if samples.len() == 1 {
        points.push(pos2(plot.right() - step, y_of(samples[0])));
    }
    for (i, value) in samples.iter().enumerate() {
        let x = plot.right() - (last - i) as f32 * step;
        points.push(pos2(x.max(plot.left()), y_of(*value)));
    }
    points
}

/// Filled sparkline of recent throughput samples (MB/s, oldest first).
pub fn sparkline(ui: &mut egui::Ui, samples: &[f32], height: f32, color: Color32) {
    let width = ui.available_width().max(60.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, height.max(24.0)), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect(
        rect,
        RADIUS,
        BG_INSET,
        Stroke::new(1.0_f32, BORDER),
        StrokeKind::Inside,
    );
    let plot = rect.shrink2(vec2(5.0, 6.0));

    // Recessive reference lines: half scale and baseline.
    painter.hline(
        plot.x_range(),
        plot.center().y,
        Stroke::new(1.0_f32, with_alpha(BORDER, 140)),
    );
    painter.hline(
        plot.x_range(),
        plot.bottom(),
        Stroke::new(1.0_f32, BORDER_STRONG),
    );

    let points = sparkline_points(samples, plot);
    if points.len() < 2 {
        painter.text(
            plot.center(),
            Align2::CENTER_CENTER,
            "waiting for the first block",
            mono(10.5),
            TEXT_DIM,
        );
        return;
    }

    // Area under the line as a triangle strip (it is not convex, so it
    // cannot be a single polygon).
    let mut mesh = Mesh::default();
    let top_color = with_alpha(color, 70);
    let bottom_color = with_alpha(color, 8);
    for point in &points {
        mesh.colored_vertex(*point, top_color);
        mesh.colored_vertex(pos2(point.x, plot.bottom()), bottom_color);
    }
    for i in 0..points.len() as u32 - 1 {
        let base = i * 2;
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base + 1, base + 3, base + 2);
    }
    painter.add(Shape::mesh(mesh));
    painter.add(Shape::line(points.clone(), Stroke::new(2.0_f32, color)));

    // Marker on the newest sample, ringed so it stays legible on the line.
    if let Some(newest) = points.last() {
        painter.circle_filled(*newest, 4.5, BG_INSET);
        painter.circle_filled(*newest, 3.0, color);
    }

    let finite = samples.iter().copied().filter(|v| v.is_finite());
    let peak = finite.clone().fold(0.0f32, f32::max);
    let low = finite.fold(f32::INFINITY, f32::min);
    if low.is_finite() {
        response.on_hover_text(format!(
            "Last {} samples\npeak {}\nlow {}",
            samples.len(),
            format_speed(f64::from(peak)),
            format_speed(f64::from(low)),
        ));
    }
}

// ---------------------------------------------------------------------------
// Block map
// ---------------------------------------------------------------------------

const CELL_GAP: f32 = 2.0;
const CELL_MIN: f32 = 5.0;
const CELL_MAX: f32 = 14.0;

/// Layout of the block-map grid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridLayout {
    /// Side length of a cell in points.
    pub cell: f32,
    /// Cells per row (at least 1).
    pub cols: usize,
    /// Number of rows that fit (at least 1).
    pub rows: usize,
}

impl GridLayout {
    /// Number of cells the grid can show.
    pub fn capacity(&self) -> usize {
        self.cols.saturating_mul(self.rows)
    }

    /// Top-left corner of cell `index` relative to the grid origin.
    pub fn cell_origin(&self, index: usize) -> (f32, f32) {
        let pitch = self.cell + CELL_GAP;
        (
            (index % self.cols) as f32 * pitch,
            (index / self.cols) as f32 * pitch,
        )
    }

    /// Index of the cell under a point given relative to the grid origin, if
    /// the point is on a cell (not in a gap or outside the grid).
    pub fn cell_at(&self, x: f32, y: f32) -> Option<usize> {
        if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 {
            return None;
        }
        let pitch = self.cell + CELL_GAP;
        let (col, row) = ((x / pitch).floor() as usize, (y / pitch).floor() as usize);
        if col >= self.cols || row >= self.rows {
            return None;
        }
        if x - col as f32 * pitch > self.cell || y - row as f32 * pitch > self.cell {
            return None;
        }
        Some(row * self.cols + col)
    }
}

/// Picks the largest cell size (between the fixed minimum and maximum) at
/// which `count` cells fit into `width` x `height`. If they do not fit even
/// at the minimum size, the minimum-size grid is returned and the caller
/// must group cells down to its [`GridLayout::capacity`].
pub fn grid_layout(count: usize, width: f32, height: f32) -> GridLayout {
    let fit = |cell: f32| -> GridLayout {
        let pitch = cell + CELL_GAP;
        let along = |extent: f32| -> usize {
            if extent.is_finite() && extent >= cell {
                (((extent + CELL_GAP) / pitch).floor() as usize).max(1)
            } else {
                1
            }
        };
        GridLayout {
            cell,
            cols: along(width),
            rows: along(height),
        }
    };
    let mut cell = CELL_MAX;
    while cell > CELL_MIN {
        let layout = fit(cell);
        if layout.capacity() >= count {
            return layout;
        }
        cell -= 1.0;
    }
    fit(CELL_MIN)
}

/// Grid of superblocks: finished ones coloured by their dominant method
/// (stored blocks hollow), pending ones dimmed. `expected` is the total
/// number of superblocks when known. `highlight_newest` rings the most
/// recently finished cell.
pub fn block_map(
    ui: &mut egui::Ui,
    done: &[Method],
    expected: Option<usize>,
    height: f32,
    highlight_newest: bool,
) {
    let width = ui.available_width().max(40.0);
    let (rect, response) =
        ui.allocate_exact_size(vec2(width, height.max(CELL_MIN)), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter().with_clip_rect(rect);
    let expected = expected.unwrap_or(done.len()).max(done.len());
    if expected == 0 {
        painter.text(
            rect.left_center(),
            Align2::LEFT_CENTER,
            "waiting for the first block",
            mono(10.5),
            TEXT_DIM,
        );
        return;
    }

    let capacity = grid_layout(usize::MAX, rect.width(), rect.height()).capacity();
    let (cells, group) = block_cells(done, expected, capacity);
    let layout = grid_layout(cells.len(), rect.width(), rect.height());
    let newest_done = cells
        .iter()
        .rposition(|cell| matches!(cell, CellState::Done(_)));

    for (index, cell) in cells.iter().enumerate().take(layout.capacity()) {
        let (dx, dy) = layout.cell_origin(index);
        let cell_rect = Rect::from_min_size(
            pos2(rect.left() + dx, rect.top() + dy),
            vec2(layout.cell, layout.cell),
        );
        match cell {
            CellState::Pending => {
                painter.rect_filled(cell_rect, 1.0, BG_RAISED);
            }
            CellState::Done(method) => {
                paint_method_mark(&painter, cell_rect, *method);
                if highlight_newest && newest_done == Some(index) {
                    painter.rect_stroke(
                        cell_rect,
                        1.0,
                        Stroke::new(1.0_f32, TEXT),
                        StrokeKind::Outside,
                    );
                }
            }
        }
    }

    if let Some(pointer) = response.hover_pos() {
        let hit = layout
            .cell_at(pointer.x - rect.left(), pointer.y - rect.top())
            .filter(|index| *index < cells.len());
        if let Some(index) = hit {
            let range = if group <= 1 {
                format!("Block {}", index + 1)
            } else {
                let first = index * group + 1;
                let last = ((index + 1) * group).min(expected);
                format!("Blocks {first}\u{2013}{last}")
            };
            let state = match cells[index] {
                CellState::Pending => "pending".to_string(),
                CellState::Done(method) if group <= 1 => method.label().to_string(),
                CellState::Done(method) => format!("mostly {}", method.label()),
            };
            response.on_hover_text_at_pointer(format!("{range}: {state}"));
        }
    }
}

// Paints one mark (block-map cell, legend swatch or bar segment) in the
// method's colour; hollow methods get an outline instead of a fill.
fn paint_method_mark(painter: &egui::Painter, rect: Rect, method: Method) {
    if method.is_hollow() {
        painter.rect(
            rect,
            1.0,
            with_alpha(method.color(), 40),
            Stroke::new(1.5_f32, method.color()),
            StrokeKind::Inside,
        );
    } else {
        painter.rect_filled(rect, 1.0, method.color());
    }
}

// ---------------------------------------------------------------------------
// Method breakdown
// ---------------------------------------------------------------------------

const SEGMENT_GAP: f32 = 2.0;
const SEGMENT_MIN: f32 = 3.0;

/// Horizontal spans `(method, x0, x1)` of the stacked breakdown bar within a
/// track of `width`, in [`Method::ALL`] order. Methods with no bytes get no
/// span; every other method gets at least a visible sliver, and neighbours
/// are separated by a small gap.
pub fn method_spans(totals: &MethodBytes, width: f32) -> Vec<(Method, f32, f32)> {
    if !width.is_finite() || width <= 0.0 {
        return Vec::new();
    }
    let shares = method_shares(totals);
    let present: Vec<Method> = Method::ALL
        .into_iter()
        .filter(|m| totals[m.index()] > 0)
        .collect();
    if present.is_empty() {
        return Vec::new();
    }
    let gaps = SEGMENT_GAP * (present.len() - 1) as f32;
    let minimums = SEGMENT_MIN * present.len() as f32;
    // On an absurdly narrow track fall back to plain proportional spans.
    let (base, flexible, gap) = if width > gaps + minimums {
        (SEGMENT_MIN, width - gaps - minimums, SEGMENT_GAP)
    } else {
        (0.0, width, 0.0)
    };

    let mut spans = Vec::with_capacity(present.len());
    let mut x = 0.0f32;
    for (i, method) in present.iter().enumerate() {
        let length = base + shares[method.index()] * flexible;
        let end = if i + 1 == present.len() {
            width
        } else {
            (x + length).min(width)
        };
        spans.push((*method, x, end));
        x = (end + gap).min(width);
    }
    spans
}

/// Stacked bar of the share of data handled by each method.
pub fn method_bar(ui: &mut egui::Ui, totals: &MethodBytes, height: f32) {
    let width = ui.available_width().max(40.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, height.max(6.0)), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter().with_clip_rect(rect);
    let spans = method_spans(totals, rect.width());
    if spans.is_empty() {
        painter.rect(
            rect,
            RADIUS,
            BG_INSET,
            Stroke::new(1.0_f32, BORDER),
            StrokeKind::Inside,
        );
        return;
    }
    for (method, x0, x1) in &spans {
        let segment = Rect::from_min_max(
            pos2(rect.left() + x0, rect.top()),
            pos2(rect.left() + x1, rect.bottom()),
        );
        if segment.width() > 0.0 {
            paint_method_mark(&painter, segment, *method);
        }
    }

    let shares = method_shares(totals);
    response.on_hover_ui(|ui| {
        for method in Method::ALL {
            let bytes = totals[method.index()];
            if bytes > 0 {
                ui.label(
                    RichText::new(format!(
                        "{:<15}{:>6}  {}",
                        method.label(),
                        format_share(shares[method.index()]),
                        format_bytes(bytes)
                    ))
                    .font(mono(11.5))
                    .color(TEXT),
                );
            }
        }
    });
}

/// Legend for the method colours, with each method's share. It doubles as
/// the legend of the block map, so every method is listed even at 0 %.
pub fn method_legend(ui: &mut egui::Ui, totals: &MethodBytes) {
    let shares = method_shares(totals);
    const SWATCH: f32 = 10.0;
    const SWATCH_GAP: f32 = 5.0;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(14.0, 4.0);
        for method in Method::ALL {
            let used = totals[method.index()] > 0;
            let text = if used {
                format!(
                    "{} {}",
                    method.label(),
                    format_share(shares[method.index()])
                )
            } else {
                method.label().to_string()
            };
            let text_color = if used { TEXT_MUTED } else { TEXT_DIM };
            let galley = ui.painter().layout_no_wrap(text, mono(11.0), text_color);
            // Swatch and label are one widget, so wrapping never separates
            // a swatch from its label.
            let size = vec2(
                SWATCH + SWATCH_GAP + galley.size().x,
                galley.size().y.max(SWATCH),
            );
            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
            if !ui.is_rect_visible(rect) {
                continue;
            }
            let painter = ui.painter();
            let swatch = Rect::from_center_size(
                pos2(rect.left() + SWATCH / 2.0, rect.center().y),
                vec2(SWATCH, SWATCH),
            );
            if used {
                paint_method_mark(painter, swatch, method);
            } else {
                // Unused methods keep their swatch, toned down.
                painter.rect_filled(swatch, 1.0, lerp_color(BG_PANEL, method.color(), 0.45));
            }
            let text_pos = pos2(
                rect.left() + SWATCH + SWATCH_GAP,
                rect.center().y - galley.size().y / 2.0,
            );
            painter.galley(text_pos, galley, text_color);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::methods::METHOD_COUNT;

    fn plot() -> Rect {
        Rect::from_min_max(pos2(10.0, 20.0), pos2(310.0, 80.0))
    }

    #[test]
    fn test_sparkline_empty_and_degenerate() {
        assert!(sparkline_points(&[], plot()).is_empty());
        let flat = Rect::from_min_max(pos2(0.0, 0.0), pos2(0.0, 50.0));
        assert!(sparkline_points(&[1.0, 2.0], flat).is_empty());
        let inverted = Rect::from_min_max(pos2(10.0, 50.0), pos2(5.0, 0.0));
        assert!(sparkline_points(&[1.0, 2.0], inverted).is_empty());
    }

    #[test]
    fn test_sparkline_single_sample_is_a_visible_stub() {
        let points = sparkline_points(&[5.0], plot());
        assert_eq!(points.len(), 2);
        assert_eq!(points[1].x, plot().right());
        assert!(points[0].x < points[1].x);
        assert_eq!(points[0].y, points[1].y);
    }

    #[test]
    fn test_sparkline_points_stay_inside_plot_and_end_on_right_edge() {
        let samples: Vec<f32> = (0..200).map(|i| (i % 17) as f32).collect();
        let p = plot();
        let points = sparkline_points(&samples, p);
        assert_eq!(points.len(), samples.len());
        assert_eq!(points.last().unwrap().x, p.right());
        assert!((points[0].x - p.left()).abs() < 0.01);
        for pair in points.windows(2) {
            assert!(pair[0].x <= pair[1].x);
        }
        for point in &points {
            assert!(point.x >= p.left() - 0.01 && point.x <= p.right() + 0.01);
            assert!(point.y >= p.top() - 0.01 && point.y <= p.bottom() + 0.01);
        }
    }

    #[test]
    fn test_sparkline_scale_leaves_headroom_and_zero_is_baseline() {
        let p = plot();
        let points = sparkline_points(&[0.0, 10.0], p);
        assert_eq!(points[0].y, p.bottom());
        assert!(points[1].y > p.top(), "peak must not touch the top edge");
        assert!(points[1].y < p.center().y);
    }

    #[test]
    fn test_sparkline_all_zero_and_non_finite_samples() {
        let p = plot();
        for point in sparkline_points(&[0.0, 0.0, 0.0], p) {
            assert_eq!(point.y, p.bottom());
        }
        for point in sparkline_points(&[f32::NAN, f32::INFINITY, 3.0], p) {
            assert!(point.y.is_finite());
            assert!(point.y >= p.top() && point.y <= p.bottom());
        }
    }

    #[test]
    fn test_grid_layout_uses_largest_cell_that_fits() {
        // 300 x 62: at 14 px (pitch 16) -> 18 cols x 4 rows = 72 cells.
        let few = grid_layout(10, 300.0, 62.0);
        assert_eq!(few.cell, CELL_MAX);
        assert_eq!((few.cols, few.rows), (18, 4));
        let many = grid_layout(200, 300.0, 62.0);
        assert!(many.cell < CELL_MAX);
        assert!(many.capacity() >= 200);
    }

    #[test]
    fn test_grid_layout_overflow_returns_minimum_grid() {
        let layout = grid_layout(usize::MAX, 300.0, 62.0);
        assert_eq!(layout.cell, CELL_MIN);
        // pitch 7: (300 + 2) / 7 = 43 cols, (62 + 2) / 7 = 9 rows.
        assert_eq!((layout.cols, layout.rows), (43, 9));
        assert!(layout.capacity() < usize::MAX);
    }

    #[test]
    fn test_grid_layout_never_zero_sized() {
        for (w, h) in [(0.0, 0.0), (-5.0, 10.0), (f32::NAN, f32::NAN), (3.0, 3.0)] {
            let layout = grid_layout(5, w, h);
            assert!(layout.cols >= 1 && layout.rows >= 1);
            assert!(layout.cell >= CELL_MIN);
        }
    }

    #[test]
    fn test_grid_cells_stay_inside_bounds() {
        let (w, h) = (300.0, 62.0);
        let layout = grid_layout(72, w, h);
        for index in 0..layout.capacity() {
            let (x, y) = layout.cell_origin(index);
            assert!(x + layout.cell <= w + 0.01, "cell {index} overflows width");
            assert!(y + layout.cell <= h + 0.01, "cell {index} overflows height");
        }
    }

    #[test]
    fn test_grid_cell_at_roundtrip_and_gaps() {
        let layout = grid_layout(72, 300.0, 62.0);
        for index in 0..layout.capacity() {
            let (x, y) = layout.cell_origin(index);
            assert_eq!(layout.cell_at(x + 1.0, y + 1.0), Some(index));
        }
        // In the gap right of the first cell.
        assert_eq!(layout.cell_at(layout.cell + 1.0, 1.0), None);
        assert_eq!(layout.cell_at(-1.0, 1.0), None);
        assert_eq!(layout.cell_at(1.0, 10_000.0), None);
        assert_eq!(layout.cell_at(f32::NAN, 1.0), None);
    }

    #[test]
    fn test_method_spans_empty_cases() {
        assert!(method_spans(&[0; METHOD_COUNT], 300.0).is_empty());
        assert!(method_spans(&[1, 0, 0, 0, 0], 0.0).is_empty());
        assert!(method_spans(&[1, 0, 0, 0, 0], f32::NAN).is_empty());
    }

    #[test]
    fn test_method_spans_single_method_fills_track() {
        let spans = method_spans(&[0, 500, 0, 0, 0], 300.0);
        assert_eq!(spans, vec![(Method::Bwt, 0.0, 300.0)]);
    }

    #[test]
    fn test_method_spans_are_ordered_disjoint_and_cover_track() {
        let totals: MethodBytes = [700, 200, 0, 1, 99];
        let width = 400.0;
        let spans = method_spans(&totals, width);
        assert_eq!(spans.len(), 4);
        assert_eq!(spans[0].1, 0.0);
        assert_eq!(spans.last().unwrap().2, width);
        for (_, x0, x1) in &spans {
            assert!(x1 > x0);
            assert!(*x0 >= 0.0 && *x1 <= width);
        }
        for pair in spans.windows(2) {
            assert!(pair[1].1 >= pair[0].2, "segments overlap");
        }
        // The tiny share is still visible.
        let neural = spans.iter().find(|s| s.0 == Method::Neural).unwrap();
        assert!(neural.2 - neural.1 >= SEGMENT_MIN - 0.01);
        // The biggest share gets the longest segment.
        let lz = spans.iter().find(|s| s.0 == Method::Lz).unwrap();
        assert!(lz.2 - lz.1 > 0.6 * width);
    }

    #[test]
    fn test_method_spans_narrow_track_stays_in_bounds() {
        let spans = method_spans(&[5, 5, 5, 5, 5], 6.0);
        assert_eq!(spans.len(), 5);
        for (_, x0, x1) in &spans {
            assert!(*x0 >= 0.0 && *x1 <= 6.0 && x1 >= x0);
        }
    }
}
