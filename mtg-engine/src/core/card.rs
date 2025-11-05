//! Card types and definitions

use crate::core::{
    CardId, CardName, Color, CounterType, Effect, GameEntity, Keyword, ManaCost, PlayerId, Subtype, Trigger,
};
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

/// Card types in MTG
/// Copy-eligible since it's a simple enum with no data fields
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CardType {
    Creature,
    Instant,
    Sorcery,
    Enchantment,
    Artifact,
    Land,
    Planeswalker,
}

/// Represents a card in the game
///
/// Cards have a unique CardId but many cards can share the same card definition.
/// This struct represents the instance of a card during gameplay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Card {
    /// Unique ID for this card instance
    pub id: CardId,

    /// Card name (e.g., "Lightning Bolt")
    pub name: CardName,

    /// Mana cost
    pub mana_cost: ManaCost,

    /// Card types (a card can be multiple types)
    pub types: SmallVec<[CardType; 2]>,

    /// Card subtypes (e.g., "Goblin", "Warrior")
    pub subtypes: SmallVec<[Subtype; 2]>,

    /// Colors of the card
    pub colors: SmallVec<[Color; 2]>,

    /// Power (for creatures)
    pub power: Option<i8>,

    /// Toughness (for creatures)
    pub toughness: Option<i8>,

    /// Temporary power bonus (until end of turn)
    pub power_bonus: i32,

    /// Temporary toughness bonus (until end of turn)
    pub toughness_bonus: i32,

    /// Oracle text
    pub text: String,

    /// Current zone owner (player who owns this card)
    pub owner: PlayerId,

    /// Current controller (can differ from owner)
    pub controller: PlayerId,

    /// Is the card tapped?
    pub tapped: bool,

    /// Turn number when this permanent entered the battlefield
    /// Used for summoning sickness (creatures can't attack the turn they enter)
    /// None = not on battlefield yet, Some(turn) = entered on this turn
    pub turn_entered_battlefield: Option<u32>,

    /// Counters on this card (using SmallVec for efficiency)
    /// Common counters: +1/+1, -1/-1, charge, loyalty
    pub counters: SmallVec<[(CounterType, u8); 2]>,

    /// Keyword abilities (Flying, First Strike, etc.)
    pub keywords: Vec<Keyword>,

    /// Effects that execute when this card resolves
    /// For spells: effects execute when spell resolves
    /// For permanents: effects may be triggered or activated abilities
    pub effects: Vec<Effect>,

    /// Triggered abilities (ETB, phase triggers, etc.)
    /// These execute automatically when their trigger condition is met
    pub triggers: Vec<Trigger>,

    /// Activated abilities (costs and effects)
    /// These can be activated by paying their cost
    pub activated_abilities: Vec<crate::core::ActivatedAbility>,
}

impl Card {
    pub fn new(id: CardId, name: impl Into<CardName>, owner: PlayerId) -> Self {
        Card {
            id,
            name: name.into(),
            mana_cost: ManaCost::new(),
            types: SmallVec::new(),
            subtypes: SmallVec::new(),
            colors: SmallVec::new(),
            power: None,
            toughness: None,
            power_bonus: 0,
            toughness_bonus: 0,
            text: String::new(),
            owner,
            controller: owner,
            tapped: false,
            turn_entered_battlefield: None,
            counters: SmallVec::new(),
            keywords: Vec::new(),
            effects: Vec::new(),
            triggers: Vec::new(),
            activated_abilities: Vec::new(),
        }
    }

    pub fn is_type(&self, card_type: &CardType) -> bool {
        self.types.contains(card_type)
    }

    pub fn is_creature(&self) -> bool {
        self.is_type(&CardType::Creature)
    }

    pub fn is_land(&self) -> bool {
        self.is_type(&CardType::Land)
    }

    pub fn is_instant(&self) -> bool {
        self.is_type(&CardType::Instant)
    }

    pub fn is_artifact(&self) -> bool {
        self.is_type(&CardType::Artifact)
    }

    pub fn is_enchantment(&self) -> bool {
        self.is_type(&CardType::Enchantment)
    }

    pub fn is_aura(&self) -> bool {
        self.is_enchantment() && self.subtypes.iter().any(|s| s.as_str().eq_ignore_ascii_case("aura"))
    }

    pub fn has_keyword(&self, keyword: &Keyword) -> bool {
        self.keywords.contains(keyword)
    }

    pub fn has_flying(&self) -> bool {
        self.has_keyword(&Keyword::Flying)
    }

    pub fn has_reach(&self) -> bool {
        self.has_keyword(&Keyword::Reach)
    }

    pub fn has_first_strike(&self) -> bool {
        self.has_keyword(&Keyword::FirstStrike)
    }

    pub fn has_double_strike(&self) -> bool {
        self.has_keyword(&Keyword::DoubleStrike)
    }

    /// Returns true if this creature deals damage in the normal damage step
    /// (i.e., has double strike OR doesn't have first strike)
    pub fn has_normal_strike(&self) -> bool {
        self.has_double_strike() || !self.has_first_strike()
    }

    pub fn has_trample(&self) -> bool {
        self.has_keyword(&Keyword::Trample)
    }

    pub fn has_lifelink(&self) -> bool {
        self.has_keyword(&Keyword::Lifelink)
    }

    pub fn has_deathtouch(&self) -> bool {
        self.has_keyword(&Keyword::Deathtouch)
    }

    pub fn has_menace(&self) -> bool {
        self.has_keyword(&Keyword::Menace)
    }

    pub fn has_hexproof(&self) -> bool {
        self.has_keyword(&Keyword::Hexproof)
    }

    pub fn has_indestructible(&self) -> bool {
        self.has_keyword(&Keyword::Indestructible)
    }

    pub fn has_defender(&self) -> bool {
        self.has_keyword(&Keyword::Defender)
    }

    pub fn has_shroud(&self) -> bool {
        self.has_keyword(&Keyword::Shroud)
    }

    pub fn tap(&mut self) {
        self.tapped = true;
    }

    pub fn untap(&mut self) {
        self.tapped = false;
    }

    pub fn add_counter(&mut self, counter_type: CounterType, amount: u8) {
        if amount == 0 {
            return;
        }

        // Add the counter
        if let Some((_, count)) = self.counters.iter_mut().find(|(t, _)| t == &counter_type) {
            *count = count.saturating_add(amount);
        } else {
            self.counters.push((counter_type, amount));
        }

        // Apply counter annihilation: +1/+1 and -1/-1 counters cancel
        let p1p1_count = self.get_counter(CounterType::P1P1);
        let m1m1_count = self.get_counter(CounterType::M1M1);

        if p1p1_count > 0 && m1m1_count > 0 {
            let to_remove = p1p1_count.min(m1m1_count);

            // Remove from +1/+1 counters
            if let Some((_, count)) = self.counters.iter_mut().find(|(t, _)| t == &CounterType::P1P1) {
                *count -= to_remove;
                if *count == 0 {
                    self.counters.retain(|(t, _)| t != &CounterType::P1P1);
                }
            }

            // Remove from -1/-1 counters
            if let Some((_, count)) = self.counters.iter_mut().find(|(t, _)| t == &CounterType::M1M1) {
                *count -= to_remove;
                if *count == 0 {
                    self.counters.retain(|(t, _)| t != &CounterType::M1M1);
                }
            }
        }
    }

    pub fn remove_counter(&mut self, counter_type: CounterType, amount: u8) -> u8 {
        if amount == 0 {
            return 0;
        }

        if let Some((_, count)) = self.counters.iter_mut().find(|(t, _)| t == &counter_type) {
            let removed = (*count).min(amount);
            *count -= removed;
            if *count == 0 {
                self.counters.retain(|(t, _)| t != &counter_type);
            }
            removed
        } else {
            0
        }
    }

    pub fn get_counter(&self, counter_type: CounterType) -> u8 {
        self.counters
            .iter()
            .find(|(t, _)| t == &counter_type)
            .map(|(_, count)| *count)
            .unwrap_or(0)
    }

    /// Get current power (including counters and temporary bonuses)
    pub fn current_power(&self) -> i8 {
        let base = self.power.unwrap_or(0);
        let plus_counters = self.get_counter(CounterType::P1P1) as i8;
        let minus_counters = self.get_counter(CounterType::M1M1) as i8;
        let bonus = self.power_bonus as i8;
        base + plus_counters - minus_counters + bonus
    }

    /// Get current toughness (including counters and temporary bonuses)
    pub fn current_toughness(&self) -> i8 {
        let base = self.toughness.unwrap_or(0);
        let plus_counters = self.get_counter(CounterType::P1P1) as i8;
        let minus_counters = self.get_counter(CounterType::M1M1) as i8;
        let bonus = self.toughness_bonus as i8;
        base + plus_counters - minus_counters + bonus
    }
}

impl GameEntity<Card> for Card {
    fn id(&self) -> CardId {
        self.id
    }

    fn name(&self) -> &str {
        self.name.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_card_creation() {
        let id = CardId::new(1);
        let owner = PlayerId::new(100);
        let card = Card::new(id, "Lightning Bolt", owner);

        assert_eq!(card.id, id);
        assert_eq!(card.name.as_str(), "Lightning Bolt");
        assert_eq!(card.owner, owner);
        assert_eq!(card.controller, owner);
        assert!(!card.tapped);
    }

    #[test]
    fn test_card_counters() {
        let id = CardId::new(1);
        let owner = PlayerId::new(100);
        let mut card = Card::new(id, "Test Creature", owner);

        card.power = Some(2);
        card.toughness = Some(2);

        assert_eq!(card.current_power(), 2);
        assert_eq!(card.current_toughness(), 2);

        card.add_counter(CounterType::P1P1, 2);
        assert_eq!(card.current_power(), 4);
        assert_eq!(card.current_toughness(), 4);

        card.add_counter(CounterType::M1M1, 1);
        assert_eq!(card.current_power(), 3);
        assert_eq!(card.current_toughness(), 3);
    }

    #[test]
    fn test_counter_annihilation() {
        let id = CardId::new(1);
        let owner = PlayerId::new(100);
        let mut card = Card::new(id, "Test Creature", owner);

        card.power = Some(2);
        card.toughness = Some(2);

        // Add 3 +1/+1 counters
        card.add_counter(CounterType::P1P1, 3);
        assert_eq!(card.get_counter(CounterType::P1P1), 3);
        assert_eq!(card.get_counter(CounterType::M1M1), 0);
        assert_eq!(card.current_power(), 5);
        assert_eq!(card.current_toughness(), 5);

        // Add 2 -1/-1 counters - should annihilate with +1/+1
        card.add_counter(CounterType::M1M1, 2);
        assert_eq!(card.get_counter(CounterType::P1P1), 1); // 3 - 2 = 1
        assert_eq!(card.get_counter(CounterType::M1M1), 0); // 2 - 2 = 0
        assert_eq!(card.current_power(), 3); // 2 base + 1 counter
        assert_eq!(card.current_toughness(), 3);

        // Add 5 -1/-1 counters
        card.add_counter(CounterType::M1M1, 5);
        assert_eq!(card.get_counter(CounterType::P1P1), 0); // 1 - 1 = 0
        assert_eq!(card.get_counter(CounterType::M1M1), 4); // 5 - 1 = 4
        assert_eq!(card.current_power(), -2); // 2 base - 4 counters
        assert_eq!(card.current_toughness(), -2);
    }

    #[test]
    fn test_remove_counter() {
        let id = CardId::new(1);
        let owner = PlayerId::new(100);
        let mut card = Card::new(id, "Test Creature", owner);

        // Add some counters
        card.add_counter(CounterType::P1P1, 5);
        assert_eq!(card.get_counter(CounterType::P1P1), 5);

        // Remove 2 counters
        let removed = card.remove_counter(CounterType::P1P1, 2);
        assert_eq!(removed, 2);
        assert_eq!(card.get_counter(CounterType::P1P1), 3);

        // Try to remove more than exists
        let removed = card.remove_counter(CounterType::P1P1, 10);
        assert_eq!(removed, 3); // Only 3 were available
        assert_eq!(card.get_counter(CounterType::P1P1), 0);

        // Counter type should be cleaned up when it reaches 0
        assert!(!card.counters.iter().any(|(t, _)| t == &CounterType::P1P1));

        // Try to remove from non-existent counter type
        let removed = card.remove_counter(CounterType::M1M1, 5);
        assert_eq!(removed, 0);
    }

    #[test]
    fn test_exact_annihilation() {
        let id = CardId::new(1);
        let owner = PlayerId::new(100);
        let mut card = Card::new(id, "Test Creature", owner);

        // Add 3 +1/+1 counters
        card.add_counter(CounterType::P1P1, 3);
        assert_eq!(card.get_counter(CounterType::P1P1), 3);

        // Add exactly 3 -1/-1 counters - should cancel completely
        card.add_counter(CounterType::M1M1, 3);
        assert_eq!(card.get_counter(CounterType::P1P1), 0);
        assert_eq!(card.get_counter(CounterType::M1M1), 0);

        // Both counter types should be cleaned up
        assert!(card.counters.is_empty());
    }

    #[test]
    fn test_other_counters_not_affected() {
        let id = CardId::new(1);
        let owner = PlayerId::new(100);
        let mut card = Card::new(id, "Test Permanent", owner);

        // Add charge counters
        card.add_counter(CounterType::Charge, 5);
        assert_eq!(card.get_counter(CounterType::Charge), 5);

        // Add +1/+1 and -1/-1 counters
        card.add_counter(CounterType::P1P1, 2);
        card.add_counter(CounterType::M1M1, 1);

        // Charge counters should not be affected by annihilation
        assert_eq!(card.get_counter(CounterType::Charge), 5);
        assert_eq!(card.get_counter(CounterType::P1P1), 1); // 2 - 1 = 1
        assert_eq!(card.get_counter(CounterType::M1M1), 0);
    }
}
