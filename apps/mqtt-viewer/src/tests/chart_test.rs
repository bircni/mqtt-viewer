use jiff::Timestamp;
use mqtt_viewer_core::decode::{Decoder, Format};

use crate::chart::{Chart, MAX_POINTS};

fn at(second: i64) -> Timestamp {
    Timestamp::from_second(second).unwrap()
}

#[test]
fn labels_include_the_pointer() {
    assert_eq!(Chart::new("a/b", "").label(), "a/b");
    assert_eq!(Chart::new("a/b", "/temp").label(), "a/b /temp");
}

#[test]
fn feeding_extracts_numbers_and_respects_pause() {
    let decoder = Decoder::new();
    let mut chart = Chart::new("t", "/temp");
    chart.feed(at(1), &decoder.decode("t", br#"{"temp": 20.5}"#, Format::Auto));
    chart.feed(at(2), &decoder.decode("t", br#"{"other": 1}"#, Format::Auto));
    chart.feed(at(3), &decoder.decode("t", b"not json", Format::Auto));
    assert_eq!(chart.points, [[1.0, 20.5]]);
    assert_eq!(chart.latest(), Some(20.5));
    chart.paused = true;
    chart.feed(at(4), &decoder.decode("t", br#"{"temp": 30}"#, Format::Auto));
    assert_eq!(chart.points.len(), 1);
    assert_eq!(Chart::new("t", "").latest(), None);
}

#[test]
fn old_points_are_dropped_when_full() {
    let mut chart = Chart::new("t", "");
    for second in 0..i64::try_from(MAX_POINTS).unwrap() + 10 {
        chart.push(at(second), 1.0);
    }
    assert_eq!(chart.points.len(), MAX_POINTS);
    assert_eq!(chart.points[0], [10.0, 1.0]);
}

#[test]
fn visible_points_follow_window_and_step_mode() {
    let mut chart = Chart::new("t", "");
    assert_eq!(chart.visible_points().len(), 0);
    for (second, value) in [(0, 1.0), (10, 2.0), (20, 3.0)] {
        chart.push(at(second), value);
    }
    assert_eq!(chart.visible_points(), [[0.0, 1.0], [10.0, 2.0], [20.0, 3.0]]);
    chart.window_secs = Some(10.0);
    assert_eq!(chart.visible_points(), [[10.0, 2.0], [20.0, 3.0]]);
    chart.stepped = true;
    assert_eq!(chart.visible_points(), [[10.0, 2.0], [20.0, 2.0], [20.0, 3.0]]);
}
