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
    }

    Ok(())
}
