---
title: Comprehensive rewind+replay hash oracle + undo-log completeness audit (mtg-614)
status: open
priority: 2
issue_type: task
created_at: 2026-06-01T16:37:41.223155180+00:00
updated_at: 2026-06-01T17:03:29.489253530+00:00
---

# Description

Built mtg-engine/tests/rewind_replay_oracle_e2e.rs: a multi-turn (combat_test_4ed.dck, 12 turns, seed 42) oracle that, at every P1 priority decision point on turn 2+, does rewind_to_turn_start + deterministic replay of recorded intra-turn choices (both players via ReplayController) and asserts compute_state_hash (Replay mode) is EXACTLY equal across the round-trip. Complements rewind_replay_hash_roundtrip_e2e.rs (mid-resolution discard). Verifies 8 points across SpellAbility/Attackers/Blockers classes (+ whole-turn replay re-runs untap/upkeep/draw/combat-damage/end/cleanup). PASSES.

STATUS 2026-06-01: committed 03a679b3 on branch netarch-undolog-complete. make validate PASSED (validate_logs/validate_03a679b3...log: 'All validation checks passed', DIVERGED:0 all 4 legs, Failed:0; oracle ran in-suite PASS 4.0s; network multideck e2e PASSED for its 3 stable scenarios). NO engine source changed; pure test addition + this audit issue. No guard deleted, manual-reset block unchanged (still ~32 lines). robots42 (old_school/03_robots_jesseisbak seed=42) remains EXCLUDED in web/test_network_multideck.js — re-enabling needs the in-stack effect-resume work below.

AUDIT FINDINGS (undo.rs::rewind_to_turn_start manual-reset block):
- HASH-RELEVANT resets (priority_player/consecutive_passes; combat.clear(); per-card damage/power_bonus/toughness_bonus/temp_base_stats) were experimentally DISABLED and the oracle STILL PASSED — rewind goes to turn START and replay re-runs the whole turn, overwriting them deterministically. Redundant for the rewind+replay round-trip path.
- SERDE-SKIP fields in the block (9 TurnStructure *_turn guards via reset_transient_guards(); pending_cast/activation/cycling_search; spell_targets; pending_library_reorders; mana_caches/mana_state_version) are NOT in the Replay hash. Transient working buffers / pure caches, reset at re-entry for REPLAY-CORRECTNESS (so steps re-run), not hash equality.

WHY THE BLOCK + GUARDS CANNOT YET GO TO ZERO (honest blockers):
1. combat.clear(): declare_attacker/declare_blocker mutate CombatState directly with NO undoable GameAction. Need new GameAction variants logging combat declarations + damage assignment.
2. per-card temp_base_power/toughness: undo.rs comment states 'have NO undo support at all' (Animate). Genuine undo-log hole; not exercised by combat_test deck so the oracle can't catch it.
3. The 9 guards protect the NO-REWIND re-entry path (AI-vs-AI WASM run_one_turn tick; check_phase_triggers/draw/combat re-entry after a blocking priority_round). reset_transient_guards() in rewind_to_turn_start only CLEARS them so replay re-runs. Deleting the guards requires unifying ALL re-entry onto rewind+replay (mtg-610 step 2) — a fancy_tui.rs/game_loop refactor.
4. robots42/mtg-559 is in-stack SPELL-RESOLUTION re-entry: resolve_top_spell_with_discard_hook re-runs from the first effect instead of resuming; needs an effect-resume index (cf. pending_activation_effect_idx).

NEXT (to finish mtg-614): (a) add GameAction variants for combat declare/damage + temp_base_stats so combat.clear()/per-card loop become reversible; (b) add spell-resolution effect-resume index (unblocks robots42); (c) unify AI-vs-AI re-entry onto rewind+replay; (d) delete guards class-by-class, each gated on the oracle (extended to cover DamageOrder/Discard/ManaSources/Animate/scry) staying exactly green; (e) drive the block to 0; (f) re-enable robots42. The oracle is the binary gate that makes false completion impossible.
