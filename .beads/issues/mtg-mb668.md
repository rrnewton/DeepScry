---
title: 'robots42 seed=42 intermittent WASM rewind+replay desync: pending_cast resume double-resolves a draw spell'
status: open
priority: 2
issue_type: bug
created_at: 2026-06-02T19:39:54.432003632+00:00
updated_at: 2026-06-03T22:31:44.381893694+00:00
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

------------------------------------------------------------------------
sig-2f (IDENTIFIED, NOT fixed — WITHIN-side rewind-fidelity, combat/deal damage): after sig-2e, a SECOND rewind-fidelity field surfaced: VERIFIER FIELD DIFF on `cards[N].damage` ("turn-start state hash changed across rewinds", robots42 turn 26). Marked damage (`card.damage += amount`) is applied at actions/mod.rs:5309 and :8449 (and likely combat.rs) with NO undo GameAction — only MarkDamagedBy{target,source} (the SOURCE, not the AMOUNT) is logged. So damage applied during a turn's REPLAY isn't undone, and the verifier's second rewind-to-turn-start leaves it stale -> hash diverges. FIX (same pattern as sig-2e/t233k): add a logged GameAction (SetDamage{card_id, prev} snapshot, or MarkDamage{card_id, amount} whose undo saturating_sub's it) and route ALL `card.damage +=` sites through it; RED-first via the native rewind oracle. NOTE the recurring `mana_state_version` field-diff is almost certainly DIAGNOSTIC NOISE (rewind_to_turn_start bumps it by design and the replay hash excludes it) — confirm it's excluded from the verifier's field-diff or exclude it.

CLASS MAP (mtg-725): the robots42 residual is a MULTI-class, multi-bug audit:
  - WITHIN-side rewind-fidelity (undo-log not a faithful inverse): sig-2e counters (FIXED), sig-2f damage (TODO), possibly more per-field holes. DETERMINISTIC + reproducible via the native rewind oracle — RECOMMENDED next tool: a whole_game_rewind_replay_e2e-style native test driving the ROBOTS deck (RandomController, fixed seed) with the per-turn rewind-fidelity check, which enumerates ALL within-side undo holes at once with NO networking/flakiness.
  - server<->shadow count/RNG lockstep (class A): sig-2c (reserved hand move, FIXED), sig-2d (reveal-mask conceal, FIXED); residual ACTION COUNT MISMATCH events remain (capture via --undo-dump, diff server vs wasm ShuffleLibrary counts / reveal counts). Needs the per-action lockstep harness to enumerate.
STATUS: 6 fixes banked on netarch-undo-holes (sig-2/2b/2c/2d/2e + t233k), 1004/1004 lib green, tree clean. robots42 ~14-20/30 (high variance) — NOT green; multi-session work remains.

------------------------------------------------------------------------
INTERMITTENCY ROOT CAUSE (cheap-audit result — answers "same --seed, different outcome"): The engine RNG IS deterministic from --seed: `mtg server --seed N` -> ServerConfig.seed=Some(N) -> game_init seed_from_u64(N) (deck shuffle + initial RNG), and the server sends rng_state to the client. The NON-determinism is the CONTROLLER seed, NOT the engine: web/test_network_gui_e2e.js spawns the native AI as `connect --controller random` WITHOUT --seed-player, so main.rs:1625 falls back to `RandomController::with_seed(player, entropy_seed)` (entropy). => P2's CHOICES differ every run => different game trajectory => latent desyncs (the sig-2* class) trigger on a subset (~37%). NOT a non-seed-entropy bug in the engine seed-derivation, and NOT (for robots42) transport/H2.
IMPLICATIONS:
  - The fix bar is unchanged (the desyncs are real and must be fixed for true green), BUT
  - a DETERMINISTIC gate is achievable: pass --seed-player to the native `connect` client AND pin the WASM controller_seed (fancy_tui controller_seed field) so the FULL game (deck + both controllers' choices) is reproducible. Then a given (engine-seed, p1-seed, p2-seed) tuple either always-passes or always-fails -> the failing path is reproducible -> fix the exact divergence; and once the class is fixed the gate is stably green instead of flaky.
  - This is ALSO why ×30 sampling is noisy and why the native in-process lockstep harness must PIN BOTH controller seeds (RandomController::with_seed with fixed seeds for both players) to be deterministic.
Actionable test-harness improvement (dovetails mtg-726): make test_network_gui_e2e.js pass --seed-player (derived from --seed) to the native client and set the WASM controller_seed deterministically, so robots42/All-Hallow's-Eve become deterministic gates.

------------------------------------------------------------------------
DETERMINISTIC GATE + TRUE SCOPE (2026-06-03, after pinning controller seeds): with both controller masters pinned (e2e fix committed), robots deck swept across 20 DISTINCT seeds (each fully reproducible now) = 3 PASS / 17 FAIL. seed=42 is a clean trajectory (6/6) but most are not — the desync class is NOT closed. Every failure is now a DETERMINISTIC, reproducible RED repro: `node web/test_network_gui_e2e.js --deck decks/old_school/03_robots_jesseisbak.dck --seed <N>`.
FAILURE BREAKDOWN (17 fails):
  - WITHIN-side REWIND/REPLAY FATAL (largest cluster, ~8): seeds 1,7,8,10,12,14,15,17. = the rewind-fidelity undo-hole family (sig-2e counters FIXED; sig-2f damage + likely more fields TODO). DETERMINISTIC + within-side ⇒ reproduce/fix via the native rewind oracle (whole_game_rewind_replay_e2e pattern) over the robots deck with pinned RandomController seeds — NO networking. HIGHEST-LEVERAGE next batch.
  - class-A server↔shadow: ACTION COUNT MISMATCH seeds 5,6,20; state hash mismatch seeds 2,11,19; Local-abilities drift seeds 9,18; seed 4 (other). = count/reveal lockstep residue beyond sig-2c/2d.
RECOMMENDED PLAN: (1) fix the within-side rewind-fidelity cluster via the native oracle (sig-2f damage = add SetDamage{prev} + route card.damage += sites through it; then re-run oracle to surface the next field) — clears ~8/20 deterministically. (2) Then class-A via per-action lockstep harness. (3) Gate = all robots seeds 1..N green + a 2nd deck, deterministic.
PASS seeds (clean trajectories): 3, 13, 16, 42.

------------------------------------------------------------------------
RESUME NOTES FOR A FRESH slot01 (context-budget handoff, 2026-06-03):
STATE: branch netarch-undo-holes, 12 commits ahead of 75d00f45 (sig-1). Tree CLEAN. Engine lib 1004/1004. Release+wasm built. 6 engine fixes + e2e determinism fix all banked. Worktree registered in worktrees/ACTIVE.md (slot01).

NEXT TASK = sig-2f (cards[N].damage undo hole) — the biggest deterministic cluster (~8/20 seeds: 1,7,8,10,12,14,15,17 → REWIND/REPLAY FATAL).
  - Fix: add GameAction::SetDamage{card_id, prev: u16/u32} (snapshot BEFORE mutation, mirror SetManaPool/SetCardCounters) — Display + undo arm restoring card.damage = prev; tolerate missing instance on undo (like sig-2d's SetRevealedToMask). Add a `log_damage(card_id)`-style helper, call it before EVERY `card.damage += ...` site: actions/mod.rs:5309 and :8449, AND audit combat.rs for combat-damage application sites (the cards[N].damage divergence is COMBAT damage). 
  - RED-first via the EXISTING native rewind oracle pattern: tests/whole_game_rewind_replay_e2e.rs + tests/rewind_replay_oracle_e2e.rs already drive a game with a recorder controller, rewind_to_turn_start, replay, and assert state-hash + gamelog round-trip per turn. BUILD a new case there driving the ROBOTS deck with a RandomController (pin the seed to a FAILING sweep seed, e.g. 7) — it will deterministically reproduce the REWIND/REPLAY FATAL with NO networking. After sig-2f, re-run the oracle; it surfaces the NEXT within-side field hole. Repeat until the oracle is green across the failing seeds. This is the class-B "no within-side undo holes" systematic proof.
  - Then re-run the 20-seed e2e sweep; class-B clears ~8/20. Remaining ~9 are class-A (server↔shadow lockstep): per coordinator, class-A is now LOWER priority — if the seed-sweep goes green it's empirically covered; only build the per-action lockstep harness if a specific seed needs per-action diffing.

GREEN BAR (coordinator): robots deck seeds 1..N all deterministic-green + a 2nd deck, robots42 STILL in the make-validate gate (NO exclusion). PASS seeds today: 3,13,16,42.

MERGE/OVERLAP FLAG: the e2e determinism fix touches web/tui_game.html (~2473, seed boot param) and the native client is at main.rs (~1625). slot04's CDN-image work ALSO edits tui_game.html but in a DISJOINT region (image source URLs ~1670/1772/2000) — no conflict expected; whoever merges second rebases that file. This fix de-risks slot04's All-Hallow's-Eve flake (same unpinned-controller gap) + mtg-726.

------------------------------------------------------------------------
sig-2f + sig-2g LANDED (2026-06-03, slot01-2) — WITHIN-side class-B FULLY GREEN:
- sig-2f (commit fec60fcf): GameAction::SetDamage{card_id,prev:i32} + GameState::log_damage() snapshot helper; routed BOTH card.damage+= sites (deal_damage_to_creature = Triskelion ping; Effect::DamageAll) through it. Combat creature damage does NOT persist to card.damage (the damage_to_creatures map in actions/combat.rs is consumed only for the lethal check), so those are the only two accumulation sites. RED-first per-action oracle test rewind_replay_oracle_e2e::per_action_undo_redo_deal_damage_to_creature.
- sig-2g (commit 3a9dbb28): GameAction::SetXPaid{card_id,prev:u8} + GameState::set_x_paid_logged() (DRY single setter, mirrors log_damage); priority.rs X-payment site (the SINGLE x_paid mutation in the engine) now routes through it. RED-first lib test basic_actions::set_x_paid_round_trips_on_undo_mb668_sig2g.

DETERMINISTIC RE-SWEEP of the 8 within-side seeds (1,7,8,10,12,14,15,17), pinned controllers:
- sig-2f cleared 6 outright: seeds 8,10,12,14,15,17 now PASS.
- sig-2g cleared the within-side REWIND/REPLAY FATAL on the last 2 (seeds 1,7): their cards[N].x_paid turn-start-hash divergence is gone.
- Seeds 1,7 NOW fail ONLY on a deeper CLASS-A server↔shadow lockstep mismatch (seed 1: ACTION COUNT server=3465 local=3462 diff=3; seed 7: P1 state hash mismatch at choice_seq=146 action_count=783). DIFFERENT class (reserved-ID/reveal/shuffle lockstep), NOT a within-side undo hole.

=> The WITHIN-side rewind-fidelity undo-hole family is now CLOSED for the robots deck (no known per-field holes remain after damage + x_paid; sig-2e counters earlier). Field enumeration order was: cards[N].counters (sig-2e) → cards[N].damage (sig-2f) → cards[N].x_paid (sig-2g). mana_state_version is confirmed diagnostic noise (excluded from the verifier field-diff).

REMAINING = CLASS-A only (server↔shadow count/RNG lockstep): seeds 2,5,6,9,11,18,19,20 (state-hash/action-count/Local-abilities drift) PLUS seeds 1,7 now (underlying class-A exposed). Per coordinator this is slot03's disjoint chunk (per-action lockstep harness; reserved-ID/reveal-mask/shuffle RNG logic in actions/mod.rs ChangeZoneAll + reveal masks — does NOT touch undo.rs damage/x_paid region or priority.rs). Recommended slot03 fork point: 3a9dbb28.

NEW PASS COUNT (deterministic 20-seed sweep): was 3/20 (3,13,16,42). +6 within-side (8,10,12,14,15,17) = now ~9/20 confirmed; class-A seeds (1,2,5,6,7,9,11,18,19,20) remain for slot03. (Full 20-seed re-sweep TBD; the 8 within-side seeds were re-run directly.)
Engine lib 1005/1005 green. full make validate: running, cite validate_logs/validate_<sha>.log before merge.

------------------------------------------------------------------------
DECK-BROAD WITHIN-SIDE CLOSURE CONFIRMED (2026-06-03, slot01-2, on integration tip e1052f17 after class-B merge):
Ran the e2e rewind oracle (web/test_network_gui_e2e.js, WASM replay verifier) on 3 decks with DIFFERENT mechanics from robots, 3 seeds each (1,7,42):
  - old_school2/fireball_multitarget (X-spell + multi-target damage → SetXPaid/SetDamage paths): seed1 class-A, seed7 PASS, seed42 PASS
  - old_school2/ur_burn (direct burn/damage): seed1 PASS, seed7 PASS, seed42 class-A
  - old_school2/white_weenie_classic (creature combat): all 3 class-A (incl. the known-early seed7 mtg-nkd71-style case)
RESULT: ZERO within-side "REWIND/REPLAY FATAL: turn-start state hash changed across rewinds" on ANY new deck/seed. Every failure observed is CLASS-A (server↔shadow: "P1/P2 state hash mismatch ... at choice_seq=N action_count=M" or "ACTION COUNT MISMATCH server=X local=Y") — NOT a within-side undo-log hole.
=> The within-side rewind-fidelity undo-hole family (counters→damage→x_paid) is DURABLY CLOSED deck-broad across 4 diverse decks (robots-artifacts/pinger, fireball-X, ur-burn, white-weenie-combat). The "no known undo-log holes" half of the netarch Stop-goal is COMPLETE.
REMAINING work for true-green = CLASS-A only (server↔shadow count/RNG lockstep), slot03's chunk (network_choice/wasm/reveal-region; per-action lockstep harness). Seeds exhibiting class-A across decks: robots 1,2,5,6,7,9,11,18,19,20; fireball 1; ur_burn 42; white_weenie 1,7,42.
========================================================================
CLASS-A RESUME (fresh slot03 — re-based off STABLE integration e1052f17, 2026-06-03)
========================================================================
CHUNK 2 = server<->shadow per-action COUNT/RNG lockstep (class-A). Distinct from
slot01-2's class-B within-side rewind-fidelity; this is the reserved-ID/reveal
count-lockstep residue beyond sig-2c/2d.

FAILING DETERMINISTIC SEEDS: 2,5,6,9,11,18,19,20 (+ now-exposed 1,7) on
decks/old_school/03_robots_jesseisbak.dck.
REPRO: node web/test_network_gui_e2e.js --deck decks/old_school/03_robots_jesseisbak.dck --seed <N> [--undo-dump]
  -> on desync, dumps debug/netarch-undo-dumps/{stamp}_{wasm,server}_undo.log + _mismatch.log;
     signals: "ACTION COUNT MISMATCH" / "state hash mismatch" / "DESYNC"; exit 1 = desync.

BASE: STABLE integration e1052f17 — contains class-B (controller-seed-pinning
determinism gate + sig-2f + sig-2g, all rebased in; old throwaway-branch SHAs
9c868364/3cffa304 are NOT ancestors after the rebase — confirm the gate
BEHAVIORALLY via the seed-2 e2e repro showing a DETERMINISTIC class-A ACTION
COUNT / state-hash divergence, not by SHA ancestry).

ROOT-CAUSE CLASS (mtg-mb668 + mtg-725): the WASM shadow records None for opponent
hidden-info events (library-search fetch, mass-draw/shuffle) because the
authoritative reveal/move isn't available at first resolution on the shadow ->
it BRANCHES ON ABSENCE (try_get -> None), the exact mtg-725 anti-pattern -> the
shadow's Fisher-Yates draw COUNT / library decrement diverges from the server ->
action-count + state-hash mismatch under rewind+replay.

STEP 1 (tooling, RED-first): build a per-action server<->shadow lockstep harness
that, per action_count, asserts parity of (action_count, RNG draw count/state,
reveal-buffer application) between golden + shadow — ENUMERATE divergences instead
of chasing browser runs one at a time. NEW module + NEW test file (do not touch
basic_actions.rs). PROVEN FIX VEHICLE: the action_count-keyed REVEAL-HISTORY
BUFFER (e27c6f97 — already solved counterspells/rogerbrand async-reveal
nondeterminism); extend/mirror it so the authoritative search-result/move + draw
reveals reach the shadow deterministically and SURVIVE rewind, so first replay
reads Some (never None). Prefer total / identity-independent reserved-ID handling
over None-driven control flow (mtg-725 principle).

OWNED FILE SURFACE: game/game_loop/network_choice.rs ; wasm/network/* +
wasm/fancy_tui.rs (shadow reserved-ID/reveal paths) ; the reveal + ChangeZoneAll
reserved-ID region of game/actions/mod.rs (~4200/9100 — NOT the damage region
~5309/8449, which is slot01-2's class-B). DISCIPLINE: undo.rs GameAction enum
APPEND-ONLY; own test file; rebase onto origin/integration before merging
(ff-only); mtg-rules-review N/A (determinism, not rules); full make validate +
validate_<sha>.log before merge.

STEP-1 CONCRETE: RED-prove seed 2 first (repro above), capture its signature
(action_count at divergence, which side's hash diverges + field, the triggering
reveal event, the count delta) — then build the harness from it.
========================================================================

========================================================================
CLASS-A STEP-1 RESULT + SCOPE PIVOT (slot03, 2026-06-03, commit e2e13400)
========================================================================
SEED-2 RED (browser, DETERMINISTIC, byte-identical x2): `FATAL: P1 state hash
mismatch! server=92a4f5db6beab84e client=6a046ceab9665b6b at choice_seq=175
action_count=950`. P1 = the WASM (browser) shadow's view of its OWN state diverges
from the server's authoritative P1 hash at AC=950 (Turn-15 cleanup, P1 hand=8
must-discard-1, Mox Emerald fresh in hand → draw-count/hidden-info reveal class).
The per-choice undo-dump gives ONLY the hash; WASM side dumped 0 blocks (its dump
fires on action-count mismatch, not state-hash mismatch — local_controller.rs:231).

STEP-1 HARNESS BUILT: tests/netarch_lockstep_oracle_e2e.rs — pure-Rust in-process
golden GameServer + two native NetworkClient shadows, network_debug on, seed pinned
exactly like the browser. RESULT: UNIFORMLY GREEN across ALL class-A seeds
(1,2,5,6,7,9,11,18,19,20) + controls (3,13,16). => the native shadow CANNOT repro
class-A, and it's STRUCTURAL: native client = blocking-thread, NO client-side
rewind; it frontier-WAITS (condvar) for the authoritative reveal so try_get always
sees Some (client.rs:120-121). Class-A = branch-on-absence DURING REWIND+REPLAY,
which ONLY the WASM shadow does (wasm/network/client.rs reveal-history buffer +
rewind_to_turn_start/unwind_state_sync_to), and that module is wasm32-only
(#[cfg(all(feature="wasm-network", target_arch="wasm32"))]) — NOT reachable from a
native cargo test. So the file is committed as a NATIVE-SHADOW LOCKSTEP REGRESSION
GUARD (valuable control; default-run lean 1 class-A + 1 control, full sweep #[ignore]d).

PIVOT for the enumerating RED oracle: drive the ENGINE's shadow rewind+replay reveal
path directly (the game_loop/mod.rs opponent_library_search_fetch_* /
shuffle_replay_byte_* / mass_draw_replay_* native-oracle pattern) on the robots-deck
reveal scenarios — the WASM client is a thin wasm32 wrapper over those NATIVE engine
primitives (process_card_reveal, rewind_to_turn_start, is_shadow_game paths in
actions/mod.rs + state.rs). If the bug is in those shared primitives, that native
oracle catches+enumerates it; if it's purely WASM buffer plumbing
(unwind_state_sync_to ordering), only the browser e2e catches it. NEXT: build that
engine-level rewind+replay oracle, RED-first, reproducing the AC=950-class divergence.
========================================================================

========================================================================
CLASS-A SEED-2 FIELD ENUMERATION (slot03, 2026-06-03, commit cea709f1)
========================================================================
Built per-card desync enumeration tooling (DebugSyncInfo + battlefield_detail
(id,tapped,ctrl) + graveyard_ids; shared DRY view_* helpers in state_hash.rs gated
any(network,wasm-network); server log_state_differences per-card diff; WASM
WASM_CARD_DETAIL log keyed by choice_seq; e2e capture to
debug/netarch-undo-dumps/<stamp>_card_detail.log). Boxed GameToHandler::ChoiceRequest
(clippy large_enum_variant after DebugSyncInfo grew).

SEED-2 SIGNATURE (choice_seq=175), server (real, choice_request.debug_info) vs WASM
shadow (real, WASM_CARD_DETAIL):
- COARSE FIELDS ALL MATCH: turn 15 Main1 active=0, Life [16,13], Hands [7,7],
  Libs [51,52], bf count 6, stack 0, gy sizes [1,0], Hand CardIds [4,16,23,32,42,51,56].
- => mtg-725 R1 (count_cards_in_zone_matching opponent hand/lib count) is RULED OUT
  for seed 2: every size + the hand ids match.
- DIVERGING fields (the finer compute_view_hash fields):
  (a) land TAP-STATUS: server bf=[(49,T,0),(50,T,0),(59,T,0),(117,T,1),(122,T,1),(123,T,1)];
      shadow bf=[(49,F,0),(50,T,0),(59,F,0),(117,T,1),(122,T,1),(123,T,1)] — cards 49
      (Library of Alexandria) & 59 (Volcanic Island) tapped on server, UNtapped on shadow.
  (b) GRAVEYARD CONTENTS (dominant): server gy=[[55],[]] vs shadow
      gy=[[60,56,52,53,54],[118,115,114,116,112]] — the shadow has 5+5 reserved-range
      cards in BOTH graveyards that the server has elsewhere (library, post-shuffle),
      and lacks server's card 55. A reserved-id ZONE-ROUTING divergence (sig-2c family /
      R2-class: shadow routes opponent reserved cards to GRAVEYARD where the server
      sends them to library/keeps them), NOT a hand-count (R1) one.

OPEN ALIGNMENT PUZZLE: the server-REJECTED client hash (6a046cea) does NOT appear in
the WASM submit log (WASM logged 0d60bb6a for seq 175; the fatal box's action_count
"950" also disagrees with the ChoiceRequest's action_count 831 for seq 175). Either
choice_seq↔hash bookkeeping is offset, or the submitted hash is computed on a 2nd
uninstrumented path (local_controller.rs:290 damage/blocker submit, or a rewind/replay
recompute). NEXT: instrument the submit/receive/rewind cycle (log seq+hash at
client.submit_choice and at the server handler receive) to confirm the logged WASM
state == the submitted state, then the graveyard-routing divergence is the confirmed
class-A root for seed 2 → fix via the sig-2c/2d symmetric-reserved-id template on the
graveyard-routing path.
========================================================================

========================================================================
CLASS-A R1 FIXED NATIVELY (slot03, 2026-06-03, commit 2528a1bb)
========================================================================
Built the native engine-level oracle (GO from coordinator). R1
(count_cards_matching_filter, actions/mod.rs:1624) RED-proven natively + FIXED via
the sig-2c/2d symmetric-reserved-id template:
- New own test file actions/tests/netarch_reserved_zone.rs: shadow with reserved
  opponent Hand/Library ids; count_cards_matching_filter(p, "Card", zone) was 0 on
  shadow vs 5 golden (RED) → 5==5 after fix. Scope guard: typed/OppOwn filters stay
  0 (no over-count).
- FIX: when try_get=None && is_shadow_game, count the reserved id iff wildcard type
  ("" / "Card" / "Permanent") + zone-owner-relative qual (YouOwn/YouCtrl) + no color
  qual. Gated on is_shadow_game → server + normal play byte-unchanged.
- lib 1024/1024 green; fmt+clippy clean. mtg-rules-review N/A (determinism).

NOTE: R1 is an audit-confirmed real bug but is NOT seed-2's divergence (seed-2 =
graveyard zone-routing, R2/sig-2c class — see prior block: shadow gy has reserved
[60,56,52,53,54] the server moved to library). R1 fix may help seeds with
opponent-hidden-zone-count effects; seed-2 needs the graveyard-routing fix next.
NEXT: R2 oracle (restricted ChangeZoneAll with Hand/Graveyard private-zone origin
skips reserved on shadow, actions/mod.rs:4312 `None => {}`) — the closest match to
seed-2's graveyard signature — then R4 (state.rs:3488 ReturnToBattlefield). Walk
R2→R4→R5→R3→R6, re-running the browser acceptance sweep (robots 1,2,5,6,7,9,11,
18-20 + fireball s1 + ur_burn s42 + white_weenie 1/7/42) as the bar.
========================================================================

========================================================================
CLASS-A SNAPSHOT VERIFICATION — browser bookkeeping MISALIGNED (slot03, commit 54a246d4)
========================================================================
Instrumented BOTH ends (WASM_SUBMIT in client.rs + SRV_P1_RECV in server.rs,
network_debug-gated) to verify the seed-2 snapshot BEFORE fixing (coordinator's
explicit ask). RESULT: the browser desync-detection bookkeeping is misaligned — the
per-choice field snapshot CANNOT be trusted to pin the diverging field.
HARD EVIDENCE (seed 2): NativeAI = player 0, so P1 = WASM. Server-rejected P1 seq=175
client_hash=6a046cea @ ac=950; the WASM's own WASM_SUBMIT seq=175 = hash=0d60bb6a @
ac=831. 6a046cea is NEVER produced by the WASM (absent from all WASM_SUBMIT). seq
173/174 hashes also differ WASM_SUBMIT (a078a280/7179b30c) vs SRV_P1_RECV
(72e7d101/55c125fa). WASM per-request ac (831) != server (950) for the same seq; WASM
shadow ac maxes 861 vs server 950 → shadow undo-log ~89 actions SHORTER (skips actions
= branch-on-absence).
=> The desync is REAL (graveyard divergence + skipped shadow actions) but the
choice_seq↔ac↔hash mapping the browser detection relies on is itself off. So the
browser is unreliable for FIELD enumeration. DECISION (consistent with coordinator's
GO on native oracles): drive ALL class-A fixes via the NATIVE engine-level oracle
(deterministic, fully observable, no transport bookkeeping); use the browser e2e ONLY
as red/green seed ACCEPTANCE. The seq↔ac misalignment is a separate follow-up (does
NOT block the native-oracle fix path). NEXT: build the R2 native oracle (restricted /
graveyard-origin ChangeZoneAll skips reserved opponent cards on shadow,
actions/mod.rs:4312 None=>{}) — the seed-2 graveyard signature — RED-prove + fix via
sig-2c/2d template; then R4/R5/R3/R6; browser acceptance sweep as the bar.
========================================================================

========================================================================
R2 CHARACTERIZED — reveal-buffer/design class, NOT count-parity (slot03, 2026-06-03)
========================================================================
Native characterization (throwaway test, not committed): a RESTRICTED ChangeZoneAll
from a HIDDEN zone (Hand) on a shadow with 5 reserved opponent ids SKIPS all 5 (stay
in hand, 0 moved) — the `None => {}` arm at actions/mod.rs:4341 (move_reserved_in_shadow
is false for a restricted move). KEY: this is NOT sig-2c count-parity-fixable — the
shadow cannot know which of the 5 hidden reserved cards match the restriction
(Creature/etc.), so it cannot reproduce the server's move COUNT without an
authoritative reveal. R2 is therefore the REVEAL-BUFFER / hidden-info design class
(coordinator note 2), and a restricted mass-move from a HIDDEN zone is likely
UNREACHABLE by real cards (you can't deterministically mass-move opponent hidden
cards by type). => R2 is NOT seed-2's cause; do NOT apply a count-parity hack here.

REVISED PLAN: seed-2's graveyard divergence is a DIFFERENT site. The reliable
diagnostic (the seq/hash/ac browser bookkeeping is misaligned per commit 54a246d4,
but the undo-log CONTENT is ground-truth) = dump the SHADOW's undo-log unconditionally
(network_debug) and DIFF it against the server's captured 354-block undo-log to find
the ~89 actions the shadow SKIPS (MoveCard/RevealCard/etc.) — that names the exact
branch-on-absence site. THEN native-oracle + fix that site. Skip R2 (design/unreachable).
Likely next sites: R4 (state.rs:3488 DelayedEffect::ReturnToBattlefield, "pure sig-2c
shape, VERIFIED" — graveyard/exile->battlefield return skipped on shadow leaves extra
cards in shadow gy/exile, matching the seed-2 "shadow keeps cards server moved out"
pattern) and the reserved-id draw/discard→graveyard routing. R1 already landed.
========================================================================
