//! Build/version identity — the single source of truth for "what build is
//! this?" across the CLI (`mtg --version`), the `/health` endpoint, and the
//! web lobby footer.
//!
//! All values are captured at COMPILE time by `build.rs` (no runtime `git`
//! exec, no `.git/` leaking into the deployed binary). Every value is
//! best-effort: when `git` is unavailable at build time (tarball release,
//! shallow clone with no history) we gracefully fall back to the Cargo
//! package version and `"unknown"` markers.
//!
//! Versioning scheme: `Major.Minor.<gitdepth>` where `<gitdepth>` is the
//! total commit count (`git rev-list --count HEAD`, matching
//! `scripts/gitdepth.sh`). Cargo's own `version` field stays at the
//! `Major.Minor` base (`0.1.0`); the full patch-versioned string is
//! assembled here for display so the displayed patch never rots in
//! `Cargo.toml`.

/// `Major.Minor` base from `Cargo.toml` (e.g. `"0.1.0"`).
pub const CARGO_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Git short SHA captured at build time (e.g. `"e6027fc"`, or
/// `"e6027fc+dirty"` for an uncommitted tree). `"unknown"` if `git` was
/// unavailable.
pub const GIT_HASH: &str = match option_env!("MTG_BUILD_SHA") {
    Some(s) if !s.is_empty() => s,
    _ => "unknown",
};

/// Git commit depth (`git rev-list --count HEAD`) as a string. Empty when
/// unavailable at build time.
pub const GIT_DEPTH: &str = match option_env!("MTG_GIT_DEPTH") {
    Some(s) => s,
    None => "",
};

/// Human-readable UTC build date (`YYYY-MM-DD`). `"unknown"` if unavailable.
pub const BUILD_DATE: &str = match option_env!("MTG_BUILD_DATE") {
    Some(s) if !s.is_empty() => s,
    _ => "unknown",
};

/// Build timestamp as Unix epoch seconds (string). `"0"` if unavailable.
/// Kept machine-readable for `/health` and `?v=` cache-busting.
pub const BUILD_TIME_EPOCH: &str = match option_env!("MTG_BUILD_TIME_EPOCH") {
    Some(s) if !s.is_empty() => s,
    _ => "0",
};

/// Full display version `Major.Minor.<gitdepth>`, assembled at build time by
/// `build.rs` from the Cargo `Major.Minor` base and the captured git depth.
/// Falls back to the bare Cargo version when git depth is unavailable.
///
/// A `&'static str`, usable directly in clap's `#[command(version = ...)]`.
pub const FULL_VERSION: &str = match option_env!("MTG_VERSION") {
    Some(s) if !s.is_empty() => s,
    _ => CARGO_VERSION,
};

/// One-line human-readable build identity:
/// `"<full_version> (<git_hash>, built <build_date>)"`.
///
/// Used by `mtg --version` and surfaced (as structured fields) on `/health`
/// and the web lobby footer. `&'static str` so it works as a clap version.
pub const VERSION_LINE: &str = match option_env!("MTG_VERSION") {
    Some(_) => concat!(
        env!("MTG_VERSION"),
        " (",
        env!("MTG_BUILD_SHA"),
        ", built ",
        env!("MTG_BUILD_DATE"),
        ")"
    ),
    // No build.rs env (should not happen in practice) — bare Cargo version.
    None => CARGO_VERSION,
};
