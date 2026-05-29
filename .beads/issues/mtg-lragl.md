---
title: 'Version info: Major.Minor.<gitdepth> + build date + git hash'
status: closed
priority: 3
issue_type: task
created_at: 2026-05-29T15:19:30.888257196+00:00
updated_at: 2026-05-29T15:19:34.366165486+00:00
closed_at: 2026-05-29T15:19:34.366165304+00:00
---

# Description

Adopt a Major.Minor.<gitdepth> display version (gitdepth = git rev-list --count HEAD, per scripts/gitdepth.sh) captured at COMPILE time, exposed via mtg --version, /health, and the web lobby footer.

Implementation (DRY — single source of truth):
- mtg-engine/build.rs: now also emits MTG_GIT_DEPTH, MTG_BUILD_DATE (UTC YYYY-MM-DD, computed via Hinnant civil_from_days so no chrono build-dep), and MTG_VERSION (Major.Minor base from CARGO_PKG_VERSION with the patch replaced by gitdepth). Graceful fallback to bare Cargo version when git is unavailable (tarball/shallow). Kept existing MTG_BUILD_SHA (+dirty) and MTG_BUILD_TIME_EPOCH.
- New mtg-engine/src/version.rs: central module exposing CARGO_VERSION, GIT_HASH, GIT_DEPTH, BUILD_DATE, BUILD_TIME_EPOCH, FULL_VERSION, and VERSION_LINE ('<full> (<sha>, built <date>)'). All &'static str (const, usable in clap).
- mtg-engine/src/web_server/mod.rs: now re-exports BUILD_SHA/BUILD_TIME_EPOCH from crate::version (deduped the prior local option_env! consts). /health JSON now reports the full Major.Minor.<gitdepth> 'version' (was bare CARGO_PKG_VERSION) plus a new 'build_date' field.
- mtg-engine/src/main.rs: clap #[command(version = mtg_engine::version::VERSION_LINE)] so 'mtg --version' prints e.g. 'mtg 0.1.2441 (908f8fa, built 2026-05-29)'.
- web/index.html: lobby footer #footer-version span populated from same-origin /health (best-effort fetch; left blank when /health 404s for standalone hosting).

Verified: mtg --version -> 'mtg 0.1.2441 (908f8faa+dirty, built 2026-05-29)' (dirty marker from pre-commit tree; clean after commit). gitdepth/date/hash all match git.
