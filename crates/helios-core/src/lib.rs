//! # Helios scan engine
//!
//! A read-only, platform-agnostic disk usage engine: it enumerates volumes,
//! walks them in parallel, and answers the questions a storage visualizer asks
//! — what is big, what is where, what kind of thing is it.
//!
//! ## Read-only by construction
//!
//! This crate performs no mutating filesystem operation of any kind, anywhere:
//! no create, write, rename, delete, truncate, permission or timestamp change
//! against a scanned volume. The only bytes it ever writes are its own snapshot
//! cache under the user's application-support directory, and exports the user
//! explicitly asks for. `tests/read_only.rs` enforces this at the source level
//! so it cannot regress silently.
//!
//! ## Fully offline
//!
//! There is no networking code here, and no dependency that performs I/O
//! beyond the standard library. No telemetry, no analytics, no update check.
//!
//! ## Layout
//!
//! | Module | Role |
//! |--------|------|
//! | [`platform`] | The only OS-specific code; one module per target |
//! | [`model`] | The arena tree every other module reads |
//! | [`scan`] | Parallel walker, progress, pause/cancel, incremental rescan |
//! | [`query`] | Filtering, sorting, top-N, category aggregation |
//! | [`treemap`] | Squarified layout for the treemap view |
//! | [`snapshot`] | Binary cache with atomic writes |
//! | [`report`] | CSV / JSON / PDF export |
//! | [`fmt`] | Byte and date formatting |
//!
//! ## Example
//!
//! ```no_run
//! use helios_core::scan::{scan, ScanControl, ScanOptions};
//! use helios_core::query::{largest, Filter};
//!
//! let options = ScanOptions::new("/Users/me/Downloads");
//! let outcome = scan(&options, ScanControl::new(), |p| {
//!     println!("{} files, {} bytes", p.files_seen, p.bytes_seen);
//! });
//!
//! for entry in largest(&outcome.tree, &Filter::default(), 10, false) {
//!     println!("{:>12}  {}", entry.size, entry.path);
//! }
//! ```

#![forbid(unsafe_op_in_unsafe_fn)]
#![warn(missing_debug_implementations)]

/// A tiny stand-in for the `bitflags` crate.
///
/// Helios keeps its dependency list to `serde` and `crossbeam-channel`, both of
/// which earn their place at the IPC and concurrency boundaries. Flag sets do
/// not: what we need is a `u16` newtype with named constants, set operations
/// and serde support, which is short enough to own outright and read in one
/// sitting — and one fewer supply-chain surface for an app whose pitch is that
/// it never touches the network.
#[macro_export]
macro_rules! bitflags_lite {
    (
        $(#[$outer:meta])*
        pub struct $name:ident: $ty:ty {
            $(
                $(#[$inner:meta])*
                const $flag:ident = $value:expr;
            )*
        }
    ) => {
        $(#[$outer])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Default,
                 ::serde::Serialize, ::serde::Deserialize)]
        #[repr(transparent)]
        pub struct $name($ty);

        impl $name {
            $(
                $(#[$inner])*
                pub const $flag: $name = $name($value);
            )*

            const NAMES: &'static [(&'static str, $name)] =
                &[$((stringify!($flag), $name::$flag)),*];

            #[inline]
            pub const fn empty() -> Self { $name(0) }

            #[inline]
            pub const fn bits(self) -> $ty { self.0 }

            #[inline]
            pub const fn from_bits_truncate(bits: $ty) -> Self {
                let mut valid: $ty = 0;
                $( valid |= $name::$flag.0; )*
                $name(bits & valid)
            }

            #[inline]
            pub const fn contains(self, other: Self) -> bool {
                self.0 & other.0 == other.0
            }

            #[inline]
            pub const fn intersects(self, other: Self) -> bool {
                self.0 & other.0 != 0
            }

            #[inline]
            pub fn insert(&mut self, other: Self) { self.0 |= other.0; }

            #[inline]
            pub fn remove(&mut self, other: Self) { self.0 &= !other.0; }

            #[inline]
            pub fn set(&mut self, other: Self, value: bool) {
                if value { self.insert(other) } else { self.remove(other) }
            }
        }

        impl ::std::ops::BitOr for $name {
            type Output = Self;
            #[inline]
            fn bitor(self, rhs: Self) -> Self { $name(self.0 | rhs.0) }
        }

        impl ::std::ops::BitAnd for $name {
            type Output = Self;
            #[inline]
            fn bitand(self, rhs: Self) -> Self { $name(self.0 & rhs.0) }
        }

        impl ::std::fmt::Debug for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                let mut first = true;
                write!(f, "{}(", stringify!($name))?;
                for (label, flag) in Self::NAMES {
                    if self.contains(*flag) {
                        if !first { write!(f, "|")?; }
                        write!(f, "{label}")?;
                        first = false;
                    }
                }
                if first { write!(f, "empty")?; }
                write!(f, ")")
            }
        }
    };
}

pub mod category;
pub mod fmt;
pub mod model;
pub mod platform;
pub mod query;
pub mod report;
pub mod scan;
pub mod snapshot;
pub mod treemap;

pub use category::Category;
pub use model::{Node, NodeFlags, NodeId, Tree};
pub use platform::Volume;
pub use scan::{
    scan, scan_blocking, ScanControl, ScanOptions, ScanOutcome, ScanProgress, ScanState,
};
pub use snapshot::{Snapshot, SnapshotMeta};

/// Semantic version of the engine, surfaced in the About panel and in exports.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod flag_tests {
    use crate::model::NodeFlags;

    #[test]
    fn set_operations_behave() {
        let mut flags = NodeFlags::DIRECTORY | NodeFlags::HIDDEN;
        assert!(flags.contains(NodeFlags::DIRECTORY));
        assert!(!flags.contains(NodeFlags::SYSTEM));
        assert!(flags.contains(NodeFlags::DIRECTORY | NodeFlags::HIDDEN));

        flags.remove(NodeFlags::HIDDEN);
        assert!(!flags.contains(NodeFlags::HIDDEN));
        flags.set(NodeFlags::SYSTEM, true);
        assert!(flags.intersects(NodeFlags::SYSTEM | NodeFlags::SYMLINK));
    }

    #[test]
    fn unknown_bits_are_dropped_on_load() {
        // Snapshots written by a future version must not resurrect flags this
        // build does not understand.
        let restored = NodeFlags::from_bits_truncate(0xFFFF);
        assert_eq!(restored.bits() & 0x8000, 0);
    }

    #[test]
    fn debug_output_names_the_flags() {
        let flags = NodeFlags::DIRECTORY | NodeFlags::PACKAGE;
        assert_eq!(format!("{flags:?}"), "NodeFlags(DIRECTORY|PACKAGE)");
        assert_eq!(format!("{:?}", NodeFlags::empty()), "NodeFlags(empty)");
    }
}
