---
title: 'Native-vs-WASM trigger/life drift: WASM tui_run_turn step-loop diverges from native run_game (Su-Chi death trigger, City of Brass self-damage)'
status: open
priority: 2
issue_type: bug
created_at: 2026-05-30T22:06:25.279984686+00:00
updated_at: 2026-05-30T22:06:25.279984686+00:00
---

# Description

Surfaced by mtg-ofl2i work (2026-05-30): after fixing CardID-assignment-order (root cause #1) and controller-seeding (root cause #2), native-vs-WASM equivalence went 0/9 → 28/36, but 8/36 still diverge from a DISTINCT THIRD root cause — NOT seeding. The WASM game-stepping path (`tui_run_turn` / `run_one_turn` step loop in mtg-engine/src/wasm/fancy_tui.rs) processes triggers / life-total changes differently from native's continuous `run_game()`.

Concrete divergences observed (fresh-bundle native_wasm_equiv_sweep, old_school2 decks, seeds 1-3):
- **Su-Chi** "when this dies, add {C}{C}{C}{C}" DEATH TRIGGER fires in native but NOT in WASM (triple_s_sage @ action #61).
- **City of Brass** `Taps` self-damage applied a DIFFERENT NUMBER OF TIMES native vs WASM (ur_burn @ action #52).
- Downstream decision drift once life totals/triggers differ (white_weenie @ #28).

These are determinism BUGS by project policy (docs/NETWORK_ARCHITECTURE.md): same seed must produce identical game-state bits across run modes. They are PRE-EXISTING engine/stepping bugs newly EXPOSED now that CardIDs + seeds are aligned (previously masked by earlier divergence). Likely cause: the WASM per-turn step loop (run_one_turn / tui_run_turn) doesn't drain triggers / apply state-based-action damage at the same points as native's single continuous run_game — a trigger or SBA processed once-per-continuous-loop vs once-per-step.

Reproduce (FRESH bundle required — make wasm-dev first):
  make wasm-dev
  bug_finding/native_wasm_equiv_sweep.sh --decks 'decks/old_school2/*.dck' --seeds 3 --max-turns 8
  # ur_burn seed-? diverges @#52 (City of Brass); triple_s_sage @#61 (Su-Chi death trigger)

BLOCKS: flipping the native-vs-WASM validate leg from --expect-divergence (tripwire) to STRICT equality. mtg-ofl2i stays OPEN until this is fixed (its #1 CardID + #2 seeding fixes are merged @b087b07b). Once this lands, native==WASM should hold and the tripwire flips to a real equality gate (mtg-ofl2i closes).

Investigation: compare native run_game's trigger/SBA processing loop vs WASM run_one_turn (fancy_tui.rs). Find where a death trigger (Su-Chi) or repeated tap-damage (City of Brass) is handled per-continuous-pass natively but skipped/duplicated in the stepped WASM path.
