// File-level allow: This file converts ApiType enum (50+ variants from Java Forge)
// to Effect enum. Only a subset of API types are currently implemented - unimplemented
// types return None, which is intentional incremental porting behavior.
#![allow(clippy::wildcard_enum_match_arm)]
//! Convert parsed ability parameters to Effect objects
//!
//! This module bridges between ability_parser (tokenized parameters) and the Effect enum.

use super::ability_parser::{AbilityParams, ApiType};
use super::svar_parser::{parse_svar, ParsedSVar, StaticAbilityMode};
use crate::core::{CardId, Effect, PlayerId, TargetRef, TargetRestriction};
use std::collections::HashMap;

/// Convert ability parameters to an Effect
///
/// This replaces the unsafe substring matching in parse_effects() with
/// proper tokenization and validation.
///
/// # Errors
///
/// Returns None if:
/// - The API type is not yet supported
/// - Required parameters are missing
/// - Parameter values are invalid
///
/// # Example
///
/// ```ignore
/// let params = AbilityParams::parse("A:SP$ DealDamage | NumDmg$ 3")?;
/// let effect = params_to_effect(&params);
/// ```
pub fn params_to_effect(params: &AbilityParams) -> Option<Effect> {
    match params.api_type {
        ApiType::DealDamage => {
            // Extract damage amount from NumDmg$ parameter
            let amount = params.get_i32("NumDmg").ok()?;
            Some(Effect::DealDamage {
                target: TargetRef::None, // Placeholder - filled in at cast time
                amount,
            })
        }

        ApiType::Draw => {
            // Extract card count from NumCards$ parameter
            let count = params.get_u8("NumCards").ok()?;
            Some(Effect::DrawCards {
                player: PlayerId::new(0), // Placeholder - filled in at cast time
                count,
            })
        }

        ApiType::Destroy => {
            // Destroy effects target a permanent
            // Parse ValidTgts to determine what types can be targeted
            // Examples: "Artifact,Enchantment" for Disenchant, "Creature" for Terror
            let restriction = params
                .get("ValidTgts")
                .map(TargetRestriction::parse)
                .unwrap_or_else(TargetRestriction::any);

            Some(Effect::DestroyPermanent {
                target: CardId::new(0), // Placeholder - filled in at cast time
                restriction,
            })
        }

        ApiType::GainLife => {
            // Extract life amount from LifeAmount$ parameter
            let amount = params.get_i32("LifeAmount").ok()?;
            Some(Effect::GainLife {
                player: PlayerId::new(0), // Placeholder - filled in at cast time
                amount,
            })
        }

        ApiType::Pump => {
            let mut power_bonus = 0;
            let mut toughness_bonus = 0;

            // Extract power bonus (NumAtt$) - optional, defaults to 0
            if let Ok(att) = params.get_i32("NumAtt") {
                power_bonus = att;
            }

            // Extract toughness bonus (NumDef$) - optional, defaults to 0
            if let Ok(def) = params.get_i32("NumDef") {
                toughness_bonus = def;
            }

            // Only create effect if at least one bonus is non-zero
            if power_bonus != 0 || toughness_bonus != 0 {
                Some(Effect::PumpCreature {
                    target: CardId::new(0), // Placeholder - filled in at cast time
                    power_bonus,
                    toughness_bonus,
                })
            } else {
                None
            }
        }

        ApiType::Tap => {
            // Check for TapAll (mass tap) vs single target tap
            if params.contains_key("TapAll") {
                None // TapAll not yet supported
            } else {
                Some(Effect::TapPermanent {
                    target: CardId::new(0), // Placeholder
                })
            }
        }

        ApiType::Untap => {
            Some(Effect::UntapPermanent {
                target: CardId::new(0), // Placeholder
            })
        }

        ApiType::Mill => {
            let count = params.get_u8("NumCards").ok()?;
            Some(Effect::Mill {
                player: PlayerId::new(0), // Placeholder
                count,
            })
        }

        ApiType::Counter => {
            Some(Effect::CounterSpell {
                target: CardId::new(0), // Placeholder
            })
        }

        ApiType::ChangeZone => {
            // Check for exile effects: Origin$ Battlefield + Destination$ Exile
            if params.get("Origin") == Some("Battlefield") && params.get("Destination") == Some("Exile") {
                Some(Effect::ExilePermanent {
                    target: CardId::new(0), // Placeholder
                })
            }
            // Check for library search effects: Origin$ Library
            else if params.get("Origin") == Some("Library") {
                let destination = match params.get("Destination") {
                    Some("Battlefield") => crate::zones::Zone::Battlefield,
                    Some("Hand") => crate::zones::Zone::Hand,
                    Some("Graveyard") => crate::zones::Zone::Graveyard,
                    _ => crate::zones::Zone::Battlefield, // Default
                };

                let enters_tapped = params.get("Tapped") == Some("True");
                let card_type_filter = params.get("ChangeType").unwrap_or("Card").to_string();

                Some(Effect::SearchLibrary {
                    player: PlayerId::new(0), // Placeholder
                    card_type_filter,
                    destination,
                    enters_tapped,
                    shuffle: true, // Library searches always shuffle (MTG Rules 701.19b)
                })
            } else {
                None // Other ChangeZone variants not yet supported
            }
        }

        ApiType::Mana => {
            // Parse mana abilities: A:AB$ Mana | Cost$ T | Produced$ G
            let produced_str = params.get("Produced")?;

            // Parse the produced mana into a ManaCost
            // Simple cases: G, W, U, B, R, C (single color)
            // Complex cases: "Combo W U" (choice), "Any" (any color), "C C" (multiple colorless)
            // Special: "Chosen" means the card's chosen_color (for Thriving lands)
            use crate::core::ManaCost;

            // Check if "Chosen" is present in the produced string
            let produces_chosen_color = produced_str.contains("Chosen");

            let mana_cost = if produced_str == "Any" {
                // Any color - for now, default to colorless
                // TODO: Implement player choice for "Any"
                ManaCost::from_string("C")
            } else if produced_str.starts_with("Combo") {
                // Combo means choice between colors (e.g., "Combo B G" = {B} or {G})
                // Parse all listed colors and return them as a ManaCost with all colors set to 1
                // The cache will detect this as ManaProductionKind::Choice
                // "Chosen" is handled separately via produces_chosen_color flag
                let colors = produced_str.strip_prefix("Combo").unwrap_or("").trim();
                let mut mana = ManaCost::default();
                for color in colors.split_whitespace() {
                    match color {
                        "W" => mana.white = 1,
                        "U" => mana.blue = 1,
                        "B" => mana.black = 1,
                        "R" => mana.red = 1,
                        "G" => mana.green = 1,
                        "C" => mana.colorless = 1,
                        "Chosen" => {} // Handled by produces_chosen_color flag
                        _ => {}
                    }
                }
                mana
            } else {
                // Direct specification: "G", "C C", "W U", etc.
                ManaCost::from_string(produced_str)
            };

            // Check for Amount$ parameter (e.g., Amount$ 2 for Sol Ring)
            let amount = params.get("Amount").and_then(|s| s.parse::<u8>().ok()).unwrap_or(1);

            // Multiply mana by amount
            let final_mana = mana_cost.multiply(amount);

            Some(Effect::AddMana {
                player: PlayerId::new(0), // Placeholder - filled in when activated
                mana: final_mana,
                produces_chosen_color,
            })
        }

        ApiType::Balance => {
            // Balance effect: SP$ Balance | Valid$ Land/Creature | Zone$ Hand/Battlefield | SubAbility$ SvarName
            // Valid$ defaults to "Land" (most common use)
            // Zone$ defaults to "Battlefield" for permanents
            // SubAbility$ references an SVar for the next effect in the chain
            let card_type = params.get("Valid").unwrap_or("Land").to_string();
            let zone = params.get("Zone").unwrap_or("Battlefield").to_string();
            let sub_ability = params.get("SubAbility").map(|s| s.to_string());

            Some(Effect::Balance {
                card_type,
                zone,
                sub_ability,
            })
        }

        ApiType::PutCounter => {
            // PutCounter effect: AB$ PutCounter | Cost$ X | Defined$ Self | CounterType$ P1P1 | CounterNum$ 1
            // Example: Foggy Swamp Vinebender - Waterbend 5: Put a +1/+1 counter on this creature
            use crate::core::CounterType;

            // Parse counter type (e.g., "P1P1" -> +1/+1 counter)
            let counter_type_str = params.get("CounterType")?;
            let counter_type = CounterType::parse(counter_type_str)?;

            // Parse counter count (default to 1)
            let amount = params.get_u8("CounterNum").unwrap_or(1);

            Some(Effect::PutCounter {
                target: CardId::new(0), // Placeholder - filled in at activation time
                counter_type,
                amount,
            })
        }

        ApiType::RemoveCounter => {
            // RemoveCounter effect: DB$ RemoveCounter | ValidTgts$ Creature | CounterType$ Any | CounterNum$ 3 | UpTo$ True
            // Example: Heartless Act mode 2 - "Remove up to three counters from target creature"
            //
            // CounterType$ can be:
            // - "P1P1" for +1/+1 counters
            // - "M1M1" for -1/-1 counters
            // - "Any" to remove any counter type
            //
            // UpTo$ True means "up to N counters" (minimum 0), otherwise exactly N counters
            use crate::core::CounterType;

            // Parse counter type (e.g., "P1P1" -> +1/+1 counter, "Any" -> P1P1 as default for now)
            let counter_type_str = params.get("CounterType").unwrap_or("P1P1");
            let counter_type = if counter_type_str == "Any" {
                // "Any" means remove any counter type - for now default to P1P1
                // TODO(mtg-charm): Support "Any" counter type properly
                CounterType::P1P1
            } else {
                CounterType::parse(counter_type_str)?
            };

            // Parse counter count (default to 1)
            let amount = params.get_u8("CounterNum").unwrap_or(1);

            // Note: UpTo$ True is tracked for targeting validation but doesn't change
            // the effect structure (amount is the maximum that CAN be removed)

            Some(Effect::RemoveCounter {
                target: CardId::new(0), // Placeholder - filled in at targeting time
                counter_type,
                amount,
            })
        }

        ApiType::Animate => {
            // Animate effect: AB$ Animate | Defined$ Self | Power$ 5 | Toughness$ 2
            // Example: Flexible Waterbender - "This creature has base power and toughness 5/2 until end of turn"
            // Sets base P/T (counters and other bonuses are added on top)

            // Parse power and toughness
            let power = params.get_i32("Power").ok()?;
            let toughness = params.get_i32("Toughness").ok()?;

            Some(Effect::SetBasePowerToughness {
                target: CardId::new(0), // Placeholder - filled in at activation time
                power,
                toughness,
            })
        }

        ApiType::Airbend => {
            // Airbend effect: DB$ Airbend | ValidTgts$ Creature
            // Example: Aang, the Last Airbender - "Airbend target creature"
            // Effect: Exile target. While exiled, its owner may cast it for {2} rather than its mana cost.
            //
            // This creates a PersistentEffect (MayPlayFromExile) when resolved.
            // Target validation uses ValidTgts$ parameter.
            //
            // Note: The target is a placeholder (CardId::new(0)) - filled in at cast time
            // when the player chooses the actual target.
            Some(Effect::Airbend {
                target: CardId::new(0), // Placeholder - filled in at cast time
            })
        }

        ApiType::Earthbend => {
            // Earthbend effect: DB$ Earthbend | Num$ 8
            // Example: Avatar Kyoshi, Earthbender - "earthbend 8, then untap that land"
            // Effect: Target land becomes 0/0 creature with haste, put N +1/+1 counters.
            //
            // Parameters:
            // - Num$: Number of +1/+1 counters (default 1)
            // - ValidTgts$: Target restriction (implied: Land.YouCtrl)
            //
            // Note: The target is a placeholder (CardId::new(0)) - filled in at cast time.
            let num_counters = params.get_u8("Num").unwrap_or(1);
            Some(Effect::Earthbend {
                target: CardId::new(0), // Placeholder - filled in at cast time
                num_counters,
            })
        }

        ApiType::Effect => {
            // Effect ability: AB$ Effect | StaticAbilities$ X | RememberObjects$ Targeted
            // Creates a persistent effect that applies to remembered objects.
            //
            // This is a complex ability type in Java Forge - it creates a pseudo-card in the
            // command zone with the specified static abilities, triggers, etc.
            //
            // For now, we support a subset: StaticAbilities$ that grant "can't be blocked"
            // Examples: Deserter's Disciple - makes a creature unblockable this turn
            //
            // StaticAbilities$ is a reference to an SVar with the actual static ability definition.
            // Common patterns we support:
            // - Mode$ CantBlockBy | ValidAttacker$ Card.IsRemembered -> CantBeBlocked effect
            //
            // Without SVars context, we fall back to name-based heuristics.
            // Use params_to_effect_with_svars() for proper SVar-based detection.
            if params.contains_key("StaticAbilities") {
                let static_ability = params.get("StaticAbilities")?;

                // Fallback: check ability name for common patterns
                // - "Unblockable", "CantBeBlocked", etc.
                let static_lower = static_ability.to_lowercase();
                if static_lower.contains("unblock") || static_lower.contains("cantblock") {
                    Some(Effect::GrantCantBeBlocked {
                        target: CardId::new(0), // Placeholder - filled in at cast time
                    })
                } else {
                    // Other static abilities not yet supported
                    log::debug!(target: "effect_converter", "AB$ Effect with unsupported StaticAbility: {}", static_ability);
                    None
                }
            } else {
                // Effect without StaticAbilities (maybe has Triggers, ReplacementEffects, etc.)
                log::debug!(target: "effect_converter", "AB$ Effect without StaticAbilities not yet supported");
                None
            }
        }

        ApiType::CopyPermanent => {
            // CopyPermanent effect: DB$ CopyPermanent | ValidTgts$ Creature.YouCtrl | NonLegendary$ True | SetPower$ 4
            // Creates a token copy of an existing permanent with optional modifications.
            //
            // Parameters:
            // - ValidTgts$: Target restriction (YouCtrl, OppCtrl, etc.)
            // - NonLegendary$ True: Remove Legendary supertype from the copy
            // - SetPower$ N: Override power to N
            // - SetToughness$ N: Override toughness to N
            // - AddTypes$ Type1 & Type2: Add creature types (& separated)
            // - SetColor$ Color: Override color
            // - AddKeywords$ Keyword: Add keywords (comma separated)
            // - NumCopies$ N: Create N copies (default 1)
            //
            // Examples:
            // - Cackling Counterpart: simple copy of own creature (YouCtrl)
            // - Ember Island Production: copy with SetPower/SetToughness/AddTypes (YouCtrl or OppCtrl modes)

            // Parse target restriction from ValidTgts$ (e.g., Creature.YouCtrl, Creature.OppCtrl)
            let restriction = params
                .get("ValidTgts")
                .map(TargetRestriction::parse)
                .unwrap_or_else(TargetRestriction::any);

            let non_legendary = params.get("NonLegendary") == Some("True");
            let set_power = params.get_i32("SetPower").ok();
            let set_toughness = params.get_i32("SetToughness").ok();
            let num_copies = params.get_u8("NumCopies").unwrap_or(1);

            // Parse AddTypes$ - types are separated by " & "
            let add_types: Vec<String> = params
                .get("AddTypes")
                .map(|s| s.split(" & ").map(|t| t.trim().to_string()).collect())
                .unwrap_or_default();

            Some(Effect::CopyPermanent {
                target: CardId::new(0),       // Placeholder - filled in at cast time
                controller: PlayerId::new(0), // Placeholder - filled in at cast time
                non_legendary,
                set_power,
                set_toughness,
                add_types,
                num_copies,
                restriction,
            })
        }

        ApiType::Charm => {
            // Modal spell: A:SP$ Charm | Choices$ DBDestroy,DBDraw | CharmNum$ 1
            //
            // This requires SVar resolution to get the actual mode effects.
            // Without SVars, we can only parse the metadata (num modes, can repeat).
            // Use params_to_charm_effect_with_svars() for full parsing with modes.
            //
            // Parameters:
            // - Choices$: comma-separated SVar names for each mode
            // - CharmNum$: number of modes to choose (default 1)
            // - MinCharmNum$: minimum modes required (default = CharmNum$)
            // - CanRepeatModes$: if present, same mode can be chosen twice

            let choices_str = params.get("Choices")?;
            let choice_names: Vec<&str> = choices_str.split(',').map(|s| s.trim()).collect();

            // Parse CharmNum$ - how many to choose (default 1)
            let num_to_choose = params.get_u8("CharmNum").unwrap_or(1);

            // Parse MinCharmNum$ - minimum to choose (default = num_to_choose)
            let min_to_choose = params.get_u8("MinCharmNum").unwrap_or(num_to_choose);

            // CanRepeatModes$ - can choose same mode twice
            let can_repeat_modes = params.contains_key("CanRepeatModes");

            // Without SVar resolution, create placeholder modes with just names
            // The actual effects will be filled in by params_to_charm_effect_with_svars
            let modes: smallvec::SmallVec<[crate::core::ModalMode; 4]> = choice_names
                .iter()
                .map(|name| crate::core::ModalMode {
                    effect: Box::new(Effect::DrawCards {
                        player: PlayerId::new(0),
                        count: 0,
                    }), // Placeholder
                    description: format!("Mode: {}", name),
                    svar_name: name.to_string(),
                })
                .collect();

            log::debug!(
                target: "effect_converter",
                "Charm with {} modes (choose {}, min {}, repeat={}): {:?}",
                modes.len(), num_to_choose, min_to_choose, can_repeat_modes, choice_names
            );

            Some(Effect::ModalChoice {
                modes,
                num_to_choose,
                min_to_choose,
                can_repeat_modes,
            })
        }

        // All other API types not yet implemented
        _ => None,
    }
}

/// Convert Charm ability parameters to a ModalChoice Effect with full SVar resolution.
///
/// This resolves each mode's SVar to get the actual effect and description.
/// Use this when you have access to the card's SVars.
///
/// # Arguments
///
/// * `params` - The parsed ability parameters (must be ApiType::Charm)
/// * `svars` - The card's SVar definitions (name -> body)
///
/// # Example
///
/// ```ignore
/// // Card has: A:SP$ Charm | Choices$ Destroy,Remove
/// // And SVar:Destroy:DB$ Destroy | ValidTgts$ Creature.!HasCounters | SpellDescription$ Destroy...
/// // And SVar:Remove:DB$ RemoveCounter | ...
/// let params = AbilityParams::parse("A:SP$ Charm | Choices$ Destroy,Remove")?;
/// let effect = params_to_charm_effect_with_svars(&params, &card.svars);
/// ```
pub fn params_to_charm_effect_with_svars(params: &AbilityParams, svars: &HashMap<String, String>) -> Option<Effect> {
    if params.api_type != ApiType::Charm {
        return None;
    }

    let choices_str = params.get("Choices")?;
    let choice_names: Vec<&str> = choices_str.split(',').map(|s| s.trim()).collect();

    let num_to_choose = params.get_u8("CharmNum").unwrap_or(1);
    let min_to_choose = params.get_u8("MinCharmNum").unwrap_or(num_to_choose);
    let can_repeat_modes = params.contains_key("CanRepeatModes");

    let mut modes: smallvec::SmallVec<[crate::core::ModalMode; 4]> = smallvec::SmallVec::new();

    for name in choice_names {
        // Look up the SVar for this mode
        if let Some(svar_body) = svars.get(name) {
            // Parse the SVar as an ability
            if let Ok(mode_params) = AbilityParams::parse(&format!("A:{}", svar_body)) {
                // Convert to effect
                let effect = params_to_effect(&mode_params).unwrap_or(Effect::DrawCards {
                    player: PlayerId::new(0),
                    count: 0,
                });

                // Extract description from SpellDescription$ if available
                let description = mode_params
                    .get("SpellDescription")
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("Mode: {}", name));

                modes.push(crate::core::ModalMode {
                    effect: Box::new(effect),
                    description,
                    svar_name: name.to_string(),
                });
            } else {
                log::warn!(
                    target: "effect_converter",
                    "Failed to parse Charm mode SVar '{}': {}",
                    name, svar_body
                );
            }
        } else {
            log::warn!(
                target: "effect_converter",
                "Charm mode SVar '{}' not found in card SVars",
                name
            );
        }
    }

    if modes.is_empty() {
        log::warn!(target: "effect_converter", "Charm has no valid modes after SVar resolution");
        return None;
    }

    Some(Effect::ModalChoice {
        modes,
        num_to_choose,
        min_to_choose,
        can_repeat_modes,
    })
}

/// Convert ability parameters to an Effect, with SVar resolution.
///
/// This is the preferred method when you have access to the card's SVars.
/// It properly resolves StaticAbilities$ references to their SVar definitions
/// and determines the effect type based on Mode$ rather than name heuristics.
///
/// # Arguments
///
/// * `params` - The parsed ability parameters
/// * `svars` - The card's SVar definitions (name -> body)
///
/// # Example
///
/// ```ignore
/// // Card has: A:AB$ Effect | StaticAbilities$ Unblockable | ...
/// // And SVar: SVar:Unblockable:Mode$ CantBlockBy | ValidAttacker$ Card.IsRemembered
/// let params = AbilityParams::parse("A:AB$ Effect | StaticAbilities$ Unblockable")?;
/// let effect = params_to_effect_with_svars(&params, &card.svars);
/// // Returns Effect::GrantCantBeBlocked because Mode$ CantBlockBy
/// ```
pub fn params_to_effect_with_svars(params: &AbilityParams, svars: &HashMap<String, String>) -> Option<Effect> {
    // For ApiType::Effect, we can do proper SVar resolution
    if params.api_type == ApiType::Effect {
        if let Some(static_ability_name) = params.get("StaticAbilities") {
            // Look up the SVar definition
            if let Some(svar_body) = svars.get(static_ability_name) {
                let parsed = parse_svar(svar_body);

                // Check if this is a static ability and convert to appropriate effect
                if let ParsedSVar::StaticAbility(def) = parsed {
                    match def.mode {
                        StaticAbilityMode::CantBlockBy => {
                            // Mode$ CantBlockBy creates a GrantCantBeBlocked effect
                            // Example: SVar:Unblockable:Mode$ CantBlockBy | ValidAttacker$ Card.IsRemembered
                            return Some(Effect::GrantCantBeBlocked {
                                target: CardId::new(0), // Placeholder - filled in at cast time
                            });
                        }
                        StaticAbilityMode::CantAttack => {
                            // Mode$ CantAttack creates a "can't attack" restriction
                            // Example: Pacifism-style effects
                            // TODO(mtg-20): Add Effect::GrantCantAttack variant
                            log::debug!(
                                target: "effect_converter",
                                "Mode$ CantAttack not yet implemented (SVar: {})",
                                static_ability_name
                            );
                        }
                        StaticAbilityMode::CantBlock => {
                            // Mode$ CantBlock creates a "can't block" restriction
                            // Example: Fear/Intimidate-style effects
                            // TODO(mtg-20): Add Effect::GrantCantBlock variant
                            log::debug!(
                                target: "effect_converter",
                                "Mode$ CantBlock not yet implemented (SVar: {})",
                                static_ability_name
                            );
                        }
                        StaticAbilityMode::Continuous => {
                            // Mode$ Continuous creates persistent continuous effects
                            // Common params:
                            // - MayPlay$ True: Permission to play from non-standard zone
                            // - AddPower$/AddToughness$: P/T modifications
                            // - AddKeyword$: Grant keywords
                            //
                            // These are usually created as StaticAbility objects during parsing,
                            // not through AB$ Effect. When they appear in AB$ Effect context,
                            // it's typically for temporary/until-end-of-turn effects.
                            if def.params.get("MayPlay") == Some(&"True".to_string()) {
                                // MayPlay effects grant permission to play cards from a zone
                                // This is commonly used by Yawgmoth's Will, Future Sight effects
                                // TODO(mtg-20): Add Effect::GrantMayPlay variant
                                log::debug!(
                                    target: "effect_converter",
                                    "Mode$ Continuous with MayPlay$ True not yet implemented (SVar: {})",
                                    static_ability_name
                                );
                            } else {
                                log::debug!(
                                    target: "effect_converter",
                                    "Mode$ Continuous with unsupported params (SVar: {})",
                                    static_ability_name
                                );
                            }
                        }
                        // Trigger modes are handled by parse_triggers(), not effect conversion
                        StaticAbilityMode::Attacks
                        | StaticAbilityMode::ChangesZone
                        | StaticAbilityMode::Phase
                        | StaticAbilityMode::SpellCast
                        | StaticAbilityMode::LandPlayed
                        | StaticAbilityMode::Sacrificed => {
                            // These are trigger modes, not static ability modes
                            // They should be processed by the trigger parser instead
                            log::debug!(
                                target: "effect_converter",
                                "Mode$ {:?} is a trigger mode, not handled by effect conversion (SVar: {})",
                                def.mode, static_ability_name
                            );
                        }
                        StaticAbilityMode::Unknown(ref mode_name) => {
                            log::debug!(
                                target: "effect_converter",
                                "Unknown Mode$ '{}' in SVar: {}",
                                mode_name, static_ability_name
                            );
                        }
                    }
                }
            }
        }
        // Fall back to name-based detection if SVar lookup fails
        return params_to_effect(params);
    }

    // For all other types, delegate to the base function
    params_to_effect(params)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_deal_damage() {
        let params = AbilityParams::parse("A:SP$ DealDamage | NumDmg$ 3").unwrap();
        let effect = params_to_effect(&params).unwrap();

        match effect {
            Effect::DealDamage { target, amount } => {
                assert_eq!(amount, 3);
                assert!(matches!(target, TargetRef::None));
            }
            _ => panic!("Expected DealDamage effect"),
        }
    }

    #[test]
    fn test_convert_draw() {
        let params = AbilityParams::parse("A:SP$ Draw | NumCards$ 2").unwrap();
        let effect = params_to_effect(&params).unwrap();

        match effect {
            Effect::DrawCards { player: _, count } => {
                assert_eq!(count, 2);
            }
            _ => panic!("Expected DrawCards effect"),
        }
    }

    #[test]
    fn test_convert_pump() {
        let params = AbilityParams::parse("A:SP$ Pump | NumAtt$ +3 | NumDef$ +2").unwrap();
        let effect = params_to_effect(&params).unwrap();

        match effect {
            Effect::PumpCreature {
                target: _,
                power_bonus,
                toughness_bonus,
            } => {
                assert_eq!(power_bonus, 3);
                assert_eq!(toughness_bonus, 2);
            }
            _ => panic!("Expected PumpCreature effect"),
        }
    }

    #[test]
    fn test_convert_missing_parameter() {
        // DealDamage without NumDmg$ should return None
        let params = AbilityParams::parse("A:SP$ DealDamage").unwrap();
        let effect = params_to_effect(&params);

        assert!(
            effect.is_none(),
            "Should return None when required parameter is missing"
        );
    }

    #[test]
    fn test_convert_unsupported_api_type() {
        // Unknown API types should return None
        let params = AbilityParams::parse("A:SP$ UnsupportedAbility | Foo$ Bar").unwrap();
        let effect = params_to_effect(&params);

        assert!(effect.is_none(), "Should return None for unsupported API types");
    }

    #[test]
    fn test_convert_mana_ability() {
        // Basic mana ability: tap to add one green mana
        let params = AbilityParams::parse("A:AB$ Mana | Cost$ T | Produced$ G").unwrap();
        let effect = params_to_effect(&params).unwrap();

        match effect {
            Effect::AddMana { player: _, mana, .. } => {
                // Verify the mana cost represents {G}
                assert_eq!(mana.green, 1);
                assert_eq!(mana.colorless, 0);
            }
            _ => panic!("Expected AddMana effect"),
        }
    }

    #[test]
    fn test_convert_mana_ability_with_amount() {
        // Sol Ring: tap to add two colorless mana
        let params = AbilityParams::parse("A:AB$ Mana | Cost$ T | Produced$ C | Amount$ 2").unwrap();
        let effect = params_to_effect(&params).unwrap();

        match effect {
            Effect::AddMana { player: _, mana, .. } => {
                assert_eq!(mana.colorless, 2);
            }
            _ => panic!("Expected AddMana effect"),
        }
    }

    #[test]
    fn test_convert_put_counter() {
        use crate::core::CounterType;

        // Foggy Swamp Vinebender: Waterbend 5 to put a +1/+1 counter on this creature
        let params =
            AbilityParams::parse("A:AB$ PutCounter | Cost$ Waterbend<5> | CounterType$ P1P1 | CounterNum$ 1").unwrap();
        let effect = params_to_effect(&params).unwrap();

        match effect {
            Effect::PutCounter {
                target: _,
                counter_type,
                amount,
            } => {
                assert_eq!(counter_type, CounterType::P1P1);
                assert_eq!(amount, 1);
            }
            _ => panic!("Expected PutCounter effect"),
        }
    }

    #[test]
    fn test_convert_airbend() {
        // Aang, the Last Airbender: ETB airbend nonland permanent
        let params = AbilityParams::parse("A:DB$ Airbend | ValidTgts$ Creature").unwrap();
        let effect = params_to_effect(&params).unwrap();

        match effect {
            Effect::Airbend { target: _ } => {
                // Effect parsed correctly - target is placeholder CardId(0)
            }
            _ => panic!("Expected Airbend effect"),
        }
    }

    #[test]
    fn test_convert_effect_with_svar_cantblockby() {
        // Deserter's Disciple pattern:
        // A:AB$ Effect | StaticAbilities$ Unblockable | RememberObjects$ Targeted
        // SVar:Unblockable:Mode$ CantBlockBy | ValidAttacker$ Card.IsRemembered
        let params =
            AbilityParams::parse("A:AB$ Effect | StaticAbilities$ Unblockable | RememberObjects$ Targeted").unwrap();

        let mut svars = HashMap::new();
        svars.insert(
            "Unblockable".to_string(),
            "Mode$ CantBlockBy | ValidAttacker$ Card.IsRemembered".to_string(),
        );

        let effect = params_to_effect_with_svars(&params, &svars).unwrap();

        match effect {
            Effect::GrantCantBeBlocked { target: _ } => {
                // Correctly identified CantBlockBy from SVar Mode$
            }
            _ => panic!("Expected GrantCantBeBlocked effect"),
        }
    }

    #[test]
    fn test_convert_effect_fallback_no_svar() {
        // When SVar is missing, falls back to name-based heuristic
        let params =
            AbilityParams::parse("A:AB$ Effect | StaticAbilities$ Unblockable | RememberObjects$ Targeted").unwrap();

        let svars = HashMap::new(); // Empty - no SVars

        let effect = params_to_effect_with_svars(&params, &svars).unwrap();

        match effect {
            Effect::GrantCantBeBlocked { target: _ } => {
                // Falls back to name-based detection ("Unblockable" contains "unblock")
            }
            _ => panic!("Expected GrantCantBeBlocked effect from name fallback"),
        }
    }

    #[test]
    fn test_convert_effect_continuous_mayplay_returns_none() {
        // Mode$ Continuous with MayPlay$ True is not yet implemented
        // It should return None (falling through to name-based check)
        let params =
            AbilityParams::parse("A:AB$ Effect | StaticAbilities$ MayPlayGraveyard | RememberObjects$ Targeted")
                .unwrap();

        let mut svars = HashMap::new();
        svars.insert(
            "MayPlayGraveyard".to_string(),
            "Mode$ Continuous | Affected$ Card.YouCtrl | AffectedZone$ Graveyard | MayPlay$ True".to_string(),
        );

        // Should return None because:
        // 1. Mode$ Continuous with MayPlay$ True is not implemented yet
        // 2. Name "MayPlayGraveyard" doesn't match fallback pattern
        let effect = params_to_effect_with_svars(&params, &svars);
        assert!(effect.is_none(), "Mode$ Continuous MayPlay not yet implemented");
    }

    #[test]
    fn test_convert_effect_trigger_modes_return_none() {
        // Trigger modes (Attacks, ChangesZone, etc.) should not be handled by effect conversion
        // They are handled by parse_triggers() instead
        let params =
            AbilityParams::parse("A:AB$ Effect | StaticAbilities$ AttackTrigger | RememberObjects$ Targeted").unwrap();

        let mut svars = HashMap::new();
        svars.insert(
            "AttackTrigger".to_string(),
            "Mode$ Attacks | ValidCard$ Card.Self | Execute$ TrigPump".to_string(),
        );

        // Should return None (not handled as an effect)
        let effect = params_to_effect_with_svars(&params, &svars);
        assert!(effect.is_none(), "Trigger modes should not produce effects");
    }

    #[test]
    fn test_parse_svar_all_modes() {
        // Verify all StaticAbilityMode variants can be parsed correctly
        use crate::loader::svar_parser::{parse_svar, ParsedSVar, StaticAbilityMode};

        let test_cases = vec![
            (
                "Mode$ CantBlockBy | ValidAttacker$ Card.Self",
                StaticAbilityMode::CantBlockBy,
            ),
            (
                "Mode$ CantAttack | Affected$ Creature.EnchantedBy",
                StaticAbilityMode::CantAttack,
            ),
            (
                "Mode$ CantBlock | Affected$ Creature.Self",
                StaticAbilityMode::CantBlock,
            ),
            ("Mode$ Continuous | MayPlay$ True", StaticAbilityMode::Continuous),
            ("Mode$ Attacks | ValidCard$ Card.Self", StaticAbilityMode::Attacks),
            (
                "Mode$ ChangesZone | Origin$ Any | Destination$ Battlefield",
                StaticAbilityMode::ChangesZone,
            ),
            ("Mode$ Phase | Phase$ Upkeep", StaticAbilityMode::Phase),
            (
                "Mode$ SpellCast | ValidCard$ Instant,Sorcery",
                StaticAbilityMode::SpellCast,
            ),
            ("Mode$ LandPlayed | ValidCard$ Land", StaticAbilityMode::LandPlayed),
            ("Mode$ Sacrificed | ValidCard$ Creature", StaticAbilityMode::Sacrificed),
        ];

        for (svar_body, expected_mode) in test_cases {
            match parse_svar(svar_body) {
                ParsedSVar::StaticAbility(def) => {
                    assert_eq!(def.mode, expected_mode, "Failed to parse mode from: {}", svar_body);
                }
                other => panic!("Expected StaticAbility, got {:?} for: {}", other, svar_body),
            }
        }
    }

    #[test]
    fn test_params_to_charm_effect_with_svars_heartless_act() {
        // Test parsing Heartless Act modal spell:
        // Choose one —
        // • Destroy target creature with no counters on it.
        // • Remove up to three counters from target creature.

        let params = AbilityParams::parse("A:SP$ Charm | Choices$ Destroy,Remove").unwrap();

        let mut svars = HashMap::new();
        svars.insert(
            "Destroy".to_string(),
            "DB$ Destroy | ValidTgts$ Creature.!HasCounters | TgtPrompt$ Select target creature with no counters on it | SpellDescription$ Destroy target creature with no counters on it.".to_string(),
        );
        svars.insert(
            "Remove".to_string(),
            "DB$ RemoveCounter | ValidTgts$ Creature | CounterType$ Any | CounterNum$ 3 | UpTo$ True | SpellDescription$ Remove up to three counters from target creature.".to_string(),
        );

        let effect = params_to_charm_effect_with_svars(&params, &svars);
        assert!(effect.is_some(), "Should parse Charm effect with SVars");

        match effect.unwrap() {
            Effect::ModalChoice {
                modes,
                num_to_choose,
                min_to_choose,
                can_repeat_modes,
            } => {
                assert_eq!(modes.len(), 2, "Should have 2 modes");
                assert_eq!(num_to_choose, 1, "Should choose 1 mode");
                assert_eq!(min_to_choose, 1, "Minimum 1 mode");
                assert!(!can_repeat_modes, "Cannot repeat modes");

                // Check first mode (Destroy)
                assert_eq!(modes[0].svar_name, "Destroy");
                assert!(
                    modes[0].description.contains("Destroy"),
                    "First mode description should mention Destroy"
                );
                assert!(
                    matches!(*modes[0].effect, Effect::DestroyPermanent { .. }),
                    "First mode should be DestroyPermanent"
                );

                // Check second mode (Remove) - RemoveCounter is now implemented
                assert_eq!(modes[1].svar_name, "Remove");
                assert!(
                    modes[1].description.contains("Remove"),
                    "Second mode description should mention Remove"
                );
                assert!(
                    matches!(*modes[1].effect, Effect::RemoveCounter { amount: 3, .. }),
                    "Second mode should be RemoveCounter with amount 3, got: {:?}",
                    modes[1].effect
                );
            }
            _ => panic!("Expected ModalChoice effect"),
        }
    }

    #[test]
    fn test_convert_remove_counter() {
        use crate::core::CounterType;

        // Heartless Act mode 2: Remove up to three counters from target creature
        let params = AbilityParams::parse(
            "A:DB$ RemoveCounter | ValidTgts$ Creature | CounterType$ Any | CounterNum$ 3 | UpTo$ True",
        )
        .unwrap();
        let effect = params_to_effect(&params).unwrap();

        match effect {
            Effect::RemoveCounter {
                target: _,
                counter_type,
                amount,
            } => {
                // "Any" counter type defaults to P1P1 for now
                assert_eq!(counter_type, CounterType::P1P1);
                assert_eq!(amount, 3);
            }
            _ => panic!("Expected RemoveCounter effect, got: {:?}", effect),
        }
    }

    #[test]
    fn test_convert_remove_counter_specific_type() {
        use crate::core::CounterType;

        // Remove specific counter type (+1/+1)
        let params =
            AbilityParams::parse("A:DB$ RemoveCounter | ValidTgts$ Creature | CounterType$ P1P1 | CounterNum$ 1")
                .unwrap();
        let effect = params_to_effect(&params).unwrap();

        match effect {
            Effect::RemoveCounter {
                target: _,
                counter_type,
                amount,
            } => {
                assert_eq!(counter_type, CounterType::P1P1);
                assert_eq!(amount, 1);
            }
            _ => panic!("Expected RemoveCounter effect"),
        }
    }

    #[test]
    fn test_charm_with_multiple_modes_to_choose() {
        // Test a modal spell where you choose 2 modes (like Cryptic Command)
        let params = AbilityParams::parse("A:SP$ Charm | Choices$ Mode1,Mode2,Mode3 | CharmNum$ 2").unwrap();

        let mut svars = HashMap::new();
        svars.insert(
            "Mode1".to_string(),
            "DB$ Draw | NumCards$ 1 | SpellDescription$ Draw a card.".to_string(),
        );
        svars.insert(
            "Mode2".to_string(),
            "DB$ Tap | ValidTgts$ Creature | SpellDescription$ Tap target creature.".to_string(),
        );
        svars.insert(
            "Mode3".to_string(),
            "DB$ DealDamage | NumDmg$ 2 | SpellDescription$ Deal 2 damage.".to_string(),
        );

        let effect = params_to_charm_effect_with_svars(&params, &svars);
        assert!(effect.is_some(), "Should parse Charm effect");

        match effect.unwrap() {
            Effect::ModalChoice {
                modes,
                num_to_choose,
                min_to_choose,
                ..
            } => {
                assert_eq!(modes.len(), 3, "Should have 3 modes");
                assert_eq!(num_to_choose, 2, "Should choose 2 modes");
                assert_eq!(min_to_choose, 2, "Minimum 2 modes");

                // Verify mode 1 is DrawCards
                assert!(
                    matches!(*modes[0].effect, Effect::DrawCards { count: 1, .. }),
                    "Mode 1 should be DrawCards"
                );
            }
            _ => panic!("Expected ModalChoice effect"),
        }
    }

    #[test]
    fn test_charm_with_can_repeat_modes() {
        // Test modal spell that can repeat modes (like Prismari Command)
        let params =
            AbilityParams::parse("A:SP$ Charm | Choices$ Mode1,Mode2 | CharmNum$ 2 | CanRepeatModes$ True").unwrap();

        let mut svars = HashMap::new();
        svars.insert(
            "Mode1".to_string(),
            "DB$ DealDamage | NumDmg$ 2 | SpellDescription$ Deal 2 damage.".to_string(),
        );
        svars.insert(
            "Mode2".to_string(),
            "DB$ Draw | NumCards$ 1 | SpellDescription$ Draw a card.".to_string(),
        );

        let effect = params_to_charm_effect_with_svars(&params, &svars);
        assert!(effect.is_some(), "Should parse Charm effect with repeatable modes");

        match effect.unwrap() {
            Effect::ModalChoice { can_repeat_modes, .. } => {
                assert!(can_repeat_modes, "Should allow repeating modes");
            }
            _ => panic!("Expected ModalChoice effect"),
        }
    }

    #[test]
    fn test_convert_copy_permanent_simple() {
        // Cackling Counterpart: simple copy of own creature
        let params = AbilityParams::parse(
            "A:SP$ CopyPermanent | ValidTgts$ Creature.YouCtrl | TgtPrompt$ Select target creature you control",
        )
        .unwrap();
        let effect = params_to_effect(&params).unwrap();

        match effect {
            Effect::CopyPermanent {
                target: _,
                controller: _,
                non_legendary,
                set_power,
                set_toughness,
                add_types,
                num_copies,
                restriction,
            } => {
                assert!(!non_legendary, "Simple copy should not remove legendary");
                assert!(set_power.is_none(), "No power override");
                assert!(set_toughness.is_none(), "No toughness override");
                assert!(add_types.is_empty(), "No added types");
                assert_eq!(num_copies, 1, "Default to 1 copy");
                // Default restriction should allow any creature
                assert!(restriction.types.is_empty() || restriction.types.contains(&crate::core::TargetType::Creature));
            }
            _ => panic!("Expected CopyPermanent effect"),
        }
    }

    #[test]
    fn test_convert_copy_permanent_with_modifications() {
        // Ember Island Production mode 1: copy with SetPower, SetToughness, AddTypes
        let params = AbilityParams::parse(
            "A:DB$ CopyPermanent | ValidTgts$ Creature.YouCtrl | NonLegendary$ True | SetPower$ 4 | SetToughness$ 4 | AddTypes$ Hero"
        ).unwrap();
        let effect = params_to_effect(&params).unwrap();

        match effect {
            Effect::CopyPermanent {
                target: _,
                controller: _,
                non_legendary,
                set_power,
                set_toughness,
                add_types,
                num_copies,
                restriction,
            } => {
                assert!(non_legendary, "Should remove legendary");
                assert_eq!(set_power, Some(4), "Power override to 4");
                assert_eq!(set_toughness, Some(4), "Toughness override to 4");
                assert_eq!(add_types, vec!["Hero".to_string()], "Should add Hero type");
                assert_eq!(num_copies, 1, "Default to 1 copy");
                // Should have YouCtrl controller restriction
                assert_eq!(
                    restriction.controller,
                    crate::core::ControllerRestriction::YouCtrl,
                    "Should have YouCtrl restriction"
                );
            }
            _ => panic!("Expected CopyPermanent effect"),
        }
    }

    #[test]
    fn test_convert_copy_permanent_multiple_types() {
        // Test parsing AddTypes$ with multiple types separated by " & "
        let params =
            AbilityParams::parse("A:DB$ CopyPermanent | ValidTgts$ Creature | AddTypes$ Warrior & Soldier").unwrap();
        let effect = params_to_effect(&params).unwrap();

        match effect {
            Effect::CopyPermanent { add_types, .. } => {
                assert_eq!(add_types, vec!["Warrior".to_string(), "Soldier".to_string()]);
            }
            _ => panic!("Expected CopyPermanent effect"),
        }
    }

    #[test]
    fn test_convert_copy_permanent_with_num_copies() {
        // Test NumCopies$ parameter
        let params = AbilityParams::parse("A:DB$ CopyPermanent | ValidTgts$ Creature | NumCopies$ 3").unwrap();
        let effect = params_to_effect(&params).unwrap();

        match effect {
            Effect::CopyPermanent { num_copies, .. } => {
                assert_eq!(num_copies, 3, "Should create 3 copies");
            }
            _ => panic!("Expected CopyPermanent effect"),
        }
    }

    #[test]
    fn test_charm_with_copy_permanent_ember_island() {
        // Test Ember Island Production: Charm with CopyPermanent modes
        let params = AbilityParams::parse("A:SP$ Charm | Choices$ DBCopy1,DBCopy2").unwrap();

        let mut svars = HashMap::new();
        svars.insert(
            "DBCopy1".to_string(),
            "DB$ CopyPermanent | ValidTgts$ Creature.YouCtrl | TgtPrompt$ Select target creature you control | NonLegendary$ True | SetPower$ 4 | SetToughness$ 4 | AddTypes$ Hero | SpellDescription$ Create a nonlegendary token that's a copy of target creature you control, except it's a 4/4 Hero.".to_string(),
        );
        svars.insert(
            "DBCopy2".to_string(),
            "DB$ CopyPermanent | ValidTgts$ Creature.OppCtrl | TgtPrompt$ Select target creature an opponent controls | NonLegendary$ True | SetPower$ 2 | SetToughness$ 2 | AddTypes$ Coward | SpellDescription$ Create a nonlegendary token that's a copy of target creature an opponent controls, except it's a 2/2 Coward.".to_string(),
        );

        let effect = params_to_charm_effect_with_svars(&params, &svars);
        assert!(effect.is_some(), "Should parse Ember Island Production charm");

        match effect.unwrap() {
            Effect::ModalChoice { modes, .. } => {
                assert_eq!(modes.len(), 2, "Should have 2 modes");

                // Mode 1: Copy as 4/4 Hero
                match &*modes[0].effect {
                    Effect::CopyPermanent {
                        non_legendary,
                        set_power,
                        set_toughness,
                        add_types,
                        ..
                    } => {
                        assert!(*non_legendary);
                        assert_eq!(*set_power, Some(4));
                        assert_eq!(*set_toughness, Some(4));
                        assert_eq!(add_types, &vec!["Hero".to_string()]);
                    }
                    _ => panic!("Mode 1 should be CopyPermanent"),
                }

                // Mode 2: Copy as 2/2 Coward
                match &*modes[1].effect {
                    Effect::CopyPermanent {
                        non_legendary,
                        set_power,
                        set_toughness,
                        add_types,
                        ..
                    } => {
                        assert!(*non_legendary);
                        assert_eq!(*set_power, Some(2));
                        assert_eq!(*set_toughness, Some(2));
                        assert_eq!(add_types, &vec!["Coward".to_string()]);
                    }
                    _ => panic!("Mode 2 should be CopyPermanent"),
                }
            }
            _ => panic!("Expected ModalChoice effect"),
        }
    }
}
