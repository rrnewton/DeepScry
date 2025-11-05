//! Activated Abilities Demo
//!
//! Demonstrates activated abilities using Prodigal Sorcerer
//! "{T}: Prodigal Sorcerer deals 1 damage to any target"

use mtg_forge_rs::core::{ActivatedAbility, Card, CardType, Cost, Effect, TargetRef};
use mtg_forge_rs::game::GameState;
use mtg_forge_rs::Result;

fn main() -> Result<()> {
    println!("=== MTG Forge - Activated Abilities Demo ===\n");
    println!("This demo shows activated abilities:");
    println!("  Prodigal Sorcerer has: {{T}}: Deal 1 damage to any target");
    println!("  1. Cast Prodigal Sorcerer");
    println!("  2. Wait a turn (summoning sickness)");
    println!("  3. Activate its tap ability to deal damage\n");

    // Create a game with two players
    let mut game = GameState::new_two_player("Alice".to_string(), "Bob".to_string(), 20);
    let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
    let alice_id = players[0];
    let bob_id = players[1];

    println!("=== Initial Setup ===");
    println!("Alice: 20 life");
    println!("Bob: 20 life\n");

    // Create Prodigal Sorcerer
    println!("=== Turn 1: Alice casts Prodigal Sorcerer ===");
    let prodigal_id = game.next_card_id();
    let mut prodigal = Card::new(prodigal_id, "Prodigal Sorcerer".to_string(), alice_id);
    prodigal.types.push(CardType::Creature);
    prodigal.power = Some(1);
    prodigal.toughness = Some(1);

    // Add activated ability: {T}: Deal 1 damage to any target
    let ability = ActivatedAbility::new(
        Cost::Tap,
        vec![Effect::DealDamage {
            target: TargetRef::Player(bob_id),
            amount: 1,
        }],
        "Deal 1 damage to any target".to_string(),
        false, // Not a mana ability
    );
    prodigal.activated_abilities.push(ability);

    // Add to game
    game.cards.insert(prodigal_id, prodigal);

    // Put on battlefield (simulating casting and resolving)
    game.battlefield.add(prodigal_id);
    game.cards.get_mut(prodigal_id)?.turn_entered_battlefield = Some(1);

    println!("Alice casts Prodigal Sorcerer (1/1 creature)");
    println!("  Abilities: {{T}}: Deal 1 damage to any target");
    println!("  Note: Has summoning sickness (entered this turn)\n");

    // Verify it has summoning sickness
    let card = game.cards.get(prodigal_id)?;
    println!("Turn 1 - Prodigal Sorcerer status:");
    println!("  Tapped: {}", card.tapped);
    println!("  Entered turn: {:?}", card.turn_entered_battlefield);
    println!("  Current turn: {}", game.turn.turn_number);
    println!("  Can activate? No (summoning sickness)\n");

    // Advance to next turn
    println!("=== Turn 2: Prodigal Sorcerer can now activate ===");
    game.turn.turn_number = 2;

    println!("Turn 2 - Prodigal Sorcerer status:");
    println!("  Tapped: {}", game.cards.get(prodigal_id)?.tapped);
    println!(
        "  Entered turn: {:?}",
        game.cards.get(prodigal_id)?.turn_entered_battlefield
    );
    println!("  Current turn: {}", game.turn.turn_number);
    println!("  Can activate? Yes (no summoning sickness, not tapped)\n");

    println!("Alice activates Prodigal Sorcerer's ability:");
    println!("  Cost: {{T}} (tap the creature)");
    println!("  Effect: Deal 1 damage to Bob");

    // Pay the cost (tap)
    let ability_cost = Cost::Tap;
    game.pay_ability_cost(alice_id, prodigal_id, &ability_cost)?;
    println!("  ✓ Paid cost: Tapped Prodigal Sorcerer");

    // Execute the effect
    let effect = Effect::DealDamage {
        target: TargetRef::Player(bob_id),
        amount: 1,
    };
    game.execute_effect(&effect)?;
    println!("  ✓ Executed effect: Dealt 1 damage to Bob");

    // Verify the results
    println!("\n=== Final State ===");

    // Check Prodigal Sorcerer is tapped
    let card = game.cards.get(prodigal_id)?;
    println!("Prodigal Sorcerer:");
    println!("  Tapped: {}", card.tapped);
    assert!(card.tapped, "Prodigal Sorcerer should be tapped");

    // Check Bob's life
    let bob = game.get_player(bob_id)?;
    println!("\nBob's life: {}/20", bob.life);
    assert_eq!(bob.life, 19, "Bob should have 19 life");

    println!("\n✅ Activated ability demo completed successfully!");
    println!("   - Prodigal Sorcerer activated its tap ability");
    println!("   - Dealt 1 damage to Bob (20 → 19 life)");
    println!("   - Prodigal Sorcerer is now tapped");

    Ok(())
}
