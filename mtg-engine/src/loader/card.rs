//! Card file loader (.txt format)
//!
//! Loads card definitions from Forge's cardsfolder format

use crate::core::{
    Card, CardName, CardType, Color, KeywordComplex, KeywordSet, KeywordSimple, ManaCost, Subtype, Trigger,
    TriggerEvent,
};
use crate::{MtgError, Result};
use smallvec::SmallVec;
use std::fs;
use std::path::Path;

/// Card loader for .txt files
pub struct CardLoader;

impl CardLoader {
    /// Load a card from a .txt file
    pub fn load_from_file(path: &Path) -> Result<CardDefinition> {
        let content = fs::read_to_string(path).map_err(MtgError::IoError)?;
        Self::parse(&content).map_err(|e| {
            // Enhance error message with file path for easier debugging
            MtgError::InvalidCardFormat(format!("Failed to parse card file '{}': {}", path.display(), e))
        })
    }

    /// Parse a card from its text content
    pub fn parse(content: &str) -> Result<CardDefinition> {
        let mut name = None;
        let mut mana_cost = ManaCost::new();
        let mut types = Vec::new();
        let mut subtypes = Vec::new();
        let mut colors = Vec::new();
        let mut power = None;
        let mut toughness = None;
        let mut oracle = String::new();
        let mut raw_abilities = Vec::new();
        let mut raw_keywords = Vec::new();

        for (line_num, line) in content.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if let Some((key, value)) = line.split_once(':') {
                let key = key.trim();
                let value = value.trim();

                match key {
                    "Name" => name = Some(CardName::new(value)),
                    "ManaCost" => mana_cost = ManaCost::from_string(value),
                    "Types" => {
                        for part in value.split_whitespace() {
                            match part {
                                "Creature" => types.push(CardType::Creature),
                                "Instant" => types.push(CardType::Instant),
                                "Sorcery" => types.push(CardType::Sorcery),
                                "Enchantment" => types.push(CardType::Enchantment),
                                "Artifact" => types.push(CardType::Artifact),
                                "Land" => types.push(CardType::Land),
                                "Planeswalker" => types.push(CardType::Planeswalker),
                                _ => subtypes.push(Subtype::new(part)),
                            }
                        }
                    }
                    "PT" => {
                        if let Some((p, t)) = value.split_once('/') {
                            let p_trimmed = p.trim();
                            let t_trimmed = t.trim();

                            // Try to parse power - if it contains non-numeric characters (*, ?, +, etc.),
                            // treat it as variable P/T and set to None (handled by card-specific logic)
                            power = p_trimmed.parse().ok();
                            toughness = t_trimmed.parse().ok();
                        } else {
                            return Err(MtgError::InvalidCardFormat(format!(
                                "Line {}: Invalid PT format '{}' (expected format: 'N/N', e.g., 'PT:2/2')",
                                line_num + 1,
                                value
                            )));
                        }
                    }
                    "Oracle" => oracle = value.to_string(),
                    // Keyword lines (K:)
                    "K" => {
                        raw_keywords.push(value.to_string());
                    }
                    // Ability lines (A:, S:, T:, SVar:, etc.)
                    "A" | "S" | "T" | "SVar" => {
                        raw_abilities.push(format!("{key}:{value}"));
                    }
                    _ => {} // Ignore other fields for now
                }
            } else {
                // Line doesn't contain a colon - might be malformed
                // Only warn if it's not empty and not a comment (already filtered above)
                // This allows for future extensibility without breaking
            }
        }

        // Derive colors from mana cost
        if mana_cost.white > 0 {
            colors.push(Color::White);
        }
        if mana_cost.blue > 0 {
            colors.push(Color::Blue);
        }
        if mana_cost.black > 0 {
            colors.push(Color::Black);
        }
        if mana_cost.red > 0 {
            colors.push(Color::Red);
        }
        if mana_cost.green > 0 {
            colors.push(Color::Green);
        }
        if colors.is_empty() {
            colors.push(Color::Colorless);
        }

        let name = name.ok_or_else(|| {
            MtgError::InvalidCardFormat(
                "Missing required 'Name:' field (add 'Name: <card name>' to the card file)".to_string(),
            )
        })?;

        Ok(CardDefinition {
            name,
            mana_cost,
            types,
            subtypes,
            colors,
            power,
            toughness,
            oracle,
            raw_abilities,
            raw_keywords,
        })
    }
}

/// Card definition (not yet instantiated in a game)
#[derive(Debug, Clone)]
pub struct CardDefinition {
    pub name: CardName,
    pub mana_cost: ManaCost,
    pub types: Vec<CardType>,
    pub subtypes: Vec<Subtype>,
    pub colors: Vec<Color>,
    pub power: Option<i8>,
    pub toughness: Option<i8>,
    pub oracle: String,
    /// Raw ability scripts from the card file (A:, S:, T: lines)
    /// We'll parse these into actual effects later
    pub raw_abilities: Vec<String>,
    /// Raw keyword scripts from the card file (K: lines)
    pub raw_keywords: Vec<String>,
}

impl CardDefinition {
    /// Create a Card instance from this definition
    pub fn instantiate(&self, id: crate::core::CardId, owner: crate::core::PlayerId) -> Card {
        let mut card = Card::new(id, self.name.clone(), owner);
        card.mana_cost = self.mana_cost;
        card.types = SmallVec::from_slice(&self.types);
        card.subtypes = self.subtypes.iter().cloned().collect();
        card.colors = SmallVec::from_slice(&self.colors);
        card.power = self.power;
        card.toughness = self.toughness;
        card.text = self.oracle.clone();

        // Populate cache after text is set (avoids allocation in gameplay)
        card.cache = crate::core::CardCache::new(&card.text, card.name.as_str());

        // Parse keywords
        card.keywords = self.parse_keywords();

        // Parse abilities into effects (simplified parser for common cases)
        card.effects = self.parse_effects();

        // Parse triggered abilities
        card.triggers = self.parse_triggers();

        // Parse activated abilities
        card.activated_abilities = self.parse_activated_abilities();

        // Add implicit mana ability for basic lands
        // Basic lands (Plains, Island, Swamp, Mountain, Forest) have an implicit "{T}: Add {color}"
        // ability that's not written in the card file
        if card.is_land()
            && self.subtypes.iter().any(|st| {
                let st_str = st.as_str();
                st_str == "Plains"
                    || st_str == "Island"
                    || st_str == "Swamp"
                    || st_str == "Mountain"
                    || st_str == "Forest"
            })
        {
            // Determine which color to produce based on subtype
            let mana_to_produce = if self.subtypes.iter().any(|st| st.as_str() == "Plains") {
                ManaCost::from_string("W")
            } else if self.subtypes.iter().any(|st| st.as_str() == "Island") {
                ManaCost::from_string("U")
            } else if self.subtypes.iter().any(|st| st.as_str() == "Swamp") {
                ManaCost::from_string("B")
            } else if self.subtypes.iter().any(|st| st.as_str() == "Mountain") {
                ManaCost::from_string("R")
            } else if self.subtypes.iter().any(|st| st.as_str() == "Forest") {
                ManaCost::from_string("G")
            } else {
                ManaCost::new() // Shouldn't happen
            };

            // Only add if we don't already have a mana ability
            // (in case the card file explicitly defines one)
            if !card.activated_abilities.iter().any(|ab| ab.is_mana_ability) {
                use crate::core::{ActivatedAbility, Cost, Effect, PlayerId};

                let ability = ActivatedAbility::new(
                    Cost::Tap,
                    vec![Effect::AddMana {
                        player: PlayerId::new(0), // Placeholder - will be filled when activated
                        mana: mana_to_produce,
                    }],
                    format!("Add {mana_to_produce}"),
                    true, // This IS a mana ability
                );
                card.activated_abilities.push(ability);
            }
        }

        card
    }

    /// Parse raw keywords into KeywordSet
    fn parse_keywords(&self) -> KeywordSet {
        let mut keyword_set = KeywordSet::new();

        for keyword_str in &self.raw_keywords {
            // Check if keyword has a parameter (colon separated)
            if let Some((kw, param)) = keyword_str.split_once(':') {
                let kw = kw.trim();
                let param = param.trim();

                // Keywords with parameters
                let complex_keyword = match kw {
                    "Madness" => KeywordComplex::Madness(param.to_string()),
                    "Flashback" => KeywordComplex::Flashback(param.to_string()),
                    "Enchant" => KeywordComplex::Enchant(param.to_string()),
                    _ => KeywordComplex::Other(keyword_str.clone()),
                };
                keyword_set.push_complex(complex_keyword);
            } else {
                // Simple keywords (no parameters)
                let kw = keyword_str.trim();
                match kw {
                    "Flying" => keyword_set.insert_simple(KeywordSimple::Flying),
                    "First Strike" => keyword_set.insert_simple(KeywordSimple::FirstStrike),
                    "Double Strike" => keyword_set.insert_simple(KeywordSimple::DoubleStrike),
                    "Deathtouch" => keyword_set.insert_simple(KeywordSimple::Deathtouch),
                    "Haste" => keyword_set.insert_simple(KeywordSimple::Haste),
                    "Hexproof" => keyword_set.insert_simple(KeywordSimple::Hexproof),
                    "Indestructible" => keyword_set.insert_simple(KeywordSimple::Indestructible),
                    "Lifelink" => keyword_set.insert_simple(KeywordSimple::Lifelink),
                    "Menace" => keyword_set.insert_simple(KeywordSimple::Menace),
                    "Reach" => keyword_set.insert_simple(KeywordSimple::Reach),
                    "Trample" => keyword_set.insert_simple(KeywordSimple::Trample),
                    "Vigilance" => keyword_set.insert_simple(KeywordSimple::Vigilance),
                    "Defender" => keyword_set.insert_simple(KeywordSimple::Defender),
                    "Shroud" => keyword_set.insert_simple(KeywordSimple::Shroud),
                    "Choose a Background" => keyword_set.insert_simple(KeywordSimple::ChooseABackground),
                    // Protection variants
                    "Protection from red" => keyword_set.insert_simple(KeywordSimple::ProtectionFromRed),
                    "Protection from blue" => keyword_set.insert_simple(KeywordSimple::ProtectionFromBlue),
                    "Protection from black" => keyword_set.insert_simple(KeywordSimple::ProtectionFromBlack),
                    "Protection from white" => keyword_set.insert_simple(KeywordSimple::ProtectionFromWhite),
                    "Protection from green" => keyword_set.insert_simple(KeywordSimple::ProtectionFromGreen),
                    _ => keyword_set.push_complex(KeywordComplex::Other(keyword_str.to_string())),
                }
            }
        }

        keyword_set
    }

    /// Parse raw abilities into Effect objects
    ///
    /// Uses tokenized parsing (ability_parser) for safety and correctness.
    /// Replaces unsafe substring matching with proper parameter extraction.
    fn parse_effects(&self) -> Vec<crate::core::Effect> {
        use super::ability_parser::AbilityParams;
        use super::effect_converter::params_to_effect;

        let mut effects = Vec::new();

        for ability in &self.raw_abilities {
            // Skip non-spell/ability lines (triggers, statics, etc.)
            // We only process A:SP$ (spell effects) and A:AB$ (activated abilities) here
            // Triggers and statics are handled by parse_triggers() and future parse_static()
            if !ability.starts_with("A:SP$") && !ability.starts_with("A:AB$") {
                continue;
            }

            // Parse ability string into tokenized parameters
            let params = match AbilityParams::parse(ability) {
                Ok(p) => p,
                Err(e) => {
                    // Log parse error but continue processing other abilities
                    eprintln!("Warning: Failed to parse ability '{}': {}", ability, e);
                    continue;
                }
            };

            // Convert parameters to Effect (if supported)
            if let Some(effect) = params_to_effect(&params) {
                effects.push(effect);
            }
            // Note: Unsupported API types are silently skipped (returns None)
            // This is intentional - we don't want to spam warnings for every unsupported ability
        }

        effects
    }

    /// Parse triggered abilities (T: lines)
    ///
    /// Uses tokenized parameter extraction for safety. Replaces unsafe substring matching.
    fn parse_triggers(&self) -> Vec<Trigger> {
        use std::collections::HashMap;

        let mut triggers = Vec::new();

        for ability in &self.raw_abilities {
            // Only process T: lines (triggered abilities)
            if !ability.starts_with("T:") {
                continue;
            }

            // Parse parameters by splitting on | (simpler than AbilityParams since triggers don't have record types)
            let mut params = HashMap::new();
            if let Some((_prefix, body)) = ability.split_once(':') {
                for param in body.split('|') {
                    let param = param.trim();
                    if param.is_empty() {
                        continue;
                    }
                    if let Some((key, value)) = param.split_once('$') {
                        params.insert(key.trim().to_string(), value.trim().to_string());
                    }
                }
            }

            // Determine trigger type from Mode$ parameter
            let mode = params.get("Mode").map(|s| s.as_str());

            // Parse ETB triggers (Mode$ ChangesZone)
            if mode == Some("ChangesZone")
                && params.get("Destination").map(|s| s.as_str()) == Some("Battlefield")
                && params.get("ValidCard").map(|s| s.as_str()) == Some("Card.Self")
            {
                use crate::core::{CardId, Effect, PlayerId, TargetRef};

                // Parse effects - check for parameters in this trigger AND in other raw_abilities
                // (for SVar resolution compatibility)
                let mut effects = Vec::new();

                // Helper: search for a parameter across all raw_abilities (for SVar lookups)
                let find_param = |key: &str| -> Option<String> {
                    for ab in &self.raw_abilities {
                        if let Some((_pre, body)) = ab.split_once(':') {
                            for param in body.split('|') {
                                if let Some((k, v)) = param.split_once('$') {
                                    if k.trim() == key {
                                        return Some(v.trim().to_string());
                                    }
                                }
                            }
                        }
                    }
                    None
                };

                // Check if we have NumCards$ parameter (draw effect)
                if let Some(num_cards_str) = params
                    .get("NumCards")
                    .map(|s| s.to_string())
                    .or_else(|| find_param("NumCards"))
                {
                    if let Ok(count) = num_cards_str.parse::<u8>() {
                        effects.push(Effect::DrawCards {
                            player: PlayerId::new(0),
                            count,
                        });
                    }
                }

                // Check if we have NumDmg$ parameter (damage effect)
                if let Some(num_dmg_str) = params
                    .get("NumDmg")
                    .map(|s| s.to_string())
                    .or_else(|| find_param("NumDmg"))
                {
                    if let Ok(amount) = num_dmg_str.parse::<i32>() {
                        effects.push(Effect::DealDamage {
                            target: TargetRef::None,
                            amount,
                        });
                    }
                }

                // Check if we have LifeAmount$ parameter (gain life effect)
                if let Some(life_amt_str) = params
                    .get("LifeAmount")
                    .map(|s| s.to_string())
                    .or_else(|| find_param("LifeAmount"))
                {
                    if let Ok(amount) = life_amt_str.parse::<i32>() {
                        effects.push(Effect::GainLife {
                            player: PlayerId::new(0),
                            amount,
                        });
                    }
                }

                // Check if we have NumAtt$/NumDef$ parameters (pump effect)
                let power_bonus = params
                    .get("NumAtt")
                    .map(|s| s.to_string())
                    .or_else(|| find_param("NumAtt"))
                    .and_then(|s| s.trim_start_matches('+').parse::<i32>().ok())
                    .unwrap_or(0);
                let toughness_bonus = params
                    .get("NumDef")
                    .map(|s| s.to_string())
                    .or_else(|| find_param("NumDef"))
                    .and_then(|s| s.trim_start_matches('+').parse::<i32>().ok())
                    .unwrap_or(0);

                if power_bonus != 0 || toughness_bonus != 0 {
                    effects.push(Effect::PumpCreature {
                        target: CardId::new(0),
                        power_bonus,
                        toughness_bonus,
                    });
                }

                // Extract description from TriggerDescription$ if available
                let description = params
                    .get("TriggerDescription")
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "When this enters the battlefield".to_string());

                // Note: This implements basic SVar resolution by searching across all raw_abilities
                // for effect parameters. Proper SVar resolution would parse SVar: lines separately.
                // TODO: Implement proper SVar parsing and Execute$ sub-ability resolution

                triggers.push(Trigger::new(TriggerEvent::EntersBattlefield, effects, description));
            }

            // Parse phase triggers (Mode$ Phase)
            if mode == Some("Phase") {
                // Determine which phase/step this triggers on using tokenized params
                let trigger_event = match params.get("Phase").map(|s| s.as_str()) {
                    Some("Upkeep") => Some(TriggerEvent::BeginningOfUpkeep),
                    Some("EndOfTurn") | Some("End") => Some(TriggerEvent::BeginningOfEndStep),
                    _ => None, // Other phases not supported yet
                };

                if let Some(event) = trigger_event {
                    // For now, create a simple trigger without complex conditions
                    // TODO(mtg-111): Support CheckSVar$, SVarCompare$, Execute$ sub-abilities
                    // TODO(mtg-111): Support ValidPlayer$ filtering (You vs Opponent vs Each)
                    // TODO(mtg-111): Support OptionalDecider$ for optional triggers

                    let description = params
                        .get("TriggerDescription")
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "At the beginning of upkeep".to_string());

                    // For now, don't add effects - phase triggers usually need complex parsing
                    // This creates a placeholder trigger that can be detected in the game loop
                    // Real implementation will need to parse Execute$ and SubAbility references
                    triggers.push(Trigger::new(event, vec![], description));
                }
            }
        }

        triggers
    }

    /// Parse activated abilities (A:AB$ lines)
    ///
    /// Uses tokenized parsing with params_to_effect() for all effect types.
    /// Eliminates unsafe substring matching.
    fn parse_activated_abilities(&self) -> Vec<crate::core::ActivatedAbility> {
        use super::ability_parser::{AbilityParams, AbilityRecordType};
        use crate::core::{ActivatedAbility, Cost};

        let mut abilities = Vec::new();

        for ability in &self.raw_abilities {
            // Only process A:AB$ lines (activated abilities)
            if !ability.starts_with("A:AB$") {
                continue;
            }

            // Parse ability string into tokenized parameters
            let params = match AbilityParams::parse(ability) {
                Ok(p) if p.record_type == AbilityRecordType::Ability => p,
                Ok(_) => {
                    eprintln!("Warning: Expected AB$ record type in '{}'", ability);
                    continue;
                }
                Err(e) => {
                    eprintln!("Warning: Failed to parse activated ability '{}': {}", ability, e);
                    continue;
                }
            };

            // Extract cost from Cost$ parameter
            let cost = if let Some(cost_str) = params.get("Cost") {
                Cost::parse(cost_str)
            } else {
                None
            };

            if cost.is_none() {
                continue; // Skip abilities without parseable cost
            }
            let cost = cost.unwrap();

            // Parse effects using the tokenized converter
            use super::ability_parser::ApiType;
            use super::effect_converter::params_to_effect;

            // Special handling for mana abilities (need is_mana_ability = true)
            let is_mana_ability = matches!(params.api_type, ApiType::Mana);

            // Try to convert parameters to effects
            let effects = if let Some(effect) = params_to_effect(&params) {
                vec![effect]
            } else {
                // Fallback to old parsing for unsupported API types
                // TODO: Remove this once all API types are migrated
                vec![]
            };

            // Extract description
            let description = params
                .get("SpellDescription")
                .unwrap_or("Activated ability")
                .to_string();

            // Only add if we have effects
            if !effects.is_empty() {
                abilities.push(ActivatedAbility::new(cost, effects, description, is_mana_ability));
            }
        }

        abilities
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_lightning_bolt() {
        let content = r#"
Name:Lightning Bolt
ManaCost:R
Types:Instant
A:SP$ DealDamage | ValidTgts$ Any | NumDmg$ 3 | SpellDescription$ CARDNAME deals 3 damage to any target.
Oracle:Lightning Bolt deals 3 damage to any target.
"#;

        let def = CardLoader::parse(content).unwrap();
        assert_eq!(def.name.as_str(), "Lightning Bolt");
        assert_eq!(def.mana_cost.red, 1);
        assert_eq!(def.types.len(), 1);
        assert!(def.types.contains(&CardType::Instant));
        assert!(def.colors.contains(&Color::Red));

        // Check that the effect is parsed
        let effects = def.parse_effects();
        assert_eq!(effects.len(), 1, "Lightning Bolt should have 1 effect");

        use crate::core::{Effect, TargetRef};
        match &effects[0] {
            Effect::DealDamage { target, amount } => {
                assert_eq!(*amount, 3, "Should deal 3 damage");
                assert!(matches!(target, TargetRef::None), "Target should be None initially");
            }
            _ => panic!("Expected DealDamage effect"),
        }
    }

    #[test]
    fn test_parse_creature() {
        let content = r#"
Name:Grizzly Bears
ManaCost:1G
Types:Creature Bear
PT:2/2
Oracle:
"#;

        let def = CardLoader::parse(content).unwrap();
        assert_eq!(def.name.as_str(), "Grizzly Bears");
        assert_eq!(def.mana_cost.generic, 1);
        assert_eq!(def.mana_cost.green, 1);
        assert!(def.types.contains(&CardType::Creature));
        assert!(def.subtypes.contains(&Subtype::new("Bear")));
        assert_eq!(def.power, Some(2));
        assert_eq!(def.toughness, Some(2));
    }

    #[test]
    fn test_load_from_cardsfolder() {
        use std::path::PathBuf;

        // Try to load Lightning Bolt from the cardsfolder
        let path = PathBuf::from("cardsfolder/l/lightning_bolt.txt");

        // Only run this test if the cardsfolder exists
        if !path.exists() {
            return;
        }

        let def = CardLoader::load_from_file(&path).unwrap();
        assert_eq!(def.name.as_str(), "Lightning Bolt");
        assert_eq!(def.mana_cost.red, 1);
        assert!(def.types.contains(&CardType::Instant));
        assert!(def.colors.contains(&Color::Red));
        assert_eq!(def.raw_abilities.len(), 1);
        assert!(def.raw_abilities[0].contains("DealDamage"));
    }

    #[test]
    fn test_parse_with_abilities() {
        let content = r#"
Name:Lightning Bolt
ManaCost:R
Types:Instant
A:SP$ DealDamage | ValidTgts$ Any | NumDmg$ 3 | SpellDescription$ CARDNAME deals 3 damage to any target.
Oracle:Lightning Bolt deals 3 damage to any target.
"#;

        let def = CardLoader::parse(content).unwrap();
        assert_eq!(def.name.as_str(), "Lightning Bolt");
        assert_eq!(def.raw_abilities.len(), 1);
        assert!(def.raw_abilities[0].starts_with("A:"));
        assert!(def.raw_abilities[0].contains("DealDamage"));
    }

    #[test]
    fn test_parse_draw_spell() {
        let content = r#"
Name:Ancestral Recall
ManaCost:U
Types:Instant
A:SP$ Draw | NumCards$ 3 | ValidTgts$ Player | TgtPrompt$ Select target player | SpellDescription$ Target player draws three cards.
Oracle:Target player draws three cards.
"#;

        let def = CardLoader::parse(content).unwrap();
        assert_eq!(def.name.as_str(), "Ancestral Recall");
        assert_eq!(def.mana_cost.blue, 1);
        assert!(def.types.contains(&CardType::Instant));
        assert!(def.colors.contains(&Color::Blue));

        // Check that the effect is parsed
        let effects = def.parse_effects();
        assert_eq!(effects.len(), 1, "Ancestral Recall should have 1 effect");

        use crate::core::Effect;
        match &effects[0] {
            Effect::DrawCards { player: _, count } => {
                assert_eq!(*count, 3, "Should draw 3 cards");
            }
            _ => panic!("Expected DrawCards effect, got {:?}", effects[0]),
        }
    }

    #[test]
    fn test_parse_destroy_spell() {
        let content = r#"
Name:Terror
ManaCost:1 B
Types:Instant
A:SP$ Destroy | ValidTgts$ Creature.nonArtifact+nonBlack | TgtPrompt$ Select target nonartifact, nonblack creature | NoRegen$ True | SpellDescription$ Destroy target nonartifact, nonblack creature. It can't be regenerated.
Oracle:Destroy target nonartifact, nonblack creature. It can't be regenerated.
"#;

        let def = CardLoader::parse(content).unwrap();
        assert_eq!(def.name.as_str(), "Terror");
        assert_eq!(def.mana_cost.generic, 1);
        assert_eq!(def.mana_cost.black, 1);
        assert!(def.types.contains(&CardType::Instant));
        assert!(def.colors.contains(&Color::Black));

        // Check that the effect is parsed
        let effects = def.parse_effects();
        assert_eq!(effects.len(), 1, "Terror should have 1 effect");

        use crate::core::Effect;
        match &effects[0] {
            Effect::DestroyPermanent { target: _ } => {
                // Success - correct effect type
            }
            _ => panic!("Expected DestroyPermanent effect, got {:?}", effects[0]),
        }
    }

    #[test]
    fn test_parse_gainlife_spell() {
        let content = r#"
Name:Angel's Mercy
ManaCost:2 W W
Types:Instant
A:SP$ GainLife | LifeAmount$ 7 | SpellDescription$ You gain 7 life.
Oracle:You gain 7 life.
"#;

        let def = CardLoader::parse(content).unwrap();
        assert_eq!(def.name.as_str(), "Angel's Mercy");
        assert_eq!(def.mana_cost.generic, 2);
        assert_eq!(def.mana_cost.white, 2);
        assert!(def.types.contains(&CardType::Instant));
        assert!(def.colors.contains(&Color::White));

        // Check that the effect is parsed
        let effects = def.parse_effects();
        assert_eq!(effects.len(), 1, "Angel's Mercy should have 1 effect");

        use crate::core::Effect;
        match &effects[0] {
            Effect::GainLife { player: _, amount } => {
                assert_eq!(*amount, 7, "Should gain 7 life");
            }
            _ => panic!("Expected GainLife effect, got {:?}", effects[0]),
        }
    }

    #[test]
    fn test_parse_activated_ability() {
        let content = r#"
Name:Prodigal Sorcerer
ManaCost:2 U
Types:Creature Human Wizard
PT:1/1
A:AB$ DealDamage | Cost$ T | ValidTgts$ Any | NumDmg$ 1 | SpellDescription$ CARDNAME deals 1 damage to any target.
Oracle:{T}: Prodigal Sorcerer deals 1 damage to any target.
"#;

        let def = CardLoader::parse(content).unwrap();
        assert_eq!(def.name.as_str(), "Prodigal Sorcerer");
        assert_eq!(def.mana_cost.generic, 2);
        assert_eq!(def.mana_cost.blue, 1);
        assert!(def.types.contains(&CardType::Creature));

        // Check that the activated ability is parsed
        let abilities = def.parse_activated_abilities();
        assert_eq!(abilities.len(), 1, "Prodigal Sorcerer should have 1 activated ability");

        let ability = &abilities[0];
        assert!(ability.cost.includes_tap(), "Should have tap cost");
        assert_eq!(ability.effects.len(), 1, "Should have 1 effect");

        use crate::core::Effect;
        match &ability.effects[0] {
            Effect::DealDamage { target: _, amount } => {
                assert_eq!(*amount, 1, "Should deal 1 damage");
            }
            _ => panic!("Expected DealDamage effect, got {:?}", ability.effects[0]),
        }
    }
}
