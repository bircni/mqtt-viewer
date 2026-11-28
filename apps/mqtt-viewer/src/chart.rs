//! Numeric time series shown in the chart panel.

use std::collections::VecDeque;
use std::fmt::Write as _;

use jiff::Timestamp;
use mqtt_viewer_core::decode::Decoded;
use mqtt_viewer_core::numeric;

/// Points kept per chart.
pub const MAX_POINTS: usize = 5000;

/// One chart: the numbers found at a JSON Pointer in the payloads of one topic.
#[derive(Debug, Clone, PartialEq)]
pub struct Chart {
    /// Topic the values come from.
    pub topic: String,
    /// JSON Pointer into the decoded payload; empty for the payload itself.
    pub pointer: String,
    /// `(unix seconds, value)` pairs, oldest first.
    pub points: VecDeque<[f64; 2]>,
    /// Whether new values are ignored.
    pub paused: bool,
    /// Draw as steps (value holds until the next sample) instead of straight lines.
    pub stepped: bool,
    /// Only show the last this many seconds; `None` shows everything kept.
    pub window_secs: Option<f64>,
    /// Fixed value range; `None` scales to the data.
    pub y_range: Option<(f64, f64)>,
    /// The plot this series is drawn in. Series sharing a plot are drawn together; the first of them carries
    /// the plot's settings (window, steps, range).
    pub plot: u64,
}

impl Chart {
    /// An empty chart for `topic` and `pointer`.
    #[must_use]
    pub fn new(topic: impl Into<String>, pointer: impl Into<String>) -> Self {
        Self {
            topic: topic.into(),
            pointer: pointer.into(),
            points: VecDeque::new(),
            paused: false,
            stepped: false,
            window_secs: None,
            y_range: None,
            plot: 0,
        }
    }

    /// Title shown above the chart.
    #[must_use]
    pub fn label(&self) -> String {
        if self.pointer.is_empty() { self.topic.clone() } else { format!("{} {}", self.topic, self.pointer) }
    }

    /// Adds the value found in `decoded`, if there is one and the chart is not paused.
    pub fn feed(&mut self, at: Timestamp, decoded: &Decoded) {
        if self.paused {
            return;
        }
        if let Some(value) = numeric::extract(decoded, &self.pointer) {
            self.push(at, value);
        }
    }

    /// Adds a point, dropping the oldest when the chart is full.
    pub fn push(&mut self, at: Timestamp, value: f64) {
        #[expect(clippy::cast_precision_loss, reason = "millisecond timestamps fit into an f64 exactly")]
        let seconds = at.as_millisecond() as f64 / 1000.0;
        self.points.push_back([seconds, value]);
        while self.points.len() > MAX_POINTS {
            self.points.pop_front();
        }
    }

    /// The points to draw: limited to the time window (ending at the newest point) and, when stepped, with an
    /// extra corner point before every change.
    #[must_use]
    pub fn visible_points(&self) -> Vec<[f64; 2]> {
        let newest = self.points.back().map_or(0.0, |point| point[0]);
        let start = self.window_secs.map_or(f64::NEG_INFINITY, |window| newest - window);
        let mut out = Vec::with_capacity(self.points.len());
        for point in self.points.iter().filter(|point| point[0] >= start) {
            if self.stepped
                && let Some(previous) = out.last().copied().map(|previous: [f64; 2]| previous[1])
            {
                out.push([point[0], previous]);
            }
            out.push(*point);
        }
        out
    }

    /// The most recent value.
    #[must_use]
    pub fn latest(&self) -> Option<f64> {
        self.points.back().map(|point| point[1])
    }
}

/// The points of `charts` as CSV: one row per point with its time, series and value.
#[must_use]
pub fn to_csv<'a>(charts: impl IntoIterator<Item = &'a Chart>) -> String {
    let mut out = String::from("timestamp,topic,pointer,value\n");
    for chart in charts {
        for [seconds, value] in &chart.points {
            #[expect(clippy::cast_possible_truncation, reason = "plot coordinates are far inside the i64 range")]
            let at = Timestamp::from_millisecond((seconds * 1000.0).round() as i64)
                .map_or_else(|_| seconds.to_string(), |at| at.to_string());
            let field = |text: &str| {
                if text.contains([',', '"', '\n']) {
                    format!("\"{}\"", text.replace('"', "\"\""))
                } else {
                    text.to_owned()
                }
            };
            let _ = writeln!(out, "{at},{},{},{value}", field(&chart.topic), field(&chart.pointer));
        }
    }
    out
}
