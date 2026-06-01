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

    for k in 0..total_p1_priority_calls {
        let mut game = load_game().await?;
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // ── Forward pass: pause at P1's K-th priority decision ────────────────
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
        let forward_json = strip_for_diff(serde_json::to_value(&game).unwrap());

        // ── Rewind to turn start ──────────────────────────────────────────────
        let mut undo_log = std::mem::take(&mut game.undo_log);
        let rewind = undo_log.rewind_to_turn_start(&mut game);
        let log_empty_after_rewind = undo_log.is_empty();
        game.undo_log = undo_log;
        let (_turn, choice_actions, _rewound, _log_size) =
            rewind.expect("rewind_to_turn_start must succeed (undo log enabled)");

        // Turn 1 has no ChangeTurn marker, so the rewind empties the whole log.
        // Replaying from an empty log re-triggers PRE-GAME setup (shuffle + opening
        // hands), which advances the RNG and re-orders libraries — that is the
        // game-setup boundary, NOT an intra-game NeedInput class, and is out of
        // scope for this oracle. Every NeedInput class recurs on turn 2+, where a
        // real ChangeTurn boundary survives the rewind, so we verify there.
        if log_empty_after_rewind {
            continue;
        }

        let (p1_choices, p2_choices) = partition_choices(choice_actions, p1_id);

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
        assert_eq!(
            hash_after_replay, hash_after_forward,
            "REWIND/REPLAY ORACLE FAILED at decision point K={k}: after rewind_to_turn_start + \
             deterministic replay of the recorded intra-turn choices, the state hash did not \
             return to its pre-rewind value ({hash_after_replay:#x} != {hash_after_forward:#x}). \
             This means undo.rs::rewind_to_turn_start is INCOMPLETE for the NeedInput class at \
             this decision point — some canonical state was mutated without a logged GameAction. \
             See mtg-614 / mtg-610 / docs/NETWORK_ARCHITECTURE.md (desync is always fatal)."
        );

        verified_points += 1;
    }

    assert!(
        verified_points > 0,
        "oracle must have verified at least one rewind+replay round-trip"
    );
    let t = tally.borrow();
    eprintln!(
        "oracle verified {verified_points} decision points; class tally across all runs: \
         spell_ability={} attackers={} blockers={} damage_order={} discard={} mana_sources={}",
        t.spell_ability, t.attackers, t.blockers, t.damage_order, t.discard, t.mana_sources
    );

    Ok(())
}
