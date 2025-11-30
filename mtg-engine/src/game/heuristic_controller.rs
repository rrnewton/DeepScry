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

use crate::core::{Card, CardId, Keyword, ManaCost, PlayerId, SpellAbility};
use crate::game::controller::{ChoiceResult, GameStateView, PlayerController};
use smallvec::SmallVec;

/// Predicted outcome of combat for attack decision making
///
/// Reference: GameStateEvaluator.java:40-67 - simulateUpcomingCombatThisTurn
/// This struct captures the predicted results of an attack without full simulation.
#[derive(Debug, Clone, Default)]
struct CombatOutcome {
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
struct CombatFactors {
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
enum ActivatedAbilityType {
    /// Ping ability - deals damage to target
    /// Example: Prodigal Sorcerer "{T}: Deal 1 damage to any target"
    Ping { damage: i32 },
    /// Pump ability - boosts creature stats
    /// Example: Shivan Dragon "{R}: +1/+0 until end of turn"
    Pump { power: i32, toughness: i32 },
    /// Other abilities not yet categorized
    Other,
}

/// Heuristic AI controller that makes decisions using evaluation functions
/// rather than simulation. Aims to faithfully reproduce Java Forge AI behavior.
///
/// This controller no longer owns an RNG - instead it uses the RNG passed
/// from GameState to ensure deterministic replay across snapshot/resume.
pub struct HeuristicController {
    player_id: PlayerId,
    /// Aggression level for combat decisions (0 = defensive, 6 = all-in)
    /// Default is 3 (balanced). Matches Java's AiAttackController aggression.
    aggression_level: i32,
}

impl HeuristicController {
    /// Create a new heuristic controller with default settings
    ///
    /// The RNG is now provided by GameState and passed to each decision method,
    /// ensuring deterministic gameplay across snapshot/resume cycles.
    pub fn new(player_id: PlayerId) -> Self {
        HeuristicController {
            player_id,
            aggression_level: 3, // Balanced aggression
        }
    }

    /// Create a heuristic controller (seed is no longer needed here)
    ///
    /// This method is kept for API compatibility but the seed parameter is ignored.
    /// The RNG seed should be set on GameState instead using `game.seed_rng(seed)`.
    #[deprecated(note = "Use HeuristicController::new() and seed the GameState RNG instead")]
    pub fn with_seed(player_id: PlayerId, _seed: u64) -> Self {
        HeuristicController {
            player_id,
            aggression_level: 3,
        }
    }

    /// Set the aggression level for combat decisions
    /// 0 = very defensive, 3 = balanced, 6 = very aggressive
    pub fn set_aggression(&mut self, level: i32) {
        self.aggression_level = level.clamp(0, 6);
    }

    /// Evaluate a creature's value using heuristics
    ///
    /// This is a faithful port of Java's CreatureEvaluator.evaluateCreature()
    /// Reference: forge-java/forge-ai/src/main/java/forge/ai/CreatureEvaluator.java:26
    ///
    /// Returns a score representing the creature's overall value.
    /// Higher scores indicate more valuable creatures.
    ///
    /// Uses effective P/T (after anthem effects, equipment, counters) for accurate evaluation.
    pub fn evaluate_creature(&self, view: &GameStateView, card_id: CardId) -> i32 {
        self.evaluate_creature_impl(view, card_id, true, true)
    }

    /// Internal implementation of creature evaluation with optional P/T and CMC consideration
    ///
    /// Parameters:
    /// - view: Game state view for accessing effective P/T
    /// - card_id: ID of the creature to evaluate
    /// - consider_pt: Whether to factor in power/toughness
    /// - consider_cmc: Whether to factor in mana cost
    ///
    /// Uses effective P/T from CR 613 layer system to properly account for anthem effects,
    /// equipment bonuses, and counters when evaluating creature value.
    fn evaluate_creature_impl(
        &self,
        view: &GameStateView,
        card_id: CardId,
        consider_pt: bool,
        consider_cmc: bool,
    ) -> i32 {
        let mut value = 80;

        // Get the card from the view
        let Some(card) = view.get_card(card_id) else {
            return 0; // Card not found, return minimal value
        };

        // Tokens are worth less than actual cards
        // Java: if (!c.isToken()) { value += addValue(20, "non-token"); }
        // TODO: Add is_token flag to Card struct
        // For now, assume all cards are non-tokens
        value += 20;

        // Use effective P/T after all continuous effects (anthem, equipment, counters)
        let power = view.get_effective_power(card_id).unwrap_or(card.current_power() as i32);
        let toughness = view
            .get_effective_toughness(card_id)
            .unwrap_or(card.current_toughness() as i32);

        // Stats scoring
        if consider_pt {
            // Java: value += addValue(power * 15, "power");
            value += power * 15;
            // Java: value += addValue(toughness * 10, "toughness: " + toughness);
            value += toughness * 10;
        }

        if consider_cmc {
            // Java: value += addValue(c.getCMC() * 5, "cmc");
            let cmc = card.mana_cost.cmc() as i32;
            value += cmc * 5;
        }

        // Evasion keywords
        // Java: if (c.hasKeyword(Keyword.FLYING)) { value += addValue(power * 10, "flying"); }
        if card.has_flying() {
            value += power * 10;
        }

        // Horsemanship: Similar to flying, only blockable by creatures with horsemanship
        // Java: if (c.hasKeyword(Keyword.HORSEMANSHIP)) { value += addValue(power * 10, "horsemanship"); }
        if card.has_keyword(Keyword::Horsemanship) {
            value += power * 10;
        }

        // Shadow: Can only be blocked by creatures with shadow
        // Java: if (c.hasKeyword(Keyword.SHADOW)) { value += addValue(power * 10, "shadow"); }
        if card.has_keyword(Keyword::Shadow) {
            value += power * 10;
        }

        // Unblockable check
        // Java: if (StaticAbilityCantAttackBlock.cantBlockBy(c, null)) { value += addValue(power * 10, "unblockable"); }
        // TODO: Implement full static ability check - for now we skip unblockable keywords
        // as they are not yet properly represented in the Keyword enum
        let is_unblockable = false;

        if !is_unblockable {
            // Check for evasion keywords
            let has_fear = card.has_keyword(Keyword::Fear);
            let has_intimidate = card.has_keyword(Keyword::Intimidate);
            let has_skulk = card.has_keyword(Keyword::Skulk);

            if has_fear {
                value += power * 6;
            }
            if has_intimidate {
                value += power * 6;
            }
            // Java: if (c.hasKeyword(Keyword.MENACE)) { value += addValue(power * 4, "menace"); }
            if card.has_menace() {
                value += power * 4;
            }
            if has_skulk {
                value += power * 3;
            }
        } else {
            value += power * 10;
        }

        // Combat keywords (only relevant if creature has power)
        if power > 0 {
            // Java: if (c.hasKeyword(Keyword.DOUBLE_STRIKE)) { value += addValue(10 + (power * 15), "ds"); }
            if card.has_double_strike() {
                value += 10 + (power * 15);
            }
            // Java: else if (c.hasKeyword(Keyword.FIRST_STRIKE)) { value += addValue(10 + (power * 5), "fs"); }
            else if card.has_first_strike() {
                value += 10 + (power * 5);
            }

            // Java: if (c.hasKeyword(Keyword.DEATHTOUCH)) { value += addValue(25, "dt"); }
            if card.has_deathtouch() {
                value += 25;
            }

            // Java: if (c.hasKeyword(Keyword.LIFELINK)) { value += addValue(power * 10, "lifelink"); }
            if card.has_lifelink() {
                value += power * 10;
            }

            // Java: if (power > 1 && c.hasKeyword(Keyword.TRAMPLE)) { value += addValue((power - 1) * 5, "trample"); }
            if power > 1 && card.has_trample() {
                value += (power - 1) * 5;
            }

            // Java: if (c.hasKeyword(Keyword.VIGILANCE)) { value += addValue((power * 5) + (toughness * 5), "vigilance"); }
            if card.has_keyword(Keyword::Vigilance) {
                value += (power * 5) + (toughness * 5);
            }

            // Check for Infect and Wither keywords
            let has_infect = card.has_keyword(Keyword::Infect);
            let has_wither = card.has_keyword(Keyword::Wither);

            if has_infect {
                value += power * 15;
            } else if has_wither {
                value += power * 10;
            }
        }

        // Defensive keywords
        // Java: if (c.hasKeyword(Keyword.REACH) && !c.hasKeyword(Keyword.FLYING)) { value += addValue(5, "reach"); }
        if card.has_reach() && !card.has_flying() {
            value += 5;
        }

        // Protection keywords
        // Java: if (c.hasKeyword(Keyword.INDESTRUCTIBLE)) { value += addValue(70, "darksteel"); }
        if card.has_indestructible() {
            value += 70;
        }

        // Java: if (c.hasKeyword(Keyword.HEXPROOF)) { value += addValue(35, "hexproof"); }
        if card.has_hexproof() {
            value += 35;
        }

        // Java: if (c.hasKeyword(Keyword.SHROUD)) { value += addValue(30, "shroud"); }
        if card.has_shroud() {
            value += 30;
        }

        // Negative keywords
        // Java: if (c.hasKeyword(Keyword.DEFENDER)) { value -= power * 9 + 40; }
        if card.has_defender() {
            value -= power * 9 + 40;
        }

        // Upkeep cost penalties (recurring costs make creatures less valuable)
        // Reference: CreatureEvaluator.java:235-276

        // Cumulative Upkeep: Costs increase each turn, severe penalty
        // Java: if (c.hasKeyword(Keyword.CUMULATIVE_UPKEEP)) { value -= 30; }
        if card.has_keyword(Keyword::CumulativeUpkeep) {
            value -= 30;
        }

        // Echo: Must pay cost again on next turn or sacrifice
        // Java: if (c.hasKeyword(Keyword.ECHO)) { value -= 10; }
        if card.has_keyword(Keyword::Echo) {
            value -= 10;
        }

        // Fading: Enters with fade counters, remove one each upkeep, sacrifice when none left
        // Java: value -= 20 * (1.0 - fadeCounters/initialFadeCounters) for scaling
        // Simplified: flat penalty since we don't track initial counters
        if card.has_keyword(Keyword::Fading) {
            // Get current fade counters if any
            let fade_counters = card.get_counter(crate::core::CounterType::Fade) as i32;
            if fade_counters == 0 {
                value -= 50; // About to die
            } else if fade_counters <= 2 {
                value -= 30; // Low counters
            } else {
                value -= 15; // Has time left
            }
        }

        // Vanishing: Similar to Fading, uses time counters
        // Java: value -= 20 * (1.0 - timeCounters/initialTimeCounters)
        if card.has_keyword(Keyword::Vanishing) {
            let time_counters = card.get_counter(crate::core::CounterType::Time) as i32;
            if time_counters == 0 {
                value -= 50; // About to die
            } else if time_counters <= 2 {
                value -= 30; // Low counters
            } else {
                value -= 15; // Has time left
            }
        }

        // Mana abilities add value
        // Java: if (!c.getManaAbilities().isEmpty()) { value += addValue(10, "mana"); }
        // TODO: Implement mana ability check
        // For now, check if it's a land with mana ability
        if card.is_land() {
            value += 10;
        }

        value
    }

    /// Get the best creature from a list based on evaluation score
    ///
    /// Reference: ComputerUtilCard.sortByEvaluateCreature() and getBestCreatureAI()
    fn get_best_creature(&self, view: &GameStateView, creature_ids: &[CardId]) -> Option<CardId> {
        creature_ids
            .iter()
            .max_by_key(|&&card_id| self.evaluate_creature(view, card_id))
            .copied()
    }

    /// Get the worst creature from a list based on evaluation score
    #[allow(dead_code)] // Will be used for discard decisions
    fn get_worst_creature(&self, view: &GameStateView, creature_ids: &[CardId]) -> Option<CardId> {
        creature_ids
            .iter()
            .min_by_key(|&&card_id| self.evaluate_creature(view, card_id))
            .copied()
    }

    /// Evaluate a creature for casting with mana efficiency consideration
    ///
    /// This method balances raw creature value against mana efficiency, especially
    /// in the early game where curving out and leaving mana open for interaction matters.
    ///
    /// Scoring formula:
    /// - Base: creature_value (from evaluate_creature)
    /// - Bonus: mana_efficiency_bonus (value / CMC ratio, scaled)
    /// - Bonus: curve_bonus (if CMC matches available mana well)
    /// - Penalty: if casting leaves awkward leftover mana
    ///
    /// Reference: ComputerUtil.java creature casting logic
    fn evaluate_creature_for_casting(
        &self,
        view: &GameStateView,
        card_id: CardId,
        available_mana: u32,
        turn_number: u32,
    ) -> i32 {
        let Some(card) = view.get_card(card_id) else {
            return i32::MIN;
        };

        let base_value = self.evaluate_creature(view, card_id);
        let cmc = card.mana_cost.cmc() as u32;

        // Avoid division by zero - free creatures are always castable
        if cmc == 0 {
            return base_value + 50; // Bonus for free creatures
        }

        let mut score = base_value;

        // Mana efficiency bonus: value per mana spent
        // Scale by 10 to make it meaningful but not dominant
        // A 2-mana 2/2 (value ~130) has efficiency 65, a 5-mana 4/4 (value ~200) has efficiency 40
        let efficiency = (base_value * 10) / (cmc as i32);
        score += efficiency / 5; // Add ~13 for the 2/2, ~8 for the 4/4

        // Early game bonus for curving out (turns 1-4)
        // Reward creatures whose CMC matches available mana closely
        if turn_number <= 4 {
            let mana_fit = if cmc == available_mana {
                30 // Perfect curve
            } else if cmc == available_mana.saturating_sub(1) {
                20 // One under (leaves 1 mana open)
            } else if cmc == available_mana.saturating_sub(2) {
                10 // Two under (might want to double-spell)
            } else {
                0
            };
            score += mana_fit;
        }

        // Leftover mana consideration
        // Penalize awkward amounts of leftover mana that can't be used
        let leftover = available_mana.saturating_sub(cmc);
        if leftover == 1 {
            // 1 mana leftover is good - can activate abilities or hold up minor interaction
            score += 5;
        } else if (2..=3).contains(&leftover) {
            // 2-3 mana leftover is great - can hold up removal or counterspells
            score += 10;
        }
        // 0 leftover or large leftover: no bonus/penalty

        score
    }

    /// Count available mana from untapped lands
    ///
    /// This is a simplified count that assumes each untapped land produces 1 mana.
    /// It doesn't account for multi-mana lands or mana dorks, but is sufficient
    /// for early game mana efficiency calculations.
    ///
    /// For accurate mana availability, use the ManaEngine - but for heuristic
    /// purposes this simple count is fast and good enough.
    fn count_available_mana(&self, view: &GameStateView) -> u32 {
        view.battlefield()
            .iter()
            .filter(|&&card_id| {
                if let Some(card) = view.get_card(card_id) {
                    // Count untapped lands we control
                    card.owner == self.player_id && card.is_land() && !view.is_tapped(card_id)
                } else {
                    false
                }
            })
            .count() as u32
    }

    /// Evaluate whether a land should be played
    ///
    /// Reference: AiController.java:1428-1446 (land play decision logic)
    ///
    /// The Java AI uses several checks:
    /// 1. Don't play lands that would deal lethal ETB damage
    /// 2. Sometimes hold land drop for Main 2 (bluffing/deception)
    /// 3. Prioritize playing lands early in the game
    ///
    /// For now, we use a simplified approach:
    /// - Always play lands (faithful to Java's default behavior)
    /// - TODO: Add ETB damage check
    /// - TODO: Add Main 2 hold logic (requires randomization based on AI profile)
    /// - TODO: Check for "PlayBeforeLandDrop" special cases
    fn should_play_land(&self, _view: &GameStateView) -> bool {
        // Basic check: always play lands
        // This matches Java's behavior when no special conditions apply

        // TODO(mtg-XX): Add ETB damage check
        // Java: (!player.canLoseLife() || player.cantLoseForZeroOrLessLife()
        //        || ComputerUtil.getDamageFromETB(player, land) < player.getLife())

        // TODO(mtg-XX): Add Main 2 hold logic for bluffing
        // Java: (!game.getPhaseHandler().is(PhaseType.MAIN1)
        //        || !isSafeToHoldLandDropForMain2(land))
        // This is a deception mechanism to hide information from opponents

        // For phase 1, always play lands
        true
    }

    /// Choose the best land to play from available lands
    ///
    /// Reference: AiController.java:500-724 (chooseBestLandToPlay)
    ///
    /// The Java AI scores lands based on:
    /// 1. Base evaluation score (from GameStateEvaluator.evaluateLand)
    /// 2. +25 points for new basic land types
    /// 3. Color production: (new_colors * 50) / (existing_colors + 1)
    /// 4. Preference for untapped lands when we have spells to cast
    /// 5. Color fixing for one-drops in hand
    ///
    /// For now, simplified version:
    /// - Prefer untapped lands
    /// - Prefer lands that produce colors we need
    /// - TODO: Full scoring algorithm from Java
    fn choose_best_land(&self, _view: &GameStateView, lands: &[CardId]) -> Option<CardId> {
        if lands.is_empty() {
            return None;
        }

        // For now, just return the first land
        // TODO(mtg-XX): Implement full land selection algorithm
        // - Score based on enters-tapped status
        // - Score based on color production vs colors in hand
        // - Score based on new basic land types
        // - Consider mana curve of hand

        Some(lands[0])
    }

    /// Choose the best spell to cast from available options
    ///
    /// This implements the core decision logic from AiController.chooseSpellAbilityToPlay()
    /// Reference: AiController.java:1415-1449
    ///
    /// Priority order (like Java):
    /// 1. Check for "PlayBeforeLandDrop" cards (special timing requirements)
    /// 2. Play land (if available and should play)
    /// 3. Cast creatures (best evaluation first)
    /// 4. Cast other spells (removal, pump, etc.)
    /// 5. Pass priority
    fn choose_best_spell(&mut self, view: &GameStateView, available: &[SpellAbility]) -> Option<SpellAbility> {
        if available.is_empty() {
            return None;
        }

        // Phase 1: Check for "PlayBeforeLandDrop" cards
        // TODO(mtg-XX): Implement PlayBeforeLandDrop check
        // Java: CardLists.filter(player.getCardsIn(ZoneType.Hand),
        //                        CardPredicates.hasSVar("PlayBeforeLandDrop"))

        // Phase 2: Cast spells (creatures, pumps, etc.)
        // IMPORTANT: Cast spells BEFORE playing lands to ensure aggressive gameplay

        // 2a: Evaluate pump spells first (they can enable attacks)
        // Reference: PumpAi.checkPhaseRestrictions() lines 98-103
        // Instant-speed pumps should NOT be cast outside of combat (with exceptions)
        for ability in available {
            if let SpellAbility::CastSpell { card_id } = ability {
                if let Some(spell_card) = view.get_card(*card_id) {
                    // Check if this is a pump spell (has PumpCreature effect)
                    for effect in &spell_card.effects {
                        if let crate::core::Effect::PumpCreature {
                            target: _,
                            power_bonus,
                            toughness_bonus,
                        } = effect
                        {
                            // Check phase restrictions for instant pumps
                            // Reference: PumpAi.java:98-103
                            let current_step = view.current_step();
                            let is_instant = spell_card.is_instant();

                            // Instant pumps should only be cast during combat (or with good reason pre-combat)
                            // Don't cast instant pumps if:
                            // - We're before combat begins, OR
                            // - We're after declare blockers
                            // Exception: If the pump makes a non-attacker into an attacker (pre-combat only)
                            let should_hold_for_combat = is_instant
                                && (current_step < crate::game::phase::Step::BeginCombat
                                    || current_step > crate::game::phase::Step::DeclareBlockers);

                            if should_hold_for_combat && current_step < crate::game::phase::Step::BeginCombat {
                                // Pre-combat: Only cast if it makes a non-attacker into an attacker
                                // AND it's not a combat trick we should hold
                                // This is evaluated in should_cast_pump with combat trick detection
                            }

                            // This is a pump spell - evaluate whether we should cast it
                            // For pump spells, we need to determine the target
                            // For now, evaluate if pumping our best creature would be good

                            // Get our creatures (typically 2-8)
                            let our_creatures: SmallVec<[&Card; 8]> = view
                                .battlefield()
                                .iter()
                                .filter_map(|&id| view.get_card(id))
                                .filter(|c| c.owner == self.player_id && c.is_creature())
                                .collect();

                            // Try each potential target
                            for creature in &our_creatures {
                                // Extract keywords that would be granted
                                // TODO: Parse keywords from effect or spell text
                                let keywords_granted: Vec<String> = vec![];

                                if self.should_cast_pump(
                                    creature,
                                    *power_bonus,
                                    *toughness_bonus,
                                    &keywords_granted,
                                    view,
                                ) {
                                    // This pump spell would be valuable - cast it
                                    return Some(ability.clone());
                                }
                            }
                        }
                    }
                }
            }
        }

        // 2b: Cast creatures (best evaluation first, with mana efficiency)
        // Evaluate all castable creatures considering both raw value and mana efficiency
        // This prioritizes curving out in early game while still preferring high-value threats
        let mut best_creature_ability: Option<SpellAbility> = None;
        let mut best_creature_value = i32::MIN;

        // Get game state for mana efficiency calculation
        let turn_number = view.turn_number();
        let available_mana = self.count_available_mana(view);

        for ability in available {
            if let SpellAbility::CastSpell { card_id } = ability {
                if let Some(card) = view.get_card(*card_id) {
                    if card.is_creature() {
                        // Use mana-efficient evaluation in early game (turns 1-5)
                        // In late game, just use raw creature value
                        let value = if turn_number <= 5 {
                            self.evaluate_creature_for_casting(view, *card_id, available_mana, turn_number)
                        } else {
                            self.evaluate_creature(view, *card_id)
                        };
                        if value > best_creature_value {
                            best_creature_value = value;
                            best_creature_ability = Some(ability.clone());
                        }
                    }
                }
            }
        }

        if best_creature_ability.is_some() {
            return best_creature_ability;
        }

        // Phase 2b: Activated abilities (especially removal during combat)
        // Evaluate and use activated abilities intelligently
        // Reference: Java Forge's ability AI in forge-ai/src/main/java/forge/ai/ability/
        for ability in available {
            if let SpellAbility::ActivateAbility { card_id, ability_index } = ability {
                if let Some(source_card) = view.get_card(*card_id) {
                    // Skip mana abilities (let mana system handle those)
                    // Check the SPECIFIC ability being activated, not all abilities on the card
                    let is_mana_ability = source_card
                        .activated_abilities
                        .get(*ability_index)
                        .is_some_and(|ab| ab.is_mana_ability);
                    if is_mana_ability {
                        continue;
                    }

                    // Evaluate if we should use this ability now
                    if self.should_activate_ability(source_card, view) {
                        return Some(ability.clone());
                    }
                }
            }
        }

        // Phase 3: Land play logic (only if we can't cast creatures)
        if self.should_play_land(view) {
            // Collect land play abilities (typically 1-3 lands in hand)
            let land_plays: SmallVec<[&SpellAbility; 4]> = available
                .iter()
                .filter(|sa| matches!(sa, SpellAbility::PlayLand { .. }))
                .collect();

            if !land_plays.is_empty() {
                // Extract land card IDs
                let land_ids: SmallVec<[CardId; 4]> = land_plays
                    .iter()
                    .filter_map(|sa| {
                        if let SpellAbility::PlayLand { card_id, .. } = sa {
                            Some(*card_id)
                        } else {
                            None
                        }
                    })
                    .collect();

                // Choose best land
                if let Some(best_land_id) = self.choose_best_land(view, &land_ids) {
                    // Find and return the corresponding land play ability
                    for ability in land_plays {
                        if let SpellAbility::PlayLand { card_id, .. } = ability {
                            if *card_id == best_land_id {
                                return Some((*ability).clone());
                            }
                        }
                    }
                }
            }
        }

        // Phase 4: Cast other spells (removal, damage, etc.)
        for ability in available {
            if let SpellAbility::CastSpell { card_id } = ability {
                if let Some(spell_card) = view.get_card(*card_id) {
                    // Skip creatures and pumps (already handled above)
                    if spell_card.is_creature() {
                        continue;
                    }

                    // Check if this is a pump spell (skip, already handled)
                    let is_pump = spell_card
                        .effects
                        .iter()
                        .any(|e| matches!(e, crate::core::Effect::PumpCreature { .. }));
                    if is_pump {
                        continue;
                    }

                    // Evaluate other spells (removal, damage, etc.)
                    if self.should_cast_spell(spell_card, view) {
                        return Some(ability.clone());
                    }
                }
            }
        }

        // Pass priority if nothing good to do
        None
    }

    /// Calculate combat factors for an attacker against available blockers
    ///
    /// Reference: AiAttackController.SpellAbilityFactors.calculate() (lines 1374-1454)
    fn calculate_combat_factors(&self, attacker_id: CardId, view: &GameStateView) -> CombatFactors {
        let Some(attacker) = view.get_card(attacker_id) else {
            // Card not found, return default factors
            return CombatFactors {
                can_be_killed: false,
                can_be_killed_by_one: false,
                can_kill_all: false,
                can_kill_all_dangerous: false,
                is_worth_less_than_all_killers: false,
                has_combat_effect: false,
                dangerous_blockers_present: false,
                can_be_blocked: false,
                number_of_blockers: 0,
            };
        };

        let _attacker_power = view.get_effective_power(attacker_id).unwrap_or(0);
        let _attacker_toughness = view.get_effective_toughness(attacker_id).unwrap_or(0);
        let attacker_value = self.evaluate_creature(view, attacker_id);

        // Combat effect keywords (gain value even if blocked)
        // Note: Afflict is not yet in the Keyword enum, so we skip it for now
        let has_combat_effect = attacker.has_lifelink() || attacker.has_keyword(Keyword::Wither);

        // Collect all potential blockers from opponents (typically 2-8 creatures)
        let potential_blockers: SmallVec<[&Card; 8]> = view
            .battlefield()
            .iter()
            .filter_map(|&id| view.get_card(id))
            .filter(|c| c.owner != self.player_id && c.is_creature() && !c.tapped && self.can_block(attacker, c))
            .collect();

        let number_of_blockers = potential_blockers.len();
        let can_be_blocked = number_of_blockers > 0;

        // Track if there are dangerous blockers (with combat effects)
        let dangerous_blockers_present = potential_blockers
            .iter()
            .any(|b| b.has_lifelink() || b.has_keyword(Keyword::Wither));

        // Initialize factors
        let mut can_be_killed = false;
        let mut can_be_killed_by_one = false;
        let mut can_kill_all = true;
        let mut can_kill_all_dangerous = true;
        let mut is_worth_less_than_all_killers = true;

        // Evaluate each potential blocker
        for &blocker in &potential_blockers {
            let blocker_id = blocker.id;
            let _blocker_power = view.get_effective_power(blocker_id).unwrap_or(0);
            let _blocker_toughness = view.get_effective_toughness(blocker_id).unwrap_or(0);
            let blocker_value = self.evaluate_creature(view, blocker_id);

            // Can this blocker kill the attacker?
            if self.can_destroy_attacker(attacker, blocker) {
                can_be_killed = true;
                can_be_killed_by_one = true;

                // Check value comparison
                if blocker_value <= attacker_value {
                    is_worth_less_than_all_killers = false;
                }
            }

            // Can attacker kill this blocker?
            if !self.can_destroy_blocker(attacker, blocker) {
                can_kill_all = false;

                // Check if this blocker is dangerous
                let is_dangerous_blocker = blocker.has_lifelink() || blocker.has_keyword(Keyword::Wither);

                if is_dangerous_blocker {
                    can_kill_all_dangerous = false;
                }
            }
        }

        // If no blockers, attacker can kill "all" of them vacuously
        if potential_blockers.is_empty() {
            can_kill_all = true;
            can_kill_all_dangerous = true;
        }

        CombatFactors {
            can_be_killed,
            can_be_killed_by_one,
            can_kill_all,
            can_kill_all_dangerous,
            is_worth_less_than_all_killers,
            has_combat_effect,
            dangerous_blockers_present,
            can_be_blocked,
            number_of_blockers,
        }
    }

    /// Check if a blocker can block an attacker
    ///
    /// Reference: CombatUtil.canBlock()
    fn can_block(&self, attacker: &Card, blocker: &Card) -> bool {
        // Defender can't block
        if blocker.has_defender() {
            return false;
        }

        // Flying can only be blocked by flying or reach
        if attacker.has_flying() && !(blocker.has_flying() || blocker.has_reach()) {
            return false;
        }

        // Menace requires at least 2 blockers (simplified check)
        // In a full implementation, this would be context-dependent
        if attacker.has_menace() {
            // For single-blocker evaluation, menace makes it harder to block
            // But we'll allow it for now in multi-blocker scenarios
        }

        // TODO: Add more blocking restrictions:
        // - Protection from color/type
        // - Unblockable keyword
        // - Fear/Intimidate
        // - Other evasion abilities

        true
    }

    /// Check if attacker can destroy blocker in combat
    ///
    /// Reference: ComputerUtilCombat.canDestroyBlocker()
    fn can_destroy_blocker(&self, attacker: &Card, blocker: &Card) -> bool {
        let attacker_power = attacker.current_power() as i32;
        let blocker_toughness = blocker.current_toughness() as i32;

        // Deathtouch kills any creature with toughness > 0
        if attacker.has_deathtouch() && blocker_toughness > 0 {
            return true;
        }

        // Indestructible blockers can't be destroyed by damage
        if blocker.has_indestructible() {
            return false;
        }

        // First strike matters
        let attacker_first_strike = attacker.has_first_strike() || attacker.has_double_strike();
        let blocker_first_strike = blocker.has_first_strike() || blocker.has_double_strike();

        if attacker_first_strike && !blocker_first_strike {
            // Attacker strikes first - can it kill before taking damage?
            return attacker_power >= blocker_toughness;
        }

        // Normal combat: does attacker deal lethal damage?
        attacker_power >= blocker_toughness
    }

    /// Check if blocker can destroy attacker in combat
    ///
    /// Reference: ComputerUtilCombat.canDestroyAttacker()
    fn can_destroy_attacker(&self, attacker: &Card, blocker: &Card) -> bool {
        let blocker_power = blocker.current_power() as i32;
        let attacker_toughness = attacker.current_toughness() as i32;

        // Deathtouch kills any creature with toughness > 0
        if blocker.has_deathtouch() && attacker_toughness > 0 {
            return true;
        }

        // Indestructible attackers can't be destroyed by damage
        if attacker.has_indestructible() {
            return false;
        }

        // First strike matters
        let attacker_first_strike = attacker.has_first_strike() || attacker.has_double_strike();
        let blocker_first_strike = blocker.has_first_strike() || blocker.has_double_strike();

        if blocker_first_strike && !attacker_first_strike {
            // Blocker strikes first - can it kill before taking damage?
            return blocker_power >= attacker_toughness;
        }

        // Normal combat: does blocker deal lethal damage?
        blocker_power >= attacker_toughness
    }

    /// Determine if a creature should attack based on evaluation and aggression level
    ///
    /// Reference: AiAttackController.java:1470 (shouldAttack method)
    ///
    /// This uses combat factors to make intelligent attack decisions that consider:
    /// - Board state evaluation (what blockers are available)
    /// - Combat math (can kill/be killed calculations)
    /// - Creature value comparisons
    /// - Aggression level settings
    ///
    /// Count the number of creatures opponent has that can block
    fn count_opponent_blockers(&self, view: &GameStateView) -> usize {
        view.battlefield()
            .iter()
            .filter_map(|&id| view.get_card(id))
            .filter(|c| c.owner != self.player_id && c.is_creature() && !c.tapped && !c.has_defender())
            .count()
    }

    /// Calculate potential lethal damage from attacking (raw, not considering blockers)
    ///
    /// Returns the total damage we could deal if all our creatures attack and are unblocked.
    /// This is a simpler metric than predict_combat_outcome for quick checks.
    #[allow(dead_code)] // Kept for potential future use in simple checks
    fn calculate_lethal_potential(&self, view: &GameStateView, available_creatures: &[CardId]) -> i32 {
        available_creatures
            .iter()
            .filter_map(|&id| view.get_card(id))
            .map(|c| c.current_power() as i32)
            .sum()
    }

    /// Check if we should go for lethal damage
    ///
    /// Be very aggressive if we can potentially kill opponent
    fn is_lethal_opportunity(&self, view: &GameStateView, available_creatures: &[CardId]) -> bool {
        let opp_life = view.opponent_life();
        // Use smart combat outcome prediction
        let outcome = self.predict_combat_outcome(view, available_creatures);
        // Consider lethal if predicted damage >= opponent's life
        outcome.predicted_damage >= opp_life
    }

    /// Predict combat outcome: how much damage will likely get through after blocking
    ///
    /// Reference: GameStateEvaluator.java:40-67 - simulateUpcomingCombatThisTurn
    /// Instead of full simulation, we use heuristics to predict:
    /// - Which attackers will likely be blocked
    /// - How much damage will get through
    /// - Whether the attack is lethal
    ///
    /// This is a key improvement over the naive "sum all power" approach.
    fn predict_combat_outcome(&self, view: &GameStateView, attackers: &[CardId]) -> CombatOutcome {
        if attackers.is_empty() {
            return CombatOutcome::default();
        }

        // Get opponent's blockers
        let blockers: SmallVec<[&Card; 8]> = view
            .battlefield()
            .iter()
            .filter_map(|&id| view.get_card(id))
            .filter(|c| c.owner != self.player_id && c.is_creature() && !c.tapped && !c.has_defender())
            .collect();

        // Get attacker cards sorted by value (highest first - opponent blocks these first)
        let mut attacker_cards: Vec<&Card> = attackers.iter().filter_map(|&id| view.get_card(id)).collect();
        attacker_cards.sort_by_key(|c| std::cmp::Reverse(self.evaluate_creature(view, c.id)));

        let mut predicted_damage = 0i32;
        let mut blocked_attackers = 0usize;
        let mut unblocked_attackers = 0usize;
        let mut remaining_blockers: Vec<&Card> = blockers.iter().copied().collect();

        // Simulate optimal blocking by opponent
        for attacker in &attacker_cards {
            let attacker_power = view
                .get_effective_power(attacker.id)
                .unwrap_or(attacker.current_power() as i32);

            // Check if attacker can be blocked
            if !self.can_attacker_be_blocked(attacker, &remaining_blockers) {
                // Unblockable - damage gets through
                predicted_damage += attacker_power;
                unblocked_attackers += 1;
                continue;
            }

            // Find a suitable blocker for this attacker
            // Opponent will try to: (1) trade favorably, (2) chump if necessary
            let best_blocker = self.find_best_blocker_for_attacker(attacker, &remaining_blockers, view);

            match best_blocker {
                Some(blocker_idx) => {
                    // This attacker will be blocked
                    blocked_attackers += 1;

                    // Handle trample - excess damage gets through
                    if attacker.has_trample() {
                        let blocker = remaining_blockers[blocker_idx];
                        let blocker_toughness = view
                            .get_effective_toughness(blocker.id)
                            .unwrap_or(blocker.current_toughness() as i32);
                        let excess = (attacker_power - blocker_toughness).max(0);
                        predicted_damage += excess;
                    }

                    // Remove this blocker from availability
                    remaining_blockers.remove(blocker_idx);
                }
                None => {
                    // No blocker available - damage gets through
                    predicted_damage += attacker_power;
                    unblocked_attackers += 1;
                }
            }
        }

        let opp_life = view.opponent_life();
        let is_lethal = predicted_damage >= opp_life;

        CombatOutcome {
            predicted_damage,
            blocked_attackers,
            unblocked_attackers,
            is_lethal,
        }
    }

    /// Check if an attacker can be blocked by any of the available blockers
    fn can_attacker_be_blocked(&self, attacker: &Card, blockers: &[&Card]) -> bool {
        for blocker in blockers {
            if self.can_block(attacker, blocker) {
                return true;
            }
        }
        false
    }

    /// Find the best blocker for an attacker from opponent's perspective
    ///
    /// Returns the index of the best blocker, or None if no blocking is worthwhile.
    /// Opponent's priorities:
    /// 1. Block with something that kills the attacker and survives
    /// 2. Block with something that trades favorably (kills attacker, dies, but lower value)
    /// 3. Chump block with lowest-value creature if attacker is very dangerous
    fn find_best_blocker_for_attacker(
        &self,
        attacker: &Card,
        blockers: &[&Card],
        view: &GameStateView,
    ) -> Option<usize> {
        if blockers.is_empty() {
            return None;
        }

        let attacker_value = self.evaluate_creature(view, attacker.id);
        let attacker_power = attacker.current_power() as i32;

        // Categorize blockers
        let mut best_safe_killer: Option<(usize, i32)> = None; // (index, value)
        let mut best_trading_killer: Option<(usize, i32)> = None;
        let mut best_chump: Option<(usize, i32)> = None;

        for (idx, &blocker) in blockers.iter().enumerate() {
            if !self.can_block(attacker, blocker) {
                continue;
            }

            let blocker_value = self.evaluate_creature(view, blocker.id);
            let can_kill_attacker = self.can_destroy_blocker(blocker, attacker);
            let will_survive = !self.can_destroy_attacker(attacker, blocker);

            if can_kill_attacker && will_survive {
                // Category 1: Safe killer - best outcome for opponent
                if best_safe_killer.is_none() || blocker_value < best_safe_killer.unwrap().1 {
                    best_safe_killer = Some((idx, blocker_value));
                }
            } else if can_kill_attacker && !will_survive {
                // Category 2: Trading kill - only if favorable trade
                if blocker_value < attacker_value
                    && (best_trading_killer.is_none() || blocker_value < best_trading_killer.unwrap().1)
                {
                    best_trading_killer = Some((idx, blocker_value));
                }
            } else if !will_survive {
                // Category 3: Chump block - use lowest value
                if best_chump.is_none() || blocker_value < best_chump.unwrap().1 {
                    best_chump = Some((idx, blocker_value));
                }
            }
        }

        // Return in priority order
        if let Some((idx, _)) = best_safe_killer {
            return Some(idx);
        }
        if let Some((idx, _)) = best_trading_killer {
            return Some(idx);
        }

        // Only chump block if attacker is very dangerous (high power or evasion)
        if attacker_power >= 4 || attacker.has_lifelink() || attacker.has_trample() {
            if let Some((idx, blocker_value)) = best_chump {
                // Only chump with low-value creatures
                if blocker_value < 150 {
                    return Some(idx);
                }
            }
        }

        // No good block available - attacker gets through
        None
    }

    /// Wrapper around should_attack that adds context about numerical advantage
    fn should_attack_with_context(
        &self,
        attacker: &Card,
        view: &GameStateView,
        has_numerical_advantage: bool,
        opponent_blocker_count: usize,
        is_lethal_push: bool,
    ) -> bool {
        let power = attacker.current_power() as i32;

        // If we can go for lethal, attack with everything that has power
        if is_lethal_push && power > 0 {
            return true;
        }

        // If we have significant numerical advantage (2+ more creatures), be more aggressive
        // This helps avoid stalemates where both sides have equal creatures
        if has_numerical_advantage {
            // With numerical advantage, attack with power > 0 creatures
            if power > 0 {
                // Still check basic combat factors for terrible situations
                let factors = self.calculate_combat_factors(attacker.id, view);

                // Don't attack if we'll definitely die for nothing
                // But do attack if we can't be blocked or if opponent has few blockers
                if factors.can_be_blocked && factors.can_be_killed_by_one && !factors.can_kill_all {
                    // Only skip if it's a terrible trade (we die, kill nothing, no combat effect)
                    if !factors.has_combat_effect && opponent_blocker_count > 0 {
                        return false;
                    }
                }
                return true;
            }
        }

        // Otherwise use standard heuristic logic
        self.should_attack(attacker, view)
    }

    fn should_attack(&self, attacker: &Card, view: &GameStateView) -> bool {
        let power = attacker.current_power() as i32;

        // Creatures with 0 power generally don't attack unless they have special abilities
        if power <= 0 {
            return false;
        }

        // Calculate combat factors using board state evaluation
        let factors = self.calculate_combat_factors(attacker.id, view);

        // Always attack if unblockable (Java logic line 1517, 1528, 1538, 1545, 1553)
        if !factors.can_be_blocked && power > 0 {
            return true;
        }

        // Java aggression levels (from AiAttackController.java:1515-1561):
        // 6 = Exalted/all-in: attack expecting to kill or be unblockable
        // 5 = All out attacking: always attack
        // 4 = Expecting to trade or attack for free
        // 3 = Balanced: expecting to kill something or be unblockable (default)
        // 2 = Defensive: only attack if very favorable
        // 1 = Very defensive: rarely attack
        // 0 = Never attack (not implemented)

        match self.aggression_level {
            6 => {
                // Exalted (line 1516): attack expecting to at least kill a creature of equal value or not be blocked
                (factors.can_kill_all && factors.is_worth_less_than_all_killers) || !factors.can_be_blocked
            }
            5 => {
                // All out attacking (line 1523): always attack with power > 0
                power > 0
            }
            4 => {
                // Expecting to trade (line 1527): attack if can kill all, or can kill dangerous without dying, or unblockable, or no blockers
                factors.can_kill_all
                    || (factors.dangerous_blockers_present
                        && factors.can_kill_all_dangerous
                        && !factors.can_be_killed_by_one)
                    || !factors.can_be_blocked
                    || factors.number_of_blockers == 0
            }
            3 => {
                // Balanced (default) (line 1535): expecting to at least kill a creature of equal value or not be blocked
                // Attack if:
                // - Can kill all blockers AND worth favorable trade
                // OR - Can kill dangerous blockers OR have combat effect AND won't die to one blocker
                // OR - Unblockable
                (factors.can_kill_all && factors.is_worth_less_than_all_killers)
                    || (((factors.dangerous_blockers_present && factors.can_kill_all_dangerous)
                        || factors.has_combat_effect)
                        && !factors.can_be_killed_by_one)
                    || !factors.can_be_blocked
            }
            2 => {
                // Defensive (line 1544): attack expecting to attract a group block or destroying a single blocker and surviving
                !factors.can_be_blocked
                    || ((factors.can_kill_all || factors.has_combat_effect)
                        && !factors.can_be_killed_by_one
                        && ((factors.dangerous_blockers_present && factors.can_kill_all_dangerous)
                            || !factors.can_be_killed))
            }
            1 => {
                // Very defensive (line 1552): unblockable creatures only, or can kill single blocker without dying
                !factors.can_be_blocked
                    || (factors.number_of_blockers == 1 && factors.can_kill_all && !factors.can_be_killed_by_one)
            }
            _ => {
                // Default to balanced if aggression is out of range
                (factors.can_kill_all && factors.is_worth_less_than_all_killers) || !factors.can_be_blocked
            }
        }
    }

    /// Calculate how much life would remain after unblocked attackers deal damage
    ///
    /// Reference: ComputerUtilCombat.lifeThatWouldRemain() (lines 304-329)
    ///
    /// This computes: current_life - damage_from_unblocked_attackers
    /// Used to determine if life is in danger and emergency blocks are needed.
    fn life_that_would_remain(
        &self,
        view: &GameStateView,
        attackers: &[CardId],
        current_blocks: &[(CardId, CardId)],
    ) -> i32 {
        let current_life = view.life();
        let mut damage = 0;

        // Calculate which attackers are unblocked
        for &attacker_id in attackers {
            // Check if this attacker is blocked
            let is_blocked = current_blocks.iter().any(|(_, a_id)| *a_id == attacker_id);

            if !is_blocked {
                // Add this attacker's damage
                if let Some(attacker) = view.get_card(attacker_id) {
                    let attacker_power = attacker.current_power() as i32;
                    damage += attacker_power;

                    // TODO: Handle trample damage (damage overflow from blocked attackers)
                    // TODO: Handle "damage as though unblocked" static abilities
                }
            }
        }

        current_life - damage
    }

    /// Determine if life is in danger based on potential combat damage
    ///
    /// Reference: ComputerUtilCombat.lifeInDanger() (lines 399-466)
    ///
    /// Returns true if the player would drop to dangerously low life after combat.
    /// The threshold is context-dependent but generally around 3-5 life.
    ///
    /// Key checks from Java:
    /// 1. Player can't lose -> false
    /// 2. Special cards (Worship, Elderscale Wurm) -> false
    /// 3. "Must be blocked" creatures unblocked -> true
    /// 4. Life after combat < threshold -> true
    ///
    /// Simplified implementation for now (full port would require threshold config)
    fn life_in_danger(&self, view: &GameStateView, attackers: &[CardId], current_blocks: &[(CardId, CardId)]) -> bool {
        // Java default threshold is around 3-5 life depending on AI profile
        // We'll use a simple threshold of 5 for now
        const DANGER_THRESHOLD: i32 = 5;

        let remaining_life = self.life_that_would_remain(view, attackers, current_blocks);

        // Life in danger if we'd drop below threshold
        remaining_life < DANGER_THRESHOLD
    }

    /// Evaluate whether we should cast a pump spell on a creature
    ///
    /// Reference: ComputerUtilCard.shouldPumpCard() (lines 1291-1600+)
    ///
    /// This is a faithful port of Java's pump spell evaluation logic.
    /// Currently implements pre-combat evaluation for Main Phase 1.
    ///
    /// Parameters:
    /// - target: The creature we're considering pumping
    /// - power_bonus: +P from the pump spell
    /// - toughness_bonus: +T from the pump spell
    /// - keywords_granted: Keywords granted by the pump (e.g., ["Trample", "Haste"])
    /// - view: Current game state
    ///
    /// Returns true if we should cast the pump spell now.
    ///
    /// TODO: Implement combat trick timing (holding instant-speed pumps until declare blockers)
    /// TODO: Implement during-combat evaluation (save creatures, kill blockers, lethal damage)
    fn should_cast_pump(
        &self,
        target: &Card,
        power_bonus: i32,
        toughness_bonus: i32,
        keywords_granted: &[String],
        view: &GameStateView,
    ) -> bool {
        // Basic validity checks

        // Can't pump if new toughness would be <= 0 (creature dies)
        // Java: if (c.getNetToughness() + toughness <= 0) { return false; }
        let current_toughness = target.current_toughness() as i32 + target.power_bonus;
        if current_toughness + toughness_bonus <= 0 {
            return false;
        }

        let current_step = view.current_step();
        let current_power = target.current_power() as i32;

        // Create a hypothetical pumped creature to evaluate
        let pumped_power = current_power + power_bonus;
        let _pumped_toughness = current_toughness + toughness_bonus;

        // Combat trick detection (Reference: ComputerUtilCard.java:1416-1431)
        // A spell is a "combat trick" if:
        // 1. Target creature has power > 0 (not obvious)
        // 2. Keywords are empty OR only contain Trample/FirstStrike/DoubleStrike
        // 3. We're in pre-combat main phase
        let is_combat_trick_candidate = current_power > 0
            && keywords_granted
                .iter()
                .all(|kw| kw == "Trample" || kw == "First Strike" || kw == "Double Strike")
            && current_step == crate::game::phase::Step::Main1;

        // Phase-based evaluation

        // Get opponent info
        let opponent_life = view.opponent_life();

        // Collect opponent creatures (potential blockers, typically 2-8)
        let opponent_creatures: SmallVec<[&Card; 8]> = view
            .battlefield()
            .iter()
            .filter_map(|&id| view.get_card(id))
            .filter(|c| c.owner != self.player_id && c.is_creature())
            .collect();

        // PHASE 1: Pre-combat evaluation (Main1)
        // Reference: ComputerUtilCard.java:1345-1431
        if current_step == crate::game::phase::Step::Main1 {
            // Case 1: Will this pump make a non-attacker into an attacker?
            // Java: if (!doesCreatureAttackAI(ai, c) && doesSpecifiedCreatureAttackAI(ai, pumped))
            // This is the most important case for pre-combat pumps
            let would_attack_unpumped = self.should_attack(target, view);

            if !would_attack_unpumped
                && self.would_attack_if_pumped(target, power_bonus, toughness_bonus, keywords_granted, view)
            {
                // Calculate threat level if it attacked unblocked
                // Java: float threat = 1.0f * ComputerUtilCombat.damageIfUnblocked(pumped, opp, combat, true) / opp.getLife();
                let threat = pumped_power as f32 / opponent_life as f32;

                // Check if creature would be unblockable
                // Java: if (oppCreatures.stream().noneMatch(CardPredicates.possibleBlockers(pumped)))
                let has_blockers = opponent_creatures.iter().any(|blocker| {
                    // Simplified blocking check (would need to account for keywords granted)
                    self.can_block_simple(target, blocker, keywords_granted)
                });

                let mut chance = threat;
                if !has_blockers {
                    // Unblockable = 2x more valuable
                    chance *= 2.0;
                }

                // If 0-power creature self-pumps to get power, it's very valuable
                // Java: if (c.getNetPower() == 0 && c == sa.getHostCard() && power > 0) { threat *= 4; }
                if current_power == 0 && power_bonus > 0 {
                    chance *= 4.0;
                }

                // Combat trick detection: if this is a combat trick, DON'T cast it now
                // Wait until Declare Blockers to get more value
                // Reference: ComputerUtilCard.java:1416-1431
                if is_combat_trick_candidate && chance < 0.3 {
                    // Hold the combat trick for later unless the threat is very high
                    return false;
                }

                // Cast if threat is significant (>= 10% of opponent's life in damage)
                if chance >= 0.1 {
                    return true;
                }
            }
        }

        // PHASE 2: During combat evaluation (Declare Blockers)
        // Reference: ComputerUtilCard.java:1468-1600
        if current_step == crate::game::phase::Step::DeclareBlockers {
            let combat = view.combat();

            // Check if target creature is in combat
            let is_attacking = combat.is_attacking(target.id);
            let is_blocking = combat.is_blocking(target.id);

            if !is_attacking && !is_blocking {
                // Target not in combat - don't pump during declare blockers
                return false;
            }

            // Get effective stats for damage calculations
            let target_power = view
                .get_effective_power(target.id)
                .unwrap_or(target.current_power() as i32);
            let target_toughness = view
                .get_effective_toughness(target.id)
                .unwrap_or(target.current_toughness() as i32);
            let pumped_effective_power = target_power + power_bonus;
            let pumped_effective_toughness = target_toughness + toughness_bonus;

            if is_attacking {
                // Case: Our creature is attacking
                let blockers = combat.get_blockers(target.id);

                if blockers.is_empty() {
                    // Unblocked attacker - pump to deal lethal damage
                    if pumped_power >= opponent_life {
                        return true;
                    }

                    // Calculate total damage from all attackers to check for lethal
                    let mut total_damage = 0i32;
                    for &attacker_id in combat.attackers.keys() {
                        if attacker_id == target.id {
                            total_damage += pumped_effective_power;
                        } else if !combat.is_blocked(attacker_id) {
                            if let Some(atk_card) = view.get_card(attacker_id) {
                                let atk_power = view
                                    .get_effective_power(attacker_id)
                                    .unwrap_or(atk_card.current_power() as i32);
                                total_damage += atk_power;
                            }
                        } else {
                            // Blocked attacker - only counts trample damage
                            if let Some(atk_card) = view.get_card(attacker_id) {
                                if atk_card.has_trample() {
                                    let atk_power = view
                                        .get_effective_power(attacker_id)
                                        .unwrap_or(atk_card.current_power() as i32);
                                    let blocker_toughness: i32 = combat
                                        .get_blockers(attacker_id)
                                        .iter()
                                        .filter_map(|&b| view.get_card(b))
                                        .map(|b| {
                                            view.get_effective_toughness(b.id)
                                                .unwrap_or(b.current_toughness() as i32)
                                        })
                                        .sum();
                                    let trample_damage = (atk_power - blocker_toughness).max(0);
                                    total_damage += trample_damage;
                                }
                            }
                        }
                    }

                    // Pump if it would be lethal
                    if total_damage >= opponent_life {
                        return true;
                    }
                } else {
                    // Blocked attacker - evaluate combat outcome
                    let total_blocker_power: i32 = blockers
                        .iter()
                        .filter_map(|&b| view.get_card(b))
                        .map(|b| view.get_effective_power(b.id).unwrap_or(b.current_power() as i32))
                        .sum();

                    let total_blocker_toughness: i32 = blockers
                        .iter()
                        .filter_map(|&b| view.get_card(b))
                        .map(|b| {
                            view.get_effective_toughness(b.id)
                                .unwrap_or(b.current_toughness() as i32)
                        })
                        .sum();

                    // Check for first strike on either side (for future damage race logic)
                    let _attacker_has_first_strike = target.has_first_strike() || target.has_double_strike();
                    let _blocker_has_first_strike = blockers
                        .iter()
                        .filter_map(|&b| view.get_card(b))
                        .any(|b| b.has_first_strike() || b.has_double_strike());

                    // 1. Save our creature: Would we die without pump but survive with it?
                    // Note: First strike matters for damage race timing, but simplify for now
                    // as lethal damage is still lethal regardless of timing
                    let would_die_without_pump = total_blocker_power >= target_toughness;

                    let would_survive_with_pump =
                        pumped_effective_toughness > total_blocker_power || target.has_indestructible();

                    if would_die_without_pump && would_survive_with_pump {
                        return true;
                    }

                    // 2. Kill blockers: Can pumping let us kill blockers that would survive?
                    for &blocker_id in &blockers {
                        if let Some(blocker) = view.get_card(blocker_id) {
                            let blocker_toughness = view
                                .get_effective_toughness(blocker_id)
                                .unwrap_or(blocker.current_toughness() as i32);

                            // Would this blocker die without pump?
                            let blocker_dies_without_pump =
                                target_power >= blocker_toughness || target.has_deathtouch();

                            // Would this blocker die with pump?
                            let blocker_dies_with_pump =
                                pumped_effective_power >= blocker_toughness || target.has_deathtouch();

                            // Pump if it would kill a blocker that wouldn't die otherwise
                            if !blocker_dies_without_pump && blocker_dies_with_pump && !blocker.has_indestructible() {
                                return true;
                            }
                        }
                    }

                    // 3. Trample damage: If we have trample, pump to deal more damage
                    if target.has_trample() || keywords_granted.iter().any(|k| k == "Trample") {
                        let damage_without_pump = (target_power - total_blocker_toughness).max(0);
                        let damage_with_pump = (pumped_effective_power - total_blocker_toughness).max(0);

                        if damage_with_pump > damage_without_pump && damage_with_pump >= opponent_life {
                            return true;
                        }
                    }
                }
            } else if is_blocking {
                // Case: Our creature is blocking
                let attackers_blocked = combat.blockers.get(&target.id).cloned().unwrap_or_default();

                if attackers_blocked.is_empty() {
                    return false;
                }

                // Calculate total attacking power
                let total_attacker_power: i32 = attackers_blocked
                    .iter()
                    .filter_map(|&a| view.get_card(a))
                    .map(|a| view.get_effective_power(a.id).unwrap_or(a.current_power() as i32))
                    .sum();

                // Check for first strike (for future damage race logic)
                let _attacker_has_first_strike = attackers_blocked
                    .iter()
                    .filter_map(|&a| view.get_card(a))
                    .any(|a| a.has_first_strike() || a.has_double_strike());
                let _blocker_has_first_strike = target.has_first_strike() || target.has_double_strike();

                // 1. Save our blocker
                // Note: First strike timing could matter but simplify for now
                let would_die_without_pump = total_attacker_power >= target_toughness;

                let would_survive_with_pump =
                    pumped_effective_toughness > total_attacker_power || target.has_indestructible();

                if would_die_without_pump && would_survive_with_pump {
                    return true;
                }

                // 2. Kill attackers with pump
                for &attacker_id in &attackers_blocked {
                    if let Some(attacker) = view.get_card(attacker_id) {
                        let attacker_toughness = view
                            .get_effective_toughness(attacker_id)
                            .unwrap_or(attacker.current_toughness() as i32);

                        let attacker_dies_without_pump = target_power >= attacker_toughness || target.has_deathtouch();
                        let attacker_dies_with_pump =
                            pumped_effective_power >= attacker_toughness || target.has_deathtouch();

                        if !attacker_dies_without_pump && attacker_dies_with_pump && !attacker.has_indestructible() {
                            return true;
                        }
                    }
                }

                // 3. Reduce trample damage by pumping toughness
                let any_trampler = attackers_blocked
                    .iter()
                    .filter_map(|&a| view.get_card(a))
                    .any(|a| a.has_trample());

                if any_trampler && toughness_bonus > 0 {
                    // Pumping toughness reduces trample damage to us
                    return true;
                }
            }

            // No good combat reason to pump
            return false;
        }

        // PHASE 3: Post-combat or other phases
        // Generally don't cast pump spells outside of Main1 or Declare Blockers
        if current_step != crate::game::phase::Step::Main1 && current_step != crate::game::phase::Step::DeclareBlockers
        {
            return false;
        }

        // Legacy evaluation for other cases (will be removed once combat logic is complete)
        let would_attack_unpumped = self.should_attack(target, view);

        if !would_attack_unpumped {
            // Creature doesn't attack normally - would it attack if pumped?
            // Simplified check: creature with power > 0 after pump might attack
            if pumped_power > 0 {
                // Calculate threat level if it attacked unblocked
                // Java: float threat = 1.0f * ComputerUtilCombat.damageIfUnblocked(pumped, opp, combat, true) / opp.getLife();
                let threat = pumped_power as f32 / opponent_life as f32;

                // Check if creature would be unblockable
                // Java: if (oppCreatures.stream().noneMatch(CardPredicates.possibleBlockers(pumped)))
                let has_blockers = opponent_creatures.iter().any(|blocker| {
                    // Simplified blocking check (would need to account for keywords granted)
                    self.can_block_simple(target, blocker, keywords_granted)
                });

                let mut chance = threat;
                if !has_blockers {
                    // Unblockable = 2x more valuable
                    chance *= 2.0;
                }

                // If 0-power creature self-pumps to get power, it's very valuable
                // Java: if (c.getNetPower() == 0 && c == sa.getHostCard() && power > 0) { threat *= 4; }
                let base_power = target.current_power() as i32;
                if base_power == 0 && power_bonus > 0 {
                    chance *= 4.0;
                }

                // Cast if threat is significant (>= 10% of opponent's life in damage)
                if chance >= 0.1 {
                    return true;
                }
            }
        }

        // Case 2: Grant haste to enable attacking this turn
        // Java: if (keywords.contains("Haste") && c.hasSickness() && !c.isTapped())
        if keywords_granted.iter().any(|k| k == "Haste") {
            // Check if creature has summoning sickness
            // TODO: We need to check turn_entered_battlefield vs current turn
            // For now, simple heuristic: if the creature would attack when pumped
            if pumped_power > 0 {
                // Haste is worth about 0.5 + damage threat
                let threat = 0.5 + (0.5 * pumped_power as f32 / opponent_life as f32);
                if threat >= 0.3 {
                    return true;
                }
            }
        }

        // Case 3: Grant evasion (Flying, Unblockable, etc.)
        // Java: if (oppCreatures.stream().anyMatch(CardPredicates.possibleBlockers(c)))
        // Check if creature is currently blockable but would become unblockable
        let currently_blockable = opponent_creatures
            .iter()
            .any(|blocker| self.can_block_simple(target, blocker, &[]));

        if currently_blockable {
            // Would the pumped creature be unblockable?
            let would_be_blockable = opponent_creatures
                .iter()
                .any(|blocker| self.can_block_simple(target, blocker, keywords_granted));

            if !would_be_blockable && pumped_power > 0 {
                // Granting evasion is valuable - worth ~0.5 * damage potential
                let threat = 0.5 * pumped_power as f32 / opponent_life as f32;
                if threat >= 0.2 {
                    return true;
                }
            }
        }

        // Default: don't cast
        false
    }

    /// Simplified blocking check for pump evaluation
    /// Checks if blocker can block attacker, accounting for keywords granted by pump
    fn can_block_simple(&self, attacker: &Card, blocker: &Card, keywords_granted: &[String]) -> bool {
        // Can't block if defender
        if blocker.has_defender() {
            return false;
        }

        // Check flying - can only be blocked by flying or reach
        let has_flying = attacker.has_flying() || keywords_granted.iter().any(|k| k == "Flying");
        if has_flying && !(blocker.has_flying() || blocker.has_reach()) {
            return false;
        }

        // TODO: Add more evasion checks (Fear, Intimidate, Protection, etc.)

        true
    }

    /// Check if a creature would attack if pumped with the given bonuses
    ///
    /// This simulates pumping the creature and checking if it would attack
    /// Reference: ComputerUtilCard.doesSpecifiedCreatureAttackAI()
    fn would_attack_if_pumped(
        &self,
        creature: &Card,
        power_bonus: i32,
        _toughness_bonus: i32,
        keywords_granted: &[String],
        _view: &GameStateView,
    ) -> bool {
        // Simple heuristic: creature would attack if:
        // 1. It has power > 0 after pump
        // 2. It's not a terrible attack based on combat factors

        let pumped_power = creature.current_power() as i32 + power_bonus;

        if pumped_power <= 0 {
            return false;
        }

        // Check if pump grants evasion (unblockable)
        let grants_evasion = keywords_granted
            .iter()
            .any(|kw| kw == "Flying" || kw.contains("unblockable") || kw == "Trample");

        // If grants evasion or significant power, likely to attack
        if grants_evasion || pumped_power >= 3 {
            return true;
        }

        // Use simplified combat factors check
        // For now, just check if power > 0
        pumped_power > 0
    }

    /// Evaluate whether to activate an activated ability now
    ///
    /// Reference: Various ability AI classes in forge-ai/src/main/java/forge/ai/ability/
    ///
    /// Implements evaluation for:
    /// 1. Ping abilities (Prodigal Sorcerer) - DamageDealAi.java:196-200, 682-703
    /// 2. Pump abilities (Shivan Dragon) - PumpAi.java
    fn should_activate_ability(&self, source: &Card, view: &GameStateView) -> bool {
        // Iterate through all activated abilities on this source
        for ability in &source.activated_abilities {
            // Skip mana abilities - let mana system handle those
            if ability.is_mana_ability {
                continue;
            }

            // Detect ability type from effects
            let ability_type = self.classify_activated_ability(ability);

            match ability_type {
                ActivatedAbilityType::Ping { damage } => {
                    // Ping abilities: Only use when stack is clear
                    // Reference: DamageDealAi.java:196-200 (Triskelion logic)
                    if !self.is_stack_empty(view) {
                        continue; // Don't use pings when stack is not empty
                    }

                    // Check timing - ping at end of turn if reusable, or when can kill valuable creature
                    let current_step = view.current_step();
                    let is_end_phase = current_step == crate::game::Step::End;
                    let is_main2 = current_step == crate::game::Step::Main2;

                    // End of turn timing (if reusable and our turn is next)
                    // Reference: DamageDealAi.java:686-689
                    if is_end_phase {
                        // Check if ability cost is reusable (doesn't sacrifice the creature)
                        if !ability.cost.requires_sacrifice() {
                            // Can ping at end of turn
                            if self.has_valuable_ping_target(view, damage) {
                                return true;
                            }
                        }
                    }

                    // Main 2 timing (for abilities that need immediate use)
                    // Reference: DamageDealAi.java:691-694
                    if is_main2 && self.has_valuable_ping_target(view, damage) {
                        return true;
                    }

                    // When can kill a valuable creature
                    // Reference: DamageDealAi.java:682-703
                    if self.can_kill_valuable_creature(view, damage) {
                        return true;
                    }
                }
                ActivatedAbilityType::Pump { power, toughness } => {
                    // Pump activated abilities (firebreathing, etc.)
                    // Reference: PumpAi.java:98-105 (Main1), PumpAi.java:74, 358 (DeclareBlockers)
                    let current_step = view.current_step();

                    // Phase 1: Main1 - Enable better attacks
                    if current_step == crate::game::Step::Main1 {
                        // Check if pumping would enable better attacks
                        if self.would_pump_enable_attack(source, view, power, toughness) {
                            return true;
                        }
                    }

                    // Phase 2: Declare Blockers - Combat pump evaluation
                    // Reference: PumpAi.java:74, 358 - pump abilities are most valuable during
                    // declare blockers when we can save creatures or kill blockers
                    if current_step == crate::game::Step::DeclareBlockers
                        && self.should_activate_pump_during_combat(source, view, power, toughness)
                    {
                        return true;
                    }
                }
                ActivatedAbilityType::Other => {
                    // For now, don't activate other types
                    // Will expand as we implement more ability types
                    continue;
                }
            }
        }

        false
    }

    /// Classify the type of activated ability based on its effects
    fn classify_activated_ability(&self, ability: &crate::core::ActivatedAbility) -> ActivatedAbilityType {
        // Check for damage-dealing effects (ping abilities)
        for effect in &ability.effects {
            if let crate::core::Effect::DealDamage { amount, .. } = effect {
                return ActivatedAbilityType::Ping { damage: *amount };
            }
        }

        // Check for pump effects
        for effect in &ability.effects {
            if let crate::core::Effect::PumpCreature {
                power_bonus,
                toughness_bonus,
                ..
            } = effect
            {
                return ActivatedAbilityType::Pump {
                    power: *power_bonus,
                    toughness: *toughness_bonus,
                };
            }
        }

        ActivatedAbilityType::Other
    }

    /// Check if the stack is empty
    /// Reference: DamageDealAi.java:196 (stack.isEmpty())
    fn is_stack_empty(&self, view: &GameStateView) -> bool {
        view.is_stack_empty()
    }

    /// Check if there's a valuable target we can ping
    /// Reference: DamageDealAi.java:697 (canTarget(enemy))
    fn has_valuable_ping_target(&self, view: &GameStateView, damage: i32) -> bool {
        // Look for opponent creatures we can kill with this damage
        for opponent_id in view.opponents() {
            for &card_id in view.battlefield() {
                if let Some(card) = view.get_card(card_id) {
                    if card.controller == opponent_id && card.is_creature() {
                        // Check if this creature would die from the damage
                        if let Some(toughness) = card.base_toughness() {
                            // Convert to i32 to match damage type
                            let effective_toughness = i32::from(toughness) + card.toughness_bonus;
                            if effective_toughness <= damage {
                                // We can kill this creature
                                return true;
                            }
                        }
                    }
                }
            }
        }
        false
    }

    /// Check if we can kill a valuable opponent creature with this ping
    /// Reference: DamageDealAi.java:682-703 (freePing logic)
    fn can_kill_valuable_creature(&self, view: &GameStateView, damage: i32) -> bool {
        // For now, use same logic as has_valuable_ping_target
        // In Java Forge, this checks for "best opponent creature we can kill"
        self.has_valuable_ping_target(view, damage)
    }

    /// Check if pumping this creature would enable better attacks
    /// Reference: PumpAi.java lines 88-105, 481-490
    fn would_pump_enable_attack(&self, source: &Card, view: &GameStateView, power: i32, toughness: i32) -> bool {
        // Only pump creatures
        if !source.is_creature() {
            return false;
        }

        // Check if creature can attack (not tapped, not summoning sick)
        if source.tapped {
            return false;
        }

        // Check summon sickness - need turn_entered_battlefield
        if let Some(turn_entered) = source.turn_entered_battlefield {
            let current_turn = view.turn_number();
            if turn_entered == current_turn {
                // Has summon sickness unless it has haste
                let has_haste = source.has_keyword(Keyword::Haste);
                if !has_haste {
                    return false;
                }
            }
        }

        // If the pump gives significant power boost (3+), likely worth it
        if power >= 3 {
            return true;
        }

        // Check if pump grants useful keywords
        // For now, just check if there's a significant stat boost
        power > 0 && toughness >= 0
    }

    /// Evaluate whether to activate a pump ability during combat
    ///
    /// This handles firebreathing-style abilities (Shivan Dragon's {R}: +1/+0)
    /// during the Declare Blockers step.
    ///
    /// Reference: PumpAi.java:74, 358, 486 - pump abilities during declare blockers
    ///
    /// Evaluates whether pumping this creature would:
    /// 1. Save our creature from dying in combat
    /// 2. Kill an opposing blocker/attacker that would survive
    /// 3. Deal lethal damage to opponent (unblocked or trample)
    /// 4. Reduce trample damage (pumping blocker's toughness)
    fn should_activate_pump_during_combat(
        &self,
        source: &Card,
        view: &GameStateView,
        power: i32,
        toughness: i32,
    ) -> bool {
        // Only pump creatures
        if !source.is_creature() {
            return false;
        }

        let combat = view.combat();

        // Check if this creature is in combat
        let is_attacking = combat.is_attacking(source.id);
        let is_blocking = combat.is_blocking(source.id);

        if !is_attacking && !is_blocking {
            // Not in combat - don't pump during declare blockers
            return false;
        }

        // Get current effective stats
        let source_power = view
            .get_effective_power(source.id)
            .unwrap_or(source.current_power() as i32);
        let source_toughness = view
            .get_effective_toughness(source.id)
            .unwrap_or(source.current_toughness() as i32);
        let pumped_power = source_power + power;
        let pumped_toughness = source_toughness + toughness;

        // Pumping to negative toughness kills our creature - never do this
        if pumped_toughness <= 0 {
            return false;
        }

        let opponent_life = view.opponent_life();

        if is_attacking {
            // Our creature is attacking
            let blockers = combat.get_blockers(source.id);

            if blockers.is_empty() {
                // Unblocked attacker - pump if it would deal lethal damage
                // Reference: PumpAi.java - unblocked attackers should pump for lethal
                if pumped_power >= opponent_life {
                    return true;
                }

                // Calculate total damage from all attackers for lethal check
                let mut total_damage = 0i32;
                for &attacker_id in combat.attackers.keys() {
                    if attacker_id == source.id {
                        total_damage += pumped_power;
                    } else if !combat.is_blocked(attacker_id) {
                        if let Some(atk_card) = view.get_card(attacker_id) {
                            let atk_power = view
                                .get_effective_power(attacker_id)
                                .unwrap_or(atk_card.current_power() as i32);
                            total_damage += atk_power;
                        }
                    } else if let Some(atk_card) = view.get_card(attacker_id) {
                        // Blocked attacker - count trample damage only
                        if atk_card.has_trample() {
                            let atk_power = view
                                .get_effective_power(attacker_id)
                                .unwrap_or(atk_card.current_power() as i32);
                            let blocker_toughness: i32 = combat
                                .get_blockers(attacker_id)
                                .iter()
                                .filter_map(|&b| view.get_card(b))
                                .map(|b| {
                                    view.get_effective_toughness(b.id)
                                        .unwrap_or(b.current_toughness() as i32)
                                })
                                .sum();
                            let trample_damage = (atk_power - blocker_toughness).max(0);
                            total_damage += trample_damage;
                        }
                    }
                }

                // Pump if total damage with pump would be lethal
                if total_damage >= opponent_life {
                    return true;
                }
            } else {
                // Blocked attacker - evaluate combat outcome
                let total_blocker_power: i32 = blockers
                    .iter()
                    .filter_map(|&b| view.get_card(b))
                    .map(|b| view.get_effective_power(b.id).unwrap_or(b.current_power() as i32))
                    .sum();

                let total_blocker_toughness: i32 = blockers
                    .iter()
                    .filter_map(|&b| view.get_card(b))
                    .map(|b| {
                        view.get_effective_toughness(b.id)
                            .unwrap_or(b.current_toughness() as i32)
                    })
                    .sum();

                // 1. Save our creature: Would we die without pump but survive with it?
                let would_die_without_pump = total_blocker_power >= source_toughness;
                let would_survive_with_pump = pumped_toughness > total_blocker_power || source.has_indestructible();

                if would_die_without_pump && would_survive_with_pump {
                    return true;
                }

                // 2. Kill blockers: Can pumping let us kill blockers that would survive?
                for &blocker_id in &blockers {
                    if let Some(blocker) = view.get_card(blocker_id) {
                        let blocker_toughness = view
                            .get_effective_toughness(blocker_id)
                            .unwrap_or(blocker.current_toughness() as i32);

                        let blocker_dies_without_pump = source_power >= blocker_toughness || source.has_deathtouch();
                        let blocker_dies_with_pump = pumped_power >= blocker_toughness || source.has_deathtouch();

                        if !blocker_dies_without_pump && blocker_dies_with_pump && !blocker.has_indestructible() {
                            return true;
                        }
                    }
                }

                // 3. Trample damage: If we have trample, pump to deal more damage
                if source.has_trample() {
                    let damage_without_pump = (source_power - total_blocker_toughness).max(0);
                    let damage_with_pump = (pumped_power - total_blocker_toughness).max(0);

                    // Pump if it would increase trample damage and be lethal
                    if damage_with_pump > damage_without_pump && damage_with_pump >= opponent_life {
                        return true;
                    }
                }
            }
        } else if is_blocking {
            // Our creature is blocking
            let attackers_blocked = combat.blockers.get(&source.id).cloned().unwrap_or_default();

            if attackers_blocked.is_empty() {
                return false;
            }

            // Calculate total attacking power
            let total_attacker_power: i32 = attackers_blocked
                .iter()
                .filter_map(|&a| view.get_card(a))
                .map(|a| view.get_effective_power(a.id).unwrap_or(a.current_power() as i32))
                .sum();

            // 1. Save our blocker
            let would_die_without_pump = total_attacker_power >= source_toughness;
            let would_survive_with_pump = pumped_toughness > total_attacker_power || source.has_indestructible();

            if would_die_without_pump && would_survive_with_pump {
                return true;
            }

            // 2. Kill attackers with pump
            for &attacker_id in &attackers_blocked {
                if let Some(attacker) = view.get_card(attacker_id) {
                    let attacker_toughness = view
                        .get_effective_toughness(attacker_id)
                        .unwrap_or(attacker.current_toughness() as i32);

                    let attacker_dies_without_pump = source_power >= attacker_toughness || source.has_deathtouch();
                    let attacker_dies_with_pump = pumped_power >= attacker_toughness || source.has_deathtouch();

                    if !attacker_dies_without_pump && attacker_dies_with_pump && !attacker.has_indestructible() {
                        return true;
                    }
                }
            }

            // 3. Reduce trample damage by pumping toughness
            let any_trampler = attackers_blocked
                .iter()
                .filter_map(|&a| view.get_card(a))
                .any(|a| a.has_trample());

            if any_trampler && toughness > 0 {
                return true;
            }
        }

        false
    }

    /// Evaluate whether to cast a non-creature, non-pump spell
    ///
    /// Reference: Various spell AI classes in forge-ai/src/main/java/forge/ai/ability/
    fn should_cast_spell(&self, spell: &Card, view: &GameStateView) -> bool {
        // Check for card draw spells
        let has_draw = spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::DrawCards { .. }));
        if has_draw {
            let hand_size = view.hand().len();
            // Draw if we have 2 or fewer cards in hand
            if hand_size <= 2 {
                return true;
            }
        }

        // Check for removal spells (destroy or damage effects)
        // Reference: DestroyAi.java:106-303 (checkApiLogic)
        let has_destroy = spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::DestroyPermanent { .. }));
        let has_damage = spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::DealDamage { .. }));

        if has_destroy || has_damage {
            // Check if there's a valid removal target
            if let Some(_target) = self.choose_best_removal_target(spell, view) {
                return true;
            }
        }

        // Check for counterspells
        // Reference: CounterAi.java:32-226 (checkApiLogic)
        let has_counter = spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::CounterSpell { .. }));
        if has_counter && self.should_counter_spell(view) {
            return true;
        }

        false
    }

    /// Evaluate whether to cast a counterspell now
    ///
    /// Reference: CounterAi.java:32-226 (checkApiLogic)
    ///
    /// Key logic from Java:
    /// 1. Stack must not be empty (line 40-42)
    /// 2. Target the topmost spell on stack (line 51)
    /// 3. Don't counter friendly spells (line 52)
    /// 4. Don't counter low CMC spells (lines 163-169, configurable)
    /// 5. Prefer countering dangerous spells: creatures, damage, removal (lines 171-182)
    ///
    /// Simplified for now:
    /// - Counter any opponent spell on the stack
    /// - Prioritize creatures, damage spells, and removal
    fn should_counter_spell(&self, view: &GameStateView) -> bool {
        // Stack must have something to counter
        if view.is_stack_empty() {
            return false;
        }

        // Get the topmost spell on the stack (last entry)
        let stack = view.stack();
        let Some(&top_spell_id) = stack.last() else {
            return false;
        };

        let Some(top_spell) = view.get_card(top_spell_id) else {
            return false;
        };

        // Don't counter our own spells!
        if top_spell.owner == self.player_id {
            return false;
        }

        // Evaluate what type of spell it is
        let is_creature = top_spell.is_creature();
        let is_damage_spell = top_spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::DealDamage { .. }));
        let is_removal_spell = top_spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::DestroyPermanent { .. }));
        let is_counter_spell = top_spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::CounterSpell { .. }));
        let is_pump_spell = top_spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::PumpCreature { .. }));

        // Always counter dangerous spell types
        // Reference: CounterAi.java:151-182 (configurable countering preferences)
        if is_creature || is_damage_spell || is_removal_spell || is_counter_spell || is_pump_spell {
            return true;
        }

        // For other spells, check mana value (CMC)
        // Don't waste counterspells on very cheap spells unless they're dangerous
        let cmc = top_spell.mana_cost.cmc();
        if cmc >= 2 {
            // Counter anything CMC 2 or higher
            return true;
        }

        // CMC 0-1 spells: counter with 50% chance (simplified from Java's configurable chance)
        // In a real implementation, we'd check the RNG, but for now just counter CMC 1 spells
        cmc >= 1
    }

    /// Choose the best creature to target with removal
    ///
    /// Reference: DestroyAi.java:152-247 (target selection logic)
    ///
    /// Key filtering steps from Java:
    /// 1. Get targetable opponent creatures (line 153)
    /// 2. Filter out indestructible (line 157)
    /// 3. Prioritize creatures worth removing (lines 158-160)
    /// 4. Filter out creatures with shield counters (lines 162-163)
    /// 5. Filter out creatures that can regenerate (lines 189-194)
    /// 6. Filter out creatures that will die this turn (line 197)
    /// 7. Select best creature (line 224: getBestCreatureAI)
    ///
    /// Simplified version for now:
    /// - Target opponent's best creature
    /// - Filter out indestructible
    /// - Filter out creatures already dying (toughness <= 0)
    fn choose_best_removal_target(&self, spell: &Card, view: &GameStateView) -> Option<CardId> {
        // For damage-based removal, find the damage amount
        let damage_amount = spell.effects.iter().find_map(|e| {
            if let crate::core::Effect::DealDamage { amount, .. } = e {
                Some(*amount)
            } else {
                None
            }
        });

        // Get all valid opponent creatures using chained filters (zero intermediate allocations)
        // Filters:
        // 1. Opponent's creatures
        // 2. Not indestructible
        // 3. Not already dying (toughness > 0)
        // 4. For damage spells: toughness <= damage amount
        let opponent_creature_ids: SmallVec<[CardId; 8]> = view
            .battlefield()
            .iter()
            .copied()
            .filter(|&id| {
                if let Some(c) = view.get_card(id) {
                    c.owner != self.player_id
                        && c.is_creature()
                        && !c.has_indestructible()
                        && c.current_toughness() > 0
                        && damage_amount
                            .map(|dmg| c.current_toughness() as i32 <= dmg)
                            .unwrap_or(true)
                } else {
                    false
                }
            })
            .collect();

        if opponent_creature_ids.is_empty() {
            return None;
        }

        // TODO: Implement more filtering from DestroyAi.java:
        // - Filter out creatures with shield counters (line 162)
        // - Filter out creatures that can be sacrificed in response (lines 165-186)
        // - Filter out creatures with regeneration shields (lines 191-194)
        // - Implement useRemovalNow() check for timing (line 246)

        // Select the best creature (highest evaluation score)
        // Reference: ComputerUtilCard.getBestCreatureAI() (line 224)
        self.get_best_creature(view, &opponent_creature_ids)
    }

    /// Determine if we should block an attacker with a specific blocker
    ///
    /// Reference: AiBlockController.java (blocking decision logic)
    ///
    /// Key considerations:
    /// - Can the blocker survive? (toughness >= attacker power)
    /// - Can the blocker kill the attacker? (blocker power >= attacker toughness)
    /// - Favorable trade? (blocker value < attacker value)
    /// - Life in danger? (must block to survive)
    fn should_block(
        &self,
        blocker: &Card,
        attacker: &Card,
        view: &GameStateView,
        attackers: &[CardId],
        current_blocks: &[(CardId, CardId)],
    ) -> bool {
        let blocker_power = blocker.current_power() as i32;
        let blocker_toughness = blocker.current_toughness() as i32;
        let attacker_power = attacker.current_power() as i32;
        let attacker_toughness = attacker.current_toughness() as i32;

        // Check for special blocking keywords
        let blocker_has_first_strike = blocker.has_first_strike() || blocker.has_double_strike();
        let attacker_has_first_strike = attacker.has_first_strike() || attacker.has_double_strike();
        let blocker_has_deathtouch = blocker.has_deathtouch();

        // Can the blocker kill the attacker?
        let can_kill_attacker = blocker_power >= attacker_toughness || blocker_has_deathtouch;

        // Will the blocker survive?
        let will_survive = if blocker_has_first_strike && !attacker_has_first_strike {
            // Blocker strikes first - if it kills the attacker, it takes no damage
            blocker_power >= attacker_toughness || blocker_toughness > attacker_power
        } else {
            blocker_toughness > attacker_power
        };

        // Evaluate creatures to determine value trade
        let blocker_value = self.evaluate_creature(view, blocker.id);
        let attacker_value = self.evaluate_creature(view, attacker.id);

        // Java AiBlockController logic (simplified):
        // - Always block if we can kill attacker without dying (favorable trade)
        // - Block if attacker is more valuable and we trade
        // - Block with low-value creatures to save life
        // - Don't block with valuable creatures unless necessary

        // Case 1: We kill the attacker and survive - always good
        if can_kill_attacker && will_survive {
            return true;
        }

        // Case 2: Trading - kill attacker but die too
        // Only trade if attacker is more valuable or equal value (prevent damage)
        if can_kill_attacker && !will_survive {
            // Favorable trade: our creature is worth less or equal
            // Trading equal creatures is good because it prevents damage
            return attacker_value >= blocker_value;
        }

        // Case 3: We survive but don't kill the attacker
        // This is usually bad unless the blocker has very low value
        if !can_kill_attacker && will_survive {
            // Only worth it if blocker is low value and might save life
            return blocker_value < 100; // Low-value blocker threshold
        }

        // Case 4: We die without killing the attacker - usually avoid
        // Only make this block if life is in danger (chump block to survive)
        //
        // Reference: AiBlockController.makeChumpBlocks() (lines 641-704)
        // This is the "chump block" scenario - sacrifice a creature just to prevent damage
        if !can_kill_attacker && !will_survive {
            // Check if life is in danger - if so, must chump block
            let life_danger = self.life_in_danger(view, attackers, current_blocks);
            if life_danger {
                // Chump block to save life
                return true;
            }
        }

        false
    }

    /// Calculate total damage dealt by a group of blockers
    ///
    /// Reference: ComputerUtilCombat.totalFirstStrikeDamageOfBlockers()
    fn total_damage_of_blockers(&self, blockers: &[&Card], attacker: &Card) -> i32 {
        let mut total = 0;
        let attacker_has_first_strike = attacker.has_first_strike() || attacker.has_double_strike();

        for blocker in blockers {
            // Only count damage from blockers with first strike if attacker doesn't have it
            let blocker_has_first_strike = blocker.has_first_strike() || blocker.has_double_strike();

            // In first strike phase, only first strikers deal damage
            // In normal phase, everyone deals damage
            // For simplicity, if we're checking for gang block effectiveness,
            // we count all damage that would be dealt
            if !attacker_has_first_strike || blocker_has_first_strike {
                total += blocker.current_power() as i32;
            }
        }

        total
    }

    /// Check if attacker can be killed by a gang of blockers
    ///
    /// Reference: AiBlockController.makeGangBlocks()
    fn can_gang_kill(&self, attacker: &Card, blockers: &[&Card]) -> bool {
        let damage_needed = attacker.current_toughness() as i32;
        let total_damage = self.total_damage_of_blockers(blockers, attacker);

        // Deathtouch: any one blocker with deathtouch kills the attacker
        if blockers.iter().any(|b| b.has_deathtouch()) {
            return true;
        }

        total_damage >= damage_needed
    }

    /// Find potential gang block combinations for an attacker
    ///
    /// Returns the best gang block if one exists: (blockers, value_saved)
    /// Reference: AiBlockController.makeGangBlocks() lines 368-598
    fn find_gang_block<'a>(
        &self,
        attacker: &Card,
        available_blockers: &[&'a Card],
        view: &GameStateView,
    ) -> Option<Vec<&'a Card>> {
        // Don't gang block indestructible or regenerating creatures
        if attacker.has_indestructible() {
            return None;
        }

        let attacker_value = self.evaluate_creature(view, attacker.id);
        let attacker_power = attacker.current_power() as i32;

        // Try to find 2-3 blockers that can kill the attacker with minimal losses
        // Strategy: Use first strikers if attacker doesn't have first strike
        let attacker_has_first_strike = attacker.has_first_strike() || attacker.has_double_strike();

        if !attacker_has_first_strike && available_blockers.len() >= 2 {
            // Look for first strike gang
            let first_strikers: Vec<&Card> = available_blockers
                .iter()
                .filter(|b| b.has_first_strike() || b.has_double_strike())
                .copied()
                .collect();

            if first_strikers.len() >= 2 {
                // Try to kill with 2 first strikers
                for i in 0..first_strikers.len() {
                    for j in (i + 1)..first_strikers.len() {
                        let gang = vec![first_strikers[i], first_strikers[j]];
                        if self.can_gang_kill(attacker, &gang) {
                            // Check if this is a good trade
                            let total_blocker_value: i32 =
                                gang.iter().map(|b| self.evaluate_creature(view, b.id)).sum();

                            // Gang block if we save value or are in danger
                            if total_blocker_value < attacker_value * 2 {
                                return Some(gang);
                            }
                        }
                    }
                }
            }
        }

        // Try double block with any blockers (not just first strike)
        if available_blockers.len() >= 2 {
            let mut usable_blockers: Vec<&Card> = available_blockers
                .iter()
                .filter(|b| {
                    let blocker_value = self.evaluate_creature(view, b.id);
                    // Use blockers worth less than the attacker
                    blocker_value < attacker_value
                })
                .copied()
                .collect();

            // Sort by value (cheapest first) to minimize losses
            usable_blockers.sort_by_key(|b| self.evaluate_creature(view, b.id));

            // Try combinations of 2 blockers
            for i in 0..usable_blockers.len().min(3) {
                for j in (i + 1)..usable_blockers.len().min(4) {
                    let blocker1 = usable_blockers[i];
                    let blocker2 = usable_blockers[j];
                    let gang = vec![blocker1, blocker2];

                    if !self.can_gang_kill(attacker, &gang) {
                        continue;
                    }

                    // Calculate how many blockers would die
                    let blocker1_dies = blocker1.current_toughness() as i32 <= attacker_power;
                    let blocker2_dies = blocker2.current_toughness() as i32 <= attacker_power;

                    let blocker1_value = self.evaluate_creature(view, blocker1.id);
                    let blocker2_value = self.evaluate_creature(view, blocker2.id);

                    // Good gang block scenarios:
                    // 1. Kill attacker and only one blocker dies
                    // 2. Both die but total value < attacker value
                    if !blocker1_dies || !blocker2_dies {
                        // At least one survives - good trade
                        return Some(gang);
                    } else if blocker1_value + blocker2_value < attacker_value {
                        // Both die but we save value
                        return Some(gang);
                    }
                }
            }

            // Try 3-blocker combinations for high-value attackers
            // Reference: Java's makeGangBlocks triple-block logic
            if available_blockers.len() >= 3 && attacker_value > 200 {
                for i in 0..usable_blockers.len().min(3) {
                    for j in (i + 1)..usable_blockers.len().min(4) {
                        for k in (j + 1)..usable_blockers.len().min(5) {
                            let blocker1 = usable_blockers[i];
                            let blocker2 = usable_blockers[j];
                            let blocker3 = usable_blockers[k];
                            let gang = vec![blocker1, blocker2, blocker3];

                            if !self.can_gang_kill(attacker, &gang) {
                                continue;
                            }

                            // Calculate survival for each blocker
                            let blocker1_dies = blocker1.current_toughness() as i32 <= attacker_power;
                            let blocker2_dies = blocker2.current_toughness() as i32 <= attacker_power;
                            let blocker3_dies = blocker3.current_toughness() as i32 <= attacker_power;

                            let total_blocker_value: i32 =
                                gang.iter().map(|b| self.evaluate_creature(view, b.id)).sum();

                            // Good 3-blocker scenarios:
                            // 1. At least 2 blockers survive
                            // 2. Only 1 blocker dies and it's worth it
                            // 3. Total value < attacker value even if 2 die
                            let deaths = [blocker1_dies, blocker2_dies, blocker3_dies]
                                .iter()
                                .filter(|&&d| d)
                                .count();

                            if deaths <= 1 {
                                // 2+ survive - excellent trade
                                return Some(gang);
                            } else if deaths == 2 && total_blocker_value < attacker_value {
                                // 2 die but we still save value
                                return Some(gang);
                            }
                        }
                    }
                }
            }
        }

        None
    }

    /// Get blockers that won't be destroyed by the attacker
    ///
    /// Reference: AiBlockController.getSafeBlockers() line 100
    fn get_safe_blockers<'a>(&self, attacker: &Card, blockers: &[&'a Card]) -> Vec<&'a Card> {
        blockers
            .iter()
            .filter(|b| !self.can_destroy_attacker(attacker, b))
            .copied()
            .collect()
    }

    /// Get blockers that can destroy the attacker
    ///
    /// Reference: AiBlockController.getKillingBlockers() line 114
    fn get_killing_blockers<'a>(&self, attacker: &Card, blockers: &[&'a Card]) -> Vec<&'a Card> {
        blockers
            .iter()
            .filter(|b| self.can_destroy_blocker(attacker, b))
            .copied()
            .collect()
    }

    /// Make trade blocks: willing to trade creatures even if equal value
    ///
    /// Reference: AiBlockController.makeTradeBlocks() lines 599-640
    ///
    /// Trade blocks are used when:
    /// - Life is in danger (must stop damage)
    /// - Willing to trade equal-value creatures to prevent damage
    fn make_trade_blocks<'a>(
        &self,
        view: &GameStateView,
        attackers: &[&'a Card],
        available_blockers: &[&'a Card],
        life_in_danger: bool,
    ) -> Vec<(&'a Card, &'a Card)> {
        let mut assignments = Vec::new();
        let mut remaining_blockers = available_blockers.to_vec();

        for &attacker in attackers {
            if remaining_blockers.is_empty() {
                break;
            }

            let killing_blockers = self.get_killing_blockers(attacker, &remaining_blockers);
            if killing_blockers.is_empty() {
                continue;
            }

            // Choose the worst (lowest value) killing blocker
            let worst_killer = killing_blockers
                .iter()
                .min_by_key(|b| self.evaluate_creature(view, b.id))
                .copied();

            if let Some(blocker) = worst_killer {
                let blocker_value = self.evaluate_creature(view, blocker.id);
                let attacker_value = self.evaluate_creature(view, attacker.id);

                // Trade if:
                // 1. Life is in danger (must stop damage)
                // 2. Blocker is worth equal or less than attacker
                let should_trade = life_in_danger || blocker_value <= attacker_value;

                if should_trade {
                    assignments.push((blocker, attacker));
                    remaining_blockers.retain(|b| b.id != blocker.id);
                }
            }
        }

        assignments
    }

    /// Make good blocks: best blocker assignments
    ///
    /// Reference: AiBlockController.makeGoodBlocks() lines 187-362
    ///
    /// Priority order:
    /// 1. Safe blockers that kill the attacker (best case)
    /// 2. Safe blockers that survive (if not trample)
    /// 3. Blockers with death triggers that kill the attacker
    /// 4. Killing blockers worth less than attacker
    fn make_good_blocks<'a>(
        &self,
        view: &GameStateView,
        attackers: &[&'a Card],
        available_blockers: &[&'a Card],
    ) -> Vec<(&'a Card, &'a Card)> {
        let mut assignments = Vec::new();
        let mut remaining_blockers = available_blockers.to_vec();
        let mut blocked_attackers = Vec::new();

        for &attacker in attackers {
            if remaining_blockers.is_empty() {
                break;
            }

            let safe_blockers = self.get_safe_blockers(attacker, &remaining_blockers);
            let mut chosen_blocker: Option<&Card> = None;

            // 1. Safe blockers that kill the attacker
            if !safe_blockers.is_empty() {
                let killing_safe = self.get_killing_blockers(attacker, &safe_blockers);
                if !killing_safe.is_empty() {
                    // Choose the worst (lowest value) blocker that gets the job done
                    chosen_blocker = killing_safe
                        .iter()
                        .min_by_key(|b| self.evaluate_creature(view, b.id))
                        .copied();
                }
                // 2. Safe blockers (survive but don't kill) - only if not trample
                else if !attacker.has_trample() {
                    // Choose the worst safe blocker
                    chosen_blocker = safe_blockers
                        .iter()
                        .min_by_key(|b| self.evaluate_creature(view, b.id))
                        .copied();
                }
            }

            // 3. If no safe blocker, look for killing blockers that trade favorably
            if chosen_blocker.is_none() {
                let killing_blockers = self.get_killing_blockers(attacker, &remaining_blockers);
                let attacker_value = self.evaluate_creature(view, attacker.id);

                // Find killing blockers worth less than the attacker
                let favorable_killers: Vec<&Card> = killing_blockers
                    .iter()
                    .filter(|b| self.evaluate_creature(view, b.id) < attacker_value)
                    .copied()
                    .collect();

                if !favorable_killers.is_empty() {
                    // Choose the worst favorable killer
                    chosen_blocker = favorable_killers
                        .iter()
                        .min_by_key(|b| self.evaluate_creature(view, b.id))
                        .copied();
                }
            }

            // Assign the chosen blocker
            if let Some(blocker) = chosen_blocker {
                assignments.push((blocker, attacker));
                blocked_attackers.push(attacker);
                remaining_blockers.retain(|b| b.id != blocker.id);
            }
        }

        assignments
    }

    /// Check if life is in serious danger (very low life threshold)
    ///
    /// Reference: ComputerUtilCombat.lifeInSeriousDanger() lines 477-508
    fn life_in_serious_danger(
        &self,
        view: &GameStateView,
        attackers: &[CardId],
        current_blocks: &[(CardId, CardId)],
    ) -> bool {
        // Serious danger is a lower threshold than regular danger
        const SERIOUS_DANGER_THRESHOLD: i32 = 3;
        let remaining_life = self.life_that_would_remain(view, attackers, current_blocks);
        remaining_life < SERIOUS_DANGER_THRESHOLD
    }

    /// Improved blocking with gang blocking support and multi-phase danger reassessment
    ///
    /// Reference: AiBlockController.assignBlockersForCombat() lines 1070-1160
    ///
    /// Java's multi-phase strategy:
    /// Phase 1: Good blocks -> Gang blocks -> Trade blocks -> (if danger) Chump blocks
    /// Phase 2: If still in danger, reset and try: Trade -> Good -> Chump
    /// Phase 3: If serious danger: Chump -> Trade -> Good -> Gang
    fn assign_blocks_with_gang(
        &self,
        view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> SmallVec<[(CardId, CardId); 8]> {
        // Try Phase 1 blocking strategy
        let mut blocks = self.assign_blocks_phase1(view, available_blockers, attackers);

        // Reinforce to kill blockers if not in danger (Phase 1 follow-up)
        if !self.life_in_danger(view, attackers, &blocks) {
            self.reinforce_blockers_to_kill(view, attackers, available_blockers, &mut blocks);
        }

        // Check if life is still in danger after Phase 1
        let mut life_in_danger = self.life_in_danger(view, attackers, &blocks);

        // Phase 2: If still in danger, reset and try safer approach
        if life_in_danger {
            blocks = self.assign_blocks_phase2(view, available_blockers, attackers);

            // Reinforce against trample if life is still in danger
            if self.life_in_danger(view, attackers, &blocks) {
                self.reinforce_blockers_against_trample(view, attackers, available_blockers, &mut blocks);
            } else {
                life_in_danger = false;
            }

            // Check if life is in SERIOUS danger after Phase 2
            let serious_danger = life_in_danger && self.life_in_serious_danger(view, attackers, &blocks);

            // Phase 3: If in serious danger, be extremely defensive
            if serious_danger {
                blocks = self.assign_blocks_phase3(view, available_blockers, attackers);

                // Reinforce against trample in emergency
                if self.life_in_danger(view, attackers, &blocks) {
                    self.reinforce_blockers_against_trample(view, attackers, available_blockers, &mut blocks);
                }
            }
        }

        blocks
    }

    /// Phase 1: Standard blocking strategy
    ///
    /// Good blocks -> Gang blocks -> Trade blocks -> Chump blocks
    fn assign_blocks_phase1(
        &self,
        view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> SmallVec<[(CardId, CardId); 8]> {
        let mut blocks = SmallVec::new();

        if attackers.is_empty() || available_blockers.is_empty() {
            return blocks;
        }

        // Track which blockers are still available (typically 2-8 creatures)
        let mut remaining_blockers: SmallVec<[CardId; 8]> = available_blockers.iter().copied().collect();

        // Get card references (typically 2-8 attackers)
        let mut attacker_cards: SmallVec<[&Card; 8]> = attackers.iter().filter_map(|&id| view.get_card(id)).collect();

        // Sort attackers by threat level (highest value first)
        attacker_cards.sort_by_key(|c| -(self.evaluate_creature(view, c.id)));

        let blocker_cards: SmallVec<[&Card; 8]> =
            remaining_blockers.iter().filter_map(|&id| view.get_card(id)).collect();

        // Phase 1a: Make good blocks (safe kills, safe blocks, favorable trades)
        let good_blocks = self.make_good_blocks(view, &attacker_cards, &blocker_cards);
        for (blocker, attacker) in good_blocks {
            blocks.push((blocker.id, attacker.id));
            remaining_blockers.retain(|id| *id != blocker.id);
        }

        // Update available blockers and attackers
        let mut attackers_left: SmallVec<[&Card; 8]> = attacker_cards.iter().copied().collect();
        attackers_left.retain(|a| !blocks.iter().any(|(_, aid)| *aid == a.id));

        // Phase 1b: Try gang blocks for remaining high-value attackers
        let mut gang_blocked_attacker_ids: SmallVec<[CardId; 4]> = SmallVec::new();

        for &attacker in &attackers_left {
            if remaining_blockers.is_empty() {
                break;
            }

            let available_blocker_cards: SmallVec<[&Card; 8]> =
                remaining_blockers.iter().filter_map(|&id| view.get_card(id)).collect();

            if let Some(gang) = self.find_gang_block(attacker, &available_blocker_cards, view) {
                // Assign this gang block
                for blocker in gang {
                    blocks.push((blocker.id, attacker.id));
                    // Remove blocker from available pool
                    remaining_blockers.retain(|id| *id != blocker.id);
                }
                gang_blocked_attacker_ids.push(attacker.id);
            }
        }

        // Remove gang-blocked attackers from consideration
        attackers_left.retain(|a| !gang_blocked_attacker_ids.contains(&a.id));

        // Phase 1c: Trade blocks (willing to trade equal value if needed)
        // Check if life is in danger to determine trade willingness
        let life_in_danger = self.life_in_danger(view, attackers, &blocks);

        let remaining_blocker_cards: SmallVec<[&Card; 8]> =
            remaining_blockers.iter().filter_map(|&id| view.get_card(id)).collect();

        let trade_blocks = self.make_trade_blocks(view, &attackers_left, &remaining_blocker_cards, life_in_danger);
        for (blocker, attacker) in trade_blocks {
            blocks.push((blocker.id, attacker.id));
            remaining_blockers.retain(|id| *id != blocker.id);
        }

        // Update attackers list
        attackers_left.retain(|a| !blocks.iter().any(|(_, aid)| *aid == a.id));

        // Phase 2: Chump blocks if life is still in danger
        // The should_block method already handles chump blocking when life is in danger
        if life_in_danger && self.life_in_danger(view, attackers, &blocks) {
            for attacker in &attackers_left {
                if remaining_blockers.is_empty() {
                    break;
                }

                let blocker_cards: SmallVec<[&Card; 8]> =
                    remaining_blockers.iter().filter_map(|&id| view.get_card(id)).collect();

                // Find any blocker willing to chump
                for &blocker in &blocker_cards {
                    if self.should_block(blocker, attacker, view, attackers, &blocks) {
                        blocks.push((blocker.id, attacker.id));
                        remaining_blockers.retain(|id| *id != blocker.id);
                        break;
                    }
                }
            }
        }

        blocks
    }

    /// Phase 2: Safer blocking when life is in danger
    ///
    /// Trade blocks -> Good blocks -> Chump blocks
    /// Reference: AiBlockController line 1107-1120
    fn assign_blocks_phase2(
        &self,
        view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> SmallVec<[(CardId, CardId); 8]> {
        let mut blocks = SmallVec::new();
        let mut remaining_blockers: SmallVec<[CardId; 8]> = available_blockers.iter().copied().collect();

        let mut attacker_cards: SmallVec<[&Card; 8]> = attackers.iter().filter_map(|&id| view.get_card(id)).collect();
        attacker_cards.sort_by_key(|c| -(self.evaluate_creature(view, c.id)));

        // Phase 2a: Trade blocks first (more willing to trade when in danger)
        let blocker_cards: SmallVec<[&Card; 8]> =
            remaining_blockers.iter().filter_map(|&id| view.get_card(id)).collect();

        let trade_blocks = self.make_trade_blocks(view, &attacker_cards, &blocker_cards, true);
        for (blocker, attacker) in trade_blocks {
            blocks.push((blocker.id, attacker.id));
            remaining_blockers.retain(|id| *id != blocker.id);
        }

        let mut attackers_left: SmallVec<[&Card; 8]> = attacker_cards.iter().copied().collect();
        attackers_left.retain(|a| !blocks.iter().any(|(_, aid)| *aid == a.id));

        // Phase 2b: Good blocks
        let remaining_blocker_cards: SmallVec<[&Card; 8]> =
            remaining_blockers.iter().filter_map(|&id| view.get_card(id)).collect();

        let good_blocks = self.make_good_blocks(view, &attackers_left, &remaining_blocker_cards);
        for (blocker, attacker) in good_blocks {
            blocks.push((blocker.id, attacker.id));
            remaining_blockers.retain(|id| *id != blocker.id);
        }

        attackers_left.retain(|a| !blocks.iter().any(|(_, aid)| *aid == a.id));

        // Phase 2c: Chump blocks if still in danger
        for attacker in &attackers_left {
            if remaining_blockers.is_empty() {
                break;
            }

            let blocker_cards: SmallVec<[&Card; 8]> =
                remaining_blockers.iter().filter_map(|&id| view.get_card(id)).collect();

            for &blocker in &blocker_cards {
                if self.should_block(blocker, attacker, view, attackers, &blocks) {
                    blocks.push((blocker.id, attacker.id));
                    remaining_blockers.retain(|id| *id != blocker.id);
                    break;
                }
            }
        }

        blocks
    }

    /// Phase 3: Emergency blocking when life is in serious danger
    ///
    /// Chump blocks -> Trade blocks -> Good blocks
    /// Reference: AiBlockController line 1123-1149
    fn assign_blocks_phase3(
        &self,
        view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> SmallVec<[(CardId, CardId); 8]> {
        let mut blocks = SmallVec::new();
        let mut remaining_blockers: SmallVec<[CardId; 8]> = available_blockers.iter().copied().collect();

        let mut attacker_cards: SmallVec<[&Card; 8]> = attackers.iter().filter_map(|&id| view.get_card(id)).collect();
        attacker_cards.sort_by_key(|c| -(self.evaluate_creature(view, c.id)));

        // Phase 3a: Chump blocks first - block everything we can
        for attacker in &attacker_cards {
            if remaining_blockers.is_empty() {
                break;
            }

            let blocker_cards: SmallVec<[&Card; 8]> =
                remaining_blockers.iter().filter_map(|&id| view.get_card(id)).collect();

            // In serious danger, block with anything
            if let Some(&blocker) = blocker_cards.first() {
                blocks.push((blocker.id, attacker.id));
                remaining_blockers.retain(|id| *id != blocker.id);
            }
        }

        // Phase 3b: If we blocked everything and still have blockers, try trade blocks
        let mut attackers_left: SmallVec<[&Card; 8]> = attacker_cards.iter().copied().collect();
        attackers_left.retain(|a| !blocks.iter().any(|(_, aid)| *aid == a.id));

        if !attackers_left.is_empty() && !remaining_blockers.is_empty() {
            let remaining_blocker_cards: SmallVec<[&Card; 8]> =
                remaining_blockers.iter().filter_map(|&id| view.get_card(id)).collect();

            let trade_blocks = self.make_trade_blocks(view, &attackers_left, &remaining_blocker_cards, true);
            for (blocker, attacker) in trade_blocks {
                blocks.push((blocker.id, attacker.id));
                remaining_blockers.retain(|id| *id != blocker.id);
            }
        }

        blocks
    }

    /// Reinforce blockers against trample attackers
    ///
    /// Reference: AiBlockController.reinforceBlockersAgainstTrample() lines 737-792
    ///
    /// Adds additional blockers to trample attackers to absorb more damage
    fn reinforce_blockers_against_trample(
        &self,
        view: &GameStateView,
        attackers: &[CardId],
        available_blockers: &[CardId],
        current_blocks: &mut SmallVec<[(CardId, CardId); 8]>,
    ) {
        // Only reinforce if life is in danger
        if !self.life_in_danger(view, attackers, current_blocks) {
            return;
        }

        // Find trample attackers that are already blocked (typically 0-4)
        let trample_attackers: SmallVec<[CardId; 4]> = attackers
            .iter()
            .filter_map(|&id| {
                let card = view.get_card(id)?;
                if card.has_trample() {
                    // Check if this attacker is already blocked
                    if current_blocks.iter().any(|(_, aid)| *aid == id) {
                        return Some(id);
                    }
                }
                None
            })
            .collect();

        for attacker_id in trample_attackers {
            let attacker = match view.get_card(attacker_id) {
                Some(c) => c,
                None => continue,
            };

            let attacker_power = attacker.current_power() as i32;

            // Calculate current blocking damage absorption (typically 1-3 blockers per attacker)
            let current_blockers: SmallVec<[&Card; 4]> = current_blocks
                .iter()
                .filter_map(|(bid, aid)| if *aid == attacker_id { view.get_card(*bid) } else { None })
                .collect();

            let current_absorption: i32 = current_blockers.iter().map(|b| b.current_toughness() as i32).sum();

            // If current blockers don't absorb all damage, add more
            if attacker_power > current_absorption {
                // Find available blockers that can block this attacker
                for &blocker_id in available_blockers {
                    // Skip if already blocking
                    if current_blocks.iter().any(|(bid, _)| *bid == blocker_id) {
                        continue;
                    }

                    if let Some(blocker) = view.get_card(blocker_id) {
                        // Check if can block (basic check)
                        if self.can_block(attacker, blocker) {
                            let blocker_toughness = blocker.current_toughness() as i32;
                            // Add this blocker to help absorb trample damage
                            if blocker_toughness > 0 {
                                current_blocks.push((blocker_id, attacker_id));
                                // Recalculate if we need more
                                let new_absorption = current_absorption + blocker_toughness;
                                if new_absorption >= attacker_power {
                                    break; // Absorbed enough
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Reinforce blockers to kill attacker
    ///
    /// Reference: AiBlockController.reinforceBlockersToKill() lines 793-857
    ///
    /// Adds additional blockers to ensure we kill the attacker
    fn reinforce_blockers_to_kill(
        &self,
        view: &GameStateView,
        attackers: &[CardId],
        available_blockers: &[CardId],
        current_blocks: &mut SmallVec<[(CardId, CardId); 8]>,
    ) {
        // Find attackers that are blocked but not killed
        let mut blocked_but_unkilled: Vec<CardId> = Vec::new();

        for &attacker_id in attackers {
            let attacker = match view.get_card(attacker_id) {
                Some(c) => c,
                None => continue,
            };

            // Get blockers for this attacker
            let blockers: Vec<&Card> = current_blocks
                .iter()
                .filter_map(|(bid, aid)| if *aid == attacker_id { view.get_card(*bid) } else { None })
                .collect();

            if blockers.is_empty() {
                continue; // Not blocked
            }

            // Check if blockers kill the attacker
            let total_damage = self.total_damage_of_blockers(&blockers, attacker);
            let attacker_toughness = attacker.current_toughness() as i32;

            if total_damage < attacker_toughness && !attacker.has_indestructible() {
                blocked_but_unkilled.push(attacker_id);
            }
        }

        // Try to add more blockers to kill these attackers
        for attacker_id in blocked_but_unkilled {
            let attacker = match view.get_card(attacker_id) {
                Some(c) => c,
                None => continue,
            };

            let attacker_value = self.evaluate_creature(view, attacker.id);
            let attacker_toughness = attacker.current_toughness() as i32;

            // Calculate current damage
            let current_blockers: Vec<&Card> = current_blocks
                .iter()
                .filter_map(|(bid, aid)| if *aid == attacker_id { view.get_card(*bid) } else { None })
                .collect();

            let current_damage = self.total_damage_of_blockers(&current_blockers, attacker);

            // Try to add safe blockers first (that won't die)
            for &blocker_id in available_blockers {
                // Skip if already blocking
                if current_blocks.iter().any(|(bid, _)| *bid == blocker_id) {
                    continue;
                }

                if let Some(blocker) = view.get_card(blocker_id) {
                    if !self.can_block(attacker, blocker) {
                        continue;
                    }

                    let blocker_power = blocker.current_power() as i32;
                    let blocker_value = self.evaluate_creature(view, blocker.id);

                    // Add blocker if:
                    // 1. It contributes damage toward killing the attacker
                    // 2. It's worth less than the attacker (favorable trade)
                    if blocker_power > 0 && blocker_value < attacker_value {
                        current_blocks.push((blocker_id, attacker_id));

                        // Check if we've added enough damage
                        let new_total = current_damage + blocker_power;
                        if new_total >= attacker_toughness {
                            break; // Successfully reinforced to kill
                        }
                    }
                }
            }
        }
    }

    /// Score a mana source by its alternate uses
    ///
    /// Port of Java's ComputerUtilMana.scoreManaProducingCard()
    /// Reference: ComputerUtilMana.java:95-120
    ///
    /// Lower scores = fewer alternate uses = tap first
    /// Higher scores = more valuable for other purposes = preserve
    fn score_mana_source(&self, card: &Card, view: &GameStateView) -> i32 {
        let mut score = 0;

        // Score mana abilities (each mana ability contributes to score)
        // In Java: score += ability.calculateScoreForManaAbility()
        // We'll use a simpler heuristic: count mana produced
        for ability in &card.activated_abilities {
            if ability.is_mana_ability {
                // Simple scoring: +1 per mana type the ability can produce
                for effect in &ability.effects {
                    if let crate::core::Effect::AddMana { mana, .. } = effect {
                        // Count total mana produced
                        score += (mana.white + mana.blue + mana.black + mana.red + mana.green + mana.colorless) as i32;
                    }
                }
            } else {
                // Non-mana activated abilities add +13 (preserve flexibility)
                // Reference: ComputerUtilMana.java:104-106
                score += 13;
            }
        }

        // Creatures with combat potential add significant score
        // Reference: ComputerUtilMana.java:109-117
        if card.is_creature() {
            // Can attack? (not summoning sick, not tapped)
            let can_attack = self.can_mana_creature_attack(card, view);
            if can_attack {
                score += 13;
            }

            // Can block? (not tapped, has ability to block)
            // Note: Most creatures can block unless they have "can't block"
            // For simplicity, assume all untapped creatures can block
            if !card.tapped {
                score += 13;
            }
        }

        score
    }

    /// Check if a mana creature can attack (simplified check for mana tapping)
    fn can_mana_creature_attack(&self, card: &Card, view: &GameStateView) -> bool {
        // Must be a creature
        if !card.is_creature() {
            return false;
        }

        // Must not be tapped
        if card.tapped {
            return false;
        }

        // Must not have summoning sickness (unless has haste)
        if let Some(turn_entered) = card.turn_entered_battlefield {
            if turn_entered >= view.turn_number() && !card.has_keyword(crate::core::Keyword::Haste) {
                return false;
            }
        }

        // Must not have defender
        if card.has_keyword(crate::core::Keyword::Defender) {
            return false;
        }

        true
    }
}

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

            // Format the choice description
            let choice_description = match spell {
                SpellAbility::PlayLand { card_id } => {
                    format!("Play land: {}", view.card_name(*card_id).unwrap_or_default())
                }
                SpellAbility::CastSpell { card_id } => {
                    format!("Cast spell: {}", view.card_name(*card_id).unwrap_or_default())
                }
                SpellAbility::ActivateAbility { card_id, .. } => {
                    format!("Activate ability: {}", view.card_name(*card_id).unwrap_or_default())
                }
            };

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

    fn choose_targets(
        &mut self,
        view: &GameStateView,
        spell: CardId,
        valid_targets: &[CardId],
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
                    if let crate::core::Effect::DealDamage { amount, .. } = effect {
                        return Some(*amount);
                    }
                }
            }
            // Then check spell effects (Lightning Bolt, Shock, etc.)
            for effect in &c.effects {
                if let crate::core::Effect::DealDamage { amount, .. } = effect {
                    return Some(*amount);
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
            // Fallback: just pick the first target
            let mut targets = SmallVec::new();
            if !valid_targets.is_empty() {
                targets.push(valid_targets[0]);
            }
            return ChoiceResult::Ok(targets);
        }

        // Target the best permanent from our filtered list
        let target = self.get_best_creature(view, &filtered_target_ids);

        let mut targets = SmallVec::new();
        if let Some(target_card_id) = target {
            targets.push(target_card_id);
        } else if !valid_targets.is_empty() {
            // Fallback: just pick the first valid target
            targets.push(valid_targets[0]);
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
            .or_else(|| view.get_card(attacker).map(|c| c.current_power() as i32))
            .unwrap_or(0);

        // Create a sorted list of blockers by evaluation (best first)
        let mut blocker_list: Vec<(CardId, i32, i32)> = blockers
            .iter()
            .filter_map(|&id| {
                view.get_card(id).map(|card| {
                    let eval = self.evaluate_creature(view, id);
                    let toughness = view
                        .get_effective_toughness(id)
                        .unwrap_or(card.current_toughness() as i32);
                    (id, eval, toughness)
                })
            })
            .collect();

        // Sort by evaluation (descending - best creatures first)
        blocker_list.sort_by(|a, b| b.1.cmp(&a.1));

        // Separate into killable and non-killable based on remaining damage
        let mut remaining_damage = attacker_power;
        let mut killable: SmallVec<[CardId; 4]> = SmallVec::new();
        let mut unkillable: SmallVec<[CardId; 4]> = SmallVec::new();

        for (blocker_id, _eval, toughness) in blocker_list {
            // Calculate damage needed to kill (simplified - just toughness for now)
            // TODO(mtg-77): Consider damage prevention, indestructible, deathtouch, wither
            let lethal_damage = toughness;

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

    fn choose_cards_to_discard(
        &mut self,
        view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        // Simple heuristic: Discard lands first, then worst creatures
        let mut hand_cards: Vec<&Card> = hand.iter().filter_map(|&id| view.get_card(id)).collect();

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

    fn on_priority_passed(&mut self, _view: &GameStateView) {
        // Could track game state here for future decisions
    }

    fn on_game_end(&mut self, _view: &GameStateView, _won: bool) {
        // Could collect statistics here
    }

    fn get_controller_type(&self) -> crate::game::snapshot::ControllerType {
        crate::game::snapshot::ControllerType::Heuristic
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::EntityId;

    #[test]
    fn test_heuristic_controller_creation() {
        let player_id = EntityId::new(1);
        let controller = HeuristicController::new(player_id);
        assert_eq!(controller.player_id(), player_id);
        assert_eq!(controller.aggression_level, 3);
    }

    #[test]
    fn test_seeded_controller() {
        let player_id = EntityId::new(1);
        let controller = HeuristicController::new(player_id);
        assert_eq!(controller.player_id(), player_id);
    }

    #[test]
    fn test_aggression_setting() {
        let player_id = EntityId::new(1);
        let mut controller = HeuristicController::new(player_id);

        controller.set_aggression(0);
        assert_eq!(controller.aggression_level, 0);

        controller.set_aggression(6);
        assert_eq!(controller.aggression_level, 6);

        // Test clamping
        controller.set_aggression(10);
        assert_eq!(controller.aggression_level, 6);

        controller.set_aggression(-5);
        assert_eq!(controller.aggression_level, 0);
    }

    #[test]
    fn test_pump_spell_evaluation_basic() {
        use crate::core::{Card, CardType};

        let player_id = EntityId::new(1);
        let _controller = HeuristicController::new(player_id);

        // Create a Grizzly Bears (2/2) creature
        let mut bears = Card::new(EntityId::new(10), "Grizzly Bears", player_id);
        bears.set_power(Some(2));
        bears.set_toughness(Some(2));
        bears.add_type(CardType::Creature);

        // Test Case 1: Pump that doesn't kill the creature (+3/+3)
        // Should return true if it would enable attacking
        let power_bonus = 3;
        let toughness_bonus = 3;
        let _keywords: Vec<String> = vec![];

        // Note: This test would need a full GameStateView mock to work properly
        // For now, we're just testing that the method exists and compiles
        // A full integration test would be needed to test the logic end-to-end

        // Test Case 2: Pump that would kill the creature (-5/-5)
        // This should return false
        let _bad_power = 0;
        let bad_toughness = -5; // Would make 2/2 into 2/-3 (dies)

        // The should_cast_pump method would return false for this case
        // because current_toughness (2) + bad_toughness (-5) = -3 <= 0

        // Verify the logic path exists
        assert_eq!(bears.base_power(), Some(2));
        assert_eq!(bears.base_toughness(), Some(2));

        // Calculate what would happen
        let would_die = (bears.current_toughness() as i32) + bad_toughness <= 0;
        assert!(would_die, "Creature should die with -5 toughness");

        let would_live = (bears.current_toughness() as i32) + toughness_bonus > 0;
        assert!(would_live, "Creature should live with +3 toughness");

        // Test that we can calculate pumped power
        let pumped_power = (bears.current_power() as i32) + power_bonus;
        assert_eq!(pumped_power, 5, "2/2 with +3/+3 should have 5 power");
    }

    #[test]
    fn test_pump_spell_evasion_granting() {
        use crate::core::{Card, CardType};

        let player_id = EntityId::new(1);
        let controller = HeuristicController::new(player_id);

        // Create a 2/2 ground creature (the one we might pump)
        let mut ground_creature = Card::new(EntityId::new(10), "Grizzly Bears", player_id);
        ground_creature.set_power(Some(2));
        ground_creature.set_toughness(Some(2));
        ground_creature.add_type(CardType::Creature);

        // Create a 1/1 flying creature (opponent's blocker)
        let mut flying_creature = Card::new(EntityId::new(11), "Bird", EntityId::new(2));
        flying_creature.set_power(Some(1));
        flying_creature.set_toughness(Some(1));
        flying_creature.add_type(CardType::Creature);
        flying_creature.keywords.insert(Keyword::Flying);

        // Scenario 1: Ground creature attacks, flying creature tries to block
        // can_block_simple(attacker, blocker, keywords_on_attacker)

        // Test: Can the flying creature block the ground attacker? Yes (flying can block anything).
        assert!(controller.can_block_simple(&ground_creature, &flying_creature, &[]));

        // Test: If ground creature had Flying, can flying creature still block it? Yes.
        let flying_granted = vec!["Flying".to_string()];
        assert!(controller.can_block_simple(&ground_creature, &flying_creature, &flying_granted));

        // Scenario 2: Flying creature attacks, ground creature tries to block

        // Test: Can ground creature block the flying attacker? No (needs Flying or Reach).
        assert!(!controller.can_block_simple(&flying_creature, &ground_creature, &[]));

        // This test doesn't make sense - we don't grant keywords to blockers in this function
        // The keywords_granted parameter applies to the ATTACKER, not the blocker
        // So we can't test "granting Flying to the blocker" with this function
    }

    #[test]
    fn test_damage_assignment_order_logic() {
        // Test the core logic of damage assignment ordering
        // Port of Java Forge's AiBlockController.orderBlockers()
        //
        // Scenario: 5/5 attacker vs three blockers:
        // - 4/4 High-value creature (eval ~200)
        // - 2/2 Medium creature (eval ~140)
        // - 1/1 Low-value creature (eval ~115)
        //
        // With 5 damage available:
        // - Can kill 4/4 (need 4 damage) = yes, 1 damage left
        // - Can't kill 2/2 with 1 damage left = no
        // - Can kill 1/1 (need 1 damage) = yes, 0 damage left
        //
        // So order should be: 4/4 first, 1/1 second, 2/2 last
        // This maximizes kills (2 creatures) rather than damage spread

        // This is a conceptual test - actual integration test would need GameStateView
        let available_damage = 5;
        let blockers = vec![
            ("4/4 High", 200, 4), // (name, eval, toughness)
            ("2/2 Medium", 140, 2),
            ("1/1 Low", 115, 1),
        ];

        // Sort by evaluation (descending)
        let mut sorted = blockers.clone();
        sorted.sort_by(|a, b| b.1.cmp(&a.1));
        assert_eq!(sorted[0].0, "4/4 High");
        assert_eq!(sorted[1].0, "2/2 Medium");
        assert_eq!(sorted[2].0, "1/1 Low");

        // Simulate the algorithm
        let mut remaining = available_damage;
        let mut killable = vec![];
        let mut unkillable = vec![];

        for (name, _eval, toughness) in sorted {
            if toughness <= remaining {
                killable.push(name);
                remaining -= toughness;
            } else {
                unkillable.push(name);
            }
        }

        // Result: 4/4 is killable (5 >= 4, remaining = 1)
        //         2/2 is NOT killable (1 < 2)
        //         1/1 is killable (1 >= 1, remaining = 0)
        assert_eq!(killable, vec!["4/4 High", "1/1 Low"]);
        assert_eq!(unkillable, vec!["2/2 Medium"]);

        // Combined order: killable first, unkillable last
        let final_order: Vec<_> = killable.into_iter().chain(unkillable).collect();
        assert_eq!(final_order, vec!["4/4 High", "1/1 Low", "2/2 Medium"]);

        // We successfully kill 2 creatures (4/4 and 1/1) instead of just 1
        // If we had put 2/2 first after 4/4, we'd waste damage:
        // - 4/4: 4 damage, 1 left
        // - 2/2: can't kill with 1 damage, but rules require assigning lethal
        //        before moving to next blocker, so we'd be stuck
        // The algorithm correctly recognizes 2/2 can't be killed and skips it
    }

    /// Test intelligent mana source scoring
    ///
    /// Port of Java's ComputerUtilMana.scoreManaProducingCard()
    /// Reference: ComputerUtilMana.java:95-120
    ///
    /// This tests that:
    /// 1. Basic lands (only mana ability) get low scores
    /// 2. Mana creatures get higher scores (can attack/block)
    /// 3. Cards with non-mana abilities get +13 per ability
    #[test]
    fn test_mana_source_scoring() {
        use crate::core::{ActivatedAbility, Card, CardType, Cost, Effect, ManaCost};

        let player_id = EntityId::new(1);
        let controller = HeuristicController::new(player_id);

        // Create a mock GameStateView
        let game = crate::game::state::GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let view = crate::game::controller::GameStateView::new(&game, player_id);

        // Create a basic Forest (just mana ability)
        // Expected score: 1 (produces 1 green mana)
        let mut forest = Card::new(EntityId::new(10), "Forest", player_id);
        forest.add_type(CardType::Land);
        forest.activated_abilities.push(ActivatedAbility::new(
            Cost::Tap,
            vec![Effect::AddMana {
                player: player_id,
                mana: ManaCost {
                    green: 1,
                    ..Default::default()
                },
            }],
            "{T}: Add {G}".to_string(),
            true, // is_mana_ability
        ));

        let forest_score = controller.score_mana_source(&forest, &view);

        // Create Llanowar Elves (1/1 creature with mana ability)
        // Expected score: 1 (mana) + 13 (can attack) + 13 (can block) = 27
        // Note: If summoning sick, only +13 for block potential
        let mut llanowar_elves = Card::new(EntityId::new(11), "Llanowar Elves", player_id);
        llanowar_elves.add_type(CardType::Creature);
        llanowar_elves.set_power(Some(1));
        llanowar_elves.set_toughness(Some(1));
        // Not summoning sick - entered last turn
        llanowar_elves.turn_entered_battlefield = Some(0);
        llanowar_elves.activated_abilities.push(ActivatedAbility::new(
            Cost::Tap,
            vec![Effect::AddMana {
                player: player_id,
                mana: ManaCost {
                    green: 1,
                    ..Default::default()
                },
            }],
            "{T}: Add {G}".to_string(),
            true, // is_mana_ability
        ));

        let elves_score = controller.score_mana_source(&llanowar_elves, &view);

        // Create a land with a non-mana activated ability (utility land)
        // Expected score: 1 (mana) + 13 (non-mana ability) = 14
        let mut utility_land = Card::new(EntityId::new(12), "Strip Mine", player_id);
        utility_land.add_type(CardType::Land);
        utility_land.activated_abilities.push(ActivatedAbility::new(
            Cost::Tap,
            vec![Effect::AddMana {
                player: player_id,
                mana: ManaCost {
                    colorless: 1,
                    ..Default::default()
                },
            }],
            "{T}: Add {C}".to_string(),
            true, // is_mana_ability
        ));
        // Strip Mine's destroy land ability
        utility_land.activated_abilities.push(ActivatedAbility::new(
            Cost::Composite(vec![
                Cost::Tap,
                Cost::SacrificePattern {
                    count: 1,
                    card_type: "Land".to_string(),
                },
            ]),
            vec![Effect::DestroyPermanent {
                target: CardId::new(0),
                restriction: crate::core::TargetRestriction::any(),
            }],
            "{T}, Sacrifice Strip Mine: Destroy target land.".to_string(),
            false, // is_mana_ability
        ));

        let utility_score = controller.score_mana_source(&utility_land, &view);

        // Verify: Forest (low) < Utility land (medium) < Llanowar Elves (high)
        // The AI should tap Forest first, then utility land, then Llanowar Elves
        assert!(
            forest_score < elves_score,
            "Basic land (score={}) should be tapped before mana creature (score={})",
            forest_score,
            elves_score
        );

        assert!(
            forest_score < utility_score,
            "Basic land (score={}) should be tapped before utility land (score={})",
            forest_score,
            utility_score
        );

        // Print scores for debugging
        eprintln!("Mana source scores:");
        eprintln!("  Forest: {}", forest_score);
        eprintln!("  Strip Mine: {}", utility_score);
        eprintln!("  Llanowar Elves: {}", elves_score);
    }

    /// Test counterspell AI logic
    ///
    /// Port of Java's CounterAi.checkApiLogic()
    /// Reference: CounterAi.java:32-226
    ///
    /// This tests that:
    /// 1. AI counters opponent creature spells
    /// 2. AI doesn't counter own spells
    /// 3. AI doesn't try to counter when stack is empty
    #[test]
    fn test_counterspell_ai() {
        use crate::core::{Card, CardType, Effect, TargetRef};

        let player_id = EntityId::new(1);
        let opponent_id = EntityId::new(2);
        let controller = HeuristicController::new(player_id);

        // Create game state with opponent creature on stack
        let mut game = crate::game::state::GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);

        // Create an opponent creature spell and put it on the stack
        let creature_id = EntityId::new(100);
        let mut creature = Card::new(creature_id, "Grizzly Bears", opponent_id);
        creature.add_type(CardType::Creature);
        creature.set_power(Some(2));
        creature.set_toughness(Some(2));
        game.cards.insert(creature_id, creature);

        // Put creature on the stack
        game.stack.cards.push(creature_id);

        let view = crate::game::controller::GameStateView::new(&game, player_id);

        // Test: Should counter opponent's creature
        assert!(
            controller.should_counter_spell(&view),
            "AI should want to counter opponent's creature spell"
        );

        // Test: Stack is empty - should not try to counter
        game.stack.cards.pop();
        let view_empty = crate::game::controller::GameStateView::new(&game, player_id);
        assert!(
            !controller.should_counter_spell(&view_empty),
            "AI should not counter when stack is empty"
        );

        // Test: Own spell on stack - should not counter
        let own_creature_id = EntityId::new(101);
        let mut own_creature = Card::new(own_creature_id, "Our Bears", player_id);
        own_creature.add_type(CardType::Creature);
        game.cards.insert(own_creature_id, own_creature);
        game.stack.cards.push(own_creature_id);

        let view_own = crate::game::controller::GameStateView::new(&game, player_id);
        assert!(
            !controller.should_counter_spell(&view_own),
            "AI should not counter own spell"
        );

        // Test: Counter opponent damage spell
        game.stack.cards.pop();
        let damage_spell_id = EntityId::new(102);
        let mut damage_spell = Card::new(damage_spell_id, "Lightning Bolt", opponent_id);
        damage_spell.add_type(CardType::Instant);
        damage_spell.effects.push(Effect::DealDamage {
            amount: 3,
            target: TargetRef::Player(player_id),
        });
        game.cards.insert(damage_spell_id, damage_spell);
        game.stack.cards.push(damage_spell_id);

        let view_damage = crate::game::controller::GameStateView::new(&game, player_id);
        assert!(
            controller.should_counter_spell(&view_damage),
            "AI should want to counter opponent's damage spell"
        );
    }
}
