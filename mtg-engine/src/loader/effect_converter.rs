//! Convert parsed ability parameters to Effect objects
//!
//! This module bridges between ability_parser (tokenized parameters) and the Effect enum.

use super::ability_parser::{AbilityParams, ApiType};
use crate::core::{CardId, Effect, PlayerId, TargetRef};

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
            Some(Effect::DestroyPermanent {
                target: CardId::new(0), // Placeholder - filled in at cast time
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

        // All other API types not yet implemented
        _ => None,
    }
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
}
