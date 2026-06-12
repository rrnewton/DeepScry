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
            keyword_args_granted: smallvec::SmallVec::new(),
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
        counterspell.effects.push(Effect::CounterSpell {
            target: bolt_id,
            spell_restriction: crate::core::TargetRestriction::any(),
            remember_mana_value: false,
        });
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
            spell_restriction: crate::core::TargetRestriction::any(),
            remember_mana_value: false,
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
        if let Some(Effect::CounterSpell { target, .. }) = counterspell.effects.get_mut(0) {
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
                keyword_args_granted: smallvec::SmallVec::new(),
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
        // And the chained dynamic life gain (= targeted creature's power, given
        // to its controller). Parser-shape check for the general construct.
        assert!(
            swords.effects.iter().any(|e| matches!(
                e,
                Effect::GainLifeDynamic {
                    amount: crate::core::DynamicAmount::TargetPower,
                    ..
                }
            )),
            "Swords to Plowshares should have GainLifeDynamic(TargetPower). Effects: {:?}",
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

        // Verify P2 (the exiled creature's controller, per Defined$
        // TargetedController) gained life equal to the creature's power (3),
        // captured via last-known information before the exile (CR 608.2g/2h).
        let life_after = game.get_player(p2_id).unwrap().life;
        assert_eq!(
            life_after,
            life_before + 3,
            "Swords' controller-of-target should gain life equal to the exiled creature's power (3)"
        );
        // The Swords caster (P1) gains nothing.
        assert_eq!(
            game.get_player(p1_id).unwrap().life,
            20,
            "Swords caster should not gain life"
        );
    }

    /// Regression (1994 World Championship compat — Winter Blast, deck-01 SB):
    /// `SP$ Tap | ValidTgts$ Creature` ("Tap X target creatures") must only be
    /// able to target CREATURES. Before the fix the TapPermanent spell-target
    /// branch checked only `!tapped` + is_legal_target, so a land was a legal
    /// target (Winter Blast could "tap" a Forest). With the fix the spell's
    /// `spell_targets_creature` flag filters the target list to creatures.
    #[test]
    fn test_winter_blast_taps_only_creatures() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        let winter_blast_id = match load_test_card(&mut game, "Winter Blast", p1_id) {
            Ok(id) => id,
            Err(e) => panic!("Failed to load Winter Blast: {e}"),
        };

        // A creature controlled by P2 — must be a valid target.
        let creature_id = game.next_entity_id();
        let mut creature = Card::new(creature_id, "Grizzly Bears".to_string(), p2_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(2));
        creature.set_base_toughness(Some(2));
        creature.controller = p2_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // A land controlled by P2 — must NOT be a valid target.
        let land_id = game.next_entity_id();
        let mut land = Card::new(land_id, "Forest".to_string(), p2_id);
        land.add_type(CardType::Land);
        land.controller = p2_id;
        game.cards.insert(land_id, land);
        game.battlefield.add(land_id);

        let valid_targets = game.get_valid_targets_for_spell(winter_blast_id).unwrap();
        assert!(
            valid_targets.contains(&creature_id),
            "Winter Blast must be able to target a creature. Valid targets: {:?}",
            valid_targets
        );
        assert!(
            !valid_targets.contains(&land_id),
            "Winter Blast (Tap target CREATURES) must NOT be able to target a land. Valid targets: {:?}",
            valid_targets
        );
    }

    /// Test Divine Offering: destroy target artifact, you gain life equal to
    /// its mana value (the dynamic-amount GainLife construct, `TargetManaValue`).
    #[test]
    fn test_divine_offering_destroy_gain_mana_value() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        let offering_id = match load_test_card(&mut game, "Divine Offering", p1_id) {
            Ok(id) => id,
            Err(e) => panic!("Failed to load Divine Offering: {e}"),
        };

        // Parser-shape: Destroy + dynamic GainLife (= target's mana value).
        let offering = game.cards.get(offering_id).unwrap();
        assert!(
            offering
                .effects
                .iter()
                .any(|e| matches!(e, Effect::DestroyPermanent { .. })),
            "Divine Offering should have DestroyPermanent. Effects: {:?}",
            offering.effects
        );
        assert!(
            offering.effects.iter().any(|e| matches!(
                e,
                Effect::GainLifeDynamic {
                    amount: crate::core::DynamicAmount::TargetManaValue,
                    ..
                }
            )),
            "Divine Offering should have GainLifeDynamic(TargetManaValue). Effects: {:?}",
            offering.effects
        );

        // A {4}-cost artifact controlled by P2 (mana value 4).
        let artifact_id = game.next_entity_id();
        let mut artifact = Card::new(artifact_id, "Test Artifact".to_string(), p2_id);
        artifact.add_type(CardType::Artifact);
        artifact.mana_cost = crate::core::ManaCost::from_string("4");
        artifact.controller = p2_id;
        game.cards.insert(artifact_id, artifact);
        game.battlefield.add(artifact_id);

        let valid_targets = game.get_valid_targets_for_spell(offering_id).unwrap();
        assert!(
            valid_targets.contains(&artifact_id),
            "Artifact should be targetable by Divine Offering. Valid: {:?}",
            valid_targets
        );

        game.stack.add(offering_id);
        let p1_life_before = game.get_player(p1_id).unwrap().life;

        let result = game.resolve_spell(offering_id, &[artifact_id]);
        assert!(result.is_ok(), "Failed to resolve Divine Offering: {:?}", result);

        // Artifact destroyed.
        assert!(
            !game.battlefield.contains(artifact_id),
            "Artifact should be destroyed (off battlefield)"
        );
        // The caster (Defined$ You) gains life equal to the artifact's mana value (4).
        assert_eq!(
            game.get_player(p1_id).unwrap().life,
            p1_life_before + 4,
            "Divine Offering caster should gain life equal to the destroyed artifact's mana value (4)"
        );
        // The artifact's controller gains nothing.
        assert_eq!(
            game.get_player(p2_id).unwrap().life,
            20,
            "Artifact controller should not gain life from Divine Offering"
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
    fn test_source_prevention_shield_matches_only_chosen_red_source() {
        // Circle of Protection: Red construct — a source-filtered shield only
        // prevents damage from the chosen red source, and only its next event
        // (CR 615.6). Damage from a different source is unaffected.
        use crate::core::{CardType, Color, DamagePreventionShield};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // A red source (the chosen one) and an unrelated source.
        let red_src = game.next_entity_id();
        let mut red_card = Card::new(red_src, "Ironclaw Orcs".to_string(), game.players[1].id);
        red_card.add_type(CardType::Creature);
        red_card.colors = smallvec::smallvec![Color::Red];
        game.cards.insert(red_src, red_card);

        let other_src = game.next_entity_id();
        let mut other_card = Card::new(other_src, "Grizzly Bears".to_string(), game.players[1].id);
        other_card.add_type(CardType::Creature);
        other_card.colors = smallvec::smallvec![Color::Green];
        game.cards.insert(other_src, other_card);

        // Install the shield protecting P1 from the chosen red source.
        game.get_player_mut(p1_id)
            .unwrap()
            .source_prevention_shields
            .push(DamagePreventionShield::colored_source_next_event(Color::Red, red_src));

        // Damage from the chosen red source is fully prevented (any magnitude).
        game.current_damage_source = Some(red_src);
        game.deal_damage(p1_id, 4).unwrap();
        assert_eq!(game.get_player(p1_id).unwrap().life, 20, "red source damage prevented");

        // Shield is spent: a second red event is NOT prevented.
        game.current_damage_source = Some(red_src);
        game.deal_damage(p1_id, 3).unwrap();
        assert_eq!(
            game.get_player(p1_id).unwrap().life,
            17,
            "shield expired after one event (CR 615.1)"
        );

        // Reinstall and confirm a non-red source is never prevented.
        game.get_player_mut(p1_id)
            .unwrap()
            .source_prevention_shields
            .push(DamagePreventionShield::colored_source_next_event(Color::Red, red_src));
        game.current_damage_source = Some(other_src);
        game.deal_damage(p1_id, 2).unwrap();
        assert_eq!(
            game.get_player(p1_id).unwrap().life,
            15,
            "green source damage not prevented by CoP:Red"
        );
        game.current_damage_source = None;
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

    /// Card compat: Ironclaw Orcs (cardsfolder/i/ironclaw_orcs.txt)
    ///
    /// Script:
    ///   ManaCost:1 R
    ///   Types:Creature Orc
    ///   PT:2/2
    ///   S:Mode$ CantBlockBy | ValidAttacker$ Creature.powerGE2 | ValidBlocker$ Creature.Self
    ///
    /// Verifies the `Mode$ CantBlockBy | ValidBlocker$ Creature.Self` shape
    /// lowers to a `StaticAbility::CantBlockMatching` carrying a `powerGE2`
    /// attacker filter (mtg-512). The block-legality enforcement is covered by
    /// `blocker_legality_test::ironclaw_orcs_cant_block_power_2_or_greater`.
    #[test]
    fn test_card_compat_ironclaw_orcs() {
        use crate::core::StaticAbility;
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/i/ironclaw_orcs.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Ironclaw Orcs should load");

        assert_eq!(def.name.as_str(), "Ironclaw Orcs");
        assert_eq!(def.mana_cost.generic, 1, "Cost should be {{1}}{{R}}");
        assert_eq!(def.mana_cost.red, 1, "Cost should include {{R}}");
        assert!(def.types.contains(&CardType::Creature));
        assert_eq!(def.power, Some(2));
        assert_eq!(def.toughness, Some(2));

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));

        let cant_block = card
            .static_abilities
            .iter()
            .find_map(|s| match s {
                StaticAbility::CantBlockMatching { attacker_filter, .. } => Some(attacker_filter),
                StaticAbility::ModifyPT { .. }
                | StaticAbility::GrantKeyword { .. }
                | StaticAbility::ReduceCost { .. }
                | StaticAbility::RaiseCost { .. }
                | StaticAbility::GrantAbility { .. }
                | StaticAbility::GainControl { .. }
                | StaticAbility::SacrificeMatchingPresent { .. }
                | StaticAbility::CantBeCast { .. }
                | StaticAbility::CantPlayLand { .. }
                | StaticAbility::CastWithFlash { .. }
                | StaticAbility::DamageIncrease { .. }
                | StaticAbility::PreventDamageToEnchantedByChosenColor { .. }
                | StaticAbility::CantAttackIfDefenderHasUntappedPowerGE { .. }
                | StaticAbility::CantAttackOrBlockMatching { .. }
                | StaticAbility::CantBeActivated { .. }
                | StaticAbility::ExtraLandPlay { .. }
                | StaticAbility::LifeFloor { .. }
                | StaticAbility::DamageToExileLibrary { .. }
                | StaticAbility::CharacteristicDefiningPt { .. }
                | StaticAbility::GrantUpkeepSacrificeUnlessPay { .. }
                | StaticAbility::AlternativeCost { .. }
                | StaticAbility::MayPlayWithoutManaCost { .. }
                | StaticAbility::MayPlayFromLibrary { .. }
                | StaticAbility::OpalescenceStyle { .. } => None,
            })
            .expect("Ironclaw Orcs must produce a CantBlockMatching static ability");

        // The filter must inclusively reject power-2 attackers and accept power-1.
        let p2 = make_creature_with_power(2);
        let p1 = make_creature_with_power(1);
        assert!(cant_block.matches(&p2), "powerGE2 filter must match a power-2 attacker");
        assert!(
            !cant_block.matches(&p1),
            "powerGE2 filter must NOT match a power-1 attacker"
        );
    }

    /// Card compat: Juggernaut (cardsfolder/j/juggernaut.txt) — mtg-897 / mtg-713 B20.
    ///
    /// Script:
    ///   ManaCost:4
    ///   Types:Artifact Creature Juggernaut
    ///   PT:5/3
    ///   S:Mode$ MustAttack | ValidCreature$ Card.Self  (attacks each combat if able)
    ///   S:Mode$ CantBlockBy | ValidAttacker$ Creature.Self | ValidBlocker$ Creature.Wall
    ///
    /// Card compat: Diamond Valley (cardsfolder/d/diamond_valley.txt)
    ///
    /// Script:
    ///   Types:Land
    ///   A:AB$ GainLife | Cost$ T Sac<1/Creature> | LifeAmount$ X
    ///   SVar:X:Sacrificed$CardToughness
    ///
    /// Verifies the activated ability survives loading with a dynamic
    /// `GainLifeDynamic(SacrificedToughness)` effect (it gains life equal to the
    /// sacrificed creature's toughness, CR 119.3 / 608.2g last-known information).
    /// Pre-fix bug (mtg-713 B10): the GainLife converter only accepted an integer
    /// `LifeAmount$`, so `LifeAmount$ X` -> `Sacrificed$CardToughness` returned
    /// None and the whole activated ability was dropped (never offered).
    #[test]
    fn test_card_compat_diamond_valley() {
        use crate::core::{DynamicAmount, Effect};
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/d/diamond_valley.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Diamond Valley should load");

        assert_eq!(def.name.as_str(), "Diamond Valley");
        assert!(def.types.contains(&CardType::Land));

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));
        let gainlife = card
            .activated_abilities
            .iter()
            .find(|ab| {
                ab.effects.iter().any(|e| {
                    matches!(
                        e,
                        Effect::GainLifeDynamic {
                            amount: DynamicAmount::SacrificedToughness,
                            ..
                        }
                    )
                })
            })
            .unwrap_or_else(|| {
                panic!(
                    "Diamond Valley must have an activated ability with \
                     GainLifeDynamic(SacrificedToughness). Abilities: {:?}",
                    card.activated_abilities
                )
            });
        // And the cost must be {T} + Sacrifice a creature.
        assert!(
            matches!(&gainlife.cost, crate::core::Cost::Composite(_)),
            "Diamond Valley's cost should be composite (Tap + Sacrifice). Cost: {:?}",
            gainlife.cost
        );
    }

    /// Verifies the `S:Mode$ MustAttack | ValidCreature$ Card.Self` self-static
    /// surfaces as Keyword::MustAttack on the instantiated card (CR 508.1a). The
    /// declare-attackers enforcement that consumes this keyword is exercised
    /// end-to-end in tests/puzzle_e2e.rs::test_juggernaut_must_attack.
    #[test]
    fn test_card_compat_juggernaut() {
        use crate::core::Keyword;
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/j/juggernaut.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Juggernaut should load");

        assert_eq!(def.name.as_str(), "Juggernaut");
        assert_eq!(def.mana_cost.generic, 4, "Cost should be {{4}}");
        assert!(def.types.contains(&CardType::Artifact));
        assert!(def.types.contains(&CardType::Creature));
        assert_eq!(def.power, Some(5));
        assert_eq!(def.toughness, Some(3));

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));
        assert!(
            card.keywords.contains(Keyword::MustAttack),
            "Juggernaut's `S:Mode$ MustAttack | ValidCreature$ Card.Self` must surface \
             as Keyword::MustAttack. Keywords: {:?}",
            card.keywords
        );
    }

    /// Build a bare vanilla creature with the given base power for filter tests.
    fn make_creature_with_power(power: i8) -> crate::core::Card {
        let mut c = crate::core::Card::new(CardId::new(99), "FilterTarget", PlayerId::new(1));
        c.set_types(smallvec::SmallVec::from_vec(vec![CardType::Creature]));
        c.set_base_power(Some(power));
        c.set_base_toughness(Some(power.max(1)));
        c
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

    /// Card compat: Fellwar Stone (cardsfolder/f/fellwar_stone.txt) — mtg-ontwf
    ///
    /// Script:
    ///   ManaCost:2
    ///   Types:Artifact
    ///   A:AB$ ManaReflected | Cost$ T | ColorOrType$ Color | Valid$ Land.OppCtrl
    ///     | ReflectProperty$ Produce | SpellDescription$ Add one mana of any color
    ///       that a land an opponent controls could produce.
    ///
    /// Verifies (parser): {2} Artifact whose ManaReflected ability (a) parses as
    /// a mana ability (CR 605), (b) carries an AddMana effect (no longer a no-op
    /// silent drop), and (c) is flagged produces_reflected_mana so the activation
    /// path constrains the produced color to the reflected set. The static cache
    /// derives AnyColor (upper bound). Runtime color constraint is verified by
    /// puzzle_e2e test_fellwar_stone_reflected_mana.
    #[test]
    fn test_card_compat_fellwar_stone() {
        use crate::core::{Effect, ManaProductionKind};
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/f/fellwar_stone.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Fellwar Stone should load");
        assert_eq!(def.name.as_str(), "Fellwar Stone");
        assert_eq!(def.mana_cost.generic, 2, "Fellwar Stone costs {{2}}");
        assert!(def.types.contains(&CardType::Artifact));

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));

        // The ManaReflected ability must parse as a mana ability that produces
        // mana (AddMana) and is flagged reflected.
        let reflected = card
            .activated_abilities
            .iter()
            .find(|ab| ab.produces_reflected_mana)
            .expect("Fellwar Stone must have a produces_reflected_mana ability (AB$ ManaReflected)");
        assert!(
            reflected.is_mana_ability,
            "ManaReflected must be a mana ability (CR 605)"
        );
        assert!(
            reflected.effects.iter().any(|e| matches!(e, Effect::AddMana { .. })),
            "ManaReflected must carry an AddMana effect (not a silent no-op). Got: {:?}",
            reflected.effects
        );

        // Static cache derives AnyColor (upper bound; the real set is dynamic).
        assert!(
            matches!(card.definition.cache.mana_production.kind, ManaProductionKind::AnyColor),
            "Fellwar Stone's cached production should be AnyColor. Got: {:?}",
            card.definition.cache.mana_production.kind
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

    /// Card compat: Earthquake Dragon (cardsfolder/e/earthquake_dragon.txt)
    ///
    /// Script:
    ///   ManaCost:14 G
    ///   Types:Creature Elemental Dragon
    ///   PT:10/10
    ///   K:Flying
    ///   K:Trample
    ///   S:Mode$ ReduceCost | ValidCard$ Card.Self | Type$ Spell | Amount$ X ...
    ///   A:AB$ ChangeZone | Cost$ 2 G Sac<1/Land> | Origin$ Graveyard
    ///                    | Destination$ Hand | ActivationZone$ Graveyard ...
    ///
    /// Verifies:
    /// - Static side: {14}{G} 10/10 with Flying + Trample.
    /// - Graveyard-return activated ability parses with `activation_zone ==
    ///   Zone::Graveyard` (fix for mtg-d8zuh).
    ///
    /// Cast/flying/combat: tests/earthquake_dragon_flying_e2e.sh
    /// Graveyard return: tests/earthquake_dragon_graveyard_return_e2e.sh
    /// Beads: mtg-502, mtg-d8zuh.
    #[test]
    fn test_card_compat_earthquake_dragon() {
        use crate::core::Keyword;
        use crate::zones::Zone;
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/e/earthquake_dragon.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Earthquake Dragon should load");
        assert_eq!(def.name.as_str(), "Earthquake Dragon");
        assert_eq!(def.mana_cost.generic, 14, "Cost generic should be 14");
        assert_eq!(def.mana_cost.green, 1, "Cost should require {{G}}");
        assert!(def.types.contains(&CardType::Creature));
        assert_eq!(def.power, Some(10));
        assert_eq!(def.toughness, Some(10));

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));
        assert!(
            card.keywords.contains(Keyword::Flying),
            "Earthquake Dragon must have Flying. Keywords: {:?}",
            card.keywords
        );
        assert!(
            card.keywords.contains(Keyword::Trample),
            "Earthquake Dragon must have Trample. Keywords: {:?}",
            card.keywords
        );

        // Verify the graveyard-return activated ability parses with Zone::Graveyard
        // (fix for mtg-d8zuh: ActivationZone$ Graveyard now stored on the ability).
        let graveyard_ab = card
            .activated_abilities
            .iter()
            .find(|ab| ab.activation_zone == Zone::Graveyard);
        assert!(
            graveyard_ab.is_some(),
            "Earthquake Dragon must have a graveyard-activated ability (ActivationZone$ Graveyard). \
             Abilities: {:?}",
            card.activated_abilities
                .iter()
                .map(|ab| (ab.description.as_str(), &ab.activation_zone))
                .collect::<Vec<_>>()
        );
    }

    /// Card compat: Concordant Crossroads (cardsfolder/c/concordant_crossroads.txt)
    ///
    /// Script:
    ///   ManaCost:G
    ///   Types:World Enchantment
    ///   S:Mode$ Continuous | Affected$ Creature | AddKeyword$ Haste
    ///                      | Description$ All creatures have haste.
    ///
    /// Verifies the parser turns the single continuous static line into a
    /// GrantKeyword(Haste) static ability whose selector is AllCreatures
    /// (bare `Affected$ Creature` ⇒ ALL creatures on the battlefield, both
    /// players — CR 702.10b haste, granted globally). The granted-haste
    /// gameplay (attack the turn it enters) is verified end-to-end by
    /// tests/concordant_crossroads_haste_e2e.sh. Beads: mtg-492.
    #[test]
    fn test_card_compat_concordant_crossroads() {
        use crate::core::effects::AffectedSelector;
        use crate::core::{Keyword, StaticAbility};
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/c/concordant_crossroads.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Concordant Crossroads should load");
        assert_eq!(def.name.as_str(), "Concordant Crossroads");
        // {G} Enchantment (World supertype is not tracked as a distinct field).
        assert_eq!(def.mana_cost.green, 1, "Cost should require {{G}}");
        assert_eq!(def.mana_cost.generic, 0, "Cost should have no generic mana");
        assert!(def.types.contains(&CardType::Enchantment), "Should be an Enchantment");

        // The single S: line must parse into a GrantKeyword(Haste) static
        // ability affecting ALL creatures (not just YouCtrl, not Self).
        let card = def.instantiate(CardId::new(1), PlayerId::new(0));
        let grants_haste_to_all = card.static_abilities.iter().any(|ability| {
            matches!(
                ability,
                StaticAbility::GrantKeyword {
                    affected: AffectedSelector::AllCreatures,
                    keyword: Keyword::Haste,
                    ..
                }
            )
        });
        assert!(
            grants_haste_to_all,
            "Concordant Crossroads must grant Haste to AllCreatures. \
             If absent, the S:Mode$ Continuous | Affected$ Creature | AddKeyword$ Haste \
             line was dropped or mis-scoped. Got: {:?}",
            card.static_abilities
        );
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

    /// Card compat: Swords to Plowshares (cardsfolder/s/swords_to_plowshares.txt)
    /// — mtg-297 / mtg-547.
    ///
    /// Script: A:SP$ ChangeZone (Battlefield->Exile) | SubAbility$ DBGainLife
    ///   SVar:DBGainLife:DB$ GainLife | Defined$ TargetedController | LifeAmount$ X
    ///   SVar:X:Targeted$CardPower
    ///
    /// Parser shape: the instantiated card must carry BOTH the exile effect and
    /// the dynamic `GainLifeDynamic(TargetPower)` for the exiled creature's
    /// controller. Runtime life gain is verified by the in-game resolution test
    /// `test_swords_to_plowshares_exile`.
    #[test]
    fn test_card_compat_swords_to_plowshares() {
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/s/swords_to_plowshares.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Swords to Plowshares should load");
        assert_eq!(def.name.as_str(), "Swords to Plowshares");
        assert!(def.types.contains(&CardType::Instant), "Swords must be an Instant");

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));
        let effects = &card.effects;
        assert!(
            effects.iter().any(|e| matches!(e, Effect::ExilePermanent { .. })),
            "Swords must exile its target. Effects: {:?}",
            effects
        );
        let gainlife = effects.iter().find_map(|e| {
            if let Effect::GainLifeDynamic { amount, player, .. } = e {
                Some((amount.clone(), *player))
            } else {
                None
            }
        });
        let (amount, player) = gainlife.expect("Swords must have a GainLifeDynamic sub-effect");
        assert_eq!(
            amount,
            crate::core::DynamicAmount::TargetPower,
            "Swords' life amount must be the target's power"
        );
        assert!(
            player.is_target_controller(),
            "Swords' life goes to the targeted creature's controller (Defined$ TargetedController)"
        );
    }

    /// Card compat: Divine Offering (cardsfolder/d/divine_offering.txt) — mtg-500.
    ///
    /// Script: A:SP$ Destroy | ValidTgts$ Artifact | SubAbility$ DBGainLife
    ///   SVar:DBGainLife:DB$ GainLife | Defined$ You | LifeAmount$ X
    ///   SVar:X:Targeted$CardManaCost
    ///
    /// Parser shape: destroy + dynamic `GainLifeDynamic(TargetManaValue)` for
    /// the caster. Runtime verified by `test_divine_offering_destroy_gain_mana_value`.
    #[test]
    fn test_card_compat_divine_offering() {
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/d/divine_offering.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Divine Offering should load");
        assert_eq!(def.name.as_str(), "Divine Offering");
        assert!(
            def.types.contains(&CardType::Instant),
            "Divine Offering must be an Instant"
        );

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));
        let effects = &card.effects;
        assert!(
            effects.iter().any(|e| matches!(e, Effect::DestroyPermanent { .. })),
            "Divine Offering must destroy its target. Effects: {:?}",
            effects
        );
        let gainlife = effects.iter().find_map(|e| {
            if let Effect::GainLifeDynamic { amount, player, .. } = e {
                Some((amount.clone(), *player))
            } else {
                None
            }
        });
        let (amount, player) = gainlife.expect("Divine Offering must have a GainLifeDynamic sub-effect");
        assert_eq!(
            amount,
            crate::core::DynamicAmount::TargetManaValue,
            "Divine Offering's life amount must be the target's mana value"
        );
        assert!(
            player.is_placeholder(),
            "Divine Offering's life goes to the caster (Defined$ You)"
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

    /// Card compat: Disintegrate (mtg-ioesm; cardsfolder/d/disintegrate.txt)
    ///
    /// Script: ManaCost:X R / Types:Sorcery
    ///   A:SP$ DealDamage | ValidTgts$ Any | NumDmg$ X | SubAbility$ DBEffect
    ///       | ReplaceDyingDefined$ ThisTargetedCard.Creature
    ///   SVar:X:Count$xPaid
    ///
    /// Parser shape: {X}{R} Sorcery, X damage to any target (DealDamageXPaid),
    /// PLUS an ExileIfWouldDieThisTurn rider synthesized from the
    /// `ReplaceDyingDefined$` clause ("if it would die this turn, exile it
    /// instead"). The rider must bind to the parent target via the
    /// reuse-previous sentinel so it never collects its own cast-time target.
    /// Card compat: Fireball (cardsfolder/f/fireball.txt)
    ///
    /// Script: ManaCost:X R / Types:Sorcery
    ///   S:Mode$ RaiseCost | ValidCard$ Card.Self | Relative$ True | ...
    ///   A:SP$ DealDamage | ValidTgts$ Any | NumDmg$ X | TargetMin$ 0
    ///     | TargetMax$ MaxTargets | DivideEvenly$ RoundedDown
    ///   SVar:X:Count$xPaid
    ///
    /// Parser shape: {X}{R} Sorcery whose SP$ DealDamage carries DivideEvenly$
    /// RoundedDown (lowered to Effect::DealDamageXPaid { divide:
    /// EvenlyRoundedDown }) AND whose self-referential Relative$ True RaiseCost
    /// sets the `spell_relative_target_cost` cache flag. Both are queried
    /// structurally (tokenized AbilityParams / cache), never via substring
    /// matching. This is the parser-shape regression guard for mtg-tyvcn.
    #[test]
    fn test_card_compat_fireball() {
        use crate::core::{DamageDivision, Effect};
        use crate::loader::ability_parser::{AbilityParams, ApiType};
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/f/fireball.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Fireball should load");
        assert_eq!(def.name.as_str(), "Fireball");
        assert!(def.mana_cost.has_x(), "Fireball cost must contain X");
        assert_eq!(def.mana_cost.red, 1, "Fireball cost should require {{R}}");
        assert!(def.types.contains(&CardType::Sorcery), "Fireball must be a Sorcery");

        // SP$ DealDamage with the variable-target + divide parameters.
        let dmg = def
            .raw_abilities
            .iter()
            .find_map(|raw| {
                let p = AbilityParams::parse(raw).ok()?;
                (p.api_type == ApiType::DealDamage).then_some(p)
            })
            .expect("Fireball must have an SP$ DealDamage spell ability");
        assert_eq!(dmg.get("NumDmg"), Some("X"), "Fireball deals X");
        assert_eq!(dmg.get("ValidTgts"), Some("Any"), "Fireball targets any target");
        assert_eq!(dmg.get("TargetMin"), Some("0"), "Fireball has TargetMin$ 0");
        assert_eq!(
            dmg.get("TargetMax"),
            Some("MaxTargets"),
            "Fireball has TargetMax$ MaxTargets"
        );
        assert_eq!(
            dmg.get("DivideEvenly"),
            Some("RoundedDown"),
            "Fireball divides its damage evenly, rounded down"
        );

        // Instantiate to materialize the parsed effects + cache.
        let card = def.instantiate(crate::core::CardId::new(1), crate::core::PlayerId::new(0));
        // The DealDamageXPaid effect must carry the EvenlyRoundedDown division.
        assert!(
            card.effects.iter().any(|e| matches!(
                e,
                Effect::DealDamageXPaid {
                    target: crate::core::TargetRef::None,
                    divide: DamageDivision::EvenlyRoundedDown,
                }
            )),
            "Fireball must produce DealDamageXPaid {{ divide: EvenlyRoundedDown }}. Got: {:?}",
            card.effects
        );
        // The relative per-target cost cache flag must be set.
        assert!(
            card.definition.cache.spell_relative_target_cost,
            "Fireball's Relative$ True RaiseCost must set spell_relative_target_cost"
        );
    }

    /// Card compat: Chain Lightning (cardsfolder/c/chain_lightning.txt) — mtg-489
    ///
    /// Script: ManaCost:R / Types:Sorcery
    ///   A:SP$ DealDamage | ValidTgts$ Any | NumDmg$ 3 | SubAbility$ DBCopy1
    ///   SVar:DBCopy1:DB$ CopySpellAbility | Defined$ Parent
    ///     | Controller$ TargetedOrController | UnlessPayer$ TargetedOrController
    ///     | UnlessCost$ R R | MayChooseTarget$ True
    ///
    /// Parser shape: {R} Sorcery whose PRIMARY mode is a fixed 3-damage
    /// `SP$ DealDamage | ValidTgts$ Any` (lowered to Effect::DealDamage), with a
    /// chained `DB$ CopySpellAbility | Defined$ Parent` SubAbility (lowered to
    /// Effect::CopySpellAbility { defined_source: CopySpellSource::Parent }).
    /// The Parent-source copy is the unimplemented optional "pay {R}{R} to copy"
    /// chain (engine gap mtg-152); the primary 3-damage burn is fully WORKING.
    /// All queries are tokenized AbilityParams lookups, never substring matching.
    #[test]
    fn test_card_compat_chain_lightning() {
        use crate::core::effects::CopySpellSource;
        use crate::core::Effect;
        use crate::loader::ability_parser::{AbilityParams, ApiType};
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/c/chain_lightning.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Chain Lightning should load");
        assert_eq!(def.name.as_str(), "Chain Lightning");
        assert_eq!(def.mana_cost.red, 1, "Chain Lightning cost should require {{R}}");
        assert!(
            def.types.contains(&CardType::Sorcery),
            "Chain Lightning must be a Sorcery"
        );

        // PRIMARY: SP$ DealDamage dealing a FIXED 3 to any target.
        let dmg = def
            .raw_abilities
            .iter()
            .find_map(|raw| {
                let p = AbilityParams::parse(raw).ok()?;
                (p.api_type == ApiType::DealDamage).then_some(p)
            })
            .expect("Chain Lightning must have an SP$ DealDamage spell ability");
        assert_eq!(dmg.get("NumDmg"), Some("3"), "Chain Lightning deals 3");
        assert_eq!(dmg.get("ValidTgts"), Some("Any"), "Chain Lightning targets any target");

        // Instantiate to materialize the parsed effects.
        let card = def.instantiate(crate::core::CardId::new(1), crate::core::PlayerId::new(0));
        // Primary 3-damage effect (the WORKING mode).
        assert!(
            card.effects
                .iter()
                .any(|e| matches!(e, Effect::DealDamage { amount: 3, .. })),
            "Chain Lightning must produce a fixed 3-damage DealDamage. Got: {:?}",
            card.effects
        );
        // The optional copy chain lowers to a Parent-source CopySpellAbility,
        // WRAPPED in an UnlessCostWrapper carrying the "{R}{R}" gate (the card's
        // `... | UnlessCost$ R R | UnlessPayer$ TargetedOrController |
        // UnlessSwitched$ True`). mtg-152: the copy is now IMPLEMENTED — the
        // wrap is what `follow_sub_ability_chain` applies so the optional payment
        // gates the copy (previously the gate was dropped and a bare
        // CopySpellAbility fired unconditionally).
        let copy_inner_is_parent = card.effects.iter().any(|e| {
            matches!(
                e,
                Effect::UnlessCostWrapper { inner_effect, .. }
                    if matches!(**inner_effect, Effect::CopySpellAbility { defined_source: CopySpellSource::Parent, .. })
            )
        });
        assert!(
            copy_inner_is_parent,
            "Chain Lightning must chain an UnlessCostWrapper {{ inner: CopySpellAbility {{ defined_source: Parent }} }} \
             carrying the {{R}}{{R}} gate. Got: {:?}",
            card.effects
        );
    }

    /// Regression: a BARE `CopySpellAbility` (no `Defined$`, the "copy a
    /// separately-TARGETED spell/ability" class — Twincast/Reverberate/Fork/
    /// Return the Favor) must lower to `CopySpellSource::TargetedSpell`, NOT
    /// `Parent`. Defaulting to Parent made these cards copy THEMSELVES forever
    /// (the commander-format infinite loop: Return the Favor self-copied without
    /// bound). Return the Favor is a Spree Charm whose `DBCopy` mode is exactly
    /// such a bare CopySpellAbility.
    #[test]
    fn test_bare_copyspellability_is_targeted_spell_not_parent() {
        use crate::core::effects::CopySpellSource;
        use crate::core::Effect;
        use crate::loader::ability_parser::AbilityParams;
        use crate::loader::effect_converter::params_to_effect;

        // Return the Favor's DBCopy SVar body (bare CopySpellAbility — has
        // TargetType$/ValidTgts$ naming ANOTHER spell, but NO Defined$).
        let params = AbilityParams::parse(
            "A:DB$ CopySpellAbility | TargetType$ Activated,Triggered,Instant,Sorcery | ValidTgts$ Card,Emblem | MayChooseTarget$ True",
        )
        .expect("bare CopySpellAbility must parse");
        let effect = params_to_effect(&params).expect("must convert");
        let Effect::CopySpellAbility { defined_source, .. } = &effect else {
            panic!("expected CopySpellAbility, got {:?}", effect);
        };
        assert_eq!(
            *defined_source,
            CopySpellSource::TargetedSpell,
            "a bare CopySpellAbility (no Defined$) must be TargetedSpell (a safe no-op), NOT Parent \
             (which self-copies forever — the commander hang)"
        );

        // And the explicit Defined$ Parent (Chain Lightning) must STILL be Parent.
        let cl = AbilityParams::parse("A:DB$ CopySpellAbility | Defined$ Parent | MayChooseTarget$ True")
            .expect("Defined$ Parent must parse");
        let cl_effect = params_to_effect(&cl).expect("must convert");
        assert!(
            matches!(
                cl_effect,
                Effect::CopySpellAbility {
                    defined_source: CopySpellSource::Parent,
                    ..
                }
            ),
            "explicit Defined$ Parent (Chain Lightning) must remain Parent. Got: {:?}",
            cl_effect
        );
    }

    /// Card compat: Drain Life (cardsfolder/d/drain_life.txt) — mtg-501 / mtg-624
    ///
    /// Script: ManaCost:X 1 B / Types:Sorcery
    ///   A:SP$ StoreSVar | ... | SubAbility$ StoreTgtPW       (cap-computing chain)
    ///   SVar:DBDamage:DB$ DealDamage | Defined$ Targeted | NumDmg$ X | SubAbility$ DBGainLife
    ///   SVar:DBGainLife:DB$ GainLife | Defined$ You | LifeAmount$ DrainedLifeCard
    ///   SVar:DrainedLifeCard:SVar$Y/LimitMax.Limit
    ///
    /// Parser shape: the StoreSVar head + chained StoreSVar nodes lower to silent
    /// Effect::NoOp (the cap is modeled directly, not via a runtime SVar store);
    /// the chain produces a DealDamage (X) and a GainLifeDynamic whose amount is
    /// DynamicAmount::DamageDealtCappedByTarget — "gain = min(damage dealt,
    /// target life/loyalty/toughness)". All queries are tokenized.
    #[test]
    fn test_card_compat_drain_life() {
        use crate::core::{DynamicAmount, Effect};
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/d/drain_life.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Drain Life should load");
        assert_eq!(def.name.as_str(), "Drain Life");
        assert!(def.types.contains(&CardType::Sorcery), "Drain Life must be a Sorcery");

        let card = def.instantiate(crate::core::CardId::new(1), crate::core::PlayerId::new(0));

        // The StoreSVar chain lowers to silent NoOp nodes (NOT Unimplemented).
        assert!(
            card.effects.iter().any(|e| matches!(e, Effect::NoOp { .. })),
            "Drain Life's StoreSVar chain must lower to Effect::NoOp (silent), not Unimplemented. Got: {:?}",
            card.effects
        );
        assert!(
            !card.effects.iter().any(|e| matches!(e, Effect::Unimplemented { .. })),
            "Drain Life must NOT contain an Unimplemented effect (StoreSVar is a recognized no-op). Got: {:?}",
            card.effects
        );
        // The damage half: `NumDmg$ X` lowers to DealDamageXPaid before casting
        // (X is resolved to a concrete DealDamage amount at cast time).
        assert!(
            card.effects
                .iter()
                .any(|e| matches!(e, Effect::DealDamage { .. } | Effect::DealDamageXPaid { .. })),
            "Drain Life must deal X damage (DealDamageXPaid). Got: {:?}",
            card.effects
        );
        // The life-gain half: GainLifeDynamic capped by the target characteristic.
        assert!(
            card.effects.iter().any(|e| matches!(
                e,
                Effect::GainLifeDynamic {
                    amount: DynamicAmount::DamageDealtCappedByTarget { .. },
                    ..
                }
            )),
            "Drain Life must gain life via DamageDealtCappedByTarget (min of damage dealt and target \
             life/loyalty/toughness). Got: {:?}",
            card.effects
        );
    }

    #[test]
    fn test_card_compat_disintegrate() {
        use crate::core::Effect;
        use crate::loader::ability_parser::{AbilityParams, ApiType};
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/d/disintegrate.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Disintegrate should load");
        assert_eq!(def.name.as_str(), "Disintegrate");
        assert_eq!(def.mana_cost.red, 1, "Cost should require {{R}}");
        assert!(def.types.contains(&CardType::Sorcery), "must be a Sorcery");

        // Primary SP$ DealDamage: X to any target, carrying the
        // ReplaceDyingDefined clause (tokenized, never substring-matched).
        let dmg = def
            .raw_abilities
            .iter()
            .find_map(|raw| {
                let p = AbilityParams::parse(raw).ok()?;
                (p.api_type == ApiType::DealDamage).then_some(p)
            })
            .expect("Disintegrate must have an SP$ DealDamage spell ability");
        assert_eq!(dmg.get("NumDmg"), Some("X"), "Disintegrate deals X");
        assert_eq!(dmg.get("ValidTgts"), Some("Any"), "Disintegrate targets any target");
        assert_eq!(
            dmg.get("ReplaceDyingDefined"),
            Some("ThisTargetedCard.Creature"),
            "Disintegrate must carry the exile-instead-of-dying clause"
        );

        // The synthesized effect list must contain BOTH the X-damage
        // (DealDamageXPaid, target None) and the ExileIfWouldDieThisTurn rider
        // bound to the reuse-previous sentinel. Instantiate to materialize the
        // parsed `effects` vec on the Card.
        let card = def.instantiate(crate::core::CardId::new(1), crate::core::PlayerId::new(0));
        let effects = &card.effects;
        assert!(
            effects.iter().any(
                |e| matches!(e, Effect::DealDamageXPaid { target, .. } if matches!(target, crate::core::TargetRef::None))
            ),
            "Disintegrate must produce a DealDamageXPaid (X to chosen target). Got: {:?}",
            effects
        );
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::ExileIfWouldDieThisTurn { target } if target.is_reuse_previous())),
            "Disintegrate must produce an ExileIfWouldDieThisTurn rider bound to the parent target. Got: {:?}",
            effects
        );
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

    /// Card compat: Paralyze (cardsfolder/p/paralyze.txt) — mtg-529
    ///
    /// Script (relevant lines):
    ///   K:Enchant:Creature
    ///   T:Mode$ ChangesZone | ... | Execute$ TrigTap        (ETB: tap enchanted)
    ///   R:Event$ Untap | Layer$ CantHappen
    ///     | ValidCard$ Creature.EnchantedBy ...             (doesn't untap)
    ///
    /// Parser-shape guard for the "doesn't untap" lock: the `R:Event$ Untap |
    /// Layer$ CantHappen` replacement must be lowered into a continuous
    /// GrantKeyword(DoesNotUntap) static targeting the enchanted creature. The
    /// untap step consults `has_keyword_with_effects(.., DoesNotUntap)` so the
    /// enchanted creature stays tapped on its controller's untap step.
    #[test]
    fn test_card_compat_paralyze() {
        use crate::core::{AffectedSelector, Keyword, StaticAbility};
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/p/paralyze.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Paralyze should load");
        assert_eq!(def.name.as_str(), "Paralyze");

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));

        // The R:Event$ Untap | Layer$ CantHappen replacement must lower into a
        // GrantKeyword(DoesNotUntap) on the enchanted creature.
        let has_doesnt_untap = card.static_abilities.iter().any(|s| {
            matches!(
                s,
                StaticAbility::GrantKeyword {
                    keyword: Keyword::DoesNotUntap,
                    affected: AffectedSelector::CreatureEnchantedBy,
                    ..
                }
            )
        });
        assert!(
            has_doesnt_untap,
            "Paralyze must grant DoesNotUntap to the enchanted creature \
             (R:Event$ Untap | Layer$ CantHappen). Statics: {:?}",
            card.static_abilities
        );

        // mtg-646: the third ability — "At the beginning of the upkeep of
        // enchanted creature's controller, that player may pay {4}. If they do,
        // untap the creature." — must parse into a BeginningOfUpkeep trigger
        // flagged `enchanted_controller_turn_only` (fires on the HOST's
        // controller's upkeep, not the Aura controller's), whose effect is an
        // UnlessCostWrapper { UntapPermanent } with a {4} mana cost and
        // switched=true (the untap runs ONLY if the {4} is paid). A bare
        // unconditional untap here would make Paralyze free to escape every
        // upkeep — strictly worse than not firing — so the UnlessCost gate is
        // load-bearing.
        use crate::core::effects::{UnlessCost, UnlessCostType};
        use crate::core::TriggerEvent;
        let upkeep = card
            .triggers
            .iter()
            .find(|t| t.event == TriggerEvent::BeginningOfUpkeep && t.enchanted_controller_turn_only)
            .expect(
                "Paralyze must have a BeginningOfUpkeep trigger flagged \
                 enchanted_controller_turn_only (ValidPlayer$ Player.EnchantedController). \
                 Dropping it makes the doesn't-untap lock permanent with no escape.",
            );
        let untap_unless = upkeep.effects.iter().any(|e| {
            matches!(
                e,
                Effect::UnlessCostWrapper {
                    inner_effect,
                    unless_cost: UnlessCost { cost: UnlessCostType::Mana(_), switched: true, .. },
                } if matches!(inner_effect.as_ref(), Effect::UntapPermanent { .. })
            )
        });
        assert!(
            untap_unless,
            "Paralyze's upkeep trigger must be an UnlessCostWrapper {{ UntapPermanent }} \
             with a {{4}} mana cost and switched=true (pay {{4}} -> untap). Got: {:?}",
            upkeep.effects
        );
    }

    /// Card compat: Psychic Purge (cardsfolder/p/psychic_purge.txt) — mtg-534
    ///
    /// Script:
    ///   ManaCost:U
    ///   Types:Sorcery
    ///   A:SP$ DealDamage | ValidTgts$ Any | NumDmg$ 1
    ///   T:Mode$ Discarded | ValidCard$ Card.Self
    ///     | ValidCause$ SpellAbility.OppCtrl | Execute$ TrigLoseLife
    ///   SVar:TrigLoseLife:DB$ LoseLife | Defined$ TriggeredCauseController
    ///     | LifeAmount$ 5
    ///
    /// Parser-shape guard for the opponent-forced-discard punisher (mtg-648):
    /// the `T:Mode$ Discarded | ValidCard$ Card.Self` self-trigger must lower
    /// into a `TriggerEvent::Discarded` flagged `requires_opponent_cause`
    /// (`ValidCause$ SpellAbility.OppCtrl`) whose effect is a `LoseLife 5`
    /// targeting the `triggered_cause_controller` sentinel. A silent drop would
    /// strip Psychic Purge's downside-for-the-opponent clause.
    #[test]
    fn test_card_compat_psychic_purge() {
        use crate::core::TriggerEvent;
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/p/psychic_purge.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Psychic Purge should load");
        assert_eq!(def.name.as_str(), "Psychic Purge");
        assert!(def.types.contains(&CardType::Sorcery));

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));

        let discarded = card
            .triggers
            .iter()
            .find(|t| t.event == TriggerEvent::Discarded)
            .expect(
                "Psychic Purge must have a Discarded self-trigger \
                 (T:Mode$ Discarded | ValidCard$ Card.Self). Silently dropping it \
                 removes the opponent-discard punisher.",
            );
        assert!(
            discarded.requires_opponent_cause,
            "Psychic Purge's Discarded trigger must set requires_opponent_cause \
             (ValidCause$ SpellAbility.OppCtrl) so it only fires on an OPPONENT-forced discard.",
        );
        let loses_5_to_cause = discarded.effects.iter().any(|e| {
            matches!(
                e,
                Effect::LoseLife { player, amount: 5 } if player.is_triggered_cause_controller()
            )
        });
        assert!(
            loses_5_to_cause,
            "Psychic Purge's Discarded trigger must make the discard's cause controller \
             lose 5 life (LoseLife 5 -> TriggeredCauseController). Got: {:?}",
            discarded.effects
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

    /// Card compat: Karma (cardsfolder/k/karma.txt) — mtg-516
    ///
    /// Script:
    ///   ManaCost:2 W W
    ///   Types:Enchantment
    ///   T:Mode$ Phase | Phase$ Upkeep | ValidPlayer$ Player | Execute$ TrigDamage
    ///   SVar:TrigDamage:DB$ DealDamage | Defined$ TriggeredPlayer | NumDmg$ X
    ///   SVar:X:Count$Valid Swamp.ActivePlayerCtrl
    ///
    /// Asserts the parsed shape: {2}{W}{W} Enchantment carrying a
    /// BeginningOfUpkeep trigger whose effect is the variable-amount
    /// DealDamageToTriggeredPlayer (counting the active player's Swamps).
    /// Before the fix the trigger parsed but produced NO effect (the phase
    /// trigger handler only recognised `Defined$ You` + fixed NumDmg), so
    /// Karma was a silent no-op — strictly weaker than printed. Gameplay
    /// behavior (each player's upkeep, damage = that player's Swamps) is
    /// covered by tests/karma_upkeep_swamp_damage_e2e.sh.
    #[test]
    fn test_card_compat_karma() {
        use crate::core::{CountExpression, TriggerEvent};
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/k/karma.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Karma should load");

        assert_eq!(def.name.as_str(), "Karma");
        assert_eq!(def.mana_cost.white, 2, "Cost should be {{2}}{{W}}{{W}}");
        assert_eq!(def.mana_cost.generic, 2, "Cost should be {{2}}{{W}}{{W}}");
        assert!(def.types.contains(&CardType::Enchantment));

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));

        let upkeep_trigger = card
            .triggers
            .iter()
            .find(|t| t.event == TriggerEvent::BeginningOfUpkeep)
            .expect(
                "Karma must have a BeginningOfUpkeep trigger. Silently dropping it \
                 makes the card a no-op (strictly weaker than printed).",
            );
        // The trigger must fire on EVERY player's upkeep (ValidPlayer$ Player),
        // not just the controller's — so it must NOT be controller_turn_only.
        assert!(
            !upkeep_trigger.controller_turn_only,
            "Karma fires on each player's upkeep, not only the controller's"
        );
        let damage_effect = upkeep_trigger
            .effects
            .iter()
            .find(|e| matches!(e, Effect::DealDamageToTriggeredPlayer { .. }))
            .expect("Karma's upkeep trigger must deal variable damage to the triggered player");
        let Effect::DealDamageToTriggeredPlayer { count, target_self } = damage_effect else {
            unreachable!("matched DealDamageToTriggeredPlayer above");
        };
        let target_self = *target_self;
        assert!(
            !target_self,
            "Karma damages the triggered (active) player, not its own controller"
        );
        assert!(
            matches!(count, CountExpression::ValidPermanents { filter, .. } if filter == "Swamp.ActivePlayerCtrl"),
            "Karma's damage count must be Count$Valid Swamp.ActivePlayerCtrl. Got: {:?}",
            count
        );
    }

    /// Card compat: Black Vise (cardsfolder/b/black_vise.txt) — mtg-cuf0e
    ///
    /// Script:
    ///   ManaCost:1
    ///   Types:Artifact
    ///   K:ETBReplacement:Other:ChooseP
    ///   SVar:ChooseP:DB$ ChoosePlayer | Defined$ You | Choices$ Player.Opponent | ...
    ///   T:Mode$ Phase | Phase$ Upkeep | ValidPlayer$ Player.Chosen | TriggerZones$ Battlefield | Execute$ TrigDamage
    ///   SVar:TrigDamage:DB$ DealDamage | Defined$ ChosenPlayer | NumDmg$ X
    ///   SVar:X:Count$ValidHand Card.ChosenCtrl/Minus.4
    ///
    /// Asserts the parsed shape: a {1} Artifact that (a) flags the ETB
    /// ChoosePlayer replacement (`cache.etb_choose_player`), and (b) carries a
    /// BeginningOfUpkeep trigger that is `chosen_player_turn_only` (the
    /// `ValidPlayer$ Player.Chosen` gate) whose effect is the variable
    /// DealDamageToTriggeredPlayer counting `Count$ValidHand .../Minus.4`. Before
    /// the fix the count parsed to Fixed(0) and there was no chosen-player slot,
    /// so Black Vise was a silent 0-damage no-op. Gameplay behavior (damage =
    /// max(0, hand-4) on the chosen player's upkeep only) is covered by
    /// tests/black_vise_chosen_upkeep_damage_e2e.sh and the native-vs-WASM leg.
    #[test]
    fn test_card_compat_black_vise() {
        use crate::core::{CountExpression, CountModifier, TriggerEvent};
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/b/black_vise.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Black Vise should load");

        assert_eq!(def.name.as_str(), "Black Vise");
        assert_eq!(def.mana_cost.generic, 1, "Cost should be {{1}}");
        assert!(def.types.contains(&CardType::Artifact));

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));

        // (a) ETB ChoosePlayer replacement flagged (drives the per-permanent
        // chosen-player slot at ETB).
        assert!(
            card.definition.cache.etb_choose_player,
            "Black Vise must flag its ETB ChoosePlayer replacement; without it no \
             chosen player is recorded and the upkeep trigger can never fire."
        );

        // (b) Upkeep trigger gated to the chosen player's turn.
        let upkeep_trigger = card
            .triggers
            .iter()
            .find(|t| t.event == TriggerEvent::BeginningOfUpkeep)
            .expect("Black Vise must have a BeginningOfUpkeep trigger.");
        assert!(
            upkeep_trigger.chosen_player_turn_only,
            "Black Vise's upkeep trigger fires only on the CHOSEN player's turn \
             (ValidPlayer$ Player.Chosen), so it must be chosen_player_turn_only."
        );
        assert!(
            !upkeep_trigger.controller_turn_only,
            "Black Vise fires on the chosen player's upkeep, not the controller's."
        );

        let damage_effect = upkeep_trigger
            .effects
            .iter()
            .find(|e| matches!(e, Effect::DealDamageToTriggeredPlayer { .. }))
            .expect("Black Vise's upkeep trigger must deal variable damage to the chosen player");
        let Effect::DealDamageToTriggeredPlayer { count, target_self } = damage_effect else {
            unreachable!("matched DealDamageToTriggeredPlayer above");
        };
        assert!(
            !*target_self,
            "Black Vise damages the chosen (triggered/active) player, not its own controller."
        );
        // Count$ValidHand Card.ChosenCtrl/Minus.4 -> CardsInHand{ minus: 4 }.
        assert!(
            matches!(
                count,
                CountExpression::CardsInHand {
                    modifier: CountModifier::Minus(4),
                    ..
                }
            ),
            "Black Vise's damage count must be Count$ValidHand .../Minus.4 \
             (CardsInHand with Minus(4)). Got: {:?}",
            count
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

    /// Card compat: Icy Manipulator (cardsfolder/i/icy_manipulator.txt) — mtg-511
    ///
    /// Script:
    ///   ManaCost:4
    ///   Types:Artifact
    ///   A:AB$ Tap | Cost$ 1 T | ValidTgts$ Artifact,Creature,Land | SpellDescription$ Tap target artifact, creature, or land.
    ///
    /// Parser-shape regression: {4} Artifact with exactly one activated ability
    /// whose effect is TapPermanent, costing {1}+T. The ability must NOT be
    /// classified as a mana ability (so the heuristic can include it in
    /// should_activate_ability / TapTarget classification). Silent drop of the
    /// ability or mis-classification as mana ability makes Icy Manipulator a
    /// useless 4-mana artifact.
    ///
    /// Heuristic targeting (mtg-zssaf fix): the heuristic controller now
    /// correctly targets an opponent's permanent (not its own source) when
    /// a TapPermanent activated ability fires. Verified by game log in
    /// tests/icy_manipulator_taps_opponent_e2e.sh.
    #[test]
    fn test_card_compat_icy_manipulator() {
        use crate::core::Effect;
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/i/icy_manipulator.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Icy Manipulator should load");

        assert_eq!(def.name.as_str(), "Icy Manipulator");
        assert_eq!(def.mana_cost.generic, 4, "Cost should be {{4}}");
        assert_eq!(
            def.mana_cost.white + def.mana_cost.blue + def.mana_cost.black + def.mana_cost.red + def.mana_cost.green,
            0,
            "Cost should be purely colorless"
        );
        assert!(def.types.contains(&CardType::Artifact));

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));

        // Must have exactly one activated ability
        assert_eq!(
            card.activated_abilities.len(),
            1,
            "Icy Manipulator must have exactly one activated ability. \
             Silent drop makes it a useless 4-mana artifact."
        );

        let ab = &card.activated_abilities[0];

        // Must NOT be classified as a mana ability
        assert!(
            !ab.is_mana_ability,
            "Icy Manipulator's Tap ability must NOT be a mana ability — \
             it is a targeted tap effect (TapPermanent), not mana production."
        );

        // The ability must produce a TapPermanent effect
        assert!(
            ab.effects.iter().any(|e| matches!(e, Effect::TapPermanent { .. })),
            "Icy Manipulator's activated ability must produce a TapPermanent effect. \
             Got: {:?}",
            ab.effects
        );

        // Cost must include a tap symbol (i.e., the cost itself taps the source)
        assert!(
            ab.cost.includes_tap(),
            "Icy Manipulator's activation cost must include a tap symbol. \
             Got: {:?}",
            ab.cost
        );
    }

    /// Card compat: Red Elemental Blast / Blue Elemental Blast — mtg-536 / mtg-487
    ///
    /// Script (REBL):
    ///   A:SP$ Charm | Choices$ DBCounter,DBDestroy
    ///   SVar:DBCounter:DB$ Counter | TargetType$ Spell | ValidTgts$ Card.Blue
    ///   SVar:DBDestroy:DB$ Destroy | ValidTgts$ Permanent.Blue
    ///
    /// Parser-shape regression for the Charm per-mode color restriction
    /// (mtg-af24s): the Counter mode must carry `spell_restriction.required_color = Blue`
    /// on its CounterSpell, and the Destroy mode must carry `required_color = Blue` on
    /// its DestroyPermanent restriction. Previously the color was dropped, so
    /// REBL could destroy/counter objects of any color (illegal targeting,
    /// CR 115.4). BEBL is the mirror with Red.
    #[test]
    fn test_card_compat_elemental_blasts() {
        use crate::core::{Color, Effect};
        use std::path::PathBuf;

        let check = |path: &str, name: &str, color: Color| {
            let p = PathBuf::from(path);
            if !p.exists() {
                eprintln!("Skipping: cardsfolder not present at {:?}", p);
                return;
            }
            let def = crate::loader::CardLoader::load_from_file(&p).unwrap_or_else(|_| panic!("{} should load", name));
            assert_eq!(def.name.as_str(), name);
            assert!(def.types.contains(&CardType::Instant));
            let card = def.instantiate(CardId::new(1), PlayerId::new(0));

            // The single SP$ Charm ability holds a ModalChoice with two modes.
            let modal = card
                .effects
                .iter()
                .find_map(|e| {
                    if let Effect::ModalChoice { modes, .. } = e {
                        Some(modes)
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| panic!("{} must parse a ModalChoice", name));

            let mut saw_counter_color = false;
            let mut saw_destroy_color = false;
            for mode in modal {
                if let Effect::CounterSpell { spell_restriction, .. } = mode.effect.as_ref() {
                    assert_eq!(
                        spell_restriction.required_color,
                        Some(color),
                        "{}: counter mode color",
                        name
                    );
                    saw_counter_color = true;
                }
                if let Effect::DestroyPermanent { restriction, .. } = mode.effect.as_ref() {
                    assert_eq!(restriction.required_color, Some(color), "{}: destroy mode color", name);
                    saw_destroy_color = true;
                }
            }
            assert!(saw_counter_color, "{}: must have a color-restricted Counter mode", name);
            assert!(saw_destroy_color, "{}: must have a color-restricted Destroy mode", name);
        };

        check(
            "../cardsfolder/r/red_elemental_blast.txt",
            "Red Elemental Blast",
            Color::Blue,
        );
        check(
            "../cardsfolder/b/blue_elemental_blast.txt",
            "Blue Elemental Blast",
            Color::Red,
        );
    }

    /// Card compat: Armageddon (cardsfolder/a/armageddon.txt) — mtg-481
    ///
    /// Script:
    ///   ManaCost:3 W
    ///   Types:Sorcery
    ///   A:SP$ DestroyAll | ValidCards$ Land
    ///
    /// Parser-shape regression: Armageddon must parse as a {3}{W} Sorcery whose
    /// spell ability is an Effect::DestroyAll restricted to lands. The full
    /// "destroy every land, creatures survive" behavior is exercised by
    /// tests/armageddon_destroys_lands_e2e.sh.
    #[test]
    fn test_card_compat_armageddon() {
        use crate::core::Effect;
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/a/armageddon.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Armageddon should load");
        assert_eq!(def.name.as_str(), "Armageddon");
        assert_eq!(def.mana_cost.generic, 3, "Cost should be {{3}}{{W}}");
        assert_eq!(def.mana_cost.white, 1, "Cost should be {{3}}{{W}}");
        assert!(def.types.contains(&CardType::Sorcery));

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));
        let has_destroy_all = card.effects.iter().any(|e| matches!(e, Effect::DestroyAll { .. }));
        assert!(
            has_destroy_all,
            "Armageddon must parse `SP$ DestroyAll | ValidCards$ Land` into an \
             Effect::DestroyAll. Got: {:?}",
            card.effects
        );
    }

    /// Card compat: Balance (cardsfolder/b/balance.txt) — mtg-483
    ///
    /// Script:
    ///   ManaCost:1 W
    ///   Types:Sorcery
    ///   A:SP$ Balance | Valid$ Land | SubAbility$ BalanceHands
    ///   SVar:BalanceHands:DB$ Balance | Zone$ Hand | SubAbility$ BalanceCreatures
    ///   SVar:BalanceCreatures:DB$ Balance | Valid$ Creature
    ///
    /// Parser-shape regression: Balance must parse as a {1}{W} Sorcery whose
    /// spell ability carries an Effect::Balance (the lands pass). The full
    /// land/hand/creature equalize chain is exercised by
    /// tests/balance_equalize_e2e.sh and test_balance_creature_sacrifice.
    #[test]
    fn test_card_compat_balance() {
        use crate::core::Effect;
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/b/balance.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Balance should load");
        assert_eq!(def.name.as_str(), "Balance");
        assert_eq!(def.mana_cost.generic, 1, "Cost should be {{1}}{{W}}");
        assert_eq!(def.mana_cost.white, 1, "Cost should be {{1}}{{W}}");
        assert!(def.types.contains(&CardType::Sorcery));

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));
        let has_balance = card.effects.iter().any(|e| matches!(e, Effect::Balance { .. }));
        assert!(
            has_balance,
            "Balance must parse its `SP$ Balance` ability into an Effect::Balance. Got: {:?}",
            card.effects
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
    /// Parser-shape regression (mtg-520): both halves — Untap AND the
    /// SubAbility$ damage-prevention effect — must parse. The fix emits
    /// `Effect::PreventAllCombatDamageThisTurn` for the `DB$ Effect |
    /// ReplacementEffects$ RPrevent1,RPrevent2 | RememberObjects$ Targeted`
    /// sub-ability, and `assign_combat_damage` checks
    /// `Card::prevent_all_combat_damage_this_turn` before assigning
    /// any combat damage to or from the targeted creature.
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
        let untap_ability = card
            .activated_abilities
            .iter()
            .find(|a| a.effects.iter().any(|e| matches!(e, Effect::UntapPermanent { .. })))
            .expect(
                "Maze of Ith must have an Untap activated ability \
                 (untap target attacking creature).",
            );

        // SubAbility (fix for mtg-520): PreventAllCombatDamageThisTurn must be
        // present in the same activated ability's effect list. The target starts
        // as a placeholder (CardId::new(0)) and is resolved at runtime from
        // last_resolved_target (set by the preceding UntapPermanent).
        let has_prevention = untap_ability
            .effects
            .iter()
            .any(|e| matches!(e, Effect::PreventAllCombatDamageThisTurn { .. }));
        assert!(
            has_prevention,
            "Maze of Ith activated ability must have PreventAllCombatDamageThisTurn \
             in its effect list (mtg-520 fix — DB$ Effect | ReplacementEffects$ RPrevent1,RPrevent2 \
             must not be silently dropped). Got effects: {:?}",
            untap_ability.effects
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

    /// Card compat: Recall (cardsfolder/r/recall.txt) — mtg-535.
    ///
    /// Script:
    ///   ManaCost:X X U
    ///   Types:Sorcery
    ///   A:SP$ Discard | Defined$ You | Mode$ TgtChoose | NumCards$ X
    ///        | RememberDiscarded$ True | SubAbility$ DBChangeZone
    ///   SVar:DBChangeZone:DB$ ChangeZone | Origin$ Graveyard | Destination$ Hand
    ///        | ChangeNum$ Y | ChangeType$ Card.YouOwn | SubAbility$ DBExile
    ///   SVar:DBExile:DB$ ChangeZone | Origin$ Stack | Destination$ Exile
    ///   SVar:X:Count$xPaid
    ///   SVar:Y:Remembered$Amount
    ///
    /// Regression (mtg-535): Previously, the DBChangeZone sub-ability was silently
    /// dropped because the ChangeZone converter matched `Origin$ Graveyard |
    /// Destination$ Hand` without `Defined$` as `MoveSelfBetweenZones` (self-return).
    /// The correct mapping for `ChangeNum$ Y` (Remembered$Amount) is
    /// `ReturnCardsFromGraveyardToHand`, which reads `remembered_cards.len()` at
    /// resolution time.
    ///
    /// Parser-shape assertions:
    ///   1. {X}{X}{U} Sorcery — ManaCost parses with 2 generic X pips + 1 blue.
    ///   2. SP$ Discard effect is present with RememberDiscarded$ True.
    ///   3. ReturnCardsFromGraveyardToHand effect is present in the effect list
    ///      (replacing the previous MoveSelfBetweenZones mis-mapping).
    ///   4. SelfExileFromStack effect is present (Recall exiles itself).
    #[test]
    fn test_card_compat_recall() {
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/r/recall.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Recall should load");
        assert_eq!(def.name.as_str(), "Recall");

        // ManaCost: X X U — two X pips and one blue
        assert_eq!(def.mana_cost.blue, 1, "Recall costs 1 blue");
        assert!(def.types.contains(&CardType::Sorcery), "Recall is a Sorcery");

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));

        // 1. Discard effect with RememberDiscarded$ True must be present.
        let has_discard_remember = card.effects.iter().any(|e| {
            matches!(
                e,
                Effect::DiscardCards {
                    remember_discarded: true,
                    ..
                } | Effect::DiscardCardsXPaid {
                    remember_discarded: true,
                    ..
                }
            )
        });
        assert!(
            has_discard_remember,
            "Recall must have a Discard effect with remember_discarded=true; got: {:?}",
            card.effects
        );

        // 2. ReturnCardsFromGraveyardToHand must be present (the fixed-mtg-535 shape).
        //    Before the fix, this was incorrectly emitted as MoveSelfBetweenZones.
        let has_return = card
            .effects
            .iter()
            .any(|e| matches!(e, Effect::ReturnCardsFromGraveyardToHand { .. }));
        assert!(
            has_return,
            "Recall must have a ReturnCardsFromGraveyardToHand effect (mtg-535 fix); \
             before the fix this was incorrectly MoveSelfBetweenZones. Got: {:?}",
            card.effects
        );

        // 3. SelfExileFromStack must be present (Recall exiles itself from the stack).
        let has_self_exile = card
            .effects
            .iter()
            .any(|e| matches!(e, Effect::SelfExileFromStack { .. }));
        assert!(
            has_self_exile,
            "Recall must have a SelfExileFromStack effect; got: {:?}",
            card.effects
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

    /// Card compat: Power Sink (cardsfolder/p/power_sink.txt) — mtg-532.
    ///
    /// Script: ManaCost:X U / Types:Instant
    ///   A:SP$ Counter | UnlessCost$ X | TargetType$ Spell | SubAbility$ TapLands
    ///   SVar:TapLands:DB$ TapAll | ValidCards$ Land.hasManaAbility
    ///       | Defined$ TargetedController | SubAbility$ ManaLose
    ///   SVar:ManaLose:DB$ DrainMana | Defined$ TargetedController
    ///
    /// Before mtg-532 the final `DB$ DrainMana` step parsed to
    /// `ApiType::Unknown("DrainMana")` and resolved as a logged no-op, so the
    /// "lose all unspent mana" rider never fired. This asserts the converter now
    /// produces a concrete `Effect::DrainMana` in the spell's effect chain with
    /// the `target_controller` sentinel (Defined$ TargetedController), so it
    /// resolves against the countered spell's controller. Runtime behavior
    /// (no "Unimplemented effect 'DrainMana'" warning, "loses all unspent mana"
    /// log line) is verified by tests/power_sink_drain_mana_e2e.sh.
    #[test]
    fn test_card_compat_power_sink() {
        use crate::loader::ability_parser::ApiType;
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/p/power_sink.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Power Sink should load");
        assert_eq!(def.name.as_str(), "Power Sink");
        assert_eq!(def.mana_cost.blue, 1);
        assert_eq!(def.mana_cost.x_count, 1, "Power Sink cost is {{X}}{{U}}");
        assert!(def.types.contains(&CardType::Instant));

        // The DrainMana ApiType must now be recognized (not Unknown).
        assert_eq!(
            ApiType::parse("DrainMana"),
            ApiType::DrainMana,
            "DrainMana must parse to its own ApiType, not Unknown"
        );

        // The flattened spell effect chain must contain a concrete DrainMana
        // carrying the target_controller sentinel (Defined$ TargetedController).
        let card = def.instantiate(CardId::new(1), PlayerId::new(0));
        let has_drain = card
            .effects
            .iter()
            .any(|e| matches!(e, Effect::DrainMana { player } if player.is_target_controller()));
        assert!(
            has_drain,
            "Power Sink must produce Effect::DrainMana with the target_controller \
             sentinel so the countered spell's controller loses unspent mana \
             (mtg-532). Got effects: {:?}",
            card.effects
        );
        // And it must NOT degrade to an Unimplemented placeholder.
        let has_unimpl = card
            .effects
            .iter()
            .any(|e| matches!(e, Effect::Unimplemented { api_type } if api_type == "DrainMana"));
        assert!(!has_unimpl, "DrainMana must not parse as Unimplemented");
    }

    /// Mechanical check that `Effect::DrainMana` empties a non-empty mana pool
    /// (Power Sink "lose all unspent mana"; mtg-532). Floats {U}{U}{C} into a
    /// player's pool, drains it, and asserts the pool is empty afterward while
    /// the OTHER player's floating mana is untouched.
    #[test]
    fn test_drain_mana_empties_pool() {
        use crate::core::{Color, Effect};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1 floats {U}{U}{C}; P2 floats {R} (must be left untouched).
        {
            let p1 = game.get_player_mut(p1_id).unwrap();
            p1.mana_pool.add_color(Color::Blue);
            p1.mana_pool.add_color(Color::Blue);
            p1.mana_pool.add_color(Color::Colorless);
        }
        {
            let p2 = game.get_player_mut(p2_id).unwrap();
            p2.mana_pool.add_color(Color::Red);
        }
        assert_eq!(game.get_player(p1_id).unwrap().mana_pool.total(), 3);

        game.execute_effect(&Effect::DrainMana { player: p1_id })
            .expect("DrainMana should resolve");

        assert!(
            game.get_player(p1_id).unwrap().mana_pool.is_empty(),
            "P1's unspent mana must be fully drained"
        );
        assert_eq!(
            game.get_player(p2_id).unwrap().mana_pool.total(),
            1,
            "P2's floating mana must be untouched"
        );
    }

    /// Card compat: Spirit Link (cardsfolder/s/spirit_link.txt) — mtg-544.
    ///
    /// Script: ManaCost:W / Types:Enchantment Aura / K:Enchant:Creature
    ///   T:Mode$ DamageDealtOnce | ValidSource$ Card.AttachedBy | Execute$ TrigGain
    ///   SVar:TrigGain:DB$ GainLife | Defined$ You | LifeAmount$ X
    ///   SVar:X:TriggerCount$DamageAmount
    ///
    /// WORKING (mtg-r9po1): the aura side (cost, types, Enchant:Creature)
    /// parses, and the "gain that much life when enchanted creature deals
    /// damage" trigger now parses into a DealsCombatDamage trigger that is
    /// attached-source-filtered (ValidSource$ Card.AttachedBy ->
    /// requires_attached_source) and carries a GainLifeDynamic { DamageDealt }
    /// effect (LifeAmount$ X / SVar:X:TriggerCount$DamageAmount). A silent drop
    /// of the T: line would leave the Aura with no triggers. Runtime lifegain is
    /// verified by puzzle_e2e test_spirit_link_aura_targeting.
    #[test]
    fn test_card_compat_spirit_link() {
        use crate::core::{DynamicAmount, TriggerEvent};
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/s/spirit_link.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Spirit Link should load");
        assert_eq!(def.name.as_str(), "Spirit Link");
        assert_eq!(def.mana_cost.white, 1, "Spirit Link costs {{W}}");
        assert!(def.types.contains(&CardType::Enchantment));

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));
        // Aura side: carries the Enchant keyword (Enchant:Creature).
        assert!(
            card.keywords.contains(Keyword::Enchant),
            "Spirit Link must carry the Enchant keyword. Keywords: {:?}",
            card.keywords
        );
        assert!(card.is_aura(), "Spirit Link should be an Aura");

        // The DamageDealtOnce trigger must parse (not be silently dropped).
        let dmg_trigger = card
            .triggers
            .iter()
            .find(|t| t.event == TriggerEvent::DealsCombatDamage)
            .expect(
                "Spirit Link must have a DealsCombatDamage trigger from its \
                 T:Mode$ DamageDealtOnce line; a silent parser drop leaves it empty",
            );
        assert!(
            !dmg_trigger.trigger_self_only,
            "ValidSource$ Card.AttachedBy must NOT be self-only (fires for the host, not the Aura)"
        );
        assert!(
            dmg_trigger.requires_attached_source,
            "ValidSource$ Card.AttachedBy must set requires_attached_source so the trigger \
             only fires when the enchanted creature deals damage"
        );
        let has_dynamic_gain = dmg_trigger.effects.iter().any(|e| {
            matches!(
                e,
                Effect::GainLifeDynamic {
                    amount: DynamicAmount::DamageDealt,
                    ..
                }
            )
        });
        assert!(
            has_dynamic_gain,
            "Spirit Link's trigger must carry GainLifeDynamic {{ DamageDealt }} \
             (LifeAmount$ X / SVar:X:TriggerCount$DamageAmount). Got: {:?}",
            dmg_trigger.effects
        );
    }

    /// Card compat: Grafted Skullcap (cardsfolder/g/grafted_skullcap.txt)
    ///
    /// Script:
    ///   ManaCost:4
    ///   Types:Artifact
    ///   T:Mode$ Phase | Phase$ Draw | ValidPlayer$ You | TriggerZones$ Battlefield
    ///     | Execute$ TrigDraw | TriggerDescription$ At the beginning of your draw
    ///     step, draw an additional card.
    ///   T:Mode$ Phase | Phase$ End of Turn | ... discard your hand.
    ///   SVar:TrigDraw:DB$ Draw
    ///
    /// Verifies the general `Phase$ Draw` → TriggerEvent::BeginningOfDraw mapping
    /// and that the `DB$ Draw` execute body is converted into a DrawCards effect
    /// on the trigger (previously the whole trigger was silently dropped because
    /// `Phase$ Draw` fell into the `_ => None` arm of the phase parser).
    #[test]
    fn test_card_compat_grafted_skullcap() {
        use crate::core::TriggerEvent;
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/g/grafted_skullcap.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Grafted Skullcap should load");
        assert_eq!(def.name.as_str(), "Grafted Skullcap");
        assert_eq!(def.mana_cost.generic, 4, "Grafted Skullcap costs {{4}}");
        assert!(def.types.contains(&CardType::Artifact));

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));

        // The draw-step trigger must be present with the new BeginningOfDraw event.
        let draw_trigger = card
            .triggers
            .iter()
            .find(|t| t.event == TriggerEvent::BeginningOfDraw)
            .expect("Grafted Skullcap must have a BeginningOfDraw trigger (Phase$ Draw)");

        // It must be controller-only (ValidPlayer$ You).
        assert!(
            draw_trigger.controller_turn_only,
            "ValidPlayer$ You draw-step trigger must be controller_turn_only"
        );

        // And it must carry a DrawCards effect (DB$ Draw), count 1, placeholder
        // player (resolved to the active player at fire time).
        assert!(
            draw_trigger
                .effects
                .iter()
                .any(|e| matches!(e, Effect::DrawCards { count: 1, player } if player.is_placeholder())),
            "BeginningOfDraw trigger must execute DrawCards(1) for the active player. Got: {:?}",
            draw_trigger.effects
        );
    }

    /// Card compat: Sylvan Library (cardsfolder/s/sylvan_library.txt)
    ///
    /// Script (relevant line):
    ///   T:Mode$ Phase | Phase$ Draw | ValidPlayer$ You | TriggerZones$ Battlefield
    ///     | Execute$ TrigDraw | ...
    ///   SVar:TrigDraw:AB$ ChooseCard | ... | Cost$ Draw<2/You> | ...
    ///
    /// Verifies the draw-step trigger now PARSES (BeginningOfDraw) rather than
    /// being silently dropped. Sylvan Library's full effect (the optional
    /// draw-two + choose-two + pay-4-life-or-return chain via AB$ ChooseCard /
    /// DB$ RepeatEach / UnlessCost$ PayLife) is NOT yet implemented — tracked as
    /// a follow-up. So we assert the trigger shape only (the card is PARTIAL).
    #[test]
    fn test_card_compat_sylvan_library() {
        use crate::core::TriggerEvent;
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/s/sylvan_library.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Sylvan Library should load");
        assert_eq!(def.name.as_str(), "Sylvan Library");
        assert_eq!(def.mana_cost.generic, 1, "Sylvan Library costs {{1}}{{G}}");
        assert_eq!(def.mana_cost.green, 1, "Sylvan Library costs {{1}}{{G}}");
        assert!(def.types.contains(&CardType::Enchantment));

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));

        // The draw-step trigger must now be recognised (not silently dropped).
        assert!(
            card.triggers.iter().any(|t| t.event == TriggerEvent::BeginningOfDraw),
            "Sylvan Library's Phase$ Draw trigger must map to BeginningOfDraw. Got: {:?}",
            card.triggers
        );
        // TODO(mtg-548): the ChooseCard / RepeatEach / pay-4-life-or-return chain
        // is not yet implemented — see follow-up issue. Card is PARTIAL until then.
    }

    /// Card compat: Mishra's Workshop (cardsfolder/m/mishras_workshop.txt).
    /// Wave 16 robots deck (mtg-523, mtg-559).
    ///
    /// Script: `A:AB$ Mana | Cost$ T | Produced$ C | Amount$ 3 | RestrictValid$ Spell.Artifact`
    ///
    /// Parser-shape guard for the multi-mana land fix: the cached mana
    /// production MUST report Colorless with `amount == 3`, and the parsed
    /// `AddMana` effect must carry 3 colorless. Before the fix the land
    /// tap-for-mana path ignored `amount` and produced a single {C}.
    #[test]
    fn test_card_compat_mishras_workshop() {
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/m/mishras_workshop.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Mishra's Workshop should load");
        assert_eq!(def.name.as_str(), "Mishra's Workshop");
        assert!(def.types.contains(&CardType::Land));

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));

        // Cached mana production (recomputed at instantiation): colorless, amount 3.
        assert!(
            matches!(
                card.definition.cache.mana_production.kind,
                crate::core::ManaProductionKind::Colorless
            ),
            "Workshop must produce colorless mana. Got: {:?}",
            card.definition.cache.mana_production.kind
        );
        assert_eq!(
            card.definition.cache.mana_production.amount, 3,
            "Workshop's {{T}}: Add {{C}}{{C}}{{C}} must derive amount 3 (Amount$ 3). Got: {}",
            card.definition.cache.mana_production.amount
        );

        let mana_ability = card
            .activated_abilities
            .iter()
            .find(|a| a.is_mana_ability)
            .expect("Workshop must have a mana ability");
        let added = mana_ability.effects.iter().find_map(|e| {
            if let Effect::AddMana { mana, .. } = e {
                Some(*mana)
            } else {
                None
            }
        });
        assert_eq!(
            added.map(|m| m.colorless),
            Some(3),
            "Workshop's AddMana effect must carry 3 colorless. Got: {:?}",
            added
        );
    }

    /// Card compat: Hurkyl's Recall (cardsfolder/h/hurkyls_recall.txt).
    /// Wave 16 robots deck (mtg-509, mtg-559).
    ///
    /// Script: `A:SP$ ChangeZoneAll | ValidTgts$ Player | ChangeType$ Artifact... |
    ///          Origin$ Battlefield | Destination$ Hand`
    ///
    /// Parser-shape guard: the spell must parse to a `ChangeZoneAll` effect
    /// with a single Battlefield origin and Hand destination, restricted to
    /// Artifacts, and NOT request a shuffle.
    #[test]
    fn test_card_compat_hurkyls_recall() {
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/h/hurkyls_recall.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Hurkyl's Recall should load");
        assert_eq!(def.name.as_str(), "Hurkyl's Recall");
        assert!(def.types.contains(&CardType::Instant));

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));
        let changezone = card.effects.iter().find_map(|e| {
            if let Effect::ChangeZoneAll {
                origins,
                destination,
                shuffle,
                ..
            } = e
            {
                Some((origins.clone(), *destination, *shuffle))
            } else {
                None
            }
        });
        let (origins, destination, shuffle) = changezone.expect("Hurkyl's Recall must parse to a ChangeZoneAll effect");
        assert_eq!(
            origins.as_slice(),
            [crate::zones::Zone::Battlefield],
            "Hurkyl's Recall returns artifacts from the battlefield"
        );
        assert_eq!(destination, crate::zones::Zone::Hand, "Returns to hand");
        assert!(!shuffle, "Hurkyl's Recall does not shuffle");
    }

    /// Card compat: Timetwister (cardsfolder/t/timetwister.txt).
    /// Wave 16 robots deck (mtg-552, mtg-559) — Power 9.
    ///
    /// Script: `A:SP$ ChangeZoneAll | ChangeType$ Card | Origin$ Hand,Graveyard |
    ///          Destination$ Library | Shuffle$ True | SubAbility$ DBDraw`
    ///          + `SVar:DBDraw:DB$ Draw | NumCards$ 7 | Defined$ Player`
    ///
    /// Parser-shape guard for the multi-origin ChangeZoneAll fix: the spell
    /// must parse to a ChangeZoneAll with BOTH Hand and Graveyard origins, a
    /// Library destination, and `shuffle == true`. Before the fix the
    /// comma-separated `Origin$ Hand,Graveyard` failed to parse and fell back
    /// to Battlefield (shuffling each player's board into the library).
    #[test]
    fn test_card_compat_timetwister() {
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/t/timetwister.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Timetwister should load");
        assert_eq!(def.name.as_str(), "Timetwister");
        assert!(def.types.contains(&CardType::Sorcery));

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));
        let changezone = card.effects.iter().find_map(|e| {
            if let Effect::ChangeZoneAll {
                origins,
                destination,
                shuffle,
                ..
            } = e
            {
                Some((origins.clone(), *destination, *shuffle))
            } else {
                None
            }
        });
        let (origins, destination, shuffle) = changezone.expect("Timetwister must parse to a ChangeZoneAll effect");
        assert!(
            origins.contains(&crate::zones::Zone::Hand) && origins.contains(&crate::zones::Zone::Graveyard),
            "Timetwister's Origin$ Hand,Graveyard must parse to BOTH zones. Got: {:?}",
            origins
        );
        assert_eq!(
            destination,
            crate::zones::Zone::Library,
            "Timetwister shuffles into the library"
        );
        assert!(shuffle, "Timetwister has Shuffle$ True");
    }

    /// Card compat: Mana Drain (cardsfolder/m/mana_drain.txt) — mtg-519
    ///
    /// Script:
    ///   ManaCost:U U
    ///   Types:Instant
    ///   A:SP$ Counter | TargetType$ Spell | RememberCounteredCMC$ True
    ///     | ValidTgts$ Card | SubAbility$ DBDelTrig
    ///   SVar:DBDelTrig:DB$ DelayedTrigger | Mode$ Phase | Phase$ Main1,Main2
    ///     | ValidPlayer$ You | Execute$ AddMana | RememberNumber$ True | SubAbility$ DBCleanup
    ///   SVar:DBCleanup:DB$ Cleanup | ClearRemembered$ True
    ///   SVar:AddMana:DB$ Mana | Produced$ C | Amount$ X
    ///
    /// Verifies (parser): {U}{U} Instant whose effect chain parses to BOTH
    /// (a) a CounterSpell with remember_mana_value=true (RememberCounteredCMC$),
    /// and (b) a CreateDelayedTrigger with a Mode$ Phase condition firing on the
    /// controller's (ValidPlayer$ You) next Main1/Main2 carrying an AddMana
    /// effect — the rider that was previously a silent drop. Runtime mana-pool
    /// behavior is verified by puzzle_e2e test_mana_drain_deferred_mana.
    #[test]
    fn test_card_compat_mana_drain() {
        use crate::core::{DelayedTriggerCondition, Effect, TriggerPhase, TurnOwner};
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/m/mana_drain.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Mana Drain should load");
        assert_eq!(def.name.as_str(), "Mana Drain");
        assert!(def.types.contains(&CardType::Instant));
        assert_eq!(def.mana_cost.blue, 2, "Mana Drain costs {{U}}{{U}}");
        assert_eq!(def.mana_cost.cmc(), 2);

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));

        // (a) CounterSpell with RememberCounteredCMC$ True.
        let counter = card.effects.iter().find_map(|e| {
            if let Effect::CounterSpell {
                remember_mana_value, ..
            } = e
            {
                Some(*remember_mana_value)
            } else {
                None
            }
        });
        assert_eq!(
            counter,
            Some(true),
            "Mana Drain must parse a CounterSpell with remember_mana_value=true. Effects: {:?}",
            card.effects
        );

        // (b) CreateDelayedTrigger with Mode$ Phase (Main1,Main2 / ValidPlayer$ You)
        // carrying an AddMana effect.
        let delayed = card.effects.iter().find_map(|e| {
            if let Effect::CreateDelayedTrigger { condition, effect, .. } = e {
                Some((condition.clone(), effect.clone()))
            } else {
                None
            }
        });
        let (condition, inner) = delayed
            .expect("Mana Drain must parse a CreateDelayedTrigger (the deferred-mana rider must not be dropped)");

        match condition {
            DelayedTriggerCondition::Phase { phases, whose_turn } => {
                assert!(
                    phases.contains(&TriggerPhase::Main1) && phases.contains(&TriggerPhase::Main2),
                    "Phase$ Main1,Main2 must parse to both main phases. Got: {:?}",
                    phases
                );
                assert_eq!(whose_turn, TurnOwner::You, "ValidPlayer$ You -> TurnOwner::You");
            }
            other @ (DelayedTriggerCondition::ZoneChange { .. }
            | DelayedTriggerCondition::LastCounterRemoved { .. }
            | DelayedTriggerCondition::SpellCast { .. }) => {
                panic!("Mana Drain delayed trigger must be Mode$ Phase. Got: {:?}", other)
            }
        }

        assert!(
            matches!(*inner, Effect::AddMana { .. }),
            "Mana Drain delayed trigger must execute an AddMana effect. Got: {:?}",
            inner
        );
    }

    /// mtg-3hwz3: City in a Bottle parses into the three general constructs:
    /// (1) a `Mode$ Always` set-origin sweep static (SacrificeMatchingPresent)
    /// whose filter requires the ARN set + non-token + Other; (2) a CantBeCast
    /// static; (3) a CantPlayLand static — both filtering on `Card.setARN`.
    /// Also verifies the set-origin filter parses `setARN` into a SetCode and
    /// that an ARN card loaded via CardDatabase is stamped with origin_set=ARN
    /// while a non-ARN card is not.
    #[test]
    fn test_card_compat_city_in_a_bottle() {
        use crate::core::{SetCode, StaticAbility};
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/c/city_in_a_bottle.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("City in a Bottle should load");
        assert_eq!(def.name.as_str(), "City in a Bottle");
        assert!(def.types.contains(&CardType::Artifact));
        assert_eq!(def.mana_cost.cmc(), 2, "City in a Bottle costs {{2}}");

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));
        let arn = SetCode::new("ARN");

        // (1) Mode$ Always state-trigger -> SacrificeMatchingPresent static,
        //     filter = Permanent.!token+setARN+Other.
        let sweep = card.static_abilities.iter().find_map(|sa| {
            if let StaticAbility::SacrificeMatchingPresent { restriction, .. } = sa {
                Some(restriction.clone())
            } else {
                None
            }
        });
        let sweep = sweep.expect("Mode$ Always must parse into a SacrificeMatchingPresent sweep static");
        assert_eq!(
            sweep.required_set.as_ref(),
            Some(&arn),
            "sweep filter must require setARN"
        );
        assert!(sweep.requires_nontoken, "sweep filter must require !token");
        assert!(sweep.requires_other, "sweep filter must require Other (self-exclusion)");

        // (2) S:Mode$ CantBeCast | ValidCard$ Card.setARN
        let cant_cast = card.static_abilities.iter().find_map(|sa| {
            if let StaticAbility::CantBeCast { valid_card, .. } = sa {
                Some(valid_card.clone())
            } else {
                None
            }
        });
        let cant_cast = cant_cast.expect("S:Mode$ CantBeCast must parse into a CantBeCast static");
        assert_eq!(
            cant_cast.required_set.as_ref(),
            Some(&arn),
            "CantBeCast must filter on setARN"
        );

        // (3) S:Mode$ CantPlayLand | ValidCard$ Card.setARN
        let cant_land = card.static_abilities.iter().find_map(|sa| {
            if let StaticAbility::CantPlayLand { valid_card, .. } = sa {
                Some(valid_card.clone())
            } else {
                None
            }
        });
        let cant_land = cant_land.expect("S:Mode$ CantPlayLand must parse into a CantPlayLand static");
        assert_eq!(
            cant_land.required_set.as_ref(),
            Some(&arn),
            "CantPlayLand must filter on setARN"
        );

        // Set-origin stamping: an ARN card carries origin_set=ARN; a non-ARN
        // card (Lightning Bolt, LEA) does not match setARN. Loaded via the
        // CardDatabase so the edition index runs.
        let cardsfolder = PathBuf::from("../cardsfolder");
        if std::fs::canonicalize(&cardsfolder).is_ok() {
            let db = CardDatabase::new(cardsfolder);
            let rt = tokio::runtime::Runtime::new().unwrap();
            let camel = rt
                .block_on(async { db.get_card("Camel").await })
                .expect("load Camel")
                .expect("Camel exists");
            assert_eq!(
                camel.origin_set.as_ref(),
                Some(&arn),
                "Camel was originally printed in ARN; origin_set must be stamped ARN"
            );
            let bolt = rt
                .block_on(async { db.get_card("Lightning Bolt").await })
                .expect("load Lightning Bolt")
                .expect("Lightning Bolt exists");
            assert_ne!(
                bolt.origin_set.as_ref(),
                Some(&arn),
                "Lightning Bolt is not an ARN card; must not be stamped ARN"
            );
        } else {
            eprintln!("Skipping origin_set stamping check: cardsfolder not canonicalizable");
        }
    }

    /// Parser-shape regression for restricted counterspells (mtg-856):
    /// Annul, Essence Scatter, Negate, and Disdainful Stroke each carry a
    /// `ValidTgts$` restriction that must limit which spells on the stack are
    /// legal targets.
    ///
    /// Before the fix, `TargetRestriction::parse` discarded nonCreature and
    /// cmcGE modifiers silently, so all four cards fell back to countering any
    /// spell (identical to plain Counterspell). After the fix each card's
    /// `CounterSpell::spell_restriction` must carry the appropriate flag.
    #[test]
    fn test_card_compat_restricted_counterspells_parser_shape() {
        use crate::core::{CardType, Effect, TargetType};
        use std::path::PathBuf;

        let cardsfolder = PathBuf::from("../cardsfolder");
        if !cardsfolder.exists() {
            eprintln!("Skipping: cardsfolder not present");
            return;
        }

        let rt = tokio::runtime::Runtime::new().unwrap();
        let db = crate::loader::CardDatabase::new(cardsfolder);

        // --- Annul: counter target artifact OR enchantment spell ---
        let annul_def = rt
            .block_on(async { db.get_card("Annul").await })
            .expect("load Annul")
            .expect("Annul exists");
        let annul = annul_def.instantiate(crate::core::CardId::new(1), PlayerId::new(0));
        let annul_effect = annul
            .effects
            .iter()
            .find(|e| matches!(e, Effect::CounterSpell { .. }))
            .expect("Annul must have a CounterSpell effect");
        if let Effect::CounterSpell { spell_restriction, .. } = annul_effect {
            assert!(
                spell_restriction.types.contains(&TargetType::Artifact),
                "Annul must restrict to Artifact targets"
            );
            assert!(
                spell_restriction.types.contains(&TargetType::Enchantment),
                "Annul must restrict to Enchantment targets"
            );
            assert!(
                !spell_restriction.types.contains(&TargetType::Creature),
                "Annul must NOT allow Creature targets"
            );
            assert!(
                !spell_restriction.requires_noncreature,
                "Annul uses type list, not requires_noncreature"
            );
            assert_eq!(spell_restriction.min_cmc, None, "Annul has no CMC restriction");
        } else {
            panic!("Expected CounterSpell effect");
        }

        // --- Essence Scatter: counter target creature spell ---
        let es_def = rt
            .block_on(async { db.get_card("Essence Scatter").await })
            .expect("load Essence Scatter")
            .expect("Essence Scatter exists");
        let es = es_def.instantiate(crate::core::CardId::new(2), PlayerId::new(0));
        let es_effect = es
            .effects
            .iter()
            .find(|e| matches!(e, Effect::CounterSpell { .. }))
            .expect("Essence Scatter must have a CounterSpell effect");
        if let Effect::CounterSpell { spell_restriction, .. } = es_effect {
            assert!(
                spell_restriction.types.contains(&TargetType::Creature),
                "Essence Scatter must restrict to Creature targets"
            );
            assert!(
                !spell_restriction.types.contains(&TargetType::Artifact),
                "Essence Scatter must NOT allow Artifact targets"
            );
            assert!(
                !spell_restriction.requires_noncreature,
                "Essence Scatter uses type list"
            );
            assert_eq!(
                spell_restriction.min_cmc, None,
                "Essence Scatter has no CMC restriction"
            );
        } else {
            panic!("Expected CounterSpell effect");
        }

        // --- Negate: counter target noncreature spell ---
        let negate_def = rt
            .block_on(async { db.get_card("Negate").await })
            .expect("load Negate")
            .expect("Negate exists");
        let negate = negate_def.instantiate(crate::core::CardId::new(3), PlayerId::new(0));
        let negate_effect = negate
            .effects
            .iter()
            .find(|e| matches!(e, Effect::CounterSpell { .. }))
            .expect("Negate must have a CounterSpell effect");
        if let Effect::CounterSpell { spell_restriction, .. } = negate_effect {
            assert!(
                spell_restriction.requires_noncreature,
                "Negate must have requires_noncreature=true"
            );
            assert!(
                spell_restriction.types.is_empty(),
                "Negate types list should be empty (uses requires_noncreature)"
            );
            assert_eq!(spell_restriction.min_cmc, None, "Negate has no CMC restriction");
        } else {
            panic!("Expected CounterSpell effect");
        }

        // --- Disdainful Stroke: counter target spell with mana value 4 or greater ---
        let ds_def = rt
            .block_on(async { db.get_card("Disdainful Stroke").await })
            .expect("load Disdainful Stroke")
            .expect("Disdainful Stroke exists");
        let ds = ds_def.instantiate(crate::core::CardId::new(4), PlayerId::new(0));
        let ds_effect = ds
            .effects
            .iter()
            .find(|e| matches!(e, Effect::CounterSpell { .. }))
            .expect("Disdainful Stroke must have a CounterSpell effect");
        if let Effect::CounterSpell { spell_restriction, .. } = ds_effect {
            assert_eq!(
                spell_restriction.min_cmc,
                Some(4),
                "Disdainful Stroke must restrict to spells with CMC >= 4"
            );
            assert!(
                !spell_restriction.requires_noncreature,
                "Disdainful Stroke has no noncreature restriction"
            );
            assert!(
                spell_restriction.types.is_empty(),
                "Disdainful Stroke has no type restriction"
            );
        } else {
            panic!("Expected CounterSpell effect");
        }

        // --- Targeting filter: verify valid_spell_targets enforces restrictions ---
        // We construct a minimal GameState, put spells on the stack, and check
        // that get_valid_targets filters correctly.

        let p1 = PlayerId::new(0);
        let p2 = PlayerId::new(1);

        // Helper: make a fake spell card of a given type and CMC on the stack
        let make_spell = |id: u32, types: &[CardType], mana_cmc: u8, game: &mut GameState| {
            let cid = crate::core::CardId::new(id);
            let mut card = crate::core::Card::new(cid, format!("TestSpell{}", id), p2);
            for t in types {
                card.add_type(*t);
            }
            // Set a simple mana cost matching the given CMC using the string parser
            // e.g. CMC 5 → "5", CMC 1 → "1", CMC 0 → ""
            let cost_str = if mana_cmc > 0 {
                mana_cmc.to_string()
            } else {
                String::new()
            };
            card.mana_cost = ManaCost::from_string(&cost_str);
            game.cards.insert(cid, card);
            game.stack.add(cid);
            cid
        };

        // Build a game with multiple spells on the stack
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);

        let creature_spell = make_spell(10, &[CardType::Creature], 1, &mut game);
        let artifact_spell = make_spell(11, &[CardType::Artifact], 2, &mut game);
        let enchantment_spell = make_spell(12, &[CardType::Enchantment], 2, &mut game);
        let sorcery_spell = make_spell(13, &[CardType::Sorcery], 2, &mut game);
        let big_spell = make_spell(14, &[CardType::Sorcery], 5, &mut game); // CMC 5

        // Add Annul on stack (targeting placeholder)
        let annul_id = crate::core::CardId::new(20);
        let annul_card = annul_def.instantiate(annul_id, p1);
        game.cards.insert(annul_id, annul_card);
        game.stack.add(annul_id);

        // Add Negate on stack (targeting placeholder)
        let negate_id = crate::core::CardId::new(21);
        let negate_card = negate_def.instantiate(negate_id, p1);
        game.cards.insert(negate_id, negate_card);
        game.stack.add(negate_id);

        // Add Disdainful Stroke on stack (targeting placeholder)
        let ds_id = crate::core::CardId::new(22);
        let ds_card = ds_def.instantiate(ds_id, p1);
        game.cards.insert(ds_id, ds_card);
        game.stack.add(ds_id);

        // Add Essence Scatter on stack (targeting placeholder)
        let es_id = crate::core::CardId::new(23);
        let es_card = es_def.instantiate(es_id, p1);
        game.cards.insert(es_id, es_card);
        game.stack.add(es_id);

        // Annul: should only see artifact + enchantment spells
        let annul_targets = game
            .get_valid_targets_for_spell(annul_id)
            .expect("get_valid_targets_for_spell(Annul)");
        assert!(
            !annul_targets.contains(&creature_spell),
            "Annul must NOT target creature spells"
        );
        assert!(
            annul_targets.contains(&artifact_spell),
            "Annul must target artifact spells"
        );
        assert!(
            annul_targets.contains(&enchantment_spell),
            "Annul must target enchantment spells"
        );
        assert!(
            !annul_targets.contains(&sorcery_spell),
            "Annul must NOT target sorcery spells"
        );

        // Negate: should see artifact + sorcery + enchantment but NOT creature
        let negate_targets = game
            .get_valid_targets_for_spell(negate_id)
            .expect("get_valid_targets_for_spell(Negate)");
        assert!(
            !negate_targets.contains(&creature_spell),
            "Negate must NOT target creature spells"
        );
        assert!(
            negate_targets.contains(&sorcery_spell),
            "Negate must target sorcery spells"
        );
        assert!(
            negate_targets.contains(&artifact_spell),
            "Negate must target artifact spells (non-creature)"
        );

        // Disdainful Stroke: only big spells (CMC >= 4)
        let ds_targets = game
            .get_valid_targets_for_spell(ds_id)
            .expect("get_valid_targets_for_spell(Disdainful Stroke)");
        assert!(
            !ds_targets.contains(&creature_spell),
            "Disdainful Stroke must NOT target CMC-1 spell"
        );
        assert!(
            !ds_targets.contains(&sorcery_spell),
            "Disdainful Stroke must NOT target CMC-2 sorcery"
        );
        assert!(
            ds_targets.contains(&big_spell),
            "Disdainful Stroke must target CMC-5 spell"
        );

        // Essence Scatter: only creature spells
        let es_targets = game
            .get_valid_targets_for_spell(es_id)
            .expect("get_valid_targets_for_spell(Essence Scatter)");
        assert!(
            es_targets.contains(&creature_spell),
            "Essence Scatter must target creature spells"
        );
        assert!(
            !es_targets.contains(&sorcery_spell),
            "Essence Scatter must NOT target sorcery spells"
        );
        assert!(
            !es_targets.contains(&artifact_spell),
            "Essence Scatter must NOT target artifact spells (non-creature)"
        );
    }

    /// Regression test for B3 (mtg-914): Wurmcoil Engine comma-separated TokenScript$.
    ///
    /// Before the fix, `execute_create_token` treated "a,b" as a single key and
    /// found nothing in `token_definitions`, silently creating zero tokens.
    /// After the fix, the comma list is split and each name is looked up
    /// individually, so both the deathtouch and lifelink Wurm tokens are minted.
    ///
    /// This test inserts both token definitions directly (no filesystem I/O),
    /// calls `execute_create_token` with the comma-joined script string, and
    /// asserts exactly two tokens land on the battlefield.
    #[test]
    fn test_execute_create_token_comma_separated_wurmcoil() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        // Minimal token definition stubs (name is sufficient for the test).
        let deathtouch_script = "c_3_3_a_phyrexian_wurm_deathtouch";
        let lifelink_script = "c_3_3_a_phyrexian_wurm_lifelink";

        // Load the real token definitions from the forge-java tokenscripts directory.
        let db = CardDatabase::new(PathBuf::from("../cardsfolder"));
        let mut dt_def = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { db.get_token(deathtouch_script).await })
            .expect("Deathtouch Wurm token should parse")
            .expect("Deathtouch Wurm token file should exist");
        dt_def.script_name = Some(deathtouch_script.to_string());

        let mut ll_def = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { db.get_token(lifelink_script).await })
            .expect("Lifelink Wurm token should parse")
            .expect("Lifelink Wurm token file should exist");
        ll_def.script_name = Some(lifelink_script.to_string());

        game.token_definitions
            .insert(deathtouch_script.to_string(), std::sync::Arc::new(dt_def));
        game.token_definitions
            .insert(lifelink_script.to_string(), std::sync::Arc::new(ll_def));

        let before = game.battlefield.cards.len();

        // Call with the comma-separated composite string exactly as stored in the
        // card script (SVar:TrigToken:DB$ Token | TokenScript$ <composite>).
        game.execute_create_token(p1_id, &format!("{},{}", deathtouch_script, lifelink_script), 1, false)
            .expect("execute_create_token should not fail");

        let after = game.battlefield.cards.len();
        assert_eq!(
            after - before,
            2,
            "Wurmcoil Engine should create exactly 2 tokens (deathtouch + lifelink). \
             before={before}, after={after}"
        );
    }

    /// Card compat: Pattern of Rebirth — Card.AttachedBy dies trigger (mtg-913 B12 follow-up).
    ///
    /// When the enchanted creature dies, Pattern of Rebirth's trigger must fire
    /// and search the controller's library for a creature to put onto the
    /// battlefield (the library search behavior is exercised here).
    ///
    /// MTG Rules: CR 603.6a/b/c — triggered abilities on Auras watch for the
    /// enchanted permanent dying. The Aura remains on the battlefield long enough
    /// for its trigger to fire; state-based actions then move the Aura to the
    /// graveyard. The search goes to the Aura controller's library (same player
    /// who owned the enchanted creature in all typical Pattern-of-Rebirth cases).
    #[test]
    fn test_card_compat_pattern_of_rebirth_attached_by_trigger() {
        use crate::core::{Card, CardType, TriggerEvent};
        use std::path::PathBuf;

        if !PathBuf::from("../cardsfolder/p/pattern_of_rebirth.txt").exists() {
            eprintln!("Skipping: cardsfolder not present");
            return;
        }

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Load Pattern of Rebirth from cardsfolder to get its real trigger.
        let por_id = load_test_card(&mut game, "Pattern of Rebirth", p1_id).expect("Pattern of Rebirth should load");

        // Verify the parser produces exactly one EquippedCreatureDies trigger.
        let por_card = game.cards.get(por_id).unwrap();
        assert_eq!(
            por_card.triggers.len(),
            1,
            "Pattern of Rebirth should have exactly one trigger"
        );
        assert_eq!(
            por_card.triggers[0].event,
            TriggerEvent::EquippedCreatureDies,
            "Pattern of Rebirth trigger should be EquippedCreatureDies"
        );

        // Place Pattern of Rebirth on the battlefield (as Aura).
        game.cards.get_mut(por_id).unwrap().definition.cache.is_aura = true;
        game.battlefield.add(por_id);

        // Create a creature for p1 that Pattern of Rebirth is attached to.
        let creature_id = game.next_card_id();
        let mut creature = Card::new(creature_id, "Llanowar Elves".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(1));
        creature.set_base_toughness(Some(1));
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Attach Pattern of Rebirth to the creature.
        game.cards.get_mut(por_id).unwrap().attached_to = Some(creature_id);

        // Add a creature to p1's library so the search can find it.
        let target_id = game.next_card_id();
        let mut target = Card::new(target_id, "Grizzly Bears".to_string(), p1_id);
        target.add_type(CardType::Creature);
        target.set_base_power(Some(2));
        target.set_base_toughness(Some(2));
        game.cards.insert(target_id, target);
        if let Some(zones) = game.player_zones.iter_mut().find(|(id, _)| *id == p1_id) {
            zones.1.library.add(target_id);
        }

        // Simulate the creature dying: move it off the battlefield and fire death triggers.
        game.battlefield.remove(creature_id);
        game.check_death_triggers(creature_id)
            .expect("death triggers should not error");

        // After the trigger fires, Pattern of Rebirth should have put Grizzly Bears
        // onto the battlefield (SearchLibrary puts the first matching creature there).
        assert!(
            game.battlefield.contains(target_id),
            "Pattern of Rebirth trigger must put a creature from library onto the battlefield"
        );
    }

    /// Card compat: Honden of Cleansing Fire — B1 fix.
    ///
    /// Script:
    ///   SVar:X:Count$Valid Shrine.YouCtrl/Times.2
    ///
    /// The `/Times.2` suffix is Forge's compact encoding of "for each Shrine you
    /// control, gain 2 life" (CR 700.4). Before the fix the filter was stored
    /// verbatim (`"Shrine.YouCtrl/Times.2"`) in `ValidPermanents { filter }`:
    ///
    ///   1. `count_permanents_matching` hit the "Unknown filter type" fallback
    ///      (because `"Shrine"` was not a recognised card type) and returned
    ///      `true` for ALL permanents — so the life gain counted the entire
    ///      battlefield, not just Shrines.
    ///   2. The `/Times.2` multiplier was never applied, so the count was
    ///      already wrong and un-multiplied.
    ///
    /// Fix (B1): split on `/Times.N` in `CountExpression::parse` to separate
    /// the filter from the multiplier, add `modifier: CountModifier::Times(2)`
    /// to `ValidPermanents`, and fall back to subtype-string matching in
    /// `count_permanents_matching` for unknown card-type tokens (CR 205.3c
    /// — enchantment subtypes like "Shrine", "Aura", etc. are valid).
    #[test]
    fn test_card_compat_honden_of_cleansing_fire_shrine_times_modifier() {
        use crate::core::{CountExpression, CountModifier, TriggerEvent};
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/h/honden_of_cleansing_fire.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Honden of Cleansing Fire should load");

        assert_eq!(def.name.as_str(), "Honden of Cleansing Fire");

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));

        // Must have a BeginningOfUpkeep trigger.
        let upkeep_trigger = card
            .triggers
            .iter()
            .find(|t| t.event == TriggerEvent::BeginningOfUpkeep)
            .expect("Honden of Cleansing Fire must have a BeginningOfUpkeep trigger");

        // The trigger's GainLifeDynamic effect must carry the correct count expression.
        // Honden: `LifeAmount$ X` with `SVar:X:Count$Valid Shrine.YouCtrl/Times.2`
        // → GainLifeDynamic { amount: DynamicAmount::Count(ValidPermanents{…}) }
        let gain_life = upkeep_trigger
            .effects
            .iter()
            .find(|e| matches!(e, Effect::GainLifeDynamic { .. }))
            .expect(
                "Honden upkeep trigger must have a GainLifeDynamic effect. \
                 If only GainLife (fixed) was found the SVar resolved to 0 (B1 bug).",
            );

        if let Effect::GainLifeDynamic { amount, .. } = gain_life {
            use crate::core::effects::DynamicAmount;
            let DynamicAmount::Count(count) = amount else {
                panic!("Honden GainLifeDynamic must use DynamicAmount::Count, got {:?}", amount);
            };
            // B1 fix: filter must be "Shrine.YouCtrl" (split from "/Times.2"),
            // and modifier must be CountModifier::Times(2).
            assert!(
                matches!(
                    count,
                    CountExpression::ValidPermanents {
                        filter,
                        modifier: CountModifier::Times(2)
                    } if filter == "Shrine.YouCtrl"
                ),
                "Honden GainLifeDynamic count must be ValidPermanents {{ filter: \"Shrine.YouCtrl\", \
                 modifier: Times(2) }}. Got: {:?}",
                count
            );
        } else {
            panic!("Expected GainLifeDynamic effect");
        }
    }

    /// Card compat: Reclaim — B6 fix (graveyard-targeting ChangeZone).
    ///
    /// Script:
    ///   A:SP$ ChangeZone | Origin$ Graveyard | Destination$ Library |
    ///   LibraryPosition$ 0 | ValidTgts$ Card.YouCtrl
    ///
    /// Before the B6 fix the converter emitted `None` for this pattern (no arm
    /// matched `Origin$ Graveyard | Destination$ Library | ValidTgts$`), so
    /// Reclaim silently resolved with no effect. The fix adds a
    /// `ReturnGraveyardCardToZone` arm that handles any
    /// `Origin$ Graveyard | ValidTgts$ … | Destination$ <non-Hand>` combination.
    ///
    /// MTG CR 701.25a: "Return [a card] from your graveyard to [zone]" is a
    /// zone-change from Graveyard to the named zone; the card is chosen on
    /// resolution (a targeted effect). CR 401.4: putting a card on top of your
    /// library (LibraryPosition 0) places it there face down.
    #[test]
    fn test_card_compat_reclaim_graveyard_to_library() {
        use crate::core::Effect;
        use crate::zones::Zone;
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/r/reclaim.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Reclaim should load");

        assert_eq!(def.name.as_str(), "Reclaim");

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));

        // Reclaim must have a ReturnGraveyardCardToZone effect.
        let gy_effect = card
            .effects
            .iter()
            .find(|e| matches!(e, Effect::ReturnGraveyardCardToZone { .. }))
            .expect(
                "Reclaim must have a ReturnGraveyardCardToZone effect (B6 fix). \
                 If this fails the converter still emits None for Graveyard→Library \
                 with ValidTgts$, so Reclaim is a silent no-op.",
            );

        if let Effect::ReturnGraveyardCardToZone {
            destination,
            library_position,
            gain_control,
            ..
        } = gy_effect
        {
            assert_eq!(
                *destination,
                Zone::Library,
                "Reclaim must return to Library, not {:?}",
                destination
            );
            assert_eq!(
                *library_position, 0,
                "Reclaim puts the card on TOP (position 0) of the library (CR 401.4)"
            );
            assert!(!gain_control, "Reclaim returns to your own library — no GainControl");
        } else {
            panic!("Expected ReturnGraveyardCardToZone");
        }
    }

    /// Card compat: Recollect — B6 fix (graveyard-targeting ChangeZone to Hand).
    ///
    /// Script:
    ///   A:SP$ ChangeZone | Origin$ Graveyard | Destination$ Hand |
    ///   ValidTgts$ Card.YouCtrl
    ///
    /// Recollect targets a card in YOUR graveyard and returns it to hand.
    /// Before B6 this correctly mapped to ReturnGraveyardCardToHand (Destination$
    /// Hand with ValidTgts$). This test pins that it still does after the refactor
    /// (the new ReturnGraveyardCardToZone arm should NOT swallow Destination$ Hand
    /// because that branch has a higher-priority check for Hand).
    #[test]
    fn test_card_compat_recollect_graveyard_to_hand() {
        use crate::core::Effect;
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/r/recollect.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Recollect should load");

        assert_eq!(def.name.as_str(), "Recollect");

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));

        // Recollect with Destination$ Hand + ValidTgts$ must use
        // ReturnGraveyardCardToHand (the existing hand-specific path),
        // NOT ReturnGraveyardCardToZone.
        assert!(
            card.effects
                .iter()
                .any(|e| matches!(e, Effect::ReturnGraveyardCardToHand { .. })),
            "Recollect must have ReturnGraveyardCardToHand (Destination$ Hand path). \
             If this fails the new ReturnGraveyardCardToZone arm incorrectly swallowed \
             the Hand destination."
        );
    }

    /// Card compat: Umezawa's Jitte — B3 fix (ModalChoice activated ability).
    ///
    /// Script (activated ability):
    ///   A:AB$ Charm | Cost$ SubCounter<1/CHARGE> | Choices$ JittePump,JitteCurse,JitteLife | Defined$ You
    ///   SVar:JittePump:DB$ Pump | Defined$ Equipped | NumAtt$ +2 | NumDef$ +2 | SpellDescription$ Equipped creature gets +2/+2 until end of turn.
    ///   SVar:JitteCurse:DB$ Pump | ValidTgts$ Creature | NumAtt$ -1 | NumDef$ -1 | IsCurse$ True | SpellDescription$ Target creature gets -1/-1 until end of turn.
    ///   SVar:JitteLife:DB$ GainLife | LifeAmount$ 2 | SpellDescription$ You gain 2 life.
    ///
    /// Before the B3 fix the Charm AB$ produced a ModalChoice effect that fell
    /// through to `execute_effect`, which logged "ModalChoice reached
    /// execute_effect" and no-oped — so the chosen mode was never applied.  This
    /// test pins that the activated ability parses into a ModalChoice with all
    /// three modes present and with real (non-placeholder DrawCards) sub-effects.
    ///
    /// MTG CR 701.4a: modal spells and abilities let you choose a mode as part of
    /// casting or activating.  CR 601.2b: for a modal spell cast from hand the
    /// mode choice is made before targeting.  For an activated ability the
    /// analogous choice happens on activation (before the ability resolves).
    #[test]
    fn test_card_compat_umezawas_jitte_modal_choice_parse() {
        use crate::core::Effect;
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/u/umezawas_jitte.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Umezawa's Jitte should load");

        assert_eq!(def.name.as_str(), "Umezawa's Jitte");

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));

        // Jitte must have exactly one non-Equip activated ability (the Charm).
        // The Equip ability is always the last one; the Charm comes before it.
        let charm_ability = card
            .activated_abilities
            .iter()
            .find(|ab| ab.effects.iter().any(|e| matches!(e, Effect::ModalChoice { .. })))
            .expect(
                "Umezawa's Jitte must have a ModalChoice activated ability (B3 fix). \
                 If this fails the AB$ Charm was not converted to ModalChoice.",
            );

        let modal_effect = charm_ability
            .effects
            .iter()
            .find(|e| matches!(e, Effect::ModalChoice { .. }))
            .expect("ModalChoice effect must be present in charm ability");

        if let Effect::ModalChoice {
            modes, num_to_choose, ..
        } = modal_effect
        {
            assert_eq!(
                modes.len(),
                3,
                "Jitte Charm must have 3 modes (JittePump, JitteCurse, JitteLife). \
                 Got {}",
                modes.len()
            );
            assert_eq!(
                *num_to_choose, 1,
                "Jitte Charm chooses exactly 1 mode. Got {}",
                num_to_choose
            );
            // Verify mode descriptions are present and non-empty.
            for (i, mode) in modes.iter().enumerate() {
                assert!(!mode.description.is_empty(), "Mode {} description must not be empty", i);
                // Sub-effects must be real effects (not placeholder DrawCards{count:0}).
                // Placeholder DrawCards{count:0} is what params_to_charm_effect emits
                // when it can't resolve SVars — a signal that SVar resolution failed.
                assert!(
                    !matches!(mode.effect.as_ref(), Effect::DrawCards { count, .. } if *count == 0),
                    "Mode {} ('{}') has a placeholder sub-effect; SVar resolution must \
                     have failed during load. Got: {:?}",
                    i,
                    mode.description,
                    mode.effect
                );
            }
        } else {
            panic!("Expected ModalChoice effect");
        }
    }

    /// Regression test for B-token (mtg-914): Avenger of Zendikar `TokenAmount$ X`
    /// where SVar:X:Count$Valid Land.YouCtrl must create one Plant token per land,
    /// not a hard-coded 1 token.
    ///
    /// Before the fix, `create_token_dynamic_from_params` returned `None` for
    /// `DynamicAmount::Count`, falling through to `params_to_effect` which called
    /// `get_u8("TokenAmount")` on "X" — failing silently and defaulting to 1.
    /// After the fix, `CreateTokenDynamic { amount: Count(ValidPermanents) }` is
    /// emitted and resolved at execution time against the live battlefield.
    ///
    /// This test verifies that the card loader parses the SVar-token chain into a
    /// `CreateTokenDynamic { amount: Count(_) }` effect (loader-shape check).
    ///
    /// See: mtg-914 (2010 WC tracker), mtg-915 (broken-card backlog).
    #[test]
    fn test_token_amount_count_expression_avenger_of_zendikar() {
        use crate::core::CardId;
        use crate::core::PlayerId;
        use crate::loader::CardDatabase;
        use std::path::PathBuf;

        let cardsfolder = PathBuf::from("../cardsfolder");
        if !cardsfolder.join("a/avenger_of_zendikar.txt").exists() {
            eprintln!("Skipping: cardsfolder not present");
            return;
        }

        let db = CardDatabase::new(cardsfolder);
        let rt = tokio::runtime::Runtime::new().unwrap();

        let card_def = rt
            .block_on(async { db.get_card("Avenger of Zendikar").await })
            .expect("DB lookup should not fail")
            .expect("Avenger of Zendikar should exist in cardsfolder");

        // Instantiate the card to get its parsed triggers.
        let card_id = CardId::new(1);
        let owner = PlayerId::new(0);
        let card = card_def.instantiate(card_id, owner);

        // The ETB trigger's Execute$ SVar (TrigToken) must parse to CreateTokenDynamic
        // with a Count(...) DynamicAmount.  The loader calls
        // create_token_dynamic_from_params which, after the fix, routes
        // DynamicAmount::Count through instead of returning None.
        let has_dynamic_count_token = card.triggers.iter().flat_map(|t| t.effects.iter()).any(|eff| {
            matches!(
                eff,
                crate::core::Effect::CreateTokenDynamic {
                    amount: crate::core::DynamicAmount::Count(_),
                    ..
                }
            )
        });

        assert!(
            has_dynamic_count_token,
            "Avenger of Zendikar ETB trigger should produce CreateTokenDynamic {{ Count(...) }}. \
             Check create_token_dynamic_from_params — DynamicAmount::Count must not fall through \
             to the params_to_effect fixed-amount path. \
             Trigger effects: {:?}",
            card.triggers.iter().flat_map(|t| t.effects.iter()).collect::<Vec<_>>()
        );
    }

    /// Card compat: Light of Day (cardsfolder/l/light_of_day.txt) — mtg-912 B7.
    ///
    /// Script:
    ///   `S:Mode$ CantAttack,CantBlock | ValidCard$ Creature.Black`
    ///
    /// Verifies that with Light of Day on the battlefield, a black creature
    /// (Drudge Skeletons) does NOT appear in the legal-attackers list and is
    /// excluded from legal blockers (GameState::is_attack_prohibited /
    /// is_block_prohibited). Without this fix the creature would appear in both
    /// lists because the CantAttack,CantBlock combined-mode shape was previously
    /// silently dropped (no match arm in the loader's mode dispatch).
    #[test]
    fn test_card_compat_light_of_day_prohibits_black_creatures() {
        use std::path::PathBuf;

        if !PathBuf::from("../cardsfolder/l/light_of_day.txt").exists() {
            eprintln!("Skipping: cardsfolder not present");
            return;
        }

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P2 controls Light of Day.
        let lod_id = load_test_card(&mut game, "Light of Day", p2_id).expect("Light of Day should load");
        game.battlefield.add(lod_id);

        // P1 controls a black creature (Drudge Skeletons).
        let skeletons_id = load_test_card(&mut game, "Drudge Skeletons", p1_id).expect("Drudge Skeletons should load");
        game.battlefield.add(skeletons_id);
        // Mark it as entered previous turn (no summoning sickness).
        game.cards.get_mut(skeletons_id).unwrap().turn_entered_battlefield = Some(0);

        let skeletons = game.cards.get(skeletons_id).unwrap();
        assert!(
            game.is_attack_prohibited(skeletons),
            "is_attack_prohibited must return true for a black creature with Light of Day on the battlefield"
        );
        assert!(
            game.is_block_prohibited(skeletons),
            "is_block_prohibited must return true for a black creature with Light of Day on the battlefield"
        );

        // A non-black creature should NOT be prohibited.
        let green_id = game.next_card_id();
        let green_creature = {
            let mut c = Card::new(green_id, "Grizzly Bears".to_string(), p1_id);
            c.add_type(CardType::Creature);
            c.colors.push(crate::core::Color::Green);
            c
        };
        assert!(
            !game.is_attack_prohibited(&green_creature),
            "is_attack_prohibited must return false for a non-black creature"
        );
        assert!(
            !game.is_block_prohibited(&green_creature),
            "is_block_prohibited must return false for a non-black creature"
        );
    }

    /// Card compat: Cursed Totem (cardsfolder/c/cursed_totem.txt) — mtg-912 B6.
    ///
    /// Script:
    ///   `S:Mode$ CantBeActivated | ValidCard$ Creature | ValidSA$ Activated`
    ///
    /// Verifies that with Cursed Totem on the battlefield,
    /// GameState::is_activated_ability_prohibited returns true for a creature
    /// and false for a non-creature.
    #[test]
    fn test_card_compat_cursed_totem_suppresses_creature_abilities() {
        use std::path::PathBuf;

        if !PathBuf::from("../cardsfolder/c/cursed_totem.txt").exists() {
            eprintln!("Skipping: cardsfolder not present");
            return;
        }

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P2 controls Cursed Totem.
        let totem_id = load_test_card(&mut game, "Cursed Totem", p2_id).expect("Cursed Totem should load");
        game.battlefield.add(totem_id);

        // P1 has a creature (Llanowar Elves — non-black so no Light of Day confusion).
        let elves_id = load_test_card(&mut game, "Llanowar Elves", p1_id).expect("Llanowar Elves should load");
        game.battlefield.add(elves_id);

        let elves = game.cards.get(elves_id).unwrap();
        assert!(
            game.is_activated_ability_prohibited(elves),
            "is_activated_ability_prohibited must be true for a creature with Cursed Totem on battlefield"
        );

        // Totem itself (an artifact, not a creature) should NOT be affected.
        let totem = game.cards.get(totem_id).unwrap();
        assert!(
            !game.is_activated_ability_prohibited(totem),
            "is_activated_ability_prohibited must be false for a non-creature artifact"
        );
    }

    /// Card compat: Overgrown Battlement — `withDefender` filter in
    /// `count_permanents_matching` (mtg-914/mtg-915, wave3).
    ///
    /// Overgrown Battlement: `{T}: Add {G} for each creature with defender
    /// you control.` — `SVar:X:Count$Valid Creature.withDefender+YouCtrl`.
    ///
    /// Before this fix `count_permanents_matching` silently ignored the
    /// `withDefender` qualifier and counted ALL your creatures. The fix adds a
    /// `filter.contains("withDefender")` guard that checks
    /// `card.has_keyword(Keyword::Defender)`, so only creatures that actually
    /// have Defender are counted (CR 702.6).
    ///
    /// Also verifies that `TargetRestriction::parse("Creature.withDefender")`
    /// sets `requires_defender = true` and that `TargetRestriction::matches`
    /// rejects creatures without Defender.
    #[test]
    fn test_withdefender_filter_count_and_target_restriction() {
        use crate::core::effects::TargetRestriction;
        use crate::core::{CardType, EntityId, Keyword, PlayerId};
        use crate::game::GameState;

        // --- Part 1: TargetRestriction::parse handles "withDefender" ---
        let tr = TargetRestriction::parse("Creature.withDefender");
        assert!(
            tr.requires_defender,
            "TargetRestriction::parse('Creature.withDefender') must set requires_defender=true"
        );
        let tr2 = TargetRestriction::parse("Creature.withDefender+YouCtrl");
        assert!(
            tr2.requires_defender,
            "TargetRestriction::parse('Creature.withDefender+YouCtrl') must set requires_defender=true"
        );

        // --- Part 2: TargetRestriction::matches enforces requires_defender ---
        let p0 = PlayerId::new(0);
        let mut creature_with_def = crate::core::Card::new(EntityId::new(1), "Wall of Stone", p0);
        creature_with_def.add_type(CardType::Creature);
        creature_with_def.keywords.insert(Keyword::Defender);

        let mut creature_no_def = crate::core::Card::new(EntityId::new(2), "Grizzly Bears", p0);
        creature_no_def.add_type(CardType::Creature);

        let restriction = TargetRestriction::parse("Creature.withDefender");
        assert!(
            restriction.matches(&creature_with_def),
            "Creature with Defender must match 'Creature.withDefender' restriction"
        );
        assert!(
            !restriction.matches(&creature_no_def),
            "Creature without Defender must NOT match 'Creature.withDefender' restriction"
        );

        // --- Part 3: count_permanents_matching counts only defender creatures ---
        // Create a game with p0 controlling two defenders and one non-defender.
        let mut game = GameState::new_two_player("P0".to_string(), "P1".to_string(), 20);
        game.turn.active_player = p0;

        let cid1 = EntityId::new(10);
        let mut c1 = crate::core::Card::new(cid1, "Wall A", p0);
        c1.add_type(CardType::Creature);
        c1.keywords.insert(Keyword::Defender);
        c1.controller = p0;
        game.cards.insert(cid1, c1);
        game.battlefield.cards.push(cid1);

        let cid2 = EntityId::new(11);
        let mut c2 = crate::core::Card::new(cid2, "Wall B", p0);
        c2.add_type(CardType::Creature);
        c2.keywords.insert(Keyword::Defender);
        c2.controller = p0;
        game.cards.insert(cid2, c2);
        game.battlefield.cards.push(cid2);

        let cid3 = EntityId::new(12);
        let mut c3 = crate::core::Card::new(cid3, "Bear", p0);
        c3.add_type(CardType::Creature);
        // No Defender keyword
        c3.controller = p0;
        game.cards.insert(cid3, c3);
        game.battlefield.cards.push(cid3);

        // Overgrown Battlement's SVar: Count$Valid Creature.withDefender+YouCtrl
        let filter = "Creature.withDefender+YouCtrl";
        let count = game
            .evaluate_count_expression(
                &crate::core::CountExpression::ValidPermanents {
                    filter: filter.to_string(),
                    modifier: crate::core::effects::CountModifier::None,
                },
                p0,
            )
            .expect("evaluate_count_expression must succeed");

        assert_eq!(
            count, 2,
            "count_permanents_matching('Creature.withDefender+YouCtrl') must count \
             only the 2 Defender creatures, not the Bear without Defender. \
             Got {} (withDefender fix not applied?)",
            count
        );
    }

    /// Card compat: Worship (cardsfolder/w/worship.txt) — mtg-912 B10.
    ///
    /// Script:
    ///   `R:Event$ LifeReduced | ValidPlayer$ You.lifeGE1 | Result$ LT1
    ///    | IsDamage$ True | IsPresent$ Creature.YouCtrl | ReplaceWith$ ReduceLoss`
    ///
    /// Verifies: with Worship + a creature on the battlefield, damage that
    /// would reduce the controller's life below 1 is capped at life - 1.
    /// Without a creature, the floor does not apply.
    #[test]
    fn test_card_compat_worship_life_floor() {
        use std::path::PathBuf;

        if !PathBuf::from("../cardsfolder/w/worship.txt").exists() {
            eprintln!("Skipping: cardsfolder not present");
            return;
        }

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Put P1 at life = 3 to make the floor clearly visible.
        game.players[0].life = 3;

        // P1 controls Worship.
        let worship_id = load_test_card(&mut game, "Worship", p1_id).expect("Worship should load");
        game.battlefield.add(worship_id);

        // P1 controls a creature (Llanowar Elves).
        let elves_id = load_test_card(&mut game, "Llanowar Elves", p1_id).expect("Llanowar Elves should load");
        game.battlefield.add(elves_id);

        // Deal 5 damage to P1 — would take life from 3 to -2 without Worship.
        // With Worship the floor kicks in: life stays at 1.
        game.deal_damage(p1_id, 5).expect("deal_damage should succeed");
        assert_eq!(
            game.players[0].life, 1,
            "Worship: life should be floored at 1 (dealt 5 from 3, would be -2)"
        );

        // Now deal 0 damage — life stays at 1 (no-op).
        game.deal_damage(p1_id, 0).expect("deal_damage with 0 should succeed");
        assert_eq!(game.players[0].life, 1, "Life must not change on 0 damage");

        // Remove the creature from the battlefield — Worship's floor no longer applies.
        game.battlefield.remove(elves_id);
        game.deal_damage(p1_id, 2).expect("deal_damage should succeed");
        assert_eq!(
            game.players[0].life, -1,
            "Without a creature, Worship's floor should NOT apply (life goes below 1)"
        );
    }

    /// Card compat: Serra Avatar (cardsfolder/s/serra_avatar.txt) — mtg-912 B4.
    ///
    /// Script:
    ///   `S:Mode$ Continuous | CharacteristicDefining$ True | SetPower$ X
    ///    | SetToughness$ X`  with `SVar:X:Count$YourLifeTotal`
    ///
    /// Verifies: Serra Avatar's power/toughness equals the controller's life total,
    /// and updates dynamically when the life total changes.
    #[test]
    fn test_card_compat_serra_avatar_cda_pt() {
        use std::path::PathBuf;

        if !PathBuf::from("../cardsfolder/s/serra_avatar.txt").exists() {
            eprintln!("Skipping: cardsfolder not present");
            return;
        }

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // P1 starts at 20 life.
        assert_eq!(game.players[0].life, 20);

        // P1 controls Serra Avatar.
        let avatar_id = load_test_card(&mut game, "Serra Avatar", p1_id).expect("Serra Avatar should load");
        game.battlefield.add(avatar_id);

        // At 20 life: P/T should be 20/20.
        let power = game.get_effective_power(avatar_id).expect("get_effective_power");
        let toughness = game
            .get_effective_toughness(avatar_id)
            .expect("get_effective_toughness");
        assert_eq!(power, 20, "Serra Avatar P should be 20 (life total) at 20 life");
        assert_eq!(toughness, 20, "Serra Avatar T should be 20 (life total) at 20 life");

        // Change life to 13 — P/T must track it immediately.
        game.players[0].life = 13;
        let power2 = game.get_effective_power(avatar_id).expect("get_effective_power");
        let toughness2 = game
            .get_effective_toughness(avatar_id)
            .expect("get_effective_toughness");
        assert_eq!(power2, 13, "Serra Avatar P should track life total: 13");
        assert_eq!(toughness2, 13, "Serra Avatar T should track life total: 13");

        // Life at 1 — P/T must be 1/1.
        game.players[0].life = 1;
        let power3 = game.get_effective_power(avatar_id).expect("get_effective_power");
        assert_eq!(power3, 1, "Serra Avatar P must be 1 at life=1");

        // Life at 0 — P/T must be 0/0.
        game.players[0].life = 0;
        let power4 = game.get_effective_power(avatar_id).expect("get_effective_power");
        assert_eq!(power4, 0, "Serra Avatar P must be 0 at life=0");
    }

    /// Card compat: Crumbling Sanctuary (cardsfolder/c/crumbling_sanctuary.txt) — mtg-912 B9.
    ///
    /// Script:
    ///   `R:Event$ DamageDone | ValidTarget$ Player | ReplaceWith$ ExileTop`
    ///   `SVar:ExileTop:DB$ Dig | Defined$ ReplacedTarget | DigNum$ X
    ///          | ChangeNum$ All | DestinationZone$ Exile`
    ///
    /// Verifies: with Crumbling Sanctuary on the battlefield, damage to a player
    /// exiles cards from their library instead of reducing their life total.
    #[test]
    fn test_card_compat_crumbling_sanctuary_damage_to_exile() {
        use std::path::PathBuf;

        if !PathBuf::from("../cardsfolder/c/crumbling_sanctuary.txt").exists() {
            eprintln!("Skipping: cardsfolder not present");
            return;
        }

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P2 controls Crumbling Sanctuary.
        let sanctuary_id =
            load_test_card(&mut game, "Crumbling Sanctuary", p2_id).expect("Crumbling Sanctuary should load");
        game.battlefield.add(sanctuary_id);

        // Give P1 a known library (5 Plains).
        for _ in 0..5 {
            let land_id = load_test_card(&mut game, "Plains", p1_id).expect("Plains should load");
            game.get_player_zones_mut(p1_id)
                .expect("P1 zones")
                .library
                .cards
                .push(land_id);
        }
        let initial_life = game.players[0].life;
        let initial_library_size = game.get_player_zones(p1_id).map(|z| z.library.cards.len()).unwrap_or(0);
        assert_eq!(initial_library_size, 5, "Setup: P1 library should have 5 cards");

        // Deal 3 damage to P1 — with Crumbling Sanctuary on the battlefield,
        // P1 exiles 3 cards from their library instead of losing 3 life.
        game.deal_damage(p1_id, 3).expect("deal_damage should succeed");

        let life_after = game.players[0].life;
        assert_eq!(
            life_after, initial_life,
            "P1 life should be unchanged — damage was redirected to exile"
        );

        let library_size_after = game.get_player_zones(p1_id).map(|z| z.library.cards.len()).unwrap_or(0);
        assert_eq!(
            library_size_after,
            initial_library_size - 3,
            "P1 library should have shrunk by 3 (cards exiled instead of life loss)"
        );

        let exile_size = game
            .player_zones
            .iter()
            .find(|(id, _)| *id == p1_id)
            .map(|(_, z)| z.exile.cards.len())
            .unwrap_or(0);
        assert_eq!(
            exile_size, 3,
            "P1's exile zone should contain 3 cards (the redirected damage)"
        );
    }

    /// Parser regression: Teferi, Time Raveler's static
    /// `S:Mode$ CantBeCast | ValidCard$ Card | Caster$ Opponent | OnlySorcerySpeed$ True`
    /// must load with `only_sorcery_speed = true` and `caster_restriction = Opponent`.
    #[test]
    fn test_card_compat_teferi_time_raveler_static_parses_only_sorcery_speed() {
        use crate::core::{CasterRestriction, StaticAbility};
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/t/teferi_time_raveler.txt");
        if !path.exists() {
            eprintln!("Skipping: cardsfolder not present at {:?}", path);
            return;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Teferi, Time Raveler should load");
        assert_eq!(def.name.as_str(), "Teferi, Time Raveler");

        let card = def.instantiate(CardId::new(1), PlayerId::new(0));

        // Find the CantBeCast static that encodes "opponents cast only at sorcery speed"
        let teferi_static = card.static_abilities.iter().find_map(|sa| {
            if let StaticAbility::CantBeCast {
                caster_restriction,
                only_sorcery_speed,
                ..
            } = sa
            {
                Some((*caster_restriction, *only_sorcery_speed))
            } else {
                None
            }
        });

        let (caster_restriction, only_sorcery_speed) =
            teferi_static.expect("Teferi should have a CantBeCast static ability");

        assert_eq!(
            caster_restriction,
            CasterRestriction::Opponent,
            "Teferi's static must restrict opponents (Caster$ Opponent)"
        );
        assert!(
            only_sorcery_speed,
            "Teferi's static must have only_sorcery_speed=true (OnlySorcerySpeed$ True)"
        );
    }

    /// Runtime enforcement: Teferi, Time Raveler's static prevents opponents from
    /// casting instants (or any spell) outside their own sorcery window.
    ///
    /// Scenario A: P2's instant in hand is NOT offered as a cast action during
    ///   P1's main phase (P2 is opponent; not sorcery window for P2).
    /// Scenario B: P2's instant IS offered during P2's own main phase with empty
    ///   stack (sorcery window for P2, so Teferi's prohibition is lifted).
    #[test]
    fn test_teferi_time_raveler_opponents_cannot_cast_outside_sorcery_window() {
        use crate::core::SpellAbility;
        use crate::game::game_loop::GameLoop;
        use crate::game::{Step, VerbosityLevel};
        use std::path::PathBuf;

        // Need both Teferi and an instant (Lightning Bolt) from the cardsfolder.
        let teferi_path = PathBuf::from("../cardsfolder/t/teferi_time_raveler.txt");
        let bolt_path = PathBuf::from("../cardsfolder/l/lightning_bolt.txt");
        if !teferi_path.exists() || !bolt_path.exists() {
            eprintln!("Skipping: cardsfolder not present");
            return;
        }

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1 controls Teferi on the battlefield.
        let teferi_id =
            load_test_card(&mut game, "Teferi, Time Raveler", p1_id).expect("Teferi, Time Raveler should load");
        game.battlefield.add(teferi_id);

        // P2 has Lightning Bolt in hand and a Mountain to pay {R}.
        let bolt_id = load_test_card(&mut game, "Lightning Bolt", p2_id).expect("Lightning Bolt should load");
        if let Some(zones) = game.get_player_zones_mut(p2_id) {
            zones.hand.add(bolt_id);
        }
        let mountain_id = load_test_card(&mut game, "Mountain", p2_id).expect("Mountain should load");
        game.battlefield.add(mountain_id);
        if let Ok(c) = game.cards.get_mut(mountain_id) {
            c.controller = p2_id;
            c.tapped = false;
        }

        // --- Scenario A: P1's main phase, stack empty ---
        // P2 is the opponent; they are NOT in a sorcery window (not their turn).
        // Teferi's prohibition SHOULD block P2 from casting Lightning Bolt.
        game.turn.active_player = p1_id;
        game.turn.current_step = Step::Main1;
        // stack is already empty

        let buffer_a = {
            let mut gl = GameLoop::new(&mut game).with_verbosity(VerbosityLevel::Silent);
            gl.push_castable_spells(p2_id);
            gl.get_abilities_buffer().to_vec()
        };

        let bolt_offered_a = buffer_a
            .iter()
            .any(|sa| matches!(sa, SpellAbility::CastSpell { card_id, .. } if *card_id == bolt_id));
        assert!(
            !bolt_offered_a,
            "Teferi, Time Raveler: opponent P2 must NOT be offered Lightning Bolt \
             during P1's main phase (not P2's sorcery window). Buffer: {:?}",
            buffer_a
        );

        // --- Scenario B: P2's own main phase, stack empty ---
        // P2 IS in a sorcery window, so Teferi's prohibition is lifted.
        game.turn.active_player = p2_id;
        game.turn.current_step = Step::Main1;

        let buffer_b = {
            let mut gl2 = GameLoop::new(&mut game).with_verbosity(VerbosityLevel::Silent);
            gl2.push_castable_spells(p2_id);
            gl2.get_abilities_buffer().to_vec()
        };

        let bolt_offered_b = buffer_b
            .iter()
            .any(|sa| matches!(sa, SpellAbility::CastSpell { card_id, .. } if *card_id == bolt_id));
        assert!(
            bolt_offered_b,
            "Teferi, Time Raveler: P2 MUST be offered Lightning Bolt during their \
             own main phase with empty stack (sorcery window lifted). Buffer: {:?}",
            buffer_b
        );
    }

    /// Card compat: Opalescence (cardsfolder/o/opalescence.txt) — mtg-912 B5.
    ///
    /// Script:
    ///   `S:Mode$ Continuous | Affected$ Enchantment.nonAura+Other
    ///    | SetPower$ AffectedX | SetToughness$ AffectedX | AddType$ Creature`
    ///   `SVar:AffectedX:Count$CardManaCost`
    ///
    /// Verifies that:
    /// 1. Opalescence parses to `StaticAbility::OpalescenceStyle`.
    /// 2. `GameState::is_opalescence_creature()` returns `true` for a non-Aura
    ///    enchantment and `false` for a non-enchantment, an Aura, and Opalescence
    ///    itself.
    /// 3. `GameState::opalescence_pt()` returns the correct mana value for a
    ///    matching enchantment.
    /// 4. Matching enchantments appear in the available-attacker list.
    #[test]
    fn test_card_compat_opalescence_enchantments_become_creatures() {
        use crate::core::StaticAbility;
        use std::path::PathBuf;

        let opalescence_path = PathBuf::from("../cardsfolder/o/opalescence.txt");
        if !opalescence_path.exists() {
            eprintln!("Skipping: cardsfolder not present");
            return;
        }

        // --- 1. Parser check: Opalescence emits OpalescenceStyle static ---
        let opalescence_def =
            crate::loader::CardLoader::load_from_file(&opalescence_path).expect("opalescence.txt should load");
        let statics = opalescence_def.parse_static_abilities();
        let has_opalescence_style = statics
            .iter()
            .any(|s| matches!(s, StaticAbility::OpalescenceStyle { .. }));
        assert!(
            has_opalescence_style,
            "Opalescence must parse to StaticAbility::OpalescenceStyle; got {:?}",
            statics
        );

        // --- 2. Runtime: is_opalescence_creature() ---
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1 controls Opalescence.
        let opal_id = load_test_card(&mut game, "Opalescence", p1_id).expect("Opalescence should load");
        game.battlefield.add(opal_id);

        // P2 controls a non-Aura enchantment: Worship (3W enchantment, CMC=4).
        // (We use Worship because it's already tested elsewhere and loads cleanly.)
        let worship_path = PathBuf::from("../cardsfolder/w/worship.txt");
        if !worship_path.exists() {
            eprintln!("Skipping Worship sub-test: cardsfolder not present");
            return;
        }
        let worship_id = load_test_card(&mut game, "Worship", p2_id).expect("Worship should load");
        game.battlefield.add(worship_id);

        let worship_card = game.cards.get(worship_id).expect("Worship in card store");
        assert!(
            game.is_opalescence_creature(worship_card),
            "is_opalescence_creature must be true for a non-Aura enchantment (Worship) with Opalescence on the battlefield"
        );

        // 2b. Opalescence itself is excluded ("other non-Aura enchantments").
        let opal_card = game.cards.get(opal_id).expect("Opalescence in card store");
        assert!(
            !game.is_opalescence_creature(opal_card),
            "is_opalescence_creature must be false for Opalescence itself (it is the source)"
        );

        // 2c. A non-enchantment permanent is not affected.
        let plains_id = load_test_card(&mut game, "Plains", p1_id).expect("Plains should load");
        game.battlefield.add(plains_id);
        let plains_card = game.cards.get(plains_id).expect("Plains in card store");
        assert!(
            !game.is_opalescence_creature(plains_card),
            "is_opalescence_creature must be false for a non-enchantment (Plains)"
        );

        // --- 3. P/T check: Worship has CMC 4 (3W), expect 4/4 ---
        let worship_card = game.cards.get(worship_id).expect("Worship");
        let pt = game.opalescence_pt(worship_card);
        assert_eq!(
            pt,
            Some(4),
            "opalescence_pt must return Some(4) for Worship (CMC=4); got {:?}",
            pt
        );

        // --- 4. Attacker list: Worship should appear as available attacker for P2 ---
        // Mark Worship as entering a previous turn (no summoning sickness).
        game.cards.get_mut(worship_id).unwrap().turn_entered_battlefield = Some(0);
        game.turn.turn_number = 1;

        // Verify via get_available_attacker_creatures_for_test (GameLoop helper).
        // We construct a GameLoop to access the helper.
        {
            let game_loop = crate::game::game_loop::GameLoop::new(&mut game);
            let available_attackers = game_loop.get_available_attacker_creatures_for_test(p2_id);
            assert!(
                available_attackers.contains(&worship_id),
                "Worship (as an Opalescence-animated enchantment-creature) must appear in \
                 the available-attacker list for P2; got {:?}",
                available_attackers
            );
        }
    }
}
