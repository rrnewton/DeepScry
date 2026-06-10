//! Card file loader (.txt format)
//!
//! Loads card definitions from Forge's cardsfolder format

use crate::core::{
    Card, CardCache, CardId, CardName, CardType, Color, Effect, Keyword, KeywordArgs, KeywordSet, ManaCost, PlayerId,
    Subtype, TargetRef, Trigger, TriggerEvent,
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

/// Convert a number to its ordinal form (1st, 2nd, 3rd, etc.)
fn ordinal(n: u8) -> String {
    let suffix = match n % 10 {
        1 if n % 100 != 11 => "st",
        2 if n % 100 != 12 => "nd",
        3 if n % 100 != 13 => "rd",
        _ => "th",
    };
    format!("{}{}", n, suffix)
}

/// Tokenize a card-script clause body into a `key -> value` map by splitting on
/// `|` (parameters) then `$` (key/value). This is the structured-parse path
/// required by CLAUDE.md ("NO HACKY STRING OPERATIONS ON STRUCTURED DATA"):
/// callers query the resulting map (`params.get("Mode")`) instead of doing
/// substring matching on the raw body. The body is the portion AFTER the
/// `S:`/`T:`/SVar prefix.
pub(crate) fn tokenize_pipe_dollar(body: &str) -> std::collections::HashMap<String, String> {
    let mut params = std::collections::HashMap::new();
    for param in body.split('|') {
        let param = param.trim();
        if param.is_empty() {
            continue;
        }
        if let Some((key, value)) = param.split_once('$') {
            params.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    params
}

/// Card loader for .txt files
pub struct CardLoader;

impl CardLoader {
    /// Load a card from a .txt file
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or parsed.
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
    ///
    /// # Errors
    ///
    /// Returns an error if the card definition cannot be parsed.
    pub fn parse_with_context(content: &str, file_context: Option<&str>) -> Result<CardDefinition> {
        set_parsing_context(file_context);
        let result = Self::parse(content);
        set_parsing_context(None);
        result
    }

    /// Parse a card from its text content
    ///
    /// # Errors
    ///
    /// Returns an error if the card definition is incomplete or malformed.
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
        let mut etb_tapped_global: Option<crate::core::effects::TargetRestriction> = None;
        let mut etb_choose_color = false;
        let mut etb_exclude_colors = Vec::new();
        // SVar name referenced by a `K:ETBReplacement:Other:<SVar>` line whose
        // body is a `DB$ ChoosePlayer` (Black Vise's `ChooseP`). Resolved to the
        // `etb_choose_player` flag AFTER all SVars are parsed (so we can confirm
        // the referenced SVar's api_type structurally rather than string-matching
        // the K-line text).
        let mut etb_choose_player_svar: Option<String> = None;
        let mut is_legendary = false;
        let mut loyalty: Option<u8> = None;

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
                                "Legendary" => is_legendary = true, // Supertype (MTG CR 205.4a)
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
                    "Loyalty" => {
                        loyalty = value.trim().parse().ok();
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
                        // ETB replacement that references an SVar (e.g. Black Vise's
                        // `K:ETBReplacement:Other:ChooseP`). The third `:`-token is
                        // the SVar name to execute as the replacement. Record it and
                        // confirm it is a `DB$ ChoosePlayer` once SVars are parsed —
                        // we do NOT string-match "ChoosePlayer" against the K-line.
                        if let Some(rest) = value.strip_prefix("ETBReplacement:") {
                            // rest = "Other:ChooseP" → take the LAST `:`-segment as
                            // the SVar name (the middle is the replacement layer).
                            if let Some(svar_name) = rest.rsplit(':').next() {
                                let svar_name = svar_name.trim();
                                if !svar_name.is_empty() && svar_name != "ChooseColor" {
                                    etb_choose_player_svar = Some(svar_name.to_string());
                                }
                            }
                        }
                    }
                    // Ability lines (A:, S:, T:, R: lines)
                    // R: (replacement effects) are retained so
                    // parse_static_abilities can lower untap-prevention locks
                    // (`R:Event$ Untap | Layer$ CantHappen`) into a continuous
                    // GrantKeyword(DoesNotUntap) static (Paralyze, Exhaustion, ...).
                    "A" | "S" | "T" | "R" => {
                        // ETB-tapped replacement (`ReplaceWith$ ETBTapped`).
                        // Classify the replacement STRUCTURALLY (tokenize on `|`
                        // then `$`, never substring-match the line — see the
                        // "No Hacky String Operations" rule):
                        //   - `ValidCard$ Card.Self` → the host itself enters
                        //     tapped (the tapped-land form; `enters_tapped` flag).
                        //   - a global predicate (`Creature.OppCtrl`, …) → OTHER
                        //     permanents matching it enter tapped while the host
                        //     is on the battlefield (Kismet et al.).
                        if key == "R" {
                            match classify_etb_tapped_replacement(value) {
                                EtbTappedReplacement::SelfHost => enters_tapped = true,
                                EtbTappedReplacement::Global(pred) => etb_tapped_global = Some(pred),
                                EtbTappedReplacement::NotApplicable => {}
                            }
                        }
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

        // Pre-parse all SVars once at load time for efficient lookup during trigger construction
        use super::ability_parser::AbilityParams;
        let mut parsed_svars = std::collections::HashMap::new();
        for (svar_name, svar_body) in &svars {
            if let Some(params) = AbilityParams::parse_svar_body(svar_body) {
                parsed_svars.insert(svar_name.clone(), params);
            }
        }

        // Resolve the ETB ChoosePlayer replacement: the K-line referenced an
        // SVar (e.g. ChooseP); confirm its parsed api_type is ChoosePlayer
        // before flagging (structured check, not a string match on the K-line).
        let etb_choose_player = etb_choose_player_svar
            .as_deref()
            .and_then(|svar_name| parsed_svars.get(svar_name))
            .is_some_and(|p| p.api_type == super::ability_parser::ApiType::ChoosePlayer);

        // Build cache BEFORE constructing struct (avoids borrow-after-move)
        let cache = CardCache::new(&oracle, name.as_str());

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
            parsed_svars,
            enters_tapped,
            etb_tapped_global,
            etb_choose_color,
            etb_exclude_colors,
            etb_choose_player,
            script_name: None, // Set by token loader
            is_legendary,
            loyalty,
            cache,
            // Stamped post-parse by CardDatabase (native) or the WASM exporter,
            // both from the editions/ data. The parser has no set context.
            origin_set: None,
        })
    }
}

/// Serialize a `HashMap<String, String>` in deterministic (sorted-key) order.
///
/// Used for `CardDefinition::svars` so identical card data always serializes
/// to identical bytes — a prerequisite for the content-addressed WASM export
/// pipeline (mtg-571), where the on-disk filename is a hash of these bytes.
/// `serde_map` collects nothing extra: it borrows each entry and feeds the
/// serializer in `BTreeMap` order, so there is no clone of the values.
fn serialize_svars_sorted<S>(
    svars: &std::collections::HashMap<String, String>,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeMap;
    let ordered: std::collections::BTreeMap<&String, &String> = svars.iter().collect();
    let mut map = serializer.serialize_map(Some(ordered.len()))?;
    for (k, v) in ordered {
        map.serialize_entry(k, v)?;
    }
    map.end()
}

/// Classification of an `R:Event$ Moved | … | ReplaceWith$ ETBTapped`
/// replacement line, produced by [`classify_etb_tapped_replacement`]. See
/// [`CardDefinition::etb_tapped_global`].
enum EtbTappedReplacement {
    /// `ValidCard$ Card.Self`: the host itself enters tapped (tapped lands).
    SelfHost,
    /// Global: OTHER permanents matching the predicate enter tapped while the
    /// host is on the battlefield (Kismet, Loxodon Gatekeeper, Orb of Dreams, …).
    Global(crate::core::effects::TargetRestriction),
    /// Not an ETB-tapped replacement, or one whose qualifiers/conditions we
    /// don't model yet — left as a no-op rather than shipping a wrong effect.
    NotApplicable,
}

/// Classify an `R:` line body STRUCTURALLY (tokenize on `|` then `$`, never a
/// substring match — see the "No Hacky String Operations" rule in CLAUDE.md).
fn classify_etb_tapped_replacement(value: &str) -> EtbTappedReplacement {
    let mut params: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for token in value.split('|') {
        if let Some((k, v)) = token.split_once('$') {
            params.insert(k.trim(), v.trim());
        }
    }
    // Must be THE ETB-tapped zone-change replacement.
    if params.get("Event") != Some(&"Moved") || params.get("ReplaceWith") != Some(&"ETBTapped") {
        return EtbTappedReplacement::NotApplicable;
    }
    // Destination, when stated, must be the battlefield.
    if matches!(params.get("Destination"), Some(&d) if d != "Battlefield") {
        return EtbTappedReplacement::NotApplicable;
    }
    let Some(&valid_card) = params.get("ValidCard") else {
        return EtbTappedReplacement::NotApplicable;
    };
    // Self-replacement form (the hundreds of tapped lands): the host taps itself.
    if valid_card == "Card.Self" {
        return EtbTappedReplacement::SelfHost;
    }
    // Global form. We only install predicates we can faithfully evaluate:
    //   * the source must apply from the battlefield (`ActiveZones$ Battlefield`
    //     or unspecified) — `the_doctors_childhood_barn` applies from Command;
    //   * no `IsPresent$` / `ValidCause$` conditional gating (archelos_lagoon_
    //     mystic, uphill_battle);
    //   * every `ValidCard$` qualifier must be a controller restriction we model
    //     (`OppCtrl`/`YouCtrl`). Qualifiers like `nonBasic`, `Snow`,
    //     `nonPhyrexian`, `cmcNotChosenEvenOdd` are silently DROPPED by
    //     `TargetRestriction::parse`, which would WIDEN the match (e.g. also tap
    //     basic lands / your own creatures) — so we refuse them rather than ship
    //     a wrong effect. Those cards stay no-ops; see mtg-713 B12 follow-up.
    if params.contains_key("IsPresent") || params.contains_key("ValidCause") {
        return EtbTappedReplacement::NotApplicable;
    }
    if matches!(params.get("ActiveZones"), Some(&z) if z != "Battlefield") {
        return EtbTappedReplacement::NotApplicable;
    }
    if !etb_tapped_predicate_is_supported(valid_card) {
        return EtbTappedReplacement::NotApplicable;
    }
    EtbTappedReplacement::Global(crate::core::effects::TargetRestriction::parse(valid_card))
}

/// True iff every `.`-qualifier in a comma-separated `ValidCard$` predicate is a
/// controller restriction that [`crate::core::effects::TargetRestriction::parse`]
/// models. The base type itself is unrestricted (card types, universal selectors
/// `Permanent`/`Card`, and bare subtypes are all faithfully handled); only
/// trailing qualifiers can silently widen the match, so those are what we check.
fn etb_tapped_predicate_is_supported(valid_card: &str) -> bool {
    for clause in valid_card.split(',') {
        for qualifier in clause.split('.').skip(1).flat_map(|q| q.split('+')) {
            if !matches!(qualifier.trim(), "OppCtrl" | "YouCtrl") {
                return false;
            }
        }
    }
    true
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
    ///
    /// DETERMINISM (mtg-571): serialized in sorted-key order via
    /// `serialize_svars_sorted`. A plain `HashMap` serializes its entries in
    /// run-to-run RANDOM iteration order, which made the WASM per-set `.bin`
    /// bytes (and thus their content-addressed blake3 filenames) unstable
    /// across exports. We keep the runtime type a `HashMap` (O(1) SVar lookups
    /// in hot paths, ~20 call sites unchanged) but emit a deterministic byte
    /// stream. bincode's map wire format is order-independent on read, so the
    /// `HashMap` deserializer reads the sorted output transparently.
    #[serde(serialize_with = "serialize_svars_sorted")]
    pub svars: std::collections::HashMap<String, String>,
    /// Pre-parsed SVars for efficient lookup during trigger/ability construction
    /// Key: SVar name, Value: Parsed AbilityParams
    /// Populated once at card load time, avoiding repeated parsing
    #[serde(skip)]
    pub parsed_svars: std::collections::HashMap<String, super::ability_parser::AbilityParams>,
    /// Does this card enter the battlefield tapped?
    /// Derived from an `R:Event$ Moved | ValidCard$ Card.Self | ... |
    /// ReplaceWith$ ETBTapped` self-replacement (the form used by hundreds of
    /// tapped lands).
    pub enters_tapped: bool,
    /// While this permanent is on the battlefield, OTHER permanents matching this
    /// predicate enter the battlefield tapped. Derived from the *global* form of
    /// the ETB-tapped replacement — `R:Event$ Moved | ValidCard$ <pred> |
    /// Destination$ Battlefield | ReplaceWith$ ETBTapped` where `<pred>` is NOT
    /// `Card.Self` (Kismet, Loxodon Gatekeeper, Frozen Aether, Imposing
    /// Sovereign, Authority of the Consuls, Blind Obedience, Root Maze, Orb of
    /// Dreams, …). The predicate's controller restriction (`OppCtrl`/`YouCtrl`)
    /// is resolved relative to THIS permanent's controller at ETB time
    /// (CR 614 replacement applied as the object enters). `None` for everything
    /// else. Predicates with qualifiers we don't yet model (`nonBasic`, `Snow`,
    /// `nonPhyrexian`, `IsPresent$`/`ValidCause$` conditions, non-battlefield
    /// `ActiveZones$`) are deliberately left `None` — see `mtg-713` B12.
    #[serde(default)]
    pub etb_tapped_global: Option<crate::core::effects::TargetRestriction>,
    /// Does this card require choosing a color when it enters the battlefield?
    /// Derived from K:ETBReplacement:Other:ChooseColor
    pub etb_choose_color: bool,
    /// Colors to exclude from the choice (from SVar:ChooseColor Exclude$ parameter)
    pub etb_exclude_colors: Vec<Color>,
    /// Does this card require choosing a player when it enters the battlefield?
    /// Derived from `K:ETBReplacement:Other:<SVar>` where the SVar is a
    /// `DB$ ChoosePlayer` (Black Vise). Serialized so the WASM per-set `.bin`
    /// carries it and the choice fires identically on both engines.
    #[serde(default)]
    pub etb_choose_player: bool,
    /// Script name (for tokens only). Used to look up token definitions.
    /// For tokens loaded from tokenscripts/, this is the filename without extension
    /// (e.g., "c_a_food_sac" for tokenscripts/c_a_food_sac.txt).
    /// For regular cards, this is None.
    // Note: skip_serializing_if was removed from the entire codebase because
    // it's incompatible with bincode (non-self-describing format) and caused bugs.
    pub script_name: Option<String>,
    /// Is this a legendary permanent?
    /// Derived from "Legendary" in Types line (e.g., "Types:Legendary Creature Human Noble")
    /// Used for legendary rule (MTG CR 704.5j)
    pub is_legendary: bool,
    /// Starting loyalty for planeswalkers (from Loyalty: field in card script)
    /// Applied as loyalty counters when the planeswalker enters the battlefield.
    pub loyalty: Option<u8>,
    /// Precomputed cache for static card properties (computed at load time)
    /// Avoids repeated string operations during gameplay
    pub cache: CardCache,
    /// The expansion this card was *originally* printed in (its earliest
    /// printing), derived from the `editions/` data, not from the card script.
    ///
    /// Powers set-origin valid predicates (`setARN`, etc.) used by City in a
    /// Bottle / Apocalypse Chime and any other card that references "a name
    /// originally printed in the <SET> expansion". `None` when no edition entry
    /// was found for the card (tokens, custom cards, or a missing `editions/`
    /// directory). Populated:
    /// - native: by [`crate::loader::CardDatabase`] after parse, from the
    ///   edition index resolved as a sibling of the cardsfolder;
    /// - WASM: at per-set bin export time from `PrimarySetAssignment`.
    ///
    /// Serialized so the WASM per-set `.bin` carries it and the network/snapshot
    /// path round-trips it deterministically.
    #[serde(default)]
    pub origin_set: Option<crate::core::SetCode>,
}

impl Default for CardDefinition {
    fn default() -> Self {
        Self {
            name: CardName::from(""),
            mana_cost: ManaCost::new(),
            types: Vec::new(),
            subtypes: Vec::new(),
            colors: Vec::new(),
            power: None,
            toughness: None,
            oracle: String::new(),
            raw_abilities: Vec::new(),
            raw_keywords: Vec::new(),
            svars: std::collections::HashMap::new(),
            parsed_svars: std::collections::HashMap::new(),
            enters_tapped: false,
            etb_tapped_global: None,
            etb_choose_color: false,
            etb_exclude_colors: Vec::new(),
            etb_choose_player: false,
            script_name: None,
            is_legendary: false,
            loyalty: None,
            cache: CardCache::default(),
            origin_set: None,
        }
    }
}

impl CardDefinition {
    /// Rebuild parsed_svars from svars after deserialization
    ///
    /// The `parsed_svars` field is skipped during serialization (because AbilityParams
    /// doesn't implement Serialize). After deserializing a CardDefinition from the network,
    /// call this method to rebuild the parsed_svars for trigger/ability parsing.
    pub fn rebuild_parsed_svars(&mut self) {
        use super::ability_parser::AbilityParams;
        self.parsed_svars.clear();
        for (svar_name, svar_body) in &self.svars {
            if let Some(params) = AbilityParams::parse_svar_body(svar_body) {
                self.parsed_svars.insert(svar_name.clone(), params);
            }
        }
    }

    /// Extract all TokenScript references from this card's abilities
    ///
    /// Scans all raw_abilities for SVar lines containing "DB$ Token" and extracts
    /// the TokenScript$ parameter value. Returns unique token script names.
    ///
    /// Example:
    /// - Input: `SVar:TrigToken:DB$ Token | TokenScript$ c_a_food_sac | TokenAmount$ 1`
    /// - Output: `["c_a_food_sac"]`
    pub fn extract_token_scripts(&self) -> Vec<String> {
        use super::ability_parser::{AbilityParams, ApiType};
        let mut token_scripts = std::collections::HashSet::new();

        for ability in &self.raw_abilities {
            if ability.starts_with("SVar:") {
                // Parse the SVar body for TokenScript$ parameter
                // Format: "SVar:NAME:DB$ Token | TokenScript$ script_name | ..."
                if let Some((_prefix, body)) = ability.split_once(':').and_then(|(_, rest)| rest.split_once(':')) {
                    if let Some(params) = AbilityParams::parse_svar_body(body) {
                        if params.api_type == ApiType::Token {
                            if let Some(script) = params.get("TokenScript") {
                                token_scripts.insert(script.to_string());
                            }
                        }
                    }
                }
            } else if ability.starts_with("A:") || ability.starts_with("T:") {
                // Also scan spell ability lines (A:) and trigger lines (T:) for TokenScript$
                // Format: "A:SP$ Token | TokenScript$ w_1_1_soldier | ..."
                // Format: "T:Mode$ SpellCast | Execute$ TrigToken | ..."
                // The A: line may contain TokenScript$ directly (e.g., Raise the Alarm)
                let body = &ability[2..];
                if let Some(params) = AbilityParams::parse_svar_body(body) {
                    if params.api_type == ApiType::Token {
                        if let Some(script) = params.get("TokenScript") {
                            token_scripts.insert(script.to_string());
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
        card.is_legendary = self.is_legendary;

        // Store the original CardDefinition BEFORE updating cache
        // (cache updates must apply to card.definition, not self which will be discarded)
        card.definition = self.clone();

        // Initialize cache with type flags (for O(1) is_land/is_creature/is_artifact checks)
        // and empty mana production (will be populated after abilities are parsed)
        card.definition.cache = crate::core::CardCache::new(&card.text, card.name.as_str());
        card.definition.cache.update_from_types(&card.types);
        card.definition
            .cache
            .update_from_subtypes(&card.subtypes, card.name.as_str());
        card.definition.cache.enters_tapped = self.enters_tapped;
        card.definition.cache.skips_untap_step = self.skips_untap_step();
        card.definition.cache.limits_land_untap = self.limits_land_untap();
        card.definition.cache.etb_choose_color = self.etb_choose_color;
        card.definition.cache.etb_exclude_colors = SmallVec::from_slice(&self.etb_exclude_colors);
        card.definition.cache.etb_choose_player = self.etb_choose_player;
        // Fireball's `{1}`-per-extra-target relative cost (CR 601.2f).
        card.definition.cache.spell_relative_target_cost = self.has_relative_self_target_cost();

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
                        amount_var: None,
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
                        amount_var: None,
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
                        amount_var: None,
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
                        amount_var: None,
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
                        amount_var: None,
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

        // Add Class level-up activated abilities for Class enchantments (CR 716).
        // Each K:Class:N:cost:abilities entry generates a sorcery-speed activated ability
        // whose effect is ClassLevelUp { target_level: N }.
        // The current level is tracked at runtime via CounterType::Level on the permanent.
        {
            let class_levels: Vec<(u8, ManaCost)> = card
                .keywords
                .iter_args()
                .filter_map(|kw| {
                    if let KeywordArgs::Class { level, cost, .. } = kw {
                        Some((*level, ManaCost::from_string(cost)))
                    } else {
                        None
                    }
                })
                .collect();

            for (level, mana_cost) in class_levels {
                use crate::core::{ActivatedAbility, Cost, Effect};
                let ability_cost = Cost::Mana(mana_cost);
                let effects = vec![Effect::ClassLevelUp {
                    class_card_id: id,
                    target_level: level,
                }];
                let description = format!("Level {} (class level-up)", level);
                // Class level-up is sorcery-speed (CR 716.2a)
                card.activated_abilities
                    .push(ActivatedAbility::new_sorcery_speed(ability_cost, effects, description));
            }
        }

        // Class enchantments start at level 1 (CR 716.1c).  Add an ETB trigger that
        // places 1 Level counter on the card as it enters the battlefield so the
        // `execute_class_level_up` guard (`current_level + 1 == target_level`) works
        // correctly: current=1 → level-up to 2; current=2 → level-up to 3.
        let has_class_levels = card
            .keywords
            .iter_args()
            .any(|kw| matches!(kw, KeywordArgs::Class { .. }));
        if has_class_levels {
            use crate::core::{CounterType, Effect, Trigger, TriggerEvent};
            let etb_level_trigger = Trigger::new(
                TriggerEvent::EntersBattlefield,
                vec![Effect::PutCounter {
                    target: CardId::self_target(),
                    counter_type: CounterType::Level,
                    amount: 1,
                }],
                "Class enters as level 1 (place 1 level counter)".to_string(),
            );
            card.triggers.push(etb_level_trigger);
        }

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

        // Add Prowess trigger if the keyword is present
        // Prowess: "Whenever you cast a noncreature spell, this creature gets +1/+1 until end of turn."
        // MTG 702.108
        if card.keywords.contains(Keyword::Prowess) {
            use crate::core::{Effect, Trigger, TriggerEvent};

            // Create SpellCast trigger with PumpCreature effect
            let mut prowess_trigger = Trigger::new(
                TriggerEvent::SpellCast,
                vec![Effect::PumpCreature {
                    target: CardId::new(0), // Placeholder - resolved at runtime to self
                    power_bonus: 1,
                    toughness_bonus: 1,
                    keywords_granted: smallvec::SmallVec::new(),
                }],
                "[noncreature] Prowess (+1/+1 until end of turn)".to_string(),
            );
            prowess_trigger.requires_noncreature = true;
            card.triggers.push(prowess_trigger);
        }

        // Update cache AFTER all abilities are parsed (including implicit mana abilities)
        // This derives mana production from Effect::AddMana in the abilities,
        // following Java Forge's approach of using structured Produced$ data.
        // Falls back to land name detection for test cards without explicit abilities.
        card.definition
            .cache
            .update_from_abilities_with_name(&card.activated_abilities, card.name.as_str());

        card
    }

    /// Parse raw keywords into KeywordSet
    pub(crate) fn parse_keywords(&self) -> KeywordSet {
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
                        // K:Enchant:<TypeSpec>[:<human-readable description>]
                        // e.g. "Enchant:Creature" (Spirit Link)
                        // e.g. "Enchant:Creature.inZoneGraveyard:creature card in a graveyard"
                        //      (Animate Dead — the trailing description must be stripped, otherwise
                        //      targeting code that splits on ".inzone" sees zone="graveyard:creature ..."
                        //      and fails to match "graveyard").
                        let type_spec = param.split(':').next().unwrap_or(param).trim();
                        let card_type = Subtype::new(type_spec);
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
                        // Ward can have mana cost or Waterbend cost
                        // Examples: "Ward:2", "Ward:Waterbend<4>"
                        if param.starts_with("Waterbend<") {
                            // Extract the amount from Waterbend<N>
                            if let Some(amount_str) = param.strip_prefix("Waterbend<").and_then(|s| s.strip_suffix('>'))
                            {
                                if let Ok(amount) = amount_str.parse::<u8>() {
                                    keyword_set.insert_complex(KeywordArgs::WardWaterbend { amount });
                                }
                            }
                        } else {
                            let cost = ManaCost::from_string(param);
                            keyword_set.insert_complex(KeywordArgs::Ward { cost });
                        }
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
                    // ===== ALTERNATE COSTS AND SPECIAL PARAMETERIZED KEYWORDS =====
                    "AlternateAdditionalCost" => {
                        // Format: "AlternateAdditionalCost:Reveal<1/Goblin>:3"
                        // Parsed but not yet used at runtime - stores raw format
                        keyword_set.insert_complex(KeywordArgs::AlternateAdditionalCost {
                            spec: param.to_string(),
                        });
                    }
                    "MustBeBlockedByAll" => {
                        // Format: "MustBeBlockedByAll:Creature.withFlying:description"
                        keyword_set.insert_complex(KeywordArgs::MustBeBlockedByAllFiltered {
                            filter: param.to_string(),
                        });
                    }
                    "MayEffectFromOpeningDeck" => {
                        // Format: "MayEffectFromOpeningDeck:DBReveal"
                        keyword_set.insert_complex(KeywordArgs::MayEffectFromOpeningDeck {
                            effect_ref: param.to_string(),
                        });
                    }
                    "Prize" => {
                        // Format: "Prize:TrigPrize"
                        keyword_set.insert_complex(KeywordArgs::Prize {
                            trigger_ref: param.to_string(),
                        });
                    }
                    "Trample" if param == "Planeswalker" => {
                        // "Trample:Planeswalker" means this creature's excess combat damage
                        // can be dealt to planeswalkers the defending player controls
                        keyword_set.insert(Keyword::Trample);
                        // TODO: Add TramplePlaneswalker variant for runtime handling
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
                    "Protection from everything" => keyword_set.insert(Keyword::ProtectionFromEverything),
                    "Protection from each color" => keyword_set.insert(Keyword::ProtectionFromEachColor),
                    // ===== LURE-TYPE EFFECTS (must be blocked) =====
                    "CARDNAME must be blocked if able." => keyword_set.insert(Keyword::MustBeBlocked),
                    "All creatures able to block CARDNAME do so." => keyword_set.insert(Keyword::MustBeBlockedByAll),
                    "CARDNAME must be blocked by two or more creatures if able." => {
                        keyword_set.insert(Keyword::MustBeBlockedByTwo)
                    }
                    "CARDNAME must be blocked by exactly one creature if able." => {
                        keyword_set.insert(Keyword::MustBeBlockedByExactlyOne)
                    }
                    // ===== COMBAT RESTRICTIONS =====
                    "CARDNAME can't attack alone." => keyword_set.insert(Keyword::CantAttackAlone),
                    "CARDNAME can't attack or block alone." => keyword_set.insert(Keyword::CantAttackOrBlockAlone),
                    // ===== DAMAGE PREVENTION =====
                    "Prevent all damage that would be dealt to CARDNAME." => {
                        keyword_set.insert(Keyword::PreventAllDamage)
                    }
                    "Prevent all combat damage that would be dealt to CARDNAME." => {
                        keyword_set.insert(Keyword::PreventAllCombatDamage)
                    }
                    "Prevent all combat damage that would be dealt to and dealt by CARDNAME." => {
                        keyword_set.insert(Keyword::PreventAllCombatDamageDealtAndReceived)
                    }
                    // ===== UNTAP AND BLOCKING =====
                    "CARDNAME untaps during each other player's untap step." => {
                        keyword_set.insert(Keyword::UntapsDuringOthersUntapStep)
                    }
                    "CARDNAME can block creatures with shadow as though they didn't have shadow." => {
                        keyword_set.insert(Keyword::CanBlockShadow)
                    }
                    // ===== DECK-BUILDING =====
                    "A deck can have any number of cards named CARDNAME." => keyword_set.insert(Keyword::DeckAnyNumber),
                    "CARDNAME can be your commander." => keyword_set.insert(Keyword::CanBeCommander),
                    "Remove CARDNAME from your deck before playing if you're not playing for ante." => {
                        keyword_set.insert(Keyword::AnteRemoval)
                    }
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
                    "For Mirrodin!" | "For Mirrodin" | "ForMirrodin" => keyword_set.insert(Keyword::ForMirrodin),
                    "Fuse" => keyword_set.insert(Keyword::Fuse),
                    "Gift" => keyword_set.insert(Keyword::Gift),
                    "Hidden agenda" | "HiddenAgenda" => keyword_set.insert(Keyword::HiddenAgenda),
                    "Ingest" => keyword_set.insert(Keyword::Ingest),
                    "Job select" | "JobSelect" => keyword_set.insert(Keyword::JobSelect),
                    "Jump-start" | "JumpStart" => keyword_set.insert(Keyword::JumpStart),
                    "Living metal" | "LivingMetal" => keyword_set.insert(Keyword::LivingMetal),
                    "Living weapon" | "Living Weapon" | "LivingWeapon" => keyword_set.insert(Keyword::LivingWeapon),
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
                    "Totem armor" | "Umbra armor" | "UmbraArmor" => keyword_set.insert(Keyword::UmbraArmor),
                    "Undaunted" => keyword_set.insert(Keyword::Undaunted),
                    "Undying" => keyword_set.insert(Keyword::Undying),
                    "Unleash" => keyword_set.insert(Keyword::Unleash),
                    // Vanishing without a counter count uses existing time counters
                    "Vanishing" => keyword_set.insert_complex(KeywordArgs::Vanishing { counters: 0 }),
                    // ===== COMMANDER/MULTIPLAYER =====
                    "Choose a Background" => keyword_set.insert(Keyword::ChooseABackground),
                    "Doctor's companion" | "DoctorsCompanion" => keyword_set.insert(Keyword::DoctorsCompanion),
                    "Friends forever" | "FriendsForever" => keyword_set.insert(Keyword::FriendsForever),
                    "Partner Survivors" | "Partner - Survivors" | "PartnerSurvivors" => {
                        keyword_set.insert(Keyword::PartnerSurvivors)
                    }
                    "Partner Father and Son" | "Partner - Father & Son" | "PartnerFatherAndSon" => {
                        keyword_set.insert(Keyword::PartnerFatherAndSon)
                    }
                    "Partner Character Select" | "Partner - Character select" | "PartnerCharacterSelect" => {
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

        // ---------------------------------------------------------------
        // Self-referential static keywords expressed as `S:Mode$ ...` lines.
        //
        // Some intrinsic combat-restriction keywords are printed on cards as
        // a static ability targeting the card itself rather than as a bare
        // `K:` line. The canonical example is Juggernaut's
        //   S:Mode$ MustAttack | ValidCreature$ Card.Self
        // which is Oracle "attacks each combat if able" (CR 508.1a). We
        // surface it as Keyword::MustAttack so the engine's declare-attackers
        // enforcement (and the heuristic eval) treat it uniformly with any
        // creature that gained the keyword by other means.
        //
        // Only the SELF shape (`ValidCreature$ Card.Self`) becomes a keyword
        // on this card. A MustAttack static that forces OTHER creatures to
        // attack (e.g. a global "all creatures attack each combat") is a
        // different mechanic and is intentionally left for a separate static
        // path — keywording it here would wrongly tag the source itself.
        for ability in &self.raw_abilities {
            let Some(body) = ability.strip_prefix("S:") else {
                continue;
            };
            let params = tokenize_pipe_dollar(body);
            if params.get("Mode").map(String::as_str) != Some("MustAttack") {
                continue;
            }
            let is_self = params
                .get("ValidCreature")
                .map(|v| v.trim() == "Card.Self")
                .unwrap_or(false);
            if is_self {
                keyword_set.insert(Keyword::MustAttack);
            }
        }

        keyword_set
    }

    /// Parse raw abilities into Effect objects
    ///
    /// Uses tokenized parsing (ability_parser) for safety and correctness.
    /// Replaces unsafe substring matching with proper parameter extraction.
    /// Follows SubAbility$ chains to resolve all effects in a spell.
    pub(crate) fn parse_effects(&self) -> Vec<crate::core::Effect> {
        use super::ability_parser::{AbilityParams, ApiType};
        use super::effect_converter::{
            params_to_charm_effect_with_svars, params_to_delayed_trigger_with_svars, params_to_effect_with_svars,
            params_to_immediate_trigger_with_svars,
        };

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
            // Use SVar-aware conversion for all spell abilities so that effects
            // like DealDamageDynamic (Count$ValidGraveyard) can resolve their SVar.
            // Charm/DelayedTrigger/ImmediateTrigger use their dedicated SVar-aware
            // converters; everything else uses params_to_effect_with_svars which
            // falls back to params_to_effect for types it doesn't specialise.
            let effect = if params.api_type == ApiType::Charm {
                params_to_charm_effect_with_svars(&params, &self.svars)
            } else if params.api_type == ApiType::DelayedTrigger {
                params_to_delayed_trigger_with_svars(&params, &self.svars)
            } else if params.api_type == ApiType::ImmediateTrigger {
                params_to_immediate_trigger_with_svars(&params, &self.svars)
            } else {
                params_to_effect_with_svars(&params, &self.svars)
            };

            if let Some(effect) = effect {
                // Balance effects store their SubAbility reference internally and are
                // handled by the game loop's resolve_balance_effect_chain(). Don't
                // follow SubAbility chain during parsing to avoid duplicate processing.
                let is_balance = matches!(effect, crate::core::Effect::Balance { .. });
                effects.push(effect);

                if !is_balance {
                    // Follow SubAbility$ chain to parse additional effects
                    // Example: A:SP$ Pump | SubAbility$ DBToken creates both Pump and Token effects
                    self.follow_sub_ability_chain(&params, &mut effects);
                }
            } else {
                // No effect was created - still follow SubAbility chain in case
                // the main ability is unsupported but SubAbility is supported
                self.follow_sub_ability_chain(&params, &mut effects);
            }

            // Disintegrate: `... | ReplaceDyingDefined$ ThisTargetedCard.Creature`
            // means "if the targeted creature would die this turn, exile it
            // instead" (CR 614). Append an ExileIfWouldDieThisTurn that binds to
            // the just-resolved DealDamage target (reuse_previous). This is
            // queried structurally from the tokenized params, never via substring
            // matching on the script body.
            if params.contains_key("ReplaceDyingDefined") {
                effects.push(crate::core::Effect::ExileIfWouldDieThisTurn {
                    target: crate::core::CardId::reuse_previous(),
                });
            }
            // Note: Unsupported API types are silently skipped (returns None)
            // This is intentional - we don't want to spam warnings for every unsupported ability
        }

        // Wire up `K:ETBReplacement:Copy:<SVar>:Optional` (Copy Artifact and other
        // Clone permanents). The keyword names an SVar (e.g. DBCopy) whose body is
        // `DB$ Clone | Choices$ ... | AddTypes$ ...`. We parse that SVar into an
        // `Effect::Clone` and push it onto the card's effect list so the spell
        // resolution path (priority.rs) can intercept it and route the
        // "which permanent to copy" choice through the controller.
        if let Some(clone_effect) = self.parse_etb_clone_effect() {
            effects.push(clone_effect);
        }

        effects
    }

    /// Build an `Effect::Clone` from a `K:ETBReplacement:Copy:<SVar>:Optional`
    /// keyword line, if present.
    ///
    /// The keyword line is parsed in tokenized form (split on `:`), never via
    /// substring matching, per the "No Hacky String Operations" rule. Returns
    /// `None` when the card has no `ETBReplacement:Copy` keyword or the named
    /// SVar does not resolve to a `DB$ Clone` effect.
    fn parse_etb_clone_effect(&self) -> Option<crate::core::Effect> {
        use super::ability_parser::{AbilityParams, ApiType};
        use super::effect_converter::params_to_effect;

        for keyword_str in &self.raw_keywords {
            // Tokenize "ETBReplacement:Copy:DBCopy:Optional"
            let mut parts = keyword_str.split(':');
            if parts.next() != Some("ETBReplacement") {
                continue;
            }
            if parts.next() != Some("Copy") {
                continue;
            }
            let svar_name = parts.next()?.trim();
            // Remaining tokens are flags; "Optional" => "you may".
            let optional = parts.any(|flag| flag.trim() == "Optional");

            let svar_body = self.svars.get(svar_name)?;
            let ability_line = format!("A:{}", svar_body);
            let params = AbilityParams::parse(&ability_line).ok()?;
            if params.api_type != ApiType::Clone {
                continue;
            }

            if let Some(crate::core::Effect::Clone {
                source,
                chosen,
                choices_filter,
                add_types,
                optional: _,
            }) = params_to_effect(&params)
            {
                return Some(crate::core::Effect::Clone {
                    source,
                    chosen,
                    choices_filter,
                    add_types,
                    optional,
                });
            }
        }
        None
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
        use super::ability_parser::{AbilityParams, ApiType};
        use super::effect_converter::{
            params_to_charm_effect_with_svars, params_to_delayed_trigger_with_svars, params_to_effect_with_svars,
            params_to_immediate_trigger_with_svars,
        };

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

        // Guard: skip sub-abilities that require CollectEvidence to have been paid.
        // `ConditionDefined$ Collected` (or `CastSA>Collected`) with `ConditionPresent$ Card`
        // and *no* `ConditionCompare$ EQ0` means "fire only when evidence IS collected."
        // CollectEvidence (the optional cost mechanic) is not yet implemented, so evidence
        // is never collected; unconditionally executing these sub-effects fires the
        // "evidence was collected" branch even when it wasn't, e.g. double-search in
        // Analyze the Pollen (mtg-834). Skip them entirely until CollectEvidence lands.
        // TODO(mtg-834): remove this guard and replace with a proper runtime condition check.
        {
            let condition_defined = sub_params.get("ConditionDefined").unwrap_or("");
            if condition_defined.contains("Collected")
                && sub_params.get("ConditionPresent").is_some()
                && sub_params.get("ConditionCompare") != Some("EQ0")
            {
                return;
            }
        }

        // Convert to effect. A `DB$ GainLife` with a dynamic `LifeAmount$`
        // (e.g. Swords to Plowshares `LifeAmount$ X` / `SVar:X:Targeted$CardPower`)
        // is not expressible as a fixed-amount `Effect::GainLife`, so route it
        // through the SVar-aware dynamic builder first.
        //
        // DelayedTrigger / ImmediateTrigger / Charm sub-abilities need the
        // SVar-aware converters (mirroring `parse_effects`) so their `Execute$`
        // / mode SVars resolve — e.g. Mana Drain's `SubAbility$ DBDelTrig`
        // (`DB$ DelayedTrigger | Mode$ Phase | ... | Execute$ AddMana`).
        // Most ApiTypes use the SVar-aware converter so that DealDamageDynamic
        // (and any future SVar-resolved effects) also work from SubAbility chains.
        // Charm/DelayedTrigger/ImmediateTrigger use their own dedicated builders.
        #[allow(clippy::wildcard_enum_match_arm)]
        let sub_effect = match sub_params.api_type {
            ApiType::Charm => params_to_charm_effect_with_svars(&sub_params, &self.svars),
            ApiType::DelayedTrigger => params_to_delayed_trigger_with_svars(&sub_params, &self.svars),
            ApiType::ImmediateTrigger => params_to_immediate_trigger_with_svars(&sub_params, &self.svars),
            _ => params_to_effect_with_svars(&sub_params, &self.svars),
        };
        // A SUB-ability can carry its own `UnlessCost$` gate (e.g. Chain
        // Lightning's `SVar:DBCopy1:DB$ CopySpellAbility | ... | UnlessCost$ R R
        // | UnlessPayer$ TargetedOrController | UnlessSwitched$ True`). The
        // head-ability path applies this via `params_to_effect_with_unless`, but
        // the SVar-aware sub-ability builders above do NOT, so without this wrap
        // the optional "may pay {R}{R}" gate was silently dropped and the copy
        // fired unconditionally (mtg-152). Wrap each sub-effect the same way.
        if let Some(effect) = self.gain_life_dynamic_from_params(&sub_params) {
            effects.push(super::effect_converter::wrap_with_unless_cost(effect, &sub_params));
        } else if let Some(effect) = sub_effect {
            effects.push(super::effect_converter::wrap_with_unless_cost(effect, &sub_params));
        }

        // Recursively follow further SubAbility chains
        self.follow_sub_ability_chain(&sub_params, effects);
    }

    /// Build an [`Effect::GainLifeDynamic`](crate::core::Effect::GainLifeDynamic)
    /// from a `DB$ GainLife` ability whose `LifeAmount$` is a dynamic value
    /// reference resolved through the card's SVars.
    ///
    /// Returns `None` for non-GainLife abilities or fixed-amount GainLife (which
    /// the standard `params_to_effect` path already handles as `Effect::GainLife`).
    /// Recognised dynamic amounts (see [`crate::core::DynamicAmount::parse`]):
    /// - `LifeAmount$ X` with `SVar:X:Targeted$CardPower`    -> `TargetPower`
    /// - `LifeAmount$ X` with `SVar:X:Targeted$CardManaCost` -> `TargetManaValue`
    ///
    /// The `Defined$` selector picks who gains the life:
    /// - `TargetedController` -> the targeted permanent's controller (Swords)
    /// - `You` / absent       -> the spell's controller (Divine Offering)
    fn gain_life_dynamic_from_params(
        &self,
        params: &super::ability_parser::AbilityParams,
    ) -> Option<crate::core::Effect> {
        use super::ability_parser::ApiType;
        use crate::core::{CardId, DynamicAmount, Effect, PlayerId};

        if params.api_type != ApiType::GainLife {
            return None;
        }
        let life_amount = params.get("LifeAmount")?;
        // A plain integer is the fixed-amount case — let params_to_effect handle it.
        // Every other (non-Fixed) DynamicAmount — TargetPower / TargetManaValue /
        // DamageDealt / Count(...) — routes through GainLifeDynamic.
        let amount = match DynamicAmount::parse(life_amount, &self.svars)? {
            DynamicAmount::Fixed(_) => return None,
            dynamic @ (DynamicAmount::TargetPower
            | DynamicAmount::TargetManaValue
            | DynamicAmount::DamageDealt
            | DynamicAmount::DamageDealtCappedByTarget { .. }
            | DynamicAmount::Count(_)) => dynamic,
        };

        // Resolve the recipient placeholder from the Defined$ selector. The
        // concrete player/reference are filled at resolution time in
        // resolve_effect_target.
        let player = match params.get("Defined") {
            Some("TargetedController") => PlayerId::target_controller(),
            _ => PlayerId::placeholder(), // You / unspecified -> spell controller
        };
        // The referenced card (whose power / mana value we read) is the spell's
        // current target; "reuse previous" tells the resolver to pull it from
        // the most recently resolved target (the preceding exile/destroy).
        Some(Effect::GainLifeDynamic {
            player,
            amount,
            reference: CardId::reuse_previous(),
        })
    }

    /// Wrap an effect in `Effect::ConditionalSelfCounter` when the SVar carries a
    /// `ConditionDefined$ Self | ConditionPresent$ Card.counters_<CMP><N>_<TYPE>`
    /// clause; otherwise return the effect unchanged.
    ///
    /// Used for chains like All Hallow's Eve's `DBMoveToGraveyard` /
    /// `DBResurrection`, which only fire when the source has zero scream
    /// counters left (`Card.counters_EQ0_SCREAM`).
    fn wrap_self_counter_condition(
        svar_params: &super::ability_parser::AbilityParams,
        effect: crate::core::Effect,
    ) -> crate::core::Effect {
        use crate::core::{CardId, Effect, SelfCounterCondition};

        if svar_params.get("ConditionDefined") != Some("Self") {
            return effect;
        }
        // ConditionPresent$ Card.counters_<CMP><N>_<TYPE> — extract the
        // counters_… clause (a `+`-joined Card filter; we only support the
        // counter sub-clause here, which is all the gated chains need).
        let Some(present) = svar_params.get("ConditionPresent") else {
            return effect;
        };
        // The filter is dotted/plus-joined, e.g. `Card.counters_EQ0_SCREAM` or
        // `Card.Self+counters_GE1_SCREAM`. Locate the `counters_` sub-clause by
        // splitting on both `.` and `+`.
        let Some(condition) = present
            .split(['.', '+'])
            .find_map(|clause| clause.strip_prefix("counters_"))
            .and_then(SelfCounterCondition::parse_clause)
        else {
            return effect;
        };
        Effect::ConditionalSelfCounter {
            source: CardId::self_target(),
            condition,
            inner: Box::new(effect),
        }
    }

    /// Extract effects from a parsed SVar (DRY helper for trigger parsing)
    ///
    /// This consolidates the duplicated ApiType->Effect conversion logic that was
    /// previously copy-pasted across ETB, dies, attacks, and sacrifice trigger handlers.
    ///
    /// Uses `params_to_effect()` from effect_converter for standard effects,
    /// plus handles special cases like:
    /// - Attach with SubAbility chains (equipment ETB)
    /// - Loot (Draw with Discard cost)
    fn extract_effects_from_svar(
        &self,
        svar_params: &super::ability_parser::AbilityParams,
    ) -> Vec<crate::core::Effect> {
        use super::ability_parser::ApiType;
        use super::effect_converter::{params_to_charm_effect_with_svars, params_to_effect};
        use crate::core::{CardId, Effect, Keyword, PlayerId};

        let mut effects = Vec::new();

        // A `DB$ GainLife` with a dynamic `LifeAmount$` (e.g. Spirit Link's
        // `LifeAmount$ X` / `SVar:X:TriggerCount$DamageAmount`, or Swords-style
        // `Targeted$CardPower`) is not expressible as a fixed `Effect::GainLife`
        // — params_to_effect would silently drop it (get_i32 fails on "X"). Route
        // it through the SVar-aware dynamic builder first (DRY with the
        // SubAbility-chain path in follow_sub_ability_chain).
        if let Some(effect) = self.gain_life_dynamic_from_params(svar_params) {
            effects.push(effect);
            return effects;
        }

        // Charm (DB$ Charm | Choices$ ...) needs the SVar-aware converter so that
        // mode SVars (e.g. DBDraw, DBToken, DBLoseLife) are resolved to real Effects
        // rather than placeholders. This mirrors the SubAbility-chain path in
        // follow_sub_ability_chain (same rule: Charm always gets the SVar builder).
        if svar_params.api_type == ApiType::Charm {
            if let Some(effect) = params_to_charm_effect_with_svars(svar_params, &self.svars) {
                effects.push(effect);
            }
            return effects;
        }

        // First, try the standard params_to_effect conversion
        if let Some(effect) = params_to_effect(svar_params) {
            // Wrap in a counter-gated conditional when the SVar carries
            //   ConditionDefined$ Self | ConditionPresent$ Card.counters_<CMP><N>_<TYPE>
            // (e.g. All Hallow's Eve's exile→graveyard move + mass resurrection,
            // which only fire on the upkeep where the final scream counter was
            // removed, counters_EQ0_SCREAM).
            effects.push(Self::wrap_self_counter_condition(svar_params, effect));
        } else {
            // Handle special cases not covered by params_to_effect

            // Special case: Loot (Draw with Discard cost)
            // AB$ Draw | Cost$ Discard<N/Card> -> Effect::Loot
            if svar_params.api_type == ApiType::Draw {
                if let Some(cost) = svar_params.get("Cost") {
                    if cost.starts_with("Discard<") {
                        let draw_count = svar_params
                            .get("NumCards")
                            .and_then(|s| s.parse::<u8>().ok())
                            .unwrap_or(1);

                        let discard_count = cost
                            .strip_prefix("Discard<")
                            .and_then(|s| s.split('/').next())
                            .and_then(|n| n.parse::<u8>().ok())
                            .unwrap_or(1);

                        effects.push(Effect::Loot {
                            player: PlayerId::new(0),
                            discard_count,
                            draw_count,
                        });
                    }
                }
            }

            // Special case: Attach with SubAbility chain (equipment ETB)
            // DB$ Attach | ValidTgts$ Creature.YouCtrl | SubAbility$ DBPump
            if svar_params.api_type == ApiType::Attach {
                effects.push(Effect::AttachEquipment {
                    source_equipment: CardId::new(0),
                    target_creature: CardId::new(0),
                });

                // Follow SubAbility chain for additional effects (e.g., keyword grant)
                if let Some(sub_ref) = svar_params.get("SubAbility") {
                    if let Some(sub_params) = self.parsed_svars.get(sub_ref) {
                        // DB$ Pump with KW$ (keyword grant)
                        if sub_params.api_type == ApiType::Pump {
                            if let Some(kw_str) = sub_params.get("KW") {
                                let keywords_granted: smallvec::SmallVec<[Keyword; 2]> = kw_str
                                    .split(" & ")
                                    .filter_map(|kw| Keyword::from_string(kw.trim()))
                                    .collect();
                                effects.push(Effect::PumpCreature {
                                    target: CardId::new(0),
                                    power_bonus: 0,
                                    toughness_bonus: 0,
                                    keywords_granted,
                                });
                            }
                        }
                    }
                }
            }
        }

        // Follow SubAbility chains for additional effects
        if let Some(sub_ref) = svar_params.get("SubAbility") {
            if let Some(sub_params) = self.parsed_svars.get(sub_ref) {
                // Only follow if we haven't already handled this above (Attach case)
                if svar_params.api_type != ApiType::Attach {
                    effects.extend(self.extract_effects_from_svar(sub_params));
                }
            }
        }

        effects
    }

    /// Parse triggered abilities (T: lines)
    ///
    /// Uses tokenized parameter extraction for safety. Replaces unsafe substring matching.
    pub fn parse_triggers(&self) -> Vec<Trigger> {
        use super::ability_parser::ApiType;
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

                // Parse effects from this trigger
                let mut effects = Vec::new();

                // Check if we have Execute$ parameter (references a SVar with effects)
                // Use pre-parsed SVars for O(1) lookup and extract_effects_from_svar helper (DRY)
                // This is the preferred mechanism for effect parsing.
                if let Some(exec_ref) = params.get("Execute") {
                    if let Some(svar_params) = self.parsed_svars.get(exec_ref) {
                        effects.extend(self.extract_effects_from_svar(svar_params));
                    }
                } else {
                    // Fallback: check for inline effect parameters (rare, but some cards use this)
                    // Only used when there's no Execute$ SVar reference

                    // Check if we have NumCards$ parameter (draw effect)
                    if let Some(num_cards_str) = params.get("NumCards").map(|s| s.to_string()) {
                        if let Ok(count) = num_cards_str.parse::<u8>() {
                            effects.push(Effect::DrawCards {
                                player: PlayerId::new(0),
                                count,
                            });
                        }
                    }

                    // Check if we have NumDmg$ parameter (damage effect)
                    if let Some(num_dmg_str) = params.get("NumDmg").map(|s| s.to_string()) {
                        if let Ok(amount) = num_dmg_str.parse::<i32>() {
                            effects.push(Effect::DealDamage {
                                target: TargetRef::None,
                                amount,
                            });
                        }
                    }

                    // Check if we have LifeAmount$ parameter (gain life effect)
                    if let Some(life_amt_str) = params.get("LifeAmount").map(|s| s.to_string()) {
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
                        .and_then(|s| s.trim_start_matches('+').parse::<i32>().ok())
                        .unwrap_or(0);
                    let toughness_bonus = params
                        .get("NumDef")
                        .map(|s| s.to_string())
                        .and_then(|s| s.trim_start_matches('+').parse::<i32>().ok())
                        .unwrap_or(0);

                    if power_bonus != 0 || toughness_bonus != 0 {
                        effects.push(Effect::PumpCreature {
                            target: CardId::new(0),
                            power_bonus,
                            toughness_bonus,
                            keywords_granted: smallvec::SmallVec::new(),
                        });
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

            // Parse Landfall triggers (Mode$ ChangesZone with ValidCard$ Land.YouCtrl)
            // Landfall triggers when a land enters under your control (not just this card)
            // Example: T:Mode$ ChangesZone | Origin$ Any | Destination$ Battlefield | ValidCard$ Land.YouCtrl | Execute$ TrigFlying
            if mode == Some("ChangesZone")
                && params.get("Destination").map(|s| s.as_str()) == Some("Battlefield")
                && params.get("ValidCard").map(|s| s.as_str()) == Some("Land.YouCtrl")
            {
                let mut effects = Vec::new();

                // Check if we have Execute$ parameter (references a SVar with effects)
                if let Some(exec_ref) = params.get("Execute") {
                    if let Some(svar_params) = self.parsed_svars.get(exec_ref) {
                        effects.extend(self.extract_effects_from_svar(svar_params));
                    }
                }

                // Extract description from TriggerDescription$ if available
                let description = params
                    .get("TriggerDescription")
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "Landfall".to_string());

                // Create trigger with [landfall] flag for runtime filtering
                // Use trigger_self_only = false since this triggers on OTHER cards entering
                let mut trigger = Trigger::new_any(
                    TriggerEvent::EntersBattlefield,
                    effects,
                    format!("[landfall] {}", description),
                );
                trigger.trigger_self_only = false;
                trigger.requires_landfall = true;
                triggers.push(trigger);
            }

            // Parse "dies" triggers (Mode$ ChangesZone with Origin$ Battlefield, Destination$ Graveyard)
            // Example: T:Mode$ ChangesZone | Origin$ Battlefield | Destination$ Graveyard | ValidCard$ Card.Self | Execute$ TrigAddMana
            if mode == Some("ChangesZone")
                && params.get("Origin").map(|s| s.as_str()) == Some("Battlefield")
                && params.get("Destination").map(|s| s.as_str()) == Some("Graveyard")
                && params.get("ValidCard").map(|s| s.as_str()) == Some("Card.Self")
            {
                let mut effects = Vec::new();

                // Check if we have Execute$ parameter (references a SVar with effects)
                // Use extract_effects_from_svar helper (DRY)
                if let Some(exec_ref) = params.get("Execute") {
                    if let Some(svar_params) = self.parsed_svars.get(exec_ref) {
                        effects.extend(self.extract_effects_from_svar(svar_params));
                    }
                }

                // Extract description from TriggerDescription$ if available
                let description = params
                    .get("TriggerDescription")
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "When this creature dies".to_string());

                triggers.push(Trigger::new(TriggerEvent::LeavesBattlefield, effects, description));
            }

            // Parse "equipped creature dies" triggers
            // T:Mode$ ChangesZone | Origin$ Battlefield | Destination$ Graveyard | ValidCard$ Card.EquippedBy | Execute$ TrigDraw
            // Example: Skullclamp - "Whenever equipped creature dies, draw two cards."
            if mode == Some("ChangesZone")
                && params.get("Origin").map(|s| s.as_str()) == Some("Battlefield")
                && params.get("Destination").map(|s| s.as_str()) == Some("Graveyard")
                && params.get("ValidCard").map(|s| s.as_str()) == Some("Card.EquippedBy")
            {
                let mut effects = Vec::new();

                if let Some(exec_ref) = params.get("Execute") {
                    if let Some(svar_params) = self.parsed_svars.get(exec_ref) {
                        effects.extend(self.extract_effects_from_svar(svar_params));
                    }
                }

                let description = params
                    .get("TriggerDescription")
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "When equipped creature dies".to_string());

                triggers.push(Trigger::new(TriggerEvent::EquippedCreatureDies, effects, description));
            }

            // Parse "creature dealt damage by this dies" triggers
            // T:Mode$ ChangesZone | Origin$ Battlefield | Destination$ Graveyard
            //   | ValidCard$ Creature.DamagedBy | TriggerZones$ Battlefield | Execute$ TrigPutCounter
            // Example: Sengir Vampire — "Whenever a creature dealt damage by Sengir
            //          Vampire this turn dies, put a +1/+1 counter on Sengir Vampire."
            //
            // The trigger fires on the source card (e.g. Sengir), not on the dying
            // card; check_death_triggers scans the battlefield for permanents whose
            // CardId appears in dying_card.damaged_by_this_turn.
            if mode == Some("ChangesZone")
                && params.get("Origin").map(|s| s.as_str()) == Some("Battlefield")
                && params.get("Destination").map(|s| s.as_str()) == Some("Graveyard")
                && params
                    .get("ValidCard")
                    .map(|s| s.as_str())
                    .is_some_and(|v| v.starts_with("Creature.DamagedBy"))
            {
                let mut effects = Vec::new();

                if let Some(exec_ref) = params.get("Execute") {
                    if let Some(svar_params) = self.parsed_svars.get(exec_ref) {
                        effects.extend(self.extract_effects_from_svar(svar_params));
                    }
                }

                let description = params
                    .get("TriggerDescription")
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "When a creature damaged by this dies".to_string());

                triggers.push(Trigger::new(TriggerEvent::DamagedCreatureDies, effects, description));
            }

            // Parse phase triggers (Mode$ Phase)
            if mode == Some("Phase") {
                // Determine which phase/step this triggers on using tokenized params
                // Normalize the phase token: Forge writes the end step as either
                // "EndOfTurn", "End", or the spaced "End of Turn" / "End Of Turn"
                // (Whirling Dervish, Berserk's delayed trigger). Strip spaces and
                // compare case-insensitively so every spelling maps to one event,
                // instead of silently dropping the spaced variants (mtg-713 B9).
                let phase_token = params.get("Phase").map(|s| s.replace(' ', ""));
                let trigger_event = match phase_token.as_deref() {
                    Some("Upkeep") => Some(TriggerEvent::BeginningOfUpkeep),
                    Some(p) if p.eq_ignore_ascii_case("EndOfTurn") || p.eq_ignore_ascii_case("End") => {
                        Some(TriggerEvent::BeginningOfEndStep)
                    }
                    Some("BeginCombat") => Some(TriggerEvent::BeginningOfCombat),
                    Some("Draw") => Some(TriggerEvent::BeginningOfDraw),
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
                    // ValidPlayer$ Player.Chosen (Black Vise): fires only on the
                    // turn of the player chosen by the ETB ChoosePlayer replacement.
                    let is_chosen_player_only = valid_player == Some("Player.Chosen");
                    // ValidPlayer$ Player.EnchantedController (Paralyze): fires on
                    // the upkeep of the ENCHANTED creature's controller — a
                    // different player than this curse Aura's controller. The
                    // firing sites gate on the attached creature's controller.
                    let is_enchanted_controller_only = valid_player == Some("Player.EnchantedController");

                    // Check if we have Execute$ parameter (references a SVar with effects)
                    // Use pre-parsed SVars for O(1) lookup
                    if let Some(exec_ref) = params.get("Execute") {
                        if let Some(svar_params) = self.parsed_svars.get(exec_ref) {
                            // DB$ DealDamage effects on a phase trigger.
                            // Two shapes are supported:
                            //   (a) fixed damage to the controller
                            //       "DB$ DealDamage | Defined$ You | NumDmg$ 1"
                            //       (e.g. Juzám Djinn) → Effect::DealDamage.
                            //   (b) variable damage to the active/chosen player,
                            //       counted against that same player:
                            //       "DB$ DealDamage | Defined$ TriggeredPlayer | NumDmg$ X"
                            //       (Karma: X = Count$Valid Swamp.ActivePlayerCtrl)
                            //       "DB$ DealDamage | Defined$ ChosenPlayer | NumDmg$ X"
                            //       (Black Vise: X = Count$ValidHand ...)
                            //     → Effect::DealDamageToTriggeredPlayer.
                            if svar_params.api_type == ApiType::DealDamage {
                                let num_dmg = svar_params.get("NumDmg").unwrap_or("1");
                                let defined = svar_params.get("Defined");

                                // A fixed numeric amount stays a plain DealDamage;
                                // anything that references an SVar (X, Y, …) is a
                                // CountExpression evaluated at trigger time.
                                let fixed_amount = num_dmg.parse::<i32>().ok();

                                match (defined, fixed_amount) {
                                    (Some("You"), Some(amount)) => {
                                        effects.push(Effect::DealDamage {
                                            target: TargetRef::Player(PlayerId::new(0)),
                                            amount,
                                        });
                                    }
                                    (Some("You"), None) => {
                                        let count = crate::core::CountExpression::parse(num_dmg, &self.svars);
                                        effects.push(Effect::DealDamageToTriggeredPlayer {
                                            count,
                                            target_self: true,
                                        });
                                    }
                                    (Some("TriggeredPlayer" | "ChosenPlayer"), _) => {
                                        let count = crate::core::CountExpression::parse(num_dmg, &self.svars);
                                        effects.push(Effect::DealDamageToTriggeredPlayer {
                                            count,
                                            target_self: false,
                                        });
                                    }
                                    _ => {}
                                }
                            }
                            // DB$ GainLife effects
                            // Example: "DB$ GainLife | Defined$ You | LifeAmount$ 1"
                            // A dynamic LifeAmount$ (X resolving to a Count$ / target
                            // characteristic) routes through GainLifeDynamic — e.g.
                            // Ivory Tower's "gain (hand size − 4) life" upkeep trigger
                            // (SVar:X:Count$ValidHand Card.YouOwn/Minus.4). The fixed
                            // path below stays for plain integer amounts.
                            if svar_params.api_type == ApiType::GainLife {
                                if let Some(effect) = self.gain_life_dynamic_from_params(svar_params) {
                                    effects.push(effect);
                                } else {
                                    let life_amount = svar_params
                                        .get("LifeAmount")
                                        .and_then(|s| s.parse::<i32>().ok())
                                        .unwrap_or(1);
                                    let target_is_controller = svar_params.get("Defined") == Some("You");

                                    if target_is_controller {
                                        effects.push(Effect::GainLife {
                                            player: PlayerId::new(0),
                                            amount: life_amount,
                                        });
                                    }
                                }
                            }
                            // DB$ Draw effects on a draw-step (or other phase)
                            // trigger. Example: Grafted Skullcap / Sylvan Library /
                            // Yawgmoth's Bargain — "At the beginning of your draw
                            // step, draw an additional card." (SVar:TrigDraw:DB$ Draw).
                            // ValidPlayer$ You makes the trigger controller-only, so
                            // the placeholder player resolves to the trigger source's
                            // owner (= active player whose draw step it is) in
                            // check_triggers_for_controller. NumCards$ (Forge) or
                            // Amount$ default to 1.
                            if svar_params.api_type == ApiType::Draw {
                                let num_cards = svar_params
                                    .get("NumCards")
                                    .or_else(|| svar_params.get("Amount"))
                                    .and_then(|s| s.parse::<u8>().ok())
                                    .unwrap_or(1);
                                // Defined$ TriggeredPlayer (Howling Mine: each
                                // player's draw step → "that player draws") routes
                                // the draw to the active player whose draw step
                                // fired, not the trigger source's controller. A
                                // bare/`Defined$ You` draw stays a placeholder (=
                                // controller / active player on a controller-only
                                // trigger like Sylvan Library).
                                let player = if svar_params.get("Defined") == Some("TriggeredPlayer") {
                                    PlayerId::triggered_player()
                                } else {
                                    PlayerId::placeholder()
                                };
                                effects.push(Effect::DrawCards {
                                    player,
                                    count: num_cards,
                                });
                            }
                            // DB$ PutCounter | Defined$ Self on a phase trigger.
                            // Whirling Dervish: "At the beginning of each end step,
                            // if CARDNAME dealt damage to an opponent this turn, put
                            // a +1/+1 counter on it." (SVar:TrigPutCounter:DB$
                            // PutCounter | Defined$ Self | CounterType$ P1P1 |
                            // CounterNum$ 1). CardId 0 placeholder = the trigger
                            // source itself, resolved in the firing site. Only
                            // Defined$ Self is handled here (the self-counter shape);
                            // other PutCounter targets on phase triggers are out of
                            // scope.
                            if svar_params.api_type == ApiType::PutCounter && svar_params.get("Defined") == Some("Self")
                            {
                                let counter_num = svar_params
                                    .get("CounterNum")
                                    .and_then(|s| s.parse::<u8>().ok())
                                    .unwrap_or(1);
                                let counter_type = svar_params
                                    .get("CounterType")
                                    .and_then(crate::core::CounterType::parse)
                                    .unwrap_or(crate::core::CounterType::P1P1);
                                effects.push(Effect::PutCounter {
                                    target: CardId::new(0),
                                    counter_type,
                                    amount: counter_num,
                                });
                            }
                            // DB$ Untap on a phase trigger (Paralyze):
                            //   "DB$ Untap | Defined$ Enchanted | UnlessCost$ 4
                            //    | UnlessPayer$ EnchantedController | UnlessSwitched$ True"
                            // "At the beginning of the upkeep of enchanted
                            // creature's controller, that player MAY pay {4}. If
                            // they do, untap the enchanted creature."
                            //
                            // The untap target is the ENCHANTED permanent
                            // (Defined$ Enchanted), resolved per-fire from the
                            // Aura's `attached_to` in check_triggers_for_controller
                            // (placeholder CardId 0 here). The optional {4} payment
                            // is modeled with the shared UnlessCostWrapper:
                            // UnlessSwitched$ True means the inner effect (untap)
                            // runs ONLY when the cost is paid — so if the
                            // controller can't or won't pay, the creature stays
                            // tapped (the doesn't-untap lock holds). This reuses
                            // the determinism-safe in-engine pay/don't-pay decision
                            // (tracked under mtg-884) rather than re-implementing
                            // it. Only handled for the Enchanted-controller phase
                            // trigger; a naive unconditional untap would make
                            // Paralyze free to escape and is explicitly avoided
                            // (mtg-646).
                            if svar_params.api_type == ApiType::Untap
                                && svar_params.get("Defined") == Some("Enchanted")
                                && is_enchanted_controller_only
                            {
                                let untap = Effect::UntapPermanent { target: CardId::new(0) };
                                effects.push(crate::loader::effect_converter::wrap_with_unless_cost(
                                    untap,
                                    svar_params,
                                ));
                            }
                            // DB$ Earthbend effects
                            // Example: "DB$ Earthbend | Num$ 8"
                            if svar_params.api_type == ApiType::Earthbend {
                                let num_counters =
                                    svar_params.get("Num").and_then(|s| s.parse::<u8>().ok()).unwrap_or(1);

                                effects.push(Effect::Earthbend {
                                    target: CardId::new(0),
                                    num_counters,
                                });

                                // Check SubAbility$ chain (e.g., DBUntap for Avatar Kyoshi)
                                if let Some(sub_ref) = svar_params.get("SubAbility") {
                                    if let Some(sub_params) = self.parsed_svars.get(sub_ref) {
                                        // DB$ Untap - untap the earthbended land
                                        if sub_params.api_type == ApiType::Untap {
                                            effects.push(Effect::UntapPermanent { target: CardId::new(0) });
                                        }
                                    }
                                }
                            }
                            // DB$ Pump effects with variable values
                            // Example: "DB$ Pump | Defined$ Self | NumAtt$ +X | NumDef$ +X"
                            // SVar:X:Count$Valid Artifact.OppCtrl
                            if svar_params.api_type == ApiType::Pump {
                                let power_str = svar_params.get("NumAtt").unwrap_or("");
                                let toughness_str = svar_params.get("NumDef").unwrap_or("");

                                // Check if either value references a variable (X) that uses Count$
                                let is_variable =
                                    power_str.contains('X') || power_str.contains('Y') || toughness_str.contains('X');

                                if is_variable {
                                    // Use svars HashMap for CountExpression parsing
                                    // Parse as variable count expressions
                                    let power_count = crate::core::CountExpression::parse(power_str, &self.svars);
                                    let toughness_count =
                                        crate::core::CountExpression::parse(toughness_str, &self.svars);

                                    effects.push(Effect::PumpCreatureVariable {
                                        target: CardId::new(0),
                                        power_count,
                                        toughness_count,
                                        keywords_granted: smallvec::SmallVec::new(),
                                    });
                                } else {
                                    // Fixed pump values
                                    let power_bonus = power_str.trim_start_matches('+').parse::<i32>().unwrap_or(0);
                                    let toughness_bonus =
                                        toughness_str.trim_start_matches('+').parse::<i32>().unwrap_or(0);

                                    if power_bonus != 0 || toughness_bonus != 0 {
                                        effects.push(Effect::PumpCreature {
                                            target: CardId::new(0),
                                            power_bonus,
                                            toughness_bonus,
                                            keywords_granted: smallvec::SmallVec::new(),
                                        });
                                    }
                                }
                            }
                            // DB$ Destroy effects (e.g. The Abyss:
                            // "DB$ Destroy | ValidTgts$ Creature.nonArtifact+ActivePlayerCtrl | NoRegen$ True").
                            // Reuse the shared `params_to_effect` converter instead of
                            // re-implementing Destroy parsing here (DRY) — it produces a
                            // DestroyPermanent with the right TargetRestriction (nonArtifact,
                            // ActivePlayerCtrl) and the no_regenerate flag. The placeholder
                            // target is resolved per-upkeep in `check_triggers_for_controller`.
                            if svar_params.api_type == ApiType::Destroy {
                                if let Some(destroy_effect) =
                                    crate::loader::effect_converter::params_to_effect(svar_params)
                                {
                                    effects.push(destroy_effect);
                                }
                            }

                            // Fallback for counter-driven self-relocation chains
                            // (All Hallow's Eve's TrigRemoveCounter →
                            // DBMoveToGraveyard → DBResurrection). We deliberately
                            // restrict this to the RemoveCounter / ChangeZone /
                            // ChangeZoneAll head SVars rather than every unhandled
                            // ApiType: the generic SVar extractor can emit effects
                            // with `Defined$` placeholders that the phase-trigger
                            // execution path does not resolve (e.g. Paralyze's
                            // `DB$ Untap | Defined$ Enchanted`), which would crash
                            // at execution. Those broader triggers stay no-ops
                            // until their own support lands.
                            if effects.is_empty()
                                && matches!(
                                    svar_params.api_type,
                                    ApiType::RemoveCounter | ApiType::ChangeZone | ApiType::ChangeZoneAll
                                )
                            {
                                effects.extend(self.extract_effects_from_svar(svar_params));
                            }
                        }
                    }

                    // Parse TriggerZones$ (zones the source must be in to fire).
                    // Defaults to [Battlefield]; All Hallow's Eve uses Exile.
                    let trigger_zones: smallvec::SmallVec<[crate::zones::Zone; 2]> = params
                        .get("TriggerZones")
                        .map(|s| {
                            s.split(',')
                                .filter_map(|z| crate::zones::Zone::from_str_lenient(z.trim()))
                                .collect()
                        })
                        .unwrap_or_default();

                    // Parse the intervening-if condition (CR 603.4):
                    //   IsPresent$ Card.Self+counters_<CMP><N>_<TYPE> | PresentZone$ <zone>
                    //   IsPresent$ Card.untapped  (Howling Mine: "if ~ is untapped")
                    // We model the `counters_…` self-condition (zone-resident
                    // upkeep triggers like All Hallow's Eve) and the tap-status
                    // self-condition (Howling Mine's each-player draw).
                    let present_self_condition = params
                        .get("IsPresent")
                        .and_then(|present| crate::core::PresentSelfCondition::parse(present));

                    // Intervening-if condition (CR 603.4):
                    //   IsPresent$ Card.Self+dealtDamageToOppThisTurn
                    // Whirling Dervish — fire only if this creature dealt damage to
                    // an opponent this turn. Matched as a tokenized clause (no
                    // substring hacks) against the per-turn engine flag.
                    let present_self_dealt_damage_to_opponent = params.get("IsPresent").is_some_and(|present| {
                        present
                            .split(['.', '+'])
                            .any(|clause| clause == "dealtDamageToOppThisTurn")
                    });

                    // Create trigger with parsed effects
                    // Set structured filter flag for controller-only triggers
                    let desc_with_flag = if is_controller_only && !effects.is_empty() {
                        format!("[controller_only] {}", description)
                    } else {
                        description
                    };

                    let mut trigger = Trigger::new(event, effects, desc_with_flag);
                    if is_controller_only {
                        trigger.controller_turn_only = true;
                    }
                    if is_chosen_player_only {
                        trigger.chosen_player_turn_only = true;
                    }
                    if is_enchanted_controller_only {
                        trigger.enchanted_controller_turn_only = true;
                    }
                    trigger.trigger_zones = trigger_zones;
                    trigger.present_self_condition = present_self_condition;
                    trigger.present_self_dealt_damage_to_opponent = present_self_dealt_damage_to_opponent;
                    triggers.push(trigger);
                }
            }

            // Parse attack triggers (Mode$ Attacks)
            // Example: T:Mode$ Attacks | ValidCard$ Card.Self | Execute$ TrigDraw | TriggerDescription$ ...
            if mode == Some("Attacks") && params.get("ValidCard").map(|s| s.as_str()) == Some("Card.Self") {
                use crate::core::{Cost, Effect, PlayerId};

                let mut effects = Vec::new();
                let mut trigger_cost: Option<Cost> = None;

                // Check if we have Execute$ parameter (references a SVar with effects)
                // Use pre-parsed SVars for O(1) lookup
                if let Some(exec_ref) = params.get("Execute") {
                    if let Some(svar_params) = self.parsed_svars.get(exec_ref) {
                        // Extract Cost$ parameter if present (for optional triggers)
                        if let Some(cost_str) = svar_params.get("Cost") {
                            trigger_cost = Cost::parse(cost_str);
                        }

                        // DB$ Draw effects (draw cards on attack)
                        if svar_params.api_type == ApiType::Draw {
                            let draw_count = svar_params
                                .get("NumCards")
                                .and_then(|s| s.parse::<u8>().ok())
                                .unwrap_or(1);
                            effects.push(Effect::DrawCards {
                                player: PlayerId::new(0),
                                count: draw_count,
                            });
                        }

                        // Check SubAbility$ chain (e.g., DBPutCounter)
                        if let Some(sub_ref) = svar_params.get("SubAbility") {
                            if let Some(sub_params) = self.parsed_svars.get(sub_ref) {
                                if sub_params.api_type == ApiType::PutCounter {
                                    let counter_num = sub_params
                                        .get("CounterNum")
                                        .and_then(|s| s.parse::<u8>().ok())
                                        .unwrap_or(1);
                                    effects.push(Effect::PutCounter {
                                        target: CardId::new(0),
                                        counter_type: crate::core::CounterType::P1P1,
                                        amount: counter_num,
                                    });
                                }
                            }
                        }

                        // DB$ PutCounter effects directly in body (for simpler cards)
                        if svar_params.api_type == ApiType::PutCounter
                            && !effects.iter().any(|e| matches!(e, Effect::PutCounter { .. }))
                        {
                            let counter_num = svar_params
                                .get("CounterNum")
                                .and_then(|s| s.parse::<u8>().ok())
                                .unwrap_or(1);
                            effects.push(Effect::PutCounter {
                                target: CardId::new(0),
                                counter_type: crate::core::CounterType::P1P1,
                                amount: counter_num,
                            });
                        }

                        // DB$ GainLife effects
                        if svar_params.api_type == ApiType::GainLife {
                            let life_amount = svar_params
                                .get("LifeAmount")
                                .and_then(|s| s.parse::<i32>().ok())
                                .unwrap_or(1);
                            effects.push(Effect::GainLife {
                                player: PlayerId::new(0),
                                amount: life_amount,
                            });
                        }

                        // DB$ DealDamage effects
                        if svar_params.api_type == ApiType::DealDamage {
                            let damage_amount = svar_params
                                .get("NumDmg")
                                .and_then(|s| s.parse::<i32>().ok())
                                .unwrap_or(1);
                            effects.push(Effect::DealDamage {
                                target: TargetRef::None,
                                amount: damage_amount,
                            });
                        }

                        // AB$ Mana / DB$ Mana effects (Firebending from attack triggers)
                        if svar_params.api_type == ApiType::Mana {
                            let is_combat_mana = svar_params.get("CombatMana") == Some("True");
                            let produced = svar_params.get("Produced").unwrap_or("C");

                            // Check if amount is X (variable based on sacrificed creature's power)
                            let amount_str = svar_params.get("Amount").unwrap_or("1");
                            let amount = if amount_str == "X" {
                                // Check if X is defined as Sacrificed$CardPower
                                let x_value = self.svars.get("X").map(|s| s.as_str());
                                if x_value == Some("Sacrificed$CardPower") {
                                    254u8 // Sentinel: use sacrificed creature's power
                                } else {
                                    0u8 // X from other source - treat as creature's own power
                                }
                            } else {
                                amount_str.parse::<u8>().unwrap_or(1)
                            };

                            // Only support combat red mana for now (Firebending style)
                            if is_combat_mana && produced.contains('R') {
                                effects.push(Effect::Firebend {
                                    controller: PlayerId::new(0),
                                    amount,
                                });
                            }
                        }

                        // DB$ Untap effects (untap target artifact or creature)
                        // Cat-Owl: "Whenever this creature attacks, untap target artifact or creature"
                        if svar_params.api_type == ApiType::Untap {
                            // CardId::new(0) is placeholder - resolved at trigger execution time
                            effects.push(Effect::UntapPermanent { target: CardId::new(0) });
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
                let mut effects = Vec::new();

                // Check ValidCard$ to determine what spells trigger this
                // Card.nonCreature = triggers on noncreature spells (instants, sorceries, etc.)
                let valid_card = params.get("ValidCard").map(|s| s.as_str());
                let is_noncreature_only = valid_card == Some("Card.nonCreature");

                // Check if we have Execute$ parameter (references a SVar with effects)
                // Use extract_effects_from_svar helper (DRY)
                if let Some(exec_ref) = params.get("Execute") {
                    if let Some(svar_params) = self.parsed_svars.get(exec_ref) {
                        effects.extend(self.extract_effects_from_svar(svar_params));
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

                // Set structured filter flag for noncreature-only triggers
                if is_noncreature_only {
                    trigger.requires_noncreature = true;
                    if !trigger.description.contains("noncreature") {
                        trigger.description = format!("[noncreature] {}", trigger.description);
                    }
                }

                triggers.push(trigger);
            }

            // Parse Sacrifice triggers (Mode$ Sacrificed)
            // Example: T:Mode$ Sacrificed | ValidCard$ Permanent.Other | Execute$ TrigPutCounter | ValidPlayer$ You
            // This triggers when the controller sacrifices a permanent
            if mode == Some("Sacrificed") {
                let mut effects = Vec::new();

                // Check ValidCard$ to determine what sacrifices trigger this
                // Permanent.Other = triggers on other permanents (not self)
                let valid_card = params.get("ValidCard").map(|s| s.as_str());
                let is_other_only = valid_card == Some("Permanent.Other") || valid_card == Some("Card.Other");

                // Check if we have Execute$ parameter (references a SVar with effects)
                // Use extract_effects_from_svar helper (DRY)
                if let Some(exec_ref) = params.get("Execute") {
                    if let Some(svar_params) = self.parsed_svars.get(exec_ref) {
                        effects.extend(self.extract_effects_from_svar(svar_params));
                    }
                }

                // Extract description from TriggerDescription$ if available
                let description = params
                    .get("TriggerDescription")
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "Whenever you sacrifice a permanent".to_string());

                // Sacrifice triggers are NOT self-only (they trigger on OTHER cards being sacrificed)
                // Use new_any() to mark trigger_self_only = false
                let mut trigger = Trigger::new_any(TriggerEvent::Sacrificed, effects, description);

                // Set structured filter flag for "other-only" triggers
                if is_other_only {
                    trigger.requires_other = true;
                    if !trigger.description.contains("[other]") {
                        trigger.description = format!("[other] {}", trigger.description);
                    }
                }

                triggers.push(trigger);
            }

            // Parse Drawn triggers (Mode$ Drawn)
            // Example: T:Mode$ Drawn | ValidCard$ Card.YouCtrl | Number$ 2 | Execute$ TrigPutCounter
            // This triggers when the controller draws their Nth card each turn
            if mode == Some("Drawn") {
                let mut effects = Vec::new();

                // Parse Number$ to get which draw triggers this (e.g., 2 = second card drawn)
                // If not specified, triggers on every draw
                let draw_number = params.get("Number").and_then(|s| s.parse::<u8>().ok());

                // Check ValidCard$ / ValidPlayer$ to determine whose draws trigger this
                // Card.YouCtrl or Card.YouOwn = triggers on controller's draws
                // Card.OppOwn or ValidPlayer$ Opponent = triggers on opponent's draws
                let valid_card = params.get("ValidCard").map(|s| s.as_str());
                let valid_player = params.get("ValidPlayer").map(|s| s.as_str());
                let triggers_on_controller_draw = match (valid_player, valid_card) {
                    (Some("Opponent"), _) => false,
                    (_, Some(vc)) if vc.contains("Opp") => false,
                    _ => true, // Default: trigger on controller's draws
                };

                // Check if we have Execute$ parameter (references a SVar with effects)
                if let Some(exec_ref) = params.get("Execute") {
                    if let Some(svar_params) = self.parsed_svars.get(exec_ref) {
                        effects.extend(self.extract_effects_from_svar(svar_params));
                    }
                }

                // Extract description from TriggerDescription$ if available
                let description = params
                    .get("TriggerDescription")
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| {
                        if let Some(n) = draw_number {
                            format!("Whenever you draw your {} card each turn", ordinal(n))
                        } else {
                            "Whenever you draw a card".to_string()
                        }
                    });

                // Create trigger with CardDrawn event
                // Draw triggers are NOT self-only (they watch for draw events, not card ETB)
                let mut trigger = Trigger::new_any(TriggerEvent::CardDrawn, effects, description);
                trigger.draw_number = draw_number;
                trigger.triggers_on_controller_draw = triggers_on_controller_draw;

                triggers.push(trigger);
            }

            // Parse Taps triggers (Mode$ Taps)
            // Example: T:Mode$ Taps | ValidCard$ Card.Self | Execute$ TrigDraw | TriggerDescription$ Whenever CARDNAME becomes tapped, draw a card.
            // This triggers when the card becomes tapped (from untapped state)
            if mode == Some("Taps") {
                let mut effects = Vec::new();

                // Check if we have Execute$ parameter (references a SVar with effects)
                if let Some(exec_ref) = params.get("Execute") {
                    if let Some(svar_params) = self.parsed_svars.get(exec_ref) {
                        effects.extend(self.extract_effects_from_svar(svar_params));
                    }
                }

                // Check ValidCard$ to determine which card triggers this
                // Card.Self = triggers only when this card becomes tapped (default)
                // Other patterns could allow for "whenever any creature becomes tapped"
                let valid_card = params.get("ValidCard").map(|s| s.as_str());
                let trigger_self_only = match valid_card {
                    Some("Card.Self") | None => true,
                    _ => false, // Other ValidCard patterns trigger on other cards
                };

                // Extract description from TriggerDescription$ if available
                let description = params
                    .get("TriggerDescription")
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "Whenever this permanent becomes tapped".to_string());

                // Create trigger with Taps event
                let trigger = if trigger_self_only {
                    Trigger::new(TriggerEvent::Taps, effects, description)
                } else {
                    Trigger::new_any(TriggerEvent::Taps, effects, description)
                };

                triggers.push(trigger);
            }

            // Parse TapsForMana triggers (Mode$ TapsForMana)
            // Example: T:Mode$ TapsForMana | ValidCard$ Creature | Activator$ You | Execute$ TrigMana
            if mode == Some("TapsForMana") {
                let mut effects = Vec::new();

                if let Some(exec_ref) = params.get("Execute") {
                    if let Some(svar_params) = self.parsed_svars.get(exec_ref) {
                        effects.extend(self.extract_effects_from_svar(svar_params));
                    }
                }

                let valid_card = params.get("ValidCard").map(|s| s.to_string());
                let activator = params.get("Activator").map(|s| s.to_string());

                let description = params
                    .get("TriggerDescription")
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "Whenever a permanent is tapped for mana".to_string());

                let mut trigger = Trigger::new_any(TriggerEvent::TapsForMana, effects, description);
                trigger.taps_for_mana_valid_card = valid_card;
                trigger.taps_for_mana_activator = activator;

                triggers.push(trigger);
            }

            // Parse AttackersDeclared triggers (Mode$ AttackersDeclared)
            // Example: T:Mode$ AttackersDeclared | AttackingPlayer$ You | ValidAttackers$ Creature.withFlying | Execute$ TrigDraw
            // This triggers once when attackers are declared, not per-creature
            if mode == Some("AttackersDeclared") {
                use crate::core::Keyword;

                let mut effects = Vec::new();

                // Check if we have Execute$ parameter (references a SVar with effects)
                if let Some(exec_ref) = params.get("Execute") {
                    if let Some(svar_params) = self.parsed_svars.get(exec_ref) {
                        effects.extend(self.extract_effects_from_svar(svar_params));
                    }
                }

                // Check AttackingPlayer$ to determine who triggers this
                // You = triggers only when controller attacks (default)
                // Opponent = triggers when opponent attacks
                let attacking_player = params.get("AttackingPlayer").map(|s| s.as_str());
                let controller_turn_only = match attacking_player {
                    Some("You") | None => true,
                    Some("Opponent") => false,
                    _ => true,
                };

                // Check ValidAttackers$ for keyword filter
                // Creature.withFlying = only triggers if a flying creature attacks
                let valid_attackers = params.get("ValidAttackers").map(|s| s.as_str());
                let valid_attackers_keyword = match valid_attackers {
                    Some(s) if s.contains("withFlying") => Some(Keyword::Flying),
                    Some(s) if s.contains("withVigilance") => Some(Keyword::Vigilance),
                    Some(s) if s.contains("withTrample") => Some(Keyword::Trample),
                    Some(s) if s.contains("withFirstStrike") => Some(Keyword::FirstStrike),
                    Some(s) if s.contains("withDoubleStrike") => Some(Keyword::DoubleStrike),
                    Some(s) if s.contains("withDeathtouch") => Some(Keyword::Deathtouch),
                    Some(s) if s.contains("withLifelink") => Some(Keyword::Lifelink),
                    Some(s) if s.contains("withHaste") => Some(Keyword::Haste),
                    Some(s) if s.contains("withReach") => Some(Keyword::Reach),
                    _ => None,
                };

                // Extract description from TriggerDescription$ if available
                let description = params
                    .get("TriggerDescription")
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "Whenever one or more creatures attack".to_string());

                // Create trigger with AttackersDeclared event
                // This is NOT a self-trigger - it monitors all attackers
                let mut trigger = Trigger::new_any(TriggerEvent::AttackersDeclared, effects, description);
                trigger.controller_turn_only = controller_turn_only;
                trigger.valid_attackers_keyword = valid_attackers_keyword;

                triggers.push(trigger);
            }

            // Parse DamageDone triggers (Mode$ DamageDone)
            // Example: T:Mode$ DamageDone | ValidSource$ Card.Self | ValidTarget$ Player | CombatDamage$ True | Execute$ TrigDiscard
            // This triggers when a creature deals damage to a player or creature
            if mode == Some("DamageDone") {
                let mut effects = Vec::new();

                // Check if we have Execute$ parameter (references a SVar with effects)
                if let Some(exec_ref) = params.get("Execute") {
                    if let Some(svar_params) = self.parsed_svars.get(exec_ref) {
                        effects.extend(self.extract_effects_from_svar(svar_params));
                    }
                }

                // Check ValidSource$ to determine whose damage triggers this
                // Card.Self = triggers only when this creature deals damage (default)
                // Creature.YouCtrl = triggers when any creature you control deals damage
                let valid_source = params.get("ValidSource").map(|s| s.as_str());
                let trigger_self_only = match valid_source {
                    Some("Card.Self" | "Creature.Self") | None => true,
                    _ => false, // Other ValidSource patterns trigger on other creatures
                };

                // Check ValidTarget$ to determine which combat-damage recipient
                // class fires this trigger (CR 510.2: combat damage is one
                // simultaneous event; ValidTarget$ restricts which recipients
                // the trigger watches).
                //   Player / Opponent / Planeswalker / Battle  -> Player
                //   Creature (and only Creature)               -> Creature
                //   absent                                     -> Any (default)
                // Complex sub-filters (e.g. ValidTarget$
                // Player.withMoreLandsThanYou) collapse to the coarse
                // player/creature class here; their finer predicate is not yet
                // enforced (matches prior behavior -- see mtg-m43mc).
                let valid_target = params.get("ValidTarget").map(|s| s.as_str());
                let combat_damage_target = match valid_target {
                    None => crate::core::CombatDamageTarget::Any,
                    Some(t)
                        if t.contains("Player")
                            || t.contains("Opponent")
                            || t.contains("Planeswalker")
                            || t.contains("Battle") =>
                    {
                        // Mixed "Creature,Player" targets still reach a player;
                        // treat as Player so player damage fires it.
                        crate::core::CombatDamageTarget::Player
                    }
                    Some(t) if t.contains("Creature") => crate::core::CombatDamageTarget::Creature,
                    // Other patterns (You, Card.Self, ...): default to Any so the
                    // trigger is not silently suppressed.
                    Some(_) => crate::core::CombatDamageTarget::Any,
                };

                // Check CombatDamage$ to determine if this requires combat damage only
                // True = only combat damage triggers this
                // If absent, any damage (combat or non-combat) triggers
                let combat_damage = params.get("CombatDamage").map(|s| s.as_str());
                let combat_damage_only = combat_damage == Some("True");

                // Check OptionalDecider$ for optional triggers
                let optional = params.contains_key("OptionalDecider");

                // Extract description from TriggerDescription$ if available
                let description = params
                    .get("TriggerDescription")
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| {
                        if combat_damage_only {
                            "Whenever this creature deals combat damage to a player".to_string()
                        } else {
                            "Whenever this creature deals damage to a player".to_string()
                        }
                    });

                // Create trigger with DealsCombatDamage event
                // For now, use DealsCombatDamage for both combat and non-combat triggers
                // The runtime can check combat_damage_only flag if needed
                let mut trigger = if trigger_self_only {
                    if optional {
                        Trigger::new_optional(TriggerEvent::DealsCombatDamage, effects, description)
                    } else {
                        Trigger::new(TriggerEvent::DealsCombatDamage, effects, description)
                    }
                } else {
                    let mut t = Trigger::new_any(TriggerEvent::DealsCombatDamage, effects, description);
                    t.optional = optional;
                    t
                };

                // Structured recipient-class filter consumed at the combat-damage
                // firing site (replaces the former dead `[any-damage]` /
                // `[damages-creature]` description markers that had no consumer).
                trigger.combat_damage_target = combat_damage_target;

                // `combat_damage_only` (CombatDamage$ True, e.g. Hypnotic
                // Specter "deals COMBAT damage to a player") records that this
                // trigger must NOT fire on non-combat damage. Both kinds share
                // the `DealsCombatDamage` event but have distinct firing sites;
                // the non-combat site (mtg-r9po1) consults this flag to skip
                // combat-only triggers, while the combat site fires them all.
                trigger.requires_combat_damage = combat_damage_only;

                triggers.push(trigger);
            }

            // Parse DamageDealtOnce triggers (Mode$ DamageDealtOnce)
            // Example (Spirit Link):
            //   T:Mode$ DamageDealtOnce | ValidSource$ Card.AttachedBy | Execute$ TrigGain
            //     | TriggerZones$ Battlefield
            //     | TriggerDescription$ Whenever enchanted creature deals damage, you gain that much life.
            // DamageDealtOnce aggregates all simultaneous damage from the source into a
            // single trigger (lifelink-batched, CR 119.3/702.15-style). We model it on
            // the same DealsCombatDamage firing site as DamageDone but with the
            // damage-amount available to the executed effects via TriggerCount$DamageAmount.
            if mode == Some("DamageDealtOnce") {
                // Resolve the executed effects (the Execute$ SVar chain). This reuses
                // extract_effects_from_svar so a `DB$ GainLife | LifeAmount$ X` with
                // `SVar:X:TriggerCount$DamageAmount` becomes a dynamic GainLife driven
                // by the damage just dealt.
                let mut effects = Vec::new();
                if let Some(exec_ref) = params.get("Execute") {
                    if let Some(svar_params) = self.parsed_svars.get(exec_ref) {
                        effects.extend(self.extract_effects_from_svar(svar_params));
                    }
                }

                // ValidSource$ Card.AttachedBy -> fire when the host (the permanent
                // this card is attached to) deals damage. Other ValidSource patterns
                // (Card.Self) fall back to the self-only case.
                let valid_source = params.get("ValidSource").map(|s| s.as_str());
                let attached_source = valid_source == Some("Card.AttachedBy");
                let self_only = matches!(valid_source, Some("Card.Self" | "Creature.Self") | None);

                let optional = params.contains_key("OptionalDecider");

                let description = params
                    .get("TriggerDescription")
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "Whenever enchanted permanent deals damage".to_string());

                let mut trigger = if self_only {
                    if optional {
                        Trigger::new_optional(TriggerEvent::DealsCombatDamage, effects, description)
                    } else {
                        Trigger::new(TriggerEvent::DealsCombatDamage, effects, description)
                    }
                } else {
                    let mut t = Trigger::new_any(TriggerEvent::DealsCombatDamage, effects, description);
                    t.optional = optional;
                    t
                };

                trigger.requires_attached_source = attached_source;

                // TriggerZones$ (defaults to [Battlefield]); the Aura lives on the
                // battlefield while watching its host.
                trigger.trigger_zones = params
                    .get("TriggerZones")
                    .map(|s| {
                        s.split(',')
                            .filter_map(|z| crate::zones::Zone::from_str_lenient(z.trim()))
                            .collect()
                    })
                    .unwrap_or_default();

                triggers.push(trigger);
            }

            // Parse Discarded triggers (Mode$ Discarded). Two distinct shapes:
            //
            // (a) ValidCard$ Card.Self — the DISCARDED CARD ITSELF triggers, on
            //     its LKI as it leaves hand (CR 603.6/603.10). Psychic Purge:
            //       T:Mode$ Discarded | ValidCard$ Card.Self
            //         | ValidCause$ SpellAbility.OppCtrl | Execute$ TrigLoseLife
            //       SVar:TrigLoseLife:DB$ LoseLife
            //         | Defined$ TriggeredCauseController | LifeAmount$ 5
            //     -> TriggerEvent::Discarded, with requires_opponent_cause from
            //        ValidCause$ ...OppCtrl, and a LoseLife targeting the cause
            //        controller (resolved at fire time in discard_card).
            //
            // (b) ValidCard$ Card.YouOwn — a battlefield permanent watching its
            //     controller's discards (Monument to Endurance):
            //       T:Mode$ Discarded | ValidCard$ Card.YouOwn | TriggerZones$
            //         Battlefield | Execute$ TrigCharm
            //     -> TriggerEvent::CardDiscarded (the long-standing behavior).
            if mode == Some("Discarded") {
                let valid_card = params.get("ValidCard").map(|s| s.as_str());

                if valid_card == Some("Card.Self") {
                    // Self-discard punisher (Psychic Purge). Build the LoseLife
                    // effect explicitly so its target is the cause-controller
                    // sentinel rather than the generic placeholder (= controller)
                    // the SVar extractor would emit.
                    let mut effects = Vec::new();
                    if let Some(exec_ref) = params.get("Execute") {
                        if let Some(svar_params) = self.parsed_svars.get(exec_ref) {
                            if svar_params.api_type == ApiType::LoseLife
                                && svar_params.get("Defined") == Some("TriggeredCauseController")
                            {
                                if let Some(amount) = svar_params.get("LifeAmount").and_then(|s| s.parse::<i32>().ok())
                                {
                                    effects.push(Effect::LoseLife {
                                        player: PlayerId::triggered_cause_controller(),
                                        amount,
                                    });
                                }
                            }
                        }
                    }

                    // Only register the trigger if we lowered its effect; an
                    // unrecognized Execute shape stays a no-op rather than a
                    // silently-wrong trigger.
                    if !effects.is_empty() {
                        let description = params
                            .get("TriggerDescription")
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| "When this card is discarded".to_string());

                        // ValidCause$ SpellAbility.OppCtrl => only fire when an
                        // OPPONENT's spell/ability caused the discard.
                        let requires_opponent_cause = params.get("ValidCause").is_some_and(|c| c.contains("OppCtrl"));

                        let mut trigger = Trigger::new(TriggerEvent::Discarded, effects, description);
                        trigger.requires_opponent_cause = requires_opponent_cause;
                        // The trigger source IS the discarded card, fired from the
                        // graveyard on its LKI; clear the default battlefield-only
                        // trigger_zones gate so the firing site (discard_card)
                        // controls when it fires.
                        trigger.trigger_zones = smallvec::SmallVec::new();
                        triggers.push(trigger);
                    }
                } else {
                    // (b) Battlefield watcher (Monument to Endurance). ValidCard$
                    // Card.YouOwn = "when the trigger's controller discards"
                    // (other ValidCard patterns not yet distinguished).
                    let mut effects = Vec::new();
                    if let Some(exec_ref) = params.get("Execute") {
                        if let Some(svar_params) = self.parsed_svars.get(exec_ref) {
                            effects.extend(self.extract_effects_from_svar(svar_params));
                        }
                    }

                    let description = params
                        .get("TriggerDescription")
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "Whenever you discard a card".to_string());

                    let trigger = Trigger::new_any(TriggerEvent::CardDiscarded, effects, description);
                    triggers.push(trigger);
                }
            }
        }

        // Parse ClassLevelGained triggers from K:Class:N:cost:AddTrigger$ X entries.
        // These are one-shot triggers that fire when the Class first reaches level N.
        // They are placed on the card at load-time so check_triggers() can find them
        // when execute_class_level_up fires TriggerEvent::ClassLevelGained { level: N }.
        for kw_str in &self.raw_keywords {
            // Match K:Class:N:cost:AddTrigger$ X
            let Some(rest) = kw_str.strip_prefix("Class:") else {
                continue;
            };
            // rest = "N:cost:abilities"
            let mut parts = rest.splitn(3, ':');
            let level_str = parts.next().unwrap_or("");
            let _cost = parts.next().unwrap_or("");
            let abilities = parts.next().unwrap_or("");
            let Ok(level) = level_str.trim().parse::<u8>() else {
                continue;
            };

            // Parse the abilities portion to look for AddTrigger$ X
            let ability_params = tokenize_pipe_dollar(abilities);
            let Some(svar_name) = ability_params.get("AddTrigger") else {
                continue;
            };

            // Look up the SVar body
            let Some(svar_body) = self.svars.get(svar_name.as_str()) else {
                continue;
            };

            // Parse the SVar body to check its mode
            let svar_params = tokenize_pipe_dollar(svar_body);
            let Some(mode) = svar_params.get("Mode") else { continue };

            if mode == "ClassLevelGained" {
                // One-time trigger: fires when the Class reaches `level`.
                // Parse effects from the Execute$ SVar reference.
                let mut effects = Vec::new();
                if let Some(exec_ref) = svar_params.get("Execute") {
                    if let Some(svar_params) = self.parsed_svars.get(exec_ref.as_str()) {
                        effects.extend(self.extract_effects_from_svar(svar_params));
                    }
                }

                let description = svar_params
                    .get("TriggerDescription")
                    .cloned()
                    .unwrap_or_else(|| format!("When this Class becomes level {}", level));

                // One-shot trigger: fires only on this card (trigger_self_only=true)
                let trigger = Trigger::new(TriggerEvent::ClassLevelGained { level }, effects, description);
                triggers.push(trigger);
            }
        }

        triggers
    }

    /// Parse activated abilities (A:AB$ lines)
    ///
    /// Uses tokenized parsing with params_to_effect() for all effect types.
    /// Eliminates unsafe substring matching.
    pub(crate) fn parse_activated_abilities(&self) -> Vec<crate::core::ActivatedAbility> {
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
            use super::effect_converter::params_to_effect_with_svars;

            // Special handling for mana abilities (need is_mana_ability = true)
            // BUT: Planeswalker loyalty abilities that produce mana (e.g., Chandra's +1: Add {R}{R})
            // are NOT regular mana abilities for the mana engine - they have loyalty costs
            // and can only be activated once per turn at sorcery speed.
            let is_planeswalker_ability = params
                .get("Planeswalker")
                .map(|s| s.eq_ignore_ascii_case("True"))
                .unwrap_or(false);
            // ManaReflected (Fellwar Stone) is also a mana ability (CR 605): it
            // adds mana, doesn't use the stack, and has no target.
            let is_reflected_mana = matches!(params.api_type, ApiType::ManaReflected);
            let is_mana_ability =
                matches!(params.api_type, ApiType::Mana | ApiType::ManaReflected) && !is_planeswalker_ability;

            // Try to convert parameters to effects (with SVar resolution for StaticAbilities$)
            let mut effects = if let Some(effect) = params_to_effect_with_svars(&params, &self.svars) {
                vec![effect]
            } else {
                // Fallback to old parsing for unsupported API types
                // TODO: Remove this once all API types are migrated
                vec![]
            };

            // Follow SubAbility$ chain to parse additional effects
            // (e.g., Bazaar of Baghdad: AB$ Draw | SubAbility$ DBDiscard)
            if !effects.is_empty() {
                self.follow_sub_ability_chain(&params, &mut effects);
            }

            // Extract description
            let description = params
                .get("SpellDescription")
                .unwrap_or("Activated ability")
                .to_string();

            // Check for SorcerySpeed$ True parameter
            let is_sorcery_speed = params
                .get("SorcerySpeed")
                .map(|s| s.eq_ignore_ascii_case("True"))
                .unwrap_or(false);

            // Check for PlayerTurn$ True parameter (activate only during your turn)
            let is_your_turn_only = params
                .get("PlayerTurn")
                .map(|s| s.eq_ignore_ascii_case("True"))
                .unwrap_or(false);

            // Check for Exhaust$ True parameter (can only activate once per game)
            let is_exhaust = params
                .get("Exhaust")
                .map(|s| s.eq_ignore_ascii_case("True"))
                .unwrap_or(false);

            // Parse "Activate only if ..." restriction:
            //   IsPresent$ <filter> | PresentZone$ <zone> | PresentCompare$ <op><n>
            // e.g. Library of Alexandria: IsPresent$ Card.YouOwn | PresentZone$ Hand
            //      | PresentCompare$ EQ7 ("exactly seven cards in hand").
            let activation_condition = Self::parse_activation_condition(&params);

            // Parse ActivationZone$ — zone in which this ability may be activated.
            // Defaults to Battlefield when absent (the common case, CR 602.1).
            // E.g. "ActivationZone$ Graveyard" allows activation while the card is
            // in the owner's graveyard (unearth, graveyard-recursion).
            let activation_zone = params
                .get("ActivationZone")
                .and_then(crate::zones::Zone::from_str_lenient)
                .unwrap_or(crate::zones::Zone::Battlefield);

            // Parse ActivationPhases$ <start>-><end> — restricts the ability to
            // a turn-step window (Jade Statue's combat-only `BeginCombat->
            // EndCombat` animate, CR 602.5). Absent for most abilities. The
            // single-range form is modelled here; the disjoint multi-range form
            // (`Upkeep->Main1,Main2->Cleanup`) parses to `None` and the ability
            // is left unrestricted for now (debug-logged, not warned, since it
            // is valid-but-unmodelled syntax — see ActivationPhaseWindow::parse).
            let activation_phases = match params.get("ActivationPhases") {
                Some(value) => match crate::core::ActivationPhaseWindow::parse(value) {
                    Some(window) => Some(window),
                    None => {
                        log::debug!(
                            "ActivationPhases$ '{value}' not modelled as a single range; ability left unrestricted"
                        );
                        None
                    }
                },
                None => None,
            };

            // Only add if we have effects
            if !effects.is_empty() {
                let mut ability = if is_sorcery_speed || is_planeswalker_ability {
                    ActivatedAbility::new_sorcery_speed(cost, effects, description)
                } else if is_your_turn_only {
                    ActivatedAbility::new_your_turn_only(cost, effects, description, is_mana_ability)
                } else {
                    ActivatedAbility::new(cost, effects, description, is_mana_ability)
                };
                // Set exhaust flag if applicable
                if is_exhaust {
                    ability.exhaust = true;
                }
                // Flag reflected-mana abilities (Fellwar Stone) so the activation
                // path constrains the produced color to the reflected set.
                if is_reflected_mana {
                    ability.produces_reflected_mana = true;
                }
                ability.activation_condition = activation_condition;
                ability.activation_zone = activation_zone;
                ability.activation_phases = activation_phases;
                abilities.push(ability);
            }
        }

        abilities
    }

    /// Parse an "Activate only if ..." restriction from an activated ability's
    /// `IsPresent$ | PresentZone$ | PresentCompare$` parameters.
    ///
    /// Returns `None` when the ability has no such restriction (the common case)
    /// or when the `PresentCompare$` operator/count cannot be parsed. Examples:
    /// - Library of Alexandria: `IsPresent$ Card.YouOwn | PresentZone$ Hand |
    ///   PresentCompare$ EQ7` → exactly 7 cards in hand.
    /// - Cryptic Caves: `IsPresent$ Land.YouCtrl | PresentCompare$ GE5` → 5+
    ///   lands on the battlefield (zone defaults to Battlefield).
    fn parse_activation_condition(
        params: &super::ability_parser::AbilityParams,
    ) -> Option<crate::core::ActivationCondition> {
        use crate::core::{ActivationCondition, CompareOp};

        let filter = params.get("IsPresent")?;
        let compare = params.get("PresentCompare")?;

        // PresentCompare is "<OP><N>", e.g. "EQ7", "GE5". Split the 2-char op.
        if compare.len() < 3 {
            return None;
        }
        let (op_str, n_str) = compare.split_at(2);
        let op = CompareOp::parse(op_str)?;
        let count: u8 = n_str.parse().ok()?;

        // PresentZone defaults to Battlefield when absent.
        let zone = params
            .get("PresentZone")
            .and_then(|z| match z {
                "Hand" => Some(crate::zones::Zone::Hand),
                "Graveyard" => Some(crate::zones::Zone::Graveyard),
                "Battlefield" => Some(crate::zones::Zone::Battlefield),
                "Exile" => Some(crate::zones::Zone::Exile),
                "Library" => Some(crate::zones::Zone::Library),
                _ => None,
            })
            .unwrap_or(crate::zones::Zone::Battlefield);

        Some(ActivationCondition {
            filter: filter.to_string(),
            zone,
            op,
            count,
        })
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
    /// Resolve the sacrifice/destroy filter for a `T:Mode$ Always` state
    /// trigger. The authoritative filter is the `ValidCards$` of the
    /// `SacrificeAll`/`DestroyAll` effect named by `Execute$` (City in a
    /// Bottle: `SVar:TrigSac:DB$ SacrificeAll | ValidCards$ ...`). Falls back
    /// to the trigger's own `IsPresent$` filter. Returns `None` if neither is
    /// present (so unrelated `Mode$ Always` triggers are left alone).
    fn always_sweep_restriction(
        &self,
        params: &std::collections::HashMap<String, String>,
    ) -> Option<crate::core::TargetRestriction> {
        // Prefer the Execute$ SVar's ValidCards$.
        if let Some(execute) = params.get("Execute") {
            if let Some(svar_body) = self.svars.get(execute) {
                let svar_params = tokenize_pipe_dollar(svar_body);
                // Only SacrificeAll / DestroyAll forms map to the sweep.
                let api = svar_params.get("DB").or_else(|| svar_params.get("SP"));
                if matches!(api.map(String::as_str), Some("SacrificeAll" | "DestroyAll")) {
                    if let Some(valid) = svar_params.get("ValidCards") {
                        return Some(crate::core::TargetRestriction::parse(valid));
                    }
                }
            }
        }
        // Fallback: the IsPresent$ condition filter.
        params
            .get("IsPresent")
            .map(|v| crate::core::TargetRestriction::parse(v))
    }

    /// Returns `true` iff every dotted modifier in a `ValidAttacker$` filter is
    /// one that [`crate::core::TargetRestriction::parse`] + `matches` evaluate
    /// faithfully. Used to gate `Mode$ CantBlockBy | ValidBlocker$ Creature.Self`
    /// emission: a modifier we silently ignore (e.g. `withoutFlying`) would
    /// collapse the filter to a bare type and wrongly forbid blocking ALL such
    /// attackers, so we decline to model those (no worse than the prior
    /// silent-drop). Base types and the `Self`/`Other` source qualifiers are
    /// fine; controller restrictions (`YouCtrl`/`OppCtrl`) are NOT, since the
    /// matcher used at the block site has no controller context.
    fn cant_block_filter_is_faithful(filter_str: &str) -> bool {
        for clause in filter_str.split(',') {
            let mut parts = clause.split('.');
            // Base type is always fine (unknown bases parse to "any type").
            let _base = parts.next();
            for modifier_group in parts {
                for modifier in modifier_group.split('+') {
                    let ok = matches!(
                        modifier,
                        "!HasCounters" | "!token" | "nonArtifact" | "White" | "Blue" | "Black" | "Red" | "Green"
                    ) || modifier.starts_with("powerGE")
                        || modifier.starts_with("powerLE");
                    if !ok {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// Parse static abilities (`S:` lines) from the card definition.
    ///
    /// # Panics
    ///
    /// Panics if the first affected selector is absent after building a
    /// non-empty selector list (internal invariant; always preceded by a
    /// `!is_empty()` check in practice).
    pub fn parse_static_abilities(&self) -> Vec<crate::core::StaticAbility> {
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

            // Pattern: TYPE.attacking+YouCtrl (e.g., "Vampire.attacking+YouCtrl")
            // For attacking creatures of a specific type you control
            if value.ends_with(".attacking+YouCtrl") {
                let subtype = value.strip_suffix(".attacking+YouCtrl")?;
                if subtype != "Creature" && subtype != "Card" && subtype != "Land" {
                    return Some(AffectedSelector::AttackingCreatureTypeYouControl {
                        subtype: crate::core::Subtype::new(subtype),
                    });
                }
            }

            // Pattern: TYPE.YouCtrl+equipped (e.g., "Warrior.YouCtrl+equipped")
            // For equipped creatures of a specific type you control
            if value.ends_with(".YouCtrl+equipped") || value.ends_with("+YouCtrl+equipped") {
                let subtype = if value.ends_with(".YouCtrl+equipped") {
                    value.strip_suffix(".YouCtrl+equipped")?
                } else {
                    // Handle Creature.TYPE+YouCtrl+equipped format
                    let remainder = value.strip_prefix("Creature.")?;
                    remainder.strip_suffix("+YouCtrl+equipped")?.split('+').next()?
                };
                if subtype != "Creature" && subtype != "Card" && subtype != "Land" {
                    return Some(AffectedSelector::EquippedCreatureTypeYouControl {
                        subtype: crate::core::Subtype::new(subtype),
                    });
                }
            }

            // Pattern: TYPE.YouCtrl+Legendary or TYPE.Legendary+YouCtrl (e.g., "Human.YouCtrl+Legendary")
            // For legendary creatures of a specific type you control
            if (value.ends_with("+Legendary") && value.contains("+YouCtrl"))
                || (value.ends_with("+YouCtrl") && value.contains(".Legendary"))
            {
                // Extract the subtype from various formats
                let subtype = if value.contains(".Legendary+YouCtrl") {
                    // Format: TYPE.Legendary+YouCtrl (e.g., "Snake.Legendary+YouCtrl")
                    value.split('.').next()?
                } else if value.contains(".YouCtrl+Legendary") {
                    // Format: TYPE.YouCtrl+Legendary (e.g., "Human.YouCtrl+Legendary")
                    value
                        .strip_suffix("+Legendary")?
                        .strip_suffix("+YouCtrl")?
                        .strip_suffix(".YouCtrl")?
                        .split('.')
                        .next()?
                } else {
                    return None;
                };
                if subtype != "Creature" && subtype != "Card" && subtype != "Land" && subtype != "Permanent" {
                    return Some(AffectedSelector::LegendarySubtypeYouControl {
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

            // Pattern: Card.SUBTYPE+Other+YouCtrl (e.g., "Card.Human+Other+YouCtrl")
            // For effects that buff other cards of a specific type you control
            if value.starts_with("Card.") && value.ends_with("+Other+YouCtrl") {
                let remainder = value.strip_prefix("Card.")?;
                let subtype = remainder.strip_suffix("+Other+YouCtrl")?;
                // Skip already-handled patterns
                if !subtype.contains('+') && !subtype.starts_with("non") {
                    return Some(AffectedSelector::CardSubtypeOtherYouControl {
                        subtype: crate::core::Subtype::new(subtype),
                    });
                }
            }

            // Pattern: Card.SUBTYPE+YouCtrl+Other (alternate ordering)
            if value.starts_with("Card.") && value.ends_with("+YouCtrl+Other") {
                let remainder = value.strip_prefix("Card.")?;
                let subtype = remainder.strip_suffix("+YouCtrl+Other")?;
                if !subtype.contains('+') && !subtype.starts_with("non") {
                    return Some(AffectedSelector::CardSubtypeOtherYouControl {
                        subtype: crate::core::Subtype::new(subtype),
                    });
                }
            }

            // Pattern: Card.SUBTYPE+YouCtrl (e.g., "Card.Horror+YouCtrl")
            // For effects that affect cards of a specific type you control
            if value.starts_with("Card.") && value.ends_with("+YouCtrl") && !value.contains("+Other") {
                let remainder = value.strip_prefix("Card.")?;
                let subtype = remainder.strip_suffix("+YouCtrl")?;
                // Skip already-handled patterns (Creature, Enchantment, Treasure, etc.)
                if !subtype.contains('+')
                    && !subtype.starts_with("non")
                    && !matches!(
                        subtype,
                        "Creature" | "Enchantment" | "Artifact" | "Treasure" | "Historic" | "IsCommander"
                    )
                {
                    return Some(AffectedSelector::CardSubtypeYouControl {
                        subtype: crate::core::Subtype::new(subtype),
                    });
                }
            }

            // Pattern: Permanent.SUBTYPE+Other+YouCtrl (e.g., "Permanent.Dwarf+Other+YouCtrl")
            // For effects that buff other permanents of a specific type
            if value.starts_with("Permanent.") && value.ends_with("+Other+YouCtrl") {
                let remainder = value.strip_prefix("Permanent.")?;
                let subtype = remainder.strip_suffix("+Other+YouCtrl")?;
                // Skip already-handled patterns
                if !subtype.contains('+') && !subtype.starts_with("non") && subtype != "Legendary" {
                    return Some(AffectedSelector::PermanentSubtypeOtherYouControl {
                        subtype: crate::core::Subtype::new(subtype),
                    });
                }
            }

            // Pattern: Permanent.SUBTYPE+YouCtrl+Other (alternate ordering)
            if value.starts_with("Permanent.") && value.ends_with("+YouCtrl+Other") {
                let remainder = value.strip_prefix("Permanent.")?;
                let subtype = remainder.strip_suffix("+YouCtrl+Other")?;
                if !subtype.contains('+') && !subtype.starts_with("non") && subtype != "Legendary" {
                    return Some(AffectedSelector::PermanentSubtypeOtherYouControl {
                        subtype: crate::core::Subtype::new(subtype),
                    });
                }
            }

            // Pattern: Card.SUBTYPE+Other (e.g., "Card.Elf+Other", "Card.Merfolk+Other")
            // For effects that buff all cards of a type except self
            if value.starts_with("Card.") && value.ends_with("+Other") && !value.contains("+YouCtrl") {
                let remainder = value.strip_prefix("Card.")?;
                let subtype = remainder.strip_suffix("+Other")?;
                if !subtype.contains('+') && !subtype.starts_with("non") {
                    return Some(AffectedSelector::SubtypeOther {
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
                "Card.Self+tapped" => AffectedSelector::SelfWhenTapped,
                "Card.Self+wasCast" => AffectedSelector::SelfWhenCast,
                "Card.Self+!attacking" => AffectedSelector::SelfWhenNotAttacking,
                "Card.Self+!attacking+!blocking" => AffectedSelector::SelfWhenNotInCombat,
                "Creature.YouCtrl+equipped" => AffectedSelector::EquippedCreaturesYouControl,
                "Creature.YouCtrl+enchanted" => AffectedSelector::EnchantedCreaturesYouControl,
                "You" => AffectedSelector::You,
                "Player" => AffectedSelector::Player,
                "Land.YouCtrl" => AffectedSelector::LandsYouControl,
                "Instant.YouCtrl" => AffectedSelector::InstantYouControl,
                "Sorcery.YouCtrl" => AffectedSelector::SorceryYouControl,
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
                "Player.Opponent" => AffectedSelector::Opponent,
                "Permanent.OppCtrl" => AffectedSelector::PermanentsOpponentControls,
                "Card.Self+attacking" => AffectedSelector::SelfWhenAttacking,
                // Legendary selectors
                "Creature.Legendary+YouCtrl" | "Permanent.Legendary+YouCtrl" => AffectedSelector::LegendaryYouControl,
                "Permanent.Other+YouCtrl+Legendary" | "Permanent.Legendary+Other+YouCtrl" => {
                    AffectedSelector::LegendaryOtherYouControl
                }
                // Non-aura enchantments
                "Enchantment.nonAura+Other" | "Enchantment.Other+nonAura" => AffectedSelector::NonAuraEnchantmentsOther,
                // State-based self selectors
                "Card.Self+untapped" => AffectedSelector::SelfWhenUntapped,
                "Card.Self+IsMonstrous" => AffectedSelector::SelfWhenMonstrous,
                "Card.Self+IsRenowned" => AffectedSelector::SelfWhenRenowned,
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
                "Card.IsCommander+YouCtrl" | "Card.YouCtrl+IsCommander" => AffectedSelector::CommanderYouControl,
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
                "Card.YouCtrl+wasCastFromExile" => AffectedSelector::CardsYouControlCastFromExile,
                "Card.Enchantment+YouCtrl" | "Enchantment.YouCtrl" => AffectedSelector::EnchantmentsYouControl,
                "Card.Historic+YouCtrl" => AffectedSelector::HistoricYouControl,
                "Card.Historic+YouOwn" => AffectedSelector::HistoricYouOwn,
                "Card.IsCommander+YouOwn" => AffectedSelector::CommanderYouOwn,
                "Artifact.!token+YouCtrl" => AffectedSelector::NonTokenArtifactsYouControl,
                "Card.Artifact+nonLegendary+YouCtrl" => AffectedSelector::NonLegendaryArtifactsYouControl,
                // Treasure selectors
                "Card.Treasure+YouCtrl" => AffectedSelector::TreasuresYouControl,
                // Self on top of library
                "Card.Self+TopLibrary" => AffectedSelector::SelfTopLibrary,
                // Power-threshold creatures (controller-agnostic), e.g.
                // Meekstone's `Creature.powerGE3` doesn't-untap lock.
                _ if value.starts_with("Creature.powerGE") => {
                    return value
                        .trim_start_matches("Creature.powerGE")
                        .parse::<i32>()
                        .ok()
                        .map(AffectedSelector::CreaturesWithPowerGE);
                }
                _ => {
                    // Try to parse tribal type patterns
                    return parse_tribal_selector(value);
                }
            };
            Some(selector)
        }

        let mut abilities = Vec::new();

        // ---------------------------------------------------------------
        // Set/cast-hoser statics + the `Mode$ Always` state-trigger sweep
        // (City in a Bottle, mtg-3hwz3). Parsed with proper tokenization
        // (split on `|` then `$`), never substring matching.
        // ---------------------------------------------------------------
        for ability in &self.raw_abilities {
            // Only S: (static) and T: (the Always state-trigger) lines.
            let body = if let Some(rest) = ability.strip_prefix("S:") {
                rest
            } else if let Some(rest) = ability.strip_prefix("T:") {
                rest
            } else {
                continue;
            };
            let params = tokenize_pipe_dollar(body);
            let Some(mode) = params.get("Mode").map(String::as_str) else {
                continue;
            };
            match mode {
                "CantBeCast" => {
                    let valid_card = params
                        .get("ValidCard")
                        .map(|v| crate::core::TargetRestriction::parse(v))
                        .unwrap_or_else(crate::core::TargetRestriction::any);
                    let description = params.get("Description").cloned().unwrap_or_default();
                    abilities.push(StaticAbility::CantBeCast {
                        valid_card,
                        description,
                    });
                }
                "CantPlayLand" => {
                    let valid_card = params
                        .get("ValidCard")
                        .map(|v| crate::core::TargetRestriction::parse(v))
                        .unwrap_or_else(crate::core::TargetRestriction::any);
                    let description = params.get("Description").cloned().unwrap_or_default();
                    abilities.push(StaticAbility::CantPlayLand {
                        valid_card,
                        description,
                    });
                }
                "CantBlockBy" => {
                    // Blocker-side block restriction (Ironclaw Orcs):
                    //   Mode$ CantBlockBy | ValidAttacker$ <filter> | ValidBlocker$ Creature.Self
                    // Only the `ValidBlocker$ Creature.Self` shape (the source
                    // creature itself is the restricted BLOCKER) is modelled.
                    // The evasion shape (no ValidBlocker$, or a ValidBlocker$
                    // that is not Creature.Self — meaning "this ATTACKER can't be
                    // blocked [except by X]") is a different mechanic and is left
                    // unparsed, exactly as before.
                    let is_self_blocker = params
                        .get("ValidBlocker")
                        .map(|v| v.trim() == "Creature.Self")
                        .unwrap_or(false);
                    if is_self_blocker {
                        if let Some(filter_str) = params.get("ValidAttacker") {
                            // Faithfulness guard: TargetRestriction::parse only
                            // honours type/color/power/token modifiers. Keyword
                            // filters like `withoutFlying` silently degrade to a
                            // bare `Creature` (matching ALL creatures), which
                            // would wrongly forbid blocking everything. Only emit
                            // the static when every dotted modifier is one we
                            // evaluate faithfully; otherwise leave it unparsed
                            // (no worse than today's silent-drop).
                            if Self::cant_block_filter_is_faithful(filter_str) {
                                let attacker_filter = crate::core::TargetRestriction::parse(filter_str);
                                let description = params.get("Description").cloned().unwrap_or_default();
                                abilities.push(StaticAbility::CantBlockMatching {
                                    attacker_filter,
                                    description,
                                });
                            }
                        }
                    }
                }
                "Always" => {
                    // State-trigger sweep. We model only the SacrificeAll/
                    // DestroyAll form keyed off an IsPresent$ filter (City in a
                    // Bottle). The filter comes from the Execute$ SVar's
                    // ValidCards$ (authoritative for WHAT to sacrifice); fall
                    // back to IsPresent$ if the SVar can't be resolved.
                    if let Some(restriction) = self.always_sweep_restriction(&params) {
                        let description = params.get("TriggerDescription").cloned().unwrap_or_default();
                        abilities.push(StaticAbility::SacrificeMatchingPresent {
                            restriction,
                            description,
                        });
                    }
                }
                _ => {}
            }
        }

        for ability in &self.raw_abilities {
            if !ability.starts_with("S:") {
                continue;
            }

            // Determine which mode this is
            let is_continuous = ability.contains("Mode$ Continuous");
            let is_reduce_cost = ability.contains("Mode$ ReduceCost");
            let is_raise_cost = ability.contains("Mode$ RaiseCost");
            let is_cast_with_flash = ability.contains("Mode$ CastWithFlash");

            // Parse S:Mode$ Continuous, S:Mode$ ReduceCost, S:Mode$ RaiseCost, or S:Mode$ CastWithFlash lines
            if !is_continuous && !is_reduce_cost && !is_raise_cost && !is_cast_with_flash {
                continue;
            }

            // Parse parameters by splitting on |
            let mut affected = AffectedSelector::Self_;
            let mut power = 0;
            let mut toughness = 0;
            let mut keyword: Option<Keyword> = None;
            let mut description = String::new();
            let mut condition: Option<crate::core::StaticCondition> = None;

            // ReduceCost-specific parameters
            let mut valid_card: Option<crate::core::CostReductionTarget> = None;
            let mut reduce_amount: u8 = 0;
            let mut is_present: Option<String> = None;
            let mut present_zone: Option<crate::zones::Zone> = None;
            let mut present_compare_min: u8 = 1; // Default: at least 1

            // RaiseCost-specific parameters
            let mut raised_cost: Option<crate::core::RaisedCost> = None;

            // GrantAbility-specific parameters
            let mut add_ability_svar: Option<String> = None;

            // CastWithFlash-specific parameters
            let mut flash_valid_card: Option<crate::core::TargetRestriction> = None;

            // GainControl-specific parameter: `GainControl$ You` on a control-stealing
            // Aura (Control Magic, Mind Control, ...). Models CR 613.2 layer-2 control.
            let mut gain_control = false;

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
                        "Condition" => {
                            // Parse condition for when this ability is active
                            // Example: "Condition$ PlayerTurn" = only during controller's turn
                            match value {
                                "PlayerTurn" => {
                                    condition = Some(crate::core::StaticCondition::PlayerTurn);
                                }
                                "NotPlayerTurn" => {
                                    condition = Some(crate::core::StaticCondition::NotPlayerTurn);
                                }
                                _ => {
                                    // Other conditions not supported yet (e.g., CounteredSpellWithCMCGE5)
                                }
                            }
                        }
                        // ReduceCost-specific parameters
                        "ValidCard" => {
                            if is_cast_with_flash {
                                flash_valid_card = Some(crate::core::TargetRestriction::parse(value));
                            } else {
                                // Parse ValidCard$ for ReduceCost
                                // Examples: "Card.nonCreature", "Card.Self", "Creature", "Dragon"
                                use crate::core::CostReductionTarget;
                                valid_card = Some(match value {
                                    "Card.nonCreature" => CostReductionTarget::NonCreature,
                                    "Card" | "Card.Self" => CostReductionTarget::AllSpells,
                                    "Creature" => CostReductionTarget::Creature,
                                    "Card.White" => CostReductionTarget::Color(crate::core::Color::White),
                                    "Card.Blue" => CostReductionTarget::Color(crate::core::Color::Blue),
                                    "Card.Black" => CostReductionTarget::Color(crate::core::Color::Black),
                                    "Card.Red" => CostReductionTarget::Color(crate::core::Color::Red),
                                    "Card.Green" => CostReductionTarget::Color(crate::core::Color::Green),
                                    _ => {
                                        // Try to parse as a subtype (e.g., "Dragon", "Spirit")
                                        CostReductionTarget::Subtype(Subtype::new(value))
                                    }
                                });
                            }
                        }
                        "Amount" => {
                            // Parse Amount$ for cost reduction/increase (e.g., "1", "2")
                            reduce_amount = value.parse().unwrap_or(0);
                            // For RaiseCost with Amount$, create a mana-based raised cost
                            if is_raise_cost && reduce_amount > 0 {
                                raised_cost = Some(crate::core::RaisedCost::Mana(reduce_amount));
                            }
                        }
                        "Cost" => {
                            // Parse Cost$ for RaiseCost sacrifice costs
                            // Format: Sac<amount/type/description> or Sac<X/type/description>
                            // Examples: "Sac<1/Land>", "Sac<X/Land/land(s)>"
                            if is_raise_cost && value.starts_with("Sac<") && value.ends_with('>') {
                                let inner = &value[4..value.len() - 1]; // Strip "Sac<" and ">"
                                let parts: Vec<&str> = inner.split('/').collect();
                                if parts.len() >= 2 {
                                    let amount_str = parts[0].trim();
                                    let valid_type = parts[1].trim().to_string();

                                    use crate::core::RaisedCostAmount;
                                    let amount = if amount_str == "X" {
                                        RaisedCostAmount::Variable("X".to_string())
                                    } else {
                                        RaisedCostAmount::Fixed(amount_str.parse().unwrap_or(1))
                                    };

                                    raised_cost = Some(crate::core::RaisedCost::Sacrifice { amount, valid_type });
                                }
                            }
                        }
                        "IsPresent" => {
                            // Parse IsPresent$ for conditional cost reduction
                            // Example: "Lesson.YouOwn"
                            is_present = Some(value.to_string());
                        }
                        "PresentZone" => {
                            // Parse PresentZone$ for where to check IsPresent
                            // Examples: "Graveyard", "Battlefield", "Hand"
                            use crate::zones::Zone;
                            present_zone = match value {
                                "Graveyard" => Some(Zone::Graveyard),
                                "Battlefield" => Some(Zone::Battlefield),
                                "Hand" => Some(Zone::Hand),
                                "Library" => Some(Zone::Library),
                                "Exile" => Some(Zone::Exile),
                                _ => None,
                            };
                        }
                        "PresentCompare" => {
                            // Parse PresentCompare$ to get minimum count
                            // Examples: "GE3" (>= 3), "GE1" (>= 1)
                            if let Some(num_str) = value.strip_prefix("GE") {
                                present_compare_min = num_str.parse().unwrap_or(1);
                            } else if let Some(num_str) = value.strip_prefix("GT") {
                                // GT = greater than, so add 1 to make it GE
                                present_compare_min = num_str.parse::<u8>().unwrap_or(0).saturating_add(1);
                            }
                            // LE and EQ are less common for cost reduction conditions
                        }
                        "AddAbility" => {
                            // Store the SVar name to parse later
                            // Example: AddAbility$ AnyMana
                            // Can also have multiple abilities separated by &
                            // For now, just take the first one
                            let svar_name = value.split('&').next().unwrap_or(value).trim();
                            add_ability_svar = Some(svar_name.to_string());
                        }
                        "GainControl" => {
                            // `GainControl$ You` — the source's controller gains control
                            // of the affected permanent (Control Magic, Mind Control, ...).
                            // We only model the "You" form (control to the aura's
                            // controller); other defined players are not yet supported.
                            if value.eq_ignore_ascii_case("You") {
                                gain_control = true;
                            }
                        }
                        _ => {} // Ignore other parameters (e.g., AddType$, Type$, Activator$)
                    }
                }
            }

            // Build a presence-based StaticCondition if IsPresent$ was specified
            // on a continuous static (e.g. Sedge Troll: IsPresent$ Swamp.YouCtrl).
            // Reuses the same is_present/present_zone/present_compare_min fields
            // already parsed for ReduceCost conditions.
            let present_condition = is_present
                .as_ref()
                .map(|filter| crate::core::StaticCondition::ControlsPresent {
                    filter: filter.clone(),
                    zone: present_zone.unwrap_or(crate::zones::Zone::Battlefield),
                    min_count: present_compare_min,
                });

            // Prefer an explicit Condition$ (PlayerTurn/NotPlayerTurn) if present,
            // otherwise fall back to the presence-based condition.
            let static_condition = condition.clone().or(present_condition);

            // Create the ability based on what was parsed
            if power != 0 || toughness != 0 {
                // P/T modification ability
                abilities.push(StaticAbility::ModifyPT {
                    affected: affected.clone(),
                    power,
                    toughness,
                    description: description.clone(),
                    condition: static_condition.clone(),
                });
            }

            if let Some(kw) = keyword {
                // Keyword grant ability
                abilities.push(StaticAbility::GrantKeyword {
                    affected: affected.clone(),
                    keyword: kw,
                    description: description.clone(),
                    condition: static_condition.clone(),
                });
            }

            // ReduceCost ability
            if is_reduce_cost {
                if let Some(ref target) = valid_card {
                    // Build condition if presence check was specified
                    let reduce_condition =
                        is_present
                            .as_ref()
                            .map(|present_filter| crate::core::CostReductionCondition {
                                is_present: present_filter.clone(),
                                present_zone: present_zone.unwrap_or(crate::zones::Zone::Battlefield),
                                min_count: present_compare_min,
                            });

                    abilities.push(StaticAbility::ReduceCost {
                        valid_card: target.clone(),
                        amount: reduce_amount,
                        condition: reduce_condition,
                        description: description.clone(),
                    });
                }
            }

            // RaiseCost ability
            if is_raise_cost {
                if let Some(cost) = raised_cost {
                    // Use valid_card if specified, otherwise default to AllSpells
                    // Clone valid_card since it may have been used in ReduceCost branch
                    let target = valid_card
                        .clone()
                        .unwrap_or(crate::core::CostReductionTarget::AllSpells);

                    abilities.push(StaticAbility::RaiseCost {
                        valid_card: target,
                        raised_cost: cost,
                        description: description.clone(),
                    });
                }
            }

            // GrantAbility - parse AddAbility$ SVar into an ActivatedAbility
            if let Some(ref svar_name) = add_ability_svar {
                if let Some(svar_body) = self.svars.get(svar_name) {
                    // Parse the SVar as an activated ability
                    // SVars look like: "AB$ Mana | Cost$ T | Produced$ Any | Amount$ 3 | SpellDescription$ ..."
                    // We need to prefix with "A:" to make it parseable
                    let ability_str = format!("A:{}", svar_body);
                    if let Some(parsed_ability) = self.parse_svar_as_activated_ability(&ability_str) {
                        abilities.push(StaticAbility::GrantAbility {
                            affected: affected.clone(),
                            ability: parsed_ability,
                            description: description.clone(),
                        });
                    }
                }
            }

            // GainControl static (control-stealing Auras: Control Magic, Mind Control).
            if gain_control {
                abilities.push(StaticAbility::GainControl {
                    affected: affected.clone(),
                    description: description.clone(),
                });
            }

            // CastWithFlash ability
            if is_cast_with_flash {
                if let Some(target) = flash_valid_card {
                    abilities.push(StaticAbility::CastWithFlash {
                        valid_card: target,
                        description: description.clone(),
                    });
                }
            }
        }

        // Replacement effects (R: lines) that act as continuous "doesn't untap"
        // locks. Paralyze (and other Auras / permanents) print
        //   R:Event$ Untap | Layer$ CantHappen | ValidCard$ Creature.EnchantedBy
        //     | ValidStepTurnToController$ You | ...
        // i.e. "Enchanted creature doesn't untap during its controller's untap
        // step." We model this as a continuous GrantKeyword(DoesNotUntap) on the
        // affected permanent: the host carries the static, the affected creature
        // receives the keyword, and the untap step skips any permanent that has
        // it. This generalizes to every `Event$ Untap | Layer$ CantHappen`
        // replacement, not just Paralyze.
        for ability in &self.raw_abilities {
            let Some(body) = ability.strip_prefix("R:") else {
                continue;
            };
            // Tokenized parse: split on `|`, then `$` (no substring matching).
            let mut params: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
            for param in body.split('|') {
                if let Some((k, v)) = param.split_once('$') {
                    params.insert(k.trim(), v.trim());
                }
            }
            if params.get("Event") != Some(&"Untap") || params.get("Layer") != Some(&"CantHappen") {
                continue;
            }
            // Which permanents are locked? Default to the enchanted creature
            // (the overwhelmingly common case); honor an explicit ValidCard$.
            let affected = params
                .get("ValidCard")
                .and_then(|v| parse_single_affected_selector(v))
                .unwrap_or(AffectedSelector::CreatureEnchantedBy);
            let description = params
                .get("Description")
                .map(|s| s.to_string())
                .unwrap_or_else(|| "Doesn't untap during its controller's untap step.".to_string());
            abilities.push(StaticAbility::GrantKeyword {
                affected,
                keyword: Keyword::DoesNotUntap,
                description,
                condition: None,
            });
        }

        abilities
    }

    /// Detect a self-referential relative per-target cost (Fireball, CR 601.2f):
    /// `S:Mode$ RaiseCost | ValidCard$ Card.Self | ... | Relative$ True`.
    ///
    /// Parsed structurally (tokenize on `|` then `$`), never via substring
    /// matching on the whole line, per the "No Hacky String Operations" rule.
    /// The cast path uses the resulting cache flag to add `(num_targets - 1)`
    /// generic mana once targets are chosen.
    pub(crate) fn has_relative_self_target_cost(&self) -> bool {
        for ability in &self.raw_abilities {
            // Static lines are stored as "S:<body>" in raw_abilities.
            let Some(body) = ability.strip_prefix("S:") else {
                continue;
            };
            let mut is_raise_cost = false;
            let mut is_self = false;
            let mut is_relative = false;
            for token in body.split('|') {
                let token = token.trim();
                let Some((key, value)) = token.split_once('$') else {
                    continue;
                };
                match (key.trim(), value.trim()) {
                    ("Mode", "RaiseCost") => is_raise_cost = true,
                    ("ValidCard", "Card.Self") => is_self = true,
                    ("Relative", "True") => is_relative = true,
                    _ => {}
                }
            }
            if is_raise_cost && is_self && is_relative {
                return true;
            }
        }
        false
    }

    /// Detect Stasis's "Players skip their untap steps" lock, expressed as the
    /// replacement `R:Event$ BeginPhase | Phase$ Untap | Skip$ True`. Parsed
    /// structurally (tokenize on `|` then `$`), never via substring matching.
    /// While such a permanent is on the battlefield the untap step is skipped
    /// for every player (CR 502 / 614 "skip" replacement on the untap step).
    pub(crate) fn skips_untap_step(&self) -> bool {
        for ability in &self.raw_abilities {
            // Replacement lines are stored as "R:<body>" in raw_abilities.
            let Some(body) = ability.strip_prefix("R:") else {
                continue;
            };
            let mut is_begin_phase = false;
            let mut is_untap_phase = false;
            let mut is_skip = false;
            for token in body.split('|') {
                let token = token.trim();
                let Some((key, value)) = token.split_once('$') else {
                    continue;
                };
                match (key.trim(), value.trim()) {
                    ("Event", "BeginPhase") => is_begin_phase = true,
                    ("Phase", "Untap") => is_untap_phase = true,
                    ("Skip", "True") => is_skip = true,
                    _ => {}
                }
            }
            if is_begin_phase && is_untap_phase && is_skip {
                return true;
            }
        }
        false
    }

    /// Detect Winter Orb's "players can't untap more than one land during their
    /// untap steps" lock, expressed as the static ability
    /// `S:Mode$ Continuous | Affected$ Player | AddKeyword$ UntapAdjust:Land:N |
    /// IsPresent$ Card.Self+untapped`. Parsed structurally (tokenize on `|` then
    /// `$`, then `:` inside the `UntapAdjust:<type>:<n>` value), never via
    /// substring matching.
    ///
    /// Returns `Some(n)` where `n` is the per-untap-step land allowance (1 for
    /// Winter Orb). The `IsPresent$ Card.Self+untapped` self-condition is NOT
    /// baked in here; it is re-evaluated at the untap step against current board
    /// state so the lock toggles correctly when Winter Orb is tapped (and stays
    /// rewind-safe — see `CardCache::limits_land_untap`).
    pub(crate) fn limits_land_untap(&self) -> Option<u8> {
        for ability in &self.raw_abilities {
            // Static-ability lines are stored as "S:<body>" in raw_abilities.
            let Some(body) = ability.strip_prefix("S:") else {
                continue;
            };
            let mut is_continuous = false;
            let mut affects_player = false;
            let mut land_allowance: Option<u8> = None;
            for token in body.split('|') {
                let token = token.trim();
                let Some((key, value)) = token.split_once('$') else {
                    continue;
                };
                match (key.trim(), value.trim()) {
                    ("Mode", "Continuous") => is_continuous = true,
                    ("Affected", "Player") => affects_player = true,
                    ("AddKeyword", v) => {
                        // UntapAdjust:<restriction>:<count>, e.g. "UntapAdjust:Land:1".
                        let mut parts = v.split(':');
                        if parts.next() == Some("UntapAdjust") && parts.next() == Some("Land") {
                            if let Some(n) = parts.next().and_then(|n| n.parse::<u8>().ok()) {
                                land_allowance = Some(n);
                            }
                        }
                    }
                    _ => {}
                }
            }
            if is_continuous && affects_player {
                if let Some(n) = land_allowance {
                    return Some(n);
                }
            }
        }
        None
    }

    /// Parse an SVar body as an activated ability
    ///
    /// Used by AddAbility$ to convert an SVar like:
    /// `AB$ Mana | Cost$ T | Produced$ Any | Amount$ 3 | SpellDescription$ Add three mana.`
    /// into an ActivatedAbility that can be granted to permanents.
    fn parse_svar_as_activated_ability(&self, ability_str: &str) -> Option<crate::core::ActivatedAbility> {
        use super::ability_parser::{AbilityParams, AbilityRecordType, ApiType};
        use super::effect_converter::params_to_effect_with_svars;
        use crate::core::{ActivatedAbility, Cost};

        let params = match AbilityParams::parse(ability_str) {
            Ok(p) if p.record_type == AbilityRecordType::Ability => p,
            _ => return None,
        };

        // Extract cost from Cost$ parameter
        let cost = params.get("Cost").and_then(Cost::parse)?;

        // Check if this is a mana ability
        let is_mana_ability = matches!(params.api_type, ApiType::Mana);

        // Parse effects
        let effects = params_to_effect_with_svars(&params, &self.svars)
            .map(|e| vec![e])
            .unwrap_or_default();

        if effects.is_empty() {
            return None;
        }

        // Extract description
        let description = params.get("SpellDescription").unwrap_or("Granted ability").to_string();

        Some(ActivatedAbility::new(cost, effects, description, is_mana_ability))
    }

    /// Parse an ongoing trigger from a raw SVar body string, using this
    /// card's own SVar context for `Execute$` resolution.
    ///
    /// Used by `apply_class_level_ongoing_abilities` when a Class level-up
    /// grants an ongoing trigger (e.g. a SpellCast trigger at level 3).
    ///
    /// Returns `None` if the body cannot be parsed as a supported trigger mode.
    pub fn parse_ongoing_trigger_from_svar_body(&self, body: &str) -> Option<crate::core::Trigger> {
        let params = tokenize_pipe_dollar(body);
        let mode = params.get("Mode").map(|s| s.as_str())?;

        // Determine trigger event from Mode
        let event = match mode {
            "SpellCast" => crate::core::TriggerEvent::SpellCast,
            "ChangesZone" => {
                // Only EntersBattlefield self-trigger supported here for now
                let dest = params.get("Destination").map(|s| s.as_str());
                if dest == Some("Battlefield") {
                    crate::core::TriggerEvent::EntersBattlefield
                } else {
                    return None;
                }
            }
            // ClassLevelGained is one-time — loaded statically, not as ongoing
            _ => return None,
        };

        // Parse effects via Execute$ SVar resolution
        let mut effects = Vec::new();
        if let Some(exec_ref) = params.get("Execute") {
            if let Some(svar_params) = self.parsed_svars.get(exec_ref.as_str()) {
                effects.extend(self.extract_effects_from_svar(svar_params));
            }
        }

        let description = params
            .get("TriggerDescription")
            .cloned()
            .unwrap_or_else(|| format!("Ongoing {} trigger", mode));

        // Build trigger — ongoing triggers are NOT self-only (they watch other spells/events)
        let mut trigger = crate::core::Trigger::new_any(event, effects, description);

        // Apply mode-specific filter flags
        if mode == "SpellCast" {
            let valid_card = params.get("ValidCard").map(|s| s.as_str());
            match valid_card {
                Some("Card.nonCreature") => trigger.requires_noncreature = true,
                // "Instant,Sorcery" — triggers specifically on instant or sorcery spells
                Some(vc)
                    if vc.split(',').any(|t| t.trim() == "Instant") && vc.split(',').any(|t| t.trim() == "Sorcery") =>
                {
                    trigger.requires_instant_or_sorcery = true;
                }
                _ => {}
            }
            // ValidActivatingPlayer$ You is already handled at runtime:
            // check_triggers filters on card.controller == caster_id (line ~7942).
        }

        Some(trigger)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression test for the content-addressed WASM export pipeline (mtg-571):
    /// a `CardDefinition` with multiple SVars must serialize to identical bytes
    /// every time, regardless of `HashMap` iteration order. Before the
    /// `serialize_svars_sorted` fix, the `svars` `HashMap` serialized in random
    /// order, so the per-set `.bin` content hash (and its filename) changed
    /// run-to-run, defeating content-addressing and racing under `make validate`.
    #[test]
    fn test_svars_serialize_deterministically() {
        let mut def = CardDefinition::default();
        // Insert many SVars in an arbitrary order; HashMap will store them in
        // some (unspecified) bucket order that can differ between processes.
        for (k, v) in [
            ("Zebra", "DB$ Draw | NumCards$ 1"),
            ("Apple", "DB$ Token | TokenScript$ c_a_food_sac"),
            ("Mango", "DB$ DealDamage | NumDmg$ 3"),
            ("Delta", "DB$ Pump | NumAtt$ 2"),
            ("Echo", "DB$ Mill | NumCards$ 5"),
            ("Bravo", "DB$ GainLife | LifeAmount$ 4"),
        ] {
            def.svars.insert(k.to_string(), v.to_string());
        }

        let first = bincode::serialize(&def).expect("serialize");

        // Rebuild from scratch with a DIFFERENT insertion order — the HashMap
        // layout may differ, but the serialized bytes must not.
        let mut def2 = CardDefinition::default();
        for (k, v) in [
            ("Mango", "DB$ DealDamage | NumDmg$ 3"),
            ("Bravo", "DB$ GainLife | LifeAmount$ 4"),
            ("Zebra", "DB$ Draw | NumCards$ 1"),
            ("Echo", "DB$ Mill | NumCards$ 5"),
            ("Apple", "DB$ Token | TokenScript$ c_a_food_sac"),
            ("Delta", "DB$ Pump | NumAtt$ 2"),
        ] {
            def2.svars.insert(k.to_string(), v.to_string());
        }
        let second = bincode::serialize(&def2).expect("serialize");

        assert_eq!(
            first, second,
            "CardDefinition svars must serialize deterministically regardless of HashMap order"
        );

        // And it must still round-trip through the HashMap deserializer the
        // WASM loader uses.
        let restored: CardDefinition = bincode::deserialize(&first).expect("deserialize");
        assert_eq!(restored.svars, def.svars);
    }

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
    fn test_parse_waterbend_effect_ability() {
        // Test parsing of Giant Koi's Waterbend ability with StaticAbilities$ SVar
        // This requires params_to_effect_with_svars to resolve the SVar reference
        let content = r#"
Name:Giant Koi
ManaCost:4 U U
Types:Creature Fish
PT:5/7
K:TypeCycling:Island:2
A:AB$ Effect | Cost$ Waterbend<3> | Defined$ Self | StaticAbilities$ Unblockable | AILogic$ Pump | SpellDescription$ This creature can't be blocked this turn.
SVar:Unblockable:Mode$ CantBlockBy | ValidAttacker$ Card.EffectSource | Description$ This creature can't be blocked this turn.
Oracle:Waterbend {3}: This creature can't be blocked this turn.
"#;

        let def = CardLoader::parse(content).unwrap();
        assert_eq!(def.name.as_str(), "Giant Koi");
        assert_eq!(def.mana_cost.generic, 4);
        assert_eq!(def.mana_cost.blue, 2);
        assert!(def.types.contains(&CardType::Creature));

        // Check that the activated ability is parsed with SVar resolution
        let abilities = def.parse_activated_abilities();
        assert_eq!(
            abilities.len(),
            1,
            "Giant Koi should have 1 activated ability (the Waterbend effect)"
        );

        let ability = &abilities[0];
        // Check the Waterbend cost is parsed
        use crate::core::costs::Cost;
        assert!(
            matches!(ability.cost, Cost::Waterbend { amount: 3 }),
            "Should have Waterbend<3> cost, got {:?}",
            ability.cost
        );

        // The effects list should NOT be empty - this was the bug we're fixing
        assert_eq!(ability.effects.len(), 1, "Should have 1 effect (GrantCantBeBlocked)");

        // Verify the effect is GrantCantBeBlocked
        use crate::core::Effect;
        assert!(
            matches!(ability.effects[0], Effect::GrantCantBeBlocked { .. }),
            "Expected GrantCantBeBlocked effect, got {:?}",
            ability.effects[0]
        );
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

    /// Parser-shape regression (1994 World Championship compat — mtg-713 B9):
    /// Whirling Dervish's end-step counter trigger uses the SPACED phase string
    /// "End of Turn", a `DB$ PutCounter | Defined$ Self` effect on a phase
    /// trigger, and a `dealtDamageToOppThisTurn` intervening-if — all three of
    /// which the loader previously dropped (so the trigger either vanished, had
    /// no effect, or fired unconditionally). Assert the trigger now parses with
    /// the right event, the +1/+1 self-PutCounter effect, and the intervening-if
    /// flag set.
    #[test]
    fn test_parse_whirling_dervish_end_step_counter_trigger() {
        use crate::core::{CounterType, Effect, TriggerEvent};

        let content = r#"
Name:Whirling Dervish
ManaCost:G G
Types:Creature Human Monk
PT:1/1
K:Protection from black
T:Mode$ Phase | Phase$ End of Turn | TriggerZones$ Battlefield | Execute$ TrigPutCounter | IsPresent$ Card.Self+dealtDamageToOppThisTurn | TriggerDescription$ At the beginning of each end step, if CARDNAME dealt damage to an opponent this turn, put a +1/+1 counter on it.
SVar:TrigPutCounter:DB$ PutCounter | Defined$ Self | CounterType$ P1P1 | CounterNum$ 1
Oracle:Protection from black\nAt the beginning of each end step, if Whirling Dervish dealt damage to an opponent this turn, put a +1/+1 counter on it.
"#;

        let def = CardLoader::parse(content).unwrap();
        let triggers = def.parse_triggers();

        let trigger = triggers
            .iter()
            .find(|t| t.event == TriggerEvent::BeginningOfEndStep)
            .expect("spaced 'End of Turn' phase string must parse to a BeginningOfEndStep trigger");

        assert!(
            trigger.present_self_dealt_damage_to_opponent,
            "dealtDamageToOppThisTurn intervening-if must be parsed onto the trigger"
        );

        let put_counter_effect = trigger
            .effects
            .iter()
            .find(|e| matches!(e, Effect::PutCounter { .. }))
            .expect("trigger must carry a DB$ PutCounter | Defined$ Self effect");
        let Effect::PutCounter {
            counter_type, amount, ..
        } = put_counter_effect
        else {
            unreachable!("matched PutCounter above");
        };
        assert_eq!(
            (*counter_type, *amount),
            (CounterType::P1P1, 1),
            "must put exactly one +1/+1 counter"
        );
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

    #[test]
    fn test_extract_token_scripts_canyon_crawler() {
        // Canyon Crawler creates a Food token via trigger
        let content = r#"
Name:Canyon Crawler
ManaCost:4 B B
Types:Creature Spider Beast
PT:6/6
K:Deathtouch
T:Mode$ ChangesZone | Origin$ Any | Destination$ Battlefield | ValidCard$ Card.Self | Execute$ TrigFood | TriggerDescription$ When this creature enters, create a Food token. (It's an artifact with "{2}, {T}, Sacrifice this token:You gain 3 life.")
SVar:TrigFood:DB$ Token | TokenScript$ c_a_food_sac | TokenOwner$ You
K:TypeCycling:Swamp:2
Oracle:Deathtouch\nWhen this creature enters, create a Food token.
"#;

        let def = CardLoader::parse(content).unwrap();
        let token_scripts = def.extract_token_scripts();

        assert!(
            token_scripts.contains(&"c_a_food_sac".to_string()),
            "Should extract c_a_food_sac token script. Got: {:?}. raw_abilities: {:?}",
            token_scripts,
            def.raw_abilities
        );
    }

    #[test]
    fn test_parse_fire_lord_ozai_attack_trigger() {
        use crate::core::{Cost, Effect, TriggerEvent};

        // Test Fire Lord Ozai's attack trigger with AB$ Mana and Sacrificed$CardPower
        // "Whenever Fire Lord Ozai attacks, you may sacrifice another creature.
        //  If you do, add an amount of {R} equal to the sacrificed creature's power."
        let content = r#"
Name:Fire Lord Ozai
ManaCost:3 B
Types:Legendary Creature Human Noble
PT:4/4
T:Mode$ Attacks | ValidCard$ Card.Self | Execute$ TrigMana | OptionalDecider$ You | TriggerDescription$ Whenever CARDNAME attacks, you may sacrifice another creature. If you do, add an amount of {R} equal to the sacrificed creature's power. Until end of combat, you don't lose this mana as steps end.
SVar:TrigMana:AB$ Mana | Cost$ Sac<1/Creature.Other/another creature> | Produced$ R | Amount$ X | CombatMana$ True
SVar:X:Sacrificed$CardPower
Oracle:Whenever Fire Lord Ozai attacks, you may sacrifice another creature. If you do, add an amount of {R} equal to the sacrificed creature's power.
"#;

        let def = CardLoader::parse(content).unwrap();
        let triggers = def.parse_triggers();

        // Verify the attack trigger was parsed
        assert_eq!(triggers.len(), 1, "Should have one trigger");

        let trigger = &triggers[0];
        assert_eq!(trigger.event, TriggerEvent::Attacks, "Should be Attacks trigger");

        // Verify it's optional (OptionalDecider$ You)
        assert!(trigger.optional, "Trigger should be optional due to 'you may'");

        // Verify it has a sacrifice cost (Creature.Other)
        assert!(trigger.cost.is_some(), "Trigger should have a sacrifice cost");
        if let Some(ref cost) = trigger.cost {
            assert!(cost.requires_sacrifice(), "Cost should require sacrifice");
            if let Cost::SacrificePattern { count, card_type } = cost {
                assert_eq!(*count, 1, "Should sacrifice 1 creature");
                assert!(
                    card_type.contains("Creature"),
                    "Card type should include Creature, got: {}",
                    card_type
                );
            }
        }

        // Verify it has a Firebend effect with amount=254 (sentinel for sacrificed creature's power)
        let has_firebend = trigger.effects.iter().any(|e| {
            if let Effect::Firebend { amount, .. } = e {
                // 254 is the sentinel for "use sacrificed creature's power"
                *amount == 254
            } else {
                false
            }
        });
        assert!(
            has_firebend,
            "Trigger should have Firebend effect with amount=254 (sacrificed creature's power). Effects: {:?}",
            trigger.effects
        );
    }

    #[test]
    fn test_prowess_keyword_expansion() {
        use crate::core::{Effect, TriggerEvent};

        // Test Prowess keyword expansion into SpellCast trigger
        // "Whenever you cast a noncreature spell, this creature gets +1/+1 until end of turn."
        let content = r#"
Name:Ty Lee, Artful Acrobat
ManaCost:2 R
Types:Legendary Creature Human Performer
PT:3/2
K:Prowess
Oracle:Prowess (Whenever you cast a noncreature spell, this creature gets +1/+1 until end of turn.)
"#;

        let def = CardLoader::parse(content).unwrap();
        // Prowess keyword expansion happens in instantiate(), not parse_triggers()
        // because it expands the keyword into a trigger after keywords are parsed
        let card = def.instantiate(CardId::new(0), crate::core::PlayerId::new(0));

        // Find the Prowess trigger (SpellCast event with PumpCreature effect)
        let prowess_trigger = card.triggers.iter().find(|t| {
            t.event == TriggerEvent::SpellCast
                && t.description.contains("Prowess")
                && t.effects.iter().any(|e| {
                    matches!(
                        e,
                        Effect::PumpCreature {
                            power_bonus: 1,
                            toughness_bonus: 1,
                            ..
                        }
                    )
                })
        });

        assert!(
            prowess_trigger.is_some(),
            "Should have a Prowess SpellCast trigger with +1/+1 pump effect. Triggers: {:?}",
            card.triggers
        );

        let trigger = prowess_trigger.unwrap();

        // Verify it's marked as noncreature-only
        assert!(
            trigger.description.contains("noncreature"),
            "Prowess trigger should be marked for noncreature spells only"
        );

        // Verify the pump effect targets self (CardId 0 placeholder)
        let pump_effect = trigger
            .effects
            .iter()
            .find(|e| matches!(e, Effect::PumpCreature { .. }));
        assert!(pump_effect.is_some(), "Should have PumpCreature effect");
        if let Some(Effect::PumpCreature {
            target,
            power_bonus,
            toughness_bonus,
            ..
        }) = pump_effect
        {
            assert_eq!(target.as_u32(), 0, "Target should be placeholder 0 (self)");
            assert_eq!(*power_bonus, 1, "Power bonus should be +1");
            assert_eq!(*toughness_bonus, 1, "Toughness bonus should be +1");
        }
    }

    #[test]
    fn test_parse_sacrifice_trigger() {
        use crate::core::{Effect, TriggerEvent};

        // Test Pirate Peddlers style sacrifice trigger:
        // "Whenever you sacrifice another permanent, put a +1/+1 counter on this creature."
        let content = r#"
Name:Pirate Peddlers
ManaCost:2 B
Types:Creature Human Pirate
PT:2/2
K:Deathtouch
T:Mode$ Sacrificed | ValidCard$ Permanent.Other | Execute$ TrigPutCounter | TriggerZones$ Battlefield | ValidPlayer$ You | TriggerDescription$ Whenever you sacrifice another permanent, put a +1/+1 counter on this creature.
SVar:TrigPutCounter:DB$ PutCounter | Defined$ Self | CounterType$ P1P1 | CounterNum$ 1
Oracle:Deathtouch\nWhenever you sacrifice another permanent, put a +1/+1 counter on this creature.
"#;

        let def = CardLoader::parse(content).unwrap();
        let triggers = def.parse_triggers();

        // Verify the sacrifice trigger was parsed
        assert_eq!(triggers.len(), 1, "Should have one trigger");

        let trigger = &triggers[0];
        assert_eq!(trigger.event, TriggerEvent::Sacrificed);

        // Verify it has the [other] flag (triggers on OTHER permanents, not self)
        assert!(
            trigger.description.contains("[other]"),
            "Trigger should be marked [other] for other permanents only. Description: {}",
            trigger.description
        );

        // Verify it has a PutCounter effect
        let has_put_counter = trigger.effects.iter().any(|e| {
            matches!(
                e,
                Effect::PutCounter {
                    counter_type: crate::core::CounterType::P1P1,
                    amount: 1,
                    ..
                }
            )
        });
        assert!(
            has_put_counter,
            "Trigger should have PutCounter effect with P1P1 counter. Effects: {:?}",
            trigger.effects
        );
    }

    #[test]
    fn test_parse_damage_done_trigger() {
        use crate::core::TriggerEvent;

        // Test Hypnotic Specter style DamageDone trigger:
        // "Whenever Hypnotic Specter deals damage to an opponent, that player discards a card at random."
        let content = r#"
Name:Hypnotic Specter
ManaCost:1 B B
Types:Creature Specter
PT:2/2
K:Flying
T:Mode$ DamageDone | ValidSource$ Card.Self | ValidTarget$ Opponent | Execute$ TrigDiscard | TriggerZones$ Battlefield | TriggerDescription$ Whenever CARDNAME deals damage to an opponent, that player discards a card at random.
SVar:TrigDiscard:DB$ Discard | Defined$ TriggeredTarget | NumCards$ 1 | Mode$ Random
Oracle:Flying\nWhenever Hypnotic Specter deals damage to an opponent, that player discards a card at random.
"#;

        let def = CardLoader::parse(content).unwrap();
        let triggers = def.parse_triggers();

        // Verify the DamageDone trigger was parsed
        assert_eq!(triggers.len(), 1, "Should have one trigger");

        let trigger = &triggers[0];
        assert_eq!(
            trigger.event,
            TriggerEvent::DealsCombatDamage,
            "Trigger should be DealsCombatDamage event"
        );
        // ValidTarget$ Opponent -> player-class recipient filter: this trigger
        // fires only on combat damage dealt to a player, not to a creature.
        assert_eq!(
            trigger.combat_damage_target,
            crate::core::CombatDamageTarget::Player,
            "ValidTarget$ Opponent should set combat_damage_target = Player. Got: {:?}",
            trigger.combat_damage_target
        );
        // Self-only trigger (ValidSource$ Card.Self)
        assert!(
            trigger.trigger_self_only,
            "ValidSource$ Card.Self should make trigger self-only"
        );
    }

    #[test]
    fn test_parse_combat_damage_done_trigger() {
        use crate::core::TriggerEvent;

        // Test combat-damage-only DamageDone trigger (Markov Blademaster style):
        // "Whenever this creature deals combat damage to a player, put a +1/+1 counter on it."
        let content = r#"
Name:Markov Blademaster
ManaCost:1 R R
Types:Creature Vampire Warrior
PT:1/1
K:Double Strike
T:Mode$ DamageDone | ValidSource$ Card.Self | ValidTarget$ Player | CombatDamage$ True | Execute$ TrigPutCounter | TriggerZones$ Battlefield | TriggerDescription$ Whenever CARDNAME deals combat damage to a player, put a +1/+1 counter on it.
SVar:TrigPutCounter:DB$ PutCounter | CounterType$ P1P1 | CounterNum$ 1
Oracle:Double strike\nWhenever Markov Blademaster deals combat damage to a player, put a +1/+1 counter on it.
"#;

        let def = CardLoader::parse(content).unwrap();
        let triggers = def.parse_triggers();

        // Verify the DamageDone trigger was parsed
        assert_eq!(triggers.len(), 1, "Should have one trigger");

        let trigger = &triggers[0];
        assert_eq!(
            trigger.event,
            TriggerEvent::DealsCombatDamage,
            "Trigger should be DealsCombatDamage event"
        );
        // ValidTarget$ Player -> player-class recipient filter (combat-damage-only).
        assert_eq!(
            trigger.combat_damage_target,
            crate::core::CombatDamageTarget::Player,
            "ValidTarget$ Player should set combat_damage_target = Player. Got: {:?}",
            trigger.combat_damage_target
        );
        // Self-only trigger (ValidSource$ Card.Self)
        assert!(
            trigger.trigger_self_only,
            "ValidSource$ Card.Self should make trigger self-only"
        );
    }

    #[test]
    fn test_parse_optional_damage_done_trigger() {
        use crate::core::TriggerEvent;

        // Test optional DamageDone trigger (with OptionalDecider$ You):
        // "Whenever ... you may ..."
        let content = r#"
Name:Zombie Cannibal
ManaCost:B
Types:Creature Zombie
PT:1/1
T:Mode$ DamageDone | ValidSource$ Card.Self | ValidTarget$ Player | CombatDamage$ True | Execute$ TrigExile | OptionalDecider$ You | TriggerDescription$ Whenever CARDNAME deals combat damage to a player, you may exile target card from that player's graveyard.
SVar:TrigExile:DB$ ChangeZone | Origin$ Graveyard | Destination$ Exile
Oracle:Whenever Zombie Cannibal deals combat damage to a player, you may exile target card from that player's graveyard.
"#;

        let def = CardLoader::parse(content).unwrap();
        let triggers = def.parse_triggers();

        // Verify the DamageDone trigger was parsed
        assert_eq!(triggers.len(), 1, "Should have one trigger");

        let trigger = &triggers[0];
        assert_eq!(
            trigger.event,
            TriggerEvent::DealsCombatDamage,
            "Trigger should be DealsCombatDamage event"
        );
        // Should be optional (OptionalDecider$ You)
        assert!(trigger.optional, "Trigger with OptionalDecider$ You should be optional");
    }

    #[test]
    fn test_parse_equipment_etb_attach_trigger() {
        use crate::core::{Effect, TriggerEvent};

        // Test Twin Blades equipment ETB trigger with DB$ Attach:
        // "When this Equipment enters, attach it to target creature you control.
        //  That creature gains double strike until end of turn."
        let content = r#"
Name:Twin Blades
ManaCost:2 R
Types:Artifact Equipment
K:Flash
T:Mode$ ChangesZone | Origin$ Any | Destination$ Battlefield | ValidCard$ Card.Self | Execute$ TrigAttach | TriggerDescription$ When this Equipment enters, attach it to target creature you control. That creature gains double strike until end of turn.
SVar:TrigAttach:DB$ Attach | ValidTgts$ Creature.YouCtrl | TgtPrompt$ Select target creature you control | SubAbility$ DBPump
SVar:DBPump:DB$ Pump | Defined$ Targeted | KW$ Double Strike
S:Mode$ Continuous | Affected$ Creature.EquippedBy | AddPower$ 1 | AddToughness$ 1 | Description$ Equipped creature gets +1/+1.
K:Equip:2
Oracle:Flash\nWhen this Equipment enters, attach it to target creature you control. That creature gains double strike until end of turn.\nEquipped creature gets +1/+1.\nEquip {2}
"#;

        let def = CardLoader::parse(content).unwrap();
        let triggers = def.parse_triggers();

        // Verify we have at least one trigger (the ETB attach trigger)
        assert!(
            !triggers.is_empty(),
            "Should have at least one trigger. Got: {:?}",
            triggers
        );

        // Find the ETB trigger
        let etb_trigger = triggers.iter().find(|t| t.event == TriggerEvent::EntersBattlefield);

        assert!(
            etb_trigger.is_some(),
            "Should have an EntersBattlefield trigger. Triggers: {:?}",
            triggers
        );

        let trigger = etb_trigger.unwrap();

        // Verify it has an AttachEquipment effect
        let has_attach = trigger
            .effects
            .iter()
            .any(|e| matches!(e, Effect::AttachEquipment { .. }));
        assert!(
            has_attach,
            "Trigger should have AttachEquipment effect. Effects: {:?}",
            trigger.effects
        );
    }

    #[test]
    fn test_parse_cracked_earth_technique_subability_chain() {
        use crate::core::Effect;

        // Cracked Earth Technique: Earthbend 3, then earthbend 3, gain 3 life
        // SubAbility chain: SP$ Earthbend -> DBEarthbend -> DBGainLife
        let content = r#"
Name:Cracked Earth Technique
ManaCost:4 G
Types:Sorcery Lesson
A:SP$ Earthbend | Num$ 3 | SubAbility$ DBEarthbend | SpellDescription$ Earthbend 3, then earthbend 3. You gain 3 life.
SVar:DBEarthbend:DB$ Earthbend | Num$ 3 | SubAbility$ DBGainLife
SVar:DBGainLife:DB$ GainLife | LifeAmount$ 3
Oracle:Earthbend 3, then earthbend 3. You gain 3 life.
"#;

        let def = CardLoader::parse(content).unwrap();
        assert_eq!(def.name.as_str(), "Cracked Earth Technique");
        assert_eq!(def.mana_cost.generic, 4);
        assert_eq!(def.mana_cost.green, 1);
        assert!(def.types.contains(&CardType::Sorcery));

        // Parse effects - should have 3 effects from SubAbility chain:
        // 1. First Earthbend (from SP$ Earthbend | Num$ 3)
        // 2. Second Earthbend (from SubAbility$ DBEarthbend -> DB$ Earthbend | Num$ 3)
        // 3. GainLife (from SubAbility$ DBGainLife -> DB$ GainLife | LifeAmount$ 3)
        let effects = def.parse_effects();

        assert!(
            effects.len() >= 2,
            "Should have at least 2 effects from SubAbility chain. Got {} effects: {:?}",
            effects.len(),
            effects
        );

        // First effect should be Earthbend with 3 counters
        let earthbend_count = effects
            .iter()
            .filter(|e| matches!(e, Effect::Earthbend { num_counters: 3, .. }))
            .count();
        assert!(
            earthbend_count >= 1,
            "Should have at least 1 Earthbend effect with 3 counters. Effects: {:?}",
            effects
        );

        // Should have GainLife effect from the end of the chain
        let gainlife_count = effects
            .iter()
            .filter(|e| matches!(e, Effect::GainLife { amount: 3, .. }))
            .count();
        assert!(
            gainlife_count >= 1,
            "Should have GainLife 3 effect from SubAbility chain. Effects: {:?}",
            effects
        );
    }

    #[test]
    fn test_parse_glider_kids_scry_etb() {
        use crate::core::{Effect, TriggerEvent};

        // Test Glider Kids ETB scry trigger:
        // "When this creature enters, scry 1."
        let content = r#"
Name:Glider Kids
ManaCost:2 W
Types:Creature Human Pilot Ally
PT:2/3
K:Flying
T:Mode$ ChangesZone | Origin$ Any | Destination$ Battlefield | ValidCard$ Card.Self | Execute$ DBScry | TriggerDescription$ When this creature enters, scry 1. (Look at the top card of your library. You may put it on the bottom.)
SVar:DBScry:DB$ Scry | ScryNum$ 1
Oracle:Flying\nWhen this creature enters, scry 1. (Look at the top card of your library. You may put it on the bottom.)
"#;

        let def = CardLoader::parse(content).unwrap();
        let triggers = def.parse_triggers();

        // Verify we have at least one trigger (the ETB scry trigger)
        assert!(
            !triggers.is_empty(),
            "Should have at least one trigger. Got: {:?}",
            triggers
        );

        // Find the ETB trigger
        let etb_trigger = triggers.iter().find(|t| t.event == TriggerEvent::EntersBattlefield);

        assert!(
            etb_trigger.is_some(),
            "Should have an EntersBattlefield trigger. Triggers: {:?}",
            triggers
        );

        let trigger = etb_trigger.unwrap();

        // Verify it has a Scry effect
        let has_scry = trigger
            .effects
            .iter()
            .any(|e| matches!(e, Effect::Scry { count: 1, .. }));
        assert!(
            has_scry,
            "Trigger should have Scry effect with count=1. Effects: {:?}",
            trigger.effects
        );
    }

    #[test]
    fn test_parse_ba_sing_se_earthbend_ability() {
        use crate::core::Effect;

        // Test Ba Sing Se activated earthbend ability:
        // "{2}{G}, {T}: Earthbend 2. Activate only as a sorcery."
        let content = r#"
Name:Ba Sing Se
ManaCost:no cost
Types:Land
A:AB$ Mana | Cost$ T | Produced$ G | SpellDescription$ Add {G}.
A:AB$ Earthbend | Cost$ 2 G T | SorcerySpeed$ True | Num$ 2 | SpellDescription$ Earthbend 2. Activate only as a sorcery.
Oracle:This land enters tapped unless you control a basic land.\n{T}: Add {G}.\n{2}{G}, {T}: Earthbend 2. Activate only as a sorcery.
"#;

        let def = CardLoader::parse(content).unwrap();
        let abilities = def.parse_activated_abilities();

        // Should have 2 activated abilities: mana and earthbend
        assert!(
            abilities.len() >= 2,
            "Ba Sing Se should have at least 2 activated abilities (mana + earthbend). Got: {:?}",
            abilities
        );

        // Find the earthbend ability (not the mana ability)
        let earthbend_ability = abilities
            .iter()
            .find(|a| a.effects.iter().any(|e| matches!(e, Effect::Earthbend { .. })));

        assert!(
            earthbend_ability.is_some(),
            "Should have an Earthbend activated ability. Abilities: {:?}",
            abilities
        );

        let ability = earthbend_ability.unwrap();

        // Check it has the correct effect
        let Effect::Earthbend { num_counters, .. } = &ability.effects[0] else {
            panic!("Expected Earthbend effect, got {:?}", ability.effects[0]);
        };
        assert_eq!(*num_counters, 2, "Earthbend should put 2 counters");

        // Check the cost includes tap and mana
        assert!(ability.cost.includes_tap(), "Earthbend ability should have tap cost");

        // Check sorcery_speed flag is set
        assert!(
            ability.sorcery_speed,
            "Earthbend ability should be sorcery-speed (activate only as a sorcery)"
        );
    }

    #[test]
    fn test_parse_jade_statue_activation_phases() {
        use crate::game::phase::Step;

        // Jade Statue: "{2}: Jade Statue becomes a 3/6 Golem artifact creature
        // until end of combat. Activate only during combat."
        let content = r#"
Name:Jade Statue
ManaCost:4
Types:Artifact
A:AB$ Animate | Cost$ 2 | Defined$ Self | Power$ 3 | Toughness$ 6 | Types$ Creature,Artifact,Golem | RemoveCreatureTypes$ True | Duration$ UntilEndOfCombat | ActivationPhases$ BeginCombat->EndCombat | SpellDescription$ CARDNAME becomes a 3/6 Golem artifact creature until end of combat. Activate only during combat.
Oracle:{2}: Jade Statue becomes a 3/6 Golem artifact creature until end of combat. Activate only during combat.
"#;

        let def = CardLoader::parse(content).unwrap();
        let abilities = def.parse_activated_abilities();
        assert_eq!(
            abilities.len(),
            1,
            "Jade Statue should have exactly one activated ability"
        );

        let window = abilities[0]
            .activation_phases
            .expect("Jade Statue's animate ability must carry an ActivationPhases$ window");
        assert_eq!(window.start, Step::BeginCombat);
        assert_eq!(window.end, Step::EndCombat);
        assert!(window.contains(Step::DeclareBlockers));
        assert!(!window.contains(Step::Main1));
    }

    #[test]
    fn test_parse_foggy_swamp_vinebender_waterbend_ability() {
        use crate::core::Effect;

        // Test Foggy Swamp Vinebender's waterbend ability:
        // "Waterbend 5: Put a +1/+1 counter on this creature. Activate only during your turn."
        let content = r#"
Name:Foggy Swamp Vinebender
ManaCost:3 G
Types:Creature Human Ranger Ally
PT:4/3
S:Mode$ CantBlockBy | ValidAttacker$ Creature.Self | ValidBlocker$ Creature.powerLE2 | Description$ This creature can't be blocked by creatures with power 2 or less.
A:AB$ PutCounter | Cost$ Waterbend<5> | Defined$ Self | CounterType$ P1P1 | CounterNum$ 1 | PlayerTurn$ True | SpellDescription$ Put a +1/+1 counter on this creature. Activate only during your turn.
Oracle:This creature can't be blocked by creatures with power 2 or less.\nWaterbend {5}: Put a +1/+1 counter on this creature. Activate only during your turn.
"#;

        let def = CardLoader::parse(content).unwrap();
        let abilities = def.parse_activated_abilities();

        // Should have 1 activated ability: the PutCounter waterbend ability
        assert_eq!(
            abilities.len(),
            1,
            "Foggy Swamp Vinebender should have 1 activated ability (waterbend PutCounter). Got: {:?}",
            abilities
        );

        let ability = &abilities[0];

        // Check it has the PutCounter effect
        let Effect::PutCounter {
            counter_type, amount, ..
        } = &ability.effects[0]
        else {
            panic!("Expected PutCounter effect, got {:?}", ability.effects[0]);
        };
        assert_eq!(*counter_type, crate::core::CounterType::P1P1);
        assert_eq!(*amount, 1);

        // Check the cost includes waterbend
        assert!(
            ability.cost.get_waterbend_amount().is_some(),
            "Waterbend ability should have waterbend cost. Cost: {:?}",
            ability.cost
        );

        // Check your_turn_only flag is set (PlayerTurn$ True)
        assert!(
            ability.your_turn_only,
            "Waterbend ability should be your-turn-only (PlayerTurn$ True)"
        );

        // Sorcery_speed should NOT be set
        assert!(
            !ability.sorcery_speed,
            "Waterbend ability should NOT be sorcery-speed (just your-turn-only)"
        );
    }

    #[test]
    fn test_parse_rebellious_captives_exhaust_ability() {
        use crate::core::{CounterType, Effect};

        // Test Rebellious Captives exhaust ability:
        // "Exhaust — {6}: Put two +1/+1 counters on this creature, then earthbend 2."
        let content = r#"
Name:Rebellious Captives
ManaCost:1 G
Types:Creature Human Peasant Ally
PT:2/2
A:AB$ PutCounter | Cost$ 6 | Defined$ Self | CounterType$ P1P1 | CounterNum$ 2 | Exhaust$ True | SubAbility$ DBEarthbend | SpellDescription$ Put two +1/+1 counters on this creature, then earthbend 2. (Target land you control becomes a 0/0 creature with haste that's still a land. Put two +1/+1 counters on it. When it dies or is exiled, return it to the battlefield tapped. Activate each exhaust ability only once.)
SVar:DBEarthbend:DB$ Earthbend | Num$ 2
Oracle:Exhaust — {6}: Put two +1/+1 counters on this creature, then earthbend 2.
"#;

        let def = CardLoader::parse(content).unwrap();
        let abilities = def.parse_activated_abilities();

        // Should have the exhaust ability
        assert_eq!(
            abilities.len(),
            1,
            "Should have 1 activated ability, got: {abilities:?}"
        );

        let ability = &abilities[0];

        // Check the exhaust flag is set
        assert!(ability.exhaust, "Exhaust ability should have exhaust=true");

        // Check it has PutCounter effect (self-targeting)
        let has_put_counter = ability.effects.iter().any(|e| {
            matches!(e, Effect::PutCounter { counter_type, amount, .. }
                if *counter_type == CounterType::P1P1 && *amount == 2)
        });
        assert!(
            has_put_counter,
            "Should have PutCounter effect with 2 +1/+1 counters. Effects: {:?}",
            ability.effects
        );

        // Check cost is 6 generic mana
        let mana_cost = ability.cost.get_mana_cost();
        assert!(mana_cost.is_some(), "Exhaust ability should have mana cost");
        assert_eq!(
            mana_cost.unwrap().generic,
            6,
            "Exhaust ability should cost 6 generic mana"
        );
    }

    #[test]
    fn test_parse_teo_attackers_declared_trigger() {
        use crate::core::{Keyword, TriggerEvent};

        // Test Teo's AttackersDeclared trigger:
        // "Whenever one or more creatures you control with flying attack, draw a card, then discard a card."
        let content = r#"
Name:Teo, Spirited Glider
ManaCost:3 U
Types:Legendary Creature Human Pilot Ally
PT:1/4
K:Flying
T:Mode$ AttackersDeclared | AttackingPlayer$ You | ValidAttackers$ Creature.withFlying | Execute$ TrigDraw | TriggerZones$ Battlefield | TriggerDescription$ Whenever one or more creatures you control with flying attack, draw a card, then discard a card.
SVar:TrigDraw:DB$ Draw | SubAbility$ DBDiscard
SVar:DBDiscard:DB$ Discard | Defined$ You | NumCards$ 1 | Mode$ TgtChoose
Oracle:Flying\nWhenever one or more creatures you control with flying attack, draw a card, then discard a card.
"#;

        let def = CardLoader::parse(content).unwrap();
        let triggers = def.parse_triggers();

        // Should have the AttackersDeclared trigger
        assert!(
            !triggers.is_empty(),
            "Should have at least one trigger. Got: {:?}",
            triggers
        );

        // Find the AttackersDeclared trigger
        let attacker_trigger = triggers.iter().find(|t| t.event == TriggerEvent::AttackersDeclared);

        assert!(
            attacker_trigger.is_some(),
            "Should have an AttackersDeclared trigger. Triggers: {:?}",
            triggers
        );

        let trigger = attacker_trigger.unwrap();

        // Check controller_turn_only is true (AttackingPlayer$ You)
        assert!(
            trigger.controller_turn_only,
            "AttackersDeclared trigger should have controller_turn_only=true (AttackingPlayer$ You)"
        );

        // Check valid_attackers_keyword is Flying
        assert_eq!(
            trigger.valid_attackers_keyword,
            Some(Keyword::Flying),
            "AttackersDeclared trigger should filter for Flying creatures"
        );

        // Check trigger_self_only is false (it's a batch trigger, not per-creature)
        assert!(
            !trigger.trigger_self_only,
            "AttackersDeclared trigger should NOT be self-only (it's a batch trigger)"
        );

        // Verify effects are present (DrawCards)
        assert!(
            !trigger.effects.is_empty(),
            "AttackersDeclared trigger should have effects. Effects: {:?}",
            trigger.effects
        );
    }

    #[test]
    fn test_parse_rabaroo_troop_landfall_trigger() {
        use crate::core::{Effect, TriggerEvent};

        // Test Rabaroo Troop landfall trigger:
        // "Landfall — Whenever a land you control enters, this creature gains flying until end of turn and you gain 1 life."
        let content = r#"
Name:Rabaroo Troop
ManaCost:3 W W
Types:Creature Rabbit Kangaroo
PT:3/5
T:Mode$ ChangesZone | Origin$ Any | Destination$ Battlefield | ValidCard$ Land.YouCtrl | TriggerZones$ Battlefield | Execute$ TrigFlying | TriggerDescription$ Landfall — Whenever a land you control enters, this creature gains flying until end of turn and you gain 1 life.
SVar:TrigFlying:DB$ Pump | Defined$ Self | KW$ Flying | SubAbility$ DBGainLife
SVar:DBGainLife:DB$ GainLife | Defined$ You | LifeAmount$ 1
Oracle:Landfall — Whenever a land you control enters, this creature gains flying until end of turn and you gain 1 life.
"#;

        let def = CardLoader::parse(content).unwrap();
        let triggers = def.parse_triggers();

        // Verify we have at least one trigger (the landfall trigger)
        assert!(
            !triggers.is_empty(),
            "Should have at least one trigger. Got: {:?}",
            triggers
        );

        // Find the landfall trigger (ETB with [landfall] marker)
        let landfall_trigger = triggers
            .iter()
            .find(|t| t.event == TriggerEvent::EntersBattlefield && t.description.contains("[landfall]"));

        assert!(
            landfall_trigger.is_some(),
            "Should have a landfall trigger (EntersBattlefield with [landfall]). Triggers: {:?}",
            triggers
        );

        let trigger = landfall_trigger.unwrap();

        // Verify trigger_self_only is false (landfall triggers on OTHER cards entering)
        assert!(
            !trigger.trigger_self_only,
            "Landfall trigger should have trigger_self_only=false"
        );

        // Verify it has a PumpCreature effect (for Flying keyword)
        let has_pump = trigger.effects.iter().any(|e| matches!(e, Effect::PumpCreature { .. }));
        assert!(
            has_pump,
            "Trigger should have PumpCreature effect (for Flying). Effects: {:?}",
            trigger.effects
        );
    }

    #[test]
    fn test_otter_penguin_subability_chain_with_effect() {
        // Test parsing Otter-Penguin's trigger with SubAbility$ chain to DB$ Effect
        // This is the pattern: TrigPump -> DBUnblockable (DB$ Effect with StaticAbilities$)
        let card_data = r#"Name:Otter-Penguin
ManaCost:1 U
Types:Creature Otter Bird
PT:2/1
T:Mode$ Drawn | ValidCard$ Card.YouCtrl | Number$ 2 | TriggerZones$ Battlefield | Execute$ TrigPump | TriggerDescription$ Whenever you draw your second card each turn, this creature gets +1/+2 until end of turn and can't be blocked this turn.
SVar:TrigPump:DB$ Pump | Defined$ Self | NumAtt$ +1 | NumDef$ +2 | SubAbility$ DBUnblockable
SVar:DBUnblockable:DB$ Effect | RememberObjects$ Self | ExileOnMoved$ Battlefield | StaticAbilities$ Unblockable
SVar:Unblockable:Mode$ CantBlockBy | ValidAttacker$ Card.IsRemembered | Description$ EFFECTSOURCE can't be blocked this turn.
Oracle:Whenever you draw your second card each turn, this creature gets +1/+2 until end of turn and can't be blocked this turn."#;

        let def = CardLoader::parse(card_data).expect("Should parse Otter-Penguin card data");
        assert_eq!(def.name.as_str(), "Otter-Penguin");

        // Parse triggers from the definition
        let triggers = def.parse_triggers();

        // Should have 1 trigger (Drawn trigger)
        assert_eq!(triggers.len(), 1, "Should have 1 trigger");

        let trigger = &triggers[0];
        assert_eq!(trigger.event, TriggerEvent::CardDrawn);
        assert_eq!(trigger.draw_number, Some(2)); // Second card drawn

        // Trigger should have 2 effects: PumpCreature and GrantCantBeBlocked
        assert!(
            trigger.effects.len() >= 2,
            "Expected at least 2 effects (Pump + GrantCantBeBlocked), got {}: {:?}",
            trigger.effects.len(),
            trigger.effects
        );

        // Check for PumpCreature effect
        let has_pump = trigger.effects.iter().any(|e| matches!(e, Effect::PumpCreature { .. }));
        assert!(
            has_pump,
            "Trigger should have PumpCreature effect: {:?}",
            trigger.effects
        );

        // Check for GrantCantBeBlocked effect (from SubAbility$ chain)
        let has_cant_be_blocked = trigger
            .effects
            .iter()
            .any(|e| matches!(e, Effect::GrantCantBeBlocked { .. }));
        assert!(
            has_cant_be_blocked,
            "Trigger should have GrantCantBeBlocked effect from SubAbility$ chain: {:?}",
            trigger.effects
        );
    }

    #[test]
    fn test_avatar_kyoshi_begin_combat_trigger() {
        // Test parsing Avatar Kyoshi's BeginCombat trigger with SubAbility$ chain
        // Trigger: earthbend 8, then untap that land
        let card_data = r#"Name:Avatar Kyoshi, Earthbender
ManaCost:5 G G G
Types:Legendary Creature Human Avatar
PT:6/6
T:Mode$ Phase | Phase$ BeginCombat | ValidPlayer$ You | Execute$ TrigEarthbend | TriggerZones$ Battlefield | TriggerDescription$ At the beginning of combat on your turn, earthbend 8, then untap that land.
SVar:TrigEarthbend:DB$ Earthbend | Num$ 8 | SubAbility$ DBUntap
SVar:DBUntap:DB$ Untap | Defined$ Targeted
Oracle:At the beginning of combat on your turn, earthbend 8, then untap that land."#;

        let def = CardLoader::parse(card_data).expect("Should parse Avatar Kyoshi card data");
        assert_eq!(def.name.as_str(), "Avatar Kyoshi, Earthbender");

        // Parse triggers from the definition
        let triggers = def.parse_triggers();

        // Should have 1 trigger (BeginCombat trigger)
        assert_eq!(triggers.len(), 1, "Should have 1 trigger");

        let trigger = &triggers[0];
        assert_eq!(trigger.event, TriggerEvent::BeginningOfCombat);

        // Trigger should have 2 effects: Earthbend + UntapPermanent
        assert!(
            trigger.effects.len() >= 2,
            "Expected at least 2 effects (Earthbend + Untap), got {}: {:?}",
            trigger.effects.len(),
            trigger.effects
        );

        // Check for Earthbend effect
        let has_earthbend = trigger
            .effects
            .iter()
            .any(|e| matches!(e, Effect::Earthbend { num_counters: 8, .. }));
        assert!(
            has_earthbend,
            "Trigger should have Earthbend effect with 8 counters: {:?}",
            trigger.effects
        );

        // Check for UntapPermanent effect (from SubAbility$ chain)
        let has_untap = trigger
            .effects
            .iter()
            .any(|e| matches!(e, Effect::UntapPermanent { .. }));
        assert!(
            has_untap,
            "Trigger should have UntapPermanent effect from SubAbility$ chain: {:?}",
            trigger.effects
        );

        // Check that the trigger is controller-only (ValidPlayer$ You)
        // This is indicated by the [controller_only] prefix in description
        assert!(
            trigger.description.contains("[controller_only]"),
            "Trigger description should have [controller_only] prefix for ValidPlayer$ You"
        );
    }

    #[test]
    fn test_elephant_mandrill_variable_pump() {
        // Test parsing Elephant-Mandrill's variable pump with Count$Valid expression
        // The card gets +X/+X where X is the number of artifacts opponents control
        let card_data = r#"Name:Elephant-Mandrill
ManaCost:2 G
Types:Creature Elephant Monkey
PT:3/2
K:Reach
T:Mode$ Phase | Phase$ BeginCombat | ValidPlayer$ You | TriggerZones$ Battlefield | Execute$ TrigPump | TriggerDescription$ At the beginning of combat on your turn, this creature gets +1/+1 until end of turn for each artifact your opponents control.
SVar:TrigPump:DB$ Pump | Defined$ Self | NumAtt$ +X | NumDef$ +X
SVar:X:Count$Valid Artifact.OppCtrl
Oracle:Reach\nAt the beginning of combat on your turn, this creature gets +1/+1 until end of turn for each artifact your opponents control."#;

        let def = CardLoader::parse(card_data).expect("Should parse Elephant-Mandrill card data");
        assert_eq!(def.name.as_str(), "Elephant-Mandrill");

        // Parse triggers from the definition
        let triggers = def.parse_triggers();

        // Should have 1 trigger (BeginCombat trigger)
        assert_eq!(triggers.len(), 1, "Should have 1 trigger");

        let trigger = &triggers[0];
        assert_eq!(trigger.event, TriggerEvent::BeginningOfCombat);

        // Trigger should have PumpCreatureVariable effect (NOT fixed PumpCreature)
        let has_variable_pump = trigger
            .effects
            .iter()
            .any(|e| matches!(e, Effect::PumpCreatureVariable { .. }));
        assert!(
            has_variable_pump,
            "Trigger should have PumpCreatureVariable effect for +X/+X: {:?}",
            trigger.effects
        );

        // Verify the count expression is ValidPermanents with Artifact.OppCtrl filter
        let pump_effect = trigger
            .effects
            .iter()
            .find(|e| matches!(e, Effect::PumpCreatureVariable { .. }))
            .unwrap();

        if let Effect::PumpCreatureVariable {
            power_count,
            toughness_count,
            ..
        } = pump_effect
        {
            // Both power and toughness should use the same Count$ expression
            assert!(
                matches!(power_count, crate::core::CountExpression::ValidPermanents { filter } if filter.contains("Artifact")),
                "Power count should be ValidPermanents with Artifact filter: {:?}",
                power_count
            );
            assert!(
                matches!(toughness_count, crate::core::CountExpression::ValidPermanents { filter } if filter.contains("Artifact")),
                "Toughness count should be ValidPermanents with Artifact filter: {:?}",
                toughness_count
            );
        }

        // Check that the trigger is controller-only (ValidPlayer$ You)
        assert!(
            trigger.description.contains("[controller_only]"),
            "Trigger description should have [controller_only] prefix for ValidPlayer$ You"
        );
    }

    #[test]
    fn test_copy_artifact_etb_clone_wiring() {
        // Copy Artifact: the K:ETBReplacement:Copy:DBCopy:Optional keyword must
        // wire its DBCopy SVar (DB$ Clone) into an Effect::Clone on the card,
        // with optional=true coming from the keyword's `Optional` flag.
        let card_data = r#"Name:Copy Artifact
ManaCost:1 U
Types:Enchantment
K:ETBReplacement:Copy:DBCopy:Optional
SVar:DBCopy:DB$ Clone | Choices$ Artifact.Other | AddTypes$ Enchantment | SpellDescription$ You may have CARDNAME enter as a copy of any artifact on the battlefield, except it's an enchantment in addition to its other types.
Oracle:You may have Copy Artifact enter as a copy of any artifact on the battlefield, except it's an enchantment in addition to its other types."#;

        let def = CardLoader::parse(card_data).expect("Should parse Copy Artifact");
        let effects = def.parse_effects();

        let clone = effects
            .iter()
            .find_map(|e| {
                if let crate::core::Effect::Clone {
                    choices_filter,
                    add_types,
                    optional,
                    ..
                } = e
                {
                    Some((choices_filter, add_types, *optional))
                } else {
                    None
                }
            })
            .expect("Copy Artifact must produce an Effect::Clone");

        let (choices_filter, add_types, optional) = clone;
        assert!(
            optional,
            "ETBReplacement `Optional` flag must set Effect::Clone.optional = true"
        );
        assert_eq!(
            add_types.as_slice(),
            &[crate::core::CardType::Enchantment],
            "AddTypes$ Enchantment must add the Enchantment card type"
        );
        assert!(
            choices_filter
                .types
                .iter()
                .any(|t| matches!(t, crate::core::TargetType::Artifact)),
            "Choices$ Artifact.Other must restrict copy targets to artifacts"
        );
    }

    #[test]
    fn test_ward_waterbend_parsing() {
        // Test parsing Ward:Waterbend<4> (The Unagi of Kyoshi Island)
        let card_data = r#"Name:The Unagi of Kyoshi Island
ManaCost:3 U U
Types:Legendary Creature Serpent
PT:5/5
K:Flash
K:Ward:Waterbend<4>
Oracle:Flash\nWard—Waterbend {4}"#;

        let def = CardLoader::parse(card_data).expect("Should parse card data");
        assert_eq!(def.name.as_str(), "The Unagi of Kyoshi Island");

        // Parse keywords
        let keywords = def.parse_keywords();

        // Should have Flash keyword
        assert!(keywords.contains(Keyword::Flash), "Should have Flash keyword");

        // Should have Ward keyword (WardWaterbend maps to Ward)
        assert!(keywords.contains(Keyword::Ward), "Should have Ward keyword");

        // Check for WardWaterbend specifically
        let has_ward_waterbend = keywords.iter().any(|kw| {
            if let Some(args) = keywords.get_args(kw) {
                matches!(args, KeywordArgs::WardWaterbend { amount: 4 })
            } else {
                false
            }
        });
        assert!(
            has_ward_waterbend,
            "Should have WardWaterbend with amount 4. Keywords: {:?}",
            keywords
        );
    }

    #[test]
    fn test_conditional_hexproof_player_turn() {
        use crate::core::{AffectedSelector, StaticAbility, StaticCondition};

        // Test parsing conditional hexproof (Avatar Kyoshi)
        // "During your turn, this creature has hexproof"
        let card_data = r#"Name:Avatar Kyoshi, Earthbender
ManaCost:5 G G G
Types:Legendary Creature Human Avatar
PT:6/6
S:Mode$ Continuous | Affected$ Card.Self | AddKeyword$ Hexproof | Condition$ PlayerTurn | Description$ During your turn, NICKNAME has hexproof.
Oracle:During your turn, Avatar Kyoshi has hexproof."#;

        let def = CardLoader::parse(card_data).expect("Should parse card data");
        assert_eq!(def.name.as_str(), "Avatar Kyoshi, Earthbender");

        // Parse static abilities
        let static_abilities = def.parse_static_abilities();

        // Should have exactly one static ability
        assert_eq!(
            static_abilities.len(),
            1,
            "Should have 1 static ability. Got: {:?}",
            static_abilities
        );

        // Check it's a GrantKeyword for Hexproof with PlayerTurn condition
        let ability = &static_abilities[0];
        if let StaticAbility::GrantKeyword {
            affected,
            keyword,
            description: _,
            condition,
        } = ability
        {
            assert_eq!(*affected, AffectedSelector::Self_, "Should affect self");
            assert_eq!(*keyword, Keyword::Hexproof, "Should grant Hexproof");
            assert_eq!(
                *condition,
                Some(StaticCondition::PlayerTurn),
                "Should have PlayerTurn condition"
            );
        } else {
            panic!("Expected GrantKeyword static ability, got: {:?}", ability);
        }
    }

    #[test]
    fn test_raise_cost_sacrifice_parsing() {
        use crate::core::{RaisedCost, RaisedCostAmount, StaticAbility};

        // Test parsing of RaiseCost with sacrifice
        let content = r#"
Name:Tectonic Split
ManaCost:4 G G
Types:Enchantment
S:Mode$ RaiseCost | ValidCard$ Card.Self | Type$ Spell | Cost$ Sac<X/Land/land(s)> | Description$ Sacrifice half your lands.
SVar:X:Count$Valid Land.YouCtrl/HalfUp
Oracle:Sacrifice half your lands, rounded up.
"#;

        let def = CardLoader::parse(content).unwrap();
        assert_eq!(def.name.as_str(), "Tectonic Split");

        // Parse static abilities directly from CardDefinition
        let static_abilities = def.parse_static_abilities();

        // Verify it has a RaiseCost static ability
        assert!(!static_abilities.is_empty(), "Should have static abilities");

        let has_raise_cost = static_abilities.iter().any(|ability| {
            matches!(
                ability,
                StaticAbility::RaiseCost {
                    raised_cost: RaisedCost::Sacrifice {
                        amount: RaisedCostAmount::Variable(x),
                        valid_type,
                    },
                    ..
                } if x == "X" && valid_type == "Land"
            )
        });

        assert!(has_raise_cost, "Should have RaiseCost::Sacrifice with X/Land");
    }

    #[test]
    fn test_raise_cost_mana_parsing() {
        use crate::core::{CostReductionTarget, RaisedCost, StaticAbility};

        // Test parsing of RaiseCost with mana amount
        let content = r#"
Name:Thalia Guardian
ManaCost:1 W
Types:Creature Human Soldier
PT:2/1
S:Mode$ RaiseCost | ValidCard$ Card.nonCreature | Type$ Spell | Amount$ 1 | Description$ Noncreature spells cost {1} more.
Oracle:Noncreature spells cost {1} more to cast.
"#;

        let def = CardLoader::parse(content).unwrap();
        assert_eq!(def.name.as_str(), "Thalia Guardian");

        // Parse static abilities directly from CardDefinition
        let static_abilities = def.parse_static_abilities();

        let has_raise_cost = static_abilities.iter().any(|ability| {
            matches!(
                ability,
                StaticAbility::RaiseCost {
                    valid_card: CostReductionTarget::NonCreature,
                    raised_cost: RaisedCost::Mana(1),
                    ..
                }
            )
        });

        assert!(has_raise_cost, "Should have RaiseCost::Mana(1) for non-creatures");
    }

    #[test]
    fn test_grant_ability_parsing() {
        use crate::core::effects::AffectedSelector;
        use crate::core::StaticAbility;

        // Test parsing of GrantAbility (AddAbility$)
        // Example: Chromatic Lantern grants lands "{T}: Add one mana of any color."
        let content = r#"
Name:Chromatic Lantern
ManaCost:3
Types:Artifact
A:AB$ Mana | Cost$ T | Produced$ Any | SpellDescription$ Add one mana of any color.
S:Mode$ Continuous | Affected$ Land.YouCtrl | AddAbility$ AnyMana | Description$ Lands you control have "{T}: Add one mana of any color."
SVar:AnyMana:AB$ Mana | Cost$ T | Produced$ Any | SpellDescription$ Add one mana of any color.
Oracle:Lands you control have "{T}: Add one mana of any color."
"#;

        let def = CardLoader::parse(content).unwrap();
        assert_eq!(def.name.as_str(), "Chromatic Lantern");

        let static_abilities = def.parse_static_abilities();

        let has_grant_ability = static_abilities.iter().any(|ability| {
            matches!(
                ability,
                StaticAbility::GrantAbility {
                    affected: AffectedSelector::LandsYouControl,
                    ability,
                    ..
                } if ability.is_mana_ability
            )
        });

        assert!(
            has_grant_ability,
            "Should have GrantAbility for lands with mana ability"
        );
    }

    #[test]
    fn test_triskelion_etb_trigger_with_put_counter() {
        use crate::core::{Effect, TriggerEvent};

        // Triskelion: ETB puts three +1/+1 counters on itself
        let content = r#"
Name:Triskelion
ManaCost:6
Types:Artifact Creature Construct
PT:1/1
T:Mode$ ChangesZone | Origin$ Any | Destination$ Battlefield | ValidCard$ Card.Self | Execute$ TrigPutCounters | TriggerDescription$ Triskelion enters the battlefield with three +1/+1 counters on it.
SVar:TrigPutCounters:DB$ PutCounter | Defined$ Self | CounterType$ P1P1 | CounterNum$ 3
Oracle:Triskelion enters the battlefield with three +1/+1 counters on it.
"#;

        let def = CardLoader::parse(content).unwrap();
        let triggers = def.parse_triggers();

        assert_eq!(triggers.len(), 1, "Should have one ETB trigger");

        let trigger = &triggers[0];
        assert_eq!(trigger.event, TriggerEvent::EntersBattlefield);

        // Must have a PutCounter effect with amount 3
        let has_put_counter_3 = trigger
            .effects
            .iter()
            .any(|e| matches!(e, Effect::PutCounter { amount: 3, .. }));
        assert!(
            has_put_counter_3,
            "ETB trigger should have PutCounter with amount 3, got effects: {:?}",
            trigger.effects
        );
    }

    #[test]
    fn test_triskelion_real_card_etb_counter_keyword() {
        // The REAL Triskelion card-script (forge-java) uses K:etbCounter:P1P1:3,
        // not the redundant SVar trigger above. Make sure the keyword survives
        // parsing so the runtime `apply_etb_counters` path can place the counters.
        use crate::core::{Keyword, KeywordArgs, PlayerId};

        let content = r#"
Name:Triskelion
ManaCost:6
Types:Artifact Creature Construct
PT:1/1
K:etbCounter:P1P1:3
A:AB$ DealDamage | AILogic$ Triskelion | Cost$ SubCounter<1/P1P1> | ValidTgts$ Any | NumDmg$ 1 | SpellDescription$ It deals 1 damage to any target.
Oracle:Triskelion enters with three +1/+1 counters on it.
"#;

        let def = CardLoader::parse(content).unwrap();
        let card = def.instantiate(crate::core::CardId::new(1), PlayerId::new(0));

        let args = card
            .keywords
            .get_args(Keyword::EtbCounter)
            .expect("etbCounter keyword should be parsed onto the card");

        if let KeywordArgs::EtbCounter {
            counter_type,
            amount,
            condition: _,
        } = args
        {
            assert_eq!(counter_type, "P1P1", "Triskelion ETB counter type");
            assert_eq!(amount, "3", "Triskelion ETB counter amount");
        } else {
            panic!("expected EtbCounter args, got {args:?}");
        }
    }

    #[test]
    fn test_bazaar_of_baghdad_draw_discard_chain() {
        use crate::core::{Effect, PlayerId};

        // Bazaar of Baghdad: T: Draw 2, then discard 3
        let content = r#"
Name:Bazaar of Baghdad
ManaCost:no cost
Types:Land
A:AB$ Draw | Cost$ T | NumCards$ 2 | SubAbility$ DBDiscard | SpellDescription$ Draw two cards, then discard three cards.
SVar:DBDiscard:DB$ Discard | Defined$ You | NumCards$ 3
Oracle:{T}: Draw two cards, then discard three cards.
"#;

        let def = CardLoader::parse(content).unwrap();
        let card = def.instantiate(crate::core::CardId::new(1), PlayerId::new(0));

        // Should have at least 1 activated ability with draw+discard effects
        assert!(!card.activated_abilities.is_empty(), "Should have activated abilities");

        let ability = &card.activated_abilities[0];
        let effects = &ability.effects;

        // Must have both Draw and Discard effects
        let has_draw_2 = effects.iter().any(|e| matches!(e, Effect::DrawCards { count: 2, .. }));
        let has_discard_3 = effects
            .iter()
            .any(|e| matches!(e, Effect::DiscardCards { count: 3, .. }));

        assert!(has_draw_2, "Should have DrawCards with count 2, got: {:?}", effects);
        assert!(
            has_discard_3,
            "Should have DiscardCards with count 3, got: {:?}",
            effects
        );
    }

    /// Regression for mtg-8scpx: a bincode round-trip of a CardDefinition drops
    /// `parsed_svars` (it is `#[serde(skip)]`). Trigger parsing resolves
    /// `Execute$ <SVar>` effects via `parsed_svars`, so WITHOUT a post-deserialize
    /// `rebuild_parsed_svars()` the trigger parses to ZERO effects — which is
    /// exactly how the WASM `load_set` path silently dropped City of Brass's
    /// `Taps` self-ping and Su-Chi's death trigger, diverging WASM from native.
    /// This test pins the contract: round-trip + rebuild MUST restore the effect.
    #[test]
    fn test_svar_trigger_survives_bincode_roundtrip_after_rebuild() {
        use crate::core::{Effect, PlayerId, TriggerEvent};

        // City of Brass: Taps trigger whose effect lives in the TrigDamage SVar.
        let content = r#"
Name:City of Brass
ManaCost:no cost
Types:Land
A:AB$ Mana | Cost$ T | Produced$ Any | Amount$ 1 | SpellDescription$ Add one mana of any color.
T:Mode$ Taps | ValidCard$ Card.Self | Execute$ TrigDamage | TriggerZones$ Battlefield | TriggerDescription$ Whenever CARDNAME becomes tapped, it deals 1 damage to you.
SVar:TrigDamage:DB$ DealDamage | Defined$ You | NumDmg$ 1
Oracle:Whenever City of Brass becomes tapped, it deals 1 damage to you.
"#;

        let def = CardLoader::parse(content).unwrap();

        // Sanity: freshly-parsed definition produces a Taps trigger that deals damage.
        let fresh = def.instantiate(crate::core::CardId::new(1), PlayerId::new(0));
        let fresh_taps = fresh
            .triggers
            .iter()
            .find(|t| t.event == TriggerEvent::Taps)
            .expect("freshly-parsed City of Brass must have a Taps trigger");
        assert!(
            fresh_taps
                .effects
                .iter()
                .any(|e| matches!(e, Effect::DealDamage { .. })),
            "freshly-parsed Taps trigger must carry the SVar DealDamage effect, got: {:?}",
            fresh_taps.effects
        );

        // Round-trip through bincode (the WASM `decks.bin`/set-bin wire format).
        let bytes = bincode::serialize(&def).expect("serialize CardDefinition");
        let mut restored: CardDefinition = bincode::deserialize(&bytes).expect("deserialize CardDefinition");

        // BEFORE rebuild: parsed_svars is empty, so the Execute$ SVar resolves to
        // nothing and the trigger is effect-less. This is the bug shape.
        assert!(
            restored.parsed_svars.is_empty(),
            "bincode-deserialized CardDefinition should arrive with empty parsed_svars"
        );
        let pre = restored.instantiate(crate::core::CardId::new(2), PlayerId::new(0));
        let pre_taps = pre.triggers.iter().find(|t| t.event == TriggerEvent::Taps);
        assert!(
            pre_taps.is_none_or(|t| t.effects.is_empty()),
            "without rebuild_parsed_svars the Execute$ SVar effect must be dropped (the mtg-8scpx bug)"
        );

        // AFTER rebuild (what load_set now does): the effect is restored.
        restored.rebuild_parsed_svars();
        let post = restored.instantiate(crate::core::CardId::new(3), PlayerId::new(0));
        let post_taps = post
            .triggers
            .iter()
            .find(|t| t.event == TriggerEvent::Taps)
            .expect("after rebuild the Taps trigger must exist");
        assert!(
            post_taps.effects.iter().any(|e| matches!(e, Effect::DealDamage { .. })),
            "after rebuild_parsed_svars the Taps trigger must carry the DealDamage effect again, got: {:?}",
            post_taps.effects
        );
    }

    /// Parser-shape regression for the 1994 World Championship compat sweep
    /// (mtg-713 B12). The `R:Event$ Moved | … | ReplaceWith$ ETBTapped`
    /// replacement comes in two forms and the loader must classify them
    /// STRUCTURALLY (not by substring-matching the line):
    ///   * `ValidCard$ Card.Self`   → host self-taps   (`enters_tapped`)
    ///   * a global predicate       → OTHER permanents (`etb_tapped_global`)
    ///
    /// Predicates carrying qualifiers we don't model yet (e.g. `nonBasic`) must
    /// be refused (left `None`) rather than silently widened.
    #[test]
    fn test_etb_tapped_replacement_classification() {
        use crate::core::effects::{ControllerRestriction, TargetType};

        // GLOBAL form (Kismet): the host must NOT self-tap, and the global
        // predicate must capture all three opponent-controlled types.
        let kismet = CardLoader::parse(
            r#"
Name:Kismet
ManaCost:3 W
Types:Enchantment
R:Event$ Moved | ValidCard$ Artifact.OppCtrl,Creature.OppCtrl,Land.OppCtrl | Destination$ Battlefield | ReplaceWith$ ETBTapped | ReplacementResult$ Updated | ActiveZones$ Battlefield | Description$ Artifacts, creatures, and lands your opponents control enter tapped.
SVar:ETBTapped:DB$ Tap | ETB$ True | Defined$ ReplacedCard
Oracle:Artifacts, creatures, and lands your opponents control enter tapped.
"#,
        )
        .unwrap();
        assert!(
            !kismet.enters_tapped,
            "Kismet hosts a GLOBAL replacement — it must NOT set its own enters_tapped"
        );
        let pred = kismet
            .etb_tapped_global
            .as_ref()
            .expect("Kismet must store a global ETB-tapped predicate");
        assert_eq!(
            pred.controller,
            ControllerRestriction::OppCtrl,
            "Kismet's predicate is OppCtrl-relative"
        );
        for ty in [TargetType::Artifact, TargetType::Creature, TargetType::Land] {
            assert!(pred.types.contains(&ty), "Kismet predicate must include {ty:?}");
        }

        // SELF form (a tapped land): host self-taps; no global predicate.
        let barren_moor = CardLoader::parse(
            r#"
Name:Barren Moor
ManaCost:no cost
Types:Land
R:Event$ Moved | ValidCard$ Card.Self | Destination$ Battlefield | ReplacementResult$ Updated | ReplaceWith$ ETBTapped | Description$ CARDNAME enters tapped.
SVar:ETBTapped:DB$ Tap | Defined$ Self | ETB$ True
Oracle:Barren Moor enters tapped.
"#,
        )
        .unwrap();
        assert!(
            barren_moor.enters_tapped,
            "Card.Self form must set the host's enters_tapped"
        );
        assert!(
            barren_moor.etb_tapped_global.is_none(),
            "a self-tapping land must NOT carry a global predicate"
        );

        // SYMMETRIC global form (Root Maze): predicate with no controller filter.
        let root_maze = CardLoader::parse(
            r#"
Name:Root Maze
ManaCost:G
Types:Enchantment
R:Event$ Moved | ValidCard$ Artifact,Land | Destination$ Battlefield | ReplaceWith$ ETBTapped | ReplacementResult$ Updated | ActiveZones$ Battlefield | Description$ Artifacts and lands enter tapped.
SVar:ETBTapped:DB$ Tap | ETB$ True | Defined$ ReplacedCard
Oracle:Artifacts and lands enter tapped.
"#,
        )
        .unwrap();
        let rm_pred = root_maze
            .etb_tapped_global
            .as_ref()
            .expect("Root Maze must store a global ETB-tapped predicate");
        assert_eq!(
            rm_pred.controller,
            ControllerRestriction::Any,
            "Root Maze is symmetric — Any controller"
        );
        assert!(!root_maze.enters_tapped, "Root Maze must not self-tap");

        // GATED form (unsupported `nonBasic` qualifier — Thalia, Heretic Cathar):
        // refused rather than over-matching every opponent land.
        let thalia = CardLoader::parse(
            r#"
Name:Thalia, Heretic Cathar
ManaCost:2 W
Types:Legendary Creature Human Soldier
PT:3/2
K:First Strike
R:Event$ Moved | ValidCard$ Creature.OppCtrl,Land.nonBasic+OppCtrl | Destination$ Battlefield | ReplaceWith$ ETBTapped | ReplacementResult$ Updated | ActiveZones$ Battlefield | Description$ Creatures and nonbasic lands your opponents control enter tapped.
SVar:ETBTapped:DB$ Tap | ETB$ True | Defined$ ReplacedCard
Oracle:First strike\nCreatures and nonbasic lands your opponents control enter tapped.
"#,
        )
        .unwrap();
        assert!(
            thalia.etb_tapped_global.is_none(),
            "unsupported `nonBasic` qualifier must be refused (no over-matching), see mtg-713 B12 follow-up"
        );
        assert!(
            !thalia.enters_tapped,
            "the gated card must not fall back to self-tap either"
        );
    }

    /// Parser-shape regression for the 1994 World Championship compat sweep
    /// (mtg-904 / mtg-713 B13). Winter Orb's `S:Mode$ Continuous | Affected$
    /// Player | AddKeyword$ UntapAdjust:Land:1 | IsPresent$ Card.Self+untapped`
    /// must lower into a `limits_land_untap = Some(1)` lock. Before the fix the
    /// `AddKeyword$ UntapAdjust:Land:N` player-keyword was unrecognized, so the
    /// untap step untapped all lands and Winter Orb was inert.
    #[test]
    fn test_winter_orb_limits_land_untap_classification() {
        let winter_orb = CardLoader::parse(
            r#"
Name:Winter Orb
ManaCost:2
Types:Artifact
S:Mode$ Continuous | Affected$ Player | AddKeyword$ UntapAdjust:Land:1 | IsPresent$ Card.Self+untapped | Description$ As long as CARDNAME is untapped, players can't untap more than one land during their untap steps.
SVar:NonStackingEffect:True
Oracle:As long as Winter Orb is untapped, players can't untap more than one land during their untap steps.
"#,
        )
        .unwrap();
        assert_eq!(
            winter_orb.limits_land_untap(),
            Some(1),
            "Winter Orb must classify into a one-land untap limit"
        );
        let instance = winter_orb.instantiate(crate::core::CardId::new(1), crate::core::PlayerId::new(0));
        assert_eq!(
            instance.definition.cache.limits_land_untap,
            Some(1),
            "the limits_land_untap lock must reach the per-instance CardCache"
        );

        // A plain artifact with no UntapAdjust static must NOT carry the lock.
        let plain = CardLoader::parse(
            r#"
Name:Plain Artifact
ManaCost:2
Types:Artifact
Oracle:A vanilla artifact.
"#,
        )
        .unwrap();
        assert_eq!(
            plain.limits_land_untap(),
            None,
            "a vanilla artifact must not classify as a land-untap lock"
        );
    }

    /// Parser-shape regression for the 1994 World Championship compat sweep
    /// (mtg-713 B1). An activated `AB$ GainControl` must convert into an
    /// `Effect::GainControl` carrying a structured `ControlDuration` (from
    /// `LoseControl$`) and a `TargetRestriction` (from `ValidTgts$`) — replacing
    /// the old "no restriction + binary until_eot" shape that left Aladdin/Old
    /// Man uncastable.
    #[test]
    #[allow(clippy::wildcard_enum_match_arm)] // closure only inspects GainControl
    fn test_gaincontrol_ability_parses_duration_and_restriction() {
        use crate::core::effects::{ControlDuration, Effect, TargetType};
        use crate::core::PlayerId;

        let gain_control = |def: &CardDefinition| -> (ControlDuration, crate::core::effects::TargetRestriction) {
            let card = def.instantiate(crate::core::CardId::new(1), PlayerId::new(0));
            card.activated_abilities
                .iter()
                .flat_map(|a| a.effects.iter())
                .find_map(|e| match e {
                    Effect::GainControl {
                        duration, restriction, ..
                    } => Some((*duration, restriction.clone())),
                    _ => None,
                })
                .expect("card must have an activated GainControl effect")
        };

        // Aladdin: ValidTgts$ Artifact, LoseControl$ LeavesPlay,LoseControl ->
        // Artifact restriction + WhileControlSource duration.
        let aladdin = CardLoader::parse(
            r#"
Name:Aladdin
ManaCost:2 R R
Types:Creature Human Rogue
PT:1/1
A:AB$ GainControl | Cost$ 1 R R T | ValidTgts$ Artifact | LoseControl$ LeavesPlay,LoseControl | SpellDescription$ Gain control of target artifact for as long as you control CARDNAME.
Oracle:{1}{R}{R}, {T}: Gain control of target artifact for as long as you control Aladdin.
"#,
        )
        .unwrap();
        let (dur, restr) = gain_control(&aladdin);
        assert_eq!(
            dur,
            ControlDuration::WhileControlSource,
            "Aladdin keeps control while it controls Aladdin"
        );
        assert!(restr.types.contains(&TargetType::Artifact), "Aladdin targets artifacts");
        assert!(!restr.power_le_source, "Aladdin has no power threshold");

        // Old Man of the Sea: ValidTgts$ Creature.powerLEX -> Creature restriction
        // with the dynamic source-power threshold flagged.
        let old_man = CardLoader::parse(
            r#"
Name:Old Man of the Sea
ManaCost:1 U U
Types:Creature Djinn
PT:2/3
A:AB$ GainControl | Cost$ T | ValidTgts$ Creature.powerLEX | LoseControl$ Untap,LeavesPlay,LoseControl,StaticCommandCheck | SpellDescription$ Gain control of target creature with power less than or equal to CARDNAME's power.
SVar:X:Count$CardPower
Oracle:{T}: Gain control of target creature with power less than or equal to Old Man of the Sea's power.
"#,
        )
        .unwrap();
        let (om_dur, om_restr) = gain_control(&old_man);
        assert!(
            om_restr.types.contains(&TargetType::Creature),
            "Old Man targets creatures"
        );
        assert!(
            om_restr.power_le_source,
            "Old Man's `powerLEX` must set the dynamic source-power threshold"
        );
        // The precise tapped + power-comparison duration is a follow-on (mtg-713 B1);
        // for now it is bounded by the source-presence duration rather than permanent.
        assert_eq!(om_dur, ControlDuration::WhileControlSource);
    }
}
