use std::time::{Duration, Instant};

use crate::stats::RateMeter;

#[test]
fn rate_needs_two_samples() {
    let mut meter = RateMeter::default();
    assert!(meter.per_second().abs() < f64::EPSILON);
    let start = Instant::now();
    meter.sample(start, 100);
    assert!(meter.per_second().abs() < f64::EPSILON);
    meter.sample(start, 200);
    assert!(meter.per_second().abs() < f64::EPSILON, "zero elapsed time must not divide by zero");
}

#[test]
fn rate_is_averaged_over_the_window() {
    let mut meter = RateMeter::new(Duration::from_secs(2));
    let start = Instant::now();
    for second in 0..=10_u64 {
        meter.sample(start + Duration::from_secs(second), second * 50);
    }
    assert!((meter.per_second() - 50.0).abs() < 1e-9, "{}", meter.per_second());
    // A burst only counts while it is inside the window.
    meter.sample(start + Duration::from_secs(11), 10 * 50 + 1000);
    assert!(meter.per_second() > 300.0);
    for second in 12..=20_u64 {
        meter.sample(start + Duration::from_secs(second), 10 * 50 + 1000);
    }
    assert!(meter.per_second().abs() < f64::EPSILON);
}

#[test]
fn counter_reset_starts_over() {
    let mut meter = RateMeter::default();
    let start = Instant::now();
    meter.sample(start, 1000);
    meter.sample(start + Duration::from_secs(1), 2000);
    meter.sample(start + Duration::from_secs(2), 5);
    assert!(meter.per_second().abs() < f64::EPSILON);
    meter.sample(start + Duration::from_secs(3), 15);
    assert!((meter.per_second() - 10.0).abs() < 1e-9);
}
