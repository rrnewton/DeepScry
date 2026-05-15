// TODO(mtg-0et0f): Remove once wildcard patterns are audited
#![allow(clippy::wildcard_enum_match_arm)]

//! Regression test for `mtg-c54e90` — Seismic Sense network desync.
//!
//! ## The bug
//!
//! Fuzz-testing the network code on integration (`fe820468`) found that
//! every state-hash desync in 45 native↔native runs occurred immediately
//! after a Seismic Sense resolution: 13/45 (29%) FATAL P2 hash mismatches,
//! all citing Seismic Sense as the last spell to resolve before divergence.
//!
//! Seismic Sense is `SP$ Dig | DigNum$ X | ChangeNum$ 1 | Optional$ True |
//! ForceRevealToController$ True | ChangeValid$ Creature,Land |
//! RestRandomOrder$ True` — i.e. "look at top X cards of your library, you
//! may put a creature/land into your hand, put the rest on bottom in
//! random order".
//!
//! In the legacy `Effect::Dig` execute_effect path, the `ChangeValid$`
//! filter was applied *before* any reveal/sync. On the network shadow
//! client the top-of-library CardIds are not yet materialized in the
//! EntityStore, so `try_get(card_id).is_some_and(matches)` returned false
//! for every candidate. The client therefore selected zero cards and
//! moved the whole top-N pile to the bottom of the library, while the
//! server moved one card to hand. Hands and libraries diverged → FATAL
//! state-hash mismatch.
//!
//! ## What this test pins
//!
//! 1. Single-player gameplay correctness: in the local game, casting
//!    Seismic Sense with a creature/land on top of library moves THAT
//!    card to hand (not to the bottom). Pre-fix, the local game already
//!    worked because the server-side filter saw real card identities, so
//!    this is mostly a cross-check.
//! 2. **The new ChoicePoint(LibrarySearch) emission**: routing the dig
//!    pick through `choose_from_library_with_hook` (in
//!    `priority.rs::resolve_top_spell_with_discard_hook`) emits a
//!    `ChoicePoint { choice: Some(ReplayChoice::LibrarySearch(_)), .. }`
//!    in the undo log, mirroring the Bazaar discard fix (mtg-cb67465c).
//!    The ChoicePoint is what bundles the server's CardRevealed messages
//!    with a synchronous request/response on the network — without it
//!    the shadow client never learns the top-N identities and desyncs.
//!
//! Network-mode coverage is provided by
//! `tests/network_vs_local_equivalence_e2e.sh 2 heuristic heuristic`,
//! which deterministically reproduces the original gabriel-vs-ryan game
//! that triggered this desync.

use mtg_forge_rs::{
    game::{replay_controller::ReplayChoice, GameLoop, HeuristicController, VerbosityLevel},
    loader::{require_cardsfolder, AsyncCardDatabase as CardDatabase},
    puzzle::{loader::load_puzzle_into_game, PuzzleFile},
    undo::GameAction,
    zones::Zone,
    Result,
};
use std::path::PathBuf;

/// Casting Seismic Sense from the puzzle's prepared state must:
/// 1. Move the top creature/land of the digger's library to the digger's hand.
/// 2. Emit a `ChoicePoint(LibrarySearch)` for snapshot/replay determinism.
///
/// Both invariants are required for the network shadow client to stay in
/// sync with the server (see mtg-c54e90 for full analysis).
#[tokio::test]
async fn test_seismic_sense_dig_self_records_choice_point() -> Result<()> {
    let cardsfolder = require_cardsfolder();
    let puzzle_path = PathBuf::from("../test_puzzles/seismic_sense_dig_self.pzl");
    let puzzle_contents = std::fs::read_to_string(&puzzle_path)?;
    let puzzle = PuzzleFile::parse(&puzzle_contents)?;

    let card_db = CardDatabase::new(cardsfolder);
    let mut game = load_puzzle_into_game(&puzzle, &card_db).await?;
    game.seed_rng(2);

    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;

    // Resolve the puzzle's "Ostrich-Horse" CardId now (before play) so we
    // can later confirm it ended up in hand and not at the bottom of the
    // library.
    let ostrich_horse_before: Option<_> =
        game.player_zones
            .iter()
            .find(|(id, _)| *id == p1_id)
            .and_then(|(_, zones)| {
                zones.library.cards.iter().copied().find(|&cid| {
                    game.cards
                        .try_get(cid)
                        .is_some_and(|c| c.name.as_str() == "Ostrich-Horse")
                })
            });
    let ostrich_horse = ostrich_horse_before.expect("puzzle should set up Ostrich-Horse in p0 library");

    let mut p1 = HeuristicController::new(p1_id);
    let mut p2 = HeuristicController::new(p2_id);

    {
        let mut game_loop = GameLoop::new(&mut game)
            .with_verbosity(VerbosityLevel::Silent)
            .with_max_turns(1);
        let _ = game_loop.run_game(&mut p1, &mut p2)?;
    }

    // 1) Ostrich-Horse should have ended up in p0's hand (the dig kept it),
    //    NOT at the bottom of the library (which is what the buggy path did
    //    on the network shadow client).
    let in_hand = game
        .get_player_zones(p1_id)
        .is_some_and(|z| z.hand.contains(ostrich_horse));
    let in_library = game
        .get_player_zones(p1_id)
        .is_some_and(|z| z.library.contains(ostrich_horse));
    assert!(
        in_hand,
        "Seismic Sense should move Ostrich-Horse from top of library to hand. \
         Found it in hand={in_hand}, library={in_library}."
    );
    assert!(
        !in_library,
        "Ostrich-Horse must leave the library after Seismic Sense resolves. \
         If this fires, the dig effect put the card back on the bottom of \
         the library — exactly the legacy execute_effect failure mode that \
         caused mtg-c54e90."
    );

    // 2) Resolution must have emitted a ChoicePoint(LibrarySearch). The
    //    routing through choose_from_library_with_hook is what bundles
    //    CardRevealed messages with the server's ChoiceRequest so the
    //    network shadow client materializes the top-N CardIds before
    //    making its matching pick.
    let library_search_choice_points: Vec<&GameAction> = game
        .undo_log
        .actions()
        .iter()
        .filter(|a| {
            matches!(
                a,
                GameAction::ChoicePoint {
                    choice: Some(ReplayChoice::LibrarySearch(_)),
                    ..
                }
            )
        })
        .collect();

    assert!(
        !library_search_choice_points.is_empty(),
        "Seismic Sense resolution must record at least one \
         ChoicePoint(LibrarySearch). Without it the network coordinator \
         never bundles CardRevealed messages with the choice request, the \
         shadow client never learns the top-N library identities, and \
         the dig fizzles (mtg-c54e90)."
    );

    // 3) Sanity: the move was a Library->Hand move_card, not some other
    //    zone shuffle (e.g. accidental discard).
    let moved_to_hand: bool = game.undo_log.actions().iter().any(|a| {
        matches!(
            a,
            GameAction::MoveCard {
                card_id,
                from_zone: Zone::Library,
                to_zone: Zone::Hand,
                ..
            } if *card_id == ostrich_horse
        )
    });
    assert!(
        moved_to_hand,
        "Expected a Library->Hand MoveCard for Ostrich-Horse in the undo log."
    );

    Ok(())
}
