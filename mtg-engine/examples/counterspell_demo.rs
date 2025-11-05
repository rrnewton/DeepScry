//! Counterspell Demo
//!
//! Demonstrates counterspell mechanics - when one player casts a spell and
//! their opponent responds with a counterspell to prevent it from resolving.

use mtg_forge_rs::core::{Card, CardType, Effect, ManaCost, TargetRef};
use mtg_forge_rs::game::GameState;
use mtg_forge_rs::Result;

fn main() -> Result<()> {
    println!("=== MTG Forge - Counterspell Demo ===\n");
    println!("This demo shows the stack and instant-speed interaction:");
    println!("  1. Alice casts Lightning Bolt targeting Bob");
    println!("  2. Bob responds with Counterspell");
    println!("  3. Counterspell resolves first (LIFO), countering Lightning Bolt");
    println!("  4. Lightning Bolt never resolves - Bob takes no damage\n");

    // Create a game with two players
    let mut game = GameState::new_two_player("Alice".to_string(), "Bob".to_string(), 20);
    let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
    let alice_id = players[0];
    let bob_id = players[1];

    println!("=== Initial Setup ===");
    println!("Alice: 20 life");
    println!("Bob: 20 life\n");

    // Add some mana sources for both players (simplified - not on battlefield, just for tracking)
    println!("=== Turn 1: Alice casts Lightning Bolt ===");

    // Alice casts Lightning Bolt targeting Bob
    let bolt_id = game.next_card_id();
    let mut bolt = Card::new(bolt_id, "Lightning Bolt".to_string(), alice_id);
    bolt.types.push(CardType::Instant);
    bolt.mana_cost = ManaCost::from_string("R");
    bolt.effects.push(Effect::DealDamage {
        target: TargetRef::Player(bob_id),
        amount: 3,
    });
    game.cards.insert(bolt_id, bolt);

    // Put Lightning Bolt on the stack
    game.stack.add(bolt_id);
    println!("Alice casts Lightning Bolt targeting Bob");
    println!("  Lightning Bolt is on the stack");
    println!("  If it resolves, Bob will take 3 damage (20 → 17 life)\n");

    // Show stack state
    println!("Stack (top to bottom): [Lightning Bolt ({bolt_id})]");
    println!("Stack size: {} spell(s)\n", game.stack.cards.len());

    // Bob responds with Counterspell
    println!("=== Bob responds with Counterspell ===");
    let counter_id = game.next_card_id();
    let mut counterspell = Card::new(counter_id, "Counterspell".to_string(), bob_id);
    counterspell.types.push(CardType::Instant);
    counterspell.mana_cost = ManaCost::from_string("UU");
    // Target Lightning Bolt
    counterspell.effects.push(Effect::CounterSpell { target: bolt_id });
    game.cards.insert(counter_id, counterspell);

    // Put Counterspell on the stack (on top of Lightning Bolt)
    game.stack.add(counter_id);
    println!("Bob casts Counterspell targeting Lightning Bolt");
    println!("  Counterspell is on the stack (on top of Lightning Bolt)");
    println!("  If it resolves, Lightning Bolt will be countered\n");

    // Show stack state
    println!("Stack (top to bottom): [Counterspell ({counter_id}), Lightning Bolt ({bolt_id})]");
    println!("Stack size: {} spell(s)\n", game.stack.cards.len());

    // Resolve Counterspell (LIFO - Last In, First Out)
    println!("=== Resolving the Stack ===");
    println!("Stack resolves in Last-In-First-Out (LIFO) order:\n");

    println!("1. Counterspell ({counter_id}) resolves:");
    game.resolve_spell(counter_id, &[])?;
    println!("   ✓ Countered Lightning Bolt ({bolt_id})");
    println!("   ✓ Lightning Bolt moved from stack to graveyard");
    println!("   ✓ Counterspell moved to graveyard\n");

    // Verify the state
    println!("=== Final State ===");

    // Check stack
    println!("Stack: {} spell(s)", game.stack.cards.len());
    assert_eq!(game.stack.cards.len(), 0, "Stack should be empty after resolution");

    // Check Bob's life
    let bob = game.get_player(bob_id)?;
    println!("Bob's life: {}", bob.life);
    assert_eq!(bob.life, 20, "Bob should still have 20 life");

    // Check graveyards
    if let Some(alice_zones) = game.get_player_zones(alice_id) {
        println!(
            "Alice's graveyard: {} card(s) [Lightning Bolt]",
            alice_zones.graveyard.cards.len()
        );
        assert!(
            alice_zones.graveyard.contains(bolt_id),
            "Lightning Bolt should be in Alice's graveyard"
        );
    }

    if let Some(bob_zones) = game.get_player_zones(bob_id) {
        println!(
            "Bob's graveyard: {} card(s) [Counterspell]",
            bob_zones.graveyard.cards.len()
        );
        assert!(
            bob_zones.graveyard.contains(counter_id),
            "Counterspell should be in Bob's graveyard"
        );
    }

    println!("\n=== Demo Complete ===");
    println!("Key concepts demonstrated:");
    println!("  ✓ The stack (LIFO ordering)");
    println!("  ✓ Instant-speed interaction");
    println!("  ✓ Counterspell mechanics");
    println!("  ✓ Spells that target other spells");
    println!("  ✓ Countered spells go to graveyard without resolving");

    Ok(())
}
