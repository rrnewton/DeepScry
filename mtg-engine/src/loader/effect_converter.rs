//! Convert parsed ability parameters to Effect objects
//!
//! This module bridges between ability_parser (tokenized parameters) and the Effect enum.

use super::ability_parser::{AbilityParams, ApiType};
use super::svar_parser::{parse_svar, ParsedSVar, StaticAbilityMode};
use crate::core::{CardId, Effect, Keyword, PlayerId, TargetRef, TargetRestriction};
use smallvec::SmallVec;
use std::collections::HashMap;

/// Extract the source color of a Circle-of-Protection-style `ChooseSource`
/// ability from its tokenized `Choices$ Card.<Color>Source` filter.
///
/// The `Choices$` value is a comma-separated list of card filters; for the
/// Circles of Protection it is exactly `Card.<Color>Source` (e.g.
/// `Card.RedSource`). We split on `.` (NOT substring-match) and read the
/// `<Color>Source` qualifier, mapping the colour word to a [`Color`]. Returns
/// `None` for any other `ChooseSource` shape (Martyr's Cause' `Card,Emblem`,
/// colourless choosers, …) so only the CoP family is converted here.
fn choose_source_color(params: &AbilityParams) -> Option<crate::core::Color> {
    use crate::core::Color;
    let choices = params.get("Choices")?;
    // CoP uses a single `Card.<Color>Source` choice; bail on multi-filter lists.
    if choices.contains(',') {
        return None;
    }
    // Tokenize `Card.<qualifier>` and read the qualifier (e.g. "RedSource").
    let qualifier = choices.split('.').nth(1)?;
    let color_word = qualifier.strip_suffix("Source")?;
    match color_word {
        "White" => Some(Color::White),
        "Blue" => Some(Color::Blue),
        "Black" => Some(Color::Black),
        "Red" => Some(Color::Red),
        "Green" => Some(Color::Green),
        _ => None,
    }
}

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
///
/// Note: Wildcard is intentional - ApiType enum has 50+ variants from Java Forge.
/// Only implemented types return Some; unimplemented types return None for incremental porting.
#[allow(clippy::wildcard_enum_match_arm)]
pub fn params_to_effect(params: &AbilityParams) -> Option<Effect> {
    match params.api_type {
        ApiType::DealDamage => {
            // Check if Defined$ specifies a player target (e.g., City of Brass "Defined$ You")
            let target = match params.get("Defined") {
                Some("You") => {
                    // "You" = the controller. Use PlayerId(0) as placeholder;
                    // resolve_effect_placeholder() maps this to the actual controller at runtime.
                    TargetRef::Player(crate::core::PlayerId::new(0))
                }
                _ => TargetRef::None, // Placeholder - filled in at cast time
            };
            // Extract damage amount from NumDmg$ parameter
            // If the value is "X" referencing SVar X = Count$xPaid, use XPaid variant
            if let Ok(amount) = params.get_i32("NumDmg") {
                Some(Effect::DealDamage { target, amount })
            } else if params.get("NumDmg") == Some("X") {
                // DivideEvenly$ RoundedDown (Fireball, CR 601.2d): the X damage is
                // split floor(X/N) among the N chosen targets at resolution.
                let divide = match params.get("DivideEvenly") {
                    Some("RoundedDown") => crate::core::DamageDivision::EvenlyRoundedDown,
                    _ => crate::core::DamageDivision::None,
                };
                Some(Effect::DealDamageXPaid { target, divide })
            } else {
                None
            }
        }

        ApiType::EachDamage => {
            // Multiple creatures deal damage to a single target
            // Example: DB$ EachDamage | DefinedDamagers$ ParentTarget | ValidTgts$ Creature.OppCtrl | NumDmg$ Count$CardPower
            //
            // DefinedDamagers$ ParentTarget = damagers come from parent ability's targets
            // NumDmg$ Count$CardPower = each damager deals damage equal to its power
            //
            // At parse time:
            // - damagers is empty (signals "use parent targets" at resolution)
            // - receiver is placeholder CardId::new(0) (filled at resolution)
            // At spell resolution, resolve_effect_target fills these from chosen_targets

            // Parse damage source: Count$CardPower means use creature's power
            let num_dmg = params.get("NumDmg").unwrap_or("0");
            let use_card_power = num_dmg.contains("CardPower");
            let fixed_damage = if use_card_power {
                0
            } else {
                num_dmg.parse::<i32>().unwrap_or(0)
            };

            Some(Effect::EachDamage {
                damagers: smallvec::SmallVec::new(), // Empty = use parent targets
                receiver: CardId::new(0),            // Placeholder = fill at resolution
                use_card_power,
                fixed_damage,
            })
        }

        ApiType::Draw => {
            // Defined$ Player = each player (Wheel of Fortune)
            // Defined$ Remembered = draw for remembered players only (Raphael's Technique)
            // otherwise controller placeholder
            let player = match params.get("Defined") {
                Some("Player") => PlayerId::all_players(),
                Some("Remembered") => PlayerId::remembered_players(),
                _ => PlayerId::placeholder(),
            };
            // Extract card count from NumCards$ parameter (default to 1 if not specified)
            // If the value is "X" referencing SVar X = Count$xPaid, use XPaid variant
            if params.get("NumCards") == Some("X") {
                Some(Effect::DrawCardsXPaid { player })
            } else {
                let count = params.get_u8("NumCards").unwrap_or(1);
                Some(Effect::DrawCards { player, count })
            }
        }

        ApiType::Discard => {
            let remember_discarded = params.get("RememberDiscarded") == Some("True");
            let optional = params.get("Optional") == Some("True");
            let remember_discarding_players = params.get("RememberDiscardingPlayers") == Some("True");
            // Player resolution semantics for Discard (mtg-564):
            //   Defined$ Player        -> each player (Mindslicer, Magus of the Wheel)
            //   Defined$ You           -> the controller (You Find the Villains' Lair);
            //                             resolved later from placeholder→card_owner
            //   ValidTgts$ Player      -> the OPPONENT (Mind Twist — "Target player
            //                             discards X cards"; we auto-pick the sole
            //                             opponent in 2-player games until a real
            //                             player-targeting UI lands)
            //   Defined$ TriggeredTarget / TriggeredPlayer
            //                          -> the player the trigger event acted on
            //                             (Hypnotic Specter — "that player discards a
            //                             card at random"). In 2-player games the
            //                             player Hypnotic Specter damaged is always the
            //                             controller's opponent, so we reuse the
            //                             target_opponent sentinel; resolved from
            //                             TriggerContext::opponent at execution time.
            //   otherwise              -> placeholder, resolved to controller
            //
            // CR 116.2c / CR 601.2c: targeted spells require a chosen target; we
            // approximate the choice as "opponent" in the absence of a player-
            // targeting UI. See mtg-564 for the long-term fix.
            let player = match (params.get("Defined"), params.get("ValidTgts")) {
                (Some("Player"), _) => PlayerId::all_players(),
                (Some("You"), _) => PlayerId::placeholder(),
                (Some("TriggeredTarget" | "TriggeredPlayer"), _) => PlayerId::target_opponent(),
                (_, Some(vt)) if vt.contains("Player") => PlayerId::target_opponent(),
                _ => PlayerId::placeholder(),
            };
            // If NumCards$ is "X" referencing SVar X = Count$xPaid, use XPaid variant
            if params.get("NumCards") == Some("X") {
                Some(Effect::DiscardCardsXPaid {
                    player,
                    remember_discarded,
                })
            } else {
                // Mode$ Hand = discard entire hand (Wheel of Fortune); otherwise fixed count
                // We use u8::MAX (255) as sentinel for "all cards in hand"
                let count = if params.get("Mode") == Some("Hand") {
                    u8::MAX // Sentinel: discard entire hand
                } else {
                    params.get_u8("NumCards").unwrap_or(1)
                };
                Some(Effect::DiscardCards {
                    player,
                    count,
                    remember_discarded,
                    optional,
                    remember_discarding_players,
                })
            }
        }

        ApiType::Destroy => {
            // Destroy effects target a permanent
            // Parse ValidTgts to determine what types can be targeted
            // Examples: "Artifact,Enchantment" for Disenchant, "Creature" for Terror
            // Defined$ Self means "destroy this card" (e.g., Chaos Orb's self-destruct)
            let restriction = params
                .get("ValidTgts")
                .map(TargetRestriction::parse)
                .unwrap_or_else(TargetRestriction::any);

            let target = if params.get("Defined") == Some("Self") {
                CardId::self_target()
            } else {
                CardId::placeholder()
            };

            // NoRegen$ True => the destroyed permanent can't be regenerated
            // (CR 701.15d), e.g. The Abyss, Terror.
            let no_regenerate = params.get("NoRegen").is_some_and(|v| v.eq_ignore_ascii_case("True"));

            Some(Effect::DestroyPermanent {
                target,
                restriction,
                no_regenerate,
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

            // Extract keywords to grant (KW$) - optional
            // Format: "KW$ Double Strike" or "KW$ Flying & Haste" (multiple separated by &)
            let keywords_granted: SmallVec<[Keyword; 2]> = params
                .get("KW")
                .map(|kw_str| {
                    kw_str
                        .split(" & ")
                        .filter_map(|kw| Keyword::from_string(kw.trim()))
                        .collect()
                })
                .unwrap_or_default();

            // Create effect if at least one bonus is non-zero, keywords are granted,
            // or a SubAbility$ chain needs target resolution (e.g., Prey Upon uses
            // SP$ Pump with +0/+0 purely as a targeting vehicle for DB$ Fight)
            let has_sub_ability = params.contains_key("SubAbility");
            if power_bonus != 0 || toughness_bonus != 0 || !keywords_granted.is_empty() || has_sub_ability {
                Some(Effect::PumpCreature {
                    target: CardId::new(0), // Placeholder - filled in at cast time
                    power_bonus,
                    toughness_bonus,
                    keywords_granted,
                })
            } else {
                None
            }
        }

        ApiType::Debuff => {
            // Debuff: Remove keywords from a creature
            // Example: AB$ Debuff | Keywords$ Defender | Defined$ Self
            // Example: AB$ Debuff | Keywords$ Flying | ValidTgts$ Creature
            // Note: Uses Keywords$ (not KW$ like Pump)
            let keywords_removed: SmallVec<[Keyword; 2]> = params
                .get("Keywords")
                .map(|kw_str| {
                    kw_str
                        .split(" & ")
                        .filter_map(|kw| Keyword::from_string(kw.trim()))
                        .collect()
                })
                .unwrap_or_default();

            if !keywords_removed.is_empty() || params.contains_key("SubAbility") {
                Some(Effect::DebuffCreature {
                    target: CardId::new(0), // Placeholder - filled in at cast time
                    keywords_removed,
                })
            } else {
                None
            }
        }

        ApiType::PumpAll => {
            // Mass pump: "Creatures you control get +1/+0 until end of turn"
            // Example: DB$ PumpAll | ValidCards$ Creature.YouCtrl | NumAtt$ +1
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

            // Get the filter (ValidCards$) - defaults to "Creature"
            let filter = params.get("ValidCards").unwrap_or("Creature").to_string();

            // Only create effect if at least one bonus is non-zero
            if power_bonus != 0 || toughness_bonus != 0 {
                Some(Effect::PumpAllCreatures {
                    controller: PlayerId::new(0), // Placeholder - filled in at effect execution
                    filter,
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
                // `Defined$ Targeted` (Falling Star: tap the creature this spell
                // already targeted, if it survived the damage) → reuse_previous
                // sentinel so the resolver taps the parent ability's target
                // instead of consuming a fresh chosen target. Mirrors the Untap
                // arm below.
                let target = if params.get("Defined") == Some("Targeted") {
                    CardId::reuse_previous()
                } else {
                    CardId::new(0) // Placeholder for independent targeting
                };
                Some(Effect::TapPermanent { target })
            }
        }

        ApiType::Untap => {
            // Check for Defined$ Targeted - use reuse_previous sentinel to share target with parent ability
            let target = if params.get("Defined") == Some("Targeted") {
                CardId::reuse_previous()
            } else {
                CardId::new(0) // Placeholder for independent targeting
            };
            Some(Effect::UntapPermanent { target })
        }

        ApiType::Mill => {
            let count = params.get_u8("NumCards").ok()?;
            Some(Effect::Mill {
                player: PlayerId::new(0), // Placeholder
                count,
            })
        }

        ApiType::Scry => {
            // Scry N - look at top N cards, put any on bottom
            // Example: "DB$ Scry | ScryNum$ 1"
            let count = params.get_u8("ScryNum").unwrap_or(1);
            Some(Effect::Scry {
                player: PlayerId::new(0), // Placeholder - filled in at trigger execution
                count,
            })
        }

        ApiType::Surveil => {
            // Surveil N - look at top N cards, put any into graveyard, rest on top (CR 701.42)
            // Example: "DB$ Surveil | Amount$ 1"
            let count = params.get_u8("Amount").unwrap_or(1);
            Some(Effect::Surveil {
                player: PlayerId::new(0), // Placeholder - filled in at trigger execution
                count,
            })
        }

        ApiType::Counter => {
            // ValidTgts$ carries the full spell restriction:
            //   - color qualifier (`Card.Blue` → Red Elemental Blast)
            //   - type restriction (`Creature` → Essence Scatter, `Artifact,Enchantment` → Annul)
            //   - nonCreature modifier (`Card.nonCreature` → Negate)
            //   - cmcGE modifier (`Card.cmcGE4` → Disdainful Stroke)
            let spell_restriction = params
                .get("ValidTgts")
                .map(TargetRestriction::parse)
                .unwrap_or_else(TargetRestriction::any);
            // RememberCounteredCMC$ True (Mana Drain): record the countered
            // spell's mana value so a chained delayed trigger can use it.
            let remember_mana_value = params.get("RememberCounteredCMC") == Some("True");
            Some(Effect::CounterSpell {
                target: CardId::new(0), // Placeholder
                spell_restriction,
                remember_mana_value,
            })
        }

        ApiType::ChangeZone => {
            // Check for exile effects: Origin$ Battlefield + Destination$ Exile
            if params.get("Origin") == Some("Battlefield") && params.get("Destination") == Some("Exile") {
                Some(Effect::ExilePermanent {
                    target: CardId::new(0), // Placeholder
                })
            }
            // Self-exile from stack: A:SP$ ChangeZone | Origin$ Stack | Destination$ Exile
            // (e.g. All Hallow's Eve "Exile this card with two scream counters on it.")
            //
            // The default sorcery-resolution path moves the card from the stack to
            // the graveyard once effects finish executing. This effect overrides
            // that: it moves the card from the stack to exile *during* effect
            // resolution, and `resolve_spell_finalize` then notices the card is
            // no longer on the stack and skips its default move-to-graveyard.
            //
            // RememberChanged$ True (the only flag we currently honour here) pushes
            // the moved card onto `GameState::remembered_cards` so chained
            // SubAbilities with `Defined$ Remembered` (e.g. the chained PutCounter
            // that places two scream counters on it) can find it.
            else if params.get("Origin") == Some("Stack") && params.get("Destination") == Some("Exile") {
                let remember_changed = params.get("RememberChanged") == Some("True");
                Some(Effect::SelfExileFromStack {
                    // Patched to the resolving spell's CardId by
                    // `resolve_self_target` during `resolve_spell_execute_effects`.
                    source: CardId::self_target(),
                    remember_changed,
                })
            }
            // Self zone-move outside the stack:
            //   DB$ ChangeZone | Defined$ Self | Origin$ <zone> | Destination$ <zone>
            // (e.g. All Hallow's Eve's exile→graveyard move once the last scream
            // counter is removed). Distinct from SelfExileFromStack (Origin$ Stack)
            // and from the battlefield→exile ExilePermanent special-case above.
            else if params.get("Defined") == Some("Self")
                && params.get("Origin") != Some("Stack")
                && params.get("Origin").is_some()
                && params.get("Destination").is_some()
            {
                let origin = params.get("Origin").and_then(crate::zones::Zone::from_str_lenient);
                let destination = params.get("Destination").and_then(crate::zones::Zone::from_str_lenient);
                match (origin, destination) {
                    (Some(origin), Some(destination)) => Some(Effect::MoveSelfBetweenZones {
                        source: CardId::self_target(),
                        origin,
                        destination,
                    }),
                    _ => None,
                }
            }
            // Check for library search effects: Origin$ Library
            else if params.get("Origin") == Some("Library") {
                let destination = params
                    .get("Destination")
                    .and_then(crate::zones::Zone::from_str_lenient)
                    .unwrap_or(crate::zones::Zone::Battlefield);

                let enters_tapped = params.get("Tapped") == Some("True");
                let card_type_filter = params.get("ChangeType").unwrap_or("Card").to_string();

                Some(Effect::SearchLibrary {
                    player: PlayerId::new(0), // Placeholder
                    card_type_filter,
                    destination,
                    enters_tapped,
                    shuffle: true, // Library searches always shuffle (MTG Rules 701.19b)
                })
            }
            // Recall: "return a card from your graveyard to your hand for each card
            // discarded this way."
            //
            //   SVar:DBChangeZone:DB$ ChangeZone | Origin$ Graveyard
            //     | Destination$ Hand | ChangeNum$ Y | ChangeType$ Card.YouOwn
            //   SVar:Y:Remembered$Amount
            //
            // ChangeNum$ Y resolves to the number of cards stored in
            // `GameState::remembered_cards` (i.e. the cards actually discarded
            // in the preceding Discard step). We detect this pattern by looking
            // for `Origin$ Graveyard | Destination$ Hand | ChangeNum$` which is
            // the dynamic-count return. The Y value must have been set via the
            // SVar mechanism; we don't look it up here — the runtime reads
            // `remembered_cards.len()` instead.
            else if params.get("Origin") == Some("Graveyard")
                && params.get("Destination") == Some("Hand")
                && params.contains_key("ChangeNum")
            {
                Some(Effect::ReturnCardsFromGraveyardToHand {
                    player: PlayerId::placeholder(),
                })
            }
            // Return-self from a non-battlefield, non-stack zone to another zone.
            // E.g. A:AB$ ChangeZone | Origin$ Graveyard | Destination$ Hand
            //        | ActivationZone$ Graveyard
            // (Earthquake Dragon: "Return CARDNAME from your graveyard to your hand.")
            // The card source is implicit (the ability's host card); use self_target()
            // so MoveSelfBetweenZones patches it to the real CardId at resolve time.
            // Targeted return: graveyard → hand for a card matching ValidTgts$
            // (e.g. Stormchaser's Talent level-2 trigger: return target instant
            //  or sorcery from your graveyard to hand).
            else if params.get("Origin") == Some("Graveyard")
                && params.get("Destination") == Some("Hand")
                && params.get("ValidTgts").is_some()
                && params.get("Defined").is_none()
            {
                // Extract type filter from ValidTgts$ (e.g. "Instant.YouCtrl,Sorcery.YouCtrl")
                // Strip qualifiers like ".YouCtrl" to get plain type names.
                let valid_tgts = params.get("ValidTgts").unwrap_or("Card");
                let type_filter: String = valid_tgts
                    .split(',')
                    .filter_map(|part| part.split('.').next())
                    .collect::<Vec<_>>()
                    .join(",");

                Some(Effect::ReturnGraveyardCardToHand {
                    player: PlayerId::placeholder(),
                    type_filter,
                })
            } else if matches!(params.get("Origin"), Some("Graveyard" | "Hand" | "Exile"))
                && params.get("Destination").is_some()
                && params.get("Defined").is_none()
            {
                let origin = params.get("Origin").and_then(crate::zones::Zone::from_str_lenient);
                let destination = params.get("Destination").and_then(crate::zones::Zone::from_str_lenient);
                match (origin, destination) {
                    (Some(origin), Some(destination)) => Some(Effect::MoveSelfBetweenZones {
                        source: CardId::self_target(),
                        origin,
                        destination,
                    }),
                    _ => None,
                }
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
                // `Produced$ Any` — produces one mana of any colour
                // (City of Brass, Mana Confluence, Vivid lands, etc.).
                //
                // We encode "any colour" as "all five colours set to 1"; the
                // mana-production cache derivation in core/card.rs converts
                // that pattern to `ManaProductionKind::AnyColor`. We must
                // NOT add `colorless = 1` here — the cache derivation
                // requires `mana.colorless == 0` for the AnyColor branch
                // (see card.rs::derive_mana_production), and any colour
                // mana doesn't include true {C} colourless anyway (CR 106.7
                // — "any colour" means W/U/B/R/G, distinct from {C}).
                //
                // Previously this returned `ManaCost::from_string("C")`,
                // which made City of Brass effectively a Colourless Source —
                // unable to satisfy any single-pip coloured cost. Pinned by
                // test_card_compat_city_of_brass.
                ManaCost {
                    white: 1,
                    blue: 1,
                    black: 1,
                    red: 1,
                    green: 1,
                    ..ManaCost::default()
                }
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

            // Check for Amount$ parameter (e.g., Amount$ 2 for Sol Ring, or Amount$ X for variable)
            let (amount, amount_var) = if let Some(amount_str) = params.get("Amount") {
                // Try parsing as fixed integer first
                if let Ok(n) = amount_str.parse::<u8>() {
                    (n, None)
                } else {
                    // It's a variable (X, Y, etc.) - store for later resolution
                    (1, Some(amount_str.to_string()))
                }
            } else {
                (1, None)
            };

            // Multiply mana by amount (for fixed amounts)
            let final_mana = mana_cost.multiply(amount);

            Some(Effect::AddMana {
                player: PlayerId::new(0), // Placeholder - filled in when activated
                mana: final_mana,
                produces_chosen_color,
                amount_var,
            })
        }

        ApiType::ManaReflected => {
            // Reflected mana (Fellwar Stone): "Add one mana of any color that a
            // land an opponent controls could produce."
            //   A:AB$ ManaReflected | Cost$ T | ColorOrType$ Color
            //     | Valid$ Land.OppCtrl | ReflectProperty$ Produce
            // The set of producible colors is DYNAMIC (depends on opponents'
            // lands at activation time), so the static cache cannot enumerate it.
            // We model the upper bound as an any-color source (all five colors set
            // to 1), exactly like `Produced$ Any`; the activation path
            // (tap_for_mana_for_cost) intersects this with the actual reflected
            // color set, flagged via ActivatedAbility::produces_reflected_mana.
            //
            // We only support the common Fellwar Stone shape here
            // (ColorOrType$ Color, ReflectProperty$ Produce); other shapes fall
            // through to no-op (still better than before — see mtg-ontwf).
            use crate::core::ManaCost;
            Some(Effect::AddMana {
                player: PlayerId::new(0),
                mana: ManaCost {
                    white: 1,
                    blue: 1,
                    black: 1,
                    red: 1,
                    green: 1,
                    ..ManaCost::default()
                },
                produces_chosen_color: false,
                amount_var: None,
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

            // Encode `Defined$` into the placeholder target so the resolver knows
            // what to substitute at execution time:
            //   - Self      → CardId::self_target() (the source card)
            //   - Remembered → CardId::remembered_card() (cards in remembered_cards)
            //   - default   → placeholder (resolved from chosen_targets)
            //
            // The Remembered case is required by chains like
            //   A:SP$ ChangeZone | Origin$ Stack | Destination$ Exile | RememberChanged$ True
            //                    | SubAbility$ DBPutCounter
            //   SVar:DBPutCounter:DB$ PutCounter | Defined$ Remembered | ...
            // (All Hallow's Eve), where the just-exiled card is the sole intended
            // target of the chained PutCounter.
            let target = match params.get("Defined") {
                Some("Self") => CardId::self_target(),
                Some("Remembered") => CardId::remembered_card(),
                _ => CardId::new(0),
            };

            Some(Effect::PutCounter {
                target,
                counter_type,
                amount,
            })
        }

        ApiType::PutCounterAll => {
            // PutCounterAll: Put counters on all permanents matching ValidCards$
            // Example: "DB$ PutCounterAll | ValidCards$ Creature.YouCtrl | CounterType$ P1P1 | CounterNum$ 1"
            use crate::core::CounterType;

            let counter_type_str = params.get("CounterType")?;
            let counter_type = CounterType::parse(counter_type_str)?;

            let amount = params.get_u8("CounterNum").unwrap_or(1);

            let restriction = params
                .get("ValidCards")
                .map(TargetRestriction::parse)
                .unwrap_or_else(TargetRestriction::any);

            Some(Effect::PutCounterAll {
                restriction,
                counter_type,
                amount,
            })
        }

        ApiType::Proliferate => {
            // Proliferate: no parameters needed - pure effect (CR 701.34a)
            // Example: "DB$ Proliferate" or "AB$ Proliferate | Cost$ B B Discard<1/Card>"
            Some(Effect::Proliferate)
        }

        ApiType::RemoveCounter => {
            // RemoveCounter effect: DB$ RemoveCounter | ValidTgts$ Creature | CounterType$ Any | CounterNum$ 3 | UpTo$ True
            // Example: Heartless Act mode 2 - "Remove up to three counters from target creature"
            //
            // CounterType$ can be:
            // - "P1P1" for +1/+1 counters
            // - "M1M1" for -1/-1 counters
            // - "Any" to remove any counter type (represented as None)
            //
            // UpTo$ True means "up to N counters" (minimum 0), otherwise exactly N counters
            use crate::core::CounterType;

            // Parse counter type (e.g., "P1P1" -> Some(+1/+1 counter), "Any" -> None)
            let counter_type_str = params.get("CounterType").unwrap_or("P1P1");
            let counter_type = if counter_type_str == "Any" {
                // "Any" means remove any counter type - represented as None
                None
            } else {
                Some(CounterType::parse(counter_type_str)?)
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

        ApiType::MultiplyCounter => {
            // MultiplyCounter: Double (or multiply) counters on a permanent
            // Example: "DB$ MultiplyCounter | Defined$ Self | CounterType$ P1P1" (double +1/+1 counters)
            // Example: "DB$ MultiplyCounter | ValidTgts$ Permanent | Multiplier$ 2" (double all counters)
            use crate::core::CounterType;

            let counter_type = params.get("CounterType").and_then(CounterType::parse);

            let multiplier = params.get_u8("Multiplier").unwrap_or(2); // Default: double

            Some(Effect::MultiplyCounter {
                target: CardId::new(0), // Placeholder - filled in at targeting/trigger time
                counter_type,
                multiplier,
            })
        }

        ApiType::Animate => {
            // Animate effect: AB$ Animate | Defined$ Self | Power$ 5 | Toughness$ 2
            // Example: Flexible Waterbender - "This creature has base power and toughness 5/2 until end of turn"
            // Also: AB$ Animate | Power$ 4 | Keywords$ Trample
            // Example: Turtle-Duck - "This creature has base power 4 and gains trample until end of turn"
            // Sets base P/T (counters and other bonuses are added on top)

            // Parse power (optional)
            let power = params.get_i32("Power").ok();

            // Parse toughness (optional)
            let toughness = params.get_i32("Toughness").ok();

            // Parse keywords (optional) - e.g., "Keywords$ Trample" or "Keywords$ Flying & First Strike"
            let keywords_granted = if let Some(kw_str) = params.get("Keywords") {
                // Parse keyword string (may be single or "&" separated)
                use crate::core::Keyword;
                let mut keywords = smallvec::SmallVec::new();
                for kw_part in kw_str.split('&').map(|s| s.trim()) {
                    if !kw_part.is_empty() {
                        if let Some(kw) = Keyword::from_string(kw_part) {
                            keywords.push(kw);
                        }
                    }
                }
                keywords
            } else {
                smallvec::smallvec![]
            };

            // Parse Types$ parameter — e.g. "Artifact,Creature,Assembly-Worker"
            // for Mishra's Factory. Per Forge-Java conventions, this is a
            // comma-separated list mixing card types (Artifact, Creature,
            // Land, Enchantment, ...) and subtypes (Assembly-Worker, etc.).
            // We split it into the two buckets; anything that isn't a known
            // CardType becomes a Subtype.
            let mut types_added: smallvec::SmallVec<[crate::core::CardType; 2]> = smallvec::SmallVec::new();
            let mut subtypes_added: smallvec::SmallVec<[crate::core::Subtype; 2]> = smallvec::SmallVec::new();
            if let Some(types_str) = params.get("Types") {
                for part in types_str.split(',').map(|s| s.trim()) {
                    if part.is_empty() {
                        continue;
                    }
                    match part {
                        "Artifact" => types_added.push(crate::core::CardType::Artifact),
                        "Creature" => types_added.push(crate::core::CardType::Creature),
                        "Land" => types_added.push(crate::core::CardType::Land),
                        "Enchantment" => types_added.push(crate::core::CardType::Enchantment),
                        "Instant" => types_added.push(crate::core::CardType::Instant),
                        "Sorcery" => types_added.push(crate::core::CardType::Sorcery),
                        "Planeswalker" => types_added.push(crate::core::CardType::Planeswalker),
                        // Anything else is a creature subtype
                        // (Assembly-Worker, Goblin, Wall, etc.)
                        other => subtypes_added.push(crate::core::Subtype::new(other)),
                    }
                }
            }

            // Per Forge-Java, "RemoveCreatureTypes$ True" strips existing
            // creature subtypes before the new ones are added.
            let remove_creature_subtypes = params
                .get("RemoveCreatureTypes")
                .map(|v| v.eq_ignore_ascii_case("true"))
                .unwrap_or(false);

            // At least one of power, toughness, keywords, or types must be set
            if power.is_none()
                && toughness.is_none()
                && keywords_granted.is_empty()
                && types_added.is_empty()
                && subtypes_added.is_empty()
            {
                return None;
            }

            Some(Effect::SetBasePowerToughness {
                target: CardId::new(0), // Placeholder - filled in at activation time
                power,
                toughness,
                keywords_granted,
                types_added,
                subtypes_added,
                remove_creature_subtypes,
            })
        }

        ApiType::AnimateAll => {
            // AnimateAll: set base P/T and/or grant keywords to all matching permanents
            // Example: AB$ AnimateAll | ValidCards$ Planeswalker.YouCtrl | Power$ 4 | Toughness$ 4
            //          | Types$ Creature,Dragon | Keywords$ Flying | AILogic$ Always
            // Example: AB$ AnimateAll | ValidCards$ Creature.YouCtrl | Keywords$ Deathtouch
            // Example: AB$ AnimateAll | ValidCards$ Permanent.OppCtrl | RemoveKeywords$ Hexproof & Indestructible

            let power = params.get_i32("Power").ok();
            let toughness = params.get_i32("Toughness").ok();

            let filter = params.get("ValidCards").unwrap_or("Creature").to_string();

            let keywords_granted = if let Some(kw_str) = params.get("Keywords") {
                use crate::core::Keyword;
                let mut keywords = smallvec::SmallVec::new();
                for kw_part in kw_str.split('&').map(|s| s.trim()) {
                    if !kw_part.is_empty() {
                        if let Some(kw) = Keyword::from_string(kw_part) {
                            keywords.push(kw);
                        }
                    }
                }
                keywords
            } else {
                smallvec::smallvec![]
            };

            // At least one of power, toughness, or keywords must be set
            if power.is_none() && toughness.is_none() && keywords_granted.is_empty() {
                return None;
            }

            Some(Effect::AnimateAll {
                controller: PlayerId::new(0), // Placeholder - filled at execution
                filter,
                power,
                toughness,
                keywords_granted,
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
            // Maze of Ith: DB$ Effect | ReplacementEffects$ RPrevent1,RPrevent2
            //              | RememberObjects$ Targeted | ExileOnMoved$ Battlefield
            //
            // This pattern creates a "prevent all combat damage dealt to and by
            // the targeted creature this turn" replacement effect. The target is
            // derived from `last_resolved_target` in the sub-ability chain (the
            // same creature that was untapped by the preceding AB$ Untap effect).
            // Detect this pattern by checking for ReplacementEffects$ containing
            // "RPrevent" (damage prevention replacement) plus RememberObjects$ Targeted.
            if params.get("ReplacementEffects").is_some_and(|v| v.contains("RPrevent"))
                && params.get("RememberObjects") == Some("Targeted")
            {
                return Some(Effect::PreventAllCombatDamageThisTurn {
                    // Placeholder — resolved from last_resolved_target in
                    // resolve_effect_target (the UntapPermanent target in the chain).
                    target: CardId::new(0),
                });
            }

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

        ApiType::Clone => {
            // Clone: the SOURCE permanent enters as a copy of a chosen permanent (CR 707).
            // Example (Copy Artifact): DB$ Clone | Choices$ Artifact.Other | AddTypes$ Enchantment
            //
            // Parameters:
            // - Choices$: filter restricting which permanents may be copied
            // - AddTypes$ Type1 & Type2: CARD types to add on top of the copied
            //   values (e.g. Enchantment). Note: these are card *types*, not
            //   subtypes (contrast with CopyPermanent's AddTypes$).
            //
            // `optional` is carried by the surrounding ETBReplacement keyword
            // (the `Optional` flag), not the SVar body, so it defaults to false
            // here and is set when the keyword is wired up in card.rs.
            let choices_filter = params
                .get("Choices")
                .map(TargetRestriction::parse)
                .unwrap_or_else(TargetRestriction::any);

            let add_types: smallvec::SmallVec<[crate::core::CardType; 1]> = params
                .get("AddTypes")
                .map(|s| s.split(" & ").filter_map(crate::core::CardType::parse).collect())
                .unwrap_or_default();

            Some(Effect::Clone {
                source: CardId::new(0), // Placeholder - filled in with the source card at ETB
                chosen: CardId::new(0), // Placeholder - chosen by the controller at resolution
                choices_filter,
                add_types,
                optional: false,
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

        ApiType::Token => {
            // Token creation: DB$ Token | TokenScript$ c_a_clue_draw | TokenOwner$ You
            //
            // Parameters:
            // - TokenScript$: Name of the token script file (e.g., c_a_clue_draw, c_a_food_sac)
            // - TokenOwner$: Who controls the token (You, Opponent, etc.)
            // - TokenAmount$: Number of tokens to create (default 1)
            //
            // Examples:
            // - Cunning Maneuver: creates Clue token (c_a_clue_draw)
            // - Canyon Crawler: creates Food token (c_a_food_sac)
            let token_script = params.get("TokenScript")?.to_string();
            let amount = params.get_u8("TokenAmount").unwrap_or(1);

            // TokenOwner$ parsing - default to controller (You)
            // "Player" means each player creates tokens
            let token_owner = params.get("TokenOwner");
            let for_each_player = token_owner == Some("Player");
            let controller = match token_owner {
                Some("Opponent") => PlayerId::new(1), // Placeholder - will be resolved at runtime
                _ => PlayerId::new(0),                // Placeholder - controller
            };

            Some(Effect::CreateToken {
                controller,
                token_script,
                amount,
                for_each_player,
            })
        }

        ApiType::DigMultiple => {
            // Dig effect: look at top N cards of a library, move some to destination
            //
            // Fire Lord Ozai: AB$ Dig | Cost$ 6 | DigNum$ 1 | ChangeNum$ All | Defined$ Opponent
            //                       | DestinationZone$ Exile | RememberChanged$ True | SubAbility$ DBEffect
            //
            // Seismic Sense: A:SP$ Dig | DigNum$ X | ChangeNum$ 1 | Optional$ True
            //                        | ForceRevealToController$ True | ChangeValid$ Creature,Land
            //                        | RestRandomOrder$ True
            //
            // Parameters:
            // - DigNum$: Number of cards to look at (default 1)
            // - ChangeNum$: Number of cards to change zones ("All" or number)
            // - Defined$: Who to affect - "Opponent" = opponents' libraries, else = own library
            // - DestinationZone$: Where to move cards (default: Hand for self, Exile for opponent)
            // - Optional$: Whether selecting cards is optional
            // - RestRandomOrder$: Whether to randomize non-selected cards before putting on bottom
            // - RememberChanged$: Whether to remember moved cards for later use
            // - MayPlay$: Whether controller may play exiled cards
            // - MayPlayWithoutManaCost$: Whether playing costs no mana

            let dig_count = params.get_u8("DigNum").unwrap_or(1);

            // Parse ChangeNum$ - "All" means all cards looked at
            let (change_count, change_all) = match params.get("ChangeNum") {
                Some("All") => (dig_count, true),
                Some(n) => (n.parse::<u8>().unwrap_or(dig_count), false),
                None => (dig_count, true), // Default to moving all
            };

            // Parse Defined$ - determines whose library to dig from
            // "Opponent" means opponents' libraries, anything else (including absent) means own
            let target_self = match params.get("Defined") {
                Some("Opponent") => false,
                _ => true, // Default: dig from own library (You, absent, etc.)
            };

            // Parse destination zone - default depends on target_self
            let destination = params
                .get("DestinationZone")
                .and_then(crate::zones::Zone::from_str_lenient)
                .unwrap_or(
                    // Default: Hand for self-dig (Impulse/Seismic Sense), Exile for opponent-dig
                    if target_self {
                        crate::zones::Zone::Hand
                    } else {
                        crate::zones::Zone::Exile
                    },
                );

            // Parse Optional$ - whether selecting cards is optional
            let optional = params.get("Optional").is_some_and(|v| v == "True");

            // Parse RestRandomOrder$ - whether to randomize non-selected cards
            let rest_random = params.get("RestRandomOrder").is_some_and(|v| v == "True");

            // Parse Reveal$ - whether to reveal dug cards to all players
            let reveal = params.get("Reveal").is_some_and(|v| v == "True");

            // Parse DestinationZone2$ - where non-selected cards go (default: Library bottom)
            let rest_destination = params
                .get("DestinationZone2")
                .and_then(crate::zones::Zone::from_str_lenient)
                .unwrap_or(crate::zones::Zone::Library);

            // Parse ChangeValid$ - filter for which cards are valid to select
            // e.g. "Creature,Land" or "Artifact" or "Permanent"
            let change_valid: smallvec::SmallVec<[crate::core::DigFilter; 2]> =
                if let Some(valid_str) = params.get("ChangeValid") {
                    valid_str
                        .split(',')
                        .filter_map(|s| crate::core::DigFilter::parse(s.trim()))
                        .collect()
                } else {
                    smallvec::SmallVec::new() // empty = any card
                };

            // Check for may play options (usually in SubAbility$ DBEffect)
            // For now, we detect may_play by presence of SubAbility with Effect
            let has_sub_ability = params.contains_key("SubAbility");
            let may_play = has_sub_ability; // SubAbility usually grants may play
            let may_play_without_mana_cost = has_sub_ability; // Fire Lord Ozai doesn't cost mana

            log::debug!(
                target: "effect_converter",
                "Dig: {} cards, change {} (all={}), dest={:?}, rest_dest={:?}, may_play={}, free={}, target_self={}, optional={}, rest_random={}, reveal={}, filters={:?}",
                dig_count, change_count, change_all, destination, rest_destination, may_play, may_play_without_mana_cost, target_self, optional, rest_random, reveal, change_valid
            );

            Some(Effect::Dig {
                dig_count,
                change_count,
                change_all,
                destination,
                rest_destination,
                may_play,
                may_play_without_mana_cost,
                target_self,
                optional,
                rest_random,
                reveal,
                change_valid,
            })
        }

        ApiType::DelayedTrigger => {
            // Delayed trigger: SP$ DelayedTrigger | Mode$ ChangesZone | Origin$ Battlefield | Destination$ Graveyard
            //                  | ValidTgts$ Creature | RememberObjects$ Targeted | ThisTurn$ True | Execute$ TrigEarthbend
            //
            // Creates a delayed trigger that fires when a condition is met.
            // Fatal Fissure: "Choose target creature. When that creature dies this turn, you earthbend 4."
            //
            // Parameters:
            // - Mode$: Trigger condition (ChangesZone, SpellCast, etc.)
            // - Origin$: Source zone for zone change (Battlefield, Any)
            // - Destination$: Destination zone for zone change (Graveyard, Exile)
            // - ValidTgts$: What can be targeted (Creature, Permanent)
            // - RememberObjects$: What to remember (Targeted)
            // - ThisTurn$: If True, trigger expires at end of turn
            // - Execute$: SVar to execute when trigger fires
            //
            // Note: This only creates a placeholder effect. The actual effect needs
            // SVar resolution via params_to_delayed_trigger_with_svars().

            let mode = params.get("Mode").unwrap_or("ChangesZone");

            // Parse zone change condition (most common for delayed triggers)
            let condition = if mode == "ChangesZone" {
                let from_zone = params
                    .get("Origin")
                    .and_then(crate::zones::Zone::from_str_lenient)
                    .unwrap_or(crate::zones::Zone::Battlefield); // Default for death triggers

                let to_zone = params
                    .get("Destination")
                    .and_then(crate::zones::Zone::from_str_lenient)
                    .unwrap_or(crate::zones::Zone::Graveyard); // Default for death triggers

                crate::core::DelayedTriggerCondition::ZoneChange {
                    from_zones: smallvec::smallvec![from_zone],
                    to_zones: smallvec::smallvec![to_zone],
                }
            } else {
                // Other modes (SpellCast, etc.) not yet supported
                log::debug!(
                    target: "effect_converter",
                    "DelayedTrigger Mode$ '{}' not yet implemented",
                    mode
                );
                return None;
            };

            // Parse expiry - ThisTurn$ True means expire at end of turn
            let expiry = if params.get("ThisTurn") == Some("True") {
                Some(crate::core::DelayedTriggerExpiry::EndOfTurn)
            } else {
                None
            };

            // Placeholder effect - the actual effect needs SVar resolution
            // This will be replaced by params_to_delayed_trigger_with_svars()
            let placeholder_effect = Effect::DrawCards {
                player: PlayerId::new(0),
                count: 0,
            };

            log::debug!(
                target: "effect_converter",
                "DelayedTrigger: mode={}, condition={:?}, expiry={:?}, execute={:?}",
                mode, condition, expiry, params.get("Execute")
            );

            Some(Effect::CreateDelayedTrigger {
                tracked_card: CardId::new(0), // Placeholder - filled in at cast time
                condition,
                effect: Box::new(placeholder_effect),
                expiry,
            })
        }

        ApiType::CopySpellAbility => {
            // Copy a spell on the stack
            // Examples:
            //   DB$ CopySpellAbility | Defined$ TriggeredSpellAbility | MayChooseTarget$ True
            //   DB$ CopySpellAbility | Defined$ Parent | Controller$ TargetedOrController | MayChooseTarget$ True
            //
            // Parameters:
            // - Defined$: What to copy
            //   - TriggeredSpellAbility = the spell that triggered this (for delayed triggers)
            //   - Parent = the current spell (for SubAbility chaining like Chain Lightning)
            // - Controller$: Who controls the copy (optional, defaults to caster)
            // - MayChooseTarget$: Can choose new targets for the copy
            use crate::core::effects::CopySpellSource;

            let may_choose_targets = params.get("MayChooseTarget") == Some("True");
            let defined_source = match params.get("Defined") {
                Some("TriggeredSpellAbility") => CopySpellSource::TriggeredSpellAbility,
                // ONLY an explicit `Defined$ Parent` is a real "copy this spell"
                // self-copy (Chain Lightning, which terminates via its {R}{R}
                // gate). A bare / absent / any-other Defined$ is the "copy a
                // separately-TARGETED spell" class (Twincast, Reverberate, Fork,
                // Return the Favor, ...): defaulting THOSE to Parent makes them
                // copy themselves forever (commander-format infinite loop). Route
                // them to the no-op TargetedSpell instead.
                Some("Parent") => CopySpellSource::Parent,
                _ => CopySpellSource::TargetedSpell,
            };
            let controller = params.get("Controller").map(String::from);

            log::debug!(
                target: "effect_converter",
                "CopySpellAbility: may_choose_targets={}, defined_source={:?}, controller={:?}",
                may_choose_targets,
                defined_source,
                controller
            );

            Some(Effect::CopySpellAbility {
                may_choose_targets,
                defined_source,
                controller,
            })
        }

        ApiType::ImmediateTrigger => {
            // Conditional sub-effect execution based on remembered cards
            // Requires SVar resolution - use params_to_immediate_trigger_with_svars() instead
            // For now, return None to indicate this needs special handling
            log::debug!(
                target: "effect_converter",
                "ImmediateTrigger requires SVar resolution - use params_to_immediate_trigger_with_svars()"
            );
            None
        }

        ApiType::DamageResolve => {
            // DamageResolve is a marker that all damage accumulated in the
            // damage map (via DealDamage | DamageMap$ True) should be applied
            // simultaneously. See ApiType::DamageResolve doc comment for the
            // full design rationale.
            //
            // Our engine applies each DealDamage immediately during effect
            // execution, so by the time we reach the trailing DamageResolve
            // in a SubAbility chain, all damage has already been dealt.
            // Returning None means we contribute no Effect to the chain;
            // SubAbility recursion (`follow_sub_ability_chain`) continues past
            // this without warning. This is the correct no-op for our model.
            //
            // Affected cards (~62): Psionic Blast, Char, Arc Trail, Boulder
            // Dash, Brothers of Fire, Bonfire of the Damned, Chandra Nalaar,
            // and other multi-damage spells/abilities. See forge-java
            // cardsfolder for the full list.
            log::debug!(
                target: "effect_converter",
                "DamageResolve: marker effect resolved as no-op (damage already dealt eagerly)"
            );
            None
        }

        ApiType::Cleanup => {
            // Clear remembered cards storage
            // Example: DB$ Cleanup | ClearRemembered$ True
            let clear_remembered = params.get("ClearRemembered") == Some("True");

            if clear_remembered {
                Some(Effect::ClearRemembered)
            } else {
                // Other Cleanup modes not implemented
                log::debug!(
                    target: "effect_converter",
                    "Cleanup without ClearRemembered$ True not implemented"
                );
                None
            }
        }

        ApiType::Regenerate => {
            // Regenerate: Add a regeneration shield to target permanent (CR 701.15a)
            // Most cards target self: "AB$ Regenerate | Cost$ B | SpellDescription$ Regenerate CARDNAME."
            // Some target other creatures: "AB$ Regenerate | ValidTgts$ Creature | ..."
            Some(Effect::Regenerate {
                target: CardId::new(0), // Placeholder - filled in at activation time
            })
        }

        ApiType::PreventDamage => {
            // PreventDamage: Create a damage prevention shield (CR 615.1)
            // Examples: "AB$ PreventDamage | Cost$ T | ValidTgts$ Any | Amount$ 1"
            //           "AB$ PreventDamage | Cost$ W T | Defined$ Self | Amount$ 1"
            //           "AB$ PreventDamage | Cost$ PayLife<2> | ValidTgts$ Creature | Amount$ 1"
            let amount = params.get_i32("Amount").unwrap_or(1);

            // Determine target type from Defined$ or ValidTgts$
            let target = match params.get("Defined") {
                Some("Self" | "ParentTarget") => {
                    TargetRef::Permanent(CardId::new(0)) // Placeholder - resolved at activation
                }
                Some("You" | "Player") => {
                    TargetRef::Player(PlayerId::new(0)) // Placeholder - resolved at activation
                }
                _ => TargetRef::None, // Will be resolved from ValidTgts$ at cast time
            };

            Some(Effect::PreventDamage { target, amount })
        }

        ApiType::ChooseSource => {
            // Circle-of-Protection family: choose a source of a given color and
            // create a source-filtered damage-prevention shield protecting the
            // controller this turn (CR 615.1, 615.6). The whole CoP card shape
            // is:
            //   A:AB$ ChooseSource | Cost$ 1 | Choices$ Card.RedSource
            //         | AILogic$ NeedsPrevention | SubAbility$ DBEffect | ...
            //   SVar:DBEffect:DB$ Effect | ReplacementEffects$ RPreventNextFromSource | ...
            //   SVar:RPreventNextFromSource:Event$ DamageDone
            //         | ValidSource$ Card.ChosenCardStrict+<Color>Source
            //         | ValidTarget$ You | PreventionEffect$ True | ...
            //
            // We collapse this into a single Effect::PreventDamageFromSource.
            // The source color is read from the `Choices$ Card.<Color>Source`
            // filter (tokenized, NOT substring-matched), generalizing across all
            // five Circles of Protection. The chosen source CardId is a
            // placeholder resolved from the ability's target at activation.
            let color = choose_source_color(params)?;
            Some(Effect::PreventDamageFromSource {
                protected: PlayerId::new(0), // Placeholder - resolved to controller at activation
                color,
                source: CardId::new(0), // Placeholder - resolved from chosen target
            })
        }

        ApiType::LoseLife => {
            // LoseLife: Target player or defined players lose life
            // Examples: "AB$ LoseLife | LifeAmount$ 2 | Defined$ Opponent"
            //           "AB$ LoseLife | LifeAmount$ 1 | ValidTgts$ Player"
            let amount = params.get_i32("LifeAmount").ok()?;

            // Placeholder - resolved at cast time. Defined$ Opponent resolves to opponent.
            Some(Effect::LoseLife {
                player: PlayerId::new(0),
                amount,
            })
        }

        ApiType::AddPhase => {
            // Extra combat phase: DB$ AddPhase | PhaseType$ Combat
            // Example: Raphael Tag Team Tough - "After this main phase, there is an additional combat phase"
            let count = params.get_u8("NumPhases").unwrap_or(1);
            Some(Effect::AddPhase { count })
        }

        ApiType::DestroyAll => {
            // DestroyAll: Destroy all permanents matching ValidCards$
            // Example: "SP$ DestroyAll | ValidCards$ Creature | NoRegen$ True" (Wrath of God)
            let restriction = params
                .get("ValidCards")
                .map(TargetRestriction::parse)
                .unwrap_or_else(TargetRestriction::any);

            let no_regenerate = params.get("NoRegen").is_some_and(|v| v.eq_ignore_ascii_case("True"));

            Some(Effect::DestroyAll {
                restriction,
                no_regenerate,
            })
        }

        ApiType::DamageAll => {
            // DamageAll: Deal damage to all creatures matching ValidCards$, optionally players
            // Example: "SP$ DamageAll | NumDmg$ 2 | ValidCards$ Creature" (Pyroclasm)
            let amount = params.get_i32("NumDmg").ok()?;

            let valid_cards = params
                .get("ValidCards")
                .map(TargetRestriction::parse)
                .unwrap_or_else(TargetRestriction::any);

            let damage_players = params.get("ValidPlayers").is_some();

            Some(Effect::DamageAll {
                amount,
                valid_cards,
                damage_players,
            })
        }

        ApiType::Sacrifice => {
            // ForceSacrifice: Target player sacrifices permanents of a type
            // Examples: "SP$ Sacrifice | ValidTgts$ Player | SacValid$ Creature" (Diabolic Edict)
            //           "SP$ Sacrifice | Amount$ 2 | SacValid$ Creature | Defined$ Player" (Barter in Blood)
            let sac_type = params.get("SacValid").unwrap_or("Creature").to_string();
            let count = params.get_i32("Amount").unwrap_or(1) as u8;

            // Placeholder - resolved at cast time.
            // Defined$ Player means "each player" (expand_all_players),
            // Defined$ Opponent or ValidTgts$ Player means opponent (default for placeholder)
            Some(Effect::ForceSacrifice {
                player: PlayerId::new(0),
                sac_type,
                count,
            })
        }

        ApiType::SacrificeAll => {
            // SacrificeAll: Each player sacrifices all permanents matching ValidCards$
            // Example: "SP$ SacrificeAll | ValidCards$ Permanent.nonColorless" (All is Dust)
            // Also handles Defined$ for targeted sacrifice (simpler form)
            let restriction = params
                .get("ValidCards")
                .map(TargetRestriction::parse)
                .unwrap_or_else(TargetRestriction::any);

            Some(Effect::SacrificeAll { restriction })
        }

        ApiType::TapAll => {
            // TapAll: Tap all permanents matching ValidCards$
            // Example: "DB$ TapAll | ValidCards$ Creature.OppCtrl" (Cryptic Command mode)
            let restriction = params
                .get("ValidCards")
                .map(TargetRestriction::parse)
                .unwrap_or_else(TargetRestriction::any);

            // Falling Star (mtg-503) digital divergence: its upstream
            // `DBTapAllDamaged` is `TapAll | ValidCards$ Creature.IsRemembered+DamagedBy`
            // — "tap each creature this card just damaged." Since we approximate
            // the flip as a single TARGETED 3 damage (see the FlipOntoBattlefield
            // arm), there is exactly one such creature: the spell's target. We
            // convert the IsRemembered mass-tap into a survival-gated single tap
            // chained to the damage target (`Tap | Defined$ Targeted`), so a
            // surviving creature is tapped and a creature that died to the 3
            // damage is not. requires_remembered is the tokenized signal for the
            // `IsRemembered` qualifier (set by TargetRestriction::parse).
            if restriction.requires_remembered {
                Some(Effect::TapPermanent {
                    target: CardId::reuse_previous(),
                })
            } else {
                Some(Effect::TapAll { restriction })
            }
        }

        ApiType::UntapAll => {
            // UntapAll: Untap all permanents matching ValidCards$
            // Example: "AB$ UntapAll | ValidCards$ Creature.YouCtrl" (Aggravated Assault)
            let restriction = params
                .get("ValidCards")
                .map(TargetRestriction::parse)
                .unwrap_or_else(TargetRestriction::any);

            Some(Effect::UntapAll { restriction })
        }

        ApiType::DrainMana => {
            // DrainMana: empty a player's unspent mana pool ("lose all unspent mana").
            // Power Sink: "DB$ DrainMana | Defined$ TargetedController".
            // The player sentinel is resolved at execution time:
            //   Defined$ TargetedController -> controller of the countered spell
            //   Defined$ You                -> the controller
            //   otherwise (default)         -> the opponent (placeholder fallback)
            let player = match params.get("Defined") {
                Some("TargetedController") => PlayerId::target_controller(),
                Some("You") => PlayerId::placeholder(),
                _ => PlayerId::target_opponent(),
            };
            Some(Effect::DrainMana { player })
        }

        ApiType::SetLife => {
            // SetLife: Set a player's life total to a specific amount
            // Example: "AB$ SetLife | Defined$ You | LifeAmount$ 10" (Angel of Grace)
            //          "SP$ SetLife | ValidTgts$ Player | LifeAmount$ 20" (Blessed Wind)
            let amount = params.get_i32("LifeAmount").ok()?;

            Some(Effect::SetLife {
                player: PlayerId::new(0),
                amount,
            })
        }

        ApiType::GainControl => {
            // GainControl: Gain control of target permanent
            // Examples:
            //   "AB$ GainControl | ValidTgts$ Creature | LoseControl$ EOT" (Threaten)
            //   "AB$ GainControl | ValidTgts$ Artifact | LoseControl$ LeavesPlay" (Aladdin)
            let untap = params.get("Untap").is_some_and(|v| v.eq_ignore_ascii_case("true"));
            // Map the `LoseControl$` token list to a structured duration. The
            // tokens are comma-separated; we classify by which are present
            // (tokenized split, not a raw substring scan of the whole value).
            use crate::core::effects::ControlDuration;
            let duration = match params.get("LoseControl") {
                None => ControlDuration::Permanent,
                Some(spec) => {
                    let tokens: smallvec::SmallVec<[&str; 4]> = spec.split(',').map(str::trim).collect();
                    if tokens.contains(&"EOT") {
                        ControlDuration::EndOfTurn
                    } else if tokens.iter().any(|t| *t == "Untap" || *t == "StaticCommandCheck") {
                        // Old Man of the Sea's tapped + power-comparison duration is
                        // not modeled yet (mtg-713 B1 follow-up). Fall back to the
                        // source-presence duration so control is at least bounded by
                        // the source leaving play rather than being permanent.
                        ControlDuration::WhileControlSource
                    } else {
                        // LeavesPlay / LoseControl: keep control while you control
                        // the source permanent (Aladdin).
                        ControlDuration::WhileControlSource
                    }
                }
            };
            // Parse the target restriction from ValidTgts$ (Aladdin `Artifact`,
            // Old Man `Creature.powerLEX`). Absent -> any permanent.
            let restriction = params
                .get("ValidTgts")
                .map(crate::core::effects::TargetRestriction::parse)
                .unwrap_or_else(crate::core::effects::TargetRestriction::any);

            Some(Effect::GainControl {
                target: CardId::placeholder(),
                new_controller: PlayerId::new(0), // Resolved at cast time
                untap,
                duration,
                restriction,
                source: None, // Threaded in at resolution (the resolving card)
            })
        }

        ApiType::Fight => {
            // Fight: Two creatures deal damage equal to their power to each other (CR 701.12)
            // Examples:
            //   "SP$ Fight | Defined$ ParentTarget | ValidTgts$ Creature.OppCtrl" (Prey Upon)
            //   "AB$ Fight | Defined$ Self | ValidTgts$ Creature.Other" (Brash Taunter)
            // The "fighter" (Defined$) is the initiating creature, "target" is from ValidTgts$
            Some(Effect::Fight {
                fighter: CardId::placeholder(), // Resolved from Defined$ at cast time
                target: CardId::placeholder(),  // Resolved from ValidTgts$ at cast time
            })
        }

        ApiType::ChangeZoneAll => {
            // ChangeZoneAll: Move all cards matching a filter from one zone to another
            // Example: "SP$ ChangeZoneAll | ChangeType$ Creature.attacking | Origin$ Battlefield | Destination$ Hand"
            // Example: "DB$ ChangeZoneAll | ChangeType$ Card | Origin$ Graveyard | Destination$ Exile"
            // `Origin$` may list MULTIPLE comma-separated zones (e.g.
            // Timetwister's `Origin$ Hand,Graveyard`). Parse each into a Zone;
            // drop unknown tokens. Default to Battlefield when nothing parses.
            let mut origins: smallvec::SmallVec<[crate::zones::Zone; 2]> = params
                .get("Origin")
                .map(|s| {
                    s.split(',')
                        .filter_map(|tok| crate::zones::Zone::from_str_lenient(tok.trim()))
                        .collect()
                })
                .unwrap_or_default();
            if origins.is_empty() {
                origins.push(crate::zones::Zone::Battlefield);
            }

            let destination = params
                .get("Destination")
                .and_then(crate::zones::Zone::from_str_lenient)
                .unwrap_or(crate::zones::Zone::Exile);

            let restriction = params
                .get("ChangeType")
                .map(TargetRestriction::parse)
                .unwrap_or_else(TargetRestriction::any);

            // `Shuffle$ True` — shuffle affected libraries after the move
            // (Timetwister, Mnemonic Nexus). Distinct from ordered moves like
            // `LibraryPosition$ -1` which deliberately do NOT shuffle.
            let shuffle = params
                .get("Shuffle")
                .map(|v| v.eq_ignore_ascii_case("true"))
                .unwrap_or(false);

            Some(Effect::ChangeZoneAll {
                restriction,
                origins,
                destination,
                shuffle,
            })
        }

        ApiType::AddTurn => {
            // AddTurn: Take extra turns
            // Example: "SP$ AddTurn | NumTurns$ 1" (Time Walk)
            let num_turns = params.get_u8("NumTurns").unwrap_or(1);
            Some(Effect::AddTurn {
                player: PlayerId::new(0), // Placeholder - filled in at cast time
                num_turns,
            })
        }

        ApiType::TapOrUntap => {
            // TapOrUntap: Tap or untap target permanent (player chooses)
            // Example: "DB$ TapOrUntap | ValidTgts$ Creature" (Bounding Krasis)
            Some(Effect::TapOrUntapPermanent {
                target: CardId::placeholder(),
            })
        }

        ApiType::ChooseColor => {
            // ChooseColor: Player chooses a color (WUBRG), stored on source card
            // Example: "AB$ ChooseColor | Cost$ G | Defined$ You | SubAbility$ Animate"
            // The chosen color is stored in Card::chosen_color and referenced by
            // subsequent abilities via "ChosenColor" patterns.
            Some(Effect::ChooseColor {
                player: PlayerId::new(0),      // Placeholder - resolved to card_owner at cast time
                source: CardId::placeholder(), // Placeholder - resolved to spell card_id at cast time
            })
        }

        ApiType::Attach => {
            // Attach Equipment or Aura to target
            // Example: DB$ Attach | ValidTgts$ Creature.YouCtrl
            Some(Effect::AttachEquipment {
                source_equipment: CardId::new(0), // Placeholder
                target_creature: CardId::new(0),  // Placeholder
            })
        }

        // `FlipOntoBattlefield` (Chaos Orb mtg-389 / Falling Star mtg-503) are
        // paper DEXTERITY cards: the player physically flips the card and it
        // affects whatever it lands on. There is no faithful digital analog, so
        // — per the project owner — we DIVERGE to a single TARGETED
        // approximation (a skilled flipper hits exactly the permanent aimed at;
        // a smart opponent spreads their board so only one is caught). We keep
        // the upstream Forge-Java card scripts UNEDITED (they live in the
        // forge-java submodule) and express the divergence here, the same place
        // the project handles every other Rust-vs-Java card divergence.
        //
        // We branch on the structured `SubAbility$` reference (tokenized, not a
        // substring match) to tell the two cards apart:
        //   - Chaos Orb:    `... | SubAbility$ DBDestroyTouched`  (DestroyAll)
        //     -> destroy TARGET nontoken permanent. The upstream chain's
        //        `DBDestroyChaosOrb` (`Destroy | Defined$ Self`) still
        //        self-destroys the Orb; the intervening `DestroyAll` over the
        //        (always-empty) Remembered set is a harmless no-op.
        //   - Falling Star: `... | SubAbility$ DBDamageTouched`   (DamageAll 3)
        //     -> deal 3 to TARGET creature. The chained `DBTapAllDamaged`
        //        (`TapAll | ValidCards$ Creature.IsRemembered+DamagedBy`) is
        //        converted to a survival-gated `Tap | Defined$ Targeted` (see
        //        the TapAll arm) so a surviving creature is tapped and a dead
        //        one is not.
        ApiType::Unknown(ref s) if s == "FlipOntoBattlefield" => {
            match params.get("SubAbility") {
                // Falling Star: deal 3 damage to target creature.
                Some("DBDamageTouched") => Some(Effect::DealDamage {
                    target: TargetRef::None, // resolved from the chosen creature target
                    amount: 3,
                }),
                // Chaos Orb (and the default): destroy target nontoken permanent.
                _ => {
                    let mut restriction = TargetRestriction::any();
                    restriction.requires_nontoken = true;
                    Some(Effect::DestroyPermanent {
                        target: CardId::placeholder(),
                        restriction,
                        no_regenerate: false,
                    })
                }
            }
        }

        // `StoreSVar` (Forge pseudo-API: stash a computed value into a card SVar
        // for a later effect to read) is an INTENTIONAL no-op in this engine.
        // Drain Life uses a StoreSVar chain only to compute its life-gain cap
        // (`Limit` = target toughness/loyalty/life); we read that cap directly
        // from the pre-damage target snapshot (DamageDealtCappedByTarget) instead
        // of maintaining a runtime SVar store, so the StoreSVar nodes do nothing.
        // Emit NoOp (silent) rather than Unimplemented (which warns about a gap).
        ApiType::Unknown(ref s) if s == "StoreSVar" => Some(Effect::NoOp {
            api_type: "StoreSVar".to_string(),
        }),

        // Recognized but not yet implemented API types produce an Unimplemented effect
        // so that spell resolution can warn instead of silently no-op'ing
        _ => {
            let api_name = params.api_type.as_str().to_string();
            log::debug!(
                target: "effect_converter",
                "API type '{}' not yet implemented, producing Unimplemented effect",
                api_name
            );
            Some(Effect::Unimplemented { api_type: api_name })
        }
    }
}

/// Convert ability parameters to an Effect, applying UnlessCost wrapping if present
///
/// This is the main entry point for effect parsing. It:
/// 1. Parses the base effect using params_to_effect
/// 2. Wraps with UnlessCostWrapper if UnlessCost$ parameter is present
///
/// Use this instead of params_to_effect when parsing spell abilities.
pub fn params_to_effect_with_unless(params: &AbilityParams) -> Option<Effect> {
    let effect = params_to_effect(params)?;
    Some(wrap_with_unless_cost(effect, params))
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
                        StaticAbilityMode::MustAttack => {
                            // SP$ Effect granting a temporary MustAttack static
                            // (Siren's Call B16 — mtg-713). Not yet wired through
                            // this temporary-effect path; the intrinsic
                            // self-keyword form (Juggernaut) is handled at card
                            // load in parse_keywords instead.
                            // TODO(mtg-713): grant temporary MustAttack via SP$ Effect.
                            log::debug!(
                                target: "effect_converter",
                                "Mode$ MustAttack via SP$ Effect not yet implemented (SVar: {})",
                                static_ability_name
                            );
                        }
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

    // DealDamage: when NumDmg$ X refers to a Count$ValidGraveyard (or any other
    // non-xPaid Count$) SVar, emit DealDamageDynamic instead of DealDamageXPaid.
    // `params_to_effect` can't do this because it has no SVar access.
    if params.api_type == ApiType::DealDamage && params.get("NumDmg") == Some("X") {
        if let Some(svar_body) = svars.get("X") {
            // Only intercept Count$... bodies that are NOT "Count$xPaid"
            // (xPaid → DealDamageXPaid, handled by params_to_effect).
            if svar_body.starts_with("Count$") && svar_body != "Count$xPaid" {
                let count = crate::core::CountExpression::parse("X", svars);
                if !matches!(count, crate::core::CountExpression::Fixed(0)) {
                    let target = match params.get("Defined") {
                        Some("You") => TargetRef::Player(crate::core::PlayerId::new(0)),
                        _ => TargetRef::None,
                    };
                    return Some(Effect::DealDamageDynamic { target, count });
                }
            }
        }
    }

    // For all other types, delegate to the base function
    params_to_effect(params)
}

/// Convert DelayedTrigger ability parameters to a CreateDelayedTrigger Effect with SVar resolution.
///
/// This resolves the Execute$ SVar to get the actual effect to execute when triggered.
///
/// # Arguments
///
/// * `params` - The parsed ability parameters (must be ApiType::DelayedTrigger)
/// * `svars` - The card's SVar definitions (name -> body)
///
/// # Example
///
/// ```ignore
/// // Card has: A:SP$ DelayedTrigger | Mode$ ChangesZone | Execute$ TrigEarthbend
/// // And SVar:TrigEarthbend:DB$ Earthbend | Num$ 4
/// let params = AbilityParams::parse("A:SP$ DelayedTrigger | ...")?;
/// let effect = params_to_delayed_trigger_with_svars(&params, &card.svars);
/// // Returns Effect::CreateDelayedTrigger with Effect::Earthbend inside
/// ```
pub fn params_to_delayed_trigger_with_svars(params: &AbilityParams, svars: &HashMap<String, String>) -> Option<Effect> {
    if params.api_type != ApiType::DelayedTrigger {
        return None;
    }

    let mode = params.get("Mode").unwrap_or("ChangesZone");

    // Parse trigger condition based on Mode$
    let condition = if mode == "ChangesZone" {
        let from_zone = params
            .get("Origin")
            .and_then(crate::zones::Zone::from_str_lenient)
            .unwrap_or(crate::zones::Zone::Battlefield); // Default for death triggers

        let to_zone = params
            .get("Destination")
            .and_then(crate::zones::Zone::from_str_lenient)
            .unwrap_or(crate::zones::Zone::Graveyard); // Default for death triggers

        crate::core::DelayedTriggerCondition::ZoneChange {
            from_zones: smallvec::smallvec![from_zone],
            to_zones: smallvec::smallvec![to_zone],
        }
    } else if mode == "SpellCast" {
        // SpellCast trigger: fires when a matching spell is cast
        // Example: "When you next cast a Lesson spell this turn"
        // Parameters:
        // - ValidCard$: Card type filter (e.g., "Lesson", "Creature", "Noncreature")
        // - ValidActivatingPlayer$: Who can trigger (You, Opponent, Any)
        let valid_card_type = params.get("ValidCard").map(|s| s.to_string());
        let you_only = params.get("ValidActivatingPlayer") == Some("You");

        crate::core::DelayedTriggerCondition::SpellCast {
            valid_card_type,
            you_only,
        }
    } else if mode == "Phase" {
        // Phase trigger: fires at the beginning of one of a set of phases.
        // Example: Mana Drain's "At the beginning of your next main phase"
        //   DB$ DelayedTrigger | Mode$ Phase | Phase$ Main1,Main2 | ValidPlayer$ You
        // Parameters:
        // - Phase$: comma-separated phase names (Upkeep, Draw, Main1, Main2,
        //   BeginCombat, EndCombat, End/EndStep, Cleanup)
        // - ValidPlayer$: Whose turn fires it (You, Opponent, Any)
        let phases: smallvec::SmallVec<[crate::core::TriggerPhase; 2]> = params
            .get("Phase")
            .into_iter()
            .flat_map(|spec| spec.split(','))
            .filter_map(|name| crate::core::TriggerPhase::from_script_name(name.trim()))
            .collect();
        if phases.is_empty() {
            log::debug!(
                target: "effect_converter",
                "DelayedTrigger Mode$ Phase with no recognized Phase$ value: {:?}",
                params.get("Phase")
            );
            return None;
        }
        let whose_turn = match params.get("ValidPlayer") {
            Some("You") => crate::core::TurnOwner::You,
            Some("Opponent") => crate::core::TurnOwner::Opponent,
            _ => crate::core::TurnOwner::Any,
        };
        crate::core::DelayedTriggerCondition::Phase { phases, whose_turn }
    } else {
        log::debug!(
            target: "effect_converter",
            "DelayedTrigger Mode$ '{}' not yet supported with SVar resolution",
            mode
        );
        return None;
    };

    // Parse expiry
    let expiry = if params.get("ThisTurn") == Some("True") {
        Some(crate::core::DelayedTriggerExpiry::EndOfTurn)
    } else {
        None
    };

    // Resolve Execute$ SVar to get the actual effect
    let execute_svar = params.get("Execute")?;
    let svar_body = svars.get(execute_svar)?;

    // Parse the SVar as an ability
    let execute_params = AbilityParams::parse(&format!("A:{}", svar_body)).ok()?;
    let execute_effect = params_to_effect(&execute_params)?;

    log::debug!(
        target: "effect_converter",
        "DelayedTrigger with SVar resolution: mode={}, execute_svar={}, effect={:?}",
        mode, execute_svar, execute_effect
    );

    Some(Effect::CreateDelayedTrigger {
        tracked_card: CardId::new(0), // Placeholder - filled in at cast time
        condition,
        effect: Box::new(execute_effect),
        expiry,
    })
}

/// Convert ImmediateTrigger ability parameters to an ImmediateTrigger Effect with SVar resolution.
///
/// This resolves the Execute$ SVar to get the actual effect to execute when the condition is met.
///
/// # Arguments
///
/// * `params` - The parsed ability parameters (must be ApiType::ImmediateTrigger)
/// * `svars` - The card's SVar definitions (name -> body)
///
/// # Example
///
/// ```ignore
/// // Card has: DB$ ImmediateTrigger | ConditionDefined$ Remembered | ConditionPresent$ Card.nonLand | Execute$ TrigPutCounter
/// // And SVar:TrigPutCounter:DB$ PutCounter | ValidTgts$ Creature.YouCtrl | CounterType$ P1P1 | CounterNum$ 1
/// let params = AbilityParams::parse("A:DB$ ImmediateTrigger | ...")?;
/// let effect = params_to_immediate_trigger_with_svars(&params, &card.svars);
/// // Returns Effect::ImmediateTrigger with Effect::PutCounter inside
/// ```
pub fn params_to_immediate_trigger_with_svars(
    params: &AbilityParams,
    svars: &HashMap<String, String>,
) -> Option<Effect> {
    if params.api_type != ApiType::ImmediateTrigger {
        return None;
    }

    // Parse condition based on ConditionPresent$
    let condition_present = params.get("ConditionPresent");
    let condition = match condition_present {
        Some("Card.nonLand") => crate::core::ImmediateTriggerCondition::RememberedNonLand,
        _ => crate::core::ImmediateTriggerCondition::AnyRemembered,
    };

    // Resolve Execute$ SVar to get the actual effect
    let execute_svar = params.get("Execute")?;
    let svar_body = svars.get(execute_svar)?;

    // Parse the SVar as an ability
    let execute_params = AbilityParams::parse(&format!("A:{}", svar_body)).ok()?;
    let execute_effect = params_to_effect(&execute_params)?;

    log::debug!(
        target: "effect_converter",
        "ImmediateTrigger with SVar resolution: condition={:?}, execute_svar={}, effect={:?}",
        condition, execute_svar, execute_effect
    );

    Some(Effect::ImmediateTrigger {
        condition,
        sub_effects: vec![execute_effect],
    })
}

/// Parse UnlessCost parameters from ability params
///
/// Parses:
/// - UnlessCost$ <cost> - the cost to pay (mana, Discard<N>, Sac<N/Type>, etc.)
/// - UnlessPayer$ <player> - who pays (default: TargetedController)
/// - UnlessSwitched$ True - if present, effect executes when paid (default: when NOT paid)
///
/// Returns None if no UnlessCost$ parameter is present.
fn parse_unless_cost(params: &AbilityParams) -> Option<crate::core::effects::UnlessCost> {
    use crate::core::effects::{UnlessCost, UnlessCostType};

    let cost_str = params.get("UnlessCost")?;

    // Parse the cost type
    let cost_type = if cost_str.starts_with("Discard<") && cost_str.ends_with('>') {
        // Format: Discard<N/Type> (e.g., "Discard<1/Card>")
        let inner = &cost_str[8..cost_str.len() - 1];
        let parts: Vec<&str> = inner.split('/').collect();
        let count = parts.first().and_then(|s| s.parse::<u8>().ok()).unwrap_or(1);
        let card_type = parts.get(1).unwrap_or(&"Card").to_string();
        UnlessCostType::Discard { count, card_type }
    } else if cost_str.starts_with("Sac<") && cost_str.ends_with('>') {
        // Format: Sac<N/Type> (e.g., "Sac<1/Creature>")
        let inner = &cost_str[4..cost_str.len() - 1];
        let parts: Vec<&str> = inner.split('/').collect();
        let count = parts.first().and_then(|s| s.parse::<u8>().ok()).unwrap_or(1);
        let valid_type = parts.get(1).unwrap_or(&"Permanent").to_string();
        UnlessCostType::Sacrifice { count, valid_type }
    } else if cost_str.starts_with("PayLife<") && cost_str.ends_with('>') {
        // Format: PayLife<N> (e.g., "PayLife<3>")
        let amount_str = &cost_str[8..cost_str.len() - 1];
        let amount = amount_str.parse::<u8>().unwrap_or(1);
        UnlessCostType::PayLife(amount)
    } else if cost_str.starts_with("Reveal<") && cost_str.ends_with('>') {
        // Format: Reveal<N/Type> (e.g., "Reveal<1/Giant>")
        let inner = &cost_str[7..cost_str.len() - 1];
        let parts: Vec<&str> = inner.split('/').collect();
        let count = parts.first().and_then(|s| s.parse::<u8>().ok()).unwrap_or(1);
        let card_type = parts.get(1).unwrap_or(&"Card").to_string();
        UnlessCostType::Reveal { count, card_type }
    } else {
        // Assume it's a mana cost (e.g., "2", "1U", "X")
        let mana_cost = crate::core::ManaCost::from_string(cost_str);
        UnlessCostType::Mana(mana_cost)
    };

    // Parse who pays (default to TargetedController for counter spells)
    let payer = params.get("UnlessPayer").unwrap_or("TargetedController");

    // Parse whether the logic is switched
    let switched = params.get("UnlessSwitched") == Some("True");

    Some(UnlessCost::new(cost_type, payer, switched))
}

/// Wrap an effect with an UnlessCost condition if the params specify one
///
/// If UnlessCost$ is present in params, wraps the effect in an UnlessCostWrapper.
/// Otherwise, returns the effect unchanged.
pub fn wrap_with_unless_cost(effect: Effect, params: &AbilityParams) -> Effect {
    if let Some(unless_cost) = parse_unless_cost(params) {
        log::debug!(
            target: "effect_converter",
            "Wrapping effect with UnlessCost: cost={:?}, payer={}, switched={}",
            unless_cost.cost, unless_cost.payer, unless_cost.switched
        );
        Effect::UnlessCostWrapper {
            inner_effect: Box::new(effect),
            unless_cost,
        }
    } else {
        effect
    }
}

#[cfg(test)]
#[allow(clippy::wildcard_enum_match_arm)] // Tests use wildcards in panic branches
mod tests {
    use super::*;
    use crate::core::Color;

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
    fn test_convert_damage_resolve_is_noop() {
        // DB$ DamageResolve is a marker — see ApiType::DamageResolve doc.
        // It must (a) be recognized (no Unknown / Unimplemented) and (b)
        // contribute no Effect to a SubAbility chain.
        let params = AbilityParams::parse("A:DB$ DamageResolve").unwrap();
        assert_eq!(
            params.api_type,
            ApiType::DamageResolve,
            "DamageResolve must parse to ApiType::DamageResolve, not Unknown"
        );

        let effect = params_to_effect(&params);
        assert!(
            effect.is_none(),
            "DamageResolve must produce no effect (no-op marker), got {:?}",
            effect
        );
    }

    #[test]
    fn test_psionic_blast_chain_parses() {
        // Psionic Blast is the canonical DamageResolve consumer:
        //   A:SP$ DealDamage | ValidTgts$ Any | NumDmg$ 4 | DamageMap$ True | SubAbility$ DBDealDamage
        //   SVar:DBDealDamage:DB$ DealDamage | Defined$ You | NumDmg$ 2 | SubAbility$ DBDamageResolve
        //   SVar:DBDamageResolve:DB$ DamageResolve
        // This test exercises only the head of the chain; full SubAbility
        // resolution is covered by the e2e gameplay log captured in the
        // commit message (no SBA-relevant ordering issues for these targets).
        let params = AbilityParams::parse(
            "A:SP$ DealDamage | ValidTgts$ Any | NumDmg$ 4 | DamageMap$ True | SubAbility$ DBDealDamage",
        )
        .unwrap();
        let effect = params_to_effect(&params).expect("head DealDamage must convert");
        match effect {
            Effect::DealDamage { amount, .. } => assert_eq!(amount, 4),
            other => panic!("Expected DealDamage(4), got {:?}", other),
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
                power_bonus,
                toughness_bonus,
                ..
            } => {
                assert_eq!(power_bonus, 3);
                assert_eq!(toughness_bonus, 2);
            }
            _ => panic!("Expected PumpCreature effect"),
        }
    }

    #[test]
    fn test_convert_pump_with_keyword() {
        use crate::core::Keyword;

        // KW$ Double Strike only (no stat bonuses)
        let params = AbilityParams::parse("A:DB$ Pump | Defined$ Targeted | KW$ Double Strike").unwrap();
        let effect = params_to_effect(&params).unwrap();

        match effect {
            Effect::PumpCreature {
                power_bonus,
                toughness_bonus,
                keywords_granted,
                ..
            } => {
                assert_eq!(power_bonus, 0, "Power bonus should be 0");
                assert_eq!(toughness_bonus, 0, "Toughness bonus should be 0");
                assert_eq!(keywords_granted.len(), 1, "Should have 1 keyword");
                assert!(
                    keywords_granted.contains(&Keyword::DoubleStrike),
                    "Should grant Double Strike"
                );
            }
            _ => panic!("Expected PumpCreature effect"),
        }
    }

    #[test]
    fn test_convert_pump_with_multiple_keywords() {
        use crate::core::Keyword;

        // KW$ Flying & Haste (multiple keywords)
        let params = AbilityParams::parse("A:SP$ Pump | NumAtt$ +1 | NumDef$ +1 | KW$ Flying & Haste").unwrap();
        let effect = params_to_effect(&params).unwrap();

        match effect {
            Effect::PumpCreature {
                power_bonus,
                toughness_bonus,
                keywords_granted,
                ..
            } => {
                assert_eq!(power_bonus, 1, "Power bonus should be +1");
                assert_eq!(toughness_bonus, 1, "Toughness bonus should be +1");
                assert_eq!(keywords_granted.len(), 2, "Should have 2 keywords");
                assert!(keywords_granted.contains(&Keyword::Flying), "Should grant Flying");
                assert!(keywords_granted.contains(&Keyword::Haste), "Should grant Haste");
            }
            _ => panic!("Expected PumpCreature effect"),
        }
    }

    #[test]
    fn test_cunning_maneuver_effects() {
        use crate::core::PlayerId;
        use crate::loader::card::CardLoader;

        let content = r#"
Name:Cunning Maneuver
ManaCost:1 R
Types:Instant
A:SP$ Pump | ValidTgts$ Creature | NumAtt$ +3 | NumDef$ +1 | SubAbility$ DBToken | SpellDescription$ Target creature gets +3/+1 until end of turn. Create a Clue token.
SVar:DBToken:DB$ Token | TokenScript$ c_a_clue_draw | TokenOwner$ You
Oracle:Target creature gets +3/+1 until end of turn. Create a Clue token.
"#;

        let def = CardLoader::parse(content).expect("Failed to parse Cunning Maneuver");
        let card = def.instantiate(crate::core::CardId::new(1), PlayerId::new(0));

        eprintln!("Cunning Maneuver has {} effects:", card.effects.len());
        for (i, effect) in card.effects.iter().enumerate() {
            eprintln!("  {}: {:?}", i, effect);
        }

        // Should have 2 effects: PumpCreature and CreateToken
        assert_eq!(card.effects.len(), 2, "Cunning Maneuver should have 2 effects");

        // First effect should be PumpCreature
        match &card.effects[0] {
            Effect::PumpCreature {
                target,
                power_bonus,
                toughness_bonus,
                ..
            } => {
                assert_eq!(target.as_u32(), 0, "Target should be placeholder 0");
                assert_eq!(*power_bonus, 3, "Power bonus should be +3");
                assert_eq!(*toughness_bonus, 1, "Toughness bonus should be +1");
            }
            other => panic!("First effect should be PumpCreature, got {:?}", other),
        }

        // Second effect should be CreateToken
        match &card.effects[1] {
            Effect::CreateToken {
                controller: _,
                token_script,
                amount,
                for_each_player,
            } => {
                assert_eq!(token_script, "c_a_clue_draw");
                assert_eq!(*amount, 1);
                assert!(!*for_each_player, "TokenOwner$ You should not set for_each_player");
            }
            other => panic!("Second effect should be CreateToken, got {:?}", other),
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
        // Unknown API types should now return Unimplemented variant (not None)
        let params = AbilityParams::parse("A:SP$ UnsupportedAbility | Foo$ Bar").unwrap();
        let effect = params_to_effect(&params);

        assert!(
            effect.is_some(),
            "Unsupported types should produce Unimplemented variant"
        );
        match effect.unwrap() {
            Effect::Unimplemented { api_type } => {
                assert_eq!(api_type, "UnsupportedAbility");
            }
            _ => panic!("Expected Unimplemented effect"),
        }
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
                // "Any" counter type now means None (any counter)
                assert_eq!(counter_type, None);
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
                assert_eq!(counter_type, Some(CounterType::P1P1));
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
    fn test_convert_clone_copy_artifact() {
        // Copy Artifact's DB$ Clone SVar body. Verifies the new ApiType::Clone
        // parses and the Choices$/AddTypes$ params map onto Effect::Clone.
        let params = AbilityParams::parse(
            "A:DB$ Clone | Choices$ Artifact.Other | AddTypes$ Enchantment | SpellDescription$ You may have CARDNAME enter as a copy of any artifact on the battlefield, except it's an enchantment in addition to its other types."
        ).unwrap();
        assert_eq!(
            params.api_type,
            crate::loader::ability_parser::ApiType::Clone,
            "DB$ Clone must parse to ApiType::Clone"
        );

        let effect = params_to_effect(&params).expect("Clone should convert to an Effect");
        match effect {
            Effect::Clone {
                choices_filter,
                add_types,
                optional,
                ..
            } => {
                // Choices$ Artifact.Other -> Artifact type restriction
                assert!(
                    choices_filter
                        .types
                        .iter()
                        .any(|t| matches!(t, crate::core::TargetType::Artifact)),
                    "Choices$ Artifact.Other should restrict to artifacts"
                );
                // AddTypes$ Enchantment -> [CardType::Enchantment]
                assert_eq!(
                    add_types.as_slice(),
                    &[crate::core::CardType::Enchantment],
                    "AddTypes$ Enchantment should add the Enchantment card type"
                );
                // The SVar body alone does not set optional; the surrounding
                // ETBReplacement keyword's `Optional` flag does.
                assert!(!optional, "SVar body should not set optional");
            }
            other => panic!("Expected Effect::Clone, got {other:?}"),
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

    #[test]
    fn test_unless_cost_discard() {
        use crate::core::effects::UnlessCostType;

        // Test Abandon Attachments pattern: "You may discard. If you do, draw 2"
        let params = AbilityParams::parse(
            "A:SP$ Draw | NumCards$ 2 | UnlessCost$ Discard<1/Card> | UnlessPayer$ You | UnlessSwitched$ True",
        )
        .unwrap();

        let effect = params_to_effect_with_unless(&params).unwrap();

        match effect {
            Effect::UnlessCostWrapper {
                inner_effect,
                unless_cost,
            } => {
                // Check inner effect is DrawCards
                match *inner_effect {
                    Effect::DrawCards { count, .. } => {
                        assert_eq!(count, 2, "Should draw 2 cards");
                    }
                    _ => panic!("Inner effect should be DrawCards"),
                }

                // Check UnlessCost
                assert!(unless_cost.switched, "Should be switched (pay to get effect)");
                assert_eq!(unless_cost.payer, "You", "Payer should be 'You'");

                match unless_cost.cost {
                    UnlessCostType::Discard { count, ref card_type } => {
                        assert_eq!(count, 1, "Should discard 1 card");
                        assert_eq!(card_type, "Card", "Should be any card");
                    }
                    _ => panic!("Cost should be Discard"),
                }
            }
            _ => panic!("Expected UnlessCostWrapper effect"),
        }
    }

    #[test]
    fn test_unless_cost_mana() {
        // Test counter spell pattern: "Counter unless controller pays 2"
        let params = AbilityParams::parse("A:SP$ Counter | UnlessCost$ 2").unwrap();

        let effect = params_to_effect_with_unless(&params).unwrap();

        match effect {
            Effect::UnlessCostWrapper {
                inner_effect,
                unless_cost,
            } => {
                // Check inner effect is CounterSpell
                assert!(matches!(*inner_effect, Effect::CounterSpell { .. }));

                // Check UnlessCost
                assert!(!unless_cost.switched, "Should not be switched (effect if NOT paid)");
                assert_eq!(unless_cost.payer, "TargetedController", "Default payer");
            }
            _ => panic!("Expected UnlessCostWrapper effect"),
        }
    }

    #[test]
    fn test_no_unless_cost() {
        // Test regular Draw without UnlessCost
        let params = AbilityParams::parse("A:SP$ Draw | NumCards$ 3").unwrap();

        let effect = params_to_effect_with_unless(&params).unwrap();

        // Should not be wrapped
        match effect {
            Effect::DrawCards { count, .. } => {
                assert_eq!(count, 3, "Should draw 3 cards");
            }
            Effect::UnlessCostWrapper { .. } => panic!("Should not be wrapped"),
            _ => panic!("Expected DrawCards effect"),
        }
    }

    #[test]
    fn test_pump_with_sub_ability_creates_effect() {
        // Prey Upon uses SP$ Pump with +0/+0 purely as a targeting vehicle for DB$ Fight
        // It must still create a PumpCreature effect so targets get collected
        let ability = "A:SP$ Pump | ValidTgts$ Creature.YouCtrl | SubAbility$ DBFight | StackDescription$ None";
        let params = AbilityParams::parse(ability).unwrap();
        let effect = params_to_effect(&params);
        assert!(
            effect.is_some(),
            "Pump with SubAbility should create PumpCreature effect even with +0/+0"
        );
        match effect.unwrap() {
            Effect::PumpCreature {
                power_bonus,
                toughness_bonus,
                ..
            } => {
                assert_eq!(power_bonus, 0);
                assert_eq!(toughness_bonus, 0);
            }
            other => panic!("Expected PumpCreature, got {:?}", other),
        }
    }

    #[test]
    fn test_pump_without_bonuses_or_sub_ability_returns_none() {
        // A bare Pump with no bonuses and no SubAbility should return None
        let ability = "A:SP$ Pump | ValidTgts$ Creature";
        let params = AbilityParams::parse(ability).unwrap();
        let effect = params_to_effect(&params);
        assert!(
            effect.is_none(),
            "Pump with no bonuses/keywords/SubAbility should return None"
        );
    }

    #[test]
    fn test_convert_choose_color() {
        let ability = "A:AB$ ChooseColor | Cost$ G | Defined$ You | SubAbility$ Animate";
        let params = AbilityParams::parse(ability).unwrap();
        let effect = params_to_effect(&params).unwrap();

        match effect {
            Effect::ChooseColor { player, source } => {
                // Player should be placeholder (0) - resolved at cast time
                assert_eq!(player.as_u32(), 0);
                // Source should be placeholder - resolved at cast time
                assert!(source.is_placeholder());
            }
            _ => panic!("Expected ChooseColor effect, got {:?}", effect),
        }
    }

    #[test]
    fn test_choose_color_from_cardsfolder() {
        // Test that Caldera Kavu's ChooseColor ability parses correctly
        let ability = "A:AB$ ChooseColor | Cost$ G | Defined$ You | SpellDescription$ CARDNAME becomes the color of your choice until end of turn.";
        let params = AbilityParams::parse(ability).unwrap();
        assert_eq!(params.api_type, ApiType::ChooseColor);
        let effect = params_to_effect(&params);
        assert!(effect.is_some(), "ChooseColor should produce an effect");
    }

    #[test]
    fn test_convert_debuff_single_keyword() {
        // Grozoth: "AB$ Debuff | Cost$ 4 | Keywords$ Defender | Defined$ Self"
        let params = AbilityParams::parse("A:AB$ Debuff | Cost$ 4 | Keywords$ Defender | Defined$ Self").unwrap();
        assert_eq!(params.api_type, ApiType::Debuff);
        let effect = params_to_effect(&params).unwrap();
        match effect {
            Effect::DebuffCreature { keywords_removed, .. } => {
                assert_eq!(keywords_removed.len(), 1);
                assert_eq!(keywords_removed[0], Keyword::Defender);
            }
            _ => panic!("Expected DebuffCreature effect"),
        }
    }

    #[test]
    fn test_convert_debuff_flying() {
        // Swooping Talon: "AB$ Debuff | Cost$ 1 | Keywords$ Flying | Defined$ Self"
        let params = AbilityParams::parse("A:AB$ Debuff | Cost$ 1 | Keywords$ Flying | Defined$ Self").unwrap();
        let effect = params_to_effect(&params).unwrap();
        match effect {
            Effect::DebuffCreature { keywords_removed, .. } => {
                assert_eq!(keywords_removed.len(), 1);
                assert_eq!(keywords_removed[0], Keyword::Flying);
            }
            _ => panic!("Expected DebuffCreature effect"),
        }
    }

    #[test]
    fn test_convert_debuff_with_sub_ability() {
        // Manor Gargoyle: "AB$ Debuff | Cost$ 1 | Keywords$ Defender | Defined$ Self | SubAbility$ DBFlight"
        let params =
            AbilityParams::parse("A:AB$ Debuff | Cost$ 1 | Keywords$ Defender | Defined$ Self | SubAbility$ DBFlight")
                .unwrap();
        let effect = params_to_effect(&params).unwrap();
        assert!(
            matches!(effect, Effect::DebuffCreature { .. }),
            "Debuff with SubAbility should produce DebuffCreature effect"
        );
    }

    #[test]
    fn test_convert_proliferate_basic() {
        // Yawgmoth, Thran Physician: "AB$ Proliferate | Cost$ B B Discard<1/Card>"
        let params =
            AbilityParams::parse("A:AB$ Proliferate | Cost$ B B Discard<1/Card> | SpellDescription$ Proliferate.")
                .unwrap();
        assert_eq!(params.api_type, ApiType::Proliferate);
        let effect = params_to_effect(&params).unwrap();
        assert!(matches!(effect, Effect::Proliferate), "Expected Proliferate effect");
    }

    #[test]
    fn test_convert_proliferate_no_cost() {
        // Proliferate as a sub-ability (no cost): "A:DB$ Proliferate"
        let params = AbilityParams::parse("A:DB$ Proliferate").unwrap();
        assert_eq!(params.api_type, ApiType::Proliferate);
        let effect = params_to_effect(&params).unwrap();
        assert!(
            matches!(effect, Effect::Proliferate),
            "Expected Proliferate effect from DB$"
        );
    }

    #[test]
    fn test_convert_proliferate_with_sub_ability() {
        // Proliferate with chained SubAbility
        let params = AbilityParams::parse("A:DB$ Proliferate | SubAbility$ DBDraw").unwrap();
        let effect = params_to_effect(&params).unwrap();
        assert!(
            matches!(effect, Effect::Proliferate),
            "Proliferate with SubAbility should parse"
        );
    }

    #[test]
    fn test_convert_animate_all_power_toughness_keywords() {
        // Sarkhan the Masterless: planeswalkers become 4/4 Dragons with Flying
        let params = AbilityParams::parse(
            "A:AB$ AnimateAll | ValidCards$ Planeswalker.YouCtrl | Power$ 4 | Toughness$ 4 | Keywords$ Flying | AILogic$ Always",
        )
        .unwrap();
        assert_eq!(params.api_type, ApiType::AnimateAll);
        let effect = params_to_effect(&params).unwrap();
        match effect {
            Effect::AnimateAll {
                filter,
                power,
                toughness,
                keywords_granted,
                ..
            } => {
                assert_eq!(filter, "Planeswalker.YouCtrl");
                assert_eq!(power, Some(4));
                assert_eq!(toughness, Some(4));
                assert_eq!(keywords_granted.len(), 1);
                assert_eq!(keywords_granted[0], Keyword::Flying);
            }
            _ => panic!("Expected AnimateAll effect"),
        }
    }

    #[test]
    fn test_convert_animate_all_keywords_only() {
        // Vraska: creatures gain Deathtouch
        let params =
            AbilityParams::parse("A:AB$ AnimateAll | ValidCards$ Creature.YouCtrl | Keywords$ Deathtouch").unwrap();
        let effect = params_to_effect(&params).unwrap();
        match effect {
            Effect::AnimateAll {
                filter,
                power,
                toughness,
                keywords_granted,
                ..
            } => {
                assert_eq!(filter, "Creature.YouCtrl");
                assert_eq!(power, None);
                assert_eq!(toughness, None);
                assert_eq!(keywords_granted.len(), 1);
                assert_eq!(keywords_granted[0], Keyword::Deathtouch);
            }
            _ => panic!("Expected AnimateAll effect"),
        }
    }

    #[test]
    fn test_convert_animate_all_multiple_keywords() {
        // Oko-style: creatures become 10/10 with Trample
        let params = AbilityParams::parse(
            "A:AB$ AnimateAll | ValidCards$ Creature.YouCtrl | Power$ 10 | Toughness$ 10 | Keywords$ Trample",
        )
        .unwrap();
        let effect = params_to_effect(&params).unwrap();
        match effect {
            Effect::AnimateAll {
                power,
                toughness,
                keywords_granted,
                ..
            } => {
                assert_eq!(power, Some(10));
                assert_eq!(toughness, Some(10));
                assert_eq!(keywords_granted.len(), 1);
                assert_eq!(keywords_granted[0], Keyword::Trample);
            }
            _ => panic!("Expected AnimateAll effect"),
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // AB$ PreventDamage parsing tests
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn test_convert_prevent_damage_any_target() {
        // Militant Monk: "AB$ PreventDamage | Cost$ T | ValidTgts$ Any | Amount$ 1"
        let params = AbilityParams::parse(
            "A:AB$ PreventDamage | Cost$ T | ValidTgts$ Any | Amount$ 1 | SpellDescription$ Prevent the next 1 damage.",
        )
        .unwrap();
        assert_eq!(params.api_type, ApiType::PreventDamage);
        let effect = params_to_effect(&params).unwrap();
        assert!(
            matches!(
                effect,
                Effect::PreventDamage {
                    target: TargetRef::None,
                    amount: 1
                }
            ),
            "Expected PreventDamage with amount 1 and no pre-resolved target, got {:?}",
            effect
        );
    }

    #[test]
    fn test_convert_prevent_damage_defined_self() {
        // Ursine Fylgja: "AB$ PreventDamage | Cost$ SubCounter<1/HEALING> | Defined$ Self | Amount$ 1"
        let params =
            AbilityParams::parse("A:AB$ PreventDamage | Cost$ SubCounter<1/HEALING> | Defined$ Self | Amount$ 1")
                .unwrap();
        assert_eq!(params.api_type, ApiType::PreventDamage);
        let effect = params_to_effect(&params).unwrap();
        assert!(
            matches!(
                effect,
                Effect::PreventDamage {
                    target: TargetRef::Permanent(_),
                    amount: 1
                }
            ),
            "Expected PreventDamage targeting self (Permanent placeholder), got {:?}",
            effect
        );
    }

    #[test]
    fn test_convert_prevent_damage_defined_you() {
        // Esper Battlemage: "AB$ PreventDamage | Cost$ W T | Defined$ You | Amount$ 2"
        let params = AbilityParams::parse("A:AB$ PreventDamage | Cost$ W T | Defined$ You | Amount$ 2").unwrap();
        assert_eq!(params.api_type, ApiType::PreventDamage);
        let effect = params_to_effect(&params).unwrap();
        assert!(
            matches!(
                effect,
                Effect::PreventDamage {
                    target: TargetRef::Player(_),
                    amount: 2
                }
            ),
            "Expected PreventDamage targeting player with amount 2, got {:?}",
            effect
        );
    }

    #[test]
    fn test_convert_prevent_damage_amount_4() {
        // Master Healer: "AB$ PreventDamage | Cost$ T | ValidTgts$ Any | Amount$ 4"
        let params = AbilityParams::parse("A:AB$ PreventDamage | Cost$ T | ValidTgts$ Any | Amount$ 4").unwrap();
        let effect = params_to_effect(&params).unwrap();
        assert!(
            matches!(effect, Effect::PreventDamage { amount: 4, .. }),
            "Expected PreventDamage with amount 4, got {:?}",
            effect
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // AB$ ChooseSource (Circle of Protection family) parsing tests
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn test_card_compat_circle_of_protection_red() {
        // Circle of Protection: Red — the source-filtered prevention construct.
        // "A:AB$ ChooseSource | Cost$ 1 | Choices$ Card.RedSource | ... | SubAbility$ DBEffect"
        // collapses to a single Effect::PreventDamageFromSource whose colour is
        // read (tokenized, not substring-matched) from `Choices$ Card.RedSource`.
        let params = AbilityParams::parse(
            "A:AB$ ChooseSource | Cost$ 1 | Choices$ Card.RedSource | AILogic$ NeedsPrevention \
             | SubAbility$ DBEffect | SpellDescription$ The next time a red source of your choice \
             would deal damage to you this turn, prevent that damage.",
        )
        .unwrap();
        assert_eq!(params.api_type, ApiType::ChooseSource);
        let effect = params_to_effect(&params).expect("CoP:Red should produce an effect");
        match effect {
            Effect::PreventDamageFromSource {
                color,
                source,
                protected,
            } => {
                assert_eq!(color, Color::Red, "CoP:Red filters on red sources");
                assert!(source.is_placeholder(), "chosen source resolved at activation");
                assert!(protected.is_placeholder(), "protected player resolved at activation");
            }
            other => panic!("Expected PreventDamageFromSource, got {:?}", other),
        }
    }

    #[test]
    fn test_convert_choose_source_all_circle_colors() {
        // The construct generalizes across the whole Circle-of-Protection
        // family: each colour's `Choices$ Card.<Color>Source` maps to the
        // matching PreventDamageFromSource colour filter.
        for (filter, expected) in [
            ("Card.WhiteSource", Color::White),
            ("Card.BlueSource", Color::Blue),
            ("Card.BlackSource", Color::Black),
            ("Card.RedSource", Color::Red),
            ("Card.GreenSource", Color::Green),
        ] {
            let ability = format!(
                "A:AB$ ChooseSource | Cost$ 1 | Choices$ {} | SubAbility$ DBEffect",
                filter
            );
            let params = AbilityParams::parse(&ability).unwrap();
            let effect = params_to_effect(&params).unwrap_or_else(|| panic!("{} should parse", filter));
            match effect {
                Effect::PreventDamageFromSource { color, .. } => {
                    assert_eq!(color, expected, "{} → {:?}", filter, expected);
                }
                other => panic!("Expected PreventDamageFromSource for {}, got {:?}", filter, other),
            }
        }
    }

    #[test]
    fn test_convert_choose_source_non_cop_returns_none() {
        // Martyr's Cause-style multi-filter chooser (`Choices$ Card,Emblem`) is
        // NOT the single-colour CoP shape, so this arm declines to convert it
        // (returns None) rather than mis-modelling it.
        let params = AbilityParams::parse(
            "A:AB$ ChooseSource | Cost$ Sac<1/Creature> | Choices$ Card,Emblem | SubAbility$ DBEffect",
        )
        .unwrap();
        assert!(
            params_to_effect(&params).is_none(),
            "non-CoP ChooseSource shapes should not convert to PreventDamageFromSource"
        );
    }

    #[test]
    fn test_convert_animate_all_no_effects_returns_none() {
        // AnimateAll with no power, toughness, or keywords should return None
        let params = AbilityParams::parse("A:AB$ AnimateAll | ValidCards$ Creature.YouCtrl").unwrap();
        let effect = params_to_effect(&params);
        assert!(
            effect.is_none(),
            "AnimateAll with no P/T or keywords should return None"
        );
    }

    #[test]
    fn test_convert_add_turn() {
        // Time Walk: "Take an extra turn after this one"
        let params = AbilityParams::parse("A:SP$ AddTurn | NumTurns$ 1").unwrap();
        let effect = params_to_effect(&params);
        assert!(effect.is_some(), "AddTurn should produce an AddTurn effect");

        match effect.unwrap() {
            Effect::AddTurn { player, num_turns } => {
                assert_eq!(num_turns, 1, "Should grant 1 extra turn");
                assert_eq!(player.as_u32(), 0, "Player should be placeholder");
            }
            _ => panic!("Expected AddTurn effect"),
        }
    }

    #[test]
    fn test_convert_destroy_all() {
        // Nevinyrral's Disk: "Destroy all artifacts, creatures, and enchantments"
        let params =
            AbilityParams::parse("A:AB$ DestroyAll | Cost$ 1 T | ValidCards$ Artifact,Creature,Enchantment").unwrap();
        let effect = params_to_effect(&params);
        assert!(effect.is_some(), "DestroyAll should produce a DestroyAll effect");

        match effect.unwrap() {
            Effect::DestroyAll { restriction, .. } => {
                // TargetRestriction should parse the comma-separated types
                assert!(!restriction.types.is_empty(), "Should have type restrictions");
            }
            _ => panic!("Expected DestroyAll effect"),
        }
    }

    #[test]
    fn test_convert_destroy_all_creatures_only() {
        // Wrath of God: "Destroy all creatures"
        let params = AbilityParams::parse("A:SP$ DestroyAll | ValidCards$ Creature").unwrap();
        let effect = params_to_effect(&params);
        assert!(effect.is_some(), "DestroyAll should produce a DestroyAll effect");

        assert!(matches!(effect.unwrap(), Effect::DestroyAll { .. }));
    }

    #[test]
    fn test_convert_chaos_orb_targeted_destroy() {
        // Chaos Orb (mtg-389) is approximated as a single targeted destroy of any
        // NONTOKEN permanent (no controller restriction — the owner may aim it at
        // their own board too), plus a Defined$ Self subability that destroys the
        // Orb. This test pins the primary-ability shape.
        let params = AbilityParams::parse(
            "A:AB$ Destroy | Cost$ 1 T | ActivationZone$ Battlefield | ValidTgts$ Permanent.!token | SubAbility$ DBDestroySelf",
        )
        .unwrap();
        let effect = params_to_effect(&params).expect("Chaos Orb destroy should produce an effect");

        match effect {
            Effect::DestroyPermanent {
                target, restriction, ..
            } => {
                assert!(
                    target.is_placeholder(),
                    "target is a placeholder until cast-time targeting"
                );
                assert!(restriction.requires_nontoken, "must restrict to nontoken permanents");
                assert_eq!(
                    restriction.controller,
                    crate::core::ControllerRestriction::Any,
                    "any controller (owner may target their own permanents)"
                );
            }
            other => panic!("Expected DestroyPermanent, got {:?}", other),
        }
    }

    #[test]
    fn test_convert_chaos_orb_flip_diverges_to_targeted_destroy() {
        // The REAL Chaos Orb card script (cardsfolder/c/chaos_orb.txt) is left
        // UNEDITED and still uses the upstream `AB$ FlipOntoBattlefield |
        // SubAbility$ DBDestroyTouched`. The divergence to a single targeted
        // destroy lives in the FlipOntoBattlefield converter arm. Pin that the
        // real card head converts to a nontoken-restricted DestroyPermanent with
        // no controller restriction (owner may aim at their own board).
        let params = AbilityParams::parse(
            "A:AB$ FlipOntoBattlefield | Cost$ 1 T | SubAbility$ DBDestroyTouched | ActivationZone$ Battlefield",
        )
        .unwrap();
        let effect = params_to_effect(&params).expect("FlipOntoBattlefield (Chaos Orb) must convert");
        match effect {
            Effect::DestroyPermanent {
                target, restriction, ..
            } => {
                assert!(
                    target.is_placeholder(),
                    "target is a placeholder until cast-time targeting"
                );
                assert!(restriction.requires_nontoken, "must restrict to nontoken permanents");
                assert_eq!(
                    restriction.controller,
                    crate::core::ControllerRestriction::Any,
                    "any controller (owner may target their own permanents)"
                );
            }
            other => panic!("Expected DestroyPermanent, got {:?}", other),
        }
    }

    #[test]
    fn test_convert_falling_star_flip_diverges_to_targeted_damage() {
        // The REAL Falling Star card script (cardsfolder/f/falling_star.txt) is
        // left UNEDITED and still uses `A:SP$ FlipOntoBattlefield | SubAbility$
        // DBDamageTouched`. The divergence to a single 3-damage-to-target lives in
        // the FlipOntoBattlefield converter arm, distinguished from Chaos Orb by
        // the tokenized SubAbility$ reference.
        let params =
            AbilityParams::parse("A:SP$ FlipOntoBattlefield | SubAbility$ DBDamageTouched | AILogic$ DamageCreatures")
                .unwrap();
        let effect = params_to_effect(&params).expect("FlipOntoBattlefield (Falling Star) must convert");
        match effect {
            Effect::DealDamage { target, amount } => {
                assert_eq!(amount, 3, "Falling Star deals 3 damage");
                assert!(
                    matches!(target, TargetRef::None),
                    "target resolved from the chosen creature at cast time, got {:?}",
                    target
                );
            }
            other => panic!("Expected DealDamage(3), got {:?}", other),
        }
    }

    #[test]
    fn test_convert_falling_star_tapall_remembered_diverges_to_single_tap() {
        // Falling Star's chained `DBTapAllDamaged` is upstream
        // `TapAll | ValidCards$ Creature.IsRemembered+DamagedBy`. Since the flip
        // is approximated as a single target, the IsRemembered mass-tap converts
        // to a survival-gated single reuse_previous tap (resolved + gated at
        // runtime). Pin that the IsRemembered TapAll becomes a TapPermanent.
        let params = AbilityParams::parse("A:DB$ TapAll | ValidCards$ Creature.IsRemembered+DamagedBy").unwrap();
        let effect = params_to_effect(&params).expect("IsRemembered TapAll must convert");
        match effect {
            Effect::TapPermanent { target } => {
                assert!(
                    target.is_reuse_previous(),
                    "IsRemembered TapAll must reuse the parent damage target"
                );
            }
            other => panic!("Expected TapPermanent reuse_previous, got {:?}", other),
        }
    }

    #[test]
    fn test_convert_chaos_orb_self_destroy_subability() {
        // The DBDestroySelf subability destroys the Orb itself (Defined$ Self).
        let params = AbilityParams::parse("A:DB$ Destroy | Defined$ Self").unwrap();
        let effect = params_to_effect(&params).expect("self-destroy should produce an effect");
        match effect {
            Effect::DestroyPermanent { target, .. } => {
                assert!(
                    target.is_self_target(),
                    "Defined$ Self must produce the self-target sentinel"
                );
            }
            other => panic!("Expected DestroyPermanent self-target, got {:?}", other),
        }
    }

    #[test]
    fn test_convert_falling_star_tap_targeted() {
        // Falling Star (mtg-503): `DB$ Tap | Defined$ Targeted` must reuse the
        // parent ability's target (reuse_previous sentinel) so the chained tap
        // hits the same creature the SP$ DealDamage targeted, not a fresh target.
        let params = AbilityParams::parse("A:DB$ Tap | Defined$ Targeted").unwrap();
        let effect = params_to_effect(&params).expect("Tap Defined$ Targeted should produce an effect");
        match effect {
            Effect::TapPermanent { target } => {
                assert!(
                    target.is_reuse_previous(),
                    "Defined$ Targeted Tap must reuse the parent target (reuse_previous sentinel)"
                );
            }
            other => panic!("Expected TapPermanent reuse_previous, got {:?}", other),
        }
    }

    #[test]
    fn test_convert_psionic_blast_self_damage_placeholder() {
        // Psionic Blast (mtg-533): the "2 damage to you" rider parses to a
        // DealDamage at a PLACEHOLDER player target (Defined$ You -> PlayerId(0)),
        // resolved to the caster at runtime. Pin that the placeholder is produced
        // (the resolution-to-caster is covered by the cross-player e2e test).
        let params = AbilityParams::parse("A:DB$ DealDamage | Defined$ You | NumDmg$ 2").unwrap();
        let effect = params_to_effect(&params).expect("Defined$ You DealDamage should produce an effect");
        match effect {
            Effect::DealDamage { target, amount } => {
                assert_eq!(amount, 2, "self-damage amount is 2");
                match target {
                    TargetRef::Player(pid) => assert!(
                        pid.is_placeholder(),
                        "Defined$ You must encode a placeholder player resolved to the caster"
                    ),
                    other => panic!("Expected TargetRef::Player(placeholder), got {:?}", other),
                }
            }
            other => panic!("Expected DealDamage, got {:?}", other),
        }
    }

    #[test]
    fn test_unimplemented_effect_produces_variant() {
        // Unknown effect types should produce Unimplemented, not None
        // Use a truly unimplemented API type (not LoseLife, which is now implemented)
        let params = AbilityParams::parse("A:SP$ RearrangeTopOfLibrary | NumCards$ 3").unwrap();
        let effect = params_to_effect(&params);
        assert!(
            effect.is_some(),
            "Unimplemented effects should produce Unimplemented variant, not None"
        );

        match effect.unwrap() {
            Effect::Unimplemented { api_type } => {
                assert_eq!(api_type, "RearrangeTopOfLibrary", "Should record the API type name");
            }
            _ => panic!("Expected Unimplemented effect"),
        }
    }
}
