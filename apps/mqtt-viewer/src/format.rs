//! Human-readable rendering of timestamps, sizes and counts.

use jiff::Timestamp;
use jiff::tz::TimeZone;
use mqtt_viewer_core::config::TimestampFormat;

/// Renders `timestamp` in the chosen style. `now` anchors the relative style, `zone` the local ones.
#[must_use]
pub fn timestamp(timestamp: Timestamp, style: TimestampFormat, now: Timestamp, zone: &TimeZone) -> String {
    match style {
        TimestampFormat::Time => timestamp.to_zoned_datetime(zone.clone()).strftime("%H:%M:%S%.3f").to_string(),
        TimestampFormat::DateTime => {
            timestamp.to_zoned_datetime(zone.clone()).strftime("%Y-%m-%d %H:%M:%S").to_string()
        }
        TimestampFormat::Iso => timestamp.to_string(),
        TimestampFormat::Relative => relative(now.duration_since(timestamp).as_secs()),
    }
}

/// `12 s ago`, `3 min ago`, ... for an age in seconds; ages in the future count as "now".
#[must_use]
pub fn relative(age_secs: i64) -> String {
    match age_secs {
        ..1 => "just now".to_owned(),
        1..60 => format!("{age_secs} s ago"),
        60..3600 => format!("{} min ago", age_secs / 60),
        3600..86_400 => format!("{} h ago", age_secs / 3600),
        _ => format!("{} d ago", age_secs / 86_400),
    }
}

/// A byte count with a binary unit, e.g. `1.5 KiB`.
#[must_use]
pub fn bytes(count: usize) -> String {
    const UNITS: [&str; 4] = ["KiB", "MiB", "GiB", "TiB"];
    if count < 1024 {
        return format!("{count} B");
    }
    #[expect(clippy::cast_precision_loss, reason = "display only")]
    let mut value = count as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// A count with spaces as thousands separators, e.g. `1 234 567`.
#[must_use]
pub fn count(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(digit);
    }
    out
}

/// A count with its noun, pluralised with an `s` unless the count is one, e.g. `1 topic`, `12 topics`.
#[must_use]
pub fn plural(value: u64, noun: &str) -> String {
    format!("{} {noun}{}", count(value), if value == 1 { "" } else { "s" })
}

/// A rate with a sensible precision, e.g. `0.4 msg/s` or `1 520 msg/s`.
#[must_use]
pub fn rate(per_second: f64) -> String {
    if per_second < 10.0 {
        format!("{per_second:.1} msg/s")
    } else {
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a rounded, non-negative rate")]
        let rounded = per_second.round() as u64;
        format!("{} msg/s", count(rounded))
    }
}
