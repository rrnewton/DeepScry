//! System memory inspection for the lobby's capacity gating.
//!
//! The server uses *system* memory pressure — not its own RSS, and not a fixed
//! game count — to decide whether to accept a new lobby join. The reasoning:
//!
//! - A fixed `max_games` requires operators to re-tune per VM size and ignores
//!   external pressure (other tenants on the host, web/asset servers, etc.).
//! - Process RSS only reflects this server's appetite; if another process
//!   eats the host's memory we'd happily keep accepting games and trip the
//!   OOM killer anyway.
//! - System "% available" is what the operator actually cares about. Linux's
//!   `MemAvailable` already discounts page cache and reclaimable slab so it
//!   genuinely represents what an allocator could grab right now.
//!
//! The check is: `used_percent = (MemTotal - MemAvailable) / MemTotal * 100`,
//! deny if `used_percent > max_memory_percent`.
//!
//! On non-Linux platforms the read returns `None` and admission is allowed —
//! callers should treat "no information" as "do not block local development".
//!
//! See also `fix-server-thread-count` for the related thread-pool sizing
//! discussion.

use std::path::Path;

/// Snapshot of system memory used for admission control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemMemory {
    /// Total physical memory in MiB.
    pub total_mb: usize,
    /// Memory currently available to allocations in MiB. On Linux this is
    /// `MemAvailable` from `/proc/meminfo`, which already excludes the
    /// reclaimable page cache, so it is a faithful "could be allocated"
    /// number rather than `MemFree`.
    pub available_mb: usize,
}

impl SystemMemory {
    /// Used percentage in `0..=100`, computed from `(total - available) / total`.
    ///
    /// Returns 0 if `total_mb` is 0 to avoid a divide-by-zero on degenerate
    /// inputs (e.g., an empty test fixture).
    pub fn used_percent(self) -> u32 {
        if self.total_mb == 0 {
            return 0;
        }
        let used = self.total_mb.saturating_sub(self.available_mb);
        // Multiply before divide to keep precision; cap at 100 to defend
        // against `available > total` (which can theoretically appear if
        // /proc/meminfo races with our parser, though we have not observed it).
        let pct = used.saturating_mul(100) / self.total_mb;
        (pct.min(100)) as u32
    }
}

/// Read the host's current memory pressure.
///
/// Returns `None` on platforms with no cheap dependency-free reader (i.e.,
/// anything not Linux). Callers must treat `None` as "no information,
/// allow the request".
pub fn current_system_memory() -> Option<SystemMemory> {
    #[cfg(target_os = "linux")]
    {
        read_system_memory_from(Path::new("/proc/meminfo"))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = Path::new("");
        None
    }
}

/// Parse a `/proc/meminfo` file. Crate-internal so unit tests can feed in
/// synthetic fixtures without depending on real host state.
pub(crate) fn read_system_memory_from(path: &Path) -> Option<SystemMemory> {
    let contents = std::fs::read_to_string(path).ok()?;
    parse_meminfo(&contents)
}

fn parse_meminfo(contents: &str) -> Option<SystemMemory> {
    let mut total_kb: Option<usize> = None;
    let mut available_kb: Option<usize> = None;
    for line in contents.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            total_kb = parse_kb(rest);
        } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
            available_kb = parse_kb(rest);
        }
        if total_kb.is_some() && available_kb.is_some() {
            break;
        }
    }
    Some(SystemMemory {
        total_mb: total_kb? / 1024,
        available_mb: available_kb? / 1024,
    })
}

fn parse_kb(rest: &str) -> Option<usize> {
    // Expected: "  16327688 kB"
    let mut iter = rest.trim().split_whitespace();
    let value: usize = iter.next()?.parse().ok()?;
    // Defensively check the unit is kB; older kernels always emit kB so this
    // should never differ in practice.
    match iter.next() {
        Some(unit) if unit.eq_ignore_ascii_case("kb") => Some(value),
        // If unit missing, assume kB (matches kernel format).
        None => Some(value),
        _ => None,
    }
}

/// Outcome of an admission check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionVerdict {
    /// The join is allowed.
    Admit,
    /// The join is denied because system memory is too tight.
    /// Carries the snapshot so callers can include it in the rejection message.
    Reject {
        /// Snapshot taken at decision time.
        memory: SystemMemory,
        /// Configured ceiling that was exceeded.
        ceiling_percent: u32,
    },
}

/// Decide whether a new game/connection should be admitted.
///
/// `ceiling_percent == 0` is treated as "no limit" to match the existing
/// `max_games == 0` convention. When `current_system_memory()` returns
/// `None` (non-Linux), the join is allowed — see module docs.
pub fn check_memory_admission(ceiling_percent: u32) -> AdmissionVerdict {
    if ceiling_percent == 0 {
        return AdmissionVerdict::Admit;
    }
    let Some(memory) = current_system_memory() else {
        return AdmissionVerdict::Admit;
    };
    if memory.used_percent() > ceiling_percent {
        AdmissionVerdict::Reject {
            memory,
            ceiling_percent,
        }
    } else {
        AdmissionVerdict::Admit
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn fixture(total_kb: usize, avail_kb: usize) -> String {
        format!(
            "MemTotal:       {total_kb} kB\nMemFree:        {avail_kb} kB\nMemAvailable:   {avail_kb} kB\nBuffers:           1024 kB\n",
        )
    }

    #[test]
    fn parses_meminfo_total_and_available() {
        let f = fixture(16_000_000, 4_000_000);
        let m = parse_meminfo(&f).expect("parse");
        assert_eq!(m.total_mb, 16_000_000 / 1024);
        assert_eq!(m.available_mb, 4_000_000 / 1024);
    }

    #[test]
    fn used_percent_basic_arithmetic() {
        let m = SystemMemory {
            total_mb: 100,
            available_mb: 25,
        };
        assert_eq!(m.used_percent(), 75);
    }

    #[test]
    fn used_percent_clamps_at_100() {
        // Defensive: available > total (race) should not produce >100.
        let m = SystemMemory {
            total_mb: 100,
            available_mb: 150,
        };
        assert_eq!(m.used_percent(), 0); // saturating_sub gives 0 used
    }

    #[test]
    fn used_percent_handles_zero_total() {
        let m = SystemMemory {
            total_mb: 0,
            available_mb: 0,
        };
        assert_eq!(m.used_percent(), 0);
    }

    #[test]
    fn missing_field_returns_none() {
        let no_avail = "MemTotal: 16000000 kB\nMemFree: 4000000 kB\n";
        assert!(parse_meminfo(no_avail).is_none());
        let no_total = "MemAvailable: 4000000 kB\n";
        assert!(parse_meminfo(no_total).is_none());
    }

    #[test]
    fn admission_zero_ceiling_always_admits() {
        assert_eq!(check_memory_admission(0), AdmissionVerdict::Admit);
    }

    #[test]
    fn read_system_memory_from_tempfile_round_trip() {
        let tmp = std::env::temp_dir().join("mtg_test_meminfo_round_trip");
        let mut f = std::fs::File::create(&tmp).expect("create tmp");
        write!(f, "{}", fixture(8_000_000, 2_000_000)).expect("write");
        let result = read_system_memory_from(&tmp);
        let _ = std::fs::remove_file(&tmp);
        let m = result.expect("read");
        assert_eq!(m.total_mb, 8_000_000 / 1024);
        assert_eq!(m.available_mb, 2_000_000 / 1024);
        assert_eq!(m.used_percent(), 75);
    }

    #[test]
    fn linux_can_read_real_meminfo() {
        // Smoke test on real /proc/meminfo. On non-Linux, current_system_memory()
        // returns None and we just verify that.
        match current_system_memory() {
            Some(m) => {
                assert!(m.total_mb > 0, "real host should report > 0 MiB total");
                assert!(m.available_mb <= m.total_mb, "available <= total");
                let pct = m.used_percent();
                assert!(pct <= 100, "used_percent must be 0..=100");
            }
            None => {
                #[cfg(target_os = "linux")]
                panic!("Linux should always expose /proc/meminfo");
            }
        }
    }
}
