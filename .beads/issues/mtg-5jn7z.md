---
title: 'validate: default memory-capped two-level cgroup + teardown hardening + ephemeral ports'
status: open
priority: 2
issue_type: task
created_at: 2026-06-09T19:45:13.019238463+00:00
updated_at: 2026-06-09T19:45:13.019238463+00:00
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
