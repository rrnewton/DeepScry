//! Combat system for MTG
//!
//! Handles declaring attackers, declaring blockers, and combat damage

use crate::core::{CardId, PlayerId};
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use std::collections::BTreeMap;

/// Combat state for the current combat phase
///
/// This tracks all combat-related information during a combat phase.
/// It's reset at the end of combat.
/// Uses BTreeMap for deterministic iteration order.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CombatState {
    /// Creatures that are attacking this combat
    /// Maps attacker ID to the player/planeswalker being attacked
    pub attackers: BTreeMap<CardId, PlayerId>,

    /// Creatures that are blocking
    /// Maps blocker ID to the list of attackers it's blocking
    pub blockers: BTreeMap<CardId, SmallVec<[CardId; 2]>>,

    /// Reverse mapping: attacker -> blockers
    /// Useful for determining if an attacker is blocked and by whom
    pub attacker_blockers: BTreeMap<CardId, SmallVec<[CardId; 4]>>,

    /// Whether combat has started this turn
    pub combat_active: bool,
}

impl CombatState {
    /// Create a new empty combat state
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare a creature as an attacker
    pub fn declare_attacker(&mut self, attacker: CardId, defending_player: PlayerId) {
        self.attackers.insert(attacker, defending_player);
        self.combat_active = true;
    }

    /// Declare a creature as a blocker
    pub fn declare_blocker(&mut self, blocker: CardId, attackers: SmallVec<[CardId; 2]>) {
        // Add blocker -> attackers mapping
        self.blockers.insert(blocker, attackers.clone());

        // Update attacker -> blockers reverse mapping
        for attacker in &attackers {
            self.attacker_blockers.entry(*attacker).or_default().push(blocker);
        }
    }

    /// Check if a creature is attacking
    pub fn is_attacking(&self, card_id: CardId) -> bool {
        self.attackers.contains_key(&card_id)
    }

    /// Check if a creature is blocking
    pub fn is_blocking(&self, card_id: CardId) -> bool {
        self.blockers.contains_key(&card_id)
    }

    /// Check if an attacker is blocked
    pub fn is_blocked(&self, attacker: CardId) -> bool {
        self.attacker_blockers
            .get(&attacker)
            .is_some_and(|blockers| !blockers.is_empty())
    }

    /// Get the blockers for a given attacker
    pub fn get_blockers(&self, attacker: CardId) -> SmallVec<[CardId; 4]> {
        self.attacker_blockers.get(&attacker).cloned().unwrap_or_default()
    }

    /// Get the player being attacked by a creature
    pub fn get_defending_player(&self, attacker: CardId) -> Option<PlayerId> {
        self.attackers.get(&attacker).copied()
    }

    /// Get all attacking creatures (returns Vec for compatibility)
    pub fn get_attackers(&self) -> Vec<CardId> {
        self.attackers.keys().copied().collect()
    }

    /// Get all attacking creatures as an iterator (zero-allocation)
    pub fn attackers_iter(&self) -> impl Iterator<Item = CardId> + '_ {
        self.attackers.keys().copied()
    }

    /// Check if there are any attackers (zero-allocation)
    pub fn has_attackers(&self) -> bool {
        !self.attackers.is_empty()
    }

    /// Get all blocking creatures (returns Vec for compatibility)
    pub fn get_blockers_list(&self) -> Vec<CardId> {
        self.blockers.keys().copied().collect()
    }

    /// Get all blocking creatures as an iterator (zero-allocation)
    pub fn blockers_iter(&self) -> impl Iterator<Item = CardId> + '_ {
        self.blockers.keys().copied()
    }

    /// Clear all combat state (called at end of combat)
    pub fn clear(&mut self) {
        self.attackers.clear();
        self.blockers.clear();
        self.attacker_blockers.clear();
        self.combat_active = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_declare_attacker() {
        let mut combat = CombatState::new();
        let attacker = CardId::new(1);
        let defender = PlayerId::new(2);

        combat.declare_attacker(attacker, defender);

        assert!(combat.is_attacking(attacker));
        assert_eq!(combat.get_defending_player(attacker), Some(defender));
        assert!(combat.combat_active);
    }

    #[test]
    fn test_declare_blocker() {
        let mut combat = CombatState::new();
        let attacker1 = CardId::new(1);
        let attacker2 = CardId::new(2);
        let blocker = CardId::new(3);
        let defender = PlayerId::new(4);

        combat.declare_attacker(attacker1, defender);
        combat.declare_attacker(attacker2, defender);

        let mut attackers_blocked = SmallVec::new();
        attackers_blocked.push(attacker1);
        attackers_blocked.push(attacker2);
        combat.declare_blocker(blocker, attackers_blocked);

        assert!(combat.is_blocking(blocker));
        assert!(combat.is_blocked(attacker1));
        assert!(combat.is_blocked(attacker2));

        let blockers1 = combat.get_blockers(attacker1);
        assert_eq!(blockers1.len(), 1);
        assert!(blockers1.contains(&blocker));
    }

    #[test]
    fn test_clear_combat() {
        let mut combat = CombatState::new();
        let attacker = CardId::new(1);
        let defender = PlayerId::new(2);

        combat.declare_attacker(attacker, defender);
        assert!(combat.combat_active);

        combat.clear();
        assert!(!combat.is_attacking(attacker));
        assert!(!combat.combat_active);
        assert_eq!(combat.attackers.len(), 0);
    }

    #[test]
    fn test_unblocked_attacker() {
        let mut combat = CombatState::new();
        let attacker = CardId::new(1);
        let defender = PlayerId::new(2);

        combat.declare_attacker(attacker, defender);

        assert!(!combat.is_blocked(attacker));
        assert_eq!(combat.get_blockers(attacker).len(), 0);
    }
}
