// TODO(mtg-211): Remove once wildcard patterns are audited
#![allow(clippy::wildcard_enum_match_arm)]

//! Rewind + replay state-hash round-trip invariant (mtg-610 / mtg-559 / mtg-53okw).
//!
//! ## What this proves
//!
//! The network re-architecture's central correctness claim is that the WASM /
//! network blocking model can be implemented by **rewind-to-checkpoint +
//! replay-forward** (re-evolving internal game state, suppressing only external
//! side-effects) — the SAME mechanism the snapshot/resume path already uses.
//! For that to be sound, the undo log must be COMPLETE: after a
//! rewind-to-turn-start followed by a deterministic replay of the recorded
//! choices, the game-state hash must return EXACTLY to the value it had before
//! the rewind.
//!
//! `mtg-610` left one question explicitly open:
//!
//! > PREREQUISITE TO VERIFY: is `undo.rs::rewind_to_turn_start` COMPLETE for
//! > arbitrary MID-RESOLUTION / in-stack state? mtg-559 suggests NOT (spell
//! > resolution re-entry re-applies effects).
//!
//! This test answers it for the concrete in-stack-resolution category that
//! mtg-559 (robots42 Copy Artifact) belongs to: a choice made **while an
//! ability is mid-resolution on the stack**. Bazaar of Baghdad's activated
//! ability draws 2 then forces a discard of 3 *during its own resolution*, so
//! the discard pick is an in-resolution / in-stack choice — exactly the shape
//! that re-run-without-rewind double-applies. We:
//!
//!   1. Play forward one turn with a deterministic bot that activates Bazaar
//!      and discards mid-resolution (this is recorded in the undo log as a
//!      `ChoicePoint { choice: Some(Discard(..)), .. }`).
//!   2. Snapshot the post-forward state hash.
//!   3. `rewind_to_turn_start`, extracting the recorded intra-turn choices.
//!   4. Replay BOTH players' recorded choices via `ReplayController`.
//!   5. Assert the post-replay state hash EQUALS the post-forward hash.
//!
//! A mismatch would mean rewind+replay is NOT a pure round-trip across
//! mid-resolution state — i.e. the undo log is incomplete and the network
//! resume model would desync. Per `docs/NETWORK_ARCHITECTURE.md`, desync is a
//! fatal error; this test makes that a compile-time-enforced regression gate.
//!
//! ## Why an `agentplay`-shaped controller rather than `RandomController`
//!
//! Deterministic replay requires the EXACT same choices the second time. The
//! existing `undo_e2e.rs` rewind test resets fresh `RandomController`s after the
//! rewind, so it can only check zone sizes (the RNG has advanced and the second
//! play diverges). Here we record the bot's choices into the undo log on the
//! forward pass and replay those exact `ReplayChoice`s, so the round-trip is
//! byte-exact and we can assert on the full state hash.

use mtg_engine::{
    core::{CardId, ManaCost, PlayerId, SpellAbility},
    game::{
        compute_state_hash,
        controller::{ChoiceContext, ChoiceResult, GameStateView, PlayerController},
        replay_controller::ReplayChoice,
        snapshot::ControllerType,
        GameLoop, ReplayController, VerbosityLevel, ZeroController,
    },
    loader::{require_cardsfolder, AsyncCardDatabase as CardDatabase},
    puzzle::{loader::load_puzzle_into_game, PuzzleFile},
    undo::GameAction,
    Result,
};
use smallvec::SmallVec;
use std::path::PathBuf;

/// Deterministic bot: activate Bazaar of Baghdad once, discard the first
/// `count` cards in hand when asked (the mid-resolution discard), pass / decline
/// everything else. Identical decisions every call so replay is exact.
///
/// After the Bazaar activation has resolved (i.e. the mid-resolution discard
/// has happened), the next priority request returns `NeedInput` so the forward
/// `run_until_input` pass STOPS mid-turn-2 — leaving real intra-turn state
/// (including the just-resolved in-stack discard) in the undo log for the
/// rewind/replay round-trip to exercise. This mirrors how a WASM human
/// controller pauses the loop.
struct BazaarBot {
    player_id: PlayerId,
    activated_bazaar: bool,
    discarded: bool,
}

impl BazaarBot {
    fn new(player_id: PlayerId) -> Self {
        Self {
            player_id,
            activated_bazaar: false,
            discarded: false,
        }
    }
}

impl PlayerController for BazaarBot {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn choose_spell_ability_to_play(
        &mut self,
        _view: &GameStateView,
        available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>> {
        if !self.activated_bazaar {
            for sa in available {
                if let SpellAbility::ActivateAbility { .. } = sa {
                    self.activated_bazaar = true;
                    return ChoiceResult::Ok(Some(sa.clone()));
                }
            }
        }
        // Once the Bazaar activation has fully resolved (discard done), pause the
        // loop mid-turn so the rewind below has genuine intra-turn state to undo.
        if self.discarded {
            return ChoiceResult::NeedInput(ChoiceContext::SpellAbility {
                available: available.to_vec(),
                formatted_choices: Vec::new(),
            });
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
        ChoiceResult::Ok(available_sources.iter().copied().collect())
    }

    fn choose_attackers(
        &mut self,
        _view: &GameStateView,
        _available_creatures: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        ChoiceResult::Ok(SmallVec::new())
    }

    fn choose_blockers(
        &mut self,
        _view: &GameStateView,
        _available_blockers: &[CardId],
        _attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
        ChoiceResult::Ok(SmallVec::new())
    }

    fn choose_damage_assignment_order(
        &mut self,
        _view: &GameStateView,
        _attacker: CardId,
        blockers: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        ChoiceResult::Ok(blockers.iter().copied().collect())
    }

    fn choose_cards_to_discard(
        &mut self,
        _view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        self.discarded = true;
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

/// Inner controller used UNDER the `ReplayController` on the replay pass: once
/// the recorded choices are exhausted it returns `NeedInput` on the next
/// priority request, so the replay stops at exactly the same mid-turn point the
/// forward pass stopped at. All other (non-priority) callbacks delegate to a
/// `ZeroController` so a stray request can't error; in practice the recorded
/// choice list ends at the priority point, so only `choose_spell_ability_to_play`
/// is reached past the replay frontier.
struct StopController {
    inner: ZeroController,
}

impl StopController {
    fn new(player_id: PlayerId) -> Self {
        Self {
            inner: ZeroController::new(player_id),
        }
    }
}

impl PlayerController for StopController {
    fn player_id(&self) -> PlayerId {
        self.inner.player_id()
    }

    fn choose_spell_ability_to_play(
        &mut self,
        _view: &GameStateView,
        available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>> {
        ChoiceResult::NeedInput(ChoiceContext::SpellAbility {
            available: available.to_vec(),
            formatted_choices: Vec::new(),
        })
    }

    fn choose_targets(
        &mut self,
        view: &GameStateView,
        spell: CardId,
        valid_targets: &[CardId],
        min_targets: usize,
        max_targets: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        self.inner
            .choose_targets(view, spell, valid_targets, min_targets, max_targets)
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        view: &GameStateView,
        cost: &ManaCost,
        available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        self.inner.choose_mana_sources_to_pay(view, cost, available_sources)
    }

    fn choose_attackers(
        &mut self,
        view: &GameStateView,
        available_creatures: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        self.inner.choose_attackers(view, available_creatures)
    }

    fn choose_blockers(
        &mut self,
        view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
        self.inner.choose_blockers(view, available_blockers, attackers)
    }

    fn choose_damage_assignment_order(
        &mut self,
        view: &GameStateView,
        attacker: CardId,
        blockers: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        self.inner.choose_damage_assignment_order(view, attacker, blockers)
    }

    fn choose_cards_to_discard(
        &mut self,
        view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        self.inner.choose_cards_to_discard(view, hand, count)
    }

    fn choose_from_library(
        &mut self,
        view: &GameStateView,
        valid_cards: &[&mtg_engine::loader::CardDefinition],
    ) -> ChoiceResult<Option<usize>> {
        self.inner.choose_from_library(view, valid_cards)
    }

    fn choose_permanents_to_sacrifice(
        &mut self,
        view: &GameStateView,
        valid_permanents: &[CardId],
        count: usize,
        card_type_description: &str,
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        self.inner
            .choose_permanents_to_sacrifice(view, valid_permanents, count, card_type_description)
    }

    fn choose_permanents_to_not_untap(
        &mut self,
        view: &GameStateView,
        may_not_untap_permanents: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        self.inner
            .choose_permanents_to_not_untap(view, may_not_untap_permanents)
    }

    fn choose_modes(
        &mut self,
        view: &GameStateView,
        spell_id: CardId,
        mode_descriptions: &[String],
        mode_count: usize,
        min_modes: usize,
        can_repeat: bool,
    ) -> ChoiceResult<SmallVec<[usize; 4]>> {
        self.inner
            .choose_modes(view, spell_id, mode_descriptions, mode_count, min_modes, can_repeat)
    }

    fn on_priority_passed(&mut self, _view: &GameStateView) {}
    fn on_game_end(&mut self, _view: &GameStateView, _won: bool) {}

    fn get_controller_type(&self) -> ControllerType {
        ControllerType::Tui
    }
}

/// Partition the intra-turn `ChoicePoint` actions returned by
/// `rewind_to_turn_start` into (our, opponent) `ReplayChoice` lists, mirroring
/// `WasmFancyTuiState::rewind_to_turn_start` (the production rewind/replay path).
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

/// THE invariant: rewind-to-turn-start + replay of the recorded choices returns
/// the game-state hash EXACTLY to its pre-rewind value, even when the turn
/// contained an in-stack / mid-resolution choice (Bazaar's resolution-time
/// discard — the same category as mtg-559 robots42).
#[tokio::test]
async fn rewind_then_replay_round_trips_state_hash_through_mid_resolution_choice() -> Result<()> {
    let cardsfolder = require_cardsfolder();
    let puzzle_path = PathBuf::from("../test_puzzles/bazaar_of_baghdad_draw_discard.pzl");
    let puzzle_contents = std::fs::read_to_string(&puzzle_path)?;
    let puzzle = PuzzleFile::parse(&puzzle_contents)?;

    let card_db = CardDatabase::new(cardsfolder);
    let mut game = load_puzzle_into_game(&puzzle, &card_db).await?;
    game.seed_rng(42);

    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;

    // ── Forward pass: activate Bazaar (mid-resolution discard), then pause ────
    // BazaarBot returns NeedInput right after the discard resolves, so the loop
    // stops mid-turn-2 with the in-stack-resolution effect freshly applied and
    // still "live" in the undo log (not wrapped up at a turn boundary).
    {
        let mut p1 = BazaarBot::new(p1_id);
        let mut p2 = ZeroController::new(p2_id);
        let mut game_loop = GameLoop::new(&mut game).with_verbosity(VerbosityLevel::Silent);
        let _ = game_loop.run_until_input(&mut p1, &mut p2)?;
    }

    // Sanity: the forward pass really did exercise a mid-resolution discard, so
    // this test is actually covering the in-stack category (not a vacuous pass).
    let discard_choicepoints = game
        .undo_log
        .actions()
        .iter()
        .filter(|a| {
            matches!(
                a,
                GameAction::ChoicePoint {
                    choice: Some(ReplayChoice::Discard(_)),
                    ..
                }
            )
        })
        .count();
    assert!(
        discard_choicepoints >= 1,
        "expected at least one mid-resolution Discard ChoicePoint from the Bazaar activation; \
         got {discard_choicepoints}. Without it this test would not cover in-stack resolution."
    );

    // Snapshot the post-forward state hash — this is the value the rewind+replay
    // round-trip must reproduce exactly.
    let hash_after_forward = compute_state_hash(&game);

    // ── Rewind to turn start, extracting the recorded intra-turn choices ──────
    let mut undo_log = std::mem::take(&mut game.undo_log);
    let rewind = undo_log.rewind_to_turn_start(&mut game);
    game.undo_log = undo_log;
    let (_turn, choice_actions, _rewound, _log_size) =
        rewind.expect("rewind_to_turn_start must succeed (undo log enabled)");

    // After rewinding, the hash MUST differ from the post-forward hash —
    // otherwise the "forward pass" did nothing and the round-trip is vacuous.
    let hash_at_turn_start = compute_state_hash(&game);
    assert_ne!(
        hash_at_turn_start, hash_after_forward,
        "rewind to turn start should have changed the state hash; if equal, the forward pass \
         applied no undoable state and the round-trip assertion below is vacuous"
    );

    let (p1_choices, p2_choices) = partition_choices(choice_actions, p1_id);

    // ── Replay BOTH players' recorded choices deterministically ───────────────
    // P1's inner controller is a StopController so that, once the recorded
    // choices are exhausted, it returns NeedInput at the SAME mid-turn priority
    // point the forward pass stopped at (rather than playing on and diverging).
    // P2 had no recorded choices this turn (ZeroController passes); its inner is
    // a plain ZeroController.
    let mut p1_replay = ReplayController::new(p1_id, Box::new(StopController::new(p1_id)), p1_choices);
    let mut p2_replay = ReplayController::new(p2_id, Box::new(ZeroController::new(p2_id)), p2_choices);

    {
        let mut game_loop = GameLoop::new(&mut game).with_verbosity(VerbosityLevel::Silent);
        let _ = game_loop.run_until_input(&mut p1_replay, &mut p2_replay)?;
    }

    // ── THE invariant assertion ───────────────────────────────────────────────
    let hash_after_replay = compute_state_hash(&game);
    assert_eq!(
        hash_after_replay, hash_after_forward,
        "REWIND/REPLAY ROUND-TRIP FAILED: after rewind_to_turn_start + deterministic replay of \
         the recorded intra-turn choices (including a mid-resolution Bazaar discard), the \
         state hash did not return to its pre-rewind value ({hash_after_replay:#x} != \
         {hash_after_forward:#x}). This means undo.rs::rewind_to_turn_start is INCOMPLETE for \
         in-stack / mid-resolution state — the network resume model (rewind+replay) would desync \
         here. See mtg-610 / mtg-559 / docs/NETWORK_ARCHITECTURE.md (desync is always fatal)."
    );

    Ok(())
}
