---
title: 'robots42 seed=42 intermittent WASM rewind+replay desync: pending_cast resume double-resolves a draw spell'
status: open
priority: 2
issue_type: bug
created_at: 2026-06-02T19:39:54.432003632+00:00
updated_at: 2026-06-03T19:50:00.873866375+00:00
---

# Description

robots42 seed=42 intermittent WASM rewind+replay desync (netarch STEP-3).

========================================================================
STATUS 2026-06-03 (slot01): sig-1 + sig-2(RNG) + sig-2b(reveal-mask) + t233k FIXED on netarch-undo-holes; sig-2c (opponent-hand shuffle) is the dominant REMAINING residual.

------------------------------------------------------------------------
sig-1 (opponent-shadow hidden-info library search) FIXED via reveal-history-buffer (commit 75d00f45). See git history.

------------------------------------------------------------------------
sig-2 (shuffle RNG not restored on partial rewind) FIXED:
GameState::shuffle_library logged ShuffleLibrary{previous_order} but NOT the RNG. A shuffle advances ChaCha12Rng; ShuffleLibrary::undo restored only library order. rewind_to_turn_start walks RNG back ONLY via per-action undos (stops AT the ChangeTurn boundary WITHOUT undoing it), so the un-restored RNG made the replayed shuffle produce a different order. FIX: ShuffleLibrary gains rng_state (Option<SmallVec<[u8;64]>>); shuffle_library captures capture_rng_state() before the shuffle; ShuffleLibrary::undo restores it (mirrors ChangeTurn). Reproducers (game_loop/mod.rs tests): shuffle_replay_byte_reproduces_after_partial_rewind, mass_draw_replay_reproduces_drawn_cards_after_partial_rewind (RED before, GREEN after).

------------------------------------------------------------------------
sig-2b (revealed_to_mask never cleared on entering hidden library) FIXED:
clear_revealed_to* existed but were NEVER called. A card put into the library kept its revealed_to_mask; maybe_reveal_to_player only logs RevealCard when !is_revealed_to(owner), so re-drawing a previously-public card (graveyard card shuffled in by Timetwister) SKIPPED the reveal. Because server and shadow shuffle independently and draw DIFFERENT cards, the RevealCard COUNT diverged. FIX: new GameState::maybe_conceal_in_library (mirrors maybe_reveal_to_player), wired into move_card via a (_ , Zone::Library) arm, RESTRICTED to is_revealed_to_all() cards (public cards are real instances on both server+shadow -> symmetric conceal; partial-mask hand cards are reserved IDs on other shadows and are left alone). Logs an undoable SetRevealedToMask{old->0}. Rules-correct (library is hidden). Reproducer: card_entering_library_is_concealed_and_re_revealed_on_draw_mb668_sig2b. Confirmed from dumps: reveal-count diff dropped 4 -> 1 and SetRevealedMask 0x03->0x00 fires for graveyard cards while correctly skipping hidden hand cards.

------------------------------------------------------------------------
mtg-t233k (mana_pool per-action undo gap) FIXED (see mtg-t233k): SetManaPool{prev} undo action + log_mana_pool helper before all 5 pay sites + partial-undo test.

GATE: robots42 seed=42 x10 went 3/10 -> ~7-9/10 with the three fixes (HIGH variance: same seed=42 gives different pass/fail run-to-run -> timing-dependent, 10-run samples are noisy). All native lib tests green.

------------------------------------------------------------------------
sig-2c FIXED (was the dominant remaining desync): the SHADOW does not shuffle the OPPONENT's hidden HAND into the library during hand-to-library effects (Timetwister; same class: Wheel/Windfall/Mind Twist).

HARD EVIDENCE (robots42 --undo-dump, Timetwister turn ~10, action_count 718 server / 717 wasm, P2 state-hash mismatch):
- SERVER actions 653-655: MoveCard(60/52/51 Hand -> Library owner=P0); then ShuffleLibrary(P0 57 cards).
- WASM: NO P0 hand->library moves (jumps straight to P1's hand cards); then ShuffleLibrary(P0 54 cards).
- Shadow shuffles P0 library with 54 vs server 57 (the 3 un-moved opponent hand cards). Fisher-Yates over a different length consumes DIFFERENT RNG -> P0 library order + shared ChaCha12Rng diverge -> cascades to P1's shuffle and even the LOCAL player's own draws (shadow drew card 103 where server drew 69/88/93).

EXACT CODE LOCATION: Effect::ChangeZoneAll in mtg-engine/src/game/actions/mod.rs (~4246-4252). The hidden-zone collection loop guards each card with `if let Some(card) = self.cards.try_get(card_id) { if restriction.matches(card) {...} }`. On the shadow the opponent's hand cards are reserved IDs with NO instance -> try_get None -> skipped -> not moved -> opponent library short by hand-size -> shuffle RNG divergence.
FIX DIRECTION: for shadow games + unrestricted mass-shuffle, move reserved (instance-less) hand IDs too (restriction cannot be evaluated without an instance; Timetwister is unrestricted). Keep non-shadow + restricted ChangeZoneAll (bounce/exile) unchanged; route reserved-ID moves through the existing is_shadow_game-tolerant move_card path. Needs a shadow reproducer + careful validation across all ChangeZoneAll cards. Likely overlaps reserved-ID / hidden-zone tracking work.

NEXT: fix sig-2c. Until then robots42 stays OUT of the make-validate gate (mirror white_weenie seed=7 / mtg-nkd71). Land sig-2 + sig-2b + t233k as a validated milestone.
========================================================================

------------------------------------------------------------------------
sig-2c FIX LANDED: Effect::ChangeZoneAll (actions/mod.rs) now moves the opponent's reserved (instance-less) hand/library CardIds into the destination on a SHADOW game when the restriction is UNRESTRICTED (new TargetRestriction::is_unrestricted()). So a Timetwister/Wheel/Windfall hand+graveyard->library mass-shuffle moves the opponent's hidden hand on the shadow exactly as the server does -> opponent library count matches -> Fisher-Yates consumes identical RNG -> server<->shadow stay in lockstep. Restricted ChangeZoneAll (typed bounce/exile) and the server (always real instances) are unchanged. Deterministic RED-first reproducer: shadow_mass_shuffle_moves_opponent_reserved_hand_to_library_mb668_sig2c (basic_actions.rs) — RED (lib 7 != 10) without the fix, GREEN with. Audited sibling effects: discard-hand (Wheel/Mind Twist) and mill collect raw CardIds (no try_get filter) so they were already count-safe; ChangeZoneAll was the unique offender. Full lib suite 1001/1001 green. Validating with robots42 x30 + a second seed/deck.

------------------------------------------------------------------------
sig-2d (REMAINING, identified — reveal-mask lockstep on library cycle): after sig-2c, robots42 x30 = 20/30. Remaining failures are around mass-draws (Timetwister/Wheel) as BOTH (i) equal-action-count content hash mismatches (RNG order already drifted) and (ii) action-count diff = +/-1.
EVIDENCE (diff=-1, server=2122 local=2123, Timetwister): server SKIPS the RevealCard for P0's own drawn cards 4 and 56 (already revealed to P0 from an earlier draw — stale revealed_to_mask retained because sig-2b only conceals is_revealed_to_all() cards, NOT owner-only-revealed cards), while the shadow (reserved/late-binding) logs the reveal unconditionally -> reveal COUNT diverges by 1 -> RNG/order drift downstream.
ROOT: maybe_reveal_to_player is CONDITIONAL (logs iff !is_revealed_to) when an instance exists, but UNCONDITIONAL on the reserved late-binding branch. The two only agree when the server's card is not-yet-revealed. A card that cycled library->hand->library->hand keeps its owner-bit on the server (skip) but is reserved on the shadow (log).
FIX DIRECTION (mtg-725 class): make library reveal/conceal SYMMETRIC regardless of hidden mask — e.g. (a) force the Library->Hand reveal unconditional (both sides always log), or (b) conceal the FULL mask on library ENTRY for instances AND log a symmetric late-binding conceal for reserved opponent cards (which always came from hand = mask nonzero). Both are broad reveal-semantics changes; needs a native reveal-count-parity reproducer first and careful validation. Deferred pending coordinator steer (native lockstep harness vs continue e2e dump-diff).

------------------------------------------------------------------------
sig-2d FIX LANDED: maybe_conceal_in_library now conceals ANY card with a non-empty revealed_to_mask on library entry (not just is_revealed_to_all), AND on a shadow logs a count-parity SetRevealedToMask for reserved (instance-less) opponent cards entering the library (they came from the owner's revealed hand, so the server logs a real conceal). This makes the library-exit (draw) reveal UNCONDITIONAL and symmetric on both sides: every library card is revealed to nobody, so every draw re-reveals regardless of prior reveal history -> RevealCard count stays in lockstep -> no RNG drift. SetRevealedToMask undo made tolerant of a missing instance (no-op) for the reserved count-parity entry. RED-first reproducers (basic_actions.rs): owner_only_revealed_card_is_concealed_entering_library_mb668_sig2d (RED: owner-only mask survived under sig-2b, redraw skipped reveal) + shadow_reserved_card_entering_library_logs_conceal_parity_mb668_sig2d. Full lib suite 1003/1003. Validating robots42 x30.

------------------------------------------------------------------------
sig-2e FIXED (was IDENTIFIED — WITHIN-side rewind-fidelity, distinct class): robots42 x30 with sig-2/2b/2c/2d + t233k = 19/30. The remaining failures are now confirmed to span MULTIPLE root classes:
  (A) server<->shadow divergence: ACTION COUNT MISMATCH + equal-count state-hash mismatch + Local-abilities drift (more count/RNG-lockstep events beyond sig-2c, e.g. additional reserved-ID/reveal mismatches around Wheel/Timetwister).
  (B) WITHIN-side rewind-fidelity (NEW, ~2/30): "REWIND/REPLAY FATAL: turn-start state hash for turn N changed across rewinds" with VERIFIER FIELD DIFF on `cards[52].counters` (and `mana_state_version`, likely diagnostic noise since rewind_to_turn_start bumps it). A counter mutation on a card (robots deck => almost certainly TRISKELION's +1/+1 counter removal, possibly as an activated-ability cost) is NOT a faithful undo-log inverse: rewind leaves counters stale and/or replay double-applies. This is the same undo-hole family as mtg-ba6uq (#4 SetCardCounters) but a path that bypasses the logged counter op.
sig-2e is DETERMINISTIC and within-side, so it is reproducible via the existing native rewind oracle (whole_game_rewind_replay_e2e / rewind_replay_oracle_e2e) driven over the robots deck — NO networking/flakiness needed. Next concrete target. CLASS (A) likely needs the per-action lockstep harness to enumerate the remaining count-divergence events.

------------------------------------------------------------------------
sig-2e FIX LANDED: Cost::SubCounter (Triskelion's "remove a +1/+1 counter: deal 1 damage" ping cost, actions/mod.rs pay_ability_cost) mutated the card via a direct `card.remove_counter(...)` with NO GameAction::RemoveCounter undo entry. Routed it through the LOGGED `self.remove_counters(card_id, counter_type, amount)` so the cost is a faithful undo-log inverse (remove_counters does NOT enforce the loyalty 0->die rule, so Triskelion still lives at 1/1 with zero counters). RED-first reproducer subcounter_cost_counter_removal_round_trips_on_undo_mb668_sig2e (basic_actions.rs): pays the cost, asserts the undo log GREW, then a partial undo restores the counter (3->2->3). RED before (no log entry, undo can't restore), GREEN after. Full lib suite 1004/1004.
