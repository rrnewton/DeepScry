//! Regression tests for blocker legality (`combat_rules::can_block`) and the
//! `validate_blocking_restrictions` engine pass.
//!
//! Bug history (mtg / bug-blockers-not-declared, May 2026):
//!   When the GUI presented a blocker option for a flying attacker that the
//!   defending player could not legally block (no flying/reach), selecting it
//!   silently produced no block — the engine's `validate_blocking_restrictions`
//!   dropped the assignment without telling the user. The fix factored the
//!   per-pair legality check into `combat_rules::can_block` so the GUI can
//!   filter the menu using the same predicate the engine uses to validate.
//!
//! These tests pin the predicate's behaviour for every per-pair evasion ability
//! so future changes can't silently regress the GUI/engine contract.

use mtg_forge_rs::core::{Card, CardType, Color, Keyword};
use mtg_forge_rs::game::{combat_rules, GameState};
use smallvec::SmallVec;

fn add_creature(
    game: &mut GameState,
    name: &str,
    owner: mtg_forge_rs::core::PlayerId,
    power: i8,
    toughness: i8,
    keywords: &[Keyword],
    colors: &[Color],
) -> mtg_forge_rs::core::CardId {
    let id = game.next_card_id();
    let mut c = Card::new(id, name, owner);
    c.set_types(SmallVec::from_vec(vec![CardType::Creature]));
    c.set_base_power(Some(power));
    c.set_base_toughness(Some(toughness));
    c.controller = owner;
    for kw in keywords {
        c.keywords.insert(*kw);
    }
    for col in colors {
        c.colors.push(*col);
    }
    game.cards.insert(id, c);
    game.battlefield.add(id);
    id
}

#[test]
fn flying_attacker_cant_be_blocked_by_ground_creature() {
    // The exact bug-blockers-not-declared scenario: Glider Kids (Flying 2/3)
    // attacks; defender has only ground creatures. The UI was offering the
    // ground creature as a blocker and the engine then silently dropped it.
    let mut game = GameState::new_two_player("Eric".to_string(), "Gabriel".to_string(), 20);
    let p1 = game.players[0].id;
    let p2 = game.players[1].id;

    let glider_kids = add_creature(&mut game, "Glider Kids", p2, 2, 3, &[Keyword::Flying], &[Color::White]);
    let ground = add_creature(&mut game, "Knowledge Seeker", p1, 2, 1, &[], &[Color::Blue]);

    assert!(
        !combat_rules::can_block(&game, glider_kids, ground),
        "ground creature must NOT be a legal blocker for a flying attacker (CR 702.9b)"
    );
}

#[test]
fn flying_attacker_can_be_blocked_by_flying_or_reach() {
    let mut game = GameState::new_two_player("Eric".to_string(), "Gabriel".to_string(), 20);
    let p1 = game.players[0].id;
    let p2 = game.players[1].id;

    let attacker = add_creature(&mut game, "Glider Kids", p2, 2, 3, &[Keyword::Flying], &[Color::White]);
    let flyer = add_creature(&mut game, "Air Bender", p1, 1, 1, &[Keyword::Flying], &[Color::Blue]);
    let reacher = add_creature(&mut game, "Spider", p1, 1, 2, &[Keyword::Reach], &[Color::Green]);

    assert!(
        combat_rules::can_block(&game, attacker, flyer),
        "flying creature must be a legal blocker for a flying attacker"
    );
    assert!(
        combat_rules::can_block(&game, attacker, reacher),
        "reach creature must be a legal blocker for a flying attacker"
    );
}

#[test]
fn tapped_creature_cant_block() {
    let mut game = GameState::new_two_player("p1".to_string(), "p2".to_string(), 20);
    let p1 = game.players[0].id;
    let p2 = game.players[1].id;

    let attacker = add_creature(&mut game, "Bear", p2, 2, 2, &[], &[Color::Green]);
    let blocker = add_creature(&mut game, "Wall", p1, 0, 4, &[], &[Color::White]);
    // Tap the blocker
    game.cards.get_mut(blocker).unwrap().tapped = true;

    assert!(
        !combat_rules::can_block(&game, attacker, blocker),
        "tapped creature must NOT be a legal blocker (CR 509.1a)"
    );
}

#[test]
fn fear_only_blocked_by_artifact_or_black() {
    let mut game = GameState::new_two_player("p1".to_string(), "p2".to_string(), 20);
    let p1 = game.players[0].id;
    let p2 = game.players[1].id;

    let attacker = add_creature(&mut game, "Fearmonger", p2, 2, 2, &[Keyword::Fear], &[Color::Black]);
    let white = add_creature(&mut game, "WhiteG", p1, 2, 2, &[], &[Color::White]);
    let black = add_creature(&mut game, "BlackG", p1, 2, 2, &[], &[Color::Black]);
    let artifact_id = game.next_card_id();
    let mut artifact = Card::new(artifact_id, "Golem", p1);
    artifact.set_types(SmallVec::from_vec(vec![CardType::Creature, CardType::Artifact]));
    artifact.set_base_power(Some(2));
    artifact.set_base_toughness(Some(2));
    artifact.controller = p1;
    game.cards.insert(artifact_id, artifact);
    game.battlefield.add(artifact_id);

    assert!(
        !combat_rules::can_block(&game, attacker, white),
        "white can't block Fear"
    );
    assert!(combat_rules::can_block(&game, attacker, black), "black can block Fear");
    assert!(
        combat_rules::can_block(&game, attacker, artifact_id),
        "artifact can block Fear"
    );
}

#[test]
fn protection_blocks_blocker() {
    let mut game = GameState::new_two_player("p1".to_string(), "p2".to_string(), 20);
    let p1 = game.players[0].id;
    let p2 = game.players[1].id;

    let attacker = add_creature(
        &mut game,
        "WhiteKnight",
        p2,
        2,
        2,
        &[Keyword::ProtectionFromRed],
        &[Color::White],
    );

    let red = add_creature(&mut game, "Goblin", p1, 2, 2, &[], &[Color::Red]);
    let blue = add_creature(&mut game, "Merfolk", p1, 2, 2, &[], &[Color::Blue]);

    assert!(
        !combat_rules::can_block(&game, attacker, red),
        "creature can't block one with protection from its color (CR 702.16)"
    );
    assert!(
        combat_rules::can_block(&game, attacker, blue),
        "different-color creature can block normally"
    );
}

#[test]
fn vanilla_creatures_can_block() {
    let mut game = GameState::new_two_player("p1".to_string(), "p2".to_string(), 20);
    let p1 = game.players[0].id;
    let p2 = game.players[1].id;

    let attacker = add_creature(&mut game, "Bear", p2, 2, 2, &[], &[Color::Green]);
    let blocker = add_creature(&mut game, "Squire", p1, 1, 2, &[], &[Color::White]);

    assert!(
        combat_rules::can_block(&game, attacker, blocker),
        "vanilla creature must be a legal blocker for a vanilla attacker"
    );
}
