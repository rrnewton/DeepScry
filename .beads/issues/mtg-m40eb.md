---
title: 'Deploy-gate hardening: add a networked-game leg to the pre-deploy smoke (post-netarch)'
status: open
priority: 3
issue_type: task
created_at: 2026-06-02T22:43:03.507988895+00:00
updated_at: 2026-06-02T22:43:03.507988895+00:00
---

# Description

DEFERRED — stage AFTER all netarch rewind/replay work lands (user 2026-06-02). The pre-deploy smoke currently runs only (a) test_web_server_smoke.js (web-asset) + (b) a LOCAL WASM game (run_autoplay_ui mode=local) — it never plays a NETWORK game, so it cannot catch network desyncs (why 7b235b32 deployed despite the open ~30% desync). Add a short networked-game leg to the pre-deploy gate (or gate deploy on CI's Network E2E being green) so a network-desyncing build can't ship. Pairs with re-enabling web/test_network_human_input.js (parked behind mtg-679) once the human-path desync is fixed. Relates mtg-610.
