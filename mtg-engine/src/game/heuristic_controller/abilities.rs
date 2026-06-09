//! Activated-ability classification, scoring, and activation timing
//!
//! Part of the heuristic AI controller, split out of the former monolithic
//! `heuristic_controller.rs`. See `heuristic_controller/README.md` for the
//! submodule map. This is a pure structural refactor of the Java-Forge AI
//! port — no decision logic changed.

use super::*;

impl HeuristicController {
    /// Evaluate whether to activate an activated ability now
    ///
    /// Reference: Various ability AI classes in forge-ai/src/main/java/forge/ai/ability/
    ///
    /// Implements evaluation for:
    /// 1. Ping abilities (Prodigal Sorcerer) - DamageDealAi.java:196-200, 682-703
    /// 2. Pump abilities (Shivan Dragon) - PumpAi.java
    pub(crate) fn should_activate_ability(&self, source: &Card, view: &GameStateView) -> bool {
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
                ActivatedAbilityType::Destroy => {
                    // Destroy abilities (Royal Assassin, etc.)
                    // Reference: DestroyAi.java in forge-ai
                    //
                    // Royal Assassin specifically targets "tapped creatures", so this ability
                    // is most valuable during/after opponent's combat when attackers are tapped.
                    //
                    // Strategy:
                    // 1. Only use when we have a valid target (handled by game loop)
                    // 2. Prioritize high-value targets
                    // 3. Use during opponent's declare attackers or after blockers declared

                    // Check timing - best used after opponent declares attackers
                    let current_step = view.current_step();
                    let is_combat = matches!(
                        current_step,
                        crate::game::Step::DeclareAttackers
                            | crate::game::Step::DeclareBlockers
                            | crate::game::Step::CombatDamage
                    );
                    let is_end_phase = current_step == crate::game::Step::End;
                    let is_main2 = current_step == crate::game::Step::Main2;

                    // During combat or end phase - good time to destroy attackers
                    // Reference: DestroyAi checks for phase restrictions
                    if is_combat || is_end_phase || is_main2 {
                        // Check if there are valuable tapped creatures to destroy
                        if self.has_valuable_destroy_target(view) {
                            return true;
                        }
                    }
                }
                ActivatedAbilityType::Regenerate => {
                    // Regeneration: activate when creature is in danger
                    // Best used proactively before combat damage or when
                    // an opponent has destroy effects.
                    // For now, always activate if we have mana — it's never bad
                    // to have a regeneration shield up.
                    let current_step = view.current_step();
                    let is_combat = matches!(
                        current_step,
                        crate::game::Step::DeclareAttackers
                            | crate::game::Step::DeclareBlockers
                            | crate::game::Step::CombatDamage
                    );
                    // Activate during combat or if creature doesn't have a shield already
                    if is_combat {
                        return true;
                    }
                }
                ActivatedAbilityType::PreventDamage => {
                    // Damage prevention: activate during combat when damage is imminent
                    // Similar to Regenerate - proactively shield before combat damage
                    let current_step = view.current_step();
                    let is_combat = matches!(
                        current_step,
                        crate::game::Step::DeclareAttackers
                            | crate::game::Step::DeclareBlockers
                            | crate::game::Step::CombatDamage
                    );
                    if is_combat {
                        return true;
                    }
                }
                ActivatedAbilityType::Debuff => {
                    // Debuff abilities: primarily "lose Defender" to enable attacking
                    // Activate before combat (Main1) so the creature can attack
                    // Reference: DebuffEffect.java - typically self-targeting
                    let current_step = view.current_step();
                    if current_step == crate::game::Step::Main1 {
                        // Check if this removes Defender from self — enables attacking
                        let removes_defender = ability.effects.iter().any(|e| {
                            if let crate::core::Effect::DebuffCreature { keywords_removed, .. } = e {
                                keywords_removed.contains(&crate::core::Keyword::Defender)
                            } else {
                                false
                            }
                        });
                        if removes_defender && source.keywords.contains(crate::core::Keyword::Defender) {
                            return true;
                        }
                        // For other keyword removals from self, also activate in Main1
                        // (e.g., Xathrid Slyblade loses Hexproof to gain FirstStrike+Deathtouch)
                        return true;
                    }
                }
                ActivatedAbilityType::TapTarget => {
                    // Tap-target abilities (Icy Manipulator, etc.)
                    // Reference: TapAi.java - best used before combat to tap blockers,
                    // or during opponent's turn to tap attackers/mana
                    let current_step = view.current_step();

                    // Before our combat: tap opponent's potential blockers
                    if current_step == crate::game::Step::BeginCombat || current_step == crate::game::Step::Main1 {
                        // Check for untapped opponent creatures
                        let has_target = view.battlefield().iter().any(|&card_id| {
                            view.get_card(card_id)
                                .is_some_and(|c| c.is_creature() && c.controller != self.player_id && !c.tapped)
                        });
                        if has_target {
                            return true;
                        }
                    }

                    // End of opponent's turn: tap their best creature
                    if current_step == crate::game::Step::End {
                        let has_target = view.battlefield().iter().any(|&card_id| {
                            view.get_card(card_id)
                                .is_some_and(|c| c.is_creature() && c.controller != self.player_id && !c.tapped)
                        });
                        if has_target {
                            return true;
                        }
                    }
                }
                ActivatedAbilityType::ZoneReturn => {
                    // Zone-return from graveyard (e.g. Earthquake Dragon).
                    // Activate during our main phase when the stack is empty —
                    // there's no reason to delay returning a powerful threat.
                    // CR 602.1: any player can activate at instant speed unless
                    // the ability says otherwise; for graveyard returns with no
                    // timing restriction, main phase is fine and avoids
                    // spurious activations during opponent turns.
                    let current_step = view.current_step();
                    let is_main = matches!(current_step, crate::game::Step::Main1 | crate::game::Step::Main2);
                    if is_main && self.is_stack_empty(view) {
                        return true;
                    }
                }
                ActivatedAbilityType::Equip => {
                    // Equip is sorcery-speed (CR 301.5c): only during our own
                    // main phase with an empty stack. Attach to a creature we
                    // control. To avoid equip-thrashing (re-attaching every turn
                    // and wasting mana), only equip when this Equipment is
                    // currently UNATTACHED. Activate in Main1 so the equipped
                    // creature benefits before combat. The engine only offers
                    // the ability when a legal target creature exists.
                    // Reference: AttachAi.java in forge-ai.
                    let current_step = view.current_step();
                    let is_main = matches!(current_step, crate::game::Step::Main1 | crate::game::Step::Main2);
                    if is_main && self.is_stack_empty(view) && self.has_equip_target(source, view) {
                        return true;
                    }
                }
                ActivatedAbilityType::DrawCard => {
                    // Crack a Clue (sacrifice-to-draw) and similar card-draw
                    // abilities. Card advantage is almost always good, so do it
                    // at sorcery speed in our Main2 with the stack empty — Main2
                    // so we keep mana available for our actual spells in Main1
                    // first, and only spend leftover mana drawing. The engine
                    // only offers the ability when its cost (incl. the {2}) is
                    // payable, so reaching here means we can afford it.
                    let current_step = view.current_step();
                    if current_step == crate::game::Step::Main2 && self.is_stack_empty(view) {
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

    /// Whether this Equipment should be equipped now: it is currently
    /// UNATTACHED (so we don't equip-thrash) and we control at least one
    /// creature to attach it to. The actual best-target pick is made in
    /// `choose_targets` (default branch → our best creature). (mtg-721)
    pub(crate) fn has_equip_target(&self, source: &Card, view: &GameStateView) -> bool {
        if source.is_attached() {
            return false;
        }
        view.battlefield().iter().any(|&id| {
            view.get_card(id)
                .is_some_and(|c| c.controller == self.player_id && c.is_creature())
        })
    }

    /// Classify the type of activated ability based on its effects
    pub(crate) fn classify_activated_ability(&self, ability: &crate::core::ActivatedAbility) -> ActivatedAbilityType {
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

        // Check for destroy effects (Royal Assassin, Atog, etc.)
        // Reference: DestroyAi.java in forge-ai
        for effect in &ability.effects {
            if matches!(effect, crate::core::Effect::DestroyPermanent { .. }) {
                return ActivatedAbilityType::Destroy;
            }
        }

        // Check for regeneration effects (Drudge Skeletons, Sedge Troll, etc.)
        for effect in &ability.effects {
            if matches!(effect, crate::core::Effect::Regenerate { .. }) {
                return ActivatedAbilityType::Regenerate;
            }
        }

        // Check for damage prevention effects (Militant Monk, Master Healer,
        // and the source-filtered Circles of Protection).
        for effect in &ability.effects {
            if matches!(
                effect,
                crate::core::Effect::PreventDamage { .. } | crate::core::Effect::PreventDamageFromSource { .. }
            ) {
                return ActivatedAbilityType::PreventDamage;
            }
        }

        // Check for debuff effects (Grozoth, Gargoyle Sentinel - lose Defender, etc.)
        for effect in &ability.effects {
            if matches!(effect, crate::core::Effect::DebuffCreature { .. }) {
                return ActivatedAbilityType::Debuff;
            }
        }

        // Check for tap-target effects (Icy Manipulator, etc.)
        // Reference: TapAi.java in forge-ai
        for effect in &ability.effects {
            if matches!(effect, crate::core::Effect::TapPermanent { .. }) {
                return ActivatedAbilityType::TapTarget;
            }
        }

        // Check for zone-return self-move (graveyard→hand, etc.)
        // E.g. Earthquake Dragon's ActivationZone$ Graveyard ability.
        for effect in &ability.effects {
            if matches!(effect, crate::core::Effect::MoveSelfBetweenZones { .. }) {
                return ActivatedAbilityType::ZoneReturn;
            }
        }

        // Check for equip (attach this Equipment to a creature you control).
        // E.g. Trusty Boomerang's `K:Equip:1`. (mtg-721)
        for effect in &ability.effects {
            if matches!(effect, crate::core::Effect::AttachEquipment { .. }) {
                return ActivatedAbilityType::Equip;
            }
        }

        // Check for card-draw abilities (crack a Clue token, etc.). (mtg-721)
        for effect in &ability.effects {
            if matches!(
                effect,
                crate::core::Effect::DrawCards { .. } | crate::core::Effect::DrawCardsXPaid { .. }
            ) {
                return ActivatedAbilityType::DrawCard;
            }
        }

        ActivatedAbilityType::Other
    }

    /// Check if the stack is empty
    /// Reference: DamageDealAi.java:196 (stack.isEmpty())
    pub(crate) fn is_stack_empty(&self, view: &GameStateView) -> bool {
        view.is_stack_empty()
    }

    /// Check if there's a valuable target we can ping
    /// Reference: DamageDealAi.java:697 (canTarget(enemy))
    pub(crate) fn has_valuable_ping_target(&self, view: &GameStateView, damage: i32) -> bool {
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
    pub(crate) fn can_kill_valuable_creature(&self, view: &GameStateView, damage: i32) -> bool {
        // For now, use same logic as has_valuable_ping_target
        // In Java Forge, this checks for "best opponent creature we can kill"
        self.has_valuable_ping_target(view, damage)
    }

    /// Check if there's a valuable tapped creature we can destroy
    /// Reference: DestroyAi.java - targets "best creature" from valid targets
    ///
    /// For Royal Assassin specifically, targets must be tapped creatures.
    /// We evaluate based on creature value - prefer destroying high-power/value targets.
    pub(crate) fn has_valuable_destroy_target(&self, view: &GameStateView) -> bool {
        // Look for opponent's tapped creatures
        // Royal Assassin can only target tapped creatures per card text
        let mut best_value = 0i32;

        for opponent_id in view.opponents() {
            for &card_id in view.battlefield() {
                if let Some(card) = view.get_card(card_id) {
                    if card.controller == opponent_id && card.is_creature() && card.tapped {
                        // Check if creature has indestructible (can't destroy it)
                        if card.has_keyword(Keyword::Indestructible) {
                            continue;
                        }

                        // Evaluate this creature's value
                        // Use power + toughness as a simple heuristic
                        let power = i32::from(card.current_power());
                        let toughness = i32::from(card.current_toughness());
                        let value = power * 10 + toughness * 5;

                        // Add bonus for dangerous keywords
                        if card.has_keyword(Keyword::Deathtouch) {
                            best_value = best_value.max(value + 50);
                        } else if card.has_keyword(Keyword::Lifelink) {
                            best_value = best_value.max(value + 30);
                        } else if card.has_keyword(Keyword::FirstStrike) || card.has_keyword(Keyword::DoubleStrike) {
                            best_value = best_value.max(value + 20);
                        } else {
                            best_value = best_value.max(value);
                        }
                    }
                }
            }
        }

        // Only activate if there's a target worth destroying
        // Threshold: at least a 2/2 creature (value 30)
        best_value >= 30
    }

    /// Check if pumping this creature would enable better attacks
    /// Reference: PumpAi.java lines 88-105, 481-490
    pub(crate) fn would_pump_enable_attack(
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
    pub(crate) fn should_activate_pump_during_combat(
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
            .unwrap_or_else(|| i32::from(source.current_power()));
        let source_toughness = view
            .get_effective_toughness(source.id)
            .unwrap_or_else(|| i32::from(source.current_toughness()));
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
                                .unwrap_or_else(|| i32::from(atk_card.current_power()));
                            total_damage += atk_power;
                        }
                    } else if let Some(atk_card) = view.get_card(attacker_id) {
                        // Blocked attacker - count trample damage only
                        if atk_card.has_trample() {
                            let atk_power = view
                                .get_effective_power(attacker_id)
                                .unwrap_or_else(|| i32::from(atk_card.current_power()));
                            let blocker_toughness: i32 = combat
                                .get_blockers(attacker_id)
                                .iter()
                                .filter_map(|&b| view.get_card(b))
                                .map(|b| {
                                    view.get_effective_toughness(b.id)
                                        .unwrap_or_else(|| i32::from(b.current_toughness()))
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
                    .map(|b| {
                        view.get_effective_power(b.id)
                            .unwrap_or_else(|| i32::from(b.current_power()))
                    })
                    .sum();

                let total_blocker_toughness: i32 = blockers
                    .iter()
                    .filter_map(|&b| view.get_card(b))
                    .map(|b| {
                        view.get_effective_toughness(b.id)
                            .unwrap_or_else(|| i32::from(b.current_toughness()))
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
                            .unwrap_or_else(|| i32::from(blocker.current_toughness()));

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
                .map(|a| {
                    view.get_effective_power(a.id)
                        .unwrap_or_else(|| i32::from(a.current_power()))
                })
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
                        .unwrap_or_else(|| i32::from(attacker.current_toughness()));

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
}
