// TODO(mtg-211): Remove once wildcard patterns are audited
#![allow(clippy::wildcard_enum_match_arm)]

//! Regression test for `bug-rewind-infinite-loop`.
//!
//! Reproduces the infinite rewind/replay loop on Bazaar of Baghdad
//! ("tap: draw 2, then discard 3").
//!
//! ## The bug
//!
//! When the user activated Bazaar and chose 3 cards to discard, the game
//! advanced — but on the very next user choice (e.g. declare attackers),
//! `WasmTuiState::run_until_choice` rewound the game to the start of the
//! turn and replayed every recorded choice. That replay rebuilt the world
//! up to but **not including** the discard pick, because the activated
//! ability's `Effect::DiscardCards` branch in `priority.rs` consumed the
//! choice but never emitted a `GameAction::ChoicePoint` into the undo log.
//! On replay the `ReplayController` therefore had no `ReplayChoice::Discard`
//! to consume, fell back to the inner `WasmHumanController` (which had no
//! pending choice), and emitted `NeedInput` for the discard again. The UI
//! interpreted this as "user must discard 3 cards" — a perfect infinite
//! loop.
//!
//! ## What this test does
//!
//! 1. Loads `bazaar_of_baghdad_draw_discard.pzl` (Bazaar in play, 5 Plains
//!    in hand, lots of Plains in library).
//! 2. Runs the game with a `BazaarBot` controller for P1 that:
//!    - On the first `choose_spell_ability_to_play`, activates Bazaar.
//!    - On `choose_cards_to_discard`, returns 3 cards from hand.
//!    - On every other call, passes / declines.
//! 3. After the activation resolves, **counts** the number of
//!    `ChoicePoint { choice: Some(ReplayChoice::Discard(_)), .. }` entries
//!    in the undo log.
//!
//! Before the fix this count was 0 (rewind/replay of any later choice
//! re-prompts for the discard). After the fix it is exactly 1.

use mtg_engine::{
    core::{CardId, ManaCost, PlayerId, SpellAbility},
    game::{
        controller::{ChoiceResult, GameStateView, PlayerController},
        replay_controller::ReplayChoice,
        snapshot::ControllerType,
        GameLoop, VerbosityLevel,
    },
    loader::{require_cardsfolder, AsyncCardDatabase as CardDatabase},
    puzzle::{loader::load_puzzle_into_game, PuzzleFile},
    undo::GameAction,
    Result,
};
use smallvec::SmallVec;
use std::path::PathBuf;

/// Controller that activates Bazaar of Baghdad, then passes everything else,
/// and returns 3 cards to discard whenever asked.
struct BazaarBot {
    player_id: PlayerId,
    /// Number of times `choose_cards_to_discard` was called.
    discard_calls: usize,
    /// Whether we already activated Bazaar at least once.
    activated_bazaar: bool,
}

impl BazaarBot {
    fn new(player_id: PlayerId) -> Self {
        Self {
            player_id,
            discard_calls: 0,
            activated_bazaar: false,
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
        // Try to activate Bazaar of Baghdad's tap ability once. After that, pass.
        if !self.activated_bazaar {
            for sa in available {
                if let SpellAbility::ActivateAbility { .. } = sa {
                    self.activated_bazaar = true;
                    return ChoiceResult::Ok(Some(sa.clone()));
                }
            }
        }
        ChoiceResult::Ok(None)
    }

    fn choose_targets(
        &mut self,
        _view: &GameStateView,
        _spell: CardId,
        _valid_targets: &[CardId],
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
        self.discard_calls += 1;
        // Discard the first `count` cards in hand. Bazaar requires 3 cards
        // and the puzzle starts with 5 Plains in hand + 2 drawn = 7.
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

/// Activated abilities that consume a discard choice via the controller MUST
/// log a `ChoicePoint { choice: Some(ReplayChoice::Discard(..)), .. }` so
/// rewind/replay can replay the discard deterministically.
///
/// Without this, the WASM TUI re-prompts the user for the discard every time
/// it rewinds for any later choice on the same turn → infinite loop.
#[tokio::test]
async fn test_bazaar_discard_logs_choice_point_for_replay() -> Result<()> {
    let cardsfolder = require_cardsfolder();
    let puzzle_path = PathBuf::from("../test_puzzles/bazaar_of_baghdad_draw_discard.pzl");
    let puzzle_contents = std::fs::read_to_string(&puzzle_path)?;
    let puzzle = PuzzleFile::parse(&puzzle_contents)?;

    let card_db = CardDatabase::new(cardsfolder);
    let mut game = load_puzzle_into_game(&puzzle, &card_db).await?;
    game.seed_rng(42);

    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;

    let mut p1 = BazaarBot::new(p1_id);
    let mut p2 = mtg_engine::game::ZeroController::new(p2_id);

    // One main phase is enough for Bazaar to be activated and resolve.
    {
        let mut game_loop = GameLoop::new(&mut game)
            .with_verbosity(VerbosityLevel::Silent)
            .with_max_turns(1);
        let _ = game_loop.run_game(&mut p1, &mut p2)?;
    }

    // The controller should have been asked to discard exactly once during the
    // Bazaar activation. (It may also be asked at cleanup if hand size > 7,
    // but with the puzzle's 5 Plains + 2 drawn = 7 cards, then discard 3 → 4
    // in hand at end of turn, so cleanup discard does NOT fire.)
    assert!(
        p1.discard_calls >= 1,
        "BazaarBot should have been asked to discard during the Bazaar activation"
    );

    // Check that the undo log recorded the discard choice as a ChoicePoint.
    let recorded_discards: Vec<&GameAction> = game
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
        .collect();

    assert_eq!(
        recorded_discards.len(),
        p1.discard_calls,
        "Each call to choose_cards_to_discard during a Bazaar activation must produce a ChoicePoint(Discard) \
         in the undo log so rewind/replay can replay it deterministically. \
         Without this, the WASM TUI infinite-loops on Bazaar of Baghdad. \
         Found {} recorded discards but the controller was asked {} times.",
        recorded_discards.len(),
        p1.discard_calls,
    );

    Ok(())
}
