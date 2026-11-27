//! Full-window overlay shown while files are dragged over the window. It
//! tells the user what a drop will do before they let go.

use std::path::Path;

use eframe::egui::{self, pos2, vec2, Align2, Id, LayerId, Order, Rect, Shape, Stroke};

use crate::paths::{display_name, route_for_existing, Route};
use crate::theme::{mono, sans, with_alpha, BG_DARK, NEXUS_CYAN, NEXUS_PURPLE_TEXT, TEXT_MUTED};
use crate::widgets::fit_middle_with;
use crate::widgets::icons::{self, Icon};

/// What dropping the hovered item would do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropIntent {
    /// The item will be compressed.
    Compress,
    /// The item is an archive and will be opened for extraction.
    Extract,
    /// The platform did not report a path yet.
    Unknown,
    /// A job is running; drops are ignored.
    Busy,
}

/// Decides the intent for a hovered path. `busy` is true while a job runs.
pub fn intent_for(path: Option<&Path>, busy: bool) -> DropIntent {
    if busy {
        return DropIntent::Busy;
    }
    match path.map(route_for_existing) {
        Some(Route::Compress) => DropIntent::Compress,
        Some(Route::Extract) => DropIntent::Extract,
        None => DropIntent::Unknown,
    }
}

/// Headline of the overlay for an intent.
pub fn headline(intent: DropIntent) -> &'static str {
    match intent {
        DropIntent::Compress => "DROP TO COMPRESS",
        DropIntent::Extract => "DROP TO EXTRACT",
        DropIntent::Unknown => "DROP TO OPEN",
        DropIntent::Busy => "A JOB IS RUNNING",
    }
}

/// Paints the overlay above everything else.
pub fn show(ctx: &egui::Context, path: Option<&Path>, extra_items: usize, busy: bool) {
    let intent = intent_for(path, busy);
    let (color, icon) = match intent {
        DropIntent::Compress | DropIntent::Unknown => (NEXUS_CYAN, Icon::Press),
        DropIntent::Extract => (NEXUS_PURPLE_TEXT, Icon::Unpack),
        DropIntent::Busy => (TEXT_MUTED, Icon::Warning),
    };

    let screen = ctx.screen_rect();
    if !screen.is_finite() || screen.width() < 40.0 || screen.height() < 40.0 {
        return;
    }
    let painter = ctx.layer_painter(LayerId::new(
        Order::Foreground,
        Id::new("nexus_drop_overlay"),
    ));
    painter.rect_filled(screen, 0.0, with_alpha(BG_DARK, 235));

    let frame = screen.shrink(14.0);
    let outline = [
        frame.left_top(),
        frame.right_top(),
        frame.right_bottom(),
        frame.left_bottom(),
        frame.left_top(),
    ];
    painter.extend(Shape::dashed_line(
        &outline,
        Stroke::new(2.0_f32, color),
        12.0,
        8.0,
    ));

    let center = screen.center();
    icons::paint(
        &painter,
        Rect::from_center_size(pos2(center.x, center.y - 46.0), vec2(64.0, 64.0)),
        icon,
        color,
    );
    painter.text(
        pos2(center.x, center.y + 14.0),
        Align2::CENTER_CENTER,
        headline(intent),
        mono(22.0),
        color,
    );

    let detail = match intent {
        DropIntent::Busy => "Cancel it or wait for it to finish before dropping files.".to_string(),
        _ => {
            let name = path.map(display_name).unwrap_or_default();
            if extra_items > 0 {
                format!(
                    "{name}  (+{extra_items} more ignored; drop a folder to pack several items)"
                )
            } else {
                name
            }
        }
    };
    if !detail.is_empty() {
        let font = sans(13.0);
        let max_width = (frame.width() - 40.0).max(40.0);
        let fitted = fit_middle_with(&detail, max_width, |text| {
            painter
                .layout_no_wrap(text.to_owned(), font.clone(), TEXT_MUTED)
                .size()
                .x
        });
        painter.text(
            pos2(center.x, center.y + 44.0),
            Align2::CENTER_CENTER,
            fitted,
            font.clone(),
            TEXT_MUTED,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intent_busy_overrides_everything() {
        assert_eq!(
            intent_for(Some(Path::new("a.nxar")), true),
            DropIntent::Busy
        );
        assert_eq!(intent_for(None, true), DropIntent::Busy);
    }

    #[test]
    fn test_intent_by_extension() {
        // Paths that do not exist are not directories, so the extension decides.
        assert_eq!(
            intent_for(Some(Path::new("no_such_dir/backup.nxar")), false),
            DropIntent::Extract
        );
        assert_eq!(
            intent_for(Some(Path::new("no_such_dir/old.NXS")), false),
            DropIntent::Extract
        );
        assert_eq!(
            intent_for(Some(Path::new("no_such_dir/report.pdf")), false),
            DropIntent::Compress
        );
        assert_eq!(intent_for(None, false), DropIntent::Unknown);
    }

    #[test]
    fn test_existing_directory_is_compressed_even_with_archive_extension() {
        let dir =
            std::env::temp_dir().join(format!("nexus_gui_test_dir_{}.nxar", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let intent = intent_for(Some(&dir), false);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(intent, DropIntent::Compress);
    }

    #[test]
    fn test_headlines() {
        assert_eq!(headline(DropIntent::Compress), "DROP TO COMPRESS");
        assert_eq!(headline(DropIntent::Extract), "DROP TO EXTRACT");
        assert!(!headline(DropIntent::Unknown).is_empty());
        assert!(!headline(DropIntent::Busy).is_empty());
    }
}
