//! Shared application state.
//!
//! Scanned trees live here, in the Rust process — never in the webview. The UI
//! holds only the few hundred rows it is currently drawing, so a 10-million-file
//! scan costs the renderer nothing and the IPC bridge never has to serialize a
//! tree. Every view is a query against this state.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use helios_core::model::Tree;
use helios_core::scan::{ScanControl, ScanStats};
use helios_core::snapshot::SnapshotMeta;

/// A completed (or cancelled) scan the UI can query.
#[derive(Debug, Clone)]
pub struct LoadedScan {
    pub meta: SnapshotMeta,
    pub tree: Arc<Tree>,
    pub stats: ScanStats,
    /// True when the tree came from the snapshot cache rather than a fresh walk.
    pub from_cache: bool,
}

#[derive(Debug, Default)]
pub struct AppState {
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    /// Keyed by scan id (the volume id, or the scanned path for a folder scan).
    scans: HashMap<String, LoadedScan>,
    /// Control handle for the scan currently in flight, if any.
    active: Option<(String, ScanControl)>,
}

impl AppState {
    pub fn insert(&self, id: String, scan: LoadedScan) {
        let mut inner = self.inner.lock().unwrap();
        inner.scans.insert(id, scan);
    }

    /// Returns a cheap handle to a scan's tree.
    ///
    /// The `Arc` clone is what lets a long query run without holding the state
    /// lock: a treemap layout over a large tree takes tens of milliseconds, and
    /// blocking every other command for that long would make the UI stutter.
    pub fn get(&self, id: &str) -> Option<LoadedScan> {
        self.inner.lock().unwrap().scans.get(id).cloned()
    }

    pub fn ids(&self) -> Vec<String> {
        self.inner.lock().unwrap().scans.keys().cloned().collect()
    }

    pub fn forget(&self, id: &str) {
        self.inner.lock().unwrap().scans.remove(id);
    }

    pub fn begin(&self, id: String, control: ScanControl) {
        let mut inner = self.inner.lock().unwrap();
        // Only one scan runs at a time: two concurrent walks contend for the
        // same disk and both finish later than either would alone.
        if let Some((_, previous)) = inner.active.take() {
            previous.cancel();
        }
        inner.active = Some((id, control));
    }

    pub fn end(&self, id: &str) {
        let mut inner = self.inner.lock().unwrap();
        if inner.active.as_ref().is_some_and(|(active, _)| active == id) {
            inner.active = None;
        }
    }

    pub fn active_control(&self) -> Option<ScanControl> {
        self.inner
            .lock()
            .unwrap()
            .active
            .as_ref()
            .map(|(_, control)| control.clone())
    }

    pub fn active_id(&self) -> Option<String> {
        self.inner
            .lock()
            .unwrap()
            .active
            .as_ref()
            .map(|(id, _)| id.clone())
    }
}
