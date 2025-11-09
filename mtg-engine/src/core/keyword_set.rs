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
/// Total: 118 keywords (92 simple + 26 complex variants)
#[derive(Debug, EnumSetType, Serialize, Deserialize)]
#[enumset(repr = "u128")]
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
    Haunt,
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

    // Keywords with type parameters
    Enchant,
    Landwalk,
    Affinity,
    Protection,
    Offering,
    Champion,

    // Keywords with amount parameters
    Amplify,
    Annihilator,
    Bushido,
    Fading,
    Vanishing,
    Dredge,
    Modular,
    Absorb,

    // Other parameterized keywords
    HexproofFrom,
    PartnerWith,
    Companion,
}

impl Keyword {
    /// Returns true if this keyword requires arguments (is complex)
    pub fn is_complex(&self) -> bool {
        matches!(
            self,
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
                | Keyword::Enchant
                | Keyword::Landwalk
                | Keyword::Affinity
                | Keyword::Protection
                | Keyword::Offering
                | Keyword::Champion
                | Keyword::Amplify
                | Keyword::Annihilator
                | Keyword::Bushido
                | Keyword::Fading
                | Keyword::Vanishing
                | Keyword::Dredge
                | Keyword::Modular
                | Keyword::Absorb
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

    // ===== OTHER PARAMETERIZED KEYWORDS =====
    /// Hexproof from (e.g., "Hexproof:Blue", "Hexproof:instants")
    /// TODO: Parse into Color | CardType once we have those enums
    HexproofFrom { from: String },
    /// Partner with specific card (e.g., "Partner:Regna")
    PartnerWith { card_name: CardName },
    /// Companion deck restriction
    /// TODO: Parse restriction into structured format
    Companion { restriction: String },
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
            KeywordArgs::HexproofFrom { .. } => Keyword::HexproofFrom,
            KeywordArgs::PartnerWith { .. } => Keyword::PartnerWith,
            KeywordArgs::Companion { .. } => Keyword::Companion,
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
