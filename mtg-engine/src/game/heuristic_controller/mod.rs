//! Heuristic AI controller - faithful port of Java Forge AI
//!
//! This implementation aims to faithfully reproduce the decision-making logic
//! of the Java Forge heuristic AI. It uses evaluation heuristics for creatures,
//! spells, and board states rather than simulation or Monte Carlo methods.
//!
//! Reference: forge-java/forge-ai/src/main/java/forge/ai/
//! - PlayerControllerAi.java (entry point)
//! - AiController.java (core logic)
//! - CreatureEvaluator.java (creature scoring)

use crate::core::{Card, CardId, Keyword, KeywordArgs, ManaCost, PlayerId, SpellAbility};
use crate::game::controller::{ChoiceResult, GameStateView, PlayerController};
use smallvec::SmallVec;

/// Predicted outcome of combat for attack decision making
///
/// Reference: GameStateEvaluator.java:40-67 - simulateUpcomingCombatThisTurn
/// This struct captures the predicted results of an attack without full simulation.
#[derive(Debug, Clone, Default)]
pub(crate) struct CombatOutcome {
    /// Total damage predicted to get through to opponent
    predicted_damage: i32,
    /// Number of attackers that will likely be blocked (for future logging/debugging)
    #[allow(dead_code)]
    blocked_attackers: usize,
    /// Number of attackers that will likely get through (for future logging/debugging)
    #[allow(dead_code)]
    unblocked_attackers: usize,
    /// Whether the attack is predicted to be lethal (for future use in advanced decisions)
    #[allow(dead_code)]
    is_lethal: bool,
}

/// Combat factors for attack decisions
///
/// Reference: AiAttackController.SpellAbilityFactors (lines 1350-1455)
///
/// This struct captures the essential combat math and board state evaluation
/// needed to make intelligent attack decisions.
pub(crate) struct CombatFactors {
    can_be_killed: bool,                  // Can attacker be killed by any blocker combination?
    can_be_killed_by_one: bool,           // Can a single blocker kill the attacker?
    can_kill_all: bool,                   // Can attacker kill all possible blockers one-on-one?
    can_kill_all_dangerous: bool,         // Can kill all dangerous blockers (lifelink/wither)?
    is_worth_less_than_all_killers: bool, // Is attacker worth less than all creatures that can kill it?
    has_combat_effect: bool,              // Does attacker gain value even if blocked? (lifelink, wither)
    dangerous_blockers_present: bool,     // Are there blockers with lifelink/wither?
    can_be_blocked: bool,                 // Can any blocker actually block this attacker?
    number_of_blockers: usize,            // Count of valid blockers
}

/// Classification of activated ability types for evaluation
pub(crate) enum ActivatedAbilityType {
    /// Ping ability - deals damage to target
    /// Example: Prodigal Sorcerer "{T}: Deal 1 damage to any target"
    Ping { damage: i32 },
    /// Pump ability - boosts creature stats
    /// Example: Shivan Dragon "{R}: +1/+0 until end of turn"
    Pump { power: i32, toughness: i32 },
    /// Destroy ability - destroys target permanent
    /// Example: Royal Assassin "{T}: Destroy target tapped creature"
    /// Reference: DestroyAi.java in forge-ai
    Destroy,
    /// Regenerate ability - adds a regeneration shield
    /// Example: Drudge Skeletons "{B}: Regenerate CARDNAME."
    Regenerate,
    /// Debuff ability - removes keywords from a creature
    /// Example: Grozoth "{4}: Lose defender until end of turn"
    Debuff,
    /// PreventDamage ability - creates a damage prevention shield
    /// Example: Militant Monk "{T}: Prevent the next 1 damage to any target"
    PreventDamage,
    /// TapTarget ability - taps a target permanent
    /// Example: Icy Manipulator "{1}, {T}: Tap target artifact, creature, or land"
    /// Reference: TapAi.java in forge-ai
    TapTarget,
    /// Zone-return ability — moves the card itself from one zone to another.
    /// Example: Earthquake Dragon "{2}{G}, Sac a land: Return CARDNAME from
    /// your graveyard to your hand." (ActivationZone$ Graveyard)
    ZoneReturn,
    /// Equip ability — attach this Equipment to a creature you control.
    /// Example: Trusty Boomerang "Equip {1}" (AttachEquipment effect).
    /// Sorcery-speed (CR 301.5c). Reference: AttachAi.java in forge-ai.
    Equip,
    /// Card-draw ability — e.g. crack a Clue token (sacrifice to draw).
    /// Example: Clue Token "{2}, Sacrifice this token: Draw a card."
    /// (DrawCards effect). Card advantage is almost always good.
    DrawCard,
    /// Other abilities not yet categorized
    Other,
}

/// Heuristic AI controller that makes decisions using evaluation functions
/// rather than simulation. Aims to faithfully reproduce Java Forge AI behavior.
///
/// `Clone`/`Serialize`/`Deserialize` are derived so that snapshot save/restore
/// (see `crate::game::snapshot::ControllerState::Heuristic`) preserves the
/// internal RNG state across stop-and-resume — without this, a heuristic
/// player would "re-roll" its bluffing/land-hold coin flips after every
/// snapshot reload, breaking determinism across execution modes.
///
/// Uses `Xoshiro256PlusPlus` (rather than `StdRng`) because it has serde
/// support that survives JSON serialization without u128 fields, matching
/// the choice already made for `RandomController`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HeuristicController {
    player_id: PlayerId,
    /// Aggression level for combat decisions (0 = defensive, 6 = all-in)
    /// Default is 3 (balanced). Matches Java's AiAttackController aggression.
    aggression_level: i32,
    /// RNG for probabilistic decisions (land drop timing, bluffing, etc.)
    ///
    /// Seeded via [`derive_player_seed`](crate::game::derive_player_seed) so
    /// all execution modes (native CLI, network, snapshot/restore, WASM)
    /// produce the same heuristic choice stream from the same master seed.
    rng: rand_xoshiro::Xoshiro256PlusPlus,
}

impl HeuristicController {
    /// Create a heuristic controller with the default (zero) seed.
    ///
    /// **Production callsites must NOT use this constructor.** It exists for
    /// tests and evaluator scaffolding that don't exercise the probabilistic
    /// heuristic branches (the lone `rng.gen_bool(0.5)` in
    /// `is_safe_to_hold_land_for_main2`). Production callers should derive
    /// a seed via [`crate::game::derive_player_seed`] and pass it to
    /// [`with_seed`](Self::with_seed) — otherwise every heuristic game uses
    /// seed 0 regardless of `--seed`, which silently breaks cross-mode
    /// determinism (see `docs/NETWORK_ARCHITECTURE.md`).
    pub fn new(player_id: PlayerId) -> Self {
        Self::with_seed(player_id, 0)
    }

    /// Create a heuristic controller with a specific seed for deterministic behavior.
    ///
    /// This is the production constructor. Pass a seed derived from the master
    /// `--seed` via [`crate::game::derive_player_seed`] so every execution mode
    /// (single-process, network, snapshot/resume, WASM) makes the same
    /// heuristic decisions for the same master seed.
    pub fn with_seed(player_id: PlayerId, seed: u64) -> Self {
        use rand::SeedableRng;
        HeuristicController {
            player_id,
            aggression_level: 3,
            rng: rand_xoshiro::Xoshiro256PlusPlus::seed_from_u64(seed),
        }
    }

    /// Set the aggression level for combat decisions
    /// 0 = very defensive, 3 = balanced, 6 = very aggressive
    pub fn set_aggression(&mut self, level: i32) {
        self.aggression_level = level.clamp(0, 6);
    }
}

// === Submodule map (see README.md) ===
// Each submodule below adds methods to `HeuristicController` via its own
// `impl HeuristicController` block, grouped by the shape of the decision the
// game asks the controller to make. This file keeps the type definition, the
// constructors, and the `PlayerController` trait impl (the decision entrypoints
// the engine calls), which dispatch into the submodules.
mod abilities;
mod combat;
mod creature_eval;
mod mana_lands;
mod spell_eval;
mod spell_selection;

#[cfg(test)]
mod tests;

impl PlayerController for HeuristicController {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn choose_spell_ability_to_play(
        &mut self,
        view: &GameStateView,
        available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>> {
        if available.is_empty() {
            let player_name = view.player_name();
            view.logger().controller_choice(
                "HEURISTIC",
                &format!("{} chose to pass priority (no available actions)", player_name),
            );
            return ChoiceResult::Ok(None);
        }

        let choice = self.choose_best_spell(view, available);
        let player_name = view.player_name();

        if let Some(ref spell) = choice {
            // Find the index of the chosen spell in the available list
            let ability_index = available.iter().position(|a| a == spell).unwrap_or(0);

            // Format the choice description using shared formatter
            let choice_description = crate::game::controller::format_spell_ability_choice(view, spell);

            view.logger().controller_choice(
                "HEURISTIC",
                &format!("{} chose {} - {}", player_name, ability_index, choice_description),
            );
        } else {
            view.logger().controller_choice(
                "HEURISTIC",
                &format!(
                    "{} chose 'p' (pass priority from {} available actions)",
                    player_name,
                    available.len()
                ),
            );
        }

        ChoiceResult::Ok(choice)
    }

    #[allow(clippy::wildcard_enum_match_arm)]
    fn choose_targets(
        &mut self,
        view: &GameStateView,
        spell: CardId,
        valid_targets: &[CardId],
        min_targets: usize,
        max_targets: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        if valid_targets.is_empty() {
            return ChoiceResult::Ok(SmallVec::new());
        }

        // Targeting heuristics:
        // - For damage/removal abilities: Target opponent's best killable creature
        // - For pump effects: Target our best creature
        // - Default for spells: Use original logic (target our creatures, fallback to opponent's)

        let spell_card = view.get_card(spell);

        // Check if this is a damage-dealing activated ability (like Prodigal Sorcerer)
        // For such abilities, we want to target opponent's creatures that can be killed
        // Reference: DamageDealAi.java - getBestCreatureAI filters for killable creatures
        let damage_amount = spell_card.and_then(|c| {
            // First check activated abilities (Prodigal Sorcerer, Tim, etc.)
            for ability in &c.activated_abilities {
                for effect in &ability.effects {
                    match effect {
                        crate::core::Effect::DealDamage { amount, .. } => return Some(*amount),
                        crate::core::Effect::DealDamageXPaid { .. } => return Some(i32::from(c.x_paid)),
                        _ => {}
                    }
                }
            }
            // Then check spell effects (Lightning Bolt, Shock, Fireball, etc.)
            for effect in &c.effects {
                match effect {
                    crate::core::Effect::DealDamage { amount, .. } => return Some(*amount),
                    crate::core::Effect::DealDamageXPaid { .. } => return Some(i32::from(c.x_paid)),
                    _ => {}
                }
            }
            None
        });

        // Check if the spell has pump effects (target self)
        let has_pump_effect = spell_card.is_some_and(|c| {
            c.effects
                .iter()
                .any(|e| matches!(e, crate::core::Effect::PumpCreature { .. }))
                || c.activated_abilities.iter().any(|a| {
                    a.effects
                        .iter()
                        .any(|e| matches!(e, crate::core::Effect::PumpCreature { .. }))
                })
        });

        // Check if the spell has debuff effects targeting others (remove keywords from opponent)
        let has_debuff_effect = spell_card.is_some_and(|c| {
            c.effects
                .iter()
                .any(|e| matches!(e, crate::core::Effect::DebuffCreature { .. }))
                || c.activated_abilities.iter().any(|a| {
                    a.effects
                        .iter()
                        .any(|e| matches!(e, crate::core::Effect::DebuffCreature { .. }))
                })
        });

        // Check if the spell has destroy effects (Sinkhole, Terror, etc.)
        // These should target opponent's permanents, not our own
        let has_destroy_effect = spell_card.is_some_and(|c| {
            c.effects
                .iter()
                .any(|e| matches!(e, crate::core::Effect::DestroyPermanent { .. }))
                || c.activated_abilities.iter().any(|a| {
                    a.effects
                        .iter()
                        .any(|e| matches!(e, crate::core::Effect::DestroyPermanent { .. }))
                })
        });

        // Check if the spell/ability has a tap effect (Icy Manipulator, etc.)
        // These should target opponent's permanents — tapping your own stuff is useless.
        // CR 602.1: activated ability effects are chosen at activation time; the
        // heuristic must pick an opponent permanent where available.
        let has_tap_effect = spell_card.is_some_and(|c| {
            c.effects
                .iter()
                .any(|e| matches!(e, crate::core::Effect::TapPermanent { .. }))
                || c.activated_abilities.iter().any(|a| {
                    a.effects
                        .iter()
                        .any(|e| matches!(e, crate::core::Effect::TapPermanent { .. }))
                })
        });

        // Choose targeting strategy based on spell/ability type
        let filtered_target_ids: Vec<CardId> = if let Some(damage) = damage_amount {
            // Damage abilities: Target opponent's best KILLABLE creature
            // Reference: DamageDealAi.java - prioritize creatures we can actually kill
            let killable_targets: Vec<CardId> = valid_targets
                .iter()
                .filter(|&&id| {
                    if let Some(card) = view.get_card(id) {
                        if card.owner == self.player_id {
                            return false; // Don't target our own creatures
                        }
                        if !card.is_creature() {
                            return false;
                        }
                        // Check if this creature would die from the damage
                        if let Some(toughness) = card.base_toughness() {
                            let effective_toughness = i32::from(toughness) + card.toughness_bonus;
                            return effective_toughness <= damage;
                        }
                    }
                    false
                })
                .copied()
                .collect();

            // If we have killable creatures, prioritize those
            // Otherwise fall back to any opponent creature (damage still useful)
            if !killable_targets.is_empty() {
                killable_targets
            } else {
                valid_targets
                    .iter()
                    .filter(|&&id| view.get_card(id).map(|c| c.owner != self.player_id).unwrap_or(false))
                    .copied()
                    .collect()
            }
        } else if has_pump_effect {
            // Pump effects: Target our best creature
            valid_targets
                .iter()
                .filter(|&&id| view.get_card(id).map(|c| c.owner == self.player_id).unwrap_or(false))
                .copied()
                .collect()
        } else if has_debuff_effect {
            // Debuff effects targeting opponent's creatures (remove protection, etc.)
            // Reference: DebuffEffect.java - target opponent's creatures with the keyword
            valid_targets
                .iter()
                .filter(|&&id| view.get_card(id).map(|c| c.owner != self.player_id).unwrap_or(false))
                .copied()
                .collect()
        } else if has_destroy_effect {
            // Destroy effects (Sinkhole, Terror, etc.): Target opponent's permanents
            // Reference: DestroyAi.java - always targets opponent's permanents
            let opponent_targets: Vec<CardId> = valid_targets
                .iter()
                .filter(|&&id| view.get_card(id).map(|c| c.owner != self.player_id).unwrap_or(false))
                .copied()
                .collect();
            if opponent_targets.is_empty() {
                // No opponent targets available - fallback to any valid target
                // (This shouldn't normally happen for removal spells, but be safe)
                valid_targets.to_vec()
            } else {
                opponent_targets
            }
        } else if has_tap_effect {
            // Tap effects (Icy Manipulator, etc.): Target opponent's permanents.
            // Tapping your own lands/creatures is self-defeating. Prefer the
            // opponent's most relevant (creature) permanent; fall back to any
            // opponent permanent; last resort is any valid target.
            // CR 602.1b: effect choice is part of activation, not a separate game
            // action — the heuristic is purely advisory and produces no
            // rules-illegal outcome regardless of which legal target it picks.
            let opponent_targets: Vec<CardId> = valid_targets
                .iter()
                .filter(|&&id| view.get_card(id).map(|c| c.owner != self.player_id).unwrap_or(false))
                .copied()
                .collect();
            if opponent_targets.is_empty() {
                valid_targets.to_vec()
            } else {
                opponent_targets
            }
        } else {
            // Default: Use original logic (target our creatures first, fallback to opponent's)
            // This maintains compatibility with the stress tests
            let our_targets: Vec<CardId> = valid_targets
                .iter()
                .filter(|&&id| view.get_card(id).map(|c| c.owner == self.player_id).unwrap_or(false))
                .copied()
                .collect();
            if our_targets.is_empty() {
                // Fallback to opponent's if we have no valid targets
                valid_targets.to_vec()
            } else {
                our_targets
            }
        };

        if filtered_target_ids.is_empty() {
            // Fallback: pick the minimum required, taking the first valid ones.
            let count = min_targets.max(1).min(valid_targets.len());
            return ChoiceResult::Ok(valid_targets.iter().take(count).copied().collect());
        }

        // For single-target spells (max_targets == 1), pick the single best
        // permanent. For variable-target spells (Fireball: max_targets > 1,
        // "X damage divided evenly among any number of targets"), spread across
        // as many of the filtered targets as allowed — preferring to hit MORE
        // targets so the divided damage covers the opponent's board. The choice
        // is deterministic and view-only, so it round-trips on the network.
        if max_targets <= 1 {
            let target = self.get_best_creature(view, &filtered_target_ids);
            let mut targets = SmallVec::new();
            if let Some(target_card_id) = target {
                targets.push(target_card_id);
            } else if !valid_targets.is_empty() {
                targets.push(valid_targets[0]);
            }
            return ChoiceResult::Ok(targets);
        }

        // Variable count: take up to max_targets from the filtered list, but at
        // least min_targets. filtered_target_ids preserves valid_targets order
        // (engine offers opponents-first), giving a deterministic selection.
        let cap = max_targets.min(filtered_target_ids.len());
        let count = cap.max(min_targets.min(valid_targets.len()));
        let mut targets: SmallVec<[CardId; 4]> = filtered_target_ids.iter().take(count).copied().collect();
        // If min_targets exceeds the filtered set, top up from remaining valid
        // targets so the lower bound is always satisfied.
        if targets.len() < min_targets {
            for &id in valid_targets {
                if targets.len() >= min_targets {
                    break;
                }
                if !targets.contains(&id) {
                    targets.push(id);
                }
            }
        }
        ChoiceResult::Ok(targets)
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        view: &GameStateView,
        cost: &ManaCost,
        available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Port of Java's ComputerUtilMana.scoreManaProducingCard()
        // Reference: ComputerUtilMana.java:95-120
        //
        // Strategy: Score each mana source by its alternate uses.
        // Sources with LOWER scores are tapped first (preserve flexibility).
        // - Lands with only mana abilities get low scores (tap these first)
        // - Creatures with mana abilities get +13 for attack and +13 for block potential
        // - Cards with non-mana activated abilities get +13 per ability

        let mut scored_sources: Vec<(CardId, i32)> = available_sources
            .iter()
            .filter_map(|&id| view.get_card(id).map(|card| (id, self.score_mana_source(card, view))))
            .collect();

        // Sort ascending by score - tap lowest score first
        scored_sources.sort_by_key(|(_, score)| *score);

        let mut sources = SmallVec::new();
        let needed = cost.cmc() as usize;

        for (source_id, _) in scored_sources.into_iter().take(needed) {
            sources.push(source_id);
        }

        ChoiceResult::Ok(sources)
    }

    fn choose_attackers(
        &mut self,
        view: &GameStateView,
        available_creatures: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Port of Java's AiAttackController.declareAttackers()
        // Reference: AiAttackController.java:818

        let mut attackers = SmallVec::new();

        // Get creature cards
        let creatures: Vec<&Card> = available_creatures.iter().filter_map(|&id| view.get_card(id)).collect();

        // Count opponent's available blockers to assess numerical advantage
        let opponent_blockers = self.count_opponent_blockers(view);
        let our_attackers_count = creatures.len();

        // Check if we have numerical advantage (more attackers than blockers)
        let has_numerical_advantage = our_attackers_count > opponent_blockers;

        // Check if we can go for lethal
        let is_lethal_push = self.is_lethal_opportunity(view, available_creatures);

        // DEBUG: Log attacker evaluation context for network equivalence debugging
        let creature_names: Vec<_> = creatures
            .iter()
            .map(|c| format!("{}({})", c.name, c.id.as_u32()))
            .collect();
        log::debug!(
            "HEURISTIC ATTACKERS [P{} Turn{}]: opp_life={}, is_lethal={}, blockers={}, available={:?}",
            self.player_id.as_u32(),
            view.turn_number(),
            view.opponent_life(),
            is_lethal_push,
            opponent_blockers,
            creature_names
        );

        // Evaluate each creature for attacking
        for creature in creatures {
            if self.should_attack_with_context(
                creature,
                view,
                has_numerical_advantage,
                opponent_blockers,
                is_lethal_push,
            ) {
                attackers.push(creature.id);
            }
        }

        if !attackers.is_empty() {
            view.logger().controller_choice(
                "HEURISTIC",
                &format!(
                    "chose {} attackers from {} available creatures (aggression={}, opponent blockers={})",
                    attackers.len(),
                    available_creatures.len(),
                    self.aggression_level,
                    opponent_blockers
                ),
            );
        } else if !available_creatures.is_empty() {
            view.logger().controller_choice(
                "HEURISTIC",
                &format!(
                    "chose not to attack with {} available creatures (aggression={}, opponent blockers={})",
                    available_creatures.len(),
                    self.aggression_level,
                    opponent_blockers
                ),
            );
        }

        ChoiceResult::Ok(attackers)
    }

    fn choose_blockers(
        &mut self,
        view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
        // Use improved blocking with gang block support
        // Reference: AiBlockController.assignBlockersForCombat() lines 1070-1160
        let blocks = self.assign_blocks_with_gang(view, available_blockers, attackers);

        if !blocks.is_empty() {
            view.logger().controller_choice(
                "HEURISTIC",
                &format!("chose {} blockers for {} attackers", blocks.len(), attackers.len()),
            );
        } else if !attackers.is_empty() && !available_blockers.is_empty() {
            view.logger().controller_choice(
                "HEURISTIC",
                &format!(
                    "chose not to block (no favorable blocks among {} blockers vs {} attackers)",
                    available_blockers.len(),
                    attackers.len()
                ),
            );
        }

        ChoiceResult::Ok(blocks)
    }

    fn choose_damage_assignment_order(
        &mut self,
        view: &GameStateView,
        attacker: CardId,
        blockers: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        // Port of Java Forge's AiBlockController.orderBlockers()
        // Reference: forge-java/forge-ai/src/main/java/forge/ai/AiBlockController.java:1175-1196
        //
        // Strategy:
        // 1. Sort blockers by evaluation (best creatures first)
        // 2. Put killable blockers at the front (where damage will be assigned first)
        // 3. Put unkillable blockers at the end (no point wasting damage on them)
        //
        // This ensures we maximize damage impact by killing the most valuable
        // creatures we can actually kill, rather than wasting damage on indestructible
        // or high-toughness creatures we can't kill anyway.

        if blockers.is_empty() {
            return ChoiceResult::Ok(SmallVec::new());
        }

        if blockers.len() == 1 {
            return ChoiceResult::Ok(blockers.iter().copied().collect());
        }

        // Get attacker's damage (using effective power after anthem effects)
        let attacker_power = view
            .get_effective_power(attacker)
            .or_else(|| view.get_card(attacker).map(|c| i32::from(c.current_power())))
            .unwrap_or(0);

        // Create a sorted list of blockers by evaluation (best first)
        // All blockers MUST be visible - they declared as blockers so server must have revealed them
        let mut blocker_list: Vec<(CardId, i32, i32, String)> = blockers
            .iter()
            .map(|&id| {
                let card = view.get_card(id).unwrap_or_else(|| {
                    panic!(
                        "FATAL: choose_damage_assignment_order called with invisible blocker {:?}. \
                        Blockers must be revealed before damage assignment. \
                        This indicates a missing CardRevealed message from the server.",
                        id
                    );
                });
                let eval = self.evaluate_creature(view, id);
                let toughness = view
                    .get_effective_toughness(id)
                    .unwrap_or_else(|| i32::from(card.current_toughness()));
                let eff_power = view
                    .get_effective_power(id)
                    .unwrap_or_else(|| i32::from(card.current_power()));
                (
                    id,
                    eval,
                    toughness,
                    format!("{}({}/{})", card.name, eff_power, toughness),
                )
            })
            .collect();

        // DEBUG: Log evaluations before sorting to detect divergence
        if blocker_list.len() > 1 {
            let attacker_name = view.get_card(attacker).map(|c| c.name.as_str()).unwrap_or("?");
            eprintln!(
                "[DEBUG-DAMAGE-ORDER] Player {:?} choosing damage order for {} attacking: {:?}",
                self.player_id,
                attacker_name,
                blocker_list
                    .iter()
                    .map(|(id, eval, tough, name)| format!("{} id={:?} eval={} tough={}", name, id, eval, tough))
                    .collect::<Vec<_>>()
            );
        }

        // Sort by evaluation (descending - best creatures first)
        blocker_list.sort_by(|a, b| b.1.cmp(&a.1));

        // Check if attacker has deathtouch - affects lethal damage calculation
        // MTG Rules 702.2c: Any nonzero damage from a source with deathtouch is lethal
        let attacker_has_deathtouch = view.has_keyword_with_effects(attacker, crate::core::Keyword::Deathtouch);

        // Separate into killable and non-killable based on remaining damage
        let mut remaining_damage = attacker_power;
        let mut killable: SmallVec<[CardId; 4]> = SmallVec::new();
        let mut unkillable: SmallVec<[CardId; 4]> = SmallVec::new();

        for (blocker_id, _eval, toughness, _name) in blocker_list {
            // Check if blocker has indestructible - can't be killed by damage
            // MTG Rules 702.12: An indestructible creature is not destroyed by lethal damage
            let blocker_has_indestructible =
                view.has_keyword_with_effects(blocker_id, crate::core::Keyword::Indestructible);

            if blocker_has_indestructible {
                // Indestructible creatures can't be killed - put at end
                unkillable.push(blocker_id);
                continue;
            }

            // Calculate damage needed to kill
            // With deathtouch: 1 damage is lethal (if toughness > 0)
            // Without deathtouch: need damage >= toughness
            let lethal_damage = if attacker_has_deathtouch && toughness > 0 {
                1 // Any nonzero damage from deathtouch is lethal
            } else {
                toughness
            };

            if lethal_damage <= remaining_damage {
                // We can kill this blocker
                killable.push(blocker_id);
                remaining_damage -= lethal_damage;
            } else {
                // Can't kill this blocker with remaining damage
                unkillable.push(blocker_id);
            }
        }

        // Combine: killable first, then unkillable
        killable.extend(unkillable);

        if killable.len() > 1 {
            view.logger().controller_choice(
                "HEURISTIC",
                &format!(
                    "ordered {} blockers for damage assignment (attacker power={})",
                    killable.len(),
                    attacker_power
                ),
            );
        }

        ChoiceResult::Ok(killable)
    }

    /// SMART damage assignment: Choose which blocker to kill first
    /// Strategy: Kill the most valuable creature first (highest evaluation score)
    fn choose_blocker_for_lethal_damage(
        &mut self,
        view: &GameStateView,
        attacker: CardId,
        killable_blockers: &[(CardId, i32)], // (blocker_id, lethal_damage_needed)
        remaining_power: i32,
    ) -> ChoiceResult<CardId> {
        if killable_blockers.is_empty() {
            return ChoiceResult::Error("No killable blockers provided".to_string());
        }

        // Single blocker - no choice needed
        if killable_blockers.len() == 1 {
            return ChoiceResult::Ok(killable_blockers[0].0);
        }

        // Evaluate each killable blocker and pick the most valuable one to kill first
        let mut best_blocker = killable_blockers[0].0;
        let mut best_eval = i32::MIN;

        for &(blocker_id, lethal_damage) in killable_blockers {
            // Skip if we don't have enough power to kill it
            if lethal_damage > remaining_power {
                continue;
            }

            let eval = self.evaluate_creature(view, blocker_id);
            if eval > best_eval {
                best_eval = eval;
                best_blocker = blocker_id;
            }
        }

        // Log the choice
        if let Some(card) = view.get_card(best_blocker) {
            view.logger().controller_choice(
                "HEURISTIC",
                &format!(
                    "assign lethal damage to {} ({}) first (eval={}, power={} for {:?})",
                    &card.name, best_blocker, best_eval, remaining_power, attacker
                ),
            );
        }

        ChoiceResult::Ok(best_blocker)
    }

    /// SMART damage assignment: Choose where to assign remaining non-lethal damage
    /// Strategy: Dump on the least valuable creature (since we can't kill anyone anyway)
    fn choose_blocker_for_remaining_damage(
        &mut self,
        view: &GameStateView,
        _attacker: CardId,
        remaining_blockers: &[CardId],
        remaining_damage: i32,
    ) -> ChoiceResult<CardId> {
        if remaining_blockers.is_empty() {
            return ChoiceResult::Error("No remaining blockers provided".to_string());
        }

        // Single blocker - no choice needed
        if remaining_blockers.len() == 1 {
            return ChoiceResult::Ok(remaining_blockers[0]);
        }

        // Find the least valuable blocker to dump damage on
        // (Since we can't kill any of them, put damage on the least important one)
        let mut worst_blocker = remaining_blockers[0];
        let mut worst_eval = i32::MAX;

        for &blocker_id in remaining_blockers {
            let eval = self.evaluate_creature(view, blocker_id);
            if eval < worst_eval {
                worst_eval = eval;
                worst_blocker = blocker_id;
            }
        }

        // Log the choice
        if let Some(card) = view.get_card(worst_blocker) {
            view.logger().controller_choice(
                "HEURISTIC",
                &format!(
                    "assign remaining {} damage to {} ({}) (eval={})",
                    remaining_damage, &card.name, worst_blocker, worst_eval,
                ),
            );
        }

        ChoiceResult::Ok(worst_blocker)
    }

    fn choose_scry_order(
        &mut self,
        view: &GameStateView,
        revealed: &[CardId],
    ) -> ChoiceResult<crate::game::ScryDecision> {
        // Heuristic owned in full by this controller (Phase C):
        //   - count lands in hand;
        //   - if we have ≥3 lands, push excess revealed lands to the
        //     bottom (heuristic doesn't need more lands);
        //   - otherwise keep all revealed cards on top.
        //
        // Order convention: ScryDecision.{top, bottom} are bottom-up,
        // last element of `top` becomes the new top of library after
        // [`GameState::scry_apply_decision`] runs. We INTENTIONALLY do
        // not reverse the keep pile here — this preserves the legacy
        // engine's existing reordering quirk so heuristic-driven games
        // remain byte-identical with pre-Phase-B logs.
        let player_id = view.player_id();
        let lands_in_hand = view
            .player_hand(player_id)
            .iter()
            .filter(|&&cid| view.get_card(cid).is_some_and(|c| c.is_land()))
            .count();
        let want_lands = lands_in_hand < 3;

        let mut top: SmallVec<[CardId; 4]> = SmallVec::new();
        let mut bottom: SmallVec<[CardId; 4]> = SmallVec::new();
        for &card_id in revealed {
            let is_land = view.get_card(card_id).is_some_and(|c| c.is_land());
            if is_land && !want_lands {
                bottom.push(card_id);
            } else {
                top.push(card_id);
            }
        }

        view.logger().controller_choice(
            "HEURISTIC",
            &format!(
                "Scry {}: keep {} on top, {} on bottom",
                revealed.len(),
                top.len(),
                bottom.len(),
            ),
        );
        ChoiceResult::Ok(crate::game::ScryDecision { top, bottom })
    }

    fn choose_surveil(
        &mut self,
        view: &GameStateView,
        revealed: &[CardId],
    ) -> ChoiceResult<crate::game::SurveilDecision> {
        // Heuristic: keep creatures and lands on top; mill instants /
        // sorceries / everything else into the graveyard (fuels
        // graveyard strategies — Flashback, Escape, etc.).
        //
        // Same order convention as choose_scry_order (no reversal of the
        // keep pile, preserving the legacy engine's quirk).
        let mut top: SmallVec<[CardId; 4]> = SmallVec::new();
        let mut graveyard: SmallVec<[CardId; 4]> = SmallVec::new();
        for &card_id in revealed {
            let dominated_by_creature_or_land = view.get_card(card_id).is_some_and(|c| c.is_creature() || c.is_land());
            if dominated_by_creature_or_land {
                top.push(card_id);
            } else {
                graveyard.push(card_id);
            }
        }

        view.logger().controller_choice(
            "HEURISTIC",
            &format!(
                "Surveil {}: keep {} on top, mill {} to graveyard",
                revealed.len(),
                top.len(),
                graveyard.len(),
            ),
        );
        ChoiceResult::Ok(crate::game::SurveilDecision { top, graveyard })
    }

    fn choose_cards_to_discard(
        &mut self,
        view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        // Simple heuristic: Discard lands first, then worst creatures.
        //
        // HARDENING (mtg-768): every id in `hand` is one of the deciding
        // player's OWN cards, so it MUST resolve in `view`. Silently dropping an
        // unresolvable id (the old `filter_map`) is exactly what masked the
        // mtg-768 desync: on a network client's shadow a just-drawn own card
        // that has not yet been materialised (its reveal still unapplied) would be
        // dropped from the discard candidate set, so the heuristic discarded the
        // WRONG cards vs the server's full-state decision — an
        // information-independence violation. We now `debug_assert` on an
        // unresolvable own card so the whole class surfaces LOUDLY in debug/test/
        // shadow builds instead of silently mis-deciding.
        let mut hand_cards: Vec<&Card> = Vec::with_capacity(hand.len());
        for &id in hand {
            match view.get_card(id) {
                Some(card) => hand_cards.push(card),
                None => debug_assert!(
                    false,
                    "choose_cards_to_discard: own hand card {id:?} is not resolvable in the shadow view — \
                     a draw/reveal was not applied before the discard decision (mtg-768 class: \
                     information-independence desync; NETWORK_ARCHITECTURE.md: Desync is ALWAYS Fatal)."
                ),
            }
        }

        // Sort by value (ascending) - discard worst cards first
        hand_cards.sort_by_key(|c| {
            if c.is_land() {
                0 // Discard lands first
            } else if c.is_creature() {
                self.evaluate_creature(view, c.id)
            } else {
                100 // Keep spells
            }
        });

        ChoiceResult::Ok(hand_cards.iter().take(count).map(|c| c.id).collect())
    }

    fn choose_from_library(
        &mut self,
        view: &GameStateView,
        valid_cards: &[&crate::loader::CardDefinition],
    ) -> ChoiceResult<Option<usize>> {
        if valid_cards.is_empty() {
            view.logger()
                .controller_choice("HEURISTIC", "Library search: fail to find (no valid cards)");
            return ChoiceResult::Ok(None);
        }

        // Score each card using CardDefinition properties (types, P/T, CMC)
        let mut best_index = 0;
        let mut best_score = i32::MIN;

        for (idx, &card_def) in valid_cards.iter().enumerate() {
            let score = self.evaluate_card_definition_for_library(view, card_def);
            if score > best_score {
                best_score = score;
                best_index = idx;
            }
        }

        let chosen_def = valid_cards[best_index];
        view.logger().controller_choice(
            "HEURISTIC",
            &format!("Library search: found {} (score: {})", chosen_def.name, best_score),
        );

        ChoiceResult::Ok(Some(best_index))
    }

    /// Network-mode counterpart of [`choose_from_library`].
    ///
    /// In network mode the authoritative library-search decision is made by the
    /// shadow CLIENT, which cannot see the hidden library card identities. The
    /// server therefore sends the candidate card *names* (built in
    /// `network::controller::NetworkController::choose_from_library` as
    /// `valid_cards.iter().map(|def| def.name)`, so this name list is index-aligned
    /// 1:1 with the server's `valid_cards` CardId slice). The server maps the index
    /// we return back to the concrete CardId.
    ///
    /// To honour the information-independence invariant (CLAUDE.md /
    /// docs/NETWORK_ARCHITECTURE.md), this MUST pick the identical index that
    /// [`choose_from_library`] would pick on the server's full-info view. We do that
    /// by looking up each name's public `CardDefinition` from the shared card
    /// definitions map (`view.game().card_definitions`) and scoring it with the exact
    /// same [`evaluate_card_definition_for_library`] used by `choose_from_library`,
    /// choosing the first-max index (matching the strict `score > best_score`
    /// tiebreak there). Card *names* are public, view-independent data — no hidden
    /// library order or zone contents are read. The previous trait default returned
    /// `Some(0)` (the first name), which disagreed with the full-info
    /// `choose_from_library` and caused the mtg-yulth desync.
    fn choose_from_library_by_names(
        &mut self,
        view: &GameStateView,
        card_names: &[String],
    ) -> ChoiceResult<Option<usize>> {
        if card_names.is_empty() {
            view.logger()
                .controller_choice("HEURISTIC", "Library search (by name): fail to find (no valid names)");
            return ChoiceResult::Ok(None);
        }

        // Score each candidate by its public CardDefinition with the SAME scoring
        // as choose_from_library; first-max wins (strict `>`), mirroring the
        // server-side index selection exactly.
        let mut best_index = 0;
        let mut best_score = i32::MIN;
        for (idx, name) in card_names.iter().enumerate() {
            let Some(card_def) = view.game().card_definitions.get(&crate::core::CardName::new(name)) else {
                // Every real library card is in the shared definitions map. A miss
                // would silently diverge server/client decisions, so treat it as a
                // fatal info-independence hazard rather than guessing.
                panic!(
                    "FATAL: heuristic choose_from_library_by_names could not resolve \
                     card name '{name}' in the card definitions map. This breaks \
                     server/client decision parity for library search (see \
                     docs/NETWORK_ARCHITECTURE.md)."
                );
            };
            let score = self.evaluate_card_definition_for_library(view, card_def);
            if score > best_score {
                best_score = score;
                best_index = idx;
            }
        }

        view.logger().controller_choice(
            "HEURISTIC",
            &format!(
                "Library search (by name): found {} (score: {})",
                card_names[best_index], best_score
            ),
        );

        ChoiceResult::Ok(Some(best_index))
    }

    fn choose_permanents_to_sacrifice(
        &mut self,
        view: &GameStateView,
        valid_permanents: &[CardId],
        count: usize,
        card_type_description: &str,
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Heuristic: Sacrifice the least valuable permanents first
        // Similar logic to choose_cards_to_discard but for permanents
        if valid_permanents.is_empty() || count == 0 {
            view.logger().controller_choice(
                "HEURISTIC",
                &format!("Sacrifice {}: nothing to sacrifice", card_type_description),
            );
            return ChoiceResult::Ok(SmallVec::new());
        }

        let mut scored_permanents: Vec<(CardId, i32)> = valid_permanents
            .iter()
            .filter_map(|&id| {
                let card = view.get_card(id)?;
                let score = if card.is_creature() {
                    // For creatures, use creature evaluation
                    self.evaluate_creature(view, id)
                } else if card.is_land() {
                    // Lands: prefer to keep dual lands, sacrifice basics first
                    use crate::game::game_state_evaluator::GameStateEvaluator;
                    GameStateEvaluator::evaluate_land(card)
                } else {
                    // For other permanents, use a basic value
                    // Higher CMC = more valuable = higher score
                    i32::from(card.mana_cost.cmc()) * 10
                };
                Some((id, score))
            })
            .collect();

        // Sort by score ascending - sacrifice lowest value first
        scored_permanents.sort_by_key(|&(_, score)| score);

        let to_sacrifice: SmallVec<[CardId; 8]> = scored_permanents.iter().take(count).map(|&(id, _)| id).collect();

        let names: Vec<String> = to_sacrifice.iter().filter_map(|&id| view.get_card_name(id)).collect();
        view.logger().controller_choice(
            "HEURISTIC",
            &format!("Sacrifice {} {}: [{}]", count, card_type_description, names.join(", ")),
        );

        ChoiceResult::Ok(to_sacrifice)
    }

    fn choose_permanents_to_not_untap(
        &mut self,
        view: &GameStateView,
        may_not_untap_permanents: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Heuristic: Keep permanents tapped if they are providing an ongoing effect
        // (e.g., control effects from Preacher, Coffin Queen, etc.)
        // For now, simple logic: keep tapped if the card has an active control effect
        // TODO(mtg-77): Improve by checking if the permanent is actively maintaining
        // a stolen creature or ongoing effect

        if may_not_untap_permanents.is_empty() {
            return ChoiceResult::Ok(SmallVec::new());
        }

        // For now, always untap (return empty list) - most permanents want to untap
        // so they can be used again. Control-stealing permanents need more complex
        // logic to detect if they're maintaining control of something valuable.
        view.logger().controller_choice(
            "HEURISTIC",
            &format!(
                "Untapping all {} permanents with MayNotUntap (default strategy)",
                may_not_untap_permanents.len()
            ),
        );

        ChoiceResult::Ok(SmallVec::new())
    }

    fn choose_modes(
        &mut self,
        view: &GameStateView,
        _spell_id: CardId,
        mode_descriptions: &[String],
        mode_count: usize,
        _min_modes: usize,
        _can_repeat: bool,
    ) -> ChoiceResult<SmallVec<[usize; 4]>> {
        // Heuristic mode selection: evaluate each mode based on current board state
        // For now, use simple text-based heuristics until we have full mode effect evaluation
        //
        // TODO(mtg-77): replace description-substring decision with structured
        // mode-effect eval (DETERMINISM HAZARD). This branches on
        // `desc.to_lowercase().contains(...)` over human-readable mode text. Per
        // docs/NETWORK_ARCHITECTURE.md controllers must be information-independent:
        // if any mode description ever interpolates runtime/hidden state, the
        // server (full state) and shadow client could score the same modes
        // differently and pick different indices -> desync. Decide from the
        // structured Effect list instead of the rendered string. Replacing this
        // is a BEHAVIOR change and must land in its own evidence-backed commit
        // with a before/after game-log diff, NOT in a pure refactor.
        // Proper mode evaluation should consider:
        // - Target availability (modes requiring targets that don't exist are useless)
        // - Board state relevance (destruction when opponent has creatures)
        // - Synergy with current game plan

        if mode_descriptions.is_empty() {
            return ChoiceResult::Ok(SmallVec::new());
        }

        // Score each mode based on simple heuristics
        let mut mode_scores: Vec<(usize, i32)> = mode_descriptions
            .iter()
            .enumerate()
            .map(|(idx, desc)| {
                let desc_lower = desc.to_lowercase();
                let mut score = 0i32;

                // Prefer removal effects
                if desc_lower.contains("destroy") || desc_lower.contains("exile") {
                    score += 50;
                }

                // Prefer damage effects
                if desc_lower.contains("damage") {
                    score += 40;
                }

                // Value counter manipulation
                if desc_lower.contains("counter") && !desc_lower.contains("counters on") {
                    score += 30;
                }

                // Value card advantage
                if desc_lower.contains("draw") || desc_lower.contains("card") {
                    score += 35;
                }

                // Value life gain/drain
                if desc_lower.contains("life") {
                    score += 20;
                }

                // Value stat boosts
                if desc_lower.contains("+") || desc_lower.contains("gets") {
                    score += 15;
                }

                (idx, score)
            })
            .collect();

        // Sort by score descending
        mode_scores.sort_by(|a, b| b.1.cmp(&a.1));

        // Take the top N modes
        let chosen: SmallVec<[usize; 4]> = mode_scores.iter().take(mode_count).map(|(idx, _)| *idx).collect();

        view.logger().controller_choice(
            "HEURISTIC",
            &format!(
                "Chose modes {:?} (scores: {:?}) from {} available",
                chosen,
                mode_scores.iter().take(mode_count).collect::<Vec<_>>(),
                mode_descriptions.len()
            ),
        );

        ChoiceResult::Ok(chosen)
    }

    fn on_priority_passed(&mut self, _view: &GameStateView) {
        // Could track game state here for future decisions
    }

    fn on_game_end(&mut self, _view: &GameStateView, _won: bool) {
        // Could collect statistics here
    }

    fn choose_from_options(&mut self, options: &[String]) -> usize {
        // For network mode, heuristic controller doesn't have access to full game state
        // to make intelligent decisions. Use simple heuristics based on option text.
        //
        // TODO(mtg-77): replace description-substring decision with structured
        // option evaluation (DETERMINISM HAZARD). This branches on
        // `opt.to_lowercase().contains(...)` over human-readable option text. Per
        // docs/NETWORK_ARCHITECTURE.md controllers must be information-independent:
        // if any option string ever interpolates runtime/hidden state, server and
        // shadow client could pick different indices -> desync. Decide from the
        // structured option model instead of the rendered string. This is a
        // BEHAVIOR change and must land in its own evidence-backed commit, NOT in
        // a pure refactor.

        if options.is_empty() {
            return 0;
        }

        // Prefer playing lands (usually first option is pass, second is land)
        for (i, opt) in options.iter().enumerate() {
            let opt_lower = opt.to_lowercase();
            if opt_lower.contains("play") && opt_lower.contains("land") {
                return i;
            }
        }

        // Prefer casting spells
        for (i, opt) in options.iter().enumerate() {
            let opt_lower = opt.to_lowercase();
            if opt_lower.contains("cast") {
                return i;
            }
        }

        // Prefer attacking
        for (i, opt) in options.iter().enumerate() {
            let opt_lower = opt.to_lowercase();
            if opt_lower.contains("attack") && !opt_lower.contains("don't") && !opt_lower.contains("no ") {
                return i;
            }
        }

        // Default: choose first non-pass option if available, otherwise pass
        if options.len() > 1 {
            1 // Skip "pass" which is usually option 0
        } else {
            0
        }
    }

    fn get_controller_type(&self) -> crate::game::snapshot::ControllerType {
        crate::game::snapshot::ControllerType::Heuristic
    }

    fn get_snapshot_state(&self) -> Option<serde_json::Value> {
        // Wrap in ControllerState::Heuristic so the snapshot's JSON has the
        // externally-tagged form expected by snapshot deserialization, i.e.
        // `{"Heuristic": {...}}`. Preserving the RNG state across
        // snapshot/resume is required for stop-and-go runs to produce the
        // same heuristic decisions as the equivalent single-process run.
        // (Internally-tagged `#[serde(tag = "controller_type")]` would break
        // bincode snapshots — see mtg-430.)
        let state = crate::game::ControllerState::Heuristic(self.clone());
        serde_json::to_value(state).ok()
    }
}
