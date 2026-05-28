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

    // RFC 3339 UTC build timestamp. We avoid pulling in `chrono` for one
    // string by formatting epoch seconds manually — operators just need
    // *some* monotonically-increasing build marker.
    let build_time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string());

    println!("cargo:rustc-env=MTG_BUILD_SHA={sha_full}");
    println!("cargo:rustc-env=MTG_BUILD_TIME_EPOCH={build_time}");

    // Re-run if HEAD moves. We deliberately do NOT depend on every
    // tracked file (would blow up incremental rebuilds); operators who
    // want the SHA stamp updated mid-edit can `touch .git/HEAD`.
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/HEAD");
}
