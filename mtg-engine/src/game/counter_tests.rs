//! Integration tests for counter mechanics

use crate::core::{Card, CardType, CounterType, Effect};
use crate::game::GameState;

#[test]
fn test_put_counter_effect() {
    let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create a creature on the battlefield
    let creature_id = game.next_card_id();
    let mut creature = Card::new(creature_id, "Test Creature", p1_id);
    creature.add_type(CardType::Creature);
    creature.set_base_power(Some(2));
    creature.set_base_toughness(Some(2));
    game.cards.insert(creature_id, creature);
    game.battlefield.add(creature_id);

    // Execute a PutCounter effect
    let effect = Effect::PutCounter {
        target: creature_id,
        counter_type: CounterType::P1P1,
        amount: 3,
    };

    game.execute_effect(&effect).unwrap();

    // Verify counter was added
    let card = game.cards.get(creature_id).unwrap();
    assert_eq!(card.get_counter(CounterType::P1P1), 3);
    assert_eq!(card.current_power(), 5); // 2 base + 3 counters
    assert_eq!(card.current_toughness(), 5);
}

#[test]
fn test_remove_counter_effect() {
    let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create a creature with counters
    let creature_id = game.next_card_id();
    let mut creature = Card::new(creature_id, "Test Creature", p1_id);
    creature.add_type(CardType::Creature);
    creature.set_base_power(Some(2));
    creature.set_base_toughness(Some(2));
    creature.add_counter(CounterType::P1P1, 5);
    game.cards.insert(creature_id, creature);
    game.battlefield.add(creature_id);

    // Execute a RemoveCounter effect
    let effect = Effect::RemoveCounter {
        target: creature_id,
        counter_type: Some(CounterType::P1P1),
        amount: 2,
    };

    game.execute_effect(&effect).unwrap();

    // Verify counter was removed
    let card = game.cards.get(creature_id).unwrap();
    assert_eq!(card.get_counter(CounterType::P1P1), 3);
    assert_eq!(card.current_power(), 5); // 2 base + 3 counters
}

#[test]
fn test_counter_undo() {
    let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create a creature on the battlefield
    let creature_id = game.next_card_id();
    let mut creature = Card::new(creature_id, "Test Creature", p1_id);
    creature.add_type(CardType::Creature);
    creature.set_base_power(Some(2));
    creature.set_base_toughness(Some(2));
    game.cards.insert(creature_id, creature);
    game.battlefield.add(creature_id);

    // Add counters using game state method (which logs for undo)
    game.add_counters(creature_id, CounterType::P1P1, 3).unwrap();

    // Verify counters were added
    {
        let card = game.cards.get(creature_id).unwrap();
        assert_eq!(card.get_counter(CounterType::P1P1), 3);
    }

    // Undo the counter addition
    game.undo().unwrap();

    // Verify counters were removed
    {
        let card = game.cards.get(creature_id).unwrap();
        assert_eq!(card.get_counter(CounterType::P1P1), 0);
    }
}

#[test]
fn test_counter_annihilation_through_effects() {
    let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create a creature on the battlefield
    let creature_id = game.next_card_id();
    let mut creature = Card::new(creature_id, "Test Creature", p1_id);
    creature.add_type(CardType::Creature);
    creature.set_base_power(Some(2));
    creature.set_base_toughness(Some(2));
    game.cards.insert(creature_id, creature);
    game.battlefield.add(creature_id);

    // Add +1/+1 counters
    let effect1 = Effect::PutCounter {
        target: creature_id,
        counter_type: CounterType::P1P1,
        amount: 5,
    };
    game.execute_effect(&effect1).unwrap();

    // Add -1/-1 counters - should annihilate
    let effect2 = Effect::PutCounter {
        target: creature_id,
        counter_type: CounterType::M1M1,
        amount: 3,
    };
    game.execute_effect(&effect2).unwrap();

    // Verify annihilation occurred
    let card = game.cards.get(creature_id).unwrap();
    assert_eq!(card.get_counter(CounterType::P1P1), 2); // 5 - 3 = 2
    assert_eq!(card.get_counter(CounterType::M1M1), 0); // annihilated
    assert_eq!(card.current_power(), 4); // 2 base + 2 counters
    assert_eq!(card.current_toughness(), 4);
}

#[test]
fn test_multiple_counter_types() {
    let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create a permanent on the battlefield
    let permanent_id = game.next_card_id();
    let mut permanent = Card::new(permanent_id, "Test Artifact", p1_id);
    permanent.add_type(CardType::Artifact);
    game.cards.insert(permanent_id, permanent);
    game.battlefield.add(permanent_id);

    // Add different types of counters
    game.add_counters(permanent_id, CounterType::Charge, 3).unwrap();
    game.add_counters(permanent_id, CounterType::P1P1, 2).unwrap();
    game.add_counters(permanent_id, CounterType::Age, 1).unwrap();

    // Verify all counters exist
    let card = game.cards.get(permanent_id).unwrap();
    assert_eq!(card.get_counter(CounterType::Charge), 3);
    assert_eq!(card.get_counter(CounterType::P1P1), 2);
    assert_eq!(card.get_counter(CounterType::Age), 1);
}

#[test]
fn test_remove_counter_undo() {
    let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create a creature with counters
    let creature_id = game.next_card_id();
    let mut creature = Card::new(creature_id, "Test Creature", p1_id);
    creature.add_type(CardType::Creature);
    creature.add_counter(CounterType::P1P1, 5);
    game.cards.insert(creature_id, creature);
    game.battlefield.add(creature_id);

    // Remove counters using game state method
    game.remove_counters(creature_id, CounterType::P1P1, 2).unwrap();

    // Verify counters were removed
    {
        let card = game.cards.get(creature_id).unwrap();
        assert_eq!(card.get_counter(CounterType::P1P1), 3);
    }

    // Undo the counter removal
    game.undo().unwrap();

    // Verify counters were restored
    {
        let card = game.cards.get(creature_id).unwrap();
        assert_eq!(card.get_counter(CounterType::P1P1), 5);
    }
}

/// mtg-ey2vf: after the undo consolidation, `GameState::undo` delegates the
/// per-variant reversal to the canonical `GameAction::undo`. The canonical
/// AddCounter/RemoveCounter arms were switched to direct `card.add_counter` /
/// `card.remove_counter` mutators precisely because the previous
/// `game.add_counters` / `game.remove_counters` calls LOG a fresh GameAction —
/// which would APPEND a spurious action to the live undo log on every undo,
/// corrupting it. This pins that the per-action undo path leaves the log
/// UNTOUCHED apart from the single popped action (no re-logging).
#[test]
fn undo_counter_does_not_corrupt_log() {
    let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
    let p1_id = game.players[0].id;

    let creature_id = game.next_card_id();
    let mut creature = Card::new(creature_id, "Test Creature", p1_id);
    creature.add_type(CardType::Creature);
    game.cards.insert(creature_id, creature);
    game.battlefield.add(creature_id);

    let log_len_baseline = game.undo_log.len();

    // Add then remove counters — each logs exactly one action.
    game.add_counters(creature_id, CounterType::P1P1, 3).unwrap();
    game.remove_counters(creature_id, CounterType::P1P1, 1).unwrap();
    assert_eq!(
        game.undo_log.len(),
        log_len_baseline + 2,
        "add + remove should log exactly two actions"
    );

    // Each undo must SHRINK the log by exactly one — never append a spurious
    // re-logged action (the bug a naive delegation to game.add/remove_counters
    // would reintroduce).
    game.undo().unwrap();
    assert_eq!(
        game.undo_log.len(),
        log_len_baseline + 1,
        "undoing RemoveCounter must pop exactly one action and append none"
    );
    game.undo().unwrap();
    assert_eq!(
        game.undo_log.len(),
        log_len_baseline,
        "undoing AddCounter must pop exactly one action and append none"
    );

    // And the state round-tripped (no counters left).
    let card = game.cards.get(creature_id).unwrap();
    assert_eq!(card.get_counter(CounterType::P1P1), 0);
}

/// mtg-ba6uq #4: undoing an `add_counters` that triggered +1/+1 ⟷ -1/-1
/// annihilation must restore the ANNIHILATED counters.
///
/// Adding one -1/-1 to a card holding +1/+1 counters cancels both (CR 122.3).
/// The old `AddCounter { type, amount }` log reversed only the added type via
/// `remove_counter`, so the annihilated +1/+1 counters were lost permanently on
/// undo. The fix logs a full pre-state snapshot (`SetCardCounters`) whose undo
/// restores the exact prior counter set.
///
/// NEGATIVE-TEST-PROVEN: with `add_counters` reverted to log `AddCounter`, the
/// post-undo P1P1 assertion (and the hash round-trip) FAIL; with the snapshot
/// fix they PASS.
#[test]
fn undo_add_counters_restores_annihilated_counters() {
    use crate::game::compute_undo_test_hash;

    let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
    let p1_id = game.players[0].id;

    let creature_id = game.next_card_id();
    let mut creature = Card::new(creature_id, "Test Creature", p1_id);
    creature.add_type(CardType::Creature);
    creature.set_base_power(Some(2));
    creature.set_base_toughness(Some(2));
    game.cards.insert(creature_id, creature);
    game.battlefield.add(creature_id);

    // Put five +1/+1 counters on (logged).
    game.add_counters(creature_id, CounterType::P1P1, 5).unwrap();
    assert_eq!(game.cards.get(creature_id).unwrap().get_counter(CounterType::P1P1), 5);

    // Snapshot the state with the five +1/+1 counters present.
    let hash_before = compute_undo_test_hash(&game);
    let log_len_before = game.undo_log.len();

    // Add three -1/-1 counters → annihilation removes 3 of each, leaving P1P1:2.
    game.add_counters(creature_id, CounterType::M1M1, 3).unwrap();
    {
        let card = game.cards.get(creature_id).unwrap();
        assert_eq!(
            card.get_counter(CounterType::P1P1),
            2,
            "annihilation: 5 - 3 = 2 P1P1 remain"
        );
        assert_eq!(
            card.get_counter(CounterType::M1M1),
            0,
            "annihilation: all M1M1 cancelled"
        );
    }
    assert_eq!(
        game.undo_log.len(),
        log_len_before + 1,
        "the annihilating add must log exactly one action"
    );

    // Undo the annihilating add.
    game.undo().unwrap();

    // The five +1/+1 counters must be FULLY restored (this is the hole: a
    // per-type AddCounter reversal would leave P1P1 stuck at 2).
    {
        let card = game.cards.get(creature_id).unwrap();
        assert_eq!(
            card.get_counter(CounterType::P1P1),
            5,
            "mtg-ba6uq #4: annihilated +1/+1 counters must be restored after undo"
        );
        assert_eq!(card.get_counter(CounterType::M1M1), 0, "no -1/-1 counters after undo");
    }
    assert_eq!(
        compute_undo_test_hash(&game),
        hash_before,
        "mtg-ba6uq #4: UndoTest hash must round-trip exactly after undoing an annihilating add_counters"
    );
}
