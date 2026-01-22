---
title: WASM network mode enters infinite loop (all AI controllers)
status: open
priority: 3
issue_type: task
labels:
- bug
- wasm
- network
created_at: 2026-01-22T20:41:08.494913065+00:00
updated_at: 2026-01-22T20:47:16.988364335+00:00
---

# Description

## Bug Description

When running WASM network mode with ANY AI controller (heuristic, random, zero), the game enters an infinite loop after connection, repeatedly calling `run_network_mode` without making any game progress.

## Affected Controllers

- **Heuristic**: ❌ Fails (infinite loop)
- **Random**: ❌ Fails (infinite loop)
- **Zero**: (likely fails, same pattern)

## Symptoms

1. WASM client connects to native server successfully
2. Game starts and receives initial messages (auth, game_started, card_revealed)
3. `run_network_mode` is called repeatedly in a loop, printing:
   ```
   run_network_mode: server assigned us 1, we_are_p1=false, opponent_id=0, our_controller=Random
   ```
4. No game progress (0 choices made)
5. Eventually server detects desync when WASM sends invalid choice

## Reproducer

### Using fuzz test (recommended):
```bash
cd /mtg-forge-rs-WinGamingPC
python3 bug_finding/network_fuzz_test.py --wasm --configs 3
```

Results: 0/5 passed for both heuristic and random controllers.

### Manual reproduction:
1. Start native server:
   ```bash
   ./target/release/mtg server --port 17771 --password play --seed 1
   ```

2. Start native opponent:
   ```bash
   ./target/release/mtg connect decks/booster_draft/avatar/gabriel_avatar_draft.dck \
     --server localhost:17771 --password play --controller random --name Native
   ```

3. Start web server:
   ```bash
   cd web && python3 -m http.server 8000
   ```

4. Open browser to http://localhost:8000/fancy.html
5. Select Network mode, Random (or Heuristic) controller
6. Connect to ws://localhost:17771 with password 'play'
7. Observe console log showing repeated `run_network_mode` calls with 0 choices made

## Server Error

```
DESYNC DETECTED: NetworkController 1 received invalid choice index 1 (only 1 options available).
Client sent indices [1]. This indicates client/server state divergence.
```

## Root Cause Analysis

The `run_network_mode` function in `wasm/fancy_tui.rs` is being called repeatedly by the JavaScript event loop but:
1. It's not actually entering the game loop
2. No choices are being made (choice_seq stays at 0)
3. The function returns immediately and gets called again

The integration between the WASM network client and the game loop is broken - the game loop never runs.

## Related Files

- `mtg-engine/src/wasm/fancy_tui.rs` - `run_network_mode` function
- `mtg-engine/src/wasm/network/` - WASM network client
- `web/fancy.html` - JavaScript integration

## Priority

Priority 3 - WASM network with AI controllers is a feature gap
