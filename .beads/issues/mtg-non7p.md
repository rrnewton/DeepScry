---
title: 'perf(bench): measure concurrent-games capacity of single-VM deployment'
status: open
priority: 3
issue_type: feature
labels:
- perf
- bench
depends_on:
  mtg-pm0lz: related
  mtg-uwv3w: blocks
created_at: 2026-05-27T20:41:33.730234773+00:00
updated_at: 2026-05-27T20:41:33.730234773+00:00
---

# Description

## Context

Once the web-server unification lands (**mtg-uwv3w**, **mtg-dbypv**, **mtg-pm0lz**), we need actual capacity numbers for the single-VM DeepScry deployment before we know how aggressively to advertise public play, whether to add a queue/backpressure layer, or whether to scale horizontally.

This is a pure benchmarking issue — no production-code changes required (modulo any instrumentation hooks we discover we need).

## Benchmark design

Spawn N parallel WebSocket clients in pairs; each pair plays a full deterministic random-AI vs random-AI game over WS using the existing `ListGames` / `CreateGame` / `JoinGame` flow from `mtg-engine/src/network/protocol.rs`.

Reuse harness pieces:
- `tests/network_game_e2e.sh` already drives one WS game end-to-end against `mtg server`. Generalize into a Python or Rust driver that can launch N games concurrently against `mtg server-web` on `wss://deepscry.net/lobby`.
- `tests/network_vs_local_equivalence.py` shows the Python WebSocket client pattern; reusable as the basis for the load driver.

## Metrics to collect

Per run (parameterized by N = concurrent games):
- Peak concurrent games successfully completed (sweep N=1, 10, 50, 100, 200, 500 until failure).
- Wall-clock to complete the full batch.
- Peak CPU% (sampled via `pidstat -p $(pidof mtg) 1`).
- Peak RSS in MB (sampled same way).
- p50 / p99 message round-trip latency, measured client-side as time between sending a choice and receiving the next prompt.
- Error count (disconnects, protocol errors, timeouts).
- File-descriptor count (`ls /proc/$(pidof mtg)/fd | wc -l`) — verify `LimitNOFILE=65536` from the systemd unit is sufficient.

## Output format

`experiment_results/<CPU_MODEL>/concurrent_games_capacity.csv`:

```csv
stamp,n_games,wall_seconds,peak_cpu_pct,peak_rss_mb,p50_latency_ms,p99_latency_ms,peak_fds,errors
2026-MM-DD_#<DEPTH>(<sha>),1,4.2,18,210,5,12,42,0
2026-MM-DD_#<DEPTH>(<sha>),10,8.1,95,480,7,28,180,0
...
```

The `stamp` column uses the project's transient-info convention: `YYYY-MM-DD_#<DEPTH>(<short-sha>)` where `DEPTH = git rev-list --count HEAD` (or `./scripts/gitdepth.sh`).

## Makefile target

Add to top-level `Makefile`:

```make
bench-concurrent-games:
    cargo build --release --features network
    python3 scripts/bench_concurrent_games.py --n $(N) --server $(SERVER) \
        --out experiment_results/$$(uname -m)/concurrent_games_capacity.csv
```

Usage: `make bench-concurrent-games N=100 SERVER=wss://deepscry.net/lobby`.

## Where to run

- First pass: local dev machine with `mtg server-web` on `ws://localhost:8080/lobby` to verify the harness.
- Real numbers: deepscry.net VM itself, or a beefier dev box (e.g. the user's main workstation, then compare). Note exact CPU model in the output path.

## Write-up

After data is collected, produce `ai_docs/concurrent_games_capacity_YYYYMMDD.md` (per parent CLAUDE.md naming):
- Table of the CSV.
- Plot (committed as PNG) of N_games vs latency-p99 and N_games vs peak_cpu.
- Recommendation: max safe N for the current VM; threshold at which to switch to multi-process / horizontal scale.

## Acceptance criteria

- `make bench-concurrent-games N=100` completes against a local server and writes a valid CSV row.
- CSV committed under `experiment_results/<CPU>/` with at minimum runs at N ∈ {1, 10, 50, 100} and one run that breaks the server (errors > 0).
- Markdown report in `ai_docs/` cites the CSV file and the commit SHA stamp.

## Dependencies

- Depends on **mtg-uwv3w** (unified server is what we're benchmarking — testing the dual-process setup would be wasted effort).
- Soft-depends on **mtg-pm0lz** (systemd's `LimitNOFILE`/`MemoryMax` may be the binding constraint at high N; useful but not required to start the benchmark).
