use serde::{Deserialize, Serialize};

use crate::core::Cost;

use super::{Effect, PresentSelfCondition};

/// A single mode in a modal spell.
///
/// Contains the effect to execute and metadata for display/targeting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModalMode {
    /// The effect to execute when this mode is chosen
    pub effect: Box<Effect>,
    /// Human-readable description (from SpellDescription$)
    pub description: String,
    /// SVar name for this mode (e.g., "DBDestroy") - used for targeting lookup
    pub svar_name: String,
    /// Additional generic mana cost for choosing this mode (from ModeCost$ in the SVar).
    /// Zero means no extra cost. Used by tiered modal spells like Fire Magic where each
    /// tier (Fire/Fira/Firaga) has a different extra cost beyond the base mana cost.
    pub mode_cost: u8,
    /// Whether this mode requires player-chosen targeting (ValidTgts$ was
    /// present in the SVar). When `true`, target selection must happen after
    /// mode selection (e.g. Jitte's JitteCurse "Target creature gets -1/-1").
    /// When `false`, the target is pre-defined (Defined$ Equipped/Self/etc.)
    /// or no targeting is needed (GainLife, DrawCards, …).
    #[serde(default)]
    pub needs_targeting: bool,
}

/// Which combat-damage recipient class a `DealsCombatDamage` trigger watches.
///
/// Combat damage is dealt as one simultaneous event (CR 510.2). A creature's
/// `DealsCombatDamage` trigger sees that one event, but the trigger's
/// `ValidTarget$` clause restricts *which* recipients count:
///
/// - `ValidTarget$ Player` / `Opponent` / `Player,Planeswalker` -> [`Player`](Self::Player):
///   fire only when the source dealt combat damage to a player (or
///   planeswalker), amount = damage dealt to players. (Hypnotic Specter,
///   Mark of Sakiko.)
/// - `ValidTarget$ Creature` -> [`Creature`](Self::Creature): fire only when
///   the source dealt combat damage to a creature, amount = damage to
///   creatures.
/// - no `ValidTarget$` restriction (or an aggregating `DamageDealtOnce`) ->
///   [`Any`](Self::Any): fire whenever the source dealt ANY combat damage,
///   amount = total combat damage dealt to all recipients (Spirit Link's
///   lifelink, CR 119.3-style).
///
/// Replaces the dead `[any-damage]` / `[damages-creature]` description markers
/// with a structured filter consumed at the single combat-damage firing site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CombatDamageTarget {
    /// Fire only on combat damage dealt to a player/planeswalker.
    Player,
    /// Fire only on combat damage dealt to a creature.
    Creature,
    /// Fire on any combat damage dealt (default; matches Lifelink semantics).
    #[default]
    Any,
}

/// Events that can trigger abilities
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TriggerEvent {
    /// When a card enters the battlefield
    /// Corresponds to: T:Mode$ ChangesZone | Origin$ Any | Destination$ Battlefield | ValidCard$ Card.Self
    EntersBattlefield,

    /// When a card leaves the battlefield
    /// Corresponds to: T:Mode$ ChangesZone | Origin$ Battlefield | Destination$ Any | ValidCard$ Card.Self
    LeavesBattlefield,

    /// At the beginning of upkeep
    /// Corresponds to: T:Mode$ Phase | Phase$ Upkeep | ValidPlayer$ You
    BeginningOfUpkeep,

    /// At the beginning of the draw step
    /// Corresponds to: T:Mode$ Phase | Phase$ Draw | ValidPlayer$ You
    /// Example: Grafted Skullcap / Sylvan Library / Yawgmoth's Bargain —
    /// "At the beginning of your draw step, draw an additional card."
    /// Fires from the battlefield after the active player's mandatory draw.
    BeginningOfDraw,

    /// At the beginning of end step
    /// Corresponds to: T:Mode$ Phase | Phase$ EndOfTurn | ValidPlayer$ You
    BeginningOfEndStep,

    /// At the beginning of combat
    /// Corresponds to: T:Mode$ Phase | Phase$ BeginCombat | ValidPlayer$ You
    BeginningOfCombat,

    /// When a spell is cast
    /// Corresponds to: T:Mode$ SpellCast | ValidCard$ ...
    SpellCast,

    /// When a creature attacks
    /// Corresponds to: T:Mode$ Attacks | ValidCard$ Card.Self
    Attacks,

    /// When a creature attacks and is not blocked (fires at the end of the
    /// declare-blockers step, after all blockers are assigned).
    /// Corresponds to: T:Mode$ AttackerUnblocked | ValidCard$ Card.Self
    /// Example: Eternal of Harsh Truths — "Whenever ~ attacks and isn't blocked, draw a card."
    /// Floral Spuzzem — "Whenever ~ attacks and isn't blocked, you may destroy target
    ///                  artifact defending player controls."
    AttackerUnblocked,

    /// When a creature blocks
    /// Corresponds to: T:Mode$ Blocks | ValidCard$ Card.Self
    Blocks,

    /// When a creature deals combat damage
    /// Corresponds to: T:Mode$ DamageDone | ValidSource$ Card.Self | CombatDamage$ True
    DealsCombatDamage,

    /// When a permanent is sacrificed
    /// Corresponds to: T:Mode$ Sacrificed | ValidCard$ Permanent.Other | ValidPlayer$ You
    Sacrificed,

    /// When a card is drawn
    /// Corresponds to: T:Mode$ Drawn | ValidCard$ Card.YouCtrl | Number$ 2
    /// The draw_number field in Trigger specifies which draw triggers (e.g., 2 = second card)
    CardDrawn,

    /// When a permanent becomes tapped
    /// Corresponds to: T:Mode$ Taps | ValidCard$ Card.Self
    /// Example: "Whenever CARDNAME becomes tapped, draw a card, then discard a card."
    Taps,

    /// When a permanent is tapped for mana
    /// Corresponds to: T:Mode$ TapsForMana | ValidCard$ ...
    TapsForMana,

    /// When one or more creatures attack (batch trigger, fires once per declare attackers step)
    /// Corresponds to: T:Mode$ AttackersDeclared | AttackingPlayer$ You | ValidAttackers$ Creature.withFlying
    /// Example: "Whenever one or more creatures you control with flying attack, draw a card."
    AttackersDeclared,

    /// When a creature equipped by this Equipment dies
    /// Corresponds to: T:Mode$ ChangesZone | Origin$ Battlefield | Destination$ Graveyard | ValidCard$ Card.EquippedBy
    /// Example: Skullclamp - "Whenever equipped creature dies, draw two cards."
    EquippedCreatureDies,

    /// When a creature dealt damage by this card this turn dies.
    /// Corresponds to: T:Mode$ ChangesZone | Origin$ Battlefield | Destination$ Graveyard | ValidCard$ Creature.DamagedBy | TriggerZones$ Battlefield
    /// Example: Sengir Vampire — "Whenever a creature dealt damage by Sengir
    /// Vampire this turn dies, put a +1/+1 counter on Sengir Vampire."
    /// Fires from the trigger source (Sengir) when ANY creature in the
    /// dying card's `damaged_by_this_turn` list contains the trigger source's
    /// CardId.
    DamagedCreatureDies,

    /// When ANY creature dies (goes to the graveyard from the battlefield),
    /// regardless of who controls it or the trigger source.
    /// Corresponds to: T:Mode$ ChangesZone | Origin$ Battlefield | Destination$ Graveyard | ValidCard$ Creature[.YouCtrl/.OppCtrl/.Other]
    /// Example: Fecundity — "Whenever a creature dies, that creature's controller may draw a card."
    /// Fires from the trigger source (Fecundity), which sits on the battlefield
    /// while OTHER creatures die. `check_death_triggers` scans the battlefield
    /// for permanents carrying this trigger when any creature dies; the dying
    /// creature's controller is threaded as `TriggeredCardController` via the
    /// `TriggerContext` so `Defined$ TriggeredCardController` resolves to them.
    /// The `ValidCard$` controller qualifier (`.YouCtrl` / `.OppCtrl`) is stored
    /// so the firing site can filter by the dying creature's controller relative
    /// to the trigger source's controller (mtg-409 follow-up, mtg-913 B12).
    CreatureDies {
        /// Controller restriction on the *dying* creature, relative to the
        /// trigger source's controller: `None` = any creature, `Some(true)` =
        /// only creatures the source's controller controls (`.YouCtrl`),
        /// `Some(false)` = only creatures an opponent controls (`.OppCtrl`).
        you_control: Option<bool>,
        /// When `true`, the trigger source's own death does NOT fire the trigger
        /// (`Creature.Other`). When `false`, the source dying also counts.
        exclude_self: bool,
    },

    /// When a Class enchantment reaches a specific level.
    ///
    /// Corresponds to: T:Mode$ ClassLevelGained | ClassLevel$ N | ValidCard$ Card.Self
    ///
    /// Fires on the Class enchantment itself after `Effect::ClassLevelUp`
    /// advances the card's `CounterType::Level` counter to `level`.  Used for
    /// one-time "when this Class becomes level N" effects (e.g. Stormchaser's
    /// Talent level-2: return an instant or sorcery from your graveyard to
    /// your hand).
    ClassLevelGained {
        /// The level that was just reached.
        level: u8,
    },

    /// When a card is discarded
    /// Corresponds to: T:Mode$ Discarded | ValidCard$ Card.YouOwn | TriggerZones$ Battlefield
    /// Example: Monument to Endurance — "Whenever you discard a card, choose one..."
    /// Fires from any permanent on the battlefield when its controller (or any
    /// player matching ValidCard$) discards a card.
    CardDiscarded,

    /// When THIS card is itself discarded (the discarded card is the trigger
    /// source), fired on its last-known information as it moves Hand→Graveyard
    /// (CR 603.6/603.10 — a leaves-the-zone trigger looking back at the object).
    ///
    /// Corresponds to: T:Mode$ Discarded | ValidCard$ Card.Self
    ///   | ValidCause$ SpellAbility.OppCtrl | Execute$ ...
    /// Example: Psychic Purge — "When a spell or ability an opponent controls
    /// causes you to discard Psychic Purge, that player loses 5 life."
    ///
    /// Distinct from [`CardDiscarded`], which is a battlefield permanent
    /// watching its controller's discards (Monument to Endurance). This event
    /// fires on the DISCARDED CARD ITSELF and is gated by `requires_opponent_
    /// cause`: it only fires when the discard was caused by a spell/ability
    /// controlled by an OPPONENT of the card's owner (the `cause` threaded into
    /// `GameState::discard_card`), so a self-discard (cleanup, your own looting)
    /// does NOT fire it.
    Discarded,
}

/// A triggered ability that executes when an event occurs
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trigger {
    /// The event that triggers this ability
    pub event: TriggerEvent,

    /// The effects to execute when triggered
    pub effects: Vec<Effect>,

    /// Description of the trigger (for logging)
    pub description: String,

    /// If true, this trigger only fires when the source card itself triggers the event
    /// (e.g., "When this creature enters" only fires for this specific creature)
    /// If false, triggers for any card matching the event (e.g., "When any creature enters")
    pub trigger_self_only: bool,

    /// If true, the player may choose whether to use this triggered ability
    /// (e.g., "you may sacrifice a creature" - player can decline)
    /// If false, the trigger is mandatory
    pub optional: bool,

    /// Cost that must be paid to execute the trigger effects (for optional triggers)
    /// e.g., sacrificing a permanent, paying life, paying mana
    /// If None, the trigger has no additional cost beyond being optional
    pub cost: Option<Cost>,

    /// For CardDrawn triggers: which draw number triggers this (e.g., 2 = "second card drawn")
    /// None means every card drawn triggers it
    pub draw_number: Option<u8>,

    /// For CardDrawn triggers: true = triggers on controller's draws, false = opponent's draws
    pub triggers_on_controller_draw: bool,

    // =========================================================================
    // STRUCTURED FILTER FLAGS - replacing string markers in description
    // These provide compile-time checked filtering instead of runtime string parsing
    // =========================================================================
    /// When true, trigger only fires if event source is DIFFERENT from trigger source
    /// Replaces "[other]" marker in description
    /// Example: "Whenever you sacrifice ANOTHER permanent" (Pirate Peddlers)
    #[serde(default)]
    pub requires_other: bool,

    /// When true, trigger only fires if event source is a Land controlled by trigger controller
    /// Replaces "[landfall]" marker in description
    /// Example: Landfall triggers like "Whenever a land enters under your control"
    #[serde(default)]
    pub requires_landfall: bool,

    /// When true, trigger only fires on controller's turn
    /// Replaces "[controller_only]" marker in description
    /// Example: Upkeep triggers that only fire on your own upkeep
    #[serde(default)]
    pub controller_turn_only: bool,

    /// When true, the trigger only fires on the turn of the player CHOSEN by the
    /// source's ETB ChoosePlayer replacement (`ValidPlayer$ Player.Chosen`).
    /// Black Vise's upkeep trigger fires only on the chosen player's upkeep.
    /// The chosen player is stored in `Card::chosen_player`; the firing sites
    /// gate on `active_player == card.chosen_player`. Mutually exclusive with
    /// `controller_turn_only` in practice (a trigger is either "your" or
    /// "chosen player's" turn, not both).
    #[serde(default)]
    pub chosen_player_turn_only: bool,

    /// When true, the trigger only fires on the upkeep of the ENCHANTED
    /// permanent's controller — a DIFFERENT player than the source Aura's
    /// controller for a curse Aura (`ValidPlayer$ Player.EnchantedController`).
    /// Paralyze's "At the beginning of the upkeep of enchanted creature's
    /// controller, that player may pay {4}; if they do, untap the creature."
    /// The firing sites gate on `active_player == cards[aura.attached_to].
    /// controller` instead of `active_player == aura.controller`. If the Aura
    /// is not attached (no `attached_to`), the trigger cannot fire.
    #[serde(default)]
    pub enchanted_controller_turn_only: bool,

    /// For [`TriggerEvent::Discarded`] self-triggers: when true the trigger
    /// fires ONLY if the discard was caused by a spell or ability controlled by
    /// an OPPONENT of the card's owner (`ValidCause$ SpellAbility.OppCtrl`).
    /// Psychic Purge — "When a spell or ability an OPPONENT controls causes you
    /// to discard this, that player loses 5 life." The firing site
    /// (`discard_card`) consults the `cause` threaded in explicitly; if that
    /// cause is absent (a discard with no spell/ability cause, e.g. the cleanup-
    /// step over-the-limit discard) or is the card's own owner (self-discard /
    /// own looting), the trigger does NOT fire.
    #[serde(default)]
    pub requires_opponent_cause: bool,

    /// When true, trigger only fires if event source is NOT a creature
    /// Replaces "[noncreature]" marker in description
    /// Example: "Whenever you cast a noncreature spell"
    #[serde(default)]
    pub requires_noncreature: bool,

    /// When true, trigger only fires if the cast spell is an instant or sorcery.
    /// Corresponds to `ValidCard$ Instant,Sorcery` on SpellCast triggers.
    /// Example: Stormchaser's Talent level 3 "Whenever you cast an instant or sorcery spell"
    #[serde(default)]
    pub requires_instant_or_sorcery: bool,

    /// When true, the SpellCast trigger fires if the cast spell is an instant
    /// (but NOT necessarily a sorcery). Corresponds to `ValidCard$ Instant`.
    /// Example: In the Eye of Chaos — "Whenever a player casts an instant spell"
    #[serde(default)]
    pub requires_instant: bool,

    /// When true, the SpellCast trigger fires if the cast spell is an enchantment.
    /// Corresponds to `ValidCard$ Enchantment`.
    /// Example: Presence of the Master — "Whenever a player casts an enchantment spell"
    #[serde(default)]
    pub requires_enchantment: bool,

    /// When true, the SpellCast trigger fires for ANY player's casts, not only
    /// the trigger source's controller. Corresponds to "whenever a player casts"
    /// (global world-enchantment triggers like In the Eye of Chaos or Presence
    /// of the Master). When false (the default), only the source controller's
    /// casts fire the trigger (Prowess, Storm, etc.).
    #[serde(default)]
    pub fires_for_any_caster: bool,

    /// When true, the trigger fires only when the event source is the
    /// permanent this trigger's card is *attached to* (`ValidSource$
    /// Card.AttachedBy`). Used by Auras/Equipment that watch the host's
    /// actions, e.g. Spirit Link's "Whenever enchanted creature deals damage,
    /// you gain that much life." The check is `attached_to == event_source`.
    #[serde(default)]
    pub requires_attached_source: bool,

    /// For `DealsCombatDamage` triggers: which combat-damage recipient class
    /// (player vs. creature vs. any) this trigger fires on. Derived from the
    /// `ValidTarget$` clause at parse time and consumed at the single
    /// combat-damage firing site (`resolve_combat_damage`), so a player-only
    /// trigger does NOT fire when the creature only damages a blocker, while
    /// Spirit Link's any-damage lifelink fires for damage to players AND
    /// creatures. Ignored for non-`DealsCombatDamage` events.
    #[serde(default)]
    pub combat_damage_target: CombatDamageTarget,

    /// For `DealsCombatDamage` triggers: when true, the trigger fires ONLY on
    /// combat damage, never on non-combat damage (`CombatDamage$ True`, e.g.
    /// Hypnotic Specter "deals COMBAT damage to a player").
    ///
    /// `DealsCombatDamage` is the shared event for both combat and non-combat
    /// "deals damage" triggers; the two have distinct firing sites
    /// (`resolve_combat_damage` for combat, `resolve_spell_execute_effects` for
    /// non-combat). The non-combat firing site (mtg-r9po1) consults this flag to
    /// skip combat-only triggers, while the combat site fires all of them.
    /// "Whenever ~ deals damage" (no COMBAT qualifier, e.g. Spirit Link's
    /// `DamageDealtOnce`) leaves this `false` so it fires on either kind.
    #[serde(default)]
    pub requires_combat_damage: bool,

    /// For AttackersDeclared triggers: keyword required on attacking creatures
    /// Corresponds to ValidAttackers$ Creature.withFlying (or other keywords)
    /// None means any attacking creature triggers it
    #[serde(default)]
    pub valid_attackers_keyword: Option<crate::core::Keyword>,

    /// Zones in which the trigger source must reside for the trigger to fire.
    ///
    /// Corresponds to `TriggerZones$`. Defaults to `[Battlefield]` (the usual
    /// case). All Hallow's Eve uses `TriggerZones$ Exile` so its upkeep trigger
    /// fires while the card sits in exile (CR 603.6e — abilities that function
    /// in a zone other than the battlefield). Empty means "any zone".
    #[serde(default)]
    pub trigger_zones: smallvec::SmallVec<[crate::zones::Zone; 2]>,

    /// Intervening-if condition: the source card must satisfy this self-state
    /// condition for the trigger to fire (CR 603.4).
    ///
    /// Corresponds to `IsPresent$ Card.Self+<filter>` (optionally combined with
    /// `PresentZone$`). Supported filters: a `counters_<CMP><N>_<TYPE>`
    /// counter-count (All Hallow's Eve: `IsPresent$ Card.Self+counters_GE1_SCREAM
    /// | PresentZone$ Exile`) and a tap-status check (Howling Mine: `IsPresent$
    /// Card.untapped` — "if CARDNAME is untapped"). None means no intervening-if
    /// check.
    #[serde(default)]
    pub present_self_condition: Option<PresentSelfCondition>,

    /// Intervening-if condition: the source card must have dealt damage to an
    /// opponent this turn for the trigger to fire (CR 603.4). Corresponds to
    /// `IsPresent$ Card.Self+dealtDamageToOppThisTurn` — Whirling Dervish's "at
    /// the beginning of each end step, if CARDNAME dealt damage to an opponent
    /// this turn, put a +1/+1 counter on it". Checked against the source card's
    /// `dealt_damage_to_opponent_this_turn` per-turn flag.
    #[serde(default)]
    pub present_self_dealt_damage_to_opponent: bool,

    /// For TapsForMana triggers: filter for the tapped permanent
    #[serde(default)]
    pub taps_for_mana_valid_card: Option<String>,

    /// For TapsForMana triggers: activator restriction (You, Opponent, Player.NonActive, etc.)
    #[serde(default)]
    pub taps_for_mana_activator: Option<String>,

    /// When true, trigger fires ONLY on opponents' turns, never on the
    /// controller's own turn. Corresponds to `ValidPlayer$ Player.Opponent`
    /// on upkeep/phase triggers. Example: Sorin, Solemn Visitor's emblem
    /// "At the beginning of each opponent's upkeep, that player sacrifices a
    /// creature." Without this flag the trigger would fire on ALL players'
    /// upkeeps including the controller's own (wrong). Mutually exclusive with
    /// `controller_turn_only` in practice — a trigger fires on your turns, the
    /// opponent's turns, or all turns.
    #[serde(default)]
    pub opponent_turn_only: bool,

    /// Mode gate for `DB$ GenericChoice`-style conditional triggers (Palace
    /// Siege). When `Some("Khans")`, the trigger only fires if the source card's
    /// `chosen_mode == Some("Khans")`; `None` means no gate (always fires).
    /// Derived from `S:Mode$ Continuous | Affected$ Card.Self+ChosenMode<X> |
    /// AddTrigger$ <SVar>` at load time.
    #[serde(default)]
    pub mode_gate: Option<String>,

    /// Intervening-if condition: trigger only fires if the defending player has
    /// more cards in hand than the attacking card's controller (CR 603.4).
    ///
    /// Corresponds to `CheckSVar$ X | SVarCompare$ GTY` where
    /// `SVar:X:Count$ValidHand Card.DefenderCtrl` and
    /// `SVar:Y:Count$ValidHand Card.YouOwn` on an Attacks trigger.
    ///
    /// Example: Robber of the Rich — "Whenever CARDNAME attacks, if defending
    /// player has more cards in hand than you, exile the top card of their library."
    #[serde(default)]
    pub requires_defender_hand_gt_controller: bool,
}

impl Trigger {
    /// Create a new trigger with trigger_self_only defaulting to true
    /// Most ETB/LTB triggers only fire for the card itself
    pub fn new(event: TriggerEvent, effects: Vec<Effect>, description: String) -> Self {
        Trigger {
            event,
            effects,
            description,
            trigger_self_only: true,           // Default: only fire for this card
            optional: false,                   // Default: mandatory trigger
            cost: None,                        // Default: no additional cost
            draw_number: None,                 // Default: trigger on any draw
            triggers_on_controller_draw: true, // Default: trigger on controller's draws
            requires_other: false,
            requires_landfall: false,
            controller_turn_only: false,
            chosen_player_turn_only: false,
            enchanted_controller_turn_only: false,
            requires_opponent_cause: false,
            requires_noncreature: false,
            requires_instant_or_sorcery: false,
            requires_instant: false,
            requires_enchantment: false,
            fires_for_any_caster: false,
            requires_attached_source: false,
            combat_damage_target: CombatDamageTarget::Any,
            requires_combat_damage: false,
            valid_attackers_keyword: None,
            trigger_zones: smallvec::SmallVec::new(),
            present_self_condition: None,
            present_self_dealt_damage_to_opponent: false,
            taps_for_mana_valid_card: None,
            taps_for_mana_activator: None,
            opponent_turn_only: false,
            mode_gate: None,
            requires_defender_hand_gt_controller: false,
        }
    }

    /// Create a new trigger that fires for any card matching the event
    pub fn new_any(event: TriggerEvent, effects: Vec<Effect>, description: String) -> Self {
        Trigger {
            event,
            effects,
            description,
            trigger_self_only: false,
            optional: false,
            cost: None,
            draw_number: None,
            triggers_on_controller_draw: true,
            requires_other: false,
            requires_landfall: false,
            controller_turn_only: false,
            chosen_player_turn_only: false,
            enchanted_controller_turn_only: false,
            requires_opponent_cause: false,
            requires_noncreature: false,
            requires_instant_or_sorcery: false,
            requires_instant: false,
            requires_enchantment: false,
            fires_for_any_caster: false,
            requires_attached_source: false,
            combat_damage_target: CombatDamageTarget::Any,
            requires_combat_damage: false,
            valid_attackers_keyword: None,
            trigger_zones: smallvec::SmallVec::new(),
            present_self_condition: None,
            present_self_dealt_damage_to_opponent: false,
            taps_for_mana_valid_card: None,
            taps_for_mana_activator: None,
            opponent_turn_only: false,
            mode_gate: None,
            requires_defender_hand_gt_controller: false,
        }
    }

    /// Create an optional trigger with a cost
    /// Used for "you may [cost]. If you do, [effect]" abilities
    pub fn new_optional_with_cost(event: TriggerEvent, effects: Vec<Effect>, description: String, cost: Cost) -> Self {
        Trigger {
            event,
            effects,
            description,
            trigger_self_only: true,
            optional: true,
            cost: Some(cost),
            draw_number: None,
            triggers_on_controller_draw: true,
            requires_other: false,
            requires_landfall: false,
            controller_turn_only: false,
            chosen_player_turn_only: false,
            enchanted_controller_turn_only: false,
            requires_opponent_cause: false,
            requires_noncreature: false,
            requires_instant_or_sorcery: false,
            requires_instant: false,
            requires_enchantment: false,
            fires_for_any_caster: false,
            requires_attached_source: false,
            combat_damage_target: CombatDamageTarget::Any,
            requires_combat_damage: false,
            valid_attackers_keyword: None,
            trigger_zones: smallvec::SmallVec::new(),
            present_self_condition: None,
            present_self_dealt_damage_to_opponent: false,
            taps_for_mana_valid_card: None,
            taps_for_mana_activator: None,
            opponent_turn_only: false,
            mode_gate: None,
            requires_defender_hand_gt_controller: false,
        }
    }

    /// Create an optional trigger without a cost
    /// Used for "you may [effect]" abilities
    pub fn new_optional(event: TriggerEvent, effects: Vec<Effect>, description: String) -> Self {
        Trigger {
            event,
            effects,
            description,
            trigger_self_only: true,
            optional: true,
            cost: None,
            draw_number: None,
            triggers_on_controller_draw: true,
            requires_other: false,
            requires_landfall: false,
            controller_turn_only: false,
            chosen_player_turn_only: false,
            enchanted_controller_turn_only: false,
            requires_opponent_cause: false,
            requires_noncreature: false,
            requires_instant_or_sorcery: false,
            requires_instant: false,
            requires_enchantment: false,
            fires_for_any_caster: false,
            requires_attached_source: false,
            combat_damage_target: CombatDamageTarget::Any,
            requires_combat_damage: false,
            valid_attackers_keyword: None,
            trigger_zones: smallvec::SmallVec::new(),
            present_self_condition: None,
            present_self_dealt_damage_to_opponent: false,
            taps_for_mana_valid_card: None,
            taps_for_mana_activator: None,
            opponent_turn_only: false,
            mode_gate: None,
            requires_defender_hand_gt_controller: false,
        }
    }
}
