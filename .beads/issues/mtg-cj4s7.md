---
title: 'deploy-cloud.sh post-deploy probe false-fails: uses :8080 HTTPS through Cloudflare (stale URL)'
status: open
priority: 3
issue_type: bug
created_at: 2026-05-30T20:52:58.321899695+00:00
updated_at: 2026-05-30T20:52:58.321899695+00:00
---

# Description

deploy-cloud.sh 'deploy' subcommand returns EXIT=1 even on a SUCCESSFUL deploy because its post-deploy probe hits https://deepscry.net:8080 — but since Cloudflare proxying was turned on, the public front door is the PORT-LESS https://deepscry.net/ (443, CF-terminated). Probing :8080 through CF yields `curl (35) SSL routines::wrong version number` → probe fails → deploy reports failure despite shipping fine.

Evidence (2026-05-30, deploy of c3266d41): pre-deploy gate PASSED, rsync completed, systemd service restarted + running (MTG Server listening, HTTPS on 0.0.0.0:8080, 32434 cards loaded). https://deepscry.net/health returns the correct new build: {"sha":"c3266d41","version":"0.1.2513"}. VM-local: https://127.0.0.1:8080/health = 200 (origin TLS fine), http :8080 = 000. So origin is healthy; only the probe URL is wrong.

Fix: the post-deploy probe should target the actual front door. Options:
1. Probe the PORT-LESS https://<host>/health (through CF) — matches how users reach it.
2. AND/OR probe the origin directly on the VM over ssh (https://127.0.0.1:8080/health) to verify the binary independent of CF.
Make the probe URL derive from config (CF-proxied vs direct) rather than hardcoding :8080. A green deploy must not exit 1.
Location: scripts/deploy-cloud.sh, the "→ probing live deploy" block near the end (the curl https://...:8080 lines).
