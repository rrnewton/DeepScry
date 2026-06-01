---
title: WASM Human controller network desync at P2 cleanup discard (choice_seq=1, action_count=45)
status: open
priority: 2
issue_type: bug
created_at: 2026-06-01T03:02:23.445705864+00:00
updated_at: 2026-06-01T03:02:23.445705864+00:00
---

# Description

PRE-EXISTING network desync, deck-INDEPENDENT, reproduces in unmodified tui_game.html. Discovered while building the native network renderer (mtg-tnsk7).

REPRO (both fail with the IDENTICAL hash regardless of deck):
  make build-network && make wasm-network
  cd web && node test_network_gui_e2e.js --human                                  # grizzly_bears
  cd web && node test_network_gui_e2e.js --human --deck decks/old_school2/white_weenie_classic.dck

SYMPTOM (identical across decks):
  FATAL: P2 state hash mismatch! server=f7ec406da80a882e client=3b7dd10b9f66711d at choice_seq=1 action_count=45
  Server state: Turn 1 Upkeep, Hands [7,7]. Client (browser P2, Human controller) has already advanced to
  'WebHuman must discard 1 cards (hand size: 8, max: 7)' on its Turn 2 — i.e. the WASM Human-controller
  network client RUNS AHEAD of the server through P2's draw step into the cleanup discard, instead of
  blocking on the server's choice stream.

SCOPE:
  - Triggered ONLY with a Human controller on the WASM network client. The RANDOM controller path
    (test_network_gui_e2e.js with no --human) syncs fine — which is the ONLY mode wired into
    'make validate' (Makefile validate-network-e2e-step runs the random mode only). That is why this
    slipped through.
  - The identical server/client hash pair across two different decks proves it is the Human-controller
    run-ahead logic (likely tui_run_turn -> run_until_choice not yielding to the remote/server choice
    stream during P2's opening draw+cleanup), NOT deck data.
  - INDEPENDENT of the renderer: reproduces through the ratzilla TUI renderer (tui_game.html) and through
    the new native card renderer (native_game.html via launch_network_game_session) at the same point.
    The native renderer work (mtg-tnsk7) is correct; this is an orthogonal engine/controller bug.

LIKELY AREA: WASM Human controller in network mode + run_until_choice gating in mtg-engine/src/wasm
  (fancy_tui.rs tui_run_turn / WasmFancyTuiState::run_until_choice) and the network coordinator's choice
  sequencing for the non-active player's cleanup discard. The client should NOT advance P2's turn-2 draw
  locally before the server has sequenced it.

NEXT: add a Human-mode network sync leg to validate once fixed (currently only random mode is in validate).
See docs/NETWORK_ARCHITECTURE.md (desync is fatal; controllers must be information-independent).
