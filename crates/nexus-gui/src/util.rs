//! Pure formatting and arithmetic helpers shared by every screen: byte
//! sizes, durations, speeds, ratios and middle-elision of long strings.
//!
//! Nothing here touches egui or the filesystem, so all of it is unit-tested.

use std::time::Duration;

const KIB: f64 = 1024.0;
const MIB: f64 = KIB * 1024.0;
const GIB: f64 = MIB * 1024.0;
const TIB: f64 = GIB * 1024.0;

/// Formats a byte count with a binary unit, e.g. `1.5 MB`.
pub fn format_bytes(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= TIB {
        format!("{:.2} TB", b / TIB)
    } else if b >= GIB {
        format!("{:.2} GB", b / GIB)
    } else if b >= MIB {
        format!("{:.1} MB", b / MIB)
    } else if b >= KIB {
        format!("{:.1} KB", b / KIB)
    } else {
        format!("{bytes} B")
    }
}

/// Formats a duration as a clock: `MM:SS`, or `H:MM:SS` from one hour up.
pub fn format_clock(d: Duration) -> String {
    let total = d.as_secs();
    let (hours, mins, secs) = (total / 3600, (total % 3600) / 60, total % 60);
    if hours > 0 {
        format!("{hours}:{mins:02}:{secs:02}")
    } else {
        format!("{mins:02}:{secs:02}")
    }
}

/// Formats a number of seconds as a clock, or `--:--` when unknown.
pub fn format_clock_opt(secs: Option<f64>) -> String {
    match secs {
        Some(s) if s.is_finite() && s >= 0.0 => {
            // Cap at 99 hours so a wild early estimate cannot overflow the field.
            format_clock(Duration::from_secs_f64(s.min(99.0 * 3600.0)))
        }
        _ => "--:--".to_string(),
    }
}

/// Formats an *estimated* duration coarsely, e.g. `< 1 s`, `~12 s`,
/// `~3 min`, `~1 h 20 min`. Returns `--` for non-finite input.
pub fn format_estimate(secs: f64) -> String {
    if !secs.is_finite() || secs < 0.0 {
        return "--".to_string();
    }
    if secs < 1.0 {
        "< 1 s".to_string()
    } else if secs < 90.0 {
        format!("~{} s", secs.round() as u64)
    } else if secs < 90.0 * 60.0 {
        format!("~{} min", (secs / 60.0).round() as u64)
    } else if secs < 48.0 * 3600.0 {
        let total_min = (secs / 60.0).round() as u64;
        format!("~{} h {:02} min", total_min / 60, total_min % 60)
    } else {
        format!("~{} days", (secs / 86_400.0).round() as u64)
    }
}

/// Formats a precise elapsed time for result screens, e.g. `0.84 s`,
/// `12.3 s`, `02:05`.
pub fn format_elapsed(d: Duration) -> String {
    let secs = d.as_secs_f64();
    if secs < 10.0 {
        format!("{secs:.2} s")
    } else if secs < 60.0 {
        format!("{secs:.1} s")
    } else {
        format_clock(d)
    }
}

/// Throughput in MiB per second; zero when no time has passed.
pub fn mb_per_sec(bytes: u64, secs: f64) -> f64 {
    if !secs.is_finite() || secs <= 0.0 {
        return 0.0;
    }
    bytes as f64 / MIB / secs
}

/// Formats a throughput value with sensible precision, e.g. `0.45 MB/s`,
/// `12.3 MB/s`, `105 MB/s`.
pub fn format_speed(mb_s: f64) -> String {
    if !mb_s.is_finite() || mb_s < 0.0 {
        return "-- MB/s".to_string();
    }
    if mb_s < 10.0 {
        format!("{mb_s:.2} MB/s")
    } else if mb_s < 100.0 {
        format!("{mb_s:.1} MB/s")
    } else {
        format!("{mb_s:.0} MB/s")
    }
}

/// Compressed size as a percentage of the original, or `None` when the
/// original size is zero (the ratio is undefined).
pub fn ratio_percent(compressed: u64, original: u64) -> Option<f64> {
    if original == 0 {
        None
    } else {
        Some(compressed as f64 / original as f64 * 100.0)
    }
}

/// Formats a ratio from [`ratio_percent`], or `--` when undefined.
pub fn format_ratio(compressed: u64, original: u64) -> String {
    match ratio_percent(compressed, original) {
        Some(r) => format!("{r:.1} %"),
        None => "-- %".to_string(),
    }
}

/// Bytes saved by compression; zero when the output is not smaller.
pub fn bytes_saved(original: u64, compressed: u64) -> u64 {
    original.saturating_sub(compressed)
}

/// `part / whole` as a fraction in `[0, 1]`; zero when `whole` is zero.
pub fn fraction_of(part: u64, whole: u64) -> f32 {
    if whole == 0 {
        0.0
    } else {
        (part as f64 / whole as f64).clamp(0.0, 1.0) as f32
    }
}

/// Shortens `text` to at most `max_chars` characters by replacing the middle
/// with an ellipsis, keeping the start and the end (where file names live).
pub fn elide_middle(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }
    if max_chars == 1 {
        return "\u{2026}".to_string();
    }
    let keep = max_chars - 1;
    // Favour the tail slightly: the end of a path is the informative part.
    let head = keep / 2;
    let tail = keep - head;
    let mut out = String::with_capacity(text.len().min(max_chars * 4));
    out.extend(text.chars().take(head));
    out.push('\u{2026}');
    out.extend(text.chars().skip(count - tail));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes_units() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(format_bytes(3 * 1024 * 1024 * 1024), "3.00 GB");
        assert_eq!(format_bytes(2 * 1024 * 1024 * 1024 * 1024), "2.00 TB");
    }

    #[test]
    fn test_format_clock() {
        assert_eq!(format_clock(Duration::from_secs(0)), "00:00");
        assert_eq!(format_clock(Duration::from_secs(75)), "01:15");
        assert_eq!(format_clock(Duration::from_secs(3599)), "59:59");
        assert_eq!(format_clock(Duration::from_secs(3661)), "1:01:01");
    }

    #[test]
    fn test_format_clock_opt_handles_unknown_and_bad_values() {
        assert_eq!(format_clock_opt(None), "--:--");
        assert_eq!(format_clock_opt(Some(f64::NAN)), "--:--");
        assert_eq!(format_clock_opt(Some(f64::INFINITY)), "--:--");
        assert_eq!(format_clock_opt(Some(-1.0)), "--:--");
        assert_eq!(format_clock_opt(Some(61.4)), "01:01");
        assert_eq!(format_clock_opt(Some(1.0e12)), "99:00:00");
    }

    #[test]
    fn test_format_estimate_buckets() {
        assert_eq!(format_estimate(0.2), "< 1 s");
        assert_eq!(format_estimate(12.4), "~12 s");
        assert_eq!(format_estimate(180.0), "~3 min");
        assert_eq!(format_estimate(4800.0), "~80 min");
        assert_eq!(format_estimate(2.0 * 3600.0 + 20.0 * 60.0), "~2 h 20 min");
        assert_eq!(format_estimate(3.0 * 86_400.0), "~3 days");
        assert_eq!(format_estimate(f64::NAN), "--");
        assert_eq!(format_estimate(f64::INFINITY), "--");
    }

    #[test]
    fn test_format_elapsed() {
        assert_eq!(format_elapsed(Duration::from_millis(840)), "0.84 s");
        assert_eq!(format_elapsed(Duration::from_millis(12_300)), "12.3 s");
        assert_eq!(format_elapsed(Duration::from_secs(125)), "02:05");
    }

    #[test]
    fn test_mb_per_sec_guards_zero_time() {
        assert_eq!(mb_per_sec(1024 * 1024, 0.0), 0.0);
        assert_eq!(mb_per_sec(1024 * 1024, -1.0), 0.0);
        assert_eq!(mb_per_sec(1024 * 1024, f64::NAN), 0.0);
        assert!((mb_per_sec(10 * 1024 * 1024, 2.0) - 5.0).abs() < 1e-9);
    }

    #[test]
    fn test_format_speed_precision() {
        assert_eq!(format_speed(0.456), "0.46 MB/s");
        assert_eq!(format_speed(12.34), "12.3 MB/s");
        assert_eq!(format_speed(105.4), "105 MB/s");
        assert_eq!(format_speed(f64::NAN), "-- MB/s");
    }

    #[test]
    fn test_ratio_and_savings() {
        assert_eq!(ratio_percent(50, 0), None);
        assert_eq!(ratio_percent(25, 100), Some(25.0));
        assert_eq!(format_ratio(382, 1000), "38.2 %");
        assert_eq!(format_ratio(1, 0), "-- %");
        assert_eq!(bytes_saved(100, 40), 60);
        assert_eq!(bytes_saved(40, 100), 0);
    }

    #[test]
    fn test_fraction_of_is_clamped() {
        assert_eq!(fraction_of(5, 0), 0.0);
        assert_eq!(fraction_of(5, 10), 0.5);
        assert_eq!(fraction_of(50, 10), 1.0);
    }

    #[test]
    fn test_elide_middle_short_text_unchanged() {
        assert_eq!(elide_middle("abc", 10), "abc");
        assert_eq!(elide_middle("abcdef", 6), "abcdef");
    }

    #[test]
    fn test_elide_middle_keeps_both_ends() {
        let out = elide_middle("C:/very/long/path/to/archive.nxar", 15);
        assert_eq!(out.chars().count(), 15);
        assert!(out.starts_with("C:/very"));
        assert!(out.ends_with("ve.nxar"));
        assert!(out.contains('\u{2026}'));
    }

    #[test]
    fn test_elide_middle_degenerate_widths() {
        assert_eq!(elide_middle("abcdef", 0), "");
        assert_eq!(elide_middle("abcdef", 1), "\u{2026}");
        assert_eq!(elide_middle("abcdef", 2), "\u{2026}f");
    }

    #[test]
    fn test_elide_middle_is_char_safe_for_unicode() {
        let out = elide_middle("données-très-longues-éèà.txt", 9);
        assert_eq!(out.chars().count(), 9);
    }
}
