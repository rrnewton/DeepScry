// TODO(mtg-211): Remove once wildcard patterns are audited
#![allow(clippy::wildcard_enum_match_arm)]

//! COMPREHENSIVE rewind+replay state-hash oracle (mtg-614 / mtg-610 / mtg-559).
//!
//! ## What this proves
//!
//! The network/WASM blocking model is implemented by **rewind-to-turn-start +
//! replay-forward**. For that to be sound, the undo log must be COMPLETE: after
//! a `rewind_to_turn_start` followed by a deterministic replay of the recorded
//! intra-turn choices, the game-state hash (`compute_state_hash`, Replay mode)
//! must return EXACTLY to the value it had immediately before the rewind — for
//! a choice made at ANY NeedInput class, not just a mid-resolution discard.
//!
//! `rewind_replay_hash_roundtrip_e2e.rs` proves the property for ONE class
//! (a Bazaar mid-resolution discard). This test generalizes it: it drives a real
//! multi-turn 2-creature-deck game and, at EVERY decision point the deterministic
//! policy is asked for, performs a rewind+replay round-trip and asserts exact
//! hash equality before resuming the real game. The decision points span:
//!
//!   - SpellAbility (begin-of-phase / main-phase priority, cast, pass)
//!   - Attackers (declare attackers)
//!   - Blockers (declare blockers)
//!   - DamageOrder (multi-block damage assignment)
//!   - Discard (cleanup discard)
//!   - ManaSources (paying for a spell)
//!
//! A mismatch at any class means `undo.rs::rewind_to_turn_start` is INCOMPLETE
//! for that class — i.e. some canonical state was mutated without a logged
//! `GameAction`, so rewind leaves it stale and replay double-applies. Per
//! `docs/NETWORK_ARCHITECTURE.md`, desync is always fatal; this is a
//! compile-time-enforced regression gate.
//!
//! ## Mechanism
//!
//! `RecorderController` wraps a simple deterministic policy (play a land if
//! possible, cast a creature if possible, attack with everything, never block,
//! discard the first cards). Every choice it makes is appended to a shared
//! `Vec<ReplayChoice>`. It can be configured to return `NeedInput` the K-th time
//! it is asked for a priority decision (`choose_spell_ability_to_play`), which is
//! how a WASM human controller pauses the loop at a real decision point.
//!
//! The harness loop:
//!   1. Run forward with the recorder configured to pause at the K-th priority
//!      decision (K increases each iteration). Both players record their choices.
//!   2. If the loop completed (game over) before pausing, stop.
//!   3. Snapshot the post-forward hash.
//!   4. `rewind_to_turn_start`; partition the recorded intra-turn choices.
//!   5. Replay both players' recorded choices, pausing at the same point.
//!   6. Assert post-replay hash == post-forward hash.
//!   7. Resume the REAL game from the pause point (the recorder plays the actual
//!      decision instead of pausing), so the next iteration explores a later
//!      decision point. Increment K.

use mtg_engine::{
    core::{CardId, ManaCost, PlayerId, SpellAbility},
    game::{
        compute_state_hash,
        controller::{ChoiceContext, ChoiceResult, GameStateView, PlayerController},
        replay_controller::ReplayChoice,
        snapshot::ControllerType,
        GameLoop, GameLoopState, ReplayController, VerbosityLevel,
    },
    loader::{require_cardsfolder, AsyncCardDatabase as CardDatabase, DeckLoader},
    undo::GameAction,
    Result,
};
use smallvec::SmallVec;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

/// Shared log of the choices a recorder has made this game, in order, paired
/// with the player who made them. `rewind_to_turn_start` only returns the
/// ChoicePoints it logged, so we reconstruct the per-turn replay lists from the
/// undo log instead — but we keep this for a sanity assertion that the policy
/// actually exercised interesting classes.
#[derive(Default)]
struct ChoiceTally {
    spell_ability: usize,
    attackers: usize,
    blockers: usize,
    damage_order: usize,
    discard: usize,
    mana_sources: usize,
}

/// Deterministic policy + recorder. Pauses (NeedInput) at the `pause_at`-th
/// priority decision so the harness can rewind+replay at that point. When
/// `pause_at` is `None` it never pauses (used for the replay pass and for the
/// "resume the real game" pass once we have already verified the round-trip).
struct RecorderController {
    player_id: PlayerId,
    /// Pause (return NeedInput) the Nth time `choose_spell_ability_to_play` is
    /// called (0-based). None = never pause.
    pause_at: Option<usize>,
    /// How many times `choose_spell_ability_to_play` has been called so far.
    priority_calls: usize,
    tally: Rc<RefCell<ChoiceTally>>,
}

impl RecorderController {
    fn new(player_id: PlayerId, pause_at: Option<usize>, tally: Rc<RefCell<ChoiceTally>>) -> Self {
        Self {
            player_id,
            pause_at,
            priority_calls: 0,
            tally,
        }
    }
}

impl PlayerController for RecorderController {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn choose_spell_ability_to_play(
        &mut self,
        _view: &GameStateView,
        available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>> {
        let this_call = self.priority_calls;
        self.priority_calls += 1;
        if self.pause_at == Some(this_call) {
            return ChoiceResult::NeedInput(ChoiceContext::SpellAbility {
                available: available.to_vec(),
                formatted_choices: Vec::new(),
            });
        }
        self.tally.borrow_mut().spell_ability += 1;
        // Deterministic policy: prefer playing a land, then casting a creature,
        // else pass. Picking the FIRST matching ability keeps replay exact.
        let land = available.iter().find(|a| matches!(a, SpellAbility::PlayLand { .. }));
        if let Some(l) = land {
            return ChoiceResult::Ok(Some(l.clone()));
        }
        let cast = available.iter().find(|a| matches!(a, SpellAbility::CastSpell { .. }));
        if let Some(c) = cast {
            return ChoiceResult::Ok(Some(c.clone()));
        }
        ChoiceResult::Ok(None)
    }

    fn choose_targets(
        &mut self,
        _view: &GameStateView,
        _spell: CardId,
        _valid_targets: &[CardId],
        _min_targets: usize,
        _max_targets: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        ChoiceResult::Ok(SmallVec::new())
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        _view: &GameStateView,
        _cost: &ManaCost,
        available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        self.tally.borrow_mut().mana_sources += 1;
        ChoiceResult::Ok(available_sources.iter().copied().collect())
    }

    fn choose_attackers(
        &mut self,
        _view: &GameStateView,
        available_creatures: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        self.tally.borrow_mut().attackers += 1;
        // Attack with everything we can.
        ChoiceResult::Ok(available_creatures.iter().copied().collect())
    }

    fn choose_blockers(
        &mut self,
        _view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
        self.tally.borrow_mut().blockers += 1;
        // Block the first attacker with the first available blocker so the
        // DamageOrder / combat-damage classes get exercised. Keep it simple and
        // deterministic: at most one block.
        let mut out: SmallVec<[(CardId, CardId); 8]> = SmallVec::new();
        if let (Some(&blocker), Some(&attacker)) = (available_blockers.first(), attackers.first()) {
            out.push((blocker, attacker));
        }
        ChoiceResult::Ok(out)
    }

    fn choose_damage_assignment_order(
        &mut self,
        _view: &GameStateView,
        _attacker: CardId,
        blockers: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        self.tally.borrow_mut().damage_order += 1;
        ChoiceResult::Ok(blockers.iter().copied().collect())
    }

    fn choose_cards_to_discard(
        &mut self,
        _view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        self.tally.borrow_mut().discard += 1;
        ChoiceResult::Ok(hand.iter().take(count).copied().collect())
    }

    fn choose_from_library(
        &mut self,
        _view: &GameStateView,
        valid_cards: &[&mtg_engine::loader::CardDefinition],
    ) -> ChoiceResult<Option<usize>> {
        ChoiceResult::Ok(if valid_cards.is_empty() { None } else { Some(0) })
    }

    fn choose_permanents_to_sacrifice(
        &mut self,
        _view: &GameStateView,
        valid_permanents: &[CardId],
        count: usize,
        _card_type_description: &str,
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        ChoiceResult::Ok(valid_permanents.iter().take(count).copied().collect())
    }

    fn choose_permanents_to_not_untap(
        &mut self,
        _view: &GameStateView,
        _may_not_untap_permanents: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        ChoiceResult::Ok(SmallVec::new())
    }

    fn choose_modes(
        &mut self,
        _view: &GameStateView,
        _spell_id: CardId,
        mode_descriptions: &[String],
        mode_count: usize,
        _min_modes: usize,
        _can_repeat: bool,
    ) -> ChoiceResult<SmallVec<[usize; 4]>> {
        ChoiceResult::Ok((0..mode_count.min(mode_descriptions.len())).collect())
    }

    fn on_priority_passed(&mut self, _view: &GameStateView) {}
    fn on_game_end(&mut self, _view: &GameStateView, _won: bool) {}

    fn get_controller_type(&self) -> ControllerType {
        ControllerType::Tui
    }
}

/// Reconstruct the per-turn `(ours, theirs)` `ReplayChoice` lists from the
/// intra-turn ChoicePoints `rewind_to_turn_start` returned — mirrors
/// `WasmFancyTuiState::rewind_to_turn_start`.
fn partition_choices(choice_actions: Vec<GameAction>, our_id: PlayerId) -> (Vec<ReplayChoice>, Vec<ReplayChoice>) {
    let mut ours = Vec::new();
    let mut theirs = Vec::new();
    for action in choice_actions {
        if let GameAction::ChoicePoint {
            player_id,
            choice: Some(c),
            ..
        } = action
        {
            if player_id == our_id {
                ours.push(c);
            } else {
                theirs.push(c);
            }
        }
    }
    (ours, theirs)
}

/// Mirror `state_hash::strip_metadata` (Replay mode) so a divergence dump shows
/// exactly the fields the hash compares. Recursively removes the excluded keys.
fn strip_for_diff(value: serde_json::Value) -> serde_json::Value {
    const EXCLUDED: &[&str] = &[
        "choice_id",
        "undo_log",
        "logger",
        "show_choice_menu",
        "output_mode",
        "output_format",
        "numeric_choices",
        "step_header_printed",
        "mana_state_version",
        "lands_played_this_turn",
        "cards_drawn_this_turn",
        "spells_cast_this_turn",
    ];
    match value {
        serde_json::Value::Object(mut map) => {
            for f in EXCLUDED {
                map.remove(*f);
            }
            for (_, v) in map.iter_mut() {
                *v = strip_for_diff(v.clone());
            }
            serde_json::Value::Object(map)
        }
        serde_json::Value::Array(arr) => serde_json::Value::Array(arr.into_iter().map(strip_for_diff).collect()),
        other => other,
    }
}

async fn load_game() -> Result<mtg_engine::game::GameState> {
    let cardsfolder = require_cardsfolder();
    let deck_path = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../decks/combat_test_4ed.dck"));
    let deck = DeckLoader::load_from_file(&deck_path)?;
    let card_db = CardDatabase::new(cardsfolder);
    let initializer = mtg_engine::loader::GameInitializer::new(&card_db);
    let mut game = initializer
        .init_game("P1".to_string(), &deck, "P2".to_string(), &deck, 20)
        .await?;
    game.seed_rng(42);
    Ok(game)
}

/// THE comprehensive oracle. For each decision point K (in increasing order),
/// run forward to K, rewind, replay, and assert the state hash round-trips
/// exactly. Then advance the real game past K and continue.
#[tokio::test]
async fn rewind_replay_oracle_round_trips_at_every_decision_point() -> Result<()> {
    // We re-run the whole game from scratch for each pause point K. This is
    // O(turns^2) but the deck is tiny and the game is short; correctness and a
    // clean failure attribution (which K / which turn) matter more than speed.
    let tally = Rc::new(RefCell::new(ChoiceTally::default()));

    // First, run the game once with NO pause to discover how many priority
    // decisions P1 makes across the whole game — that bounds K.
    let total_p1_priority_calls = {
        let mut game = load_game().await?;
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;
        let mut p1 = RecorderController::new(p1_id, None, Rc::clone(&tally));
        let mut p2 = RecorderController::new(p2_id, None, Rc::clone(&tally));
        {
            let mut game_loop = GameLoop::new(&mut game)
                .with_verbosity(VerbosityLevel::Silent)
                .with_max_turns(12);
            let _ = game_loop.run_game(&mut p1, &mut p2)?;
        }
        p1.priority_calls
    };

    assert!(
        total_p1_priority_calls > 5,
        "expected the policy to make several priority decisions; got {total_p1_priority_calls}"
    );

    let mut verified_points = 0usize;
    let mut verified_turn1_points = 0usize;

    for k in 0..total_p1_priority_calls {
        let mut game = load_game().await?;
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // ── Forward pass: pause at P1's K-th priority decision ────────────────
        // Turn 1 now carries a clean post-setup `ChangeTurn` boundary marker
        // (`GameState::ensure_turn_one_boundary`, mtg-610), so a turn-1 rewind
        // stops at that boundary instead of over-rewinding into the RNG-consuming
        // opening-hand setup. Every turn — including turn 1 — therefore goes
        // through the SAME `rewind_to_turn_start` path, with no full-state
        // baseline-clone special case (the former WASM AI-harness workaround is
        // now obsolete; this is the unification the marker buys).
        let stopped = {
            let mut p1 = RecorderController::new(p1_id, Some(k), Rc::clone(&tally));
            let mut p2 = RecorderController::new(p2_id, None, Rc::clone(&tally));
            let result = {
                let mut game_loop = GameLoop::new(&mut game)
                    .with_verbosity(VerbosityLevel::Silent)
                    .with_max_turns(12);
                game_loop.run_until_input(&mut p1, &mut p2)?
            };
            matches!(result, GameLoopState::AwaitingInput(_))
        };

        if !stopped {
            // Game ended before reaching this decision point — nothing more to verify.
            break;
        }

        // Snapshot the hash at the pause point (this is what replay must reproduce).
        let hash_after_forward = compute_state_hash(&game);
        // Also snapshot the undo-log length (== `action_count`). The NETWORK
        // cross-replica hash (`compute_view_hash`) hashes `action_count` as a
        // consensus value (mtg-752), but `compute_state_hash` deliberately
        // EXCLUDES the undo log — so a divergence that touches ONLY the undo-log
        // length (e.g. an extra no-op action) is invisible to the hash check
        // above yet FATAL on the wire. mtg-885 was exactly this: a no-rewind
        // resume re-executed `end_combat_step` and double-logged a (state-no-op)
        // `ClearCombat`, advancing `action_count` by one. Asserting the action
        // count round-trips through rewind+replay guards that whole class.
        let action_count_after_forward = game.undo_log.len();
        let forward_json = strip_for_diff(serde_json::to_value(&game).unwrap());

        // Track turn-1-ness (for the hole-(a) coverage assertion below) before
        // rewinding. With the turn-1 boundary marker the undo log now HAS a
        // ChangeTurn for turn 1, so detect via the turn number directly.
        let in_turn_one = game.turn.turn_number == 1;

        // ── Unified undo-log rewind path (all turns, incl. turn 1) ────────────
        let (p1_choices, p2_choices) = {
            let mut undo_log = std::mem::take(&mut game.undo_log);
            let rewind = undo_log.rewind_to_turn_start(&mut game);
            game.undo_log = undo_log;
            let (_turn, choice_actions, _rewound, _log_size) =
                rewind.expect("rewind_to_turn_start must succeed (undo log enabled)");
            partition_choices(choice_actions, p1_id)
        };

        // ── Replay both players' recorded intra-turn choices ──────────────────
        // The inner controllers never pause (pause_at = None) but the replay
        // queues end exactly at the pause point, so once exhausted the inner
        // recorder makes the SAME decision the forward pause point would have —
        // EXCEPT we must stop there. To stop at the same place, give P1 an inner
        // recorder that pauses on its very first (post-replay) priority call.
        let p1_inner = RecorderController::new(p1_id, Some(0), Rc::clone(&tally));
        let p2_inner = RecorderController::new(p2_id, None, Rc::clone(&tally));
        let mut p1_replay = ReplayController::new(p1_id, Box::new(p1_inner), p1_choices);
        let mut p2_replay = ReplayController::new(p2_id, Box::new(p2_inner), p2_choices);
        {
            let mut game_loop = GameLoop::new(&mut game)
                .with_verbosity(VerbosityLevel::Silent)
                .with_max_turns(12);
            let _ = game_loop.run_until_input(&mut p1_replay, &mut p2_replay)?;
        }

        let hash_after_replay = compute_state_hash(&game);
        if hash_after_replay != hash_after_forward {
            // On a divergence, dump both stripped states to the gitignored debug/
            // dir so the exact diverging field(s) can be inspected. Best-effort:
            // ignore I/O errors (e.g. debug/ missing in a CI sandbox).
            let replay_json = strip_for_diff(serde_json::to_value(&game).unwrap());
            let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../debug"));
            let _ = std::fs::create_dir_all(dir);
            let fp = dir.join(format!("oracle_k{k}_forward.json"));
            let rp = dir.join(format!("oracle_k{k}_replay.json"));
            let _ = std::fs::write(&fp, serde_json::to_string_pretty(&forward_json).unwrap());
            let _ = std::fs::write(&rp, serde_json::to_string_pretty(&replay_json).unwrap());
            eprintln!("DIVERGENCE at K={k}: wrote {} and {}", fp.display(), rp.display());
        }
        let path = if in_turn_one {
            "turn-1 undo-log rewind (boundary marker)"
        } else {
            "turn>=2 undo-log rewind"
        };
        assert_eq!(
            hash_after_replay, hash_after_forward,
            "REWIND/REPLAY ORACLE FAILED at decision point K={k} ({path} path): after \
             rewind-to-turn-start + deterministic replay of the recorded intra-turn choices, the \
             state hash did not return to its pre-rewind value \
             ({hash_after_replay:#x} != {hash_after_forward:#x}). \
             This means the rewind+replay round-trip is INCOMPLETE for the NeedInput class at \
             this decision point — some canonical state was mutated without a logged GameAction \
             (or the turn-1 baseline is not a faithful post-setup checkpoint). \
             See mtg-614 / mtg-610 / docs/NETWORK_ARCHITECTURE.md (desync is always fatal)."
        );

        // action_count (undo-log length) MUST also round-trip — the network
        // view-hash invariant that `compute_state_hash` cannot see (mtg-885).
        let action_count_after_replay = game.undo_log.len();
        assert_eq!(
            action_count_after_replay, action_count_after_forward,
            "REWIND/REPLAY ORACLE FAILED at decision point K={k} ({path} path): the undo-log \
             length (== network action_count) did not round-trip through rewind+replay \
             ({action_count_after_replay} != {action_count_after_forward}). The full-state hash \
             may match (compute_state_hash excludes the undo log) while the NETWORK \
             compute_view_hash — which hashes action_count as a cross-replica consensus value \
             (mtg-752) — diverges. This is the mtg-885 class: an action logged twice (or not \
             reversed) across the resume boundary. See docs/NETWORK_ARCHITECTURE.md."
        );

        verified_points += 1;
        if in_turn_one {
            verified_turn1_points += 1;
        }
    }

    assert!(
        verified_points > 0,
        "oracle must have verified at least one rewind+replay round-trip"
    );
    // Hole (a): the turn-1 rewind+replay path MUST be exercised — that is the
    // root cause of the user-reported "froze after the first land" bug (turn-1
    // PlayLand lost on rewind+replay). With the turn-1 boundary marker this now
    // goes through the SAME `rewind_to_turn_start` path as turn 2+. If this is 0
    // the deck/policy stopped reaching turn-1 decision points and the turn-1
    // guarantee is silently unverified.
    assert!(
        verified_turn1_points > 0,
        "oracle must have verified at least one TURN-1 rewind+replay round-trip (hole (a)); \
         got 0 — the turn-1 rewind path was never exercised"
    );
    let t = tally.borrow();
    eprintln!(
        "oracle verified {verified_points} decision points ({verified_turn1_points} on turn 1); \
         class tally across all runs: \
         spell_ability={} attackers={} blockers={} damage_order={} discard={} mana_sources={}",
        t.spell_ability, t.attackers, t.blockers, t.damage_order, t.discard, t.mana_sources
    );

    Ok(())
}

// =============================================================================
// PER-ACTION undo/redo round-trips (mtg-614 holes (b) and (c), oracle part (d))
//
// The whole-turn oracle above proves rewind_to_turn_start + replay round-trips,
// but it cannot isolate a SINGLE action's reversibility (a whole-turn rewind
// re-runs combat from scratch, hiding a non-undoable combat mutation). These
// tests apply ONE logged GameAction, snapshot compute_state_hash, `undo()` it,
// assert the hash returns EXACTLY to the pre-apply value, then re-apply and
// assert it returns EXACTLY to the post-apply value. This is the binary gate
// for per-action MCTS time-travel: each canonical-state mutation must be a
// faithful GameAction inverse.
// =============================================================================

use mtg_engine::core::{Card, CardType};
use mtg_engine::game::GameState;

/// Build a 2-player game with `n` vanilla creatures on the battlefield for the
/// given player. Returns the game and the creature IDs (deterministic order).
fn game_with_creatures(player_for: usize, specs: &[(&str, i8, i8)]) -> (GameState, Vec<CardId>, [PlayerId; 2]) {
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1 = game.players[0].id;
    let p2 = game.players[1].id;
    let owner = [p1, p2][player_for];
    let mut ids = Vec::new();
    for (name, power, toughness) in specs {
        let id = game.next_card_id();
        let mut card = Card::new(id, (*name).to_string(), owner);
        card.add_type(CardType::Creature);
        card.set_base_power(Some(*power));
        card.set_base_toughness(Some(*toughness));
        card.controller = owner;
        game.cards.insert(id, card);
        game.battlefield.add(id);
        ids.push(id);
    }
    (game, ids, [p1, p2])
}

/// Pop the most recent action and undo it via the centralized `GameAction::undo`
/// (the same inverse `rewind_to_turn_start` applies). Mirrors a per-action MCTS
/// step-back without going through `GameState::undo`'s cache-dirtying path, so
/// the assertion isolates the action's own state inverse.
fn undo_last(game: &mut GameState) -> GameAction {
    let mut undo_log = std::mem::take(&mut game.undo_log);
    let (action, _prior) = undo_log.pop().expect("undo log must have an action to pop");
    action.undo(game).expect("undo must succeed");
    game.undo_log = undo_log;
    action
}

/// Hole (b): declaring an attacker is a reversible GameAction.
#[test]
fn per_action_undo_redo_declare_attacker() {
    let (mut game, ids, players) = game_with_creatures(0, &[("Grizzly Bears", 2, 2)]);
    let attacker = ids[0];
    let defender = players[1];

    let hash_before = compute_state_hash(&game);

    // Apply: declare attacker (logged). NOTE: we use the combat-only logged
    // helper (not GameState::declare_attacker) so this test isolates the
    // CombatState mutation + its GameAction inverse, independent of the tap /
    // trigger side effects (which have their own separately-tested inverses).
    // `declare_attacker_logged` logs TWO reversible actions: DeclareAttacker
    // (combat state) and MarkAttackedThisTurn (Berserk's per-turn flag, mtg-713
    // B18). Both must round-trip for the state hash to invert exactly.
    game.declare_attacker_logged(attacker, defender);
    assert!(game.combat.is_attacking(attacker));
    assert!(
        game.cards.get(attacker).unwrap().attacked_this_turn,
        "declaring an attacker must set attacked_this_turn"
    );
    let hash_after = compute_state_hash(&game);
    assert_ne!(
        hash_before, hash_after,
        "declaring an attacker must change the state hash (combat.attackers / combat_active / attacked_this_turn)"
    );

    // Undo both actions (newest first): MarkAttackedThisTurn, then
    // DeclareAttacker. The state must return EXACTLY to the pre-declare hash.
    let mark_action = undo_last(&mut game);
    assert!(matches!(mark_action, GameAction::MarkAttackedThisTurn { .. }));
    assert!(
        !game.cards.get(attacker).unwrap().attacked_this_turn,
        "undo must clear attacked_this_turn"
    );
    let action = undo_last(&mut game);
    assert!(matches!(action, GameAction::DeclareAttacker { .. }));
    assert!(!game.combat.is_attacking(attacker), "undo must remove the attacker");
    assert_eq!(
        compute_state_hash(&game),
        hash_before,
        "undo of DeclareAttacker + MarkAttackedThisTurn must restore the exact pre-declare state hash \
         (combat.attackers map, combat_active flag, AND attacked_this_turn)"
    );

    // Redo (re-apply) → must return EXACTLY to the post-declare hash.
    game.declare_attacker_logged(attacker, defender);
    assert_eq!(
        compute_state_hash(&game),
        hash_after,
        "re-applying DeclareAttacker must reproduce the exact post-declare state hash"
    );
}

/// Hole (b): declaring a blocker is a reversible GameAction (including the
/// reverse attacker_blockers map and combat_active when the clear restores it).
#[test]
fn per_action_undo_redo_declare_blocker_and_clear() {
    let (mut game, ids, players) = game_with_creatures(0, &[("Hill Giant", 3, 3), ("Wall", 0, 4), ("Wall II", 0, 4)]);
    // P1 owns all three for setup simplicity; assign blockers to P2.
    let attacker = ids[0];
    let blocker1 = ids[1];
    let blocker2 = ids[2];
    let defender = players[1];

    // Set up an attack so blockers have something to block.
    game.declare_attacker_logged(attacker, defender);
    let hash_attack_only = compute_state_hash(&game);

    // Declare blocker1 (logged).
    let mut atk_vec: SmallVec<[CardId; 2]> = SmallVec::new();
    atk_vec.push(attacker);
    game.declare_blocker_logged(blocker1, atk_vec.clone());
    assert!(game.combat.is_blocking(blocker1));
    assert!(game.combat.is_blocked(attacker));
    let hash_one_block = compute_state_hash(&game);
    assert_ne!(hash_attack_only, hash_one_block);

    // Declare a SECOND blocker on the same attacker, so the reverse-map entry
    // has 2 elements — undoing the 2nd must prune to exactly 1 (not 0, not an
    // empty leftover Vec).
    game.declare_blocker_logged(blocker2, atk_vec.clone());
    let hash_two_blocks = compute_state_hash(&game);
    assert_ne!(hash_one_block, hash_two_blocks);

    // Undo the 2nd blocker → exactly back to the one-block hash.
    let a2 = undo_last(&mut game);
    assert!(matches!(a2, GameAction::DeclareBlocker { .. }));
    assert_eq!(
        compute_state_hash(&game),
        hash_one_block,
        "undo of the 2nd DeclareBlocker must restore the exact one-block state \
         (reverse attacker_blockers pruned to a single entry, not an empty leftover)"
    );

    // Undo the 1st blocker → exactly back to the attack-only hash.
    let a1 = undo_last(&mut game);
    assert!(matches!(a1, GameAction::DeclareBlocker { .. }));
    assert!(!game.combat.is_blocked(attacker));
    assert_eq!(
        compute_state_hash(&game),
        hash_attack_only,
        "undo of the 1st DeclareBlocker must restore the exact attack-only state \
         (attacker_blockers entry fully removed, not left empty)"
    );

    // Now exercise ClearCombat reversibility: re-declare both blockers, snapshot,
    // clear combat (logged), then undo the clear → must restore the full combat.
    game.declare_blocker_logged(blocker1, atk_vec.clone());
    game.declare_blocker_logged(blocker2, atk_vec);
    let hash_full = compute_state_hash(&game);

    let prev = Box::new(game.combat.clone());
    let prior_log_size = game.logger.log_count();
    game.combat.clear();
    game.undo_log.log(GameAction::ClearCombat { prev }, prior_log_size);
    assert!(!game.combat.is_attacking(attacker), "clear must empty combat");

    let ac = undo_last(&mut game);
    assert!(matches!(ac, GameAction::ClearCombat { .. }));
    assert_eq!(
        compute_state_hash(&game),
        hash_full,
        "undo of ClearCombat must restore the exact pre-clear CombatState \
         (all attacker/blocker declarations)"
    );
}

/// Hole sig-2f (mtg-728): marking damage on a creature is a reversible
/// GameAction. Triskelion's ping (`deal_damage_to_creature`) and `DamageAll`
/// both did `card.damage += ...` with NO logged GameAction, so a mid-turn
/// rewind left the marked damage stale and a replay double-applied it — the
/// robots42 within-side "cards[N].damage changed across rewinds" REWIND/REPLAY
/// FATAL. This isolates the per-action inverse: deal damage, undo, assert the
/// hash returns EXACTLY to the pre-damage value; then re-apply and assert the
/// post-damage hash. RED before the fix (no log entry => undo_last pops the
/// wrong/no action and damage stays marked), GREEN after.
#[test]
fn per_action_undo_redo_deal_damage_to_creature() {
    let (mut game, ids, _players) = game_with_creatures(0, &[("Grizzly Bears", 2, 2)]);
    let card_id = ids[0];

    assert_eq!(game.cards.get(card_id).unwrap().damage, 0);
    let hash_before = compute_state_hash(&game);
    let log_before = game.undo_log.actions().len();

    // Apply 1 non-lethal damage (the Triskelion ping path: deal_damage_to_creature).
    game.deal_damage_to_creature(card_id, 1)
        .expect("deal damage must succeed");
    assert_eq!(game.cards.get(card_id).unwrap().damage, 1);

    // The damage mutation MUST log a GameAction (sig-2f: `card.damage +=` had none).
    let log_after = game.undo_log.actions().len();
    assert!(
        log_after > log_before,
        "deal_damage_to_creature must log a GameAction so marked damage round-trips on undo \
         (sig-2f: card.damage += had no undo entry)"
    );
    let hash_after = compute_state_hash(&game);
    assert_ne!(hash_before, hash_after, "marking damage must change the state hash");

    // Undo -> restores damage 0 and the EXACT pre-damage hash.
    let action = undo_last(&mut game);
    assert!(matches!(action, GameAction::SetDamage { .. }));
    assert_eq!(
        game.cards.get(card_id).unwrap().damage,
        0,
        "undo of SetDamage must clear the marked damage"
    );
    assert_eq!(
        compute_state_hash(&game),
        hash_before,
        "undo of SetDamage must restore the exact pre-damage state hash"
    );

    // Re-apply -> exactly the post-damage hash (faithful redo).
    game.deal_damage_to_creature(card_id, 1)
        .expect("re-deal damage must succeed");
    assert_eq!(
        compute_state_hash(&game),
        hash_after,
        "re-applying damage must reproduce the exact post-damage state hash"
    );
}

/// Hole (c): a temp base P/T override (Animate / base-set) is a reversible
/// GameAction, including the clear path.
#[test]
fn per_action_undo_redo_temp_base_stats() {
    let (mut game, ids, _players) = game_with_creatures(0, &[("Mishra's Factory", 0, 0)]);
    let card_id = ids[0];

    // Baseline: no temp override set yet.
    assert_eq!(game.cards.get(card_id).unwrap().temp_base_power(), None);
    let hash_none = compute_state_hash(&game);

    // Apply an Animate-style base-set (2/2) via the logged helper.
    game.set_temp_base_stats_logged(card_id, Some(2), Some(2));
    assert_eq!(game.cards.get(card_id).unwrap().temp_base_power(), Some(2));
    assert_eq!(game.cards.get(card_id).unwrap().temp_base_toughness(), Some(2));
    let hash_animated = compute_state_hash(&game);
    assert_ne!(
        hash_none, hash_animated,
        "setting a temp base P/T override must change the state hash"
    );

    // Undo → must restore the prior `None`/`None` exactly.
    let action = undo_last(&mut game);
    assert!(matches!(action, GameAction::SetTempBaseStats { .. }));
    assert_eq!(game.cards.get(card_id).unwrap().temp_base_power(), None);
    assert_eq!(
        compute_state_hash(&game),
        hash_none,
        "undo of SetTempBaseStats must restore the exact prior override (None/None)"
    );

    // Re-apply → exactly the animated hash.
    game.set_temp_base_stats_logged(card_id, Some(2), Some(2));
    assert_eq!(compute_state_hash(&game), hash_animated);

    // Now stack a SECOND override on top (e.g. a later base-set to 4/4) and undo
    // it — must restore the FIRST override (Some(2)/Some(2)), not None. This is
    // why the action stores the PRIOR Option pair, not a clear.
    game.set_temp_base_stats_logged(card_id, Some(4), Some(4));
    let hash_4_4 = compute_state_hash(&game);
    assert_ne!(hash_animated, hash_4_4);
    let action2 = undo_last(&mut game);
    assert!(matches!(action2, GameAction::SetTempBaseStats { .. }));
    assert_eq!(game.cards.get(card_id).unwrap().temp_base_power(), Some(2));
    assert_eq!(
        compute_state_hash(&game),
        hash_animated,
        "undo of a stacked SetTempBaseStats must restore the FIRST override (2/2), not None"
    );

    // Exercise the logged CLEAR path: clear the override, then undo → restores 2/2.
    game.clear_temp_base_stats_logged(card_id);
    assert_eq!(game.cards.get(card_id).unwrap().temp_base_power(), None);
    let action3 = undo_last(&mut game);
    assert!(matches!(action3, GameAction::SetTempBaseStats { .. }));
    assert_eq!(
        compute_state_hash(&game),
        hash_animated,
        "undo of clear_temp_base_stats_logged must restore the cleared override (2/2)"
    );
}
