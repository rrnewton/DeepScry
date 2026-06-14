//! Self-relaunch of `mtg server-web` inside a memory-capped cgroup.
//!
//! ## Why
//!
//! A memory leak (or an unbounded effect loop — see the workspace
//! `CLAUDE.md` "Validates must be memory-capped" incident, where a
//! self-copying spell ballooned one `mtg` process to ~40 GB and took down
//! the whole box) should kill ONLY this server, not the host. The fix is a
//! kernel-enforced hard ceiling: run the server inside a cgroup with
//! `MemoryMax = N%` of total system RAM. If the server's cgroup blows the
//! cap it is OOM-killed at its own limit; the machine stays alive.
//!
//! A transient `systemd-run --user --scope` places the process AND all its
//! descendants (the embedded lobby task, any proxied children) in one
//! cgroup, so a Ctrl-C / SIGTERM to the scope brings the whole tree down
//! together — no orphaned `mtg` left behind. This mirrors how `make
//! validate` wraps its run in a `systemd-run --user --scope` outer cgroup
//! (see `scripts/validate.py` / `scripts/validate_cgroup.py`); we reuse the
//! same `--user`-scope mechanism here rather than inventing a second one.
//!
//! ## How
//!
//! The user's preferred design is SELF re-exec, not Makefile/wrapper
//! plumbing: `mtg server-web --mem-cap-pct N` re-launches itself via
//! `systemd-run --user --scope -p MemoryMax=<bytes> -- <argv...>`. A guard
//! environment variable ([`GUARD_ENV`]) is set on the child so the
//! re-exec'd copy does NOT recurse. On Unix we replace the current process
//! image (`exec`) so there is no lingering parent.
//!
//! ## Graceful degradation
//!
//! Everything here is best-effort. If `systemd-run --user` is missing, the
//! user session bus is unavailable, or the host has no cgroup v2, we log a
//! warning and return so the server runs UNCAPPED rather than failing to
//! start. A dev box without systemd-user delegation must still be able to
//! run `mtg server-web`.

use std::path::Path;

use anyhow::{Context, Result};

/// Env var that marks "we are already running inside the managed cgroup".
/// Set on the re-exec'd child so it does not wrap itself again (infinite
/// re-exec guard).
pub const GUARD_ENV: &str = "MTG_IN_MEM_CGROUP";

/// A validated memory-cap percentage in `1..=100`. `0` is rejected at
/// construction (a 0% cap would OOM-kill the server instantly); callers
/// treat `--mem-cap-pct 0` as "disabled" BEFORE building this type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemCapPct(u32);

impl MemCapPct {
    /// Validate a raw percentage. Returns `None` for `0` or `> 100`.
    pub fn new(pct: u32) -> Option<Self> {
        if (1..=100).contains(&pct) {
            Some(Self(pct))
        } else {
            None
        }
    }

    /// The percentage as `1..=100`.
    pub fn get(self) -> u32 {
        self.0
    }

    /// `MemoryMax` byte budget = this percentage of `total_ram_bytes`.
    /// Multiply-before-divide to avoid truncation on the percentage.
    pub fn bytes_of(self, total_ram_bytes: u64) -> u64 {
        total_ram_bytes.saturating_mul(u64::from(self.0)) / 100
    }
}

/// Total physical RAM in bytes, read from `/proc/meminfo` via the existing
/// [`crate::network::memory`] reader (DRY — one meminfo parser in the
/// codebase). `None` on non-Linux or an unparseable meminfo.
fn total_ram_bytes() -> Option<u64> {
    let mem = crate::network::memory::current_system_memory()?;
    // `total_mb` is MiB.
    Some((mem.total_mb as u64).saturating_mul(1024 * 1024))
}

/// Re-exec the current process inside a transient memory-capped cgroup if
/// `--mem-cap-pct` was requested and we are not already inside one.
///
/// On success this **does not return** — it replaces the process image
/// (Unix `exec`). It returns `Ok(())` only when the server should continue
/// UNCAPPED in the current process, for one of these reasons:
///   * `pct == 0` (cap explicitly disabled),
///   * we are already inside the managed cgroup ([`GUARD_ENV`] set),
///   * a non-fatal capability gap (no `systemd-run --user`, no cgroup v2,
///     unreadable meminfo) — logged as a warning.
///
/// # Errors
///
/// Returns `Err` only for a genuinely unexpected failure to BUILD the
/// re-exec (e.g. `std::env::current_exe()` failing to resolve this binary's
/// path), which the caller surfaces. Capability gaps (missing `systemd-run`,
/// no cgroup v2) are NOT errors — they degrade to an uncapped `Ok(())`.
pub fn reexec_under_mem_cgroup_if_requested(pct: u32) -> Result<()> {
    // Already wrapped? Never recurse.
    if std::env::var_os(GUARD_ENV).is_some() {
        log::info!("[web-server] running inside managed mem cgroup ({GUARD_ENV} set)");
        return Ok(());
    }

    let Some(cap) = MemCapPct::new(pct) else {
        if pct == 0 {
            log::info!("[web-server] --mem-cap-pct 0: memory cgroup disabled");
        } else {
            log::warn!("[web-server] --mem-cap-pct {pct} out of range 1..=100; running uncapped");
        }
        return Ok(());
    };

    let Some(total) = total_ram_bytes() else {
        log::warn!("[web-server] could not read total system RAM; running uncapped");
        return Ok(());
    };
    let max_bytes = cap.bytes_of(total);
    let total_mib = total / (1024 * 1024);

    if !systemd_run_user_available() {
        log::warn!(
            "[web-server] `systemd-run --user` unavailable (no user systemd / cgroup v2 \
             / XDG_RUNTIME_DIR); running UNCAPPED. Memory will NOT be capped at {}%.",
            cap.get()
        );
        return Ok(());
    }

    do_reexec(cap, max_bytes, total_mib)
}

/// Build and perform the `systemd-run --user --scope` re-exec. Unix-only
/// `exec` replacement; on non-Unix we currently just warn and return (the
/// deployment target is Linux).
fn do_reexec(cap: MemCapPct, max_bytes: u64, total_mib: u64) -> Result<()> {
    let exe = std::env::current_exe().context("resolving current executable for mem-cgroup re-exec")?;
    // argv[1..] of the ORIGINAL invocation — re-pass everything so the child
    // sees the same `server-web ...` flags (including `--mem-cap-pct N`,
    // which the GUARD_ENV var neutralises on the second pass).
    let forwarded: Vec<String> = std::env::args().skip(1).collect();

    log::info!(
        "[web-server] re-exec under systemd-run --user --scope: MemoryMax={max_bytes} bytes \
         ({}% of {total_mib} MiB total RAM)",
        cap.get(),
    );

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let mut cmd = std::process::Command::new("systemd-run");
        cmd.arg("--user")
            .arg("--scope")
            // Quiet systemd-run's own "Running as unit" chatter to stderr.
            .arg("--quiet")
            // Give the scope a stable, greppable name.
            .arg(format!("--unit=mtg-server-web-{}", std::process::id()))
            .arg("-p")
            .arg(format!("MemoryMax={max_bytes}"))
            // Keep the cap RAM-real: with MemorySwapMax=0 a leak is
            // OOM-killed at the cap instead of silently swap-thrashing the
            // host. (MemoryMax is already a hard limit.)
            .arg("-p")
            .arg("MemorySwapMax=0")
            .arg("--")
            .arg(&exe)
            .args(&forwarded)
            .env(GUARD_ENV, "1");
        // `exec` replaces this image; only returns on failure.
        let err = cmd.exec();
        // If we get here, exec failed — degrade to uncapped rather than dying.
        log::warn!(
            "[web-server] failed to exec systemd-run ({err}); running UNCAPPED. \
             Memory will NOT be capped at {}%.",
            cap.get()
        );
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (cap, max_bytes, total_mib, exe, forwarded);
        log::warn!("[web-server] --mem-cap-pct is only supported on Unix; running uncapped");
        Ok(())
    }
}

/// Is `systemd-run --user` usable in this environment? Cheap checks only:
/// the binary exists on `PATH`, the host has cgroup v2, and a user systemd
/// instance appears reachable (`XDG_RUNTIME_DIR` set, which user `--user`
/// units require for the session bus / runtime dir).
fn systemd_run_user_available() -> bool {
    if !binary_on_path("systemd-run") {
        return false;
    }
    // cgroup v2 unified hierarchy present?
    if !Path::new("/sys/fs/cgroup/cgroup.controllers").exists() {
        return false;
    }
    // `--user` scopes need the per-user runtime dir / session bus.
    if std::env::var_os("XDG_RUNTIME_DIR").is_none() {
        return false;
    }
    true
}

/// Does `name` resolve on `PATH`? Pure `PATH` walk — no subprocess.
fn binary_on_path(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(name).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mem_cap_pct_rejects_out_of_range() {
        assert!(MemCapPct::new(0).is_none());
        assert!(MemCapPct::new(101).is_none());
        assert_eq!(MemCapPct::new(1).map(MemCapPct::get), Some(1));
        assert_eq!(MemCapPct::new(70).map(MemCapPct::get), Some(70));
        assert_eq!(MemCapPct::new(100).map(MemCapPct::get), Some(100));
    }

    #[test]
    fn bytes_of_computes_percentage() {
        let cap = MemCapPct::new(70).unwrap();
        // 70% of 100 GiB.
        let total = 100u64 * 1024 * 1024 * 1024;
        assert_eq!(cap.bytes_of(total), 70 * 1024 * 1024 * 1024);
        // No overflow near u64 max-ish totals (multiply-before-divide stays
        // within u64 for realistic RAM sizes).
        let cap100 = MemCapPct::new(100).unwrap();
        assert_eq!(cap100.bytes_of(total), total);
    }

    #[test]
    fn guard_env_short_circuits() {
        // With the guard set, we must NOT attempt a re-exec — function
        // returns Ok and leaves the process in place. (We cannot assert the
        // negative-of-exec directly, but the guard branch returns before any
        // exec path.)
        temp_env_set(GUARD_ENV, "1");
        let r = reexec_under_mem_cgroup_if_requested(70);
        temp_env_unset(GUARD_ENV);
        assert!(r.is_ok());
    }

    #[test]
    fn zero_pct_disables() {
        // pct 0 returns Ok without touching systemd.
        temp_env_unset(GUARD_ENV);
        assert!(reexec_under_mem_cgroup_if_requested(0).is_ok());
    }

    // Minimal env helpers (std::env::set_var is unsafe-free on this toolchain
    // for test-thread-local intent; tests here are not run in parallel with
    // other GUARD_ENV readers).
    fn temp_env_set(k: &str, v: &str) {
        std::env::set_var(k, v);
    }
    fn temp_env_unset(k: &str) {
        std::env::remove_var(k);
    }
}
