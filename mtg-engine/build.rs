//! Build script — captures git short SHA + build timestamp into env vars
//! that the binary reads via `option_env!`.
//!
//! Why: the deploy story needs a stable, queryable "what's running"
//! identifier for the `/health` endpoint and for cache-busting `?v=`
//! query strings on `/pkg/*` URLs. Doing this at build time is cheap
//! (no `git` exec at runtime), avoids leaking `.git/` into the deployed
//! binary, and naturally drifts with each rebuild.
//!
//! Both vars are best-effort: if `git` is missing or this is a tarball
//! build, we fall back to "unknown" and "" respectively. The binary
//! still works; `/health` just reports "unknown".

use std::process::Command;

fn main() {
    // Capture short SHA. `--always` falls back to a SHA1 abbrev even when
    // no tags exist; `--dirty=+` appends `+` if the working tree has
    // uncommitted changes (deploy-time visibility into "is this a clean
    // build?").
    let sha = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);

    let sha_full = if dirty { format!("{sha}+dirty") } else { sha };

    // Git depth = total commit count (matches scripts/gitdepth.sh). This is
    // the patch component of the displayed `Major.Minor.<gitdepth>` version.
    // Falls back to empty when git is unavailable (tarball / shallow clone
    // with no history); the binary then displays the bare Cargo version.
    let git_depth = Command::new("git")
        .args(["rev-list", "--count", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_default();

    // Build timestamp as Unix epoch seconds (machine-readable, kept for the
    // existing /health `build_time_epoch` field and ?v= cache-busting).
    let epoch_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let build_time = epoch_secs.to_string();

    // Human-readable UTC build date (YYYY-MM-DD) for the --version / footer
    // display. Format manually from epoch to avoid a `chrono` build-dep.
    let build_date = format_utc_date(epoch_secs);

    // Assemble the full display version `Major.Minor.<gitdepth>`. CARGO_PKG_VERSION
    // is the `Major.Minor.0` base from Cargo.toml; we replace its patch component
    // with the live git depth so the displayed patch never rots. When git depth is
    // unavailable (tarball / shallow clone), fall back to the bare Cargo version.
    let cargo_version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string());
    let full_version = if git_depth.is_empty() {
        cargo_version
    } else {
        // major.minor = first two dot-separated components of the Cargo version.
        let mut parts = cargo_version.splitn(3, '.');
        let major = parts.next().unwrap_or("0");
        let minor = parts.next().unwrap_or("0");
        format!("{major}.{minor}.{git_depth}")
    };

    println!("cargo:rustc-env=MTG_BUILD_SHA={sha_full}");
    println!("cargo:rustc-env=MTG_BUILD_TIME_EPOCH={build_time}");
    println!("cargo:rustc-env=MTG_GIT_DEPTH={git_depth}");
    println!("cargo:rustc-env=MTG_BUILD_DATE={build_date}");
    println!("cargo:rustc-env=MTG_VERSION={full_version}");

    // Re-run if HEAD moves. We deliberately do NOT depend on every
    // tracked file (would blow up incremental rebuilds); operators who
    // want the SHA stamp updated mid-edit can `touch .git/HEAD`.
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/HEAD");
}

/// Convert Unix epoch seconds to a `YYYY-MM-DD` UTC date string using the
/// civil-from-days algorithm (Howard Hinnant's `civil_from_days`). Pure
/// arithmetic — no external crate needed for one date string at build time.
fn format_utc_date(epoch_secs: u64) -> String {
    let days = (epoch_secs / 86_400) as i64;
    // Shift epoch (1970-01-01) to an era anchored at 0000-03-01.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11], Mar=0
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    format!("{year:04}-{m:02}-{d:02}")
}
