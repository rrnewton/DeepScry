//! Unit tests for the ExtraLandPlay mechanic (AdjustLandPlays$ N).
//!
//! Covers both the static-ability path (Oracle of Mul Daya — permanent grant)
//! and the effect path (Explore — temporary end-of-turn grant via
//! `Effect::ExtraLandPlay`).

use mtg_engine::core::{Card, CardId, CardType, Effect, PlayerId, StaticAbility};
use mtg_engine::game::GameState;
use mtg_engine::zones::Zone;
use smallvec::SmallVec;

/// Place a card in a zone (helper).
///
/// Only Battlefield and Hand are needed by these tests.
fn put_card_in_zone(game: &mut GameState, card_id: CardId, zone: Zone, player_id: PlayerId) {
    match zone {
        Zone::Battlefield => {
            game.battlefield.cards.push(card_id);
        }
        Zone::Hand => {
            if let Some(zones) = game.get_player_zones_mut(player_id) {
                zones.hand.add(card_id);
            }
        }
        Zone::Library | Zone::Graveyard | Zone::Exile | Zone::Stack | Zone::Command => {
            // Not needed for these tests.
        }
    }
}

/// Create a card that has `StaticAbility::ExtraLandPlay { amount: N }`.
/// Simulates Oracle of Mul Daya (amount=1) or Azusa (amount=2) on the battlefield.
fn create_extra_land_permanent(game: &mut GameState, owner: PlayerId, amount: u8) -> CardId {
    let id = game.next_card_id();
    let mut card = Card::new(id, "Oracle of Mul Daya", owner);
    card.set_types(SmallVec::from_vec(vec![CardType::Creature]));
    card.controller = owner;
    card.static_abilities.push(StaticAbility::ExtraLandPlay {
        amount,
        description: "You may play an additional land on each of your turns.".to_string(),
    });
    game.cards.insert(id, card);
    id
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests: effective_max_lands() and can_play_land_effective()
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_effective_max_lands_default_is_one() {
    let game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1 = game.players[0].id;

    // No ExtraLandPlay statics, no persistent effects → max is 1
    assert_eq!(game.effective_max_lands(p1), 1);
}

#[test]
fn test_effective_max_lands_oracle_adds_one() {
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1 = game.players[0].id;

    let oracle_id = create_extra_land_permanent(&mut game, p1, 1);
    put_card_in_zone(&mut game, oracle_id, Zone::Battlefield, p1);

    assert_eq!(game.effective_max_lands(p1), 2);
}

#[test]
fn test_effective_max_lands_azusa_adds_two() {
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1 = game.players[0].id;

    let azusa_id = create_extra_land_permanent(&mut game, p1, 2);
    put_card_in_zone(&mut game, azusa_id, Zone::Battlefield, p1);

    assert_eq!(game.effective_max_lands(p1), 3);
}

#[test]
fn test_effective_max_lands_two_oracle_effects_stack() {
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1 = game.players[0].id;

    // Two copies of a +1 land-play card
    let id1 = create_extra_land_permanent(&mut game, p1, 1);
    let id2 = create_extra_land_permanent(&mut game, p1, 1);
    put_card_in_zone(&mut game, id1, Zone::Battlefield, p1);
    put_card_in_zone(&mut game, id2, Zone::Battlefield, p1);

    assert_eq!(game.effective_max_lands(p1), 3);
}

#[test]
fn test_oracle_only_counts_for_its_controller() {
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1 = game.players[0].id;
    let p2 = game.players[1].id;

    // P2 controls the Oracle
    let oracle_id = create_extra_land_permanent(&mut game, p2, 1);
    put_card_in_zone(&mut game, oracle_id, Zone::Battlefield, p2);

    // P1's limit is still 1
    assert_eq!(game.effective_max_lands(p1), 1);
    // P2's limit is 2
    assert_eq!(game.effective_max_lands(p2), 2);
}

#[test]
fn test_can_play_land_effective_respects_extra_grant() {
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1 = game.players[0].id;

    let oracle_id = create_extra_land_permanent(&mut game, p1, 1);
    put_card_in_zone(&mut game, oracle_id, Zone::Battlefield, p1);

    // Initially P1 has played 0 lands; limit is 2 → can play
    assert!(game.can_play_land_effective(p1));

    // Simulate having played one land (lands_played_this_turn = 1)
    game.get_player_mut(p1).unwrap().lands_played_this_turn = 1;
    assert!(game.can_play_land_effective(p1)); // still can play (limit = 2)

    // Simulate having played two lands (lands_played_this_turn = 2)
    game.get_player_mut(p1).unwrap().lands_played_this_turn = 2;
    assert!(!game.can_play_land_effective(p1)); // cannot play (limit = 2)
}

#[test]
fn test_can_play_land_effective_without_extra_grant() {
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1 = game.players[0].id;

    // 0 lands played → can play (limit = 1)
    assert!(game.can_play_land_effective(p1));

    // 1 land played → cannot play (limit = 1)
    game.get_player_mut(p1).unwrap().lands_played_this_turn = 1;
    assert!(!game.can_play_land_effective(p1));
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests: Effect::ExtraLandPlay execution via execute_effect (Explore path)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_execute_extra_land_play_temporary_grant() {
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1 = game.players[0].id;

    // Before any Effect: limit = 1
    assert_eq!(game.effective_max_lands(p1), 1);

    // Execute Effect::ExtraLandPlay (Explore path)
    game.execute_effect(&Effect::ExtraLandPlay { player: p1, amount: 1 })
        .expect("execute_effect(ExtraLandPlay) failed");

    // After the effect: limit = 2 (temporary persistent effect added)
    assert_eq!(game.effective_max_lands(p1), 2);
}

#[test]
fn test_extra_land_play_persistent_effect_stacks_with_static() {
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1 = game.players[0].id;

    // Oracle on battlefield (+1 static)
    let oracle_id = create_extra_land_permanent(&mut game, p1, 1);
    put_card_in_zone(&mut game, oracle_id, Zone::Battlefield, p1);

    // Plus Explore effect (+1 temporary)
    game.execute_effect(&Effect::ExtraLandPlay { player: p1, amount: 1 })
        .expect("execute_effect(ExtraLandPlay) failed");

    // Static + temporary = +2 → limit is 3
    assert_eq!(game.effective_max_lands(p1), 3);
}

#[test]
fn test_execute_extra_land_play_zero_amount_noop() {
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1 = game.players[0].id;

    // amount=0 should be a no-op
    game.execute_effect(&Effect::ExtraLandPlay { player: p1, amount: 0 })
        .expect("execute_effect(ExtraLandPlay amount=0) failed");

    assert_eq!(game.effective_max_lands(p1), 1);
}
