---
title: 'Deploy script: pre-deploy smoke should run a NETWORKED random/random game + final SUCCESS/FAILURE must be unmissable'
status: open
priority: 3
issue_type: task
depends_on:
  mtg-q97bw: related
created_at: 2026-06-06T00:34:56.607412752+00:00
updated_at: 2026-06-06T00:34:56.607412752+00:00
---

# Description

Two deploy-script (scripts/deploy-cloud.sh) UX gaps (user, 2026-06-06, after deploying 1070b585).

(1) DOES the pre-deploy smoke run a random/random game? YES, partially: the 'PRE-DEPLOY GATE: headless WASM-boot smoke' (deploy-cloud.sh ~522) runs 'python3 scripts/mtg_wasm_game.py --p1 random --p2 random --seed 42 --max-turns 3' and ABORTS the deploy if it cannot deserialize bins / launch. BUT it is (a) single-client WASM, (b) only 3 turns, (c) seed 42 fixed, (d) silently SKIPPED if python3+playwright is unavailable. So it is a boot smoke, not a gameplay/desync gate. ENHANCEMENT: optionally run a NETWORKED random/random game (native+WASM, more turns, maybe a couple random seeds) as a stronger pre-deploy gate that would catch desync/gameplay regressions before they hit the live site (ties into the fuzz prize mtg-q97bw and the all-debug-checks corpus). Keep it opt-outable + chromium-gated like the existing legs.

(2) FINAL MESSAGE BURIED: the deploy DOES print 'sent N bytes ... -> restarting service ... systemctl status ... -> probing live deploy: ... ✓ / 200 ... ✓ all probes passed ... ═══ ✓ deploy complete ═══', but the success banner is buried after a wall of systemctl-status + per-asset probe output and is easy to miss (user could not tell if the deploy succeeded; the WS-upgrade probe also logs a benign 'Handshake not finished' WARN that looks alarming). FIX: emit an UNMISSABLE final one-liner at the very end — e.g. a bright '✓ DEPLOY SUCCEEDED — sha=<sha> live at <url>' or '✗ DEPLOY FAILED — <reason>' — and suppress/annotate the benign WS-probe WARN so it does not read as an error. Related: mtg-q97bw (fuzz prize), the web-polish batch.
