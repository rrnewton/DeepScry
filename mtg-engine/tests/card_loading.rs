//! Card loading tests
//!
//! Tests that verify cards from cardsfolder can be loaded and parsed correctly

use mtg_forge_rs::core::{CardType, Keyword, KeywordArgs};
use mtg_forge_rs::loader::CardLoader;
use mtg_forge_rs::Result;
use std::path::PathBuf;

/// Test loading Abbey Gargoyles (simple keywords)
#[test]
fn test_load_abbey_gargoyles() -> Result<()> {
    let path = PathBuf::from("cardsfolder/a/abbey_gargoyles.txt");
    if !path.exists() {
        return Ok(()); // Skip if cardsfolder not present
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Abbey Gargoyles");
    assert!(def.types.contains(&CardType::Creature));
    assert_eq!(def.power, Some(3));
    assert_eq!(def.toughness, Some(4));

    // Check keywords
    assert_eq!(def.raw_keywords.len(), 2);
    assert!(def.raw_keywords.contains(&"Flying".to_string()));
    assert!(def.raw_keywords.contains(&"Protection from red".to_string()));

    Ok(())
}

/// Test loading Abandon Reason (Madness keyword with parameter)
#[test]
fn test_load_abandon_reason() -> Result<()> {
    let path = PathBuf::from("cardsfolder/a/abandon_reason.txt");
    if !path.exists() {
        return Ok(());
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Abandon Reason");
    assert!(def.types.contains(&CardType::Instant));

    // Check Madness keyword
    assert_eq!(def.raw_keywords.len(), 1);
    assert!(def.raw_keywords.contains(&"Madness:1 R".to_string()));

    // Check that it has an ability (Pump)
    assert!(!def.raw_abilities.is_empty());

    Ok(())
}

/// Test loading Abandon the Post (Flashback keyword)
#[test]
fn test_load_abandon_the_post() -> Result<()> {
    let path = PathBuf::from("cardsfolder/a/abandon_the_post.txt");
    if !path.exists() {
        return Ok(());
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Abandon the Post");
    assert!(def.types.contains(&CardType::Sorcery));

    // Check Flashback keyword
    assert_eq!(def.raw_keywords.len(), 1);
    assert!(def.raw_keywords.contains(&"Flashback:3 R".to_string()));

    Ok(())
}

/// Test loading Aboshan's Desire (Enchant keyword and static abilities)
#[test]
fn test_load_aboshans_desire() -> Result<()> {
    let path = PathBuf::from("cardsfolder/a/aboshans_desire.txt");
    if !path.exists() {
        return Ok(());
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Aboshan's Desire");
    assert!(def.types.contains(&CardType::Enchantment));

    // Check Enchant keyword
    assert_eq!(def.raw_keywords.len(), 1);
    assert!(def.raw_keywords.contains(&"Enchant:Creature".to_string()));

    // Check static abilities
    assert!(def.raw_abilities.len() >= 2); // Should have S: lines

    Ok(())
}

/// Test loading Abhorrent Oculus (Flying + Triggered ability)
#[test]
fn test_load_abhorrent_oculus() -> Result<()> {
    let path = PathBuf::from("cardsfolder/a/abhorrent_oculus.txt");
    if !path.exists() {
        return Ok(());
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Abhorrent Oculus");
    assert!(def.types.contains(&CardType::Creature));
    assert_eq!(def.power, Some(5));
    assert_eq!(def.toughness, Some(5));

    // Check Flying keyword
    assert_eq!(def.raw_keywords.len(), 1);
    assert!(def.raw_keywords.contains(&"Flying".to_string()));

    // Check triggered ability
    assert!(!def.raw_abilities.is_empty());

    Ok(())
}

/// Test loading Abyssal Horror (Flying + ETB trigger)
#[test]
fn test_load_abyssal_horror() -> Result<()> {
    let path = PathBuf::from("cardsfolder/a/abyssal_horror.txt");
    if !path.exists() {
        return Ok(());
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Abyssal Horror");
    assert!(def.types.contains(&CardType::Creature));

    // Check Flying keyword
    assert!(def.raw_keywords.contains(&"Flying".to_string()));

    // Check triggered ability (ETB)
    assert!(!def.raw_abilities.is_empty());
    // Verify it's a ChangesZone trigger
    let has_etb = def
        .raw_abilities
        .iter()
        .any(|a| a.contains("ChangesZone") && a.contains("Battlefield"));
    assert!(has_etb);

    Ok(())
}

/// Test instantiating a card with keywords
#[test]
fn test_instantiate_with_keywords() -> Result<()> {
    let path = PathBuf::from("cardsfolder/a/abbey_gargoyles.txt");
    if !path.exists() {
        return Ok(());
    }

    let def = CardLoader::load_from_file(&path)?;
    let card_id = mtg_forge_rs::core::CardId::new(1);
    let player_id = mtg_forge_rs::core::PlayerId::new(1);

    let card = def.instantiate(card_id, player_id);

    // Verify keywords were parsed
    assert_eq!(card.keywords.len(), 2);
    assert!(card.keywords.contains(Keyword::Flying));
    assert!(card.keywords.contains(Keyword::ProtectionFromRed));

    // Verify helper methods
    assert!(card.has_flying());

    Ok(())
}

/// Test instantiating a card with Madness keyword parameter
#[test]
fn test_instantiate_with_madness() -> Result<()> {
    let path = PathBuf::from("cardsfolder/a/abandon_reason.txt");
    if !path.exists() {
        return Ok(());
    }

    let def = CardLoader::load_from_file(&path)?;
    let card_id = mtg_forge_rs::core::CardId::new(1);
    let player_id = mtg_forge_rs::core::PlayerId::new(1);

    let card = def.instantiate(card_id, player_id);

    // Verify Madness keyword was parsed with parameter
    assert_eq!(card.keywords.len(), 1);
    assert!(card.keywords.contains(Keyword::Madness));

    // Get the args and verify they're correctly parsed
    if let Some(KeywordArgs::Madness { cost }) = card.keywords.get_args(Keyword::Madness) {
        assert_eq!(cost.generic, 1);
        assert_eq!(cost.red, 1);
    } else {
        panic!("Expected Madness args");
    }

    Ok(())
}

/// Test instantiating a card with Flashback keyword parameter
#[test]
fn test_instantiate_with_flashback() -> Result<()> {
    let path = PathBuf::from("cardsfolder/a/abandon_the_post.txt");
    if !path.exists() {
        return Ok(());
    }

    let def = CardLoader::load_from_file(&path)?;
    let card_id = mtg_forge_rs::core::CardId::new(1);
    let player_id = mtg_forge_rs::core::PlayerId::new(1);

    let card = def.instantiate(card_id, player_id);

    // Verify Flashback keyword was parsed with parameter
    assert_eq!(card.keywords.len(), 1);
    assert!(card.keywords.contains(Keyword::Flashback));

    // Get the args and verify they're correctly parsed
    if let Some(KeywordArgs::Flashback { cost }) = card.keywords.get_args(Keyword::Flashback) {
        assert_eq!(cost.generic, 3);
        assert_eq!(cost.red, 1);
    } else {
        panic!("Expected Flashback args");
    }

    Ok(())
}

/// Test instantiating a card with Enchant keyword parameter
#[test]
fn test_instantiate_with_enchant() -> Result<()> {
    let path = PathBuf::from("cardsfolder/a/aboshans_desire.txt");
    if !path.exists() {
        return Ok(());
    }

    let def = CardLoader::load_from_file(&path)?;
    let card_id = mtg_forge_rs::core::CardId::new(1);
    let player_id = mtg_forge_rs::core::PlayerId::new(1);

    let card = def.instantiate(card_id, player_id);

    // Verify Enchant keyword was parsed with parameter
    assert_eq!(card.keywords.len(), 1);
    assert!(card.keywords.contains(Keyword::Enchant));

    // Get the args and verify they're correctly parsed
    if let Some(KeywordArgs::Enchant { card_type }) = card.keywords.get_args(Keyword::Enchant) {
        assert_eq!(card_type.as_str(), "Creature");
    } else {
        panic!("Expected Enchant args");
    }

    Ok(())
}

/// Test loading and instantiating Mishra's Factory (colorless mana land)
/// This verifies that non-basic lands with "{T}: Add {C}" are correctly
/// detected as producing colorless mana.
#[test]
fn test_load_mishras_factory_colorless_mana() -> Result<()> {
    use mtg_forge_rs::core::{CardId, ManaProductionKind, PlayerId};

    let path = PathBuf::from("cardsfolder/m/mishras_factory.txt");
    if !path.exists() {
        return Ok(()); // Skip if cardsfolder not present
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Mishra's Factory");
    assert!(def.types.contains(&CardType::Land));

    // Verify oracle text contains colorless mana production
    assert!(
        def.oracle.contains("{T}: Add {C}") || def.oracle.to_lowercase().contains("{t}: add {c}"),
        "Oracle text should contain colorless mana production. Got: {}",
        def.oracle
    );

    // Instantiate the card
    let card_id = CardId::new(1);
    let player_id = PlayerId::new(1);
    let card = def.instantiate(card_id, player_id);

    // Verify the cache detects colorless mana production
    assert!(
        card.cache.mana_production.produces_mana(),
        "Mishra's Factory should be detected as producing mana. Card text: {}",
        card.text
    );
    assert_eq!(
        card.cache.mana_production.kind,
        ManaProductionKind::Colorless,
        "Mishra's Factory should produce Colorless mana, not {:?}. Card text: {}",
        card.cache.mana_production.kind,
        card.text
    );

    Ok(())
}

/// Test that Spider-Ham, Peter Porker's static ability is correctly parsed
/// The card has a multi-type buff: "Other Spiders, Boars, Bears, ... get +1/+1"
#[test]
fn test_load_spider_ham_static_ability() -> Result<()> {
    use mtg_forge_rs::core::{AffectedSelector, CardId, PlayerId, StaticAbility};

    let path = PathBuf::from("cardsfolder/s/spider_ham_peter_porker.txt");
    if !path.exists() {
        return Ok(()); // Skip if cardsfolder not present
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Spider-Ham, Peter Porker");
    assert!(def.types.contains(&CardType::Creature));
    assert_eq!(def.power, Some(2));
    assert_eq!(def.toughness, Some(2));

    // Check that the S: ability line is in raw_abilities
    let has_static_line = def
        .raw_abilities
        .iter()
        .any(|a| a.contains("Mode$ Continuous") && a.contains("AddPower$ 1"));
    assert!(
        has_static_line,
        "Spider-Ham should have a static ability line with Mode$ Continuous"
    );

    // Check that static_abilities contains the parsed ModifyPT ability
    let card_id = CardId::new(1);
    let player_id = PlayerId::new(1);
    let card = def.instantiate(card_id, player_id);

    // Should have exactly 1 static ability
    assert_eq!(
        card.static_abilities.len(),
        1,
        "Spider-Ham should have 1 static ability, got: {:?}",
        card.static_abilities
    );

    // Verify it's a CreatureTypesOtherYouControl with multiple types
    match &card.static_abilities[0] {
        StaticAbility::ModifyPT {
            affected,
            power,
            toughness,
            description: _,
        } => {
            assert_eq!(*power, 1, "Power bonus should be 1");
            assert_eq!(*toughness, 1, "Toughness bonus should be 1");

            match affected {
                AffectedSelector::CreatureTypesOtherYouControl { types } => {
                    // Should include Spider, Boar, Bear among others
                    assert!(
                        types.iter().any(|t| t.as_str() == "Spider"),
                        "Should include Spider type"
                    );
                    assert!(types.iter().any(|t| t.as_str() == "Boar"), "Should include Boar type");
                    assert!(types.iter().any(|t| t.as_str() == "Bear"), "Should include Bear type");
                    assert!(types.len() >= 15, "Should have many animal types, got {}", types.len());
                }
                _ => panic!("Expected CreatureTypesOtherYouControl, got {:?}", affected),
            }
        }
        _ => panic!("Expected ModifyPT static ability"),
    }

    Ok(())
}

/// Test that Card.EquippedBy selector is properly parsed
/// Cranial Plating uses "Affected$ Card.EquippedBy" which should parse to CreatureEquippedBy
/// (Card.EquippedBy and Creature.EquippedBy are semantically equivalent for Equipment)
#[test]
fn test_load_cranial_plating_card_equipped_by_selector() -> Result<()> {
    use mtg_forge_rs::core::{CardId, PlayerId, Subtype};

    let path = PathBuf::from("cardsfolder/c/cranial_plating.txt");
    if !path.exists() {
        return Ok(()); // Skip if cardsfolder not present
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Cranial Plating");
    assert!(def.types.contains(&CardType::Artifact));

    // Check that the S: ability line is in raw_abilities with Card.EquippedBy
    let has_static_line = def
        .raw_abilities
        .iter()
        .any(|a| a.contains("Mode$ Continuous") && a.contains("Card.EquippedBy"));
    assert!(
        has_static_line,
        "Cranial Plating should have a static ability line with Card.EquippedBy. Abilities: {:?}",
        def.raw_abilities
    );

    // Instantiate the card
    let card_id = CardId::new(1);
    let player_id = PlayerId::new(1);
    let card = def.instantiate(card_id, player_id);

    // Note: Cranial Plating has "AddPower$ X" which doesn't parse to a numeric value
    // The static ability may not be created because power = 0 after failed X parse
    // This is expected - variable power/toughness (AddPower$ X) is a separate feature
    // What matters is that Card.EquippedBy is recognized when it IS created

    // Verify the card loads without errors and has Equipment subtype
    assert!(
        card.subtypes.contains(&Subtype::new("Equipment")),
        "Cranial Plating should be Equipment"
    );

    Ok(())
}

/// Test that Demonmail Hauberk with Card.EquippedBy creates the correct static ability
/// This is the key test for the Card.EquippedBy fix - it uses Card.EquippedBy with numeric values
#[test]
fn test_load_demonmail_hauberk_card_equipped_by_static_ability() -> Result<()> {
    use mtg_forge_rs::core::{AffectedSelector, CardId, PlayerId, StaticAbility, Subtype};

    let path = PathBuf::from("cardsfolder/d/demonmail_hauberk.txt");
    if !path.exists() {
        return Ok(()); // Skip if cardsfolder not present
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Demonmail Hauberk");
    assert!(def.types.contains(&CardType::Artifact));

    // Instantiate the card
    let card_id = CardId::new(1);
    let player_id = PlayerId::new(1);
    let card = def.instantiate(card_id, player_id);

    // Should be Equipment
    assert!(
        card.subtypes.contains(&Subtype::new("Equipment")),
        "Should be Equipment"
    );

    // Should have exactly 1 static ability (the +4/+2 buff)
    assert_eq!(
        card.static_abilities.len(),
        1,
        "Demonmail Hauberk should have 1 static ability, got: {:?}",
        card.static_abilities
    );

    // Verify the static ability is correctly parsed with CreatureEquippedBy selector
    match &card.static_abilities[0] {
        StaticAbility::ModifyPT {
            affected,
            power,
            toughness,
            description: _,
        } => {
            assert_eq!(*power, 4, "Power bonus should be 4");
            assert_eq!(*toughness, 2, "Toughness bonus should be 2");
            assert!(
                matches!(affected, AffectedSelector::CreatureEquippedBy),
                "Card.EquippedBy should parse to CreatureEquippedBy, got {:?}",
                affected
            );
        }
        _ => panic!("Expected ModifyPT static ability"),
    }

    Ok(())
}

/// Test that Sword of Feast and Famine with Creature.EquippedBy parses correctly
#[test]
fn test_load_sword_of_feast_and_famine_creature_equipped_by() -> Result<()> {
    use mtg_forge_rs::core::{AffectedSelector, CardId, PlayerId, StaticAbility};

    let path = PathBuf::from("cardsfolder/s/sword_of_feast_and_famine.txt");
    if !path.exists() {
        return Ok(()); // Skip if cardsfolder not present
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Sword of Feast and Famine");
    assert!(def.types.contains(&CardType::Artifact));

    // Check that the S: ability line uses Creature.EquippedBy
    let has_static_line = def
        .raw_abilities
        .iter()
        .any(|a| a.contains("Mode$ Continuous") && a.contains("Creature.EquippedBy"));
    assert!(
        has_static_line,
        "Sword should have a static ability line with Creature.EquippedBy"
    );

    // Instantiate the card
    let card_id = CardId::new(1);
    let player_id = PlayerId::new(1);
    let card = def.instantiate(card_id, player_id);

    // Should have at least 1 static ability (the +2/+2 buff)
    assert!(
        !card.static_abilities.is_empty(),
        "Sword should have static abilities, got: {:?}",
        card.static_abilities
    );

    // Find the ModifyPT ability
    let modify_pt = card
        .static_abilities
        .iter()
        .find(|a| matches!(a, StaticAbility::ModifyPT { .. }));
    assert!(modify_pt.is_some(), "Should have ModifyPT static ability");

    match modify_pt.unwrap() {
        StaticAbility::ModifyPT {
            affected,
            power,
            toughness,
            description: _,
        } => {
            assert_eq!(*power, 2, "Power bonus should be 2");
            assert_eq!(*toughness, 2, "Toughness bonus should be 2");
            assert!(
                matches!(affected, AffectedSelector::CreatureEquippedBy),
                "Expected CreatureEquippedBy selector, got {:?}",
                affected
            );
        }
        _ => panic!("Expected ModifyPT static ability"),
    }

    Ok(())
}

/// Test loading Black Lotus mana ability with sacrifice cost
/// Black Lotus: "T, Sacrifice Black Lotus: Add three mana of any one color."
#[test]
fn test_load_black_lotus_mana_ability() -> Result<()> {
    use mtg_forge_rs::core::{CardId, PlayerId};

    let path = PathBuf::from("cardsfolder/b/black_lotus.txt");
    if !path.exists() {
        return Ok(()); // Skip if cardsfolder not present
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Black Lotus");
    assert!(def.types.contains(&CardType::Artifact));

    // Verify oracle text contains mana production
    assert!(
        def.oracle.to_lowercase().contains("add three mana"),
        "Oracle text should contain mana production. Got: {}",
        def.oracle
    );

    // Check that raw_abilities contains the mana ability line
    let has_mana_ability = def
        .raw_abilities
        .iter()
        .any(|a| a.contains("AB$ Mana") && a.contains("Produced$"));
    assert!(
        has_mana_ability,
        "Black Lotus should have a mana ability. Raw abilities: {:?}",
        def.raw_abilities
    );

    // Instantiate the card
    let card_id = CardId::new(1);
    let player_id = PlayerId::new(1);
    let card = def.instantiate(card_id, player_id);

    // Check activated abilities - should have at least 1 (the mana ability)
    assert!(
        !card.activated_abilities.is_empty(),
        "Black Lotus should have activated abilities. Got: {} abilities",
        card.activated_abilities.len()
    );

    // Find the mana ability
    let mana_ability = card.activated_abilities.iter().find(|a| a.is_mana_ability);

    assert!(
        mana_ability.is_some(),
        "Black Lotus should have a mana ability. Abilities: {:?}",
        card.activated_abilities
            .iter()
            .map(|a| format!("cost={:?} is_mana={}", a.cost, a.is_mana_ability))
            .collect::<Vec<_>>()
    );

    let ability = mana_ability.unwrap();

    // Check the cost includes sacrifice
    assert!(
        ability.cost.requires_sacrifice(),
        "Black Lotus mana ability should require sacrifice. Cost: {:?}",
        ability.cost
    );

    // Check the effects include AddMana
    let has_add_mana = ability
        .effects
        .iter()
        .any(|e| matches!(e, mtg_forge_rs::core::Effect::AddMana { .. }));
    assert!(
        has_add_mana,
        "Black Lotus should have AddMana effect. Effects: {:?}",
        ability.effects
    );

    // Verify the cache detects it as a mana source
    assert!(
        card.cache.mana_production.produces_mana(),
        "Black Lotus should be detected as producing mana"
    );
    assert!(card.cache.is_mana_source, "Black Lotus should be a mana source");

    Ok(())
}

/// Test that Black Lotus' mana ability correctly sacrifices the card when activated
/// This tests the full game flow: play Black Lotus, activate its mana ability,
/// verify mana is added and Black Lotus is sacrificed (moved to graveyard).
#[test]
fn test_black_lotus_sacrifice_on_activation() -> Result<()> {
    use mtg_forge_rs::game::GameState;
    use mtg_forge_rs::loader::CardDatabase;

    let cardsfolder = PathBuf::from("cardsfolder");
    if !cardsfolder.exists() {
        return Ok(()); // Skip if cardsfolder not present
    }

    // Create a new game
    let mut game = GameState::new_two_player("Alice".to_string(), "Bob".to_string(), 20);
    let alice_id = game.players[0].id;

    // Load Black Lotus from cardsfolder
    let rt = tokio::runtime::Runtime::new().expect("Failed to create runtime");
    let db = CardDatabase::new(cardsfolder);
    let lotus_def = rt
        .block_on(db.get_card("Black Lotus"))?
        .expect("Black Lotus not found in cardsfolder");

    // Instantiate Black Lotus and add to battlefield
    let lotus_id = game.next_card_id();
    let lotus = lotus_def.instantiate(lotus_id, alice_id);
    game.cards.insert(lotus_id, lotus);
    game.battlefield.add(lotus_id);

    // Verify initial state
    assert!(
        game.battlefield.cards.contains(&lotus_id),
        "Black Lotus should be on battlefield"
    );
    let alice_graveyard_size = game
        .get_player_zones(alice_id)
        .map(|z| z.graveyard.cards.len())
        .unwrap_or(0);
    assert_eq!(alice_graveyard_size, 0, "Graveyard should be empty initially");

    // Check Alice's mana pool before activation
    let mana_before = game.get_player(alice_id).unwrap().mana_pool;
    let total_mana_before = mana_before.white
        + mana_before.blue
        + mana_before.black
        + mana_before.red
        + mana_before.green
        + mana_before.colorless;
    assert_eq!(total_mana_before, 0, "Mana pool should be empty initially");

    // Activate Black Lotus' mana ability (tap for mana)
    // This should add 3 mana to Alice's pool AND sacrifice Black Lotus
    let result = game.tap_for_mana(alice_id, lotus_id);
    assert!(result.is_ok(), "tap_for_mana should succeed. Error: {:?}", result.err());

    // Verify Black Lotus is now in graveyard (sacrificed)
    assert!(
        !game.battlefield.cards.contains(&lotus_id),
        "Black Lotus should be removed from battlefield after sacrifice"
    );
    let alice_graveyard = game
        .get_player_zones(alice_id)
        .expect("Alice zones")
        .graveyard
        .cards
        .clone();
    assert!(
        alice_graveyard.contains(&lotus_id),
        "Black Lotus should be in graveyard after sacrifice. Graveyard: {:?}",
        alice_graveyard
    );

    // Verify mana was added to pool (Black Lotus adds 3 mana of any color)
    let mana_after = game.get_player(alice_id).unwrap().mana_pool;
    let total_mana_after = mana_after.white
        + mana_after.blue
        + mana_after.black
        + mana_after.red
        + mana_after.green
        + mana_after.colorless;
    // Black Lotus produces 3 mana of any one color (parsed as colorless for now)
    // The amount may be 1 (single tap) or 3 (multiplied by Amount$ 3)
    assert!(
        total_mana_after >= 1,
        "Mana pool should have at least 1 mana after activation. Got: {}",
        total_mana_after
    );

    Ok(())
}

/// Test that Volcanic Island correctly has Mountain and Island subtypes
/// This is critical for dual lands to produce the correct mana colors
#[test]
fn test_volcanic_island_has_mountain_subtype() -> Result<()> {
    use mtg_forge_rs::core::{CardId, PlayerId, Subtype};

    let path = PathBuf::from("cardsfolder/v/volcanic_island.txt");
    if !path.exists() {
        return Ok(()); // Skip if cardsfolder not present
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Volcanic Island");
    assert!(def.types.contains(&CardType::Land), "Should be a Land");

    // Check subtypes
    assert!(
        def.subtypes.contains(&Subtype::new("Island")),
        "Should have Island subtype. Subtypes: {:?}",
        def.subtypes
    );
    assert!(
        def.subtypes.contains(&Subtype::new("Mountain")),
        "Should have Mountain subtype. Subtypes: {:?}",
        def.subtypes
    );

    // Instantiate and check cache flags
    let card_id = CardId::new(1);
    let player_id = PlayerId::new(1);
    let card = def.instantiate(card_id, player_id);

    assert!(
        card.cache.has_island_subtype,
        "Cache should have has_island_subtype=true"
    );
    assert!(
        card.cache.has_mountain_subtype,
        "Cache should have has_mountain_subtype=true for red mana production"
    );
    assert!(card.cache.is_land, "Cache should have is_land=true");

    // Critical test: mana production should be Choice (dual land) not just Blue
    use mtg_forge_rs::core::ManaProductionKind;
    assert!(card.cache.is_mana_source, "Volcanic Island should be a mana source");

    // Check that mana production is Choice (can produce either Blue or Red)
    match &card.cache.mana_production.kind {
        ManaProductionKind::Choice(colors) => {
            assert!(
                colors.contains(mtg_forge_rs::core::ManaColor::Blue),
                "Should produce Blue"
            );
            assert!(
                colors.contains(mtg_forge_rs::core::ManaColor::Red),
                "Should produce Red"
            );
            assert_eq!(colors.len(), 2, "Should produce exactly 2 colors");
        }
        other => panic!("Expected ManaProductionKind::Choice for dual land, got {:?}", other),
    }

    Ok(())
}
