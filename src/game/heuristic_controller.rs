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

        // Phase 2: Cast creatures (best evaluation first)
        // IMPORTANT: Cast creatures BEFORE playing lands to ensure aggressive gameplay
        // TODO(mtg-XX): Evaluate creature quality and choose best
        // For now, just cast the first creature we find
        for ability in available {
            if matches!(ability, SpellAbility::CastSpell { .. }) {
                return Some(ability.clone());
            }
        }

        // Phase 2b: Activated abilities (especially removal during combat)
        // Check if any activated abilities are worth using
        // For now, just activate any non-mana ability (like Royal Assassin)
        // TODO(mtg-XX): Evaluate activated abilities intelligently
        // - Prioritize removal abilities during opponent's combat
        // - Check if there are valid targets
        // - Evaluate value of using the ability now vs later
        for ability in available {
            if matches!(ability, SpellAbility::ActivateAbility { .. }) {
                // For now, always activate available abilities
                // This will make Royal Assassin use its ability when available
                return Some(ability.clone());
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

        // Phase 4: Cast other spells
        // TODO(mtg-XX): Evaluate removal, pump spells, card draw, etc.
        // Java: Has separate logic for each spell type with evaluation functions

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

    /// Wrapper around should_attack that adds context about numerical advantage
    fn should_attack_with_context(
        &self,
        attacker: &Card,
        view: &GameStateView,
        has_numerical_advantage: bool,
        opponent_blocker_count: usize,
    ) -> bool {
        // If we have significant numerical advantage (2+ more creatures), be more aggressive
        // This helps avoid stalemates where both sides have equal creatures
        if has_numerical_advantage {
            // With numerical advantage, attack with power > 0 creatures
            let power = attacker.power.unwrap_or(0) as i32;
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

        // Evaluate each creature for attacking
        for creature in creatures {
            if self.should_attack_with_context(creature, view, has_numerical_advantage, opponent_blockers) {
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
        // Port of Java's AiBlockController.assignBlockersForCombat()
        // Reference: AiBlockController.java:998

        let mut blocks = SmallVec::new();

        if attackers.is_empty() || available_blockers.is_empty() {
            return blocks;
        }

        // Get card references
        let mut attacker_cards: Vec<&Card> = attackers.iter().filter_map(|&id| view.get_card(id)).collect();

        let blocker_cards: Vec<&Card> = available_blockers.iter().filter_map(|&id| view.get_card(id)).collect();

        // Sort attackers by threat level (evaluation score descending)
        // Block the most threatening creatures first
        attacker_cards.sort_by_key(|c| -(self.evaluate_creature(c)));

        // For each attacker (most threatening first), try to find a blocker
        for attacker in &attacker_cards {
            // Find best blocker for this attacker
            let mut best_blocker: Option<&Card> = None;
            let mut best_score = i32::MIN;

            for &blocker in &blocker_cards {
                // Skip if this blocker is already assigned
                if blocks.iter().any(|(b_id, _)| *b_id == blocker.id) {
                    continue;
                }

                // Check if this is a good block
                if self.should_block(blocker, attacker, view, attackers, &blocks) {
                    // Score this blocking assignment
                    // Prefer blockers that:
                    // 1. Kill the attacker and survive (best)
                    // 2. Trade favorably (kill high-value attacker with low-value blocker)
                    // 3. Minimize damage taken

                    let blocker_power = blocker.power.unwrap_or(0) as i32;
                    let blocker_toughness = blocker.toughness.unwrap_or(0) as i32;
                    let attacker_power = attacker.power.unwrap_or(0) as i32;
                    let attacker_toughness = attacker.toughness.unwrap_or(0) as i32;

                    let can_kill = blocker_power >= attacker_toughness || blocker.has_deathtouch();
                    let will_survive = blocker_toughness > attacker_power;

                    let score = if can_kill && will_survive {
                        1000 // Best case: kill and survive
                    } else if can_kill {
                        500 - self.evaluate_creature(blocker) // Trade, prefer cheaper blocker
                    } else if will_survive {
                        100 // Survive without killing (chump block)
                    } else {
                        -1000 // Both die - bad unless necessary
                    };

                    if score > best_score {
                        best_score = score;
                        best_blocker = Some(blocker);
                    }
                }
            }

            // Assign the best blocker if we found one
            if let Some(blocker) = best_blocker {
                blocks.push((blocker.id, attacker.id));
            }
        }

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
}
