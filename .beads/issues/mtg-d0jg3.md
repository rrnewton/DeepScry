---
title: WASM Network Client - Architecture and Sync Tracking
status: open
priority: 1
issue_type: task
labels:
- wasm
- network
- tracking
created_at: 2026-01-23T01:47:39.764992958+00:00
updated_at: 2026-03-31T16:14:31.198354358+00:00
---

# Description



---

### Network Reliability Improvements (2026-03-31_#1982)

**BTreeMap Migration for Deterministic Iteration:**
- Replaced HashMap with BTreeMap in 6 game-critical maps (combat damage, legendary rule, color counts)
- Added Ord derives/impls for Color and CardName
- Eliminates entire class of iteration-order desync bugs structurally

**Random Port Allocation for Test Isolation:**
- All network E2E tests now use random ports from range 10000-60000
- Added isPortAvailable() + getRandomPorts() to test_network_utils.js
- Eliminates EADDRINUSE failures from port conflicts

**Expanded CI Coverage:**
- test_network_gui_e2e.js: Now accepts --deck and --seed CLI args
- test_network_multideck.js: Runs multiple deck/seed combos (white_weenie, monored, 01_rogue, counterspells)
- CI network job runs: baseline + multideck (--quick, 2 scenarios) + click+log test
- Makefile validate-network-e2e-step updated to match

**Known Issue: Transient Race Condition**
- Sporadic desync at combat damage step (action_count off by 1)
- Not reproducible on re-run (passes 3/3 consecutive attempts)
- Likely sync_callback timing issue in WASM client
