//! Efficient keyword storage using enumset with strongly-typed arguments
//!
//! This module provides the `KeywordSet` abstraction that stores keywords efficiently:
//! - All keywords (simple and complex) use `EnumSet<Keyword>` for O(1) membership tests
//! - Complex keyword arguments use `SmallVec<KeywordArgs, 2>` for strongly-typed parameters
//!
//! This matches the Java Forge implementation which uses `EnumSet<Keyword>` for efficient
//! keyword storage and operations.

use crate::core::{CardName, ManaCost, Subtype};
use enumset::{EnumSet, EnumSetType};
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

/// All MTG keywords (both simple and complex)
/// These are stored as a bitset using EnumSet for O(1) operations
/// Total: ~220 keywords (simple + complex variants)
#[derive(Debug, EnumSetType, Serialize, Deserialize)]
#[enumset(repr = "array")]
pub enum Keyword {
    // ===== EVERGREEN KEYWORDS (appear in most sets) =====
    Flying,
    FirstStrike,
    DoubleStrike,
    Deathtouch,
    Haste,
    Hexproof,
    Indestructible,
    Lifelink,
    Menace,
    Reach,
    Trample,
    Vigilance,
    Defender,
    Shroud,
    Flash,

    // ===== EVASION ABILITIES =====
    Fear,
    Intimidate,
    Horsemanship,
    Shadow,
    Skulk,

    // ===== PROTECTION (specific colors - parameterized Protection is in KeywordArgs) =====
    ProtectionFromRed,
    ProtectionFromBlue,
    ProtectionFromBlack,
    ProtectionFromWhite,
    ProtectionFromGreen,

    // ===== COMBAT-RELATED =====
    Banding,
    Flanking,
    Phasing,
    Wither,
    Infect,

    // ===== KEYWORD ACTIONS/ABILITIES =====
    Changeling,
    Convoke,
    Delve,
    Improvise,
    SplitSecond,
    Cascade,
    Storm,
    Gravestorm,
    Conspire,
    Retrace,
    Prowess,

    // ===== SET-SPECIFIC MECHANICS (alphabetically sorted) =====
    Aftermath,
    Ascend,
    Assist,
    Bargain,
    BattleCry,
    Cipher,
    Compleated,
    Daybound,
    Decayed,
    Demonstrate,
    Dethrone,
    Devoid,
    DoubleAgenda,
    DoubleTeam,
    Enlist,
    Epic,
    Evolve,
    Exalted,
    Exploit,
    Extort,
    ForMirrodin,
    Fuse,
    Gift,
    HiddenAgenda,
    Ingest,
    JobSelect,
    JumpStart,
    LivingMetal,
    LivingWeapon,
    Melee,
    Mentor,
    Myriad,
    Nightbound,
    Persist,
    Provoke,
    Ravenous,
    ReadAhead,
    Rebound,
    Riot,
    Soulbond,
    SpaceSculptor,
    Spree,
    StartYourEngines,
    Sunburst,
    Tiered,
    Training,
    UmbraArmor,
    Undaunted,
    Undying,
    Unleash,

    // ===== COMMANDER/MULTIPLAYER =====
    ChooseABackground,
    DoctorsCompanion,
    FriendsForever,
    PartnerSurvivors,
    PartnerFatherAndSon,
    PartnerCharacterSelect,

    // ===== MAYFLASH VARIANTS =====
    MayFlashSac,

    // ===== COMPLEX KEYWORDS (with arguments stored separately in KeywordArgs) =====
    // Keywords with cost parameters
    Madness,
    Flashback,
    Kicker,
    Cycling,
    Equip,
    Morph,
    Evoke,
    Buyback,
    Echo,
    Suspend,
    AuraSwap,
    Bestow,
    Blitz,
    CumulativeUpkeep,
    Dash,
    Disguise,
    Disturb,
    Embalm,
    Encore,
    Entwine,
    Escalate,
    Escape,
    Eternalize,
    Foretell,
    Fortify,
    Freerunning,
    Harmonize,
    LevelUp,
    MayFlashCost,
    Megamorph,
    Miracle,
    MoreThanMeetsTheEye,
    Multikicker,
    Mutate,
    Offspring,
    Outlast,
    Overload,
    Plot,
    Prowl,
    Prototype,
    Reconfigure,
    Reflect,
    Scavenge,
    Sneak,
    Specialize,
    Spectacle,
    Squad,
    Strive,
    Surge,
    Transfigure,
    Transmute,
    Unearth,
    Ward,
    Warp,
    WebSlinging,

    // Keywords with type parameters
    Enchant,
    Landwalk,
    Affinity,
    Protection,
    Offering,
    Champion,
    BandsWithOther,

    // Keywords with amount parameters
    Amplify,
    Annihilator,
    Bushido,
    Fading,
    Vanishing,
    Dredge,
    Modular,
    Absorb,
    Afflict,
    Afterlife,
    Bloodthirst,
    Casualty,
    Crew,
    Fabricate,
    Frenzy,
    Graft,
    Hideaway,
    Mobilize,
    Poisonous,
    Rampage,
    Renown,
    Ripple,
    Saddle,
    Soulshift,
    StartingIntensity,
    Station,
    Toxic,
    Tribute,

    // Keywords with cost + amount parameters
    Adapt,
    Awaken,
    Backup,
    Impending,
    Monstrosity,
    Reinforce,

    // Keywords with cost + type parameters
    Splice,
    Typecycling,

    // Keywords with amount + type parameters
    // (Amplify is already above in amount section)

    // Special complex keywords (custom structures)
    Emerge,
    Firebending,
    Ninjutsu,
    Partner,
    Haunt,
    Replicate,
    MayEffectFromOpeningHand,
    Mayhem,
    Recover,
    Visit,
    DeckLimit,
    Dungeon,

    // Saga and enchantment-related
    Chapter,
    Class,

    // ETB (Enter the battlefield) effects
    ETBReplacement,
    EtbCounter,

    // Other parameterized keywords
    HexproofFrom,
    PartnerWith,
    Companion,
    Craft,
    Devour,
}

impl Keyword {
    /// Returns true if this keyword requires arguments (is complex)
    pub fn is_complex(&self) -> bool {
        matches!(
            self,
            // Cost-based keywords
            Keyword::Madness
                | Keyword::Flashback
                | Keyword::Kicker
                | Keyword::Cycling
                | Keyword::Equip
                | Keyword::Morph
                | Keyword::Evoke
                | Keyword::Buyback
                | Keyword::Echo
                | Keyword::Suspend
                | Keyword::AuraSwap
                | Keyword::Bestow
                | Keyword::Blitz
                | Keyword::CumulativeUpkeep
                | Keyword::Dash
                | Keyword::Disguise
                | Keyword::Disturb
                | Keyword::Embalm
                | Keyword::Encore
                | Keyword::Entwine
                | Keyword::Escalate
                | Keyword::Escape
                | Keyword::Eternalize
                | Keyword::Foretell
                | Keyword::Fortify
                | Keyword::Freerunning
                | Keyword::Harmonize
                | Keyword::LevelUp
                | Keyword::MayFlashCost
                | Keyword::Megamorph
                | Keyword::Miracle
                | Keyword::MoreThanMeetsTheEye
                | Keyword::Multikicker
                | Keyword::Mutate
                | Keyword::Offspring
                | Keyword::Outlast
                | Keyword::Overload
                | Keyword::Plot
                | Keyword::Prowl
                | Keyword::Prototype
                | Keyword::Reconfigure
                | Keyword::Reflect
                | Keyword::Scavenge
                | Keyword::Sneak
                | Keyword::Specialize
                | Keyword::Spectacle
                | Keyword::Squad
                | Keyword::Strive
                | Keyword::Surge
                | Keyword::Transfigure
                | Keyword::Transmute
                | Keyword::Unearth
                | Keyword::Ward
                | Keyword::Warp
                | Keyword::WebSlinging
                // Type-based keywords
                | Keyword::Enchant
                | Keyword::Landwalk
                | Keyword::Affinity
                | Keyword::Protection
                | Keyword::Offering
                | Keyword::Champion
                | Keyword::BandsWithOther
                // Amount-based keywords
                | Keyword::Amplify
                | Keyword::Annihilator
                | Keyword::Bushido
                | Keyword::Fading
                | Keyword::Vanishing
                | Keyword::Dredge
                | Keyword::Modular
                | Keyword::Absorb
                | Keyword::Afflict
                | Keyword::Afterlife
                | Keyword::Bloodthirst
                | Keyword::Casualty
                | Keyword::Crew
                | Keyword::Fabricate
                | Keyword::Frenzy
                | Keyword::Graft
                | Keyword::Hideaway
                | Keyword::Mobilize
                | Keyword::Poisonous
                | Keyword::Rampage
                | Keyword::Renown
                | Keyword::Ripple
                | Keyword::Saddle
                | Keyword::Soulshift
                | Keyword::StartingIntensity
                | Keyword::Station
                | Keyword::Toxic
                | Keyword::Tribute
                // Cost + Amount keywords
                | Keyword::Adapt
                | Keyword::Awaken
                | Keyword::Backup
                | Keyword::Impending
                | Keyword::Monstrosity
                | Keyword::Reinforce
                // Cost + Type keywords
                | Keyword::Splice
                | Keyword::Typecycling
                // Special complex keywords
                | Keyword::Emerge
                | Keyword::Firebending
                | Keyword::Ninjutsu
                | Keyword::Partner
                | Keyword::Craft
                | Keyword::Devour
                | Keyword::Haunt
                | Keyword::Replicate
                | Keyword::MayEffectFromOpeningHand
                | Keyword::Mayhem
                | Keyword::Recover
                | Keyword::Visit
                | Keyword::DeckLimit
                | Keyword::Dungeon
                // Saga and enchantment-related
                | Keyword::Chapter
                | Keyword::Class
                // ETB effects
                | Keyword::ETBReplacement
                | Keyword::EtbCounter
                // Other parameterized
                | Keyword::HexproofFrom
                | Keyword::PartnerWith
                | Keyword::Companion
        )
    }
}

/// Strongly-typed keyword arguments for complex keywords
/// Each variant has parsed, type-safe fields instead of raw strings
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeywordArgs {
    // ===== COST-BASED KEYWORDS =====
    /// Madness cost (e.g., "Madness:1 R" → Madness { cost: ManaCost { red: 1, generic: 1, ... } })
    Madness { cost: ManaCost },
    /// Flashback cost (e.g., "Flashback:3 R")
    Flashback { cost: ManaCost },
    /// Kicker cost (e.g., "Kicker:2")
    Kicker { cost: ManaCost },
    /// Cycling cost (e.g., "Cycling:2")
    Cycling { cost: ManaCost },
    /// Equip cost (e.g., "Equip:2")
    Equip { cost: ManaCost },
    /// Morph cost (e.g., "Morph:3 G")
    Morph { cost: ManaCost },
    /// Evoke cost (e.g., "Evoke:2 G")
    Evoke { cost: ManaCost },
    /// Buyback cost (e.g., "Buyback:3")
    Buyback { cost: ManaCost },
    /// Echo cost (e.g., "Echo:2 G")
    Echo { cost: ManaCost },
    /// Suspend - time counters and cost (e.g., "Suspend:3:G" → 3 time counters, cost G)
    Suspend { time_counters: u8, cost: ManaCost },

    // ===== TYPE-BASED KEYWORDS =====
    /// Enchant type (e.g., "Enchant:Creature")
    Enchant { card_type: Subtype },
    /// Landwalk type (e.g., "Landwalk:Island")
    Landwalk { land_type: Subtype },
    /// Affinity type (e.g., "Affinity:Artifact")
    Affinity { card_type: Subtype },
    /// Protection from (e.g., "Protection:Red", "Protection:Artifacts")
    Protection { from: Subtype },
    /// Offering type (e.g., "Offering:Spirit")
    Offering { creature_type: Subtype },
    /// Champion type (e.g., "Champion:Goblin")
    Champion { creature_type: Subtype },

    // ===== AMOUNT-BASED KEYWORDS =====
    /// Amplify (e.g., "Amplify:2:Beast" → amount 2, creature type Beast)
    Amplify { amount: u8, creature_type: Subtype },
    /// Annihilator amount (e.g., "Annihilator:2")
    Annihilator { amount: u8 },
    /// Bushido amount (e.g., "Bushido:2")
    Bushido { amount: u8 },
    /// Fading counters (e.g., "Fading:3")
    Fading { counters: u8 },
    /// Vanishing counters (e.g., "Vanishing:3")
    Vanishing { counters: u8 },
    /// Dredge amount (e.g., "Dredge:3")
    Dredge { amount: u8 },
    /// Modular counters (e.g., "Modular:2")
    Modular { counters: u8 },
    /// Absorb amount (e.g., "Absorb:2")
    Absorb { amount: u8 },
    /// Afflict amount (e.g., "Afflict:2")
    Afflict { amount: u8 },
    /// Afterlife amount (e.g., "Afterlife:1")
    Afterlife { amount: u8 },
    /// Bloodthirst amount (e.g., "Bloodthirst:2")
    Bloodthirst { amount: u8 },
    /// Casualty amount (e.g., "Casualty:2")
    Casualty { amount: u8 },
    /// Crew amount (e.g., "Crew:3")
    Crew { amount: u8 },
    /// Fabricate amount (e.g., "Fabricate:2")
    Fabricate { amount: u8 },
    /// Frenzy amount (e.g., "Frenzy:1")
    Frenzy { amount: u8 },
    /// Graft amount (e.g., "Graft:2")
    Graft { amount: u8 },
    /// Hideaway amount (e.g., "Hideaway:4")
    Hideaway { amount: u8 },
    /// Mobilize amount (e.g., "Mobilize:1")
    Mobilize { amount: u8 },
    /// Poisonous amount (e.g., "Poisonous:2")
    Poisonous { amount: u8 },
    /// Rampage amount (e.g., "Rampage:1")
    Rampage { amount: u8 },
    /// Renown amount (e.g., "Renown:1")
    Renown { amount: u8 },
    /// Ripple amount (e.g., "Ripple:4")
    Ripple { amount: u8 },
    /// Saddle amount (e.g., "Saddle:2")
    Saddle { amount: u8 },
    /// Soulshift amount (e.g., "Soulshift:3")
    Soulshift { amount: u8 },
    /// Starting intensity amount (e.g., "Starting intensity:3")
    StartingIntensity { amount: u8 },
    /// Station amount (e.g., "Station:5")
    Station { amount: u8 },
    /// Toxic amount (e.g., "Toxic:1")
    Toxic { amount: u8 },
    /// Tribute amount (e.g., "Tribute:2")
    Tribute { amount: u8 },

    // ===== COST-BASED KEYWORDS (additional) =====
    /// Aura swap cost (e.g., "Aura swap:2")
    AuraSwap { cost: ManaCost },
    /// Bestow cost (e.g., "Bestow:3 W")
    Bestow { cost: ManaCost },
    /// Blitz cost (e.g., "Blitz:1 R")
    Blitz { cost: ManaCost },
    /// Cumulative upkeep cost (e.g., "Cumulative upkeep:1")
    CumulativeUpkeep { cost: ManaCost },
    /// Dash cost (e.g., "Dash:1 R")
    Dash { cost: ManaCost },
    /// Disguise cost (e.g., "Disguise:2 U")
    Disguise { cost: ManaCost },
    /// Disturb cost (e.g., "Disturb:2 W")
    Disturb { cost: ManaCost },
    /// Embalm cost (e.g., "Embalm:4 W")
    Embalm { cost: ManaCost },
    /// Encore cost (e.g., "Encore:5 B")
    Encore { cost: ManaCost },
    /// Entwine cost (e.g., "Entwine:2")
    Entwine { cost: ManaCost },
    /// Escalate cost (e.g., "Escalate:2")
    Escalate { cost: ManaCost },
    /// Escape cost (e.g., "Escape:4 R R, Exile three other cards from your graveyard")
    /// TODO: Parse exile count separately
    Escape { cost: ManaCost },
    /// Eternalize cost (e.g., "Eternalize:4 U U")
    Eternalize { cost: ManaCost },
    /// Foretell cost (e.g., "Foretell:U")
    Foretell { cost: ManaCost },
    /// Fortify cost (e.g., "Fortify:3")
    Fortify { cost: ManaCost },
    /// Freerunning cost (e.g., "Freerunning:1 U B")
    Freerunning { cost: ManaCost },
    /// Harmonize cost (e.g., "Harmonize:2 G")
    Harmonize { cost: ManaCost },
    /// Level up cost (e.g., "Level up:3")
    LevelUp { cost: ManaCost },
    /// MayFlashCost (e.g., "MayFlashCost:2")
    MayFlashCost { cost: ManaCost },
    /// Megamorph cost (e.g., "Megamorph:5 G")
    Megamorph { cost: ManaCost },
    /// Miracle cost (e.g., "Miracle:W")
    Miracle { cost: ManaCost },
    /// More Than Meets the Eye cost (e.g., "More Than Meets the Eye:1 W")
    MoreThanMeetsTheEye { cost: ManaCost },
    /// Multikicker cost (e.g., "Multikicker:1 R")
    Multikicker { cost: ManaCost },
    /// Mutate cost (e.g., "Mutate:2 G U")
    Mutate { cost: ManaCost },
    /// Offspring cost (e.g., "Offspring:2")
    Offspring { cost: ManaCost },
    /// Outlast cost (e.g., "Outlast:W")
    Outlast { cost: ManaCost },
    /// Overload cost (e.g., "Overload:6 R")
    Overload { cost: ManaCost },
    /// Plot cost (e.g., "Plot:1 G")
    Plot { cost: ManaCost },
    /// Prowl cost (e.g., "Prowl:U B")
    Prowl { cost: ManaCost },
    /// Prototype cost (e.g., "Prototype:1 R")
    /// TODO: Parse power/toughness from prototype
    Prototype { cost: ManaCost },
    /// Reconfigure cost (e.g., "Reconfigure:2")
    Reconfigure { cost: ManaCost },
    /// Reflect cost (e.g., "Reflect:2 R R")
    Reflect { cost: ManaCost },
    /// Scavenge cost (e.g., "Scavenge:4 G G")
    Scavenge { cost: ManaCost },
    /// Sneak cost (e.g., "Sneak:1 U")
    Sneak { cost: ManaCost },
    /// Specialize cost (e.g., "Specialize:2")
    Specialize { cost: ManaCost },
    /// Spectacle cost (e.g., "Spectacle:U B")
    Spectacle { cost: ManaCost },
    /// Squad cost (e.g., "Squad:2")
    Squad { cost: ManaCost },
    /// Strive cost (e.g., "Strive:R")
    Strive { cost: ManaCost },
    /// Surge cost (e.g., "Surge:3 U")
    Surge { cost: ManaCost },
    /// Transfigure cost (e.g., "Transfigure:1 B")
    Transfigure { cost: ManaCost },
    /// Transmute cost (e.g., "Transmute:1 U U")
    Transmute { cost: ManaCost },
    /// Unearth cost (e.g., "Unearth:B")
    Unearth { cost: ManaCost },
    /// Ward cost (e.g., "Ward:2")
    Ward { cost: ManaCost },
    /// Warp cost (e.g., "Warp:3 B")
    Warp { cost: ManaCost },
    /// Web-slinging cost (e.g., "Web-slinging:2 U")
    WebSlinging { cost: ManaCost },

    // ===== TYPE-BASED KEYWORDS (additional) =====
    /// Bands with other type (e.g., "Bands with other:Legends")
    BandsWithOther { creature_type: Subtype },

    // ===== COST + AMOUNT KEYWORDS =====
    /// Adapt (e.g., "Adapt:3:2 G" → cost 2G, amount 3)
    Adapt { cost: ManaCost, amount: u8 },
    /// Awaken (e.g., "Awaken:3:4 U U" → cost 4UU, amount 3)
    Awaken { cost: ManaCost, amount: u8 },
    /// Backup (e.g., "Backup:1" → amount 1)
    /// NOTE: Backup in Java is KeywordWithAmount, not cost
    Backup { amount: u8 },
    /// Impending (e.g., "Impending:3:3 B" → cost 3B, time counters 3)
    Impending { cost: ManaCost, amount: u8 },
    /// Monstrosity (e.g., "Monstrosity:3:2 G" → cost 2G, amount 3)
    Monstrosity { cost: ManaCost, amount: u8 },
    /// Reinforce (e.g., "Reinforce:2:1 G" → cost 1G, amount 2)
    Reinforce { cost: ManaCost, amount: u8 },

    // ===== COST + TYPE KEYWORDS =====
    /// Splice (e.g., "Splice:Arcane:2 U" → splice onto Arcane, cost 2U)
    Splice { cost: ManaCost, card_type: Subtype },
    /// Typecycling (e.g., "Typecycling:Basic:2" → basic landcycling, cost 2)
    Typecycling { cost: ManaCost, card_type: Subtype },

    // ===== SPECIAL COMPLEX KEYWORDS =====
    /// Emerge (e.g., "Emerge:5 G G" → cost, creature type implicit)
    /// TODO: Parse creature type requirement
    Emerge { cost: ManaCost },
    /// Firebending (mana production keyword)
    /// TODO: Parse mana production amount/type
    Firebending { mana: String },
    /// Ninjutsu (e.g., "Ninjutsu:U B" → cost, zone is hand by default)
    /// TODO: Parse zone (hand vs graveyard for commander ninjutsu)
    Ninjutsu { cost: ManaCost },
    /// Partner (base keyword, not PartnerWith)
    Partner,
    /// Craft (e.g., "Craft:Exile this artifact, Exile another artifact you control")
    /// TODO: Parse craft requirements properly
    Craft { requirements: String },
    /// Devour (e.g., "Devour:2" → amount, creature types TBD)
    /// TODO: Parse creature type restrictions
    Devour { amount: u8 },

    // ===== OTHER PARAMETERIZED KEYWORDS =====
    /// Hexproof from (e.g., "Hexproof:Blue", "Hexproof:instants")
    /// TODO: Parse into Color | CardType once we have those enums
    HexproofFrom { from: String },
    /// Partner with specific card (e.g., "Partner:Regna")
    PartnerWith { card_name: CardName },
    /// Companion deck restriction
    /// TODO: Parse restriction into structured format
    Companion { restriction: String },

    // ===== SAGA AND CLASS ENCHANTMENT KEYWORDS =====
    /// Chapter (e.g., "Chapter:3:DBCantBlock,DBSearch,DBToken")
    /// TODO: Parse abilities properly
    Chapter { chapter_number: u8, abilities: String },
    /// Class (e.g., "Class:2:W:AddTrigger$ TriggerEnter")
    /// TODO: Parse level, cost, and abilities properly
    Class { level: u8, cost: String, abilities: String },

    // ===== ETB (ENTER THE BATTLEFIELD) KEYWORDS =====
    /// ETB replacement effects (e.g., "ETBReplacement:Copy:DBCopy:Optional")
    /// TODO: Parse into structured format
    ETBReplacement { effect_type: String, details: String },
    /// ETB counter (e.g., "etbCounter:P1P1:2" or "etbCounter:LOYALTY:Y:no Condition:...")
    /// TODO: Parse counter type, amount, and conditions
    EtbCounter {
        counter_type: String,
        amount: String,
        condition: String,
    },

    // ===== ADDITIONAL SPECIAL KEYWORDS =====
    /// Haunt (e.g., "Haunt:TrigDestroy")
    /// TODO: Parse trigger details
    Haunt { trigger: String },
    /// Replicate (e.g., "Replicate:tapXType<1/Horror>")
    /// TODO: Parse cost properly
    Replicate { cost: String },
    /// MayEffectFromOpeningHand (e.g., "MayEffectFromOpeningHand:ExileCard")
    /// Leyline-type effects
    MayEffectFromOpeningHand { effect: String },
    /// Mayhem (e.g., "Mayhem:2" or "Mayhem:2 R")
    /// TODO: Parse cost properly
    Mayhem { cost: String },
    /// Recover (e.g., "Recover:1 G")
    Recover { cost: ManaCost },
    /// Visit (e.g., "Visit:TrigFood")
    /// Dungeon/attraction mechanic
    Visit { trigger: String },
    /// DeckLimit (e.g., "DeckLimit:1:Megalegendary (Your deck can have only one copy of this card.)")
    DeckLimit { limit: u8, description: String },
    /// Dungeon (e.g., "Dungeon:DBPortal,DBDungeon,DBBazaar,...")
    /// Specifies dungeon rooms
    Dungeon { rooms: String },
}

impl KeywordArgs {
    /// Get the keyword that these args belong to
    pub fn keyword(&self) -> Keyword {
        match self {
            KeywordArgs::Madness { .. } => Keyword::Madness,
            KeywordArgs::Flashback { .. } => Keyword::Flashback,
            KeywordArgs::Kicker { .. } => Keyword::Kicker,
            KeywordArgs::Cycling { .. } => Keyword::Cycling,
            KeywordArgs::Equip { .. } => Keyword::Equip,
            KeywordArgs::Morph { .. } => Keyword::Morph,
            KeywordArgs::Evoke { .. } => Keyword::Evoke,
            KeywordArgs::Buyback { .. } => Keyword::Buyback,
            KeywordArgs::Echo { .. } => Keyword::Echo,
            KeywordArgs::Suspend { .. } => Keyword::Suspend,
            KeywordArgs::Enchant { .. } => Keyword::Enchant,
            KeywordArgs::Landwalk { .. } => Keyword::Landwalk,
            KeywordArgs::Affinity { .. } => Keyword::Affinity,
            KeywordArgs::Protection { .. } => Keyword::Protection,
            KeywordArgs::Offering { .. } => Keyword::Offering,
            KeywordArgs::Champion { .. } => Keyword::Champion,
            KeywordArgs::Amplify { .. } => Keyword::Amplify,
            KeywordArgs::Annihilator { .. } => Keyword::Annihilator,
            KeywordArgs::Bushido { .. } => Keyword::Bushido,
            KeywordArgs::Fading { .. } => Keyword::Fading,
            KeywordArgs::Vanishing { .. } => Keyword::Vanishing,
            KeywordArgs::Dredge { .. } => Keyword::Dredge,
            KeywordArgs::Modular { .. } => Keyword::Modular,
            KeywordArgs::Absorb { .. } => Keyword::Absorb,
            KeywordArgs::Afflict { .. } => Keyword::Afflict,
            KeywordArgs::Afterlife { .. } => Keyword::Afterlife,
            KeywordArgs::Bloodthirst { .. } => Keyword::Bloodthirst,
            KeywordArgs::Casualty { .. } => Keyword::Casualty,
            KeywordArgs::Crew { .. } => Keyword::Crew,
            KeywordArgs::Fabricate { .. } => Keyword::Fabricate,
            KeywordArgs::Frenzy { .. } => Keyword::Frenzy,
            KeywordArgs::Graft { .. } => Keyword::Graft,
            KeywordArgs::Hideaway { .. } => Keyword::Hideaway,
            KeywordArgs::Mobilize { .. } => Keyword::Mobilize,
            KeywordArgs::Poisonous { .. } => Keyword::Poisonous,
            KeywordArgs::Rampage { .. } => Keyword::Rampage,
            KeywordArgs::Renown { .. } => Keyword::Renown,
            KeywordArgs::Ripple { .. } => Keyword::Ripple,
            KeywordArgs::Saddle { .. } => Keyword::Saddle,
            KeywordArgs::Soulshift { .. } => Keyword::Soulshift,
            KeywordArgs::StartingIntensity { .. } => Keyword::StartingIntensity,
            KeywordArgs::Station { .. } => Keyword::Station,
            KeywordArgs::Toxic { .. } => Keyword::Toxic,
            KeywordArgs::Tribute { .. } => Keyword::Tribute,
            KeywordArgs::AuraSwap { .. } => Keyword::AuraSwap,
            KeywordArgs::Bestow { .. } => Keyword::Bestow,
            KeywordArgs::Blitz { .. } => Keyword::Blitz,
            KeywordArgs::CumulativeUpkeep { .. } => Keyword::CumulativeUpkeep,
            KeywordArgs::Dash { .. } => Keyword::Dash,
            KeywordArgs::Disguise { .. } => Keyword::Disguise,
            KeywordArgs::Disturb { .. } => Keyword::Disturb,
            KeywordArgs::Embalm { .. } => Keyword::Embalm,
            KeywordArgs::Encore { .. } => Keyword::Encore,
            KeywordArgs::Entwine { .. } => Keyword::Entwine,
            KeywordArgs::Escalate { .. } => Keyword::Escalate,
            KeywordArgs::Escape { .. } => Keyword::Escape,
            KeywordArgs::Eternalize { .. } => Keyword::Eternalize,
            KeywordArgs::Foretell { .. } => Keyword::Foretell,
            KeywordArgs::Fortify { .. } => Keyword::Fortify,
            KeywordArgs::Freerunning { .. } => Keyword::Freerunning,
            KeywordArgs::Harmonize { .. } => Keyword::Harmonize,
            KeywordArgs::LevelUp { .. } => Keyword::LevelUp,
            KeywordArgs::MayFlashCost { .. } => Keyword::MayFlashCost,
            KeywordArgs::Megamorph { .. } => Keyword::Megamorph,
            KeywordArgs::Miracle { .. } => Keyword::Miracle,
            KeywordArgs::MoreThanMeetsTheEye { .. } => Keyword::MoreThanMeetsTheEye,
            KeywordArgs::Multikicker { .. } => Keyword::Multikicker,
            KeywordArgs::Mutate { .. } => Keyword::Mutate,
            KeywordArgs::Offspring { .. } => Keyword::Offspring,
            KeywordArgs::Outlast { .. } => Keyword::Outlast,
            KeywordArgs::Overload { .. } => Keyword::Overload,
            KeywordArgs::Plot { .. } => Keyword::Plot,
            KeywordArgs::Prowl { .. } => Keyword::Prowl,
            KeywordArgs::Prototype { .. } => Keyword::Prototype,
            KeywordArgs::Reconfigure { .. } => Keyword::Reconfigure,
            KeywordArgs::Reflect { .. } => Keyword::Reflect,
            KeywordArgs::Scavenge { .. } => Keyword::Scavenge,
            KeywordArgs::Sneak { .. } => Keyword::Sneak,
            KeywordArgs::Specialize { .. } => Keyword::Specialize,
            KeywordArgs::Spectacle { .. } => Keyword::Spectacle,
            KeywordArgs::Squad { .. } => Keyword::Squad,
            KeywordArgs::Strive { .. } => Keyword::Strive,
            KeywordArgs::Surge { .. } => Keyword::Surge,
            KeywordArgs::Transfigure { .. } => Keyword::Transfigure,
            KeywordArgs::Transmute { .. } => Keyword::Transmute,
            KeywordArgs::Unearth { .. } => Keyword::Unearth,
            KeywordArgs::Ward { .. } => Keyword::Ward,
            KeywordArgs::Warp { .. } => Keyword::Warp,
            KeywordArgs::WebSlinging { .. } => Keyword::WebSlinging,
            KeywordArgs::BandsWithOther { .. } => Keyword::BandsWithOther,
            KeywordArgs::Adapt { .. } => Keyword::Adapt,
            KeywordArgs::Awaken { .. } => Keyword::Awaken,
            KeywordArgs::Backup { .. } => Keyword::Backup,
            KeywordArgs::Impending { .. } => Keyword::Impending,
            KeywordArgs::Monstrosity { .. } => Keyword::Monstrosity,
            KeywordArgs::Reinforce { .. } => Keyword::Reinforce,
            KeywordArgs::Splice { .. } => Keyword::Splice,
            KeywordArgs::Typecycling { .. } => Keyword::Typecycling,
            KeywordArgs::Emerge { .. } => Keyword::Emerge,
            KeywordArgs::Firebending { .. } => Keyword::Firebending,
            KeywordArgs::Ninjutsu { .. } => Keyword::Ninjutsu,
            KeywordArgs::Partner => Keyword::Partner,
            KeywordArgs::Craft { .. } => Keyword::Craft,
            KeywordArgs::Devour { .. } => Keyword::Devour,
            KeywordArgs::HexproofFrom { .. } => Keyword::HexproofFrom,
            KeywordArgs::PartnerWith { .. } => Keyword::PartnerWith,
            KeywordArgs::Companion { .. } => Keyword::Companion,
            KeywordArgs::Chapter { .. } => Keyword::Chapter,
            KeywordArgs::Class { .. } => Keyword::Class,
            KeywordArgs::ETBReplacement { .. } => Keyword::ETBReplacement,
            KeywordArgs::EtbCounter { .. } => Keyword::EtbCounter,
            KeywordArgs::Haunt { .. } => Keyword::Haunt,
            KeywordArgs::Replicate { .. } => Keyword::Replicate,
            KeywordArgs::MayEffectFromOpeningHand { .. } => Keyword::MayEffectFromOpeningHand,
            KeywordArgs::Mayhem { .. } => Keyword::Mayhem,
            KeywordArgs::Recover { .. } => Keyword::Recover,
            KeywordArgs::Visit { .. } => Keyword::Visit,
            KeywordArgs::DeckLimit { .. } => Keyword::DeckLimit,
            KeywordArgs::Dungeon { .. } => Keyword::Dungeon,
        }
    }
}

/// Efficient keyword storage combining bitset membership with strongly-typed arguments
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeywordSet {
    /// All keywords stored as a bitset (O(1) membership for both simple and complex)
    keywords: EnumSet<Keyword>,
    /// Arguments for complex keywords (SmallVec avoids allocation for ≤2 complex keywords)
    /// This is a performance optimization: most cards have 0-2 complex keywords
    args: SmallVec<[KeywordArgs; 2]>,
}

impl KeywordSet {
    /// Create an empty keyword set
    pub fn new() -> Self {
        Self {
            keywords: EnumSet::new(),
            args: SmallVec::new(),
        }
    }

    /// Check if a keyword is present (O(1) bitset check)
    pub fn contains(&self, keyword: Keyword) -> bool {
        self.keywords.contains(keyword)
    }

    /// Add a simple keyword (no arguments)
    pub fn insert(&mut self, keyword: Keyword) {
        debug_assert!(!keyword.is_complex(), "Use insert_complex for keywords with arguments");
        self.keywords.insert(keyword);
    }

    /// Add a complex keyword with its arguments
    pub fn insert_complex(&mut self, args: KeywordArgs) {
        let keyword = args.keyword();
        self.keywords.insert(keyword);
        self.args.push(args);
    }

    /// Remove a keyword (both simple and complex)
    pub fn remove(&mut self, keyword: Keyword) {
        self.keywords.remove(keyword);
        // Also remove args if this was a complex keyword
        if keyword.is_complex() {
            self.args.retain(|args| args.keyword() != keyword);
        }
    }

    /// Get arguments for a complex keyword
    /// Returns None if the keyword is not present or is not complex
    pub fn get_args(&self, keyword: Keyword) -> Option<&KeywordArgs> {
        if !keyword.is_complex() || !self.keywords.contains(keyword) {
            return None;
        }
        self.args.iter().find(|args| args.keyword() == keyword)
    }

    /// Iterate over all keywords (both simple and complex)
    pub fn iter(&self) -> impl Iterator<Item = Keyword> + '_ {
        self.keywords.iter()
    }

    /// Iterate over all keyword arguments
    pub fn iter_args(&self) -> impl Iterator<Item = &KeywordArgs> + '_ {
        self.args.iter()
    }

    /// Get the number of keywords (simple + complex)
    pub fn len(&self) -> usize {
        self.keywords.len()
    }

    /// Check if the set is empty
    pub fn is_empty(&self) -> bool {
        self.keywords.is_empty()
    }

    /// Clear all keywords
    pub fn clear(&mut self) {
        self.keywords.clear();
        self.args.clear();
    }
}

impl Default for KeywordSet {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keyword_set_simple() {
        let mut set = KeywordSet::new();
        assert!(set.is_empty());

        set.insert(Keyword::Flying);
        set.insert(Keyword::Haste);

        assert_eq!(set.len(), 2);
        assert!(set.contains(Keyword::Flying));
        assert!(set.contains(Keyword::Haste));
        assert!(!set.contains(Keyword::Trample));
    }

    #[test]
    fn test_keyword_set_complex() {
        let mut set = KeywordSet::new();

        let madness_cost = ManaCost {
            generic: 1,
            red: 1,
            ..ManaCost::new()
        };
        set.insert_complex(KeywordArgs::Madness { cost: madness_cost });

        assert_eq!(set.len(), 1);
        assert!(set.contains(Keyword::Madness));

        let args = set.get_args(Keyword::Madness).unwrap();
        match args {
            KeywordArgs::Madness { cost } => {
                assert_eq!(cost.generic, 1);
                assert_eq!(cost.red, 1);
            }
            _ => panic!("Wrong args type"),
        }
    }

    #[test]
    fn test_keyword_set_mixed() {
        let mut set = KeywordSet::new();

        set.insert(Keyword::Flying);
        set.insert_complex(KeywordArgs::Madness { cost: ManaCost::new() });

        assert_eq!(set.len(), 2);
        assert!(set.contains(Keyword::Flying));
        assert!(set.contains(Keyword::Madness));
    }

    #[test]
    fn test_keyword_set_iteration() {
        let mut set = KeywordSet::new();

        set.insert(Keyword::Flying);
        set.insert(Keyword::Haste);
        set.insert_complex(KeywordArgs::Madness { cost: ManaCost::new() });

        let keyword_count = set.iter().count();
        let args_count = set.iter_args().count();

        assert_eq!(keyword_count, 3); // Flying, Haste, Madness
        assert_eq!(args_count, 1); // Only Madness has args
    }

    #[test]
    fn test_keyword_set_clear() {
        let mut set = KeywordSet::new();

        set.insert(Keyword::Flying);
        set.insert_complex(KeywordArgs::Madness { cost: ManaCost::new() });

        assert_eq!(set.len(), 2);

        set.clear();
        assert!(set.is_empty());
    }

    #[test]
    fn test_keyword_set_remove() {
        let mut set = KeywordSet::new();

        set.insert(Keyword::Flying);
        set.insert(Keyword::Haste);
        set.insert_complex(KeywordArgs::Madness { cost: ManaCost::new() });

        assert_eq!(set.len(), 3);

        set.remove(Keyword::Flying);
        assert_eq!(set.len(), 2);
        assert!(!set.contains(Keyword::Flying));
        assert!(set.contains(Keyword::Haste));

        // Remove complex keyword should also remove args
        set.remove(Keyword::Madness);
        assert_eq!(set.len(), 1);
        assert!(!set.contains(Keyword::Madness));
        assert_eq!(set.args.len(), 0);
    }

    #[test]
    fn test_is_complex() {
        assert!(!Keyword::Flying.is_complex());
        assert!(!Keyword::Haste.is_complex());
        assert!(Keyword::Madness.is_complex());
        assert!(Keyword::Flashback.is_complex());
        assert!(Keyword::Equip.is_complex());
    }

    #[test]
    fn test_get_args_none_for_simple() {
        let mut set = KeywordSet::new();
        set.insert(Keyword::Flying);

        assert!(set.get_args(Keyword::Flying).is_none());
    }
}
