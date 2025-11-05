//! Basic Land Demo
//!
//! Demonstrates that basic lands (Plains, Island, Swamp, Mountain, Forest)
//! automatically get implicit mana abilities even though they're not explicitly
//! written in the card files.

use mtg_forge_rs::game::GameState;
use mtg_forge_rs::loader::CardDatabase;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== MTG Forge - Basic Land Mana Abilities Demo ===\n");
    println!("This demo shows that basic lands get implicit mana abilities:");
    println!("  Plains: {{T}}: Add {{W}}");
    println!("  Island: {{T}}: Add {{U}}");
    println!("  Swamp: {{T}}: Add {{B}}");
    println!("  Mountain: {{T}}: Add {{R}}");
    println!("  Forest: {{T}}: Add {{G}}\n");

    // Create a game with two players
    let mut game = GameState::new_two_player("Alice".to_string(), "Bob".to_string(), 20);
    let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
    let alice_id = players[0];

    // Load card database
    let cardsfolder = PathBuf::from("./cardsfolder");
    let db = CardDatabase::new(cardsfolder);

    println!("=== Loading Basic Lands from Cardsfolder ===\n");

    // Load and play each basic land
    let basic_lands = vec![
        ("Forest", "G", "Green"),
        ("Plains", "W", "White"),
        ("Island", "U", "Blue"),
        ("Swamp", "B", "Black"),
        ("Mountain", "R", "Red"),
    ];

    let mut land_ids = Vec::new();

    for (land_name, mana_symbol, color_name) in &basic_lands {
        println!("Loading {land_name}...");

        // Load the land from cardsfolder
        let card_def = db
            .get_card(land_name)
            .await?
            .unwrap_or_else(|| panic!("{land_name} not found in cardsfolder"));

        // Instantiate the card
        let card_id = game.next_card_id();
        let card = card_def.instantiate(card_id, alice_id);

        println!("  Loaded: {}", card.name.as_str());
        println!("  Types: {:?}", card.types);
        println!("  Subtypes: {:?}", card.subtypes);
        println!("  Activated abilities: {}", card.activated_abilities.len());

        // Verify it has a mana ability
        let has_mana_ability = card.activated_abilities.iter().any(|ab| ab.is_mana_ability);
        println!("  Has mana ability: {has_mana_ability}");

        if let Some(mana_ability) = card.activated_abilities.iter().find(|ab| ab.is_mana_ability) {
            println!("  Description: {}", mana_ability.description);
        }

        // Add to game
        game.cards.insert(card_id, card);
        game.battlefield.add(card_id);
        land_ids.push((card_id, mana_symbol, color_name));

        println!();
    }

    // Now activate each land to produce mana
    println!("=== Activating Mana Abilities ===\n");
    println!("Initial mana pool:");
    let alice = game.get_player(alice_id)?;
    println!(
        "  W:{} U:{} B:{} R:{} G:{} C:{}\n",
        alice.mana_pool.white,
        alice.mana_pool.blue,
        alice.mana_pool.black,
        alice.mana_pool.red,
        alice.mana_pool.green,
        alice.mana_pool.colorless
    );

    for (land_id, expected_mana, color_name) in &land_ids {
        let land_name = game.cards.get(*land_id)?.name.clone();

        println!("Activating {} ({})", land_name.as_str(), color_name);

        // Get the mana ability
        let ability = game
            .cards
            .get(*land_id)?
            .activated_abilities
            .iter()
            .find(|ab| ab.is_mana_ability)
            .expect("Should have mana ability")
            .clone();

        // Pay the cost (tap)
        use mtg_forge_rs::core::Cost;
        game.pay_ability_cost(alice_id, *land_id, &Cost::Tap)?;
        println!("  ✓ Paid cost: Tapped {}", land_name.as_str());

        // Execute the effect
        for effect in &ability.effects {
            // Fix placeholder player ID
            let fixed_effect = match effect {
                mtg_forge_rs::core::Effect::AddMana { player, mana } if player.as_u32() == 0 => {
                    mtg_forge_rs::core::Effect::AddMana {
                        player: alice_id,
                        mana: *mana,
                    }
                }
                _ => effect.clone(),
            };

            game.execute_effect(&fixed_effect)?;
        }
        println!("  ✓ Added {{{expected_mana}}}");
        println!();
    }

    // Check final mana pool
    println!("=== Final Mana Pool ===");
    let alice = game.get_player(alice_id)?;
    println!(
        "W:{} U:{} B:{} R:{} G:{} C:{}",
        alice.mana_pool.white,
        alice.mana_pool.blue,
        alice.mana_pool.black,
        alice.mana_pool.red,
        alice.mana_pool.green,
        alice.mana_pool.colorless
    );

    // Verify we have one of each color
    assert_eq!(alice.mana_pool.white, 1, "Should have 1 white mana");
    assert_eq!(alice.mana_pool.blue, 1, "Should have 1 blue mana");
    assert_eq!(alice.mana_pool.black, 1, "Should have 1 black mana");
    assert_eq!(alice.mana_pool.red, 1, "Should have 1 red mana");
    assert_eq!(alice.mana_pool.green, 1, "Should have 1 green mana");

    // Verify all lands are tapped
    for (land_id, _, _) in &land_ids {
        assert!(
            game.cards.get(*land_id)?.tapped,
            "{} should be tapped",
            game.cards.get(*land_id)?.name.as_str()
        );
    }

    println!("\n✅ Basic land mana abilities demo completed successfully!");
    println!("   - All 5 basic lands loaded from cardsfolder");
    println!("   - Each land automatically has implicit mana ability");
    println!("   - Each land produced 1 mana of its color");
    println!("   - All lands are now tapped");

    Ok(())
}
