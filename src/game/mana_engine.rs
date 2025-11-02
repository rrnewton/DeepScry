//! Mana availability computation and cost payment checking
//!
//! This module provides efficient querying of whether a player can produce
//! enough mana to pay a given cost. It maintains cached state of available
//! mana sources partitioned into simple and complex sources.
//!
//! # Architecture
//!
//! The mana engine operates in two phases:
//!
//! 1. **Update Phase**: Scans the battlefield to identify and cache mana-producing permanents
//! 2. **Query Phase**: Answers questions about whether specific costs can be paid
//!
//! ## Mana Source Classification
//!
//! - **Simple sources**: Lands that produce a single specific color (e.g., Mountain → R, Plains → W)
//!   - Cached as `ManaCapacity` counters (WUBRGC)
//!   - O(1) query time - just compare counts
//!   - Currently supports: Plains, Island, Swamp, Mountain, Forest, Wastes
//!
//! - **Complex sources**: Lands with choices or conditional costs (e.g., City of Brass → any color)
//!   - Stored as list of `CardId`s for future search
//!   - Not yet implemented - requires search algorithm
//!   - Examples: dual lands, fetch lands, City of Brass
//!
//! ## Performance Characteristics
//!
//! - **Update**: O(n) where n = number of battlefield permanents
//!   - Linear scan of battlefield
//!   - Should be called when permanents enter/leave or tap/untap
//!   - Not called on every mana payment - only when state changes
//!
//! - **Query (simple sources only)**: O(1)
//!   - Just arithmetic comparisons of cached counters
//!   - Critical path for spell selection AI
//!
//! - **Query (with complex sources)**: Not yet implemented
//!   - Will require small search (likely << 20 sources in practice)
//!
//! ## Integration with GameState
//!
//! The `ManaEngine` does not directly modify `GameState`. It is a read-only
//! cache layer that:
//!
//! 1. Reads battlefield state during `update()`
//! 2. Answers queries about mana availability
//! 3. Actual mana pool modification happens in `GameState::mana_pool`
//!
//! This separation allows the engine to be used speculatively (e.g., "what if
//! I had these lands?") without affecting the game state.
//!
//! # Usage Examples
//!
//! ## Basic Usage - Check if a spell is castable
//!
//! ```ignore
//! use mtg_forge_rs::game::{ManaEngine, GameState};
//! use mtg_forge_rs::core::{ManaCost, PlayerId};
//!
//! let game = GameState::new_two_player("Alice".to_string(), "Bob".to_string(), 20);
//! let alice_id = game.players[0].id;
//!
//! // Create and update the mana engine
//! let mut engine = ManaEngine::new(alice_id);
//! engine.update(&game);
//!
//! // Check if we can cast Lightning Bolt (R)
//! let mut bolt_cost = ManaCost::new();
//! bolt_cost.red = 1;
//! let can_cast = engine.can_pay(&bolt_cost);
//! ```
//!
//! ## Integrating with AI Controllers
//!
//! ```ignore
//! // In your controller's choose_spell_ability_to_play():
//! let mut engine = ManaEngine::new(player_id);
//! engine.update(&game);
//!
//! // Filter available spells to only those we can afford
//! let affordable_spells: Vec<_> = available_spells
//!     .into_iter()
//!     .filter(|spell| {
//!         let cost = get_spell_cost(spell);
//!         engine.can_pay(&cost)
//!     })
//!     .collect();
//! ```
//!
//! ## Maintaining the Engine Across Game Actions
//!
//! For efficiency, you can maintain a `ManaEngine` instance and update it
//! only when the battlefield changes:
//!
//! ```ignore
//! impl MyController {
//!     fn on_permanent_entered(&mut self, card_id: CardId, game: &GameState) {
//!         self.mana_engine.update(game);  // Rebuild cache
//!     }
//!
//!     fn on_permanent_tapped(&mut self, card_id: CardId, game: &GameState) {
//!         self.mana_engine.update(game);  // Rebuild cache
//!     }
//! }
//! ```
//!
//! # Future Enhancements
//!
//! - **Complex source handling**: Implement search algorithm for dual lands, City of Brass, etc.
//! - **Creature mana abilities**: Recognize Llanowar Elves, Birds of Paradise
//! - **Conditional sources**: Handle lands with tap conditions (e.g., "T: Add G if you control a Forest")
//! - **Mana filtering**: Track color identity restrictions (e.g., Commander format)
//! - **Cost reduction**: Handle effects like Goblin Electromancer that reduce spell costs

use crate::core::{CardId, ManaCost, PlayerId};
use crate::game::mana_payment::{
    GreedyManaResolver, ManaColor, ManaPaymentResolver, ManaProduction, ManaProductionKind, ManaSource,
    SimpleManaResolver,
};
use crate::game::GameState;

/// Maximum mana production capacity
///
/// Represents the maximum amount of mana of each color that can be produced
/// by tapping all available simple mana sources.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ManaCapacity {
    /// White mana
    pub white: u8,
    /// Blue mana
    pub blue: u8,
    /// Black mana
    pub black: u8,
    /// Red mana
    pub red: u8,
    /// Green mana
    pub green: u8,
    /// Colorless mana
    pub colorless: u8,
}

impl ManaCapacity {
    /// Create a new empty mana capacity
    pub fn new() -> Self {
        Self::default()
    }

    /// Get total mana available
    pub fn total(&self) -> u8 {
        self.white
            .saturating_add(self.blue)
            .saturating_add(self.black)
            .saturating_add(self.red)
            .saturating_add(self.green)
            .saturating_add(self.colorless)
    }

    /// Check if this capacity can pay for a mana cost
    ///
    /// For simple costs (only specific colors, no hybrid/phyrexian), this is
    /// a straightforward comparison. Returns false if the cost requires more
    /// mana of any color than we can produce.
    pub fn can_pay_simple(&self, cost: &ManaCost) -> bool {
        // Check specific color requirements
        if cost.white > self.white {
            return false;
        }
        if cost.blue > self.blue {
            return false;
        }
        if cost.black > self.black {
            return false;
        }
        if cost.red > self.red {
            return false;
        }
        if cost.green > self.green {
            return false;
        }
        if cost.colorless > self.colorless {
            return false;
        }

        // Check if we have enough total mana for generic requirement
        // Generic can be paid with any color or colorless mana
        let remaining_capacity = self
            .total()
            .saturating_sub(cost.white)
            .saturating_sub(cost.blue)
            .saturating_sub(cost.black)
            .saturating_sub(cost.red)
            .saturating_sub(cost.green)
            .saturating_sub(cost.colorless);

        remaining_capacity >= cost.generic
    }
}

/// Per-player mana engine
///
/// Maintains cached information about a player's mana-producing capabilities
/// and provides efficient queries for whether costs can be paid.
///
/// ## Usage
///
/// ```ignore
/// let mut engine = ManaEngine::new(player_id);
/// engine.update(&game); // Scan battlefield and cache mana sources
/// let can_cast = engine.can_pay(&mana_cost);
/// ```
pub struct ManaEngine {
    player_id: PlayerId,
    /// Simple mana sources (lands producing a single color)
    simple_sources: Vec<CardId>,
    /// Complex mana sources (lands with choices or conditions)
    complex_sources: Vec<CardId>,
    /// Cached capacity from simple sources
    simple_capacity: ManaCapacity,
    /// All mana sources as ManaSource structs (for resolver)
    mana_sources: Vec<ManaSource>,
    /// Payment resolver (strategy pattern for complex mana handling)
    resolver: Box<dyn ManaPaymentResolver>,
}

impl ManaEngine {
    /// Create a new mana engine for a player
    pub fn new(player_id: PlayerId) -> Self {
        Self {
            player_id,
            simple_sources: Vec::new(),
            complex_sources: Vec::new(),
            simple_capacity: ManaCapacity::new(),
            mana_sources: Vec::new(),
            resolver: Box::new(SimpleManaResolver::new()),
        }
    }

    /// Update the engine by scanning the battlefield for mana sources
    ///
    /// This should be called whenever:
    /// - A new permanent enters the battlefield
    /// - A permanent leaves the battlefield
    /// - A permanent becomes tapped/untapped
    pub fn update(&mut self, game: &GameState) {
        // Clear previous state
        self.simple_sources.clear();
        self.complex_sources.clear();
        self.simple_capacity = ManaCapacity::new();
        self.mana_sources.clear();

        // Scan battlefield for mana-producing permanents owned by this player
        // This includes lands and creatures with mana abilities (e.g., Llanowar Elves)
        for &card_id in &game.battlefield.cards {
            if let Ok(card) = game.cards.get(card_id) {
                // Check if this is a mana-producing permanent owned by this player
                let is_mana_source = card.is_land() || has_mana_ability(card);
                if card.owner == self.player_id && is_mana_source {
                    // Determine if this source has summoning sickness (for creatures with mana abilities)
                    let has_summoning_sickness = if card.is_creature() {
                        if let Some(entered_turn) = card.turn_entered_battlefield {
                            entered_turn == game.turn.turn_number && !card.has_keyword(&crate::core::Keyword::Haste)
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                    // Determine the mana production type
                    if let Some(color_char) = get_simple_mana_color(card.name.as_str()) {
                        // Simple source - produces exactly one color
                        let color = match color_char {
                            'W' => ManaColor::White,
                            'U' => ManaColor::Blue,
                            'B' => ManaColor::Black,
                            'R' => ManaColor::Red,
                            'G' => ManaColor::Green,
                            'C' => {
                                // Colorless is handled separately in ManaProduction
                                self.simple_sources.push(card_id);
                                if !card.tapped {
                                    self.simple_capacity.colorless += 1;
                                }
                                self.mana_sources.push(ManaSource {
                                    card_id,
                                    production: ManaProduction::free(ManaProductionKind::Colorless),
                                    is_tapped: card.tapped,
                                    has_summoning_sickness,
                                });
                                continue;
                            }
                            _ => continue, // Unknown color
                        };

                        self.simple_sources.push(card_id);
                        if !card.tapped {
                            match color {
                                ManaColor::White => self.simple_capacity.white += 1,
                                ManaColor::Blue => self.simple_capacity.blue += 1,
                                ManaColor::Black => self.simple_capacity.black += 1,
                                ManaColor::Red => self.simple_capacity.red += 1,
                                ManaColor::Green => self.simple_capacity.green += 1,
                            }
                        }

                        self.mana_sources.push(ManaSource {
                            card_id,
                            production: ManaProduction::free(ManaProductionKind::Fixed(color)),
                            is_tapped: card.tapped,
                            has_summoning_sickness,
                        });
                    } else if card.is_creature() {
                        // Check for creature mana abilities (Llanowar Elves, Birds of Paradise)
                        if let Some(production) = get_creature_mana_production(card) {
                            // Creatures with mana abilities are complex sources
                            self.complex_sources.push(card_id);
                            self.mana_sources.push(ManaSource {
                                card_id,
                                production,
                                is_tapped: card.tapped,
                                has_summoning_sickness,
                            });
                        }
                    } else if let Some(production) = get_complex_mana_production(card) {
                        // Complex source - dual land or any-color land
                        self.complex_sources.push(card_id);
                        self.mana_sources.push(ManaSource {
                            card_id,
                            production,
                            is_tapped: card.tapped,
                            has_summoning_sickness,
                        });
                    }
                    // If we can't parse it, just ignore it for now
                }
            }
        }

        // Switch to GreedyManaResolver if we have complex sources
        if !self.complex_sources.is_empty() {
            self.resolver = Box::new(GreedyManaResolver::new());
        } else {
            self.resolver = Box::new(SimpleManaResolver::new());
        }
    }

    /// Check if the player can pay for a mana cost
    ///
    /// This considers all mana sources (simple and complex) and determines
    /// whether there exists a way to tap them to produce the required mana.
    pub fn can_pay(&self, cost: &ManaCost) -> bool {
        // Use the resolver to check payment
        self.resolver.can_pay(cost, &self.mana_sources)
    }

    /// Get the current mana capacity from simple sources only
    pub fn simple_capacity(&self) -> ManaCapacity {
        self.simple_capacity
    }

    /// Get the list of simple mana sources
    pub fn simple_sources(&self) -> &[CardId] {
        &self.simple_sources
    }

    /// Get the list of complex mana sources
    pub fn complex_sources(&self) -> &[CardId] {
        &self.complex_sources
    }
}

/// Determine if a land is a simple mana source (produces exactly one color)
///
/// Returns the color character if it's a simple source: W, U, B, R, G, C
/// Returns None if it's a complex source or not a basic land.
fn get_simple_mana_color(land_name: &str) -> Option<char> {
    match land_name {
        "Plains" => Some('W'),
        "Island" => Some('U'),
        "Swamp" => Some('B'),
        "Mountain" => Some('R'),
        "Forest" => Some('G'),
        // Wastes produces colorless
        "Wastes" => Some('C'),
        // Any other land is considered complex for now
        _ => None,
    }
}

/// Check if a creature has a mana-producing activated ability
///
/// Detects patterns like "{T}: Add {G}" or "Add one mana of any color" in oracle text.
/// This is used to identify creatures like Llanowar Elves and Birds of Paradise.
fn has_mana_ability(card: &crate::core::Card) -> bool {
    use crate::core::CardType;

    // Only creatures can have mana abilities (for Phase 4)
    if !card.types.contains(&CardType::Creature) {
        return false;
    }

    let text_lower = card.text.to_lowercase();
    // Check for tap-to-add-mana patterns
    // Examples: "{T}: Add {G}", "Add one mana of any color", "{T}: Add {C}"
    text_lower.contains("{t}: add") || (text_lower.contains("add") && text_lower.contains("mana"))
}

/// Determine mana production for a creature with mana abilities
///
/// Analyzes oracle text to determine what mana a creature can produce.
/// Examples: Llanowar Elves "{T}: Add {G}", Birds of Paradise "Add one mana of any color"
fn get_creature_mana_production(card: &crate::core::Card) -> Option<ManaProduction> {
    let text_lower = card.text.to_lowercase();

    // Check for any-color production (Birds of Paradise pattern)
    if text_lower.contains("any color") {
        return Some(ManaProduction::free(ManaProductionKind::AnyColor));
    }

    // Check for specific color production patterns
    // Pattern: "{T}: Add {G}" or similar
    if text_lower.contains("{t}: add {w}") || text_lower.contains("add {w}") {
        return Some(ManaProduction::free(ManaProductionKind::Fixed(ManaColor::White)));
    }
    if text_lower.contains("{t}: add {u}") || text_lower.contains("add {u}") {
        return Some(ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Blue)));
    }
    if text_lower.contains("{t}: add {b}") || text_lower.contains("add {b}") {
        return Some(ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Black)));
    }
    if text_lower.contains("{t}: add {r}") || text_lower.contains("add {r}") {
        return Some(ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Red)));
    }
    if text_lower.contains("{t}: add {g}") || text_lower.contains("add {g}") {
        return Some(ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Green)));
    }
    if text_lower.contains("{t}: add {c}") || text_lower.contains("add {c}") {
        return Some(ManaProduction::free(ManaProductionKind::Colorless));
    }

    None
}

/// Determine mana production for complex lands
///
/// Analyzes card subtypes and abilities to determine what mana a land can produce.
/// Returns None if this isn't a mana-producing land or should be handled by simple check.
fn get_complex_mana_production(card: &crate::core::Card) -> Option<ManaProduction> {
    use crate::core::CardType;

    // Must be a land
    if !card.types.contains(&CardType::Land) {
        return None;
    }

    // Check for dual lands by looking at basic land subtypes
    let mut colors = Vec::new();

    // Check subtypes for basic land types
    for subtype in &card.subtypes {
        let color = match subtype.as_str() {
            "Plains" => Some(ManaColor::White),
            "Island" => Some(ManaColor::Blue),
            "Swamp" => Some(ManaColor::Black),
            "Mountain" => Some(ManaColor::Red),
            "Forest" => Some(ManaColor::Green),
            _ => None,
        };
        if let Some(c) = color {
            colors.push(c);
        }
    }

    // If we have exactly 2 basic land subtypes, it's a dual land
    if colors.len() == 2 {
        return Some(ManaProduction::free(ManaProductionKind::Choice(colors)));
    }

    // Check oracle text for any-color lands (City of Brass pattern)
    // Example: "Add one mana of any color"
    let text_lower = card.text.to_lowercase();
    if text_lower.contains("any color") {
        return Some(ManaProduction::free(ManaProductionKind::AnyColor));
    }

    // Not a complex source we can handle yet
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Card, CardType};

    #[test]
    fn test_mana_capacity_total() {
        let capacity = ManaCapacity {
            white: 2,
            blue: 1,
            black: 0,
            red: 3,
            green: 1,
            colorless: 0,
        };
        assert_eq!(capacity.total(), 7);
    }

    #[test]
    fn test_can_pay_simple_exact() {
        let capacity = ManaCapacity {
            white: 2,
            blue: 1,
            black: 1,
            red: 2,
            green: 1,
            colorless: 0,
        };

        // Exact match
        let cost = ManaCost {
            generic: 0,
            white: 2,
            blue: 1,
            black: 1,
            red: 2,
            green: 1,
            colorless: 0,
            x_count: 0,
        };
        assert!(capacity.can_pay_simple(&cost));
    }

    #[test]
    fn test_can_pay_simple_insufficient_color() {
        let capacity = ManaCapacity {
            white: 1,
            blue: 1,
            black: 1,
            red: 1,
            green: 1,
            colorless: 0,
        };

        // Requires 2 red, but we only have 1
        let cost = ManaCost {
            generic: 0,
            white: 0,
            blue: 0,
            black: 0,
            red: 2,
            green: 0,
            colorless: 0,
            x_count: 0,
        };
        assert!(!capacity.can_pay_simple(&cost));
    }

    #[test]
    fn test_can_pay_simple_with_generic() {
        let capacity = ManaCapacity {
            white: 1,
            blue: 1,
            black: 1,
            red: 2,
            green: 1,
            colorless: 0,
        };

        // Cost: 1R (1 generic + 1 red)
        // We have 2 red, so 1 can be used for the red requirement
        // and we have 5 other mana for the generic
        let cost = ManaCost {
            generic: 1,
            white: 0,
            blue: 0,
            black: 0,
            red: 1,
            green: 0,
            colorless: 0,
            x_count: 0,
        };
        assert!(capacity.can_pay_simple(&cost));
    }

    #[test]
    fn test_simple_mana_color_recognition() {
        assert_eq!(get_simple_mana_color("Plains"), Some('W'));
        assert_eq!(get_simple_mana_color("Island"), Some('U'));
        assert_eq!(get_simple_mana_color("Swamp"), Some('B'));
        assert_eq!(get_simple_mana_color("Mountain"), Some('R'));
        assert_eq!(get_simple_mana_color("Forest"), Some('G'));
        assert_eq!(get_simple_mana_color("Wastes"), Some('C'));
        assert_eq!(get_simple_mana_color("City of Brass"), None);
        assert_eq!(get_simple_mana_color("Taiga"), None);
    }

    #[test]
    fn test_mana_engine_update_simple_sources() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Add some basic lands to the battlefield
        let mountain_id = game.next_card_id();
        let mut mountain = Card::new(mountain_id, "Mountain".to_string(), p1_id);
        mountain.types.push(CardType::Land);
        mountain.controller = p1_id;
        game.cards.insert(mountain_id, mountain);
        game.battlefield.add(mountain_id);

        let island_id = game.next_card_id();
        let mut island = Card::new(island_id, "Island".to_string(), p1_id);
        island.types.push(CardType::Land);
        island.controller = p1_id;
        game.cards.insert(island_id, island);
        game.battlefield.add(island_id);

        // Create engine and update
        let mut engine = ManaEngine::new(p1_id);
        engine.update(&game);

        // Should have detected 2 simple sources
        assert_eq!(engine.simple_sources().len(), 2);
        assert_eq!(engine.complex_sources().len(), 0);

        // Should have correct capacity
        assert_eq!(engine.simple_capacity().red, 1);
        assert_eq!(engine.simple_capacity().blue, 1);
        assert_eq!(engine.simple_capacity().total(), 2);
    }

    #[test]
    fn test_mana_engine_can_pay() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Add 3 mountains
        for _ in 0..3 {
            let land_id = game.next_card_id();
            let mut land = Card::new(land_id, "Mountain".to_string(), p1_id);
            land.types.push(CardType::Land);
            land.controller = p1_id;
            game.cards.insert(land_id, land);
            game.battlefield.add(land_id);
        }

        let mut engine = ManaEngine::new(p1_id);
        engine.update(&game);

        // Should be able to pay for 2R (Lightning Bolt)
        let bolt_cost = ManaCost::from_string("2R");
        assert!(engine.can_pay(&bolt_cost));

        // Should not be able to pay for 4R
        let expensive_cost = ManaCost::from_string("4R");
        assert!(!engine.can_pay(&expensive_cost));

        // Should not be able to pay for 1U (requires blue)
        let blue_cost = ManaCost::from_string("1U");
        assert!(!engine.can_pay(&blue_cost));
    }

    #[test]
    fn test_creature_mana_ability_detection() {
        use crate::core::EntityId;

        let p1_id = EntityId::new(0);

        // Test Llanowar Elves pattern: "{T}: Add {G}"
        let mut llanowar = Card::new(EntityId::new(1), "Llanowar Elves".to_string(), p1_id);
        llanowar.types.push(CardType::Creature);
        llanowar.text = "{T}: Add {G}.".to_string();
        assert!(has_mana_ability(&llanowar));
        assert_eq!(
            get_creature_mana_production(&llanowar),
            Some(ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Green)))
        );

        // Test Birds of Paradise pattern: "Add one mana of any color"
        let mut birds = Card::new(EntityId::new(2), "Birds of Paradise".to_string(), p1_id);
        birds.types.push(CardType::Creature);
        birds.text = "{T}: Add one mana of any color.".to_string();
        assert!(has_mana_ability(&birds));
        assert_eq!(
            get_creature_mana_production(&birds),
            Some(ManaProduction::free(ManaProductionKind::AnyColor))
        );

        // Test non-mana creature
        let mut bear = Card::new(EntityId::new(3), "Grizzly Bears".to_string(), p1_id);
        bear.types.push(CardType::Creature);
        bear.text = "".to_string();
        assert!(!has_mana_ability(&bear));
    }

    #[test]
    fn test_mana_engine_with_llanowar_elves() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Add a Forest and Llanowar Elves
        let forest_id = game.next_card_id();
        let mut forest = Card::new(forest_id, "Forest".to_string(), p1_id);
        forest.types.push(CardType::Land);
        forest.controller = p1_id;
        game.cards.insert(forest_id, forest);
        game.battlefield.add(forest_id);

        let elf_id = game.next_card_id();
        let mut elf = Card::new(elf_id, "Llanowar Elves".to_string(), p1_id);
        elf.types.push(CardType::Creature);
        elf.controller = p1_id;
        elf.text = "{T}: Add {G}.".to_string();
        elf.turn_entered_battlefield = Some(game.turn.turn_number - 1); // Not summoning sick
        game.cards.insert(elf_id, elf);
        game.battlefield.add(elf_id);

        let mut engine = ManaEngine::new(p1_id);
        engine.update(&game);

        // Should have 1 simple source (Forest) and 1 complex source (Llanowar Elves)
        assert_eq!(engine.simple_sources().len(), 1);
        assert_eq!(engine.complex_sources().len(), 1);

        // Should be able to pay for GG (2 green mana)
        let gg_cost = ManaCost::from_string("GG");
        assert!(engine.can_pay(&gg_cost));

        // Should be able to pay for 1G (1 generic + 1 green)
        let one_g_cost = ManaCost::from_string("1G");
        assert!(engine.can_pay(&one_g_cost));
    }

    #[test]
    fn test_mana_engine_summoning_sickness() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Add a Forest
        let forest_id = game.next_card_id();
        let mut forest = Card::new(forest_id, "Forest".to_string(), p1_id);
        forest.types.push(CardType::Land);
        forest.controller = p1_id;
        game.cards.insert(forest_id, forest);
        game.battlefield.add(forest_id);

        // Add Llanowar Elves with summoning sickness (entered this turn)
        let elf_id = game.next_card_id();
        let mut elf = Card::new(elf_id, "Llanowar Elves".to_string(), p1_id);
        elf.types.push(CardType::Creature);
        elf.controller = p1_id;
        elf.text = "{T}: Add {G}.".to_string();
        elf.turn_entered_battlefield = Some(game.turn.turn_number); // Summoning sick!
        game.cards.insert(elf_id, elf);
        game.battlefield.add(elf_id);

        let mut engine = ManaEngine::new(p1_id);
        engine.update(&game);

        // Should detect the creature as complex source
        assert_eq!(engine.complex_sources().len(), 1);

        // The mana source should have summoning_sickness flag set
        let creature_source = engine
            .mana_sources
            .iter()
            .find(|s| s.card_id == elf_id)
            .expect("Should find Llanowar Elves");
        assert!(creature_source.has_summoning_sickness);

        // Should only be able to pay for G (from Forest), not GG
        let g_cost = ManaCost::from_string("G");
        assert!(engine.can_pay(&g_cost));

        let gg_cost = ManaCost::from_string("GG");
        assert!(!engine.can_pay(&gg_cost)); // Can't use summoning-sick creature
    }
}
