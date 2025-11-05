//! Card loading tests
//!
//! Tests that verify cards from cardsfolder can be loaded and parsed correctly

use mtg_forge_rs::core::{CardType, Keyword};
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
    assert!(card.keywords.contains(&Keyword::Flying));
    assert!(card.keywords.contains(&Keyword::ProtectionFromRed));

    // Verify helper methods
    assert!(card.has_flying());
    assert!(card.has_keyword(&Keyword::ProtectionFromRed));

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
    assert!(matches!(card.keywords[0], Keyword::Madness(_)));

    if let Keyword::Madness(cost) = &card.keywords[0] {
        assert_eq!(cost, "1 R");
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
    assert!(matches!(card.keywords[0], Keyword::Flashback(_)));

    if let Keyword::Flashback(cost) = &card.keywords[0] {
        assert_eq!(cost, "3 R");
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
    assert!(matches!(card.keywords[0], Keyword::Enchant(_)));

    if let Keyword::Enchant(target_type) = &card.keywords[0] {
        assert_eq!(target_type, "Creature");
    }

    Ok(())
}
