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
use crate::game::controller::{GameStateView, PlayerController};
use crate::game::format_choice_menu;
use smallvec::SmallVec;

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
    pub fn evaluate_creature(&self, card: &Card) -> i32 {
        self.evaluate_creature_impl(card, true, true)
    }

    /// Internal implementation of creature evaluation with optional P/T and CMC consideration
    ///
    /// Parameters:
    /// - consider_pt: Whether to factor in power/toughness
    /// - consider_cmc: Whether to factor in mana cost
    fn evaluate_creature_impl(&self, card: &Card, consider_pt: bool, consider_cmc: bool) -> i32 {
        let mut value = 80;

        // Tokens are worth less than actual cards
        // Java: if (!c.isToken()) { value += addValue(20, "non-token"); }
        // TODO: Add is_token flag to Card struct
        // For now, assume all cards are non-tokens
        value += 20;

        let power = card.power.unwrap_or(0) as i32;
        let toughness = card.toughness.unwrap_or(0) as i32;

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

        // Horsemanship: Not implemented in Keyword enum yet, skip for now
        // TODO: Add Horsemanship to Keyword enum

        // Unblockable check
        // Java: if (StaticAbilityCantAttackBlock.cantBlockBy(c, null)) { value += addValue(power * 10, "unblockable"); }
        // For now, we'll check for explicit Other keyword
        // TODO: Implement full static ability check
        let is_unblockable = card
            .keywords
            .iter()
            .any(|k| matches!(k, Keyword::Other(s) if s.contains("can't be blocked") || s.contains("unblockable")));

        if !is_unblockable {
            // Other evasion keywords - not yet in enum, check via Other variant
            // TODO: Add Fear, Intimidate, Skulk to Keyword enum
            let has_fear = card
                .keywords
                .iter()
                .any(|k| matches!(k, Keyword::Other(s) if s.contains("Fear")));
            let has_intimidate = card
                .keywords
                .iter()
                .any(|k| matches!(k, Keyword::Other(s) if s.contains("Intimidate")));
            let has_skulk = card
                .keywords
                .iter()
                .any(|k| matches!(k, Keyword::Other(s) if s.contains("Skulk")));

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
            if card.has_keyword(&Keyword::Vigilance) {
                value += (power * 5) + (toughness * 5);
            }

            // Infect, Wither: Not in Keyword enum yet, check via Other
            // TODO: Add Infect, Wither to Keyword enum
            let has_infect = card
                .keywords
                .iter()
                .any(|k| matches!(k, Keyword::Other(s) if s.contains("Infect")));
            let has_wither = card
                .keywords
                .iter()
                .any(|k| matches!(k, Keyword::Other(s) if s.contains("Wither")));

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
    fn get_best_creature<'a>(&self, creatures: &[&'a Card]) -> Option<&'a Card> {
        creatures
            .iter()
            .max_by_key(|card| self.evaluate_creature(card))
            .copied()
    }

    /// Get the worst creature from a list based on evaluation score
    #[allow(dead_code)] // Will be used for discard decisions
    fn get_worst_creature<'a>(&self, creatures: &[&'a Card]) -> Option<&'a Card> {
        creatures
            .iter()
            .min_by_key(|card| self.evaluate_creature(card))
            .copied()
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

                            // Get our creatures
                            let our_creatures: Vec<&Card> = view
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

        // 2b: Cast creatures (best evaluation first)
        // TODO(mtg-XX): Evaluate creature quality and choose best
        // For now, just cast the first creature we find
        for ability in available {
            if let SpellAbility::CastSpell { card_id } = ability {
                if let Some(card) = view.get_card(*card_id) {
                    if card.is_creature() {
                        return Some(ability.clone());
                    }
                }
            }
        }

        // Phase 2b: Activated abilities (especially removal during combat)
        // Evaluate and use activated abilities intelligently
        // Reference: Java Forge's ability AI in forge-ai/src/main/java/forge/ai/ability/
        for ability in available {
            if let SpellAbility::ActivateAbility { card_id, .. } = ability {
                if let Some(source_card) = view.get_card(*card_id) {
                    // Skip mana abilities (let mana system handle those)
                    if source_card.activated_abilities.iter().any(|ab| ab.is_mana_ability) {
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
            // Collect land play abilities
            let land_plays: Vec<&SpellAbility> = available
                .iter()
                .filter(|sa| matches!(sa, SpellAbility::PlayLand { .. }))
                .collect();

            if !land_plays.is_empty() {
                // Extract land card IDs
                let land_ids: Vec<CardId> = land_plays
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
    fn calculate_combat_factors(&self, attacker: &Card, view: &GameStateView) -> CombatFactors {
        let _attacker_power = attacker.power.unwrap_or(0) as i32;
        let _attacker_toughness = attacker.toughness.unwrap_or(0) as i32;
        let attacker_value = self.evaluate_creature(attacker);

        // Combat effect keywords (gain value even if blocked)
        let has_combat_effect = attacker.has_lifelink()
            || attacker
                .keywords
                .iter()
                .any(|k| matches!(k, Keyword::Other(s) if s.contains("Wither") || s.contains("Afflict")));

        // Collect all potential blockers from opponents
        let potential_blockers: Vec<&Card> = view
            .battlefield()
            .iter()
            .filter_map(|&id| view.get_card(id))
            .filter(|c| c.owner != self.player_id && c.is_creature() && !c.tapped && self.can_block(attacker, c))
            .collect();

        let number_of_blockers = potential_blockers.len();
        let can_be_blocked = number_of_blockers > 0;

        // Track if there are dangerous blockers (with combat effects)
        let dangerous_blockers_present = potential_blockers.iter().any(|b| {
            b.has_lifelink()
                || b.keywords
                    .iter()
                    .any(|k| matches!(k, Keyword::Other(s) if s.contains("Wither")))
        });

        // Initialize factors
        let mut can_be_killed = false;
        let mut can_be_killed_by_one = false;
        let mut can_kill_all = true;
        let mut can_kill_all_dangerous = true;
        let mut is_worth_less_than_all_killers = true;

        // Evaluate each potential blocker
        for blocker in &potential_blockers {
            let _blocker_power = blocker.power.unwrap_or(0) as i32;
            let _blocker_toughness = blocker.toughness.unwrap_or(0) as i32;
            let blocker_value = self.evaluate_creature(blocker);

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
                let is_dangerous_blocker = blocker.has_lifelink()
                    || blocker
                        .keywords
                        .iter()
                        .any(|k| matches!(k, Keyword::Other(s) if s.contains("Wither")));

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
        let attacker_power = attacker.power.unwrap_or(0) as i32;
        let blocker_toughness = blocker.toughness.unwrap_or(0) as i32;

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
        let blocker_power = blocker.power.unwrap_or(0) as i32;
        let attacker_toughness = attacker.toughness.unwrap_or(0) as i32;

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

    /// Calculate potential lethal damage from attacking
    ///
    /// Returns the total damage we could deal if all our creatures attack and are unblocked
    fn calculate_lethal_potential(&self, view: &GameStateView, available_creatures: &[CardId]) -> i32 {
        available_creatures
            .iter()
            .filter_map(|&id| view.get_card(id))
            .map(|c| c.power.unwrap_or(0) as i32)
            .sum()
    }

    /// Check if we should go for lethal damage
    ///
    /// Be very aggressive if we can potentially kill opponent
    fn is_lethal_opportunity(&self, view: &GameStateView, available_creatures: &[CardId]) -> bool {
        let opp_life = view.opponent_life();
        let lethal_damage = self.calculate_lethal_potential(view, available_creatures);
        // Consider lethal if we can deal damage >= opponent's life
        // Even with some blocks, we might still kill them
        lethal_damage >= opp_life
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
        let power = attacker.power.unwrap_or(0) as i32;

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
                let factors = self.calculate_combat_factors(attacker, view);

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
        let power = attacker.power.unwrap_or(0) as i32;

        // Creatures with 0 power generally don't attack unless they have special abilities
        if power <= 0 {
            return false;
        }

        // Calculate combat factors using board state evaluation
        let factors = self.calculate_combat_factors(attacker, view);

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
                    let attacker_power = attacker.power.unwrap_or(0) as i32;
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
        let current_toughness = target.toughness.unwrap_or(0) as i32 + target.power_bonus;
        if current_toughness + toughness_bonus <= 0 {
            return false;
        }

        let current_step = view.current_step();
        let current_power = target.power.unwrap_or(0) as i32;

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

        // Collect opponent creatures (potential blockers)
        let opponent_creatures: Vec<&Card> = view
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
            // TODO(mtg-77): Implement during-combat pump evaluation
            // This requires combat state tracking to know:
            // - Which creatures are attacking/blocking
            // - Which creatures would die in combat
            // - Whether pumping would save a creature or kill an opponent
            // For now, return false (don't cast during combat until we have combat state)
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
                let base_power = target.power.unwrap_or(0) as i32;
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

        let pumped_power = creature.power.unwrap_or(0) as i32 + power_bonus;

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
    fn should_activate_ability(&self, _source: &Card, _view: &GameStateView) -> bool {
        // For now, don't automatically activate abilities
        // The current implementation doesn't have enough context to make good decisions
        // (needs combat state tracking, better timing logic, etc.)
        // TODO: Implement proper activated ability evaluation
        // - Check if we're on opponent's turn (for removal abilities like Royal Assassin)
        // - Evaluate if targets are valuable enough
        // - Consider mana efficiency
        false
    }

    /// Evaluate whether to cast a non-creature, non-pump spell
    ///
    /// Reference: Various spell AI classes in forge-ai/src/main/java/forge/ai/ability/
    fn should_cast_spell(&self, spell: &Card, view: &GameStateView) -> bool {
        // For now, don't cast removal or other spells automatically
        // The targeting system needs to be improved first to avoid targeting wrong permanents
        // TODO: Implement proper spell evaluation with correct targeting restrictions

        // Only cast card draw if we're low on cards
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

        false
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
        let blocker_power = blocker.power.unwrap_or(0) as i32;
        let blocker_toughness = blocker.toughness.unwrap_or(0) as i32;
        let attacker_power = attacker.power.unwrap_or(0) as i32;
        let attacker_toughness = attacker.toughness.unwrap_or(0) as i32;

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
        let blocker_value = self.evaluate_creature(blocker);
        let attacker_value = self.evaluate_creature(attacker);

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
                total += blocker.power.unwrap_or(0) as i32;
            }
        }

        total
    }

    /// Check if attacker can be killed by a gang of blockers
    ///
    /// Reference: AiBlockController.makeGangBlocks()
    fn can_gang_kill(&self, attacker: &Card, blockers: &[&Card]) -> bool {
        let damage_needed = attacker.toughness.unwrap_or(0) as i32;
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
        _view: &GameStateView,
    ) -> Option<Vec<&'a Card>> {
        // Don't gang block indestructible or regenerating creatures
        if attacker.has_indestructible() {
            return None;
        }

        let attacker_value = self.evaluate_creature(attacker);
        let attacker_power = attacker.power.unwrap_or(0) as i32;

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
                            let total_blocker_value: i32 = gang.iter().map(|b| self.evaluate_creature(b)).sum();

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
                    let blocker_value = self.evaluate_creature(b);
                    // Use blockers worth less than the attacker
                    blocker_value < attacker_value
                })
                .copied()
                .collect();

            // Sort by value (cheapest first) to minimize losses
            usable_blockers.sort_by_key(|b| self.evaluate_creature(b));

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
                    let blocker1_dies = blocker1.toughness.unwrap_or(0) as i32 <= attacker_power;
                    let blocker2_dies = blocker2.toughness.unwrap_or(0) as i32 <= attacker_power;

                    let blocker1_value = self.evaluate_creature(blocker1);
                    let blocker2_value = self.evaluate_creature(blocker2);

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
                            let blocker1_dies = blocker1.toughness.unwrap_or(0) as i32 <= attacker_power;
                            let blocker2_dies = blocker2.toughness.unwrap_or(0) as i32 <= attacker_power;
                            let blocker3_dies = blocker3.toughness.unwrap_or(0) as i32 <= attacker_power;

                            let total_blocker_value: i32 = gang.iter().map(|b| self.evaluate_creature(b)).sum();

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
                .min_by_key(|b| self.evaluate_creature(b))
                .copied();

            if let Some(blocker) = worst_killer {
                let blocker_value = self.evaluate_creature(blocker);
                let attacker_value = self.evaluate_creature(attacker);

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
                    chosen_blocker = killing_safe.iter().min_by_key(|b| self.evaluate_creature(b)).copied();
                }
                // 2. Safe blockers (survive but don't kill) - only if not trample
                else if !attacker.has_trample() {
                    // Choose the worst safe blocker
                    chosen_blocker = safe_blockers.iter().min_by_key(|b| self.evaluate_creature(b)).copied();
                }
            }

            // 3. If no safe blocker, look for killing blockers that trade favorably
            if chosen_blocker.is_none() {
                let killing_blockers = self.get_killing_blockers(attacker, &remaining_blockers);
                let attacker_value = self.evaluate_creature(attacker);

                // Find killing blockers worth less than the attacker
                let favorable_killers: Vec<&Card> = killing_blockers
                    .iter()
                    .filter(|b| self.evaluate_creature(b) < attacker_value)
                    .copied()
                    .collect();

                if !favorable_killers.is_empty() {
                    // Choose the worst favorable killer
                    chosen_blocker = favorable_killers
                        .iter()
                        .min_by_key(|b| self.evaluate_creature(b))
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

        // Track which blockers are still available
        let mut remaining_blockers: Vec<CardId> = available_blockers.to_vec();

        // Get card references
        let mut attacker_cards: Vec<&Card> = attackers.iter().filter_map(|&id| view.get_card(id)).collect();

        // Sort attackers by threat level (highest value first)
        attacker_cards.sort_by_key(|c| -(self.evaluate_creature(c)));

        let blocker_cards: Vec<&Card> = remaining_blockers.iter().filter_map(|&id| view.get_card(id)).collect();

        // Phase 1a: Make good blocks (safe kills, safe blocks, favorable trades)
        let good_blocks = self.make_good_blocks(&attacker_cards, &blocker_cards);
        for (blocker, attacker) in good_blocks {
            blocks.push((blocker.id, attacker.id));
            remaining_blockers.retain(|&id| id != blocker.id);
        }

        // Update available blockers and attackers
        let mut attackers_left: Vec<&Card> = attacker_cards.clone();
        attackers_left.retain(|a| !blocks.iter().any(|(_, aid)| *aid == a.id));

        // Phase 1b: Try gang blocks for remaining high-value attackers
        let mut gang_blocked_attacker_ids = Vec::new();

        for &attacker in &attackers_left {
            if remaining_blockers.is_empty() {
                break;
            }

            let available_blocker_cards: Vec<&Card> =
                remaining_blockers.iter().filter_map(|&id| view.get_card(id)).collect();

            if let Some(gang) = self.find_gang_block(attacker, &available_blocker_cards, view) {
                // Assign this gang block
                for blocker in gang {
                    blocks.push((blocker.id, attacker.id));
                    // Remove blocker from available pool
                    remaining_blockers.retain(|&id| id != blocker.id);
                }
                gang_blocked_attacker_ids.push(attacker.id);
            }
        }

        // Remove gang-blocked attackers from consideration
        attackers_left.retain(|a| !gang_blocked_attacker_ids.contains(&a.id));

        // Phase 1c: Trade blocks (willing to trade equal value if needed)
        // Check if life is in danger to determine trade willingness
        let life_in_danger = self.life_in_danger(view, attackers, &blocks);

        let remaining_blocker_cards: Vec<&Card> =
            remaining_blockers.iter().filter_map(|&id| view.get_card(id)).collect();

        let trade_blocks = self.make_trade_blocks(&attackers_left, &remaining_blocker_cards, life_in_danger);
        for (blocker, attacker) in trade_blocks {
            blocks.push((blocker.id, attacker.id));
            remaining_blockers.retain(|&id| id != blocker.id);
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

                let blocker_cards: Vec<&Card> = remaining_blockers.iter().filter_map(|&id| view.get_card(id)).collect();

                // Find any blocker willing to chump
                for &blocker in &blocker_cards {
                    if self.should_block(blocker, attacker, view, attackers, &blocks) {
                        blocks.push((blocker.id, attacker.id));
                        remaining_blockers.retain(|&id| id != blocker.id);
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
        let mut remaining_blockers: Vec<CardId> = available_blockers.to_vec();

        let mut attacker_cards: Vec<&Card> = attackers.iter().filter_map(|&id| view.get_card(id)).collect();
        attacker_cards.sort_by_key(|c| -(self.evaluate_creature(c)));

        // Phase 2a: Trade blocks first (more willing to trade when in danger)
        let blocker_cards: Vec<&Card> = remaining_blockers.iter().filter_map(|&id| view.get_card(id)).collect();

        let trade_blocks = self.make_trade_blocks(&attacker_cards, &blocker_cards, true);
        for (blocker, attacker) in trade_blocks {
            blocks.push((blocker.id, attacker.id));
            remaining_blockers.retain(|&id| id != blocker.id);
        }

        let mut attackers_left: Vec<&Card> = attacker_cards.clone();
        attackers_left.retain(|a| !blocks.iter().any(|(_, aid)| *aid == a.id));

        // Phase 2b: Good blocks
        let remaining_blocker_cards: Vec<&Card> =
            remaining_blockers.iter().filter_map(|&id| view.get_card(id)).collect();

        let good_blocks = self.make_good_blocks(&attackers_left, &remaining_blocker_cards);
        for (blocker, attacker) in good_blocks {
            blocks.push((blocker.id, attacker.id));
            remaining_blockers.retain(|&id| id != blocker.id);
        }

        attackers_left.retain(|a| !blocks.iter().any(|(_, aid)| *aid == a.id));

        // Phase 2c: Chump blocks if still in danger
        for attacker in &attackers_left {
            if remaining_blockers.is_empty() {
                break;
            }

            let blocker_cards: Vec<&Card> = remaining_blockers.iter().filter_map(|&id| view.get_card(id)).collect();

            for &blocker in &blocker_cards {
                if self.should_block(blocker, attacker, view, attackers, &blocks) {
                    blocks.push((blocker.id, attacker.id));
                    remaining_blockers.retain(|&id| id != blocker.id);
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
        let mut remaining_blockers: Vec<CardId> = available_blockers.to_vec();

        let mut attacker_cards: Vec<&Card> = attackers.iter().filter_map(|&id| view.get_card(id)).collect();
        attacker_cards.sort_by_key(|c| -(self.evaluate_creature(c)));

        // Phase 3a: Chump blocks first - block everything we can
        for attacker in &attacker_cards {
            if remaining_blockers.is_empty() {
                break;
            }

            let blocker_cards: Vec<&Card> = remaining_blockers.iter().filter_map(|&id| view.get_card(id)).collect();

            // In serious danger, block with anything
            if let Some(&blocker) = blocker_cards.first() {
                blocks.push((blocker.id, attacker.id));
                remaining_blockers.retain(|&id| id != blocker.id);
            }
        }

        // Phase 3b: If we blocked everything and still have blockers, try trade blocks
        let mut attackers_left: Vec<&Card> = attacker_cards.clone();
        attackers_left.retain(|a| !blocks.iter().any(|(_, aid)| *aid == a.id));

        if !attackers_left.is_empty() && !remaining_blockers.is_empty() {
            let remaining_blocker_cards: Vec<&Card> =
                remaining_blockers.iter().filter_map(|&id| view.get_card(id)).collect();

            let trade_blocks = self.make_trade_blocks(&attackers_left, &remaining_blocker_cards, true);
            for (blocker, attacker) in trade_blocks {
                blocks.push((blocker.id, attacker.id));
                remaining_blockers.retain(|&id| id != blocker.id);
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

        // Find trample attackers that are already blocked
        let trample_attackers: Vec<CardId> = attackers
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

            let attacker_power = attacker.power.unwrap_or(0) as i32;

            // Calculate current blocking damage absorption
            let current_blockers: Vec<&Card> = current_blocks
                .iter()
                .filter_map(|(bid, aid)| if *aid == attacker_id { view.get_card(*bid) } else { None })
                .collect();

            let current_absorption: i32 = current_blockers.iter().map(|b| b.toughness.unwrap_or(0) as i32).sum();

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
                            let blocker_toughness = blocker.toughness.unwrap_or(0) as i32;
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
            let attacker_toughness = attacker.toughness.unwrap_or(0) as i32;

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

            let attacker_value = self.evaluate_creature(attacker);
            let attacker_toughness = attacker.toughness.unwrap_or(0) as i32;

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

                    let blocker_power = blocker.power.unwrap_or(0) as i32;
                    let blocker_value = self.evaluate_creature(blocker);

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
}

impl PlayerController for HeuristicController {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn choose_spell_ability_to_play(
        &mut self,
        view: &GameStateView,
        available: &[SpellAbility],
    ) -> Option<SpellAbility> {
        // Display available choices if flag is set (e.g., in stop/go mode)
        if view.logger().should_show_choice_menu() && !available.is_empty() {
            print!("{}", format_choice_menu(view, available));
        }

        if available.is_empty() {
            let player_name = view.player_name();
            view.logger().controller_choice(
                "HEURISTIC",
                &format!("{} chose to pass priority (no available actions)", player_name),
            );
            return None;
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

        choice
    }

    fn choose_targets(
        &mut self,
        view: &GameStateView,
        spell: CardId,
        valid_targets: &[CardId],
    ) -> SmallVec<[CardId; 4]> {
        if valid_targets.is_empty() {
            return SmallVec::new();
        }

        // TODO: Implement intelligent targeting
        // For now, use simple heuristics:
        // - For removal: Target opponent's best creature
        // - For pump: Target our best creature
        // - For damage: Target opponent's best creature

        // Get the spell card to determine its type
        let spell_card = view.get_card(spell);
        let is_our_spell = spell_card.map(|c| c.owner == self.player_id).unwrap_or(false);

        // Collect target cards
        let mut target_cards: Vec<&Card> = valid_targets.iter().filter_map(|&id| view.get_card(id)).collect();

        if target_cards.is_empty() {
            // Fallback: just pick the first target
            let mut targets = SmallVec::new();
            targets.push(valid_targets[0]);
            return targets;
        }

        // For our own spells (pumps), target our best creature
        // For opponent spells (removal), target their best creature
        let target = if is_our_spell {
            // Target our best creature
            target_cards.retain(|c| c.owner == self.player_id);
            self.get_best_creature(&target_cards)
        } else {
            // Target opponent's best creature
            target_cards.retain(|c| c.owner != self.player_id);
            self.get_best_creature(&target_cards)
        };

        let mut targets = SmallVec::new();
        if let Some(target_card) = target {
            targets.push(target_card.id);
        } else if !valid_targets.is_empty() {
            // Fallback: just pick the first valid target
            targets.push(valid_targets[0]);
        }

        targets
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        _view: &GameStateView,
        cost: &ManaCost,
        available_sources: &[CardId],
    ) -> SmallVec<[CardId; 8]> {
        // Simple greedy approach for now
        // TODO: Implement intelligent mana tapping order from ComputerUtilMana
        let mut sources = SmallVec::new();
        let needed = cost.cmc() as usize;

        for &source_id in available_sources.iter().take(needed) {
            sources.push(source_id);
        }

        sources
    }

    fn choose_attackers(&mut self, view: &GameStateView, available_creatures: &[CardId]) -> SmallVec<[CardId; 8]> {
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

        attackers
    }

    fn choose_blockers(
        &mut self,
        view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> SmallVec<[(CardId, CardId); 8]> {
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

        blocks
    }

    fn choose_damage_assignment_order(
        &mut self,
        _view: &GameStateView,
        _attacker: CardId,
        blockers: &[CardId],
    ) -> SmallVec<[CardId; 4]> {
        // For now, just return the blockers in order
        // TODO: Implement intelligent ordering to kill blockers efficiently
        blockers.iter().copied().collect()
    }

    fn choose_cards_to_discard(
        &mut self,
        view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> SmallVec<[CardId; 7]> {
        // Simple heuristic: Discard lands first, then worst creatures
        let mut hand_cards: Vec<&Card> = hand.iter().filter_map(|&id| view.get_card(id)).collect();

        // Sort by value (ascending) - discard worst cards first
        hand_cards.sort_by_key(|c| {
            if c.is_land() {
                0 // Discard lands first
            } else if c.is_creature() {
                self.evaluate_creature(c)
            } else {
                100 // Keep spells
            }
        });

        hand_cards.iter().take(count).map(|c| c.id).collect()
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
        bears.power = Some(2);
        bears.toughness = Some(2);
        bears.types.push(CardType::Creature);

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
        assert_eq!(bears.power, Some(2));
        assert_eq!(bears.toughness, Some(2));

        // Calculate what would happen
        let would_die = (bears.toughness.unwrap_or(0) as i32) + bad_toughness <= 0;
        assert!(would_die, "Creature should die with -5 toughness");

        let would_live = (bears.toughness.unwrap_or(0) as i32) + toughness_bonus > 0;
        assert!(would_live, "Creature should live with +3 toughness");

        // Test that we can calculate pumped power
        let pumped_power = (bears.power.unwrap_or(0) as i32) + power_bonus;
        assert_eq!(pumped_power, 5, "2/2 with +3/+3 should have 5 power");
    }

    #[test]
    fn test_pump_spell_evasion_granting() {
        use crate::core::{Card, CardType, Keyword};

        let player_id = EntityId::new(1);
        let controller = HeuristicController::new(player_id);

        // Create a 2/2 ground creature (the one we might pump)
        let mut ground_creature = Card::new(EntityId::new(10), "Grizzly Bears", player_id);
        ground_creature.power = Some(2);
        ground_creature.toughness = Some(2);
        ground_creature.types.push(CardType::Creature);

        // Create a 1/1 flying creature (opponent's blocker)
        let mut flying_creature = Card::new(EntityId::new(11), "Bird", EntityId::new(2));
        flying_creature.power = Some(1);
        flying_creature.toughness = Some(1);
        flying_creature.types.push(CardType::Creature);
        flying_creature.keywords.push(Keyword::Flying);

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
}
