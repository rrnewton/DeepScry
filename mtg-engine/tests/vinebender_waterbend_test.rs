//! Regression tests for Foggy Swamp Vinebender's Waterbend activated ability.
//!
//! Bug: bug-vinebender-triple-activation
#![allow(clippy::wildcard_enum_match_arm)]
//!
//! Reported behaviour (game.html, eric vs gabriel avatar decks, seed 42):
//!   - P2 cast Foggy Swamp Vinebender (4/3) and was reported to immediately
//!     activate its "Waterbend 5: put a +1/+1 counter on this creature"
//!     ability three times in a row WITHOUT actually paying the cost.
//!   - The +1/+1 counters did not appear on the creature.
//!
//! Expected behaviour:
//!   1. Activation must require paying Waterbend 5 (5 mana from lands and/or
//!      tapped creatures/artifacts other than the source).
//!   2. After successful activation, a +1/+1 counter must be placed on the
//!      Vinebender (Defined$ Self resolves the placeholder target to the
//!      source card).
//!   3. The ability must NOT be reported as available when the controller
//!      cannot pay Waterbend 5.
//!   4. Once the activation cost has been paid (lands tapped), the ability
//!      must drop out of the available actions until the cards untap.
//!   5. The ability is "your-turn-only" (PlayerTurn$ True) — it must NOT be
//!      offered on the opponent's turn.
//!
//! These tests load the actual cardsfolder/f/foggy_swamp_vinebender.txt so a
//! parser regression also fails them.

use mtg_forge_rs::core::{CardId, CounterType, PlayerId, SpellAbility};
use mtg_forge_rs::game::{GameLoop, GameState, VerbosityLevel};
use mtg_forge_rs::loader::CardLoader;
use std::path::PathBuf;

/// Load a card from `cardsfolder/<dir>/<file>.txt`, instantiate it on the
/// battlefield for `owner`, and return its CardId.
fn put_card_on_battlefield(game: &mut GameState, name: &str, dir: &str, owner: PlayerId) -> CardId {
    let path = PathBuf::from(format!("../cardsfolder/{}/{}", dir, file_name_for(name)));
    let def = CardLoader::load_from_file(&path).unwrap_or_else(|e| {
        panic!("failed to load {}: {}", path.display(), e);
    });
    let card_id = game.next_card_id();
    let mut card = def.instantiate(card_id, owner);
    card.controller = owner;
    // Ensure the card is on the battlefield from a previous turn so it has no
    // summoning sickness for tap-cost abilities. Vinebender's Waterbend cost
    // is not Tap, so summoning sickness does not block it, but lands we add
    // alongside need to be old too.
    card.turn_entered_battlefield = Some(0);
    game.cards.insert(card_id, card);
    game.battlefield.add(card_id);
    card_id
}

fn file_name_for(card_name: &str) -> String {
    // cardsfolder uses lowercase + underscores
    let mut s = String::with_capacity(card_name.len() + 4);
    for ch in card_name.chars() {
        if ch == ' ' {
            s.push('_');
        } else if ch.is_ascii_alphanumeric() {
            s.extend(ch.to_lowercase());
        }
    }
    s.push_str(".txt");
    s
}

/// Add a vanilla basic Forest to `owner` on the battlefield. Marks it as
/// having entered a previous turn so `T: Add G` is legal immediately.
fn add_forest(game: &mut GameState, owner: PlayerId) -> CardId {
    let id = game.next_card_id();
    // Use the actual basic Forest definition so the {T}: Add {G} ability gets
    // registered (instantiate() injects the implicit basic-land mana ability
    // when the subtype "Forest" is present).
    let path = PathBuf::from("../cardsfolder/f/forest.txt");
    let def = CardLoader::load_from_file(&path).expect("load Forest");
    let mut card = def.instantiate(id, owner);
    card.controller = owner;
    card.turn_entered_battlefield = Some(0);
    game.cards.insert(id, card);
    game.battlefield.add(id);
    id
}

/// Make `player` the active player and put us in main phase 1.
fn set_active(game: &mut GameState, player: PlayerId) {
    game.turn.active_player = player;
    game.turn.priority_player = Some(player);
    game.turn.current_step = mtg_forge_rs::game::Step::Main1;
    // Bump turn so summoning sickness checks pass for things we placed on turn 0.
    game.turn.turn_number = 2;
}

// --------------------------------------------------------------------------
// Tests
// --------------------------------------------------------------------------

/// With FIVE untapped Forests + the Vinebender on the battlefield, the
/// Waterbend 5 activated ability must show up as available on the
/// controller's own turn.
#[test]
fn vinebender_ability_available_when_cost_payable() {
    let mut game = GameState::new_two_player("Eric".to_string(), "Gabriel".to_string(), 20);
    let p2 = game.players[1].id;

    let vinebender = put_card_on_battlefield(&mut game, "Foggy Swamp Vinebender", "f", p2);
    for _ in 0..5 {
        add_forest(&mut game, p2);
    }
    set_active(&mut game, p2);

    let mut game_loop = GameLoop::new(&mut game).with_verbosity(VerbosityLevel::Silent);
    game_loop.push_activatable_abilities_for_test(p2);
    let abilities = game_loop.get_abilities_buffer();

    let waterbend = abilities.iter().any(|sa| match sa {
        SpellAbility::ActivateAbility {
            card_id,
            ability_index: _,
        } => *card_id == vinebender,
        _ => false,
    });

    assert!(
        waterbend,
        "Waterbend 5 ability MUST be in available actions when 5 untapped Forests \
         can pay the cost. Got: {:?}",
        abilities
    );
}

/// With only TWO Forests + the Vinebender, the controller cannot afford
/// Waterbend 5 (2 lands + 0 tappable non-source creatures < 5). The ability
/// must NOT appear in the available actions.
#[test]
fn vinebender_ability_unavailable_when_cost_unpayable() {
    let mut game = GameState::new_two_player("Eric".to_string(), "Gabriel".to_string(), 20);
    let p2 = game.players[1].id;

    let vinebender = put_card_on_battlefield(&mut game, "Foggy Swamp Vinebender", "f", p2);
    add_forest(&mut game, p2);
    add_forest(&mut game, p2);
    set_active(&mut game, p2);

    let mut game_loop = GameLoop::new(&mut game).with_verbosity(VerbosityLevel::Silent);
    game_loop.push_activatable_abilities_for_test(p2);
    let abilities = game_loop.get_abilities_buffer();

    let waterbend = abilities.iter().any(|sa| match sa {
        SpellAbility::ActivateAbility {
            card_id,
            ability_index: _,
        } => *card_id == vinebender,
        _ => false,
    });

    assert!(
        !waterbend,
        "Waterbend 5 ability MUST NOT be available with only 2 lands. Got: {:?}",
        abilities
    );
}

/// `PlayerTurn$ True` — the ability must NOT be offered on the opponent's
/// turn even when the cost could be paid.
#[test]
fn vinebender_ability_unavailable_on_opponents_turn() {
    let mut game = GameState::new_two_player("Eric".to_string(), "Gabriel".to_string(), 20);
    let p1 = game.players[0].id;
    let p2 = game.players[1].id;

    let vinebender = put_card_on_battlefield(&mut game, "Foggy Swamp Vinebender", "f", p2);
    for _ in 0..5 {
        add_forest(&mut game, p2);
    }
    set_active(&mut game, p1); // It's P1's turn — Vinebender belongs to P2

    let mut game_loop = GameLoop::new(&mut game).with_verbosity(VerbosityLevel::Silent);
    game_loop.push_activatable_abilities_for_test(p2);
    let abilities = game_loop.get_abilities_buffer();

    let waterbend = abilities.iter().any(|sa| match sa {
        SpellAbility::ActivateAbility {
            card_id,
            ability_index: _,
        } => *card_id == vinebender,
        _ => false,
    });

    assert!(
        !waterbend,
        "Waterbend 5 ability MUST NOT be available on opponent's turn (PlayerTurn$ True). \
         Got: {:?}",
        abilities
    );
}

/// Paying the cost via `pay_ability_cost` must actually tap five permanents
/// (lands, since none of the other permanents in this scenario are tappable).
#[test]
fn vinebender_paying_waterbend_taps_five_permanents() {
    let mut game = GameState::new_two_player("Eric".to_string(), "Gabriel".to_string(), 20);
    let p2 = game.players[1].id;

    let vinebender = put_card_on_battlefield(&mut game, "Foggy Swamp Vinebender", "f", p2);
    let mut forests = Vec::new();
    for _ in 0..5 {
        forests.push(add_forest(&mut game, p2));
    }
    set_active(&mut game, p2);

    // Use the Waterbend cost from the ability itself so the test reflects
    // exactly what the engine will pay when the ability is activated.
    let cost = {
        let card = game.cards.get(vinebender).expect("Vinebender exists");
        card.activated_abilities[0].cost.clone()
    };
    assert_eq!(cost.get_waterbend_amount(), Some(5), "expected Waterbend 5");

    game.pay_ability_cost(p2, vinebender, &cost)
        .expect("Waterbend 5 should be payable with 5 untapped Forests");

    let tapped_lands = forests
        .iter()
        .filter(|&&id| game.cards.get(id).map(|c| c.tapped).unwrap_or(false))
        .count();
    assert_eq!(
        tapped_lands, 5,
        "All 5 Forests must be tapped after paying Waterbend 5 (got {} tapped)",
        tapped_lands
    );

    // The Vinebender itself MUST NOT be tapped — the source can never be
    // used to pay its own Waterbend cost.
    let vine_tapped = game.cards.get(vinebender).unwrap().tapped;
    assert!(
        !vine_tapped,
        "Vinebender (source) must not tap to pay its own Waterbend cost"
    );
}

/// End-to-end: activating Vinebender's ability via the priority/game-loop
/// path must place a +1/+1 counter on the creature. This is the bug
/// reporter's primary symptom: counters did not appear after activation.
#[test]
fn vinebender_activation_places_p1p1_counter_on_self() {
    let mut game = GameState::new_two_player("Eric".to_string(), "Gabriel".to_string(), 20);
    let p2 = game.players[1].id;

    let vinebender = put_card_on_battlefield(&mut game, "Foggy Swamp Vinebender", "f", p2);
    for _ in 0..5 {
        add_forest(&mut game, p2);
    }
    set_active(&mut game, p2);

    // Sanity: starts with 0 +1/+1 counters
    assert_eq!(
        game.cards.get(vinebender).unwrap().get_counter(CounterType::P1P1),
        0,
        "Vinebender must start with no +1/+1 counters"
    );

    // Pay the cost the way the priority loop does
    let (cost, effects) = {
        let card = game.cards.get(vinebender).expect("Vinebender exists");
        let ab = &card.activated_abilities[0];
        (ab.cost.clone(), ab.effects.clone())
    };
    game.pay_ability_cost(p2, vinebender, &cost)
        .expect("Waterbend 5 payment must succeed");

    // Now resolve effects — emulate the SELF_TARGET fix-up the priority loop
    // does via `resolve_self_target` in actions/mod.rs (PutCounter
    // Defined$ Self → source). After commit cf19a07b, the parser encodes
    // `Defined$ Self` as `CardId::self_target()` (sentinel u32::MAX-3) rather
    // than the old id=0 placeholder, so we must match `is_self_target()` here.
    for effect in &effects {
        let fixed = match effect {
            mtg_forge_rs::core::Effect::PutCounter {
                target,
                counter_type,
                amount,
            } if target.is_self_target() => mtg_forge_rs::core::Effect::PutCounter {
                target: vinebender,
                counter_type: *counter_type,
                amount: *amount,
            },
            other => other.clone(),
        };
        game.execute_effect(&fixed).expect("PutCounter effect must execute");
    }

    let counters = game.cards.get(vinebender).unwrap().get_counter(CounterType::P1P1);
    assert_eq!(
        counters, 1,
        "Foggy Swamp Vinebender must have ONE +1/+1 counter after activating Waterbend 5. \
         Got {} counters — bug-vinebender-triple-activation regressed?",
        counters
    );

    // And its effective P/T must reflect the counter (4/3 + 1/1 = 5/4)
    let card = game.cards.get(vinebender).unwrap();
    assert_eq!(card.current_power(), 5, "Power must be 5 (4 base + 1 counter)");
    assert_eq!(card.current_toughness(), 4, "Toughness must be 4 (3 base + 1 counter)");
}

/// After paying Waterbend 5 once (which taps all 5 Forests), the ability
/// must drop out of `get_available_spell_abilities` — there are no more
/// untapped permanents to pay a second activation. The bug reporter saw
/// THREE activations in a row, which would only happen if cost payment is
/// not actually tapping permanents, OR if the cost check is bypassed.
#[test]
fn vinebender_ability_unavailable_after_paying_cost_once() {
    let mut game = GameState::new_two_player("Eric".to_string(), "Gabriel".to_string(), 20);
    let p2 = game.players[1].id;

    let vinebender = put_card_on_battlefield(&mut game, "Foggy Swamp Vinebender", "f", p2);
    for _ in 0..5 {
        add_forest(&mut game, p2);
    }
    set_active(&mut game, p2);

    let cost = game.cards.get(vinebender).unwrap().activated_abilities[0].cost.clone();
    game.pay_ability_cost(p2, vinebender, &cost)
        .expect("first Waterbend 5 payment must succeed");

    // Now check available actions: ability must NOT be in the list because
    // all 5 Forests are tapped and the only untapped permanent is the
    // Vinebender itself (which cannot pay its own Waterbend cost).
    let mut game_loop = GameLoop::new(&mut game).with_verbosity(VerbosityLevel::Silent);
    game_loop.push_activatable_abilities_for_test(p2);
    let abilities = game_loop.get_abilities_buffer();

    let waterbend = abilities.iter().any(|sa| match sa {
        SpellAbility::ActivateAbility {
            card_id,
            ability_index: _,
        } => *card_id == vinebender,
        _ => false,
    });

    assert!(
        !waterbend,
        "After paying Waterbend 5 once, the ability MUST drop from available actions \
         (all 5 lands tapped). Bug-vinebender-triple-activation: AI was activating it \
         3 times. Got: {:?}",
        abilities
    );
}
