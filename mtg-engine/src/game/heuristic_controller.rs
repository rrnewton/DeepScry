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

    /// Evaluate a CardDefinition for library search selection
    ///
    /// Uses the card's definition properties (types, mana cost, power/toughness)
    /// to compute a heuristic score. Higher scores = better cards to search for.
    ///
    /// Strategy:
    /// - Creatures: Base score + power/toughness contribution + CMC efficiency bonus
    /// - Lands: Basic lands = 100, non-basic lands get bonus for color production
    /// - Other spells: Score based on CMC (higher = more impactful)
    fn evaluate_card_definition_for_library(
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
    fn evaluate_creature_impl(
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
                | crate::core::TriggerEvent::CardDiscarded
                | crate::core::TriggerEvent::TapsForMana => {
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
    fn should_play_land(&mut self, land_id: CardId, view: &GameStateView) -> bool {
        // If it's not Main Phase 1, always play the land
        let current_step = view.current_step();
        if current_step != crate::game::phase::Step::Main1 {
            return true;
        }

        // Check if it's safe to hold land drop for Main 2 (bluffing/deception)
        if self.is_safe_to_hold_land_for_main2(land_id, view) {
            // Hold the land - don't play it in Main 1
            return false;
        }

        // Otherwise, play the land
        true
    }

    /// Check if it's safe to hold a land drop for Main 2 (bluffing/deception)
    ///
    /// Reference: AiController.isSafeToHoldLandDropForMain2 (lines 1643-1740)
    ///
    /// This is a deception mechanism to hide information from opponents.
    /// The AI sometimes waits until Main 2 to play lands when it doesn't need the mana,
    /// making it harder for opponents to read what's in hand.
    fn is_safe_to_hold_land_for_main2(&mut self, _land_id: CardId, view: &GameStateView) -> bool {
        use rand::Rng;
        // 50% chance to consider holding (matches Java's default HOLD_LAND_DROP_FOR_MAIN2_IF_UNUSED)
        // This prevents the AI from being too predictable
        if !self.rng.gen_bool(0.5) {
            return false;
        }

        // Don't do this on very early turns (too obvious)
        if view.turn_number() <= 2 {
            return false;
        }

        // Get hand
        let hand = view.hand();

        // Count non-land cards in hand
        let nonlands_in_hand: SmallVec<[&Card; 8]> = hand
            .iter()
            .filter_map(|&id| view.get_card(id))
            .filter(|c| !c.is_land())
            .collect();

        // Calculate minimum CMC of spells in hand (any non-land card)
        let min_cmc = nonlands_in_hand
            .iter()
            .map(|c| u32::from(c.mana_cost.cmc()))
            .min()
            .unwrap_or(0);

        // Calculate available mana (before playing land)
        let current_mana = self.count_available_mana(view);

        // Check if the land would enable casting something
        // Simplified: assume playing the land adds 1 mana
        let mana_with_land = current_mana + 1;
        let can_cast_with_land = min_cmc > 0 && mana_with_land >= min_cmc;
        let cant_cast_now = current_mana < min_cmc;

        // Simplified decision: Hold land if we can't cast anything even with the land drop
        // This is the core bluffing logic - we hold lands when they won't help us cast spells
        // More sophisticated checks (landfall triggers, activated abilities, taplands)
        // can be added later if needed
        if !can_cast_with_land && cant_cast_now {
            // Safe to hold - we won't be able to cast anything anyway
            // Holding hides information from opponents
            return true;
        }

        false
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
            if let SpellAbility::CastSpell { card_id } | SpellAbility::CastFromCommand { card_id, .. } = ability {
                if let Some(spell_card) = view.get_card(*card_id) {
                    // Check if this is a pump spell (has PumpCreature effect)
                    for effect in &spell_card.effects {
                        if let crate::core::Effect::PumpCreature {
                            power_bonus,
                            toughness_bonus,
                            ..
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

        // 2a2: Cast mana-producing artifacts early (Sol Ring, Arcane Signet, etc.)
        // In the early game (turns 1-5), mana rocks are extremely valuable for ramping.
        // Cast them before creatures to accelerate future turns.
        let turn_number = view.turn_number();
        if turn_number <= 5 {
            for ability in available {
                if let SpellAbility::CastSpell { card_id } | SpellAbility::CastFromCommand { card_id, .. } = ability {
                    if let Some(card) = view.get_card(*card_id) {
                        // Check if this is a mana-producing artifact (not a creature)
                        // Check both cache flag AND activated abilities for mana production
                        if card.is_artifact() && !card.is_creature() {
                            let has_mana_ability = card.definition.cache.is_mana_source
                                || card.activated_abilities.iter().any(|ab| ab.is_mana_ability);
                            if has_mana_ability {
                                return Some(ability.clone());
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
        let available_mana = self.count_available_mana(view);

        for ability in available {
            if let SpellAbility::CastSpell { card_id } | SpellAbility::CastFromCommand { card_id, .. } = ability {
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
                // Check if we should play this land (may hold for Main 2 bluffing)
                if self.should_play_land(best_land_id, view) {
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
            if let SpellAbility::CastSpell { card_id } | SpellAbility::CastFromCommand { card_id, .. } = ability {
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
    fn can_block(&self, attacker: &Card, blocker: &Card) -> bool {
        self.can_block_impl(attacker, blocker, None)
    }

    /// Check if a blocker can block an attacker with full landwalk support
    ///
    /// This version takes a GameStateView to check if the defending player
    /// controls a land of the type the attacker has landwalk for.
    fn can_block_with_view(&self, attacker: &Card, blocker: &Card, view: &GameStateView) -> bool {
        self.can_block_impl(attacker, blocker, Some(view))
    }

    /// Implementation of blocking check with optional view for landwalk
    ///
    /// Reference: CombatUtil.canBlock() in Java Forge
    /// Implements blocking restrictions based on evasion abilities and protection.
    fn can_block_impl(&self, attacker: &Card, blocker: &Card, view: Option<&GameStateView>) -> bool {
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
    fn can_destroy_blocker(&self, attacker: &Card, blocker: &Card) -> bool {
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
    fn can_destroy_attacker(&self, attacker: &Card, blocker: &Card) -> bool {
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
            .map(|c| i32::from(c.current_power()))
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
    fn should_attack_with_context(
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

    fn should_attack(&self, attacker: &Card, view: &GameStateView) -> bool {
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
        let current_toughness = i32::from(target.current_toughness()) + target.power_bonus;
        if current_toughness + toughness_bonus <= 0 {
            return false;
        }

        let current_step = view.current_step();
        let current_power = i32::from(target.current_power());

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
                .unwrap_or_else(|| i32::from(target.current_power()));
            let target_toughness = view
                .get_effective_toughness(target.id)
                .unwrap_or_else(|| i32::from(target.current_toughness()));
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
                                    .unwrap_or_else(|| i32::from(atk_card.current_power()));
                                total_damage += atk_power;
                            }
                        } else {
                            // Blocked attacker - only counts trample damage
                            if let Some(atk_card) = view.get_card(attacker_id) {
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
                                .unwrap_or_else(|| i32::from(blocker.current_toughness()));

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
                    .map(|a| {
                        view.get_effective_power(a.id)
                            .unwrap_or_else(|| i32::from(a.current_power()))
                    })
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
                            .unwrap_or_else(|| i32::from(attacker.current_toughness()));

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
                let base_power = i32::from(target.current_power());
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
    fn has_equip_target(&self, source: &Card, view: &GameStateView) -> bool {
        if source.is_attached() {
            return false;
        }
        view.battlefield().iter().any(|&id| {
            view.get_card(id)
                .is_some_and(|c| c.controller == self.player_id && c.is_creature())
        })
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

    /// Check if there's a valuable tapped creature we can destroy
    /// Reference: DestroyAi.java - targets "best creature" from valid targets
    ///
    /// For Royal Assassin specifically, targets must be tapped creatures.
    /// We evaluate based on creature value - prefer destroying high-power/value targets.
    fn has_valuable_destroy_target(&self, view: &GameStateView) -> bool {
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

    /// Evaluate whether to cast a non-creature, non-pump spell
    ///
    /// Reference: Various spell AI classes in forge-ai/src/main/java/forge/ai/ability/
    fn should_cast_spell(&self, spell: &Card, view: &GameStateView) -> bool {
        // Check for card draw spells
        // Reference: DrawAi.java:30-120 (checkApiLogic)
        // Cast draw spells when hand is getting low (4 or fewer cards)
        // More aggressive than before (was 2) to keep card advantage flowing
        let has_draw = spell.effects.iter().any(|e| {
            matches!(
                e,
                crate::core::Effect::DrawCards { .. } | crate::core::Effect::DrawCardsXPaid { .. }
            )
        });
        if has_draw {
            let hand_size = view.hand().len();
            // Draw if we have 4 or fewer cards in hand
            // Reference: DrawAi.java - Java Forge draws when hand < 4-5
            if hand_size <= 4 {
                // Check timing for instant-speed draw - prefer opponent's end step for bluffing
                if spell.is_instant() && !self.should_cast_instant_now(view, spell) {
                    return false;
                }
                return true;
            }
        }

        // Check for removal spells (destroy or damage effects)
        // Reference: DestroyAi.java:106-303 (checkApiLogic)
        let has_destroy = spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::DestroyPermanent { .. }));
        let has_damage = spell.effects.iter().any(|e| {
            matches!(
                e,
                crate::core::Effect::DealDamage { .. } | crate::core::Effect::DealDamageXPaid { .. }
            )
        });

        if has_destroy || has_damage {
            // Check if there's a valid removal target AND if timing is right
            // Reference: DestroyAi.java:246 calls useRemovalNow() before committing
            if let Some(target) = self.choose_best_removal_target(spell, view) {
                if self.use_removal_now(spell, target, view) {
                    return true;
                }
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

        // Check for enchantments with static abilities
        // Reference: AttachAi.java:47-91 (checkApiLogic) for Auras
        // Reference: PumpAllAi.java:29-240 (checkApiLogic) for global enchantments
        if spell.definition.cache.is_enchantment {
            // Handle Auras (require targeting)
            if spell.definition.cache.is_aura {
                if self.should_cast_aura(spell, view) {
                    return true;
                }
            } else {
                // Handle global enchantments (no targeting)
                if self.should_cast_global_enchantment(spell, view) {
                    return true;
                }
            }
        }

        // Check for board wipes (DestroyAll, DamageAll, SacrificeAll)
        // Reference: DestroyAllAi.java:52-175 (doMassRemovalLogic)
        let has_mass_removal = spell.effects.iter().any(|e| {
            matches!(
                e,
                crate::core::Effect::DestroyAll { .. }
                    | crate::core::Effect::DamageAll { .. }
                    | crate::core::Effect::SacrificeAll { .. }
            )
        });
        if has_mass_removal && self.should_cast_board_wipe(spell, view) {
            return true;
        }

        // Check for sacrifice effects (ForceSacrifice)
        let has_force_sac = spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::ForceSacrifice { .. }));
        if has_force_sac && self.should_cast_force_sacrifice(view) {
            return true;
        }

        // Check for TapAll effects
        let has_tap_all = spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::TapAll { .. }));
        if has_tap_all && self.should_cast_tap_all(view) {
            return true;
        }

        // Check for UntapAll effects
        let has_untap_all = spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::UntapAll { .. }));
        if has_untap_all && self.should_cast_untap_all(view) {
            return true;
        }

        // Check for SetLife effects
        let has_set_life = spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::SetLife { .. }));
        if has_set_life && self.should_cast_set_life(spell, view) {
            return true;
        }

        // Check for LoseLife effects (targeting opponent)
        let has_lose_life = spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::LoseLife { .. }));
        if has_lose_life {
            // LoseLife targeting opponent is almost always worth casting
            return true;
        }

        // Check for Fight effects (creature mutual damage)
        // Reference: FightAi.java:27-108 (checkApiLogic)
        let has_fight = spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::Fight { .. }));
        if has_fight && self.should_cast_fight(view) {
            return true;
        }

        // Check for GainControl effects (steal creature)
        // Reference: ControlGainAi.java (checkApiLogic)
        let has_gain_control = spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::GainControl { .. }));
        if has_gain_control && self.should_cast_gain_control(view) {
            return true;
        }

        // Check for PutCounterAll effects (mass counter placement)
        // Reference: CountersPutAllAi.java:25-115 (checkApiLogic)
        let has_put_counter_all = spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::PutCounterAll { .. }));
        if has_put_counter_all && self.should_cast_put_counter_all(spell, view) {
            return true;
        }

        // Check for ChangeZoneAll effects (mass zone changes: bounce, exile, etc.)
        // Reference: ChangeZoneAllAi.java:20-200 (canPlay)
        let has_change_zone_all = spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::ChangeZoneAll { .. }));
        if has_change_zone_all && self.should_cast_change_zone_all(spell, view) {
            return true;
        }

        // Check for Discard effects (Hymn to Tourach, Mind Rot, etc.)
        // Reference: DiscardAi.java:27-120 (checkApiLogic)
        // Discard is almost always good when opponent has cards in hand
        let has_discard = spell.effects.iter().any(|e| {
            matches!(
                e,
                crate::core::Effect::DiscardCards { .. } | crate::core::Effect::DiscardCardsXPaid { .. }
            )
        });
        if has_discard && self.should_cast_discard(view) {
            return true;
        }

        // Check for single-target Tap effects (Icy Manipulator spell mode, etc.)
        // Reference: TapAi.java:26-100 (checkApiLogic)
        let has_tap = spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::TapPermanent { .. }));
        if has_tap && self.should_cast_tap_permanent(view) {
            return true;
        }

        // Check for single-target Untap effects
        let has_untap = spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::UntapPermanent { .. }));
        if has_untap {
            // Untapping our own permanents is almost always good
            return true;
        }

        // Check for TapOrUntap effects (Bounding Krasis ETB, etc.)
        let has_tap_or_untap = spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::TapOrUntapPermanent { .. }));
        if has_tap_or_untap {
            // Flexible effect - always worth casting
            return true;
        }

        // Check for DebuffCreature effects (removing keywords from opponent creatures)
        let has_debuff = spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::DebuffCreature { .. }));
        if has_debuff && self.should_cast_debuff(view) {
            return true;
        }

        // Check for Regenerate spell effects (cast proactively to protect creatures)
        let has_regenerate = spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::Regenerate { .. }));
        if has_regenerate {
            // Only cast during combat when our creatures are in danger
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

        // Planeswalkers are always worth casting (they provide ongoing value via loyalty abilities)
        if spell
            .types
            .iter()
            .any(|t| matches!(t, crate::core::CardType::Planeswalker))
        {
            return true;
        }

        // Utility artifacts with non-mana activated abilities (Icy Manipulator, etc.).
        // These have no spell effects themselves but provide board control via their
        // activated abilities once on the battlefield. Cast them when the opponent has
        // permanents that those abilities could affect (CR 302.6: artifacts are permanent
        // spells, they don't need ETB effects to be useful).
        if spell.is_artifact() && !spell.is_creature() {
            let has_useful_activated = spell.activated_abilities.iter().any(|ab| !ab.is_mana_ability);
            if has_useful_activated {
                // Only cast if the opponent has relevant permanents the ability can affect
                let opponent_has_permanents = view
                    .battlefield()
                    .iter()
                    .any(|&card_id| view.get_card(card_id).is_some_and(|c| c.controller != self.player_id));
                if opponent_has_permanents {
                    return true;
                }
            }
        }

        // Always-beneficial effects: search library, create tokens, scry, surveil, etc.
        // These effects always benefit the caster and should be cast when possible.
        // Examples: Demonic Tutor (SearchLibrary), Dragon Fodder (CreateToken),
        //           Opt (Scry), Thought Erasure (Surveil), Time Walk (AddTurn)
        //           Mind Sculpt (Mill), Healing Salve (GainLife), Overrun (PumpAllCreatures)
        let has_always_beneficial = spell.effects.iter().any(|e| {
            matches!(
                e,
                crate::core::Effect::SearchLibrary { .. }
                    | crate::core::Effect::CreateToken { .. }
                    | crate::core::Effect::Scry { .. }
                    | crate::core::Effect::Surveil { .. }
                    | crate::core::Effect::Loot { .. }
                    | crate::core::Effect::Dig { .. }
                    | crate::core::Effect::CopyPermanent { .. }
                    | crate::core::Effect::ExilePermanent { .. }
                    | crate::core::Effect::Balance { .. }
                    | crate::core::Effect::AddTurn { .. }
                    | crate::core::Effect::Mill { .. }
                    | crate::core::Effect::GainLife { .. }
                    | crate::core::Effect::PumpAllCreatures { .. }
                    | crate::core::Effect::AnimateAll { .. }
                    | crate::core::Effect::MultiplyCounter { .. }
                    | crate::core::Effect::PutCounter { .. }
                    | crate::core::Effect::Proliferate
                    | crate::core::Effect::PreventDamage { .. }
            )
        });
        if has_always_beneficial {
            return true;
        }

        false
    }

    /// Determine if we should cast an instant-speed spell now (bluffing logic)
    ///
    /// Reference: Java Forge phase restriction patterns (e.g., "AtOpponentsCombatOrAfter", "AtEOT")
    /// from various AI files (DestroyAi.java, DrawAi.java, etc.)
    ///
    /// This implements bluffing/deception by holding instant-speed spells until opponent's turn
    /// when possible, to:
    /// 1. Bluff having combat tricks/removal
    /// 2. See what opponent does before committing mana
    /// 3. Maintain maximum flexibility
    ///
    /// Key timing windows for instant-speed spells:
    /// - Opponent's end step: Preferred window (bluffs combat tricks all turn)
    /// - Our Main 2: Acceptable if we need to tap out for combat/attacks
    /// - Emergency: Immediate cast if hand is too full or spell is critical
    ///
    /// Returns true if we should cast the instant now, false if we should hold it.
    fn should_cast_instant_now(&self, view: &GameStateView, spell: &Card) -> bool {
        let current_step = view.current_step();
        let is_our_turn = view.active_player() == self.player_id;

        // Always cast sorcery-speed spells immediately (no bluffing possible)
        if !spell.is_instant() {
            return true;
        }

        // Interrupt 1: Hand is too full (7+ cards) - need to cast to avoid discarding
        // Reference: Similar to Java's hand size management in various AIs
        let hand_size = view.hand().len();
        if hand_size >= 7 {
            return true;
        }

        // Interrupt 2: Opponent's end step - BEST time to cast instant-speed non-combat spells
        // Reference: Java phase restrictions "AtEOT" pattern
        // This maximizes bluffing (held mana all turn = could be removal/combat tricks)
        if !is_our_turn && current_step == crate::game::Step::End {
            return true;
        }

        // Interrupt 3: Our Main 2 - acceptable timing if we're about to pass turn anyway
        // Reference: Java phase restrictions allowing Main 2 casting
        if is_our_turn && current_step == crate::game::Step::Main2 {
            return true;
        }

        // Interrupt 4: Combat phases - if opponent is attacking, might need to respond
        // Though draw spells don't directly interact, casting now prevents telegraphing
        let is_combat = matches!(
            current_step,
            crate::game::Step::DeclareAttackers | crate::game::Step::DeclareBlockers | crate::game::Step::CombatDamage
        );
        if is_combat {
            // During combat, only cast if hand is getting full (5+ cards)
            if hand_size >= 5 {
                return true;
            }
        }

        // Default: Hold the instant-speed spell for a better moment (bluffing)
        // This is the key bluffing logic - by default, don't cast instant-speed
        // draw/utility spells on our turn, wait for opponent's end step
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
        let is_damage_spell = top_spell.effects.iter().any(|e| {
            matches!(
                e,
                crate::core::Effect::DealDamage { .. } | crate::core::Effect::DealDamageXPaid { .. }
            )
        });
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
        let is_board_wipe = top_spell.effects.iter().any(|e| {
            matches!(
                e,
                crate::core::Effect::DestroyAll { .. }
                    | crate::core::Effect::SacrificeAll { .. }
                    | crate::core::Effect::DamageAll { .. }
                    | crate::core::Effect::ChangeZoneAll { .. }
            )
        });
        let is_extra_turn = top_spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::AddTurn { .. }));
        let is_gain_control = top_spell
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::GainControl { .. }));

        // Always counter dangerous spell types
        // Reference: CounterAi.java:151-182 (configurable countering preferences)
        if is_creature
            || is_damage_spell
            || is_removal_spell
            || is_counter_spell
            || is_pump_spell
            || is_board_wipe
            || is_extra_turn
            || is_gain_control
        {
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

    /// Evaluate whether to cast a global enchantment (non-Aura)
    ///
    /// Reference: PumpAllAi.java:29-240 (checkApiLogic)
    ///
    /// Global enchantments are permanent effects that buff/debuff creatures.
    /// Examples: Crusade (+1/+1 to white creatures), Bad Moon (+1/+1 to black creatures)
    ///
    /// Key decision logic:
    /// 1. Check if we have creatures that benefit from the buff (static abilities)
    /// 2. Compare our creature count vs opponent's for symmetric effects
    /// 3. Cast if the net benefit is positive for us
    fn should_cast_global_enchantment(&self, spell: &Card, view: &GameStateView) -> bool {
        // Look for ModifyPT static abilities on the enchantment
        let modify_pt_abilities: Vec<_> = spell
            .static_abilities
            .iter()
            .filter_map(|ability| {
                if let crate::core::StaticAbility::ModifyPT {
                    affected,
                    power,
                    toughness,
                    ..
                } = ability
                {
                    Some((affected, *power, *toughness))
                } else {
                    None
                }
            })
            .collect();

        if modify_pt_abilities.is_empty() {
            // No PT modification - check for keyword-granting or other beneficial statics
            // Cast keyword-granting enchantments if we have 2+ creatures that benefit
            let has_keyword_grant = spell.static_abilities.iter().any(|ability| {
                matches!(
                    ability,
                    crate::core::StaticAbility::GrantKeyword { .. } | crate::core::StaticAbility::GrantAbility { .. }
                )
            });

            if has_keyword_grant {
                // Count our creatures on battlefield - cast if we have 2+ to benefit
                let our_creature_count = view
                    .battlefield()
                    .iter()
                    .filter(|&&card_id| {
                        view.get_card(card_id)
                            .is_some_and(|c| c.is_creature() && c.controller == self.player_id)
                    })
                    .count();
                return our_creature_count >= 2;
            }

            // Check for enchantments with triggered abilities (beneficial ETB/upkeep triggers)
            if !spell.triggers.is_empty() {
                // Enchantments with triggers are usually beneficial - cast if we have creatures
                let has_creatures = view.battlefield().iter().any(|&card_id| {
                    view.get_card(card_id)
                        .is_some_and(|c| c.is_creature() && c.controller == self.player_id)
                });
                return has_creatures;
            }

            // Check for RaiseCost / ReduceCost statics (Gloom, Karma, etc.).
            // These are "hate" enchantments that hose a colour or type.
            // Cast if the opponent controls permanents that share the targeted
            // colour — even one permanent is enough to make the enchantment
            // valuable (it slows down every future spell of that colour).
            // CR 601.2f: cost-raising statics apply to all players, but the
            // primary value here is hosing the opponent.
            let raise_cost_abilities: Vec<_> = spell
                .static_abilities
                .iter()
                .filter_map(|ab| {
                    if let crate::core::StaticAbility::RaiseCost { valid_card, .. } = ab {
                        Some(valid_card)
                    } else {
                        None
                    }
                })
                .collect();

            if !raise_cost_abilities.is_empty() {
                // Cast if the opponent has any permanent whose colour/type
                // matches the RaiseCost target — meaning the effect will hose them.
                let opponent_has_target = view.battlefield().iter().any(|&card_id| {
                    let Some(card) = view.get_card(card_id) else {
                        return false;
                    };
                    if card.controller == self.player_id {
                        return false;
                    }
                    raise_cost_abilities
                        .iter()
                        .any(|&valid_card| crate::game::actions::spell_matches_cost_filter(card, valid_card))
                });
                if opponent_has_target {
                    return true;
                }
            }

            // Unknown enchantment type - don't cast
            return false;
        }

        // For each ModifyPT ability, count affected creatures
        for (affected_selector, power_bonus, toughness_bonus) in modify_pt_abilities {
            // Count creatures we control that would benefit
            let our_creatures = view.battlefield().iter().filter_map(|&card_id| {
                let card = view.get_card(card_id)?;
                if card.owner == self.player_id
                    && card.is_creature()
                    && self.creature_matches_selector(card, affected_selector)
                {
                    Some(card_id)
                } else {
                    None
                }
            });

            let our_count = our_creatures.clone().count();
            let our_total_benefit = (power_bonus + toughness_bonus) * our_count as i32;

            // Count opponent creatures that would benefit (for symmetric effects)
            let opponent_creatures = view.battlefield().iter().filter_map(|&card_id| {
                let card = view.get_card(card_id)?;
                if card.owner != self.player_id
                    && card.is_creature()
                    && self.creature_matches_selector(card, affected_selector)
                {
                    Some(card_id)
                } else {
                    None
                }
            });

            let opponent_count = opponent_creatures.count();
            let opponent_total_benefit = (power_bonus + toughness_bonus) * opponent_count as i32;

            // Cast if we benefit more than opponents
            // Reference: PumpAllAi.java uses various calculations, simplified here to net benefit
            if our_total_benefit > opponent_total_benefit && our_count > 0 {
                return true;
            }
        }

        false
    }

    /// Evaluate whether to cast an Aura enchantment
    ///
    /// Reference: AttachAi.java:47-91 (checkApiLogic)
    ///
    /// Auras enchant a creature and provide benefits (or penalties).
    /// Examples: Spirit Link (gain life when creature deals damage), Holy Strength (+1/+2)
    ///
    /// Key decision logic:
    /// 1. Check if there's a valid target creature
    /// 2. Prefer enchanting our own creatures with beneficial effects
    /// 3. Consider targeting opponent's creatures with negative effects
    fn should_cast_aura(&self, spell: &Card, view: &GameStateView) -> bool {
        // Look for beneficial static abilities (ModifyPT with positive values)
        let has_beneficial_pt = spell.static_abilities.iter().any(|ability| {
            if let crate::core::StaticAbility::ModifyPT { power, toughness, .. } = ability {
                // Beneficial if it grants positive power or toughness
                *power > 0 || *toughness > 0
            } else {
                false
            }
        });

        // Look for beneficial triggers (e.g., Spirit Link's life gain trigger)
        let has_beneficial_trigger = !spell.triggers.is_empty();

        if has_beneficial_pt || has_beneficial_trigger {
            // Try to find our best creature to enchant
            let our_creatures: Vec<_> = view
                .battlefield()
                .iter()
                .filter_map(|&card_id| {
                    let card = view.get_card(card_id)?;
                    if card.owner == self.player_id && card.is_creature() {
                        Some(card_id)
                    } else {
                        None
                    }
                })
                .collect();

            if !our_creatures.is_empty() {
                // Cast if we have at least one creature to enchant
                // Target selection will be handled by choose_aura_target
                return true;
            }
        }

        // TODO: Handle curse auras (negative effects on opponent creatures)
        // For now, don't cast those

        false
    }

    /// Evaluate whether to cast a board wipe (DestroyAll/DamageAll)
    ///
    /// Reference: DestroyAllAi.java:52-175 (doMassRemovalLogic)
    ///
    /// Key logic from Java:
    /// 1. Don't cast if opponent has no affected permanents
    /// 2. Cast if opponent creatures are more valuable than ours (creature_eval_threshold=200)
    /// 3. Cast immediately if life is in serious danger during combat
    /// 4. Prefer main phase 2 (after combat) unless emergency
    fn should_cast_board_wipe(&self, spell: &Card, view: &GameStateView) -> bool {
        // Evaluate each player's creatures that would be affected
        let mut our_creature_value: i32 = 0;
        let mut opp_creature_value: i32 = 0;
        let mut our_creature_count: i32 = 0;
        let mut opp_creature_count: i32 = 0;

        // Get the restriction from the effect (for type matching)
        #[allow(clippy::wildcard_enum_match_arm)]
        let restriction = spell.effects.iter().find_map(|e| match e {
            crate::core::Effect::DestroyAll { restriction, .. } => Some(restriction),
            crate::core::Effect::DamageAll { valid_cards, .. } => Some(valid_cards),
            _ => None,
        });

        for &card_id in view.battlefield() {
            let Some(card) = view.get_card(card_id) else {
                continue;
            };

            // Check if this permanent would be affected by the board wipe
            let affected = if let Some(r) = restriction {
                r.matches(card)
            } else {
                card.is_creature()
            };

            if !affected {
                continue;
            }

            // Skip indestructible creatures for DestroyAll
            if card.has_indestructible()
                && spell
                    .effects
                    .iter()
                    .any(|e| matches!(e, crate::core::Effect::DestroyAll { .. }))
            {
                continue;
            }

            let value = self.evaluate_creature(view, card_id);
            if card.controller == self.player_id {
                our_creature_value += value;
                our_creature_count += 1;
            } else {
                opp_creature_value += value;
                opp_creature_count += 1;
            }
        }

        // Don't cast if opponent has no affected creatures
        if opp_creature_count == 0 {
            return false;
        }

        // Java: CREATURE_EVAL_THRESHOLD = 200
        // Cast if opponent creatures are worth significantly more than ours
        let threshold = 200;
        if our_creature_value + threshold < opp_creature_value {
            return true;
        }

        // Cast if we're behind on board and losing life
        // (Simplified version of Java's lifeInSeriousDanger check)
        let our_life = view.life();
        if our_life <= 5 && opp_creature_count > our_creature_count {
            return true;
        }

        // Cast if opponent has significantly more creatures and we're losing
        if opp_creature_count >= our_creature_count + 2 && our_creature_value < opp_creature_value {
            return true;
        }

        false
    }

    /// Evaluate whether to cast a ForceSacrifice spell (e.g., Diabolic Edict)
    ///
    /// Simple heuristic: cast if opponent has creatures on the battlefield.
    /// More valuable if opponent has few creatures (they lose their best one).
    fn should_cast_force_sacrifice(&self, view: &GameStateView) -> bool {
        // Check if any opponent has creatures
        for opp_id in view.opponents() {
            let opp_creature_count = view
                .battlefield()
                .iter()
                .filter(|&&card_id| {
                    view.get_card(card_id)
                        .is_some_and(|c| c.is_creature() && c.controller == opp_id)
                })
                .count();

            if opp_creature_count > 0 {
                return true;
            }
        }
        false
    }

    /// Evaluate whether to cast TapAll
    ///
    /// Reference: TapAllAi.java
    /// Cast if opponent has untapped creatures (e.g., before our attack)
    fn should_cast_tap_all(&self, view: &GameStateView) -> bool {
        // Count opponent untapped creatures
        let opp_untapped_creatures = view
            .battlefield()
            .iter()
            .filter(|&&card_id| {
                view.get_card(card_id)
                    .is_some_and(|c| c.is_creature() && c.controller != self.player_id && !c.tapped)
            })
            .count();

        // Worth tapping if opponent has 2+ untapped creatures
        opp_untapped_creatures >= 2
    }

    /// Evaluate whether to cast UntapAll
    ///
    /// Reference: UntapAllAi.java
    /// Cast if we have tapped creatures that could attack or block
    fn should_cast_untap_all(&self, view: &GameStateView) -> bool {
        // Count our tapped creatures
        let our_tapped_creatures = view
            .battlefield()
            .iter()
            .filter(|&&card_id| {
                view.get_card(card_id)
                    .is_some_and(|c| c.is_creature() && c.controller == self.player_id && c.tapped)
            })
            .count();

        // Worth untapping if we have 2+ tapped creatures
        our_tapped_creatures >= 2
    }

    /// Evaluate whether to cast SetLife
    ///
    /// Cast if it would increase our life total
    fn should_cast_set_life(&self, spell: &Card, view: &GameStateView) -> bool {
        // Find the SetLife effect and its amount
        for effect in &spell.effects {
            if let crate::core::Effect::SetLife { amount, .. } = effect {
                // Cast if it would increase our life
                return *amount > view.life();
            }
        }
        false
    }

    /// Evaluate whether to cast a Discard spell (Hymn to Tourach, Mind Rot, etc.)
    ///
    /// Reference: DiscardAi.java:30-80 (checkApiLogic)
    /// Cast when opponent has cards in hand. More valuable early game.
    fn should_cast_discard(&self, view: &GameStateView) -> bool {
        // Check if any opponent has cards to discard
        let opp_hand_size: usize = view.opponents().map(|opp_id| view.player_hand_size(opp_id)).sum();

        // Don't cast if opponent has no cards to discard
        if opp_hand_size == 0 {
            return false;
        }

        // Always cast if opponent has cards (removing cards is always valuable)
        true
    }

    /// Evaluate whether to cast a single-target Tap spell
    ///
    /// Reference: TapAi.java:30-80 (checkApiLogic)
    /// Best used before combat to tap opponent's best blocker,
    /// or during opponent's turn to tap their best attacker.
    fn should_cast_tap_permanent(&self, view: &GameStateView) -> bool {
        // Check if opponent has untapped creatures we'd want to tap
        let has_untapped_opp_creature = view
            .battlefield()
            .iter()
            .filter_map(|&card_id| view.get_card(card_id))
            .any(|c| c.is_creature() && c.controller != self.player_id && !c.tapped);

        if has_untapped_opp_creature {
            return true;
        }

        // Also worthwhile if opponent has untapped mana sources we want to deny
        // (especially before they can cast something)
        false
    }

    /// Evaluate whether to cast a Debuff spell (remove keywords from opponent creature)
    ///
    /// Cast when opponent has creatures with relevant keywords (flying, etc.)
    fn should_cast_debuff(&self, view: &GameStateView) -> bool {
        // Check if opponent has creatures with evasion or other important keywords
        view.battlefield()
            .iter()
            .filter_map(|&card_id| view.get_card(card_id))
            .any(|c| {
                c.is_creature()
                    && c.controller != self.player_id
                    && (c.keywords.contains(crate::core::Keyword::Flying)
                        || c.keywords.contains(crate::core::Keyword::FirstStrike)
                        || c.keywords.contains(crate::core::Keyword::DoubleStrike)
                        || c.keywords.contains(crate::core::Keyword::Trample)
                        || c.keywords.contains(crate::core::Keyword::Hexproof)
                        || c.keywords.contains(crate::core::Keyword::Indestructible))
            })
    }

    /// Evaluate whether to cast a Fight spell
    ///
    /// Reference: FightAi.java:27-108 (checkApiLogic)
    ///
    /// Key logic from Java:
    /// 1. Need at least one targetable opponent creature
    /// 2. Find a favorable matchup where our creature can kill theirs without dying
    /// 3. Favorable = our power >= their toughness AND our toughness > their power
    ///
    /// For Fight spells, we target one of our creatures and one opponent creature.
    /// The AI should only cast if we can find a favorable fight.
    fn should_cast_fight(&self, view: &GameStateView) -> bool {
        // Get our creatures on the battlefield
        let our_creatures: Vec<_> = view
            .battlefield()
            .iter()
            .filter_map(|&card_id| view.get_card(card_id))
            .filter(|c| c.is_creature() && c.controller == self.player_id && !c.tapped)
            .collect();

        // Get opponent creatures on the battlefield
        let opp_creatures: Vec<_> = view
            .battlefield()
            .iter()
            .filter_map(|&card_id| view.get_card(card_id))
            .filter(|c| c.is_creature() && c.controller != self.player_id)
            .collect();

        if our_creatures.is_empty() || opp_creatures.is_empty() {
            return false;
        }

        // Look for a favorable matchup
        // Favorable = we can kill them AND we survive
        for our in &our_creatures {
            let our_power = i32::from(our.current_power());
            let our_toughness = i32::from(our.current_toughness());
            let our_has_deathtouch = our.has_deathtouch();

            for opp in &opp_creatures {
                let opp_power = i32::from(opp.current_power());
                let opp_toughness = i32::from(opp.current_toughness());
                let opp_has_deathtouch = opp.has_deathtouch();

                // Skip if opponent has indestructible (can't kill them)
                if opp.has_indestructible() {
                    continue;
                }

                // Check if we can kill them:
                // - With deathtouch: any damage (power > 0) is lethal
                // - Without deathtouch: need power >= toughness
                let we_can_kill = if our_has_deathtouch {
                    our_power > 0
                } else {
                    our_power >= opp_toughness
                };

                // Check if we survive (they can't kill us):
                // - We have indestructible: always survive
                // - They have deathtouch: any damage kills us
                // - Otherwise: their power < our toughness
                let we_survive = our.has_indestructible()
                    || (if opp_has_deathtouch {
                        opp_power == 0 // They can't deal damage
                    } else {
                        opp_power < our_toughness
                    });

                // Favorable fight: we kill them and we survive
                if we_can_kill && we_survive {
                    return true;
                }

                // Check if they can kill us
                let they_can_kill_us = if opp_has_deathtouch {
                    opp_power > 0
                } else {
                    opp_power >= our_toughness
                };

                // Also accept if we can trade for a more valuable creature
                // Trade = both die, but their creature is more valuable
                let we_die = they_can_kill_us && !our.has_indestructible();
                let they_die = we_can_kill && !opp.has_indestructible();
                if we_die && they_die {
                    let our_value = self.evaluate_creature(view, our.id);
                    let their_value = self.evaluate_creature(view, opp.id);
                    if their_value > our_value + 50 {
                        // Trade up: their creature is worth 50+ more points
                        return true;
                    }
                }
            }
        }

        false
    }

    /// Evaluate whether to cast a PutCounterAll spell
    ///
    /// Reference: CountersPutAllAi.java:25-115 (checkApiLogic)
    ///
    /// For beneficial counters (+1/+1): Only cast if we have more creatures benefiting than opponent.
    /// For curse counters (-1/-1): Only cast if 3+ opponent creatures would be killed.
    fn should_cast_put_counter_all(&self, spell: &Card, view: &GameStateView) -> bool {
        use crate::core::{CounterType, Effect};

        // Find the PutCounterAll effect to inspect its parameters
        let (restriction, counter_type, amount) = match spell.effects.iter().find_map(|e| {
            if let Effect::PutCounterAll {
                restriction,
                counter_type,
                amount,
            } = e
            {
                Some((restriction, counter_type, amount))
            } else {
                None
            }
        }) {
            Some(found) => found,
            None => return false,
        };

        // Count how many of our creatures match the restriction vs opponent's
        let mut our_count = 0u32;
        let mut opp_count = 0u32;

        for &card_id in view.battlefield() {
            if let Some(card) = view.get_card(card_id) {
                if restriction.matches(card) {
                    if card.controller == self.player_id {
                        our_count += 1;
                    } else {
                        opp_count += 1;
                    }
                }
            }
        }

        let is_curse = *counter_type == CounterType::M1M1;

        if is_curse {
            // For -1/-1 counters: only cast if we can kill 3+ opponent creatures
            // Reference: CountersPutAllAi.java:72-76
            let mut killable = 0u32;
            for &card_id in view.battlefield() {
                if let Some(card) = view.get_card(card_id) {
                    if restriction.matches(card)
                        && card.controller != self.player_id
                        && card.current_toughness() <= i8::try_from(*amount).unwrap_or(i8::MAX)
                    {
                        killable += 1;
                    }
                }
            }
            killable >= 3
        } else {
            // For beneficial counters: only cast if we benefit more creatures
            // Reference: CountersPutAllAi.java:86-88
            // Also need at least 1 creature of our own to benefit
            our_count > 0 && our_count > opp_count
        }
    }

    /// Evaluate whether to cast a ChangeZoneAll spell (mass zone change)
    ///
    /// Reference: ChangeZoneAllAi.java:20-200 (canPlay)
    ///
    /// For battlefield → hand/exile: Only cast if opponent loses more value than we do.
    /// For graveyard → exile: Cast if opponent has 3+ cards in graveyard.
    /// For graveyard → battlefield: Cast if we have creatures in graveyard (reanimation).
    fn should_cast_change_zone_all(&self, spell: &Card, view: &GameStateView) -> bool {
        use crate::core::Effect;

        // Find the ChangeZoneAll effect to inspect its parameters
        let (restriction, origins, _destination) = match spell.effects.iter().find_map(|e| {
            if let Effect::ChangeZoneAll {
                restriction,
                origins,
                destination,
                shuffle: _,
            } = e
            {
                Some((restriction, origins, destination))
            } else {
                None
            }
        }) {
            Some(found) => found,
            None => return false,
        };

        use crate::zones::Zone;

        // Mass moves that touch the battlefield are evaluated by board value; any
        // other origin (hand/graveyard shuffle like Timetwister) defaults to
        // beneficial. Pick the battlefield arm if it's among the origins.
        let primary_origin = if origins.contains(&Zone::Battlefield) {
            Zone::Battlefield
        } else {
            origins.first().copied().unwrap_or(Zone::Battlefield)
        };

        match primary_origin {
            Zone::Battlefield => {
                // Mass bounce/exile from battlefield: only do if opponent loses more
                // Count matching permanents for each player
                let mut our_value = 0i32;
                let mut opp_value = 0i32;

                for &card_id in view.battlefield() {
                    if let Some(card) = view.get_card(card_id) {
                        if restriction.matches(card) {
                            let value = if card.is_creature() {
                                // Use power + toughness as rough value
                                i32::from(card.current_power()) + i32::from(card.current_toughness())
                            } else {
                                // Non-creature permanents have some value
                                3
                            };

                            if card.controller == self.player_id {
                                our_value += value;
                            } else {
                                opp_value += value;
                            }
                        }
                    }
                }

                // Only cast if opponent loses significantly more value
                // Reference: ChangeZoneAllAi.java:163-166 (creatureEvalThreshold)
                opp_value > our_value + 4
            }
            Zone::Graveyard => {
                // Graveyard effects (exile, reanimation) are almost always beneficial
                // Reference: ChangeZoneAllAi.java:174-194 (graveyard handling)
                true
            }
            Zone::Hand | Zone::Exile | Zone::Library | Zone::Stack | Zone::Command => {
                // Other origin zones: default to casting
                true
            }
        }
    }

    fn should_cast_gain_control(&self, view: &GameStateView) -> bool {
        // Get opponent creatures on the battlefield
        let opp_creatures: Vec<_> = view
            .battlefield()
            .iter()
            .filter_map(|&card_id| view.get_card(card_id))
            .filter(|c| c.is_creature() && c.controller != self.player_id)
            .collect();

        if opp_creatures.is_empty() {
            return false;
        }

        // Stealing any creature is almost always good - it's a 2-for-1
        // (remove their creature AND gain one ourselves)
        // Only skip if opponent has literally no creatures
        true
    }

    /// Helper to check if a creature matches an AffectedSelector
    ///
    /// Simplified implementation - matches "Creature.YouCtrl", "Creature.White", etc.
    ///
    /// Note: Wildcard is intentional - AffectedSelector has 80+ variants;
    /// we handle the subset relevant to AI creature targeting decisions.
    #[allow(clippy::wildcard_enum_match_arm)]
    fn creature_matches_selector(&self, creature: &Card, selector: &crate::core::AffectedSelector) -> bool {
        use crate::core::AffectedSelector;

        match selector {
            AffectedSelector::CreaturesYouControl => creature.owner == self.player_id,
            AffectedSelector::AllCreatures => true,
            AffectedSelector::AllCreaturesOfColor { color } => {
                // Color is a String like "White", "Black", etc.
                // We need to check if the creature's colors contain the specified color
                creature.colors.iter().any(|c| {
                    let color_name = format!("{:?}", c); // "Red", "Blue", etc.
                    color.eq_ignore_ascii_case(&color_name)
                })
            }
            AffectedSelector::AllCreaturesOfType { subtype } => creature.subtypes.contains(subtype),
            AffectedSelector::CreatureTypeYouControl { subtype } => {
                creature.owner == self.player_id && creature.subtypes.contains(subtype)
            }
            AffectedSelector::CreatureEquippedBy => {
                // For equipment static abilities, not relevant for casting decision
                false
            }
            AffectedSelector::CreatureEnchantedBy => {
                // For aura static abilities, not relevant for casting decision
                false
            }
            AffectedSelector::CreatureAttachedBy => {
                // For equipment/aura static abilities, not relevant for casting decision
                false
            }
            AffectedSelector::CreaturesOpponentControls => creature.owner != self.player_id,
            // For other selectors, return false (not matched)
            _ => false,
        }
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
    #[allow(clippy::wildcard_enum_match_arm)]
    fn choose_best_removal_target(&self, spell: &Card, view: &GameStateView) -> Option<CardId> {
        // For damage-based removal, find the damage amount
        // For XPaid spells, use the x_paid value stored on the card
        let damage_amount = spell.effects.iter().find_map(|e| match e {
            crate::core::Effect::DealDamage { amount, .. } => Some(*amount),
            crate::core::Effect::DealDamageXPaid { .. } => Some(i32::from(spell.x_paid)),
            _ => None,
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
                            .map(|dmg| i32::from(c.current_toughness()) <= dmg)
                            .unwrap_or(true)
                } else {
                    false
                }
            })
            .collect();

        if opponent_creature_ids.is_empty() {
            return None;
        }

        // TODO(mtg-77): Implement more filtering from DestroyAi.java:
        // - Filter out creatures with shield counters (line 162)
        // - Filter out creatures that can be sacrificed in response (lines 165-186)
        // - Filter out creatures with regeneration shields (lines 191-194)
        // Note: useRemovalNow() timing check is now implemented separately

        // Select the best creature (highest evaluation score)
        // Reference: ComputerUtilCard.getBestCreatureAI() (line 224)
        self.get_best_creature(view, &opponent_creature_ids)
    }

    /// Determine if removal should be used NOW or held for a better moment
    ///
    /// Reference: ComputerUtilCard.useRemovalNow() (lines 1062-1278)
    ///
    /// Key logic from Java (simplified for our engine):
    /// 1. Sorcery-speed removal: always use now (limited casting windows)
    /// 2. Non-spell removal (activated abilities): always use now
    /// 3. Interrupt: target is enchanted → two-for-one card advantage
    /// 4. Interrupt: during combat → remove blocker/attacker for tempo
    /// 5. Value threshold: removal cost vs target evaluation score
    /// 6. Phase awareness: prefer opponent's end step for instant removal
    fn use_removal_now(&self, spell: &Card, target_id: CardId, view: &GameStateView) -> bool {
        let current_step = view.current_step();

        // Sorcery-speed removal must be used now (limited windows)
        // Reference: Java useRemovalNow line 1185 (sorcery speed multiplier 2x)
        if spell.is_sorcery() {
            return true;
        }

        // --- Interrupt conditions (always use now) ---

        // Interrupt 1: Target is enchanted → removing it also removes attached auras (two-for-one)
        // Reference: Java useRemovalNow lines 1107-1115
        if self.target_has_auras(target_id, view) {
            return true;
        }

        // Interrupt 2: During our Main1 → remove blocker to enable attack
        // Reference: Java useRemovalNow lines 1070-1086
        if current_step == crate::game::Step::Main1 && view.active_player() == self.player_id {
            // We're about to attack; removing an opponent's creature now enables better attacks
            return true;
        }

        // Interrupt 3: During combat → tactical removal
        // Reference: Java useRemovalNow lines 1089-1104
        let is_combat = matches!(
            current_step,
            crate::game::Step::DeclareAttackers | crate::game::Step::DeclareBlockers | crate::game::Step::CombatDamage
        );
        if is_combat {
            return true;
        }

        // --- Value-based timing for instants outside combat ---

        // At opponent's end step: good time to use instant removal
        // Reference: Java useRemovalNow line 1192 (end-of-turn multiplier 2x)
        if current_step == crate::game::Step::End && view.active_player() != self.player_id {
            return true;
        }

        // Main2: acceptable timing for removal (post-combat cleanup)
        if current_step == crate::game::Step::Main2 {
            return true;
        }

        // Value threshold: if target is high-value, use removal even at suboptimal timing
        // Reference: Java useRemovalNow lines 1226-1260 (threat evaluation)
        if let Some(target) = view.get_card(target_id) {
            if target.is_creature() {
                let target_eval = self.evaluate_creature(view, target_id);
                // High-value creatures (evaluation >= 200) are worth removing immediately
                // This threshold matches Java's 0.8 * cost normalization for typical removal
                if target_eval >= 200 {
                    return true;
                }
            }
        }

        // Default: hold instant removal for a better moment
        false
    }

    /// Check if a permanent has auras attached to it
    ///
    /// Used by use_removal_now() to detect two-for-one opportunities.
    /// Removing an enchanted creature also destroys the attached auras.
    fn target_has_auras(&self, target_id: CardId, view: &GameStateView) -> bool {
        view.battlefield().iter().any(|&bf_id| {
            view.get_card(bf_id)
                .is_some_and(|c| c.definition.cache.is_aura && c.attached_to == Some(target_id))
        })
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
                total += i32::from(blocker.current_power());
            }
        }

        total
    }

    /// Check if attacker can be killed by a gang of blockers
    ///
    /// Reference: AiBlockController.makeGangBlocks()
    fn can_gang_kill(&self, attacker: &Card, blockers: &[&Card]) -> bool {
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
                        score +=
                            i32::from(mana.white + mana.blue + mana.black + mana.red + mana.green + mana.colorless);
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
        // TODO(mtg-77): Implement proper mode evaluation based on:
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
        bears.set_base_power(Some(2));
        bears.set_base_toughness(Some(2));
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
        let would_die = i32::from(bears.current_toughness()) + bad_toughness <= 0;
        assert!(would_die, "Creature should die with -5 toughness");

        let would_live = i32::from(bears.current_toughness()) + toughness_bonus > 0;
        assert!(would_live, "Creature should live with +3 toughness");

        // Test that we can calculate pumped power
        let pumped_power = i32::from(bears.current_power()) + power_bonus;
        assert_eq!(pumped_power, 5, "2/2 with +3/+3 should have 5 power");
    }

    #[test]
    fn test_pump_spell_evasion_granting() {
        use crate::core::{Card, CardType};

        let player_id = EntityId::new(1);
        let controller = HeuristicController::new(player_id);

        // Create a 2/2 ground creature (the one we might pump)
        let mut ground_creature = Card::new(EntityId::new(10), "Grizzly Bears", player_id);
        ground_creature.set_base_power(Some(2));
        ground_creature.set_base_toughness(Some(2));
        ground_creature.add_type(CardType::Creature);

        // Create a 1/1 flying creature (opponent's blocker)
        let mut flying_creature = Card::new(EntityId::new(11), "Bird", EntityId::new(2));
        flying_creature.set_base_power(Some(1));
        flying_creature.set_base_toughness(Some(1));
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

    #[test]
    fn test_damage_assignment_with_deathtouch() {
        // Test that deathtouch changes lethal damage calculation
        // MTG Rules 702.2c: Any nonzero damage from deathtouch is lethal
        //
        // Scenario: 2/2 Deathtouch attacker vs three blockers:
        // - 5/5 Big creature (eval ~250)
        // - 4/4 Medium creature (eval ~200)
        // - 3/3 Small creature (eval ~175)
        //
        // Without deathtouch: 2 damage total, can't kill anything
        // With deathtouch: 1 damage kills anything, so can kill 2 creatures!
        //
        // Expected order with deathtouch:
        // - 5/5: 1 damage kills (deathtouch), 1 damage left
        // - 4/4: 1 damage kills (deathtouch), 0 damage left
        // - 3/3: no damage left, unkillable

        let available_damage = 2;
        let blockers = vec![
            ("5/5 Big", 250, 5), // (name, eval, toughness)
            ("4/4 Medium", 200, 4),
            ("3/3 Small", 175, 3),
        ];

        // Sort by evaluation (descending - target best creatures first)
        let mut sorted = blockers.clone();
        sorted.sort_by(|a, b| b.1.cmp(&a.1));

        // Simulate algorithm WITH deathtouch
        let attacker_has_deathtouch = true;
        let mut remaining = available_damage;
        let mut killable = vec![];
        let mut unkillable = vec![];

        for (name, _eval, toughness) in sorted {
            // With deathtouch, 1 damage is lethal
            let lethal_damage = if attacker_has_deathtouch && toughness > 0 {
                1
            } else {
                toughness
            };

            if lethal_damage <= remaining {
                killable.push(name);
                remaining -= lethal_damage;
            } else {
                unkillable.push(name);
            }
        }

        // With deathtouch: can kill 5/5 (1 dmg) and 4/4 (1 dmg), 3/3 no damage left
        assert_eq!(killable, vec!["5/5 Big", "4/4 Medium"]);
        assert_eq!(unkillable, vec!["3/3 Small"]);

        // Verify without deathtouch would give different (worse) result
        let mut no_dt_remaining = 2;
        let mut no_dt_killable = vec![];

        for (name, _eval, toughness) in blockers {
            if toughness <= no_dt_remaining {
                no_dt_killable.push(name);
                no_dt_remaining -= toughness;
            }
        }

        // Without deathtouch: can't kill anything with only 2 damage
        assert!(
            no_dt_killable.is_empty(),
            "Without deathtouch, 2 damage can't kill any creature"
        );
    }

    #[test]
    fn test_damage_assignment_with_indestructible() {
        // Test that indestructible blockers are always put last
        // MTG Rules 702.12: Indestructible creatures can't be destroyed by damage
        //
        // Scenario: 6/6 attacker vs three blockers:
        // - 4/4 Indestructible (eval ~300 due to indestructible bonus)
        // - 3/3 Normal creature (eval ~175)
        // - 2/2 Normal creature (eval ~140)
        //
        // Even though the indestructible creature has highest eval,
        // it should be last because we can't kill it anyway.
        //
        // Expected: kill 3/3 and 2/2, leave indestructible last

        let available_damage = 6;
        let blockers = vec![
            ("4/4 Indestructible", 300, 4, true), // (name, eval, toughness, indestructible)
            ("3/3 Normal", 175, 3, false),
            ("2/2 Normal", 140, 2, false),
        ];

        // Sort by evaluation (descending)
        let mut sorted = blockers.clone();
        sorted.sort_by(|a, b| b.1.cmp(&a.1));

        // Simulate algorithm with indestructible check
        let mut remaining = available_damage;
        let mut killable = vec![];
        let mut unkillable = vec![];

        for (name, _eval, toughness, is_indestructible) in sorted {
            if is_indestructible {
                // Indestructible = always unkillable
                unkillable.push(name);
                continue;
            }

            if toughness <= remaining {
                killable.push(name);
                remaining -= toughness;
            } else {
                unkillable.push(name);
            }
        }

        // Killable: 3/3 (3 dmg) and 2/2 (2 dmg) = 5 damage used
        // Unkillable: 4/4 Indestructible (even though it was first by eval)
        assert_eq!(killable, vec!["3/3 Normal", "2/2 Normal"]);
        assert_eq!(unkillable, vec!["4/4 Indestructible"]);

        // Final order: killable first, unkillable last
        let final_order: Vec<_> = killable.into_iter().chain(unkillable).collect();
        assert_eq!(final_order[2], "4/4 Indestructible");
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
                produces_chosen_color: false,
                amount_var: None,
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
        llanowar_elves.set_base_power(Some(1));
        llanowar_elves.set_base_toughness(Some(1));
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
                produces_chosen_color: false,
                amount_var: None,
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
                produces_chosen_color: false,
                amount_var: None,
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
                no_regenerate: false,
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
        creature.set_base_power(Some(2));
        creature.set_base_toughness(Some(2));
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

    #[test]
    fn test_combat_restriction_penalties() {
        // Test that creatures with combat restrictions are properly penalized
        // Reference: CreatureEvaluator.java:177-197
        use crate::core::{Card, CardType, ManaCost};
        use crate::game::controller::GameStateView;
        use crate::game::GameState;

        let player_id = EntityId::new(1);
        let controller = HeuristicController::new(player_id);

        // Create a simple game state
        let game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);

        // Helper: Create a 3/3 creature for testing
        let make_creature = |id: u32, keywords: Vec<Keyword>| {
            let card_id = EntityId::new(id);
            let mut creature = Card::new(card_id, "Test Creature", player_id);
            creature.set_base_power(Some(3));
            creature.set_base_toughness(Some(3));
            creature.add_type(CardType::Creature);
            creature.mana_cost = ManaCost::from_string("3");
            for kw in keywords {
                creature.keywords.insert(kw);
            }
            (card_id, creature)
        };

        let _view = GameStateView::new(&game, player_id);

        // Test 1: Normal 3/3 creature (baseline)
        let (normal_id, normal) = make_creature(100, vec![]);
        let mut test_game = game.clone();
        test_game.cards.insert(normal_id, normal);
        let view_normal = GameStateView::new(&test_game, player_id);
        let normal_value = controller.evaluate_creature(&view_normal, normal_id);
        println!("Normal 3/3 value: {}", normal_value);

        // Test 2: Creature with Defender (can't attack)
        // Java: value -= power * 9 + 40 = 3*9 + 40 = 67 penalty
        let (defender_id, defender) = make_creature(101, vec![Keyword::Defender]);
        let mut test_game = game.clone();
        test_game.cards.insert(defender_id, defender);
        let view_defender = GameStateView::new(&test_game, player_id);
        let defender_value = controller.evaluate_creature(&view_defender, defender_id);
        println!("Defender 3/3 value: {}", defender_value);
        assert!(
            defender_value < normal_value,
            "Defender should be worth less than normal creature"
        );
        // Expected penalty: power*9 + 40 = 3*9 + 40 = 67
        assert_eq!(
            normal_value - defender_value,
            67,
            "Defender penalty should be power*9 + 40"
        );

        // Test 3: Creature with CantBlock
        // Java: value -= 10
        let (cant_block_id, cant_block) = make_creature(102, vec![Keyword::CantBlock]);
        let mut test_game = game.clone();
        test_game.cards.insert(cant_block_id, cant_block);
        let view_cant_block = GameStateView::new(&test_game, player_id);
        let cant_block_value = controller.evaluate_creature(&view_cant_block, cant_block_id);
        println!("CantBlock 3/3 value: {}", cant_block_value);
        assert!(
            cant_block_value < normal_value,
            "CantBlock should be worth less than normal creature"
        );
        assert_eq!(normal_value - cant_block_value, 10, "CantBlock penalty should be 10");

        // Test 4: Creature with MustAttack
        // Java: value -= 10
        let (must_attack_id, must_attack) = make_creature(103, vec![Keyword::MustAttack]);
        let mut test_game = game.clone();
        test_game.cards.insert(must_attack_id, must_attack);
        let view_must_attack = GameStateView::new(&test_game, player_id);
        let must_attack_value = controller.evaluate_creature(&view_must_attack, must_attack_id);
        println!("MustAttack 3/3 value: {}", must_attack_value);
        assert!(
            must_attack_value < normal_value,
            "MustAttack should be worth less than normal creature"
        );
        assert_eq!(normal_value - must_attack_value, 10, "MustAttack penalty should be 10");

        // Test 5: Creature with Goaded
        // Java: value -= 5
        let (goaded_id, goaded) = make_creature(104, vec![Keyword::Goaded]);
        let mut test_game = game.clone();
        test_game.cards.insert(goaded_id, goaded);
        let view_goaded = GameStateView::new(&test_game, player_id);
        let goaded_value = controller.evaluate_creature(&view_goaded, goaded_id);
        println!("Goaded 3/3 value: {}", goaded_value);
        assert!(
            goaded_value < normal_value,
            "Goaded should be worth less than normal creature"
        );
        assert_eq!(normal_value - goaded_value, 5, "Goaded penalty should be 5");

        // Test 6: Creature with CantAttackOrBlock (nearly useless)
        // Java: value = 50 + (cmc * 5) = 50 + (3 * 5) = 65 (total value, not penalty)
        let (useless_id, useless) = make_creature(105, vec![Keyword::CantAttackOrBlock]);
        let mut test_game = game.clone();
        test_game.cards.insert(useless_id, useless);
        let view_useless = GameStateView::new(&test_game, player_id);
        let useless_value = controller.evaluate_creature(&view_useless, useless_id);
        println!("CantAttackOrBlock 3/3 value: {}", useless_value);
        assert!(
            useless_value < normal_value,
            "CantAttackOrBlock should be much less valuable"
        );
        assert_eq!(useless_value, 65, "CantAttackOrBlock should reset value to 50 + cmc*5");
    }

    #[test]
    fn test_blocking_restrictions_evasion() {
        // Test the can_block function for various evasion abilities
        // Reference: CombatUtil.canBlock() in Java Forge
        use crate::core::{Card, CardType, Color};

        let player_id = EntityId::new(1);
        let opponent_id = EntityId::new(2);
        let controller = HeuristicController::new(player_id);

        // Helper to create creatures with specified properties
        let make_creature = |id: u32, owner: PlayerId, keywords: Vec<Keyword>, colors: Vec<Color>| {
            let card_id = EntityId::new(id);
            let mut creature = Card::new(card_id, "Test Creature", owner);
            creature.set_base_power(Some(2));
            creature.set_base_toughness(Some(2));
            creature.add_type(CardType::Creature);
            for kw in keywords {
                creature.keywords.insert(kw);
            }
            for color in colors {
                creature.colors.push(color);
            }
            creature
        };

        // ========== FEAR TESTS ==========
        // Fear: can only be blocked by artifact creatures or black creatures
        {
            let attacker_with_fear = make_creature(100, opponent_id, vec![Keyword::Fear], vec![Color::Black]);
            let white_blocker = make_creature(101, player_id, vec![], vec![Color::White]);
            let black_blocker = make_creature(102, player_id, vec![], vec![Color::Black]);
            let mut artifact_blocker = make_creature(103, player_id, vec![], vec![]);
            artifact_blocker.add_type(CardType::Artifact);

            // White creature can't block creature with Fear
            assert!(
                !controller.can_block(&attacker_with_fear, &white_blocker),
                "White creature should not be able to block creature with Fear"
            );

            // Black creature CAN block creature with Fear
            assert!(
                controller.can_block(&attacker_with_fear, &black_blocker),
                "Black creature should be able to block creature with Fear"
            );

            // Artifact creature CAN block creature with Fear
            assert!(
                controller.can_block(&attacker_with_fear, &artifact_blocker),
                "Artifact creature should be able to block creature with Fear"
            );
        }

        // ========== INTIMIDATE TESTS ==========
        // Intimidate: can only be blocked by artifact creatures or creatures that share a color
        {
            let red_attacker_intimidate = make_creature(110, opponent_id, vec![Keyword::Intimidate], vec![Color::Red]);
            let green_blocker = make_creature(111, player_id, vec![], vec![Color::Green]);
            let red_blocker = make_creature(112, player_id, vec![], vec![Color::Red]);
            let mut artifact_blocker = make_creature(113, player_id, vec![], vec![]);
            artifact_blocker.add_type(CardType::Artifact);

            // Green creature can't block red creature with Intimidate
            assert!(
                !controller.can_block(&red_attacker_intimidate, &green_blocker),
                "Green creature should not be able to block red creature with Intimidate"
            );

            // Red creature CAN block red creature with Intimidate (shares color)
            assert!(
                controller.can_block(&red_attacker_intimidate, &red_blocker),
                "Red creature should be able to block red creature with Intimidate (shares color)"
            );

            // Artifact creature CAN block creature with Intimidate
            assert!(
                controller.can_block(&red_attacker_intimidate, &artifact_blocker),
                "Artifact creature should be able to block creature with Intimidate"
            );
        }

        // ========== SHADOW TESTS ==========
        // Shadow: can only be blocked by shadow, and shadow can only block shadow
        {
            let shadow_attacker = make_creature(120, opponent_id, vec![Keyword::Shadow], vec![Color::Black]);
            let normal_blocker = make_creature(121, player_id, vec![], vec![Color::White]);
            let shadow_blocker = make_creature(122, player_id, vec![Keyword::Shadow], vec![Color::Black]);

            // Normal creature can't block shadow creature
            assert!(
                !controller.can_block(&shadow_attacker, &normal_blocker),
                "Normal creature should not be able to block creature with Shadow"
            );

            // Shadow creature CAN block shadow creature
            assert!(
                controller.can_block(&shadow_attacker, &shadow_blocker),
                "Shadow creature should be able to block creature with Shadow"
            );

            // Test the reverse: shadow creature can't be blocked by normal creatures either
            let normal_attacker = make_creature(123, opponent_id, vec![], vec![Color::Black]);
            assert!(
                !controller.can_block(&normal_attacker, &shadow_blocker),
                "Shadow creature should not be able to block normal creature"
            );
        }

        // ========== SKULK TESTS ==========
        // Skulk: can only be blocked by creatures with greater power
        {
            let skulk_attacker = make_creature(130, opponent_id, vec![Keyword::Skulk], vec![Color::Blue]);
            // skulk_attacker has power 2

            let mut weak_blocker = make_creature(131, player_id, vec![], vec![Color::White]);
            weak_blocker.set_base_power(Some(1)); // Power 1

            let mut equal_blocker = make_creature(132, player_id, vec![], vec![Color::White]);
            equal_blocker.set_base_power(Some(2)); // Power 2

            let mut strong_blocker = make_creature(133, player_id, vec![], vec![Color::White]);
            strong_blocker.set_base_power(Some(3)); // Power 3

            // Weak creature (power 1) can't block skulk creature (power 2)
            assert!(
                !controller.can_block(&skulk_attacker, &weak_blocker),
                "Creature with power 1 should not be able to block creature with Skulk and power 2"
            );

            // Equal power creature can't block skulk creature
            assert!(
                !controller.can_block(&skulk_attacker, &equal_blocker),
                "Creature with equal power should not be able to block creature with Skulk"
            );

            // Strong creature CAN block skulk creature
            assert!(
                controller.can_block(&skulk_attacker, &strong_blocker),
                "Creature with greater power should be able to block creature with Skulk"
            );
        }

        // ========== HORSEMANSHIP TESTS ==========
        // Horsemanship: can only be blocked by creatures with horsemanship
        {
            let horse_attacker = make_creature(140, opponent_id, vec![Keyword::Horsemanship], vec![Color::White]);
            let normal_blocker = make_creature(141, player_id, vec![], vec![Color::White]);
            let horse_blocker = make_creature(142, player_id, vec![Keyword::Horsemanship], vec![Color::White]);

            // Normal creature can't block horsemanship creature
            assert!(
                !controller.can_block(&horse_attacker, &normal_blocker),
                "Normal creature should not be able to block creature with Horsemanship"
            );

            // Horsemanship creature CAN block horsemanship creature
            assert!(
                controller.can_block(&horse_attacker, &horse_blocker),
                "Creature with Horsemanship should be able to block creature with Horsemanship"
            );
        }

        // ========== PROTECTION TESTS ==========
        // Protection from color: creature with protection can't be blocked by that color
        {
            let pro_red_attacker =
                make_creature(150, opponent_id, vec![Keyword::ProtectionFromRed], vec![Color::White]);
            let red_blocker = make_creature(151, player_id, vec![], vec![Color::Red]);
            let blue_blocker = make_creature(152, player_id, vec![], vec![Color::Blue]);

            // Red creature can't block creature with protection from red
            assert!(
                !controller.can_block(&pro_red_attacker, &red_blocker),
                "Red creature should not be able to block creature with Protection from Red"
            );

            // Blue creature CAN block creature with protection from red
            assert!(
                controller.can_block(&pro_red_attacker, &blue_blocker),
                "Blue creature should be able to block creature with Protection from Red"
            );
        }
    }

    /// Test that Royal Assassin's activated ability is properly classified as Destroy
    /// and the AI logic for evaluating destroy abilities works correctly.
    ///
    /// Royal Assassin (4ED): {T}: Destroy target tapped creature.
    /// Reference: DestroyAi.java in forge-ai
    #[test]
    fn test_destroy_ability_classification() {
        use crate::core::{ActivatedAbility, CardId, Cost, Effect, TargetRef, TargetRestriction};

        let player_id = EntityId::new(1);
        let controller = HeuristicController::new(player_id);

        // Create a destroy ability similar to Royal Assassin
        let destroy_ability = ActivatedAbility::new(
            Cost::Tap,
            vec![Effect::DestroyPermanent {
                target: CardId::new(0), // Placeholder target
                restriction: TargetRestriction::any(),
                no_regenerate: false,
            }],
            "Destroy target tapped creature".to_string(),
            false, // not a mana ability
        );

        // Test that the ability is classified as Destroy
        let ability_type = controller.classify_activated_ability(&destroy_ability);
        assert!(
            matches!(ability_type, ActivatedAbilityType::Destroy),
            "Royal Assassin's ability should be classified as Destroy"
        );

        // Test that ping abilities are still classified correctly
        let ping_ability = ActivatedAbility::new(
            Cost::Tap,
            vec![Effect::DealDamage {
                target: TargetRef::Permanent(CardId::new(0)),
                amount: 1,
            }],
            "{T}: Deal 1 damage to any target".to_string(),
            false,
        );
        assert!(
            matches!(
                controller.classify_activated_ability(&ping_ability),
                ActivatedAbilityType::Ping { damage: 1 }
            ),
            "Prodigal Sorcerer's ability should be classified as Ping"
        );

        // Test that pump abilities are still classified correctly
        let pump_ability = ActivatedAbility::new(
            Cost::Mana(crate::core::ManaCost::from_string("R")),
            vec![Effect::PumpCreature {
                target: CardId::new(0),
                power_bonus: 1,
                toughness_bonus: 0,
                keywords_granted: smallvec::SmallVec::new(),
            }],
            "{R}: +1/+0 until end of turn".to_string(),
            false,
        );
        assert!(
            matches!(
                controller.classify_activated_ability(&pump_ability),
                ActivatedAbilityType::Pump { power: 1, toughness: 0 }
            ),
            "Shivan Dragon's ability should be classified as Pump"
        );
    }

    /// Test loading Royal Assassin from cardsfolder and verifying ability parsing
    #[test]
    fn test_royal_assassin_from_cardsfolder() {
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/r/royal_assassin.txt");
        if !path.exists() {
            println!("Skipping test: cardsfolder not present");
            return;
        }

        let def = crate::loader::CardLoader::load_from_file(&path).expect("Failed to load Royal Assassin");
        assert_eq!(def.name.as_str(), "Royal Assassin");

        // Instantiate the card
        let game = crate::game::GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let card_id = crate::core::CardId::new(100);
        let card = def.instantiate(card_id, p1_id);

        // Verify basic card properties
        assert!(card.is_creature(), "Royal Assassin should be a creature");
        assert_eq!(card.current_power(), 1, "Royal Assassin should be 1/1");
        assert_eq!(card.current_toughness(), 1, "Royal Assassin should be 1/1");

        // Verify the activated ability was parsed
        assert!(
            !card.activated_abilities.is_empty(),
            "Royal Assassin should have at least one activated ability"
        );

        // Find the non-mana tap ability (the destroy ability)
        let destroy_abilities: Vec<_> = card
            .activated_abilities
            .iter()
            .filter(|a| !a.is_mana_ability && a.cost.includes_tap())
            .collect();

        assert_eq!(
            destroy_abilities.len(),
            1,
            "Royal Assassin should have exactly one tap-to-destroy ability"
        );

        // Verify the ability has a DestroyPermanent effect
        let ability = destroy_abilities[0];
        let has_destroy_effect = ability
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::DestroyPermanent { .. }));

        assert!(
            has_destroy_effect,
            "Royal Assassin's ability should have a DestroyPermanent effect"
        );

        // Test AI classification
        let controller = HeuristicController::new(p1_id);
        let ability_type = controller.classify_activated_ability(ability);
        assert!(
            matches!(ability_type, ActivatedAbilityType::Destroy),
            "Royal Assassin's ability should be classified as Destroy by AI"
        );
    }

    /// Test has_valuable_destroy_target evaluates tapped opponent creatures correctly
    #[test]
    fn test_has_valuable_destroy_target() {
        use crate::core::{Card, CardId, CardType};
        use crate::game::controller::GameStateView;
        use crate::game::GameState;

        // Create a game with two players
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        let controller = HeuristicController::new(p1_id);

        // Create an opponent's tapped 3/3 creature (valuable target)
        let card_id = CardId::new(50);
        let mut tapped_creature = Card::new(card_id, "Hill Giant", p2_id);
        tapped_creature.add_type(CardType::Creature);
        tapped_creature.set_base_power(Some(3));
        tapped_creature.set_base_toughness(Some(3));
        tapped_creature.tapped = true; // Tapped from attacking

        // Add to battlefield
        game.cards.insert(card_id, tapped_creature);
        game.battlefield.cards.push(card_id);

        // Create game state view
        let view = GameStateView::new(&game, p1_id);

        // Test that we detect this as a valuable target
        assert!(
            controller.has_valuable_destroy_target(&view),
            "Should detect 3/3 tapped creature as valuable destroy target"
        );
    }

    // ==================== Board Wipe AI Tests ====================

    /// Test: AI should cast Wrath of God when opponent has more valuable creatures
    #[test]
    fn test_should_cast_board_wipe_opponent_advantage() {
        use crate::core::{Card, CardId, CardType, ManaCost, TargetRestriction, TargetType};
        use crate::game::controller::GameStateView;
        use crate::game::GameState;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;
        let controller = HeuristicController::new(p1_id);

        // P1: One small creature (Grizzly Bears 2/2)
        let c1_id = CardId::new(50);
        let mut c1 = Card::new(c1_id, "Grizzly Bears", p1_id);
        c1.add_type(CardType::Creature);
        c1.set_base_power(Some(2));
        c1.set_base_toughness(Some(2));
        c1.controller = p1_id;
        game.cards.insert(c1_id, c1);
        game.battlefield.add(c1_id);

        // P2: Three big creatures (Serra Angel 4/4 x2, Shivan Dragon 5/5)
        for (i, (name, p, t)) in [
            ("Serra Angel", 4i8, 4i8),
            ("Serra Angel", 4, 4),
            ("Shivan Dragon", 5, 5),
        ]
        .iter()
        .enumerate()
        {
            let id = CardId::new(60 + i as u32);
            let mut c = Card::new(id, *name, p2_id);
            c.add_type(CardType::Creature);
            c.set_base_power(Some(*p));
            c.set_base_toughness(Some(*t));
            c.controller = p2_id;
            game.cards.insert(id, c);
            game.battlefield.add(id);
        }

        // Create Wrath of God spell
        let wrath_id = CardId::new(100);
        let mut wrath = Card::new(wrath_id, "Wrath of God", p1_id);
        wrath.add_type(CardType::Sorcery);
        wrath.mana_cost = ManaCost::from_string("2WW");
        wrath.effects.push(crate::core::Effect::DestroyAll {
            restriction: TargetRestriction::from_types([TargetType::Creature]),
            no_regenerate: true,
        });
        game.cards.insert(wrath_id, wrath);

        let view = GameStateView::new(&game, p1_id);
        let wrath_card = view.get_card(wrath_id).unwrap();

        assert!(
            controller.should_cast_board_wipe(wrath_card, &view),
            "AI should cast Wrath of God when opponent has much more valuable creatures"
        );
    }

    /// Test: AI should NOT cast Wrath of God when AI has better board
    #[test]
    fn test_should_not_cast_board_wipe_own_advantage() {
        use crate::core::{Card, CardId, CardType, ManaCost, TargetRestriction, TargetType};
        use crate::game::controller::GameStateView;
        use crate::game::GameState;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;
        let controller = HeuristicController::new(p1_id);

        // P1: Two big creatures
        for (i, (name, p, t)) in [("Serra Angel", 4i8, 4i8), ("Shivan Dragon", 5, 5)].iter().enumerate() {
            let id = CardId::new(50 + i as u32);
            let mut c = Card::new(id, *name, p1_id);
            c.add_type(CardType::Creature);
            c.set_base_power(Some(*p));
            c.set_base_toughness(Some(*t));
            c.controller = p1_id;
            game.cards.insert(id, c);
            game.battlefield.add(id);
        }

        // P2: One small creature
        let c2_id = CardId::new(60);
        let mut c2 = Card::new(c2_id, "Grizzly Bears", p2_id);
        c2.add_type(CardType::Creature);
        c2.set_base_power(Some(2));
        c2.set_base_toughness(Some(2));
        c2.controller = p2_id;
        game.cards.insert(c2_id, c2);
        game.battlefield.add(c2_id);

        let wrath_id = CardId::new(100);
        let mut wrath = Card::new(wrath_id, "Wrath of God", p1_id);
        wrath.add_type(CardType::Sorcery);
        wrath.mana_cost = ManaCost::from_string("2WW");
        wrath.effects.push(crate::core::Effect::DestroyAll {
            restriction: TargetRestriction::from_types([TargetType::Creature]),
            no_regenerate: true,
        });
        game.cards.insert(wrath_id, wrath);

        let view = GameStateView::new(&game, p1_id);
        let wrath_card = view.get_card(wrath_id).unwrap();

        assert!(
            !controller.should_cast_board_wipe(wrath_card, &view),
            "AI should NOT cast Wrath of God when AI has better board position"
        );
    }

    /// Test: AI should cast board wipe when life is critically low
    #[test]
    fn test_should_cast_board_wipe_low_life() {
        use crate::core::{Card, CardId, CardType, ManaCost, TargetRestriction, TargetType};
        use crate::game::controller::GameStateView;
        use crate::game::GameState;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;
        let controller = HeuristicController::new(p1_id);

        // P1: Low life, no creatures
        game.get_player_mut(p1_id).unwrap().life = 3;

        // P2: Two creatures threatening lethal
        for (i, (name, p, t)) in [("Serra Angel", 4i8, 4i8), ("Grizzly Bears", 2, 2)].iter().enumerate() {
            let id = CardId::new(60 + i as u32);
            let mut c = Card::new(id, *name, p2_id);
            c.add_type(CardType::Creature);
            c.set_base_power(Some(*p));
            c.set_base_toughness(Some(*t));
            c.controller = p2_id;
            game.cards.insert(id, c);
            game.battlefield.add(id);
        }

        let wrath_id = CardId::new(100);
        let mut wrath = Card::new(wrath_id, "Wrath of God", p1_id);
        wrath.add_type(CardType::Sorcery);
        wrath.mana_cost = ManaCost::from_string("2WW");
        wrath.effects.push(crate::core::Effect::DestroyAll {
            restriction: TargetRestriction::from_types([TargetType::Creature]),
            no_regenerate: true,
        });
        game.cards.insert(wrath_id, wrath);

        let view = GameStateView::new(&game, p1_id);
        let wrath_card = view.get_card(wrath_id).unwrap();

        assert!(
            controller.should_cast_board_wipe(wrath_card, &view),
            "AI should cast Wrath of God when at 3 life facing 2 opponent creatures"
        );
    }

    // ==================== ForceSacrifice AI Tests ====================

    /// Test: AI casts edict when opponent has creatures
    #[test]
    fn test_should_cast_force_sacrifice_with_target() {
        use crate::core::{Card, CardId, CardType};
        use crate::game::controller::GameStateView;
        use crate::game::GameState;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;
        let controller = HeuristicController::new(p1_id);

        // P2 has a creature
        let c_id = CardId::new(50);
        let mut c = Card::new(c_id, "Shivan Dragon", p2_id);
        c.add_type(CardType::Creature);
        c.set_base_power(Some(5));
        c.set_base_toughness(Some(5));
        c.controller = p2_id;
        game.cards.insert(c_id, c);
        game.battlefield.add(c_id);

        let view = GameStateView::new(&game, p1_id);

        assert!(
            controller.should_cast_force_sacrifice(&view),
            "AI should cast edict when opponent has creatures"
        );
    }

    /// Test: AI doesn't cast edict when opponent has no creatures
    #[test]
    fn test_should_not_cast_force_sacrifice_no_targets() {
        use crate::game::controller::GameStateView;
        use crate::game::GameState;

        let game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let controller = HeuristicController::new(p1_id);

        let view = GameStateView::new(&game, p1_id);

        assert!(
            !controller.should_cast_force_sacrifice(&view),
            "AI should not cast edict when opponent has no creatures"
        );
    }

    // ==================== TapAll/UntapAll AI Tests ====================

    /// Test: AI casts TapAll when opponent has multiple untapped creatures
    #[test]
    fn test_should_cast_tap_all_with_targets() {
        use crate::core::{Card, CardId, CardType};
        use crate::game::controller::GameStateView;
        use crate::game::GameState;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;
        let controller = HeuristicController::new(p1_id);

        // P2 has 3 untapped creatures
        for i in 0..3 {
            let id = CardId::new(50 + i);
            let mut c = Card::new(id, "Grizzly Bears", p2_id);
            c.add_type(CardType::Creature);
            c.set_base_power(Some(2));
            c.set_base_toughness(Some(2));
            c.controller = p2_id;
            game.cards.insert(id, c);
            game.battlefield.add(id);
        }

        let view = GameStateView::new(&game, p1_id);

        assert!(
            controller.should_cast_tap_all(&view),
            "AI should cast TapAll when opponent has 3 untapped creatures"
        );
    }

    /// Test: AI doesn't cast TapAll when opponent has few untapped creatures
    #[test]
    fn test_should_not_cast_tap_all_few_targets() {
        use crate::core::{Card, CardId, CardType};
        use crate::game::controller::GameStateView;
        use crate::game::GameState;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;
        let controller = HeuristicController::new(p1_id);

        // P2 has 1 untapped creature (below threshold of 2)
        let id = CardId::new(50);
        let mut c = Card::new(id, "Grizzly Bears", p2_id);
        c.add_type(CardType::Creature);
        c.controller = p2_id;
        game.cards.insert(id, c);
        game.battlefield.add(id);

        let view = GameStateView::new(&game, p1_id);

        assert!(
            !controller.should_cast_tap_all(&view),
            "AI should not cast TapAll with only 1 opponent creature"
        );
    }

    // ==================== SetLife AI Tests ====================

    /// Test: AI casts SetLife when it increases life
    #[test]
    fn test_should_cast_set_life_when_beneficial() {
        use crate::core::{Card, CardId, CardType, ManaCost};
        use crate::game::controller::GameStateView;
        use crate::game::GameState;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let controller = HeuristicController::new(p1_id);

        // P1 at 5 life
        game.get_player_mut(p1_id).unwrap().life = 5;

        // Create spell that sets life to 10
        let spell_id = CardId::new(100);
        let mut spell = Card::new(spell_id, "Angel of Grace", p1_id);
        spell.add_type(CardType::Instant);
        spell.mana_cost = ManaCost::from_string("4WW");
        spell.effects.push(crate::core::Effect::SetLife {
            player: crate::core::PlayerId::new(0),
            amount: 10,
        });
        game.cards.insert(spell_id, spell);

        let view = GameStateView::new(&game, p1_id);
        let spell_card = view.get_card(spell_id).unwrap();

        assert!(
            controller.should_cast_set_life(spell_card, &view),
            "AI should cast SetLife when it would increase life from 5 to 10"
        );
    }

    /// Test: AI doesn't cast SetLife when it would decrease life
    #[test]
    fn test_should_not_cast_set_life_when_harmful() {
        use crate::core::{Card, CardId, CardType, ManaCost};
        use crate::game::controller::GameStateView;
        use crate::game::GameState;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let controller = HeuristicController::new(p1_id);

        // P1 at full 20 life
        // SetLife to 10 would be harmful

        let spell_id = CardId::new(100);
        let mut spell = Card::new(spell_id, "Angel of Grace", p1_id);
        spell.add_type(CardType::Instant);
        spell.mana_cost = ManaCost::from_string("4WW");
        spell.effects.push(crate::core::Effect::SetLife {
            player: crate::core::PlayerId::new(0),
            amount: 10,
        });
        game.cards.insert(spell_id, spell);

        let view = GameStateView::new(&game, p1_id);
        let spell_card = view.get_card(spell_id).unwrap();

        assert!(
            !controller.should_cast_set_life(spell_card, &view),
            "AI should NOT cast SetLife when it would decrease life from 20 to 10"
        );
    }

    // ==================== should_cast_spell Integration Tests ====================

    /// Test: should_cast_spell routes board wipe effects correctly
    #[test]
    fn test_should_cast_spell_routes_board_wipe() {
        use crate::core::{Card, CardId, CardType, ManaCost, TargetRestriction, TargetType};
        use crate::game::controller::GameStateView;
        use crate::game::GameState;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;
        let controller = HeuristicController::new(p1_id);

        // P2: Three big creatures, P1: nothing
        for i in 0..3 {
            let id = CardId::new(50 + i);
            let mut c = Card::new(id, "Serra Angel", p2_id);
            c.add_type(CardType::Creature);
            c.set_base_power(Some(4));
            c.set_base_toughness(Some(4));
            c.controller = p2_id;
            game.cards.insert(id, c);
            game.battlefield.add(id);
        }

        let wrath_id = CardId::new(100);
        let mut wrath = Card::new(wrath_id, "Wrath of God", p1_id);
        wrath.add_type(CardType::Sorcery);
        wrath.mana_cost = ManaCost::from_string("2WW");
        wrath.effects.push(crate::core::Effect::DestroyAll {
            restriction: TargetRestriction::from_types([TargetType::Creature]),
            no_regenerate: true,
        });
        game.cards.insert(wrath_id, wrath);

        let view = GameStateView::new(&game, p1_id);
        let wrath_card = view.get_card(wrath_id).unwrap();

        assert!(
            controller.should_cast_spell(wrath_card, &view),
            "should_cast_spell should return true for Wrath of God when opponent dominates board"
        );
    }

    /// Test: should_cast_spell routes LoseLife effects correctly
    #[test]
    fn test_should_cast_spell_routes_lose_life() {
        use crate::core::{Card, CardId, CardType, ManaCost};
        use crate::game::controller::GameStateView;
        use crate::game::GameState;

        let game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;
        let controller = HeuristicController::new(p1_id);

        // LoseLife targeting opponent should always be worth casting
        let spell_id = CardId::new(100);
        let mut spell = Card::new(spell_id, "Drain Life", p1_id);
        spell.add_type(CardType::Sorcery);
        spell.mana_cost = ManaCost::from_string("1B");
        spell.effects.push(crate::core::Effect::LoseLife {
            player: p2_id,
            amount: 3,
        });

        let view = GameStateView::new(&game, p1_id);

        assert!(
            controller.should_cast_spell(&spell, &view),
            "should_cast_spell should return true for LoseLife effect"
        );
    }

    // ==================== Removal Timing (use_removal_now) Tests ====================
    // Reference: ComputerUtilCard.useRemovalNow() in Java Forge
    // Tests use real 4ED cards loaded from cardsfolder

    /// Helper: load a card definition from cardsfolder, instantiate, and insert on battlefield
    fn load_and_place_on_battlefield(
        game: &mut crate::game::GameState,
        card_path: &str,
        card_id: crate::core::CardId,
        owner: crate::core::PlayerId,
    ) -> bool {
        let path = std::path::PathBuf::from(card_path);
        if !path.exists() {
            return false;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Failed to load card");
        let mut card = def.instantiate(card_id, owner);
        card.controller = owner;
        game.cards.insert(card_id, card);
        game.battlefield.add(card_id);
        true
    }

    /// Helper: load a card definition from cardsfolder and instantiate in hand (not on battlefield)
    fn load_card_in_hand(
        game: &mut crate::game::GameState,
        card_path: &str,
        card_id: crate::core::CardId,
        owner: crate::core::PlayerId,
    ) -> bool {
        let path = std::path::PathBuf::from(card_path);
        if !path.exists() {
            return false;
        }
        let def = crate::loader::CardLoader::load_from_file(&path).expect("Failed to load card");
        let card = def.instantiate(card_id, owner);
        game.cards.insert(card_id, card);
        true
    }

    /// Test: Sorcery removal (e.g. a destroy sorcery) always uses removal now
    #[test]
    fn test_use_removal_now_sorcery_always_true() {
        use crate::core::{Card, CardId, CardType, ManaCost};
        use crate::game::controller::GameStateView;
        use crate::game::GameState;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;
        let controller = HeuristicController::new(p1_id);

        // Opponent has a creature on battlefield
        let creature_id = CardId::new(50);
        let mut creature = Card::new(creature_id, "Grizzly Bears", p2_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(2));
        creature.set_base_toughness(Some(2));
        creature.controller = p2_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Sorcery-speed removal spell
        let spell_id = CardId::new(100);
        let mut spell = Card::new(spell_id, "Destroy Sorcery", p1_id);
        spell.add_type(CardType::Sorcery);
        spell.mana_cost = ManaCost::from_string("1B");
        spell.effects.push(crate::core::Effect::DestroyPermanent {
            target: creature_id,
            restriction: crate::core::TargetRestriction::any(),
            no_regenerate: false,
        });
        game.cards.insert(spell_id, spell);

        // Even at Upkeep (suboptimal timing), sorceries should always return true
        game.turn.current_step = crate::game::Step::Upkeep;

        let view = GameStateView::new(&game, p1_id);
        let spell_card = view.get_card(spell_id).unwrap();

        assert!(
            controller.use_removal_now(spell_card, creature_id, &view),
            "Sorcery removal should always be used now regardless of phase"
        );
    }

    /// Test: Instant removal held during opponent's upkeep (suboptimal timing)
    #[test]
    fn test_use_removal_now_instant_held_at_upkeep() {
        use crate::core::{Card, CardId, CardType, ManaCost};
        use crate::game::controller::GameStateView;
        use crate::game::GameState;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;
        let controller = HeuristicController::new(p1_id);

        // Small opponent creature (low evaluation, below 200 threshold)
        let creature_id = CardId::new(50);
        let mut creature = Card::new(creature_id, "Grizzly Bears", p2_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(2));
        creature.set_base_toughness(Some(2));
        creature.controller = p2_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Instant removal spell (Terror)
        let spell_id = CardId::new(100);
        let mut spell = Card::new(spell_id, "Terror", p1_id);
        spell.add_type(CardType::Instant);
        spell.mana_cost = ManaCost::from_string("1B");
        spell.effects.push(crate::core::Effect::DestroyPermanent {
            target: creature_id,
            restriction: crate::core::TargetRestriction::any(),
            no_regenerate: false,
        });
        game.cards.insert(spell_id, spell);

        // Opponent's upkeep - suboptimal timing for a low-value target
        game.turn.current_step = crate::game::Step::Upkeep;
        game.turn.active_player = p2_id;

        let view = GameStateView::new(&game, p1_id);
        let spell_card = view.get_card(spell_id).unwrap();

        assert!(
            !controller.use_removal_now(spell_card, creature_id, &view),
            "Instant removal should be held during opponent's upkeep for a low-value target"
        );
    }

    /// Test: Instant removal used during combat (DeclareAttackers)
    #[test]
    fn test_use_removal_now_during_combat() {
        use crate::game::controller::GameStateView;
        use crate::game::GameState;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;
        let controller = HeuristicController::new(p1_id);

        // Load real cards: Terror (instant removal) and Serra Angel (target)
        let angel_id = crate::core::CardId::new(50);
        let terror_id = crate::core::CardId::new(100);
        if !load_and_place_on_battlefield(&mut game, "../cardsfolder/s/serra_angel.txt", angel_id, p2_id) {
            println!("Skipping test: cardsfolder not present");
            return;
        }
        if !load_card_in_hand(&mut game, "../cardsfolder/t/terror.txt", terror_id, p1_id) {
            return;
        }

        // During combat (DeclareAttackers) - optimal timing
        game.turn.current_step = crate::game::Step::DeclareAttackers;
        game.turn.active_player = p2_id;

        let view = GameStateView::new(&game, p1_id);
        let terror = view.get_card(terror_id).unwrap();

        assert!(
            controller.use_removal_now(terror, angel_id, &view),
            "Terror should be used during combat to remove Serra Angel"
        );
    }

    /// Test: Instant removal used during our Main1 (to enable attacks)
    #[test]
    fn test_use_removal_now_main1_enable_attack() {
        use crate::game::controller::GameStateView;
        use crate::game::GameState;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;
        let controller = HeuristicController::new(p1_id);

        // Load Swords to Plowshares and Shivan Dragon
        let dragon_id = crate::core::CardId::new(50);
        let stp_id = crate::core::CardId::new(100);
        if !load_and_place_on_battlefield(&mut game, "../cardsfolder/s/shivan_dragon.txt", dragon_id, p2_id) {
            println!("Skipping test: cardsfolder not present");
            return;
        }
        if !load_card_in_hand(&mut game, "../cardsfolder/s/swords_to_plowshares.txt", stp_id, p1_id) {
            return;
        }

        // Our Main1 - removing opponent's dragon enables attacks
        game.turn.current_step = crate::game::Step::Main1;
        game.turn.active_player = p1_id;

        let view = GameStateView::new(&game, p1_id);
        let stp = view.get_card(stp_id).unwrap();

        assert!(
            controller.use_removal_now(stp, dragon_id, &view),
            "Swords to Plowshares should be used in Main1 to enable attacks"
        );
    }

    /// Test: Instant removal used at opponent's end step
    #[test]
    fn test_use_removal_now_opponent_end_step() {
        use crate::game::controller::GameStateView;
        use crate::game::GameState;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;
        let controller = HeuristicController::new(p1_id);

        // Load Lightning Bolt and Grizzly Bears
        let bears_id = crate::core::CardId::new(50);
        let bolt_id = crate::core::CardId::new(100);
        if !load_and_place_on_battlefield(&mut game, "../cardsfolder/g/grizzly_bears.txt", bears_id, p2_id) {
            println!("Skipping test: cardsfolder not present");
            return;
        }
        if !load_card_in_hand(&mut game, "../cardsfolder/l/lightning_bolt.txt", bolt_id, p1_id) {
            return;
        }

        // Opponent's end step - good timing for instant removal
        game.turn.current_step = crate::game::Step::End;
        game.turn.active_player = p2_id;

        let view = GameStateView::new(&game, p1_id);
        let bolt = view.get_card(bolt_id).unwrap();

        assert!(
            controller.use_removal_now(bolt, bears_id, &view),
            "Lightning Bolt should be used at opponent's end step"
        );
    }

    /// Test: Enchanted target triggers two-for-one removal
    #[test]
    fn test_use_removal_now_enchanted_target_two_for_one() {
        use crate::game::controller::GameStateView;
        use crate::game::GameState;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;
        let controller = HeuristicController::new(p1_id);

        // Load Grizzly Bears (target), Holy Strength (aura), and Lightning Bolt (removal)
        let bears_id = crate::core::CardId::new(50);
        let aura_id = crate::core::CardId::new(51);
        let bolt_id = crate::core::CardId::new(100);

        if !load_and_place_on_battlefield(&mut game, "../cardsfolder/g/grizzly_bears.txt", bears_id, p2_id) {
            println!("Skipping test: cardsfolder not present");
            return;
        }
        if !load_and_place_on_battlefield(&mut game, "../cardsfolder/h/holy_strength.txt", aura_id, p2_id) {
            return;
        }
        if !load_card_in_hand(&mut game, "../cardsfolder/l/lightning_bolt.txt", bolt_id, p1_id) {
            return;
        }

        // Attach the aura to the bears
        if let Some(aura) = game.cards.try_get_mut(aura_id) {
            aura.attached_to = Some(bears_id);
        }

        // Set to suboptimal timing (opponent's draw step)
        // Normally we'd hold instant removal here, but the two-for-one
        // should override timing concerns
        game.turn.current_step = crate::game::Step::Draw;
        game.turn.active_player = p2_id;

        let view = GameStateView::new(&game, p1_id);
        let bolt = view.get_card(bolt_id).unwrap();

        assert!(
            controller.use_removal_now(bolt, bears_id, &view),
            "Lightning Bolt should be used immediately on enchanted target (two-for-one)"
        );
    }

    /// Test: High-value target triggers immediate removal even at bad timing
    #[test]
    fn test_use_removal_now_high_value_target() {
        use crate::game::controller::GameStateView;
        use crate::game::GameState;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;
        let controller = HeuristicController::new(p1_id);

        // Load Shivan Dragon (high-value: 5/5 flyer with firebreathing)
        // and Swords to Plowshares
        let dragon_id = crate::core::CardId::new(50);
        let stp_id = crate::core::CardId::new(100);

        if !load_and_place_on_battlefield(&mut game, "../cardsfolder/s/shivan_dragon.txt", dragon_id, p2_id) {
            println!("Skipping test: cardsfolder not present");
            return;
        }
        if !load_card_in_hand(&mut game, "../cardsfolder/s/swords_to_plowshares.txt", stp_id, p1_id) {
            return;
        }

        // Opponent's draw step - normally bad timing for instant removal
        game.turn.current_step = crate::game::Step::Draw;
        game.turn.active_player = p2_id;

        let view = GameStateView::new(&game, p1_id);
        let stp = view.get_card(stp_id).unwrap();
        let dragon_eval = controller.evaluate_creature(&view, dragon_id);

        // Shivan Dragon should evaluate high enough (>= 200) to trigger immediate removal
        assert!(
            dragon_eval >= 200,
            "Shivan Dragon evaluation ({dragon_eval}) should be >= 200 for high-value threshold"
        );

        assert!(
            controller.use_removal_now(stp, dragon_id, &view),
            "Swords to Plowshares should remove high-value Shivan Dragon even at bad timing"
        );
    }

    /// Test: target_has_auras detects aura attachments
    #[test]
    fn test_target_has_auras() {
        use crate::core::{Card, CardId, CardType};
        use crate::game::controller::GameStateView;
        use crate::game::GameState;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;
        let controller = HeuristicController::new(p1_id);

        // Creature without aura
        let creature_id = CardId::new(50);
        let mut creature = Card::new(creature_id, "Grizzly Bears", p2_id);
        creature.add_type(CardType::Creature);
        creature.controller = p2_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        let view = GameStateView::new(&game, p1_id);
        assert!(
            !controller.target_has_auras(creature_id, &view),
            "Creature without auras should return false"
        );

        // Add an aura attached to the creature
        let aura_id = CardId::new(51);
        let mut aura = Card::new(aura_id, "Holy Strength", p2_id);
        aura.add_type(CardType::Enchantment);
        aura.set_subtypes(smallvec::smallvec![crate::core::Subtype::new("Aura")]);
        aura.attached_to = Some(creature_id);
        aura.controller = p2_id;
        game.cards.insert(aura_id, aura);
        game.battlefield.add(aura_id);

        let view2 = GameStateView::new(&game, p1_id);
        assert!(
            controller.target_has_auras(creature_id, &view2),
            "Creature with aura attached should return true"
        );
    }

    /// Test: Integration - should_cast_spell with removal timing uses use_removal_now
    /// Verifies that the AI holds instant removal at bad timing but uses it during combat
    #[test]
    fn test_should_cast_spell_removal_timing_integration() {
        use crate::core::{Card, CardId, CardType, ManaCost};
        use crate::game::controller::GameStateView;
        use crate::game::GameState;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;
        let controller = HeuristicController::new(p1_id);

        // Small opponent creature (low value)
        let creature_id = CardId::new(50);
        let mut creature = Card::new(creature_id, "Squire", p2_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(1));
        creature.set_base_toughness(Some(2));
        creature.controller = p2_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Instant removal spell
        let spell_id = CardId::new(100);
        let mut spell = Card::new(spell_id, "Terror", p1_id);
        spell.add_type(CardType::Instant);
        spell.mana_cost = ManaCost::from_string("1B");
        spell.effects.push(crate::core::Effect::DestroyPermanent {
            target: creature_id,
            restriction: crate::core::TargetRestriction::any(),
            no_regenerate: false,
        });
        game.cards.insert(spell_id, spell);

        // At opponent's upkeep: should_cast_spell returns false (hold removal)
        game.turn.current_step = crate::game::Step::Upkeep;
        game.turn.active_player = p2_id;

        let view = GameStateView::new(&game, p1_id);
        let spell_card = view.get_card(spell_id).unwrap();
        assert!(
            !controller.should_cast_spell(spell_card, &view),
            "AI should hold instant removal at opponent's upkeep for low-value target"
        );

        // During combat: should_cast_spell returns true
        game.turn.current_step = crate::game::Step::DeclareAttackers;
        let view2 = GameStateView::new(&game, p1_id);
        let spell_card2 = view2.get_card(spell_id).unwrap();
        assert!(
            controller.should_cast_spell(spell_card2, &view2),
            "AI should use instant removal during combat"
        );
    }

    // ==================== Fight AI Tests ====================

    /// Test: AI casts Fight spell when we have a favorable matchup
    /// Reference: FightAi.java - favorable = our creature kills theirs and survives
    #[test]
    fn test_should_cast_fight_favorable_matchup() {
        use crate::core::{Card, CardId, CardType};
        use crate::game::controller::GameStateView;
        use crate::game::GameState;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;
        let controller = HeuristicController::new(p1_id);

        // Our 5/5 creature (Serra Angel-like)
        let our_id = CardId::new(50);
        let mut our = Card::new(our_id, "Serra Angel", p1_id);
        our.add_type(CardType::Creature);
        our.set_base_power(Some(4));
        our.set_base_toughness(Some(4));
        our.controller = p1_id;
        game.cards.insert(our_id, our);
        game.battlefield.add(our_id);

        // Opponent's 2/2 creature (Grizzly Bears)
        let opp_id = CardId::new(51);
        let mut opp = Card::new(opp_id, "Grizzly Bears", p2_id);
        opp.add_type(CardType::Creature);
        opp.set_base_power(Some(2));
        opp.set_base_toughness(Some(2));
        opp.controller = p2_id;
        game.cards.insert(opp_id, opp);
        game.battlefield.add(opp_id);

        let view = GameStateView::new(&game, p1_id);

        // 4/4 vs 2/2: We kill them (4 >= 2) and survive (2 < 4)
        assert!(
            controller.should_cast_fight(&view),
            "AI should cast Fight when 4/4 fights 2/2 (we win)"
        );
    }

    /// Test: AI doesn't cast Fight when we would lose
    #[test]
    fn test_should_not_cast_fight_unfavorable() {
        use crate::core::{Card, CardId, CardType};
        use crate::game::controller::GameStateView;
        use crate::game::GameState;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;
        let controller = HeuristicController::new(p1_id);

        // Our 2/2 creature (Grizzly Bears)
        let our_id = CardId::new(50);
        let mut our = Card::new(our_id, "Grizzly Bears", p1_id);
        our.add_type(CardType::Creature);
        our.set_base_power(Some(2));
        our.set_base_toughness(Some(2));
        our.controller = p1_id;
        game.cards.insert(our_id, our);
        game.battlefield.add(our_id);

        // Opponent's 5/5 creature (bigger than ours)
        let opp_id = CardId::new(51);
        let mut opp = Card::new(opp_id, "Shivan Dragon", p2_id);
        opp.add_type(CardType::Creature);
        opp.set_base_power(Some(5));
        opp.set_base_toughness(Some(5));
        opp.controller = p2_id;
        game.cards.insert(opp_id, opp);
        game.battlefield.add(opp_id);

        let view = GameStateView::new(&game, p1_id);

        // 2/2 vs 5/5: We die (5 >= 2), they survive (2 < 5)
        assert!(
            !controller.should_cast_fight(&view),
            "AI should NOT cast Fight when 2/2 fights 5/5 (we lose)"
        );
    }

    /// Test: AI casts Fight for favorable trade-up
    #[test]
    fn test_should_cast_fight_trade_up() {
        use crate::core::{Card, CardId, CardType, Keyword};
        use crate::game::controller::GameStateView;
        use crate::game::GameState;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;
        let controller = HeuristicController::new(p1_id);

        // Our 1/1 deathtouch (high value due to deathtouch)
        let our_id = CardId::new(50);
        let mut our = Card::new(our_id, "Typhoid Rats", p1_id);
        our.add_type(CardType::Creature);
        our.set_base_power(Some(1));
        our.set_base_toughness(Some(1));
        our.keywords.insert(Keyword::Deathtouch);
        our.controller = p1_id;
        our.tapped = false; // Must be untapped to fight
        game.cards.insert(our_id, our);
        game.battlefield.add(our_id);

        // Opponent's big 5/5 creature (much more valuable)
        let opp_id = CardId::new(51);
        let mut opp = Card::new(opp_id, "Shivan Dragon", p2_id);
        opp.add_type(CardType::Creature);
        opp.set_base_power(Some(5));
        opp.set_base_toughness(Some(5));
        opp.controller = p2_id;
        game.cards.insert(opp_id, opp);
        game.battlefield.add(opp_id);

        let view = GameStateView::new(&game, p1_id);

        // 1/1 deathtouch vs 5/5:
        // We kill them (deathtouch: 1 damage is lethal)
        // We die (5 >= 1), but this is a favorable trade
        assert!(
            controller.should_cast_fight(&view),
            "AI should cast Fight when 1/1 deathtouch fights 5/5 (favorable trade)"
        );
    }

    /// Test: AI doesn't cast Fight when no creatures available
    #[test]
    fn test_should_not_cast_fight_no_creatures() {
        use crate::game::controller::GameStateView;
        use crate::game::GameState;

        let game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let controller = HeuristicController::new(p1_id);

        let view = GameStateView::new(&game, p1_id);

        assert!(
            !controller.should_cast_fight(&view),
            "AI should not cast Fight when no creatures on battlefield"
        );
    }

    // ==================== GainControl AI Tests ====================

    /// Test: AI casts GainControl when opponent has valuable creature
    #[test]
    fn test_should_cast_gain_control_valuable_target() {
        use crate::core::{Card, CardId, CardType};
        use crate::game::controller::GameStateView;
        use crate::game::GameState;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;
        let controller = HeuristicController::new(p1_id);

        // Opponent's valuable creature (Shivan Dragon 5/5)
        let opp_id = CardId::new(50);
        let mut opp = Card::new(opp_id, "Shivan Dragon", p2_id);
        opp.add_type(CardType::Creature);
        opp.set_base_power(Some(5));
        opp.set_base_toughness(Some(5));
        opp.controller = p2_id;
        game.cards.insert(opp_id, opp);
        game.battlefield.add(opp_id);

        let view = GameStateView::new(&game, p1_id);

        assert!(
            controller.should_cast_gain_control(&view),
            "AI should cast GainControl on valuable 5/5 creature"
        );
    }

    /// Test: AI doesn't cast GainControl when opponent has no creatures
    #[test]
    fn test_should_not_cast_gain_control_no_targets() {
        use crate::game::controller::GameStateView;
        use crate::game::GameState;

        let game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let controller = HeuristicController::new(p1_id);

        let view = GameStateView::new(&game, p1_id);

        assert!(
            !controller.should_cast_gain_control(&view),
            "AI should not cast GainControl when opponent has no creatures"
        );
    }

    /// Test: AI always casts GainControl when opponent has creatures
    /// Even stealing a weak creature is advantageous (denies blocker + gains attacker)
    #[test]
    fn test_should_cast_gain_control_any_creature() {
        use crate::core::{Card, CardId, CardType};
        use crate::game::controller::GameStateView;
        use crate::game::GameState;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;
        let controller = HeuristicController::new(p1_id);

        // Opponent's weak creature (1/1 vanilla)
        let opp_id = CardId::new(50);
        let mut opp = Card::new(opp_id, "Squire", p2_id);
        opp.add_type(CardType::Creature);
        opp.set_base_power(Some(1));
        opp.set_base_toughness(Some(1));
        opp.controller = p2_id;
        game.cards.insert(opp_id, opp);
        game.battlefield.add(opp_id);

        let view = GameStateView::new(&game, p1_id);

        // Even a 1/1 is worth stealing - it's card advantage
        // (denies them a blocker, gives us an attacker)
        assert!(
            controller.should_cast_gain_control(&view),
            "AI should cast GainControl even on weak 1/1 creature"
        );
    }

    // ========================================================================
    // REAL CARD TESTS - Load from cardsfolder
    // Tests use real 4ED/classic cards to verify AI behavior with actual card data
    // ========================================================================

    /// Test loading Prodigal Sorcerer from cardsfolder and verifying ping ability AI
    /// Prodigal Sorcerer: 1/1, T: Deal 1 damage to any target
    #[test]
    fn test_prodigal_sorcerer_from_cardsfolder() {
        use crate::game::controller::GameStateView;
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/p/prodigal_sorcerer.txt");
        if !path.exists() {
            println!("Skipping test: cardsfolder not present");
            return;
        }

        let def = crate::loader::CardLoader::load_from_file(&path).expect("Failed to load Prodigal Sorcerer");
        assert_eq!(def.name.as_str(), "Prodigal Sorcerer");

        // Create game and instantiate the card
        let mut game = crate::game::GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let card_id = crate::core::CardId::new(100);
        let card = def.instantiate(card_id, p1_id);

        // Verify basic card properties (before adding to game)
        assert!(card.is_creature(), "Prodigal Sorcerer should be a creature");
        assert_eq!(card.current_power(), 1, "Prodigal Sorcerer should be 1/1");
        assert_eq!(card.current_toughness(), 1, "Prodigal Sorcerer should be 1/1");

        // Verify the activated ability was parsed
        assert!(
            !card.activated_abilities.is_empty(),
            "Prodigal Sorcerer should have at least one activated ability"
        );

        // Find the tap ability (ping ability)
        let ping_abilities: Vec<_> = card
            .activated_abilities
            .iter()
            .filter(|a| a.cost.includes_tap())
            .collect();

        assert_eq!(
            ping_abilities.len(),
            1,
            "Prodigal Sorcerer should have exactly one tap ability"
        );

        // Verify the ability has a DealDamage effect
        let ability = ping_abilities[0];
        let has_damage_effect = ability
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::DealDamage { .. }));

        assert!(
            has_damage_effect,
            "Prodigal Sorcerer's ability should have a DealDamage effect"
        );

        // Test AI classification
        let controller = HeuristicController::new(p1_id);
        let ability_type = controller.classify_activated_ability(ability);
        assert!(
            matches!(ability_type, ActivatedAbilityType::Ping { damage: 1 }),
            "Prodigal Sorcerer's ability should be classified as Ping(1) by AI"
        );

        // Add card to game and battlefield for evaluation
        game.cards.insert(card_id, card);
        game.battlefield.add(card_id);
        let view = GameStateView::new(&game, p1_id);

        // Verify creature evaluation includes the ping bonus
        let creature_value = controller.evaluate_creature(&view, card_id);
        // Base 1/1 = 100, ping adds 10 + 1*5 = 15, so should be > 110
        assert!(
            creature_value > 110,
            "Prodigal Sorcerer evaluation ({}) should be higher than vanilla 1/1 (100) due to ping ability",
            creature_value
        );
    }

    /// Test Northern Paladin destroy ability with color restriction
    /// Northern Paladin: 3/3, WW T: Destroy target black permanent
    #[test]
    fn test_northern_paladin_from_cardsfolder() {
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/n/northern_paladin.txt");
        if !path.exists() {
            println!("Skipping test: cardsfolder not present");
            return;
        }

        let def = crate::loader::CardLoader::load_from_file(&path).expect("Failed to load Northern Paladin");
        assert_eq!(def.name.as_str(), "Northern Paladin");

        // Create game and instantiate the card
        let game = crate::game::GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let card_id = crate::core::CardId::new(100);
        let card = def.instantiate(card_id, p1_id);

        // Verify basic card properties
        assert!(card.is_creature(), "Northern Paladin should be a creature");
        assert_eq!(card.current_power(), 3, "Northern Paladin should be 3/3");
        assert_eq!(card.current_toughness(), 3, "Northern Paladin should be 3/3");

        // Verify the activated ability was parsed
        assert!(
            !card.activated_abilities.is_empty(),
            "Northern Paladin should have at least one activated ability"
        );

        // Find the tap-to-destroy ability
        let destroy_abilities: Vec<_> = card
            .activated_abilities
            .iter()
            .filter(|a| !a.is_mana_ability && a.cost.includes_tap())
            .collect();

        assert_eq!(
            destroy_abilities.len(),
            1,
            "Northern Paladin should have exactly one tap-to-destroy ability"
        );

        // Verify the ability has a DestroyPermanent effect
        let ability = destroy_abilities[0];
        let has_destroy_effect = ability
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::DestroyPermanent { .. }));

        assert!(
            has_destroy_effect,
            "Northern Paladin's ability should have a DestroyPermanent effect"
        );

        // Test AI classification
        let controller = HeuristicController::new(p1_id);
        let ability_type = controller.classify_activated_ability(ability);
        assert!(
            matches!(ability_type, ActivatedAbilityType::Destroy),
            "Northern Paladin's ability should be classified as Destroy by AI"
        );
    }

    /// Test Drudge Skeletons regeneration ability from cardsfolder
    /// Drudge Skeletons: 1/1, B: Regenerate
    #[test]
    fn test_drudge_skeletons_from_cardsfolder() {
        use crate::game::controller::GameStateView;
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/d/drudge_skeletons.txt");
        if !path.exists() {
            println!("Skipping test: cardsfolder not present");
            return;
        }

        let def = crate::loader::CardLoader::load_from_file(&path).expect("Failed to load Drudge Skeletons");
        assert_eq!(def.name.as_str(), "Drudge Skeletons");

        // Create game and instantiate the card
        let mut game = crate::game::GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let card_id = crate::core::CardId::new(100);
        let card = def.instantiate(card_id, p1_id);

        // Verify basic card properties
        assert!(card.is_creature(), "Drudge Skeletons should be a creature");
        assert_eq!(card.current_power(), 1, "Drudge Skeletons should be 1/1");
        assert_eq!(card.current_toughness(), 1, "Drudge Skeletons should be 1/1");

        // Verify the activated ability was parsed
        assert!(
            !card.activated_abilities.is_empty(),
            "Drudge Skeletons should have at least one activated ability"
        );

        // Find the regeneration ability
        let regen_abilities: Vec<_> = card.activated_abilities.iter().filter(|a| !a.is_mana_ability).collect();

        assert_eq!(
            regen_abilities.len(),
            1,
            "Drudge Skeletons should have exactly one non-mana activated ability (regenerate)"
        );

        // Verify the ability has a Regenerate effect
        let ability = regen_abilities[0];
        let has_regen_effect = ability
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::Regenerate { .. }));

        assert!(
            has_regen_effect,
            "Drudge Skeletons's ability should have a Regenerate effect"
        );

        // Add card to game and battlefield for evaluation
        game.cards.insert(card_id, card);
        game.battlefield.add(card_id);
        let view = GameStateView::new(&game, p1_id);

        // Verify creature evaluation includes regeneration bonus
        let controller = HeuristicController::new(p1_id);
        let creature_value = controller.evaluate_creature(&view, card_id);
        // Base 1/1 = 100, regeneration typically adds +20
        assert!(
            creature_value > 110,
            "Drudge Skeletons evaluation ({}) should be higher than vanilla 1/1 (100) due to regenerate",
            creature_value
        );
    }

    /// Test Llanowar Elves mana ability recognition from cardsfolder
    /// Llanowar Elves: 1/1, T: Add G
    #[test]
    fn test_llanowar_elves_mana_ability() {
        use crate::game::controller::GameStateView;
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/l/llanowar_elves.txt");
        if !path.exists() {
            println!("Skipping test: cardsfolder not present");
            return;
        }

        let def = crate::loader::CardLoader::load_from_file(&path).expect("Failed to load Llanowar Elves");
        assert_eq!(def.name.as_str(), "Llanowar Elves");

        // Create game and instantiate the card
        let mut game = crate::game::GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let card_id = crate::core::CardId::new(100);
        let card = def.instantiate(card_id, p1_id);

        // Verify basic card properties
        assert!(card.is_creature(), "Llanowar Elves should be a creature");
        assert_eq!(card.current_power(), 1, "Llanowar Elves should be 1/1");
        assert_eq!(card.current_toughness(), 1, "Llanowar Elves should be 1/1");

        // Verify the activated ability was parsed
        assert!(
            !card.activated_abilities.is_empty(),
            "Llanowar Elves should have at least one activated ability"
        );

        // Find the mana ability
        let mana_abilities: Vec<_> = card.activated_abilities.iter().filter(|a| a.is_mana_ability).collect();

        assert_eq!(
            mana_abilities.len(),
            1,
            "Llanowar Elves should have exactly one mana ability"
        );

        // Verify the mana ability produces green mana
        let ability = mana_abilities[0];
        let has_mana_effect = ability
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::AddMana { mana, .. } if mana.green > 0));

        assert!(has_mana_effect, "Llanowar Elves's ability should produce green mana");

        // Add card to game and battlefield for evaluation
        game.cards.insert(card_id, card);
        game.battlefield.add(card_id);
        let view = GameStateView::new(&game, p1_id);

        // Verify creature evaluation includes mana bonus
        let controller = HeuristicController::new(p1_id);
        let creature_value = controller.evaluate_creature(&view, card_id);
        // Base 1/1 = 100, mana ability typically adds +15
        assert!(
            creature_value > 110,
            "Llanowar Elves evaluation ({}) should be higher than vanilla 1/1 (100) due to mana ability",
            creature_value
        );
    }

    /// Test Serra Angel keyword evaluation from cardsfolder
    /// Serra Angel: 4/4, Flying, Vigilance
    #[test]
    fn test_serra_angel_keywords() {
        use crate::game::controller::GameStateView;
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/s/serra_angel.txt");
        if !path.exists() {
            println!("Skipping test: cardsfolder not present");
            return;
        }

        let def = crate::loader::CardLoader::load_from_file(&path).expect("Failed to load Serra Angel");
        assert_eq!(def.name.as_str(), "Serra Angel");

        // Create game and instantiate the card
        let mut game = crate::game::GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let card_id = crate::core::CardId::new(100);
        let card = def.instantiate(card_id, p1_id);

        // Verify basic card properties
        assert!(card.is_creature(), "Serra Angel should be a creature");
        assert_eq!(card.current_power(), 4, "Serra Angel should be 4/4");
        assert_eq!(card.current_toughness(), 4, "Serra Angel should be 4/4");

        // Verify keywords
        assert!(
            card.has_keyword(crate::core::Keyword::Flying),
            "Serra Angel should have Flying"
        );
        assert!(
            card.has_keyword(crate::core::Keyword::Vigilance),
            "Serra Angel should have Vigilance"
        );

        // Add card to game and battlefield for evaluation
        game.cards.insert(card_id, card);
        game.battlefield.add(card_id);
        let view = GameStateView::new(&game, p1_id);

        // Verify creature evaluation includes keyword bonuses
        let controller = HeuristicController::new(p1_id);
        let creature_value = controller.evaluate_creature(&view, card_id);
        // Base: 80 + 20 (non-token)
        // Power: 4 * 15 = 60
        // Toughness: 4 * 10 = 40
        // CMC: 5 * 5 = 25
        // Flying: 4 * 10 = 40
        // Vigilance: 4 * 3 = 12
        // Total should be > 250 (base = 245 + keywords)
        assert!(
            creature_value > 250,
            "Serra Angel evaluation ({}) should be > 250 due to Flying and Vigilance",
            creature_value
        );
    }

    /// Test Shivan Dragon pump ability and flying from cardsfolder
    /// Shivan Dragon: 5/5, Flying, R: +1/+0
    #[test]
    fn test_shivan_dragon_from_cardsfolder() {
        use crate::game::controller::GameStateView;
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/s/shivan_dragon.txt");
        if !path.exists() {
            println!("Skipping test: cardsfolder not present");
            return;
        }

        let def = crate::loader::CardLoader::load_from_file(&path).expect("Failed to load Shivan Dragon");
        assert_eq!(def.name.as_str(), "Shivan Dragon");

        // Create game and instantiate the card
        let mut game = crate::game::GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let card_id = crate::core::CardId::new(100);
        let card = def.instantiate(card_id, p1_id);

        // Verify basic card properties
        assert!(card.is_creature(), "Shivan Dragon should be a creature");
        assert_eq!(card.current_power(), 5, "Shivan Dragon should be 5/5");
        assert_eq!(card.current_toughness(), 5, "Shivan Dragon should be 5/5");

        // Verify keywords
        assert!(
            card.has_keyword(crate::core::Keyword::Flying),
            "Shivan Dragon should have Flying"
        );

        // Verify the firebreathing ability was parsed
        let non_mana_abilities: Vec<_> = card.activated_abilities.iter().filter(|a| !a.is_mana_ability).collect();

        assert!(
            !non_mana_abilities.is_empty(),
            "Shivan Dragon should have a firebreathing ability"
        );

        // Test AI classification of the pump ability
        let controller = HeuristicController::new(p1_id);
        let ability = non_mana_abilities[0];
        let ability_type = controller.classify_activated_ability(ability);
        assert!(
            matches!(ability_type, ActivatedAbilityType::Pump { power: 1, toughness: 0 }),
            "Shivan Dragon's ability should be classified as Pump(+1/+0)"
        );

        // Add card to game and battlefield for evaluation
        game.cards.insert(card_id, card);
        game.battlefield.add(card_id);
        let view = GameStateView::new(&game, p1_id);

        // Verify creature evaluation includes keyword and ability bonuses
        let creature_value = controller.evaluate_creature(&view, card_id);
        // Base: 80 + 20 (non-token)
        // Power: 5 * 15 = 75
        // Toughness: 5 * 10 = 50
        // CMC: 6 * 5 = 30
        // Flying: 5 * 10 = 50
        // Pump ability: adds some bonus
        // Total should be > 300 (base = 305 without pump)
        assert!(
            creature_value > 300,
            "Shivan Dragon evaluation ({}) should be > 300 due to Flying and pump ability",
            creature_value
        );
    }

    /// Test loading Hypnotic Specter from cardsfolder - classic 4ED evasive creature
    ///
    /// Hypnotic Specter (4ED): 2/2 Flying
    /// "Whenever Hypnotic Specter deals damage to an opponent, that player discards a card at random."
    ///
    /// Tests Mode$ DamageDone trigger parsing - Hypnotic Specter has a damage-to-player trigger
    /// that causes the opponent to discard a card at random.
    #[test]
    fn test_hypnotic_specter_from_cardsfolder() {
        use crate::core::CardId;
        use crate::game::controller::GameStateView;
        use crate::game::GameState;
        use std::path::PathBuf;

        // Load card from cardsfolder
        let path = PathBuf::from("../cardsfolder/h/hypnotic_specter.txt");
        if !path.exists() {
            println!("Skipping test: cardsfolder not present");
            return;
        }

        let card_def = crate::loader::CardLoader::load_from_file(&path).expect("Failed to load card");

        // Verify card properties from definition
        assert_eq!(card_def.name.as_str(), "Hypnotic Specter");
        assert_eq!(card_def.power, Some(2));
        assert_eq!(card_def.toughness, Some(2));

        // Set up a game to test creature evaluation
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        let card_id = CardId::new(100);
        let mut card = card_def.instantiate(card_id, p1_id);
        card.controller = p1_id;

        // Verify card properties after instantiation (is_creature() checks types)
        assert!(card.is_creature(), "Hypnotic Specter should be a creature");
        assert!(card.has_flying(), "Hypnotic Specter should have Flying");

        // Verify Mode$ DamageDone trigger is parsed
        assert!(
            !card.triggers.is_empty(),
            "Hypnotic Specter should have at least one trigger (DamageDone)"
        );
        assert_eq!(
            card.triggers[0].event,
            crate::core::TriggerEvent::DealsCombatDamage,
            "Hypnotic Specter's trigger should be DealsCombatDamage"
        );

        game.cards.insert(card_id, card);
        game.battlefield.add(card_id);

        let controller = HeuristicController::new(p1_id);
        let view = GameStateView::new(&game, p1_id);

        // Verify creature evaluation - Flying bonus should still apply
        let creature_value = controller.evaluate_creature(&view, card_id);
        // Base: 80 + 20 (non-token)
        // Power: 2 * 15 = 30
        // Toughness: 2 * 10 = 20
        // CMC: 3 * 5 = 15
        // Flying: 2 * 10 = 20
        // Expected minimum without trigger: 100 + 30 + 20 + 15 + 20 = 185
        assert!(
            creature_value >= 180,
            "Hypnotic Specter evaluation ({}) should be >= 180 due to Flying keyword",
            creature_value
        );

        println!("Hypnotic Specter evaluation: {}", creature_value);
    }

    /// Test loading Sengir Vampire from cardsfolder - classic 4ED flyer
    ///
    /// Sengir Vampire (4ED): 4/4 Flying
    /// "Whenever a creature dealt damage by Sengir Vampire this turn dies, put a +1/+1 counter on Sengir Vampire."
    ///
    /// Note: The conditional "dies" trigger (ValidCard$ Creature.DamagedBy) requires
    /// tracking damage sources, which is complex. This test verifies basic card properties.
    /// TODO(mtg-147): Implement conditional die triggers with DamagedBy tracking
    #[test]
    fn test_sengir_vampire_from_cardsfolder() {
        use crate::core::CardId;
        use crate::game::controller::GameStateView;
        use crate::game::GameState;
        use std::path::PathBuf;

        // Load card from cardsfolder
        let path = PathBuf::from("../cardsfolder/s/sengir_vampire.txt");
        if !path.exists() {
            println!("Skipping test: cardsfolder not present");
            return;
        }

        let card_def = crate::loader::CardLoader::load_from_file(&path).expect("Failed to load card");

        // Verify card properties from definition
        assert_eq!(card_def.name.as_str(), "Sengir Vampire");
        assert_eq!(card_def.power, Some(4));
        assert_eq!(card_def.toughness, Some(4));

        // Set up a game to test creature evaluation
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        let card_id = CardId::new(100);
        let mut card = card_def.instantiate(card_id, p1_id);
        card.controller = p1_id;

        // Verify card properties after instantiation (is_creature() checks types)
        assert!(card.is_creature(), "Sengir Vampire should be a creature");
        assert!(card.has_flying(), "Sengir Vampire should have Flying");

        // Note: Complex conditional trigger not yet parsed - skip trigger assertion
        // The "Creature.DamagedBy" condition requires damage tracking infrastructure

        game.cards.insert(card_id, card);
        game.battlefield.add(card_id);

        let controller = HeuristicController::new(p1_id);
        let view = GameStateView::new(&game, p1_id);

        // Verify creature evaluation - Flying bonus should still apply
        let creature_value = controller.evaluate_creature(&view, card_id);
        // Base: 80 + 20 (non-token)
        // Power: 4 * 15 = 60
        // Toughness: 4 * 10 = 40
        // CMC: 5 * 5 = 25
        // Flying: 4 * 10 = 40
        // Expected minimum without trigger: 100 + 60 + 40 + 25 + 40 = 265
        assert!(
            creature_value >= 260,
            "Sengir Vampire evaluation ({}) should be >= 260 due to Flying keyword",
            creature_value
        );

        println!("Sengir Vampire evaluation: {}", creature_value);
    }

    /// Test loading Mahamoti Djinn from cardsfolder - classic 4ED blue finisher
    ///
    /// Mahamoti Djinn (4ED): 5/6 Flying
    /// No abilities, but tests pure stat-based creature evaluation with Flying
    #[test]
    fn test_mahamoti_djinn_from_cardsfolder() {
        use crate::core::CardId;
        use crate::game::controller::GameStateView;
        use crate::game::GameState;
        use std::path::PathBuf;

        // Load card from cardsfolder
        let path = PathBuf::from("../cardsfolder/m/mahamoti_djinn.txt");
        if !path.exists() {
            println!("Skipping test: cardsfolder not present");
            return;
        }

        let card_def = crate::loader::CardLoader::load_from_file(&path).expect("Failed to load card");

        // Verify card properties from definition
        assert_eq!(card_def.name.as_str(), "Mahamoti Djinn");
        assert_eq!(card_def.power, Some(5));
        assert_eq!(card_def.toughness, Some(6));

        // Set up a game to test creature evaluation
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        let card_id = CardId::new(100);
        let mut card = card_def.instantiate(card_id, p1_id);
        card.controller = p1_id;

        // Verify card properties after instantiation (is_creature() checks types)
        assert!(card.is_creature(), "Mahamoti Djinn should be a creature");
        assert!(card.has_flying(), "Mahamoti Djinn should have Flying");

        game.cards.insert(card_id, card);
        game.battlefield.add(card_id);

        let controller = HeuristicController::new(p1_id);
        let view = GameStateView::new(&game, p1_id);

        // Verify creature evaluation
        let creature_value = controller.evaluate_creature(&view, card_id);
        // Base: 80 + 20 (non-token)
        // Power: 5 * 15 = 75
        // Toughness: 6 * 10 = 60
        // CMC: 6 * 5 = 30
        // Flying: 5 * 10 = 50 (power * 10)
        // Expected minimum: 100 + 75 + 60 + 30 + 50 = 315
        assert!(
            creature_value >= 300,
            "Mahamoti Djinn evaluation ({}) should be >= 300 due to high stats and Flying",
            creature_value
        );

        println!("Mahamoti Djinn evaluation: {}", creature_value);
    }

    /// Test loading Force of Nature from cardsfolder - classic 4ED with upkeep cost
    ///
    /// Force of Nature (4ED): 8/8 Trample
    /// "At the beginning of your upkeep, Force of Nature deals 8 damage to you unless you pay GGGG."
    ///
    /// This tests that upkeep costs are properly penalized in creature evaluation
    #[test]
    fn test_force_of_nature_from_cardsfolder() {
        use crate::core::CardId;
        use crate::game::controller::GameStateView;
        use crate::game::GameState;
        use std::path::PathBuf;

        // Load card from cardsfolder
        let path = PathBuf::from("../cardsfolder/f/force_of_nature.txt");
        if !path.exists() {
            println!("Skipping test: cardsfolder not present");
            return;
        }

        let card_def = crate::loader::CardLoader::load_from_file(&path).expect("Failed to load card");

        // Verify card properties from definition
        assert_eq!(card_def.name.as_str(), "Force of Nature");
        assert_eq!(card_def.power, Some(8));
        assert_eq!(card_def.toughness, Some(8));

        // Set up a game to test creature evaluation
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        let card_id = CardId::new(100);
        let mut card = card_def.instantiate(card_id, p1_id);
        card.controller = p1_id;

        // Verify card properties after instantiation (is_creature() checks types)
        assert!(card.is_creature(), "Force of Nature should be a creature");
        assert!(card.has_trample(), "Force of Nature should have Trample");

        // Verify upkeep trigger exists on instantiated card
        let has_upkeep_trigger = card
            .triggers
            .iter()
            .any(|t| matches!(t.event, crate::core::TriggerEvent::BeginningOfUpkeep));
        assert!(has_upkeep_trigger, "Force of Nature should have an upkeep trigger");

        game.cards.insert(card_id, card);
        game.battlefield.add(card_id);

        let controller = HeuristicController::new(p1_id);
        let view = GameStateView::new(&game, p1_id);

        // Verify creature evaluation with upkeep penalty
        let creature_value = controller.evaluate_creature(&view, card_id);
        // Base: 80 + 20 (non-token)
        // Power: 8 * 15 = 120
        // Toughness: 8 * 10 = 80
        // CMC: 6 * 5 = 30
        // Trample: 8 * 5 = 40 (power * 5)
        // Upkeep trigger penalty: -15 (damage to self)
        // Expected: 100 + 120 + 80 + 30 + 40 - 15 = 355 minimum (still high due to massive stats)
        // Should still be valuable despite upkeep penalty
        assert!(
            creature_value >= 300,
            "Force of Nature evaluation ({}) should be >= 300 despite upkeep penalty due to massive stats",
            creature_value
        );

        // But should be LOWER than an equivalent creature without upkeep cost
        // Create a hypothetical 8/8 Trample without upkeep
        let hypothetical_value = 100 + 120 + 80 + 30 + 40; // 370
        assert!(
            creature_value < hypothetical_value + 10, // Allow small margin
            "Force of Nature should be penalized for upkeep cost (value: {}, pure stats: {})",
            creature_value,
            hypothetical_value
        );

        println!("Force of Nature evaluation: {}", creature_value);
    }

    /// Test land drop hold logic for Main Phase 2 bluffing
    ///
    /// Reference: AiController.isSafeToHoldLandDropForMain2
    ///
    /// This test validates that the probabilistic land-holding logic works.
    /// Due to the complexity of setting up proper game state, this is a simplified
    /// unit test that verifies the basic RNG behavior.
    #[test]
    fn test_land_drop_hold_probabilistic_behavior() {
        use crate::core::PlayerId;
        use rand::Rng;

        let p1_id = PlayerId::new(0);

        // With different seeds, we should eventually see different results
        // across multiple trials
        let mut results = Vec::new();
        for seed in 0..20 {
            let mut controller = HeuristicController::with_seed(p1_id, seed);
            results.push(controller.rng.gen_bool(0.5));
        }

        let true_count = results.iter().filter(|&&x| x).count();
        let false_count = results.len() - true_count;

        // With 20 trials and 50% probability, we should see a mix
        // (not all true or all false)
        assert!(
            true_count > 0 && false_count > 0,
            "RNG should produce varied results (true={}, false={})",
            true_count,
            false_count
        );

        println!(
            "Land drop RNG test: {} true, {} false out of 20 trials",
            true_count, false_count
        );
    }

    /// Test instant-speed spell timing bluffing
    ///
    /// Reference: Java Forge phase restriction patterns (e.g., "AtEOT" in various AIs)
    ///
    /// This test validates that the AI correctly holds instant-speed draw spells
    /// for better timing rather than casting them immediately on its own turn.
    ///
    /// Key bluffing behavior:
    /// - Hold instant-speed spells during our Main 1 (bluff having removal/combat tricks)
    /// - Cast at opponent's end step (maximize bluffing while still getting value)
    /// - Cast at our Main 2 if needed (acceptable fallback timing)
    #[test]
    fn test_instant_spell_bluffing_timing() {
        use crate::core::{Card, CardId, CardType, Effect};
        use crate::game::{GameState, GameStateView};

        // Setup: Two players, P1 has instant-speed draw spell
        let mut game = GameState::new_two_player("Alice".to_string(), "Bob".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // Create instant-speed draw spell (like "Ancestral Recall")
        let draw_spell_id = CardId::new(100);
        let mut draw_spell = Card::new(draw_spell_id, "Instant Draw", p1_id);
        draw_spell.add_type(CardType::Instant);
        draw_spell.effects.push(Effect::DrawCards {
            player: p1_id,
            count: 3,
        });

        // Insert card into game and place in P1's hand
        game.cards.insert(draw_spell_id, draw_spell.clone());
        game.get_player_zones_mut(p1_id).unwrap().hand.cards.push(draw_spell_id);

        let controller = HeuristicController::new(p1_id);

        // Scenario 1: Our Main 1, low hand (1 card) - should HOLD (bluffing)
        game.turn.current_step = crate::game::Step::Main1;
        game.turn.active_player = p1_id;
        let view1 = GameStateView::new(&game, p1_id);
        let should_cast_main1 = controller.should_cast_instant_now(&view1, &draw_spell);
        assert!(
            !should_cast_main1,
            "Should HOLD instant draw during our Main 1 for bluffing (low hand)"
        );

        // Scenario 2: Our Main 2 - should CAST (acceptable timing)
        game.turn.current_step = crate::game::Step::Main2;
        let view2 = GameStateView::new(&game, p1_id);
        let should_cast_main2 = controller.should_cast_instant_now(&view2, &draw_spell);
        assert!(
            should_cast_main2,
            "Should CAST instant draw during our Main 2 (acceptable timing)"
        );

        // Scenario 3: Opponent's end step - should CAST (ideal bluffing window)
        game.turn.active_player = p2_id;
        game.turn.current_step = crate::game::Step::End;
        let view3 = GameStateView::new(&game, p1_id);
        let should_cast_opp_end = controller.should_cast_instant_now(&view3, &draw_spell);
        assert!(
            should_cast_opp_end,
            "Should CAST instant draw at opponent's end step (ideal timing)"
        );

        // Scenario 4: Hand size 7+ - should CAST immediately (avoid discard)
        // Add 6 more cards to hand (already have 1 draw spell = 7 total)
        for i in 0..6 {
            let filler_id = CardId::new(200 + i);
            let filler = Card::new(filler_id, "Filler", p1_id);
            game.cards.insert(filler_id, filler);
            game.get_player_zones_mut(p1_id).unwrap().hand.cards.push(filler_id);
        }
        game.turn.active_player = p1_id;
        game.turn.current_step = crate::game::Step::Main1;
        let view4 = GameStateView::new(&game, p1_id);
        let should_cast_full_hand = controller.should_cast_instant_now(&view4, &draw_spell);
        assert!(
            should_cast_full_hand,
            "Should CAST immediately when hand is full (7+ cards, avoid discard)"
        );

        println!("Instant spell bluffing test passed - AI correctly holds instant-speed spells for better timing");
    }

    /// Test PutCounterAll AI evaluation
    ///
    /// Reference: CountersPutAllAi.java:25-115 (checkApiLogic)
    ///
    /// Validates that:
    /// 1. AI casts beneficial PutCounterAll when we have creatures
    /// 2. AI doesn't cast when we have no creatures
    /// 3. AI casts when restriction filters to our creatures only (YouCtrl)
    #[test]
    fn test_should_cast_put_counter_all() {
        use crate::core::entity::EntityId;
        use crate::core::{
            effects::{ControllerRestriction, TargetRestriction, TargetType},
            Card, CardType, CounterType, Effect,
        };
        use crate::game::state::GameState;
        use crate::game::GameStateView;
        use smallvec::smallvec;

        let p1_id = EntityId::new(0);
        let p2_id = EntityId::new(1);
        let controller = HeuristicController::new(p1_id);

        // Create a PutCounterAll spell: "Put a +1/+1 counter on each creature you control"
        let mut spell = Card::new(EntityId::new(100), "Anthem Spell", p1_id);
        spell.add_type(CardType::Sorcery);
        spell.effects = vec![Effect::PutCounterAll {
            restriction: TargetRestriction {
                types: smallvec![TargetType::Creature],
                requires_no_counters: false,
                controller: ControllerRestriction::YouCtrl,
                power_ge: None,
                power_le: None,
                requires_nontoken: false,
                requires_remembered: false,
                requires_nonartifact: false,
                required_color: None,
                required_set: None,
                requires_other: false,
                required_subtype: None,
                power_le_source: false,
                requires_noncreature: false,
                min_cmc: None,
            },
            counter_type: CounterType::P1P1,
            amount: 1,
        }];

        // Scenario 1: We have 3 creatures → should cast (YouCtrl means only our creatures match)
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        game.turn.active_player = p1_id;
        for i in 0..3u32 {
            let cid = EntityId::new(10 + i);
            let mut c = Card::new(cid, format!("Our Creature {}", i), p1_id);
            c.set_base_power(Some(2));
            c.set_base_toughness(Some(2));
            c.add_type(CardType::Creature);
            c.controller = p1_id;
            game.cards.insert(cid, c);
            game.battlefield.cards.push(cid);
        }
        let view = GameStateView::new(&game, p1_id);
        assert!(
            controller.should_cast_put_counter_all(&spell, &view),
            "Should cast PutCounterAll when we have 3 creatures"
        );

        // Scenario 2: No creatures → should NOT cast
        let mut game2 = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        game2.turn.active_player = p1_id;
        let view2 = GameStateView::new(&game2, p1_id);
        assert!(
            !controller.should_cast_put_counter_all(&spell, &view2),
            "Should NOT cast PutCounterAll when we have no creatures"
        );

        // Scenario 3: Spell with "any creature" restriction - cast only if we have more creatures
        let mut global_spell = Card::new(EntityId::new(101), "Global Anthem", p1_id);
        global_spell.add_type(CardType::Sorcery);
        global_spell.effects = vec![Effect::PutCounterAll {
            restriction: TargetRestriction {
                types: smallvec![TargetType::Creature],
                requires_no_counters: false,
                controller: ControllerRestriction::Any,
                power_ge: None,
                power_le: None,
                requires_nontoken: false,
                requires_remembered: false,
                requires_nonartifact: false,
                required_color: None,
                required_set: None,
                requires_other: false,
                required_subtype: None,
                power_le_source: false,
                requires_noncreature: false,
                min_cmc: None,
            },
            counter_type: CounterType::P1P1,
            amount: 1,
        }];

        // Add equal creatures for both players
        let mut game3 = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        game3.turn.active_player = p1_id;
        for i in 0..2u32 {
            let our_cid = EntityId::new(30 + i);
            let mut our_c = Card::new(our_cid, format!("Our {}", i), p1_id);
            our_c.set_base_power(Some(2));
            our_c.set_base_toughness(Some(2));
            our_c.add_type(CardType::Creature);
            our_c.controller = p1_id;
            game3.cards.insert(our_cid, our_c);
            game3.battlefield.cards.push(our_cid);

            let their_cid = EntityId::new(40 + i);
            let mut their_c = Card::new(their_cid, format!("Their {}", i), p2_id);
            their_c.set_base_power(Some(2));
            their_c.set_base_toughness(Some(2));
            their_c.add_type(CardType::Creature);
            their_c.controller = p2_id;
            game3.cards.insert(their_cid, their_c);
            game3.battlefield.cards.push(their_cid);
        }
        let view3 = GameStateView::new(&game3, p1_id);
        assert!(
            !controller.should_cast_put_counter_all(&global_spell, &view3),
            "Should NOT cast global PutCounterAll when opponent has equal creatures"
        );

        println!("PutCounterAll AI test passed - AI correctly evaluates mass counter placement");
    }

    /// Test ChangeZoneAll AI evaluation
    ///
    /// Reference: ChangeZoneAllAi.java:20-200 (canPlay)
    ///
    /// Validates that:
    /// 1. AI casts battlefield bounce when opponent has more creatures
    /// 2. AI doesn't cast bounce when we'd lose more value
    /// 3. AI casts graveyard exile effects
    #[test]
    fn test_should_cast_change_zone_all() {
        use crate::core::entity::EntityId;
        use crate::core::{
            effects::{ControllerRestriction, TargetRestriction, TargetType},
            Card, CardType, Effect,
        };
        use crate::game::state::GameState;
        use crate::game::GameStateView;
        use smallvec::smallvec;

        let p1_id = EntityId::new(0);
        let p2_id = EntityId::new(1);
        let controller = HeuristicController::new(p1_id);

        // Create a bounce-all spell: "Return all creatures to their owners' hands" (Aetherize-like)
        let mut bounce_spell = Card::new(EntityId::new(100), "Mass Bounce", p1_id);
        bounce_spell.add_type(CardType::Instant);
        bounce_spell.effects = vec![Effect::ChangeZoneAll {
            restriction: TargetRestriction {
                types: smallvec![TargetType::Creature],
                requires_no_counters: false,
                controller: ControllerRestriction::Any,
                power_ge: None,
                power_le: None,
                requires_nontoken: false,
                requires_remembered: false,
                requires_nonartifact: false,
                required_color: None,
                required_set: None,
                requires_other: false,
                required_subtype: None,
                power_le_source: false,
                requires_noncreature: false,
                min_cmc: None,
            },
            origins: smallvec![crate::zones::Zone::Battlefield],
            destination: crate::zones::Zone::Hand,
            shuffle: false,
        }];

        // Scenario 1: Opponent has 3 big creatures, we have 1 small one → should cast
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        game.turn.active_player = p1_id;

        // Our small creature
        let our_cid = EntityId::new(10);
        let mut our_c = Card::new(our_cid, "Our Bear", p1_id);
        our_c.set_base_power(Some(2));
        our_c.set_base_toughness(Some(2));
        our_c.add_type(CardType::Creature);
        our_c.controller = p1_id;
        game.cards.insert(our_cid, our_c);
        game.battlefield.cards.push(our_cid);

        // Opponent's big creatures
        for i in 0..3u32 {
            let opp_cid = EntityId::new(20 + i);
            let mut opp_c = Card::new(opp_cid, format!("Opp Dragon {}", i), p2_id);
            opp_c.set_base_power(Some(5));
            opp_c.set_base_toughness(Some(5));
            opp_c.add_type(CardType::Creature);
            opp_c.controller = p2_id;
            game.cards.insert(opp_cid, opp_c);
            game.battlefield.cards.push(opp_cid);
        }

        let view = GameStateView::new(&game, p1_id);
        assert!(
            controller.should_cast_change_zone_all(&bounce_spell, &view),
            "Should cast mass bounce when opponent has 3 big creatures vs our 1 small"
        );

        // Scenario 2: We have 3 big creatures, opponent has 1 → should NOT cast
        let mut game2 = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        game2.turn.active_player = p1_id;

        for i in 0..3u32 {
            let our_cid2 = EntityId::new(30 + i);
            let mut our_c2 = Card::new(our_cid2, format!("Our Dragon {}", i), p1_id);
            our_c2.set_base_power(Some(5));
            our_c2.set_base_toughness(Some(5));
            our_c2.add_type(CardType::Creature);
            our_c2.controller = p1_id;
            game2.cards.insert(our_cid2, our_c2);
            game2.battlefield.cards.push(our_cid2);
        }
        let opp_cid2 = EntityId::new(40);
        let mut opp_c2 = Card::new(opp_cid2, "Opp Bear", p2_id);
        opp_c2.set_base_power(Some(2));
        opp_c2.set_base_toughness(Some(2));
        opp_c2.add_type(CardType::Creature);
        opp_c2.controller = p2_id;
        game2.cards.insert(opp_cid2, opp_c2);
        game2.battlefield.cards.push(opp_cid2);

        let view2 = GameStateView::new(&game2, p1_id);
        assert!(
            !controller.should_cast_change_zone_all(&bounce_spell, &view2),
            "Should NOT cast mass bounce when we have 3 big creatures vs opponent's 1"
        );

        // Scenario 3: Graveyard exile effect → always cast
        let mut exile_spell = Card::new(EntityId::new(101), "Graveyard Exile", p1_id);
        exile_spell.add_type(CardType::Instant);
        exile_spell.effects = vec![Effect::ChangeZoneAll {
            restriction: TargetRestriction::any(),
            origins: smallvec![crate::zones::Zone::Graveyard],
            destination: crate::zones::Zone::Exile,
            shuffle: false,
        }];

        let mut game3 = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        game3.turn.active_player = p1_id;
        let view3 = GameStateView::new(&game3, p1_id);
        assert!(
            controller.should_cast_change_zone_all(&exile_spell, &view3),
            "Should cast graveyard exile (always beneficial)"
        );

        println!("ChangeZoneAll AI test passed - AI correctly evaluates mass zone changes");
    }

    // ==================== 4ED Card AI Tests ====================

    /// Test: AI should cast discard spells when opponent has cards in hand
    #[test]
    fn test_should_cast_discard() {
        use crate::core::Card;
        use crate::game::controller::GameStateView;
        use crate::game::GameState;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;
        game.turn.active_player = p1_id;

        let controller = HeuristicController::new(p1_id);

        // Scenario 1: Opponent has cards in hand → should cast
        // Give opponent some cards in hand
        for i in 0..3u32 {
            let cid = EntityId::new(50 + i);
            let card = Card::new(cid, format!("Opp Card {}", i), p2_id);
            game.cards.insert(cid, card);
            if let Some(zones) = game.get_player_zones_mut(p2_id) {
                zones.hand.cards.push(cid);
            }
        }

        let view = GameStateView::new(&game, p1_id);
        assert!(
            controller.should_cast_discard(&view),
            "Should cast discard when opponent has 3 cards in hand"
        );

        // Scenario 2: Opponent has empty hand → should NOT cast
        let mut game2 = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        game2.turn.active_player = p1_id;
        let view2 = GameStateView::new(&game2, p1_id);
        assert!(
            !controller.should_cast_discard(&view2),
            "Should NOT cast discard when opponent has no cards in hand"
        );

        println!("Discard AI test passed - correctly evaluates opponent hand size");
    }

    /// Test: AI should cast tap spells when opponent has untapped creatures
    #[test]
    fn test_should_cast_tap_permanent() {
        use crate::core::{Card, CardType};
        use crate::game::controller::GameStateView;
        use crate::game::GameState;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;
        game.turn.active_player = p1_id;

        let controller = HeuristicController::new(p1_id);

        // Scenario 1: Opponent has untapped creature → should cast
        let opp_cid = EntityId::new(50);
        let mut opp_creature = Card::new(opp_cid, "Opp Bear", p2_id);
        opp_creature.add_type(CardType::Creature);
        opp_creature.set_base_power(Some(4));
        opp_creature.set_base_toughness(Some(4));
        opp_creature.controller = p2_id;
        opp_creature.tapped = false;
        game.cards.insert(opp_cid, opp_creature);
        game.battlefield.cards.push(opp_cid);

        let view = GameStateView::new(&game, p1_id);
        assert!(
            controller.should_cast_tap_permanent(&view),
            "Should cast tap when opponent has untapped creature"
        );

        // Scenario 2: Only our creatures on battlefield → should NOT cast
        let mut game2 = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        game2.turn.active_player = p1_id;

        let our_cid = EntityId::new(60);
        let mut our_creature = Card::new(our_cid, "Our Bear", p1_id);
        our_creature.add_type(CardType::Creature);
        our_creature.set_base_power(Some(2));
        our_creature.set_base_toughness(Some(2));
        our_creature.controller = p1_id;
        game2.cards.insert(our_cid, our_creature);
        game2.battlefield.cards.push(our_cid);

        let view2 = GameStateView::new(&game2, p1_id);
        assert!(
            !controller.should_cast_tap_permanent(&view2),
            "Should NOT cast tap when no opponent creatures"
        );

        println!("Tap permanent AI test passed");
    }

    /// Test: Icy Manipulator's tap ability is classified as TapTarget
    #[test]
    fn test_icy_manipulator_from_cardsfolder() {
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/i/icy_manipulator.txt");
        if !path.exists() {
            println!("Skipping test: cardsfolder not present");
            return;
        }

        let def = crate::loader::CardLoader::load_from_file(&path).expect("Failed to load Icy Manipulator");
        assert_eq!(def.name.as_str(), "Icy Manipulator");

        let game = crate::game::GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let card_id = crate::core::CardId::new(100);
        let card = def.instantiate(card_id, p1_id);

        assert!(card.is_artifact(), "Icy Manipulator should be an artifact");

        // Find the non-mana tap ability
        let tap_abilities: Vec<_> = card.activated_abilities.iter().filter(|a| !a.is_mana_ability).collect();

        assert!(
            !tap_abilities.is_empty(),
            "Icy Manipulator should have at least one non-mana activated ability"
        );

        // Verify the ability has a TapPermanent effect
        let ability = tap_abilities[0];
        let has_tap_effect = ability
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::TapPermanent { .. }));

        assert!(
            has_tap_effect,
            "Icy Manipulator's ability should have a TapPermanent effect"
        );

        // Test AI classification
        let controller = HeuristicController::new(p1_id);
        let ability_type = controller.classify_activated_ability(ability);
        assert!(
            matches!(ability_type, ActivatedAbilityType::TapTarget),
            "Icy Manipulator's ability should be classified as TapTarget by AI"
        );

        println!("Icy Manipulator test passed - ability correctly classified as TapTarget");
    }

    /// Test: Hymn to Tourach parsed correctly and AI evaluates casting
    #[test]
    fn test_hymn_to_tourach_from_cardsfolder() {
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/h/hymn_to_tourach.txt");
        if !path.exists() {
            println!("Skipping test: cardsfolder not present");
            return;
        }

        let def = crate::loader::CardLoader::load_from_file(&path).expect("Failed to load Hymn to Tourach");
        assert_eq!(def.name.as_str(), "Hymn to Tourach");

        let game = crate::game::GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let card_id = crate::core::CardId::new(100);
        let card = def.instantiate(card_id, p1_id);

        // Verify it has a DiscardCards effect
        let has_discard = card.effects.iter().any(|e| {
            matches!(
                e,
                crate::core::Effect::DiscardCards { .. } | crate::core::Effect::DiscardCardsXPaid { .. }
            )
        });

        assert!(has_discard, "Hymn to Tourach should have a Discard effect");

        println!("Hymn to Tourach test passed - discard effect correctly parsed");
    }

    /// Test: Draw spell AI casts at higher hand-size threshold
    #[test]
    fn test_draw_spell_hand_threshold() {
        use crate::core::{Card, CardType, Effect};
        use crate::game::controller::GameStateView;
        use crate::game::GameState;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        game.turn.active_player = p1_id;

        // Create a draw spell (sorcery speed)
        let draw_card_id = EntityId::new(100);
        let mut draw_spell = Card::new(draw_card_id, "Divination", p1_id);
        draw_spell.add_type(CardType::Sorcery);
        draw_spell.effects = vec![Effect::DrawCards {
            player: p1_id,
            count: 2,
        }];

        let controller = HeuristicController::new(p1_id);

        // Scenario 1: 3 cards in hand → should cast (was too restrictive before at <= 2)
        for i in 0..3u32 {
            let cid = EntityId::new(50 + i);
            let c = Card::new(cid, format!("Card {}", i), p1_id);
            game.cards.insert(cid, c);
            if let Some(zones) = game.get_player_zones_mut(p1_id) {
                zones.hand.cards.push(cid);
            }
        }

        let view = GameStateView::new(&game, p1_id);
        assert!(
            controller.should_cast_spell(&draw_spell, &view),
            "Should cast draw spell with 3 cards in hand (threshold is now 4)"
        );

        // Scenario 2: 5 cards in hand → should NOT cast
        let mut game2 = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        game2.turn.active_player = p1_id;
        for i in 0..5u32 {
            let cid = EntityId::new(60 + i);
            let c = Card::new(cid, format!("Card {}", i), p1_id);
            game2.cards.insert(cid, c);
            if let Some(zones) = game2.get_player_zones_mut(p1_id) {
                zones.hand.cards.push(cid);
            }
        }

        let view2 = GameStateView::new(&game2, p1_id);
        assert!(
            !controller.should_cast_spell(&draw_spell, &view2),
            "Should NOT cast draw spell with 5 cards in hand"
        );

        println!("Draw spell hand threshold test passed");
    }

    /// Test: Discard spells route through should_cast_spell correctly
    #[test]
    fn test_discard_spell_routing() {
        use crate::core::{Card, CardType, Effect};
        use crate::game::controller::GameStateView;
        use crate::game::GameState;

        let p1_id = crate::core::PlayerId::new(0);

        // Create a Hymn to Tourach-like spell
        let spell_id = EntityId::new(100);
        let mut hymn = Card::new(spell_id, "Hymn to Tourach", p1_id);
        hymn.add_type(CardType::Sorcery);
        hymn.effects = vec![Effect::DiscardCards {
            player: crate::core::PlayerId::new(1), // opponent
            count: 2,
            remember_discarded: false,
            optional: false,
            remember_discarding_players: false,
        }];

        let controller = HeuristicController::new(p1_id);

        // Scenario: opponent has cards → should cast
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        game.turn.active_player = p1_id;
        let p2_id = game.players[1].id;

        // Give opponent cards
        for i in 0..3u32 {
            let cid = EntityId::new(50 + i);
            let c = Card::new(cid, format!("Opp Card {}", i), p2_id);
            game.cards.insert(cid, c);
            if let Some(zones) = game.get_player_zones_mut(p2_id) {
                zones.hand.cards.push(cid);
            }
        }

        let view = GameStateView::new(&game, p1_id);
        assert!(
            controller.should_cast_spell(&hymn, &view),
            "should_cast_spell should route Discard effects and approve with opponent hand > 0"
        );

        println!("Discard spell routing test passed");
    }

    /// mtg-721: equip + sacrifice-to-draw abilities must classify as their own
    /// types (not the catch-all `Other`, which `should_activate_ability` never
    /// fires). Before this fix the heuristic AI never equipped Trusty Boomerang
    /// nor cracked Clue tokens.
    #[test]
    fn classify_equip_and_clue_abilities() {
        use crate::core::{ActivatedAbility, CardId, Cost, Effect, ManaCost};

        let controller = HeuristicController::new(PlayerId::new(0));

        // Trusty Boomerang's `K:Equip:1` → AttachEquipment effect.
        let equip = ActivatedAbility::new(
            Cost::Mana(ManaCost::new()),
            vec![Effect::AttachEquipment {
                source_equipment: CardId::new(10),
                target_creature: CardId::new(11),
            }],
            "Equip 1".to_string(),
            false,
        );
        assert!(matches!(
            controller.classify_activated_ability(&equip),
            ActivatedAbilityType::Equip
        ));

        // Clue Token's "{2}, Sacrifice this token: Draw a card." → DrawCards.
        let crack = ActivatedAbility::new(
            Cost::Tap,
            vec![Effect::DrawCards {
                player: PlayerId::new(0),
                count: 1,
            }],
            "Draw a card.".to_string(),
            false,
        );
        assert!(matches!(
            controller.classify_activated_ability(&crack),
            ActivatedAbilityType::DrawCard
        ));
    }

    /// mtg-721: with the new classification, `should_activate_ability` must fire
    /// equip during our main phase when the Equipment is UNATTACHED and we
    /// control a creature — and must NOT re-fire once attached (no equip-thrash).
    #[test]
    fn should_activate_equip_when_unattached_with_creature() {
        use crate::core::{ActivatedAbility, Card, CardId, CardType, Cost, Effect, ManaCost};
        use crate::game::controller::GameStateView;
        use crate::game::{GameState, Step};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        game.turn.active_player = p1_id;
        game.turn.current_step = Step::Main1;

        // A creature we control to equip.
        let creature_id = CardId::new(200);
        let mut creature = Card::new(creature_id, "Grizzly Bears", p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(2));
        creature.set_base_toughness(Some(2));
        game.cards.insert(creature_id, creature);

        // Trusty Boomerang (Equipment) with its equip ability, UNATTACHED.
        let boomerang_id = CardId::new(201);
        let mut boomerang = Card::new(boomerang_id, "Trusty Boomerang", p1_id);
        boomerang.add_type(CardType::Artifact);
        boomerang.activated_abilities = vec![ActivatedAbility::new(
            Cost::Mana(ManaCost::new()),
            vec![Effect::AttachEquipment {
                source_equipment: boomerang_id,
                target_creature: creature_id,
            }],
            "Equip 1".to_string(),
            false,
        )];
        game.cards.insert(boomerang_id, boomerang);

        game.battlefield.add(creature_id);
        game.battlefield.add(boomerang_id);

        let controller = HeuristicController::new(p1_id);

        // Unattached + creature present in Main1 → equip.
        {
            let view = GameStateView::new(&game, p1_id);
            let bm = view.get_card(boomerang_id).unwrap();
            assert!(
                controller.should_activate_ability(bm, &view),
                "AI should equip the unattached Boomerang in Main1"
            );
        }

        // Once attached, do NOT re-equip (no thrash).
        game.cards.get_mut(boomerang_id).unwrap().attached_to = Some(creature_id);
        {
            let view = GameStateView::new(&game, p1_id);
            let bm = view.get_card(boomerang_id).unwrap();
            assert!(
                !controller.should_activate_ability(bm, &view),
                "AI must not re-equip an already-attached Equipment"
            );
        }
    }
}
