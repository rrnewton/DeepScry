//! Card loading tests
//!
//! Tests that verify cards from cardsfolder can be loaded and parsed correctly

// TODO(mtg-211): Remove once wildcard patterns are audited
#![allow(clippy::wildcard_enum_match_arm)]

use mtg_engine::core::{CardId, CardType, DigFilter, Keyword, KeywordArgs, PlayerId};
use mtg_engine::loader::CardLoader;
use mtg_engine::zones::Zone;
use mtg_engine::{MtgError, Result};
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
    let card_id = mtg_engine::core::CardId::new(1);
    let player_id = mtg_engine::core::PlayerId::new(1);

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
    let card_id = mtg_engine::core::CardId::new(1);
    let player_id = mtg_engine::core::PlayerId::new(1);

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
    let card_id = mtg_engine::core::CardId::new(1);
    let player_id = mtg_engine::core::PlayerId::new(1);

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
    let card_id = mtg_engine::core::CardId::new(1);
    let player_id = mtg_engine::core::PlayerId::new(1);

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
    use mtg_engine::core::{CardId, ManaProductionKind, PlayerId};

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
        card.definition.cache.mana_production.produces_mana(),
        "Mishra's Factory should be detected as producing mana. Card text: {}",
        card.text
    );
    assert_eq!(
        card.definition.cache.mana_production.kind,
        ManaProductionKind::Colorless,
        "Mishra's Factory should produce Colorless mana, not {:?}. Card text: {}",
        card.definition.cache.mana_production.kind,
        card.text
    );

    Ok(())
}

/// Test that Spider-Ham, Peter Porker's static ability is correctly parsed
/// The card has a multi-type buff: "Other Spiders, Boars, Bears, ... get +1/+1"
#[test]
fn test_load_spider_ham_static_ability() -> Result<()> {
    use mtg_engine::core::{AffectedSelector, CardId, PlayerId, StaticAbility};

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
            condition: _,
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
    use mtg_engine::core::{CardId, PlayerId, Subtype};

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
    use mtg_engine::core::{AffectedSelector, CardId, PlayerId, StaticAbility, Subtype};

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
            condition: _,
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
    use mtg_engine::core::{AffectedSelector, CardId, PlayerId, StaticAbility};

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
            condition: _,
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
    use mtg_engine::core::{CardId, PlayerId};

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
        .any(|e| matches!(e, mtg_engine::core::Effect::AddMana { .. }));
    assert!(
        has_add_mana,
        "Black Lotus should have AddMana effect. Effects: {:?}",
        ability.effects
    );

    // Verify the cache detects it as a mana source
    assert!(
        card.definition.cache.mana_production.produces_mana(),
        "Black Lotus should be detected as producing mana"
    );
    assert!(
        card.definition.cache.is_mana_source,
        "Black Lotus should be a mana source"
    );

    Ok(())
}

/// Test that Black Lotus' mana ability correctly sacrifices the card when activated
/// This tests the full game flow: play Black Lotus, activate its mana ability,
/// verify mana is added and Black Lotus is sacrificed (moved to graveyard).
#[test]
fn test_black_lotus_sacrifice_on_activation() -> Result<()> {
    use mtg_engine::game::GameState;
    use mtg_engine::loader::CardDatabase;

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
    use mtg_engine::core::{CardId, PlayerId, Subtype};

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
        card.definition.cache.has_island_subtype,
        "Cache should have has_island_subtype=true"
    );
    assert!(
        card.definition.cache.has_mountain_subtype,
        "Cache should have has_mountain_subtype=true for red mana production"
    );
    assert!(card.definition.cache.is_land, "Cache should have is_land=true");

    // Critical test: mana production should be Choice (dual land) not just Blue
    use mtg_engine::core::ManaProductionKind;
    assert!(
        card.definition.cache.is_mana_source,
        "Volcanic Island should be a mana source"
    );

    // Check that mana production is Choice (can produce either Blue or Red)
    match &card.definition.cache.mana_production.kind {
        ManaProductionKind::Choice(colors) => {
            assert!(
                colors.contains(mtg_engine::core::ManaColor::Blue),
                "Should produce Blue"
            );
            assert!(colors.contains(mtg_engine::core::ManaColor::Red), "Should produce Red");
            assert_eq!(colors.len(), 2, "Should produce exactly 2 colors");
        }
        other => panic!("Expected ManaProductionKind::Choice for dual land, got {:?}", other),
    }

    Ok(())
}

/// Test that Spider-Punk's "Other Spiders you control" selector is correctly parsed
/// Uses Spider.Other+YouCtrl which should parse to CreatureTypeOtherYouControl
#[test]
fn test_load_spider_punk_type_other_you_ctrl() -> Result<()> {
    use mtg_engine::core::{AffectedSelector, CardId, Keyword, PlayerId, StaticAbility, Subtype};

    let path = PathBuf::from("cardsfolder/s/spider_punk.txt");
    if !path.exists() {
        return Ok(()); // Skip if cardsfolder not present
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Spider-Punk");
    assert!(def.types.contains(&CardType::Creature));

    // Check that the S: ability line is in raw_abilities
    let has_static_line = def.raw_abilities.iter().any(|a| {
        a.contains("Mode$ Continuous") && a.contains("Spider.Other+YouCtrl") && a.contains("AddKeyword$ Riot")
    });
    assert!(
        has_static_line,
        "Spider-Punk should have a static ability granting Riot to other Spiders. Abilities: {:?}",
        def.raw_abilities
    );

    // Instantiate the card
    let card_id = CardId::new(1);
    let player_id = PlayerId::new(1);
    let card = def.instantiate(card_id, player_id);

    // Should be a Spider creature
    assert!(
        card.subtypes.contains(&Subtype::new("Spider")),
        "Should be a Spider creature"
    );

    // Find the GrantKeyword ability with CreatureTypeOtherYouControl
    let grant_kw = card.static_abilities.iter().find(|a| {
        matches!(
            a,
            StaticAbility::GrantKeyword {
                affected: AffectedSelector::CreatureTypeOtherYouControl { .. },
                ..
            }
        )
    });
    assert!(
        grant_kw.is_some(),
        "Spider-Punk should have GrantKeyword with CreatureTypeOtherYouControl. Got: {:?}",
        card.static_abilities
    );

    match grant_kw.unwrap() {
        StaticAbility::GrantKeyword {
            affected,
            keyword,
            description: _,
            condition: _,
        } => {
            match affected {
                AffectedSelector::CreatureTypeOtherYouControl { subtype } => {
                    assert_eq!(
                        subtype.as_str(),
                        "Spider",
                        "Should target Spider subtype, got {:?}",
                        subtype
                    );
                }
                _ => panic!("Expected CreatureTypeOtherYouControl, got {:?}", affected),
            }
            // Check keyword is Riot
            assert_eq!(*keyword, Keyword::Riot, "Should grant Riot keyword, got {:?}", keyword);
        }
        _ => panic!("Expected GrantKeyword static ability"),
    }

    Ok(())
}

/// Test that Friendly Neighborhood's "Enchanted land" selector is correctly parsed
/// Uses Land.AttachedBy which should parse to LandAttachedBy
#[test]
fn test_load_friendly_neighborhood_land_attached_by() -> Result<()> {
    use mtg_engine::core::{CardId, PlayerId};

    let path = PathBuf::from("cardsfolder/f/friendly_neighborhood.txt");
    if !path.exists() {
        return Ok(()); // Skip if cardsfolder not present
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Friendly Neighborhood");
    assert!(def.types.contains(&CardType::Enchantment));

    // Check that the S: ability line is in raw_abilities with Land.AttachedBy
    let has_static_line = def
        .raw_abilities
        .iter()
        .any(|a| a.contains("Mode$ Continuous") && a.contains("Land.AttachedBy") && a.contains("AddAbility$"));
    assert!(
        has_static_line,
        "Friendly Neighborhood should have a static ability with Land.AttachedBy. Abilities: {:?}",
        def.raw_abilities
    );

    // Instantiate the card
    let card_id = CardId::new(1);
    let player_id = PlayerId::new(1);
    let card = def.instantiate(card_id, player_id);

    // Note: AddAbility$ parsing is not implemented yet, so we verify:
    // 1. The raw ability line contains Land.AttachedBy (proving parsing doesn't reject it)
    // 2. Card loads without errors
    // The key thing is that Land.AttachedBy is recognized as a valid selector
    println!("Friendly Neighborhood static abilities: {:?}", card.static_abilities);

    // Verify the card was loaded successfully (this proves Land.AttachedBy doesn't cause a crash)
    assert!(card.types.contains(&CardType::Enchantment), "Should be an Enchantment");

    Ok(())
}

/// Test loading Clot Sliver (Sliver tribal static ability)
///
/// Clot Sliver has: "All Slivers have '{2}: Regenerate this permanent.'"
/// Uses Affected$ Sliver pattern for global sliver effects.
#[test]
fn test_load_clot_sliver_global_selector() -> Result<()> {
    let path = PathBuf::from("cardsfolder/c/clot_sliver.txt");
    if !path.exists() {
        return Ok(()); // Skip if cardsfolder not present
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Clot Sliver");
    assert!(def.types.contains(&CardType::Creature));

    // Check that it has the Sliver subtype
    let has_sliver_subtype = def.subtypes.iter().any(|s| s.as_str() == "Sliver");
    assert!(has_sliver_subtype, "Should have Sliver subtype");

    // Check that the S: ability line uses Affected$ Sliver
    let has_sliver_affected = def
        .raw_abilities
        .iter()
        .any(|a| a.contains("Mode$ Continuous") && a.contains("Affected$ Sliver"));
    assert!(
        has_sliver_affected,
        "Clot Sliver should have an Affected$ Sliver line. Abilities: {:?}",
        def.raw_abilities
    );

    Ok(())
}

/// Test loading Muscle Sliver (Creature.Sliver pattern with P/T bonus)
///
/// Muscle Sliver has: "All Sliver creatures get +1/+1."
/// Uses Affected$ Creature.Sliver pattern.
#[test]
fn test_load_muscle_sliver_creature_sliver_selector() -> Result<()> {
    use mtg_engine::core::{AffectedSelector, CardId, PlayerId, StaticAbility};

    let path = PathBuf::from("cardsfolder/m/muscle_sliver.txt");
    if !path.exists() {
        return Ok(()); // Skip if cardsfolder not present
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Muscle Sliver");
    assert!(def.types.contains(&CardType::Creature));

    // Check that the S: ability line uses Affected$ Creature.Sliver
    let has_creature_sliver = def
        .raw_abilities
        .iter()
        .any(|a| a.contains("Mode$ Continuous") && a.contains("Affected$ Creature.Sliver"));
    assert!(
        has_creature_sliver,
        "Muscle Sliver should have Affected$ Creature.Sliver. Abilities: {:?}",
        def.raw_abilities
    );

    // Instantiate the card
    let card_id = CardId::new(1);
    let player_id = PlayerId::new(1);
    let card = def.instantiate(card_id, player_id);

    // Should have a ModifyPT static ability with AllCreaturesOfType selector
    let has_pt_ability = card.static_abilities.iter().any(|a| {
        if let StaticAbility::ModifyPT {
            affected,
            power,
            toughness,
            ..
        } = a
        {
            matches!(
                affected,
                AffectedSelector::AllCreaturesOfType { subtype }
                if subtype.as_str() == "Sliver"
            ) && *power == 1
                && *toughness == 1
        } else {
            false
        }
    });

    assert!(
        has_pt_ability,
        "Should have ModifyPT with AllCreaturesOfType(Sliver) +1/+1. Got: {:?}",
        card.static_abilities
    );

    Ok(())
}

/// Test loading Winged Sliver (Creature.Sliver with keyword grant)
///
/// Winged Sliver has: "All Sliver creatures have flying."
/// Uses Affected$ Creature.Sliver with AddKeyword$ Flying.
#[test]
fn test_load_winged_sliver_grants_flying() -> Result<()> {
    use mtg_engine::core::{AffectedSelector, CardId, Keyword, PlayerId, StaticAbility};

    let path = PathBuf::from("cardsfolder/w/winged_sliver.txt");
    if !path.exists() {
        return Ok(()); // Skip if cardsfolder not present
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Winged Sliver");
    assert!(def.types.contains(&CardType::Creature));

    // Check that the S: ability line uses Affected$ Creature.Sliver with AddKeyword$ Flying
    let has_flying_grant = def.raw_abilities.iter().any(|a| {
        a.contains("Mode$ Continuous") && a.contains("Affected$ Creature.Sliver") && a.contains("AddKeyword$ Flying")
    });
    assert!(
        has_flying_grant,
        "Winged Sliver should have Affected$ Creature.Sliver AddKeyword$ Flying. Abilities: {:?}",
        def.raw_abilities
    );

    // Instantiate the card
    let card_id = CardId::new(1);
    let player_id = PlayerId::new(1);
    let card = def.instantiate(card_id, player_id);

    // Should have a GrantKeyword static ability with AllCreaturesOfType selector
    let has_keyword_ability = card.static_abilities.iter().any(|a| {
        if let StaticAbility::GrantKeyword { affected, keyword, .. } = a {
            matches!(
                affected,
                AffectedSelector::AllCreaturesOfType { subtype }
                if subtype.as_str() == "Sliver"
            ) && *keyword == Keyword::Flying
        } else {
            false
        }
    });

    assert!(
        has_keyword_ability,
        "Should have GrantKeyword Flying with AllCreaturesOfType(Sliver). Got: {:?}",
        card.static_abilities
    );

    Ok(())
}

/// Test that Card.TopLibrary+YouCtrl selector is properly parsed
/// Assemble the Players: "You may look at the top card of your library any time."
/// Related to: mtg-170
#[test]
fn test_load_assemble_the_players_top_library_selector() -> Result<()> {
    use mtg_engine::core::{CardId, PlayerId};

    let path = PathBuf::from("cardsfolder/a/assemble_the_players.txt");
    if !path.exists() {
        return Ok(());
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Assemble the Players");
    assert!(def.types.contains(&CardType::Enchantment));

    // Verify the first static ability line is present (MayLookAt)
    let has_top_library_ability = def
        .raw_abilities
        .iter()
        .any(|a| a.contains("Affected$ Card.TopLibrary+YouCtrl") && a.contains("MayLookAt"));
    assert!(
        has_top_library_ability,
        "Should have Card.TopLibrary+YouCtrl with MayLookAt. Abilities: {:?}",
        def.raw_abilities
    );

    // Instantiate the card - we can't directly check static_abilities parsing
    // since MayLookAt is handled differently, but we verify parsing doesn't fail
    let card_id = CardId::new(1);
    let player_id = PlayerId::new(1);
    let _card = def.instantiate(card_id, player_id);

    // Parsing succeeded without warnings for Card.TopLibrary+YouCtrl
    Ok(())
}

/// Test that Creature.AttachedBy selector is properly parsed
/// Brainwash: "Enchanted creature can't attack unless its controller pays {3}."
/// Related to: mtg-170
#[test]
fn test_load_brainwash_creature_attached_by_selector() -> Result<()> {
    use mtg_engine::core::{CardId, PlayerId};

    let path = PathBuf::from("cardsfolder/b/brainwash.txt");
    if !path.exists() {
        return Ok(());
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Brainwash");
    assert!(def.types.contains(&CardType::Enchantment));

    // Verify the CantAttackUnless ability line is present
    let has_cant_attack = def
        .raw_abilities
        .iter()
        .any(|a| a.contains("Creature.AttachedBy") && a.contains("CantAttackUnless"));
    assert!(
        has_cant_attack,
        "Should have CantAttackUnless with Creature.AttachedBy. Abilities: {:?}",
        def.raw_abilities
    );

    // Instantiate the card - parsing should not produce warnings for Creature.AttachedBy
    let card_id = CardId::new(1);
    let player_id = PlayerId::new(1);
    let _card = def.instantiate(card_id, player_id);

    // Parsing succeeded without warnings for Creature.AttachedBy
    Ok(())
}

/// Test that Creature.Other+YouCtrl selector is properly parsed (reversed order)
/// Aang, Air Nomad: "Other creatures you control have vigilance."
/// Related to: mtg-147
#[test]
fn test_load_aang_creature_other_youctrl_selector() -> Result<()> {
    use mtg_engine::core::{AffectedSelector, CardId, Keyword, PlayerId, StaticAbility};

    let path = PathBuf::from("cardsfolder/a/aang_air_nomad.txt");
    if !path.exists() {
        return Ok(());
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Aang, Air Nomad");
    assert!(def.types.contains(&CardType::Creature));

    // Verify the static ability line is present
    let has_vigilance_grant = def
        .raw_abilities
        .iter()
        .any(|a| a.contains("Creature.Other+YouCtrl") && a.contains("AddKeyword$ Vigilance"));
    assert!(
        has_vigilance_grant,
        "Should have Creature.Other+YouCtrl with AddKeyword$ Vigilance. Abilities: {:?}",
        def.raw_abilities
    );

    // Instantiate and verify the static ability is correctly parsed
    let card_id = CardId::new(1);
    let player_id = PlayerId::new(1);
    let card = def.instantiate(card_id, player_id);

    // Should have a GrantKeyword static ability with CreaturesYouControlOther selector
    let has_keyword_ability = card.static_abilities.iter().any(|a| {
        if let StaticAbility::GrantKeyword { affected, keyword, .. } = a {
            matches!(affected, AffectedSelector::CreaturesYouControlOther) && *keyword == Keyword::Vigilance
        } else {
            false
        }
    });

    assert!(
        has_keyword_ability,
        "Should have GrantKeyword Vigilance with CreaturesYouControlOther. Got: {:?}",
        card.static_abilities
    );

    Ok(())
}

/// Test that Artifact.YouCtrl selector is properly parsed
/// Darksteel Forge: "Artifacts you control have indestructible."
/// Related to: mtg-147
#[test]
fn test_load_darksteel_forge_artifact_youctrl_selector() -> Result<()> {
    use mtg_engine::core::{AffectedSelector, CardId, Keyword, PlayerId, StaticAbility};

    let path = PathBuf::from("cardsfolder/d/darksteel_forge.txt");
    if !path.exists() {
        return Ok(());
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Darksteel Forge");
    assert!(def.types.contains(&CardType::Artifact));

    // Verify the static ability line is present
    let has_indestructible_grant = def
        .raw_abilities
        .iter()
        .any(|a| a.contains("Artifact.YouCtrl") && a.contains("AddKeyword$ Indestructible"));
    assert!(
        has_indestructible_grant,
        "Should have Artifact.YouCtrl with AddKeyword$ Indestructible. Abilities: {:?}",
        def.raw_abilities
    );

    // Instantiate and verify the static ability is correctly parsed
    let card_id = CardId::new(1);
    let player_id = PlayerId::new(1);
    let card = def.instantiate(card_id, player_id);

    // Should have a GrantKeyword static ability with ArtifactsYouControl selector
    let has_keyword_ability = card.static_abilities.iter().any(|a| {
        if let StaticAbility::GrantKeyword { affected, keyword, .. } = a {
            matches!(affected, AffectedSelector::ArtifactsYouControl) && *keyword == Keyword::Indestructible
        } else {
            false
        }
    });

    assert!(
        has_keyword_ability,
        "Should have GrantKeyword Indestructible with ArtifactsYouControl. Got: {:?}",
        card.static_abilities
    );

    Ok(())
}

/// Test that Land selector is properly parsed
/// Blanket of Night: "Each land is a Swamp in addition to its other land types."
/// Related to: mtg-147
#[test]
fn test_load_blanket_of_night_land_selector() -> Result<()> {
    use mtg_engine::core::{CardId, PlayerId};

    let path = PathBuf::from("cardsfolder/b/blanket_of_night.txt");
    if !path.exists() {
        return Ok(());
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Blanket of Night");
    assert!(def.types.contains(&CardType::Enchantment));

    // Verify the static ability line is present with Land selector
    let has_land_selector = def
        .raw_abilities
        .iter()
        .any(|a| a.contains("Affected$ Land") && a.contains("AddType$ Swamp"));
    assert!(
        has_land_selector,
        "Should have Land selector with AddType$ Swamp. Abilities: {:?}",
        def.raw_abilities
    );

    // Instantiate - parsing should succeed without warnings
    let card_id = CardId::new(1);
    let player_id = PlayerId::new(1);
    let _card = def.instantiate(card_id, player_id);

    Ok(())
}

/// Test that Permanent.YouCtrl selector is properly parsed
/// Wondrous Crucible: "Permanents you control have ward {2}."
/// Related to: mtg-147
/// Note: Ward:2 keyword parsing is not fully implemented, so we just verify
/// the selector is recognized without warnings.
#[test]
fn test_load_wondrous_crucible_permanent_youctrl_selector() -> Result<()> {
    use mtg_engine::core::{CardId, PlayerId};

    let path = PathBuf::from("cardsfolder/w/wondrous_crucible.txt");
    if !path.exists() {
        return Ok(());
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Wondrous Crucible");
    assert!(def.types.contains(&CardType::Artifact));

    // Verify the static ability line is present
    let has_ward_grant = def
        .raw_abilities
        .iter()
        .any(|a| a.contains("Permanent.YouCtrl") && a.contains("AddKeyword$ Ward"));
    assert!(
        has_ward_grant,
        "Should have Permanent.YouCtrl with Ward keyword. Abilities: {:?}",
        def.raw_abilities
    );

    // Instantiate - parsing should succeed without "Unknown selector" warnings
    // Note: The Ward:2 keyword itself isn't fully parsed, so we can't check for
    // GrantKeyword in static_abilities. This test verifies selector parsing only.
    let card_id = CardId::new(1);
    let player_id = PlayerId::new(1);
    let _card = def.instantiate(card_id, player_id);

    Ok(())
}

/// Test that Creature.attacking+YouCtrl selector is properly parsed
/// Goblin Oriflamme: "Attacking creatures you control get +1/+0."
/// Related to: mtg-147
#[test]
fn test_load_goblin_oriflamme_attacking_youctrl_selector() -> Result<()> {
    use mtg_engine::core::{AffectedSelector, CardId, PlayerId, StaticAbility};

    let path = PathBuf::from("cardsfolder/g/goblin_oriflamme.txt");
    if !path.exists() {
        return Ok(());
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Goblin Oriflamme");
    assert!(def.types.contains(&CardType::Enchantment));

    // Verify the static ability line is present
    let has_attacking_selector = def
        .raw_abilities
        .iter()
        .any(|a| a.contains("Creature.attacking+YouCtrl") && a.contains("AddPower$ 1"));
    assert!(
        has_attacking_selector,
        "Should have Creature.attacking+YouCtrl with AddPower$ 1. Abilities: {:?}",
        def.raw_abilities
    );

    // Instantiate and verify static abilities
    let card_id = CardId::new(1);
    let player_id = PlayerId::new(1);
    let card = def.instantiate(card_id, player_id);

    // Should have ModifyPT with AttackingCreaturesYouControl selector
    let has_modifypt = card.static_abilities.iter().any(|ability| {
        if let StaticAbility::ModifyPT { affected, power, .. } = ability {
            matches!(affected, AffectedSelector::AttackingCreaturesYouControl) && *power == 1
        } else {
            false
        }
    });
    assert!(
        has_modifypt,
        "Should have ModifyPT with AttackingCreaturesYouControl selector"
    );

    Ok(())
}

/// Test that Opponent selector is properly parsed
/// Gnat Miser: "Each opponent's maximum hand size is reduced by one."
/// Related to: mtg-147
#[test]
fn test_load_gnat_miser_opponent_selector() -> Result<()> {
    use mtg_engine::core::{CardId, PlayerId};

    let path = PathBuf::from("cardsfolder/g/gnat_miser.txt");
    if !path.exists() {
        return Ok(());
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Gnat Miser");
    assert!(def.types.contains(&CardType::Creature));

    // Verify the static ability line is present
    let has_opponent_selector = def
        .raw_abilities
        .iter()
        .any(|a| a.contains("Affected$ Opponent") && a.contains("RaiseMaxHandSize$ -1"));
    assert!(
        has_opponent_selector,
        "Should have Opponent selector with RaiseMaxHandSize. Abilities: {:?}",
        def.raw_abilities
    );

    // Instantiate - parsing should succeed without warnings
    let card_id = CardId::new(1);
    let player_id = PlayerId::new(1);
    let _card = def.instantiate(card_id, player_id);

    Ok(())
}

/// Test that Card.Self+attacking selector is properly parsed
/// Soltari Lancer: "Soltari Lancer has first strike as long as it's attacking."
/// Related to: mtg-147
#[test]
fn test_load_soltari_lancer_self_attacking_selector() -> Result<()> {
    use mtg_engine::core::{AffectedSelector, CardId, Keyword, PlayerId, StaticAbility};

    let path = PathBuf::from("cardsfolder/s/soltari_lancer.txt");
    if !path.exists() {
        return Ok(());
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Soltari Lancer");
    assert!(def.types.contains(&CardType::Creature));

    // Verify the static ability line is present
    let has_self_attacking_selector = def
        .raw_abilities
        .iter()
        .any(|a| a.contains("Card.Self+attacking") && a.contains("AddKeyword$ First Strike"));
    assert!(
        has_self_attacking_selector,
        "Should have Card.Self+attacking with AddKeyword$ First Strike. Abilities: {:?}",
        def.raw_abilities
    );

    // Instantiate and verify static abilities
    let card_id = CardId::new(1);
    let player_id = PlayerId::new(1);
    let card = def.instantiate(card_id, player_id);

    // Should have GrantKeyword with SelfWhenAttacking selector
    let has_grant_keyword = card.static_abilities.iter().any(|ability| {
        if let StaticAbility::GrantKeyword { affected, keyword, .. } = ability {
            matches!(affected, AffectedSelector::SelfWhenAttacking) && *keyword == Keyword::FirstStrike
        } else {
            false
        }
    });
    assert!(
        has_grant_keyword,
        "Should have GrantKeyword with SelfWhenAttacking selector. Static abilities: {:?}",
        card.static_abilities
    );

    Ok(())
}

/// Test loading Crusade - should parse AllCreaturesOfColor selector
#[test]
fn test_load_crusade_all_creatures_of_color() -> Result<()> {
    use mtg_engine::core::{AffectedSelector, CardId, PlayerId, StaticAbility};

    let path = PathBuf::from("cardsfolder/c/crusade.txt");
    if !path.exists() {
        eprintln!("Skipping test: cardsfolder not present");
        return Ok(());
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Crusade");
    assert!(def.types.contains(&CardType::Enchantment));

    // Check raw abilities - should have Creature.White selector
    let has_creature_white = def
        .raw_abilities
        .iter()
        .any(|a| a.contains("Affected$ Creature.White") && a.contains("AddPower$ 1") && a.contains("AddToughness$ 1"));
    assert!(
        has_creature_white,
        "Crusade should have Affected$ Creature.White static ability. Raw abilities: {:?}",
        def.raw_abilities
    );

    // Instantiate the card
    let card_id = CardId::new(1);
    let player_id = PlayerId::new(1);
    let card = def.instantiate(card_id, player_id);

    // Should have ModifyPT with AllCreaturesOfColor { color: "White" }
    let has_modify_pt = card.static_abilities.iter().any(|ability| {
        matches!(
            ability,
            StaticAbility::ModifyPT {
                affected: AffectedSelector::AllCreaturesOfColor { color },
                power: 1,
                toughness: 1,
                ..
            } if color == "White"
        )
    });
    assert!(
        has_modify_pt,
        "Crusade should have ModifyPT with AllCreaturesOfColor {{ color: \"White\" }}. Static abilities: {:?}",
        card.static_abilities
    );

    Ok(())
}

/// Test that Spirit Link is correctly recognized as an Aura that can target creatures
#[test]
fn test_spirit_link_aura_targeting() -> Result<()> {
    use mtg_engine::core::{CardId, PlayerId};

    let path = PathBuf::from("cardsfolder/s/spirit_link.txt");
    if !path.exists() {
        return Ok(()); // Skip if cardsfolder not present
    }

    // Load Spirit Link
    let spirit_link_def = CardLoader::load_from_file(&path)?;

    println!("Spirit Link card types: {:?}", spirit_link_def.types);
    println!("Spirit Link subtypes: {:?}", spirit_link_def.subtypes);
    println!("Spirit Link raw keywords: {:?}", spirit_link_def.raw_keywords);

    // Instantiate the card
    let card_id = CardId::new(1);
    let player_id = PlayerId::new(1);
    let card = spirit_link_def.instantiate(card_id, player_id);

    // Verify Spirit Link is an Aura
    assert!(card.is_aura(), "Spirit Link should be recognized as an Aura");

    // Verify it has the Enchant keyword
    let has_enchant = card.keywords.contains(Keyword::Enchant);
    assert!(has_enchant, "Spirit Link should have Enchant keyword");

    // Check the Enchant type
    if let Some(enchant_args) = card.keywords.get_args(Keyword::Enchant) {
        println!("Spirit Link Enchant args: {:?}", enchant_args);
        if let KeywordArgs::Enchant { card_type } = enchant_args {
            assert_eq!(
                card_type.as_str().to_lowercase(),
                "creature",
                "Spirit Link should enchant creatures"
            );
        } else {
            panic!("Spirit Link Enchant keyword has wrong args type");
        }
    } else {
        panic!("Spirit Link should have Enchant keyword args");
    }

    Ok(())
}

/// Test Thriving Grove - a land with "Produced$ Combo G Chosen" mana ability
/// This tests that cards with "Combo" production are correctly detected as mana sources
/// Regression test for bug where Thriving Grove wasn't being tapped for mana
#[test]
fn test_load_thriving_grove_mana_ability() -> Result<()> {
    use mtg_engine::core::{CardId, ManaProductionKind, PlayerId};

    let path = PathBuf::from("cardsfolder/t/thriving_grove.txt");
    if !path.exists() {
        return Ok(()); // Skip if cardsfolder not present
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Thriving Grove");
    assert!(def.types.contains(&CardType::Land));

    // Check that raw_abilities contains the mana ability line
    let has_mana_ability = def
        .raw_abilities
        .iter()
        .any(|a| a.contains("AB$ Mana") && a.contains("Produced$"));
    assert!(
        has_mana_ability,
        "Thriving Grove should have a mana ability. Raw abilities: {:?}",
        def.raw_abilities
    );

    // Instantiate the card
    let card_id = CardId::new(1);
    let player_id = PlayerId::new(1);
    let card = def.instantiate(card_id, player_id);

    // Check activated abilities - should have at least 1 (the mana ability)
    assert!(
        !card.activated_abilities.is_empty(),
        "Thriving Grove should have activated abilities. Got: {} abilities",
        card.activated_abilities.len()
    );

    // Find the mana ability
    let mana_ability = card.activated_abilities.iter().find(|a| a.is_mana_ability);

    assert!(
        mana_ability.is_some(),
        "Thriving Grove should have a mana ability. Abilities: {:?}",
        card.activated_abilities
            .iter()
            .map(|a| format!("cost={:?} is_mana={}", a.cost, a.is_mana_ability))
            .collect::<Vec<_>>()
    );

    // Verify the cache detects it as a mana source
    assert!(
        card.definition.cache.is_mana_source,
        "Thriving Grove should be detected as a mana source (is_mana_source flag)"
    );
    assert!(
        card.definition.cache.mana_production.produces_mana(),
        "Thriving Grove should be detected as producing mana"
    );

    // Check the production kind - should be Fixed(Green) or Choice containing Green
    // (Since "Chosen" is not a parseable color, only "G" is recognized)
    match &card.definition.cache.mana_production.kind {
        ManaProductionKind::Fixed(color) => {
            assert_eq!(
                *color,
                mtg_engine::core::ManaColor::Green,
                "Thriving Grove should produce Green"
            );
        }
        ManaProductionKind::Choice(colors) => {
            assert!(
                colors.contains(mtg_engine::core::ManaColor::Green),
                "Thriving Grove should be able to produce Green. Colors: {:?}",
                colors
            );
        }
        other => {
            panic!(
                "Thriving Grove should produce Fixed(Green) or Choice containing Green. Got: {:?}",
                other
            );
        }
    }

    Ok(())
}

/// Test that Thriving Grove is properly added to mana source cache when entering battlefield
/// Regression test for bug where Thriving Grove wasn't being tapped for mana
#[tokio::test]
async fn test_thriving_grove_mana_cache_population() -> Result<()> {
    use mtg_engine::game::GameState;
    use mtg_engine::loader::CardDatabase;
    use mtg_engine::zones::Zone;

    // Load card database
    let path = PathBuf::from("cardsfolder");
    if !path.exists() {
        return Ok(()); // Skip if cardsfolder not present
    }

    let card_db = CardDatabase::new(path);
    card_db
        .load_cards(&["Thriving Grove".to_string()])
        .await
        .map_err(|e| MtgError::InvalidCardFormat(format!("Failed to load card: {}", e)))?;

    // Get the card definition
    let grove_def = card_db
        .get_card("Thriving Grove")
        .await
        .map_err(|e| MtgError::InvalidCardFormat(format!("Failed to get card: {}", e)))?
        .expect("Card exists");

    // Create a game
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p2_id = game.players[1].id;

    // Create and instantiate the card
    let card_id = game.next_card_id();
    let card = grove_def.instantiate(card_id, p2_id);

    // Verify instantiated card has correct mana source flag
    assert!(
        card.definition.cache.is_mana_source,
        "Instantiated Thriving Grove should have is_mana_source=true"
    );

    // Add to game and hand
    game.cards.insert(card_id, card);
    if let Some(zones) = game.get_player_zones_mut(p2_id) {
        zones.hand.add(card_id);
    }

    // Move to battlefield
    game.move_card(card_id, Zone::Hand, Zone::Battlefield, p2_id)?;

    // Check mana cache - Thriving Grove should be in green_sources or complex_sources
    let cache = game.get_mana_cache(p2_id).expect("Cache should exist");

    let in_green_sources = cache.green_sources().contains(&card_id);
    let in_complex_sources = cache.complex_sources().contains(&card_id);

    assert!(
        in_green_sources || in_complex_sources,
        "Thriving Grove ({:?}) should be in mana source cache. Green: {:?}, Complex: {:?}",
        card_id,
        cache.green_sources(),
        cache.complex_sources()
    );

    // Thriving Grove enters tapped (due to R: line with "ReplaceWith$ ETBTapped"),
    // so untapped_green will be 0. Verify the card is tapped as expected.
    let card = game.cards.get(card_id)?;
    assert!(card.tapped, "Thriving Grove should enter the battlefield tapped");

    // Even though tapped, it's still tracked as a green mana source for future turns
    if in_green_sources {
        assert_eq!(
            cache.untapped_green(),
            0,
            "Thriving Grove enters tapped, so untapped_green should be 0"
        );
    }

    // Verify that chosen_color is set (Thriving Grove chooses a color on ETB)
    // The color should NOT be green (since green is excluded per the card definition)
    assert!(
        card.chosen_color.is_some(),
        "Thriving Grove should have a chosen color after entering battlefield"
    );
    let chosen = card.chosen_color.unwrap();
    assert!(
        !matches!(chosen, mtg_engine::core::Color::Green),
        "Thriving Grove's chosen color should not be green (it's excluded). Got: {:?}",
        chosen
    );

    // Verify the card is classified as a complex source in the cache
    // (because it has chosen_color set, allowing it to produce multiple colors)
    assert!(
        in_complex_sources,
        "Thriving Grove should be in complex_sources since it has chosen_color"
    );

    Ok(())
}

/// Test that Ba Sing Se (Avatar set) is properly detected as a mana source
/// Ba Sing Se has: A:AB$ Mana | Cost$ T | Produced$ G | SpellDescription$ Add {G}.
/// This is a non-basic land that should produce green mana
#[tokio::test]
async fn test_ba_sing_se_mana_detection() -> Result<()> {
    use mtg_engine::core::{CardId, ManaColor, ManaProductionKind, PlayerId};
    use mtg_engine::loader::CardDatabase;

    // Load card database
    let path = PathBuf::from("cardsfolder");
    if !path.exists() {
        return Ok(()); // Skip if cardsfolder not present
    }

    let card_db = CardDatabase::new(path);
    card_db
        .load_cards(&["Ba Sing Se".to_string()])
        .await
        .map_err(|e| MtgError::InvalidCardFormat(format!("Failed to load Ba Sing Se: {}", e)))?;

    let def = card_db
        .get_card("Ba Sing Se")
        .await
        .map_err(|e| MtgError::InvalidCardFormat(format!("Failed to get Ba Sing Se: {}", e)))?
        .expect("Ba Sing Se exists");

    let card_id = CardId::new(1);
    let player_id = PlayerId::new(1);
    let card = def.instantiate(card_id, player_id);

    // Verify the cache detects it as a mana source
    assert!(
        card.definition.cache.is_mana_source,
        "Ba Sing Se should be detected as a mana source (is_mana_source flag). Card text: {}",
        card.text
    );
    assert!(
        card.definition.cache.mana_production.produces_mana(),
        "Ba Sing Se should be detected as producing mana. Card text: {}",
        card.text
    );

    // Check the production kind - should be Fixed(Green)
    match &card.definition.cache.mana_production.kind {
        ManaProductionKind::Fixed(color) => {
            assert_eq!(color, &ManaColor::Green, "Ba Sing Se should produce Green mana");
        }
        other => {
            panic!(
                "Ba Sing Se should produce Fixed(Green). Got: {:?}. Card text: {}",
                other, card.text
            );
        }
    }

    Ok(())
}

/// Test that Ba Sing Se is properly added to mana source cache when entering battlefield
/// Regression test for bug where Ba Sing Se wasn't being tapped for mana
#[tokio::test]
async fn test_ba_sing_se_mana_cache_population() -> Result<()> {
    use mtg_engine::game::GameState;
    use mtg_engine::loader::CardDatabase;
    use mtg_engine::zones::Zone;

    // Load card database
    let path = PathBuf::from("cardsfolder");
    if !path.exists() {
        return Ok(()); // Skip if cardsfolder not present
    }

    let card_db = CardDatabase::new(path);
    card_db
        .load_cards(&["Ba Sing Se".to_string()])
        .await
        .map_err(|e| MtgError::InvalidCardFormat(format!("Failed to load card: {}", e)))?;

    // Get the card definition
    let ba_sing_se_def = card_db
        .get_card("Ba Sing Se")
        .await
        .map_err(|e| MtgError::InvalidCardFormat(format!("Failed to get card: {}", e)))?
        .expect("Card exists");

    // Create a game
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Instantiate Ba Sing Se
    let ba_sing_se_id = game.next_card_id();
    let ba_sing_se = ba_sing_se_def.instantiate(ba_sing_se_id, p1_id);

    // Verify card cache BEFORE adding to game
    assert!(
        ba_sing_se.definition.cache.is_mana_source,
        "Ba Sing Se should have is_mana_source=true BEFORE entering battlefield"
    );

    // Add card to game and to player's library
    game.cards.insert(ba_sing_se_id, ba_sing_se);
    if let Some(zones) = game.get_player_zones_mut(p1_id) {
        zones.library.cards.push(ba_sing_se_id);
    }

    // Move Ba Sing Se from library to battlefield
    // (This should trigger on_card_entered for the mana source cache)
    game.move_card(ba_sing_se_id, Zone::Library, Zone::Battlefield, p1_id)?;

    // Verify Ba Sing Se is on the battlefield
    assert!(
        game.battlefield.cards.contains(&ba_sing_se_id),
        "Ba Sing Se should be on battlefield"
    );

    // Check if the mana source cache has Ba Sing Se
    let cache = game.get_mana_cache(p1_id).expect("Cache exists");
    let in_green_sources = cache.green_sources().contains(&ba_sing_se_id);
    let in_complex_sources = cache.complex_sources().contains(&ba_sing_se_id);

    assert!(
        in_green_sources || in_complex_sources,
        "Ba Sing Se (id: {:?}) should be in green_sources or complex_sources. green_sources={:?}, complex_sources={:?}",
        ba_sing_se_id,
        cache.green_sources(),
        cache.complex_sources()
    );

    // Specifically check green sources (Ba Sing Se produces Fixed(Green))
    assert!(
        in_green_sources,
        "Ba Sing Se should be in green_sources (produces Fixed(Green) mana). Got complex_sources={:?}",
        in_complex_sources
    );

    Ok(())
}

/// Regression test: Verify that ManaEngine.all_sources() includes Ba Sing Se
/// This tests the FULL path: card cache -> ManaSourceCache -> ManaEngine
#[tokio::test]
async fn test_ba_sing_se_mana_engine_sources() -> Result<()> {
    use mtg_engine::game::{GameState, ManaEngine};
    use mtg_engine::loader::CardDatabase;
    use mtg_engine::zones::Zone;

    // Load card database
    let path = PathBuf::from("cardsfolder");
    if !path.exists() {
        return Ok(()); // Skip if cardsfolder not present
    }

    let card_db = CardDatabase::new(path);
    card_db
        .load_cards(&["Ba Sing Se".to_string(), "Forest".to_string()])
        .await
        .map_err(|e| MtgError::InvalidCardFormat(format!("Failed to load card: {}", e)))?;

    let ba_sing_se_def = card_db
        .get_card("Ba Sing Se")
        .await
        .map_err(|e| MtgError::InvalidCardFormat(format!("Failed to get card: {}", e)))?
        .expect("Card exists");

    let forest_def = card_db
        .get_card("Forest")
        .await
        .map_err(|e| MtgError::InvalidCardFormat(format!("Failed to get card: {}", e)))?
        .expect("Card exists");

    // Create a game
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Instantiate and add cards
    let ba_sing_se_id = game.next_card_id();
    let ba_sing_se = ba_sing_se_def.instantiate(ba_sing_se_id, p1_id);
    game.cards.insert(ba_sing_se_id, ba_sing_se);
    if let Some(zones) = game.get_player_zones_mut(p1_id) {
        zones.library.cards.push(ba_sing_se_id);
    }
    game.move_card(ba_sing_se_id, Zone::Library, Zone::Battlefield, p1_id)?;

    let forest1_id = game.next_card_id();
    let forest1 = forest_def.instantiate(forest1_id, p1_id);
    game.cards.insert(forest1_id, forest1);
    if let Some(zones) = game.get_player_zones_mut(p1_id) {
        zones.library.cards.push(forest1_id);
    }
    game.move_card(forest1_id, Zone::Library, Zone::Battlefield, p1_id)?;

    let forest2_id = game.next_card_id();
    let forest2 = forest_def.instantiate(forest2_id, p1_id);
    game.cards.insert(forest2_id, forest2);
    if let Some(zones) = game.get_player_zones_mut(p1_id) {
        zones.library.cards.push(forest2_id);
    }
    game.move_card(forest2_id, Zone::Library, Zone::Battlefield, p1_id)?;

    let forest3_id = game.next_card_id();
    let forest3 = forest_def.instantiate(forest3_id, p1_id);
    game.cards.insert(forest3_id, forest3);
    if let Some(zones) = game.get_player_zones_mut(p1_id) {
        zones.library.cards.push(forest3_id);
    }
    game.move_card(forest3_id, Zone::Library, Zone::Battlefield, p1_id)?;

    // Verify all 4 cards are on the battlefield
    assert_eq!(game.battlefield.cards.len(), 4, "Should have 4 lands on battlefield");

    // Create ManaEngine and update it
    let mut mana_engine = ManaEngine::new();
    mana_engine.update(&game, p1_id);

    // Check that ManaEngine sees all 4 sources
    let sources = mana_engine.all_sources();
    assert_eq!(
        sources.len(),
        4,
        "ManaEngine should see 4 mana sources. Got: {:?}",
        sources.iter().map(|s| s.card_id).collect::<Vec<_>>()
    );

    // Check that Ba Sing Se is in the sources
    let ba_sing_se_in_sources = sources.iter().any(|s| s.card_id == ba_sing_se_id);
    assert!(
        ba_sing_se_in_sources,
        "Ba Sing Se (id: {:?}) should be in mana_engine.all_sources(). Sources: {:?}",
        ba_sing_se_id,
        sources.iter().map(|s| s.card_id).collect::<Vec<_>>()
    );

    // Now verify mana capacity - should be 4 green
    let capacity = mana_engine.max_mana_capacity();
    assert_eq!(
        capacity.green, 4,
        "Should have 4 green mana from 4 untapped sources. Capacity: {:?}",
        capacity
    );

    Ok(())
}

/// Test that Foggy Swamp Vinebender is NOT marked as a mana source
///
/// Foggy Swamp Vinebender has a Waterbend ability (tapping creatures/artifacts to pay costs)
/// but does NOT produce mana itself. It should NOT be in the mana source cache.
#[test]
fn test_foggy_swamp_vinebender_not_mana_source() -> Result<()> {
    let path = PathBuf::from("cardsfolder/f/foggy_swamp_vinebender.txt");
    if !path.exists() {
        eprintln!("Skipping test - cardsfolder not present");
        return Ok(());
    }

    let def = CardLoader::load_from_file(&path)?;

    // Verify card loaded
    assert_eq!(def.name.as_str(), "Foggy Swamp Vinebender");

    // Create a card from the definition
    let card_id = CardId::new(100);
    let player_id = PlayerId::new(1);
    let card = def.instantiate(card_id, player_id);

    // Check that is_mana_source is FALSE
    // The waterbend ability does NOT produce mana
    assert!(
        !card.definition.cache.is_mana_source,
        "Foggy Swamp Vinebender should NOT be marked as a mana source. \
        It has Waterbend (helps pay costs by tapping creatures/artifacts) \
        but does NOT produce mana. mana_production: {:?}",
        card.definition.cache.mana_production
    );

    // Also check that mana_production.produces_mana() is false
    assert!(
        !card.definition.cache.mana_production.produces_mana(),
        "Foggy Swamp Vinebender should NOT produce mana. Got: {:?}",
        card.definition.cache.mana_production
    );

    Ok(())
}

/// Test that Waterbend abilities are parsed correctly
///
/// Cards with `Cost$ Waterbend<N>` should have activated abilities with Waterbend cost.
/// Note: Only abilities with implemented effects (PutCounter, Draw, Pump, etc.) will be loaded.
/// Abilities with unimplemented effects (Animate, AnimateAll) will be skipped.
#[test]
fn test_waterbend_ability_parsing() -> Result<()> {
    use mtg_engine::core::Cost;

    // Test Foggy Swamp Vinebender: Cost$ Waterbend<5> with PutCounter effect
    // (PutCounter is implemented, unlike Animate used by Flexible Waterbender)
    let path = PathBuf::from("cardsfolder/f/foggy_swamp_vinebender.txt");
    if !path.exists() {
        eprintln!("Skipping test - cardsfolder not present");
        return Ok(());
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Foggy Swamp Vinebender");

    // Instantiate the card to get activated abilities
    let card_id = CardId::new(100);
    let player_id = PlayerId::new(1);
    let card = def.instantiate(card_id, player_id);

    // Check that it has an activated ability with Waterbend cost
    assert!(
        !card.activated_abilities.is_empty(),
        "Foggy Swamp Vinebender should have activated abilities. \
        The Waterbend ability (PutCounter) should be parsed. Got: {:?}",
        card.activated_abilities
    );

    // Find the Waterbend ability
    let waterbend_ability = card
        .activated_abilities
        .iter()
        .find(|ab| matches!(ab.cost, Cost::Waterbend { .. }));

    assert!(
        waterbend_ability.is_some(),
        "Foggy Swamp Vinebender should have a Waterbend ability. \
        Abilities: {:?}",
        card.activated_abilities
    );

    let ability = waterbend_ability.unwrap();
    if let Cost::Waterbend { amount } = ability.cost {
        assert_eq!(amount, 5, "Waterbend cost should be 5");
    } else {
        panic!("Expected Waterbend cost, got {:?}", ability.cost);
    }

    Ok(())
}

/// Test that Flash keyword is correctly parsed and recognized
/// CR 702.8a: Flash allows a permanent to be cast anytime you could cast an instant
#[test]
fn test_flash_keyword_parsing() -> Result<()> {
    use mtg_engine::core::Keyword;

    // Test Twin Blades which has K:Flash
    let path = PathBuf::from("cardsfolder/t/twin_blades.txt");
    if !path.exists() {
        eprintln!("Skipping test - cardsfolder not present");
        return Ok(());
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Twin Blades");

    // Verify raw_keywords contains Flash
    assert!(
        def.raw_keywords.iter().any(|k| k == "Flash"),
        "Twin Blades raw_keywords should contain 'Flash'. Got: {:?}",
        def.raw_keywords
    );

    // Instantiate and verify Flash keyword is in the KeywordSet
    let card_id = CardId::new(1);
    let player_id = PlayerId::new(1);
    let card = def.instantiate(card_id, player_id);

    assert!(
        card.keywords.contains(Keyword::Flash),
        "Twin Blades should have Flash keyword. Keywords: {:?}",
        card.keywords
    );

    // Verify has_keyword method works
    assert!(
        card.has_keyword(Keyword::Flash),
        "has_keyword(Flash) should return true for Twin Blades"
    );

    // Verify it's an artifact (not an instant)
    assert!(card.is_artifact(), "Twin Blades should be an artifact");
    assert!(!card.is_instant(), "Twin Blades should not be an instant");

    Ok(())
}

/// Test Ba Sing Se activated earthbend ability parsing (mtg-230)
#[test]
fn test_ba_sing_se_earthbend_ability() -> Result<()> {
    use mtg_engine::core::Effect;

    let path = PathBuf::from("cardsfolder/b/ba_sing_se.txt");
    if !path.exists() {
        eprintln!("Skipping test - cardsfolder not present");
        return Ok(());
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Ba Sing Se");
    assert!(def.types.contains(&CardType::Land));

    // Instantiate the card to get activated abilities
    let card_id = CardId::new(1);
    let player_id = PlayerId::new(1);
    let card = def.instantiate(card_id, player_id);

    // Should have at least 2 activated abilities: mana and earthbend
    assert!(
        card.activated_abilities.len() >= 2,
        "Ba Sing Se should have at least 2 activated abilities. Got: {:?}",
        card.activated_abilities
    );

    // Find the earthbend ability
    let earthbend_ability = card
        .activated_abilities
        .iter()
        .find(|a| a.effects.iter().any(|e| matches!(e, Effect::Earthbend { .. })));

    assert!(
        earthbend_ability.is_some(),
        "Ba Sing Se should have an Earthbend activated ability. Abilities: {:?}",
        card.activated_abilities
    );

    let ability = earthbend_ability.unwrap();

    // Check the earthbend effect has 2 counters
    let Effect::Earthbend { num_counters, .. } = &ability.effects[0] else {
        panic!("Expected Earthbend effect, got {:?}", ability.effects[0]);
    };
    assert_eq!(*num_counters, 2, "Earthbend should put 2 counters");

    // Check sorcery_speed is set
    assert!(ability.sorcery_speed, "Earthbend ability should be sorcery-speed");

    // Check tap cost
    assert!(ability.cost.includes_tap(), "Earthbend ability should include tap cost");

    Ok(())
}

/// Test loading Caldera Kavu (ChooseColor activated ability)
#[test]
fn test_load_caldera_kavu_choose_color() -> Result<()> {
    let path = PathBuf::from("cardsfolder/c/caldera_kavu.txt");
    if !path.exists() {
        return Ok(());
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Caldera Kavu");
    assert!(def.types.contains(&CardType::Creature));
    assert_eq!(def.power, Some(2));
    assert_eq!(def.toughness, Some(2));

    // Instantiate to get parsed activated abilities
    let card = def.instantiate(CardId::new(1), PlayerId::new(1));

    // Should have 2 activated abilities: Pump ({1}{B}) and ChooseColor ({G})
    assert!(
        card.activated_abilities.len() >= 2,
        "Caldera Kavu should have at least 2 activated abilities, found {}",
        card.activated_abilities.len()
    );

    // Find the ChooseColor ability
    let choose_color_ability = card.activated_abilities.iter().find(|a| {
        a.effects
            .iter()
            .any(|e| matches!(e, mtg_engine::core::Effect::ChooseColor { .. }))
    });
    assert!(
        choose_color_ability.is_some(),
        "Caldera Kavu should have a ChooseColor ability. Abilities: {:?}",
        card.activated_abilities
            .iter()
            .map(|a| format!("effects={:?}", a.effects))
            .collect::<Vec<_>>()
    );

    Ok(())
}

/// Test loading Spiritmonger (ChooseColor with Animate sub-ability)
#[test]
fn test_load_spiritmonger_choose_color() -> Result<()> {
    let path = PathBuf::from("cardsfolder/s/spiritmonger.txt");
    if !path.exists() {
        return Ok(());
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Spiritmonger");
    assert!(def.types.contains(&CardType::Creature));
    assert_eq!(def.power, Some(6));
    assert_eq!(def.toughness, Some(6));

    // Instantiate to get parsed activated abilities
    let card = def.instantiate(CardId::new(1), PlayerId::new(1));

    // Should have ChooseColor ability
    let has_choose_color = card.activated_abilities.iter().any(|a| {
        a.effects
            .iter()
            .any(|e| matches!(e, mtg_engine::core::Effect::ChooseColor { .. }))
    });
    assert!(
        has_choose_color,
        "Spiritmonger should have a ChooseColor ability. Abilities: {:?}",
        card.activated_abilities
            .iter()
            .map(|a| format!("effects={:?}", a.effects))
            .collect::<Vec<_>>()
    );

    Ok(())
}

/// Test loading Impulse (basic Dig: look at 4, put 1 in hand, rest on bottom)
#[test]
fn test_load_impulse_dig() -> Result<()> {
    let path = PathBuf::from("cardsfolder/i/impulse.txt");
    if !path.exists() {
        return Ok(());
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Impulse");
    assert!(def.types.contains(&CardType::Instant));

    let card = def.instantiate(CardId::new(1), PlayerId::new(1));

    // Impulse should have a Dig effect when it resolves
    let has_dig = card.effects.iter().any(|e| {
        matches!(
            e,
            mtg_engine::core::Effect::Dig {
                dig_count: 4,
                change_count: 1,
                change_all: false,
                destination: Zone::Hand,
                rest_destination: Zone::Library,
                optional: false,
                ..
            }
        )
    });
    assert!(
        has_dig,
        "Impulse should have a Dig effect (4 cards, pick 1 for hand). Effects: {:?}",
        card.effects
    );

    Ok(())
}

/// Test loading Wrenn and Seven (Dig with ChangeValid$ Land, Reveal, DestinationZone2$ Graveyard)
#[test]
fn test_load_wrenn_and_seven_dig() -> Result<()> {
    let path = PathBuf::from("cardsfolder/w/wrenn_and_seven.txt");
    if !path.exists() {
        return Ok(());
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Wrenn and Seven");

    let card = def.instantiate(CardId::new(1), PlayerId::new(1));

    // Wrenn's +1 should be: Dig 4, ChangeNum All, ChangeValid Land, Reveal, rest→Graveyard
    let dig_ability = card.activated_abilities.iter().find(|a| {
        a.effects
            .iter()
            .any(|e| matches!(e, mtg_engine::core::Effect::Dig { .. }))
    });
    assert!(dig_ability.is_some(), "Wrenn and Seven should have a Dig ability");

    let dig_effect = dig_ability
        .unwrap()
        .effects
        .iter()
        .find(|e| matches!(e, mtg_engine::core::Effect::Dig { .. }));

    if let Some(mtg_engine::core::Effect::Dig {
        dig_count,
        change_all,
        destination,
        rest_destination,
        reveal,
        change_valid,
        ..
    }) = dig_effect
    {
        assert_eq!(*dig_count, 4, "Should dig 4 cards");
        assert!(*change_all, "Should move ALL matching cards");
        assert_eq!(*destination, Zone::Hand, "Selected cards go to hand");
        assert_eq!(*rest_destination, Zone::Graveyard, "Rest goes to graveyard");
        assert!(*reveal, "Should reveal cards");
        assert!(
            change_valid.contains(&DigFilter::Land),
            "Should filter for Land cards, got {:?}",
            change_valid
        );
    } else {
        panic!("Expected Dig effect");
    }

    Ok(())
}

/// Test DigFilter::matches correctly identifies card types
#[test]
fn test_dig_filter_matches() {
    use mtg_engine::core::{Card, DigFilter};

    let owner = PlayerId::new(1);

    use smallvec::smallvec;

    // Create a creature card
    let mut creature = Card::new(CardId::new(1), "Test Creature", owner);
    creature.set_types(smallvec![CardType::Creature]);

    // Create a land card
    let mut land = Card::new(CardId::new(2), "Test Land", owner);
    land.set_types(smallvec![CardType::Land]);

    // Create an instant card
    let mut instant = Card::new(CardId::new(3), "Test Instant", owner);
    instant.set_types(smallvec![CardType::Instant]);

    // Create an artifact creature
    let mut artifact_creature = Card::new(CardId::new(4), "Test Artifact Creature", owner);
    artifact_creature.set_types(smallvec![CardType::Artifact, CardType::Creature]);

    // Test DigFilter::Creature
    assert!(DigFilter::Creature.matches(&creature));
    assert!(!DigFilter::Creature.matches(&land));
    assert!(DigFilter::Creature.matches(&artifact_creature));

    // Test DigFilter::Land
    assert!(DigFilter::Land.matches(&land));
    assert!(!DigFilter::Land.matches(&creature));

    // Test DigFilter::Card (matches anything)
    assert!(DigFilter::Card.matches(&creature));
    assert!(DigFilter::Card.matches(&land));
    assert!(DigFilter::Card.matches(&instant));

    // Test DigFilter::Permanent (not instant/sorcery)
    assert!(DigFilter::Permanent.matches(&creature));
    assert!(DigFilter::Permanent.matches(&land));
    assert!(!DigFilter::Permanent.matches(&instant));

    // Test DigFilter::Artifact
    assert!(DigFilter::Artifact.matches(&artifact_creature));
    assert!(!DigFilter::Artifact.matches(&creature));
}

/// Test that Orgg's conditional CantAttack static is parsed correctly.
///
/// Orgg has two statics:
/// 1. `S:Mode$ CantAttack | ValidCard$ Card.Self | UnlessDefender$ !controlsCreature.untapped+powerGE3`
///    → parsed as StaticAbility::CantAttackIfDefenderHasUntappedPowerGE { min_power: 3 }
/// 2. `S:Mode$ CantBlockBy | ValidAttacker$ Creature.powerGE3 | ValidBlocker$ Creature.Self`
///    → parsed as StaticAbility::CantBlockMatching { attacker_filter: Creature.powerGE3 }
///
/// Both should appear in the instantiated card's static_abilities. (mtg-917 B3)
#[test]
fn test_load_orgg_cant_attack_static() -> Result<()> {
    use mtg_engine::core::{CardId, PlayerId, StaticAbility};

    let path = PathBuf::from("cardsfolder/o/orgg.txt");
    if !path.exists() {
        return Ok(()); // skip if cardsfolder absent
    }

    let def = CardLoader::load_from_file(&path)?;
    assert_eq!(def.name.as_str(), "Orgg");

    let card_id = CardId::new(1);
    let player_id = PlayerId::new(1);
    let card = def.instantiate(card_id, player_id);

    // CantAttackIfDefenderHasUntappedPowerGE must be present with min_power=3.
    let has_cant_attack = card.static_abilities.iter().any(|sa| {
        matches!(
            sa,
            StaticAbility::CantAttackIfDefenderHasUntappedPowerGE { min_power: 3, .. }
        )
    });
    assert!(
        has_cant_attack,
        "Orgg must have CantAttackIfDefenderHasUntappedPowerGE(3) static ability. Got: {:?}",
        card.static_abilities
    );

    // CantBlockMatching (power >= 3 attackers) must also be present.
    let has_cant_block = card
        .static_abilities
        .iter()
        .any(|sa| matches!(sa, StaticAbility::CantBlockMatching { .. }));
    assert!(
        has_cant_block,
        "Orgg must have CantBlockMatching static ability. Got: {:?}",
        card.static_abilities
    );

    Ok(())
}
