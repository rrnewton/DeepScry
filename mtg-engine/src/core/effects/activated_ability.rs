use serde::{Deserialize, Serialize};

use super::Effect;

/// Cache for expensive string operations on ActivatedAbility
/// Pre-computed at ability creation time to avoid repeated allocations during gameplay
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbilityCache {
    /// Lowercase version of description (computed once)
    pub description_lowercase: String,

    /// Pre-computed contains() checks for targeting restrictions
    pub targets_tapped: bool,
    pub targets_untapped: bool,
    pub targets_creature: bool,
    pub targets_land: bool,
    pub requires_target: bool,
    /// True when the ability targets a player (any player), e.g. "target player".
    /// Used to enumerate all players as valid targets for effects like SetLife
    /// that target a player (e.g. "Target player's life total becomes 10").
    pub targets_player: bool,
    /// True when the ability targets an opponent specifically, e.g. "target opponent".
    /// Used to enumerate only opponents as valid targets for effects like Sorin
    /// Markov's -3 ("Target opponent's life total becomes 10.").
    pub targets_opponent: bool,
}

impl AbilityCache {
    /// Create a new cache from ability description
    pub fn new(description: &str) -> Self {
        let desc_lower = description.to_lowercase();

        // Check for "target opponent" before "target player" to distinguish them.
        // "target opponent" is a subset of "target player" semantically, but the
        // engine enumerates different player sets for each.
        let targets_opponent = desc_lower.contains("target opponent");
        // "target player" catches abilities that target any player (including self),
        // but exclude the opponent-only case to avoid double-counting.
        let targets_player = !targets_opponent && desc_lower.contains("target player");

        AbilityCache {
            // Store lowercase version
            description_lowercase: desc_lower.clone(),

            // Targeting restriction flags
            targets_tapped: desc_lower.contains("tapped"),
            targets_untapped: desc_lower.contains("untapped"),
            targets_creature: desc_lower.contains("creature"),
            targets_land: desc_lower.contains("land"),
            // requires_target: true if this ability itself needs a target selected at activation.
            // We check for "target" as a word in the description, but exclude cases where "target"
            // appears only inside a token's sub-ability text (e.g. Tibalt's Devil token:
            // "Create a 1/1 red Devil ... with 'deals 1 damage to any target.'").
            // Heuristic: if the description starts with "Create" (token creation), the activation
            // itself never needs a target — any "target" in the text belongs to the token's ability.
            requires_target: (desc_lower.contains("target") && !desc_lower.starts_with("create"))
                || desc_lower.starts_with("equip"),
            targets_player,
            targets_opponent,
        }
    }
}

/// An activated ability that can be activated by paying a cost
/// Example: "{T}: Deal 1 damage to any target" (Prodigal Sorcerer)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivatedAbility {
    /// The cost to activate this ability
    pub cost: crate::core::Cost,

    /// The effects that execute when this ability resolves
    pub effects: Vec<Effect>,

    /// Description of the ability (for logging and display)
    pub description: String,

    /// Whether this is a mana ability (doesn't use the stack)
    pub is_mana_ability: bool,

    /// Whether this ability can only be activated at sorcery speed
    /// "Activate only as a sorcery" (CR 602.5d, CR 307.5)
    /// Requires: priority, main phase, your turn, stack empty
    pub sorcery_speed: bool,

    /// Whether this ability can only be activated during your turn
    /// "Activate only during your turn" (PlayerTurn$ True)
    /// Less restrictive than sorcery_speed - only checks turn ownership
    pub your_turn_only: bool,

    /// Whether this is an exhaust ability (can only be activated once per game)
    /// "Exhaust$ True" - activate each exhaust ability only once
    pub exhaust: bool,

    /// Optional "Activate only if ..." restriction from
    /// `IsPresent$ | PresentZone$ | PresentCompare$` (Library of Alexandria's
    /// "exactly seven cards in hand", Cryptic Caves' "five or more lands", ...).
    /// `None` = no extra restriction. Checked in `can_activate` alongside the
    /// other timing/cost gates.
    #[serde(default)]
    pub activation_condition: Option<crate::core::ActivationCondition>,

    /// Optional turn-step window restriction from `ActivationPhases$
    /// <start>-><end>` (Jade Statue's combat-only `BeginCombat->EndCombat`
    /// animate). `None` = activatable in any step (the common case). Checked
    /// in `can_activate` alongside the other timing gates. Rewind-safe: reads
    /// only the deterministically-reconstructed current step (CR 602.5).
    #[serde(default)]
    pub activation_phases: Option<crate::core::ActivationPhaseWindow>,

    /// True for `AB$ ManaReflected` abilities (Fellwar Stone: "Add one mana of
    /// any color that a land an opponent controls could produce"). The static
    /// mana-production cache treats such a source as AnyColor (an upper bound),
    /// but at activation time the produced color is constrained to the set of
    /// colors the `Valid$` lands could actually produce — computed from public
    /// battlefield state, so it stays information-independent / deterministic.
    #[serde(default)]
    pub produces_reflected_mana: bool,

    /// The zone in which this ability can be activated (default: Battlefield).
    ///
    /// Parsed from `ActivationZone$` in the card script, e.g.:
    ///   `ActivationZone$ Graveyard` — activatable while the card is in the
    ///   owner's graveyard (CR 702.25b unearth, graveyard-recursion engines).
    ///   `ActivationZone$ Hand`      — activatable from hand (cycling uses a
    ///   dedicated SpellAbility::Cycle variant, but this field handles any
    ///   future hand-activated non-cycle abilities).
    ///
    /// The action enumerator uses this to walk the correct zone when building
    /// the list of available actions (see `push_activatable_abilities`).
    #[serde(default = "default_activation_zone")]
    pub activation_zone: crate::zones::Zone,

    /// Cache for expensive string operations (computed at creation time)
    pub cache: AbilityCache,
}

/// Serde default helper — `Zone::Battlefield` is the overwhelmingly common case.
fn default_activation_zone() -> crate::zones::Zone {
    crate::zones::Zone::Battlefield
}

impl ActivatedAbility {
    /// Create a new activated ability
    pub fn new(cost: crate::core::Cost, effects: Vec<Effect>, description: String, is_mana_ability: bool) -> Self {
        let cache = AbilityCache::new(&description);

        ActivatedAbility {
            cost,
            effects,
            description,
            is_mana_ability,
            sorcery_speed: false,  // Default to instant speed
            your_turn_only: false, // Default to any turn
            exhaust: false,        // Default to non-exhaust
            activation_condition: None,
            activation_phases: None,
            produces_reflected_mana: false,
            activation_zone: crate::zones::Zone::Battlefield,
            cache,
        }
    }

    /// Create a new sorcery-speed activated ability
    pub fn new_sorcery_speed(cost: crate::core::Cost, effects: Vec<Effect>, description: String) -> Self {
        let cache = AbilityCache::new(&description);

        ActivatedAbility {
            cost,
            effects,
            description,
            is_mana_ability: false, // Sorcery-speed abilities are not mana abilities
            sorcery_speed: true,
            your_turn_only: false, // sorcery_speed implies your turn already
            exhaust: false,
            activation_condition: None,
            activation_phases: None,
            produces_reflected_mana: false,
            activation_zone: crate::zones::Zone::Battlefield,
            cache,
        }
    }

    /// Create a new your-turn-only activated ability
    /// Less restrictive than sorcery speed - can be activated any time during my turn
    pub fn new_your_turn_only(
        cost: crate::core::Cost,
        effects: Vec<Effect>,
        description: String,
        is_mana_ability: bool,
    ) -> Self {
        let cache = AbilityCache::new(&description);

        ActivatedAbility {
            cost,
            effects,
            description,
            is_mana_ability,
            sorcery_speed: false, // Not sorcery speed
            your_turn_only: true, // Your turn only
            exhaust: false,
            activation_condition: None,
            activation_phases: None,
            produces_reflected_mana: false,
            activation_zone: crate::zones::Zone::Battlefield,
            cache,
        }
    }
}
