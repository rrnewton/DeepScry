use serde::{Deserialize, Serialize};

use super::activated_ability::ActivatedAbility;
use super::{CasterRestriction, CountExpression, TargetRestriction};

/// Static ability that creates continuous effects
///
/// ## CR 613: Interaction of Continuous Effects
///
/// Static abilities create continuous effects that modify characteristics
/// of game objects. They are always "on" and don't use the stack.
///
/// Example from Spider-Suit:
/// ```text
/// S:Mode$ Continuous | Affected$ Creature.EquippedBy | AddPower$ 2 | AddToughness$ 2
/// ```
///
/// This creates a continuous effect in Layer 7c (MODIFYPT) that gives
/// the equipped creature +2/+2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StaticAbility {
    /// Continuous effect that modifies power/toughness
    ///
    /// Corresponds to: `S:Mode$ Continuous | AddPower$ X | AddToughness$ Y`
    /// Applied in CR 613 Layer 7c (MODIFYPT)
    ModifyPT {
        /// Selector for which cards are affected
        /// Example: "Creature.EquippedBy" = creature equipped by this Equipment
        /// Example: "Creature.YouCtrl" = creatures you control
        affected: AffectedSelector,

        /// Power bonus (can be negative)
        power: i32,

        /// Toughness bonus (can be negative)
        toughness: i32,

        /// Description for logging
        description: String,

        /// Optional condition for when this ability is active.
        /// None = always active. Example: Sedge Troll's +1/+1 is conditional
        /// on `IsPresent$ Swamp.YouCtrl` (see [`StaticCondition::ControlsPresent`]).
        condition: Option<StaticCondition>,
    },

    /// Continuous effect that grants a keyword ability
    ///
    /// Corresponds to: `S:Mode$ Continuous | AddKeyword$ Keyword`
    /// Applied in CR 613 Layer 6 (Abilities)
    ///
    /// Example: Spider-Punk grants Riot to other Spiders:
    /// `S:Mode$ Continuous | Affected$ Spider.Other+YouCtrl | AddKeyword$ Riot`
    GrantKeyword {
        /// Selector for which cards are affected
        affected: AffectedSelector,

        /// The keyword to grant
        keyword: crate::core::Keyword,

        /// Description for logging
        description: String,

        /// Optional condition for when this ability is active
        /// None = always active, Some(PlayerTurn) = only during controller's turn
        condition: Option<StaticCondition>,
    },

    /// Cost reduction static ability
    ///
    /// Corresponds to: `S:Mode$ ReduceCost | ValidCard$ X | Type$ Spell | Amount$ N`
    ///
    /// Example from Gran-Gran:
    /// `S:Mode$ ReduceCost | ValidCard$ Card.nonCreature | Type$ Spell | Activator$ You |
    ///  Amount$ 1 | IsPresent$ Lesson.YouOwn | PresentZone$ Graveyard | PresentCompare$ GE3`
    ///
    /// This reduces the cost of non-creature spells by {1} when there are 3+ Lessons
    /// in the controller's graveyard.
    ReduceCost {
        /// Which cards get the cost reduction
        /// Examples: "Card.nonCreature" = non-creature cards, "Card.Self" = only this card
        valid_card: CostReductionTarget,

        /// How much generic mana to reduce: either a compile-time constant or a
        /// `CountExpression` evaluated against the caster's game state at cast
        /// time (e.g. Eddymurk Crab: number of instants/sorceries in graveyard).
        amount: CostReductionAmount,

        /// Condition for when the reduction applies (presence checks)
        condition: Option<CostReductionCondition>,

        /// Description for logging
        description: String,
    },

    /// Cost increase static ability
    ///
    /// Corresponds to: `S:Mode$ RaiseCost | ValidCard$ X | Type$ Spell | Amount$ N`
    /// or: `S:Mode$ RaiseCost | ValidCard$ Card.Self | Cost$ Sac<X/Land>`
    ///
    /// Example from Thalia, Guardian of Thraben:
    /// `S:Mode$ RaiseCost | ValidCard$ Card.nonCreature | Type$ Spell | Amount$ 1`
    ///
    /// Example from Tectonic Split:
    /// `S:Mode$ RaiseCost | ValidCard$ Card.Self | Type$ Spell | Cost$ Sac<X/Land/land(s)>`
    /// with `SVar:X:Count$Valid Land.YouCtrl/HalfUp`
    RaiseCost {
        /// Which cards get the cost increase
        /// Examples: "Card.nonCreature" = non-creature cards, "Card.Self" = only this card
        valid_card: CostReductionTarget,

        /// The additional cost to add
        raised_cost: RaisedCost,

        /// Description for logging
        description: String,
    },

    /// Grant an activated ability to affected permanents
    ///
    /// Corresponds to: `S:Mode$ Continuous | Affected$ Land.YouCtrl | AddAbility$ AnyMana`
    /// with `SVar:AnyMana:AB$ Mana | Cost$ T | Produced$ Any | Amount$ 3`
    ///
    /// Example from Tectonic Split:
    /// Grants lands "{T}: Add three mana of any one color."
    ///
    /// Example from Chromatic Lantern:
    /// Grants lands "{T}: Add one mana of any color."
    GrantAbility {
        /// Selector for which cards are affected
        /// Example: "Land.YouCtrl" = lands you control
        affected: AffectedSelector,

        /// The ability to grant (stored as parsed ActivatedAbility)
        ability: ActivatedAbility,

        /// Description for logging
        description: String,
    },

    /// Continuous control-changing effect (CR 613.2 / layer 2).
    ///
    /// Corresponds to: `S:Mode$ Continuous | Affected$ Card.EnchantedBy | GainControl$ You`
    /// as printed on control-stealing Auras (Control Magic, Mind Control, Persuasion,
    /// Enslave, Confiscate, ...). The source Aura's controller gains control of the
    /// affected permanent for as long as the source has the static ability and the
    /// affected permanent is the source's attach target.
    ///
    /// Unlike `Effect::GainControl` (the one-shot `AB$ GainControl` of Threaten /
    /// Aladdin), this is a *continuous* effect that is re-derived every state-based
    /// check, so control reverts automatically the moment the Aura leaves the
    /// battlefield (destroyed, bounced, or the host dies) — no explicit "lose
    /// control at end of turn" bookkeeping is required.
    GainControl {
        /// Selector for which permanent is affected (typically `Card.EnchantedBy`).
        affected: AffectedSelector,

        /// Description for logging.
        description: String,
    },

    /// Continuous "destroy/sacrifice any matching permanent" sweep — the
    /// `T:Mode$ Always` state-trigger pattern (CR 603.8, applied like a
    /// state-based action). While the source permanent is on the battlefield,
    /// every battlefield permanent matching `restriction` is moved to its
    /// owner's graveyard (sacrificed). Re-checked at every state-based-action
    /// pass, so it covers BOTH the one-time on-enter sweep AND "destroy any
    /// such permanent that enters afterward" with a single rule.
    ///
    /// General machinery: City in a Bottle uses
    /// `ValidCards$ Permanent.!token+setARN+Other`, but any "whenever one or
    /// more permanents matching X are on the battlefield, sacrifice them"
    /// state-trigger maps here. The `Other` qualifier in `restriction`
    /// excludes the source itself (checked via `matches_excluding`).
    SacrificeMatchingPresent {
        /// Filter for which permanents are continuously swept.
        restriction: TargetRestriction,
        /// Description for logging.
        description: String,
    },

    /// Cast-prohibition static: spells matching `valid_card` can't be cast
    /// while the source is on the battlefield. Corresponds to
    /// `S:Mode$ CantBeCast | ValidCard$ <filter>` (City in a Bottle:
    /// `ValidCard$ Card.setARN`). General color/set/type-hoser machinery.
    ///
    /// The optional `caster_restriction` narrows who is prohibited:
    /// - `None` / `CasterRestriction::Any` — applies to everyone (all players)
    /// - `CasterRestriction::YouNonActive` — restricts the source's controller
    ///   only while they are the **non-active** player (Fires of Invention line 1)
    /// - `CasterRestriction::You` — restricts the source's controller only
    ///   (Fires of Invention line 2: NumLimitEachTurn, Form of the Squirrel)
    /// - `CasterRestriction::Opponent` — restricts opponents only
    CantBeCast {
        /// Which cards may not be cast (a card filter such as `Card.setARN`).
        valid_card: TargetRestriction,
        /// Who is prohibited from casting matching cards.
        caster_restriction: CasterRestriction,
        /// If `Some(zone)`, the prohibition only applies when the card is being
        /// cast from that specific zone (e.g. `Origin$ Hand` in Experimental
        /// Frenzy: "you can't cast spells from your hand").
        /// `None` means the restriction applies regardless of origin zone.
        origin_restriction: Option<crate::zones::Zone>,
        /// If `true`, the prohibition is lifted when the affected player IS in
        /// a sorcery window (active player, main phase, empty stack). This
        /// models Teferi, Time Raveler's static: "Each opponent can cast spells
        /// only any time they could cast a sorcery." Corresponds to
        /// `OnlySorcerySpeed$ True` in the Forge card script.
        ///
        /// Concretely: the prohibition fires if the caster_restriction matches
        /// AND the caster is NOT currently in a sorcery window.
        only_sorcery_speed: bool,
        /// Description for logging.
        description: String,
    },

    /// Land-play prohibition static: lands (and, in Forge, spells) matching
    /// `valid_card` can't be played/cast while the source is on the
    /// battlefield. Corresponds to `S:Mode$ CantPlayLand | ValidCard$ <filter>`
    /// (City in a Bottle: "can't play lands ... originally printed in ARN").
    CantPlayLand {
        /// Which cards may not be played as lands (e.g. `Card.setARN`).
        valid_card: TargetRestriction,
        /// Who is restricted from playing lands. Most uses restrict everyone
        /// (`CasterRestriction::Any`), but Experimental Frenzy uses
        /// `Player$ You` to restrict only the source's controller.
        player_restriction: CasterRestriction,
        /// If `Some(zone)`, the prohibition only applies when the land is
        /// being played from that specific zone (e.g. `Origin$ Hand` in
        /// Experimental Frenzy: "you can't play lands from your hand").
        /// `None` means the restriction applies regardless of origin zone.
        origin_restriction: Option<crate::zones::Zone>,
        /// Description for logging.
        description: String,
    },

    /// Per-creature block restriction (CR 509.1b / 509.4): the source creature
    /// (the *blocker*) can't be declared as a blocker for any attacker matching
    /// `attacker_filter`. Corresponds to the
    /// `S:Mode$ CantBlockBy | ValidAttacker$ <filter> | ValidBlocker$ Creature.Self`
    /// shape, where `ValidBlocker$ Creature.Self` pins the restriction to the
    /// source itself (Ironclaw Orcs: `ValidAttacker$ Creature.powerGE2`,
    /// "can't block creatures with power 2 or greater").
    ///
    /// This is the *blocker-side* form of `CantBlockBy`; the *evasion* form
    /// (`ValidAttacker$ Creature.Self` with no `ValidBlocker$`, meaning "this
    /// attacker can't be blocked") is a different shape and is NOT modelled here.
    CantBlockMatching {
        /// Filter for which ATTACKERS this creature may not block
        /// (e.g. `Creature.powerGE2`). Evaluated against the attacker card.
        attacker_filter: TargetRestriction,
        /// Description for logging.
        description: String,
    },

    /// Allows casting spells as though they had flash
    ///
    /// Corresponds to: `S:Mode$ CastWithFlash | ValidCard$ <filter>`
    CastWithFlash {
        /// Which cards are affected (e.g. Card.nonCreature)
        valid_card: TargetRestriction,
        /// Description for logging
        description: String,
    },

    /// Damage-increase replacement effect (CR 614.1a): when a qualifying red
    /// source controlled by this permanent's controller would deal damage to an
    /// opponent or opponent-controlled permanent, it deals that much plus
    /// `bonus` instead.
    ///
    /// Corresponds to Torbran, Thane of Red Fell's static:
    ///   `R:Event$ DamageDone | ValidSource$ Card.RedSource+YouCtrl
    ///    | ValidTarget$ Player.Opponent,Permanent.OppCtrl | ReplaceWith$ DmgPlus2`
    /// where `DmgPlus2` resolves to `ReplaceCount$DamageAmount/Plus.2`.
    ///
    /// This is deliberately narrow: it only models the "RedSource + YouCtrl →
    /// Opponent/OppCtrl target → +N" shape (the shape Torbran has). Generalising
    /// to arbitrary ValidSource/ValidTarget predicates can be done later when
    /// another card requires it.
    DamageIncrease {
        /// Extra damage to add per damage event (e.g. 2 for Torbran).
        bonus: u32,
        /// Description for logging.
        description: String,
    },

    /// Continuous damage-prevention replacement effect (CR 614.1e / 615.1):
    /// prevent all damage from sources of the chosen color to the enchanted
    /// creature.
    ///
    /// Corresponds to Prismatic Ward's static:
    ///   `R:Event$ DamageDone | Prevent$ True | ValidTarget$ Creature.EnchantedBy
    ///    | ValidSource$ Card.ChosenColor`
    ///
    /// The chosen color is stored on the Aura card at ETB time (via
    /// `K:ETBReplacement:Other:ChooseColor`). At damage resolution, if the
    /// source card's colors include the chosen color and the target creature is
    /// the enchanted creature, the damage is prevented.
    PreventDamageToEnchantedByChosenColor {
        /// Description for logging.
        description: String,
    },

    /// Attack prohibition conditional on the defending player's board state.
    ///
    /// Corresponds to Orgg's static:
    ///   `S:Mode$ CantAttack | ValidCard$ Card.Self
    ///    | UnlessDefender$ !controlsCreature.untapped+powerGE<N>`
    ///
    /// The source creature can't attack if the defending player controls at
    /// least one untapped creature whose power is >= `min_power`. This models
    /// the "can't attack unless defender has NO untapped creature with power ≥ N"
    /// restriction from CR 508.1 (attack legality).
    ///
    /// Evaluated at declare-attackers time (CR 508.1c — "the creature can't attack").
    CantAttackIfDefenderHasUntappedPowerGE {
        /// Minimum power a defending creature must have to lock out the attacker.
        min_power: i32,
        /// Description for logging.
        description: String,
    },

    /// Global attack/block prohibition for a set of creatures (CR 508.1c / 509.1b).
    ///
    /// Corresponds to `S:Mode$ CantAttack | ValidCard$ <filter>`,
    /// `S:Mode$ CantBlock | ValidCard$ <filter>`, or the combined
    /// `S:Mode$ CantAttack,CantBlock | ValidCard$ <filter>` (Light of Day).
    ///
    /// While the source permanent is on the battlefield, ALL battlefield
    /// creatures matching `filter` (regardless of controller) are prohibited
    /// from attacking (if `cant_attack`) and/or blocking (if `cant_block`).
    /// This is distinct from `CantAttackIfDefenderHasUntappedPowerGE` (Orgg),
    /// which restricts one specific creature conditionally.
    CantAttackOrBlockMatching {
        /// Attack prohibition: if true, matching creatures can't attack.
        cant_attack: bool,
        /// Block prohibition: if true, matching creatures can't block.
        cant_block: bool,
        /// Which creatures are restricted.
        filter: TargetRestriction,
        /// Description for logging.
        description: String,
    },

    /// Activated-ability lock: while the source is on the battlefield, no
    /// creature (matching `creature_filter`) may activate an activated ability.
    ///
    /// Corresponds to Cursed Totem:
    ///   `S:Mode$ CantBeActivated | ValidCard$ Creature | ValidSA$ Activated`
    ///
    /// Evaluated at action-generation time: when collecting activated abilities
    /// for a player, any activated ability on a card matching `creature_filter`
    /// is suppressed.
    CantBeActivated {
        /// Creatures whose activated abilities are suppressed.
        creature_filter: TargetRestriction,
        /// Description for logging.
        description: String,
    },

    /// Name-based activated-ability lock (Pithing Needle).
    ///
    /// Corresponds to Pithing Needle:
    ///   `S:Mode$ CantBeActivated | ValidCard$ Card.NamedCard | ValidSA$ Activated.!ManaAbility`
    ///
    /// The source card's `Card::chosen_name` field holds the chosen name (set at ETB).
    /// All non-mana activated abilities on any source whose name matches `chosen_name`
    /// are suppressed while Pithing Needle is on the battlefield.
    ///
    /// Evaluated in `GameState::is_activated_ability_prohibited_by_name`.
    CantBeActivatedByName {
        /// Description for logging.
        description: String,
    },

    /// Allows the controller to play additional lands per turn.
    ///
    /// Corresponds to: `S:Mode$ Continuous | Affected$ You | AdjustLandPlays$ N`
    ///
    /// Permanent form (on-battlefield static): Oracle of Mul Daya, Exploration enchantment,
    /// Azusa Lost but Seeking, etc. The extra plays accumulate from all such statics
    /// currently on the battlefield and controlled by the relevant player.
    ///
    /// Applied in `GameState::effective_max_lands()` which sums all `ExtraLandPlay`
    /// statics on battlefield permanents plus `PersistentEffectKind::ExtraLandPlay`
    /// for temporary grants (e.g. the Explore spell).
    ExtraLandPlay {
        /// Number of additional lands per turn (typically 1, 2 for Azusa).
        amount: u8,
        /// Description for logging.
        description: String,
    },

    /// Life-floor replacement effect (CR 614.1e): while the source is on the
    /// battlefield and the controller controls a creature, damage cannot reduce
    /// the controller's life total below 1.
    ///
    /// Corresponds to Worship:
    ///   `R:Event$ LifeReduced | ValidPlayer$ You.lifeGE1 | Result$ LT1
    ///    | IsDamage$ True | IsPresent$ Creature.YouCtrl | ReplaceWith$ ReduceLoss`
    ///
    /// Applied in `GameState::deal_damage`: before dealing damage to the
    /// controller, if they control a creature and their life is >= 1, cap
    /// the damage so life stays at 1.
    LifeFloor {
        /// Description for logging.
        description: String,
    },

    /// Damage-redirect: damage dealt to a player is replaced by that player
    /// exiling that many cards from the top of their library instead
    /// (CR 614.1a zone-change replacement).
    ///
    /// Corresponds to Crumbling Sanctuary:
    ///   `R:Event$ DamageDone | ValidTarget$ Player | ReplaceWith$ ExileTop`
    ///   `SVar:ExileTop:DB$ Dig | Defined$ ReplacedTarget | DigNum$ X
    ///          | ChangeNum$ All | DestinationZone$ Exile`
    ///
    /// Applied in `GameState::deal_damage`: all damage to any player is
    /// redirected — that player mills-to-exile that many cards instead.
    /// NonStackingEffect: only the first Sanctuary's replacement fires.
    DamageToExileLibrary {
        /// Description for logging.
        description: String,
    },

    /// Characteristic-defining P/T static (CR 613.4a, Layer 7a).
    ///
    /// Corresponds to Serra Avatar:
    ///   `S:Mode$ Continuous | CharacteristicDefining$ True | SetPower$ X
    ///    | SetToughness$ X`  with `SVar:X:Count$YourLifeTotal`
    ///
    /// The creature's power and toughness are each defined by `source`,
    /// evaluated dynamically at the game layer (not at parse time) so life
    /// total changes propagate immediately.
    ///
    /// Applied in `GameState::get_pt_breakdown`, layer 7a (characteristic_value).
    CharacteristicDefiningPt {
        /// What value to set power to.
        power_source: CdaPtSource,
        /// What value to set toughness to (often the same as `power_source`).
        toughness_source: CdaPtSource,
        /// Description for logging.
        description: String,
    },

    /// Continuous effect that grants a "sacrifice unless you pay {N}" upkeep
    /// trigger to all permanents matching `affected`.
    ///
    /// Corresponds to: `S:Mode$ Continuous | Affected$ X | AddTrigger$ UpkeepCostTrigger`
    /// where `UpkeepCostTrigger` resolves to
    ///   `Mode$ Phase | Phase$ Upkeep | ValidPlayer$ You | Execute$ TrigUpkeep`
    /// and `TrigUpkeep` resolves to `DB$ Sacrifice | UnlessPayer$ You | UnlessCost$ N`.
    ///
    /// Example: Energy Flux ("All artifacts have 'At the beginning of your upkeep,
    /// sacrifice this artifact unless you pay {2}.'").
    /// Example: Aura Flux ("Other enchantments have 'At the beginning of your upkeep,
    /// sacrifice this enchantment unless you pay {2}.'").
    ///
    /// While the source is on the battlefield, at the beginning of each player's
    /// upkeep every affected permanent's controller must pay `unless_cost` generic
    /// mana or sacrifice that permanent. Applied in `check_phase_triggers` by
    /// scanning all `GrantUpkeepSacrificeUnlessPay` statics on the battlefield.
    GrantUpkeepSacrificeUnlessPay {
        /// Filter for which permanents are affected (e.g. `Artifact`, `Enchantment.Other`).
        affected: AffectedSelector,
        /// Generic mana cost to avoid sacrifice (e.g. 2 for Energy Flux / Aura Flux).
        unless_cost: u8,
        /// Description for logging.
        description: String,
    },

    /// Alternative-cost static ability.
    ///
    /// Corresponds to: `S:Mode$ AlternativeCost | ValidSA$ Spell.Self | EffectZone$ All
    ///   | Cost$ 0 | CheckSVar$ <var> | Description$ ...`
    ///
    /// Example: Summoning Trap — may be cast for {0} instead of its normal {5}{G}{G}
    /// cost when a creature spell you cast was countered earlier this turn.
    ///
    /// `condition`: which runtime flag must be true on the casting player.
    /// `alt_cost`: the alternative mana cost (e.g. {0} = `ManaCost::zero()`).
    /// `description`: for logging/display.
    ///
    /// Applied in `push_castable_spells` (actions.rs): when the condition is met,
    /// the spell is offered a second time with `override_cost = Some(alt_cost)`.
    AlternativeCost {
        /// Runtime condition that must be satisfied for the alt cost to be offered.
        condition: AltCostCondition,
        /// The alternative mana cost to use when condition is met.
        alt_cost: crate::core::ManaCost,
        /// Description for logging/display.
        description: String,
    },

    /// Alternative-cost static that replaces the mana cost with a permanent-return
    /// cost (bounce a matching permanent to hand instead of paying mana).
    ///
    /// Corresponds to: `S:Mode$ AlternativeCost | ValidSA$ Spell.Self
    ///   | Cost$ Return<N/Type> | Description$ ...`
    ///
    /// Example: Daze — "You may return an Island you control to its owner's hand
    /// rather than pay this spell's mana cost." (CR 601.2b alt-cost).
    ///
    /// Applied in `push_castable_spells` (actions.rs): when `condition` is met and
    /// the player controls at least `count` untapped permanents matching `card_type`
    /// on the battlefield, the spell is offered as
    /// `SpellAbility::CastFromHandWithReturnCost { card_id, count, card_type }`.
    AlternativeCostReturn {
        /// Runtime condition that must be satisfied (usually `Always`).
        condition: AltCostCondition,
        /// Number of permanents to return.
        count: u8,
        /// Type filter for the permanents to return (e.g. `"Island"`).
        card_type: String,
        /// Description for logging/display.
        description: String,
    },

    /// Continuous static: while the source is on the battlefield, the controller
    /// may cast nonland spells with CMC ≤ the value of `cmc_limit_svar` without
    /// paying their mana costs (Fires of Invention, CR 702.25).
    ///
    /// Corresponds to:
    ///   `S:Mode$ Continuous | Affected$ Card.nonLand+cmcLEX | MayPlay$ True
    ///    | MayPlayWithoutManaCost$ True | AffectedZone$ Hand,...`
    /// with `SVar:X:Count$Valid Land.YouCtrl`
    ///
    /// Applied in `push_castable_spells`: cards in hand whose CMC ≤ land count
    /// are offered as `CastFromHandWithAltCost { alternative_cost: ManaCost::zero() }`.
    MayPlayWithoutManaCost {
        /// SVar name that holds the CMC limit expression (e.g. "X" →
        /// "Count$Valid Land.YouCtrl").
        cmc_limit_svar: String,
        /// Description for logging/display.
        description: String,
    },

    /// Continuous static: while the source is on the battlefield, the controller
    /// may cast or play the top card of their library (Experimental Frenzy,
    /// Future Sight, etc.; CR 702.150).
    ///
    /// Corresponds to:
    ///   `S:Mode$ Continuous | Affected$ Card.TopLibrary+YouCtrl
    ///    | AffectedZone$ Library | MayPlay$ True`
    ///
    /// Applied in `push_castable_from_library`: the top card of the controlling
    /// player's library is offered as `SpellAbility::CastFromLibrary` (for
    /// non-land spells) or `SpellAbility::PlayLandFromLibrary` (for land cards).
    MayPlayFromLibrary {
        /// Description for logging/display.
        description: String,
    },

    /// Torpor Orb: while this permanent is on the battlefield, creatures entering
    /// the battlefield don't cause triggered abilities to trigger (CR 603.6b).
    ///
    /// Corresponds to Torpor Orb's card script:
    ///   `S:Mode$ DisableTriggers | ValidCause$ Creature | ValidMode$ ChangesZone,ChangesZoneAll
    ///    | Destination$ Battlefield`
    ///
    /// Applied in `check_triggers_inner`: before firing any
    /// `TriggerEvent::EntersBattlefield` trigger, we check whether any permanent
    /// on the battlefield has this static and the entering card is a creature. If
    /// so, the trigger is suppressed (the trigger still "technically" triggers per
    /// CR 603.6b — it just doesn't go on the stack).
    ///
    /// MTG rules: CR 603.6b — "Some effects can turn off abilities. If an effect
    /// states that abilities of a permanent are turned off, that permanent loses
    /// all abilities for the duration of the effect."  Torpor Orb is the canonical
    /// example of suppressing ETB triggers across ALL creatures while it's in play.
    DisableCreatureEtbTriggers {
        /// Description for logging.
        description: String,
    },

    /// Opalescence-style continuous effect (CR 613, Layers 4 + 7b):
    /// each other non-Aura enchantment on the battlefield becomes a creature
    /// in addition to its other types, with base power and base toughness each
    /// equal to its mana value.
    ///
    /// Corresponds to Opalescence:
    ///   `S:Mode$ Continuous | Affected$ Enchantment.nonAura+Other
    ///    | SetPower$ AffectedX | SetToughness$ AffectedX | AddType$ Creature`
    ///   `SVar:AffectedX:Count$CardManaCost`
    ///
    /// Applied at two layers:
    ///  - **Layer 4** (type): `GameState::is_opalescence_creature()` returns `true`
    ///    for any non-aura enchantment while this static is in play; the attacker
    ///    and blocker collectors treat such permanents as creatures.
    ///  - **Layer 7b** (set P/T): `get_pt_breakdown()` applies `setpt_value =
    ///    Some((cmc, cmc))` for affected permanents (mana value from printed cost).
    ///
    /// MTG rules: CR 613.1a (layer 4), CR 613.4b (layer 7b), CR 110.4 (creature types).
    OpalescenceStyle {
        /// Description for logging/display.
        description: String,
    },

    /// Token-creation replacement (CR 614): when tokens would be created under
    /// this permanent's controller's control, also create additional tokens of
    /// a different script.
    ///
    /// Corresponds to:
    /// `R:Event$ CreateToken | ActiveZones$ Battlefield | ValidToken$ Card.YouCtrl
    ///  | ReplaceWith$ <svar>`
    /// where `<svar>` resolves to
    /// `DB$ ReplaceToken | Type$ AddToken | Amount$ N | TokenScript$ <script>`.
    ///
    /// Example: Donatello, the Brains — "If one or more tokens would be created
    /// under your control, those tokens plus an additional Mutagen token are
    /// created instead." (TMNT Commander set)
    TokenCreationBonus {
        /// Token script name for the additional token (e.g. `"c_a_mutagen_sac"`).
        token_script: String,
        /// Number of extra tokens to create per creation event.
        amount: u8,
        /// Human-readable description for logging.
        description: String,
    },
}

/// Condition checked at cast time for an [`StaticAbility::AlternativeCost`] or
/// [`StaticAbility::AlternativeCostReturn`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AltCostCondition {
    /// True when one of the casting player's creature spells was countered
    /// this turn by an opponent's effect (Summoning Trap condition).
    HadCreatureCounteredThisTurn,
    /// Always available (no extra condition beyond being able to pay the cost).
    /// Used by Daze: "You may return an Island you control to its owner's hand
    /// rather than pay this spell's mana cost." — CR 601.2b.
    Always,
}

/// Source expression for a CharacteristicDefiningPt static ability.
///
/// Each variant evaluates to a single integer used as power or toughness.
/// Evaluated dynamically at game-state layer-7a resolution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CdaPtSource {
    /// P/T = controller's current life total.
    /// Serra Avatar: `SVar:X:Count$YourLifeTotal`
    ControllerLifeTotal,

    /// Count of creature cards across ALL players' graveyards, with an optional
    /// arithmetic post-modifier.
    ///
    /// Lhurgoyf power: `SVar:X:Count$ValidGraveyard Creature` → modifier None.
    /// Lhurgoyf toughness: `SVar:Y:Count$ValidGraveyard Creature/Plus.1` → modifier Plus(1).
    ///
    /// Graveyard contents are public (CR 400.2), so this is information-independent
    /// and safe for network determinism.
    AllGraveyardCreatures {
        /// Arithmetic post-modifier applied to the raw count (`/Plus.N`, `/Minus.N`).
        modifier: crate::core::CountModifier,
    },
}

/// Target selector for cost reduction abilities
///
/// Specifies which cards get their costs reduced by a ReduceCost static ability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CostReductionTarget {
    /// Non-creature spells
    /// Corresponds to: `ValidCard$ Card.nonCreature`
    NonCreature,

    /// All spells (no restriction)
    /// Corresponds to: `ValidCard$ Card` or no ValidCard parameter
    AllSpells,

    /// Only this card itself, regardless of zone (EffectZone$ All).
    /// Corresponds to: `ValidCard$ Card.Self | EffectZone$ All`.
    /// The reduction is applied directly from the card being cast, not
    /// from a battlefield permanent. Used by cards that reduce their own
    /// casting cost based on game state (e.g. Eddymurk Crab).
    SelfCard,

    /// Creature spells only
    /// Corresponds to: `ValidCard$ Creature`
    Creature,

    /// Spells of a specific subtype
    /// Corresponds to: `ValidCard$ Dragon`, `ValidCard$ Spirit`, etc.
    Subtype(crate::core::Subtype),

    /// Spells of a specific color (CR 105.1 / CR 202.2)
    /// Corresponds to: `ValidCard$ Card.White`, `ValidCard$ Card.Blue`, etc.
    /// Used by colour-hate enchantments — Gloom (white), Karma (swamps),
    /// CoP-style hosers — where the effect targets any spell that is the
    /// named colour regardless of controller.
    Color(crate::core::Color),
}

/// Condition for when a cost reduction applies
///
/// Used for abilities like Gran-Gran's "as long as there are three or more
/// Lesson cards in your graveyard"
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostReductionCondition {
    /// What cards must be present (e.g., "Lesson.YouOwn")
    pub is_present: String,

    /// Which zone to check (e.g., Graveyard)
    pub present_zone: crate::zones::Zone,

    /// Minimum count required (from PresentCompare$ GE3 -> 3)
    pub min_count: u8,
}

/// Amount for a cost reduction — either a compile-time fixed generic count or
/// a `CountExpression` evaluated against the caster at cast time.
///
/// Fixed: `Amount$ 2` → reduce by exactly 2.
/// Dynamic: `Amount$ X` with `SVar:X:Count$ValidGraveyard Instant.YouOwn,Sorcery.YouOwn`
/// → reduce by the number of instants/sorceries in your graveyard (e.g. Eddymurk Crab).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CostReductionAmount {
    /// Reduce by a fixed number of generic mana.
    Fixed(u8),
    /// Reduce by a count evaluated at cast time (e.g. graveyard count).
    Dynamic(CountExpression),
}

impl CostReductionAmount {
    /// Return the fixed amount if known at load time, or `None` if dynamic.
    pub fn fixed(&self) -> Option<u8> {
        match self {
            CostReductionAmount::Fixed(n) => Some(*n),
            CostReductionAmount::Dynamic(_) => None,
        }
    }
}

/// Represents what additional cost is raised by a RaiseCost ability
///
/// Can be either a mana cost increase or a non-mana cost like sacrifice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RaisedCost {
    /// Increase generic mana cost by this amount
    /// Corresponds to: `Amount$ N` where N is a number
    Mana(u8),

    /// Sacrifice N permanents of the given type
    /// Corresponds to: `Cost$ Sac<N/Type>` or `Cost$ Sac<X/Type>`
    Sacrifice {
        /// The amount to sacrifice (fixed or variable)
        amount: RaisedCostAmount,
        /// The type of permanent to sacrifice (e.g., "Land", "Creature")
        valid_type: String,
    },
}

/// Amount for a raised cost - can be fixed or variable (X)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RaisedCostAmount {
    /// A fixed amount (e.g., Sac<1/Land>)
    Fixed(u8),
    /// A variable amount referencing an SVar (e.g., Sac<X/Land> with SVar:X:...)
    Variable(String),
}

/// Represents the type of cost for an UnlessCost condition
///
/// These correspond to the `UnlessCost$` parameter in card scripts.
/// Common patterns:
/// - `UnlessCost$ 2` - pay {2} generic mana
/// - `UnlessCost$ Discard<1/Card>` - discard 1 card
/// - `UnlessCost$ Sac<1/Creature>` - sacrifice 1 creature
/// - `UnlessCost$ PayLife<3>` - pay 3 life
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnlessCostType {
    /// Pay mana cost (e.g., "2", "1U", "X")
    Mana(crate::core::ManaCost),
    /// Discard N cards of the given type
    /// Format: `Discard<N/Type>` (e.g., Discard<1/Card>)
    Discard { count: u8, card_type: String },
    /// Sacrifice N permanents of the given type
    /// Format: `Sac<N/Type>` (e.g., Sac<1/Creature>)
    Sacrifice { count: u8, valid_type: String },
    /// Pay N life
    /// Format: `PayLife<N>`
    PayLife(u8),
    /// Reveal N cards of the given type from hand
    /// Format: `Reveal<N/Type>` (e.g., Reveal<1/Giant>)
    Reveal { count: u8, card_type: String },
    /// Return N permanents of the given type from the battlefield to their owner's hand.
    /// Format: `Return<N/Type>` (e.g., `Return<1/Island.untapped>`)
    /// Used by karoo lands (Coral Atoll, Everglades, Dormant Volcano, etc.):
    /// "sacrifice ~ unless you return a matching land you control to hand."
    ReturnToHand { count: u8, card_type: String },
}

/// Represents an UnlessCost condition that wraps an effect
///
/// In MTG Forge card scripts, this corresponds to:
/// - `UnlessCost$ <cost>` - the cost to pay
/// - `UnlessPayer$ <player>` - who pays (defaults to TargetedController)
/// - `UnlessSwitched$ True` - if present, effect executes when paid (otherwise when NOT paid)
///
/// # Examples
///
/// **Counter unless pays**: Effect executes when cost is NOT paid
/// ```text
/// DB$ Counter | UnlessCost$ 2 | UnlessPayer$ TargetedController
/// ```
///
/// **You may discard to draw**: Effect executes when cost IS paid (switched)
/// ```text
/// SP$ Draw | NumCards$ 2 | UnlessCost$ Discard<1/Card> | UnlessPayer$ You | UnlessSwitched$ True
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnlessCost {
    /// The cost to pay
    pub cost: UnlessCostType,
    /// Who pays the cost (resolved player reference)
    /// Common values: "You", "TargetedController", "Player"
    pub payer: String,
    /// If true, effect executes when paid; if false, when not paid
    pub switched: bool,
}

impl UnlessCost {
    /// Create a new UnlessCost
    pub fn new(cost: UnlessCostType, payer: &str, switched: bool) -> Self {
        Self {
            cost,
            payer: payer.to_string(),
            switched,
        }
    }
}

/// Condition for when a static ability is active
///
/// Used for abilities like "During your turn, this creature has hexproof"
/// or "Sedge Troll gets +1/+1 as long as you control a Swamp".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StaticCondition {
    /// Active only during the controller's turn
    /// Corresponds to: `Condition$ PlayerTurn`
    PlayerTurn,
    /// Active only during opponents' turns
    /// Corresponds to: `Condition$ NotPlayerTurn`
    NotPlayerTurn,
    /// Active only while the source's controller has at least `min_count`
    /// permanents (or cards in `zone`) matching `filter`.
    ///
    /// Corresponds to: `IsPresent$ <filter>` (+ optional `PresentZone$`,
    /// `PresentCompare$`). Example from Sedge Troll:
    /// `S:Mode$ Continuous | ... | IsPresent$ Swamp.YouCtrl` — only active
    /// while the controller controls a Swamp.
    ControlsPresent {
        /// Card filter, e.g. `"Swamp.YouCtrl"` (subtype `.` ownership/control).
        filter: String,
        /// Zone in which to look for matching cards (default Battlefield).
        zone: crate::zones::Zone,
        /// Minimum number of matching cards required for the condition to hold.
        min_count: u8,
    },
}

/// Comparison operator for `PresentCompare$` activation/static conditions.
///
/// Forge encodes these as `EQ7`, `GE2`, `LE3`, etc. — a two-letter operator
/// followed by a count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompareOp {
    /// `EQ` — equal to.
    Equal,
    /// `GE` — greater than or equal to.
    GreaterEqual,
    /// `LE` — less than or equal to.
    LessEqual,
    /// `GT` — strictly greater than.
    Greater,
    /// `LT` — strictly less than.
    Less,
}

impl CompareOp {
    /// Parse the two-letter Forge operator prefix (`EQ`/`GE`/`LE`/`GT`/`LT`).
    pub fn parse(prefix: &str) -> Option<Self> {
        match prefix {
            "EQ" => Some(CompareOp::Equal),
            "GE" => Some(CompareOp::GreaterEqual),
            "LE" => Some(CompareOp::LessEqual),
            "GT" => Some(CompareOp::Greater),
            "LT" => Some(CompareOp::Less),
            _ => None,
        }
    }

    /// Evaluate `actual <op> threshold`.
    pub fn matches(self, actual: usize, threshold: usize) -> bool {
        match self {
            CompareOp::Equal => actual == threshold,
            CompareOp::GreaterEqual => actual >= threshold,
            CompareOp::LessEqual => actual <= threshold,
            CompareOp::Greater => actual > threshold,
            CompareOp::Less => actual < threshold,
        }
    }
}

/// Restriction on when an activated ability may be activated, derived from
/// `IsPresent$ <filter> | PresentZone$ <zone> | PresentCompare$ <op><n>`.
///
/// "Activate only if you have exactly seven cards in hand" (Library of
/// Alexandria, Magus of the Library), "Activate only if you control two or
/// more white permanents" (Mistveil Plains), "...five or more lands"
/// (Cryptic Caves), etc. The count is over cards in `zone` matching `filter`
/// from the activating player's perspective, compared to `count` via `op`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivationCondition {
    /// Forge `IsPresent$` filter, e.g. `"Card.YouOwn"`, `"Land.YouCtrl"`,
    /// `"Permanent.White+YouCtrl"`.
    pub filter: String,
    /// Zone to count in (default Battlefield; Hand for Library of Alexandria).
    pub zone: crate::zones::Zone,
    /// Comparison operator.
    pub op: CompareOp,
    /// Threshold count.
    pub count: u8,
}

/// Restriction on the turn-step window in which an activated ability may be
/// activated, derived from `ActivationPhases$ <start>-><end>`.
///
/// "Activate only during combat" (Jade Statue's `BeginCombat->EndCombat`
/// animate, CR 602.5: an ability's activation-timing restriction is part of
/// the ability). The activating step must satisfy `start <= step <= end` in
/// turn order. Because [`Step`](crate::game::phase::Step) is declared in turn
/// order and derives `Ord`, the window is a simple inclusive range check —
/// no per-turn flag, so it is trivially rewind-safe (it reads only the
/// current step, which is reconstructed deterministically on replay).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivationPhaseWindow {
    /// First step (inclusive) at which the ability may be activated.
    pub start: crate::game::phase::Step,
    /// Last step (inclusive) at which the ability may be activated.
    pub end: crate::game::phase::Step,
}

impl ActivationPhaseWindow {
    /// True if `step` falls within `[start, end]` (inclusive), in turn order.
    pub fn contains(&self, step: crate::game::phase::Step) -> bool {
        self.start <= step && step <= self.end
    }

    /// Parse a `ActivationPhases$ <start>-><end>` value (e.g.
    /// `"BeginCombat->EndCombat"`). A bare single step (`"Upkeep"`) is treated
    /// as a one-step window. Returns `None` if either token is unrecognised.
    ///
    /// Only the single contiguous-range form is modelled here. Forge also has a
    /// *disjoint* multi-range form (`"Upkeep->Main1,Main2->Cleanup"` = "any time
    /// except combat", used by a handful of cards like Aggravated Assault). Those
    /// values contain a comma and/or more than one `->`; we return `None` for
    /// them so the loader leaves the ability unrestricted rather than mis-gating
    /// it to a wrong window. (TODO(mtg-713 B6 follow-up): model disjoint windows
    /// as a small set/vec of ranges if a championship card needs the
    /// except-combat case enforced.)
    pub fn parse(value: &str) -> Option<Self> {
        use crate::game::phase::Step;
        // Reject the disjoint multi-range form (comma list or >1 arrow).
        if value.contains(',') || value.matches("->").count() > 1 {
            return None;
        }
        let (start_tok, end_tok) = match value.split_once("->") {
            Some((s, e)) => (s, e),
            None => (value, value),
        };
        let start = Step::from_script_name(start_tok.trim())?;
        let end = Step::from_script_name(end_tok.trim())?;
        // Guard against an inverted range (would never match any step).
        if start > end {
            return None;
        }
        Some(ActivationPhaseWindow { start, end })
    }
}

/// Selector for which cards are affected by a static ability
///
/// Parsed from the `Affected$` parameter in card scripts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AffectedSelector {
    /// The creature equipped by this Equipment
    /// Corresponds to: `Affected$ Creature.EquippedBy`
    CreatureEquippedBy,

    /// Creatures you control
    /// Corresponds to: `Affected$ Creature.YouCtrl`
    CreaturesYouControl,

    /// Other creatures you control (excluding self)
    /// Corresponds to: `Affected$ Creature.YouCtrl+Other`
    /// Used by cards like Elesh Norn that grant bonuses to "other creatures you control"
    CreaturesYouControlOther,

    /// All creatures
    /// Corresponds to: `Affected$ Creature`
    AllCreatures,

    /// All creatures (any controller) whose current power is >= the threshold.
    /// Corresponds to: `ValidCard$ Creature.powerGE<N>` (e.g. Meekstone's
    /// `Creature.powerGE3` doesn't-untap lock). Controller-agnostic: power is
    /// the creature's current (effective) power, evaluated continuously.
    CreaturesWithPowerGE(i32),

    /// This card itself
    /// Corresponds to: `Affected$ Card.Self`
    Self_,

    /// The land this Aura is attached to
    /// Corresponds to: `Affected$ Land.AttachedBy`
    /// Used by Auras that grant abilities to enchanted lands (e.g., Friendly Neighborhood)
    LandAttachedBy,

    /// Single creature type you control (tribal lords)
    /// Corresponds to: `Affected$ Goblin.YouCtrl`, `Affected$ Zombie.YouCtrl`, etc.
    /// Used by tribal lord cards that grant bonuses to a single creature type
    CreatureTypeYouControl {
        /// The creature subtype (e.g., "Goblin", "Zombie")
        subtype: crate::core::Subtype,
    },

    /// Single creature type you control, excluding self
    /// Corresponds to: `Affected$ Goblin.Other+YouCtrl`
    /// Used by tribal lord cards that exclude themselves from the bonus
    CreatureTypeOtherYouControl {
        /// The creature subtype (e.g., "Goblin", "Zombie")
        subtype: crate::core::Subtype,
    },

    /// Multiple creature types you control, excluding self
    /// Corresponds to: `Affected$ Spider.Other+YouCtrl,Boar.Other+YouCtrl,...`
    /// Used by cards like Spider-Ham that grant bonuses to multiple creature types
    /// The `Other` qualifier excludes the source card itself
    CreatureTypesOtherYouControl {
        /// List of creature subtypes (e.g., ["Spider", "Boar", "Goat", ...])
        types: Vec<crate::core::Subtype>,
    },

    /// The creature enchanted by this Aura
    /// Corresponds to: `Affected$ Creature.EnchantedBy`
    CreatureEnchantedBy,

    /// Artifact creatures you control, excluding self
    /// Corresponds to: `Affected$ Creature.Artifact+Other+YouCtrl`
    /// Used by cards like Master of Etherium that buff artifact creatures
    CreatureCardTypeOtherYouControl {
        /// The card type (e.g., "Artifact")
        card_type: crate::core::CardType,
    },

    /// Artifact creatures you control, including self
    /// Corresponds to: `Affected$ Creature.Artifact+YouCtrl`
    CreatureCardTypeYouControl {
        /// The card type (e.g., "Artifact")
        card_type: crate::core::CardType,
    },

    /// Land creatures you control
    /// Corresponds to: `Affected$ Creature.Land+YouCtrl`
    /// Used by cards that grant abilities to animated lands
    LandCreaturesYouControl,

    /// Non-human creatures you control, excluding self
    /// Corresponds to: `Affected$ Creature.nonHuman+Other+YouCtrl`
    /// Used by cards like Mikaeus, the Unhallowed
    CreatureNonTypeOtherYouControl {
        /// The creature subtype to exclude (e.g., "Human")
        excluded_subtype: crate::core::Subtype,
    },

    /// This card itself when equipped
    /// Corresponds to: `Affected$ Card.Self+equipped`
    /// Used by cards like Leonin Lightbringer, Kitesail Apprentice
    SelfWhenEquipped,

    /// This card itself when enchanted
    /// Corresponds to: `Affected$ Card.Self+enchanted`
    /// Used by cards like Thran Golem, Flaring Flame-Kin
    SelfWhenEnchanted,

    /// Creatures you control that are equipped
    /// Corresponds to: `Affected$ Creature.YouCtrl+equipped`
    /// Used by cards like Kemba, Kha Enduring
    EquippedCreaturesYouControl,

    /// Creatures you control that are enchanted
    /// Corresponds to: `Affected$ Creature.YouCtrl+enchanted`
    /// Used by cards like Sphere of Safety
    EnchantedCreaturesYouControl,

    /// All creatures of a specific type (global, not just yours)
    /// Corresponds to: `Affected$ Sliver`, `Affected$ Creature.Sliver`, `Affected$ Permanent.Sliver`
    /// Used by Sliver lords that affect ALL Slivers on the battlefield (both players)
    AllCreaturesOfType {
        /// The creature subtype (e.g., "Sliver")
        subtype: crate::core::Subtype,
    },

    /// The controller of this permanent (You)
    /// Corresponds to: `Affected$ You`
    /// Used by cards that grant abilities or effects to their controller
    /// Example: Absolute Virtue grants Protection to you
    You,

    /// All players in the game
    /// Corresponds to: `Affected$ Player`
    /// Used by symmetrical effects that affect all players equally
    Player,

    /// Lands you control
    /// Corresponds to: `Affected$ Land.YouCtrl`
    /// Used by cards like Chromatic Lantern that grant abilities to your lands
    LandsYouControl,

    /// Instants you control
    /// Corresponds to: `Affected$ Instant.YouCtrl`
    InstantYouControl,

    /// Sorceries you control
    /// Corresponds to: `Affected$ Sorcery.YouCtrl`
    SorceryYouControl,

    /// Opponent's creatures
    /// Corresponds to: `Affected$ Creature.OppCtrl`
    /// Used by cards that debuff or affect enemy creatures
    CreaturesOpponentControls,

    /// Top card of your library
    /// Corresponds to: `Affected$ Card.TopLibrary+YouCtrl`
    /// Used by cards that let you look at or play the top card of your library
    /// Example: Courser of Kruphix, Garruk's Horde
    TopCardOfLibrary,

    /// Creature with something attached to it
    /// Corresponds to: `Affected$ Creature.AttachedBy`
    /// Used by Auras and Equipment that grant bonuses to the attached creature
    CreatureAttachedBy,

    /// Artifacts you control
    /// Corresponds to: `Affected$ Artifact.YouCtrl`
    /// Used by cards that grant bonuses to your artifacts
    ArtifactsYouControl,

    /// Other artifacts you control (excluding self)
    /// Corresponds to: `Affected$ Artifact.YouCtrl+Other` or `Artifact.Other+YouCtrl`
    /// Used by cards like Master of Etherium that affect other artifacts
    ArtifactsYouControlOther,

    /// All lands on the battlefield
    /// Corresponds to: `Affected$ Land`
    /// Used by global land effects (e.g., mass land animation)
    AllLands,

    /// Permanents you control
    /// Corresponds to: `Affected$ Permanent.YouCtrl`
    /// Used by cards that affect all your permanents regardless of type
    PermanentsYouControl,

    /// Token creatures you control
    /// Corresponds to: `Affected$ Creature.token+YouCtrl`
    /// Used by cards that buff token creatures specifically
    TokenCreaturesYouControl,

    /// Token creatures of a specific type you control.
    ///
    /// Corresponds to: `Affected$ Zombie.token+YouCtrl`, `Affected$ Spirit.token+YouCtrl`
    /// Used by cards that specifically buff token creatures of a certain type
    TokenCreatureTypeYouControl {
        /// The creature subtype (e.g., "Zombie", "Spirit")
        subtype: crate::core::Subtype,
    },

    /// Attacking creatures you control
    /// Corresponds to: `Affected$ Creature.attacking+YouCtrl`
    /// Used by cards that buff your attacking creatures
    AttackingCreaturesYouControl,

    /// All attacking creatures (regardless of controller)
    /// Corresponds to: `Affected$ Creature.attacking`
    /// Used by cards that affect all attackers
    AllAttackingCreatures,

    /// Opponent player(s)
    /// Corresponds to: `Affected$ Opponent`
    /// Used by effects that target or affect opponents
    Opponent,

    /// This card itself when attacking
    /// Corresponds to: `Affected$ Card.Self+attacking`
    /// Used by cards like Soltari Lancer that gain abilities while attacking
    SelfWhenAttacking,

    /// The artifact enchanted by this Aura
    /// Corresponds to: `Affected$ Artifact.EnchantedBy`
    /// Used by Auras that attach to artifacts (e.g., Splinter)
    ArtifactEnchantedBy,

    /// The planeswalker enchanted by this Aura
    /// Corresponds to: `Affected$ Planeswalker.EnchantedBy`
    /// Used by Auras that attach to planeswalkers
    PlaneswalkerEnchantedBy,

    /// The equipment enchanted by this Aura
    /// Corresponds to: `Affected$ Equipment.EnchantedBy`
    /// Used by Auras that attach to equipment
    EquipmentEnchantedBy,

    /// The land enchanted by this Aura
    /// Corresponds to: `Affected$ Land.EnchantedBy`
    /// Used by Auras that attach to lands (e.g., Squirrel Nest)
    LandEnchantedBy,

    /// Any permanent this Aura/Equipment is attached to
    /// Corresponds to: `Affected$ Card.AttachedBy`
    /// Used by generic Auras that can enchant any permanent type
    /// More generic than Creature.AttachedBy or Land.AttachedBy
    CardAttachedBy,

    /// Lands you own (not just control)
    /// Corresponds to: `Affected$ Land.YouOwn`
    /// Used by cards like Crucible of Worlds that let you play lands from graveyard
    LandsYouOwn,

    /// This card itself when untapped.
    ///
    /// Corresponds to: `Affected$ Card.Self+untapped`
    /// Used by cards that get bonuses while untapped (e.g., Wall of Roots +0/+3)
    SelfWhenUntapped,

    /// This card itself when monstrous (Monstrosity has been activated).
    ///
    /// Corresponds to: `Affected$ Card.Self+IsMonstrous`
    /// Used by cards with Monstrosity that gain abilities when monstrous
    SelfWhenMonstrous,

    /// Self when renowned (has +1/+1 counters from Renown ability)
    ///
    /// Corresponds to: `Affected$ Card.Self+IsRenowned`
    /// Used by cards with Renown that gain abilities when renowned
    SelfWhenRenowned,

    /// Tapped creatures you control, other than self.
    ///
    /// Corresponds to: `Affected$ Creature.tapped+YouCtrl+Other`
    /// Used by cards that benefit from or affect tapped creatures
    TappedCreaturesYouControlOther,

    /// Untapped creatures you control, other than self.
    ///
    /// Corresponds to: `Affected$ Creature.untapped+YouCtrl+Other`
    /// Used by cards that benefit from or affect untapped creatures
    UntappedCreaturesYouControlOther,

    /// Non-land permanents you control.
    ///
    /// Corresponds to: `Affected$ Card.YouCtrl+nonLand`, `Affected$ Permanent.nonLand+YouCtrl`
    /// Used by cards that affect all non-land permanents you control
    NonLandPermanentsYouControl,

    /// Non-land cards you own (in any zone).
    ///
    /// Corresponds to: `Affected$ Card.YouOwn+nonLand`
    /// Used by cards that affect non-land cards you own
    NonLandCardsYouOwn,

    /// OR combination of multiple selectors (matches if ANY selector matches).
    ///
    /// Corresponds to comma-separated Affected$ values like:
    /// - `Affected$ Goblin.YouCtrl+Other,Orc.YouCtrl+Other` (tribal lords)
    /// - `Affected$ Instant,Sorcery` (spell type OR)
    /// - `Affected$ Creature.PairedWith,Creature.Self+Paired` (soulbond)
    ///
    /// Used when a card affects multiple distinct categories of permanents.
    Any(Vec<AffectedSelector>),

    /// All permanents on the battlefield.
    ///
    /// Corresponds to: `Affected$ Permanent`
    /// Used by effects that affect all permanents regardless of type or controller
    AllPermanents,

    /// All cards (any zone, any controller).
    ///
    /// Corresponds to: `Affected$ Card`
    /// Used by very broad effects that can affect cards in any zone
    AllCards,

    /// Cards you control (on the battlefield).
    ///
    /// Corresponds to: `Affected$ Card.YouCtrl`
    /// Used by effects that affect all your permanents
    CardsYouControl,

    /// Cards owned by opponents.
    ///
    /// Corresponds to: `Affected$ Card.OppOwn`
    /// Used by effects that affect cards owned by opponents
    CardsOpponentOwns,

    /// This card itself when it has a minimum number of a specific counter type.
    ///
    /// Corresponds to: `Affected$ Card.Self+counters_GE*_TYPE`
    /// Examples:
    /// - `Card.Self+counters_GE8_CHARGE` (at least 8 charge counters)
    /// - `Card.Self+counters_GE1_P1P1` (at least 1 +1/+1 counter)
    ///
    /// Used by cards that gain abilities when they have enough counters.
    SelfWithCounters {
        /// The counter type (e.g., "CHARGE", "P1P1", "DIVINITY")
        counter_type: String,
        /// The minimum number of counters required
        minimum: u32,
    },

    /// Non-basic lands (either you control or all).
    ///
    /// Corresponds to: `Affected$ Land.nonBasic`, `Affected$ Land.nonBasic+YouCtrl`
    /// Used by effects that affect non-basic lands
    NonBasicLands,

    /// Creatures of a specific color, other than self.
    ///
    /// Corresponds to: `Affected$ Creature.Black+Other`, `Affected$ Creature.White+Other`
    /// Used by cards that buff creatures of a specific color excluding themselves
    CreatureColorOther {
        /// The color name (e.g., "Black", "White", "Blue")
        color: String,
    },

    /// All creatures of a specific color (including self).
    ///
    /// Corresponds to: `Affected$ Creature.White`, `Affected$ Creature.Black`, etc.
    /// Used by cards like Crusade that buff all creatures of a color
    AllCreaturesOfColor {
        /// The color name (e.g., "Black", "White", "Blue")
        color: String,
    },

    /// Humans equipped by this equipment.
    ///
    /// Corresponds to: `Affected$ Human.EquippedBy`
    /// Used by equipment that specifically grants bonuses to equipped Humans
    HumanEquippedBy,

    /// Cards that entered the battlefield this turn (usually self).
    ///
    /// Corresponds to: `Affected$ Card.Self+ThisTurnEntered`
    /// Used by cards that have effects when they ETB
    SelfThisTurnEntered,

    /// Card exiled with this source (imprint, exile-based effects).
    ///
    /// Corresponds to: `Affected$ Card.ExiledWithSource`
    /// Used by imprint effects like Chrome Mox, Isochron Scepter
    CardExiledWithSource,

    /// Top card of library (generic, any player).
    ///
    /// Corresponds to: `Affected$ Card.TopLibrary`
    /// Used by Future Sight-like effects
    TopOfLibrary,

    /// Land card on top of your library.
    ///
    /// Corresponds to: `Affected$ Land.TopLibrary+YouCtrl`
    /// Used by effects that let you play lands from the top of your library
    LandTopOfLibrary,

    /// Non-land creature on top of your library.
    ///
    /// Corresponds to: `Affected$ Creature.TopLibrary+YouCtrl+nonLand`
    /// Used by effects that care about creature cards on top of library
    CreatureTopOfLibraryNonLand,

    /// Commander you control.
    ///
    /// Corresponds to: `Affected$ Card.IsCommander+YouCtrl`
    /// Used by Commander-specific cards
    CommanderYouControl,

    /// Creature equipped by a legendary equipment.
    ///
    /// Corresponds to: `Affected$ Card.EquippedBy+Legendary`
    /// Used by legendary equipment with special abilities
    EquippedByLegendary,

    /// Top card of library you own.
    ///
    /// Corresponds to: `Affected$ Card.TopLibrary+YouOwn`
    /// Used by effects that affect cards you own on top of library
    TopOfLibraryYouOwn,

    /// Any permanent this is attached to (generic).
    ///
    /// Corresponds to: `Affected$ Permanent.AttachedBy`
    /// Used by generic auras/equipment that affect any permanent type
    PermanentAttachedBy,

    /// Non-creature artifacts.
    ///
    /// Corresponds to: `Affected$ Artifact.nonCreature`
    /// Used by effects that only affect non-creature artifacts
    ArtifactsNonCreature,

    /// All artifacts.
    ///
    /// Corresponds to: `Affected$ Artifact`
    /// Used by effects that affect all artifacts regardless of controller
    AllArtifacts,

    /// Basic lands you control.
    ///
    /// Corresponds to: `Affected$ Land.Basic+YouCtrl`
    /// Used by effects that affect basic lands you control
    BasicLandsYouControl,

    /// Specific basic land type (e.g., Mountain, Forest, Island).
    ///
    /// Corresponds to: `Affected$ Mountain`, `Affected$ Forest`, etc.
    /// Used by effects that affect specific land types
    SpecificLandType {
        /// The land type name (e.g., "Mountain", "Island")
        land_type: String,
    },

    /// Non-land cards with CMC at most X.
    ///
    /// Corresponds to: `Affected$ Card.nonLand+cmcLEX`
    /// Used by effects that care about converted mana cost
    NonLandCmcLE {
        /// The maximum CMC (often X, which would be resolved at runtime)
        max_cmc: i32,
    },

    /// Creature of a specific type with flying that opponent controls.
    ///
    /// Corresponds to: `Affected$ Creature.withFlying+OppCtrl`
    /// Used by effects that target flying creatures opponents control
    CreatureWithFlyingOppCtrl,

    /// Other creatures of a specific type (zombies, etc.) you control.
    ///
    /// Corresponds to: `Affected$ Creature.Zombie+Other`
    /// Used by zombie lords and similar effects
    CreatureTypeOther {
        /// The creature subtype
        subtype: crate::core::Subtype,
    },

    /// Slivers you control (specific handling).
    ///
    /// Corresponds to: `Affected$ Permanent.Sliver+YouCtrl`
    /// Used by Slivers that only affect your own Slivers
    SliversYouControl,

    /// Equipment attached to a permanent.
    ///
    /// Corresponds to: `Affected$ Permanent.EquippedBy`
    /// Used by effects that care about equipped permanents
    PermanentEquippedBy,

    /// Vehicles this is attached to.
    ///
    /// Corresponds to: `Affected$ Vehicle.AttachedBy`
    /// Used by crew-related effects
    VehicleAttachedBy,

    /// Non-land cards you own without Foretell.
    ///
    /// Corresponds to: `Affected$ Card.nonLand+YouOwn+withoutForetell`
    /// Used by Foretell-related effects
    NonLandCardsYouOwnWithoutForetell,

    /// Non-land cards on top of library.
    ///
    /// Corresponds to: `Affected$ Card.TopLibrary+YouOwn+nonLand`
    /// Used by effects that care about non-land cards on top
    TopOfLibraryNonLand,

    /// Remembered cards (from imprint or other memory effects).
    ///
    /// Corresponds to: `Affected$ Card.IsRemembered`
    /// Used by effects that reference previously imprinted/remembered cards
    RememberedCards,

    /// Creature cards that were cast (not put into play).
    ///
    /// Corresponds to: `Affected$ Card.Creature+YouCtrl+wasCast`
    /// Used by effects that care about cast vs. put into play
    CreatureYouControlWasCast,

    /// Cards of a specific type that you own.
    ///
    /// Corresponds to: `Affected$ Instant.YouOwn`, `Affected$ Sorcery.YouOwn`, etc.
    /// Used by flashback-granting effects like Snapcaster Mage's ability
    /// or cards that let you cast spells from graveyard
    CardTypeYouOwn {
        /// The card type (e.g., Instant, Sorcery, Aura, Equipment)
        card_type: crate::core::CardType,
    },

    /// Cards of a specific subtype that you own.
    ///
    /// Corresponds to: `Affected$ Aura.YouOwn`, `Affected$ Equipment.YouOwn`
    /// where Aura/Equipment are subtypes, not card types.
    /// Used by effects that grant flashback or graveyard casting
    SubtypeYouOwn {
        /// The subtype (e.g., "Aura", "Equipment", "Merfolk", "Druid")
        subtype: crate::core::Subtype,
    },

    /// Card type on top of your library.
    ///
    /// Corresponds to: `Affected$ Instant.TopLibrary+YouCtrl`, `Affected$ Sorcery.TopLibrary+YouCtrl`
    /// Used by effects that let you cast specific card types from top of library
    CardTypeTopLibrary {
        /// The card type (e.g., Instant, Sorcery)
        card_type: crate::core::CardType,
    },

    /// Subtype on top of your library (non-land).
    ///
    /// Corresponds to: `Affected$ Angel.TopLibrary+YouCtrl+nonLand`, `Affected$ Human.TopLibrary+YouCtrl+nonLand`
    /// Used by effects that let you cast specific creature types from top of library
    SubtypeTopLibraryNonLand {
        /// The creature subtype (e.g., "Angel", "Human")
        subtype: crate::core::Subtype,
    },

    /// Permanent of a specific subtype you control.
    ///
    /// Corresponds to: `Affected$ Permanent.Servo+YouCtrl`, `Affected$ Permanent.Thopter+YouCtrl`
    /// Used by effects that buff specific permanent types
    PermanentSubtypeYouControl {
        /// The subtype (e.g., "Servo", "Thopter")
        subtype: crate::core::Subtype,
    },

    /// Creature equipped by this equipment, if it has a specific subtype.
    ///
    /// Corresponds to: `Affected$ Card.EquippedBy+Human`, `Affected$ Card.EquippedBy+Angel`
    /// Used by equipment that grants bonuses to specific creature types
    EquippedBySubtype {
        /// The required subtype (e.g., "Human", "Angel")
        subtype: crate::core::Subtype,
    },

    /// Non-creature artifacts you control.
    ///
    /// Corresponds to: `Affected$ Artifact.nonCreature+YouCtrl`
    /// Used by effects that affect non-creature artifacts you control
    ArtifactsNonCreatureYouControl,

    /// Other artifact creatures you control.
    ///
    /// Corresponds to: `Affected$ Artifact.Creature+YouCtrl+Other`
    /// Used by cards like Master of Etherium
    ArtifactCreaturesYouControlOther,

    /// Treasure tokens/permanents you control.
    ///
    /// Corresponds to: `Affected$ Card.Treasure+YouCtrl`
    /// Used by cards that buff or care about Treasures
    TreasuresYouControl,

    /// Cards you control that were cast (not put onto battlefield).
    ///
    /// Corresponds to: `Affected$ Card.YouCtrl+wasCast`
    /// Used by effects that care about cast vs ETB
    CardsYouControlWasCast,

    /// Self card on top of library.
    ///
    /// Corresponds to: `Affected$ Card.Self+TopLibrary`
    /// Used by top-of-library casting effects on self
    SelfTopLibrary,

    /// Instant spells of a specific color you control.
    ///
    /// Corresponds to: `Affected$ Instant.Red+YouCtrl`, `Affected$ Instant.Green+YouCtrl`
    /// Used by effects that grant abilities to colored instants
    InstantColorYouControl {
        /// The color (e.g., "Red", "Green")
        color: String,
    },

    /// Sorcery spells of a specific color you control.
    ///
    /// Corresponds to: `Affected$ Sorcery.Red+YouCtrl`, `Affected$ Sorcery.Green+YouCtrl`
    /// Used by effects that grant abilities to colored sorceries
    SorceryColorYouControl {
        /// The color (e.g., "Red", "Green")
        color: String,
    },

    /// Card type with subtype on top of library.
    ///
    /// Corresponds to: `Affected$ Card.TopLibrary+YouCtrl+Bird`, `Affected$ Card.TopLibrary+YouCtrl+Land`
    /// Used by effects that let you play specific types from top of library
    TopLibraryWithSubtype {
        /// The subtype filter (e.g., "Bird", "Land")
        subtype: crate::core::Subtype,
    },

    /// Permanents opponent controls.
    ///
    /// Corresponds to: `Affected$ Permanent.OppCtrl`
    /// Used by effects that debuff or affect enemy permanents
    PermanentsOpponentControls,

    /// Attacking creatures of a specific type you control.
    ///
    /// Corresponds to: `Affected$ Vampire.attacking+YouCtrl`, `Affected$ Pirate.attacking+YouCtrl`
    /// Used by tribal cards that grant bonuses to attacking creatures of a type
    AttackingCreatureTypeYouControl {
        /// The creature subtype (e.g., "Vampire", "Pirate")
        subtype: crate::core::Subtype,
    },

    /// Legendary creatures or permanents.
    ///
    /// Corresponds to: `Affected$ Creature.Legendary+YouCtrl`, `Affected$ Permanent.Legendary+YouCtrl`
    /// Used by effects that affect legendary permanents
    LegendaryYouControl,

    /// Other legendary permanents you control.
    ///
    /// Corresponds to: `Affected$ Permanent.Other+YouCtrl+Legendary`
    /// Used by effects that buff other legendaries
    LegendaryOtherYouControl,

    /// Equipped creatures of a specific type you control.
    ///
    /// Corresponds to: `Affected$ Warrior.YouCtrl+equipped`, `Affected$ Knight.YouCtrl+equipped`
    /// Used by equipment-matters tribal effects
    EquippedCreatureTypeYouControl {
        /// The creature subtype (e.g., "Warrior", "Knight")
        subtype: crate::core::Subtype,
    },

    /// Legendary creatures of a specific type you control.
    ///
    /// Corresponds to: `Affected$ Human.YouCtrl+Legendary`, `Affected$ Snake.Legendary+YouCtrl`
    /// Used by legendary-matters tribal effects
    LegendarySubtypeYouControl {
        /// The creature subtype (e.g., "Human", "Snake")
        subtype: crate::core::Subtype,
    },

    /// Other non-aura enchantments.
    ///
    /// Corresponds to: `Affected$ Enchantment.nonAura+Other`
    /// Used by cards that care about non-aura enchantments (excluding self)
    NonAuraEnchantmentsOther,

    /// This card itself when tapped.
    ///
    /// Corresponds to: `Affected$ Card.Self+tapped`
    /// Used by cards that gain abilities or stats when tapped
    SelfWhenTapped,

    /// This card itself if it was cast (not put onto battlefield).
    ///
    /// Corresponds to: `Affected$ Card.Self+wasCast`
    /// Used by effects that care about whether the card was cast
    SelfWhenCast,

    /// Enchantments you control.
    ///
    /// Corresponds to: `Affected$ Card.Enchantment+YouCtrl`, `Affected$ Enchantment.YouCtrl`
    /// Used by effects that affect your enchantments
    EnchantmentsYouControl,

    /// Historic permanents you control (legendary, artifact, or saga).
    ///
    /// Corresponds to: `Affected$ Card.Historic+YouCtrl`
    /// Used by effects that care about historic cards
    HistoricYouControl,

    /// Historic permanents you own (any zone).
    ///
    /// Corresponds to: `Affected$ Card.Historic+YouOwn`
    /// Used by effects that grant flashback or graveyard access to historic cards
    HistoricYouOwn,

    /// Card subtype with Other+YouCtrl pattern (e.g., `Card.Human+Other+YouCtrl`).
    ///
    /// Corresponds to: `Affected$ Card.Human+Other+YouCtrl`, etc.
    /// Different from creature-specific tribal lords - this is Card-prefixed
    CardSubtypeOtherYouControl {
        /// The subtype (e.g., "Human", "Merfolk")
        subtype: crate::core::Subtype,
    },

    /// Card subtype with YouCtrl pattern (e.g., `Card.Horror+YouCtrl`).
    ///
    /// Corresponds to: `Affected$ Card.Horror+YouCtrl`, etc.
    CardSubtypeYouControl {
        /// The subtype (e.g., "Horror", "Satyr")
        subtype: crate::core::Subtype,
    },

    /// Permanent subtype with Other+YouCtrl pattern (e.g., `Permanent.Dwarf+Other+YouCtrl`).
    ///
    /// Corresponds to: `Affected$ Permanent.Dwarf+Other+YouCtrl`, etc.
    /// For permanents (not just creatures) of a type
    PermanentSubtypeOtherYouControl {
        /// The subtype (e.g., "Dwarf", "Elf")
        subtype: crate::core::Subtype,
    },

    /// This card itself when NOT attacking.
    ///
    /// Corresponds to: `Affected$ Card.Self+!attacking`
    /// Used by cards that have abilities when not attacking
    SelfWhenNotAttacking,

    /// This card itself when NOT attacking and NOT blocking.
    ///
    /// Corresponds to: `Affected$ Card.Self+!attacking+!blocking`
    /// Used by cards that have abilities when not in combat
    SelfWhenNotInCombat,

    /// Artifact permanents that are not tokens.
    ///
    /// Corresponds to: `Affected$ Artifact.!token+YouCtrl`
    /// Used by effects that only affect non-token artifacts
    NonTokenArtifactsYouControl,

    /// Artifacts that are not legendary.
    ///
    /// Corresponds to: `Affected$ Card.Artifact+nonLegendary+YouCtrl`
    /// Used by effects that only affect non-legendary artifacts
    NonLegendaryArtifactsYouControl,

    /// Cards that were cast from exile.
    ///
    /// Corresponds to: `Affected$ Card.YouCtrl+wasCastFromExile`
    /// Used by exile-casting effects (foretell, suspend, etc.)
    CardsYouControlCastFromExile,

    /// Commander you own (any zone).
    ///
    /// Corresponds to: `Affected$ Card.IsCommander+YouOwn`
    /// Used by Commander-specific effects
    CommanderYouOwn,

    /// Elf creatures other than self.
    ///
    /// Corresponds to: `Affected$ Card.Elf+Other`
    /// Used by elf lords that affect all elves
    SubtypeOther {
        /// The subtype (e.g., "Elf", "Merfolk")
        subtype: crate::core::Subtype,
    },
}
