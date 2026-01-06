//! Card file loader (.txt format)
//!
//! Loads card definitions from Forge's cardsfolder format

use crate::core::{
    Card, CardId, CardName, CardType, Color, Effect, Keyword, KeywordArgs, KeywordSet, ManaCost, PlayerId, Subtype,
    TargetRef, Trigger, TriggerEvent,
};
use crate::{MtgError, Result};
use smallvec::SmallVec;
use std::cell::RefCell;
#[cfg(feature = "native")]
use std::fs;
#[cfg(feature = "native")]
use std::path::Path;

// Thread-local storage for the current file being parsed (for warning context)
thread_local! {
    static PARSING_FILE_CONTEXT: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Set the parsing file context for warnings
fn set_parsing_context(path: Option<&str>) {
    PARSING_FILE_CONTEXT.with(|ctx| {
        *ctx.borrow_mut() = path.map(|s| s.to_string());
    });
}

/// Emit a warning with file context if available
fn warn_with_context(message: &str) {
    PARSING_FILE_CONTEXT.with(|ctx| {
        if let Some(ref path) = *ctx.borrow() {
            eprintln!("Warning [{}]: {}", path, message);
        } else {
            eprintln!("Warning: {}", message);
        }
    });
}

/// Card loader for .txt files
pub struct CardLoader;

impl CardLoader {
    /// Load a card from a .txt file
    #[cfg(feature = "native")]
    pub fn load_from_file(path: &Path) -> Result<CardDefinition> {
        let content = fs::read_to_string(path).map_err(MtgError::IoError)?;
        // Set context for warnings during parsing
        let path_str = path.display().to_string();
        set_parsing_context(Some(&path_str));
        let result = Self::parse(&content).map_err(|e| {
            // Enhance error message with file path for easier debugging
            MtgError::InvalidCardFormat(format!("Failed to parse card file '{}': {}", path.display(), e))
        });
        // Clear context after parsing
        set_parsing_context(None);
        result
    }

    /// Parse a card with explicit file context for warnings
    pub fn parse_with_context(content: &str, file_context: Option<&str>) -> Result<CardDefinition> {
        set_parsing_context(file_context);
        let result = Self::parse(content);
        set_parsing_context(None);
        result
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
        let mut svars = std::collections::HashMap::new();
        let mut enters_tapped = false;
        let mut etb_choose_color = false;
        let mut etb_exclude_colors = Vec::new();

        for (line_num, line) in content.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // Stop parsing at ALTERNATE section (double-faced card back)
            // We only parse the front face of double-faced cards
            if line == "ALTERNATE" {
                break;
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
                    "Oracle" => oracle = value.replace("\\n", "\n"),
                    // Keyword lines (K:)
                    "K" => {
                        raw_keywords.push(value.to_string());
                        // Check for ETB replacement that requires choosing a color
                        // Format: K:ETBReplacement:Other:ChooseColor
                        if value.contains("ETBReplacement") && value.contains("ChooseColor") {
                            etb_choose_color = true;
                        }
                    }
                    // Ability lines (A:, S:, T: lines)
                    "A" | "S" | "T" => {
                        raw_abilities.push(format!("{key}:{value}"));
                    }
                    // Script variables (SVar:NAME:body)
                    // Format: "SVar" key with value "NAME:body"
                    "SVar" => {
                        raw_abilities.push(format!("{key}:{value}"));
                        // Also parse into svars HashMap for SubAbility resolution
                        // Value format: "NAME:DB$ ApiType | Param$ Value | ..."
                        if let Some((svar_name, svar_body)) = value.split_once(':') {
                            svars.insert(svar_name.trim().to_string(), svar_body.trim().to_string());
                            // Check for ChooseColor SVar with Exclude$ parameter
                            // Format: SVar:ChooseColor:DB$ ChooseColor | Exclude$ green | ...
                            if svar_name.trim() == "ChooseColor" && svar_body.contains("Exclude$") {
                                for param in svar_body.split('|') {
                                    let param = param.trim();
                                    if let Some((key, excluded)) = param.split_once('$') {
                                        if key.trim() == "Exclude" {
                                            // Parse excluded colors (comma-separated)
                                            for color_str in excluded.split(',') {
                                                match color_str.trim().to_lowercase().as_str() {
                                                    "white" => etb_exclude_colors.push(Color::White),
                                                    "blue" => etb_exclude_colors.push(Color::Blue),
                                                    "black" => etb_exclude_colors.push(Color::Black),
                                                    "red" => etb_exclude_colors.push(Color::Red),
                                                    "green" => etb_exclude_colors.push(Color::Green),
                                                    _ => {}
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // Replacement effects (R: lines)
                    // Check for ETB tapped replacement: "ReplaceWith$ ETBTapped"
                    "R" => {
                        if value.contains("ReplaceWith$ ETBTapped") {
                            enters_tapped = true;
                        }
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
            svars,
            enters_tapped,
            etb_choose_color,
            etb_exclude_colors,
        })
    }
}

/// Card definition (not yet instantiated in a game)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
    /// Script variables (SVar:NAME:...) for SubAbility chaining and other references
    /// Key: SVar name, Value: SVar body (DB$, AB$, etc.)
    pub svars: std::collections::HashMap<String, String>,
    /// Does this card enter the battlefield tapped?
    /// Derived from R: lines containing "ReplaceWith$ ETBTapped"
    pub enters_tapped: bool,
    /// Does this card require choosing a color when it enters the battlefield?
    /// Derived from K:ETBReplacement:Other:ChooseColor
    pub etb_choose_color: bool,
    /// Colors to exclude from the choice (from SVar:ChooseColor Exclude$ parameter)
    pub etb_exclude_colors: Vec<Color>,
}

impl CardDefinition {
    /// Extract all TokenScript references from this card's abilities
    ///
    /// Scans all raw_abilities for SVar lines containing "DB$ Token" and extracts
    /// the TokenScript$ parameter value. Returns unique token script names.
    ///
    /// Example:
    /// - Input: `SVar:TrigToken:DB$ Token | TokenScript$ c_a_food_sac | TokenAmount$ 1`
    /// - Output: `["c_a_food_sac"]`
    pub fn extract_token_scripts(&self) -> Vec<String> {
        let mut token_scripts = std::collections::HashSet::new();

        for ability in &self.raw_abilities {
            // Look for SVar lines with DB$ Token
            if ability.starts_with("SVar:") && ability.contains("DB$ Token") {
                // Parse the SVar body for TokenScript$ parameter
                // Format: "SVar:NAME:DB$ Token | TokenScript$ script_name | ..."
                if let Some((_prefix, body)) = ability.split_once(':').and_then(|(_, rest)| rest.split_once(':')) {
                    // Split by | and look for TokenScript$
                    for param in body.split('|') {
                        let param = param.trim();
                        if let Some((key, value)) = param.split_once('$') {
                            if key.trim() == "TokenScript" {
                                token_scripts.insert(value.trim().to_string());
                            }
                        }
                    }
                }
            }
        }

        token_scripts.into_iter().collect()
    }

    /// Create a Card instance from this definition
    pub fn instantiate(&self, id: crate::core::CardId, owner: crate::core::PlayerId) -> Card {
        let mut card = Card::new(id, self.name.clone(), owner);
        card.mana_cost = self.mana_cost;
        card.types = SmallVec::from_slice(&self.types);
        card.subtypes = self.subtypes.iter().cloned().collect();
        card.colors = SmallVec::from_slice(&self.colors);
        card.set_base_power(self.power);
        card.set_base_toughness(self.toughness);
        card.text = self.oracle.clone();

        // Initialize cache with type flags (for O(1) is_land/is_creature/is_artifact checks)
        // and empty mana production (will be populated after abilities are parsed)
        card.cache = crate::core::CardCache::new(&card.text, card.name.as_str());
        card.cache.update_from_types(&card.types);
        card.cache.update_from_subtypes(&card.subtypes, card.name.as_str());
        card.cache.enters_tapped = self.enters_tapped;
        card.cache.etb_choose_color = self.etb_choose_color;
        card.cache.etb_exclude_colors = SmallVec::from_slice(&self.etb_exclude_colors);

        // Parse keywords
        card.keywords = self.parse_keywords();

        // Parse abilities into effects (simplified parser for common cases)
        card.effects = self.parse_effects();

        // Parse triggered abilities
        card.triggers = self.parse_triggers();

        // Parse activated abilities
        card.activated_abilities = self.parse_activated_abilities();

        // Parse static abilities (continuous effects)
        card.static_abilities = self.parse_static_abilities();

        // Add implicit mana abilities for lands with basic land types (CR 305.6)
        // Plains, Island, Swamp, Mountain, Forest have intrinsic "{T}: Add {color}" abilities.
        // Dual lands like Volcanic Island (Island Mountain) get BOTH abilities.
        if card.is_land() && !card.activated_abilities.iter().any(|ab| ab.is_mana_ability) {
            use crate::core::{ActivatedAbility, Cost, Effect, PlayerId};

            // Check each basic land type and add corresponding mana ability
            let has_plains = self.subtypes.iter().any(|st| st.as_str() == "Plains");
            let has_island = self.subtypes.iter().any(|st| st.as_str() == "Island");
            let has_swamp = self.subtypes.iter().any(|st| st.as_str() == "Swamp");
            let has_mountain = self.subtypes.iter().any(|st| st.as_str() == "Mountain");
            let has_forest = self.subtypes.iter().any(|st| st.as_str() == "Forest");

            if has_plains {
                let mana = ManaCost::from_string("W");
                let ability = ActivatedAbility::new(
                    Cost::Tap,
                    vec![Effect::AddMana {
                        player: PlayerId::new(0),
                        mana,
                        produces_chosen_color: false,
                    }],
                    "Add {W}".to_string(),
                    true,
                );
                card.activated_abilities.push(ability);
            }
            if has_island {
                let mana = ManaCost::from_string("U");
                let ability = ActivatedAbility::new(
                    Cost::Tap,
                    vec![Effect::AddMana {
                        player: PlayerId::new(0),
                        mana,
                        produces_chosen_color: false,
                    }],
                    "Add {U}".to_string(),
                    true,
                );
                card.activated_abilities.push(ability);
            }
            if has_swamp {
                let mana = ManaCost::from_string("B");
                let ability = ActivatedAbility::new(
                    Cost::Tap,
                    vec![Effect::AddMana {
                        player: PlayerId::new(0),
                        mana,
                        produces_chosen_color: false,
                    }],
                    "Add {B}".to_string(),
                    true,
                );
                card.activated_abilities.push(ability);
            }
            if has_mountain {
                let mana = ManaCost::from_string("R");
                let ability = ActivatedAbility::new(
                    Cost::Tap,
                    vec![Effect::AddMana {
                        player: PlayerId::new(0),
                        mana,
                        produces_chosen_color: false,
                    }],
                    "Add {R}".to_string(),
                    true,
                );
                card.activated_abilities.push(ability);
            }
            if has_forest {
                let mana = ManaCost::from_string("G");
                let ability = ActivatedAbility::new(
                    Cost::Tap,
                    vec![Effect::AddMana {
                        player: PlayerId::new(0),
                        mana,
                        produces_chosen_color: false,
                    }],
                    "Add {G}".to_string(),
                    true,
                );
                card.activated_abilities.push(ability);
            }
        }

        // Add implicit Equip activated ability for Equipment with Equip keyword
        // Equipment with K:Equip:X should have an activated ability that attaches to a target creature
        if card.is_artifact() && card.subtypes.iter().any(|st| st.as_str() == "Equipment") {
            // Check if this Equipment has the Equip keyword with a cost
            if let Some(KeywordArgs::Equip { cost }) = card.keywords.get_args(Keyword::Equip) {
                use crate::core::{ActivatedAbility, CardId, Cost, Effect};

                // Create activated ability: "{equip_cost}: Attach to target creature you control"
                // The target_creature CardId will be filled in when the ability is activated
                let ability_cost = Cost::Mana(*cost);

                let effects = vec![Effect::AttachEquipment {
                    source_equipment: id,            // This Equipment
                    target_creature: CardId::new(0), // Placeholder - filled in during activation
                }];

                let description = format!("Equip {}", cost);

                // Equip is sorcery-speed (CR 702.6a: "Activate only as a sorcery")
                card.activated_abilities
                    .push(ActivatedAbility::new_sorcery_speed(ability_cost, effects, description));
            }
        }

        // Copy SVars for SubAbility resolution during effect execution
        card.svars = self.svars.clone();

        // Add Firebending attack trigger if the keyword is present
        // Firebending N: "Whenever this creature attacks, add N {R}. This mana lasts until end of combat."
        if let Some(KeywordArgs::Firebending { amount }) = card.keywords.get_args(Keyword::Firebending) {
            use crate::core::{Effect, PlayerId, Trigger, TriggerEvent};

            // Create attack trigger with Firebend effect
            // amount=0 is a sentinel for "use creature's power" (Firebending X)
            let description = if *amount == 0 {
                format!(
                    "Firebending X, where X is {}'s power (add X {{R}}, lasts until end of combat)",
                    card.name
                )
            } else {
                format!(
                    "Firebending {} (add {} {{R}}, lasts until end of combat)",
                    amount, amount
                )
            };

            let firebend_trigger = Trigger::new(
                TriggerEvent::Attacks,
                vec![Effect::Firebend {
                    controller: PlayerId::new(0), // Placeholder - resolved at runtime
                    amount: *amount,              // 0 means use creature's power
                }],
                description,
            );
            card.triggers.push(firebend_trigger);
        }

        // Update cache AFTER all abilities are parsed (including implicit mana abilities)
        // This derives mana production from Effect::AddMana in the abilities,
        // following Java Forge's approach of using structured Produced$ data.
        // Falls back to land name detection for test cards without explicit abilities.
        card.cache
            .update_from_abilities_with_name(&card.activated_abilities, card.name.as_str());

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

                // Parse keywords with parameters into strongly-typed KeywordArgs
                match kw {
                    "Madness" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Madness { cost });
                    }
                    "Flashback" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Flashback { cost });
                    }
                    "Kicker" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Kicker { cost });
                    }
                    "Cycling" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Cycling { cost });
                    }
                    "Equip" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Equip { cost });
                    }
                    "Morph" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Morph { cost });
                    }
                    "Evoke" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Evoke { cost });
                    }
                    "Buyback" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Buyback { cost });
                    }
                    "Echo" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Echo { cost });
                    }
                    "Suspend" => {
                        // Suspend format: "Suspend:3:G" -> time_counters=3, cost=G
                        if let Some((time_str, cost_str)) = param.split_once(':') {
                            if let Ok(time_counters) = time_str.trim().parse::<u8>() {
                                let cost = ManaCost::from_string(cost_str.trim());
                                keyword_set.insert_complex(KeywordArgs::Suspend { time_counters, cost });
                            }
                        }
                    }
                    "Enchant" => {
                        let card_type = Subtype::new(param);
                        keyword_set.insert_complex(KeywordArgs::Enchant { card_type });
                    }
                    "Landwalk" => {
                        let land_type = Subtype::new(param);
                        keyword_set.insert_complex(KeywordArgs::Landwalk { land_type });
                    }
                    "Affinity" => {
                        let card_type = Subtype::new(param);
                        keyword_set.insert_complex(KeywordArgs::Affinity { card_type });
                    }
                    "Protection" => {
                        let from = Subtype::new(param);
                        keyword_set.insert_complex(KeywordArgs::Protection { from });
                    }
                    "Offering" => {
                        let creature_type = Subtype::new(param);
                        keyword_set.insert_complex(KeywordArgs::Offering { creature_type });
                    }
                    "Champion" => {
                        let creature_type = Subtype::new(param);
                        keyword_set.insert_complex(KeywordArgs::Champion { creature_type });
                    }
                    "Amplify" => {
                        // Amplify format: "Amplify:2:Beast" -> amount=2, creature_type=Beast
                        if let Some((amount_str, type_str)) = param.split_once(':') {
                            if let Ok(amount) = amount_str.trim().parse::<u8>() {
                                let creature_type = Subtype::new(type_str.trim());
                                keyword_set.insert_complex(KeywordArgs::Amplify { amount, creature_type });
                            }
                        }
                    }
                    "Annihilator" => {
                        if let Ok(amount) = param.parse::<u8>() {
                            keyword_set.insert_complex(KeywordArgs::Annihilator { amount });
                        }
                    }
                    "Bushido" => {
                        if let Ok(amount) = param.parse::<u8>() {
                            keyword_set.insert_complex(KeywordArgs::Bushido { amount });
                        }
                    }
                    "Fading" => {
                        if let Ok(counters) = param.parse::<u8>() {
                            keyword_set.insert_complex(KeywordArgs::Fading { counters });
                        }
                    }
                    "Vanishing" => {
                        if let Ok(counters) = param.parse::<u8>() {
                            keyword_set.insert_complex(KeywordArgs::Vanishing { counters });
                        }
                    }
                    "Dredge" => {
                        if let Ok(amount) = param.parse::<u8>() {
                            keyword_set.insert_complex(KeywordArgs::Dredge { amount });
                        }
                    }
                    "Modular" => {
                        if let Ok(counters) = param.parse::<u8>() {
                            keyword_set.insert_complex(KeywordArgs::Modular { counters });
                        }
                    }
                    "Absorb" => {
                        if let Ok(amount) = param.parse::<u8>() {
                            keyword_set.insert_complex(KeywordArgs::Absorb { amount });
                        }
                    }
                    "Hexproof" => {
                        // HexproofFrom (e.g., "Hexproof:Blue")
                        keyword_set.insert_complex(KeywordArgs::HexproofFrom {
                            from: param.to_string(),
                        });
                    }
                    "Partner" => {
                        // PartnerWith (e.g., "Partner:Regna")
                        let card_name = CardName::new(param);
                        keyword_set.insert_complex(KeywordArgs::PartnerWith { card_name });
                    }
                    "Companion" => {
                        keyword_set.insert_complex(KeywordArgs::Companion {
                            restriction: param.to_string(),
                        });
                    }
                    // ===== COST-BASED KEYWORDS (additional) =====
                    "Aura swap" | "AuraSwap" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::AuraSwap { cost });
                    }
                    "Bestow" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Bestow { cost });
                    }
                    "Blitz" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Blitz { cost });
                    }
                    "Cumulative upkeep" | "CumulativeUpkeep" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::CumulativeUpkeep { cost });
                    }
                    "Dash" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Dash { cost });
                    }
                    "Disguise" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Disguise { cost });
                    }
                    "Disturb" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Disturb { cost });
                    }
                    "Embalm" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Embalm { cost });
                    }
                    "Encore" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Encore { cost });
                    }
                    "Entwine" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Entwine { cost });
                    }
                    "Escalate" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Escalate { cost });
                    }
                    "Escape" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Escape { cost });
                    }
                    "Eternalize" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Eternalize { cost });
                    }
                    "Foretell" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Foretell { cost });
                    }
                    "Fortify" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Fortify { cost });
                    }
                    "Freerunning" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Freerunning { cost });
                    }
                    "Harmonize" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Harmonize { cost });
                    }
                    "Level up" | "LevelUp" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::LevelUp { cost });
                    }
                    "MayFlashCost" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::MayFlashCost { cost });
                    }
                    "Megamorph" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Megamorph { cost });
                    }
                    "Miracle" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Miracle { cost });
                    }
                    "More Than Meets the Eye" | "MoreThanMeetsTheEye" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::MoreThanMeetsTheEye { cost });
                    }
                    "Multikicker" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Multikicker { cost });
                    }
                    "Mutate" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Mutate { cost });
                    }
                    "Offspring" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Offspring { cost });
                    }
                    "Outlast" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Outlast { cost });
                    }
                    "Overload" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Overload { cost });
                    }
                    "Plot" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Plot { cost });
                    }
                    "Prowl" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Prowl { cost });
                    }
                    "Prototype" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Prototype { cost });
                    }
                    "Reconfigure" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Reconfigure { cost });
                    }
                    "Reflect" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Reflect { cost });
                    }
                    "Scavenge" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Scavenge { cost });
                    }
                    "Sneak" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Sneak { cost });
                    }
                    "Specialize" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Specialize { cost });
                    }
                    "Spectacle" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Spectacle { cost });
                    }
                    "Squad" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Squad { cost });
                    }
                    "Strive" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Strive { cost });
                    }
                    "Surge" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Surge { cost });
                    }
                    "Transfigure" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Transfigure { cost });
                    }
                    "Transmute" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Transmute { cost });
                    }
                    "Unearth" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Unearth { cost });
                    }
                    "Ward" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Ward { cost });
                    }
                    "Warp" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Warp { cost });
                    }
                    "Web-slinging" | "WebSlinging" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::WebSlinging { cost });
                    }
                    // ===== AMOUNT-BASED KEYWORDS =====
                    "Afflict" => {
                        if let Ok(amount) = param.parse::<u8>() {
                            keyword_set.insert_complex(KeywordArgs::Afflict { amount });
                        }
                    }
                    "Afterlife" => {
                        if let Ok(amount) = param.parse::<u8>() {
                            keyword_set.insert_complex(KeywordArgs::Afterlife { amount });
                        }
                    }
                    "Bloodthirst" => {
                        if let Ok(amount) = param.parse::<u8>() {
                            keyword_set.insert_complex(KeywordArgs::Bloodthirst { amount });
                        }
                    }
                    "Casualty" => {
                        if let Ok(amount) = param.parse::<u8>() {
                            keyword_set.insert_complex(KeywordArgs::Casualty { amount });
                        }
                    }
                    "Crew" => {
                        if let Ok(amount) = param.parse::<u8>() {
                            keyword_set.insert_complex(KeywordArgs::Crew { amount });
                        }
                    }
                    "Fabricate" => {
                        if let Ok(amount) = param.parse::<u8>() {
                            keyword_set.insert_complex(KeywordArgs::Fabricate { amount });
                        }
                    }
                    "Frenzy" => {
                        if let Ok(amount) = param.parse::<u8>() {
                            keyword_set.insert_complex(KeywordArgs::Frenzy { amount });
                        }
                    }
                    "Graft" => {
                        if let Ok(amount) = param.parse::<u8>() {
                            keyword_set.insert_complex(KeywordArgs::Graft { amount });
                        }
                    }
                    "Hideaway" => {
                        if let Ok(amount) = param.parse::<u8>() {
                            keyword_set.insert_complex(KeywordArgs::Hideaway { amount });
                        }
                    }
                    "Mobilize" => {
                        if let Ok(amount) = param.parse::<u8>() {
                            keyword_set.insert_complex(KeywordArgs::Mobilize { amount });
                        }
                    }
                    "Poisonous" => {
                        if let Ok(amount) = param.parse::<u8>() {
                            keyword_set.insert_complex(KeywordArgs::Poisonous { amount });
                        }
                    }
                    "Rampage" => {
                        if let Ok(amount) = param.parse::<u8>() {
                            keyword_set.insert_complex(KeywordArgs::Rampage { amount });
                        }
                    }
                    "Renown" => {
                        if let Ok(amount) = param.parse::<u8>() {
                            keyword_set.insert_complex(KeywordArgs::Renown { amount });
                        }
                    }
                    "Ripple" => {
                        if let Ok(amount) = param.parse::<u8>() {
                            keyword_set.insert_complex(KeywordArgs::Ripple { amount });
                        }
                    }
                    "Saddle" => {
                        if let Ok(amount) = param.parse::<u8>() {
                            keyword_set.insert_complex(KeywordArgs::Saddle { amount });
                        }
                    }
                    "Soulshift" => {
                        if let Ok(amount) = param.parse::<u8>() {
                            keyword_set.insert_complex(KeywordArgs::Soulshift { amount });
                        }
                    }
                    "Starting intensity" | "StartingIntensity" => {
                        if let Ok(amount) = param.parse::<u8>() {
                            keyword_set.insert_complex(KeywordArgs::StartingIntensity { amount });
                        }
                    }
                    "Station" => {
                        if let Ok(amount) = param.parse::<u8>() {
                            keyword_set.insert_complex(KeywordArgs::Station { amount });
                        }
                    }
                    "Toxic" => {
                        if let Ok(amount) = param.parse::<u8>() {
                            keyword_set.insert_complex(KeywordArgs::Toxic { amount });
                        }
                    }
                    "Tribute" => {
                        if let Ok(amount) = param.parse::<u8>() {
                            keyword_set.insert_complex(KeywordArgs::Tribute { amount });
                        }
                    }
                    // ===== COST + AMOUNT KEYWORDS =====
                    "Adapt" => {
                        // Format: "Adapt:AMOUNT:COST"
                        if let Some((amount_str, cost_str)) = param.split_once(':') {
                            if let Ok(amount) = amount_str.trim().parse::<u8>() {
                                let cost = ManaCost::from_string(cost_str.trim());
                                keyword_set.insert_complex(KeywordArgs::Adapt { cost, amount });
                            }
                        }
                    }
                    "Awaken" => {
                        // Format: "Awaken:AMOUNT:COST"
                        if let Some((amount_str, cost_str)) = param.split_once(':') {
                            if let Ok(amount) = amount_str.trim().parse::<u8>() {
                                let cost = ManaCost::from_string(cost_str.trim());
                                keyword_set.insert_complex(KeywordArgs::Awaken { cost, amount });
                            }
                        }
                    }
                    "Backup" => {
                        // Format: "Backup:AMOUNT" (amount only, no cost!)
                        if let Ok(amount) = param.parse::<u8>() {
                            keyword_set.insert_complex(KeywordArgs::Backup { amount });
                        }
                    }
                    "Impending" => {
                        // Format: "Impending:AMOUNT:COST"
                        if let Some((amount_str, cost_str)) = param.split_once(':') {
                            if let Ok(amount) = amount_str.trim().parse::<u8>() {
                                let cost = ManaCost::from_string(cost_str.trim());
                                keyword_set.insert_complex(KeywordArgs::Impending { cost, amount });
                            }
                        }
                    }
                    "Monstrosity" => {
                        // Format: "Monstrosity:AMOUNT:COST"
                        if let Some((amount_str, cost_str)) = param.split_once(':') {
                            if let Ok(amount) = amount_str.trim().parse::<u8>() {
                                let cost = ManaCost::from_string(cost_str.trim());
                                keyword_set.insert_complex(KeywordArgs::Monstrosity { cost, amount });
                            }
                        }
                    }
                    "Reinforce" => {
                        // Format: "Reinforce:AMOUNT:COST"
                        if let Some((amount_str, cost_str)) = param.split_once(':') {
                            if let Ok(amount) = amount_str.trim().parse::<u8>() {
                                let cost = ManaCost::from_string(cost_str.trim());
                                keyword_set.insert_complex(KeywordArgs::Reinforce { cost, amount });
                            }
                        }
                    }
                    // ===== COST + TYPE KEYWORDS =====
                    "Splice" => {
                        // Format: "Splice:TYPE:COST"
                        if let Some((type_str, cost_str)) = param.split_once(':') {
                            let card_type = Subtype::new(type_str.trim());
                            let cost = ManaCost::from_string(cost_str.trim());
                            keyword_set.insert_complex(KeywordArgs::Splice { cost, card_type });
                        }
                    }
                    "Typecycling" | "TypeCycling" => {
                        // Format: "Typecycling:TYPE:COST" or "TypeCycling:TYPE:COST"
                        if let Some((type_str, cost_str)) = param.split_once(':') {
                            let card_type = Subtype::new(type_str.trim());
                            let cost = ManaCost::from_string(cost_str.trim());
                            keyword_set.insert_complex(KeywordArgs::Typecycling { cost, card_type });
                        }
                    }
                    // ===== TYPE-BASED KEYWORDS (additional) =====
                    "Bands with other" | "BandsWithOther" => {
                        // Format: "Bands with other:TYPE"
                        let creature_type = Subtype::new(param);
                        keyword_set.insert_complex(KeywordArgs::BandsWithOther { creature_type });
                    }
                    // ===== SPECIAL COMPLEX KEYWORDS =====
                    "Emerge" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Emerge { cost });
                    }
                    "Firebending" => {
                        // Parse amount (e.g., "1", "2", or "X" for power-based)
                        // For now, we handle numeric amounts only
                        // "X" (where X is creature's power) requires runtime evaluation
                        if let Ok(amount) = param.parse::<u8>() {
                            keyword_set.insert_complex(KeywordArgs::Firebending { amount });
                        } else if param == "X" {
                            // X = creature's power, will be resolved at runtime
                            // Use 0 as sentinel for "use creature's power"
                            keyword_set.insert_complex(KeywordArgs::Firebending { amount: 0 });
                        } else {
                            // Try to parse complex expression like "X:, where X is this creature's power"
                            // For now, use 0 (power-based) as default
                            keyword_set.insert_complex(KeywordArgs::Firebending { amount: 0 });
                        }
                    }
                    "Ninjutsu" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Ninjutsu { cost });
                    }
                    "Craft" => {
                        keyword_set.insert_complex(KeywordArgs::Craft {
                            requirements: param.to_string(),
                        });
                    }
                    "Devour" => {
                        if let Ok(amount) = param.parse::<u8>() {
                            keyword_set.insert_complex(KeywordArgs::Devour { amount });
                        }
                    }
                    // ===== SAGA AND CLASS ENCHANTMENT KEYWORDS =====
                    "Chapter" => {
                        // Format: "Chapter:3:DBCantBlock,DBSearch,DBToken"
                        if let Some((chapter_num_str, abilities_str)) = param.split_once(':') {
                            if let Ok(chapter_number) = chapter_num_str.trim().parse::<u8>() {
                                keyword_set.insert_complex(KeywordArgs::Chapter {
                                    chapter_number,
                                    abilities: abilities_str.trim().to_string(),
                                });
                            }
                        }
                    }
                    "Class" => {
                        // Format: "Class:2:W:AddTrigger$ TriggerEnter"
                        let parts: Vec<&str> = param.split(':').collect();
                        if parts.len() >= 3 {
                            if let Ok(level) = parts[0].trim().parse::<u8>() {
                                let cost = parts[1].trim().to_string();
                                let abilities = parts[2..].join(":").trim().to_string();
                                keyword_set.insert_complex(KeywordArgs::Class { level, cost, abilities });
                            }
                        }
                    }
                    // ===== ETB (ENTER THE BATTLEFIELD) KEYWORDS =====
                    "ETBReplacement" => {
                        // Format: "ETBReplacement:Copy:DBCopy:Optional"
                        if let Some((effect_type_str, details_str)) = param.split_once(':') {
                            keyword_set.insert_complex(KeywordArgs::ETBReplacement {
                                effect_type: effect_type_str.trim().to_string(),
                                details: details_str.trim().to_string(),
                            });
                        }
                    }
                    "etbCounter" => {
                        // Format: "etbCounter:P1P1:2" or "etbCounter:LOYALTY:Y:no Condition:..."
                        let parts: Vec<&str> = param.split(':').collect();
                        if parts.len() >= 2 {
                            let counter_type = parts[0].trim().to_string();
                            let amount = parts[1].trim().to_string();
                            let condition = if parts.len() > 2 {
                                parts[2..].join(":").trim().to_string()
                            } else {
                                String::new()
                            };
                            keyword_set.insert_complex(KeywordArgs::EtbCounter {
                                counter_type,
                                amount,
                                condition,
                            });
                        }
                    }
                    // ===== ADDITIONAL SPECIAL KEYWORDS =====
                    "Haunt" => {
                        keyword_set.insert_complex(KeywordArgs::Haunt {
                            trigger: param.to_string(),
                        });
                    }
                    "Replicate" => {
                        keyword_set.insert_complex(KeywordArgs::Replicate {
                            cost: param.to_string(),
                        });
                    }
                    "MayEffectFromOpeningHand" => {
                        keyword_set.insert_complex(KeywordArgs::MayEffectFromOpeningHand {
                            effect: param.to_string(),
                        });
                    }
                    "Mayhem" => {
                        keyword_set.insert_complex(KeywordArgs::Mayhem {
                            cost: param.to_string(),
                        });
                    }
                    "Recover" => {
                        let cost = ManaCost::from_string(param);
                        keyword_set.insert_complex(KeywordArgs::Recover { cost });
                    }
                    "Visit" => {
                        keyword_set.insert_complex(KeywordArgs::Visit {
                            trigger: param.to_string(),
                        });
                    }
                    "DeckLimit" => {
                        // Format: "DeckLimit:1:Description"
                        if let Some((limit_str, description)) = param.split_once(':') {
                            if let Ok(limit) = limit_str.trim().parse::<u8>() {
                                keyword_set.insert_complex(KeywordArgs::DeckLimit {
                                    limit,
                                    description: description.trim().to_string(),
                                });
                            }
                        }
                    }
                    "Dungeon" => {
                        keyword_set.insert_complex(KeywordArgs::Dungeon {
                            rooms: param.to_string(),
                        });
                    }
                    _ => {
                        // Unknown parameterized keyword - log warning
                        warn_with_context(&format!("Unknown parameterized keyword '{}' in '{}'", kw, keyword_str));
                    }
                }
            } else {
                // Simple keywords (no parameters)
                let kw = keyword_str.trim();
                match kw {
                    // ===== EVERGREEN KEYWORDS =====
                    "Flying" => keyword_set.insert(Keyword::Flying),
                    "First Strike" => keyword_set.insert(Keyword::FirstStrike),
                    "Double Strike" => keyword_set.insert(Keyword::DoubleStrike),
                    "Deathtouch" => keyword_set.insert(Keyword::Deathtouch),
                    "Haste" => keyword_set.insert(Keyword::Haste),
                    "Hexproof" => keyword_set.insert(Keyword::Hexproof),
                    "Indestructible" => keyword_set.insert(Keyword::Indestructible),
                    "Lifelink" => keyword_set.insert(Keyword::Lifelink),
                    "Menace" => keyword_set.insert(Keyword::Menace),
                    "Reach" => keyword_set.insert(Keyword::Reach),
                    "Trample" => keyword_set.insert(Keyword::Trample),
                    "Vigilance" => keyword_set.insert(Keyword::Vigilance),
                    "Defender" => keyword_set.insert(Keyword::Defender),
                    "Shroud" => keyword_set.insert(Keyword::Shroud),
                    "Flash" => keyword_set.insert(Keyword::Flash),
                    // ===== EVASION ABILITIES =====
                    "Fear" => keyword_set.insert(Keyword::Fear),
                    "Intimidate" => keyword_set.insert(Keyword::Intimidate),
                    "Horsemanship" => keyword_set.insert(Keyword::Horsemanship),
                    "Shadow" => keyword_set.insert(Keyword::Shadow),
                    "Skulk" => keyword_set.insert(Keyword::Skulk),
                    // ===== PROTECTION (specific colors) =====
                    "Protection from red" => keyword_set.insert(Keyword::ProtectionFromRed),
                    "Protection from blue" => keyword_set.insert(Keyword::ProtectionFromBlue),
                    "Protection from black" => keyword_set.insert(Keyword::ProtectionFromBlack),
                    "Protection from white" => keyword_set.insert(Keyword::ProtectionFromWhite),
                    "Protection from green" => keyword_set.insert(Keyword::ProtectionFromGreen),
                    // ===== COMBAT-RELATED =====
                    "Banding" => keyword_set.insert(Keyword::Banding),
                    "Flanking" => keyword_set.insert(Keyword::Flanking),
                    "Phasing" => keyword_set.insert(Keyword::Phasing),
                    "Wither" => keyword_set.insert(Keyword::Wither),
                    "Infect" => keyword_set.insert(Keyword::Infect),
                    // ===== KEYWORD ACTIONS/ABILITIES =====
                    "Changeling" => keyword_set.insert(Keyword::Changeling),
                    "Convoke" => keyword_set.insert(Keyword::Convoke),
                    "Delve" => keyword_set.insert(Keyword::Delve),
                    "Improvise" => keyword_set.insert(Keyword::Improvise),
                    "Split second" | "SplitSecond" => keyword_set.insert(Keyword::SplitSecond),
                    "Cascade" => keyword_set.insert(Keyword::Cascade),
                    "Storm" => keyword_set.insert(Keyword::Storm),
                    "Gravestorm" => keyword_set.insert(Keyword::Gravestorm),
                    "Conspire" => keyword_set.insert(Keyword::Conspire),
                    "Retrace" => keyword_set.insert(Keyword::Retrace),
                    "Prowess" => keyword_set.insert(Keyword::Prowess),
                    // ===== SET-SPECIFIC MECHANICS =====
                    "Aftermath" => keyword_set.insert(Keyword::Aftermath),
                    "Ascend" => keyword_set.insert(Keyword::Ascend),
                    "Assist" => keyword_set.insert(Keyword::Assist),
                    "Bargain" => keyword_set.insert(Keyword::Bargain),
                    "Battle cry" | "BattleCry" => keyword_set.insert(Keyword::BattleCry),
                    "Cipher" => keyword_set.insert(Keyword::Cipher),
                    "Compleated" => keyword_set.insert(Keyword::Compleated),
                    "Daybound" => keyword_set.insert(Keyword::Daybound),
                    "Decayed" => keyword_set.insert(Keyword::Decayed),
                    "Demonstrate" => keyword_set.insert(Keyword::Demonstrate),
                    "Dethrone" => keyword_set.insert(Keyword::Dethrone),
                    "Devoid" => keyword_set.insert(Keyword::Devoid),
                    "Double agenda" | "DoubleAgenda" => keyword_set.insert(Keyword::DoubleAgenda),
                    "Double team" | "DoubleTeam" => keyword_set.insert(Keyword::DoubleTeam),
                    "Enlist" => keyword_set.insert(Keyword::Enlist),
                    "Epic" => keyword_set.insert(Keyword::Epic),
                    "Evolve" => keyword_set.insert(Keyword::Evolve),
                    "Exalted" => keyword_set.insert(Keyword::Exalted),
                    "Exploit" => keyword_set.insert(Keyword::Exploit),
                    "Extort" => keyword_set.insert(Keyword::Extort),
                    "For Mirrodin!" | "ForMirrodin" => keyword_set.insert(Keyword::ForMirrodin),
                    "Fuse" => keyword_set.insert(Keyword::Fuse),
                    "Gift" => keyword_set.insert(Keyword::Gift),
                    "Hidden agenda" | "HiddenAgenda" => keyword_set.insert(Keyword::HiddenAgenda),
                    "Ingest" => keyword_set.insert(Keyword::Ingest),
                    "Job select" | "JobSelect" => keyword_set.insert(Keyword::JobSelect),
                    "Jump-start" | "JumpStart" => keyword_set.insert(Keyword::JumpStart),
                    "Living metal" | "LivingMetal" => keyword_set.insert(Keyword::LivingMetal),
                    "Living weapon" | "LivingWeapon" => keyword_set.insert(Keyword::LivingWeapon),
                    "Melee" => keyword_set.insert(Keyword::Melee),
                    "Mentor" => keyword_set.insert(Keyword::Mentor),
                    "Myriad" => keyword_set.insert(Keyword::Myriad),
                    "Nightbound" => keyword_set.insert(Keyword::Nightbound),
                    "Persist" => keyword_set.insert(Keyword::Persist),
                    "Provoke" => keyword_set.insert(Keyword::Provoke),
                    "Ravenous" => keyword_set.insert(Keyword::Ravenous),
                    "Read ahead" | "ReadAhead" => keyword_set.insert(Keyword::ReadAhead),
                    "Rebound" => keyword_set.insert(Keyword::Rebound),
                    "Riot" => keyword_set.insert(Keyword::Riot),
                    "Soulbond" => keyword_set.insert(Keyword::Soulbond),
                    "Space sculptor" | "SpaceSculptor" => keyword_set.insert(Keyword::SpaceSculptor),
                    "Spree" => keyword_set.insert(Keyword::Spree),
                    "Start your engines" | "StartYourEngines" => keyword_set.insert(Keyword::StartYourEngines),
                    "Sunburst" => keyword_set.insert(Keyword::Sunburst),
                    "Tiered" => keyword_set.insert(Keyword::Tiered),
                    "Training" => keyword_set.insert(Keyword::Training),
                    "Totem armor" | "UmbraArmor" => keyword_set.insert(Keyword::UmbraArmor),
                    "Undaunted" => keyword_set.insert(Keyword::Undaunted),
                    "Undying" => keyword_set.insert(Keyword::Undying),
                    "Unleash" => keyword_set.insert(Keyword::Unleash),
                    // ===== COMMANDER/MULTIPLAYER =====
                    "Choose a Background" => keyword_set.insert(Keyword::ChooseABackground),
                    "Doctor's companion" | "DoctorsCompanion" => keyword_set.insert(Keyword::DoctorsCompanion),
                    "Friends forever" | "FriendsForever" => keyword_set.insert(Keyword::FriendsForever),
                    "Partner Survivors" | "PartnerSurvivors" => keyword_set.insert(Keyword::PartnerSurvivors),
                    "Partner Father and Son" | "PartnerFatherAndSon" => {
                        keyword_set.insert(Keyword::PartnerFatherAndSon)
                    }
                    "Partner Character Select" | "PartnerCharacterSelect" => {
                        keyword_set.insert(Keyword::PartnerCharacterSelect)
                    }
                    // Partner (no arguments) - complex keyword for Java compatibility
                    "Partner" => keyword_set.insert_complex(KeywordArgs::Partner),
                    // ===== MAYFLASH VARIANTS =====
                    "MayFlashSac" => keyword_set.insert(Keyword::MayFlashSac),
                    // ===== UNTAP RELATED =====
                    "You may choose not to untap CARDNAME during your untap step."
                    | "You may choose not to untap CARDNAME during your untap step" => {
                        keyword_set.insert(Keyword::MayNotUntap)
                    }
                    _ => {
                        // Unknown simple keyword - log warning
                        warn_with_context(&format!("Unknown simple keyword '{}'", keyword_str));
                    }
                }
            }
        }

        keyword_set
    }

    /// Parse raw abilities into Effect objects
    ///
    /// Uses tokenized parsing (ability_parser) for safety and correctness.
    /// Replaces unsafe substring matching with proper parameter extraction.
    /// Follows SubAbility$ chains to resolve all effects in a spell.
    fn parse_effects(&self) -> Vec<crate::core::Effect> {
        use super::ability_parser::{AbilityParams, ApiType};
        use super::effect_converter::{params_to_charm_effect_with_svars, params_to_effect};

        let mut effects = Vec::new();

        for ability in &self.raw_abilities {
            // Skip non-spell lines (triggers, activated abilities, statics, etc.)
            // We only process A:SP$ (spell effects) here
            // Activated abilities are handled by parse_activated_abilities()
            // Triggers are handled by parse_triggers()
            // Statics are handled by parse_static_abilities()
            if !ability.starts_with("A:SP$") {
                continue;
            }

            // Parse ability string into tokenized parameters
            let params = match AbilityParams::parse(ability) {
                Ok(p) => p,
                Err(e) => {
                    // Log parse error but continue processing other abilities
                    warn_with_context(&format!("Failed to parse ability '{}': {}", ability, e));
                    continue;
                }
            };

            // Convert parameters to Effect (if supported)
            // For Charm abilities, use SVar-aware conversion to resolve mode effects
            let effect = if params.api_type == ApiType::Charm {
                params_to_charm_effect_with_svars(&params, &self.svars)
            } else {
                params_to_effect(&params)
            };

            if let Some(effect) = effect {
                effects.push(effect);
            }

            // Follow SubAbility$ chain to parse additional effects
            // Example: A:SP$ Pump | SubAbility$ DBToken creates both Pump and Token effects
            self.follow_sub_ability_chain(&params, &mut effects);
            // Note: Unsupported API types are silently skipped (returns None)
            // This is intentional - we don't want to spam warnings for every unsupported ability
        }

        effects
    }

    /// Follow SubAbility$ chain to parse additional effects
    ///
    /// When a spell has SubAbility$ param pointing to an SVar, we parse that SVar
    /// as an additional effect. This handles cards like Cunning Maneuver which has:
    /// - A:SP$ Pump | SubAbility$ DBToken
    /// - SVar:DBToken:DB$ Token | TokenScript$ c_a_clue_draw
    fn follow_sub_ability_chain(
        &self,
        params: &super::ability_parser::AbilityParams,
        effects: &mut Vec<crate::core::Effect>,
    ) {
        use super::ability_parser::AbilityParams;
        use super::effect_converter::params_to_effect;

        // Check if there's a SubAbility$ reference
        let sub_ability_name = match params.get("SubAbility") {
            Some(name) => name,
            None => return,
        };

        // Look up the SVar definition
        let svar_body = match self.svars.get(sub_ability_name) {
            Some(body) => body,
            None => {
                log::debug!(
                    target: "card_parser",
                    "SubAbility$ {} not found in SVars",
                    sub_ability_name
                );
                return;
            }
        };

        // Parse the SVar as an ability (DB$ or AB$ prefix)
        // Convert "DB$ Token | ..." to "A:DB$ Token | ..." for AbilityParams parsing
        let ability_line = format!("A:{}", svar_body);
        let sub_params = match AbilityParams::parse(&ability_line) {
            Ok(p) => p,
            Err(e) => {
                log::debug!(
                    target: "card_parser",
                    "Failed to parse SubAbility$ {} ({}): {}",
                    sub_ability_name, svar_body, e
                );
                return;
            }
        };

        // Convert to effect
        if let Some(effect) = params_to_effect(&sub_params) {
            effects.push(effect);
        }

        // Recursively follow further SubAbility chains
        self.follow_sub_ability_chain(&sub_params, effects);
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

                // Check if we have Execute$ parameter (references a SVar with effects)
                if let Some(exec_ref) = params.get("Execute").map(|s| s.to_string()) {
                    // Look up the SVar that Execute$ references
                    // Example: Execute$ TrigToken looks for "SVar:TrigToken:..."
                    for ab in &self.raw_abilities {
                        if ab.starts_with(&format!("SVar:{}:", exec_ref)) {
                            // Parse the SVar body
                            if let Some((_prefix, body)) = ab.split_once(':').and_then(|(_, rest)| rest.split_once(':'))
                            {
                                // Parse DB$ Token effects
                                // Example: "DB$ Token | TokenAmount$ 1 | TokenScript$ c_a_food_sac | TokenOwner$ You"
                                if body.contains("DB$ Token") {
                                    // Parse token parameters
                                    let mut token_script = String::new();
                                    let mut token_amount = 1u8;

                                    for param in body.split('|') {
                                        let param = param.trim();
                                        if let Some((key, value)) = param.split_once('$') {
                                            let key = key.trim();
                                            let value = value.trim();

                                            match key {
                                                "TokenScript" => {
                                                    token_script = value.to_string();
                                                }
                                                "TokenAmount" => {
                                                    if let Ok(amt) = value.parse::<u8>() {
                                                        token_amount = amt;
                                                    }
                                                }
                                                _ => {}
                                            }
                                        }
                                    }

                                    // Only add the token effect if we found a token script
                                    if !token_script.is_empty() {
                                        effects.push(Effect::CreateToken {
                                            controller: PlayerId::new(0), // Placeholder - filled at trigger time
                                            token_script,
                                            amount: token_amount,
                                        });
                                    }
                                }

                                // Parse DB$ ChangeZone effects (exile, bounce, etc.)
                                // Example: "DB$ ChangeZone | Origin$ Battlefield | Destination$ Exile | ValidTgts$ Permanent.nonLand+OppCtrl"
                                // This is used by cards like Web Up (Oblivion Ring-style effects)
                                if body.contains("DB$ ChangeZone") {
                                    let mut origin = String::new();
                                    let mut destination = String::new();

                                    for param in body.split('|') {
                                        let param = param.trim();
                                        if let Some((key, value)) = param.split_once('$') {
                                            let key = key.trim();
                                            let value = value.trim();

                                            match key {
                                                "Origin" => origin = value.to_string(),
                                                "Destination" => destination = value.to_string(),
                                                _ => {}
                                            }
                                        }
                                    }

                                    // Handle exile effects: Origin$ Battlefield + Destination$ Exile
                                    if origin == "Battlefield" && destination == "Exile" {
                                        effects.push(Effect::ExilePermanent {
                                            target: CardId::new(0), // Placeholder - filled at trigger time
                                        });
                                    }
                                }

                                // Parse AB$ Draw | Cost$ Discard<N/Card> effects (looting)
                                // Example: "AB$ Draw | Cost$ Discard<1/Card>"
                                // Used by Yuyan Archers: "you may discard a card. If you do, draw a card."
                                if (body.contains("AB$ Draw") || body.contains("DB$ Draw"))
                                    && body.contains("Cost$ Discard")
                                {
                                    // Parse NumCards$ if present, default to 1
                                    let mut draw_count = 1u8;
                                    // Parse discard count from Cost$ Discard<N/...>
                                    let mut discard_count = 1u8;

                                    for param in body.split('|') {
                                        let param = param.trim();
                                        if let Some((key, value)) = param.split_once('$') {
                                            let key = key.trim();
                                            let value = value.trim();

                                            if key == "NumCards" {
                                                if let Ok(n) = value.parse::<u8>() {
                                                    draw_count = n;
                                                }
                                            } else if key == "Cost" && value.starts_with("Discard<") {
                                                // Parse "Discard<1/Card>" format
                                                if let Some(num_str) =
                                                    value.strip_prefix("Discard<").and_then(|s| s.split('/').next())
                                                {
                                                    if let Ok(n) = num_str.parse::<u8>() {
                                                        discard_count = n;
                                                    }
                                                }
                                            }
                                        }
                                    }

                                    // Create a Loot effect (discard N to draw N)
                                    // We use Effect::Loot which represents optional looting
                                    effects.push(Effect::Loot {
                                        player: PlayerId::new(0), // Placeholder - controller
                                        discard_count,
                                        draw_count,
                                    });
                                }

                                // Parse DB$ Earthbend effects
                                // Example: "DB$ Earthbend | Num$ 2"
                                // Used by cards like Badgermole, Avatar Kyoshi
                                if body.contains("DB$ Earthbend") {
                                    let mut num_counters = 1u8;

                                    for param in body.split('|') {
                                        let param = param.trim();
                                        if let Some((key, value)) = param.split_once('$') {
                                            let key = key.trim();
                                            let value = value.trim();

                                            if key == "Num" {
                                                if let Ok(num) = value.parse::<u8>() {
                                                    num_counters = num;
                                                }
                                            }
                                        }
                                    }

                                    effects.push(Effect::Earthbend {
                                        target: CardId::new(0), // Placeholder - filled at trigger time
                                        num_counters,
                                    });
                                }
                            }
                            break;
                        }
                    }
                }

                // Extract description from TriggerDescription$ if available
                let description = params
                    .get("TriggerDescription")
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "When this enters the battlefield".to_string());

                // Note: This implements basic SVar resolution by searching across all raw_abilities
                // for effect parameters. Proper SVar resolution would parse SVar: lines separately.

                triggers.push(Trigger::new(TriggerEvent::EntersBattlefield, effects, description));
            }

            // Parse "dies" triggers (Mode$ ChangesZone with Origin$ Battlefield, Destination$ Graveyard)
            // Example: T:Mode$ ChangesZone | Origin$ Battlefield | Destination$ Graveyard | ValidCard$ Card.Self | Execute$ TrigAddMana
            if mode == Some("ChangesZone")
                && params.get("Origin").map(|s| s.as_str()) == Some("Battlefield")
                && params.get("Destination").map(|s| s.as_str()) == Some("Graveyard")
                && params.get("ValidCard").map(|s| s.as_str()) == Some("Card.Self")
            {
                use crate::core::{Effect, ManaCost, PlayerId};

                let mut effects = Vec::new();

                // Check if we have Execute$ parameter (references a SVar with effects)
                if let Some(exec_ref) = params.get("Execute").map(|s| s.to_string()) {
                    // Look up the SVar that Execute$ references
                    // Example: Execute$ TrigAddMana looks for "SVar:TrigAddMana:..."
                    for ab in &self.raw_abilities {
                        if ab.starts_with(&format!("SVar:{}:", exec_ref)) {
                            // Parse the SVar body
                            if let Some((_prefix, body)) = ab.split_once(':').and_then(|(_, rest)| rest.split_once(':'))
                            {
                                // Parse DB$ Mana effects (add mana when creature dies)
                                // Example: "DB$ Mana | Produced$ C | Amount$ 4"
                                if body.contains("DB$ Mana") {
                                    let mut produced = String::new();
                                    let mut amount = 1u32;

                                    for param in body.split('|') {
                                        let param = param.trim();
                                        if let Some((key, value)) = param.split_once('$') {
                                            let key = key.trim();
                                            let value = value.trim();

                                            match key {
                                                "Produced" => {
                                                    produced = value.to_string();
                                                }
                                                "Amount" => {
                                                    if let Ok(amt) = value.parse::<u32>() {
                                                        amount = amt;
                                                    }
                                                }
                                                _ => {}
                                            }
                                        }
                                    }

                                    // Convert Produced$ value to ManaCost
                                    // C = colorless, W/U/B/R/G = colors
                                    if !produced.is_empty() {
                                        let mana_str = match produced.as_str() {
                                            "C" => format!("{{{}}}", amount),
                                            "W" => "{W}".repeat(amount as usize),
                                            "U" => "{U}".repeat(amount as usize),
                                            "B" => "{B}".repeat(amount as usize),
                                            "R" => "{R}".repeat(amount as usize),
                                            "G" => "{G}".repeat(amount as usize),
                                            _ => format!("{{{}}}", amount), // Default to colorless
                                        };
                                        let mana = ManaCost::from_string(&mana_str);
                                        if mana.cmc() > 0 {
                                            effects.push(Effect::AddMana {
                                                player: PlayerId::new(0), // Placeholder, resolved at trigger time
                                                mana,
                                                produces_chosen_color: false,
                                            });
                                        }
                                    }
                                }
                            }
                            break;
                        }
                    }
                }

                // Extract description from TriggerDescription$ if available
                let description = params
                    .get("TriggerDescription")
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "When this creature dies".to_string());

                triggers.push(Trigger::new(TriggerEvent::LeavesBattlefield, effects, description));
            }

            // Parse phase triggers (Mode$ Phase)
            if mode == Some("Phase") {
                // Determine which phase/step this triggers on using tokenized params
                let trigger_event = match params.get("Phase").map(|s| s.as_str()) {
                    Some("Upkeep") => Some(TriggerEvent::BeginningOfUpkeep),
                    Some("EndOfTurn" | "End") => Some(TriggerEvent::BeginningOfEndStep),
                    Some("BeginCombat") => Some(TriggerEvent::BeginningOfCombat),
                    _ => None, // Other phases not supported yet
                };

                if let Some(event) = trigger_event {
                    // TODO(mtg-111): Support OptionalDecider$ for optional triggers

                    let description = params
                        .get("TriggerDescription")
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "At the beginning of upkeep".to_string());

                    // Parse effects from Execute$ SVar reference
                    let mut effects = Vec::new();

                    // Check ValidPlayer$ to determine if trigger is for "You" only
                    // "You" means only triggers on the controller's upkeep
                    let valid_player = params.get("ValidPlayer").map(|s| s.as_str());
                    let is_controller_only = valid_player == Some("You");

                    // Check if we have Execute$ parameter (references a SVar with effects)
                    if let Some(exec_ref) = params.get("Execute").map(|s| s.to_string()) {
                        // Look up the SVar that Execute$ references
                        // Example: Execute$ TrigDealDamage looks for "SVar:TrigDealDamage:..."
                        for ab in &self.raw_abilities {
                            if ab.starts_with(&format!("SVar:{}:", exec_ref)) {
                                // Parse the SVar body
                                if let Some((_prefix, body)) =
                                    ab.split_once(':').and_then(|(_, rest)| rest.split_once(':'))
                                {
                                    // Parse DB$ DealDamage effects
                                    // Example: "DB$ DealDamage | Defined$ You | NumDmg$ 1"
                                    if body.contains("DB$ DealDamage") {
                                        let mut damage_amount = 1i32;
                                        let mut target_is_controller = false;

                                        for param in body.split('|') {
                                            let param = param.trim();
                                            if let Some((key, value)) = param.split_once('$') {
                                                let key = key.trim();
                                                let value = value.trim();

                                                match key {
                                                    "NumDmg" => {
                                                        if let Ok(amt) = value.parse::<i32>() {
                                                            damage_amount = amt;
                                                        }
                                                    }
                                                    "Defined" => {
                                                        // "You" means the controller of the card
                                                        target_is_controller = value == "You";
                                                    }
                                                    _ => {}
                                                }
                                            }
                                        }

                                        // Use placeholder PlayerId(0) for controller - resolved at trigger time
                                        if target_is_controller {
                                            effects.push(Effect::DealDamage {
                                                target: TargetRef::Player(PlayerId::new(0)),
                                                amount: damage_amount,
                                            });
                                        }
                                    }

                                    // Parse DB$ GainLife effects
                                    // Example: "DB$ GainLife | Defined$ You | LifeAmount$ 1"
                                    if body.contains("DB$ GainLife") {
                                        let mut life_amount = 1i32;
                                        let mut target_is_controller = false;

                                        for param in body.split('|') {
                                            let param = param.trim();
                                            if let Some((key, value)) = param.split_once('$') {
                                                let key = key.trim();
                                                let value = value.trim();

                                                match key {
                                                    "LifeAmount" => {
                                                        if let Ok(amt) = value.parse::<i32>() {
                                                            life_amount = amt;
                                                        }
                                                    }
                                                    "Defined" => {
                                                        target_is_controller = value == "You";
                                                    }
                                                    _ => {}
                                                }
                                            }
                                        }

                                        if target_is_controller {
                                            effects.push(Effect::GainLife {
                                                player: PlayerId::new(0), // Placeholder, resolved at trigger time
                                                amount: life_amount,
                                            });
                                        }
                                    }

                                    // Parse DB$ Earthbend effects
                                    // Example: "DB$ Earthbend | Num$ 8"
                                    // Used by cards like Avatar Kyoshi (begin combat trigger)
                                    if body.contains("DB$ Earthbend") {
                                        let mut num_counters = 1u8;

                                        for param in body.split('|') {
                                            let param = param.trim();
                                            if let Some((key, value)) = param.split_once('$') {
                                                let key = key.trim();
                                                let value = value.trim();

                                                if key == "Num" {
                                                    if let Ok(num) = value.parse::<u8>() {
                                                        num_counters = num;
                                                    }
                                                }
                                            }
                                        }

                                        effects.push(Effect::Earthbend {
                                            target: CardId::new(0), // Placeholder - filled at trigger time
                                            num_counters,
                                        });
                                    }
                                }
                                break;
                            }
                        }
                    }

                    // Create trigger with parsed effects
                    // Note: is_controller_only flag is stored in description for now
                    // A proper implementation would add a field to Trigger struct
                    let desc_with_flag = if is_controller_only && !effects.is_empty() {
                        format!("[controller_only] {}", description)
                    } else {
                        description
                    };

                    triggers.push(Trigger::new(event, effects, desc_with_flag));
                }
            }

            // Parse attack triggers (Mode$ Attacks)
            // Example: T:Mode$ Attacks | ValidCard$ Card.Self | Execute$ TrigDraw | TriggerDescription$ ...
            if mode == Some("Attacks") && params.get("ValidCard").map(|s| s.as_str()) == Some("Card.Self") {
                use crate::core::{Cost, Effect, PlayerId};

                let mut effects = Vec::new();
                let mut trigger_cost: Option<Cost> = None;

                // Check if we have Execute$ parameter (references a SVar with effects)
                if let Some(exec_ref) = params.get("Execute").map(|s| s.to_string()) {
                    // Look up the SVar that Execute$ references
                    for ab in &self.raw_abilities {
                        if ab.starts_with(&format!("SVar:{}:", exec_ref)) {
                            // Parse the SVar body
                            if let Some((_prefix, body)) = ab.split_once(':').and_then(|(_, rest)| rest.split_once(':'))
                            {
                                // Extract Cost$ parameter if present (for optional triggers)
                                // Example: "Cost$ Sac<1/Artifact.Other;Creature.Other/...>"
                                for param in body.split('|') {
                                    let param = param.trim();
                                    if let Some((key, value)) = param.split_once('$') {
                                        if key.trim() == "Cost" {
                                            trigger_cost = Cost::parse(value.trim());
                                        }
                                    }
                                }

                                // Parse AB$ Draw effects (draw cards on attack)
                                // Example: "AB$ Draw | Cost$ Sac<...> | SubAbility$ DBPutCounter"
                                if body.contains("AB$ Draw") || body.contains("DB$ Draw") {
                                    // Check for NumCards parameter
                                    let mut draw_count = 1u8;
                                    for param in body.split('|') {
                                        let param = param.trim();
                                        if let Some((key, value)) = param.split_once('$') {
                                            if key.trim() == "NumCards" {
                                                if let Ok(n) = value.trim().parse::<u8>() {
                                                    draw_count = n;
                                                }
                                            }
                                        }
                                    }
                                    effects.push(Effect::DrawCards {
                                        player: PlayerId::new(0), // Placeholder, resolved at trigger time
                                        count: draw_count,
                                    });
                                }

                                // Parse SubAbility$ to follow chains (e.g., DBPutCounter)
                                let mut sub_ability_ref: Option<String> = None;
                                for param in body.split('|') {
                                    let param = param.trim();
                                    if let Some((key, value)) = param.split_once('$') {
                                        if key.trim() == "SubAbility" {
                                            sub_ability_ref = Some(value.trim().to_string());
                                        }
                                    }
                                }

                                // If there's a SubAbility, look it up and parse it
                                if let Some(sub_ref) = sub_ability_ref {
                                    for sub_ab in &self.raw_abilities {
                                        if sub_ab.starts_with(&format!("SVar:{}:", sub_ref)) {
                                            if let Some((_, sub_body)) =
                                                sub_ab.split_once(':').and_then(|(_, rest)| rest.split_once(':'))
                                            {
                                                // Parse DB$ PutCounter from SubAbility
                                                if sub_body.contains("DB$ PutCounter") {
                                                    let mut counter_num = 1u8;
                                                    for param in sub_body.split('|') {
                                                        let param = param.trim();
                                                        if let Some((key, value)) = param.split_once('$') {
                                                            if key.trim() == "CounterNum" {
                                                                if let Ok(n) = value.trim().parse::<u8>() {
                                                                    counter_num = n;
                                                                }
                                                            }
                                                        }
                                                    }
                                                    effects.push(Effect::PutCounter {
                                                        target: CardId::new(0), // Placeholder - self
                                                        counter_type: crate::core::CounterType::P1P1,
                                                        amount: counter_num,
                                                    });
                                                }
                                            }
                                            break;
                                        }
                                    }
                                }

                                // Parse DB$ PutCounter effects directly in body (for simpler cards)
                                if body.contains("DB$ PutCounter")
                                    && !effects.iter().any(|e| matches!(e, Effect::PutCounter { .. }))
                                {
                                    let mut counter_num = 1u8;
                                    for param in body.split('|') {
                                        let param = param.trim();
                                        if let Some((key, value)) = param.split_once('$') {
                                            if key.trim() == "CounterNum" {
                                                if let Ok(n) = value.trim().parse::<u8>() {
                                                    counter_num = n;
                                                }
                                            }
                                        }
                                    }
                                    effects.push(Effect::PutCounter {
                                        target: CardId::new(0), // Placeholder - self
                                        counter_type: crate::core::CounterType::P1P1,
                                        amount: counter_num,
                                    });
                                }

                                // Parse DB$ GainLife effects
                                if body.contains("DB$ GainLife") {
                                    let mut life_amount = 1i32;
                                    for param in body.split('|') {
                                        let param = param.trim();
                                        if let Some((key, value)) = param.split_once('$') {
                                            if key.trim() == "LifeAmount" {
                                                if let Ok(amt) = value.trim().parse::<i32>() {
                                                    life_amount = amt;
                                                }
                                            }
                                        }
                                    }
                                    effects.push(Effect::GainLife {
                                        player: PlayerId::new(0),
                                        amount: life_amount,
                                    });
                                }

                                // Parse DB$ DealDamage effects
                                if body.contains("DB$ DealDamage") {
                                    let mut damage_amount = 1i32;
                                    for param in body.split('|') {
                                        let param = param.trim();
                                        if let Some((key, value)) = param.split_once('$') {
                                            if key.trim() == "NumDmg" {
                                                if let Ok(amt) = value.trim().parse::<i32>() {
                                                    damage_amount = amt;
                                                }
                                            }
                                        }
                                    }
                                    effects.push(Effect::DealDamage {
                                        target: TargetRef::None, // Will need targeting
                                        amount: damage_amount,
                                    });
                                }
                            }
                            break;
                        }
                    }
                }

                // Extract description from TriggerDescription$ if available
                let description = params
                    .get("TriggerDescription")
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "Whenever this creature attacks".to_string());

                // Check if trigger is optional (has "you may" in description or OptionalDecider$)
                let is_optional =
                    description.to_lowercase().contains("you may") || params.contains_key("OptionalDecider");

                // Create appropriate trigger type based on optional and cost
                let trigger = if is_optional {
                    if let Some(cost) = trigger_cost {
                        Trigger::new_optional_with_cost(TriggerEvent::Attacks, effects, description, cost)
                    } else {
                        Trigger::new_optional(TriggerEvent::Attacks, effects, description)
                    }
                } else {
                    Trigger::new(TriggerEvent::Attacks, effects, description)
                };

                triggers.push(trigger);
            }

            // Parse SpellCast triggers (Mode$ SpellCast)
            // Example: T:Mode$ SpellCast | ValidCard$ Card.nonCreature | ValidActivatingPlayer$ You | Execute$ TrigCounter
            // This triggers when the controller casts a spell matching ValidCard$ criteria
            if mode == Some("SpellCast") {
                use crate::core::Effect;

                let mut effects = Vec::new();

                // Check ValidCard$ to determine what spells trigger this
                // Card.nonCreature = triggers on noncreature spells (instants, sorceries, etc.)
                let valid_card = params.get("ValidCard").map(|s| s.as_str());
                let is_noncreature_only = valid_card == Some("Card.nonCreature");

                // Check if we have Execute$ parameter (references a SVar with effects)
                if let Some(exec_ref) = params.get("Execute").map(|s| s.to_string()) {
                    // Look up the SVar that Execute$ references
                    for ab in &self.raw_abilities {
                        if ab.starts_with(&format!("SVar:{}:", exec_ref)) {
                            // Parse the SVar body
                            if let Some((_prefix, body)) = ab.split_once(':').and_then(|(_, rest)| rest.split_once(':'))
                            {
                                // Parse DB$ PutCounter effects (common for SpellCast triggers like Boar-q-pine)
                                if body.contains("DB$ PutCounter") {
                                    let mut counter_num = 1u8;
                                    for param in body.split('|') {
                                        let param = param.trim();
                                        if let Some((key, value)) = param.split_once('$') {
                                            if key.trim() == "CounterNum" {
                                                if let Ok(n) = value.trim().parse::<u8>() {
                                                    counter_num = n;
                                                }
                                            }
                                        }
                                    }
                                    effects.push(Effect::PutCounter {
                                        target: CardId::new(0), // Placeholder - self (Defined$ Self)
                                        counter_type: crate::core::CounterType::P1P1,
                                        amount: counter_num,
                                    });
                                }

                                // Parse DB$ Pump effects (for Prowess-like abilities)
                                if body.contains("DB$ Pump") {
                                    let mut power_bonus = 1i32;
                                    let mut toughness_bonus = 1i32;
                                    for param in body.split('|') {
                                        let param = param.trim();
                                        if let Some((key, value)) = param.split_once('$') {
                                            let key = key.trim();
                                            let value = value.trim();
                                            if key == "NumAtt" {
                                                if let Ok(n) = value.parse::<i32>() {
                                                    power_bonus = n;
                                                }
                                            } else if key == "NumDef" {
                                                if let Ok(n) = value.parse::<i32>() {
                                                    toughness_bonus = n;
                                                }
                                            }
                                        }
                                    }
                                    effects.push(Effect::PumpCreature {
                                        target: CardId::new(0), // Placeholder - self
                                        power_bonus,
                                        toughness_bonus,
                                    });
                                }
                            }
                            break;
                        }
                    }
                }

                // Extract description from TriggerDescription$ if available
                let description = params
                    .get("TriggerDescription")
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "Whenever you cast a noncreature spell".to_string());

                // SpellCast triggers are NOT self-only (they trigger on OTHER cards being cast)
                // Use new_any() to mark trigger_self_only = false
                let mut trigger = Trigger::new_any(TriggerEvent::SpellCast, effects, description);

                // Store noncreature-only flag in trigger for runtime filtering
                // We'll use a naming convention in the description for now
                if is_noncreature_only && !trigger.description.contains("noncreature") {
                    trigger.description = format!("[noncreature] {}", trigger.description);
                }

                triggers.push(trigger);
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
                    warn_with_context(&format!("Expected AB$ record type in '{}'", ability));
                    continue;
                }
                Err(e) => {
                    warn_with_context(&format!("Failed to parse activated ability '{}': {}", ability, e));
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

    /// Parse static abilities (S: lines) that create continuous effects
    ///
    /// Parses S:Mode$ Continuous lines from card data, which define Equipment bonuses,
    /// anthem effects, and other continuous effects that don't use the stack.
    ///
    /// ## Example Spider-Suit
    ///
    /// ```text
    /// S:Mode$ Continuous | Affected$ Creature.EquippedBy | AddPower$ 2 | AddToughness$ 2
    /// ```
    ///
    /// This creates a StaticAbility::ModifyPT that grants +2/+2 to the equipped creature
    /// in CR 613 Layer 7c (MODIFYPT).
    fn parse_static_abilities(&self) -> Vec<crate::core::StaticAbility> {
        use crate::core::{AffectedSelector, StaticAbility};

        /// Check if a string represents a known card type
        fn is_card_type(s: &str) -> Option<CardType> {
            match s {
                "Artifact" => Some(CardType::Artifact),
                "Land" => Some(CardType::Land),
                "Legendary" => None, // Supertype, not card type
                "Snow" => None,      // Supertype
                "Tribal" => None,    // Special
                _ => None,
            }
        }

        /// Parse power/toughness value from AddPower$/AddToughness$ parameter.
        ///
        /// Handles:
        /// - Simple integers: "2", "-1"
        /// - SVar references: "X", "Y", "Z", "-X", "AffectedX"
        /// - Count expressions: "Count$Valid..." (for counting cards)
        /// - Variable names with negation: "-AttackingX", "-NotAttackingX"
        ///
        /// SVar references indicate variable P/T that depends on game state
        /// (count of artifacts, enchantments, etc.). These are parsed as 0
        /// for now until full SVar evaluation is implemented.
        fn parse_pt_value(value: &str, param: &str, original: &str, ability: &str) -> i32 {
            // Try parsing as integer first
            if let Ok(n) = value.parse::<i32>() {
                return n;
            }

            // Check for known SVar patterns - these are variable references
            // that we accept silently (even though we return 0)
            let known_var_patterns = [
                "X",
                "Y",
                "Z",
                "-X",
                "-Y",
                "-Z",
                "AffectedX",
                "AffectedY",
                "AffectedZ",
                "-AffectedX",
                "-AffectedY",
                "-AffectedZ",
            ];
            if known_var_patterns.contains(&value) {
                // Known variable - silently return 0
                // TODO(mtg-147): Implement SVar evaluation for variable P/T
                return 0;
            }

            // Accept Count$ expressions (e.g., "Count$Valid Artifact.YouCtrl")
            // These are inline count expressions that reference game state
            if value.starts_with("Count$") {
                // TODO(mtg-147): Implement SVar Count$ expression evaluation
                return 0;
            }

            // Accept any variable name pattern (e.g., "-AttackingX", "NotAttackingY", "YourSpeed")
            // These typically reference SVars defined elsewhere in the card
            // Pattern: optional minus, then alphabetic chars (variable name)
            let trimmed = value.trim_start_matches('-');
            if !trimmed.is_empty() && trimmed.chars().all(|c| c.is_alphabetic() || c == '_') {
                // Looks like a variable reference - silently accept
                // TODO(mtg-147): Implement SVar evaluation for variable P/T
                return 0;
            }

            // Unknown pattern - warn and return 0
            warn_with_context(&format!("Failed to parse {}$ '{}' in '{}'", param, original, ability));
            0
        }

        /// Parse tribal type selector patterns
        ///
        /// Handles patterns like:
        /// - "Goblin.YouCtrl" → CreatureTypeYouControl { Goblin }
        /// - "Goblin.Other+YouCtrl" → CreatureTypeOtherYouControl { Goblin }
        /// - "Creature.Goblin+YouCtrl" → CreatureTypeYouControl { Goblin }
        /// - "Creature.Goblin+Other+YouCtrl" → CreatureTypeOtherYouControl { Goblin }
        /// - "Creature.Artifact+YouCtrl" → CreatureCardTypeYouControl { Artifact }
        /// - "Creature.Artifact+Other+YouCtrl" → CreatureCardTypeOtherYouControl { Artifact }
        /// - "Creature.Land+YouCtrl" → LandCreaturesYouControl
        /// - "Creature.nonHuman+Other+YouCtrl" → CreatureNonTypeOtherYouControl { Human }
        /// - "Sliver" → AllCreaturesOfType { Sliver }
        /// - "Creature.Sliver" → AllCreaturesOfType { Sliver }
        /// - "Permanent.Sliver" → AllCreaturesOfType { Sliver }
        ///
        /// Returns None if the pattern doesn't match a recognized format.
        fn parse_tribal_selector(value: &str) -> Option<AffectedSelector> {
            // Pattern: Bare subtype (e.g., "Sliver") - all creatures of that type globally
            // Used by Sliver lords that affect ALL Slivers, not just your own
            // Note: Only match actual creature subtypes, not card types
            if !value.contains('.') && !value.contains('+') {
                // Common creature subtypes that use this pattern
                let known_global_subtypes = [
                    "Sliver", "Eldrazi", "Ally", "Ninja", "Samurai", "Wizard", "Merfolk", "Goblin", "Dragon", "Angel",
                ];
                if known_global_subtypes.contains(&value) {
                    return Some(AffectedSelector::AllCreaturesOfType {
                        subtype: crate::core::Subtype::new(value),
                    });
                }
            }

            // Pattern: Creature.COLOR (e.g., "Creature.White") - ALL creatures of a color
            // MUST come BEFORE the generic Creature.TYPE pattern to avoid "White" being treated as subtype
            let color_names = ["White", "Blue", "Black", "Red", "Green"];
            if value.starts_with("Creature.") && !value.contains('+') {
                let color_name = value.strip_prefix("Creature.")?;
                if color_names.contains(&color_name) {
                    return Some(AffectedSelector::AllCreaturesOfColor {
                        color: color_name.to_string(),
                    });
                }
            }

            // Pattern: Creature.TYPE or Permanent.TYPE (e.g., "Creature.Sliver", "Permanent.Sliver")
            // All creatures of that type globally - used by Sliver lords
            if (value.starts_with("Creature.") || value.starts_with("Permanent."))
                && !value.contains("+YouCtrl")
                && !value.contains("+OppCtrl")
                && !value.contains("+Other")
            {
                let subtype = if value.starts_with("Creature.") {
                    value.strip_prefix("Creature.")?
                } else {
                    value.strip_prefix("Permanent.")?
                };
                // Make sure we're not matching reserved types or colors (already handled above)
                if subtype != "YouCtrl"
                    && subtype != "OppCtrl"
                    && subtype != "EnchantedBy"
                    && subtype != "EquippedBy"
                    && subtype != "AttachedBy"
                    && !color_names.contains(&subtype)
                {
                    return Some(AffectedSelector::AllCreaturesOfType {
                        subtype: crate::core::Subtype::new(subtype),
                    });
                }
            }
            // Pattern: Creature.nonTYPE+Other+YouCtrl (e.g., "Creature.nonHuman+Other+YouCtrl")
            // For cards like Mikaeus, the Unhallowed that buff non-Human creatures
            if value.starts_with("Creature.non") && value.ends_with("+Other+YouCtrl") {
                let remainder = value.strip_prefix("Creature.non")?;
                let excluded_type = remainder.strip_suffix("+Other+YouCtrl")?;
                return Some(AffectedSelector::CreatureNonTypeOtherYouControl {
                    excluded_subtype: crate::core::Subtype::new(excluded_type),
                });
            }

            // Pattern: Creature.Land+YouCtrl (land creatures you control)
            // For cards that grant abilities to animated lands
            if value == "Creature.Land+YouCtrl" {
                return Some(AffectedSelector::LandCreaturesYouControl);
            }

            // Pattern: Creature.TYPE+Other+YouCtrl (e.g., "Creature.Goblin+Other+YouCtrl" or "Creature.Artifact+Other+YouCtrl")
            // This is the most common format for tribal lords / type-based lords
            if value.starts_with("Creature.") && value.ends_with("+Other+YouCtrl") {
                let remainder = value.strip_prefix("Creature.")?;
                let type_str = remainder.strip_suffix("+Other+YouCtrl")?;

                // Check if it's a card type (like Artifact) vs a subtype (like Goblin)
                if let Some(card_type) = is_card_type(type_str) {
                    return Some(AffectedSelector::CreatureCardTypeOtherYouControl { card_type });
                }

                // Otherwise, treat as subtype (tribal)
                return Some(AffectedSelector::CreatureTypeOtherYouControl {
                    subtype: crate::core::Subtype::new(type_str),
                });
            }

            // Pattern: Creature.TYPE+YouCtrl+Other (alternate ordering)
            // e.g., "Creature.Artifact+YouCtrl+Other" - same as +Other+YouCtrl
            if value.starts_with("Creature.") && value.ends_with("+YouCtrl+Other") {
                let remainder = value.strip_prefix("Creature.")?;
                let type_str = remainder.strip_suffix("+YouCtrl+Other")?;

                // Check if it's a card type (like Artifact) vs a subtype (like Goblin)
                if let Some(card_type) = is_card_type(type_str) {
                    return Some(AffectedSelector::CreatureCardTypeOtherYouControl { card_type });
                }

                // Otherwise, treat as subtype (tribal)
                return Some(AffectedSelector::CreatureTypeOtherYouControl {
                    subtype: crate::core::Subtype::new(type_str),
                });
            }

            // Pattern: Creature.TYPE+YouCtrl (e.g., "Creature.Zombie+YouCtrl" or "Creature.Artifact+YouCtrl")
            // For cards that also buff themselves (no "Other")
            if value.starts_with("Creature.") && value.ends_with("+YouCtrl") && !value.contains("+Other") {
                let remainder = value.strip_prefix("Creature.")?;
                let type_str = remainder.strip_suffix("+YouCtrl")?;

                // Check if it's a card type (like Artifact) vs a subtype (like Goblin)
                if let Some(card_type) = is_card_type(type_str) {
                    return Some(AffectedSelector::CreatureCardTypeYouControl { card_type });
                }

                // Otherwise, treat as subtype (tribal)
                return Some(AffectedSelector::CreatureTypeYouControl {
                    subtype: crate::core::Subtype::new(type_str),
                });
            }

            // Pattern: TYPE.YouCtrl (e.g., "Goblin.YouCtrl")
            // Simpler format without "Creature." prefix
            if value.ends_with(".YouCtrl") && !value.contains('+') {
                let subtype = value.strip_suffix(".YouCtrl")?;
                // Verify it's not a generic type like "Creature" (already handled)
                if subtype != "Creature" && subtype != "Card" && subtype != "Land" {
                    return Some(AffectedSelector::CreatureTypeYouControl {
                        subtype: crate::core::Subtype::new(subtype),
                    });
                }
            }

            // Pattern: TYPE.Other+YouCtrl (e.g., "Goblin.Other+YouCtrl")
            // Simpler format without "Creature." prefix
            if value.ends_with(".Other+YouCtrl") {
                let subtype = value.strip_suffix(".Other+YouCtrl")?;
                if subtype != "Creature" && subtype != "Card" && subtype != "Land" {
                    return Some(AffectedSelector::CreatureTypeOtherYouControl {
                        subtype: crate::core::Subtype::new(subtype),
                    });
                }
            }

            // Pattern: TYPE.YouCtrl+Other (alternate ordering, e.g., "Goblin.YouCtrl+Other")
            // Same as above but with different parameter order
            if value.ends_with(".YouCtrl+Other") {
                let subtype = value.strip_suffix(".YouCtrl+Other")?;
                if subtype != "Creature" && subtype != "Card" && subtype != "Land" {
                    return Some(AffectedSelector::CreatureTypeOtherYouControl {
                        subtype: crate::core::Subtype::new(subtype),
                    });
                }
            }

            // Pattern: TYPE.token+YouCtrl (e.g., "Zombie.token+YouCtrl")
            // For token creatures of a specific type you control
            if value.ends_with(".token+YouCtrl") {
                let subtype = value.strip_suffix(".token+YouCtrl")?;
                if subtype != "Creature" && subtype != "Card" && subtype != "Land" {
                    return Some(AffectedSelector::TokenCreatureTypeYouControl {
                        subtype: crate::core::Subtype::new(subtype),
                    });
                }
            }

            // Pattern: Card.Self+counters_GE*_TYPE (e.g., "Card.Self+counters_GE8_CHARGE")
            // For cards that gain abilities when they have enough counters
            if value.starts_with("Card.Self+counters_GE") {
                let remainder = value.strip_prefix("Card.Self+counters_GE")?;
                // Parse the counter threshold and type (e.g., "8_CHARGE" -> 8, "CHARGE")
                if let Some((num_str, counter_type)) = remainder.split_once('_') {
                    if let Ok(minimum) = num_str.parse::<u32>() {
                        return Some(AffectedSelector::SelfWithCounters {
                            counter_type: counter_type.to_string(),
                            minimum,
                        });
                    }
                }
            }

            // Pattern: Card.Self+ChosenMode* (e.g., "Card.Self+ChosenModeKhans")
            // For cards with modal choices - treat as self
            if value.starts_with("Card.Self+ChosenMode") {
                return Some(AffectedSelector::Self_);
            }

            // Pattern: Creature.COLOR+Other (e.g., "Creature.Black+Other")
            // For cards that buff creatures of a specific color excluding themselves
            let color_names = ["White", "Blue", "Black", "Red", "Green"];
            for color in &color_names {
                let pattern = format!("Creature.{}+Other", color);
                if value == pattern {
                    return Some(AffectedSelector::CreatureColorOther {
                        color: color.to_string(),
                    });
                }
            }

            // Pattern: Creature.COLOR (e.g., "Creature.White") - ALL creatures of a color
            // For cards like Crusade that buff all creatures of a color (including self)
            for color in &color_names {
                let pattern = format!("Creature.{}", color);
                if value == pattern {
                    return Some(AffectedSelector::AllCreaturesOfColor {
                        color: color.to_string(),
                    });
                }
            }

            // Pattern: Creature.TYPE+Other (e.g., "Creature.Zombie+Other")
            // For cards that buff other creatures of a specific type (excluding themselves)
            if value.starts_with("Creature.") && value.ends_with("+Other") && !value.contains("+YouCtrl") {
                let remainder = value.strip_prefix("Creature.")?;
                let subtype = remainder.strip_suffix("+Other")?;
                // Don't match card types or colors (already handled)
                if !color_names.contains(&subtype) && is_card_type(subtype).is_none() {
                    return Some(AffectedSelector::CreatureTypeOther {
                        subtype: crate::core::Subtype::new(subtype),
                    });
                }
            }

            // Pattern: Card.nonLand+cmcLEX (e.g., "Card.nonLand+cmcLE3")
            // For effects that care about converted mana cost
            if value.starts_with("Card.nonLand+cmcLE") {
                let cmc_str = value.strip_prefix("Card.nonLand+cmcLE")?;
                // Handle both numeric and X values
                let max_cmc = if cmc_str == "X" {
                    0
                } else {
                    cmc_str.parse::<i32>().unwrap_or(0)
                };
                return Some(AffectedSelector::NonLandCmcLE { max_cmc });
            }

            // Pattern: TYPE.YouOwn (e.g., "Merfolk.YouOwn", "Druid.YouOwn")
            // For effects that grant flashback or let you cast from graveyard
            if value.ends_with(".YouOwn") && !value.contains('+') {
                let type_part = value.strip_suffix(".YouOwn")?;
                // Check if it's a card type (Instant, Sorcery, etc.)
                if let Some(card_type) = is_card_type(type_part) {
                    return Some(AffectedSelector::CardTypeYouOwn { card_type });
                }
                // Otherwise treat as subtype (creature type like Merfolk, Druid)
                return Some(AffectedSelector::SubtypeYouOwn {
                    subtype: crate::core::Subtype::new(type_part),
                });
            }

            // Pattern: TYPE.TopLibrary+YouCtrl (e.g., "Instant.TopLibrary+YouCtrl")
            // For effects that let you cast specific card types from top of library
            if value.ends_with(".TopLibrary+YouCtrl") && !value.contains("+nonLand") {
                let type_part = value.strip_suffix(".TopLibrary+YouCtrl")?;
                // Check if it's a card type
                if let Some(card_type) = is_card_type(type_part) {
                    return Some(AffectedSelector::CardTypeTopLibrary { card_type });
                }
                // For subtypes (creature types), use SubtypeTopLibraryNonLand
                // (most top-of-library effects for creature types are nonLand anyway)
                return Some(AffectedSelector::SubtypeTopLibraryNonLand {
                    subtype: crate::core::Subtype::new(type_part),
                });
            }

            // Pattern: TYPE.TopLibrary+YouCtrl+nonLand (e.g., "Angel.TopLibrary+YouCtrl+nonLand")
            // For effects that let you cast non-land cards of a type from top of library
            if value.ends_with(".TopLibrary+YouCtrl+nonLand") || value.ends_with(".TopLibrary+YouOwn+nonLand") {
                let suffix = if value.ends_with(".TopLibrary+YouCtrl+nonLand") {
                    ".TopLibrary+YouCtrl+nonLand"
                } else {
                    ".TopLibrary+YouOwn+nonLand"
                };
                let type_part = value.strip_suffix(suffix)?;
                return Some(AffectedSelector::SubtypeTopLibraryNonLand {
                    subtype: crate::core::Subtype::new(type_part),
                });
            }

            // Pattern: Permanent.TYPE+YouCtrl (e.g., "Permanent.Servo+YouCtrl")
            // For effects that buff all permanents of a specific subtype you control
            if value.starts_with("Permanent.") && value.ends_with("+YouCtrl") && !value.contains("+Other") {
                let remainder = value.strip_prefix("Permanent.")?;
                let subtype = remainder.strip_suffix("+YouCtrl")?;
                // Skip already-handled patterns like "Permanent.Sliver+YouCtrl"
                if subtype != "Sliver" && subtype != "nonLand" {
                    return Some(AffectedSelector::PermanentSubtypeYouControl {
                        subtype: crate::core::Subtype::new(subtype),
                    });
                }
            }

            // Pattern: Card.EquippedBy+TYPE (e.g., "Card.EquippedBy+Human")
            // For equipment that grants bonuses to specific creature types
            if value.starts_with("Card.EquippedBy+") {
                let subtype = value.strip_prefix("Card.EquippedBy+")?;
                // Skip "Legendary" which is already handled specially
                if subtype != "Legendary" {
                    return Some(AffectedSelector::EquippedBySubtype {
                        subtype: crate::core::Subtype::new(subtype),
                    });
                }
            }

            // Pattern: Instant.COLOR+YouCtrl (e.g., "Instant.Red+YouCtrl")
            // For effects that grant abilities to colored instants
            if value.starts_with("Instant.") && value.ends_with("+YouCtrl") {
                let remainder = value.strip_prefix("Instant.")?;
                let color = remainder.strip_suffix("+YouCtrl")?;
                // Only handle color patterns (Red, Green, Blue, White, Black)
                if matches!(color, "Red" | "Green" | "Blue" | "White" | "Black") {
                    return Some(AffectedSelector::InstantColorYouControl {
                        color: color.to_string(),
                    });
                }
            }

            // Pattern: Sorcery.COLOR+YouCtrl (e.g., "Sorcery.Red+YouCtrl")
            // For effects that grant abilities to colored sorceries
            if value.starts_with("Sorcery.") && value.ends_with("+YouCtrl") {
                let remainder = value.strip_prefix("Sorcery.")?;
                let color = remainder.strip_suffix("+YouCtrl")?;
                // Only handle color patterns
                if matches!(color, "Red" | "Green" | "Blue" | "White" | "Black") {
                    return Some(AffectedSelector::SorceryColorYouControl {
                        color: color.to_string(),
                    });
                }
            }

            // Pattern: Card.TopLibrary+YouCtrl+SUBTYPE (e.g., "Card.TopLibrary+YouCtrl+Bird")
            // For effects that let you play specific types from top of library
            if value.starts_with("Card.TopLibrary+YouCtrl+") {
                let subtype = value.strip_prefix("Card.TopLibrary+YouCtrl+")?;
                // Skip modifiers that aren't subtypes
                if !subtype.starts_with("with") && !subtype.starts_with("has") && !subtype.starts_with("non") {
                    return Some(AffectedSelector::TopLibraryWithSubtype {
                        subtype: crate::core::Subtype::new(subtype),
                    });
                }
            }

            None
        }

        /// Parse a single Affected$ selector value into an AffectedSelector.
        ///
        /// This combines explicit matches for known selectors with the tribal pattern parser.
        /// Returns None if the selector cannot be parsed (caller should emit warning).
        fn parse_single_affected_selector(value: &str) -> Option<AffectedSelector> {
            // First, try explicit matches for known selectors
            let selector = match value {
                "Creature.EquippedBy" | "Card.EquippedBy" => AffectedSelector::CreatureEquippedBy,
                "Creature.EnchantedBy" | "Card.EnchantedBy" | "Permanent.EnchantedBy" => {
                    AffectedSelector::CreatureEnchantedBy
                }
                "Creature.YouCtrl" => AffectedSelector::CreaturesYouControl,
                "Creature.YouCtrl+Other" | "Creature.Other+YouCtrl" => AffectedSelector::CreaturesYouControlOther,
                "Creature" => AffectedSelector::AllCreatures,
                "Card.Self" => AffectedSelector::Self_,
                "Land.AttachedBy" | "Land.EnchantedBy" => AffectedSelector::LandAttachedBy,
                "Artifact.EnchantedBy" => AffectedSelector::ArtifactEnchantedBy,
                "Planeswalker.EnchantedBy" => AffectedSelector::PlaneswalkerEnchantedBy,
                "Equipment.EnchantedBy" => AffectedSelector::EquipmentEnchantedBy,
                "Card.Self+equipped" => AffectedSelector::SelfWhenEquipped,
                "Card.Self+enchanted" => AffectedSelector::SelfWhenEnchanted,
                "Creature.YouCtrl+equipped" => AffectedSelector::EquippedCreaturesYouControl,
                "Creature.YouCtrl+enchanted" => AffectedSelector::EnchantedCreaturesYouControl,
                "You" => AffectedSelector::You,
                "Player" => AffectedSelector::Player,
                "Land.YouCtrl" => AffectedSelector::LandsYouControl,
                "Creature.OppCtrl" => AffectedSelector::CreaturesOpponentControls,
                "Card.TopLibrary+YouCtrl" => AffectedSelector::TopCardOfLibrary,
                "Creature.AttachedBy" => AffectedSelector::CreatureAttachedBy,
                "Card.AttachedBy" => AffectedSelector::CardAttachedBy,
                "Land.YouOwn" => AffectedSelector::LandsYouOwn,
                "Artifact.YouCtrl" => AffectedSelector::ArtifactsYouControl,
                "Artifact.YouCtrl+Other" | "Artifact.Other+YouCtrl" => AffectedSelector::ArtifactsYouControlOther,
                "Land" => AffectedSelector::AllLands,
                "Permanent.YouCtrl" => AffectedSelector::PermanentsYouControl,
                "Creature.token+YouCtrl" => AffectedSelector::TokenCreaturesYouControl,
                "Creature.attacking+YouCtrl" => AffectedSelector::AttackingCreaturesYouControl,
                "Creature.attacking" => AffectedSelector::AllAttackingCreatures,
                "Opponent" => AffectedSelector::Opponent,
                "Card.Self+attacking" => AffectedSelector::SelfWhenAttacking,
                // State-based self selectors
                "Card.Self+untapped" => AffectedSelector::SelfWhenUntapped,
                "Card.Self+IsMonstrous" => AffectedSelector::SelfWhenMonstrous,
                "Card.Self+ThisTurnEntered" => AffectedSelector::SelfThisTurnEntered,
                // Generic permanent and card selectors
                "Permanent" => AffectedSelector::AllPermanents,
                "Card" => AffectedSelector::AllCards,
                "Card.YouCtrl" => AffectedSelector::CardsYouControl,
                "Card.OppOwn" => AffectedSelector::CardsOpponentOwns,
                // Non-basic lands
                "Land.nonBasic" | "Land.nonBasic+YouCtrl" => AffectedSelector::NonBasicLands,
                // Human-specific equipment
                "Human.EquippedBy" => AffectedSelector::HumanEquippedBy,
                // Artifact selectors with control
                "Artifact.nonCreature+YouCtrl" => AffectedSelector::ArtifactsNonCreatureYouControl,
                "Artifact.Creature+YouCtrl+Other" | "Artifact.Creature+Other+YouCtrl" => {
                    AffectedSelector::ArtifactCreaturesYouControlOther
                }
                // Tapped/untapped state selectors for creatures
                "Creature.tapped+YouCtrl+Other" | "Creature.YouCtrl+tapped+Other" => {
                    AffectedSelector::TappedCreaturesYouControlOther
                }
                "Creature.untapped+YouCtrl+Other" | "Creature.YouCtrl+untapped+Other" => {
                    AffectedSelector::UntappedCreaturesYouControlOther
                }
                // Non-land permanents
                "Card.YouCtrl+nonLand" | "Card.nonLand+YouCtrl" => AffectedSelector::NonLandPermanentsYouControl,
                "Permanent.nonLand+YouCtrl" | "Permanent.YouCtrl+nonLand" => {
                    AffectedSelector::NonLandPermanentsYouControl
                }
                "Card.YouOwn+nonLand" | "Card.nonLand+YouOwn" => AffectedSelector::NonLandCardsYouOwn,
                // Spell types for stack effects (parsed but not yet implemented for P/T)
                "Instant" => AffectedSelector::Self_,
                "Sorcery" => AffectedSelector::Self_,
                // CardType.YouOwn selectors (for flashback and graveyard casting effects)
                "Instant.YouOwn" => AffectedSelector::CardTypeYouOwn {
                    card_type: CardType::Instant,
                },
                "Sorcery.YouOwn" => AffectedSelector::CardTypeYouOwn {
                    card_type: CardType::Sorcery,
                },
                "Enchantment.YouOwn" => AffectedSelector::CardTypeYouOwn {
                    card_type: CardType::Enchantment,
                },
                "Artifact.YouOwn" => AffectedSelector::CardTypeYouOwn {
                    card_type: CardType::Artifact,
                },
                // Subtype.YouOwn selectors (Aura, Equipment are subtypes)
                "Aura.YouOwn" => AffectedSelector::SubtypeYouOwn {
                    subtype: Subtype::new("Aura"),
                },
                "Equipment.YouOwn" => AffectedSelector::SubtypeYouOwn {
                    subtype: Subtype::new("Equipment"),
                },
                // Exile-based effects
                "Card.ExiledWithSource" => AffectedSelector::CardExiledWithSource,
                // Top of library selectors
                "Card.TopLibrary" => AffectedSelector::TopOfLibrary,
                "Land.TopLibrary+YouCtrl" => AffectedSelector::LandTopOfLibrary,
                "Creature.TopLibrary+YouCtrl+nonLand" => AffectedSelector::CreatureTopOfLibraryNonLand,
                "Card.TopLibrary+YouOwn" => AffectedSelector::TopOfLibraryYouOwn,
                "Card.TopLibrary+YouOwn+nonLand" => AffectedSelector::TopOfLibraryNonLand,
                // Commander-specific
                "Card.IsCommander+YouCtrl" => AffectedSelector::CommanderYouControl,
                // Equipment selectors
                "Card.EquippedBy+Legendary" => AffectedSelector::EquippedByLegendary,
                // Attachment selectors
                "Permanent.AttachedBy" => AffectedSelector::PermanentAttachedBy,
                "Permanent.EquippedBy" => AffectedSelector::PermanentEquippedBy,
                "Vehicle.AttachedBy" => AffectedSelector::VehicleAttachedBy,
                // Artifact selectors
                "Artifact.nonCreature" => AffectedSelector::ArtifactsNonCreature,
                "Artifact" => AffectedSelector::AllArtifacts,
                // Land selectors
                "Land.Basic+YouCtrl" => AffectedSelector::BasicLandsYouControl,
                // Basic land types
                "Mountain" => AffectedSelector::SpecificLandType {
                    land_type: "Mountain".to_string(),
                },
                "Forest" => AffectedSelector::SpecificLandType {
                    land_type: "Forest".to_string(),
                },
                "Island" => AffectedSelector::SpecificLandType {
                    land_type: "Island".to_string(),
                },
                "Plains" => AffectedSelector::SpecificLandType {
                    land_type: "Plains".to_string(),
                },
                "Swamp" => AffectedSelector::SpecificLandType {
                    land_type: "Swamp".to_string(),
                },
                // Flying/keyword-based selectors
                "Creature.withFlying+OppCtrl" => AffectedSelector::CreatureWithFlyingOppCtrl,
                // Sliver selectors
                "Permanent.Sliver+YouCtrl" => AffectedSelector::SliversYouControl,
                // Foretell selectors
                "Card.nonLand+YouOwn+withoutForetell" => AffectedSelector::NonLandCardsYouOwnWithoutForetell,
                // Remembered cards
                "Card.IsRemembered" => AffectedSelector::RememberedCards,
                // Cast-based selectors
                "Card.Creature+YouCtrl+wasCast" => AffectedSelector::CreatureYouControlWasCast,
                "Card.YouCtrl+wasCast" => AffectedSelector::CardsYouControlWasCast,
                // Treasure selectors
                "Card.Treasure+YouCtrl" => AffectedSelector::TreasuresYouControl,
                // Self on top of library
                "Card.Self+TopLibrary" => AffectedSelector::SelfTopLibrary,
                _ => {
                    // Try to parse tribal type patterns
                    return parse_tribal_selector(value);
                }
            };
            Some(selector)
        }

        let mut abilities = Vec::new();

        for ability in &self.raw_abilities {
            if !ability.starts_with("S:") {
                continue;
            }

            // Parse S:Mode$ Continuous lines
            if !ability.contains("Mode$ Continuous") {
                continue;
            }

            // Parse parameters by splitting on |
            let mut affected = AffectedSelector::Self_;
            let mut power = 0;
            let mut toughness = 0;
            let mut keyword: Option<Keyword> = None;
            let mut description = String::new();

            // Split by | and parse each parameter
            for param in ability.split('|') {
                let param = param.trim();
                if let Some((key, value)) = param.split_once('$') {
                    let key = key.trim();
                    let value = value.trim();

                    match key {
                        "Affected" => {
                            // Check for comma-separated selectors (e.g., "Creature.Zombie+Other+YouCtrl,Creature.Skeleton+YouCtrl")
                            if value.contains(',') {
                                // First, try to parse as tribal creature types (shortcut for common case)
                                // Handles both old format (TYPE.Other+YouCtrl) and new format (Creature.TYPE+Other+YouCtrl)
                                let types: Vec<Subtype> = value
                                    .split(',')
                                    .filter_map(|part| {
                                        let part = part.trim();
                                        // Pattern: Creature.TYPE+Other+YouCtrl (e.g., "Creature.Zombie+Other+YouCtrl")
                                        if part.starts_with("Creature.") && part.contains("+YouCtrl") {
                                            let remainder = part.strip_prefix("Creature.")?;
                                            // Extract the TYPE part (before any + modifier)
                                            let subtype = remainder.split('+').next()?.trim();
                                            return Some(Subtype::new(subtype));
                                        }
                                        // Pattern: TYPE.Other+YouCtrl (legacy format)
                                        if part.contains(".Other+YouCtrl") {
                                            return part.split('.').next().map(|t| Subtype::new(t.trim()));
                                        }
                                        // Pattern: TYPE.YouCtrl (no "Other" qualifier)
                                        if part.contains(".YouCtrl") && !part.contains("+Other") {
                                            return part.split('.').next().map(|t| Subtype::new(t.trim()));
                                        }
                                        None
                                    })
                                    .collect();

                                if !types.is_empty() {
                                    // Pure tribal shortcut - all parts matched creature types
                                    affected = AffectedSelector::CreatureTypesOtherYouControl { types };
                                } else {
                                    // Fallback: Parse each part as an individual selector and wrap in Any
                                    // This handles complex OR patterns like:
                                    // - "Goblin.YouCtrl+Other,Orc.YouCtrl+Other"
                                    // - "Instant,Sorcery"
                                    // - "Creature.PairedWith,Creature.Self+Paired"
                                    let selectors: Vec<AffectedSelector> = value
                                        .split(',')
                                        .filter_map(|part| parse_single_affected_selector(part.trim()))
                                        .collect();

                                    if selectors.len() == 1 {
                                        // Single selector parsed - use it directly
                                        affected = selectors.into_iter().next().unwrap();
                                    } else if selectors.len() > 1 {
                                        // Multiple selectors parsed - wrap in Any (OR)
                                        affected = AffectedSelector::Any(selectors);
                                    } else {
                                        // No selectors could be parsed - emit warning
                                        warn_with_context(&format!(
                                            "Failed to parse comma-separated Affected$ selector '{}' in '{}'",
                                            value, ability
                                        ));
                                        affected = AffectedSelector::Self_;
                                    }
                                }
                            } else {
                                // Single selector - use the unified parser
                                affected = if let Some(parsed) = parse_single_affected_selector(value) {
                                    parsed
                                } else {
                                    warn_with_context(&format!(
                                        "Unknown Affected$ selector '{}' in '{}'",
                                        value, ability
                                    ));
                                    AffectedSelector::Self_
                                };
                            }
                        }
                        "AddPower" => {
                            // Remove leading + if present, then parse
                            let value_trimmed = value.trim_start_matches('+');
                            power = parse_pt_value(value_trimmed, "AddPower", value, ability);
                        }
                        "AddToughness" => {
                            // Remove leading + if present, then parse
                            let value_trimmed = value.trim_start_matches('+');
                            toughness = parse_pt_value(value_trimmed, "AddToughness", value, ability);
                        }
                        "Description" => {
                            description = value.to_string();
                        }
                        "AddKeyword" => {
                            // Parse keyword name to Keyword enum
                            // Handle both single keywords and &-separated keywords
                            // (e.g., "Flying" or "Flying & Vigilance")
                            let keyword_str = value.split('&').next().unwrap_or(value).trim();
                            match Keyword::from_string(keyword_str) {
                                Some(k) => keyword = Some(k),
                                None => {
                                    // Some keywords may not be implemented yet
                                    if !keyword_str.is_empty() {
                                        // Only warn for non-empty keywords
                                        // Note: Many cards use AddKeyword$ with complex values we don't support yet
                                    }
                                }
                            }
                        }
                        _ => {} // Ignore other parameters (e.g., AddType$, AddAbility$)
                    }
                }
            }

            // Create the ability based on what was parsed
            if power != 0 || toughness != 0 {
                // P/T modification ability
                abilities.push(StaticAbility::ModifyPT {
                    affected: affected.clone(),
                    power,
                    toughness,
                    description: description.clone(),
                });
            }

            if let Some(kw) = keyword {
                // Keyword grant ability
                abilities.push(StaticAbility::GrantKeyword {
                    affected,
                    keyword: kw,
                    description,
                });
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
        let Effect::DealDamage { target, amount } = &effects[0] else {
            panic!("Expected DealDamage effect, got {:?}", effects[0]);
        };
        assert_eq!(*amount, 3, "Should deal 3 damage");
        assert!(matches!(target, TargetRef::None), "Target should be None initially");
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
        let Effect::DrawCards { player: _, count } = &effects[0] else {
            panic!("Expected DrawCards effect, got {:?}", effects[0]);
        };
        assert_eq!(*count, 3, "Should draw 3 cards");
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
        let Effect::DestroyPermanent { target: _, .. } = &effects[0] else {
            panic!("Expected DestroyPermanent effect, got {:?}", effects[0]);
        };
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
        let Effect::GainLife { player: _, amount } = &effects[0] else {
            panic!("Expected GainLife effect, got {:?}", effects[0]);
        };
        assert_eq!(*amount, 7, "Should gain 7 life");
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
        let Effect::DealDamage { target: _, amount } = &ability.effects[0] else {
            panic!("Expected DealDamage effect, got {:?}", ability.effects[0]);
        };
        assert_eq!(*amount, 1, "Should deal 1 damage");
    }

    #[test]
    fn test_parse_affected_you_selector() {
        // Test parsing of Affected$ You selector
        // Using Aegis of the Gods: "You have hexproof"
        let content = r#"
Name:Aegis of the Gods
ManaCost:1 W
Types:Enchantment Creature Human Soldier
PT:2/1
S:Mode$ Continuous | Affected$ You | AddKeyword$ Hexproof | Description$ You have hexproof.
Oracle:You have hexproof. (You can't be the target of spells or abilities your opponents control.)
"#;

        let def = CardLoader::parse(content).unwrap();
        assert_eq!(def.name.as_str(), "Aegis of the Gods");

        // Check that the static ability is parsed with You selector
        let abilities = def.parse_static_abilities();
        assert!(!abilities.is_empty(), "Should have static abilities");

        use crate::core::effects::AffectedSelector;
        use crate::core::StaticAbility;

        // Should have a GrantKeyword ability with Affected$ You
        let has_you_selector = abilities.iter().any(|ability| {
            if let StaticAbility::GrantKeyword { affected, .. } = ability {
                matches!(affected, AffectedSelector::You)
            } else {
                false
            }
        });
        assert!(has_you_selector, "Should have GrantKeyword with You selector");
    }

    #[test]
    fn test_parse_affected_land_youctrl_selector() {
        // Test parsing of Affected$ Land.YouCtrl selector
        // Using Chromatic Lantern: "Lands you control have mana ability"
        let content = r#"
Name:Chromatic Lantern
ManaCost:3
Types:Artifact
S:Mode$ Continuous | Affected$ Land.YouCtrl | AddAbility$ AnyMana | Description$ Lands you control have "{T}: Add one mana of any color."
SVar:AnyMana:AB$ Mana | Cost$ T | Produced$ Any | Amount$ 1 | SpellDescription$ Add one mana of any color.
A:AB$ Mana | Cost$ T | Produced$ Any | SpellDescription$ Add one mana of any color.
Oracle:Lands you control have "{T}: Add one mana of any color."
"#;

        let def = CardLoader::parse(content).unwrap();
        assert_eq!(def.name.as_str(), "Chromatic Lantern");

        // Check that the static ability is parsed without warning
        // (warning happens if selector is unknown and falls through to Self_)
        let abilities = def.parse_static_abilities();

        // Note: AddAbility$ is not the same as AddKeyword$, so we won't have a GrantKeyword
        // But the point is that the selector is parsed correctly
        // We can verify by re-parsing with debug output or by not seeing warnings
        // For now, we just verify the card parses without panic
        let _ = abilities;
    }

    #[test]
    fn test_parse_affected_creature_oppctrl_selector() {
        // Test parsing of Affected$ Creature.OppCtrl selector
        // Using a mock card definition
        let content = r#"
Name:Test Debuff Lord
ManaCost:2 B
Types:Creature Zombie
PT:2/2
S:Mode$ Continuous | Affected$ Creature.OppCtrl | AddPower$ -1 | AddToughness$ -1 | Description$ Creatures your opponents control get -1/-1.
Oracle:Creatures your opponents control get -1/-1.
"#;

        let def = CardLoader::parse(content).unwrap();
        assert_eq!(def.name.as_str(), "Test Debuff Lord");

        // Check that the static ability is parsed with OppCtrl selector
        let abilities = def.parse_static_abilities();
        assert!(!abilities.is_empty(), "Should have static abilities");

        use crate::core::effects::AffectedSelector;
        use crate::core::StaticAbility;

        // Should have a ModifyPT ability with Affected$ Creature.OppCtrl
        let has_oppctrl_selector = abilities.iter().any(|ability| {
            if let StaticAbility::ModifyPT {
                affected,
                power,
                toughness,
                ..
            } = ability
            {
                matches!(affected, AffectedSelector::CreaturesOpponentControls) && *power == -1 && *toughness == -1
            } else {
                false
            }
        });
        assert!(
            has_oppctrl_selector,
            "Should have ModifyPT with CreaturesOpponentControls selector"
        );
    }

    #[test]
    fn test_parse_affected_player_selector() {
        // Test parsing of Affected$ Player selector
        let content = r#"
Name:Test Symmetrical Effect
ManaCost:2 W W
Types:Enchantment
S:Mode$ Continuous | Affected$ Player | AddKeyword$ Hexproof | Description$ Each player has hexproof.
Oracle:Each player has hexproof.
"#;

        let def = CardLoader::parse(content).unwrap();
        assert_eq!(def.name.as_str(), "Test Symmetrical Effect");

        // Check that the static ability is parsed with Player selector
        let abilities = def.parse_static_abilities();
        assert!(!abilities.is_empty(), "Should have static abilities");

        use crate::core::effects::AffectedSelector;
        use crate::core::StaticAbility;

        // Should have a GrantKeyword ability with Affected$ Player
        let has_player_selector = abilities.iter().any(|ability| {
            if let StaticAbility::GrantKeyword { affected, .. } = ability {
                matches!(affected, AffectedSelector::Player)
            } else {
                false
            }
        });
        assert!(has_player_selector, "Should have GrantKeyword with Player selector");
    }

    #[test]
    fn test_parse_attack_trigger() {
        use crate::core::TriggerEvent;

        // Test parsing Mode$ Attacks triggers (like Beetle-Headed Merchants)
        let content = r#"
Name:Test Attack Trigger Creature
ManaCost:4 B
Types:Creature Human Citizen
PT:5/4
T:Mode$ Attacks | ValidCard$ Card.Self | Execute$ TrigDraw | TriggerDescription$ Whenever this creature attacks, draw a card.
SVar:TrigDraw:DB$ Draw | NumCards$ 1
Oracle:Whenever this creature attacks, draw a card.
"#;

        let def = CardLoader::parse(content).unwrap();
        let triggers = def.parse_triggers();

        // Verify the attack trigger was parsed
        assert_eq!(triggers.len(), 1, "Should have one trigger");

        let trigger = &triggers[0];
        assert_eq!(
            trigger.event,
            TriggerEvent::Attacks,
            "Trigger should be on attacks event"
        );
        assert!(
            trigger.description.contains("attacks"),
            "Description should mention attacks"
        );
    }

    #[test]
    fn test_parse_attack_trigger_with_put_counter() {
        use crate::core::{Effect, TriggerEvent};

        // Test attack trigger that puts counters (similar to Beetle-Headed Merchants' effect)
        let content = r#"
Name:Test Counter on Attack
ManaCost:3 G
Types:Creature Beast
PT:3/3
T:Mode$ Attacks | ValidCard$ Card.Self | Execute$ TrigPutCounter | TriggerDescription$ Whenever this creature attacks, put a +1/+1 counter on it.
SVar:TrigPutCounter:DB$ PutCounter | CounterType$ P1P1 | CounterNum$ 1
Oracle:Whenever this creature attacks, put a +1/+1 counter on it.
"#;

        let def = CardLoader::parse(content).unwrap();
        let triggers = def.parse_triggers();

        // Verify the attack trigger was parsed
        assert_eq!(triggers.len(), 1, "Should have one trigger");

        let trigger = &triggers[0];
        assert_eq!(trigger.event, TriggerEvent::Attacks);

        // Verify it has a PutCounter effect
        let has_put_counter = trigger.effects.iter().any(|e| matches!(e, Effect::PutCounter { .. }));
        assert!(has_put_counter, "Trigger should have PutCounter effect");
    }

    #[test]
    fn test_parse_optional_attack_trigger_with_sacrifice_cost() {
        use crate::core::{Cost, Effect, TriggerEvent};

        // Test Beetle-Headed Merchants style trigger:
        // "Whenever this creature attacks, you may sacrifice another creature or artifact.
        //  If you do, draw a card and put a +1/+1 counter on this creature."
        let content = r#"
Name:Beetle-Headed Merchants
ManaCost:4 B
Types:Creature Human Citizen
PT:5/4
T:Mode$ Attacks | ValidCard$ Card.Self | Execute$ TrigDraw | TriggerDescription$ Whenever this creature attacks, you may sacrifice another creature or artifact. If you do, draw a card and put a +1/+1 counter on this creature.
SVar:TrigDraw:AB$ Draw | Cost$ Sac<1/Artifact.Other;Creature.Other/another creature or artifact> | SubAbility$ DBPutCounter
SVar:DBPutCounter:DB$ PutCounter | CounterType$ P1P1 | CounterNum$ 1
Oracle:Whenever this creature attacks, you may sacrifice another creature or artifact. If you do, draw a card and put a +1/+1 counter on this creature.
"#;

        let def = CardLoader::parse(content).unwrap();
        let triggers = def.parse_triggers();

        // Verify the attack trigger was parsed
        assert_eq!(triggers.len(), 1, "Should have one trigger");

        let trigger = &triggers[0];
        assert_eq!(trigger.event, TriggerEvent::Attacks);

        // Verify it's marked as optional (because of "you may" in description)
        assert!(
            trigger.optional,
            "Trigger should be optional due to 'you may' in description"
        );

        // Verify it has a sacrifice cost
        assert!(trigger.cost.is_some(), "Trigger should have a sacrifice cost");
        if let Some(ref cost) = trigger.cost {
            assert!(cost.requires_sacrifice(), "Cost should require sacrifice");
            // Verify it's a SacrificePattern cost
            if let Cost::SacrificePattern { count, card_type } = cost {
                assert_eq!(*count, 1, "Should sacrifice 1 permanent");
                assert!(
                    card_type.contains("Artifact") || card_type.contains("Creature"),
                    "Card type should include Artifact or Creature"
                );
            }
        }

        // Verify it has both Draw and PutCounter effects
        let has_draw = trigger.effects.iter().any(|e| matches!(e, Effect::DrawCards { .. }));
        let has_put_counter = trigger.effects.iter().any(|e| matches!(e, Effect::PutCounter { .. }));
        assert!(has_draw, "Trigger should have DrawCards effect");
        assert!(has_put_counter, "Trigger should have PutCounter effect");
    }

    #[test]
    fn test_parse_non_optional_attack_trigger() {
        use crate::core::TriggerEvent;

        // Test a mandatory attack trigger (no "you may")
        let content = r#"
Name:Mandatory Attack Trigger
ManaCost:2 R
Types:Creature Warrior
PT:3/2
T:Mode$ Attacks | ValidCard$ Card.Self | Execute$ TrigDamage | TriggerDescription$ Whenever this creature attacks, it deals 1 damage to each opponent.
SVar:TrigDamage:DB$ DealDamage | NumDmg$ 1
Oracle:Whenever this creature attacks, it deals 1 damage to each opponent.
"#;

        let def = CardLoader::parse(content).unwrap();
        let triggers = def.parse_triggers();

        assert_eq!(triggers.len(), 1, "Should have one trigger");

        let trigger = &triggers[0];
        assert_eq!(trigger.event, TriggerEvent::Attacks);

        // Verify it's NOT optional (no "you may")
        assert!(!trigger.optional, "Trigger should NOT be optional - it's mandatory");

        // Verify it has no cost
        assert!(trigger.cost.is_none(), "Mandatory trigger should have no cost");
    }
}
