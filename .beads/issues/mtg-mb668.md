---
title: 'robots42 seed=42 intermittent WASM rewind+replay desync: pending_cast resume double-resolves a draw spell'
status: open
priority: 2
issue_type: bug
created_at: 2026-06-02T19:39:54.432003632+00:00
updated_at: 2026-06-03T18:00:39.398263153+00:00
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
sig-2c (RESIDUAL, NOT yet fixed — dominant remaining desync): the SHADOW does not shuffle the OPPONENT's hidden HAND into the library during hand-to-library effects (Timetwister; same class: Wheel/Windfall/Mind Twist).

HARD EVIDENCE (robots42 --undo-dump, Timetwister turn ~10, action_count 718 server / 717 wasm, P2 state-hash mismatch):
- SERVER actions 653-655: MoveCard(60/52/51 Hand -> Library owner=P0); then ShuffleLibrary(P0 57 cards).
- WASM: NO P0 hand->library moves (jumps straight to P1's hand cards); then ShuffleLibrary(P0 54 cards).
- Shadow shuffles P0 library with 54 vs server 57 (the 3 un-moved opponent hand cards). Fisher-Yates over a different length consumes DIFFERENT RNG -> P0 library order + shared ChaCha12Rng diverge -> cascades to P1's shuffle and even the LOCAL player's own draws (shadow drew card 103 where server drew 69/88/93).

EXACT CODE LOCATION: Effect::ChangeZoneAll in mtg-engine/src/game/actions/mod.rs (~4246-4252). The hidden-zone collection loop guards each card with `if let Some(card) = self.cards.try_get(card_id) { if restriction.matches(card) {...} }`. On the shadow the opponent's hand cards are reserved IDs with NO instance -> try_get None -> skipped -> not moved -> opponent library short by hand-size -> shuffle RNG divergence.
FIX DIRECTION: for shadow games + unrestricted mass-shuffle, move reserved (instance-less) hand IDs too (restriction cannot be evaluated without an instance; Timetwister is unrestricted). Keep non-shadow + restricted ChangeZoneAll (bounce/exile) unchanged; route reserved-ID moves through the existing is_shadow_game-tolerant move_card path. Needs a shadow reproducer + careful validation across all ChangeZoneAll cards. Likely overlaps reserved-ID / hidden-zone tracking work.

NEXT: fix sig-2c. Until then robots42 stays OUT of the make-validate gate (mirror white_weenie seed=7 / mtg-nkd71). Land sig-2 + sig-2b + t233k as a validated milestone.
========================================================================
