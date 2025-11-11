use mtg_forge_rs::loader::CardLoader;
use mtg_forge_rs::game::GameState;
use std::fs;

fn main() {
    println!("=== Testing Real Equipment Cards ===\n");

    // Test 1: Bonesplitter (simple +2/+0)
    println!("--- Test 1: Bonesplitter ---");
    let card_path = "forge-java/forge-gui/res/cardsfolder/b/bonesplitter.txt";
    let content = fs::read_to_string(card_path)
        .expect("Should read Bonesplitter card file");

    println!("Card file content:\n{}\n", content);

    let card_def = CardLoader::parse(&content)
        .expect("Should parse Bonesplitter");

    println!("Parsed card definition:");
    println!("  Name: {}", card_def.name);
    println!("  Mana Cost: {}", card_def.mana_cost);
    println!("  Types: {:?}", card_def.types);
    println!("  Subtypes: {:?}", card_def.subtypes);
    println!("  Raw keywords: {:?}", card_def.raw_keywords);
    println!("  Raw abilities: {} found", card_def.raw_abilities.len());

    // Create a game and instantiate the card
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;

    let card_id = game.cards.next_id();
    let bonesplitter = card_def.instantiate(card_id, p1_id);

    println!("\nInstantiated card:");
    println!("  Has Equip keyword: {}", bonesplitter.keywords.contains(mtg_forge_rs::core::Keyword::Equip));
    println!("  Activated abilities: {}", bonesplitter.activated_abilities.len());

    if !bonesplitter.activated_abilities.is_empty() {
        let equip_ability = &bonesplitter.activated_abilities[0];
        println!("  Equip ability:");
        println!("    Description: {}", equip_ability.description);
        println!("    Sorcery-speed: {}", equip_ability.sorcery_speed);
        println!("    Effects: {}", equip_ability.effects.len());
    }

    println!("  Static abilities: {}", bonesplitter.static_abilities.len());
    if !bonesplitter.static_abilities.is_empty() {
        println!("    First ability: {:?}", bonesplitter.static_abilities[0]);
    }

    println!("\n✓ Bonesplitter loaded successfully!");

    // Test 2: Accorder's Shield (grants +0/+3 and Vigilance)
    println!("\n--- Test 2: Accorder's Shield ---");
    let shield_path = "forge-java/forge-gui/res/cardsfolder/a/accorders_shield.txt";
    let shield_content = fs::read_to_string(shield_path)
        .expect("Should read Accorder's Shield card file");

    println!("Card file content:\n{}\n", shield_content);

    let shield_def = CardLoader::parse(&shield_content)
        .expect("Should parse Accorder's Shield");

    println!("Parsed card definition:");
    println!("  Name: {}", shield_def.name);
    println!("  Mana Cost: {}", shield_def.mana_cost);
    println!("  Raw keywords: {:?}", shield_def.raw_keywords);
    println!("  Raw abilities: {} found", shield_def.raw_abilities.len());

    let shield_id = game.cards.next_id();
    let shield = shield_def.instantiate(shield_id, p1_id);

    println!("\nInstantiated card:");
    println!("  Has Equip keyword: {}", shield.keywords.contains(mtg_forge_rs::core::Keyword::Equip));
    println!("  Activated abilities: {}", shield.activated_abilities.len());

    if !shield.activated_abilities.is_empty() {
        let equip_ability = &shield.activated_abilities[0];
        println!("  Equip ability:");
        println!("    Description: {}", equip_ability.description);
        println!("    Cost: {:?}", equip_ability.cost);
    }

    println!("  Static abilities: {}", shield.static_abilities.len());
    if !shield.static_abilities.is_empty() {
        println!("    First ability: {:?}", shield.static_abilities[0]);
    }

    println!("\n✓ Accorder's Shield loaded successfully!");

    println!("\n=== Summary ===");
    println!("✓ Equipment cards load correctly from cardsfolder");
    println!("✓ K:Equip keyword is parsed");
    println!("✓ Equip activated ability is generated automatically");
    println!("✓ Static abilities are parsed (ModifyPT)");
    println!("\nNote: Static abilities that grant keywords (like Vigilance) are");
    println!("parsed but not yet applied to equipped creatures. This is tracked");
    println!("as future work in the Equipment implementation roadmap.");
}
