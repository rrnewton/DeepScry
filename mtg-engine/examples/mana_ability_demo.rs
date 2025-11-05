//! Mana Abilities Demo
//!
//! Demonstrates mana abilities like Llanowar Elves and Sol Ring
//! that produce mana to help cast spells

use mtg_forge_rs::core::{ActivatedAbility, Card, CardType, Cost, Effect, ManaCost};
use mtg_forge_rs::game::GameState;
use mtg_forge_rs::Result;

fn main() -> Result<()> {
    println!("=== MTG Forge - Mana Abilities Demo ===\n");
    println!("This demo shows mana abilities:");
    println!("  Llanowar Elves: {{T}}: Add {{G}}");
    println!("  Sol Ring: {{T}}: Add {{C}}{{C}}");
    println!("  1. Cast permanents with mana abilities");
    println!("  2. Wait a turn (summoning sickness for creatures)");
    println!("  3. Activate mana abilities to add mana to pool\n");

    // Create a game with two players
    let mut game = GameState::new_two_player("Alice".to_string(), "Bob".to_string(), 20);
    let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
    let alice_id = players[0];

    println!("=== Initial Setup ===");
    println!("Alice: 20 life, mana pool empty\n");

    // Create Llanowar Elves
    println!("=== Turn 1: Alice casts Llanowar Elves ===");
    let elves_id = game.next_card_id();
    let mut elves = Card::new(elves_id, "Llanowar Elves".to_string(), alice_id);
    elves.types.push(CardType::Creature);
    elves.power = Some(1);
    elves.toughness = Some(1);

    // Add mana ability: {T}: Add {G}
    let ability = ActivatedAbility::new(
        Cost::Tap,
        vec![Effect::AddMana {
            player: alice_id,
            mana: ManaCost::from_string("G"),
        }],
        "Add {G}".to_string(),
        true, // This IS a mana ability
    );
    elves.activated_abilities.push(ability);

    // Add to battlefield (simulating casting)
    game.cards.insert(elves_id, elves);
    game.battlefield.add(elves_id);
    game.cards.get_mut(elves_id)?.turn_entered_battlefield = Some(1);

    println!("Alice casts Llanowar Elves (1/1 creature)");
    println!("  Abilities: {{T}}: Add {{G}}");
    println!("  Note: Has summoning sickness (entered this turn)\n");

    // Create Sol Ring
    println!("=== Turn 1: Alice also casts Sol Ring ===");
    let sol_ring_id = game.next_card_id();
    let mut sol_ring = Card::new(sol_ring_id, "Sol Ring".to_string(), alice_id);
    sol_ring.types.push(CardType::Artifact);

    // Add mana ability: {T}: Add {C}{C}
    let ability = ActivatedAbility::new(
        Cost::Tap,
        vec![Effect::AddMana {
            player: alice_id,
            mana: ManaCost::from_string("CC"), // 2 colorless mana
        }],
        "Add {C}{C}".to_string(),
        true, // This IS a mana ability
    );
    sol_ring.activated_abilities.push(ability);

    // Add to battlefield
    game.cards.insert(sol_ring_id, sol_ring);
    game.battlefield.add(sol_ring_id);
    // Artifacts don't have summoning sickness

    println!("Alice casts Sol Ring (artifact)");
    println!("  Abilities: {{T}}: Add {{C}}{{C}}");
    println!("  Note: Artifacts don't have summoning sickness\n");

    // Check initial mana pool
    println!("Turn 1 - Alice's mana pool:");
    let alice = game.get_player(alice_id)?;
    println!(
        "  W:{} U:{} B:{} R:{} G:{} C:{}",
        alice.mana_pool.white,
        alice.mana_pool.blue,
        alice.mana_pool.black,
        alice.mana_pool.red,
        alice.mana_pool.green,
        alice.mana_pool.colorless
    );
    println!();

    // Try to activate Sol Ring (should work - no summoning sickness)
    println!("Alice activates Sol Ring:");
    println!("  Cost: {{T}} (tap the artifact)");
    println!("  Effect: Add {{C}}{{C}}");

    // Pay the cost (tap)
    game.pay_ability_cost(alice_id, sol_ring_id, &Cost::Tap)?;
    println!("  ✓ Paid cost: Tapped Sol Ring");

    // Execute the effect
    let effect = Effect::AddMana {
        player: alice_id,
        mana: ManaCost::from_string("CC"),
    };
    game.execute_effect(&effect)?;
    println!("  ✓ Executed effect: Added {{C}}{{C}} to mana pool");

    // Check mana pool after Sol Ring
    println!("\nAfter Sol Ring activation:");
    let alice = game.get_player(alice_id)?;
    println!(
        "  W:{} U:{} B:{} R:{} G:{} C:{}",
        alice.mana_pool.white,
        alice.mana_pool.blue,
        alice.mana_pool.black,
        alice.mana_pool.red,
        alice.mana_pool.green,
        alice.mana_pool.colorless
    );
    println!();

    // Advance to turn 2
    println!("=== Turn 2: Llanowar Elves can now activate ===");
    game.turn.turn_number = 2;

    println!("Alice activates Llanowar Elves:");
    println!("  Cost: {{T}} (tap the creature)");
    println!("  Effect: Add {{G}}");

    // Pay the cost (tap)
    game.pay_ability_cost(alice_id, elves_id, &Cost::Tap)?;
    println!("  ✓ Paid cost: Tapped Llanowar Elves");

    // Execute the effect
    let effect = Effect::AddMana {
        player: alice_id,
        mana: ManaCost::from_string("G"),
    };
    game.execute_effect(&effect)?;
    println!("  ✓ Executed effect: Added {{G}} to mana pool");

    // Final mana pool
    println!("\n=== Final State ===");
    println!("Alice's mana pool:");
    let alice = game.get_player(alice_id)?;
    println!(
        "  W:{} U:{} B:{} R:{} G:{} C:{}",
        alice.mana_pool.white,
        alice.mana_pool.blue,
        alice.mana_pool.black,
        alice.mana_pool.red,
        alice.mana_pool.green,
        alice.mana_pool.colorless
    );

    // Verify the results
    assert_eq!(alice.mana_pool.green, 1, "Should have 1 green mana");
    assert_eq!(alice.mana_pool.colorless, 2, "Should have 2 colorless mana");
    assert!(game.cards.get(elves_id)?.tapped, "Llanowar Elves should be tapped");
    assert!(game.cards.get(sol_ring_id)?.tapped, "Sol Ring should be tapped");

    println!("\n✅ Mana abilities demo completed successfully!");
    println!("   - Sol Ring added {{C}}{{C}} (2 colorless mana)");
    println!("   - Llanowar Elves added {{G}} (1 green mana)");
    println!("   - Both permanents are now tapped");
    println!("   - Alice has 1 green + 2 colorless mana available");

    Ok(())
}
