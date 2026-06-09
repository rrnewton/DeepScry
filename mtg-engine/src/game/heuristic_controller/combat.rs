//! Combat decisions: attack evaluation, block assignment, and combat math
//!
//! Part of the heuristic AI controller, split out of the former monolithic
//! `heuristic_controller.rs`. See `heuristic_controller/README.md` for the
//! submodule map. This is a pure structural refactor of the Java-Forge AI
//! port — no decision logic changed.

use super::*;

impl HeuristicController {
    /// Calculate combat factors for an attacker against available blockers
    ///
    /// Reference: AiAttackController.SpellAbilityFactors.calculate() (lines 1374-1454)
    pub(crate) fn calculate_combat_factors(&self, attacker_id: CardId, view: &GameStateView) -> CombatFactors {
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
        // Use can_block_with_view for full landwalk support
        let potential_blockers: SmallVec<[&Card; 8]> = view
            .battlefield()
            .iter()
            .filter_map(|&id| view.get_card(id))
            .filter(|c| {
                c.owner != self.player_id && c.is_creature() && !c.tapped && self.can_block_with_view(attacker, c, view)
            })
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
    /// Reference: CombatUtil.canBlock() in Java Forge
    /// Implements blocking restrictions based on evasion abilities and protection.
    ///
    /// Note: This is a simplified version that doesn't check landwalk.
    /// Use `can_block_with_view` when view is available for full landwalk support.
    pub(crate) fn can_block(&self, attacker: &Card, blocker: &Card) -> bool {
        self.can_block_impl(attacker, blocker, None)
    }

    /// Check if a blocker can block an attacker with full landwalk support
    ///
    /// This version takes a GameStateView to check if the defending player
    /// controls a land of the type the attacker has landwalk for.
    pub(crate) fn can_block_with_view(&self, attacker: &Card, blocker: &Card, view: &GameStateView) -> bool {
        self.can_block_impl(attacker, blocker, Some(view))
    }

    /// Implementation of blocking check with optional view for landwalk
    ///
    /// Reference: CombatUtil.canBlock() in Java Forge
    /// Implements blocking restrictions based on evasion abilities and protection.
    pub(crate) fn can_block_impl(&self, attacker: &Card, blocker: &Card, view: Option<&GameStateView>) -> bool {
        // Defender can't block (creatures with Defender can't attack, but CAN block)
        // NOTE: has_defender() on BLOCKER is wrong - Defender doesn't prevent blocking
        // Defender prevents ATTACKING, not blocking. A Wall with Defender can still block.

        // Flying: can only be blocked by flying or reach
        // Reference: CR 702.9b
        if attacker.has_flying() && !(blocker.has_flying() || blocker.has_reach()) {
            return false;
        }

        // Horsemanship: can only be blocked by creatures with horsemanship
        // Reference: CR 702.31
        if attacker.has_horsemanship() && !blocker.has_horsemanship() {
            return false;
        }

        // Shadow: can only be blocked by creatures with shadow, and
        // creatures with shadow can only block creatures with shadow
        // Reference: CR 702.28
        if attacker.has_shadow() != blocker.has_shadow() {
            // Shadow creatures can only be blocked by shadow creatures
            // Non-shadow creatures can only be blocked by non-shadow creatures
            return false;
        }

        // Fear: can only be blocked by artifact creatures or black creatures
        // Reference: CR 702.36
        if attacker.has_fear() {
            let is_artifact = blocker.is_artifact();
            let is_black = blocker.is_color(crate::core::Color::Black);
            if !is_artifact && !is_black {
                return false;
            }
        }

        // Intimidate: can only be blocked by artifact creatures or creatures
        // that share a color with this creature
        // Reference: CR 702.13
        if attacker.has_intimidate() {
            let is_artifact = blocker.is_artifact();
            let shares_color = attacker.colors.iter().any(|c| blocker.is_color(*c));
            if !is_artifact && !shares_color {
                return false;
            }
        }

        // Skulk: can only be blocked by creatures with greater power
        // Reference: CR 702.119
        if attacker.has_skulk() {
            let blocker_power = blocker.current_power();
            let attacker_power = attacker.current_power();
            if blocker_power <= attacker_power {
                return false;
            }
        }

        // Landwalk: can't be blocked if defending player controls a land of the appropriate type
        // Reference: CR 702.14
        if attacker.has_keyword(Keyword::Landwalk) {
            if let Some(view) = view {
                // Check each landwalk type the creature has
                for keyword_args in attacker.keywords.iter_args() {
                    if let KeywordArgs::Landwalk { land_type } = keyword_args {
                        // Check if defending player (blocker's controller) controls a land with this subtype
                        let defender_has_land =
                            view.battlefield().iter().filter_map(|&id| view.get_card(id)).any(|c| {
                                c.controller == blocker.controller
                                    && c.is_land()
                                    && c.subtypes
                                        .iter()
                                        .any(|st| st.as_str().eq_ignore_ascii_case(land_type.as_str()))
                            });

                        if defender_has_land {
                            // Attacker can't be blocked due to landwalk
                            return false;
                        }
                    }
                }
            }
        }

        // Protection from color: creature with protection can't be blocked
        // by creatures of that color
        // Reference: CR 702.16
        // Check if attacker has protection from blocker's colors
        for color in &blocker.colors {
            if attacker.has_protection_from(*color) {
                return false;
            }
        }

        // CR 509.1b / 509.4: per-creature block restriction (Ironclaw Orcs:
        // "can't block creatures with power 2 or greater"). Mirrors
        // combat_rules::can_block so the AI never proposes a block the engine
        // would then silently drop.
        for static_ability in &blocker.static_abilities {
            if let crate::core::StaticAbility::CantBlockMatching { attacker_filter, .. } = static_ability {
                if attacker_filter.matches(attacker) {
                    return false;
                }
            }
        }

        // Menace requires at least 2 blockers (simplified check)
        // In a full implementation, this would be context-dependent
        // For now we allow single blocking to preserve existing logic
        // The actual enforcement happens in declare_blockers

        true
    }

    /// Check if attacker can destroy blocker in combat
    ///
    /// Reference: ComputerUtilCombat.canDestroyBlocker()
    pub(crate) fn can_destroy_blocker(&self, attacker: &Card, blocker: &Card) -> bool {
        let attacker_power = i32::from(attacker.current_power());
        let blocker_toughness = i32::from(blocker.current_toughness());

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
    pub(crate) fn can_destroy_attacker(&self, attacker: &Card, blocker: &Card) -> bool {
        let blocker_power = i32::from(blocker.current_power());
        let attacker_toughness = i32::from(attacker.current_toughness());

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
    pub(crate) fn count_opponent_blockers(&self, view: &GameStateView) -> usize {
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
    pub(crate) fn calculate_lethal_potential(&self, view: &GameStateView, available_creatures: &[CardId]) -> i32 {
        available_creatures
            .iter()
            .filter_map(|&id| view.get_card(id))
            .map(|c| i32::from(c.current_power()))
            .sum()
    }

    /// Check if we should go for lethal damage
    ///
    /// Be very aggressive if we can potentially kill opponent
    pub(crate) fn is_lethal_opportunity(&self, view: &GameStateView, available_creatures: &[CardId]) -> bool {
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
    pub(crate) fn predict_combat_outcome(&self, view: &GameStateView, attackers: &[CardId]) -> CombatOutcome {
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
                .unwrap_or_else(|| i32::from(attacker.current_power()));

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
                            .unwrap_or_else(|| i32::from(blocker.current_toughness()));
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
    pub(crate) fn can_attacker_be_blocked(&self, attacker: &Card, blockers: &[&Card]) -> bool {
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
    pub(crate) fn find_best_blocker_for_attacker(
        &self,
        attacker: &Card,
        blockers: &[&Card],
        view: &GameStateView,
    ) -> Option<usize> {
        if blockers.is_empty() {
            return None;
        }

        let attacker_value = self.evaluate_creature(view, attacker.id);
        let attacker_power = i32::from(attacker.current_power());

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
    pub(crate) fn should_attack_with_context(
        &self,
        attacker: &Card,
        view: &GameStateView,
        has_numerical_advantage: bool,
        opponent_blocker_count: usize,
        is_lethal_push: bool,
    ) -> bool {
        let power = i32::from(attacker.current_power());

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

    pub(crate) fn should_attack(&self, attacker: &Card, view: &GameStateView) -> bool {
        let power = i32::from(attacker.current_power());

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
    pub(crate) fn life_that_would_remain(
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
                    let attacker_power = i32::from(attacker.current_power());
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
    pub(crate) fn life_in_danger(
        &self,
        view: &GameStateView,
        attackers: &[CardId],
        current_blocks: &[(CardId, CardId)],
    ) -> bool {
        // Java default threshold is around 3-5 life depending on AI profile
        // We'll use a simple threshold of 5 for now
        const DANGER_THRESHOLD: i32 = 5;

        let remaining_life = self.life_that_would_remain(view, attackers, current_blocks);

        // Life in danger if we'd drop below threshold
        remaining_life < DANGER_THRESHOLD
    }

    /// Simplified blocking check for pump evaluation
    /// Checks if blocker can block attacker, accounting for keywords granted by pump
    pub(crate) fn can_block_simple(&self, attacker: &Card, blocker: &Card, keywords_granted: &[String]) -> bool {
        // Check flying - can only be blocked by flying or reach
        let has_flying = attacker.has_flying() || keywords_granted.iter().any(|k| k == "Flying");
        if has_flying && !(blocker.has_flying() || blocker.has_reach()) {
            return false;
        }

        // Check horsemanship
        let has_horsemanship = attacker.has_horsemanship() || keywords_granted.iter().any(|k| k == "Horsemanship");
        if has_horsemanship && !blocker.has_horsemanship() {
            return false;
        }

        // Check shadow
        let has_shadow = attacker.has_shadow() || keywords_granted.iter().any(|k| k == "Shadow");
        if has_shadow != blocker.has_shadow() {
            return false;
        }

        // Check fear
        let has_fear = attacker.has_fear() || keywords_granted.iter().any(|k| k == "Fear");
        if has_fear {
            let is_artifact = blocker.is_artifact();
            let is_black = blocker.is_color(crate::core::Color::Black);
            if !is_artifact && !is_black {
                return false;
            }
        }

        // Check intimidate
        let has_intimidate = attacker.has_intimidate() || keywords_granted.iter().any(|k| k == "Intimidate");
        if has_intimidate {
            let is_artifact = blocker.is_artifact();
            let shares_color = attacker.colors.iter().any(|c| blocker.is_color(*c));
            if !is_artifact && !shares_color {
                return false;
            }
        }

        // Check skulk
        let has_skulk = attacker.has_skulk() || keywords_granted.iter().any(|k| k == "Skulk");
        if has_skulk {
            let blocker_power = blocker.current_power();
            let attacker_power = attacker.current_power();
            if blocker_power <= attacker_power {
                return false;
            }
        }

        // Check protection from blocker's colors
        for color in &blocker.colors {
            if attacker.has_protection_from(*color) {
                return false;
            }
        }

        true
    }

    /// Check if a creature would attack if pumped with the given bonuses
    ///
    /// This simulates pumping the creature and checking if it would attack
    /// Reference: ComputerUtilCard.doesSpecifiedCreatureAttackAI()
    pub(crate) fn would_attack_if_pumped(
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

        let pumped_power = i32::from(creature.current_power()) + power_bonus;

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

    /// Determine if we should block an attacker with a specific blocker
    ///
    /// Reference: AiBlockController.java (blocking decision logic)
    ///
    /// Key considerations:
    /// - Can the blocker survive? (toughness >= attacker power)
    /// - Can the blocker kill the attacker? (blocker power >= attacker toughness)
    /// - Favorable trade? (blocker value < attacker value)
    /// - Life in danger? (must block to survive)
    pub(crate) fn should_block(
        &self,
        blocker: &Card,
        attacker: &Card,
        view: &GameStateView,
        attackers: &[CardId],
        current_blocks: &[(CardId, CardId)],
    ) -> bool {
        let blocker_power = i32::from(blocker.current_power());
        let blocker_toughness = i32::from(blocker.current_toughness());
        let attacker_power = i32::from(attacker.current_power());
        let attacker_toughness = i32::from(attacker.current_toughness());

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
    pub(crate) fn total_damage_of_blockers(&self, blockers: &[&Card], attacker: &Card) -> i32 {
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
                total += i32::from(blocker.current_power());
            }
        }

        total
    }

    /// Check if attacker can be killed by a gang of blockers
    ///
    /// Reference: AiBlockController.makeGangBlocks()
    pub(crate) fn can_gang_kill(&self, attacker: &Card, blockers: &[&Card]) -> bool {
        let damage_needed = i32::from(attacker.current_toughness());
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
    pub(crate) fn find_gang_block<'a>(
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
        let attacker_power = i32::from(attacker.current_power());

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
                    let blocker1_dies = i32::from(blocker1.current_toughness()) <= attacker_power;
                    let blocker2_dies = i32::from(blocker2.current_toughness()) <= attacker_power;

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
                            let blocker1_dies = i32::from(blocker1.current_toughness()) <= attacker_power;
                            let blocker2_dies = i32::from(blocker2.current_toughness()) <= attacker_power;
                            let blocker3_dies = i32::from(blocker3.current_toughness()) <= attacker_power;

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
    pub(crate) fn get_safe_blockers<'a>(&self, attacker: &Card, blockers: &[&'a Card]) -> Vec<&'a Card> {
        blockers
            .iter()
            .filter(|b| !self.can_destroy_attacker(attacker, b))
            .copied()
            .collect()
    }

    /// Get blockers that can destroy the attacker
    ///
    /// Reference: AiBlockController.getKillingBlockers() line 114
    pub(crate) fn get_killing_blockers<'a>(&self, attacker: &Card, blockers: &[&'a Card]) -> Vec<&'a Card> {
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
    pub(crate) fn make_trade_blocks<'a>(
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
    pub(crate) fn make_good_blocks<'a>(
        &self,
        view: &GameStateView,
        attackers: &[&'a Card],
        available_blockers: &[&'a Card],
    ) -> Vec<(&'a Card, &'a Card)> {
        let mut assignments = Vec::new();
        let mut remaining_blockers = available_blockers.to_vec();

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
                remaining_blockers.retain(|b| b.id != blocker.id);
            }
        }

        assignments
    }

    /// Check if life is in serious danger (very low life threshold)
    ///
    /// Reference: ComputerUtilCombat.lifeInSeriousDanger() lines 477-508
    pub(crate) fn life_in_serious_danger(
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
    pub(crate) fn assign_blocks_with_gang(
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
    pub(crate) fn assign_blocks_phase1(
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
    pub(crate) fn assign_blocks_phase2(
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
    pub(crate) fn assign_blocks_phase3(
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
    pub(crate) fn reinforce_blockers_against_trample(
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

            let attacker_power = i32::from(attacker.current_power());

            // Calculate current blocking damage absorption (typically 1-3 blockers per attacker)
            let current_blockers: SmallVec<[&Card; 4]> = current_blocks
                .iter()
                .filter_map(|(bid, aid)| if *aid == attacker_id { view.get_card(*bid) } else { None })
                .collect();

            let current_absorption: i32 = current_blockers.iter().map(|b| i32::from(b.current_toughness())).sum();

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
                            let blocker_toughness = i32::from(blocker.current_toughness());
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
    pub(crate) fn reinforce_blockers_to_kill(
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
            let attacker_toughness = i32::from(attacker.current_toughness());

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
            let attacker_toughness = i32::from(attacker.current_toughness());

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

                    let blocker_power = i32::from(blocker.current_power());
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
}
