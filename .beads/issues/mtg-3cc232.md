---
title: Deterministic builds + sccache cross-worktree caching (closed experiment)
status: open
priority: 3
issue_type: task
labels:
- optimization
- build
created_at: 2026-05-15T17:03:18.906943720+00:00
updated_at: 2026-05-15T17:03:18.906943720+00:00
---

# Description

## Summary

Investigation into deterministic builds (trim-paths) + sccache for
cross-worktree caching to save disk space and CPU time on rebuilds.

## Status

CLOSED experiment (May 2026). Findings preserved here from `tg` notes
before migration off the local task graph.

- Phase 1 (trim-paths): NECESSARY but NOT SUFFICIENT. Rust artifacts
  become 99-100% byte-identical across worktrees, but C artifacts
  (cc-rs, configure+make, ~478 MB of build output) remain non-deterministic
  because cc-rs ignores `CARGO_TRIM_PATHS`.
- Phase 2 (sccache): ~1.4% hit rate cross-worktree because trim-paths
  injects a per-worktree `--remap-path-prefix=/.../mtg-forge-rsN/target=/cargo/build-dir`
  into every rustc invocation, and sccache hashes that argument verbatim.

## Recommendation

DISABLED until either:
- (a) trim-paths is replaced with a path-canonicalizing alternative, OR
- (b) sccache learns to strip `SCCACHE_BASEDIR` from `--remap-path-prefix`
  args, OR
- (c) cc-rs gains support that consumes `CARGO_TRIM_PATHS` automatically
  (would unblock the C-artifact 0% hit rate).

For C artifacts specifically, three workarounds were considered:
- Drop debug info from C deps in release (`DEBUG=false` for sys crates)
- Inject `CFLAGS=-ffile-prefix-map=$PWD=.` via `.cargo/config.toml [env]`
- Wait for cc-rs `CARGO_TRIM_PATHS` support upstream

## Experimental config (preserved verbatim)

The experiment lived in `mtg-forge-rs2` worktree (now removed). The
files that were added:

`.cargo/config.toml`:
```toml
## Phase 1 build determinism experiment.
## trim-paths strips absolute paths (workspace path, registry path, toolchain sysroot)
## from compiled artifacts so two worktrees with different absolute paths produce
## bit-identical dependency outputs. This is a prerequisite for sccache hits
## across worktrees and for reproducible CI artifacts.
#
## Stable in Rust 1.81+; this repo pins nightly-2025-11-28 (rustc 1.93-nightly).

[profile.dev]
trim-paths = "all"

[profile.release]
trim-paths = "all"
```

`Cargo.toml` prelude (BEFORE `[workspace]`):
```toml
cargo-features = ["trim-paths"]
```

(`cargo-features` line required because `trim-paths` is unstable in
cargo 1.93-nightly. Bare config alone errored: `'feature trim-paths is
required ... not stabilized in this version of Cargo'`.)

## Phase 1 measurements

Setup: two worktrees on identical commits, sequential cold builds with
`cargo build --release --features network`. Toolchain pinned
nightly-2025-11-28 (rustc 1.93.0-nightly).

| Metric                          | ws1            | ws2            |
|---------------------------------|----------------|----------------|
| Wall                            | 1m30.4s        | 1m28.9s        |
| CPU                             | 929.9s (1104%) | 915.9s (1151%) |
| MaxRSS                          | 2.99 GiB       | 3.02 GiB       |
| Total target/                   | 3.0 GB         | 3.0 GB         |
| target/release/build (C)        | 478 MB         | 478 MB         |
| target/release/deps (Rust)      | 2.5 GB         | 2.5 GB         |
| jemalloc build dir              | 299 MB         | 299 MB         |

Cross-worktree determinism (after trim-paths):

| Artifact class        | Identical | %        |
|-----------------------|-----------|----------|
| .rmeta files          | 287/287   | 100%     |
| .rlib files           | 285/288   | 99.0%    |
| C .o files            | 0/217     | 0%       |
| Final binaries        | 0/3       | 0% (exp) |

Three differing rlibs: `libmtg_forge_rs.rlib` (different commits =
expected), `liblibmimalloc_sys-*.rlib` and `libtikv_jemalloc_sys-*.rlib`
(rlib bundles non-deterministic C .o files).

C non-determinism cause: verified by `strings hpa.pic.o`:
- ws1 contains `/home/newton/working_copies/mtg/mtg-forge-rs/target/release/build/...`
- ws2 contains `/home/newton/working_copies/mtg/mtg-forge-rs2/target/release/build/...`

`target/release/build/libmimalloc-sys-*/output` shows NO
`-ffile-prefix-map` / `-fdebug-prefix-map` / `-fmacro-prefix-map`
flags. cargo did not set `CARGO_TRIM_PATHS` in the build-script env
(or cc 1.2.49 ignores it). `DEBUG=Some(true)` is propagated, so cc-rs
adds `-g` and embeds DWARF paths. Compounded by
`[profile.release] debug = true` in workspace `Cargo.toml`.

## Phase 2 measurements

sccache 0.10 with `RUSTC_WRAPPER=sccache` on identical commit, identical
toolchain, two worktrees. Cross-worktree rustc hit rate: ~1.4%. Root
cause: trim-paths' `--remap-path-prefix=$PWD/target=/cargo/build-dir`
embeds the absolute worktree path into the rustc cmdline, which sccache
hashes verbatim.

## Residual path leakage in final mtg binary (Phase 1)

1. Source-level: 1 occurrence of `env!("CARGO_MANIFEST_DIR")` at
   `mtg-engine/src/network/server.rs:2217` (fn `bug_report_repo_root`).
   trim-paths intentionally does NOT rewrite `env!()` — explicit source
   choice.
2. C registry source paths (e.g.
   `/home/newton/.cargo/registry/src/.../ring-0.17.14/crypto/...`) linger
   in the final binary because they originate from C debuginfo (point
   above).

## Related (closed) tg tasks

- `goal-deterministic-builds` (parent goal)
- `phase1-deterministic-builds` (Phase 1 work)
- `phase2-sccache` (Phase 2 work)
