---
title: 'validate: default memory-capped two-level cgroup + teardown hardening + ephemeral ports'
status: open
priority: 2
issue_type: task
created_at: 2026-06-09T19:45:13.019238463+00:00
updated_at: 2026-06-09T20:27:19.544389861+00:00
---

# Description

Make `make validate` SYSTEMICALLY SAFE against OOM (user #1 ask after the box-wedging OOM incident) + harden teardown + stop cross-slot port collisions. On claude/validate-streamline-2.

== MEMORY CAPS (DEFAULT, not opt-in) ==
1. Every full `make validate` re-execs into an outer `systemd-run --user --scope` with MemoryMax set BY DEFAULT (a sane cap derived from total RAM). A runaway (e.g. the Return-the-Favor infinite-copy loop) gets cgroup-OOM-killed at the cap instead of wedging the box. --max-mem overrides the default cap explicitly.
2. Per-step INNER cgroup (the two-level model): each step runs in its own child cgroup under the scope with its OWN MemoryMax from that step's characterized baseline (1.25x typical peak RSS; relax to 1.5x only if too tight).
3. Baselines characterized AFTER slot01's commander runaway fix lands (a baseline measured during the leak is garbage) — determ.commander excluded/remeasured post-fix.
4. Actionable OOM message: when a step/scope hits its cap, the error states (a) which step/scope, (b) WHERE the baseline is defined (file+symbol), (c) how to SAFELY raise it (confirm genuine growth, not an unbounded leak, first).

== TEARDOWN (landed earlier on this branch) ==
Two-level cgroup teardown proven across SIGINT/SIGTERM/SIGKILL (setsid-escapee-proof via cgroup.kill; killpg fallback); recursive proc-scan recovery; whole-run peak-RSS from scope memory.peak. Cross-slot safe (cwd-keyed).

== EPHEMERAL PORTS ==
web/test_human_input.js + test_font_size_layout.js + test_game_gui.js moved off hardcoded port 8767 to getRandomPorts() (like test_action_affordance.js) — stops cross-slot browser-suite ECONNREFUSED/EADDRINUSE collisions under concurrent validates.

SAFETY: NEVER run an uncapped validate while the commander loop is live in integration (until slot01 lands). Land caps once slot01 greens integration.

(Re-filed with a fresh hash ID after the mtg-882 numeric-ID collided with slot03's heuristic_controller refactor issue.)

== IMPLEMENTED 2026-06-09_#3073(badd088e) ==
DONE: (1) DEFAULT outer-scope memory cap — every full `make validate` now caps MemoryMax=1.25x VALIDATE_TOTAL_RSS_BASELINE_BYTES (=30G on this 70G/16-core box; clamped to 0.85*RAM, floor 8G) + MemorySwapMax=0 so a runaway is OOM-KILLED at the cap, not swapped. --no-max-mem opts out (DANGEROUS). (2) Per-step INNER cgroup memory.max=1.25x PER_STEP_RSS_BASELINE[tag] + memory.swap.max=0; determ.commander EXCLUDED (slot01 runaway-fix pending). (3) Baselines in ONE place: scripts/validate.py PER_STEP_RSS_BASELINE + VALIDATE_TOTAL_RSS_BASELINE_BYTES + MEM_CAP_FACTOR. Per-step peak RSS now recorded to each step's detail for re-tuning. (4) Actionable OOM message: states which step, the baseline location (file+symbol), and the confirm-genuine-growth-not-a-leak-before-bumping procedure.

EMPIRICALLY VERIFIED: a 2 GiB memory-hog step under a 200 MiB inner cap is OOM-killed (exit -9, oom_kills=1, peak capped at exactly 200 MiB) while the outer scope + supervisor stay ALIVE — the two-level model kills only the runaway step. Default-cap resolution + per-step-cap table unit-tested. Ephemeral-ports fix landed (badd088e): 2 concurrent test_human_input.js → both exit 0, ZERO port collisions.

REMAINING: characterize REAL per-step baselines after slot01's commander fix greens integration (current PER_STEP_RSS_BASELINE values are conservative estimates from the 2026-06-09 -j16 run, commander excluded). NEVER run uncapped while commander loop is live.

== COMPLETE + GREEN 2026-06-09_#3077(56d7c988) ==
All done. Full `make validate` GREEN on the final tree: 33 passed / 0 failed / 0 skipped, wall-clock 579s, whole-run scope peak 16.2G (< 30G default outer cap), ZERO OOM-kills. Artifact: validate_logs/validate_56d7c988d705a777b12138fc44de510741df678b.log. Ff-mergeable onto integration @2c9d1808.

KEY FIX found this round: per-step inner caps were a SILENT NO-OP in real runs — the disowned utilization sampler sat in the scope ROOT, so enabling +memory on subtree_control hit the cgroup-v2 no-internal-processes rule (EBUSY, swallowed). Fixed by draining the WHOLE root into supervisor/ (d6dc7897). Verified: subtree_control now 'cpu memory pids', per-step memory.max/swap.max actually applied.

REAL baselines characterized (commander loop now fixed @2c9d1808): determ.commander 31.7M (was the ~40GB runaway — NOW INCLUDED with a 640M cap), nextest 6.8-9.0G (cap 10G), build 3.8-4.1G (cap 6.25G), examples 4.5G, clippy 2.7G, multideck 3.2G, wasm.browser 0.69G. Every cap verified >= measured peak (1.39x-20x margin). PER_STEP_RSS_BASELINE retuned from these measurements (56d7c988).

DELIVERED: (1) DEFAULT outer cap 30G + swap=0 (runaway OOM-killed, never wedges box). (2) Per-step inner caps from real baselines + swap=0 (one runaway step killed, run+host survive — empirically proven with a 2GB hog under 200MB cap). (3) Baselines in ONE place (scripts/validate.py). (4) Actionable OOM message (which step, baseline file+symbol, confirm-not-a-leak-before-bump). (5) mtg-882 collision fixed. (6) ephemeral ports (2 concurrent test_human_input.js → 0 collisions).
