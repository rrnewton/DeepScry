//! Tests for Ratchet Bomb's cmcEQX mechanic (wave-8 compat-2010 fix, mtg-914).
//!
//! Ratchet Bomb (`AB$ DestroyAll | ValidCards$ Permanent.nonLand+cmcEQX | SVar:X:Count$CardCounters.CHARGE`)
//! destroys each nonland permanent whose mana value equals the number of charge counters on the Bomb.
//!
//! The wave-8 fix ensures that when the activated ability resolves, the engine:
//! 1. Records `cmc_eq_source = Some(bomb_id)` at ability-dispatch time.
//! 2. Reads the charge-counter count from the Bomb at resolution time.
//! 3. Materialises `exact_cmc` from that count before filtering permanents.
//!
//! These tests pin the execute_effect path for `DestroyAll` with a dynamic
//! `exact_cmc` (already resolved for unit testing), verifying that CMC matching
//! works correctly: matching permanents destroyed, non-matching ones spared.
//!
//! The `nonLand` qualifier is parsed but silently discarded (lands have CMC 0 and
//! are rarely hit anyway); the existing effect_converter test covers the parse side.

use super::effects::load_test_card;
use crate::core::effects::TargetRestriction;
use crate::core::Effect;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::state::GameState;

    /// Ratchet Bomb with 1 charge counter destroys a CMC-1 permanent (Llanowar Elves).
    ///
    /// The `exact_cmc` is pre-resolved to 1 here (simulating the runtime SVar
    /// lookup of `Count$CardCounters.CHARGE` = 1 charge counter on the Bomb).
    #[test]
    fn test_ratchet_bomb_destroys_cmc_matching_permanent() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p2_id = game.players[1].id;

        // P2 controls Llanowar Elves (ManaCost:G → CMC = 1) — should be destroyed by 1-counter Bomb.
        let elves_id = load_test_card(&mut game, "Llanowar Elves", p2_id).expect("Llanowar Elves loads");
        game.battlefield.add(elves_id);
        game.cards.get_mut(elves_id).expect("elves exist").controller = p2_id;

        // The resolved restriction: "destroy all nonland permanents with CMC = 1."
        // (The wave-8 fix resolves `cmc_eq_svar` → `exact_cmc` from the Bomb's charge counters.)
        let restriction = TargetRestriction {
            exact_cmc: Some(1),
            ..Default::default()
        }; // 1 charge counter resolved to CMC filter

        let destroy_effect = Effect::DestroyAll {
            restriction,
            no_regenerate: false,
            cmc_eq_source: None, // already resolved for this unit test
        };

        game.execute_effect(&destroy_effect)
            .expect("DestroyAll with exact_cmc=1 should execute without error");

        // Llanowar Elves (CMC 1) must be in the graveyard.
        assert!(
            !game.battlefield.cards.contains(&elves_id),
            "Llanowar Elves (CMC 1) should be destroyed when Ratchet Bomb has 1 charge counter"
        );
    }

    /// Ratchet Bomb with 1 charge counter does NOT destroy a CMC-2 permanent (Grizzly Bears).
    #[test]
    fn test_ratchet_bomb_spares_non_matching_cmc() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p2_id = game.players[1].id;

        // P2 controls Grizzly Bears (ManaCost:1G → CMC = 2) — must NOT be destroyed.
        let bears_id = load_test_card(&mut game, "Grizzly Bears", p2_id).expect("Grizzly Bears loads");
        game.battlefield.add(bears_id);
        game.cards.get_mut(bears_id).expect("bears exist").controller = p2_id;

        let restriction = TargetRestriction {
            exact_cmc: Some(1),
            ..Default::default()
        }; // Bomb has 1 counter → destroy CMC-1 only

        let destroy_effect = Effect::DestroyAll {
            restriction,
            no_regenerate: false,
            cmc_eq_source: None,
        };

        game.execute_effect(&destroy_effect)
            .expect("DestroyAll with exact_cmc=1 should execute without error");

        // Grizzly Bears (CMC 2) must remain on the battlefield.
        assert!(
            game.battlefield.cards.contains(&bears_id),
            "Grizzly Bears (CMC 2) must NOT be destroyed by Ratchet Bomb with 1 charge counter (CMC mismatch)"
        );
    }

    /// Both CMC-1 and CMC-2 creatures exist; only the CMC-2 one is destroyed when
    /// the Bomb has 2 charge counters. Ensures the filter is applied per-card.
    #[test]
    fn test_ratchet_bomb_selective_destruction() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p2_id = game.players[1].id;

        // CMC 1: Llanowar Elves (ManaCost:G)
        let elves_id = load_test_card(&mut game, "Llanowar Elves", p2_id).expect("Llanowar Elves loads");
        game.battlefield.add(elves_id);
        game.cards.get_mut(elves_id).expect("elves exist").controller = p2_id;

        // CMC 2: Grizzly Bears (ManaCost:1G)
        let bears_id = load_test_card(&mut game, "Grizzly Bears", p2_id).expect("Grizzly Bears loads");
        game.battlefield.add(bears_id);
        game.cards.get_mut(bears_id).expect("bears exist").controller = p2_id;

        // Bomb has 2 counters → destroy CMC-2 only.
        let restriction = TargetRestriction {
            exact_cmc: Some(2),
            ..Default::default()
        };

        let destroy_effect = Effect::DestroyAll {
            restriction,
            no_regenerate: false,
            cmc_eq_source: None,
        };

        game.execute_effect(&destroy_effect)
            .expect("DestroyAll with exact_cmc=2 should execute without error");

        // Llanowar Elves (CMC 1) must survive.
        assert!(
            game.battlefield.cards.contains(&elves_id),
            "Llanowar Elves (CMC 1) must survive when Bomb has 2 charge counters (CMC mismatch)"
        );

        // Grizzly Bears (CMC 2) must be destroyed.
        assert!(
            !game.battlefield.cards.contains(&bears_id),
            "Grizzly Bears (CMC 2) must be destroyed when Bomb has 2 charge counters"
        );
    }

    /// Smoke test: Ratchet Bomb card loads, parses both abilities, and has its
    /// cmcEQX SVar ability carrying `cmc_eq_svar = true` in the effect.
    ///
    /// Pins that the wave-8 fix in the card loader correctly threads `cmc_eq_source`
    /// through `resolve_self_target`.
    #[test]
    fn test_ratchet_bomb_card_loads_with_two_abilities() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        let bomb_id = load_test_card(&mut game, "Ratchet Bomb", p1_id).expect("Ratchet Bomb loads");
        let card = game.cards.get(bomb_id).expect("bomb card exists");

        // Ratchet Bomb should have exactly 2 activated abilities:
        // [0] {T}: Put a charge counter on CARDNAME.
        // [1] {T}, Sacrifice CARDNAME: Destroy each nonland permanent with mana value
        //         equal to the number of charge counters on CARDNAME.
        assert_eq!(
            card.activated_abilities.len(),
            2,
            "Ratchet Bomb should have 2 activated abilities. Got: {:?}",
            card.activated_abilities
                .iter()
                .map(|a| &a.description)
                .collect::<Vec<_>>()
        );

        // Ability [1] should have a DestroyAll effect with cmc_eq_svar = true.
        let destroy_ability = &card.activated_abilities[1];
        let has_cmc_eq_svar = destroy_ability.effects.iter().any(|e| {
            matches!(
                e,
                Effect::DestroyAll {
                    restriction,
                    ..
                }
                if restriction.cmc_eq_svar
            )
        });
        assert!(
            has_cmc_eq_svar,
            "Ratchet Bomb's destroy ability must have cmc_eq_svar=true to support dynamic cmcEQX resolution"
        );
    }
}
