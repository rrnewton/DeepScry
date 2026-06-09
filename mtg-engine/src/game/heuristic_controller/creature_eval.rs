//! Creature and card valuation heuristics
//!
//! Part of the heuristic AI controller, split out of the former monolithic
//! `heuristic_controller.rs`. See `heuristic_controller/README.md` for the
//! submodule map. This is a pure structural refactor of the Java-Forge AI
//! port — no decision logic changed.

use super::*;

impl HeuristicController {
    /// Evaluate a creature's value using heuristics
    ///
    /// This is a faithful port of Java's CreatureEvaluator.evaluateCreature()
    /// Reference: forge-java/forge-ai/src/main/java/forge/ai/CreatureEvaluator.java:26
    ///
    /// Returns a score representing the creature's overall value.
    /// Higher scores indicate more valuable creatures.
    ///
    /// Uses effective P/T (after anthem effects, equipment, counters) for accurate evaluation.
    ///
    /// `pub` (not `pub(crate)`) because the `creature_evaluation_test` integration
    /// test crate and `game_state_evaluator` both call it as public API.
    pub fn evaluate_creature(&self, view: &GameStateView, card_id: CardId) -> i32 {
        self.evaluate_creature_impl(view, card_id, true, true)
    }

    /// Evaluate a CardDefinition for library search selection
    ///
    /// Uses the card's definition properties (types, mana cost, power/toughness)
    /// to compute a heuristic score. Higher scores = better cards to search for.
    ///
    /// Strategy:
    /// - Creatures: Base score + power/toughness contribution + CMC efficiency bonus
    /// - Lands: Basic lands = 100, non-basic lands get bonus for color production
    /// - Other spells: Score based on CMC (higher = more impactful)
    pub(crate) fn evaluate_card_definition_for_library(
        &self,
        _view: &GameStateView,
        card_def: &crate::loader::CardDefinition,
    ) -> i32 {
        let cmc = i32::from(card_def.mana_cost.cmc());

        // Check card type via cache flags
        if card_def.cache.is_creature {
            // Creatures: value based on power + toughness and CMC efficiency
            // Base score: 80
            // P/T contribution: (power + toughness) * 10 (e.g., 4/4 = +80)
            // CMC efficiency: higher CMC creatures are generally more impactful
            let power = i32::from(card_def.power.unwrap_or(0));
            let toughness = i32::from(card_def.toughness.unwrap_or(0));
            let stats_score = (power + toughness) * 10;
            let cmc_bonus = cmc * 5;
            80 + stats_score + cmc_bonus
        } else if card_def.cache.is_land {
            // Lands: basic lands get 100, non-basic lands get bonus
            let name = card_def.name.as_str();
            let is_basic = matches!(
                name,
                "Plains"
                    | "Island"
                    | "Swamp"
                    | "Mountain"
                    | "Forest"
                    | "Snow-Covered Plains"
                    | "Snow-Covered Island"
                    | "Snow-Covered Swamp"
                    | "Snow-Covered Mountain"
                    | "Snow-Covered Forest"
                    | "Wastes"
            );
            if is_basic {
                100
            } else {
                // Non-basic lands: bonus for color flexibility
                // +5 per color the land can produce
                100 + 5 * (card_def.colors.len() as i32).max(1)
            }
        } else {
            // Instants, sorceries, enchantments, artifacts, planeswalkers
            // Higher CMC spells are generally more impactful (board wipes, finishers)
            // Base score: 50, +30 per CMC
            50 + 30 * cmc
        }
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
    pub(crate) fn evaluate_creature_impl(
        &self,
        view: &GameStateView,
        card_id: CardId,
        consider_pt: bool,
        consider_cmc: bool,
    ) -> i32 {
        let mut value = 80;

        // Get the card from the view - MUST be visible or it's a missing reveal bug
        let Some(card) = view.get_card(card_id) else {
            panic!(
                "FATAL: evaluate_creature called on invisible card {:?}. \
                This indicates a missing CardRevealed message from the server. \
                The network architecture requires all cards be revealed before \
                they can be evaluated for decision-making.",
                card_id
            );
        };

        // Tokens are worth less than actual cards
        // Java: if (!c.isToken()) { value += addValue(20, "non-token"); }
        if !card.is_token {
            value += 20;
        }

        // Use effective P/T after all continuous effects (anthem, equipment, counters)
        // CRITICAL: get_effective_power should ALWAYS succeed for battlefield creatures.
        // If it fails (returns None), that indicates a bug - an unrevealed card somewhere
        // in the continuous effects calculation chain. The fallback hides the bug!
        let effective_power_opt = view.get_effective_power(card_id);
        let effective_toughness_opt = view.get_effective_toughness(card_id);

        // Check if we're being forced to use the fallback
        if effective_power_opt.is_none() || effective_toughness_opt.is_none() {
            eprintln!(
                "WARNING: get_effective_power/toughness returned None for battlefield creature {:?} '{}'. \
                 This indicates a bug in the continuous effects chain (likely an unrevealed card). \
                 Falling back to base P/T which may cause divergence.",
                card_id, card.name
            );
        }

        let power = effective_power_opt.unwrap_or_else(|| i32::from(card.current_power()));
        let toughness = effective_toughness_opt.unwrap_or_else(|| i32::from(card.current_toughness()));

        // Stats scoring
        if consider_pt {
            // Java: value += addValue(power * 15, "power");
            value += power * 15;
            // Java: value += addValue(toughness * 10, "toughness: " + toughness);
            value += toughness * 10;
        }

        if consider_cmc {
            // Java: value += addValue(c.getCMC() * 5, "cmc");
            let cmc = i32::from(card.mana_cost.cmc());
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

        // Landwalk: Unblockable if defending player controls a land of the appropriate type
        // Reference: CR 702.14 - Landwalk makes creature unblockable if opponent has that land type
        // Java: checks Keyword.LANDWALK and validates against opponent's lands
        // Bonus is power * 10 (same as flying/shadow) when opponent has the relevant land
        if card.has_keyword(Keyword::Landwalk) {
            // Check each landwalk type the creature has
            for keyword_args in card.keywords.iter_args() {
                if let KeywordArgs::Landwalk { land_type } = keyword_args {
                    // Check if any opponent controls a land with this subtype
                    let opponent_has_land = view.battlefield().iter().filter_map(|&id| view.get_card(id)).any(|c| {
                        c.controller != card.controller
                            && c.is_land()
                            && c.subtypes
                                .iter()
                                .any(|st| st.as_str().eq_ignore_ascii_case(land_type.as_str()))
                    });

                    if opponent_has_land {
                        // Landwalk is as good as flying when active (unblockable)
                        value += power * 10;
                        break; // Only count once even if multiple landwalks match
                    }
                }
            }
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

        // Ward: Makes creature harder to target (opponent must pay extra)
        // Java: if (c.hasKeyword(Keyword.WARD)) { value += addValue(10, "ward"); }
        if card.has_keyword(Keyword::Ward) {
            value += 10;
        }

        // Protection (generic check - any protection is valuable)
        // Java: if (c.hasKeyword(Keyword.PROTECTION)) { value += addValue(20, "protection"); }
        // Note: We check for specific protection colors and the generic Protection keyword
        if card.has_keyword(Keyword::Protection)
            || card.has_keyword(Keyword::ProtectionFromRed)
            || card.has_keyword(Keyword::ProtectionFromBlue)
            || card.has_keyword(Keyword::ProtectionFromBlack)
            || card.has_keyword(Keyword::ProtectionFromWhite)
            || card.has_keyword(Keyword::ProtectionFromGreen)
        {
            value += 20;
        }

        // Combat enhancement keywords (magnitude-based bonuses from Java CreatureEvaluator)
        // Reference: forge-java/forge-ai/src/main/java/forge/ai/CreatureEvaluator.java:115-131

        // Flanking: +15 bonus per count (old combat keyword from Mirage)
        // Java: value += addValue(c.getAmountOfKeyword(Keyword.FLANKING) * 15, "flanking");
        if card.has_keyword(Keyword::Flanking) {
            value += 15; // TODO: multiply by count when we track stacking
        }

        // Exalted: +15 bonus per count (Alara mechanic)
        // Java: value += addValue(c.getAmountOfKeyword(Keyword.EXALTED) * 15, "exalted");
        if card.has_keyword(Keyword::Exalted) {
            value += 15; // TODO: multiply by count when we track stacking
        }

        // Prowess: +5 bonus per count (Khans mechanic)
        // Java: value += addValue(c.getAmountOfKeyword(Keyword.PROWESS) * 5, "prowess");
        if card.has_keyword(Keyword::Prowess) {
            value += 5; // TODO: multiply by count when we track stacking
        }

        // Melee: +18 bonus per count (Conspiracy mechanic)
        // Java: value += addValue(c.getAmountOfKeyword(Keyword.MELEE) * 18, "melee");
        if card.has_keyword(Keyword::Melee) {
            value += 18; // TODO: multiply by count when we track stacking
        }

        // Outlast: +10 bonus (can add +1/+1 counters)
        // Java: if (c.hasKeyword(Keyword.OUTLAST)) { value += addValue(10, "outlast"); }
        if card.has_keyword(Keyword::Outlast) {
            value += 10;
        }

        // Magnitude-based threat keywords (per-level bonuses)

        // Annihilator: Major threat, +50 per level (Eldrazi)
        // Java: value += addValue(c.getKeywordMagnitude(Keyword.ANNIHILATOR) * 50, "annihilator");
        if card.has_keyword(Keyword::Annihilator) {
            // For now, assume base level 1 until we can read magnitude
            value += 50; // TODO: multiply by magnitude
        }

        // Afflict: +5 per level (damage when blocked)
        // Java: value += addValue(c.getKeywordMagnitude(Keyword.AFFLICT) * 5, "afflict");
        if card.has_keyword(Keyword::Afflict) {
            value += 5; // TODO: multiply by magnitude
        }

        // Toxic: +5 per level (poison counters)
        // Java: value += addValue(c.getKeywordMagnitude(Keyword.TOXIC) * 5, "toxic");
        if card.has_keyword(Keyword::Toxic) {
            value += 5; // TODO: multiply by magnitude
        }

        // Rampage: Direct magnitude bonus
        // Java: value += addValue(c.getKeywordMagnitude(Keyword.RAMPAGE), "rampage");
        if card.has_keyword(Keyword::Rampage) {
            value += 5; // TODO: use actual magnitude
        }

        // Bushido: +16 per level (old Kamigawa combat bonus)
        // Java: value += addValue(c.getKeywordMagnitude(Keyword.BUSHIDO) * 16, "bushido");
        if card.has_keyword(Keyword::Bushido) {
            value += 16; // TODO: multiply by magnitude
        }

        // Absorb: +11 per level (damage prevention)
        // Java: value += addValue(c.getKeywordMagnitude(Keyword.ABSORB) * 11, "absorb");
        if card.has_keyword(Keyword::Absorb) {
            value += 11; // TODO: multiply by magnitude
        }

        // Resurrection keywords (creature comes back)

        // Undying: Creature returns with +1/+1 counter (very valuable)
        // Java: ComputerUtilCard.java:1872-1883 (hasActiveUndyingOrPersist)
        // Only valuable if creature has NO +1/+1 counters (otherwise it won't return)
        if card.has_keyword(Keyword::Undying) {
            let has_p1p1 = card.get_counter(crate::core::CounterType::P1P1) > 0;
            if !has_p1p1 {
                value += 25; // Will return from death
            }
            // No bonus if already has counters - Undying won't trigger
        }

        // Persist: Creature returns with -1/-1 counter (valuable but weaker return)
        // Only valuable if creature has NO -1/-1 counters (otherwise it won't return)
        if card.has_keyword(Keyword::Persist) {
            let has_m1m1 = card.get_counter(crate::core::CounterType::M1M1) > 0;
            if !has_m1m1 {
                value += 20; // Will return from death
            }
            // No bonus if already has counters - Persist won't trigger
        }

        // Negative keywords
        // Java: if (c.hasKeyword(Keyword.DEFENDER)) { value -= power * 9 + 40; }
        if card.has_defender() {
            value -= power * 9 + 40;
        }

        // Combat restriction penalties
        // Reference: CreatureEvaluator.java:177-197
        // Java: if (c.hasKeyword("CARDNAME can't attack or block.")) { value = addValue(50 + (c.getCMC() * 5), "useless"); }
        if card.has_keyword(Keyword::CantAttackOrBlock) {
            // Reset everything - creature that can't attack or block is nearly useless
            let cmc = i32::from(card.mana_cost.cmc());
            value = 50 + (cmc * 5);
        } else {
            // "Can't attack" - Java: if (c.hasKeyword("CARDNAME can't attack.")) { value -= power * 9 + 40; }
            // This is already covered by Defender above, but we also check explicit CantAttack
            // Java treats Defender the same as "can't attack" with same penalty
            if card.has_keyword(Keyword::CantAttack) {
                value -= power * 9 + 40;
            }

            // "Can't block" - Java: else if (c.hasKeyword("CARDNAME can't block.")) { value -= subValue(10, "cant-block"); }
            if card.has_keyword(Keyword::CantBlock) {
                value -= 10;
            }

            // Goaded - Java: else if (c.isGoaded()) { value -= subValue(5, "goaded"); }
            if card.has_keyword(Keyword::Goaded) {
                value -= 5;
            }

            // Must attack - Java: List<GameEntity> mAEnt = StaticAbilityMustAttack.entitiesMustAttack(c);
            // if (mAEnt.contains(c)) { value -= subValue(10, "must-attack"); }
            if card.has_keyword(Keyword::MustAttack) {
                value -= 10;
            }
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
            let fade_counters = i32::from(card.get_counter(crate::core::CounterType::Fade));
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
            let time_counters = i32::from(card.get_counter(crate::core::CounterType::Time));
            if time_counters == 0 {
                value -= 50; // About to die
            } else if time_counters <= 2 {
                value -= 30; // Low counters
            } else {
                value -= 15; // Has time left
            }
        }

        // Mana abilities add value
        // Java: if (!c.getManaAbilities().isEmpty()) { value += addValue(10, "manadork"); }
        // Check activated abilities for mana production (Llanowar Elves, Birds of Paradise, etc.)
        let has_mana_ability = card.activated_abilities.iter().any(|ab| ab.is_mana_ability);
        if has_mana_ability || card.is_land() {
            value += 10;
        }

        // Activated ability bonuses (non-mana)
        // Creatures with useful activated abilities are more valuable
        // Reference: Java CreatureEvaluator doesn't explicitly score these,
        // but the AI ability evaluators (DestroyAi, PumpAi, DamageDealAi) all
        // consider source creature value when deciding to activate.
        for ability in &card.activated_abilities {
            if ability.is_mana_ability {
                continue;
            }
            let ability_type = self.classify_activated_ability(ability);
            match ability_type {
                ActivatedAbilityType::Ping { damage } => {
                    // Repeatable damage is very valuable (Prodigal Sorcerer)
                    value += 10 + damage * 5;
                }
                ActivatedAbilityType::Pump { power, toughness } => {
                    // Firebreathing/pump abilities (Shivan Dragon, Granite Gargoyle)
                    value += 5 + power * 3 + toughness * 2;
                }
                ActivatedAbilityType::Destroy => {
                    // Destroy abilities are extremely valuable (Royal Assassin)
                    value += 40;
                }
                ActivatedAbilityType::Regenerate => {
                    // Regeneration makes creatures harder to kill (Drudge Skeletons, Sedge Troll)
                    // Roughly equivalent to a toughness bonus
                    value += 20;
                }
                ActivatedAbilityType::PreventDamage => {
                    // Damage prevention is defensive value, similar to regenerate
                    value += 15;
                }
                ActivatedAbilityType::Debuff => {
                    // Debuff abilities (lose Defender) add value since they unlock attacking
                    value += 15;
                }
                ActivatedAbilityType::TapTarget => {
                    // Tap-target abilities (Icy Manipulator) provide repeatable control
                    value += 25;
                }
                ActivatedAbilityType::ZoneReturn => {
                    // Zone-return from graveyard: moderate value — allows recursion
                    value += 15;
                }
                ActivatedAbilityType::Equip | ActivatedAbilityType::DrawCard => {
                    // These appear on Equipment / token artifacts, not creatures;
                    // they do not add to a creature's combat/eval value here.
                    // (Their activation is handled in should_activate_ability.)
                }
                ActivatedAbilityType::Other => {}
            }
        }

        // Triggered ability bonuses
        // Creatures with beneficial triggers are more valuable
        for trigger in &card.triggers {
            match trigger.event {
                crate::core::TriggerEvent::DealsCombatDamage => {
                    // Combat damage triggers (Sengir Vampire, Hypnotic Specter)
                    // Valuable because they reward successful attacks
                    value += 15;
                }
                crate::core::TriggerEvent::EntersBattlefield => {
                    // ETB effects (value depends on the effect)
                    value += 10;
                }
                crate::core::TriggerEvent::LeavesBattlefield => {
                    // Death/LTB triggers can be valuable or just cleanup
                    value += 5;
                }
                crate::core::TriggerEvent::Attacks => {
                    // Attack triggers (value scales with the effect)
                    value += 10;
                }
                crate::core::TriggerEvent::Blocks => {
                    // Block triggers
                    value += 5;
                }
                crate::core::TriggerEvent::BeginningOfUpkeep => {
                    // Upkeep triggers are often COSTS (sacrifice unless pay)
                    // Check if this is a negative trigger (sacrifice, damage to self)
                    let is_negative = trigger.effects.iter().any(|e| {
                        matches!(
                            e,
                            crate::core::Effect::DealDamage { .. }
                                | crate::core::Effect::DealDamageXPaid { .. }
                                | crate::core::Effect::DiscardCards { .. }
                                | crate::core::Effect::DiscardCardsXPaid { .. }
                                | crate::core::Effect::Mill { .. }
                        )
                    });
                    if is_negative {
                        value -= 15;
                    }
                    // Positive upkeep triggers are rarer, handled by keyword checks above
                }
                crate::core::TriggerEvent::DamagedCreatureDies => {
                    // "Whenever a creature dealt damage by this card this turn dies, ..."
                    // (Sengir Vampire, Baron Sengir, Abattoir Ghoul, Blood Cultist).
                    // High value: scales with successful combat — directly rewards
                    // attacking and trading favorably.
                    value += 15;
                }
                crate::core::TriggerEvent::BeginningOfDraw => {
                    // Draw-step triggers are usually card advantage (Grafted
                    // Skullcap, Sylvan Library, Yawgmoth's Bargain): "draw an
                    // additional card." Reward the extra draw.
                    value += 10;
                }
                crate::core::TriggerEvent::BeginningOfEndStep
                | crate::core::TriggerEvent::BeginningOfCombat
                | crate::core::TriggerEvent::SpellCast
                | crate::core::TriggerEvent::Sacrificed
                | crate::core::TriggerEvent::CardDrawn
                | crate::core::TriggerEvent::Taps
                | crate::core::TriggerEvent::AttackersDeclared
                | crate::core::TriggerEvent::EquippedCreatureDies
                | crate::core::TriggerEvent::ClassLevelGained { .. }
                | crate::core::TriggerEvent::CardDiscarded => {
                    // Other triggers get a small bonus
                    value += 5;
                }
            }
        }

        // Equipment/Aura attachment bonus
        // Creatures with attachments are more valuable because losing them
        // causes the equipment/aura to fall off (losing investment).
        // Reference: Java ComputerUtilCard uses enchantment count in evaluation
        let attachment_count = view
            .battlefield()
            .iter()
            .filter(|&&other_id| {
                view.get_card(other_id).is_some_and(|other| {
                    other.attached_to == Some(card_id) && (other.is_equipment() || other.is_aura())
                })
            })
            .count() as i32;
        if attachment_count > 0 {
            // Each attachment adds value (equipment/aura investment)
            value += attachment_count * 15;
        }

        // +1/+1 counter bonus (beyond the P/T already counted)
        // Creatures with counters represent accumulated investment
        let p1p1_counters = i32::from(card.get_counter(crate::core::CounterType::P1P1));
        if p1p1_counters > 0 {
            value += p1p1_counters * 5; // Each counter adds extra value beyond raw stats
        }

        value
    }

    /// Get the best creature from a list based on evaluation score
    ///
    /// Reference: ComputerUtilCard.sortByEvaluateCreature() and getBestCreatureAI()
    pub(crate) fn get_best_creature(&self, view: &GameStateView, creature_ids: &[CardId]) -> Option<CardId> {
        creature_ids
            .iter()
            .max_by_key(|&&card_id| self.evaluate_creature(view, card_id))
            .copied()
    }

    /// Get the worst creature from a list based on evaluation score
    #[allow(dead_code)] // Will be used for discard decisions
    pub(crate) fn get_worst_creature(&self, view: &GameStateView, creature_ids: &[CardId]) -> Option<CardId> {
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
    pub(crate) fn evaluate_creature_for_casting(
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
        let cmc = u32::from(card.mana_cost.cmc());

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
}
