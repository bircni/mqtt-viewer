use jiff::Timestamp;
use jiff::tz::TimeZone;
use mqtt_viewer_core::config::TimestampFormat;

use crate::format::{bytes, count, plural, rate, relative, timestamp};

#[test]
fn timestamps_follow_the_chosen_style() {
    let at = Timestamp::from_millisecond(1_700_000_000_123).unwrap();
    let now = Timestamp::from_second(1_700_000_090).unwrap();
    let utc = TimeZone::UTC;
    assert_eq!(timestamp(at, TimestampFormat::Time, now, &utc), "22:13:20.123");
    assert_eq!(timestamp(at, TimestampFormat::DateTime, now, &utc), "2023-11-14 22:13:20");
    assert_eq!(timestamp(at, TimestampFormat::Iso, now, &utc), "2023-11-14T22:13:20.123Z");
    assert_eq!(timestamp(at, TimestampFormat::Relative, now, &utc), "1 min ago");
    let berlin = TimeZone::get("Europe/Berlin").unwrap();
    assert_eq!(timestamp(at, TimestampFormat::Time, now, &berlin), "23:13:20.123");
}

#[test]
fn relative_ages_use_the_largest_fitting_unit() {
    let table = [(-5, "just now"), (0, "just now"), (1, "1 s ago"), (59, "59 s ago"), (60, "1 min ago")];
    for (age, expected) in table {
        assert_eq!(relative(age), expected);
    }
    assert_eq!(relative(3599), "59 min ago");
    assert_eq!(relative(3600), "1 h ago");
    assert_eq!(relative(86_399), "23 h ago");
    assert_eq!(relative(86_400 * 3), "3 d ago");
}

#[test]
fn byte_counts_use_binary_units() {
    assert_eq!(bytes(0), "0 B");
    assert_eq!(bytes(1023), "1023 B");
    assert_eq!(bytes(1024), "1.0 KiB");
    assert_eq!(bytes(1536), "1.5 KiB");
    assert_eq!(bytes(5 * 1024 * 1024), "5.0 MiB");
    assert_eq!(bytes(3 * 1024 * 1024 * 1024), "3.0 GiB");
    assert_eq!(bytes(usize::MAX), "16777216.0 TiB");
}

#[test]
fn counts_are_grouped_in_thousands() {
    assert_eq!(count(0), "0");
    assert_eq!(count(999), "999");
    assert_eq!(count(1000), "1 000");
    assert_eq!(count(1_234_567), "1 234 567");
    assert_eq!(plural(1, "topic"), "1 topic");
    assert_eq!(plural(0, "topic"), "0 topics");
    assert_eq!(plural(2500, "message"), "2 500 messages");
}

#[test]
fn rates_get_a_sensible_precision() {
    assert_eq!(rate(0.0), "0.0 msg/s");
    assert_eq!(rate(0.44), "0.4 msg/s");
    assert_eq!(rate(9.96), "10.0 msg/s");
    assert_eq!(rate(10.4), "10 msg/s");
    assert_eq!(rate(1520.6), "1 521 msg/s");
}
