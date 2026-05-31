//! Strongly-typed wrappers for game concepts
//!
//! This module provides newtypes to prevent type confusion and make the code
//! more self-documenting. Instead of using bare Strings for different concepts,
//! we wrap them in distinct types that cannot be mixed up.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Card subtype (creature type, artifact type, land type, etc.)
///
/// Examples: "Goblin", "Warrior", "Equipment", "Island"
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Subtype(String);

impl Subtype {
    pub fn new(s: impl Into<String>) -> Self {
        Subtype(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Subtype {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for Subtype {
    fn from(s: String) -> Self {
        Subtype(s)
    }
}

impl From<&str> for Subtype {
    fn from(s: &str) -> Self {
        Subtype(s.to_string())
    }
}

/// Counter type enum for all official MTG counter types
///
/// Represents all official MTG counter types from the comprehensive Java implementation.
/// Each counter has a display name for text-based output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CounterType {
    // Power/Toughness Modifiers
    M1M1, // -1/-1
    P1P1, // +1/+1
    M0M1, // -0/-1
    M0M2, // -0/-2
    M1M0, // -1/-0
    M2M1, // -2/-1
    M2M2, // -2/-2
    P0P1, // +0/+1
    P0P2, // +0/+2
    P1P0, // +1/+0
    P1P2, // +1/+2
    P2P0, // +2/+0
    P2P2, // +2/+2

    // Planeswalker
    Loyalty,

    // Alphabetical Counter Types
    Acorn,
    Aegis,
    Age,
    Aim,
    Arrow,
    Arrowhead,
    Awakening,
    Bait,
    Blaze,
    Blessing,
    Blight,
    Blood,
    Bloodline,
    Bloodstain,
    Bore,
    Bounty,
    Brain,
    Bribery,
    Brick,
    Burden,
    Cage,
    Carrion,
    Cell,
    Charge,
    Chorus,
    Coin,
    Collection,
    Component,
    Contested,
    Corpse,
    Corruption,
    Croak,
    Credit,
    Crystal,
    Cube,
    Currency,
    Death,
    Defense,
    Delay,
    Depletion,
    Descent,
    Despair,
    Devotion,
    Discovery,
    Divinity,
    Doom,
    Dread,
    Dream,
    Duty,
    Echo,
    Egg,
    Elixir,
    Ember,
    Eon,
    Eruption,
    Exposure,
    Eyeball,
    Eyestalk,
    Everything,
    Fade,
    Fate,
    Feather,
    Feeding,
    Fellowship,
    Fetch,
    Filibuster,
    Film,
    Finality,
    Fire,
    Flame,
    Flavor,
    Flood,
    Foreshadow,
    Fungus,
    Funk,
    Fury,
    Fuse,
    Gem,
    Ghostform,
    Glyph,
    Gold,
    Growth,
    Harmony,
    Hatching,
    Hatchling,
    Healing,
    Hit,
    Hone,
    Hope,
    Hoofprint,
    Hour,
    Hourglass,
    Hunger,
    Husk,
    Ice,
    Impostor,
    Incarnation,
    Incubation,
    Ingredient,
    Infection,
    Influence,
    Ingenuity,
    Intel,
    Intervention,
    Invitation,
    Isolation,
    Javelin,
    Judgment,
    Ki,
    Kick,
    Knowledge,
    Landmark,
    Level,
    Loot,
    Lore,
    Luck,
    Manabond,
    Magnet,
    Mana,
    Manifestation,
    Mannequin,
    Matrix,
    Memory,
    Midway,
    Mine,
    Mining,
    Mire,
    Music,
    Muster,
    Necrodermis,
    Net,
    Nest,
    Oil,
    Omen,
    Ore,
    Page,
    Pain,
    Paralyzation,
    Petal,
    Petrification,
    Pin,
    Plague,
    Plot,
    Pressure,
    Phylactery,
    Phyresis,
    Point,
    Polyp,
    Possession,
    Prey,
    Pupa,
    Quest,
    Rally,
    Release,
    Reprieve,
    Rejection,
    Rev,
    Revival,
    Ribbon,
    Ritual,
    Rope,
    Rust,
    Scream,
    Scroll,
    Shell,
    Shield,
    Shred,
    Silver,
    Skewer,
    Sleep,
    Slumber,
    Sleight,
    Slime,
    Soul,
    Soot,
    Spite,
    Spore,
    Stash,
    Storage,
    Story,
    Strife,
    Study,
    Stun,
    Supply,
    Takeover,
    Task,
    Theft,
    Tide,
    Time,
    Tower,
    Training,
    Trap,
    Treasure,
    Unity,
    Unlock,
    Valor,
    Velocity,
    Verse,
    Vitality,
    Vortex,
    Voyage,
    Wage,
    Winch,
    Wind,
    Wish,
    Wreck,

    // Player Counters
    Energy,
    Experience,
    Poison,
    Rad,
    Ticket,
}

impl CounterType {
    /// Get the display name for this counter (as shown on cards)
    pub fn display_name(&self) -> &'static str {
        match self {
            CounterType::M1M1 => "-1/-1",
            CounterType::P1P1 => "+1/+1",
            CounterType::M0M1 => "-0/-1",
            CounterType::M0M2 => "-0/-2",
            CounterType::M1M0 => "-1/-0",
            CounterType::M2M1 => "-2/-1",
            CounterType::M2M2 => "-2/-2",
            CounterType::P0P1 => "+0/+1",
            CounterType::P0P2 => "+0/+2",
            CounterType::P1P0 => "+1/+0",
            CounterType::P1P2 => "+1/+2",
            CounterType::P2P0 => "+2/+0",
            CounterType::P2P2 => "+2/+2",
            CounterType::Loyalty => "LOYAL",
            CounterType::Acorn => "ACORN",
            CounterType::Aegis => "AEGIS",
            CounterType::Age => "AGE",
            CounterType::Aim => "AIM",
            CounterType::Arrow => "ARROW",
            CounterType::Arrowhead => "ARWHD",
            CounterType::Awakening => "AWAKE",
            CounterType::Bait => "BAIT",
            CounterType::Blaze => "BLAZE",
            CounterType::Blessing => "BLESS",
            CounterType::Blight => "BLGHT",
            CounterType::Blood => "BLOOD",
            CounterType::Bloodline => "BLDLN",
            CounterType::Bloodstain => "BLDST",
            CounterType::Bore => "BORE",
            CounterType::Bounty => "BOUNT",
            CounterType::Brain => "BRAIN",
            CounterType::Bribery => "BRIBE",
            CounterType::Brick => "BRICK",
            CounterType::Burden => "BURDEN",
            CounterType::Cage => "CAGE",
            CounterType::Carrion => "CRRON",
            CounterType::Cell => "CELL",
            CounterType::Charge => "CHARG",
            CounterType::Chorus => "CHRUS",
            CounterType::Coin => "COIN",
            CounterType::Collection => "CLLCT",
            CounterType::Component => "COMPN",
            CounterType::Contested => "CONTES",
            CounterType::Corpse => "CRPSE",
            CounterType::Corruption => "CRPTN",
            CounterType::Croak => "CROAK",
            CounterType::Credit => "CRDIT",
            CounterType::Crystal => "CRYST",
            CounterType::Cube => "CUBE",
            CounterType::Currency => "CURR",
            CounterType::Death => "DEATH",
            CounterType::Defense => "DEF",
            CounterType::Delay => "DELAY",
            CounterType::Depletion => "DPLT",
            CounterType::Descent => "DESCT",
            CounterType::Despair => "DESPR",
            CounterType::Devotion => "DEVOT",
            CounterType::Discovery => "DISCO",
            CounterType::Divinity => "DVNTY",
            CounterType::Doom => "DOOM",
            CounterType::Dread => "DREAD",
            CounterType::Dream => "DREAM",
            CounterType::Duty => "DUTY",
            CounterType::Echo => "ECHO",
            CounterType::Egg => "EGG",
            CounterType::Elixir => "ELIXR",
            CounterType::Ember => "EMBER",
            CounterType::Eon => "EON",
            CounterType::Eruption => "ERUPTION",
            CounterType::Exposure => "EXPOSURE",
            CounterType::Eyeball => "EYE",
            CounterType::Eyestalk => "EYES",
            CounterType::Everything => "EVRY",
            CounterType::Fade => "FADE",
            CounterType::Fate => "FATE",
            CounterType::Feather => "FTHR",
            CounterType::Feeding => "FEED",
            CounterType::Fellowship => "FLWS",
            CounterType::Fetch => "FETCH",
            CounterType::Filibuster => "FLBTR",
            CounterType::Film => "FILM",
            CounterType::Finality => "FINAL",
            CounterType::Fire => "FIRE",
            CounterType::Flame => "FLAME",
            CounterType::Flavor => "FLAVOR",
            CounterType::Flood => "FLOOD",
            CounterType::Foreshadow => "FRSHD",
            CounterType::Fungus => "FNGUS",
            CounterType::Funk => "FUNK",
            CounterType::Fury => "FURY",
            CounterType::Fuse => "FUSE",
            CounterType::Gem => "GEM",
            CounterType::Ghostform => "GHSTF",
            CounterType::Glyph => "GLYPH",
            CounterType::Gold => "GOLD",
            CounterType::Growth => "GRWTH",
            CounterType::Harmony => "HRMNY",
            CounterType::Hatching => "HATCH",
            CounterType::Hatchling => "HTCHL",
            CounterType::Healing => "HEAL",
            CounterType::Hit => "HIT",
            CounterType::Hone => "HONE",
            CounterType::Hope => "HOPE",
            CounterType::Hoofprint => "HOOF",
            CounterType::Hour => "HOUR",
            CounterType::Hourglass => "HRGLS",
            CounterType::Hunger => "HUNGR",
            CounterType::Husk => "HUSK",
            CounterType::Ice => "ICE",
            CounterType::Impostor => "IMPO",
            CounterType::Incarnation => "INCRN",
            CounterType::Incubation => "INCBT",
            CounterType::Ingredient => "INGRD",
            CounterType::Infection => "INFCT",
            CounterType::Influence => "INFL",
            CounterType::Ingenuity => "INGTY",
            CounterType::Intel => "INTEL",
            CounterType::Intervention => "INTRV",
            CounterType::Invitation => "INVIT",
            CounterType::Isolation => "ISOLT",
            CounterType::Javelin => "JAVLN",
            CounterType::Judgment => "JUDGM",
            CounterType::Ki => "KI",
            CounterType::Kick => "KICK",
            CounterType::Knowledge => "KNOWL",
            CounterType::Landmark => "LNMRK",
            CounterType::Level => "LEVEL",
            CounterType::Loot => "LOOT",
            CounterType::Lore => "LORE",
            CounterType::Luck => "LUCK",
            CounterType::Manabond => "MANA",
            CounterType::Magnet => "MAGNT",
            CounterType::Mana => "MANA",
            CounterType::Manifestation => "MNFST",
            CounterType::Mannequin => "MANQN",
            CounterType::Matrix => "MATRX",
            CounterType::Memory => "MEMRY",
            CounterType::Midway => "MDWAY",
            CounterType::Mine => "MINE",
            CounterType::Mining => "MINNG",
            CounterType::Mire => "MIRE",
            CounterType::Music => "MUSIC",
            CounterType::Muster => "MUSTR",
            CounterType::Necrodermis => "NECRO",
            CounterType::Net => "NET",
            CounterType::Nest => "NEST",
            CounterType::Oil => "OIL",
            CounterType::Omen => "OMEN",
            CounterType::Ore => "ORE",
            CounterType::Page => "PAGE",
            CounterType::Pain => "PAIN",
            CounterType::Paralyzation => "PRLYZ",
            CounterType::Petal => "PETAL",
            CounterType::Petrification => "PETRI",
            CounterType::Pin => "PIN",
            CounterType::Plague => "PLGUE",
            CounterType::Plot => "PLOT",
            CounterType::Pressure => "PRESS",
            CounterType::Phylactery => "PHYLA",
            CounterType::Phyresis => "PHYRE",
            CounterType::Point => "POINT",
            CounterType::Polyp => "POLYP",
            CounterType::Possession => "POSSN",
            CounterType::Prey => "PREY",
            CounterType::Pupa => "PUPA",
            CounterType::Quest => "QUEST",
            CounterType::Rally => "RALLY",
            CounterType::Release => "RELEASE",
            CounterType::Reprieve => "REPR",
            CounterType::Rejection => "REJECT",
            CounterType::Rev => "REV",
            CounterType::Revival => "REVIVL",
            CounterType::Ribbon => "RIBBON",
            CounterType::Ritual => "RITUAL",
            CounterType::Rope => "ROPE",
            CounterType::Rust => "RUST",
            CounterType::Scream => "SCREM",
            CounterType::Scroll => "SCRLL",
            CounterType::Shell => "SHELL",
            CounterType::Shield => "SHLD",
            CounterType::Shred => "SHRED",
            CounterType::Silver => "SILVER",
            CounterType::Skewer => "SKEWER",
            CounterType::Sleep => "SLEEP",
            CounterType::Slumber => "SLMBR",
            CounterType::Sleight => "SLGHT",
            CounterType::Slime => "SLIME",
            CounterType::Soul => "SOUL",
            CounterType::Soot => "SOOT",
            CounterType::Spite => "SPITE",
            CounterType::Spore => "SPORE",
            CounterType::Stash => "STASH",
            CounterType::Storage => "STORG",
            CounterType::Story => "STORY",
            CounterType::Strife => "STRFE",
            CounterType::Study => "STUDY",
            CounterType::Stun => "STUN",
            CounterType::Supply => "SPPLY",
            CounterType::Takeover => "TKVR",
            CounterType::Task => "TASK",
            CounterType::Theft => "THEFT",
            CounterType::Tide => "TIDE",
            CounterType::Time => "TIME",
            CounterType::Tower => "TOWER",
            CounterType::Training => "TRAIN",
            CounterType::Trap => "TRAP",
            CounterType::Treasure => "TRSUR",
            CounterType::Unity => "UNITY",
            CounterType::Unlock => "UNLCK",
            CounterType::Valor => "VALOR",
            CounterType::Velocity => "VELO",
            CounterType::Verse => "VERSE",
            CounterType::Vitality => "VITAL",
            CounterType::Vortex => "VORTX",
            CounterType::Voyage => "VOYAGE",
            CounterType::Wage => "WAGE",
            CounterType::Winch => "WINCH",
            CounterType::Wind => "WIND",
            CounterType::Wish => "WISH",
            CounterType::Wreck => "WRECK",
            CounterType::Energy => "ENRGY",
            CounterType::Experience => "EXP",
            CounterType::Poison => "POISN",
            CounterType::Rad => "RAD",
            CounterType::Ticket => "TICKET",
        }
    }

    /// Parse a counter type from a string
    ///
    /// Handles the special cases for power/toughness counters like "+1/+1" -> P1P1
    ///
    /// Note: We use `parse` instead of implementing `FromStr` to return `Option<Self>`
    /// rather than `Result<Self, Error>`, which is more convenient for our use case.
    pub fn parse(s: &str) -> Option<Self> {
        // Replace special characters for power/toughness counters
        let normalized = s.replace('/', "").replace('+', "P").replace('-', "M").to_uppercase();

        match normalized.as_str() {
            "M1M1" => Some(CounterType::M1M1),
            "P1P1" => Some(CounterType::P1P1),
            "M0M1" => Some(CounterType::M0M1),
            "M0M2" => Some(CounterType::M0M2),
            "M1M0" => Some(CounterType::M1M0),
            "M2M1" => Some(CounterType::M2M1),
            "M2M2" => Some(CounterType::M2M2),
            "P0P1" => Some(CounterType::P0P1),
            "P0P2" => Some(CounterType::P0P2),
            "P1P0" => Some(CounterType::P1P0),
            "P1P2" => Some(CounterType::P1P2),
            "P2P0" => Some(CounterType::P2P0),
            "P2P2" => Some(CounterType::P2P2),
            "LOYAL" | "LOYALTY" => Some(CounterType::Loyalty),
            "ACORN" => Some(CounterType::Acorn),
            "AEGIS" => Some(CounterType::Aegis),
            "AGE" => Some(CounterType::Age),
            "AIM" => Some(CounterType::Aim),
            "ARROW" => Some(CounterType::Arrow),
            "ARWHD" | "ARROWHEAD" => Some(CounterType::Arrowhead),
            "AWAKE" | "AWAKENING" => Some(CounterType::Awakening),
            "BAIT" => Some(CounterType::Bait),
            "BLAZE" => Some(CounterType::Blaze),
            "BLESS" | "BLESSING" => Some(CounterType::Blessing),
            "BLGHT" | "BLIGHT" => Some(CounterType::Blight),
            "BLOOD" => Some(CounterType::Blood),
            "BLDLN" | "BLOODLINE" => Some(CounterType::Bloodline),
            "BLDST" | "BLOODSTAIN" => Some(CounterType::Bloodstain),
            "BORE" => Some(CounterType::Bore),
            "BOUNT" | "BOUNTY" => Some(CounterType::Bounty),
            "BRAIN" => Some(CounterType::Brain),
            "BRIBE" | "BRIBERY" => Some(CounterType::Bribery),
            "BRICK" => Some(CounterType::Brick),
            "BURDEN" => Some(CounterType::Burden),
            "CAGE" => Some(CounterType::Cage),
            "CRRON" | "CARRION" => Some(CounterType::Carrion),
            "CELL" => Some(CounterType::Cell),
            "CHARG" | "CHARGE" => Some(CounterType::Charge),
            "CHRUS" | "CHORUS" => Some(CounterType::Chorus),
            "COIN" => Some(CounterType::Coin),
            "CLLCT" | "COLLECTION" => Some(CounterType::Collection),
            "COMPN" | "COMPONENT" => Some(CounterType::Component),
            "CONTES" | "CONTESTED" => Some(CounterType::Contested),
            "CRPSE" | "CORPSE" => Some(CounterType::Corpse),
            "CRPTN" | "CORRUPTION" => Some(CounterType::Corruption),
            "CROAK" => Some(CounterType::Croak),
            "CRDIT" | "CREDIT" => Some(CounterType::Credit),
            "CRYST" | "CRYSTAL" => Some(CounterType::Crystal),
            "CUBE" => Some(CounterType::Cube),
            "CURR" | "CURRENCY" => Some(CounterType::Currency),
            "DEATH" => Some(CounterType::Death),
            "DEF" | "DEFENSE" => Some(CounterType::Defense),
            "DELAY" => Some(CounterType::Delay),
            "DPLT" | "DEPLETION" => Some(CounterType::Depletion),
            "DESCT" | "DESCENT" => Some(CounterType::Descent),
            "DESPR" | "DESPAIR" => Some(CounterType::Despair),
            "DEVOT" | "DEVOTION" => Some(CounterType::Devotion),
            "DISCO" | "DISCOVERY" => Some(CounterType::Discovery),
            "DVNTY" | "DIVINITY" => Some(CounterType::Divinity),
            "DOOM" => Some(CounterType::Doom),
            "DREAD" => Some(CounterType::Dread),
            "DREAM" => Some(CounterType::Dream),
            "DUTY" => Some(CounterType::Duty),
            "ECHO" => Some(CounterType::Echo),
            "EGG" => Some(CounterType::Egg),
            "ELIXR" | "ELIXIR" => Some(CounterType::Elixir),
            "EMBER" => Some(CounterType::Ember),
            "EON" => Some(CounterType::Eon),
            "ERUPTION" => Some(CounterType::Eruption),
            "EXPOSURE" => Some(CounterType::Exposure),
            "EYE" | "EYEBALL" => Some(CounterType::Eyeball),
            "EYES" | "EYESTALK" => Some(CounterType::Eyestalk),
            "EVRY" | "EVERYTHING" => Some(CounterType::Everything),
            "FADE" => Some(CounterType::Fade),
            "FATE" => Some(CounterType::Fate),
            "FTHR" | "FEATHER" => Some(CounterType::Feather),
            "FEED" | "FEEDING" => Some(CounterType::Feeding),
            "FLWS" | "FELLOWSHIP" => Some(CounterType::Fellowship),
            "FETCH" => Some(CounterType::Fetch),
            "FLBTR" | "FILIBUSTER" => Some(CounterType::Filibuster),
            "FILM" => Some(CounterType::Film),
            "FINAL" | "FINALITY" => Some(CounterType::Finality),
            "FIRE" => Some(CounterType::Fire),
            "FLAME" => Some(CounterType::Flame),
            "FLAVOR" => Some(CounterType::Flavor),
            "FLOOD" => Some(CounterType::Flood),
            "FRSHD" | "FORESHADOW" => Some(CounterType::Foreshadow),
            "FNGUS" | "FUNGUS" => Some(CounterType::Fungus),
            "FUNK" => Some(CounterType::Funk),
            "FURY" => Some(CounterType::Fury),
            "FUSE" => Some(CounterType::Fuse),
            "GEM" => Some(CounterType::Gem),
            "GHSTF" | "GHOSTFORM" => Some(CounterType::Ghostform),
            "GLYPH" => Some(CounterType::Glyph),
            "GOLD" => Some(CounterType::Gold),
            "GRWTH" | "GROWTH" => Some(CounterType::Growth),
            "HRMNY" | "HARMONY" => Some(CounterType::Harmony),
            "HATCH" | "HATCHING" => Some(CounterType::Hatching),
            "HTCHL" | "HATCHLING" => Some(CounterType::Hatchling),
            "HEAL" | "HEALING" => Some(CounterType::Healing),
            "HIT" => Some(CounterType::Hit),
            "HONE" => Some(CounterType::Hone),
            "HOPE" => Some(CounterType::Hope),
            "HOOF" | "HOOFPRINT" => Some(CounterType::Hoofprint),
            "HOUR" => Some(CounterType::Hour),
            "HRGLS" | "HOURGLASS" => Some(CounterType::Hourglass),
            "HUNGR" | "HUNGER" => Some(CounterType::Hunger),
            "HUSK" => Some(CounterType::Husk),
            "ICE" => Some(CounterType::Ice),
            "IMPO" | "IMPOSTOR" => Some(CounterType::Impostor),
            "INCRN" | "INCARNATION" => Some(CounterType::Incarnation),
            "INCBT" | "INCUBATION" => Some(CounterType::Incubation),
            "INGRD" | "INGREDIENT" => Some(CounterType::Ingredient),
            "INFCT" | "INFECTION" => Some(CounterType::Infection),
            "INFL" | "INFLUENCE" => Some(CounterType::Influence),
            "INGTY" | "INGENUITY" => Some(CounterType::Ingenuity),
            "INTEL" => Some(CounterType::Intel),
            "INTRV" | "INTERVENTION" => Some(CounterType::Intervention),
            "INVIT" | "INVITATION" => Some(CounterType::Invitation),
            "ISOLT" | "ISOLATION" => Some(CounterType::Isolation),
            "JAVLN" | "JAVELIN" => Some(CounterType::Javelin),
            "JUDGM" | "JUDGMENT" => Some(CounterType::Judgment),
            "KI" => Some(CounterType::Ki),
            "KICK" => Some(CounterType::Kick),
            "KNOWL" | "KNOWLEDGE" => Some(CounterType::Knowledge),
            "LNMRK" | "LANDMARK" => Some(CounterType::Landmark),
            "LEVEL" => Some(CounterType::Level),
            "LOOT" => Some(CounterType::Loot),
            "LORE" => Some(CounterType::Lore),
            "LUCK" => Some(CounterType::Luck),
            "MAGNT" | "MAGNET" => Some(CounterType::Magnet),
            "MANA" | "MANABOND" => Some(CounterType::Mana),
            "MNFST" | "MANIFESTATION" => Some(CounterType::Manifestation),
            "MANQN" | "MANNEQUIN" => Some(CounterType::Mannequin),
            "MATRX" | "MATRIX" => Some(CounterType::Matrix),
            "MEMRY" | "MEMORY" => Some(CounterType::Memory),
            "MDWAY" | "MIDWAY" => Some(CounterType::Midway),
            "MINE" => Some(CounterType::Mine),
            "MINNG" | "MINING" => Some(CounterType::Mining),
            "MIRE" => Some(CounterType::Mire),
            "MUSIC" => Some(CounterType::Music),
            "MUSTR" | "MUSTER" => Some(CounterType::Muster),
            "NECRO" | "NECRODERMIS" => Some(CounterType::Necrodermis),
            "NET" => Some(CounterType::Net),
            "NEST" => Some(CounterType::Nest),
            "OIL" => Some(CounterType::Oil),
            "OMEN" => Some(CounterType::Omen),
            "ORE" => Some(CounterType::Ore),
            "PAGE" => Some(CounterType::Page),
            "PAIN" => Some(CounterType::Pain),
            "PRLYZ" | "PARALYZATION" => Some(CounterType::Paralyzation),
            "PETAL" => Some(CounterType::Petal),
            "PETRI" | "PETRIFICATION" => Some(CounterType::Petrification),
            "PIN" => Some(CounterType::Pin),
            "PLGUE" | "PLAGUE" => Some(CounterType::Plague),
            "PLOT" => Some(CounterType::Plot),
            "PRESS" | "PRESSURE" => Some(CounterType::Pressure),
            "PHYLA" | "PHYLACTERY" => Some(CounterType::Phylactery),
            "PHYRE" | "PHYRESIS" => Some(CounterType::Phyresis),
            "POINT" => Some(CounterType::Point),
            "POLYP" => Some(CounterType::Polyp),
            "POSSN" | "POSSESSION" => Some(CounterType::Possession),
            "PREY" => Some(CounterType::Prey),
            "PUPA" => Some(CounterType::Pupa),
            "QUEST" => Some(CounterType::Quest),
            "RALLY" => Some(CounterType::Rally),
            "RELEASE" => Some(CounterType::Release),
            "REPR" | "REPRIEVE" => Some(CounterType::Reprieve),
            "REJECT" | "REJECTION" => Some(CounterType::Rejection),
            "REV" => Some(CounterType::Rev),
            "REVIVL" | "REVIVAL" => Some(CounterType::Revival),
            "RIBBON" => Some(CounterType::Ribbon),
            "RITUAL" => Some(CounterType::Ritual),
            "ROPE" => Some(CounterType::Rope),
            "RUST" => Some(CounterType::Rust),
            "SCREM" | "SCREAM" => Some(CounterType::Scream),
            "SCRLL" | "SCROLL" => Some(CounterType::Scroll),
            "SHELL" => Some(CounterType::Shell),
            "SHLD" | "SHIELD" => Some(CounterType::Shield),
            "SHRED" => Some(CounterType::Shred),
            "SILVER" => Some(CounterType::Silver),
            "SKEWER" => Some(CounterType::Skewer),
            "SLEEP" => Some(CounterType::Sleep),
            "SLMBR" | "SLUMBER" => Some(CounterType::Slumber),
            "SLGHT" | "SLEIGHT" => Some(CounterType::Sleight),
            "SLIME" => Some(CounterType::Slime),
            "SOUL" => Some(CounterType::Soul),
            "SOOT" => Some(CounterType::Soot),
            "SPITE" => Some(CounterType::Spite),
            "SPORE" => Some(CounterType::Spore),
            "STASH" => Some(CounterType::Stash),
            "STORG" | "STORAGE" => Some(CounterType::Storage),
            "STORY" => Some(CounterType::Story),
            "STRFE" | "STRIFE" => Some(CounterType::Strife),
            "STUDY" => Some(CounterType::Study),
            "STUN" => Some(CounterType::Stun),
            "SPPLY" | "SUPPLY" => Some(CounterType::Supply),
            "TKVR" | "TAKEOVER" => Some(CounterType::Takeover),
            "TASK" => Some(CounterType::Task),
            "THEFT" => Some(CounterType::Theft),
            "TIDE" => Some(CounterType::Tide),
            "TIME" => Some(CounterType::Time),
            "TOWER" => Some(CounterType::Tower),
            "TRAIN" | "TRAINING" => Some(CounterType::Training),
            "TRAP" => Some(CounterType::Trap),
            "TRSUR" | "TREASURE" => Some(CounterType::Treasure),
            "UNITY" => Some(CounterType::Unity),
            "UNLCK" | "UNLOCK" => Some(CounterType::Unlock),
            "VALOR" => Some(CounterType::Valor),
            "VELO" | "VELOCITY" => Some(CounterType::Velocity),
            "VERSE" => Some(CounterType::Verse),
            "VITAL" | "VITALITY" => Some(CounterType::Vitality),
            "VORTX" | "VORTEX" => Some(CounterType::Vortex),
            "VOYAGE" => Some(CounterType::Voyage),
            "WAGE" => Some(CounterType::Wage),
            "WINCH" => Some(CounterType::Winch),
            "WIND" => Some(CounterType::Wind),
            "WISH" => Some(CounterType::Wish),
            "WRECK" => Some(CounterType::Wreck),
            "ENRGY" | "ENERGY" => Some(CounterType::Energy),
            "EXP" | "EXPERIENCE" => Some(CounterType::Experience),
            "POISN" | "POISON" => Some(CounterType::Poison),
            "RAD" => Some(CounterType::Rad),
            "TICKET" => Some(CounterType::Ticket),
            _ => None,
        }
    }

    /// Check if this is a player counter (not a permanent counter)
    pub fn is_player_counter(&self) -> bool {
        matches!(
            self,
            CounterType::Energy
                | CounterType::Experience
                | CounterType::Poison
                | CounterType::Rad
                | CounterType::Ticket
        )
    }

    /// Check if this modifies power/toughness
    pub fn is_power_toughness_modifier(&self) -> bool {
        matches!(
            self,
            CounterType::M1M1
                | CounterType::P1P1
                | CounterType::M0M1
                | CounterType::M0M2
                | CounterType::M1M0
                | CounterType::M2M1
                | CounterType::M2M2
                | CounterType::P0P1
                | CounterType::P0P2
                | CounterType::P1P0
                | CounterType::P1P2
                | CounterType::P2P0
                | CounterType::P2P2
        )
    }

    /// Get the power/toughness modification amount (if applicable)
    /// Returns (power_mod, toughness_mod)
    ///
    /// Note: Wildcard is intentional here - only P/T counter types (13 of 200+)
    /// have modifiers. All other counter types (Charge, Loyalty, etc.) return None.
    #[allow(clippy::wildcard_enum_match_arm)]
    pub fn power_toughness_mod(&self) -> Option<(i32, i32)> {
        match self {
            CounterType::M1M1 => Some((-1, -1)),
            CounterType::P1P1 => Some((1, 1)),
            CounterType::M0M1 => Some((0, -1)),
            CounterType::M0M2 => Some((0, -2)),
            CounterType::M1M0 => Some((-1, 0)),
            CounterType::M2M1 => Some((-2, -1)),
            CounterType::M2M2 => Some((-2, -2)),
            CounterType::P0P1 => Some((0, 1)),
            CounterType::P0P2 => Some((0, 2)),
            CounterType::P1P0 => Some((1, 0)),
            CounterType::P1P2 => Some((1, 2)),
            CounterType::P2P0 => Some((2, 0)),
            CounterType::P2P2 => Some((2, 2)),
            _ => None,
        }
    }
}

impl fmt::Display for CounterType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

/// Card name (distinct from other string types)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CardName(std::sync::Arc<str>);

impl PartialOrd for CardName {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CardName {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

impl CardName {
    pub fn new(s: impl Into<String>) -> Self {
        CardName(s.into().into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn to_lowercase(&self) -> String {
        self.0.to_lowercase()
    }
}

impl fmt::Display for CardName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for CardName {
    fn from(s: String) -> Self {
        CardName(s.into())
    }
}

impl From<&str> for CardName {
    fn from(s: &str) -> Self {
        CardName(s.into())
    }
}

impl Serialize for CardName {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for CardName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(CardName(s.into()))
    }
}

/// Set / edition code (e.g. `ARN` = Arabian Nights, `LEA` = Limited Edition
/// Alpha). Identifies the Magic expansion a card was printed in.
///
/// This is the engine-wide handle for "set-origin" predicates in card scripts
/// such as `setARN` (`ValidCard$ Card.setARN`, `Permanent.setARN`). It is a
/// distinct strong type rather than a bare `String` so that set comparisons in
/// the valid/filter system can never be confused with card names, subtypes, or
/// other short uppercase strings.
///
/// Set codes are short ASCII (2-4 chars). They are interned as an `Arc<str>`
/// for cheap clone/compare. Comparison is case-insensitive at construction
/// time: codes are normalized to uppercase so `setARN` matches an edition file
/// whose `Code=ARN` (and ScryfallCode/Code2 casing) regardless of source case.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SetCode(std::sync::Arc<str>);

impl SetCode {
    /// Construct a normalized (uppercase) set code.
    pub fn new(s: impl AsRef<str>) -> Self {
        SetCode(s.as_ref().trim().to_ascii_uppercase().into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SetCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for SetCode {
    fn from(s: &str) -> Self {
        SetCode::new(s)
    }
}

impl From<String> for SetCode {
    fn from(s: String) -> Self {
        SetCode::new(s)
    }
}

impl Serialize for SetCode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for SetCode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        // Already-normalized on the wire, but normalize defensively.
        Ok(SetCode::new(s))
    }
}

/// Player name (distinct from other string types)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PlayerName(std::sync::Arc<str>);

impl PlayerName {
    pub fn new(s: impl Into<String>) -> Self {
        PlayerName(s.into().into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PlayerName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for PlayerName {
    fn from(s: String) -> Self {
        PlayerName(s.into())
    }
}

impl From<&str> for PlayerName {
    fn from(s: &str) -> Self {
        PlayerName(s.into())
    }
}

impl Serialize for PlayerName {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for PlayerName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(PlayerName(s.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subtype() {
        let subtype = Subtype::new("Goblin");
        assert_eq!(subtype.as_str(), "Goblin");
        assert_eq!(subtype.to_string(), "Goblin");
    }

    #[test]
    fn test_counter_type() {
        let counter = CounterType::P1P1;
        assert_eq!(counter.display_name(), "+1/+1");
        assert_eq!(counter.to_string(), "+1/+1");
        assert!(counter.is_power_toughness_modifier());
        assert_eq!(counter.power_toughness_mod(), Some((1, 1)));
    }

    #[test]
    fn test_counter_type_parsing() {
        assert_eq!(CounterType::parse("+1/+1"), Some(CounterType::P1P1));
        assert_eq!(CounterType::parse("-1/-1"), Some(CounterType::M1M1));
        assert_eq!(CounterType::parse("charge"), Some(CounterType::Charge));
        assert_eq!(CounterType::parse("CHARG"), Some(CounterType::Charge));
        assert_eq!(CounterType::parse("loyalty"), Some(CounterType::Loyalty));
        assert_eq!(CounterType::parse("LOYAL"), Some(CounterType::Loyalty));
        assert_eq!(CounterType::parse("poison"), Some(CounterType::Poison));
        assert_eq!(CounterType::parse("unknown"), None);
    }

    #[test]
    fn test_player_counters() {
        assert!(CounterType::Energy.is_player_counter());
        assert!(CounterType::Poison.is_player_counter());
        assert!(!CounterType::P1P1.is_player_counter());
        assert!(!CounterType::Charge.is_player_counter());
    }

    #[test]
    fn test_power_toughness_mods() {
        assert_eq!(CounterType::M1M1.power_toughness_mod(), Some((-1, -1)));
        assert_eq!(CounterType::P2P2.power_toughness_mod(), Some((2, 2)));
        assert_eq!(CounterType::P1P0.power_toughness_mod(), Some((1, 0)));
        assert_eq!(CounterType::M0M2.power_toughness_mod(), Some((0, -2)));
        assert_eq!(CounterType::Charge.power_toughness_mod(), None);
    }

    #[test]
    fn test_card_name() {
        let name = CardName::new("Lightning Bolt");
        assert_eq!(name.as_str(), "Lightning Bolt");
    }

    #[test]
    fn test_player_name() {
        let name = PlayerName::new("Player1");
        assert_eq!(name.as_str(), "Player1");
    }
}
