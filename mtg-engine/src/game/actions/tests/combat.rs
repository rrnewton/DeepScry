use crate::core::{Card, CardType, Keyword};
use crate::game::state::GameState;
use crate::game::ZeroController;

#[cfg(test)]
mod tests {
    use super::super::effects::load_test_card;
    use super::*;

    #[test]
    fn test_declare_attacker() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];

        // Create a creature
        let creature_id = game.next_card_id();
        let mut creature = Card::new(creature_id, "Grizzly Bears".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(2));
        creature.set_base_toughness(Some(2));
        creature.controller = p1_id;
        game.cards.insert(creature_id, creature);

        // Put creature on battlefield
        game.battlefield.add(creature_id);

        // Declare attacker
        let result = game.declare_attacker(p1_id, creature_id);
        assert!(result.is_ok(), "Failed to declare attacker: {result:?}");

        // Check creature is attacking
        assert!(game.combat.is_attacking(creature_id));

        // Check creature is tapped
        let creature = game.cards.get(creature_id).unwrap();
        assert!(creature.tapped);
    }

    #[test]
    fn test_declare_blocker() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // Create an attacker
        let attacker_id = game.next_card_id();
        let mut attacker = Card::new(attacker_id, "Goblin".to_string(), p1_id);
        attacker.add_type(CardType::Creature);
        attacker.set_base_power(Some(2));
        attacker.set_base_toughness(Some(1));
        attacker.controller = p1_id;
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // Declare as attacker
        game.combat.declare_attacker(attacker_id, p2_id);

        // Create a blocker
        let blocker_id = game.next_card_id();
        let mut blocker = Card::new(blocker_id, "Wall".to_string(), p2_id);
        blocker.add_type(CardType::Creature);
        blocker.set_base_power(Some(0));
        blocker.set_base_toughness(Some(3));
        blocker.controller = p2_id;
        game.cards.insert(blocker_id, blocker);
        game.battlefield.add(blocker_id);

        // Declare blocker
        let result = game.declare_blocker(p2_id, blocker_id, vec![attacker_id]);
        assert!(result.is_ok(), "Failed to declare blocker: {result:?}");

        // Check blocker is blocking
        assert!(game.combat.is_blocking(blocker_id));
        assert!(game.combat.is_blocked(attacker_id));

        let blockers = game.combat.get_blockers(attacker_id);
        assert_eq!(blockers.len(), 1);
        assert!(blockers.contains(&blocker_id));
    }

    #[test]
    fn test_combat_damage_unblocked() {
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // Create an attacker
        let attacker_id = game.next_card_id();
        let mut attacker = Card::new(attacker_id, "Dragon".to_string(), p1_id);
        attacker.add_type(CardType::Creature);
        attacker.set_base_power(Some(5));
        attacker.set_base_toughness(Some(5));
        attacker.controller = p1_id;
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // Declare as attacker (unblocked)
        game.combat.declare_attacker(attacker_id, p2_id);

        // Create controllers
        let mut controller1 = ZeroController::new(p1_id);
        let mut controller2 = ZeroController::new(p2_id);

        // Assign combat damage
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // Check defending player took damage
        let p2 = game.get_player(p2_id).unwrap();
        assert_eq!(p2.life, 15); // 20 - 5 = 15
    }

    #[test]
    fn test_combat_damage_sets_dealt_damage_to_opponent_flag() {
        // Whirling Dervish (mtg-713 B9): a creature that deals combat damage to
        // an opponent must have its `dealt_damage_to_opponent_this_turn` flag set
        // (drives the end-step intervening-if counter, CR 603.4). Verify it is
        // unset before combat and set after dealing unblocked damage to a player.
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        let attacker_id = game.next_card_id();
        let mut attacker = Card::new(attacker_id, "Whirling Dervish".to_string(), p1_id);
        attacker.add_type(CardType::Creature);
        attacker.set_base_power(Some(1));
        attacker.set_base_toughness(Some(1));
        attacker.controller = p1_id;
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        assert!(
            !game.cards.get(attacker_id).unwrap().dealt_damage_to_opponent_this_turn,
            "flag must start false"
        );

        game.combat.declare_attacker(attacker_id, p2_id);
        let mut controller1 = ZeroController::new(p1_id);
        let mut controller2 = ZeroController::new(p2_id);
        game.assign_combat_damage(&mut controller1, &mut controller2, false)
            .unwrap();

        assert_eq!(game.get_player(p2_id).unwrap().life, 19, "P2 takes 1 combat damage");
        assert!(
            game.cards.get(attacker_id).unwrap().dealt_damage_to_opponent_this_turn,
            "dealing combat damage to an opponent must set dealt_damage_to_opponent_this_turn"
        );
    }

    #[test]
    fn test_combat_damage_blocked() {
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // Create an attacker (3/3)
        let attacker_id = game.next_card_id();
        let mut attacker = Card::new(attacker_id, "Bear".to_string(), p1_id);
        attacker.add_type(CardType::Creature);
        attacker.set_base_power(Some(3));
        attacker.set_base_toughness(Some(3));
        attacker.controller = p1_id;
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // Create a blocker (2/2)
        let blocker_id = game.next_card_id();
        let mut blocker = Card::new(blocker_id, "Wolf".to_string(), p2_id);
        blocker.add_type(CardType::Creature);
        blocker.set_base_power(Some(2));
        blocker.set_base_toughness(Some(2));
        blocker.controller = p2_id;
        game.cards.insert(blocker_id, blocker);
        game.battlefield.add(blocker_id);

        // Declare attacker and blocker
        game.combat.declare_attacker(attacker_id, p2_id);
        let blocker_vec = smallvec::smallvec![attacker_id];
        game.combat.declare_blocker(blocker_id, blocker_vec);

        // Create controllers
        let mut controller1 = ZeroController::new(p1_id);
        let mut controller2 = ZeroController::new(p2_id);

        // Assign combat damage
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // Check defending player took no damage (blocked)
        let p2 = game.get_player(p2_id).unwrap();
        assert_eq!(p2.life, 20);

        // Check blocker died (took 2 damage, toughness 2)
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(zones.graveyard.contains(blocker_id));
        }

        // Check attacker took 2 damage but has toughness 3, so it survives
        assert!(game.battlefield.contains(attacker_id));
    }

    /// mtg-m43mc / mtg-r9po1: a "deals combat damage" trigger (here Spirit
    /// Link's pseudo-lifelink) must fire when the source deals combat damage to
    /// a CREATURE, not only to a player. Combat damage is one simultaneous event
    /// (CR 510.2); the Aura's any-recipient trigger watches damage to all
    /// recipients (CR 119.3-style, matching Lifelink). Before the fix, the
    /// firing site only iterated creatures that dealt damage to a PLAYER, so a
    /// blocked enchanted creature never fired the trigger.
    ///
    /// Scenario: P0's 3/3 attacker is enchanted with Spirit Link and is BLOCKED
    /// by a P1 4/4. The attacker deals 3 combat damage to the blocker (none to a
    /// player); P0 must still gain 3 life from Spirit Link's trigger.
    #[test]
    fn test_spirit_link_fires_on_combat_damage_to_creature() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // P0's 3/3 attacker.
        let attacker_id = game.next_card_id();
        let mut attacker = Card::new(attacker_id, "Bear".to_string(), p1_id);
        attacker.add_type(CardType::Creature);
        attacker.set_base_power(Some(3));
        attacker.set_base_toughness(Some(3));
        attacker.controller = p1_id;
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P1's 4/4 blocker (survives, so the attacker only ever damages a
        // creature -- never a player -- isolating the bug).
        let blocker_id = game.next_card_id();
        let mut blocker = Card::new(blocker_id, "Wall".to_string(), p2_id);
        blocker.add_type(CardType::Creature);
        blocker.set_base_power(Some(4));
        blocker.set_base_toughness(Some(4));
        blocker.controller = p2_id;
        game.cards.insert(blocker_id, blocker);
        game.battlefield.add(blocker_id);

        // Spirit Link enchanting P0's attacker (real card from cardsfolder so we
        // exercise the parsed DamageDealtOnce -> any-recipient trigger).
        let aura_id = match load_test_card(&mut game, "Spirit Link", p1_id) {
            Ok(id) => id,
            Err(_) => {
                eprintln!("Skipping: Spirit Link not present in cardsfolder");
                return;
            }
        };
        {
            let aura = game.cards.get_mut(aura_id).unwrap();
            aura.attached_to = Some(attacker_id);
            aura.controller = p1_id;
        }
        game.battlefield.add(aura_id);

        let p0_life_before = game.get_player(p1_id).unwrap().life;
        let p1_life_before = game.get_player(p2_id).unwrap().life;

        // Declare attacker + block.
        game.combat.declare_attacker(attacker_id, p2_id);
        let blocker_vec = smallvec::smallvec![attacker_id];
        game.combat.declare_blocker(blocker_id, blocker_vec);

        let mut controller1 = ZeroController::new(p1_id);
        let mut controller2 = ZeroController::new(p2_id);
        game.assign_combat_damage(&mut controller1, &mut controller2, false)
            .expect("combat damage should resolve");

        // The defending player took NO damage (attacker was blocked), proving the
        // lifegain comes from combat damage dealt to the CREATURE, not a player.
        let p1_life_after = game.get_player(p2_id).unwrap().life;
        assert_eq!(
            p1_life_after, p1_life_before,
            "defending player should take no combat damage (attacker was blocked)"
        );

        // Spirit Link's trigger must have fired for the 3 damage dealt to the
        // blocker: P0 gains 3 life.
        let p0_life_after = game.get_player(p1_id).unwrap().life;
        assert_eq!(
            p0_life_after,
            p0_life_before + 3,
            "Spirit Link must gain 3 life when the enchanted creature deals 3 combat damage to a blocker \
             (before={p0_life_before}, after={p0_life_after}); the DealsCombatDamage trigger must fire on \
             damage to a creature, not only to a player (mtg-m43mc)"
        );
    }

    /// mtg-m43mc: a player-only "deals combat damage to a player" trigger must
    /// NOT fire when the creature only deals combat damage to a blocker. Uses
    /// Hypnotic Specter (random discard on combat damage to a player). The
    /// recipient-class gate (`CombatDamageTarget::Player`) must suppress it.
    #[test]
    fn test_player_only_trigger_does_not_fire_on_creature_damage() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // P0 attacker = Hypnotic Specter (2/2 flyer with the player-only trigger).
        let attacker_id = match load_test_card(&mut game, "Hypnotic Specter", p1_id) {
            Ok(id) => id,
            Err(_) => {
                eprintln!("Skipping: Hypnotic Specter not present in cardsfolder");
                return;
            }
        };
        {
            let a = game.cards.get_mut(attacker_id).unwrap();
            a.controller = p1_id;
        }
        game.battlefield.add(attacker_id);

        // P1 blocker (4/4) and a card in P1's hand that would be discarded if the
        // trigger erroneously fired.
        let blocker_id = game.next_card_id();
        let mut blocker = Card::new(blocker_id, "Wall".to_string(), p2_id);
        blocker.add_type(CardType::Creature);
        blocker.set_base_power(Some(4));
        blocker.set_base_toughness(Some(4));
        blocker.controller = p2_id;
        game.cards.insert(blocker_id, blocker);
        game.battlefield.add(blocker_id);

        let hand_card_id = game.next_card_id();
        let mut hand_card = Card::new(hand_card_id, "Forest".to_string(), p2_id);
        hand_card.add_type(CardType::Land);
        game.cards.insert(hand_card_id, hand_card);
        if let Some(zones) = game.get_player_zones_mut(p2_id) {
            zones.hand.add(hand_card_id);
        }
        let p1_hand_before = game.get_player_zones(p2_id).map(|z| z.hand.cards.len()).unwrap_or(0);

        game.combat.declare_attacker(attacker_id, p2_id);
        let blocker_vec = smallvec::smallvec![attacker_id];
        game.combat.declare_blocker(blocker_id, blocker_vec);

        let mut controller1 = ZeroController::new(p1_id);
        let mut controller2 = ZeroController::new(p2_id);
        game.assign_combat_damage(&mut controller1, &mut controller2, false)
            .expect("combat damage should resolve");

        // Player-only trigger must NOT have fired: P1's hand is unchanged.
        let p1_hand_after = game.get_player_zones(p2_id).map(|z| z.hand.cards.len()).unwrap_or(0);
        assert_eq!(
            p1_hand_after, p1_hand_before,
            "Hypnotic Specter's 'deals combat damage to a player' trigger must NOT fire when it only \
             damages a blocker (recipient-class gate); P1 should not have discarded (mtg-m43mc)"
        );
    }

    /// mtg-r9po1: Spirit Link must fire on NON-combat damage too (CR 119.3 —
    /// lifelink-style "gain that much life" triggers on ANY damage the source
    /// deals). Scenario: an enchanted pinger creature whose activated ability
    /// (`DealDamage` resolving from the stack) deals 1 non-combat damage to the
    /// opponent. P0 must gain 1 life from Spirit Link, via the SAME shared
    /// `DealsCombatDamage` trigger path the combat site uses (no Spirit-Link
    /// hack, no platform branch).
    #[test]
    fn test_spirit_link_fires_on_noncombat_damage() {
        use crate::core::{Effect, TargetRef};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // P0's "pinger": a creature whose ability deals 1 non-combat damage to
        // the opponent. Modelled by putting a DealDamage effect on the card and
        // resolving it from the stack (the exact shared resolution path a real
        // `{T}: deal 1` activated ability — Prodigal Sorcerer — flows through).
        let pinger_id = game.next_card_id();
        let mut pinger = Card::new(pinger_id, "Prodigal Sorcerer".to_string(), p1_id);
        pinger.add_type(CardType::Creature);
        pinger.set_base_power(Some(1));
        pinger.set_base_toughness(Some(1));
        pinger.controller = p1_id;
        pinger.effects = vec![Effect::DealDamage {
            target: TargetRef::Player(p2_id),
            amount: 1,
        }];
        game.cards.insert(pinger_id, pinger);
        game.battlefield.add(pinger_id);

        // Spirit Link enchanting the pinger (real card -> parsed DamageDealtOnce
        // trigger with ValidSource$ Card.AttachedBy + TriggerCount$DamageAmount).
        let aura_id = match load_test_card(&mut game, "Spirit Link", p1_id) {
            Ok(id) => id,
            Err(_) => {
                eprintln!("Skipping: Spirit Link not present in cardsfolder");
                return;
            }
        };
        {
            let aura = game.cards.get_mut(aura_id).unwrap();
            aura.attached_to = Some(pinger_id);
            aura.controller = p1_id;
        }
        game.battlefield.add(aura_id);

        let p0_life_before = game.get_player(p1_id).unwrap().life;
        let p1_life_before = game.get_player(p2_id).unwrap().life;

        // Resolve the pinger's damage ability from the stack (shared non-combat
        // damage path -> deal_damage -> accumulator -> deals-damage trigger).
        game.resolve_spell(pinger_id, &[])
            .expect("pinger ability should resolve");

        // Opponent took 1 non-combat damage.
        assert_eq!(
            game.get_player(p2_id).unwrap().life,
            p1_life_before - 1,
            "opponent should take 1 non-combat damage from the pinger"
        );

        // Spirit Link fired on the NON-combat damage: P0 gains exactly 1 life.
        assert_eq!(
            game.get_player(p1_id).unwrap().life,
            p0_life_before + 1,
            "Spirit Link must gain 1 life when the enchanted creature deals 1 NON-combat damage \
             (before={p0_life_before}); the deals-damage trigger must fire off the general \
             deal_damage path, not only combat (mtg-r9po1, CR 119.3)"
        );

        // The transient per-resolution accumulator is cleared back to None after
        // resolution (no leaked un-serialized state).
        assert_eq!(
            game.damage_dealt_by_source, None,
            "damage_dealt_by_source must be cleared after resolution"
        );
    }

    /// mtg-r9po1 de-dup guard: a single COMBAT-damage event must still fire
    /// Spirit Link exactly ONCE (not twice). The non-combat firing site keys off
    /// the per-resolution accumulator, which combat never sets, so combat damage
    /// fires only via its own per-creature path.
    #[test]
    fn test_spirit_link_combat_damage_not_double_counted() {
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // P0's 3/3 attacker, enchanted with Spirit Link, attacking an open P1.
        let attacker_id = game.next_card_id();
        let mut attacker = Card::new(attacker_id, "Bear".to_string(), p1_id);
        attacker.add_type(CardType::Creature);
        attacker.set_base_power(Some(3));
        attacker.set_base_toughness(Some(3));
        attacker.controller = p1_id;
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        let aura_id = match load_test_card(&mut game, "Spirit Link", p1_id) {
            Ok(id) => id,
            Err(_) => {
                eprintln!("Skipping: Spirit Link not present in cardsfolder");
                return;
            }
        };
        {
            let aura = game.cards.get_mut(aura_id).unwrap();
            aura.attached_to = Some(attacker_id);
            aura.controller = p1_id;
        }
        game.battlefield.add(aura_id);

        let p0_life_before = game.get_player(p1_id).unwrap().life;

        game.combat.declare_attacker(attacker_id, p2_id);
        let mut controller1 = ZeroController::new(p1_id);
        let mut controller2 = ZeroController::new(p2_id);
        game.assign_combat_damage(&mut controller1, &mut controller2, false)
            .expect("combat damage should resolve");

        // P0 gains EXACTLY 3 (the combat damage), not 6 (double-fire).
        assert_eq!(
            game.get_player(p1_id).unwrap().life,
            p0_life_before + 3,
            "Spirit Link must fire exactly ONCE for a single combat-damage event (gain 3, not 6); \
             the non-combat firing site must not double-count combat damage (mtg-r9po1)"
        );
    }

    #[test]
    fn test_combat_damage_multiple_blockers() {
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // Create a powerful attacker (5/5)
        let attacker_id = game.next_card_id();
        let mut attacker = Card::new(attacker_id, "Dragon".to_string(), p1_id);
        attacker.add_type(CardType::Creature);
        attacker.set_base_power(Some(5));
        attacker.set_base_toughness(Some(5));
        attacker.controller = p1_id;
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // Create first blocker (2/2)
        let blocker1_id = game.next_card_id();
        let mut blocker1 = Card::new(blocker1_id, "Bear".to_string(), p2_id);
        blocker1.add_type(CardType::Creature);
        blocker1.set_base_power(Some(2));
        blocker1.set_base_toughness(Some(2));
        blocker1.controller = p2_id;
        game.cards.insert(blocker1_id, blocker1);
        game.battlefield.add(blocker1_id);

        // Create second blocker (3/3)
        let blocker2_id = game.next_card_id();
        let mut blocker2 = Card::new(blocker2_id, "Wolf".to_string(), p2_id);
        blocker2.add_type(CardType::Creature);
        blocker2.set_base_power(Some(3));
        blocker2.set_base_toughness(Some(3));
        blocker2.controller = p2_id;
        game.cards.insert(blocker2_id, blocker2);
        game.battlefield.add(blocker2_id);

        // Declare attacker and both blockers
        game.combat.declare_attacker(attacker_id, p2_id);
        let blocker_vec = smallvec::smallvec![attacker_id];
        game.combat.declare_blocker(blocker1_id, blocker_vec.clone());
        game.combat.declare_blocker(blocker2_id, blocker_vec);

        // Create controllers
        let mut controller1 = ZeroController::new(p1_id);
        let mut controller2 = ZeroController::new(p2_id);

        // Assign combat damage
        // ZeroController will keep the order as-is
        // Dragon (5 power) assigns: 2 to first blocker (lethal), 3 to second blocker (lethal)
        // Both blockers (2+3=5 power) deal 5 damage back to Dragon (lethal)
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // Check defending player took no damage (blocked)
        let p2 = game.get_player(p2_id).unwrap();
        assert_eq!(p2.life, 20);

        // Check both blockers died
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(blocker1_id),
                "First blocker should be in graveyard"
            );
            assert!(
                zones.graveyard.contains(blocker2_id),
                "Second blocker should be in graveyard"
            );
        }

        // Check attacker died (took 5 damage, toughness 5)
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(zones.graveyard.contains(attacker_id), "Attacker should be in graveyard");
        }
    }

    #[test]
    fn test_summoning_sickness_blocks_attack() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create a creature and put it on battlefield
        let creature_id = game.next_entity_id();
        let mut creature = Card::new(creature_id, "Grizzly Bears".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(2));
        creature.set_base_toughness(Some(2));
        creature.controller = p1_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Mark it as entering this turn (summoning sickness)
        if let Ok(card) = game.cards.get_mut(creature_id) {
            card.turn_entered_battlefield = Some(game.turn.turn_number);
        }

        // Try to declare it as an attacker - should fail due to summoning sickness
        let result = game.declare_attacker(p1_id, creature_id);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("summoning sickness"));
    }

    #[test]
    fn test_summoning_sickness_allows_attack_next_turn() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create a creature and put it on battlefield
        let creature_id = game.next_entity_id();
        let mut creature = Card::new(creature_id, "Grizzly Bears".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(2));
        creature.set_base_toughness(Some(2));
        creature.controller = p1_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Mark it as entering on a previous turn
        if let Ok(card) = game.cards.get_mut(creature_id) {
            card.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        }

        // Declare it as an attacker - should succeed
        let result = game.declare_attacker(p1_id, creature_id);
        assert!(result.is_ok());
        assert!(game.combat.is_attacking(creature_id));
    }

    #[test]
    fn test_haste_bypasses_summoning_sickness() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create a creature with haste
        let creature_id = game.next_entity_id();
        let mut creature = Card::new(creature_id, "Lightning Elemental".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(4));
        creature.set_base_toughness(Some(1));
        creature.controller = p1_id;
        creature.keywords.insert(Keyword::Haste);
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Mark it as entering this turn
        if let Ok(card) = game.cards.get_mut(creature_id) {
            card.turn_entered_battlefield = Some(game.turn.turn_number);
        }

        // Declare it as an attacker - should succeed because of haste
        let result = game.declare_attacker(p1_id, creature_id);
        assert!(result.is_ok());
        assert!(game.combat.is_attacking(creature_id));
    }

    #[test]
    fn test_defender_blocks_attack() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create a creature with defender
        let creature_id = game.next_entity_id();
        let mut creature = Card::new(creature_id, "Wall of Stone".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(0));
        creature.set_base_toughness(Some(8));
        creature.controller = p1_id;
        creature.keywords.insert(Keyword::Defender);
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Mark it as entering on a previous turn (no summoning sickness)
        if let Ok(card) = game.cards.get_mut(creature_id) {
            card.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        }

        // Try to declare it as an attacker - should fail because of defender
        let result = game.declare_attacker(p1_id, creature_id);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("defender can't attack"));
        assert!(!game.combat.is_attacking(creature_id));
    }

    #[test]
    fn test_vigilance_creature_stays_untapped() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Load Serra Angel (4/4 with Flying and Vigilance)
        let creature_id = load_test_card(&mut game, "Serra Angel", p1_id).expect("Failed to load Serra Angel");

        if let Ok(creature) = game.cards.get_mut(creature_id) {
            creature.controller = p1_id;
        }
        game.battlefield.add(creature_id);

        // Mark it as entering on a previous turn (no summoning sickness)
        if let Ok(card) = game.cards.get_mut(creature_id) {
            card.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        }

        // Declare it as an attacker
        let result = game.declare_attacker(p1_id, creature_id);
        assert!(result.is_ok());
        assert!(game.combat.is_attacking(creature_id));

        // Check that creature is still untapped (vigilance effect)
        let card = game.cards.get(creature_id).unwrap();
        assert!(
            !card.tapped,
            "Creature with vigilance should not be tapped after attacking"
        );
    }

    #[test]
    fn test_non_vigilance_creature_gets_tapped() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create a creature WITHOUT vigilance
        let creature_id = game.next_entity_id();
        let mut creature = Card::new(creature_id, "Grizzly Bears".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(2));
        creature.set_base_toughness(Some(2));
        creature.controller = p1_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Mark it as entering on a previous turn (no summoning sickness)
        if let Ok(card) = game.cards.get_mut(creature_id) {
            card.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        }

        // Declare it as an attacker
        let result = game.declare_attacker(p1_id, creature_id);
        assert!(result.is_ok());
        assert!(game.combat.is_attacking(creature_id));

        // Check that creature is tapped (normal attack behavior)
        let card = game.cards.get(creature_id).unwrap();
        assert!(
            card.tapped,
            "Creature without vigilance should be tapped after attacking"
        );
    }

    #[test]
    fn test_flying_creature_blocked_by_flying() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Load Storm Crow (1/2 with Flying) as attacker
        let attacker_id = load_test_card(&mut game, "Storm Crow", p1_id).expect("Failed to load Storm Crow");

        if let Ok(attacker) = game.cards.get_mut(attacker_id) {
            attacker.controller = p1_id;
            attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        }
        game.battlefield.add(attacker_id);

        // P2: Load Segovian Angel (1/1 with Flying and Vigilance) as blocker
        let blocker_id = load_test_card(&mut game, "Segovian Angel", p2_id).expect("Failed to load Segovian Angel");

        if let Ok(blocker) = game.cards.get_mut(blocker_id) {
            blocker.controller = p2_id;
        }
        game.battlefield.add(blocker_id);

        // Declare attacker
        game.declare_attacker(p1_id, attacker_id).unwrap();

        // Blocker with flying should be able to block attacker with flying
        let result = game.declare_blocker(p2_id, blocker_id, vec![attacker_id]);
        assert!(
            result.is_ok(),
            "Creature with flying should be able to block creature with flying"
        );
    }

    #[test]
    fn test_flying_creature_blocked_by_reach() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Load Storm Crow (1/2 with Flying) as attacker
        let attacker_id = load_test_card(&mut game, "Storm Crow", p1_id).expect("Failed to load Storm Crow");

        if let Ok(attacker) = game.cards.get_mut(attacker_id) {
            attacker.controller = p1_id;
            attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        }
        game.battlefield.add(attacker_id);

        // P2: Load Giant Spider (2/4 with Reach) as blocker
        let blocker_id = load_test_card(&mut game, "Giant Spider", p2_id).expect("Failed to load Giant Spider");

        if let Ok(blocker) = game.cards.get_mut(blocker_id) {
            blocker.controller = p2_id;
        }
        game.battlefield.add(blocker_id);

        // Declare attacker
        game.declare_attacker(p1_id, attacker_id).unwrap();

        // Blocker with reach should be able to block attacker with flying
        let result = game.declare_blocker(p2_id, blocker_id, vec![attacker_id]);
        assert!(
            result.is_ok(),
            "Creature with reach should be able to block creature with flying"
        );
    }

    #[test]
    fn test_flying_creature_cannot_be_blocked_by_non_flying() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a creature with Flying (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Storm Crow".to_string(), p1_id);
        attacker.add_type(CardType::Creature);
        attacker.set_base_power(Some(1));
        attacker.set_base_toughness(Some(2));
        attacker.controller = p1_id;
        attacker.keywords.insert(Keyword::Flying);
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create a creature without Flying or Reach (blocker)
        let blocker_id = game.next_entity_id();
        let mut blocker = Card::new(blocker_id, "Grizzly Bears".to_string(), p2_id);
        blocker.add_type(CardType::Creature);
        blocker.set_base_power(Some(2));
        blocker.set_base_toughness(Some(2));
        blocker.controller = p2_id;
        game.cards.insert(blocker_id, blocker);
        game.battlefield.add(blocker_id);

        // Declare attacker
        game.declare_attacker(p1_id, attacker_id).unwrap();

        // Blocker without flying or reach should NOT be able to block attacker with flying
        let result = game.declare_blocker(p2_id, blocker_id, vec![attacker_id]);
        assert!(
            result.is_err(),
            "Creature without flying or reach should not be able to block creature with flying"
        );
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("cannot block attacker with flying"));
    }

    #[test]
    fn test_non_flying_creature_blocked_by_any() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a creature without Flying (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Grizzly Bears".to_string(), p1_id);
        attacker.add_type(CardType::Creature);
        attacker.set_base_power(Some(2));
        attacker.set_base_toughness(Some(2));
        attacker.controller = p1_id;
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create a creature without Flying or Reach (blocker)
        let blocker_id = game.next_entity_id();
        let mut blocker = Card::new(blocker_id, "Hill Giant".to_string(), p2_id);
        blocker.add_type(CardType::Creature);
        blocker.set_base_power(Some(3));
        blocker.set_base_toughness(Some(3));
        blocker.controller = p2_id;
        game.cards.insert(blocker_id, blocker);
        game.battlefield.add(blocker_id);

        // Declare attacker
        game.declare_attacker(p1_id, attacker_id).unwrap();

        // Any creature should be able to block a non-flying creature
        let result = game.declare_blocker(p2_id, blocker_id, vec![attacker_id]);
        assert!(
            result.is_ok(),
            "Any creature should be able to block a non-flying creature"
        );
    }

    #[test]
    fn test_flying_and_reach_blocker_can_block_flying() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a creature with Flying (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Storm Crow".to_string(), p1_id);
        attacker.add_type(CardType::Creature);
        attacker.set_base_power(Some(1));
        attacker.set_base_toughness(Some(2));
        attacker.controller = p1_id;
        attacker.keywords.insert(Keyword::Flying);
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create a creature with both Flying AND Reach (blocker)
        let blocker_id = game.next_entity_id();
        let mut blocker = Card::new(blocker_id, "Mystic Drake".to_string(), p2_id);
        blocker.add_type(CardType::Creature);
        blocker.set_base_power(Some(2));
        blocker.set_base_toughness(Some(3));
        blocker.controller = p2_id;
        blocker.keywords.insert(Keyword::Flying);
        blocker.keywords.insert(Keyword::Reach);
        game.cards.insert(blocker_id, blocker);
        game.battlefield.add(blocker_id);

        // Declare attacker
        game.declare_attacker(p1_id, attacker_id).unwrap();

        // Blocker with both flying and reach should be able to block attacker with flying
        let result = game.declare_blocker(p2_id, blocker_id, vec![attacker_id]);
        assert!(
            result.is_ok(),
            "Creature with flying and reach should be able to block creature with flying"
        );
    }

    #[test]
    fn test_first_strike_creature_kills_before_taking_damage() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Load Advance Scout (1/1 with First Strike) as attacker
        let attacker_id = load_test_card(&mut game, "Advance Scout", p1_id).expect("Failed to load Advance Scout");

        // Set attacker power/toughness to 2/2 so test works as before
        if let Ok(attacker) = game.cards.get_mut(attacker_id) {
            attacker.set_base_power(Some(2));
            attacker.set_base_toughness(Some(2));
            attacker.controller = p1_id;
            attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        }
        game.battlefield.add(attacker_id);

        // P2: Load Grizzly Bears (2/2 vanilla) as blocker
        let blocker_id = load_test_card(&mut game, "Grizzly Bears", p2_id).expect("Failed to load Grizzly Bears");

        if let Ok(blocker) = game.cards.get_mut(blocker_id) {
            blocker.controller = p2_id;
        }
        game.battlefield.add(blocker_id);

        // Declare combat
        game.combat.declare_attacker(attacker_id, p2_id);
        let attacker_vec = smallvec::smallvec![attacker_id];
        game.combat.declare_blocker(blocker_id, attacker_vec);

        // Create controllers
        let mut controller1 = ZeroController::new(p1_id);
        let mut controller2 = ZeroController::new(p2_id);

        // First strike damage step: attacker deals 2 damage, blocker takes none
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, true);
        assert!(result.is_ok(), "Failed to assign first strike damage: {result:?}");

        // Blocker should be dead (took 2 damage, toughness 2)
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(blocker_id),
                "Blocker should be in graveyard after first strike damage"
            );
        }

        // Normal damage step: only attacker can deal damage (blocker is dead)
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign normal damage: {result:?}");

        // Attacker should still be alive (never took damage)
        assert!(game.battlefield.contains(attacker_id), "Attacker should still be alive");

        // Check attacker is undamaged
        if let Ok(attacker) = game.cards.get(attacker_id) {
            assert_eq!(attacker.current_toughness(), 2, "Attacker should be undamaged");
        }
    }

    #[test]
    fn test_double_strike_creature_deals_damage_twice() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Load Adorned Pouncer (1/1 with Double Strike) as attacker
        let attacker_id = load_test_card(&mut game, "Adorned Pouncer", p1_id).expect("Failed to load Adorned Pouncer");

        // Set power/toughness to 3/3 so test works as before
        if let Ok(attacker) = game.cards.get_mut(attacker_id) {
            attacker.set_base_power(Some(3));
            attacker.set_base_toughness(Some(3));
            attacker.controller = p1_id;
            attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        }
        game.battlefield.add(attacker_id);

        // Declare unblocked attacker
        game.combat.declare_attacker(attacker_id, p2_id);

        // Create controllers
        let mut controller1 = ZeroController::new(p1_id);
        let mut controller2 = ZeroController::new(p2_id);

        // First strike damage step: attacker deals 3 damage to player
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, true);
        assert!(result.is_ok(), "Failed to assign first strike damage: {result:?}");

        // Check player took 3 damage
        let p2 = game.get_player(p2_id).unwrap();
        assert_eq!(p2.life, 17, "Player should have taken 3 damage in first strike step");

        // Normal damage step: attacker deals another 3 damage to player
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign normal damage: {result:?}");

        // Check player took another 3 damage (total 6)
        let p2 = game.get_player(p2_id).unwrap();
        assert_eq!(
            p2.life, 14,
            "Player should have taken 6 total damage from double strike"
        );
    }

    #[test]
    fn test_double_strike_vs_first_strike() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 2/2 creature with Double Strike (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Double Strike Knight".to_string(), p1_id);
        attacker.add_type(CardType::Creature);
        attacker.set_base_power(Some(2));
        attacker.set_base_toughness(Some(2));
        attacker.controller = p1_id;
        attacker.keywords.insert(Keyword::DoubleStrike);
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create a 2/2 creature with First Strike (blocker)
        let blocker_id = game.next_entity_id();
        let mut blocker = Card::new(blocker_id, "First Strike Soldier".to_string(), p2_id);
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

        // First strike damage step: both creatures deal damage simultaneously
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, true);
        assert!(result.is_ok(), "Failed to assign first strike damage: {result:?}");

        // Both creatures should be dead (both took 2 damage, both have 2 toughness)
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(
                zones.graveyard.contains(attacker_id),
                "Double strike attacker should be in graveyard"
            );
        }
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(blocker_id),
                "First strike blocker should be in graveyard"
            );
        }

        // Normal damage step: no creatures left to deal damage
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign normal damage: {result:?}");

        // Both creatures should still be in graveyards
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(zones.graveyard.contains(attacker_id));
        }
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(zones.graveyard.contains(blocker_id));
        }
    }

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
}
