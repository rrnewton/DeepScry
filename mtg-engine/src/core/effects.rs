//! Card effects and ability system

use crate::core::{CardId, Keyword, PlayerId};
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

/// Target reference for effects
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetRef {
    /// Target a player
    Player(PlayerId),
    /// Target a creature or other permanent
    Permanent(CardId),
    /// No target (e.g., "each player", "all creatures")
    None,
}

/// Controller restriction for targeting
///
/// Used by spells like Cackling Counterpart ("target creature you control")
/// or Ember Island Production modes to restrict targets by controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ControllerRestriction {
    /// Target can be controlled by anyone (no restriction)
    #[default]
    Any,
    /// Target must be controlled by the spell/ability's controller
    YouCtrl,
    /// Target must be controlled by an opponent
    OppCtrl,
}

/// Types of permanents that can be targeted
///
/// Used by spells like Disenchant (Artifact, Enchantment) or Terror (Creature)
/// to restrict what can be legally targeted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetType {
    /// Any permanent (no restriction)
    Any,
    /// Must be an artifact
    Artifact,
    /// Must be an enchantment
    Enchantment,
    /// Must be a creature
    Creature,
    /// Must be a land
    Land,
    /// Must be a planeswalker
    Planeswalker,
}

impl TargetType {
    /// Check if a card matches this target type restriction
    pub fn matches(&self, card: &crate::core::Card) -> bool {
        match self {
            TargetType::Any => true,
            TargetType::Artifact => card.is_artifact(),
            TargetType::Enchantment => card.is_enchantment(),
            TargetType::Creature => card.is_creature(),
            TargetType::Land => card.is_land(),
            TargetType::Planeswalker => card.is_planeswalker(),
        }
    }
}

/// Restrictions on what types of permanents can be targeted
///
/// For spells like Disenchant ("destroy target artifact or enchantment"),
/// this would contain [Artifact, Enchantment].
/// For Terror ("destroy target creature"), this would contain [Creature].
/// An empty vec means any permanent can be targeted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetRestriction {
    /// Valid target types (if empty, any permanent is valid)
    pub types: SmallVec<[TargetType; 2]>,
    /// If true, target must have no counters on it (e.g., Heartless Act mode 1)
    #[serde(default)]
    pub requires_no_counters: bool,
    /// Controller restriction (e.g., YouCtrl, OppCtrl)
    #[serde(default)]
    pub controller: ControllerRestriction,
    /// Minimum power requirement (e.g., powerGE4 means power >= 4)
    #[serde(default)]
    pub power_ge: Option<i32>,
    /// Maximum power requirement (e.g., powerLE2 means power <= 2)
    #[serde(default)]
    pub power_le: Option<i32>,
}

impl TargetRestriction {
    /// Create a restriction allowing any permanent
    pub fn any() -> Self {
        Self {
            types: SmallVec::new(),
            requires_no_counters: false,
            controller: ControllerRestriction::Any,
            power_ge: None,
            power_le: None,
        }
    }

    /// Create a restriction from a list of target types
    pub fn from_types(types: impl IntoIterator<Item = TargetType>) -> Self {
        Self {
            types: types.into_iter().collect(),
            requires_no_counters: false,
            controller: ControllerRestriction::Any,
            power_ge: None,
            power_le: None,
        }
    }

    /// Check if a card matches this restriction (type, counter, and power checks)
    ///
    /// Returns true if:
    /// - types is empty (any permanent allowed), OR card matches at least one of the specified types
    /// - requires_no_counters is false, OR card has no counters
    /// - power_ge is None, OR card's power >= power_ge
    /// - power_le is None, OR card's power <= power_le
    ///
    /// Note: This does NOT check controller restrictions. Use `matches_with_controller`
    /// for full validation including controller checks.
    pub fn matches(&self, card: &crate::core::Card) -> bool {
        // Check counter restriction first
        if self.requires_no_counters && card.has_counters() {
            return false;
        }

        // Check power restrictions (for creatures)
        if let Some(min_power) = self.power_ge {
            if i32::from(card.current_power()) < min_power {
                return false;
            }
        }
        if let Some(max_power) = self.power_le {
            if i32::from(card.current_power()) > max_power {
                return false;
            }
        }

        // Check type restriction
        if self.types.is_empty() {
            return true; // No type restriction
        }
        self.types.iter().any(|t| t.matches(card))
    }

    /// Check if a card matches this restriction including controller checks
    ///
    /// # Arguments
    /// * `card` - The target card to check
    /// * `spell_controller` - The controller of the spell/ability
    /// * `target_controller` - The controller of the target card
    ///
    /// Returns true if all restrictions match:
    /// - Type restriction matches
    /// - Counter restriction matches
    /// - Controller restriction matches (YouCtrl/OppCtrl/Any)
    pub fn matches_with_controller(
        &self,
        card: &crate::core::Card,
        spell_controller: PlayerId,
        target_controller: PlayerId,
    ) -> bool {
        // Check type and counter restrictions
        if !self.matches(card) {
            return false;
        }

        // Check controller restriction
        match self.controller {
            ControllerRestriction::Any => true,
            ControllerRestriction::YouCtrl => target_controller == spell_controller,
            ControllerRestriction::OppCtrl => target_controller != spell_controller,
        }
    }

    /// Parse ValidTgts string from Java Forge format
    ///
    /// Examples:
    /// - "Artifact,Enchantment" -> [Artifact, Enchantment]
    /// - "Creature" -> [Creature]
    /// - "Creature.YouCtrl" -> [Creature] with YouCtrl controller restriction
    /// - "Creature.OppCtrl" -> [Creature] with OppCtrl controller restriction
    /// - "Creature.nonArtifact+nonBlack" -> [Creature] (modifiers ignored for now)
    /// - "Creature.!HasCounters" -> [Creature] with requires_no_counters=true
    /// - "Creature.powerGE4" -> [Creature] with power_ge=4
    /// - "Creature.powerLE2" -> [Creature] with power_le=2
    pub fn parse(valid_tgts: &str) -> Self {
        let mut types = SmallVec::new();
        let mut requires_no_counters = false;
        let mut controller = ControllerRestriction::Any;
        let mut power_ge = None;
        let mut power_le = None;

        for part in valid_tgts.split(',') {
            // Check for modifiers after the base type
            // Example: "Creature.YouCtrl" or "Creature.Other+YouCtrl+powerLE2"
            let parts: Vec<&str> = part.split('.').collect();
            let base_type = parts.first().map(|s| s.trim()).unwrap_or("");

            // Check for modifiers (may be combined with +)
            for modifier_part in parts.iter().skip(1) {
                // Split by + to handle combined modifiers like "Other+YouCtrl"
                for modifier in modifier_part.split('+') {
                    match modifier {
                        "!HasCounters" => requires_no_counters = true,
                        "YouCtrl" => controller = ControllerRestriction::YouCtrl,
                        "OppCtrl" => controller = ControllerRestriction::OppCtrl,
                        m if m.starts_with("powerGE") => {
                            // Parse powerGE4 -> power_ge = 4
                            if let Ok(n) = m.trim_start_matches("powerGE").parse::<i32>() {
                                power_ge = Some(n);
                            }
                        }
                        m if m.starts_with("powerLE") => {
                            // Parse powerLE2 -> power_le = 2
                            if let Ok(n) = m.trim_start_matches("powerLE").parse::<i32>() {
                                power_le = Some(n);
                            }
                        }
                        _ => {} // Other modifiers ignored for now
                    }
                }
            }

            match base_type {
                "Artifact" => types.push(TargetType::Artifact),
                "Enchantment" => types.push(TargetType::Enchantment),
                "Creature" => types.push(TargetType::Creature),
                "Land" => types.push(TargetType::Land),
                "Planeswalker" => types.push(TargetType::Planeswalker),
                // "Any", "Permanent", or unrecognized - allow any
                _ => {}
            }
        }

        Self {
            types,
            requires_no_counters,
            controller,
            power_ge,
            power_le,
        }
    }
}

/// Basic card effects that can be executed
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Effect {
    /// Deal damage to a target
    /// Example: "Lightning Bolt deals 3 damage to any target"
    DealDamage { target: TargetRef, amount: i32 },

    /// Draw cards
    /// Example: "Draw a card"
    DrawCards { player: PlayerId, count: u8 },

    /// Looting effect (discard then draw)
    /// Example: "Discard a card, then draw a card"
    /// Corresponds to: AB$ Draw | Cost$ Discard<N/Card> (requires discarding N cards first)
    Loot {
        player: PlayerId,
        discard_count: u8,
        draw_count: u8,
    },

    /// Discard cards
    /// Example: "Discard a card"
    /// Corresponds to: DB$ Discard | Defined$ You | NumCards$ 1
    DiscardCards { player: PlayerId, count: u8 },

    /// Gain life
    /// Example: "You gain 3 life"
    GainLife { player: PlayerId, amount: i32 },

    /// Destroy a permanent
    /// Example: "Destroy target creature" or "Destroy target artifact or enchantment"
    DestroyPermanent {
        target: CardId,
        /// Restriction on what types can be targeted (e.g., [Artifact, Enchantment] for Disenchant)
        restriction: TargetRestriction,
    },

    /// Tap a permanent
    /// Example: "Tap target creature"
    TapPermanent { target: CardId },

    /// Untap a permanent
    /// Example: "Untap target land"
    UntapPermanent { target: CardId },

    /// Pump (temporary stat boost and/or keyword grant) until end of turn
    /// Example: "Target creature gets +3/+3 until end of turn"
    /// Example with keyword: "Target creature gains double strike until end of turn"
    PumpCreature {
        target: CardId,
        power_bonus: i32,
        toughness_bonus: i32,
        /// Keywords to grant (e.g., Double Strike from KW$ parameter)
        keywords_granted: smallvec::SmallVec<[Keyword; 2]>,
    },

    /// Pump all creatures matching a filter until end of turn
    /// Example: "Creatures you control get +1/+0 until end of turn"
    PumpAllCreatures {
        controller: PlayerId,
        /// Filter string like "Creature.YouCtrl" or "Creature"
        filter: String,
        power_bonus: i32,
        toughness_bonus: i32,
    },

    /// Mill cards from library to graveyard
    /// Example: "Target player mills 3 cards"
    Mill { player: PlayerId, count: u8 },

    /// Scry - look at top N cards and put any number on bottom
    /// Example: "Scry 1" or "Scry 2"
    /// Corresponds to: DB$ Scry | ScryNum$ N
    ///
    /// AI heuristic: Keep spells, put excess lands on bottom
    Scry { player: PlayerId, count: u8 },

    /// Counter a spell on the stack
    /// Example: "Counter target spell"
    CounterSpell { target: CardId },

    /// Add mana to a player's mana pool
    /// Example: "Add {G}" or "Add {C}{C}"
    AddMana {
        player: PlayerId,
        mana: crate::core::ManaCost,
        /// If true, this ability also produces mana of the card's chosen color
        /// (for cards like Thriving lands that have "Produced$ Combo G Chosen")
        produces_chosen_color: bool,
    },

    /// Put counters on a permanent
    /// Example: "Put a +1/+1 counter on target creature"
    PutCounter {
        target: CardId,
        counter_type: crate::core::CounterType,
        amount: u8,
    },

    /// Remove counters from a permanent
    /// Example: "Remove a +1/+1 counter from target creature"
    /// When counter_type is None, removes counters of any type (CounterType$ Any)
    RemoveCounter {
        target: CardId,
        /// None means "any counter type" (CounterType$ Any)
        counter_type: Option<crate::core::CounterType>,
        amount: u8,
    },

    /// Exile a permanent
    /// Example: "Exile target creature" (Swords to Plowshares)
    /// Moves a card from the battlefield to the exile zone
    ExilePermanent { target: CardId },

    /// Search library for a card and put it into a zone
    /// Example: "Search your library for a basic land card, put it onto the battlefield tapped, then shuffle"
    /// Corresponds to: AB$ ChangeZone | Origin$ Library | Destination$ Battlefield | ChangeType$ Land.Basic
    SearchLibrary {
        /// Player whose library to search
        player: PlayerId,
        /// Card type filter (e.g., "Land.Basic", "Creature", "Land")
        card_type_filter: String,
        /// Destination zone for the found card
        destination: crate::zones::Zone,
        /// Whether the card enters tapped (for battlefield)
        enters_tapped: bool,
        /// Whether to shuffle after searching
        shuffle: bool,
    },

    /// Attach Equipment to target creature
    /// Example: Spider-Suit's Equip ability
    /// Corresponds to: K:Equip:3
    /// The source_equipment field is filled in when the ability is activated
    AttachEquipment {
        /// The Equipment to attach (filled in during activation)
        source_equipment: CardId,
        /// Target creature to attach to
        target_creature: CardId,
    },

    /// Create token(s) under a player's control
    /// Example: Spider-Ham creates a Food token
    /// Corresponds to: DB$ Token | TokenAmount$ 1 | TokenScript$ c_a_food_sac | TokenOwner$ You
    /// When for_each_player is true, corresponds to: TokenOwner$ Player (each player creates tokens)
    CreateToken {
        /// Player who will control the tokens (ignored if for_each_player is true)
        controller: PlayerId,
        /// Token script name (e.g., "c_a_food_sac" for Food token)
        token_script: String,
        /// Number of tokens to create
        amount: u8,
        /// If true, each player creates the tokens (TokenOwner$ Player)
        for_each_player: bool,
    },

    /// Create a token that's a copy of an existing permanent
    /// Example: Cackling Counterpart, Ember Island Production
    /// Corresponds to: DB$ CopyPermanent | ValidTgts$ Creature.YouCtrl | SetPower$ 4 | AddTypes$ Hero
    ///
    /// Creates a token with the same characteristics as the target permanent,
    /// optionally with modifications (different P/T, additional types, etc.)
    CopyPermanent {
        /// The permanent to copy
        target: CardId,
        /// Player who will control the token
        controller: PlayerId,
        /// If true, remove Legendary supertype from the copy
        non_legendary: bool,
        /// Override the copy's power (None = use original)
        set_power: Option<i32>,
        /// Override the copy's toughness (None = use original)
        set_toughness: Option<i32>,
        /// Types to add to the copy (e.g., ["Hero"], ["Coward"])
        add_types: Vec<String>,
        /// Number of copies to create (default 1)
        num_copies: u8,
        /// Target restriction from ValidTgts$ (e.g., Creature.YouCtrl, Creature.OppCtrl)
        restriction: TargetRestriction,
    },

    /// Balance effect - equalizes a type of permanent/cards across all players
    /// Example: "Each player sacrifices creatures until all players control the same number"
    /// Corresponds to: SP$ Balance | Valid$ Creature/Land | Zone$ Battlefield/Hand
    ///
    /// The spell controller's card type and zone define what to balance.
    /// Each player must sacrifice/discard down to match the player with the fewest.
    ///
    /// SubAbility chaining: After this Balance effect resolves, the sub_ability (if any)
    /// is looked up in the card's SVars and executed. This enables Balance's full
    /// Land → Hand → Creature chain.
    Balance {
        /// What type of card to balance ("Creature", "Land", or empty for any permanent)
        card_type: String,
        /// Zone to balance ("Battlefield" or "Hand")
        zone: String,
        /// Optional SubAbility$ reference (SVar name to execute after this effect)
        sub_ability: Option<String>,
    },

    /// Set base power and toughness until end of turn
    /// Example: Flexible Waterbender - "This creature has base power and toughness 5/2 until end of turn"
    /// Corresponds to: AB$ Animate | Defined$ Self | Power$ 5 | Toughness$ 2
    /// Also: AB$ Animate | Defined$ Self | Power$ 4 | Keywords$ Trample
    ///
    /// This effect sets the creature's base P/T (not a modifier), which then has +1/+1 counters added on top.
    /// The effect lasts until end of turn.
    /// Power and Toughness are optional - None means "don't change".
    /// Keywords can be granted along with P/T changes.
    SetBasePowerToughness {
        target: CardId,
        power: Option<i32>,
        toughness: Option<i32>,
        /// Keywords to grant (e.g., Trample from Keywords$ parameter)
        keywords_granted: smallvec::SmallVec<[Keyword; 2]>,
    },

    /// Airbend: Exile a permanent and grant its owner permission to cast it for {2}.
    ///
    /// Avatar set mechanic (CR 701.65b). Effect:
    /// "Exile [target]. While it's exiled, its owner may cast it for {2} rather than its mana cost."
    ///
    /// Corresponds to: `DB$ Airbend | ValidTgts$ Creature`
    ///
    /// Implementation:
    /// 1. Exile the target permanent
    /// 2. Create a PersistentEffect (MayPlayFromExile) that grants cast permission
    /// 3. The effect is cleaned up when the card leaves exile or is cast
    ///
    /// Cards using this:
    /// - Aang, the Last Airbender: ETB airbends nonland permanent
    /// - Monk Gyatso: Triggered on targeting other creatures
    /// - Glider Staff: ETB airbend creature
    /// - Airbender Ascension: ETB airbend creature
    Airbend {
        /// The permanent to airbend (will be exiled)
        target: CardId,
    },

    /// Earthbend: Target land becomes a 0/0 creature with haste, put N +1/+1 counters.
    ///
    /// Avatar set mechanic (CR 701.65a). Effect:
    /// "Target land you control becomes a 0/0 creature with haste that's still a land.
    /// Put N +1/+1 counters on it. When it dies or is exiled, return it to the
    /// battlefield tapped."
    ///
    /// Corresponds to: `DB$ Earthbend | Num$ 8`
    ///
    /// Implementation:
    /// 1. Add Creature type to the land (permanently)
    /// 2. Set base power/toughness to 0/0
    /// 3. Add Haste keyword
    /// 4. Put N +1/+1 counters on it
    /// 5. Create a DelayedTrigger for return-to-battlefield on death/exile
    ///
    /// Cards using this:
    /// - Avatar Kyoshi, Earthbender: "earthbend 8, then untap that land"
    /// - Bumi, Unleashed: "earthbend 4"
    /// - Badgermole: "earthbend 2"
    Earthbend {
        /// The land to earthbend (becomes a creature)
        target: CardId,
        /// Number of +1/+1 counters to put on the land
        num_counters: u8,
    },

    /// Firebend: Add red mana to combat mana pool (lasts until end of combat).
    ///
    /// Avatar set mechanic. Effect: "Add N {R}. This mana lasts until end of combat."
    ///
    /// Corresponds to: `DB$ Mana | CombatMana$ True | Produced$ R | Amount$ N`
    /// or keyword `K:Firebending:N`
    ///
    /// Implementation:
    /// 1. Add N red mana to the player's combat_mana_pool
    /// 2. The combat mana is cleared at end of combat (in end_combat_step)
    ///
    /// Cards using this:
    /// - Firebending Student: "Firebending X, where X is this creature's power"
    /// - Azula, Ruthless Firebender: "Firebending 1"
    /// - Fire Nation Cadets: "Firebending 1"
    Firebend {
        /// The player who gets the mana
        controller: PlayerId,
        /// Amount of red mana to add
        amount: u8,
    },

    /// Grant "can't be blocked" until end of turn.
    ///
    /// Effect: Target creature can't be blocked this turn.
    ///
    /// Corresponds to: `AB$ Effect | StaticAbilities$ Unblockable | RememberObjects$ Targeted`
    /// (with SVar: `Mode$ CantBlockBy | ValidAttacker$ Card.IsRemembered`)
    ///
    /// Implementation:
    /// 1. Create a PersistentEffect (CantBeBlocked) for the target creature
    /// 2. The effect is cleaned up at end of turn
    ///
    /// Cards using this:
    /// - Deserter's Disciple: "Another target creature you control with power 2 or less can't be blocked this turn."
    GrantCantBeBlocked {
        /// The creature that can't be blocked
        target: CardId,
    },

    /// Modal spell choice - player selects modes from multiple predefined effects.
    ///
    /// Example: Heartless Act - "Choose one — Destroy target creature with no counters on it;
    ///                           or Remove up to three counters from target creature."
    /// Corresponds to: A:SP$ Charm | Choices$ Destroy,Remove
    ///
    /// During resolution, the controller is prompted to choose modes, then the selected
    /// modes' effects are resolved in order. Each mode has its own targeting requirements.
    ///
    /// Cards using this:
    /// - Heartless Act, Abzan Charm, Cryptic Command, Commands, etc.
    ModalChoice {
        /// The available modes the player can choose from.
        /// Each is a tuple of (effect, description, SVar name).
        /// The SVar name is used to look up targeting info.
        modes: SmallVec<[ModalMode; 4]>,

        /// Number of modes to select (e.g., 1 for "Choose one", 2 for "Choose two")
        num_to_choose: u8,

        /// Minimum number of modes to select (default = num_to_choose)
        min_to_choose: u8,

        /// Whether the same mode can be chosen multiple times
        can_repeat_modes: bool,
    },

    /// Dig: Exile top N cards from opponents' libraries.
    ///
    /// Effect: Look at the top N cards of each opponent's library, exile some/all.
    ///
    /// Corresponds to: `AB$ Dig | DigNum$ N | ChangeNum$ All | Defined$ Opponent | DestinationZone$ Exile`
    ///
    /// Implementation:
    /// 1. For each opponent, look at top N cards of their library
    /// 2. Move ChangeNum cards to the destination zone (Exile)
    /// 3. Optionally grant "may play" permission (via MayPlay$ True)
    ///
    /// Cards using this:
    /// - Fire Lord Ozai: "{6}: Exile the top card of each opponent's library. Until end of turn,
    ///   you may play one of those cards without paying its mana cost." (target_self=false)
    /// - Seismic Sense: "Look at top X cards of your library. You may reveal a creature or land
    ///   and put it into your hand. Put the rest on bottom in random order." (target_self=true)
    ///
    /// TODO(mtg-0iad2): Implement "may play without paying mana cost" via persistent effects
    Dig {
        /// Number of cards to look at from each library (DigNum$)
        dig_count: u8,
        /// Number of cards to change zones (ChangeNum$ - "All" means all)
        change_count: u8,
        /// Whether ALL cards should be moved (ChangeNum$ All)
        change_all: bool,
        /// Destination zone for selected cards (Hand for most Dig, Exile for Fire Lord Ozai)
        destination: crate::zones::Zone,
        /// Whether to grant "may play" permission for exiled cards
        may_play: bool,
        /// Whether "may play" costs no mana
        may_play_without_mana_cost: bool,
        /// Whether to dig from own library (true, default) or opponents' libraries (false)
        /// Parsed from Defined$ parameter: "You"/"" = true, "Opponent" = false
        target_self: bool,
        /// Whether selecting a card is optional (Optional$ True)
        optional: bool,
        /// Whether to put non-selected cards on bottom of library in random order
        /// (RestRandomOrder$ True)
        rest_random: bool,
    },

    /// Create a delayed trigger that fires when a condition is met.
    ///
    /// Corresponds to: `SP$ DelayedTrigger | Mode$ ChangesZone | Origin$ Battlefield | Destination$ Graveyard | Execute$ TrigEffect`
    ///
    /// Example: Fatal Fissure - "Choose target creature. When that creature dies this turn, you earthbend 4."
    ///
    /// Implementation:
    /// 1. Remember the targeted card
    /// 2. Create a DelayedTrigger with the specified condition (e.g., ZoneChange from Battlefield to Graveyard)
    /// 3. When the condition is met, execute the specified effect
    /// 4. If ThisTurn$ True, the trigger expires at end of turn
    ///
    /// Cards using this:
    /// - Fatal Fissure: Delayed trigger on creature death -> earthbend 4
    CreateDelayedTrigger {
        /// The card to track (target of the spell)
        tracked_card: CardId,
        /// The condition that fires the trigger
        condition: crate::core::DelayedTriggerCondition,
        /// The effect to execute when triggered
        effect: Box<Effect>,
        /// When the trigger expires (usually EndOfTurn for ThisTurn$ True)
        expiry: Option<crate::core::DelayedTriggerExpiry>,
    },

    /// Copy a spell on the stack
    ///
    /// Corresponds to: `DB$ CopySpellAbility | Defined$ TriggeredSpellAbility | MayChooseTarget$ True`
    ///
    /// This effect is typically used as the Execute$ target of a DB$ DelayedTrigger
    /// with Mode$ SpellCast. When the trigger fires (e.g., "When you cast a Lesson spell"),
    /// this effect copies the triggering spell.
    ///
    /// Cards using this:
    /// - Jeong Jeong: "copy it and you may choose new targets for the copy"
    CopySpellAbility {
        /// Whether the player may choose new targets for the copy
        may_choose_targets: bool,
    },
}

/// Categorization of effects for targeting purposes.
///
/// Used by targeting.rs to determine what targets need to be collected for spells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectTargetCategory {
    /// Effect targets players or has no targeting requirements.
    /// Examples: DrawCards, GainLife, Mill, Scry, CreateToken
    NoTargetNeeded,

    /// Effect requires a creature or permanent target.
    /// Examples: DestroyPermanent, TapPermanent, PumpCreature, ExilePermanent
    RequiresTarget,

    /// Effect uses filters to affect multiple permanents (no explicit targeting).
    /// Examples: PumpAllCreatures
    UsesFilter,

    /// Effect contains inner effects with their own targeting (modal spells).
    /// Examples: ModalChoice
    HasInnerTargeting,
}

impl Effect {
    /// Returns the targeting category for this effect.
    ///
    /// This is used to avoid duplicating effect categorization across targeting.rs.
    /// When a new Effect variant is added, this method must be updated.
    pub fn target_category(&self) -> EffectTargetCategory {
        match self {
            // Effects targeting players or with no target
            Effect::DrawCards { .. }
            | Effect::Loot { .. }
            | Effect::DiscardCards { .. }
            | Effect::GainLife { .. }
            | Effect::Mill { .. }
            | Effect::Scry { .. }
            | Effect::AddMana { .. }
            | Effect::Balance { .. }
            | Effect::CreateToken { .. }
            | Effect::Dig { .. }
            | Effect::SearchLibrary { .. }
            | Effect::Firebend { .. }
            | Effect::CopySpellAbility { .. } => EffectTargetCategory::NoTargetNeeded,

            // Effects using filters (affect multiple permanents)
            Effect::PumpAllCreatures { .. } => EffectTargetCategory::UsesFilter,

            // Modal spells have inner targeting
            Effect::ModalChoice { .. } => EffectTargetCategory::HasInnerTargeting,

            // Effects requiring creature/permanent/spell targets
            Effect::DealDamage { .. }
            | Effect::DestroyPermanent { .. }
            | Effect::TapPermanent { .. }
            | Effect::UntapPermanent { .. }
            | Effect::PumpCreature { .. }
            | Effect::CounterSpell { .. }
            | Effect::PutCounter { .. }
            | Effect::RemoveCounter { .. }
            | Effect::ExilePermanent { .. }
            | Effect::AttachEquipment { .. }
            | Effect::CopyPermanent { .. }
            | Effect::SetBasePowerToughness { .. }
            | Effect::Airbend { .. }
            | Effect::Earthbend { .. }
            | Effect::GrantCantBeBlocked { .. }
            | Effect::CreateDelayedTrigger { .. } => EffectTargetCategory::RequiresTarget,
        }
    }

    /// Returns true if this effect needs no explicit targeting (targets players, uses filters, etc.)
    ///
    /// This is a convenience helper combining NoTargetNeeded and UsesFilter categories.
    pub fn needs_no_creature_target(&self) -> bool {
        matches!(
            self.target_category(),
            EffectTargetCategory::NoTargetNeeded | EffectTargetCategory::UsesFilter
        )
    }
}

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
    pub cost: Option<super::Cost>,

    /// For CardDrawn triggers: which draw number triggers this (e.g., 2 = "second card drawn")
    /// None means every card drawn triggers it
    pub draw_number: Option<u8>,

    /// For CardDrawn triggers: true = triggers on controller's draws, false = opponent's draws
    pub triggers_on_controller_draw: bool,
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
        }
    }

    /// Create an optional trigger with a cost
    /// Used for "you may [cost]. If you do, [effect]" abilities
    pub fn new_optional_with_cost(
        event: TriggerEvent,
        effects: Vec<Effect>,
        description: String,
        cost: super::Cost,
    ) -> Self {
        Trigger {
            event,
            effects,
            description,
            trigger_self_only: true,
            optional: true,
            cost: Some(cost),
            draw_number: None,
            triggers_on_controller_draw: true,
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
        }
    }
}

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
    },
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
}

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
}

impl AbilityCache {
    /// Create a new cache from ability description
    pub fn new(description: &str) -> Self {
        let desc_lower = description.to_lowercase();

        AbilityCache {
            // Store lowercase version
            description_lowercase: desc_lower.clone(),

            // Targeting restriction flags
            targets_tapped: desc_lower.contains("tapped"),
            targets_untapped: desc_lower.contains("untapped"),
            targets_creature: desc_lower.contains("creature"),
            targets_land: desc_lower.contains("land"),
            requires_target: desc_lower.contains("target"),
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

    /// Cache for expensive string operations (computed at creation time)
    pub cache: AbilityCache,
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
            cache,
        }
    }

    /// Create a new your-turn-only activated ability
    /// Less restrictive than sorcery speed - can be activated any time during your turn
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
            cache,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_effect_creation() {
        let player_id = PlayerId::new(1);
        let card_id = CardId::new(100);

        let damage_effect = Effect::DealDamage {
            target: TargetRef::Player(player_id),
            amount: 3,
        };

        let Effect::DealDamage { target, amount } = damage_effect else {
            panic!("Wrong effect type: expected DealDamage, got {damage_effect:?}");
        };
        assert_eq!(amount, 3);
        assert_eq!(target, TargetRef::Player(player_id));

        let draw_effect = Effect::DrawCards {
            player: player_id,
            count: 2,
        };

        let Effect::DrawCards { player, count } = draw_effect else {
            panic!("Wrong effect type: expected DrawCards, got {draw_effect:?}");
        };
        assert_eq!(player, player_id);
        assert_eq!(count, 2);

        let destroy_effect = Effect::DestroyPermanent {
            target: card_id,
            restriction: TargetRestriction::any(),
        };

        let Effect::DestroyPermanent { target, .. } = destroy_effect else {
            panic!("Wrong effect type: expected DestroyPermanent, got {destroy_effect:?}");
        };
        assert_eq!(target, card_id);
    }
}
