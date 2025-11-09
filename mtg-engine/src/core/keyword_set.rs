//! Efficient keyword storage using enumset for simple keywords
//!
//! This module provides the `KeywordSet` abstraction that stores keywords efficiently:
//! - Simple keywords (no parameters) use `EnumSet<KeywordSimple>` for O(1) membership tests
//! - Complex keywords (with parameters) use `Vec<KeywordComplex>`
//!
//! This matches the Java Forge implementation which uses `EnumSet<Keyword>` for efficient
//! keyword storage and operations.

use enumset::{EnumSet, EnumSetType};
use serde::{Deserialize, Serialize};

/// Simple keywords with no parameters
/// These are stored as a bitset using EnumSet for O(1) operations
/// Total: 92 simple keywords matching Java Forge's SimpleKeyword.class keywords
#[derive(Debug, EnumSetType, Serialize, Deserialize)]
#[enumset(repr = "u128")]
pub enum KeywordSimple {
    // Evergreen keywords (appear in most sets)
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

    // Evasion abilities
    Fear,
    Intimidate,
    Horsemanship,
    Shadow,
    Skulk,

    // Protection (specific colors - full Protection is parameterized)
    ProtectionFromRed,
    ProtectionFromBlue,
    ProtectionFromBlack,
    ProtectionFromWhite,
    ProtectionFromGreen,

    // Combat-related
    Banding,
    Flanking,
    Phasing,
    Wither,
    Infect,

    // Keyword actions/abilities
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

    // Set-specific mechanics (alphabetically sorted)
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

    // Commander/Multiplayer
    ChooseABackground,
    DoctorsCompanion,
    FriendsForever,
    PartnerSurvivors,
    PartnerFatherAndSon,
    PartnerCharacterSelect,

    // Mayflash variants
    MayFlashSac,
}

/// Complex keywords with parameters (stored as strings for now)
/// These keywords have parameters like costs, types, or amounts
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeywordComplex {
    // Keywords with cost parameters
    /// Madness cost (e.g., "Madness:1 R")
    Madness(String),
    /// Flashback cost (e.g., "Flashback:3 R")
    Flashback(String),
    /// Kicker cost (e.g., "Kicker:2")
    Kicker(String),
    /// Cycling cost (e.g., "Cycling:2")
    Cycling(String),
    /// Equip cost (e.g., "Equip:2")
    Equip(String),
    /// Morph cost (e.g., "Morph:3 G")
    Morph(String),
    /// Evoke cost (e.g., "Evoke:2 G")
    Evoke(String),
    /// Buyback cost (e.g., "Buyback:3")
    Buyback(String),
    /// Echo cost (e.g., "Echo:2 G")
    Echo(String),
    /// Suspend cost and time counters (e.g., "Suspend:3:G")
    Suspend(String),

    // Keywords with type parameters
    /// Enchant type (e.g., "Enchant:Creature")
    Enchant(String),
    /// Landwalk type (e.g., "Landwalk:Island")
    Landwalk(String),
    /// Affinity type (e.g., "Affinity:Artifact")
    Affinity(String),
    /// Protection (e.g., "Protection:Red", "Protection:Artifacts")
    Protection(String),
    /// Offering type (e.g., "Offering:Spirit")
    Offering(String),
    /// Champion type (e.g., "Champion:Goblin")
    Champion(String),

    // Keywords with amount parameters
    /// Amplify (e.g., "Amplify:2:Beast")
    Amplify(String),
    /// Annihilator amount (e.g., "Annihilator:2")
    Annihilator(String),
    /// Bushido amount (e.g., "Bushido:2")
    Bushido(String),
    /// Fading counters (e.g., "Fading:3")
    Fading(String),
    /// Vanishing counters (e.g., "Vanishing:3")
    Vanishing(String),
    /// Dredge amount (e.g., "Dredge:3")
    Dredge(String),
    /// Modular counters (e.g., "Modular:2")
    Modular(String),
    /// Absorb amount (e.g., "Absorb:2")
    Absorb(String),

    // Hexproof variants
    /// Hexproof from (e.g., "Hexproof:Blue", "Hexproof:instants")
    HexproofFrom(String),

    // Partner variant with parameter
    /// Partner with specific (e.g., "Partner:Regna")
    PartnerWith(String),

    // Companion deck restriction
    /// Companion restriction (e.g., "Companion:...")
    Companion(String),

    /// Catch-all for other keywords not yet explicitly supported
    Other(String),
}

/// Efficient keyword storage combining simple and complex keywords
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeywordSet {
    /// Simple keywords stored as a bitset (O(1) membership)
    simple: EnumSet<KeywordSimple>,
    /// Complex keywords with parameters
    complex: Vec<KeywordComplex>,
}

impl KeywordSet {
    /// Create an empty keyword set
    pub fn new() -> Self {
        Self {
            simple: EnumSet::new(),
            complex: Vec::new(),
        }
    }

    /// Check if a simple keyword is present (O(1))
    pub fn contains_simple(&self, keyword: KeywordSimple) -> bool {
        self.simple.contains(keyword)
    }

    /// Check if a complex keyword variant is present (O(n) in number of complex keywords)
    pub fn contains_complex(&self, keyword: &KeywordComplex) -> bool {
        self.complex.contains(keyword)
    }

    /// Check if a keyword is present (migration helper - delegates to contains_keyword)
    /// This provides backward compatibility for code that does card.keywords.contains(&Keyword::Flying)
    pub fn contains(&self, keyword: &Keyword) -> bool {
        self.contains_keyword(keyword)
    }

    /// Add a simple keyword
    pub fn insert_simple(&mut self, keyword: KeywordSimple) {
        self.simple.insert(keyword);
    }

    /// Add a complex keyword
    pub fn push_complex(&mut self, keyword: KeywordComplex) {
        self.complex.push(keyword);
    }

    /// Add a keyword (migration helper - delegates to insert_keyword)
    /// This provides backward compatibility for code that does card.keywords.push(Keyword::Flying)
    pub fn push(&mut self, keyword: Keyword) {
        self.insert_keyword(keyword);
    }

    /// Remove a simple keyword
    pub fn remove_simple(&mut self, keyword: KeywordSimple) {
        self.simple.remove(keyword);
    }

    /// Iterate over all simple keywords
    pub fn iter_simple(&self) -> impl Iterator<Item = KeywordSimple> + '_ {
        self.simple.iter()
    }

    /// Iterate over all complex keywords
    pub fn iter_complex(&self) -> impl Iterator<Item = &KeywordComplex> + '_ {
        self.complex.iter()
    }

    /// Iterate over all keywords as Keyword enum values (migration helper)
    /// This provides backward compatibility for code that expects to iterate over Vec<Keyword>
    /// NOTE: New keywords not in the old Keyword enum are mapped to Keyword::Other
    pub fn iter(&self) -> impl Iterator<Item = Keyword> + '_ {
        // Chain simple keywords (converted to Keyword) with complex keywords (converted to Keyword)
        let simple_iter = self.iter_simple().map(|simple| match simple {
            KeywordSimple::Flying => Keyword::Flying,
            KeywordSimple::FirstStrike => Keyword::FirstStrike,
            KeywordSimple::DoubleStrike => Keyword::DoubleStrike,
            KeywordSimple::Deathtouch => Keyword::Deathtouch,
            KeywordSimple::Haste => Keyword::Haste,
            KeywordSimple::Hexproof => Keyword::Hexproof,
            KeywordSimple::Indestructible => Keyword::Indestructible,
            KeywordSimple::Lifelink => Keyword::Lifelink,
            KeywordSimple::Menace => Keyword::Menace,
            KeywordSimple::Reach => Keyword::Reach,
            KeywordSimple::Trample => Keyword::Trample,
            KeywordSimple::Vigilance => Keyword::Vigilance,
            KeywordSimple::Defender => Keyword::Defender,
            KeywordSimple::Shroud => Keyword::Shroud,
            KeywordSimple::ProtectionFromRed => Keyword::ProtectionFromRed,
            KeywordSimple::ProtectionFromBlue => Keyword::ProtectionFromBlue,
            KeywordSimple::ProtectionFromBlack => Keyword::ProtectionFromBlack,
            KeywordSimple::ProtectionFromWhite => Keyword::ProtectionFromWhite,
            KeywordSimple::ProtectionFromGreen => Keyword::ProtectionFromGreen,
            KeywordSimple::ChooseABackground => Keyword::ChooseABackground,
            // All other keywords map to Other for backward compatibility
            _ => Keyword::Other(format!("{:?}", simple)),
        });

        let complex_iter = self.iter_complex().map(|complex| match complex {
            KeywordComplex::Madness(cost) => Keyword::Madness(cost.clone()),
            KeywordComplex::Flashback(cost) => Keyword::Flashback(cost.clone()),
            KeywordComplex::Enchant(target) => Keyword::Enchant(target.clone()),
            // All other complex keywords (including new ones) map to Other
            KeywordComplex::Other(s) => Keyword::Other(s.clone()),
            _ => Keyword::Other(format!("{:?}", complex)),
        });

        simple_iter.chain(complex_iter)
    }

    /// Get the number of keywords (simple + complex)
    pub fn len(&self) -> usize {
        self.simple.len() + self.complex.len()
    }

    /// Check if the set is empty
    pub fn is_empty(&self) -> bool {
        self.simple.is_empty() && self.complex.is_empty()
    }

    /// Clear all keywords
    pub fn clear(&mut self) {
        self.simple.clear();
        self.complex.clear();
    }
}

impl Default for KeywordSet {
    fn default() -> Self {
        Self::new()
    }
}

/// Implement IntoIterator for &KeywordSet to support `for keyword in &card.keywords`
impl<'a> IntoIterator for &'a KeywordSet {
    type Item = Keyword;
    type IntoIter = Box<dyn Iterator<Item = Keyword> + 'a>;

    fn into_iter(self) -> Self::IntoIter {
        Box::new(self.iter())
    }
}

// Migration helpers to convert from old Keyword enum to new KeywordSet
use crate::core::effects::Keyword;

impl KeywordSet {
    /// Create a KeywordSet from a Vec<Keyword> (for migration)
    pub fn from_keyword_vec(keywords: Vec<Keyword>) -> Self {
        let mut set = Self::new();
        for keyword in keywords {
            set.insert_keyword(keyword);
        }
        set
    }

    /// Insert a Keyword into this set (migration helper)
    pub fn insert_keyword(&mut self, keyword: Keyword) {
        match keyword {
            // Simple keywords
            Keyword::Flying => self.insert_simple(KeywordSimple::Flying),
            Keyword::FirstStrike => self.insert_simple(KeywordSimple::FirstStrike),
            Keyword::DoubleStrike => self.insert_simple(KeywordSimple::DoubleStrike),
            Keyword::Deathtouch => self.insert_simple(KeywordSimple::Deathtouch),
            Keyword::Haste => self.insert_simple(KeywordSimple::Haste),
            Keyword::Hexproof => self.insert_simple(KeywordSimple::Hexproof),
            Keyword::Indestructible => self.insert_simple(KeywordSimple::Indestructible),
            Keyword::Lifelink => self.insert_simple(KeywordSimple::Lifelink),
            Keyword::Menace => self.insert_simple(KeywordSimple::Menace),
            Keyword::Reach => self.insert_simple(KeywordSimple::Reach),
            Keyword::Trample => self.insert_simple(KeywordSimple::Trample),
            Keyword::Vigilance => self.insert_simple(KeywordSimple::Vigilance),
            Keyword::Defender => self.insert_simple(KeywordSimple::Defender),
            Keyword::Shroud => self.insert_simple(KeywordSimple::Shroud),
            Keyword::ProtectionFromRed => self.insert_simple(KeywordSimple::ProtectionFromRed),
            Keyword::ProtectionFromBlue => self.insert_simple(KeywordSimple::ProtectionFromBlue),
            Keyword::ProtectionFromBlack => self.insert_simple(KeywordSimple::ProtectionFromBlack),
            Keyword::ProtectionFromWhite => self.insert_simple(KeywordSimple::ProtectionFromWhite),
            Keyword::ProtectionFromGreen => self.insert_simple(KeywordSimple::ProtectionFromGreen),
            Keyword::ChooseABackground => self.insert_simple(KeywordSimple::ChooseABackground),

            // Complex keywords
            Keyword::Madness(cost) => self.push_complex(KeywordComplex::Madness(cost)),
            Keyword::Flashback(cost) => self.push_complex(KeywordComplex::Flashback(cost)),
            Keyword::Enchant(target) => self.push_complex(KeywordComplex::Enchant(target)),
            Keyword::Other(s) => self.push_complex(KeywordComplex::Other(s)),
        }
    }

    /// Check if a Keyword is present (migration helper)
    pub fn contains_keyword(&self, keyword: &Keyword) -> bool {
        match keyword {
            // Simple keywords
            Keyword::Flying => self.contains_simple(KeywordSimple::Flying),
            Keyword::FirstStrike => self.contains_simple(KeywordSimple::FirstStrike),
            Keyword::DoubleStrike => self.contains_simple(KeywordSimple::DoubleStrike),
            Keyword::Deathtouch => self.contains_simple(KeywordSimple::Deathtouch),
            Keyword::Haste => self.contains_simple(KeywordSimple::Haste),
            Keyword::Hexproof => self.contains_simple(KeywordSimple::Hexproof),
            Keyword::Indestructible => self.contains_simple(KeywordSimple::Indestructible),
            Keyword::Lifelink => self.contains_simple(KeywordSimple::Lifelink),
            Keyword::Menace => self.contains_simple(KeywordSimple::Menace),
            Keyword::Reach => self.contains_simple(KeywordSimple::Reach),
            Keyword::Trample => self.contains_simple(KeywordSimple::Trample),
            Keyword::Vigilance => self.contains_simple(KeywordSimple::Vigilance),
            Keyword::Defender => self.contains_simple(KeywordSimple::Defender),
            Keyword::Shroud => self.contains_simple(KeywordSimple::Shroud),
            Keyword::ProtectionFromRed => self.contains_simple(KeywordSimple::ProtectionFromRed),
            Keyword::ProtectionFromBlue => self.contains_simple(KeywordSimple::ProtectionFromBlue),
            Keyword::ProtectionFromBlack => self.contains_simple(KeywordSimple::ProtectionFromBlack),
            Keyword::ProtectionFromWhite => self.contains_simple(KeywordSimple::ProtectionFromWhite),
            Keyword::ProtectionFromGreen => self.contains_simple(KeywordSimple::ProtectionFromGreen),
            Keyword::ChooseABackground => self.contains_simple(KeywordSimple::ChooseABackground),

            // Complex keywords
            Keyword::Madness(cost) => self.contains_complex(&KeywordComplex::Madness(cost.clone())),
            Keyword::Flashback(cost) => self.contains_complex(&KeywordComplex::Flashback(cost.clone())),
            Keyword::Enchant(target) => self.contains_complex(&KeywordComplex::Enchant(target.clone())),
            Keyword::Other(s) => self.contains_complex(&KeywordComplex::Other(s.clone())),
        }
    }

    /// Convert this KeywordSet back to Vec<Keyword> (for migration/compatibility)
    /// NOTE: New keywords not in the old Keyword enum are mapped to Keyword::Other
    pub fn to_keyword_vec(&self) -> Vec<Keyword> {
        let mut result = Vec::new();

        // Add simple keywords
        for simple in self.iter_simple() {
            let keyword = match simple {
                KeywordSimple::Flying => Keyword::Flying,
                KeywordSimple::FirstStrike => Keyword::FirstStrike,
                KeywordSimple::DoubleStrike => Keyword::DoubleStrike,
                KeywordSimple::Deathtouch => Keyword::Deathtouch,
                KeywordSimple::Haste => Keyword::Haste,
                KeywordSimple::Hexproof => Keyword::Hexproof,
                KeywordSimple::Indestructible => Keyword::Indestructible,
                KeywordSimple::Lifelink => Keyword::Lifelink,
                KeywordSimple::Menace => Keyword::Menace,
                KeywordSimple::Reach => Keyword::Reach,
                KeywordSimple::Trample => Keyword::Trample,
                KeywordSimple::Vigilance => Keyword::Vigilance,
                KeywordSimple::Defender => Keyword::Defender,
                KeywordSimple::Shroud => Keyword::Shroud,
                KeywordSimple::ProtectionFromRed => Keyword::ProtectionFromRed,
                KeywordSimple::ProtectionFromBlue => Keyword::ProtectionFromBlue,
                KeywordSimple::ProtectionFromBlack => Keyword::ProtectionFromBlack,
                KeywordSimple::ProtectionFromWhite => Keyword::ProtectionFromWhite,
                KeywordSimple::ProtectionFromGreen => Keyword::ProtectionFromGreen,
                KeywordSimple::ChooseABackground => Keyword::ChooseABackground,
                // Map all other new keywords to Other for backward compatibility
                _ => Keyword::Other(format!("{:?}", simple)),
            };
            result.push(keyword);
        }

        // Add complex keywords
        for complex in self.iter_complex() {
            let keyword = match complex {
                KeywordComplex::Madness(cost) => Keyword::Madness(cost.clone()),
                KeywordComplex::Flashback(cost) => Keyword::Flashback(cost.clone()),
                KeywordComplex::Enchant(target) => Keyword::Enchant(target.clone()),
                KeywordComplex::Other(s) => Keyword::Other(s.clone()),
                // Map all other new complex keywords to Other
                _ => Keyword::Other(format!("{:?}", complex)),
            };
            result.push(keyword);
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keyword_set_simple() {
        let mut set = KeywordSet::new();
        assert!(set.is_empty());

        set.insert_simple(KeywordSimple::Flying);
        set.insert_simple(KeywordSimple::Haste);

        assert_eq!(set.len(), 2);
        assert!(set.contains_simple(KeywordSimple::Flying));
        assert!(set.contains_simple(KeywordSimple::Haste));
        assert!(!set.contains_simple(KeywordSimple::Trample));
    }

    #[test]
    fn test_keyword_set_complex() {
        let mut set = KeywordSet::new();

        set.push_complex(KeywordComplex::Madness("1 R".to_string()));
        set.push_complex(KeywordComplex::Flashback("3 R".to_string()));

        assert_eq!(set.len(), 2);
        assert!(set.contains_complex(&KeywordComplex::Madness("1 R".to_string())));
        assert!(!set.contains_complex(&KeywordComplex::Madness("2 R".to_string())));
    }

    #[test]
    fn test_keyword_set_mixed() {
        let mut set = KeywordSet::new();

        set.insert_simple(KeywordSimple::Flying);
        set.push_complex(KeywordComplex::Madness("1 R".to_string()));

        assert_eq!(set.len(), 2);
        assert!(set.contains_simple(KeywordSimple::Flying));
        assert!(set.contains_complex(&KeywordComplex::Madness("1 R".to_string())));
    }

    #[test]
    fn test_keyword_set_iteration() {
        let mut set = KeywordSet::new();

        set.insert_simple(KeywordSimple::Flying);
        set.insert_simple(KeywordSimple::Haste);
        set.push_complex(KeywordComplex::Madness("1 R".to_string()));

        let simple_count = set.iter_simple().count();
        let complex_count = set.iter_complex().count();

        assert_eq!(simple_count, 2);
        assert_eq!(complex_count, 1);
    }

    #[test]
    fn test_keyword_set_clear() {
        let mut set = KeywordSet::new();

        set.insert_simple(KeywordSimple::Flying);
        set.push_complex(KeywordComplex::Madness("1 R".to_string()));

        assert_eq!(set.len(), 2);

        set.clear();
        assert!(set.is_empty());
    }

    #[test]
    fn test_keyword_set_remove() {
        let mut set = KeywordSet::new();

        set.insert_simple(KeywordSimple::Flying);
        set.insert_simple(KeywordSimple::Haste);

        assert_eq!(set.len(), 2);

        set.remove_simple(KeywordSimple::Flying);
        assert_eq!(set.len(), 1);
        assert!(!set.contains_simple(KeywordSimple::Flying));
        assert!(set.contains_simple(KeywordSimple::Haste));
    }
}
