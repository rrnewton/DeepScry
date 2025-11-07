//! Allocator selection and statistics tracking for benchmarks
//!
//! This module provides a unified interface for allocator statistics that works
//! with both stats_alloc (allocation tracking) and mimalloc (high performance).
//!
//! ## Feature flags
//!
//! - `bench-stats-alloc`: Use stats_alloc with allocation tracking
//! - `bench-mimalloc`: Use mimalloc for maximum performance (no tracking)
//! - `bench-jemalloc`: Use jemalloc with optional statistics support
//!
//! These features are mutually exclusive - only one can be enabled at a time.
//! If no feature is enabled, the system default allocator (glibc malloc) is used.
//!
//! ## Usage
//!
//! ```rust
//! use allocator::{AllocStats, track_allocations};
//!
//! let stats = track_allocations(|| {
//!     // Code to measure
//! });
//!
//! println!("Allocated: {} bytes", stats.bytes_allocated);
//! ```

// Compile-time checks: ensure features are mutually exclusive
#[cfg(all(feature = "bench-stats-alloc", feature = "bench-mimalloc"))]
compile_error!("Features 'bench-stats-alloc' and 'bench-mimalloc' are mutually exclusive. Enable only one.");

#[cfg(all(feature = "bench-stats-alloc", feature = "bench-jemalloc"))]
compile_error!("Features 'bench-stats-alloc' and 'bench-jemalloc' are mutually exclusive. Enable only one.");

#[cfg(all(feature = "bench-mimalloc", feature = "bench-jemalloc"))]
compile_error!("Features 'bench-mimalloc' and 'bench-jemalloc' are mutually exclusive. Enable only one.");

// Import allocator types
#[cfg(feature = "bench-stats-alloc")]
use stats_alloc::{Region, StatsAlloc, INSTRUMENTED_SYSTEM};
#[cfg(feature = "bench-stats-alloc")]
use std::alloc::System;

// Global allocator selection via feature flags
#[cfg(feature = "bench-stats-alloc")]
#[global_allocator]
pub static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

#[cfg(feature = "bench-mimalloc")]
#[global_allocator]
pub static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[cfg(feature = "bench-jemalloc")]
#[global_allocator]
pub static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

/// Allocation statistics - works with or without tracking
#[derive(Debug, Clone, Copy, Default)]
#[allow(dead_code)] // Some fields used only in specific features
pub struct AllocStats {
    pub bytes_allocated: usize,
    pub bytes_deallocated: usize,
    pub bytes_reallocated: usize,
    pub allocations: usize,
    pub deallocations: usize,
    pub reallocations: usize,
}

impl AllocStats {
    /// Create zero stats (used when tracking is disabled)
    pub const fn zero() -> Self {
        AllocStats {
            bytes_allocated: 0,
            bytes_deallocated: 0,
            bytes_reallocated: 0,
            allocations: 0,
            deallocations: 0,
            reallocations: 0,
        }
    }

    /// Calculate net bytes (allocated - deallocated)
    #[allow(dead_code)]
    pub fn net_bytes(&self) -> i64 {
        self.bytes_allocated as i64 - self.bytes_deallocated as i64
    }
}

#[cfg(feature = "bench-stats-alloc")]
impl From<stats_alloc::Stats> for AllocStats {
    fn from(stats: stats_alloc::Stats) -> Self {
        AllocStats {
            bytes_allocated: stats.bytes_allocated,
            bytes_deallocated: stats.bytes_deallocated,
            bytes_reallocated: stats.bytes_reallocated.max(0) as usize,
            allocations: stats.allocations,
            deallocations: stats.deallocations,
            reallocations: stats.reallocations,
        }
    }
}

/// Guard for allocation tracking - measures allocations in its scope
pub struct AllocTracker {
    #[cfg(feature = "bench-stats-alloc")]
    region: Region<'static, System>,
    #[cfg(not(feature = "bench-stats-alloc"))]
    _phantom: (),
}

impl AllocTracker {
    /// Create a new allocation tracker
    pub fn new() -> Self {
        #[cfg(feature = "bench-stats-alloc")]
        {
            AllocTracker {
                region: Region::new(GLOBAL),
            }
        }
        #[cfg(not(feature = "bench-stats-alloc"))]
        {
            AllocTracker { _phantom: () }
        }
    }

    /// Get statistics since tracker was created
    pub fn stats(&self) -> AllocStats {
        #[cfg(feature = "bench-stats-alloc")]
        {
            self.region.change().into()
        }
        #[cfg(not(feature = "bench-stats-alloc"))]
        {
            AllocStats::zero()
        }
    }
}

impl Default for AllocTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Track allocations for a closure
///
/// Returns the allocation statistics for the closure execution.
///
/// # Example
///
/// ```rust
/// let stats = track_allocations(|| {
///     let v = vec![1, 2, 3, 4, 5];
///     black_box(v);
/// });
/// println!("Allocated {} bytes", stats.bytes_allocated);
/// ```
#[allow(dead_code)]
pub fn track_allocations<F, R>(f: F) -> AllocStats
where
    F: FnOnce() -> R,
{
    let tracker = AllocTracker::new();
    let _ = f();
    tracker.stats()
}

/// Get the name of the current allocator
#[allow(dead_code)]
pub fn allocator_name() -> &'static str {
    #[cfg(feature = "bench-stats-alloc")]
    {
        "stats_alloc (glibc malloc with tracking)"
    }
    #[cfg(feature = "bench-mimalloc")]
    {
        "mimalloc (high performance, no tracking)"
    }
    #[cfg(feature = "bench-jemalloc")]
    {
        "jemalloc (high performance, optional stats)"
    }
    #[cfg(not(any(
        feature = "bench-stats-alloc",
        feature = "bench-mimalloc",
        feature = "bench-jemalloc"
    )))]
    {
        "system default (glibc malloc)"
    }
}

/// Check if allocation tracking is enabled
#[allow(dead_code)]
pub const fn has_tracking() -> bool {
    cfg!(feature = "bench-stats-alloc")
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::{allocator_name, has_tracking, track_allocations, AllocStats, AllocTracker};

    #[test]
    fn test_allocator_name() {
        let name = allocator_name();
        assert!(!name.is_empty());
    }

    #[test]
    fn test_alloc_tracker() {
        let tracker = AllocTracker::new();
        #[allow(clippy::useless_vec)]
        let _v = vec![1, 2, 3, 4, 5]; // Intentionally using vec! to force heap allocation
        let stats = tracker.stats();

        // With stats_alloc, should see allocations
        // With mimalloc, stats will be zero
        if has_tracking() {
            assert!(stats.bytes_allocated > 0);
        } else {
            assert_eq!(stats.bytes_allocated, 0);
        }
    }

    #[test]
    fn test_track_allocations() {
        let stats = track_allocations(|| {
            let _v = vec![1u8; 1000];
        });

        if has_tracking() {
            assert!(stats.bytes_allocated >= 1000);
        } else {
            assert_eq!(stats.bytes_allocated, 0);
        }
    }

    #[test]
    fn test_net_bytes() {
        let stats = AllocStats {
            bytes_allocated: 1000,
            bytes_deallocated: 300,
            ..Default::default()
        };
        assert_eq!(stats.net_bytes(), 700);
    }
}
