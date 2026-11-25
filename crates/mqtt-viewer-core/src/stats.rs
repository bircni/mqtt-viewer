//! Message rate measurement.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Computes a messages-per-second rate from periodic samples of a running total.
#[derive(Debug, Clone)]
pub struct RateMeter {
    window: Duration,
    samples: VecDeque<(Instant, u64)>,
}

impl Default for RateMeter {
    fn default() -> Self {
        Self::new(Duration::from_secs(5))
    }
}

impl RateMeter {
    /// A meter averaging over `window`.
    #[must_use]
    pub const fn new(window: Duration) -> Self {
        Self { window, samples: VecDeque::new() }
    }

    /// Records that the running total was `total` at `now`.
    pub fn sample(&mut self, now: Instant, total: u64) {
        if self.samples.back().is_some_and(|(_, last)| total < *last) {
            // The counter was reset (new connection); start over.
            self.samples.clear();
        }
        self.samples.push_back((now, total));
        while self.samples.len() > 2
            && self.samples.front().is_some_and(|(at, _)| now.saturating_duration_since(*at) > self.window)
        {
            self.samples.pop_front();
        }
    }

    /// Messages per second over the window; 0 until two samples exist.
    #[must_use]
    pub fn per_second(&self) -> f64 {
        let (Some((first_at, first)), Some((last_at, last))) = (self.samples.front(), self.samples.back()) else {
            return 0.0;
        };
        let seconds = last_at.saturating_duration_since(*first_at).as_secs_f64();
        if seconds <= 0.0 {
            return 0.0;
        }
        #[expect(clippy::cast_precision_loss, reason = "a rate does not need more than 52 bits of message count")]
        let delta = last.saturating_sub(*first) as f64;
        delta / seconds
    }
}
