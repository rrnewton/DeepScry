//! Player representation

use crate::core::{GameEntity, ManaPool, PlayerId, PlayerName};
use serde::{Deserialize, Serialize};

/// Represents a player in the game
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Player {
    /// Unique ID for this player
    pub id: PlayerId,

    /// Player name
    pub name: PlayerName,

    /// Life total
    pub life: i32,

    /// Mana pool (regular mana, empties at end of each step)
    pub mana_pool: ManaPool,

    /// Combat mana pool (mana that lasts until end of combat, e.g., from Firebending)
    /// Only allocated when needed (None in the common case for zero overhead).
    /// This is an optimization: most games never use Firebending, so we avoid
    /// allocating a ManaPool for every player. The Option check is a single
    /// well-predicted branch in the common (None) case.
    pub combat_mana_pool: Option<ManaPool>,

    /// Has the player lost?
    pub has_lost: bool,

    /// Lands played this turn
    pub lands_played_this_turn: u8,

    /// Maximum lands per turn (usually 1)
    pub max_lands_per_turn: u8,

    /// Maximum hand size (usually 7, modified by some effects)
    pub max_hand_size: usize,
}

impl Player {
    pub fn new(id: PlayerId, name: impl Into<PlayerName>, starting_life: i32) -> Self {
        Player {
            id,
            name: name.into(),
            life: starting_life,
            mana_pool: ManaPool::new(),
            combat_mana_pool: None, // Only allocated when Firebending is used
            has_lost: false,
            lands_played_this_turn: 0,
            max_lands_per_turn: 1,
            max_hand_size: 7, // Standard MTG hand size limit
        }
    }

    pub fn gain_life(&mut self, amount: i32) {
        self.life += amount;
    }

    pub fn lose_life(&mut self, amount: i32) {
        self.life -= amount;
        if self.life <= 0 {
            self.has_lost = true;
        }
    }

    pub fn can_play_land(&self) -> bool {
        self.lands_played_this_turn < self.max_lands_per_turn
    }

    pub fn play_land(&mut self) {
        self.lands_played_this_turn += 1;
    }

    pub fn reset_lands_played(&mut self) {
        self.lands_played_this_turn = 0;
    }

    pub fn empty_mana_pool(&mut self) {
        self.mana_pool.clear();
    }

    /// Clear combat mana pool (at end of combat)
    /// This is a no-op if no combat mana was ever added (Option is None).
    #[inline]
    pub fn empty_combat_mana_pool(&mut self) {
        // Fast path: if None, nothing to do (well-predicted branch)
        if self.combat_mana_pool.is_some() {
            self.combat_mana_pool = None;
        }
    }

    /// Check if player has any combat mana
    #[inline]
    pub fn has_combat_mana(&self) -> bool {
        self.combat_mana_pool.as_ref().is_some_and(|pool| pool.total() > 0)
    }

    /// Add mana to combat mana pool (lazy initialization)
    /// Called by Firebending and similar effects.
    #[inline]
    pub fn add_combat_mana(&mut self, color: crate::core::Color) {
        self.combat_mana_pool.get_or_insert_with(ManaPool::new).add_color(color);
    }

    /// Get total available mana (regular + combat)
    /// Fast path when no combat mana exists.
    #[inline]
    pub fn total_available_mana(&self) -> ManaPool {
        match &self.combat_mana_pool {
            None => self.mana_pool, // Fast path: just return regular pool (Copy)
            Some(combat) => ManaPool {
                white: self.mana_pool.white + combat.white,
                blue: self.mana_pool.blue + combat.blue,
                black: self.mana_pool.black + combat.black,
                red: self.mana_pool.red + combat.red,
                green: self.mana_pool.green + combat.green,
                colorless: self.mana_pool.colorless + combat.colorless,
            },
        }
    }

    /// Pay a mana cost from total available mana (regular + combat pools).
    ///
    /// Spends from regular pool first, then combat pool for the remainder.
    /// This is used during combat when Firebending has added combat mana.
    ///
    /// # Errors
    ///
    /// Returns an error message if insufficient total mana to pay the cost.
    pub fn pay_from_total_mana(&mut self, cost: &crate::core::ManaCost) -> Result<(), String> {
        let total = self.total_available_mana();
        if !total.can_pay(cost) {
            return Err(format!(
                "Insufficient total mana to pay {}. Have: {}W {}U {}B {}R {}G {}C (regular) + combat",
                cost,
                self.mana_pool.white,
                self.mana_pool.blue,
                self.mana_pool.black,
                self.mana_pool.red,
                self.mana_pool.green,
                self.mana_pool.colorless
            ));
        }

        // Fast path: no combat mana, just pay from regular pool
        let Some(combat) = self.combat_mana_pool.as_mut() else {
            return self.mana_pool.pay_cost(cost);
        };

        // Slow path: have combat mana, need to coordinate payment between pools
        // Strategy: Pay colored requirements first (from both pools), then generic

        // Pay colored costs - use regular pool first, then combat pool
        // White
        let regular_white = cost.white.min(self.mana_pool.white);
        self.mana_pool.white -= regular_white;
        let combat_white = (cost.white - regular_white).min(combat.white);
        combat.white -= combat_white;

        // Blue
        let regular_blue = cost.blue.min(self.mana_pool.blue);
        self.mana_pool.blue -= regular_blue;
        let combat_blue = (cost.blue - regular_blue).min(combat.blue);
        combat.blue -= combat_blue;

        // Black
        let regular_black = cost.black.min(self.mana_pool.black);
        self.mana_pool.black -= regular_black;
        let combat_black = (cost.black - regular_black).min(combat.black);
        combat.black -= combat_black;

        // Red
        let regular_red = cost.red.min(self.mana_pool.red);
        self.mana_pool.red -= regular_red;
        let combat_red = (cost.red - regular_red).min(combat.red);
        combat.red -= combat_red;

        // Green
        let regular_green = cost.green.min(self.mana_pool.green);
        self.mana_pool.green -= regular_green;
        let combat_green = (cost.green - regular_green).min(combat.green);
        combat.green -= combat_green;

        // Colorless
        let regular_colorless = cost.colorless.min(self.mana_pool.colorless);
        self.mana_pool.colorless -= regular_colorless;
        let combat_colorless = (cost.colorless - regular_colorless).min(combat.colorless);
        combat.colorless -= combat_colorless;

        // Pay generic cost from remaining mana (any color, regular first then combat)
        let mut generic_remaining = cost.generic;

        // From regular pool (WUBRG order)
        for color_mana in [
            &mut self.mana_pool.white,
            &mut self.mana_pool.blue,
            &mut self.mana_pool.black,
            &mut self.mana_pool.red,
            &mut self.mana_pool.green,
            &mut self.mana_pool.colorless,
        ] {
            let used = generic_remaining.min(*color_mana);
            *color_mana -= used;
            generic_remaining -= used;
            if generic_remaining == 0 {
                break;
            }
        }

        // From combat pool if still needed (WUBRG order)
        if generic_remaining > 0 {
            for color_mana in [
                &mut combat.white,
                &mut combat.blue,
                &mut combat.black,
                &mut combat.red,
                &mut combat.green,
                &mut combat.colorless,
            ] {
                let used = generic_remaining.min(*color_mana);
                *color_mana -= used;
                generic_remaining -= used;
                if generic_remaining == 0 {
                    break;
                }
            }
        }

        // If combat pool is now empty, deallocate it
        if combat.total() == 0 {
            self.combat_mana_pool = None;
        }

        Ok(())
    }
}

impl GameEntity<Player> for Player {
    fn id(&self) -> PlayerId {
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
    fn test_player_creation() {
        let id = PlayerId::new(1);
        let player = Player::new(id, "Player1", 20);

        assert_eq!(player.id, id);
        assert_eq!(player.name.as_str(), "Player1");
        assert_eq!(player.life, 20);
        assert!(!player.has_lost);
    }

    #[test]
    fn test_player_life() {
        let id = PlayerId::new(1);
        let mut player = Player::new(id, "Player2", 20);

        player.lose_life(5);
        assert_eq!(player.life, 15);
        assert!(!player.has_lost);

        player.lose_life(15);
        assert_eq!(player.life, 0);
        assert!(player.has_lost);

        player.gain_life(10);
        assert_eq!(player.life, 10);
        // has_lost stays true once triggered
        assert!(player.has_lost);
    }

    #[test]
    fn test_land_playing() {
        let id = PlayerId::new(1);
        let mut player = Player::new(id, "Charlie", 20);

        assert!(player.can_play_land());
        player.play_land();
        assert!(!player.can_play_land());

        player.reset_lands_played();
        assert!(player.can_play_land());
    }
}
