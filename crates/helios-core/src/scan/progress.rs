//! Scan progress accounting and completion estimation.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::control::ScanState;

/// A progress sample, shipped to the UI at most once per
/// [`ScanOptions::progress_interval`](super::ScanOptions::progress_interval).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanProgress {
    pub state: ScanState,
    pub files_seen: u64,
    pub dirs_seen: u64,
    pub bytes_seen: u64,
    /// Directories reused wholesale from the previous snapshot.
    pub dirs_reused: u64,
    pub errors: u64,
    /// Directory currently being read, for the "Scanning …" line. Sampled, not
    /// exhaustive — with 8 threads there are 8 of these at any moment.
    pub current_path: String,
    pub elapsed_ms: u64,
    /// `None` until there is enough signal to estimate honestly.
    pub eta_ms: Option<u64>,
    /// 0.0–1.0, derived from bytes seen against the volume's used bytes.
    pub fraction: Option<f32>,
}

/// Turns raw counters into an ETA.
///
/// The estimate is anchored on the volume's used-bytes figure from the OS
/// rather than on file counts, because bytes are what the user watches and
/// because directory counts are wildly non-uniform across a tree. Early
/// samples are suppressed entirely: an ETA that swings from "4 hours" to "20
/// seconds" in the first moments is worse than no ETA at all.
#[derive(Debug)]
pub struct EtaEstimator {
    started: Instant,
    expected_bytes: Option<u64>,
    /// Exponentially smoothed bytes/second.
    rate: f64,
    last_sample: Option<(Instant, u64)>,
}

/// Below this we do not show an ETA at all.
const MIN_ELAPSED: Duration = Duration::from_millis(1500);
/// Weight of each new observation in the smoothed rate.
const SMOOTHING: f64 = 0.25;

impl EtaEstimator {
    pub fn new(expected_bytes: Option<u64>) -> Self {
        EtaEstimator {
            started: Instant::now(),
            expected_bytes: expected_bytes.filter(|b| *b > 0),
            rate: 0.0,
            last_sample: None,
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Feeds a new byte count and returns `(eta, fraction_complete)`.
    pub fn update(&mut self, bytes_seen: u64) -> (Option<u64>, Option<f32>) {
        let now = Instant::now();
        if let Some((prev_at, prev_bytes)) = self.last_sample {
            let dt = now.duration_since(prev_at).as_secs_f64();
            if dt > 0.05 {
                let instant_rate = bytes_seen.saturating_sub(prev_bytes) as f64 / dt;
                self.rate = if self.rate == 0.0 {
                    instant_rate
                } else {
                    self.rate * (1.0 - SMOOTHING) + instant_rate * SMOOTHING
                };
                self.last_sample = Some((now, bytes_seen));
            }
        } else {
            self.last_sample = Some((now, bytes_seen));
        }

        let expected = self.expected_bytes;
        let fraction = expected.map(|e| (bytes_seen as f64 / e as f64).clamp(0.0, 1.0) as f32);

        if self.started.elapsed() < MIN_ELAPSED || self.rate <= 0.0 {
            return (None, fraction);
        }
        let eta = expected.and_then(|expected| {
            let remaining = expected.saturating_sub(bytes_seen) as f64;
            // Scans routinely exceed their estimate (compressed volumes,
            // reused snapshots); report "almost done" rather than a negative.
            (remaining > 0.0).then(|| (remaining / self.rate * 1000.0) as u64)
        });
        (eta, fraction)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_eta_before_the_warmup_window() {
        let mut est = EtaEstimator::new(Some(1_000_000));
        let (eta, fraction) = est.update(100_000);
        assert!(
            eta.is_none(),
            "ETA must stay silent while it would be noise"
        );
        assert_eq!(fraction, Some(0.1));
    }

    #[test]
    fn fraction_is_clamped_when_a_scan_overshoots() {
        let mut est = EtaEstimator::new(Some(1_000));
        let (_, fraction) = est.update(9_999);
        assert_eq!(fraction, Some(1.0));
    }

    #[test]
    fn unknown_expected_size_yields_no_fraction() {
        let mut est = EtaEstimator::new(None);
        assert_eq!(est.update(5_000), (None, None));
    }
}
