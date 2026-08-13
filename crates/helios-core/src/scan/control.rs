//! Pause / resume / cancel for an in-flight scan.
//!
//! The handle is cloneable and `Send + Sync`, so the UI thread (or a Tauri
//! command) can hold one while worker threads observe it. Pausing parks the
//! workers on a condvar rather than spinning, so a paused scan costs no CPU —
//! important because a user pausing a scan usually wants their machine back.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Condvar, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScanState {
    Running,
    Paused,
    Cancelled,
    Done,
}

const RUNNING: u8 = 0;
const PAUSED: u8 = 1;
const CANCELLED: u8 = 2;

#[derive(Debug, Default)]
struct Inner {
    state: AtomicU8,
    lock: Mutex<()>,
    resumed: Condvar,
}

/// Cloneable control handle. All clones refer to the same scan.
#[derive(Debug, Clone, Default)]
pub struct ScanControl {
    inner: Arc<Inner>,
}

impl ScanControl {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self) -> ScanState {
        match self.inner.state.load(Ordering::Acquire) {
            PAUSED => ScanState::Paused,
            CANCELLED => ScanState::Cancelled,
            _ => ScanState::Running,
        }
    }

    pub fn pause(&self) {
        let _ =
            self.inner
                .state
                .compare_exchange(RUNNING, PAUSED, Ordering::AcqRel, Ordering::Relaxed);
    }

    pub fn resume(&self) {
        if self
            .inner
            .state
            .compare_exchange(PAUSED, RUNNING, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
        {
            // Take the lock so a worker cannot check the state and start
            // waiting between our store and the notify, which would otherwise
            // leave it parked forever.
            let _guard = self.inner.lock.lock().unwrap();
            self.inner.resumed.notify_all();
        }
    }

    /// Cancellation is terminal: a cancelled scan can never go back to running.
    pub fn cancel(&self) {
        self.inner.state.store(CANCELLED, Ordering::Release);
        let _guard = self.inner.lock.lock().unwrap();
        self.inner.resumed.notify_all();
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.state.load(Ordering::Acquire) == CANCELLED
    }

    /// Blocks while paused. Returns `false` if the scan was cancelled and the
    /// caller should stop.
    pub fn wait_if_paused(&self) -> bool {
        if self.inner.state.load(Ordering::Acquire) != PAUSED {
            return !self.is_cancelled();
        }
        let mut guard = self.inner.lock.lock().unwrap();
        while self.inner.state.load(Ordering::Acquire) == PAUSED {
            guard = self.inner.resumed.wait(guard).unwrap();
        }
        !self.is_cancelled()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    #[test]
    fn pause_blocks_until_resume() {
        let control = ScanControl::new();
        control.pause();
        let passed = Arc::new(AtomicBool::new(false));

        let worker = {
            let (control, passed) = (control.clone(), passed.clone());
            std::thread::spawn(move || {
                control.wait_if_paused();
                passed.store(true, Ordering::Release);
            })
        };

        std::thread::sleep(Duration::from_millis(50));
        assert!(!passed.load(Ordering::Acquire), "worker ran while paused");
        control.resume();
        worker.join().unwrap();
        assert!(passed.load(Ordering::Acquire));
        assert_eq!(control.state(), ScanState::Running);
    }

    #[test]
    fn cancel_releases_paused_workers_and_is_terminal() {
        let control = ScanControl::new();
        control.pause();
        let worker = {
            let control = control.clone();
            std::thread::spawn(move || control.wait_if_paused())
        };
        control.cancel();
        assert!(!worker.join().unwrap(), "cancelled wait must report stop");

        control.resume();
        assert_eq!(control.state(), ScanState::Cancelled);
    }
}
