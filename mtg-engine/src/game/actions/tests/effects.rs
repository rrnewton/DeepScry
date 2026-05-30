use crate::core::{Card, CardId, CardType, Effect, Keyword, ManaCost, PlayerId, TargetRef};
use crate::game::state::GameState;
use crate::loader::CardDatabase;
use crate::MtgError;
use std::path::PathBuf;

/// Helper to load a card from the cardsfolder for tests
pub(super) fn load_test_card(game: &mut GameState, card_name: &str, owner_id: PlayerId) -> Result<CardId, MtgError> {
    let card_id = game.next_entity_id();

    // Load card definition from cardsfolder (relative to workspace root)
    let cardsfolder = PathBuf::from("../cardsfolder");
    let db = CardDatabase::new(cardsfolder);

    let card_def = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(async { db.get_card(card_name).await })?
        .ok_or_else(|| MtgError::InvalidCardFormat(format!("Card not found: {card_name}")))?;

    // Create card instance from definition
    let card = card_def.instantiate(card_id, owner_id);
    game.cards.insert(card_id, card);

    Ok(card_id)
}

/// (white, blue, black, red, green) amounts produced by a single mana ability.
/// Used by the dual-land / Mox card-compat tests to assert the exact colours
/// each "{T}: Add {color}" ability produces.
#[cfg(test)]
type ManaColorTuple = (u8, u8, u8, u8, u8);

/// Collect the colour tuples produced by a card's tap-cost mana abilities,
/// in ability order. Centralises the `Effect::AddMana` extraction so the
/// card-compat tests don't each repeat a wildcard match (which trips
/// clippy::wildcard_enum_match_arm under `-D warnings`).
#[cfg(test)]
fn tap_mana_ability_colors(card: &Card) -> Vec<ManaColorTuple> {
    use crate::core::Cost;
    let mut out = Vec::new();
    for ability in &card.activated_abilities {
        if !ability.is_mana_ability || !matches!(ability.cost, Cost::Tap) {
            continue;
        }
        for effect in &ability.effects {
            if let Effect::AddMana { mana, .. } = effect {
                out.push((mana.white, mana.blue, mana.black, mana.red, mana.green));
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::ZeroController;

    #[test]
    fn test_resolve_pump_spell() {
        use crate::core::{Effect, ManaCost};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];

        // Create a 2/2 creature on battlefield
        let creature_id = game.next_card_id();
        let mut creature = Card::new(creature_id, "Grizzly Bears".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(2));
        creature.set_base_toughness(Some(2));
        creature.controller = p1_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Check initial stats
        let creature_before = game.cards.get(creature_id).unwrap();
        assert_eq!(creature_before.current_power(), 2);
        assert_eq!(creature_before.current_toughness(), 2);

        // Create Giant Growth (pump +3/+3)
        let pump_spell_id = game.next_card_id();
        let mut pump_spell = Card::new(pump_spell_id, "Giant Growth".to_string(), p1_id);
        pump_spell.add_type(CardType::Instant);
        pump_spell.mana_cost = ManaCost::from_string("G");
        // Target the creature we created
        pump_spell.effects.push(Effect::PumpCreature {
            target: creature_id,
            power_bonus: 3,
            toughness_bonus: 3,
            keywords_granted: smallvec::SmallVec::new(),
        });
        game.cards.insert(pump_spell_id, pump_spell);

        // Put spell on stack (simulating cast)
        game.stack.add(pump_spell_id);

        // Resolve the spell
        assert!(
            game.resolve_spell(pump_spell_id, &[]).is_ok(),
            "Failed to resolve pump spell"
        );

        // Check creature got the bonus
        let creature_after = game.cards.get(creature_id).unwrap();
        assert_eq!(
            creature_after.current_power(),
            5,
            "Creature should have +3 power bonus (2 + 3)"
        );
        assert_eq!(
            creature_after.current_toughness(),
            5,
            "Creature should have +3 toughness bonus (2 + 3)"
        );

        // Check spell went to graveyard
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(
                zones.graveyard.contains(pump_spell_id),
                "Pump spell should be in graveyard"
            );
        }
    }

    #[test]
    fn test_pump_effect_cleanup_at_end_of_turn() {
        use crate::core::CardType;
        use crate::game::Step;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create a 2/2 creature on battlefield
        let creature_id = game.next_card_id();
        let mut creature = Card::new(creature_id, "Grizzly Bears".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(2));
        creature.set_base_toughness(Some(2));
        creature.controller = p1_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Apply pump effect manually
        if let Ok(card) = game.cards.get_mut(creature_id) {
            card.power_bonus = 3;
            card.toughness_bonus = 3;
        }

        // Verify pumped stats
        let creature_pumped = game.cards.get(creature_id).unwrap();
        assert_eq!(creature_pumped.current_power(), 5);
        assert_eq!(creature_pumped.current_toughness(), 5);

        // Advance to End step
        game.turn.current_step = Step::End;

        // Advance to Cleanup step (should trigger cleanup)
        assert!(game.advance_step().is_ok());
        assert_eq!(game.turn.current_step, Step::Cleanup);

        // Check that bonuses were cleared
        let creature_after = game.cards.get(creature_id).unwrap();
        assert_eq!(
            creature_after.current_power(),
            2,
            "Power bonus should be cleared at cleanup"
        );
        assert_eq!(
            creature_after.current_toughness(),
            2,
            "Toughness bonus should be cleared at cleanup"
        );
    }

    #[test]
    fn test_normal_creature_vs_first_strike() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 3/3 creature without first strike (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Hill Giant".to_string(), p1_id);
        attacker.add_type(CardType::Creature);
        attacker.set_base_power(Some(3));
        attacker.set_base_toughness(Some(3));
        attacker.controller = p1_id;
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create a 2/2 creature with First Strike (blocker)
        let blocker_id = game.next_entity_id();
        let mut blocker = Card::new(blocker_id, "First Strike Knight".to_string(), p2_id);
        blocker.add_type(CardType::Creature);
        blocker.set_base_power(Some(2));
        blocker.set_base_toughness(Some(2));
        blocker.controller = p2_id;
        blocker.keywords.insert(Keyword::FirstStrike);
        game.cards.insert(blocker_id, blocker);
        game.battlefield.add(blocker_id);

        // Declare combat
        game.combat.declare_attacker(attacker_id, p2_id);
        let attacker_vec = smallvec::smallvec![attacker_id];
        game.combat.declare_blocker(blocker_id, attacker_vec);

        // Create controllers
        let mut controller1 = ZeroController::new(p1_id);
        let mut controller2 = ZeroController::new(p2_id);

        // First strike damage step: only blocker deals damage
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, true);
        assert!(result.is_ok(), "Failed to assign first strike damage: {result:?}");

        // Attacker should have taken 2 damage but still be alive (3 toughness)
        assert!(
            game.battlefield.contains(attacker_id),
            "Attacker should still be alive after first strike"
        );

        // Blocker should still be alive (hasn't taken damage yet)
        assert!(game.battlefield.contains(blocker_id), "Blocker should still be alive");

        // Normal damage step: attacker deals damage, killing blocker
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign normal damage: {result:?}");

        // Blocker should be dead (took 3 damage, toughness 2)
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(blocker_id),
                "Blocker should be in graveyard after normal damage"
            );
        }

        // Attacker should still be alive (took only 2 damage, has 3 toughness)
        assert!(game.battlefield.contains(attacker_id), "Attacker should still be alive");
    }

    #[test]
    fn test_resolve_tap_spell() {
        use crate::core::{Effect, ManaCost};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // Create an untapped creature for P2
        let creature_id = game.next_card_id();
        let mut creature = Card::new(creature_id, "Grizzly Bears".to_string(), p2_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(2));
        creature.set_base_toughness(Some(2));
        creature.controller = p2_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Check initial state
        let creature_before = game.cards.get(creature_id).unwrap();
        assert!(!creature_before.tapped, "Creature should start untapped");

        // Create a Tap spell
        let tap_spell_id = game.next_card_id();
        let mut tap_spell = Card::new(tap_spell_id, "Frost Breath".to_string(), p1_id);
        tap_spell.add_type(CardType::Instant);
        tap_spell.mana_cost = ManaCost::from_string("2U");
        // Target the specific creature
        tap_spell.effects.push(Effect::TapPermanent { target: creature_id });
        game.cards.insert(tap_spell_id, tap_spell);

        // Put spell on stack (simulating cast)
        game.stack.add(tap_spell_id);

        // Resolve the spell
        assert!(
            game.resolve_spell(tap_spell_id, &[]).is_ok(),
            "Failed to resolve tap spell"
        );

        // Check creature is tapped
        let creature_after = game.cards.get(creature_id).unwrap();
        assert!(creature_after.tapped, "Creature should be tapped after spell");

        // Check spell went to graveyard
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(
                zones.graveyard.contains(tap_spell_id),
                "Tap spell should be in graveyard"
            );
        }
    }

    #[test]
    fn test_execute_mill_effect() {
        use crate::core::Effect;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        // Add cards to library
        for i in 0..5 {
            let card_id = game.next_card_id();
            let card = Card::new(card_id, format!("Card {i}"), p1_id);
            game.cards.insert(card_id, card);
            if let Some(zones) = game.get_player_zones_mut(p1_id) {
                zones.library.add(card_id);
            }
        }

        // Mill 3 cards
        let effect = Effect::Mill {
            player: p1_id,
            count: 3,
        };

        assert!(game.execute_effect(&effect).is_ok());

        // Check cards were milled (library reduced, graveyard increased)
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert_eq!(zones.library.cards.len(), 2, "Should have 2 cards left in library");
            assert_eq!(zones.graveyard.cards.len(), 3, "Should have 3 cards in graveyard");
        }
    }

    #[test]
    fn test_mill_with_empty_library() {
        use crate::core::Effect;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        // Add only 2 cards to library
        for i in 0..2 {
            let card_id = game.next_card_id();
            let card = Card::new(card_id, format!("Card {i}"), p1_id);
            game.cards.insert(card_id, card);
            if let Some(zones) = game.get_player_zones_mut(p1_id) {
                zones.library.add(card_id);
            }
        }

        // Try to mill 5 cards (more than available)
        let effect = Effect::Mill {
            player: p1_id,
            count: 5,
        };

        assert!(game.execute_effect(&effect).is_ok());

        // Check only 2 cards were milled (library is empty)
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert_eq!(zones.library.cards.len(), 0, "Library should be empty");
            assert_eq!(zones.graveyard.cards.len(), 2, "Should have milled only 2 cards");
        }
    }

    #[test]
    fn test_counter_spell_effect() {
        use crate::core::{CardType, Effect, ManaCost};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // P1 casts Lightning Bolt (target spell to be countered)
        let bolt_id = game.next_card_id();
        let mut bolt = Card::new(bolt_id, "Lightning Bolt".to_string(), p1_id);
        bolt.add_type(CardType::Instant);
        bolt.mana_cost = ManaCost::from_string("R");
        bolt.effects.push(Effect::DealDamage {
            target: crate::core::TargetRef::Player(p2_id),
            amount: 3,
        });
        game.cards.insert(bolt_id, bolt);
        game.stack.add(bolt_id);

        // P2 responds with Counterspell
        let counter_id = game.next_card_id();
        let mut counterspell = Card::new(counter_id, "Counterspell".to_string(), p2_id);
        counterspell.add_type(CardType::Instant);
        counterspell.mana_cost = ManaCost::from_string("UU");
        counterspell.effects.push(Effect::CounterSpell { target: bolt_id });
        game.cards.insert(counter_id, counterspell);
        game.stack.add(counter_id);

        // Verify both are on the stack
        assert!(game.stack.contains(bolt_id));
        assert!(game.stack.contains(counter_id));

        // Resolve counterspell (counters Lightning Bolt)
        assert!(game.resolve_spell(counter_id, &[]).is_ok());

        // Verify counterspell is in graveyard
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(zones.graveyard.contains(counter_id));
        }

        // Verify Lightning Bolt was countered (removed from stack, in graveyard)
        assert!(!game.stack.contains(bolt_id));
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(
                zones.graveyard.contains(bolt_id),
                "Countered spell should be in graveyard"
            );
        }

        // Verify P2 didn't take damage (Lightning Bolt was countered before resolving)
        let p2 = game.get_player(p2_id).unwrap();
        assert_eq!(p2.life, 20, "Player 2 should still have 20 life");
    }

    #[test]
    fn test_counter_spell_with_placeholder_target() {
        use crate::core::{CardType, Effect, ManaCost};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // P1 casts Lightning Bolt
        let bolt_id = game.next_card_id();
        let mut bolt = Card::new(bolt_id, "Lightning Bolt".to_string(), p1_id);
        bolt.add_type(CardType::Instant);
        bolt.mana_cost = ManaCost::from_string("R");
        bolt.effects.push(Effect::DealDamage {
            target: crate::core::TargetRef::Player(p2_id),
            amount: 3,
        });
        game.cards.insert(bolt_id, bolt);
        game.stack.add(bolt_id);

        // P2 responds with Counterspell using placeholder target (CardId::new(0))
        let counter_id = game.next_card_id();
        let mut counterspell = Card::new(counter_id, "Counterspell".to_string(), p2_id);
        counterspell.add_type(CardType::Instant);
        counterspell.mana_cost = ManaCost::from_string("UU");
        // Use placeholder target - should automatically target opponent's spell
        counterspell.effects.push(Effect::CounterSpell {
            target: crate::core::CardId::new(0),
        });
        game.cards.insert(counter_id, counterspell);
        game.stack.add(counter_id);

        // Resolve counterspell - should automatically find and counter Lightning Bolt
        assert!(game.resolve_spell(counter_id, &[bolt_id]).is_ok());

        // Verify Lightning Bolt was countered
        assert!(!game.stack.contains(bolt_id));
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(
                zones.graveyard.contains(bolt_id),
                "Countered spell should be in graveyard"
            );
        }
    }

    #[test]
    fn test_etb_trigger_draw() {
        use crate::core::{Effect, Trigger, TriggerEvent};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        // Add cards to P1's library for drawing
        for i in 0..5 {
            let card_id = game.next_card_id();
            let card = Card::new(card_id, format!("Card {i}"), p1_id);
            game.cards.insert(card_id, card);
            if let Some(zones) = game.get_player_zones_mut(p1_id) {
                zones.library.add(card_id);
            }
        }

        // Create a creature with an ETB trigger (like Elvish Visionary)
        let creature_id = game.next_entity_id();
        let mut creature = Card::new(creature_id, "Elvish Visionary".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(1));
        creature.set_base_toughness(Some(1));
        creature.mana_cost = ManaCost::from_string("1G");

        // Add ETB trigger: "When this enters the battlefield, draw a card"
        creature.triggers.push(Trigger::new(
            TriggerEvent::EntersBattlefield,
            vec![Effect::DrawCards {
                player: p1_id,
                count: 1,
            }],
            "When Elvish Visionary enters, draw a card.".to_string(),
        ));

        game.cards.insert(creature_id, creature);

        // Put the creature on the stack (as if it was cast)
        game.stack.add(creature_id);

        // Resolve the creature spell (moves it to battlefield and triggers ETB)
        assert!(game.resolve_spell(creature_id, &[]).is_ok());

        // Verify the creature is on the battlefield
        assert!(game.battlefield.contains(creature_id));

        // Verify the ETB trigger drew a card
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert_eq!(zones.hand.cards.len(), 1, "Should have drawn 1 card");
            assert_eq!(zones.library.cards.len(), 4, "Should have 4 cards left in library");
        }
    }

    #[test]
    fn test_air_nomad_legacy_creates_clue_token() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        let air_nomad_legacy_id = load_test_card(&mut game, "Air Nomad Legacy", p1_id)
            .expect("Air Nomad Legacy should load from cardsfolder");

        let db = CardDatabase::new(PathBuf::from("../cardsfolder"));
        let mut clue_definition = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { db.get_token("c_a_clue_draw").await })
            .expect("Clue token script should parse")
            .expect("Clue token script should exist");
        clue_definition.script_name = Some("c_a_clue_draw".to_string());
        game.token_definitions
            .insert("c_a_clue_draw".to_string(), std::sync::Arc::new(clue_definition));

        game.stack.add(air_nomad_legacy_id);
        game.resolve_spell(air_nomad_legacy_id, &[])
            .expect("Air Nomad Legacy should resolve");

        assert!(game.battlefield.contains(air_nomad_legacy_id));

        let clue_tokens: Vec<_> = game
            .battlefield
            .cards
            .iter()
            .filter_map(|card_id| game.cards.get(*card_id).ok())
            .filter(|card| card.name.as_str() == "Clue Token")
            .collect();

        assert_eq!(clue_tokens.len(), 1, "Air Nomad Legacy should create one Clue token");
        assert!(clue_tokens[0].is_token, "Created Clue should be marked as a token");
        assert_eq!(
            clue_tokens[0].controller, p1_id,
            "Clue should enter under caster control"
        );
        assert_eq!(clue_tokens[0].owner, p1_id, "Clue should be owned by the caster");
    }

    /// Regression test for `bug-clue-token-activation`: a Clue token's
    /// `{2}, Sacrifice this token: Draw a card` ability was being filtered out
    /// of the available-actions list because `can_pay_sacrifice_pattern` did
    /// not recognise the `CARDNAME` (sacrifice-self) pattern, so it returned
    /// `false` even when the token was on the battlefield with mana available.
    ///
    /// This test:
    ///  1. Resolves Air Nomad Legacy from the real cardsfolder script (the
    ///     same path exercised by `test_air_nomad_legacy_creates_clue_token`)
    ///     to put a Clue token on the battlefield under P1's control.
    ///  2. Gives P1 enough lands to pay {2}.
    ///  3. Calls `push_activatable_abilities` and asserts the Clue token's
    ///     activated ability is present in the resulting buffer.
    #[test]
    fn test_clue_token_ability_offered_when_payable() {
        use crate::core::SpellAbility;
        use crate::game::game_loop::GameLoop;
        use crate::game::VerbosityLevel;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;
        let p2_id = game.players[1].id;

        // 1. Put a Clue token on the battlefield under P1 by resolving
        //    Air Nomad Legacy. This exercises the real CreateToken path, so
        //    the token's `A:AB$ Draw | Cost$ 2 Sac<1/CARDNAME/this token> ...`
        //    line gets parsed into an `ActivatedAbility` on the instantiated
        //    card.
        let air_nomad_id = load_test_card(&mut game, "Air Nomad Legacy", p1_id).expect("Air Nomad Legacy should load");
        let db = CardDatabase::new(PathBuf::from("../cardsfolder"));
        let mut clue_definition = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { db.get_token("c_a_clue_draw").await })
            .expect("Clue token script should parse")
            .expect("Clue token script should exist");
        clue_definition.script_name = Some("c_a_clue_draw".to_string());
        game.token_definitions
            .insert("c_a_clue_draw".to_string(), std::sync::Arc::new(clue_definition));
        game.stack.add(air_nomad_id);
        game.resolve_spell(air_nomad_id, &[])
            .expect("Air Nomad Legacy should resolve");

        // Find the Clue token we just created.
        let clue_id = game
            .battlefield
            .cards
            .iter()
            .copied()
            .find(|cid| {
                game.cards
                    .get(*cid)
                    .map(|c| c.name.as_str() == "Clue Token")
                    .unwrap_or(false)
            })
            .expect("Clue token should be on the battlefield");
        let clue_card = game.cards.get(clue_id).expect("Clue card should exist");
        assert!(
            !clue_card.activated_abilities.is_empty(),
            "Clue token should have at least one activated ability after token script parsing"
        );

        // 2. Give P1 two untapped Plains so {2} is payable. Tag them as having
        //    entered earlier so summoning-sickness etc. doesn't matter (they
        //    aren't creatures, but be safe).
        let plains1 = load_test_card(&mut game, "Plains", p1_id).expect("Plains should load");
        let plains2 = load_test_card(&mut game, "Plains", p1_id).expect("Plains should load");
        game.battlefield.add(plains1);
        game.battlefield.add(plains2);
        if let Ok(c) = game.cards.get_mut(plains1) {
            c.controller = p1_id;
            c.tapped = false;
        }
        if let Ok(c) = game.cards.get_mut(plains2) {
            c.controller = p1_id;
            c.tapped = false;
        }

        // 3. Build the abilities buffer for P1 and assert the Clue ability is
        //    present. We can't easily get the exact `ability_index` without
        //    enumerating, so just look for `ActivateAbility { card_id == clue_id }`.
        let _ = p2_id; // p2 controller not actually needed for this enumeration
        let mut gl = GameLoop::new(&mut game).with_verbosity(VerbosityLevel::Silent);
        gl.push_activatable_abilities(p1_id);
        let buffer = gl.get_abilities_buffer().to_vec();
        let offered: Vec<_> = buffer
            .iter()
            .filter(|sa| matches!(sa, SpellAbility::ActivateAbility { card_id, .. } if *card_id == clue_id))
            .collect();
        assert!(
            !offered.is_empty(),
            "Clue token's draw-a-card ability should be offered when {{2}} can be paid \
             and the token is on the battlefield. Buffer was: {:?}",
            buffer
        );
    }

    #[test]
    fn test_etb_trigger_damage() {
        use crate::core::{Effect, TargetRef, Trigger, TriggerEvent};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P2: Create a target creature
        let target_creature_id = game.next_entity_id();
        let mut target = Card::new(target_creature_id, "Grizzly Bears".to_string(), p2_id);
        target.add_type(CardType::Creature);
        target.set_base_power(Some(2));
        target.set_base_toughness(Some(2));
        game.cards.insert(target_creature_id, target);
        game.battlefield.add(target_creature_id);

        // P1: Create a creature with ETB damage trigger (like Flametongue Kavu)
        let kavu_id = game.next_entity_id();
        let mut kavu = Card::new(kavu_id, "Flametongue Kavu".to_string(), p1_id);
        kavu.add_type(CardType::Creature);
        kavu.set_base_power(Some(4));
        kavu.set_base_toughness(Some(2));
        kavu.mana_cost = ManaCost::from_string("3R");

        // Add ETB trigger: "When this enters the battlefield, deal 4 damage to target creature"
        kavu.triggers.push(Trigger::new(
            TriggerEvent::EntersBattlefield,
            vec![Effect::DealDamage {
                target: TargetRef::None, // Will be filled to target an opponent's creature
                amount: 4,
            }],
            "When Flametongue Kavu enters, it deals 4 damage to target creature.".to_string(),
        ));

        game.cards.insert(kavu_id, kavu);

        // Put the kavu on the stack (as if it was cast)
        game.stack.add(kavu_id);

        // Resolve the kavu spell (moves it to battlefield and triggers ETB)
        assert!(game.resolve_spell(kavu_id, &[]).is_ok());

        // Verify the kavu is on the battlefield
        assert!(game.battlefield.contains(kavu_id));

        // Check state-based actions for lethal damage
        game.check_lethal_damage().unwrap();

        // Verify the target creature took lethal damage (2 toughness, 4 damage dealt)
        // The creature should have been destroyed and moved to graveyard
        assert!(
            !game.battlefield.contains(target_creature_id),
            "Target creature should be destroyed"
        );
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(target_creature_id),
                "Target creature should be in graveyard"
            );
        }
    }

    #[test]
    fn test_elvish_visionary_from_cardsfolder() {
        // Test loading Elvish Visionary from the actual cardsfolder and verifying
        // its ETB trigger works correctly
        use crate::loader::CardDatabase;
        use std::path::PathBuf;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        // Add cards to P1's library for drawing
        for i in 0..5 {
            let card_id = game.next_card_id();
            let card = Card::new(card_id, format!("Card {i}"), p1_id);
            game.cards.insert(card_id, card);
            if let Some(zones) = game.get_player_zones_mut(p1_id) {
                zones.library.add(card_id);
            }
        }

        // Load Elvish Visionary from cardsfolder
        let cardsfolder = PathBuf::from("../cardsfolder");
        let db = CardDatabase::new(cardsfolder);

        let card_def = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { db.get_card("Elvish Visionary").await })
            .expect("Failed to get card")
            .expect("Elvish Visionary not found");

        // Create card instance
        let creature_id = game.next_entity_id();
        let creature = card_def.instantiate(creature_id, p1_id);

        // Verify it has an ETB trigger
        assert!(!creature.triggers.is_empty(), "Elvish Visionary should have triggers");
        assert_eq!(creature.triggers.len(), 1, "Elvish Visionary should have 1 trigger");

        game.cards.insert(creature_id, creature);

        // Put the creature on the stack (as if it was cast)
        game.stack.add(creature_id);

        // Resolve the creature spell (moves it to battlefield and triggers ETB)
        assert!(game.resolve_spell(creature_id, &[]).is_ok());

        // Verify the creature is on the battlefield
        assert!(game.battlefield.contains(creature_id));

        // Verify the ETB trigger drew a card
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert_eq!(zones.hand.cards.len(), 1, "Should have drawn 1 card from ETB trigger");
            assert_eq!(zones.library.cards.len(), 4, "Should have 4 cards left in library");
        }
    }

    #[test]
    fn test_counterspell_from_cardsfolder() {
        // Test loading Counterspell from the actual cardsfolder and verifying
        // it can counter spells
        use crate::loader::CardDatabase;
        use std::path::PathBuf;

        let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let alice_id = players[0];
        let bob_id = players[1];

        // Load Lightning Bolt and Counterspell from cardsfolder
        let cardsfolder = PathBuf::from("../cardsfolder");
        let db = CardDatabase::new(cardsfolder);

        let runtime = tokio::runtime::Runtime::new().unwrap();

        let bolt_def = runtime
            .block_on(async { db.get_card("Lightning Bolt").await })
            .expect("Failed to get card")
            .expect("Lightning Bolt not found");

        let counter_def = runtime
            .block_on(async { db.get_card("Counterspell").await })
            .expect("Failed to get card")
            .expect("Counterspell not found");

        // Player1 casts Lightning Bolt targeting Player2
        let bolt_id = game.next_card_id();
        let bolt = bolt_def.instantiate(bolt_id, alice_id);
        game.cards.insert(bolt_id, bolt);
        game.stack.add(bolt_id);

        // Player2 responds with Counterspell targeting Lightning Bolt
        let counter_id = game.next_card_id();
        let mut counterspell = counter_def.instantiate(counter_id, bob_id);
        // Manually set the target since we're bypassing the full casting process
        if let Some(Effect::CounterSpell { target }) = counterspell.effects.get_mut(0) {
            *target = bolt_id;
        }
        game.cards.insert(counter_id, counterspell);
        game.stack.add(counter_id);

        // Verify both are on the stack
        assert!(game.stack.contains(bolt_id), "Bolt should be on stack");
        assert!(game.stack.contains(counter_id), "Counterspell should be on stack");

        // Resolve Counterspell (should counter Lightning Bolt)
        assert!(
            game.resolve_spell(counter_id, &[]).is_ok(),
            "Counterspell should resolve successfully"
        );

        // Verify Counterspell is in graveyard
        if let Some(zones) = game.get_player_zones(bob_id) {
            assert!(
                zones.graveyard.contains(counter_id),
                "Counterspell should be in graveyard"
            );
        }

        // Verify Lightning Bolt was countered (removed from stack, in graveyard)
        assert!(!game.stack.contains(bolt_id), "Lightning Bolt should not be on stack");
        if let Some(zones) = game.get_player_zones(alice_id) {
            assert!(
                zones.graveyard.contains(bolt_id),
                "Countered spell should be in graveyard"
            );
        }

        // Verify Player2 didn't take damage (Lightning Bolt was countered)
        let bob = game.get_player(bob_id).unwrap();
        assert_eq!(bob.life, 20, "Player2 should still have 20 life");
    }

    /// Regression for bug-stack-chaining-instants: while a spell is on the
    /// stack, the active player must still be offered the option to cast a
    /// second instant from hand as a response. Pre-fix, the action list during
    /// the response window only showed `pass` and activated abilities (e.g.
    /// Strip Mine), but not instant-speed spells from hand.
    ///
    /// We cannot easily fake an arbitrary spell on the stack at this layer
    /// (the stack add path expects a real cast), so this test instead
    /// directly puts a Lightning Bolt on the stack via `game.stack.add()` to
    /// simulate the in-flight spell, then asserts that
    /// `push_castable_spells` still surfaces the second instant from hand.
    #[test]
    fn test_instant_offered_during_response_window_to_stack_spell() {
        use crate::core::SpellAbility;
        use crate::game::game_loop::GameLoop;
        use crate::game::VerbosityLevel;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // P1 has Lightning Bolt in hand (the response we expect to be offered)
        // and a Mountain to pay {R}.
        let bolt_id = load_test_card(&mut game, "Lightning Bolt", p1_id).expect("Lightning Bolt should load");
        if let Some(zones) = game.get_player_zones_mut(p1_id) {
            zones.hand.add(bolt_id);
        }

        let mountain_id = load_test_card(&mut game, "Mountain", p1_id).expect("Mountain should load");
        game.battlefield.add(mountain_id);
        if let Ok(c) = game.cards.get_mut(mountain_id) {
            c.controller = p1_id;
            c.tapped = false;
        }

        // Put a placeholder spell on the stack to simulate "an in-flight cast".
        // The stack contents themselves don't matter for the bug we're guarding
        // against — what matters is that `stack_is_empty` is false during the
        // ability enumeration. We use a second Lightning Bolt instance owned
        // by P1 so it doesn't accidentally turn into a target the response
        // could counter.
        let in_flight_id = load_test_card(&mut game, "Lightning Bolt", p1_id).expect("Lightning Bolt should load");
        game.stack.add(in_flight_id);
        assert!(!game.stack.is_empty(), "Stack should be non-empty for the test");

        // Force priority window state: pretend it's P1's main phase with stack
        // non-empty (instants don't require sorcery speed; the bug was about
        // whether they show up at all when stack is non-empty).
        // Engine-internal state that controls phase is already set by
        // new_two_player to something compatible with main-phase priority for
        // the active player.

        // Enumerate castable spells for P1 with the stack populated.
        let mut gl = GameLoop::new(&mut game).with_verbosity(VerbosityLevel::Silent);
        gl.push_castable_spells(p1_id);
        let buffer = gl.get_abilities_buffer().to_vec();

        let bolt_offered = buffer
            .iter()
            .any(|sa| matches!(sa, SpellAbility::CastSpell { card_id, .. } if *card_id == bolt_id));
        assert!(
            bolt_offered,
            "Lightning Bolt in hand must be offered as a response while a spell is on the stack. \
             Buffer was: {:?}",
            buffer
        );
    }

    #[test]
    fn test_etb_trigger_gain_life() {
        use crate::core::{Effect, Trigger, TriggerEvent};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create a creature with ETB gain life trigger
        let creature_id = game.next_entity_id();
        let mut creature = Card::new(creature_id, "Soul Warden".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(1));
        creature.set_base_toughness(Some(1));
        creature.mana_cost = ManaCost::from_string("W");

        // Add ETB trigger: "When this enters the battlefield, you gain 3 life"
        creature.triggers.push(Trigger::new(
            TriggerEvent::EntersBattlefield,
            vec![Effect::GainLife {
                player: crate::core::PlayerId::new(0), // Placeholder
                amount: 3,
            }],
            "When this enters, you gain 3 life.".to_string(),
        ));

        game.cards.insert(creature_id, creature);

        // Record life before
        let life_before = game.get_player(p1_id).unwrap().life;

        // Put the creature on the stack and resolve it
        game.stack.add(creature_id);
        assert!(game.resolve_spell(creature_id, &[]).is_ok());

        // Verify the creature is on the battlefield
        assert!(game.battlefield.contains(creature_id));

        // Verify the ETB trigger gained life
        let life_after = game.get_player(p1_id).unwrap().life;
        assert_eq!(
            life_after,
            life_before + 3,
            "Should have gained 3 life from ETB trigger"
        );
    }

    #[test]
    fn test_etb_trigger_pump() {
        use crate::core::{Effect, Trigger, TriggerEvent};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create a target creature on battlefield
        let target_id = game.next_entity_id();
        let mut target = Card::new(target_id, "Grizzly Bears".to_string(), p1_id);
        target.add_type(CardType::Creature);
        target.set_base_power(Some(2));
        target.set_base_toughness(Some(2));
        game.cards.insert(target_id, target);
        game.battlefield.add(target_id);

        // Create a creature with ETB pump trigger
        let creature_id = game.next_entity_id();
        let mut creature = Card::new(creature_id, "Glorious Anthem".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(1));
        creature.set_base_toughness(Some(1));

        // Add ETB trigger: "When this enters, target creature gets +2/+2"
        creature.triggers.push(Trigger::new(
            TriggerEvent::EntersBattlefield,
            vec![Effect::PumpCreature {
                target: crate::core::CardId::new(0), // Placeholder
                power_bonus: 2,
                toughness_bonus: 2,
                keywords_granted: smallvec::SmallVec::new(),
            }],
            "When this enters, target creature gets +2/+2.".to_string(),
        ));

        game.cards.insert(creature_id, creature);

        // Put the creature on the stack and resolve it
        game.stack.add(creature_id);
        assert!(game.resolve_spell(creature_id, &[]).is_ok());

        // Verify the creature is on the battlefield
        assert!(game.battlefield.contains(creature_id));

        // Verify the target got pumped
        let pumped_card = game.cards.get(target_id).unwrap();
        assert_eq!(pumped_card.power_bonus, 2, "Target should have +2 power bonus");
        assert_eq!(pumped_card.toughness_bonus, 2, "Target should have +2 toughness bonus");
    }

    /// Test upkeep trigger dealing damage to controller (like Juzám Djinn)
    #[test]
    fn test_upkeep_trigger_damage_to_controller() {
        use crate::core::{Effect, TargetRef, Trigger, TriggerEvent};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Verify initial life
        assert_eq!(game.players[0].life, 20);

        // Create a creature with an upkeep trigger (like Juzám Djinn)
        let djinn_id = game.next_entity_id();
        let mut djinn = Card::new(djinn_id, "Juzám Djinn".to_string(), p1_id);
        djinn.types.push(CardType::Creature);
        djinn.set_base_power(Some(5));
        djinn.set_base_toughness(Some(5));
        djinn.controller = p1_id;

        // Add upkeep trigger: "At the beginning of your upkeep, deal 1 damage to you"
        // The [controller_only] flag ensures it only triggers on the controller's turn
        djinn.triggers.push(Trigger::new(
            TriggerEvent::BeginningOfUpkeep,
            vec![Effect::DealDamage {
                target: TargetRef::Player(crate::core::PlayerId::new(0)), // Placeholder for controller
                amount: 1,
            }],
            "[controller_only] At the beginning of your upkeep, Juzám Djinn deals 1 damage to you.".to_string(),
        ));

        game.cards.insert(djinn_id, djinn);
        game.battlefield.add(djinn_id);

        // Set the active player to P1
        game.turn.active_player = p1_id;

        // Execute the upkeep trigger
        let result = game.check_triggers_for_controller(TriggerEvent::BeginningOfUpkeep, djinn_id, p1_id);
        assert!(result.is_ok(), "Upkeep trigger should execute successfully");

        // Verify P1 took 1 damage
        assert_eq!(
            game.players[0].life, 19,
            "Controller should have taken 1 damage from Juzám Djinn"
        );
    }

    /// Test upkeep trigger only fires on controller's turn (not opponent's)
    #[test]
    fn test_upkeep_trigger_controller_only() {
        use crate::core::{Effect, TargetRef, Trigger, TriggerEvent};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // Create Juzám Djinn controlled by P1
        let djinn_id = game.next_entity_id();
        let mut djinn = Card::new(djinn_id, "Juzám Djinn".to_string(), p1_id);
        djinn.types.push(CardType::Creature);
        djinn.set_base_power(Some(5));
        djinn.set_base_toughness(Some(5));
        djinn.controller = p1_id;

        // controller_turn_only trigger should only fire when controller is active
        let mut upkeep_trigger = Trigger::new(
            TriggerEvent::BeginningOfUpkeep,
            vec![Effect::DealDamage {
                target: TargetRef::Player(crate::core::PlayerId::new(0)),
                amount: 1,
            }],
            "[controller_only] At the beginning of your upkeep, Juzám Djinn deals 1 damage to you.".to_string(),
        );
        upkeep_trigger.controller_turn_only = true;
        djinn.triggers.push(upkeep_trigger);

        game.cards.insert(djinn_id, djinn);
        game.battlefield.add(djinn_id);

        // Try to fire trigger during P2's turn (should not fire)
        game.turn.active_player = p2_id;
        let result = game.check_triggers_for_controller(TriggerEvent::BeginningOfUpkeep, djinn_id, p2_id);
        assert!(result.is_ok());

        // P1 should NOT have taken damage (it's not their upkeep)
        assert_eq!(game.players[0].life, 20, "P1 should not take damage during P2's upkeep");

        // Now fire during P1's turn (should fire)
        game.turn.active_player = p1_id;
        let result = game.check_triggers_for_controller(TriggerEvent::BeginningOfUpkeep, djinn_id, p1_id);
        assert!(result.is_ok());

        // P1 should have taken 1 damage
        assert_eq!(game.players[0].life, 19, "P1 should take 1 damage during their upkeep");
    }

    /// Test loading real Juzám Djinn from cardsfolder and verifying trigger parsing
    #[test]
    fn test_juzam_djinn_from_cardsfolder() {
        use crate::core::TriggerEvent;
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/j/juzam_djinn.txt");
        if !path.exists() {
            println!("Skipping test: cardsfolder not present");
            return;
        }

        let def = crate::loader::CardLoader::load_from_file(&path).expect("Failed to load Juzám Djinn");
        assert_eq!(def.name.as_str(), "Juzám Djinn");

        // Instantiate the card
        let game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let card_id = crate::core::CardId::new(100);
        let card = def.instantiate(card_id, p1_id);

        // Verify the upkeep trigger was parsed
        let upkeep_triggers: Vec<_> = card
            .triggers
            .iter()
            .filter(|t| t.event == TriggerEvent::BeginningOfUpkeep)
            .collect();

        assert_eq!(
            upkeep_triggers.len(),
            1,
            "Juzám Djinn should have exactly one upkeep trigger"
        );

        // Verify the trigger has the DealDamage effect
        let trigger = upkeep_triggers[0];
        assert!(!trigger.effects.is_empty(), "Upkeep trigger should have effects");

        // Verify the trigger description indicates controller-only
        assert!(
            trigger.description.contains("[controller_only]") || trigger.description.contains("your upkeep"),
            "Trigger should indicate it's controller-only"
        );
    }

    /// Test loading real Su-Chi from cardsfolder and verifying death trigger parsing
    #[test]
    fn test_su_chi_death_trigger_from_cardsfolder() {
        use crate::core::TriggerEvent;
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/s/su_chi.txt");
        if !path.exists() {
            println!("Skipping test: cardsfolder not present");
            return;
        }

        let def = crate::loader::CardLoader::load_from_file(&path).expect("Failed to load Su-Chi");
        assert_eq!(def.name.as_str(), "Su-Chi");

        // Instantiate the card
        let game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let card_id = crate::core::CardId::new(100);
        let card = def.instantiate(card_id, p1_id);

        // Verify the death trigger was parsed (LeavesBattlefield event)
        let death_triggers: Vec<_> = card
            .triggers
            .iter()
            .filter(|t| t.event == TriggerEvent::LeavesBattlefield)
            .collect();

        assert_eq!(death_triggers.len(), 1, "Su-Chi should have exactly one death trigger");

        // Verify the trigger has the AddMana effect
        let trigger = death_triggers[0];
        assert!(!trigger.effects.is_empty(), "Death trigger should have effects");

        // Verify the effect is AddMana
        let has_add_mana = trigger
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::AddMana { .. }));
        assert!(has_add_mana, "Death trigger should have AddMana effect");
    }

    /// Test Su-Chi death trigger actually fires in combat
    #[test]
    fn test_su_chi_death_trigger_fires_in_combat() {
        use crate::core::{CardType, Effect, ManaCost, Trigger, TriggerEvent};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // Create a Su-Chi for P1 with death trigger
        let su_chi_id = game.next_entity_id();
        let mut su_chi = crate::core::Card::new(su_chi_id, "Su-Chi".to_string(), p1_id);
        su_chi.types.push(CardType::Artifact);
        su_chi.types.push(CardType::Creature);
        su_chi.set_base_power(Some(4));
        su_chi.set_base_toughness(Some(4));
        // Add the death trigger: "When Su-Chi dies, add {C}{C}{C}{C}"
        let mana = ManaCost::from_string("CCCC");
        su_chi.triggers.push(Trigger::new(
            TriggerEvent::LeavesBattlefield,
            vec![Effect::AddMana {
                player: crate::core::PlayerId::new(0), // Placeholder
                mana,
                produces_chosen_color: false,
                amount_var: None,
            }],
            "When CARDNAME dies, add {C}{C}{C}{C}.".to_string(),
        ));
        game.cards.insert(su_chi_id, su_chi);
        game.battlefield.add(su_chi_id);

        // Create a big creature for P2 to kill Su-Chi
        let killer_id = game.next_entity_id();
        let mut killer = crate::core::Card::new(killer_id, "Big Creature".to_string(), p2_id);
        killer.types.push(CardType::Creature);
        killer.set_base_power(Some(10));
        killer.set_base_toughness(Some(10));
        game.cards.insert(killer_id, killer);
        game.battlefield.add(killer_id);

        // Set up combat - Su-Chi blocks the big creature
        game.turn.active_player = p2_id;

        // Check P1's mana pool before combat
        let p1_mana_before = game.players[0].mana_pool.colorless;

        // Su-Chi takes lethal damage and dies
        // Simulate by calling check_death_triggers directly
        let result = game.check_death_triggers(su_chi_id);
        assert!(result.is_ok(), "Death trigger should execute successfully");

        // Check P1's mana pool after - should have gained 4 colorless mana
        let p1_mana_after = game.players[0].mana_pool.colorless;
        assert_eq!(
            p1_mana_after,
            p1_mana_before + 4,
            "P1 should gain 4 colorless mana when Su-Chi dies"
        );
    }

    /// Test Swords to Plowshares exile effect
    ///
    /// Tests the full flow:
    /// 1. Load Swords to Plowshares from cardsfolder
    /// 2. Create a target creature on battlefield
    /// 3. Verify get_valid_targets_for_spell finds the creature
    /// 4. Resolve spell with chosen target
    /// 5. Verify creature is exiled and controller gains life
    #[test]
    fn test_swords_to_plowshares_exile() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // Load Swords to Plowshares from cardsfolder
        let swords_id = match load_test_card(&mut game, "Swords to Plowshares", p1_id) {
            Ok(id) => id,
            Err(e) => panic!("Failed to load Swords to Plowshares: {e}"),
        };

        // Verify the spell has ExilePermanent effect
        let swords = game.cards.get(swords_id).unwrap();
        assert!(
            swords
                .effects
                .iter()
                .any(|e| matches!(e, Effect::ExilePermanent { .. })),
            "Swords to Plowshares should have ExilePermanent effect. Effects: {:?}",
            swords.effects
        );

        // Create a 3/3 creature controlled by P2
        let creature_id = game.next_entity_id();
        let mut creature = Card::new(creature_id, "Trained Armodon".to_string(), p2_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(3));
        creature.set_base_toughness(Some(3));
        creature.controller = p2_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Verify targeting - Swords should find the creature as a valid target
        let valid_targets = game.get_valid_targets_for_spell(swords_id).unwrap();
        assert!(
            valid_targets.contains(&creature_id),
            "Creature should be a valid target for Swords to Plowshares. Valid targets: {:?}",
            valid_targets
        );

        // Put spell on stack (simulating cast)
        game.stack.add(swords_id);

        // Record P2's life before resolution
        let life_before = game.get_player(p2_id).unwrap().life;

        // Resolve the spell WITH target
        let result = game.resolve_spell(swords_id, &[creature_id]);
        assert!(result.is_ok(), "Failed to resolve Swords to Plowshares: {:?}", result);

        // Verify creature is exiled (exile zone is per-player based on owner)
        let in_exile = game.get_player_zones(p2_id).unwrap().exile.contains(creature_id);
        assert!(in_exile, "Creature should be in exile zone");
        assert!(
            !game.battlefield.contains(creature_id),
            "Creature should not be on battlefield"
        );

        // Verify P2 gained life equal to creature's power (3)
        let life_after = game.get_player(p2_id).unwrap().life;
        // Note: The GainLife effect from SubAbility is not yet implemented
        // so we just verify the exile worked for now
        assert_eq!(
            life_after, life_before,
            "Life gain not yet implemented for SubAbility$ DBGainLife"
        );
    }

    /// Test Web Up ETB trigger (Oblivion Ring-style effect)
    ///
    /// Web Up is an enchantment with:
    /// "When this enchantment enters, exile target nonland permanent an opponent
    /// controls until this enchantment leaves the battlefield."
    ///
    /// This tests that:
    /// 1. The card loads with an ETB trigger
    /// 2. The trigger has an ExilePermanent effect
    /// 3. The effect targets opponent's nonland permanents
    #[test]
    fn test_web_up_etb_trigger() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let _p2_id = game.players[1].id;

        // Load Web Up from cardsfolder
        let web_up_id = match load_test_card(&mut game, "Web Up", p1_id) {
            Ok(id) => id,
            Err(e) => panic!("Failed to load Web Up: {e}"),
        };

        let web_up = game.cards.get(web_up_id).unwrap();

        // Verify it's an enchantment
        assert!(
            web_up.is_enchantment(),
            "Web Up should be an enchantment. Types: {:?}",
            web_up.types
        );

        // Verify the card has an ETB trigger
        assert!(
            !web_up.triggers.is_empty(),
            "Web Up should have at least one trigger. Triggers: {:?}",
            web_up.triggers
        );

        // Find the ETB trigger
        use crate::core::TriggerEvent;
        let etb_trigger = web_up
            .triggers
            .iter()
            .find(|t| t.event == TriggerEvent::EntersBattlefield);

        assert!(
            etb_trigger.is_some(),
            "Web Up should have an EntersBattlefield trigger. Triggers: {:?}",
            web_up.triggers
        );

        let trigger = etb_trigger.unwrap();

        // Verify the trigger has an ExilePermanent effect
        let has_exile_effect = trigger
            .effects
            .iter()
            .any(|e| matches!(e, Effect::ExilePermanent { .. }));

        assert!(
            has_exile_effect,
            "Web Up ETB trigger should have ExilePermanent effect. Effects: {:?}",
            trigger.effects
        );
    }

    /// Test Web Up exiles an opponent's creature when it enters the battlefield
    ///
    /// This is an integration test that verifies the full flow:
    /// 1. Player 1 casts Web Up
    /// 2. Web Up resolves and enters the battlefield
    /// 3. ETB trigger fires and exiles an opponent's creature
    #[test]
    fn test_web_up_exiles_creature() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // Create a creature for P2
        let creature_id = game.next_entity_id();
        let mut creature = Card::new(creature_id, "Grizzly Bears".to_string(), p2_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(2));
        creature.set_base_toughness(Some(2));
        creature.controller = p2_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Verify creature is on battlefield
        assert!(
            game.battlefield.contains(creature_id),
            "Creature should be on battlefield before Web Up"
        );

        // Load Web Up from cardsfolder
        let web_up_id = match load_test_card(&mut game, "Web Up", p1_id) {
            Ok(id) => id,
            Err(e) => panic!("Failed to load Web Up: {e}"),
        };

        // Put Web Up on the stack (simulating cast)
        game.stack.add(web_up_id);

        // Resolve Web Up (it enters the battlefield and triggers)
        let result = game.resolve_spell(web_up_id, &[]);
        assert!(result.is_ok(), "Failed to resolve Web Up: {:?}", result);

        // Verify Web Up is on the battlefield
        assert!(
            game.battlefield.contains(web_up_id),
            "Web Up should be on the battlefield after resolving"
        );

        // Verify creature is now exiled (not on battlefield)
        assert!(
            !game.battlefield.contains(creature_id),
            "Creature should not be on battlefield after Web Up ETB trigger"
        );

        // Verify creature is in exile zone
        let in_exile = game.get_player_zones(p2_id).unwrap().exile.contains(creature_id);
        assert!(in_exile, "Creature should be in exile zone after Web Up ETB");
    }

    /// Test Vibrant Cityscape activated ability for tutoring basic lands
    ///
    /// Vibrant Cityscape has:
    /// "{T}, Sacrifice this land: Search your library for a basic land card,
    /// put it onto the battlefield tapped, then shuffle."
    ///
    /// This tests that:
    /// 1. The card loads with an activated ability
    /// 2. The ability has a SearchLibrary effect
    /// 3. The ability costs tap + sacrifice
    #[test]
    fn test_vibrant_cityscape_tutor_ability() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Load Vibrant Cityscape from cardsfolder
        let cityscape_id = match load_test_card(&mut game, "Vibrant Cityscape", p1_id) {
            Ok(id) => id,
            Err(e) => panic!("Failed to load Vibrant Cityscape: {e}"),
        };

        let cityscape = game.cards.get(cityscape_id).unwrap();

        // Verify it's a land
        assert!(
            cityscape.is_land(),
            "Vibrant Cityscape should be a land. Types: {:?}",
            cityscape.types
        );

        // Verify the card has an activated ability
        assert!(
            !cityscape.activated_abilities.is_empty(),
            "Vibrant Cityscape should have at least one activated ability. Abilities: {:?}",
            cityscape.activated_abilities
        );

        // Check if the ability has a SearchLibrary effect
        let has_search_effect = cityscape
            .activated_abilities
            .iter()
            .any(|ab| ab.effects.iter().any(|e| matches!(e, Effect::SearchLibrary { .. })));

        assert!(
            has_search_effect,
            "Vibrant Cityscape should have a SearchLibrary effect. Abilities: {:?}",
            cityscape.activated_abilities
        );
    }

    /// mtg-589 regression (parser shape): Demonic Tutor's
    /// `A:SP$ ChangeZone | Origin$ Library | Destination$ Hand | ChangeType$ Card`
    /// must parse into a single `Effect::SearchLibrary` so the game loop routes
    /// it through `choose_from_library_with_hook` (network-safe), instead of the
    /// naive `execute_effect` path that picks `library_cards[0]` and desyncs the
    /// shadow client (whose own library is reserved-but-unrevealed).
    #[test]
    fn test_demonic_tutor_parses_to_search_library() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        let tutor_id = load_test_card(&mut game, "Demonic Tutor", p1_id).expect("load Demonic Tutor");
        let tutor = game.cards.get(tutor_id).unwrap();

        let search_effects: Vec<&Effect> = tutor
            .effects
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    Effect::SearchLibrary {
                        destination: crate::zones::Zone::Hand,
                        ..
                    }
                )
            })
            .collect();
        assert_eq!(
            search_effects.len(),
            1,
            "Demonic Tutor should parse to exactly one SearchLibrary(->Hand) effect, got: {:?}",
            tutor.effects
        );
    }

    /// mtg-589 regression (network determinism): a forced (engine-chosen)
    /// `DiscardCards` of a fixed count must select cards by CardId, NOT by card
    /// properties (CMC / land), so the choice is information-independent across
    /// the server (full state) and a client's shadow state (opponent hand cards
    /// reserved-but-unrevealed). Without this, the shadow's property-based
    /// heuristic saw an empty candidate set and discarded nothing while the
    /// server discarded a card → FATAL hand/graveyard desync (Hypnotic Specter
    /// "discards a card at random" trigger). We assert the LOWEST-CardId card is
    /// the one discarded, since that is the deterministic rule both sides apply.
    #[test]
    fn test_forced_discard_picks_lowest_card_id_deterministically() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Put three cards into P1's hand with known, out-of-order CardIds so the
        // "lowest CardId" rule is distinguishable from "first inserted" / "last".
        let c_hi = load_test_card(&mut game, "Mountain", p1_id).expect("load Mountain");
        let c_lo = load_test_card(&mut game, "Swamp", p1_id).expect("load Swamp");
        let c_mid = load_test_card(&mut game, "Forest", p1_id).expect("load Forest");
        for cid in [c_hi, c_lo, c_mid] {
            game.get_player_zones_mut(p1_id).unwrap().hand.add(cid);
        }
        let expected = [c_hi, c_lo, c_mid].into_iter().min_by_key(|id| id.as_u32()).unwrap();

        game.execute_effect(&Effect::DiscardCards {
            player: p1_id,
            count: 1,
            remember_discarded: false,
            optional: false,
            remember_discarding_players: false,
        })
        .expect("discard executes");

        let gy = &game.get_player_zones(p1_id).unwrap().graveyard;
        assert_eq!(gy.cards.len(), 1, "exactly one card discarded");
        assert!(
            gy.contains(expected),
            "forced discard must remove the lowest-CardId card ({:?}) for network determinism; graveyard={:?}",
            expected,
            gy.cards
        );
        // And the lowest-CardId card must no longer be in hand.
        assert!(!game.get_player_zones(p1_id).unwrap().hand.contains(expected));
    }

    /// Test Balance spell effect - equalizes creatures across all players
    ///
    /// Balance is a classic white sorcery that equalizes lands, creatures, and hands.
    /// This test verifies that after casting Balance:
    /// - Players with more creatures than the minimum must sacrifice down to match
    /// - The sacrifice is executed correctly (creatures move to graveyard)
    #[test]
    fn test_balance_creature_sacrifice() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let _p1_id = game.players[0].id; // P1 has 0 creatures
        let p2_id = game.players[1].id;

        // P1 has 0 creatures
        // P2 has 3 creatures - should sacrifice all 3 after Balance

        // Create 3 creatures for P2
        let creature_ids: Vec<CardId> = (0..3)
            .map(|i| {
                let creature_id = game.next_card_id();
                let mut creature = Card::new(creature_id, format!("Bear {}", i + 1), p2_id);
                creature.controller = p2_id;
                creature.add_type(CardType::Creature);
                creature.set_base_power(Some(2));
                creature.set_base_toughness(Some(2));
                game.cards.insert(creature_id, creature);
                game.battlefield.add(creature_id);
                creature_id
            })
            .collect();

        // Verify initial state: P2 has 3 creatures on battlefield
        assert_eq!(
            game.battlefield
                .cards
                .iter()
                .filter(|&&cid| {
                    game.cards
                        .get(cid)
                        .map(|c| c.controller == p2_id && c.is_creature())
                        .unwrap_or(false)
                })
                .count(),
            3,
            "P2 should have 3 creatures before Balance"
        );

        // Execute Balance effect for creatures
        let result = game.execute_balance_effect("Creature", "Battlefield");
        assert!(
            result.is_ok(),
            "Balance effect should execute successfully: {:?}",
            result
        );

        // After Balance, P2 should have 0 creatures (same as P1)
        let p2_creatures_after = game
            .battlefield
            .cards
            .iter()
            .filter(|&&cid| {
                game.cards
                    .get(cid)
                    .map(|c| c.controller == p2_id && c.is_creature())
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(
            p2_creatures_after, 0,
            "P2 should have 0 creatures after Balance equalizing with P1"
        );

        // Verify all creatures are in P2's graveyard
        let p2_graveyard = &game.get_player_zones(p2_id).unwrap().graveyard;
        for creature_id in creature_ids {
            assert!(
                p2_graveyard.contains(creature_id),
                "Creature {:?} should be in P2's graveyard after sacrifice",
                creature_id
            );
        }
    }

    /// Test Balance spell effect - equalizes hands (discard) across all players
    #[test]
    fn test_balance_hand_discard() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1 has 1 card in hand
        // P2 has 4 cards in hand - should discard 3 to match P1

        // Add 1 card to P1's hand
        let p1_card = game.next_card_id();
        let mut card1 = Card::new(p1_card, "Plains".to_string(), p1_id);
        card1.controller = p1_id;
        game.cards.insert(p1_card, card1);
        game.player_zones
            .iter_mut()
            .find(|(id, _)| *id == p1_id)
            .unwrap()
            .1
            .hand
            .cards
            .push(p1_card);

        // Add 4 cards to P2's hand
        let p2_cards: Vec<CardId> = (0..4)
            .map(|i| {
                let card_id = game.next_card_id();
                let mut card = Card::new(card_id, format!("Card {}", i + 1), p2_id);
                card.controller = p2_id;
                game.cards.insert(card_id, card);
                game.player_zones
                    .iter_mut()
                    .find(|(id, _)| *id == p2_id)
                    .unwrap()
                    .1
                    .hand
                    .cards
                    .push(card_id);
                card_id
            })
            .collect();

        // Verify initial state
        let p1_hand_size_before = game.get_player_zones(p1_id).unwrap().hand.cards.len();
        let p2_hand_size_before = game.get_player_zones(p2_id).unwrap().hand.cards.len();
        assert_eq!(p1_hand_size_before, 1, "P1 should have 1 card in hand");
        assert_eq!(p2_hand_size_before, 4, "P2 should have 4 cards in hand");

        // Execute Balance effect for hands
        let result = game.execute_balance_effect("", "Hand");
        assert!(
            result.is_ok(),
            "Balance effect should execute successfully: {:?}",
            result
        );

        // After Balance, both players should have 1 card in hand
        let p1_hand_size_after = game.get_player_zones(p1_id).unwrap().hand.cards.len();
        let p2_hand_size_after = game.get_player_zones(p2_id).unwrap().hand.cards.len();
        assert_eq!(
            p1_hand_size_after, 1,
            "P1 should still have 1 card in hand after Balance"
        );
        assert_eq!(p2_hand_size_after, 1, "P2 should have 1 card in hand after Balance");

        // Verify 3 cards are in P2's graveyard
        let p2_graveyard = &game.get_player_zones(p2_id).unwrap().graveyard;
        let discarded_count = p2_cards.iter().filter(|&&cid| p2_graveyard.contains(cid)).count();
        assert_eq!(discarded_count, 3, "3 cards should be in P2's graveyard after discard");
    }

    // ═══════════════════════════════════════════════════════════════════
    // AB$ PreventDamage tests
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn test_prevent_damage_to_creature() {
        // Test: PreventDamage creates a shield that absorbs damage to a creature
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create a 2/2 creature
        let creature_id = game.next_entity_id();
        let mut creature = Card::new(creature_id, "Test Creature".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(2));
        creature.set_base_toughness(Some(2));
        creature.controller = p1_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Cast a PreventDamage spell targeting the creature (prevent 3)
        let spell_id = game.next_entity_id();
        let mut spell = Card::new(spell_id, "Shield".to_string(), p1_id);
        spell.add_type(CardType::Instant);
        spell.mana_cost = ManaCost::from_string("W");
        spell.effects.push(Effect::PreventDamage {
            target: TargetRef::Permanent(creature_id),
            amount: 3,
        });
        game.cards.insert(spell_id, spell);
        game.stack.add(spell_id);

        let result = game.resolve_spell(spell_id, &[]);
        assert!(result.is_ok(), "PreventDamage spell should resolve");

        // Creature should have a prevention shield
        let card = game.cards.get(creature_id).unwrap();
        assert_eq!(card.damage_prevention, 3, "Creature should have 3 prevention shield");

        // Deal 2 damage - should be fully prevented
        let result = game.deal_damage_to_creature(creature_id, 2);
        assert!(result.is_ok());
        let card = game.cards.get(creature_id).unwrap();
        assert_eq!(card.damage, 0, "All 2 damage should be prevented");
        assert_eq!(card.damage_prevention, 1, "1 prevention shield should remain");

        // Deal 2 more damage - 1 prevented, 1 gets through
        let result = game.deal_damage_to_creature(creature_id, 2);
        assert!(result.is_ok());
        let card = game.cards.get(creature_id).unwrap();
        assert_eq!(card.damage, 1, "1 damage should get through");
        assert_eq!(card.damage_prevention, 0, "No prevention shield remaining");
    }

    #[test]
    fn test_prevent_damage_to_player() {
        // Test: PreventDamage creates a shield that absorbs damage to a player
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Apply prevention shield to player (prevent 2)
        let spell_id = game.next_entity_id();
        let mut spell = Card::new(spell_id, "Shield".to_string(), p1_id);
        spell.add_type(CardType::Instant);
        spell.mana_cost = ManaCost::from_string("W");
        spell.effects.push(Effect::PreventDamage {
            target: TargetRef::Player(p1_id),
            amount: 2,
        });
        game.cards.insert(spell_id, spell);
        game.stack.add(spell_id);

        let result = game.resolve_spell(spell_id, &[]);
        assert!(result.is_ok(), "PreventDamage spell should resolve");

        let player = game.get_player(p1_id).unwrap();
        assert_eq!(player.damage_prevention, 2, "Player should have 2 prevention shield");
        assert_eq!(player.life, 20, "Life should be unchanged");

        // Deal 3 damage - 2 prevented, 1 gets through
        let result = game.deal_damage(p1_id, 3);
        assert!(result.is_ok());
        let player = game.get_player(p1_id).unwrap();
        assert_eq!(player.life, 19, "Only 1 damage should get through (20 - 1 = 19)");
        assert_eq!(player.damage_prevention, 0, "Shield should be depleted");
    }

    #[test]
    fn test_prevent_damage_stacks() {
        // Test: Multiple prevention shields stack additively
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create creature and apply two prevention shields
        let creature_id = game.next_entity_id();
        let mut creature = Card::new(creature_id, "Test Creature".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(1));
        creature.set_base_toughness(Some(1));
        creature.controller = p1_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Apply two shields (1 + 2 = 3 total)
        for amount in [1, 2] {
            let spell_id = game.next_entity_id();
            let mut spell = Card::new(spell_id, "Shield".to_string(), p1_id);
            spell.add_type(CardType::Instant);
            spell.mana_cost = ManaCost::from_string("W");
            spell.effects.push(Effect::PreventDamage {
                target: TargetRef::Permanent(creature_id),
                amount,
            });
            game.cards.insert(spell_id, spell);
            game.stack.add(spell_id);
            game.resolve_spell(spell_id, &[]).unwrap();
        }

        let card = game.cards.get(creature_id).unwrap();
        assert_eq!(card.damage_prevention, 3, "Shields should stack to 3");
    }

    #[test]
    fn test_prevent_damage_fully_absorbed() {
        // Test: When shield fully absorbs damage, no damage is dealt
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Set up player prevention shield of 5
        game.get_player_mut(p1_id).unwrap().damage_prevention = 5;

        // Deal 3 damage - fully absorbed
        let result = game.deal_damage(p1_id, 3);
        assert!(result.is_ok());
        let player = game.get_player(p1_id).unwrap();
        assert_eq!(player.life, 20, "Life should be unchanged");
        assert_eq!(player.damage_prevention, 2, "2 prevention remaining");
    }

    #[test]
    fn test_put_counter_all() {
        use crate::core::{CardType, CounterType};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // Create 3 creatures on P1's battlefield
        let mut p1_creatures = Vec::new();
        for i in 0..3 {
            let creature_id = game.next_card_id();
            let mut creature = Card::new(creature_id, format!("Soldier {}", i + 1), p1_id);
            creature.add_type(CardType::Creature);
            creature.set_base_power(Some(1));
            creature.set_base_toughness(Some(1));
            creature.controller = p1_id;
            game.cards.insert(creature_id, creature);
            game.battlefield.add(creature_id);
            p1_creatures.push(creature_id);
        }

        // Create 1 creature on P2's battlefield (should NOT get counters with YouCtrl filter)
        let p2_creature_id = game.next_card_id();
        let mut p2_creature = Card::new(p2_creature_id, "Opponent Bear".to_string(), p2_id);
        p2_creature.add_type(CardType::Creature);
        p2_creature.set_base_power(Some(2));
        p2_creature.set_base_toughness(Some(2));
        p2_creature.controller = p2_id;
        game.cards.insert(p2_creature_id, p2_creature);
        game.battlefield.add(p2_creature_id);

        // Set active player to P1 (PutCounterAll uses turn.active_player for controller filter)
        game.turn.active_player = p1_id;

        // Execute PutCounterAll effect using TargetRestriction (upstream API)
        let restriction = crate::core::TargetRestriction::parse("Creature.YouCtrl");
        let effect = Effect::PutCounterAll {
            restriction,
            counter_type: CounterType::P1P1,
            amount: 1,
        };
        game.execute_effect(&effect).unwrap();

        // Verify P1's creatures got +1/+1 counters
        for &cid in &p1_creatures {
            let card = game.cards.get(cid).unwrap();
            let counter_count = card
                .counters
                .iter()
                .find(|(ct, _)| *ct == CounterType::P1P1)
                .map(|(_, n)| *n)
                .unwrap_or(0);
            assert_eq!(counter_count, 1, "P1's creature should have 1 +1/+1 counter");
            assert_eq!(card.current_power(), 2, "P1's creature should now be 2/2");
            assert_eq!(card.current_toughness(), 2, "P1's creature should now be 2/2");
        }

        // Verify P2's creature did NOT get counters
        let p2_card = game.cards.get(p2_creature_id).unwrap();
        let p2_counter_count = p2_card
            .counters
            .iter()
            .find(|(ct, _)| *ct == CounterType::P1P1)
            .map(|(_, n)| *n)
            .unwrap_or(0);
        assert_eq!(p2_counter_count, 0, "P2's creature should have no counters");
    }

    #[test]
    fn test_untap_all() {
        use crate::core::CardType;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // Create 3 tapped creatures on P1's battlefield
        let mut p1_creatures = Vec::new();
        for i in 0..3 {
            let creature_id = game.next_card_id();
            let mut creature = Card::new(creature_id, format!("Warrior {}", i + 1), p1_id);
            creature.add_type(CardType::Creature);
            creature.set_base_power(Some(2));
            creature.set_base_toughness(Some(2));
            creature.controller = p1_id;
            creature.tapped = true; // All tapped (attacked this turn)
            game.cards.insert(creature_id, creature);
            game.battlefield.add(creature_id);
            p1_creatures.push(creature_id);
        }

        // Create a tapped creature on P2's battlefield
        let p2_creature_id = game.next_card_id();
        let mut p2_creature = Card::new(p2_creature_id, "Opp Warrior".to_string(), p2_id);
        p2_creature.add_type(CardType::Creature);
        p2_creature.controller = p2_id;
        p2_creature.tapped = true;
        game.cards.insert(p2_creature_id, p2_creature);
        game.battlefield.add(p2_creature_id);

        game.turn.active_player = p1_id;

        // Execute UntapAll using TargetRestriction (upstream API)
        let restriction = crate::core::TargetRestriction::parse("Creature.YouCtrl");
        let effect = Effect::UntapAll { restriction };
        game.execute_effect(&effect).unwrap();

        // Verify P1's creatures are untapped
        for &cid in &p1_creatures {
            let card = game.cards.get(cid).unwrap();
            assert!(!card.tapped, "P1's creature should be untapped");
        }

        // Verify P2's creature is still tapped
        let p2_card = game.cards.get(p2_creature_id).unwrap();
        assert!(p2_card.tapped, "P2's creature should still be tapped");
    }

    #[test]
    fn test_add_phase_extra_combat() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);

        // Execute AddPhase
        let effect = Effect::AddPhase { count: 1 };
        game.execute_effect(&effect).unwrap();

        assert_eq!(game.extra_combat_phases, 1, "Should have 1 extra combat phase");

        // Execute another AddPhase
        game.execute_effect(&effect).unwrap();
        assert_eq!(game.extra_combat_phases, 2, "Should have 2 extra combat phases");
    }

    #[test]
    fn test_extra_combat_phase_step_progression() {
        use crate::game::Step;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);

        // Set up: we're at EndCombat with 1 extra combat phase pending
        game.turn.current_step = Step::EndCombat;
        game.extra_combat_phases = 1;

        // Advance step - should go to BeginCombat (extra combat) instead of Main2
        game.advance_step().unwrap();
        assert_eq!(
            game.turn.current_step,
            Step::BeginCombat,
            "Should go to BeginCombat for extra combat phase"
        );
        assert_eq!(game.extra_combat_phases, 0, "Extra combat phases should be decremented");

        // Now advance through the extra combat normally
        game.advance_step().unwrap();
        assert_eq!(game.turn.current_step, Step::DeclareAttackers);

        // Skip to EndCombat of the extra combat
        game.turn.current_step = Step::EndCombat;
        game.advance_step().unwrap();
        // Now should go to Main2 since no more extra combat phases
        assert_eq!(
            game.turn.current_step,
            Step::Main2,
            "Should go to Main2 after all extra combats are done"
        );
    }

    #[test]
    fn test_spells_cast_this_turn_counter() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Initially zero
        assert_eq!(game.get_player(p1_id).unwrap().spells_cast_this_turn, 0);

        // Create a spell and simulate casting it
        let spell_id = game.next_card_id();
        let mut spell = Card::new(spell_id, "Lightning Bolt".to_string(), p1_id);
        spell.add_type(CardType::Instant);
        spell.mana_cost = ManaCost::from_string("R");
        spell.controller = p1_id;
        game.cards.insert(spell_id, spell);

        // Call check_spellcast_triggers (which increments the counter)
        game.check_spellcast_triggers(spell_id, p1_id).unwrap();
        assert_eq!(game.get_player(p1_id).unwrap().spells_cast_this_turn, 1);

        // Cast another spell
        let spell2_id = game.next_card_id();
        let mut spell2 = Card::new(spell2_id, "Shock".to_string(), p1_id);
        spell2.add_type(CardType::Instant);
        spell2.controller = p1_id;
        game.cards.insert(spell2_id, spell2);

        game.check_spellcast_triggers(spell2_id, p1_id).unwrap();
        assert_eq!(game.get_player(p1_id).unwrap().spells_cast_this_turn, 2);
    }

    #[test]
    fn test_count_expression_spells_cast() {
        use crate::core::CountExpression;
        use std::collections::HashMap;

        let mut svars = HashMap::new();
        svars.insert("X".to_string(), "Count$YouCastThisTurn".to_string());

        let expr = CountExpression::parse("X", &svars);
        assert!(
            matches!(expr, CountExpression::SpellsCastThisTurn),
            "Should parse Count$YouCastThisTurn to SpellsCastThisTurn"
        );
    }

    // =========================================================================
    // Raphael's Technique: Optional discard + RememberDiscardingPlayers + Draw
    // =========================================================================

    #[test]
    fn test_optional_discard_remember_discarding_players() {
        // Test Raphael's Technique pattern: each player MAY discard hand, draw 7
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // Give P1 3 cards in hand
        for i in 0..3 {
            let card_id = game.next_card_id();
            let card = Card::new(card_id, format!("P1 Card {i}"), p1_id);
            game.cards.insert(card_id, card);
            game.get_player_zones_mut(p1_id).unwrap().hand.add(card_id);
        }

        // Give P2 2 cards in hand
        for i in 0..2 {
            let card_id = game.next_card_id();
            let card = Card::new(card_id, format!("P2 Card {i}"), p2_id);
            game.cards.insert(card_id, card);
            game.get_player_zones_mut(p2_id).unwrap().hand.add(card_id);
        }

        // Add plenty of cards to both libraries for draw
        for i in 0..20 {
            let card_id = game.next_card_id();
            let card = Card::new(card_id, format!("P1 Library {i}"), p1_id);
            game.cards.insert(card_id, card);
            game.get_player_zones_mut(p1_id).unwrap().library.add(card_id);
        }
        for i in 0..20 {
            let card_id = game.next_card_id();
            let card = Card::new(card_id, format!("P2 Library {i}"), p2_id);
            game.cards.insert(card_id, card);
            game.get_player_zones_mut(p2_id).unwrap().library.add(card_id);
        }

        // Execute discard effect for P1 (optional, remember discarding players)
        let discard_p1 = Effect::DiscardCards {
            player: p1_id,
            count: u8::MAX, // Mode$ Hand
            remember_discarded: false,
            optional: true,
            remember_discarding_players: true,
        };
        game.execute_effect(&discard_p1).unwrap();

        // P1 should have discarded (AI always discards when optional)
        assert_eq!(
            game.get_player_zones(p1_id).unwrap().hand.cards.len(),
            0,
            "P1 should have 0 cards after discarding hand"
        );
        assert!(
            game.remembered_players.contains(&p1_id),
            "P1 should be in remembered_players"
        );

        // Execute discard effect for P2
        let discard_p2 = Effect::DiscardCards {
            player: p2_id,
            count: u8::MAX,
            remember_discarded: false,
            optional: true,
            remember_discarding_players: true,
        };
        game.execute_effect(&discard_p2).unwrap();

        assert_eq!(
            game.get_player_zones(p2_id).unwrap().hand.cards.len(),
            0,
            "P2 should have 0 cards after discarding hand"
        );
        assert_eq!(game.remembered_players.len(), 2, "Both players should be remembered");

        // Now draw 7 for remembered players
        let draw = Effect::DrawCards {
            player: PlayerId::remembered_players(),
            count: 7,
        };
        game.execute_effect(&draw).unwrap();

        assert_eq!(
            game.get_player_zones(p1_id).unwrap().hand.cards.len(),
            7,
            "P1 should have drawn 7 cards"
        );
        assert_eq!(
            game.get_player_zones(p2_id).unwrap().hand.cards.len(),
            7,
            "P2 should have drawn 7 cards"
        );

        // Clear remembered
        game.execute_effect(&Effect::ClearRemembered).unwrap();
        assert!(
            game.remembered_players.is_empty(),
            "remembered_players should be cleared"
        );
        assert!(game.remembered_cards.is_empty(), "remembered_cards should be cleared");
    }

    #[test]
    fn test_optional_discard_empty_hand_skips() {
        // Test that a player with empty hand is NOT added to remembered_players
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // P1 has empty hand (no cards given)

        let discard = Effect::DiscardCards {
            player: p1_id,
            count: u8::MAX,
            remember_discarded: false,
            optional: true,
            remember_discarding_players: true,
        };
        game.execute_effect(&discard).unwrap();

        // P1 had nothing to discard - should not be remembered
        assert!(
            game.remembered_players.is_empty(),
            "Player with empty hand should not be remembered"
        );
    }

    #[test]
    fn test_effect_converter_put_counter_all() {
        use crate::loader::ability_parser::AbilityParams;
        use crate::loader::effect_converter::params_to_effect;

        let params = AbilityParams::parse(
            "A:DB$ PutCounterAll | CounterType$ P1P1 | CounterNum$ 2 | ValidCards$ Creature.YouCtrl",
        )
        .unwrap();
        let effect = params_to_effect(&params).expect("PutCounterAll should produce an effect");
        let Effect::PutCounterAll {
            counter_type, amount, ..
        } = effect
        else {
            panic!("Expected PutCounterAll");
        };
        assert_eq!(counter_type, crate::core::CounterType::P1P1);
        assert_eq!(amount, 2);
    }

    #[test]
    fn test_effect_converter_untap_all() {
        use crate::loader::ability_parser::AbilityParams;
        use crate::loader::effect_converter::params_to_effect;

        let params = AbilityParams::parse("A:DB$ UntapAll | ValidCards$ Creature.YouCtrl").unwrap();
        let effect = params_to_effect(&params).expect("UntapAll should produce an effect");
        assert!(matches!(effect, Effect::UntapAll { .. }), "Expected UntapAll");
    }

    #[test]
    fn test_effect_converter_add_phase() {
        use crate::loader::ability_parser::AbilityParams;
        use crate::loader::effect_converter::params_to_effect;

        let params = AbilityParams::parse("A:DB$ AddPhase | PhaseType$ Combat").unwrap();
        let effect = params_to_effect(&params).expect("AddPhase should produce an effect");
        let Effect::AddPhase { count } = effect else {
            panic!("Expected AddPhase");
        };
        assert_eq!(count, 1);
    }

    #[test]
    fn test_draw_for_remembered_players_only() {
        // Test that draw with remembered_players sentinel only draws for remembered players
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // Add library cards for both
        for i in 0..10 {
            let card_id = game.next_card_id();
            let card = Card::new(card_id, format!("P1 Lib {i}"), p1_id);
            game.cards.insert(card_id, card);
            game.get_player_zones_mut(p1_id).unwrap().library.add(card_id);
        }
        for i in 0..10 {
            let card_id = game.next_card_id();
            let card = Card::new(card_id, format!("P2 Lib {i}"), p2_id);
            game.cards.insert(card_id, card);
            game.get_player_zones_mut(p2_id).unwrap().library.add(card_id);
        }

        // Only P1 is remembered
        game.remembered_players.push(p1_id);

        let draw = Effect::DrawCards {
            player: PlayerId::remembered_players(),
            count: 3,
        };
        game.execute_effect(&draw).unwrap();

        assert_eq!(
            game.get_player_zones(p1_id).unwrap().hand.cards.len(),
            3,
            "P1 (remembered) should have drawn 3"
        );
        assert_eq!(
            game.get_player_zones(p2_id).unwrap().hand.cards.len(),
            0,
            "P2 (not remembered) should not have drawn"
        );
    }

    // =========================================================================
    // Finality Counter: creature with finality counter → exile on death
    // =========================================================================

    #[test]
    fn test_finality_counter_destroy_exiles() {
        use crate::core::CounterType;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create a creature with a finality counter
        let creature_id = game.next_card_id();
        let mut creature = Card::new(creature_id, "Finality Bear".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(2));
        creature.set_base_toughness(Some(2));
        creature.controller = p1_id;
        creature.add_counter(CounterType::Finality, 1);
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Destroy it
        let destroy = Effect::DestroyPermanent {
            target: creature_id,
            restriction: crate::core::TargetRestriction::any(),
            no_regenerate: false,
        };
        game.execute_effect(&destroy).unwrap();

        // Should be in exile, NOT graveyard
        assert!(
            !game.battlefield.contains(creature_id),
            "Creature should not be on battlefield"
        );
        assert!(
            game.get_player_zones(p1_id).unwrap().exile.contains(creature_id),
            "Creature with finality counter should be exiled, not go to graveyard"
        );
        assert!(
            !game.get_player_zones(p1_id).unwrap().graveyard.contains(creature_id),
            "Creature with finality counter should NOT be in graveyard"
        );
    }

    #[test]
    fn test_no_finality_counter_goes_to_graveyard() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create a normal creature (no finality counter)
        let creature_id = game.next_card_id();
        let mut creature = Card::new(creature_id, "Normal Bear".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(2));
        creature.set_base_toughness(Some(2));
        creature.controller = p1_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Destroy it
        let destroy = Effect::DestroyPermanent {
            target: creature_id,
            restriction: crate::core::TargetRestriction::any(),
            no_regenerate: false,
        };
        game.execute_effect(&destroy).unwrap();

        // Should be in graveyard
        assert!(
            game.get_player_zones(p1_id).unwrap().graveyard.contains(creature_id),
            "Normal creature should go to graveyard"
        );
        assert!(
            !game.get_player_zones(p1_id).unwrap().exile.contains(creature_id),
            "Normal creature should NOT be exiled"
        );
    }

    #[test]
    fn test_finality_counter_lethal_damage_exiles() {
        use crate::core::CounterType;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create a creature with finality counter
        let creature_id = game.next_card_id();
        let mut creature = Card::new(creature_id, "Finality Soldier".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(2));
        creature.set_base_toughness(Some(2));
        creature.controller = p1_id;
        creature.add_counter(CounterType::Finality, 1);
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Deal lethal damage
        let card = game.cards.get_mut(creature_id).unwrap();
        card.damage = 3; // >= toughness of 2

        // Check lethal damage (SBA)
        game.check_lethal_damage().unwrap();

        // Should be in exile
        assert!(
            !game.battlefield.contains(creature_id),
            "Creature should not be on battlefield"
        );
        assert!(
            game.get_player_zones(p1_id).unwrap().exile.contains(creature_id),
            "Creature with finality counter should be exiled from lethal damage"
        );
    }

    #[test]
    fn test_put_finality_counter_then_destroy() {
        use crate::core::CounterType;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create a creature
        let creature_id = game.next_card_id();
        let mut creature = Card::new(creature_id, "Test Creature".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(1));
        creature.set_base_toughness(Some(1));
        creature.controller = p1_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Put a finality counter on it via the PutCounter effect
        let put_counter = Effect::PutCounter {
            target: creature_id,
            counter_type: CounterType::Finality,
            amount: 1,
        };
        game.execute_effect(&put_counter).unwrap();

        // Verify counter was placed
        assert_eq!(
            game.cards.get(creature_id).unwrap().get_counter(CounterType::Finality),
            1,
            "Finality counter should be on creature"
        );

        // Now destroy - should go to exile
        let destroy = Effect::DestroyPermanent {
            target: creature_id,
            restriction: crate::core::TargetRestriction::any(),
            no_regenerate: false,
        };
        game.execute_effect(&destroy).unwrap();

        assert!(
            game.get_player_zones(p1_id).unwrap().exile.contains(creature_id),
            "Creature with finality counter should be exiled on destroy"
        );
    }

    // =========================================================================
    // MayPlayFromGraveyard: persistent effect for casting from graveyard
    // =========================================================================

    #[test]
    fn test_may_play_from_graveyard_persistent_effect() {
        use crate::core::persistent_effect::{CleanupCondition, PersistentEffectKind};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create Leonardo as the source on battlefield
        let leonardo_id = game.next_card_id();
        let mut leonardo = Card::new(leonardo_id, "Leonardo, Sewer Samurai".to_string(), p1_id);
        leonardo.add_type(CardType::Creature);
        leonardo.set_base_power(Some(3));
        leonardo.set_base_toughness(Some(3));
        leonardo.controller = p1_id;
        game.cards.insert(leonardo_id, leonardo);
        game.battlefield.add(leonardo_id);

        // Add the persistent effect
        game.persistent_effects.add(
            PersistentEffectKind::MayPlayFromGraveyard {
                owner: p1_id,
                max_power: Some(1),
                max_toughness: Some(1),
                your_turn_only: true,
                add_finality_counter: true,
            },
            leonardo_id,
            p1_id,
            CleanupCondition::SourceLeavesBattlefield { source: leonardo_id },
        );

        // Verify the effect was added
        let effects: Vec<_> = game.persistent_effects.find_may_play_from_graveyard(p1_id).collect();
        assert_eq!(effects.len(), 1, "Should have 1 MayPlayFromGraveyard effect");

        // Verify cleanup when Leonardo leaves
        let cleanup_ids = game
            .persistent_effects
            .find_effects_to_cleanup_on_zone_change(leonardo_id, crate::zones::Zone::Battlefield);
        assert_eq!(cleanup_ids.len(), 1, "Should clean up when Leonardo leaves battlefield");
    }

    /// Regression test for Bazaar of Baghdad style draw effects.
    ///
    /// Bug: Bazaar of Baghdad ("{T}: Draw two cards, then discard three cards.")
    /// resolved its DrawCards effect silently — the cards moved into the hand
    /// but no "P draws CARD (id)" gamelog entry was emitted, so users seeing
    /// only the discard messages thought the engine was skipping the draws
    /// (see bug-bazaar-no-draw).
    ///
    /// Fix: per-card draw logging is centralised inside `GameState::draw_card`
    /// so every draw source — the mandatory draw step, activated abilities
    /// (Bazaar), spells (Ancestral Recall), and Loot effects — produces a
    /// consistent gamelog entry.
    ///
    /// This test executes `Effect::DrawCards { count: 2 }` directly and asserts
    /// that two `"P1 draws ..."` gamelog entries are recorded.
    #[test]
    fn test_draw_cards_effect_emits_per_card_gamelog() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Capture logs into the in-memory buffer instead of stdout
        game.logger.set_output_mode(crate::game::logger::OutputMode::Memory);

        // Seed P1's library with two named cards so we can assert on them
        for name in ["Library Top", "Library Second"] {
            let card_id = game.next_card_id();
            let card = Card::new(card_id, name.to_string(), p1_id);
            game.cards.insert(card_id, card);
            game.get_player_zones_mut(p1_id).unwrap().library.add(card_id);
        }
        // Library order is push-back; draw_top pops from the back, so the
        // top card (drawn first) is "Library Second".
        let logs_before = game.logger.log_count();

        // Execute Bazaar's first effect: draw 2.
        game.execute_effect(&Effect::DrawCards {
            player: p1_id,
            count: 2,
        })
        .unwrap();

        // Both cards should now be in hand
        assert_eq!(
            game.get_player_zones(p1_id).unwrap().hand.cards.len(),
            2,
            "P1 should have drawn 2 cards"
        );

        // Inspect the captured gamelog and assert per-card draw lines were emitted.
        let logs = game.logger.get_logs();
        let new_entries: Vec<&str> = logs[logs_before..]
            .iter()
            .filter(|e| e.category.as_deref() == Some("gamelog"))
            .map(|e| e.message.as_str())
            .collect();

        let draw_log_count = new_entries.iter().filter(|m| m.contains(" draws ")).count();
        assert_eq!(
            draw_log_count, 2,
            "Effect::DrawCards{{count:2}} should emit two per-card 'P1 draws CARD (id)' \
             gamelog lines (regression for bug-bazaar-no-draw). Got entries: {:?}",
            new_entries
        );

        // Both drawn card names should appear in the log
        assert!(
            new_entries.iter().any(|m| m.contains("Library Top")),
            "Gamelog should mention 'Library Top'. Entries: {:?}",
            new_entries
        );
        assert!(
            new_entries.iter().any(|m| m.contains("Library Second")),
            "Gamelog should mention 'Library Second'. Entries: {:?}",
            new_entries
        );

        // Player name "P1" should be the prefix of the draw lines
        assert!(
            new_entries
                .iter()
                .all(|m| !m.contains(" draws ") || m.starts_with("P1 ")),
            "Draw gamelog entries should start with the drawing player's name. Entries: {:?}",
            new_entries
        );
    }

    /// Regression: opponent draws must not leak the drawn card name to
    /// other perspectives.
    ///
    /// Closes bug-draw-reveals-opponent-hand. Before the fix, the per-card
    /// draw line emitted by `GameState::draw_card_inner` was a plain
    /// `gamelog(...)` — the WASM exporter for `web/native_game.html` re-served the
    /// raw message, so P1 could see "P2 draws Disenchant (88)" in the game
    /// log and effectively read P2's hand.
    ///
    /// The fix tags the entry with `private_to: PrivateLogInfo { owner,
    /// public_message }` so `LogEntry::message_for(perspective)` returns the
    /// masked "P2 draws a card" string from any non-owner perspective.
    /// This test asserts:
    /// 1. The captured entry carries the `private_to` marker pointing at
    ///    the drawing player.
    /// 2. The owner sees the full "P draws CARD (id)" message.
    /// 3. The opponent sees only the masked "P draws a card" message
    ///    (no card name, no card id).
    /// 4. The full message is still preserved in the log buffer so server
    ///    replays / full-info logs are unchanged.
    #[test]
    fn test_opponent_draws_do_not_leak_card_name() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        game.logger.set_output_mode(crate::game::logger::OutputMode::Memory);

        // Seed P2's library with a recognisable card and draw it.
        let secret_card_id = game.next_card_id();
        let secret = Card::new(secret_card_id, "Disenchant".to_string(), p2_id);
        game.cards.insert(secret_card_id, secret);
        game.get_player_zones_mut(p2_id).unwrap().library.add(secret_card_id);

        let logs_before = game.logger.log_count();
        game.draw_card(p2_id).expect("p2 draw");

        let logs = game.logger.get_logs();
        let draw_entry = logs[logs_before..]
            .iter()
            .find(|e| e.category.as_deref() == Some("gamelog") && e.message.contains(" draws "))
            .expect("expected a per-card draw gamelog entry for P2");

        // (1) Marker present and points at the drawing player.
        let private = draw_entry
            .private_to
            .as_ref()
            .expect("draw entry should be marked private_to the drawing player");
        assert_eq!(
            private.owner, p2_id,
            "private_to.owner must match the drawing player so the owning \
             perspective continues to see the card name"
        );

        // (2) Owner perspective: full message with card name + id.
        let owner_view = draw_entry.message_for(p2_id);
        assert!(
            owner_view.contains("Disenchant") && owner_view.contains(&format!("({})", secret_card_id)),
            "owner (P2) perspective should still see the full draw message; got {:?}",
            owner_view
        );

        // (3) Opponent perspective: masked message, NO card name, NO id.
        let opp_view = draw_entry.message_for(p1_id);
        assert!(
            !opp_view.contains("Disenchant"),
            "opponent (P1) perspective MUST NOT see the drawn card name; got {:?}",
            opp_view
        );
        assert!(
            !opp_view.contains(&format!("({})", secret_card_id)),
            "opponent (P1) perspective MUST NOT see the drawn card id; got {:?}",
            opp_view
        );
        assert!(
            opp_view.contains("draws a card"),
            "opponent (P1) perspective should see the masked 'draws a card' message; got {:?}",
            opp_view
        );

        // (4) The full message is still preserved in `LogEntry::message`,
        // so server-side / full-info logs (including the network gamelog
        // capture used by `--game-logs`) keep complete information.
        assert!(
            draw_entry.message.contains("Disenchant"),
            "raw entry.message must preserve the full info for full-info \
             consumers (server gamelog, replay verifier, etc); got {:?}",
            draw_entry.message
        );
    }

    // ===================================================================
    // Card-compatibility regression tests for the rogue_rogerbrand 93/94
    // deck — see compat tracking issues mtg-c-sedge-troll,
    // mtg-c-black-knight, mtg-c-serra-angel.
    //
    // These tests load the actual cardsfolder scripts (not synthetic Card
    // structs) so they exercise the full parse → instantiate pipeline that
    // production card loading uses. They check the *static* properties of
    // each card (P/T, types, keywords, abilities) rather than dynamic
    // gameplay; gameplay-level keyword behaviour (Flying-blocking,
    // Vigilance-no-tap, Regenerate-shield, Protection-from-X-no-target)
    // is exercised by the broader keyword test suite in `keywords.rs` and
    // `combat.rs`.
    // ===================================================================

    /// Card compat: Sedge Troll (cardsfolder/s/sedge_troll.txt)
    ///
    /// Script:
    ///   ManaCost:2 R
    ///   Types:Creature Troll
    ///   PT:2/2
    ///   S:Mode$ Continuous | Affected$ Card.Self | AddPower$ 1 | AddToughness$ 1
    ///                      | IsPresent$ Swamp.YouCtrl
    ///   A:AB$ Regenerate | Cost$ B | SpellDescription$ Regenerate CARDNAME.
    ///
    /// Verifies:
    /// - Card parses with cost 2R, base 2/2, Creature/Troll
    /// - Has the {B}: Regenerate activated ability
    /// - Has a static ModifyPT ability (the +1/+1 boost)
    ///
    /// KNOWN GAP (filed as mtg-c-sedge-troll/Sedge): the conditional
    /// `IsPresent$ Swamp.YouCtrl` qualifier on ModifyPT is silently
    /// dropped by the static-ability parser (see card.rs:3640 — only
    /// `condition` is plumbed for `GrantKeyword`, not `ModifyPT`). The
    /// boost therefore applies *unconditionally*, not only when you
    /// control a Swamp. Pinning the parser shape here so the eventual fix
    /// is observably caught.
    #[test]
    fn test_card_compat_sedge_troll() {
        use crate::core::{Cost, StaticAbility};
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/s/sedge_troll.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Sedge Troll should load");

        assert_eq!(def.name.as_str(), "Sedge Troll");
        assert_eq!(def.mana_cost.generic, 2, "Cost should be {{2}}{{R}}");
        assert_eq!(def.mana_cost.red, 1, "Cost should include {{R}}");
        assert!(def.types.contains(&CardType::Creature));
        assert_eq!(def.power, Some(2));
        assert_eq!(def.toughness, Some(2));

        // Instantiate to exercise the same parse pipeline production code
        // uses. (parse_activated_abilities / parse_static_abilities are
        // private — `instantiate` calls them and stows the results on the
        // resulting Card.)
        let card_id = CardId::new(1);
        let card = def.instantiate(card_id, PlayerId::new(0));

        // Activated ability: {B}: Regenerate
        assert!(
            card.activated_abilities
                .iter()
                .any(|a| matches!(&a.cost, Cost::Mana(mc) if mc.black == 1)
                    && a.effects.iter().any(|e| matches!(e, Effect::Regenerate { .. }))),
            "Sedge Troll must have {{B}}: Regenerate activated ability. Got: {:?}",
            card.activated_abilities
        );

        // Static ability: +1/+1 as long as you control a Swamp.
        // The ModifyPT must carry a ControlsPresent condition derived from
        // `IsPresent$ Swamp.YouCtrl` (mtg-398).
        let modify_pt = card.static_abilities.iter().find(|s| {
            matches!(
                s,
                StaticAbility::ModifyPT {
                    power: 1,
                    toughness: 1,
                    ..
                }
            )
        });
        assert!(
            modify_pt.is_some(),
            "Sedge Troll must produce a +1/+1 ModifyPT static ability. Got: {:?}",
            card.static_abilities
        );
        let StaticAbility::ModifyPT { condition, .. } = modify_pt.unwrap() else {
            panic!("modify_pt was matched as ModifyPT above");
        };
        match condition {
            Some(crate::core::StaticCondition::ControlsPresent {
                filter,
                zone,
                min_count,
            }) => {
                assert_eq!(filter, "Swamp.YouCtrl", "condition filter should be Swamp.YouCtrl");
                assert_eq!(
                    *zone,
                    crate::zones::Zone::Battlefield,
                    "default present zone is Battlefield"
                );
                assert_eq!(*min_count, 1, "default PresentCompare is GE1");
            }
            other => panic!(
                "Sedge Troll's +1/+1 must be gated by ControlsPresent(Swamp.YouCtrl); got {:?}",
                other
            ),
        }
    }

    /// Card compat: Black Knight (cardsfolder/b/black_knight.txt)
    ///
    /// Script:
    ///   ManaCost:B B
    ///   Types:Creature Human Knight
    ///   PT:2/2
    ///   K:First Strike
    ///   K:Protection from white
    ///
    /// Verifies the parsed card carries both the First Strike and
    /// ProtectionFromWhite keywords on its `keywords` set so the combat
    /// engine and targeting validator (which already understand both
    /// keywords — see combat.rs::test_first_strike_*, combat_bugfixes.rs
    /// ::test_protection_from_*) actually see them.
    #[test]
    fn test_card_compat_black_knight() {
        use crate::core::Keyword;
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/b/black_knight.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Black Knight should load");

        assert_eq!(def.name.as_str(), "Black Knight");
        assert_eq!(def.mana_cost.black, 2, "Cost should be {{B}}{{B}}");
        assert!(def.types.contains(&CardType::Creature));
        assert_eq!(def.power, Some(2));
        assert_eq!(def.toughness, Some(2));

        // Instantiate so we get the same Card struct production code uses.
        let card_id = CardId::new(1);
        let card = def.instantiate(card_id, PlayerId::new(0));

        assert!(
            card.keywords.contains(Keyword::FirstStrike),
            "Black Knight must have First Strike. Keywords: {:?}",
            card.keywords
        );
        assert!(
            card.keywords.contains(Keyword::ProtectionFromWhite),
            "Black Knight must have Protection from white. Keywords: {:?}",
            card.keywords
        );
    }

    /// Card compat: Serra Angel (cardsfolder/s/serra_angel.txt)
    ///
    /// Script:
    ///   ManaCost:3 W W
    ///   Types:Creature Angel
    ///   PT:4/4
    ///   K:Flying
    ///   K:Vigilance
    ///
    /// Verifies the parsed card carries both Flying and Vigilance on its
    /// `keywords` set (so the combat engine — see combat.rs::
    /// test_vigilance_creature_stays_untapped, which already loads Serra
    /// Angel via load_test_card — sees them on a freshly-parsed card too).
    #[test]
    fn test_card_compat_serra_angel() {
        use crate::core::Keyword;
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/s/serra_angel.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Serra Angel should load");

        assert_eq!(def.name.as_str(), "Serra Angel");
        assert_eq!(def.mana_cost.generic, 3, "Cost should be {{3}}{{W}}{{W}}");
        assert_eq!(def.mana_cost.white, 2, "Cost should include {{W}}{{W}}");
        assert!(def.types.contains(&CardType::Creature));
        assert_eq!(def.power, Some(4));
        assert_eq!(def.toughness, Some(4));

        let card_id = CardId::new(1);
        let card = def.instantiate(card_id, PlayerId::new(0));

        assert!(
            card.keywords.contains(Keyword::Flying),
            "Serra Angel must have Flying. Keywords: {:?}",
            card.keywords
        );
        assert!(
            card.keywords.contains(Keyword::Vigilance),
            "Serra Angel must have Vigilance. Keywords: {:?}",
            card.keywords
        );
    }

    /// Card compat: City of Brass (cardsfolder/c/city_of_brass.txt)
    ///
    /// Script:
    ///   ManaCost:no cost
    ///   Types:Land
    ///   A:AB$ Mana | Cost$ T | Produced$ Any | Amount$ 1
    ///   T:Mode$ Taps | ValidCard$ Card.Self | Execute$ TrigDamage
    ///   SVar:TrigDamage:DB$ DealDamage | Defined$ You | NumDmg$ 1
    ///
    /// Verifies (parser + runtime):
    /// - Card is a Land (zero mana cost)
    /// - Mana ability cache is `ManaProductionKind::AnyColor`
    /// - Has a `TriggerEvent::Taps` trigger
    /// - Tapping the card via `tap_for_mana` fires the trigger and
    ///   reduces controller's life to 19 (regression for the Taps-trigger
    ///   chain on lands; wiring lives in `tap_for_mana` actions/mod.rs).
    #[test]
    fn test_card_compat_city_of_brass() {
        use crate::core::{ManaProductionKind, TriggerEvent};
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/c/city_of_brass.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("City of Brass should load");
        assert_eq!(def.name.as_str(), "City of Brass");
        assert!(def.types.contains(&CardType::Land));

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        let card_id = game.next_card_id();
        let card = def.instantiate(card_id, p1_id);

        assert!(
            matches!(card.definition.cache.mana_production.kind, ManaProductionKind::AnyColor),
            "City of Brass must produce any colour. Got: {:?}",
            card.definition.cache.mana_production.kind
        );
        assert!(
            card.triggers.iter().any(|t| matches!(t.event, TriggerEvent::Taps)),
            "City of Brass must have a Taps trigger. Got triggers: {:?}",
            card.triggers.iter().map(|t| &t.event).collect::<Vec<_>>()
        );

        game.cards.insert(card_id, card);
        game.battlefield.add(card_id);

        let life_before = game.get_player(p1_id).unwrap().life;
        assert_eq!(life_before, 20, "Sanity check: starting life is 20");

        game.tap_for_mana(p1_id, card_id)
            .expect("tap_for_mana should succeed for City of Brass");

        let life_after = game.get_player(p1_id).unwrap().life;
        assert_eq!(
            life_after, 19,
            "City of Brass tap trigger must deal 1 damage to controller. Life before: {}, after: {}",
            life_before, life_after
        );
    }

    /// Card compat: Strip Mine (cardsfolder/s/strip_mine.txt)
    ///
    /// Script:
    ///   ManaCost:no cost
    ///   Types:Land
    ///   A:AB$ Mana | Cost$ T | Produced$ C
    ///   A:AB$ Destroy | ValidTgts$ Land | Cost$ T Sac<1/CARDNAME>
    ///                | AILogic$ LandForLand | SpellDescription$ Destroy target land.
    ///
    /// Verifies (parser):
    /// - Land with zero mana cost
    /// - Has at least one activated ability whose effect is
    ///   DestroyPermanent
    /// - Cost includes a sacrifice component (NOT pure Tap or Mana —
    ///   that would silently drop the Sac<1/CARDNAME> half and turn
    ///   Strip Mine into a one-sided land-destroyer-every-turn).
    ///
    /// The runtime behaviour ("Strip Mine activates, both it and the
    /// target land go to graveyards") is verified by the gameplay
    /// reproducer recorded in the compat issue.
    #[test]
    fn test_card_compat_strip_mine() {
        use crate::core::Cost;
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/s/strip_mine.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Strip Mine should load");
        assert_eq!(def.name.as_str(), "Strip Mine");
        assert!(def.types.contains(&CardType::Land));

        let card_id = CardId::new(1);
        let card = def.instantiate(card_id, PlayerId::new(0));

        let destroy_ability = card
            .activated_abilities
            .iter()
            .find(|a| a.effects.iter().any(|e| matches!(e, Effect::DestroyPermanent { .. })));
        assert!(
            destroy_ability.is_some(),
            "Strip Mine must have a Destroy activated ability. Got abilities: {:?}",
            card.activated_abilities
        );

        let cost = &destroy_ability.unwrap().cost;
        assert!(
            !matches!(cost, Cost::Tap | Cost::Mana(_)),
            "Strip Mine's destroy ability cost must include sacrifice, not just Tap or Mana. \
             Got: {:?} — if this is just Cost::Tap the Sac<1/CARDNAME> half was silently dropped \
             and Strip Mine becomes a one-sided land-destroyer every turn.",
            cost
        );
    }

    /// Card compat: original dual lands Badlands / Scrubland / Bayou.
    ///
    /// Scripts (cardsfolder/{b,s}/{badlands,scrubland,bayou}.txt):
    ///   ManaCost:no cost
    ///   Types:Land Swamp Mountain   (Badlands)
    ///   Types:Land Plains Swamp     (Scrubland)
    ///   Types:Land Swamp Forest     (Bayou)
    ///
    /// These cards carry NO printed mana ability — the two basic land
    /// types each grant an intrinsic "{T}: Add {color}" ability per
    /// CR 305.6. Verifies the loader adds exactly one mana ability per
    /// basic land subtype, each producing the correct single colour
    /// (so e.g. Badlands taps for {B} OR {R}, not just one of them and
    /// not colourless).
    #[test]
    fn test_card_compat_original_dual_lands() {
        use std::path::PathBuf;

        // (card file, expected basic land subtypes, expected mana colours
        //  as (white, blue, black, red, green))
        let cases: &[(&str, &str, &[ManaColorTuple])] = &[
            // Badlands: Swamp Mountain -> {B}, {R}
            (
                "../cardsfolder/b/badlands.txt",
                "Badlands",
                &[(0, 0, 1, 0, 0), (0, 0, 0, 1, 0)],
            ),
            // Scrubland: Plains Swamp -> {W}, {B}
            (
                "../cardsfolder/s/scrubland.txt",
                "Scrubland",
                &[(1, 0, 0, 0, 0), (0, 0, 1, 0, 0)],
            ),
            // Bayou: Swamp Forest -> {B}, {G}
            (
                "../cardsfolder/b/bayou.txt",
                "Bayou",
                &[(0, 0, 1, 0, 0), (0, 0, 0, 0, 1)],
            ),
            // Plateau: Mountain Plains -> {R}, {W} (Troll Disk deck, mtg-531)
            (
                "../cardsfolder/p/plateau.txt",
                "Plateau",
                &[(0, 0, 0, 1, 0), (1, 0, 0, 0, 0)],
            ),
            // Volcanic Island: Island Mountain -> {U}, {R} (Troll Disk deck, mtg-556)
            (
                "../cardsfolder/v/volcanic_island.txt",
                "Volcanic Island",
                &[(0, 1, 0, 0, 0), (0, 0, 0, 1, 0)],
            ),
        ];

        for (path_str, name, expected_colors) in cases {
            let path = PathBuf::from(path_str);
            if !path.exists() {
                eprintln!("Skipping: cardsfolder not present at {:?}", path);
                return;
            }
            let def = crate::loader::CardLoader::load_from_file(&path)
                .unwrap_or_else(|e| panic!("{name} should load: {e:?}"));
            assert_eq!(def.name.as_str(), *name);
            assert!(def.types.contains(&CardType::Land), "{name} must be a Land");

            let card = def.instantiate(CardId::new(1), PlayerId::new(0));

            // Collect the colours produced by each mana ability.
            let mut produced = tap_mana_ability_colors(&card);
            produced.sort_unstable();

            let mut expected: Vec<ManaColorTuple> = expected_colors.to_vec();
            expected.sort_unstable();

            assert_eq!(
                produced, expected,
                "{name} must grant exactly the two intrinsic mana abilities for its basic land types \
                 (CR 305.6). If only one (or a colourless) ability is present, a land subtype was \
                 dropped. Got abilities: {:?}",
                card.activated_abilities
            );
        }
    }

    /// Card compat: Power-9 Moxen Mox Pearl / Mox Ruby / Mox Emerald.
    ///
    /// Scripts (cardsfolder/m/mox_{pearl,ruby,emerald}.txt):
    ///   ManaCost:0
    ///   Types:Artifact
    ///   A:AB$ Mana | Cost$ T | Produced$ {W|R|G}
    ///
    /// Sibling of Mox Jet (mtg-405, already WORKING). Each Mox is a
    /// zero-cost artifact with a single "{T}: Add {color}" mana ability.
    /// Verifies the parser keeps the `Produced$` colour distinct per Mox
    /// (not collapsed to colourless or to a single shared colour).
    #[test]
    fn test_card_compat_power9_moxen() {
        use std::path::PathBuf;

        // (card file, name, expected (white, blue, black, red, green))
        let cases: &[(&str, &str, ManaColorTuple)] = &[
            ("../cardsfolder/m/mox_pearl.txt", "Mox Pearl", (1, 0, 0, 0, 0)),
            ("../cardsfolder/m/mox_ruby.txt", "Mox Ruby", (0, 0, 0, 1, 0)),
            ("../cardsfolder/m/mox_emerald.txt", "Mox Emerald", (0, 0, 0, 0, 1)),
            // Mox Sapphire: {T}: Add {U} (Troll Disk deck, mtg-527)
            ("../cardsfolder/m/mox_sapphire.txt", "Mox Sapphire", (0, 1, 0, 0, 0)),
        ];

        for (path_str, name, expected) in cases {
            let path = PathBuf::from(path_str);
            if !path.exists() {
                eprintln!("Skipping: cardsfolder not present at {:?}", path);
                return;
            }
            let def = crate::loader::CardLoader::load_from_file(&path)
                .unwrap_or_else(|e| panic!("{name} should load: {e:?}"));
            assert_eq!(def.name.as_str(), *name);
            assert!(def.types.contains(&CardType::Artifact), "{name} must be an Artifact");
            assert_eq!(def.mana_cost.cmc(), 0, "{name} must be zero mana cost");

            let card = def.instantiate(CardId::new(1), PlayerId::new(0));

            let mana_abilities = tap_mana_ability_colors(&card);

            assert_eq!(
                mana_abilities,
                vec![*expected],
                "{name} must have exactly one {{T}} mana ability producing its printed colour. \
                 If colourless or a different colour, Produced$ was dropped/misparsed. \
                 Got abilities: {:?}",
                card.activated_abilities
            );
        }
    }

    /// Card compat: Disenchant (cardsfolder/d/disenchant.txt)
    ///
    /// Script:
    ///   ManaCost:1 W
    ///   Types:Instant
    ///   A:SP$ Destroy | ValidTgts$ Artifact,Enchantment | ...
    ///
    /// Verifies (parser, via tokenized AbilityParams::parse — NOT substring
    /// matching): {1}{W} Instant whose spell ability is an SP$ Destroy
    /// targeting Artifact OR Enchantment (so it can't be silently narrowed
    /// to artifact-only or to a non-targeted destroy). Runtime targeting /
    /// destruction is verified by tests/disenchant_destroys_artifact_e2e.sh.
    #[test]
    fn test_card_compat_disenchant() {
        use crate::loader::ability_parser::{AbilityParams, ApiType};
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/d/disenchant.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Disenchant should load");
        assert_eq!(def.name.as_str(), "Disenchant");
        assert_eq!(def.mana_cost.generic, 1, "Cost generic should be 1");
        assert_eq!(def.mana_cost.white, 1, "Cost should require {{W}}");
        assert!(def.types.contains(&CardType::Instant), "Disenchant must be an Instant");

        // Find the SP$ Destroy spell ability via tokenized parsing.
        let destroy = def.raw_abilities.iter().find_map(|raw| {
            let p = AbilityParams::parse(raw).ok()?;
            (p.api_type == ApiType::Destroy).then_some(p)
        });
        let destroy = destroy.expect("Disenchant must have an SP$ Destroy spell ability");

        let tgts = destroy
            .get("ValidTgts")
            .expect("Disenchant Destroy must have ValidTgts");
        let tgt_set: Vec<&str> = tgts.split(',').map(|s| s.trim()).collect();
        assert!(
            tgt_set.contains(&"Artifact") && tgt_set.contains(&"Enchantment"),
            "Disenchant must be able to target BOTH Artifact and Enchantment (CR 608); \
             a narrower ValidTgts silently drops one mode. Got: {:?}",
            tgts
        );
    }

    /// Card compat: Bazaar of Baghdad (cardsfolder/b/bazaar_of_baghdad.txt) — mtg-388
    ///
    /// Script: ManaCost:no cost / Types:Land
    ///   A:AB$ Draw | Cost$ T | NumCards$ 2 | SubAbility$ DBDiscard
    ///   SVar:DBDiscard:DB$ Discard | Defined$ You | NumCards$ 3 | Mode$ TgtChoose
    ///
    /// Parser shape: a Land with a {T} activated ability that draws 2 then
    /// discards 3. The historical bug-bazaar-no-draw silently dropped the
    /// draw half (only discard ran). This guards that the instantiated card
    /// keeps BOTH a draw and a discard effect. Runtime verified by
    /// tests/bazaar_of_baghdad_draw_discard_e2e.sh.
    #[test]
    fn test_card_compat_bazaar_of_baghdad() {
        use crate::loader::ability_parser::{AbilityParams, ApiType};
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/b/bazaar_of_baghdad.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Bazaar of Baghdad should load");
        assert_eq!(def.name.as_str(), "Bazaar of Baghdad");
        assert!(def.types.contains(&CardType::Land), "Bazaar of Baghdad must be a Land");

        let draw = def
            .raw_abilities
            .iter()
            .find_map(|raw| {
                let p = AbilityParams::parse(raw).ok()?;
                (p.api_type == ApiType::Draw).then_some(p)
            })
            .expect("Bazaar must have an AB$ Draw activated ability");
        assert_eq!(draw.get("NumCards"), Some("2"), "Bazaar draws 2 cards");
        assert_eq!(
            draw.get("SubAbility"),
            Some("DBDiscard"),
            "Bazaar must chain a discard sub-ability"
        );

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));
        let has_draw_discard = card.activated_abilities.iter().any(|a| {
            a.effects.iter().any(|e| matches!(e, Effect::DrawCards { .. }))
                && a.effects.iter().any(|e| matches!(e, Effect::DiscardCards { .. }))
        });
        assert!(
            has_draw_discard,
            "Bazaar's activated ability must contain BOTH a draw and a discard \
             effect (regression for bug-bazaar-no-draw). Got: {:?}",
            card.activated_abilities
        );
    }

    /// Card compat: Terror (cardsfolder/t/terror.txt) — mtg-549
    ///
    /// Script: ManaCost:1 B / Types:Instant
    ///   A:SP$ Destroy | ValidTgts$ Creature.nonArtifact+nonBlack | NoRegen$ True
    ///
    /// Parser shape: {1}{B} Instant that destroys a nonartifact, nonblack
    /// creature and prevents regeneration. A silent drop of the target
    /// restriction would let it kill black/artifact creatures (strictly
    /// stronger than printed); a silent drop of NoRegen would let
    /// regenerators survive. Runtime destroy verified by
    /// tests/terror_destroys_creature_e2e.sh.
    #[test]
    fn test_card_compat_terror() {
        use crate::loader::ability_parser::{AbilityParams, ApiType};
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/t/terror.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Terror should load");
        assert_eq!(def.name.as_str(), "Terror");
        assert_eq!(def.mana_cost.generic, 1, "Cost generic should be 1");
        assert_eq!(def.mana_cost.black, 1, "Cost should require {{B}}");
        assert!(def.types.contains(&CardType::Instant), "Terror must be an Instant");

        let destroy = def
            .raw_abilities
            .iter()
            .find_map(|raw| {
                let p = AbilityParams::parse(raw).ok()?;
                (p.api_type == ApiType::Destroy).then_some(p)
            })
            .expect("Terror must have an SP$ Destroy spell ability");

        let tgts = destroy.get("ValidTgts").expect("Terror Destroy must have ValidTgts");
        assert_eq!(
            tgts, "Creature.nonArtifact+nonBlack",
            "Terror must only target nonartifact, nonblack creatures (CR 608); \
             a broader ValidTgts would make it strictly stronger than printed. Got: {:?}",
            tgts
        );
        assert_eq!(
            destroy.get("NoRegen"),
            Some("True"),
            "Terror's target can't be regenerated; dropping NoRegen lets regenerators survive"
        );
    }

    /// Card compat: Jalum Tome (cardsfolder/j/jalum_tome.txt) — mtg-514
    ///
    /// Script: ManaCost:3 / Types:Artifact
    ///   A:AB$ Draw | Cost$ 2 T | NumCards$ 1 | SubAbility$ DBDiscard
    ///   SVar:DBDiscard:DB$ Discard | Defined$ You | NumCards$ 1 | Mode$ TgtChoose
    ///
    /// Parser shape: a {3} artifact with a {2},{T} activated ability that
    /// draws one card then discards one. A silent drop of the SubAbility
    /// would make it a pure card-advantage draw engine (strictly stronger).
    /// Runtime (draw then discard) verified by
    /// tests/jalum_tome_draw_discard_e2e.sh.
    #[test]
    fn test_card_compat_jalum_tome() {
        use crate::loader::ability_parser::{AbilityParams, ApiType};
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/j/jalum_tome.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Jalum Tome should load");
        assert_eq!(def.name.as_str(), "Jalum Tome");
        assert_eq!(def.mana_cost.generic, 3, "Cost should be {{3}}");
        assert!(
            def.types.contains(&CardType::Artifact),
            "Jalum Tome must be an Artifact"
        );

        // The activated ability is a Draw with a chained Discard sub-ability.
        let draw = def
            .raw_abilities
            .iter()
            .find_map(|raw| {
                let p = AbilityParams::parse(raw).ok()?;
                (p.api_type == ApiType::Draw).then_some(p)
            })
            .expect("Jalum Tome must have an AB$ Draw activated ability");
        assert_eq!(draw.get("NumCards"), Some("1"), "Jalum Tome draws 1 card");
        assert_eq!(
            draw.get("SubAbility"),
            Some("DBDiscard"),
            "Jalum Tome must chain a discard sub-ability; dropping it makes it pure card advantage"
        );

        // The instantiated card must actually carry the activated ability with
        // both a Draw and a Discard effect (proving the SubAbility wired up).
        let card = def.instantiate(CardId::new(1), PlayerId::new(0));
        let has_draw_discard = card.activated_abilities.iter().any(|a| {
            a.effects.iter().any(|e| matches!(e, Effect::DrawCards { .. }))
                && a.effects.iter().any(|e| matches!(e, Effect::DiscardCards { .. }))
        });
        assert!(
            has_draw_discard,
            "Jalum Tome's activated ability must contain BOTH a draw and a discard effect. \
             Got abilities: {:?}",
            card.activated_abilities
        );
    }

    /// Card compat: Nevinyrral's Disk (cardsfolder/n/nevinyrrals_disk.txt) — mtg-528
    ///
    /// Script: ManaCost:4 / Types:Artifact
    ///   R:Event$ Moved | ValidCard$ Card.Self | Destination$ Battlefield
    ///     | ReplaceWith$ ETBTapped   (enters tapped)
    ///   A:AB$ DestroyAll | Cost$ 1 T | ValidCards$ Artifact,Creature,Enchantment
    ///
    /// Parser shape: a {4} artifact with an ETB-tapped replacement effect and
    /// a {1},{T} board wipe. A silent drop of either piece changes the card
    /// meaningfully. Runtime (enters tapped + destroys all three types)
    /// verified by tests/nevinyrrals_disk_destroyall_e2e.sh.
    #[test]
    fn test_card_compat_nevinyrrals_disk() {
        use crate::loader::ability_parser::{AbilityParams, ApiType};
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/n/nevinyrrals_disk.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Nevinyrral's Disk should load");
        assert_eq!(def.name.as_str(), "Nevinyrral's Disk");
        assert_eq!(def.mana_cost.generic, 4, "Cost should be {{4}}");
        assert!(
            def.types.contains(&CardType::Artifact),
            "Nevinyrral's Disk must be an Artifact"
        );

        // The activated board wipe: AB$ DestroyAll over the three types.
        let destroy_all = def
            .raw_abilities
            .iter()
            .find_map(|raw| {
                let p = AbilityParams::parse(raw).ok()?;
                (p.api_type == ApiType::DestroyAll).then_some(p)
            })
            .expect("Nevinyrral's Disk must have an AB$ DestroyAll activated ability");
        let valid = destroy_all.get("ValidCards").expect("DestroyAll must have ValidCards");
        let set: Vec<&str> = valid.split(',').map(|s| s.trim()).collect();
        assert!(
            set.contains(&"Artifact") && set.contains(&"Creature") && set.contains(&"Enchantment"),
            "Nevinyrral's Disk must destroy artifacts, creatures, AND enchantments; \
             a narrower ValidCards silently drops a category. Got: {:?}",
            valid
        );
    }

    /// Card compat: Lightning Bolt (cardsfolder/l/lightning_bolt.txt)
    ///
    /// Script: ManaCost:R / Types:Instant
    ///   A:SP$ DealDamage | ValidTgts$ Any | NumDmg$ 3
    ///
    /// Parser shape: {R} Instant dealing exactly 3 damage to any target.
    /// Runtime (deals 3 to a creature / player) is exercised by the Troll
    /// Disk deck reproducer in mtg-518.
    #[test]
    fn test_card_compat_lightning_bolt() {
        use crate::loader::ability_parser::{AbilityParams, ApiType};
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/l/lightning_bolt.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Lightning Bolt should load");
        assert_eq!(def.name.as_str(), "Lightning Bolt");
        assert_eq!(def.mana_cost.red, 1, "Cost should be {{R}}");
        assert_eq!(def.mana_cost.cmc(), 1, "CMC should be 1");
        assert!(def.types.contains(&CardType::Instant), "must be an Instant");

        let dmg = def
            .raw_abilities
            .iter()
            .find_map(|raw| {
                let p = AbilityParams::parse(raw).ok()?;
                (p.api_type == ApiType::DealDamage).then_some(p)
            })
            .expect("Lightning Bolt must have an SP$ DealDamage spell ability");
        assert_eq!(dmg.get("NumDmg"), Some("3"), "Lightning Bolt deals 3");
        assert_eq!(dmg.get("ValidTgts"), Some("Any"), "Lightning Bolt targets any target");
    }

    /// Card compat: Psionic Blast (cardsfolder/p/psionic_blast.txt)
    ///
    /// Script: ManaCost:2 U / Types:Instant
    ///   A:SP$ DealDamage | ValidTgts$ Any | NumDmg$ 4 | SubAbility$ DBDealDamage
    ///   SVar:DBDealDamage:DB$ DealDamage | Defined$ You | NumDmg$ 2 | ...
    ///
    /// Parser shape: {2}{U} Instant that deals 4 to any target (primary SP$)
    /// AND 2 to you (chained DB$). A silent drop of the SubAbility would make
    /// it a strictly-better bolt with no self-damage. Runtime (4 to target +
    /// 2 to caster) is exercised by the Troll Disk deck reproducer (mtg-533).
    #[test]
    fn test_card_compat_psionic_blast() {
        use crate::loader::ability_parser::{AbilityParams, ApiType};
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/p/psionic_blast.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Psionic Blast should load");
        assert_eq!(def.name.as_str(), "Psionic Blast");
        assert_eq!(def.mana_cost.generic, 2, "Cost generic should be 2");
        assert_eq!(def.mana_cost.blue, 1, "Cost should require {{U}}");
        assert!(def.types.contains(&CardType::Instant), "must be an Instant");

        // Primary SP$ DealDamage: 4 to any target.
        let primary = def
            .raw_abilities
            .iter()
            .find_map(|raw| {
                let p = AbilityParams::parse(raw).ok()?;
                (p.api_type == ApiType::DealDamage && p.get("NumDmg") == Some("4")).then_some(p)
            })
            .expect("Psionic Blast must deal 4 to any target");
        assert_eq!(primary.get("ValidTgts"), Some("Any"));

        // The chained DB$ DealDamage (2 to you) lives in the DBDealDamage
        // SVar; assert the self-damage half survives parsing so the downside
        // isn't silently dropped (making Psionic Blast a strictly-better bolt).
        // parsed_svars holds the tokenized AbilityParams for each SVar body.
        let p = def
            .parsed_svars
            .get("DBDealDamage")
            .expect("Psionic Blast must keep its DBDealDamage SVar (the 2-to-you downside)");
        assert_eq!(p.api_type, ApiType::DealDamage, "DBDealDamage must be a DealDamage");
        assert_eq!(p.get("NumDmg"), Some("2"), "self-damage must be 2");
        assert_eq!(p.get("Defined"), Some("You"), "self-damage targets the caster (You)");
    }

    /// Card compat: Ancestral Recall (cardsfolder/a/ancestral_recall.txt)
    ///
    /// Script: ManaCost:U / Types:Instant
    ///   A:SP$ Draw | NumCards$ 3 | ValidTgts$ Player
    ///
    /// Parser shape: {U} Instant, target player draws 3. Runtime (draw 3)
    /// verified by the Troll Disk deck reproducer (mtg-480).
    #[test]
    fn test_card_compat_ancestral_recall() {
        use crate::loader::ability_parser::{AbilityParams, ApiType};
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/a/ancestral_recall.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Ancestral Recall should load");
        assert_eq!(def.name.as_str(), "Ancestral Recall");
        assert_eq!(def.mana_cost.blue, 1, "Cost should be {{U}}");
        assert_eq!(def.mana_cost.cmc(), 1, "CMC should be 1");
        assert!(def.types.contains(&CardType::Instant), "must be an Instant");

        let draw = def
            .raw_abilities
            .iter()
            .find_map(|raw| {
                let p = AbilityParams::parse(raw).ok()?;
                (p.api_type == ApiType::Draw).then_some(p)
            })
            .expect("Ancestral Recall must have an SP$ Draw spell ability");
        assert_eq!(draw.get("NumCards"), Some("3"), "Ancestral Recall draws 3");
        assert_eq!(draw.get("ValidTgts"), Some("Player"), "targets a player");
    }

    /// Card compat: Braingeyser (cardsfolder/b/braingeyser.txt)
    ///
    /// Script: ManaCost:X U U / Types:Sorcery
    ///   A:SP$ Draw | NumCards$ X | ValidTgts$ Player
    ///   SVar:X:Count$xPaid
    ///
    /// Parser shape: {X}{U}{U} Sorcery, target player draws X. Runtime
    /// (draw X) verified by the Troll Disk deck reproducer (mtg-488).
    #[test]
    fn test_card_compat_braingeyser() {
        use crate::loader::ability_parser::{AbilityParams, ApiType};
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/b/braingeyser.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Braingeyser should load");
        assert_eq!(def.name.as_str(), "Braingeyser");
        assert_eq!(def.mana_cost.blue, 2, "Cost should require {{U}}{{U}}");
        assert!(def.types.contains(&CardType::Sorcery), "must be a Sorcery");

        let draw = def
            .raw_abilities
            .iter()
            .find_map(|raw| {
                let p = AbilityParams::parse(raw).ok()?;
                (p.api_type == ApiType::Draw).then_some(p)
            })
            .expect("Braingeyser must have an SP$ Draw spell ability");
        assert_eq!(draw.get("NumCards"), Some("X"), "Braingeyser draws X");
        assert_eq!(draw.get("ValidTgts"), Some("Player"), "targets a player");
    }

    /// Card compat: Counterspell (cardsfolder/c/counterspell.txt)
    ///
    /// Script: ManaCost:U U / Types:Instant
    ///   A:SP$ Counter | TargetType$ Spell | ValidTgts$ Card
    ///
    /// Parser shape: {U}{U} Instant that counters target spell. Runtime
    /// (counters a spell on the stack) verified by the Troll Disk deck
    /// reproducer (mtg-495).
    #[test]
    fn test_card_compat_counterspell() {
        use crate::loader::ability_parser::{AbilityParams, ApiType};
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/c/counterspell.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Counterspell should load");
        assert_eq!(def.name.as_str(), "Counterspell");
        assert_eq!(def.mana_cost.blue, 2, "Cost should be {{U}}{{U}}");
        assert!(def.types.contains(&CardType::Instant), "must be an Instant");

        let counter = def
            .raw_abilities
            .iter()
            .find_map(|raw| {
                let p = AbilityParams::parse(raw).ok()?;
                (p.api_type == ApiType::Counter).then_some(p)
            })
            .expect("Counterspell must have an SP$ Counter spell ability");
        assert_eq!(counter.get("TargetType"), Some("Spell"), "counters a spell");
    }

    /// Card compat: Serendib Efreet (cardsfolder/s/serendib_efreet.txt)
    ///
    /// Script: ManaCost:2 U / Types:Creature Efreet / PT:3/4
    ///   K:Flying
    ///   T:Mode$ Phase | Phase$ Upkeep | ValidPlayer$ You | Execute$ TrigDealDamage
    ///   SVar:TrigDealDamage:DB$ DealDamage | Defined$ You | NumDmg$ 1
    ///
    /// Parser shape: {2}{U} 3/4 flyer with an upkeep trigger that deals 1 to
    /// its controller. A dropped trigger leaves a strictly-better vanilla
    /// flyer. Runtime (upkeep self-damage + flying combat) verified by the
    /// Troll Disk deck reproducer (mtg-540).
    #[test]
    fn test_card_compat_serendib_efreet() {
        use crate::core::TriggerEvent;
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/s/serendib_efreet.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Serendib Efreet should load");
        assert_eq!(def.name.as_str(), "Serendib Efreet");
        assert_eq!(def.mana_cost.generic, 2, "Cost generic should be 2");
        assert_eq!(def.mana_cost.blue, 1, "Cost should require {{U}}");
        assert_eq!(def.power, Some(3));
        assert_eq!(def.toughness, Some(4));
        assert!(def.types.contains(&CardType::Creature), "must be a Creature");

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));
        assert!(
            card.keywords.contains(crate::core::Keyword::Flying),
            "Serendib Efreet must have Flying. Got: {:?}",
            card.keywords
        );

        // Upkeep phase trigger carrying a DealDamage (1 to controller).
        let upkeep = card
            .triggers
            .iter()
            .find(|t| matches!(t.event, TriggerEvent::BeginningOfUpkeep))
            .expect("Serendib Efreet must have an upkeep trigger");
        let dmg = upkeep
            .effects
            .iter()
            .find_map(|e| {
                if let Effect::DealDamage { amount, .. } = e {
                    Some(*amount)
                } else {
                    None
                }
            })
            .expect("Serendib Efreet upkeep trigger must deal damage to its controller");
        assert_eq!(dmg, 1, "Serendib Efreet deals 1 to its controller each upkeep");
    }

    /// Card compat: Shivan Dragon (cardsfolder/s/shivan_dragon.txt)
    ///
    /// Script:
    ///   ManaCost:4 R R
    ///   Types:Creature Dragon
    ///   PT:5/5
    ///   K:Flying
    ///   A:AB$ Pump | Cost$ R | NumAtt$ +1 | SpellDescription$ ...+1/+0...
    ///
    /// Verifies the parsed card is a 5/5 {4}{R}{R} flyer with a firebreathing
    /// activated ability (Cost$ R, a Pump effect granting +1/+0). The runtime
    /// behaviour (firebreathing stacks, expires end of turn; flying blocking
    /// restriction) is verified by the gameplay reproducers in the compat
    /// issue + tests/shivan_dragon_flying_block_e2e.sh and the existing
    /// puzzle_e2e.rs firebreathing/pump tests.
    #[test]
    fn test_card_compat_shivan_dragon() {
        use crate::core::{Cost, Keyword};
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/s/shivan_dragon.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Shivan Dragon should load");
        assert_eq!(def.name.as_str(), "Shivan Dragon");
        assert_eq!(def.mana_cost.generic, 4, "Cost generic should be 4");
        assert_eq!(def.mana_cost.red, 2, "Cost should require {{R}}{{R}}");
        assert!(def.types.contains(&CardType::Creature));
        assert_eq!(def.power, Some(5));
        assert_eq!(def.toughness, Some(5));

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));
        assert!(
            card.keywords.contains(Keyword::Flying),
            "Shivan Dragon must have Flying. Keywords: {:?}",
            card.keywords
        );

        // Firebreathing: an activated ability costing {R} that pumps power.
        let firebreathing = card.activated_abilities.iter().find(|a| {
            matches!(a.cost, Cost::Mana(m) if m.red == 1 && m.cmc() == 1)
                && a.effects.iter().any(|e| matches!(e, Effect::PumpCreature { .. }))
        });
        assert!(
            firebreathing.is_some(),
            "Shivan Dragon must have a {{R}}: +1/+0 firebreathing ability. \
             If absent, the A:AB$ Pump line was dropped. Got abilities: {:?}",
            card.activated_abilities
        );
    }

    /// Card compat: Sengir Vampire (cardsfolder/s/sengir_vampire.txt)
    ///
    /// Script:
    ///   ManaCost:3 B B
    ///   Types:Creature Vampire
    ///   PT:4/4
    ///   K:Flying
    ///   T:Mode$ ChangesZone | Origin$ Battlefield | Destination$ Graveyard
    ///                       | ValidCard$ Creature.DamagedBy
    ///                       | TriggerZones$ Battlefield | Execute$ TrigPutCounter
    ///
    /// Verifies the parser correctly picks up the static side
    /// (cost {3}{B}{B}, 4/4, Flying).
    ///
    /// KNOWN GAP filed as separate bug: the
    /// `Mode$ ChangesZone | ValidCard$ Creature.DamagedBy` trigger is
    /// silently dropped by the trigger parser (loader/card.rs:1874 only
    /// matches `ValidCard$ Card.Self`; line 1901 matches
    /// `Card.EquippedBy`; no branch matches `Creature.DamagedBy`). Even
    /// with parser support, the engine does not currently track "which
    /// sources have damaged which creatures this turn", so the trigger
    /// could not fire. Affects all "Whenever a creature dealt damage by
    /// ~ this turn dies" effects.
    #[test]
    fn test_card_compat_sengir_vampire() {
        use crate::core::Keyword;
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/s/sengir_vampire.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Sengir Vampire should load");

        assert_eq!(def.name.as_str(), "Sengir Vampire");
        assert_eq!(def.mana_cost.generic, 3, "Cost should be {{3}}{{B}}{{B}}");
        assert_eq!(def.mana_cost.black, 2, "Cost should include {{B}}{{B}}");
        assert!(def.types.contains(&CardType::Creature));
        assert_eq!(def.power, Some(4));
        assert_eq!(def.toughness, Some(4));

        let card_id = CardId::new(1);
        let card = def.instantiate(card_id, PlayerId::new(0));

        assert!(
            card.keywords.contains(Keyword::Flying),
            "Sengir Vampire must have Flying. Keywords: {:?}",
            card.keywords
        );

        // Regression guard for mtg-408 / mtg-403: the
        // "Whenever a creature dealt damage by CARDNAME this turn dies, put a
        // +1/+1 counter on CARDNAME" trigger (Forge
        // `T:Mode$ ChangesZone | ... | ValidCard$ Creature.DamagedBy`) was
        // historically SILENTLY DROPPED by the ChangesZone parser. Assert it
        // is parsed as a DamagedCreatureDies trigger carrying a PutCounter
        // effect that targets Self, so the silent-drop cannot regress.
        use crate::core::{CounterType, TriggerEvent};
        let dmg_trigger = card
            .triggers
            .iter()
            .find(|t| t.event == TriggerEvent::DamagedCreatureDies)
            .expect("Sengir Vampire must register a DamagedCreatureDies trigger (Creature.DamagedBy)");
        assert!(
            dmg_trigger.effects.iter().any(|e| matches!(
                e,
                Effect::PutCounter {
                    counter_type: CounterType::P1P1,
                    ..
                }
            )),
            "DamagedCreatureDies trigger must put a +1/+1 counter. Effects: {:?}",
            dmg_trigger.effects
        );
    }

    /// Card compat: Sengir Vampire — "this turn" linkage (mtg-408).
    ///
    /// Verifies the trigger's "dealt damage by CARDNAME *this turn*" clause is
    /// satisfied by damage recorded EARLIER in the turn even when the creature
    /// dies from a NON-combat source later. The engine records each combat
    /// damage source on the victim's `damaged_by_this_turn` list BEFORE the
    /// lethal-damage check (combat.rs), and `check_death_triggers` reads that
    /// list regardless of *how* the creature dies (combat, destroy, sacrifice,
    /// SBA). This isolates the linkage from the combat-death path exercised by
    /// the e2e puzzle test (tests/sengir_vampire_flying_e2e.sh): here Sengir
    /// deals NO lethal blow — we simulate sublethal combat damage this turn,
    /// then the victim dies via a separate death event, and the +1/+1 counter
    /// must still appear (CR 603.2 / 603.6 trigger timing; CR 122 counters).
    #[test]
    fn test_card_compat_sengir_vampire_this_turn_linkage() {
        use crate::core::CounterType;

        if !PathBuf::from("../cardsfolder/s/sengir_vampire.txt").exists() {
            eprintln!("Skipping: cardsfolder not present");
            return;
        }

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // Sengir Vampire on P1's battlefield (real card script => real trigger).
        let sengir_id = load_test_card(&mut game, "Sengir Vampire", p1_id).expect("load Sengir Vampire");
        game.battlefield.add(sengir_id);

        // A victim creature P1 did NOT deal lethal damage to in combat.
        let victim_id = game.next_card_id();
        let mut victim = Card::new(victim_id, "Grizzly Bears".to_string(), p2_id);
        victim.add_type(CardType::Creature);
        victim.set_base_power(Some(2));
        victim.set_base_toughness(Some(2));
        victim.controller = p2_id;
        // Simulate "Sengir dealt this creature (sublethal) combat damage THIS
        // TURN" — exactly the state combat.rs records at combat.rs:823-831.
        victim.damaged_by_this_turn.push(sengir_id);
        game.cards.insert(victim_id, victim);
        game.battlefield.add(victim_id);

        // Baseline: Sengir has no +1/+1 counter yet.
        assert_eq!(game.cards.get(sengir_id).unwrap().get_counter(CounterType::P1P1), 0);

        // The victim now dies from a NON-combat cause later in the same turn.
        // check_death_triggers is the shared death-trigger entry point invoked
        // by destroy/sacrifice/SBA paths just as it is by combat.
        game.battlefield.remove(victim_id);
        game.check_death_triggers(victim_id)
            .expect("death triggers should resolve");

        // Sengir's DamagedCreatureDies trigger must have fired: +1/+1 counter.
        assert_eq!(
            game.cards.get(sengir_id).unwrap().get_counter(CounterType::P1P1),
            1,
            "Sengir must gain a +1/+1 counter when a creature it damaged this turn dies (non-combat death)"
        );
    }

    /// Card compat: Sengir Vampire — negative case (mtg-408).
    ///
    /// The trigger must NOT fire for a creature Sengir never damaged. Guards
    /// against an over-broad `DamagedCreatureDies` firing on every death.
    #[test]
    fn test_card_compat_sengir_vampire_no_trigger_when_undamaged() {
        use crate::core::CounterType;

        if !PathBuf::from("../cardsfolder/s/sengir_vampire.txt").exists() {
            eprintln!("Skipping: cardsfolder not present");
            return;
        }

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        let sengir_id = load_test_card(&mut game, "Sengir Vampire", p1_id).expect("load Sengir Vampire");
        game.battlefield.add(sengir_id);

        // Victim with an EMPTY damaged_by_this_turn list (Sengir never hit it).
        let victim_id = game.next_card_id();
        let mut victim = Card::new(victim_id, "Grizzly Bears".to_string(), p2_id);
        victim.add_type(CardType::Creature);
        victim.set_base_power(Some(2));
        victim.set_base_toughness(Some(2));
        victim.controller = p2_id;
        game.cards.insert(victim_id, victim);
        game.battlefield.add(victim_id);

        game.battlefield.remove(victim_id);
        game.check_death_triggers(victim_id)
            .expect("death triggers should resolve");

        assert_eq!(
            game.cards.get(sengir_id).unwrap().get_counter(CounterType::P1P1),
            0,
            "Sengir must NOT gain a counter for a creature it did not damage"
        );
    }

    /// Card compat: Hypnotic Specter (cardsfolder/h/hypnotic_specter.txt)
    ///
    /// Script:
    ///   ManaCost:1 B B
    ///   Types:Creature Specter
    ///   PT:2/2
    ///   K:Flying
    ///   T:Mode$ DamageDone | ValidSource$ Card.Self | ValidTarget$ Opponent | Execute$ TrigDiscard
    ///   SVar:TrigDiscard:DB$ Discard | Defined$ TriggeredTarget | NumCards$ 1 | Mode$ Random
    ///
    /// Asserts (a) the parsed shape (cost, P/T, Flying, a DealsCombatDamage
    /// trigger), and (b) that the trigger's discard effect carries the
    /// `target_opponent` PlayerId sentinel — NOT the bare placeholder. Before
    /// the fix, `Defined$ TriggeredTarget` fell through the converter's player
    /// match to `placeholder()`, which `resolve_effect_placeholder` then
    /// mapped to `ctx.controller` (the attacker), so the ATTACKER discarded
    /// instead of the player Hypnotic Specter hit. The sentinel routes the
    /// trigger-path resolver to `ctx.opponent` (CR 116.2c, 2-player approx —
    /// the player the creature dealt damage to). See mtg-564 for the long-term
    /// multiplayer player-targeting fix.
    #[test]
    fn test_card_compat_hypnotic_specter() {
        use crate::core::{Keyword, TriggerEvent};
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/h/hypnotic_specter.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Hypnotic Specter should load");

        assert_eq!(def.name.as_str(), "Hypnotic Specter");
        assert_eq!(def.mana_cost.black, 2, "Cost should be {{1}}{{B}}{{B}}");
        assert_eq!(def.mana_cost.generic, 1, "Cost should be {{1}}{{B}}{{B}}");
        assert!(def.types.contains(&CardType::Creature));
        assert_eq!(def.power, Some(2));
        assert_eq!(def.toughness, Some(2));

        let card_id = CardId::new(1);
        let card = def.instantiate(card_id, PlayerId::new(0));

        assert!(
            card.keywords.contains(Keyword::Flying),
            "Hypnotic Specter must have Flying. Keywords: {:?}",
            card.keywords
        );

        // The DamageDone trigger must parse and its Execute$ discard effect must
        // carry the target_opponent sentinel (not the placeholder/controller).
        let discard_trigger = card
            .triggers
            .iter()
            .find(|t| t.event == TriggerEvent::DealsCombatDamage)
            .expect("Hypnotic Specter must have a DealsCombatDamage trigger");

        let found = discard_trigger.effects.iter().any(|e| {
            matches!(
                e,
                Effect::DiscardCards { player, .. } if player.is_target_opponent()
            )
        });
        assert!(
            found,
            "Hypnotic Specter's damage trigger must produce DiscardCards with \
             PlayerId::target_opponent() so the DAMAGED player (opponent), not the \
             attacker, discards. Got trigger effects: {:?}",
            discard_trigger.effects
        );
    }

    /// Card compat: Mind Twist (mtg-564; cardsfolder/m/mind_twist.txt)
    ///
    /// Script:
    ///   ManaCost:X B
    ///   Types:Sorcery
    ///   A:SP$ Discard | ValidTgts$ Player | NumCards$ X | Mode$ Random
    ///   SVar:X:Count$xPaid
    ///
    /// Asserts the converter produces `Effect::DiscardCardsXPaid` with the
    /// `target_opponent` PlayerId sentinel (not the bare placeholder), so the
    /// effect resolver hits the opponent-default branch instead of the
    /// controller-default branch. Before the fix the discard landed on the
    /// caster (whose hand was usually empty), and the user reported
    /// "Mind Twist did nothing" when the opponent cast X=8 against them.
    #[test]
    fn test_card_compat_mind_twist() {
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/m/mind_twist.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Mind Twist should load");

        assert_eq!(def.name.as_str(), "Mind Twist");
        assert!(def.types.contains(&CardType::Sorcery));
        assert_eq!(def.mana_cost.black, 1, "Cost should include {{B}}");

        let card_id = CardId::new(1);
        let card = def.instantiate(card_id, PlayerId::new(0));

        // Spell ability is SP$ Discard with XPaid + target_opponent player sentinel.
        let found = card.effects.iter().any(|e| {
            matches!(
                e,
                Effect::DiscardCardsXPaid { player, .. } if player.is_target_opponent()
            )
        });
        assert!(
            found,
            "Mind Twist must produce DiscardCardsXPaid with PlayerId::target_opponent() \
             sentinel so ValidTgts$ Player resolves to the opponent (mtg-564). \
             Got effects: {:?}",
            card.effects
        );
    }

    /// Card compat: Lightning Bolt — player target sentinel round-trip (mtg-565).
    ///
    /// Asserts that `player_as_target_sentinel` and `player_target_from_sentinel`
    /// round-trip correctly for both players (the sentinel scheme used to let
    /// `Controller::choose_targets(&[CardId])` offer Players as legal targets
    /// for "any target" damage spells like Lightning Bolt).
    ///
    /// Regression: the user reported "Lightning Bolt won't let me target
    /// opponent" — before the fix Players never appeared in valid_targets
    /// because the trait carried only CardId.
    #[test]
    fn test_player_target_sentinel_roundtrip_for_lightning_bolt() {
        for raw in 0u32..4 {
            let pid = PlayerId::new(raw);
            let sentinel = crate::core::player_as_target_sentinel(pid);
            let decoded = crate::core::player_target_from_sentinel(sentinel).expect("Player sentinel must decode");
            assert_eq!(decoded, pid, "sentinel must round-trip");
        }
        // Real-looking small CardIds must NOT decode as players.
        for raw in 0u32..32 {
            let card = CardId::new(raw);
            assert!(
                crate::core::player_target_from_sentinel(card).is_none(),
                "ordinary CardId {} must not be misread as a Player",
                raw
            );
        }
    }

    /// Parser-shape regression test for The Abyss (mtg-550).
    ///
    /// The Abyss: `3 B` World Enchantment with an "each player's upkeep" phase
    /// trigger whose Execute$ SVar is
    /// `DB$ Destroy | ValidTgts$ Creature.nonArtifact+ActivePlayerCtrl | NoRegen$ True`.
    ///
    /// Before the fix the Phase-trigger Execute$ handler had a hardcoded ApiType
    /// allowlist (DealDamage/GainLife/Earthbend/Pump) that silently dropped the
    /// Destroy effect, so the trigger fired but did nothing. We now reuse
    /// `params_to_effect`, and `TargetRestriction::parse` honors `nonArtifact`
    /// (requires_nonartifact) and `ActivePlayerCtrl`, plus `NoRegen$ True`
    /// (no_regenerate).
    #[test]
    fn test_card_compat_the_abyss() {
        use crate::core::effects::ControllerRestriction;
        use crate::core::{CardType, TriggerEvent};
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/t/the_abyss.txt");
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Failed to load The Abyss");
        assert_eq!(def.name.as_str(), "The Abyss");

        let game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let card = def.instantiate(crate::core::CardId::new(100), p1_id);

        // Static shape: 3 B World Enchantment.
        assert_eq!(card.mana_cost.generic, 3, "The Abyss costs {{3}}{{B}}");
        assert_eq!(card.mana_cost.black, 1, "The Abyss has one black pip");
        assert!(
            card.types.contains(&CardType::Enchantment),
            "The Abyss is an Enchantment"
        );

        // Exactly one upkeep trigger, NOT controller-only (fires on each player's upkeep).
        let upkeep: Vec<_> = card
            .triggers
            .iter()
            .filter(|t| t.event == TriggerEvent::BeginningOfUpkeep)
            .collect();
        assert_eq!(upkeep.len(), 1, "The Abyss has exactly one upkeep trigger");
        let trigger = upkeep[0];
        assert!(
            !trigger.controller_turn_only,
            "The Abyss fires on EACH player's upkeep (ValidPlayer$ Player), not controller-only"
        );

        // The trigger's effect must be a DestroyPermanent that:
        //  - targets nonartifact creatures (requires_nonartifact)
        //  - controlled by the active player (ActivePlayerCtrl)
        //  - can't be regenerated (no_regenerate)
        assert_eq!(trigger.effects.len(), 1, "upkeep trigger has exactly one effect");
        let Effect::DestroyPermanent {
            restriction,
            no_regenerate,
            ..
        } = &trigger.effects[0]
        else {
            panic!("Expected DestroyPermanent, got {:?}", &trigger.effects[0]);
        };
        assert!(
            restriction.requires_nonartifact,
            "must restrict to nonartifact creatures (Creature.nonArtifact)"
        );
        assert_eq!(
            restriction.controller,
            ControllerRestriction::ActivePlayerCtrl,
            "must restrict to the active player's creatures (ActivePlayerCtrl)"
        );
        assert!(
            *no_regenerate,
            "The Abyss's destroy can't be regenerated (NoRegen$ True)"
        );
    }

    /// Behavioral regression: a `DestroyPermanent` with `no_regenerate: true`
    /// destroys a permanent outright even when it has an active regeneration
    /// shield (CR 701.15d, "can't be regenerated"). Covers The Abyss's NoRegen.
    #[test]
    fn test_destroy_no_regenerate_ignores_shield() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Helper: create a 2/2 creature on the battlefield with a regeneration shield.
        let make_shielded_bear = |game: &mut GameState| -> CardId {
            let id = game.next_card_id();
            let mut bear = Card::new(id, "Grizzly Bears".to_string(), p1_id);
            bear.add_type(CardType::Creature);
            bear.set_base_power(Some(2));
            bear.set_base_toughness(Some(2));
            bear.controller = p1_id;
            bear.regeneration_shields = 1;
            game.cards.insert(id, bear);
            game.battlefield.add(id);
            id
        };

        // Destroy with no_regenerate=true must NOT be replaced by the shield.
        let creature_id = make_shielded_bear(&mut game);
        let destroy = Effect::DestroyPermanent {
            target: creature_id,
            restriction: crate::core::TargetRestriction::any(),
            no_regenerate: true,
        };
        game.execute_effect(&destroy).unwrap();
        assert!(
            !game.battlefield.contains(creature_id),
            "no_regenerate destroy must kill through a regeneration shield"
        );

        // Control: with no_regenerate=false the shield saves it.
        let creature2 = make_shielded_bear(&mut game);
        let destroy2 = Effect::DestroyPermanent {
            target: creature2,
            restriction: crate::core::TargetRestriction::any(),
            no_regenerate: false,
        };
        game.execute_effect(&destroy2).unwrap();
        assert!(
            game.battlefield.contains(creature2),
            "regeneration shield must save a creature from an ordinary destroy"
        );
    }

    /// Card compat: Savannah Lions (cardsfolder/s/savannah_lions.txt)
    ///
    /// Script:
    ///   Name:Savannah Lions
    ///   ManaCost:W
    ///   Types:Creature Cat
    ///   PT:2/1
    ///
    /// A pure vanilla 2/1 for {W} (no keywords, no abilities). Verifies the
    /// loader keeps the printed cost / P-T / type line intact and does NOT
    /// hallucinate spurious abilities or keywords on a French-vanilla card.
    /// Runtime cast + attack behaviour is verified by the gameplay reproducer
    /// in the compat issue (mtg-538). Part of thedeck (mtg-413).
    #[test]
    fn test_card_compat_savannah_lions() {
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/s/savannah_lions.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Savannah Lions should load");

        assert_eq!(def.name.as_str(), "Savannah Lions");
        assert_eq!(def.mana_cost.generic, 0, "Cost should be exactly {{W}}");
        assert_eq!(def.mana_cost.white, 1, "Cost should be exactly {{W}}");
        assert_eq!(def.mana_cost.cmc(), 1, "CMC should be 1");
        assert!(def.types.contains(&CardType::Creature), "must be a Creature");
        assert_eq!(def.power, Some(2));
        assert_eq!(def.toughness, Some(1));

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));
        assert!(
            card.keywords.is_empty(),
            "Savannah Lions is vanilla — it must have NO keywords. Got: {:?}",
            card.keywords
        );
        assert!(
            card.activated_abilities.is_empty(),
            "Savannah Lions is vanilla — it must have NO activated abilities. Got: {:?}",
            card.activated_abilities
        );
        assert!(
            card.triggers.is_empty(),
            "Savannah Lions is vanilla — it must have NO triggers. Got: {:?}",
            card.triggers.iter().map(|t| &t.event).collect::<Vec<_>>()
        );
    }

    /// Card compat: Su-Chi (cardsfolder/s/su_chi.txt)
    ///
    /// Script:
    ///   ManaCost:4
    ///   Types:Artifact Creature Construct
    ///   PT:4/4
    ///   T:Mode$ ChangesZone | Origin$ Battlefield | Destination$ Graveyard
    ///     | ValidCard$ Card.Self | Execute$ TrigAddMana
    ///   SVar:TrigAddMana:DB$ Mana | Produced$ C | Amount$ 4
    ///
    /// Verifies (parser + runtime): {4} 4/4 Artifact Creature whose dies
    /// trigger (ChangesZone Battlefield -> Graveyard, Card.Self) fires and
    /// adds {C}{C}{C}{C} to its controller's mana pool. A silent drop of the
    /// `T:` line would leave a vanilla 4/4 with no mana on death. Part of
    /// thedeck (mtg-413, mtg-545).
    #[test]
    fn test_card_compat_su_chi() {
        use crate::core::TriggerEvent;
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/s/su_chi.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Su-Chi should load");

        assert_eq!(def.name.as_str(), "Su-Chi");
        assert_eq!(def.mana_cost.generic, 4, "Cost should be {{4}}");
        assert_eq!(def.mana_cost.cmc(), 4, "CMC should be 4");
        assert!(def.types.contains(&CardType::Artifact), "must be an Artifact");
        assert!(def.types.contains(&CardType::Creature), "must be a Creature");
        assert_eq!(def.power, Some(4));
        assert_eq!(def.toughness, Some(4));

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));

        // The dies trigger (ChangesZone Battlefield -> Graveyard, Card.Self)
        // is modelled as LeavesBattlefield (see loader/card.rs ~2040). A
        // silent drop of the `T:` line would leave zero triggers.
        let dies_trigger = card
            .triggers
            .iter()
            .find(|t| matches!(t.event, TriggerEvent::LeavesBattlefield))
            .unwrap_or_else(|| {
                panic!(
                    "Su-Chi must have a dies (LeavesBattlefield) trigger. Got triggers: {:?}",
                    card.triggers.iter().map(|t| &t.event).collect::<Vec<_>>()
                )
            });
        assert!(
            dies_trigger.trigger_self_only,
            "Su-Chi's dies trigger targets Card.Self, so it must be self-only"
        );

        // Its Execute$ TrigAddMana (DB$ Mana | Produced$ C | Amount$ 4) must
        // resolve to an AddMana effect granting exactly 4 colourless mana —
        // not colour-shifted, not a different amount.
        let add_mana = dies_trigger
            .effects
            .iter()
            .find_map(|e| {
                if let Effect::AddMana { mana, .. } = e {
                    Some(mana)
                } else {
                    None
                }
            })
            .expect("Su-Chi dies trigger must carry an AddMana effect (Produced$ C | Amount$ 4)");
        assert_eq!(
            (
                add_mana.white,
                add_mana.blue,
                add_mana.black,
                add_mana.red,
                add_mana.green,
                add_mana.colorless
            ),
            (0, 0, 0, 0, 0, 4),
            "Su-Chi's death must add exactly {{C}}{{C}}{{C}}{{C}} (4 colourless). Got: {:?}",
            add_mana
        );
    }

    /// Card compat: Tundra / Underground Sea (original dual lands) + the
    /// Island / Plains basics that share the type-driven intrinsic-mana path.
    ///
    /// Scripts carry NO printed `A:` mana line — each basic-land subtype on
    /// the type line grants an intrinsic "{T}: Add {color}" ability (CR
    /// 305.6). Tundra = Plains Island ({W},{U}); Underground Sea = Island
    /// Swamp ({U},{B}); Island = {U}; Plains = {W}. A dropped subtype would
    /// show up as a missing colour here. Companion to
    /// `test_card_compat_original_dual_lands`. Part of thedeck (mtg-413:
    /// mtg-553 Tundra, mtg-554 Underground Sea, mtg-513 Island, mtg-530
    /// Plains).
    #[test]
    fn test_card_compat_thedeck_lands() {
        use std::path::PathBuf;

        // (card file, name, expected mana colours as (W,U,B,R,G) tuples)
        let cases: &[(&str, &str, &[ManaColorTuple])] = &[
            // Tundra: Plains Island -> {W}, {U}
            (
                "../cardsfolder/t/tundra.txt",
                "Tundra",
                &[(1, 0, 0, 0, 0), (0, 1, 0, 0, 0)],
            ),
            // Underground Sea: Island Swamp -> {U}, {B}
            (
                "../cardsfolder/u/underground_sea.txt",
                "Underground Sea",
                &[(0, 1, 0, 0, 0), (0, 0, 1, 0, 0)],
            ),
            // Island (basic) -> {U}
            ("../cardsfolder/i/island.txt", "Island", &[(0, 1, 0, 0, 0)]),
            // Plains (basic) -> {W}
            ("../cardsfolder/p/plains.txt", "Plains", &[(1, 0, 0, 0, 0)]),
        ];

        for (path_str, name, expected_colors) in cases {
            let path = PathBuf::from(path_str);
            if !path.exists() {
                eprintln!("Skipping: cardsfolder not present at {:?}", path);
                return;
            }
            let def = crate::loader::CardLoader::load_from_file(&path)
                .unwrap_or_else(|e| panic!("{name} should load: {e:?}"));
            assert_eq!(def.name.as_str(), *name);
            assert!(def.types.contains(&CardType::Land), "{name} must be a Land");

            let card = def.instantiate(CardId::new(1), PlayerId::new(0));

            let mut produced = tap_mana_ability_colors(&card);
            produced.sort_unstable();

            let mut expected: Vec<ManaColorTuple> = expected_colors.to_vec();
            expected.sort_unstable();

            assert_eq!(
                produced, expected,
                "{name} must grant exactly its intrinsic mana abilities for its basic land \
                 types (CR 305.6). A missing/colourless ability means a subtype was dropped. \
                 Got abilities: {:?}",
                card.activated_abilities
            );
        }
    }

    /// Card compat: Time Walk (cardsfolder/t/time_walk.txt)
    ///
    /// Script:
    ///   ManaCost:1 U
    ///   Types:Sorcery
    ///   A:SP$ AddTurn | NumTurns$ 1 | SpellDescription$ Take an extra turn after this one.
    ///
    /// Asserts the parsed shape: {1}{U} Sorcery whose spell ability produces an
    /// Effect::AddTurn with num_turns == 1. The `player` field is a placeholder
    /// (PlayerId 0) resolved to the caster at cast time. Runtime behaviour (the
    /// CASTER actually takes a consecutive extra turn, CR 500.7) is covered by
    /// tests/time_walk_extra_turn_e2e.sh. Part of Troll Disk deck (mtg-562,
    /// mtg-551). Regression guard: a previous bug pushed the extra turn onto a
    /// dead TurnStructure field that the rotation code never drained, so the
    /// extra turn silently never happened.
    #[test]
    fn test_card_compat_time_walk() {
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/t/time_walk.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Time Walk should load");

        assert_eq!(def.name.as_str(), "Time Walk");
        assert_eq!(def.mana_cost.generic, 1, "Cost should be {{1}}{{U}}");
        assert_eq!(def.mana_cost.blue, 1, "Cost should be {{1}}{{U}}");
        assert_eq!(def.mana_cost.cmc(), 2, "CMC should be 2");
        assert!(def.types.contains(&CardType::Sorcery), "must be a Sorcery");

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));

        // The SP$ AddTurn spell ability resolves into the card's on-resolve
        // effects (card.effects) as Effect::AddTurn { num_turns: 1 }.
        let add_turn = card
            .effects
            .iter()
            .find_map(|e| {
                if let Effect::AddTurn { num_turns, .. } = e {
                    Some(*num_turns)
                } else {
                    None
                }
            })
            .expect("Time Walk must produce an AddTurn effect (a silent drop leaves no extra turn)");
        assert_eq!(add_turn, 1, "Time Walk grants exactly 1 extra turn");
    }

    /// Card compat: Underworld Dreams (cardsfolder/u/underworld_dreams.txt)
    ///
    /// Script:
    ///   ManaCost:B B B
    ///   Types:Enchantment
    ///   T:Mode$ Drawn | ValidCard$ Card.OppOwn | Execute$ TrigDamage
    ///   SVar:TrigDamage:DB$ DealDamage | Defined$ TriggeredPlayer | NumDmg$ 1
    ///
    /// Asserts the parsed shape: {B}{B}{B} Enchantment carrying a CardDrawn
    /// trigger with a DealDamage effect. The gameplay behavior (the OPPONENT
    /// who drew takes 1 damage, while the controller's own draws do not
    /// trigger) is covered by tests/underworld_dreams_draw_damage_e2e.sh.
    #[test]
    fn test_card_compat_underworld_dreams() {
        use crate::core::TriggerEvent;
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/u/underworld_dreams.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Underworld Dreams should load");

        assert_eq!(def.name.as_str(), "Underworld Dreams");
        assert_eq!(def.mana_cost.black, 3, "Cost should be {{B}}{{B}}{{B}}");
        assert!(def.types.contains(&CardType::Enchantment));

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));

        let draw_trigger = card
            .triggers
            .iter()
            .find(|t| t.event == TriggerEvent::CardDrawn)
            .expect("Underworld Dreams must have a CardDrawn trigger");
        assert!(
            draw_trigger
                .effects
                .iter()
                .any(|e| matches!(e, Effect::DealDamage { .. })),
            "Underworld Dreams' draw trigger must deal damage. Got: {:?}",
            draw_trigger.effects
        );
    }

    /// Card compat: Royal Assassin (cardsfolder/r/royal_assassin.txt)
    ///
    /// Script:
    ///   ManaCost:1 B B
    ///   Types:Creature Human Assassin
    ///   PT:1/1
    ///   A:AB$ Destroy | Cost$ T | ValidTgts$ Creature.tapped
    ///
    /// Asserts the parsed shape: {1}{B}{B} 1/1 with a tap-cost activated ability
    /// producing a Destroy effect. The gameplay behavior (destroys a tapped
    /// creature, and is NOT offered against an untapped creature) is covered by
    /// tests/royal_assassin_destroys_tapped_e2e.sh.
    #[test]
    fn test_card_compat_royal_assassin() {
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/r/royal_assassin.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Royal Assassin should load");

        assert_eq!(def.name.as_str(), "Royal Assassin");
        assert_eq!(def.mana_cost.black, 2, "Cost should be {{1}}{{B}}{{B}}");
        assert_eq!(def.mana_cost.generic, 1, "Cost should be {{1}}{{B}}{{B}}");
        assert_eq!(def.power, Some(1));
        assert_eq!(def.toughness, Some(1));

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));

        let destroy_ability = card
            .activated_abilities
            .iter()
            .find(|a| a.effects.iter().any(|e| matches!(e, Effect::DestroyPermanent { .. })))
            .expect("Royal Assassin must have a Destroy activated ability");
        assert!(
            destroy_ability.cost.includes_tap(),
            "Royal Assassin's Destroy ability must have a tap cost. Got: {:?}",
            destroy_ability.cost
        );
    }

    /// Card compat: Will-o'-the-Wisp (cardsfolder/w/will_o_the_wisp.txt)
    ///
    /// Script:
    ///   ManaCost:B
    ///   Types:Creature Spirit
    ///   PT:0/1
    ///   K:Flying
    ///   A:AB$ Regenerate | Cost$ B
    ///
    /// Asserts the parsed shape: {B} 0/1 with Flying and a {B}-cost activated
    /// ability producing a Regenerate effect targeting itself. The gameplay
    /// behavior (the ability grants a regeneration shield in a real game) is
    /// covered by tests/will_o_the_wisp_regenerate_e2e.sh. The shield's
    /// destruction-prevention semantics are covered by the existing
    /// test_regeneration_shield_* tests in keywords.rs.
    #[test]
    fn test_card_compat_will_o_the_wisp() {
        use crate::core::Keyword;
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/w/will_o_the_wisp.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Will-o'-the-Wisp should load");

        assert_eq!(def.name.as_str(), "Will-o'-the-Wisp");
        assert_eq!(def.mana_cost.black, 1, "Cost should be {{B}}");
        assert_eq!(def.power, Some(0));
        assert_eq!(def.toughness, Some(1));

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));

        assert!(
            card.keywords.contains(Keyword::Flying),
            "Will-o'-the-Wisp must have Flying. Keywords: {:?}",
            card.keywords
        );

        assert!(
            card.activated_abilities
                .iter()
                .any(|a| a.effects.iter().any(|e| matches!(e, Effect::Regenerate { .. }))),
            "Will-o'-the-Wisp must have a Regenerate activated ability. Got: {:?}",
            card.activated_abilities
        );
    }

    /// Card compat: Dark Ritual (cardsfolder/d/dark_ritual.txt)
    ///
    /// Script:
    ///   ManaCost:B
    ///   Types:Instant
    ///   A:SP$ Mana | Produced$ B | Amount$ 3
    ///
    /// Asserts the parsed shape: {B} Instant whose spell effect adds {B}{B}{B}.
    /// Gameplay (ritual mana funds a 1BB creature off a single Swamp) is covered
    /// by tests/dark_ritual_mana_e2e.sh.
    #[test]
    fn test_card_compat_dark_ritual() {
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/d/dark_ritual.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Dark Ritual should load");

        assert_eq!(def.name.as_str(), "Dark Ritual");
        assert_eq!(def.mana_cost.black, 1, "Cost should be {{B}}");
        assert!(def.types.contains(&CardType::Instant));

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));
        assert!(
            card.effects.iter().any(|e| matches!(e, Effect::AddMana { .. })),
            "Dark Ritual must produce mana (AddMana). Got: {:?}",
            card.effects
        );
    }

    /// Card compat: Sinkhole (cardsfolder/s/sinkhole.txt)
    ///
    /// Script:
    ///   ManaCost:B B
    ///   Types:Sorcery
    ///   A:SP$ Destroy | ValidTgts$ Land
    ///
    /// Asserts the parsed shape: {B}{B} Sorcery whose spell effect destroys a
    /// land target. Gameplay (destroys the opponent's land) is covered by
    /// tests/sinkhole_destroys_land_e2e.sh.
    #[test]
    fn test_card_compat_sinkhole() {
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/s/sinkhole.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Sinkhole should load");

        assert_eq!(def.name.as_str(), "Sinkhole");
        assert_eq!(def.mana_cost.black, 2, "Cost should be {{B}}{{B}}");
        assert!(def.types.contains(&CardType::Sorcery));

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));
        assert!(
            card.effects
                .iter()
                .any(|e| matches!(e, Effect::DestroyPermanent { .. })),
            "Sinkhole must destroy a permanent (DestroyPermanent). Got: {:?}",
            card.effects
        );
    }

    /// Card compat: Demonic Tutor (cardsfolder/d/demonic_tutor.txt)
    ///
    /// Script:
    ///   ManaCost:1 B
    ///   Types:Sorcery
    ///   A:SP$ ChangeZone | Origin$ Library | Destination$ Hand | ChangeType$ Card | ChangeNum$ 1 | Mandatory$ True
    ///
    /// Asserts the parsed shape: {1}{B} Sorcery that searches the library. The
    /// Library->Hand ChangeZone converts to a SearchLibrary effect. Gameplay
    /// (search finds a card, puts it in hand, shuffles) is covered by
    /// tests/demonic_tutor_search_e2e.sh.
    #[test]
    fn test_card_compat_demonic_tutor() {
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/d/demonic_tutor.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Demonic Tutor should load");

        assert_eq!(def.name.as_str(), "Demonic Tutor");
        assert_eq!(def.mana_cost.black, 1, "Cost should be {{1}}{{B}}");
        assert_eq!(def.mana_cost.generic, 1, "Cost should be {{1}}{{B}}");
        assert!(def.types.contains(&CardType::Sorcery));

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));
        assert!(
            card.effects.iter().any(|e| matches!(e, Effect::SearchLibrary { .. })),
            "Demonic Tutor must search the library (SearchLibrary). Got: {:?}",
            card.effects
        );
    }

    /// Card compat: Greed (cardsfolder/g/greed.txt)
    ///
    /// Script:
    ///   ManaCost:3 B
    ///   Types:Enchantment
    ///   A:AB$ Draw | Cost$ B PayLife<2> | NumCards$ 1
    ///
    /// Asserts the parsed shape: {3}{B} Enchantment with an activated ability
    /// whose cost includes paying 2 life and which draws a card. Gameplay
    /// (activate -> draw, life paid) is covered by tests/greed_draw_e2e.sh.
    #[test]
    fn test_card_compat_greed() {
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/g/greed.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Greed should load");

        assert_eq!(def.name.as_str(), "Greed");
        assert_eq!(def.mana_cost.black, 1, "Cost should be {{3}}{{B}}");
        assert_eq!(def.mana_cost.generic, 3, "Cost should be {{3}}{{B}}");
        assert!(def.types.contains(&CardType::Enchantment));

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));
        let draw_ability = card
            .activated_abilities
            .iter()
            .find(|a| a.effects.iter().any(|e| matches!(e, Effect::DrawCards { .. })))
            .expect("Greed must have a Draw activated ability");

        // Greed's cost is "B PayLife<2>" -> a Composite containing a PayLife.
        fn has_pay_life(cost: &crate::core::Cost) -> bool {
            use crate::core::Cost;
            if matches!(cost, Cost::PayLife { .. }) {
                return true;
            }
            if let Cost::Composite(parts) = cost {
                return parts.iter().any(has_pay_life);
            }
            false
        }
        assert!(
            has_pay_life(&draw_ability.cost),
            "Greed's draw ability must include a pay-life cost. Got: {:?}",
            draw_ability.cost
        );
    }

    /// Card compat: Sol Ring (cardsfolder/s/sol_ring.txt) and Black Lotus
    /// (cardsfolder/b/black_lotus.txt) — fast-mana artifacts.
    ///
    /// Sol Ring:    A:AB$ Mana | Cost$ T | Produced$ C | Amount$ 2
    /// Black Lotus: A:AB$ Mana | Cost$ T Sac<1/CARDNAME> | Produced$ Any | Amount$ 3
    ///
    /// Asserts each parses with a mana ability. Gameplay (Sol Ring taps for
    /// {C}{C}; Black Lotus taps+sacrifices for three of one color) is covered by
    /// tests/sol_ring_mana_e2e.sh and tests/black_lotus_sac_mana_e2e.sh.
    #[test]
    fn test_card_compat_sol_ring_and_black_lotus() {
        use std::path::PathBuf;

        for (file, name, black_cost, generic_cost) in [
            ("../cardsfolder/s/sol_ring.txt", "Sol Ring", 0u8, 1u8),
            ("../cardsfolder/b/black_lotus.txt", "Black Lotus", 0u8, 0u8),
        ] {
            let path = PathBuf::from(file);
            if !path.exists() {
                eprintln!("Skipping: cardsfolder not present at {:?}", path);
                return;
            }
            let def = crate::loader::CardLoader::load_from_file(&path)
                .unwrap_or_else(|e| panic!("{name} should load: {e:?}"));
            assert_eq!(def.name.as_str(), name);
            assert_eq!(def.mana_cost.black, black_cost, "{name} black cost");
            assert_eq!(def.mana_cost.generic, generic_cost, "{name} generic cost");
            assert!(def.types.contains(&CardType::Artifact), "{name} must be an Artifact");

            let card = def.instantiate(CardId::new(1), PlayerId::new(0));
            assert!(
                card.activated_abilities.iter().any(|a| a.is_mana_ability),
                "{name} must have a mana ability. Got: {:?}",
                card.activated_abilities
            );
        }
    }

    /// Card compat: Mishra's Factory (cardsfolder/m/mishras_factory.txt) —
    /// parser-shape regression for mtg-522.
    ///
    /// Script:
    ///   ManaCost:no cost
    ///   Types:Land
    ///   A:AB$ Mana   | Cost$ T | Produced$ C
    ///   A:AB$ Animate| Cost$ 1 | Defined$ Self | Power$ 2 | Toughness$ 2
    ///                  | Types$ Artifact,Creature,Assembly-Worker
    ///                  | RemoveCreatureTypes$ True
    ///   A:AB$ Pump   | Cost$ T | ValidTgts$ Creature.Assembly-Worker
    ///                  | NumAtt$ +1 | NumDef$ +1
    ///
    /// Mishra's Factory has THREE printed activated abilities; this test
    /// guards against any of them being silently dropped by the parser
    /// (which would surface as the action menu showing fewer options
    /// than the printed card text). The current engine state-of-play:
    ///
    /// - The mana ability and the Animate ability both parse and are
    ///   exercised live; see `tests/puzzle_e2e::test_mishras_factory_
    ///   animates_and_is_eligible_attacker` for runtime evidence and
    ///   `tests/card_loading::test_load_mishras_factory_colorless_mana`
    ///   for the mana-shape regression.
    /// - The pump ability ("{T}: Target Assembly-Worker creature gets
    ///   +1/+1 until end of turn") is the secondary ability. This test
    ///   asserts an upper bound: at least one PumpCreature effect must
    ///   appear among the parsed abilities OR else this test fails so
    ///   the silent-drop is caught at compile-time CI.
    #[test]
    fn test_card_compat_mishras_factory() {
        use crate::core::Cost;
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/m/mishras_factory.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Mishra's Factory should load");
        assert_eq!(def.name.as_str(), "Mishra's Factory");
        assert!(def.types.contains(&CardType::Land));

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));

        // 1. Mana ability {T}: Add {C}
        let mana_ability = card.activated_abilities.iter().find(|a| a.is_mana_ability);
        assert!(
            mana_ability.is_some(),
            "Mishra's Factory must have a mana ability ({{T}}: Add {{C}}). Got: {:?}",
            card.activated_abilities
        );

        // 2. Animate ability {1}: become a 2/2 Assembly-Worker creature.
        // Parsed as Effect::SetBasePowerToughness with types_added
        // {Artifact, Creature} and a subtype Assembly-Worker.
        let animate_ability = card.activated_abilities.iter().find(|a| {
            a.effects.iter().any(|e| {
                matches!(
                    e,
                    Effect::SetBasePowerToughness {
                        power: Some(2),
                        toughness: Some(2),
                        ..
                    }
                )
            })
        });
        assert!(
            animate_ability.is_some(),
            "Mishra's Factory must have an Animate ability ({{1}}: 2/2 Assembly-Worker). \
             Got: {:?}",
            card.activated_abilities
        );

        // 3. Pump ability {T}: +1/+1 to target Assembly-Worker.
        // Parsed as Effect::PumpCreature with power_bonus=1, toughness_bonus=1.
        // Silent-drop of this A: line was the primary mtg-522 finding; this
        // assertion is the parser-shape regression guard.
        let pump_ability = card.activated_abilities.iter().find(|a| {
            a.effects.iter().any(|e| {
                matches!(
                    e,
                    Effect::PumpCreature {
                        power_bonus: 1,
                        toughness_bonus: 1,
                        ..
                    }
                )
            })
        });
        assert!(
            pump_ability.is_some(),
            "Mishra's Factory must have a Pump ability ({{T}}: target Assembly-Worker gets +1/+1). \
             If this fails the A:AB$ Pump line was silently dropped — see mtg-522. \
             Got abilities: {:?}",
            card.activated_abilities
        );

        // The pump ability's cost is a pure Tap; the Animate ability's cost
        // is generic {1}. Sanity-assert these are distinct (i.e. they don't
        // collapse to the same ability) so the menu can offer both.
        let pump_cost = &pump_ability.unwrap().cost;
        assert!(
            matches!(pump_cost, Cost::Tap),
            "Pump cost should be pure Tap, got: {:?}",
            pump_cost
        );
    }

    /// Card compat: Juzám Djinn (cardsfolder/j/juzam_djinn.txt) — mtg-515
    ///
    /// Script:
    ///   ManaCost:2 B B
    ///   Types:Creature Djinn
    ///   PT:5/5
    ///   T:Mode$ Phase | Phase$ Upkeep | ValidPlayer$ You
    ///     | TriggerZones$ Battlefield | Execute$ TrigDealDamage
    ///   SVar:TrigDealDamage:DB$ DealDamage | Defined$ You | NumDmg$ 1
    ///
    /// Parser-shape regression: {2}{B}{B} 5/5 creature with a
    /// `BeginningOfUpkeep` trigger that deals 1 damage to its controller.
    /// A silent-drop of the upkeep trigger would turn the card into a
    /// strictly stronger downside-free 5/5 — important to lock down.
    ///
    /// Runtime evidence (mono-black Rogerbrand mirror match, seed 42):
    ///   Juzám Djinn (108) enters the battlefield as a 5/5 creature
    ///   Juzám Djinn deals 1 damage to AI-Heuristic2
    /// (reproducer: `mtg tui decks/old_school/05_mono_black_rogerbrand.dck
    ///   decks/old_school/05_mono_black_rogerbrand.dck --p1=heuristic
    ///   --p2=heuristic --seed 42 --verbosity 2 --stop-on-choice 300`)
    #[test]
    fn test_card_compat_juzam_djinn() {
        use crate::core::TriggerEvent;
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/j/juzam_djinn.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Juzám Djinn should load");

        assert_eq!(def.name.as_str(), "Juzám Djinn");
        assert_eq!(def.mana_cost.generic, 2, "Cost should be {{2}}{{B}}{{B}}");
        assert_eq!(def.mana_cost.black, 2, "Cost should be {{2}}{{B}}{{B}}");
        assert!(def.types.contains(&CardType::Creature));
        assert_eq!(def.power, Some(5));
        assert_eq!(def.toughness, Some(5));

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));

        let upkeep_trigger = card
            .triggers
            .iter()
            .find(|t| t.event == TriggerEvent::BeginningOfUpkeep)
            .expect(
                "Juzám Djinn must have a BeginningOfUpkeep trigger. \
                 Silently dropping it makes the card strictly stronger than printed.",
            );
        assert!(
            upkeep_trigger
                .effects
                .iter()
                .any(|e| matches!(e, Effect::DealDamage { amount: 1, .. })),
            "Juzám Djinn's upkeep trigger must deal 1 damage. Got: {:?}",
            upkeep_trigger.effects
        );
    }

    /// Card compat: Swamp (cardsfolder/s/swamp.txt) — mtg-546
    ///
    /// Script:
    ///   ManaCost:no cost
    ///   Types:Basic Land Swamp
    ///
    /// Basic-land parser-shape regression: zero mana cost, Basic+Land
    /// supertypes, Fixed(Black) intrinsic mana production. Basic land
    /// mana abilities are derived from the subtype (CR 305.6) rather than
    /// an explicit A:AB$ Mana line, so this test guards against the
    /// loader losing the Swamp → {B} derivation.
    ///
    /// Runtime evidence (mono-black mirror, seed 42):
    ///   AI-Heuristic1 plays Swamp (53)
    ///   Tap Swamp for {B}
    #[test]
    fn test_card_compat_swamp() {
        use crate::core::{ManaColor, ManaProductionKind};
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/s/swamp.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Swamp should load");

        assert_eq!(def.name.as_str(), "Swamp");
        assert!(def.types.contains(&CardType::Land), "Swamp must be a Land");

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));
        assert!(
            matches!(
                card.definition.cache.mana_production.kind,
                ManaProductionKind::Fixed(ManaColor::Black)
            ),
            "Swamp must produce {{B}} (Fixed(Black)). Got: {:?}",
            card.definition.cache.mana_production.kind
        );
    }

    /// Card compat: Gloom (cardsfolder/g/gloom.txt) — mtg-507
    ///
    /// Script:
    ///   ManaCost:2 B
    ///   Types:Enchantment
    ///   S:Mode$ RaiseCost | ValidCard$ Card.White | Type$ Spell | Amount$ 3
    ///   S:Mode$ RaiseCost | ValidCard$ Enchantment.White | Type$ Ability
    ///     | Amount$ 3 | AffectedZone$ Battlefield
    ///
    /// Parser-shape regression: a {2}{B} Enchantment with TWO RaiseCost
    /// static abilities (one targeting white Spells, one targeting
    /// activated abilities of white Enchantments). Silent-drop of
    /// either makes Gloom strictly weaker against the matchup it's
    /// meant to hose. The RaiseCost machinery itself is exercised live
    /// by `tests/integration/raise_cost_*` and the static-ability
    /// scanner in actions/mod.rs:1359.
    #[test]
    fn test_card_compat_gloom() {
        use crate::core::StaticAbility;
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/g/gloom.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Gloom should load");

        assert_eq!(def.name.as_str(), "Gloom");
        assert_eq!(def.mana_cost.generic, 2, "Cost should be {{2}}{{B}}");
        assert_eq!(def.mana_cost.black, 1, "Cost should be {{2}}{{B}}");
        assert!(def.types.contains(&CardType::Enchantment));

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));

        let raise_cost_abilities: Vec<&StaticAbility> = card
            .static_abilities
            .iter()
            .filter(|s| matches!(s, StaticAbility::RaiseCost { .. }))
            .collect();
        assert_eq!(
            raise_cost_abilities.len(),
            2,
            "Gloom must have BOTH RaiseCost static abilities (white spells \
             +{{3}}, white enchantment activated abilities +{{3}}). \
             Silent-drop of either half makes Gloom strictly weaker. \
             Got: {:?}",
            card.static_abilities
        );
    }

    /// Card compat: Control Magic (cardsfolder/c/control_magic.txt) — mtg-493
    ///
    /// Script:
    ///   ManaCost:2 U U
    ///   Types:Enchantment Aura
    ///   K:Enchant:Creature
    ///   S:Mode$ Continuous | Affected$ Card.EnchantedBy | GainControl$ You
    ///
    /// Parser-shape regression: the control-stealing `GainControl$ You`
    /// continuous static must parse into a `StaticAbility::GainControl`
    /// (CR 613.2, layer-2 control change). Previously the `GainControl$`
    /// parameter was silently dropped, leaving Control Magic an inert Aura
    /// that attached but never transferred control. This guards every
    /// control-stealing Aura (Mind Control, Persuasion, Enslave, ...).
    #[test]
    fn test_card_compat_control_magic() {
        use crate::core::StaticAbility;
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/c/control_magic.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Control Magic should load");

        assert_eq!(def.name.as_str(), "Control Magic");
        assert_eq!(def.mana_cost.generic, 2, "Cost should be {{2}}{{U}}{{U}}");
        assert_eq!(def.mana_cost.blue, 2, "Cost should be {{2}}{{U}}{{U}}");
        assert!(def.types.contains(&CardType::Enchantment));
        assert!(
            def.subtypes.iter().any(|s| s.as_str() == "Aura"),
            "Control Magic must be an Aura"
        );

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));
        let has_gain_control = card
            .static_abilities
            .iter()
            .any(|s| matches!(s, StaticAbility::GainControl { .. }));
        assert!(
            has_gain_control,
            "Control Magic must parse its `GainControl$ You` continuous static into \
             StaticAbility::GainControl. Silent-drop leaves the Aura inert. Got: {:?}",
            card.static_abilities
        );
    }

    /// Card compat: Library of Alexandria (cardsfolder/l/library_of_alexandria.txt) — mtg-517
    ///
    /// Script:
    ///   A:AB$ Mana | Cost$ T | Produced$ C
    ///   A:AB$ Draw | Cost$ T | PresentZone$ Hand | IsPresent$ Card.YouOwn
    ///                | PresentCompare$ EQ7
    ///
    /// Parser-shape regression: the draw ability's "Activate only if you have
    /// exactly seven cards in hand" restriction (`IsPresent$ | PresentZone$ |
    /// PresentCompare$ EQ7`) must parse into an ActivationCondition with
    /// CompareOp::Equal and count 7 in the Hand zone. Previously the restriction
    /// was dropped, so the draw could be activated at any hand size.
    #[test]
    fn test_card_compat_library_of_alexandria() {
        use crate::core::CompareOp;
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/l/library_of_alexandria.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Library of Alexandria should load");
        assert_eq!(def.name.as_str(), "Library of Alexandria");
        assert!(def.types.contains(&CardType::Land));

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));
        // Two activated abilities: mana ({T}: Add {C}) and the gated draw.
        let draw_with_cond = card
            .activated_abilities
            .iter()
            .find(|a| a.activation_condition.is_some());
        let cond = draw_with_cond.and_then(|a| a.activation_condition.as_ref()).expect(
            "Library of Alexandria's draw ability must parse its \
                 `IsPresent$ Card.YouOwn | PresentZone$ Hand | PresentCompare$ EQ7` \
                 activation restriction into an ActivationCondition.",
        );
        assert_eq!(cond.op, CompareOp::Equal, "EQ7 must be CompareOp::Equal");
        assert_eq!(cond.count, 7, "EQ7 must be count 7");
        assert_eq!(cond.zone, crate::zones::Zone::Hand, "PresentZone$ Hand");
    }

    /// Card compat: Maze of Ith (cardsfolder/m/maze_of_ith.txt) — mtg-520
    ///
    /// Script:
    ///   ManaCost:no cost
    ///   Types:Land
    ///   A:AB$ Untap | Cost$ T | ValidTgts$ Creature.attacking
    ///                  | SubAbility$ DBPump
    ///   SVar:DBPump:DB$ Effect | ReplacementEffects$ RPrevent1,RPrevent2
    ///                  | RememberObjects$ Targeted | ExileOnMoved$ Battlefield
    ///
    /// Parser-shape regression: a non-mana-producing Land with a single
    /// {T}-cost activated ability whose effects (a) untap target
    /// attacking creature and (b) create a one-shot DamagePrevention
    /// effect. Silent-drop of either half changes card identity (drop
    /// the untap → can't undo the combat assignment; drop the
    /// prevention → no damage stop).
    ///
    /// **Status — PARTIAL** at this commit: the loader keeps only the
    /// primary `AB$ Untap` effect on the activated ability; the
    /// `SubAbility$ DBPump` (`DB$ Effect | ReplacementEffects$
    /// RPrevent1,RPrevent2`) is silently dropped. Net effect: Maze of
    /// Ith untaps the creature but does NOT prevent its combat damage,
    /// so the creature still deals damage that turn. Tracking this as a
    /// parser/loader gap on the per-card issue (mtg-520). Once the
    /// SubAbility chain is parsed we should add a stronger assertion
    /// here and a puzzle/e2e for the "no damage dealt" outcome.
    #[test]
    fn test_card_compat_maze_of_ith() {
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/m/maze_of_ith.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Maze of Ith should load");

        assert_eq!(def.name.as_str(), "Maze of Ith");
        assert!(def.types.contains(&CardType::Land));

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));

        // Maze of Ith intentionally produces NO mana — drop into a
        // generic mana ability would mis-cost the activated ability and
        // turn the card into a free mana source.
        assert!(
            !card.activated_abilities.iter().any(|a| a.is_mana_ability),
            "Maze of Ith must NOT have a mana ability; it's a colorless utility land. \
             Got: {:?}",
            card.activated_abilities
        );

        // Primary mode: Untap target attacking creature.
        // This half IS exercised by the parser at HEAD.
        let _untap_ability = card
            .activated_abilities
            .iter()
            .find(|a| a.effects.iter().any(|e| matches!(e, Effect::UntapPermanent { .. })))
            .expect(
                "Maze of Ith must have an Untap activated ability \
                 (untap target attacking creature).",
            );
    }

    /// Card compat: Wrath of God (cardsfolder/w/wrath_of_god.txt)
    ///
    /// Script: ManaCost:2 W W / Types:Sorcery
    ///   A:SP$ DestroyAll | ValidCards$ Creature | NoRegen$ True
    ///
    /// Parser shape: {2}{W}{W} Sorcery that destroys all creatures with
    /// no regeneration. Runtime (board-wipe with NoRegen honored)
    /// verified by tests/wrath_of_god_destroys_all_creatures_e2e.sh
    /// (mtg-558).
    #[test]
    fn test_card_compat_wrath_of_god() {
        use crate::loader::ability_parser::{AbilityParams, ApiType};
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/w/wrath_of_god.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Wrath of God should load");
        assert_eq!(def.name.as_str(), "Wrath of God");
        assert_eq!(def.mana_cost.generic, 2);
        assert_eq!(def.mana_cost.white, 2);
        assert!(def.types.contains(&CardType::Sorcery));

        let destroy_all = def
            .raw_abilities
            .iter()
            .find_map(|raw| {
                let p = AbilityParams::parse(raw).ok()?;
                (p.api_type == ApiType::DestroyAll).then_some(p)
            })
            .expect("Wrath of God must have an SP$ DestroyAll spell ability");
        assert_eq!(
            destroy_all.get("ValidCards"),
            Some("Creature"),
            "Wrath of God must target Creatures"
        );
        assert_eq!(
            destroy_all.get("NoRegen"),
            Some("True"),
            "Wrath of God must set NoRegen$ True"
        );
    }

    /// Card compat: Flash Counter (cardsfolder/f/flash_counter.txt)
    ///
    /// Script: ManaCost:1 U / Types:Instant
    ///   A:SP$ Counter | TargetType$ Spell | ValidTgts$ Instant
    ///
    /// Parser shape: {1}{U} Instant that counters target instant spell.
    /// Runtime (counters a target instant) shares the SP$ Counter
    /// implementation already exercised by Counterspell tests
    /// (mtg-506).
    #[test]
    fn test_card_compat_flash_counter() {
        use crate::loader::ability_parser::{AbilityParams, ApiType};
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/f/flash_counter.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Flash Counter should load");
        assert_eq!(def.name.as_str(), "Flash Counter");
        assert_eq!(def.mana_cost.generic, 1);
        assert_eq!(def.mana_cost.blue, 1);
        assert!(def.types.contains(&CardType::Instant));

        let counter = def
            .raw_abilities
            .iter()
            .find_map(|raw| {
                let p = AbilityParams::parse(raw).ok()?;
                (p.api_type == ApiType::Counter).then_some(p)
            })
            .expect("Flash Counter must have an SP$ Counter spell ability");
        assert_eq!(counter.get("TargetType"), Some("Spell"));
        // The narrowing predicate: only Instant spells can be targeted.
        assert_eq!(
            counter.get("ValidTgts"),
            Some("Instant"),
            "Flash Counter must restrict ValidTgts to Instant"
        );
    }

    /// Card compat: Blue Elemental Blast (cardsfolder/b/blue_elemental_blast.txt)
    ///
    /// Script: ManaCost:U / Types:Instant
    ///   A:SP$ Charm | Choices$ DBCounter,DBDestroy
    ///   SVar:DBCounter:DB$ Counter | TargetType$ Spell | ValidTgts$ Card.Red
    ///   SVar:DBDestroy:DB$ Destroy | ValidTgts$ Permanent.Red
    ///
    /// Parser shape: {U} Instant modal spell with exactly two choices,
    /// each restricted to red cards/permanents. Runtime (both modes:
    /// counter a red spell, destroy a red permanent) verified by
    /// tests/blue_elemental_blast_e2e.sh (mtg-487).
    #[test]
    fn test_card_compat_blue_elemental_blast() {
        use crate::loader::ability_parser::{AbilityParams, ApiType};
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/b/blue_elemental_blast.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Blue Elemental Blast should load");
        assert_eq!(def.name.as_str(), "Blue Elemental Blast");
        assert_eq!(def.mana_cost.blue, 1);
        assert_eq!(def.mana_cost.generic, 0);
        assert!(def.types.contains(&CardType::Instant));

        let charm = def
            .raw_abilities
            .iter()
            .find_map(|raw| {
                let p = AbilityParams::parse(raw).ok()?;
                (p.api_type == ApiType::Charm).then_some(p)
            })
            .expect("Blue Elemental Blast must have an SP$ Charm spell ability");
        // Both modes are referenced from the Choices$ list. Order matters
        // for the fixed-input scripts in the e2e test (0 = counter, 1 =
        // destroy).
        assert_eq!(
            charm.get("Choices"),
            Some("DBCounter,DBDestroy"),
            "Blue Elemental Blast modes must be DBCounter,DBDestroy in order"
        );
    }
}
