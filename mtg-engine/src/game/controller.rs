//! Player controller interface
//!
//! This module defines the PlayerController trait that AI and UI implementations
//! must implement. The design matches Java Forge's PlayerController.java where
//! the controller chooses from available spell abilities (lands, spells, abilities)
//! and makes decisions during the spell casting process.
//!
//! ## Key Differences from Previous Design
//!
//! - **Unified Spell Ability Selection**: Instead of separate methods for lands
//!   and spells, `choose_spell_ability_to_play()` returns any playable ability
//! - **Correct Mana Timing**: Mana is tapped during step 6 of 8 in the casting
//!   process, AFTER the spell is on the stack
//! - **Callback-Based Casting**: Controller provides callbacks for targeting and
//!   mana payment during the casting sequence

use crate::core::{CardId, ManaCost, PlayerId, SpellAbility};
use crate::game::{GameState, Step};
use crate::loader::CardDefinition;
use crate::zones::Zone;
use smallvec::SmallVec;
use std::collections::HashMap;

/// Format available spell/ability choices as a menu
///
/// This creates a standardized menu showing all available actions for a player.
/// The format matches the rich input syntax for easy copy-paste:
/// - `[0] pass` - Pass priority
/// - `[1] play Mountain` - Play a land
/// - `[2] cast Lightning Bolt` - Cast a spell
/// - `[3] activate Forest` - Activate an ability
///
/// All controllers should use this function when showing choices to maintain
/// a consistent format across the codebase.
///
/// See docs/FIXED_INPUT_SYNTAX.md for full input syntax documentation.
pub fn format_choice_menu(view: &GameStateView, available: &[SpellAbility]) -> String {
    let mut output = String::new();
    let player_name = view.player_name();

    output.push_str(&format!("\n{} available actions:\n", player_name));

    // Pass is ALWAYS option 0
    output.push_str("  [0] pass\n");

    // Sort abilities in canonical order: PlayLand, CastSpell, ActivateAbility
    let sorted = sort_spell_abilities(available);

    // Actions are indexed starting at 1
    for (idx, ability) in sorted.iter().enumerate() {
        let display_idx = idx + 1; // Shift indices by 1 to make room for pass at 0
        match ability {
            SpellAbility::PlayLand { card_id } => {
                let name = view.card_name(*card_id).unwrap_or_default();
                output.push_str(&format!("  [{}] play {}\n", display_idx, name));
            }
            SpellAbility::CastSpell { card_id } => {
                let name = view.card_name(*card_id).unwrap_or_default();
                output.push_str(&format!("  [{}] cast {}\n", display_idx, name));
            }
            SpellAbility::ActivateAbility { card_id, .. } => {
                let name = view.card_name(*card_id).unwrap_or_default();
                output.push_str(&format!("  [{}] activate {}\n", display_idx, name));
            }
            SpellAbility::CastFromExile {
                card_id,
                alternative_cost,
                ..
            } => {
                let name = view.card_name(*card_id).unwrap_or_default();
                output.push_str(&format!(
                    "  [{}] Cast from exile: {} (for {})\n",
                    display_idx, name, alternative_cost
                ));
            }
            SpellAbility::CastFromCommand { card_id, total_cost } => {
                let name = view.card_name(*card_id).unwrap_or_default();
                output.push_str(&format!(
                    "  [{}] cast {} from command zone ({})\n",
                    display_idx, name, total_cost
                ));
            }
            SpellAbility::Cycle {
                card_id,
                cost,
                search_type,
            } => {
                let name = view.card_name(*card_id).unwrap_or_default();
                let type_str = match search_type {
                    Some(st) => format!("{}cycling", st.as_str()),
                    None => "Cycling".to_string(),
                };
                output.push_str(&format!("  [{}] {} {} ({})\n", display_idx, type_str, name, cost));
            }
            SpellAbility::CastFromGraveyard { card_id, .. } => {
                let name = view.card_name(*card_id).unwrap_or_default();
                output.push_str(&format!(
                    "  [{}] cast from graveyard: {} (with finality)\n",
                    display_idx, name
                ));
            }
        }
    }

    output
}

/// Format attacker selection prompt
///
/// This creates a standardized prompt for the declare attackers step.
/// Controllers don't need to print this themselves - it's printed by the game loop.
pub fn format_attackers_prompt(view: &GameStateView, available_creatures: &[CardId]) -> String {
    let mut output = String::new();
    let player_name = view.player_name();

    output.push_str(&format!("\n--- Declare Attackers ({}) ---\n", player_name));

    if available_creatures.is_empty() {
        output.push_str("  No creatures available to attack\n");
    } else {
        output.push_str(&format!("Available creatures ({}):\n", available_creatures.len()));
        for (idx, &card_id) in available_creatures.iter().enumerate() {
            let name = view.card_name(card_id).unwrap_or_else(|| format!("Card {card_id:?}"));
            let tapped = if view.is_tapped(card_id) { " (tapped)" } else { "" };

            // Try to get power/toughness info (using effective P/T with anthem effects)
            if let Some(card) = view.get_card(card_id) {
                if card.is_creature() {
                    let power = view
                        .get_effective_power(card_id)
                        .unwrap_or_else(|| i32::from(card.current_power()));
                    let toughness = view
                        .get_effective_toughness(card_id)
                        .unwrap_or_else(|| i32::from(card.current_toughness()));
                    output.push_str(&format!("  [{}] {} {}/{}{}\n", idx, name, power, toughness, tapped));
                } else {
                    output.push_str(&format!("  [{}] {}{}\n", idx, name, tapped));
                }
            } else {
                output.push_str(&format!("  [{}] {}{}\n", idx, name, tapped));
            }
        }
    }

    output
}

/// Format blocker selection prompt
///
/// This creates a standardized prompt for the declare blockers step.
/// Controllers don't need to print this themselves - it's printed by the game loop.
pub fn format_blockers_prompt(view: &GameStateView, available_blockers: &[CardId], attackers: &[CardId]) -> String {
    let mut output = String::new();
    let player_name = view.player_name();

    output.push_str(&format!("\n--- Declare Blockers ({}) ---\n", player_name));

    output.push_str(&format!("Attacking creatures ({}):\n", attackers.len()));
    for (idx, &card_id) in attackers.iter().enumerate() {
        let name = view.card_name(card_id).unwrap_or_else(|| format!("Card {card_id:?}"));
        if let Some(card) = view.get_card(card_id) {
            if card.is_creature() {
                let power = view
                    .get_effective_power(card_id)
                    .unwrap_or_else(|| i32::from(card.current_power()));
                let toughness = view
                    .get_effective_toughness(card_id)
                    .unwrap_or_else(|| i32::from(card.current_toughness()));
                output.push_str(&format!("  [{}] {} {}/{}\n", idx, name, power, toughness));
            } else {
                output.push_str(&format!("  [{}] {}\n", idx, name));
            }
        } else {
            output.push_str(&format!("  [{}] {}\n", idx, name));
        }
    }

    if available_blockers.is_empty() {
        output.push_str("\nNo creatures available to block\n");
    } else {
        output.push_str(&format!("\nAvailable blockers ({}):\n", available_blockers.len()));
        for (idx, &card_id) in available_blockers.iter().enumerate() {
            let name = view.card_name(card_id).unwrap_or_else(|| format!("Card {card_id:?}"));
            let tapped = if view.is_tapped(card_id) { " (tapped)" } else { "" };

            if let Some(card) = view.get_card(card_id) {
                if card.is_creature() {
                    let power = view
                        .get_effective_power(card_id)
                        .unwrap_or_else(|| i32::from(card.current_power()));
                    let toughness = view
                        .get_effective_toughness(card_id)
                        .unwrap_or_else(|| i32::from(card.current_toughness()));
                    output.push_str(&format!("  [{}] {} {}/{}{}\n", idx, name, power, toughness, tapped));
                } else {
                    output.push_str(&format!("  [{}] {}{}\n", idx, name, tapped));
                }
            } else {
                output.push_str(&format!("  [{}] {}{}\n", idx, name, tapped));
            }
        }
    }

    output
}

/// Format discard selection prompt
///
/// This creates a standardized prompt for discarding cards to hand size.
/// Controllers don't need to print this themselves - it's printed by the game loop.
pub fn format_discard_prompt(view: &GameStateView, hand: &[CardId], count: usize) -> String {
    let mut output = String::new();
    let player_name = view.player_name();

    output.push_str(&format!("\n--- Discard to Hand Size ({}) ---\n", player_name));
    output.push_str(&format!("Must discard {} card(s)\n", count));

    output.push_str(&format!("\nYour hand ({} cards):\n", hand.len()));
    for (idx, &card_id) in hand.iter().enumerate() {
        let name = view.card_name(card_id).unwrap_or_else(|| format!("Card {card_id:?}"));
        output.push_str(&format!("  [{}] {}\n", idx, name));
    }

    output
}

/// Format target selection prompt
///
/// This creates a standardized prompt for choosing targets for a spell or ability.
/// Controllers don't need to print this themselves - it's printed by the game loop.
pub fn format_targets_prompt(view: &GameStateView, spell: CardId, valid_targets: &[CardId]) -> String {
    let mut output = String::new();
    let spell_name = view.card_name(spell).unwrap_or_else(|| format!("Card {spell:?}"));

    output.push_str(&format!("\n--- Choose Targets for: {} ---\n", spell_name));

    if valid_targets.is_empty() {
        output.push_str("  No valid targets\n");
    } else {
        output.push_str(&format!("Valid targets ({}):\n", valid_targets.len()));
        for (idx, &card_id) in valid_targets.iter().enumerate() {
            let name = view.card_name(card_id).unwrap_or_else(|| format!("Card {card_id:?}"));
            let tapped = if view.is_tapped(card_id) { " (tapped)" } else { "" };

            // Try to get additional info (using effective P/T with anthem effects)
            if let Some(card) = view.get_card(card_id) {
                if card.is_creature() {
                    let power = view
                        .get_effective_power(card_id)
                        .unwrap_or_else(|| i32::from(card.current_power()));
                    let toughness = view
                        .get_effective_toughness(card_id)
                        .unwrap_or_else(|| i32::from(card.current_toughness()));
                    output.push_str(&format!("  [{}] {} {}/{}{}\n", idx, name, power, toughness, tapped));
                } else {
                    output.push_str(&format!("  [{}] {}{}\n", idx, name, tapped));
                }
            } else {
                output.push_str(&format!("  [{}] {}{}\n", idx, name, tapped));
            }
        }
    }

    output
}

// =============================================================================
// Shared Menu Formatting Functions
// =============================================================================
// These functions provide consistent formatting for menu choices across both
// native TUI and WASM implementations. Both implementations should use these
// to ensure identical user-facing strings.

/// Get sort key for a SpellAbility
///
/// Returns a number used for canonical ordering:
/// 0 = PlayLand, 1 = CastSpell, 2 = CastFromExile, 3 = ActivateAbility, 4 = Cycle
fn spell_ability_sort_key(ability: &SpellAbility) -> u8 {
    match ability {
        SpellAbility::PlayLand { .. } => 0,
        SpellAbility::CastSpell { .. } => 1,
        SpellAbility::CastFromCommand { .. } => 2,
        SpellAbility::CastFromExile { .. } => 3,
        SpellAbility::ActivateAbility { .. } => 4,
        SpellAbility::Cycle { .. } => 5,
        SpellAbility::CastFromGraveyard { .. } => 6,
    }
}

/// Sort spell abilities in canonical order
///
/// Order: PlayLand first, then CastSpell, then ActivateAbility.
/// Pass is always index 0 in formatted choices, but that's handled separately.
pub fn sort_spell_abilities(abilities: &[SpellAbility]) -> Vec<SpellAbility> {
    let mut sorted: Vec<SpellAbility> = abilities.to_vec();
    sorted.sort_by_key(spell_ability_sort_key);
    sorted
}

/// Format a single spell ability choice for display
///
/// Returns a string matching the rich input syntax:
/// - "play Forest" for lands
/// - "cast Lightning Bolt" for spells
/// - "activate Forest" for abilities
///
/// See docs/FIXED_INPUT_SYNTAX.md for full input syntax documentation.
pub fn format_spell_ability_choice(view: &GameStateView, ability: &SpellAbility) -> String {
    match ability {
        SpellAbility::PlayLand { card_id } => {
            let name = view.card_name(*card_id).unwrap_or_default();
            format!("play {}", name)
        }
        SpellAbility::CastSpell { card_id } => {
            let name = view.card_name(*card_id).unwrap_or_default();
            format!("cast {}", name)
        }
        SpellAbility::ActivateAbility { card_id, .. } => {
            let name = view.card_name(*card_id).unwrap_or_default();
            format!("activate {}", name)
        }
        SpellAbility::CastFromExile {
            card_id,
            alternative_cost,
            ..
        } => {
            let name = view.card_name(*card_id).unwrap_or_default();
            format!("Cast from exile: {} (for {})", name, alternative_cost)
        }
        SpellAbility::CastFromCommand { card_id, total_cost } => {
            let name = view.card_name(*card_id).unwrap_or_default();
            format!("cast {} from command zone ({})", name, total_cost)
        }
        SpellAbility::Cycle {
            card_id,
            cost,
            search_type,
        } => {
            let name = view.card_name(*card_id).unwrap_or_default();
            let type_str = match search_type {
                Some(st) => format!("{}cycling", st.as_str()),
                None => "cycle".to_string(),
            };
            format!("{} {} ({})", type_str, name, cost)
        }
        SpellAbility::CastFromGraveyard { card_id, .. } => {
            let name = view.card_name(*card_id).unwrap_or_default();
            format!("cast from graveyard {} (with finality)", name)
        }
    }
}

///
/// Index 0 is always "pass", subsequent indices are formatted abilities
/// using the rich input syntax (e.g., "play Mountain", "cast Lightning Bolt").
///
/// This is used by both TUI and WASM to generate menu choices.
///
/// See docs/FIXED_INPUT_SYNTAX.md for full input syntax documentation.
pub fn format_spell_ability_choices(view: &GameStateView, available: &[SpellAbility]) -> Vec<String> {
    std::iter::once("pass".to_string())
        .chain(
            available
                .iter()
                .map(|ability| format_spell_ability_choice(view, ability)),
        )
        .collect()
}

/// Format a single card for target/selection display
///
/// Includes ownership indicator and ID disambiguation when needed.
/// Format: "<Name> #ID [T] (yours)" or "<Name> (theirs)"
///
/// # Arguments
/// * `view` - Game state view
/// * `card_id` - The card to format
/// * `viewer_id` - The player viewing this choice (for ownership)
/// * `name_counts` - Map of card names to occurrence counts (for ID disambiguation)
pub fn format_card_choice(
    view: &GameStateView,
    card_id: CardId,
    viewer_id: PlayerId,
    name_counts: &HashMap<String, usize>,
) -> String {
    let name = view.card_name(card_id).unwrap_or_default();

    // Determine ownership
    let controller = view.get_card(card_id).map(|c| c.controller);
    let ownership = if controller == Some(viewer_id) {
        "(yours)"
    } else {
        "(theirs)"
    };

    // Show ID only if there are duplicates of this card name
    let id_part = if *name_counts.get(&name).unwrap_or(&0) > 1 {
        format!(" #{}", card_id.as_u32())
    } else {
        String::new()
    };

    let tapped = if view.is_tapped(card_id) { " [T]" } else { "" };
    format!("{}{}{} {}", name, id_part, tapped, ownership)
}

/// Count occurrences of each card name in a list
///
/// Used by format_card_choice to determine if ID disambiguation is needed.
pub fn count_card_names(view: &GameStateView, cards: &[CardId]) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for &card_id in cards {
        let name = view.card_name(card_id).unwrap_or_default();
        *counts.entry(name).or_insert(0) += 1;
    }
    counts
}

/// Format all cards for target selection display
///
/// Each card is formatted with ownership and ID disambiguation.
pub fn format_card_choices(view: &GameStateView, cards: &[CardId], viewer_id: PlayerId) -> Vec<String> {
    let name_counts = count_card_names(view, cards);
    cards
        .iter()
        .map(|&card_id| format_card_choice(view, card_id, viewer_id, &name_counts))
        .collect()
}

/// Format target choices with "No target" as first option
///
/// Returns a Vec where index 0 is "No target" and subsequent indices are formatted targets.
pub fn format_target_choices(view: &GameStateView, valid_targets: &[CardId], viewer_id: PlayerId) -> Vec<String> {
    std::iter::once("No target".to_string())
        .chain(format_card_choices(view, valid_targets, viewer_id))
        .collect()
}

/// Format attacker choices with "Done" as first option
///
/// Returns a Vec where index 0 is "Done" and subsequent indices are formatted creatures.
pub fn format_attacker_choices(
    view: &GameStateView,
    available_creatures: &[CardId],
    viewer_id: PlayerId,
) -> Vec<String> {
    std::iter::once("Done".to_string())
        .chain(format_card_choices(view, available_creatures, viewer_id))
        .collect()
}

/// Format blocker assignment choices
///
/// Returns a Vec where index 0 is "Done" and subsequent entries are "Blocker blocks Attacker" pairs.
pub fn format_blocker_choices(
    view: &GameStateView,
    available_blockers: &[CardId],
    attackers: &[CardId],
    viewer_id: PlayerId,
) -> Vec<String> {
    let blocker_names = format_card_choices(view, available_blockers, viewer_id);
    let attacker_names = format_card_choices(view, attackers, viewer_id);

    let mut choices = vec!["Done".to_string()];
    for blocker_name in &blocker_names {
        for attacker_name in &attacker_names {
            choices.push(format!("{} blocks {}", blocker_name, attacker_name));
        }
    }
    choices
}

/// Get the standard prompt for spell/ability selection
pub fn prompt_spell_ability(player_name: &str) -> String {
    format!("Priority {}: Choose action", player_name)
}

/// Get the standard prompt for target selection
pub fn prompt_target(spell_name: &str) -> String {
    format!("Choose target for: {}", spell_name)
}

/// Get the standard prompt for attacker declaration
pub const PROMPT_ATTACKERS: &str = "Declare Attackers (select creatures or Done)";

/// Get the standard prompt for blocker declaration
pub const PROMPT_BLOCKERS: &str = "Declare Blockers (select pairs or Done)";

/// Get the standard prompt for mana source selection
pub fn prompt_mana_source(current: usize, total: usize) -> String {
    format!("Pay mana {}/{}: Select source", current, total)
}

/// Get the standard prompt for discard selection
pub fn prompt_discard(count: usize) -> String {
    format!("Discard {} card(s):", count)
}

/// Get the standard prompt for library search
pub const PROMPT_LIBRARY_SEARCH: &str = "Search library:";

/// Get the standard prompt for damage order assignment
pub const PROMPT_DAMAGE_ORDER: &str = "Choose damage order:";

/// Read-only view of game state for controllers
///
/// This provides access to game information without allowing mutation.
/// Controllers should only inspect this view to make decisions.
pub struct GameStateView<'a> {
    game: &'a GameState,
    player_id: PlayerId,
}

impl<'a> GameStateView<'a> {
    /// Create a new view of the game state from a player's perspective
    pub fn new(game: &'a GameState, player_id: PlayerId) -> Self {
        GameStateView { game, player_id }
    }

    /// Get the player ID this view is for
    pub fn player_id(&self) -> PlayerId {
        self.player_id
    }

    /// Get cards in this player's hand
    pub fn hand(&self) -> &[CardId] {
        self.game
            .get_player_zones(self.player_id)
            .map(|zones| zones.hand.cards.as_slice())
            .unwrap_or(&[])
    }

    /// Get cards in a specific player's hand
    pub fn player_hand(&self, player_id: PlayerId) -> &[CardId] {
        self.game
            .get_player_zones(player_id)
            .map(|zones| zones.hand.cards.as_slice())
            .unwrap_or(&[])
    }

    /// Get a player's hand SIZE (including hidden cards in network mode)
    ///
    /// This is used for network hash computation where we need the total
    /// hand size for any player.
    pub fn player_hand_size(&self, player_id: PlayerId) -> usize {
        self.game
            .get_player_zones(player_id)
            .map(|zones| zones.hand.len())
            .unwrap_or(0)
    }

    /// Get cards on the battlefield
    pub fn battlefield(&self) -> &[CardId] {
        &self.game.battlefield.cards
    }

    /// Get cards in this player's graveyard
    pub fn graveyard(&self) -> &[CardId] {
        self.game
            .get_player_zones(self.player_id)
            .map(|zones| zones.graveyard.cards.as_slice())
            .unwrap_or(&[])
    }

    /// Get cards in a specific player's graveyard
    pub fn player_graveyard(&self, player_id: PlayerId) -> &[CardId] {
        self.game
            .get_player_zones(player_id)
            .map(|zones| zones.graveyard.cards.as_slice())
            .unwrap_or(&[])
    }

    /// Get cards in a player's command zone
    pub fn player_command_zone(&self, player_id: PlayerId) -> &[CardId] {
        self.game
            .get_player_zones(player_id)
            .map(|zones| zones.command.cards.as_slice())
            .unwrap_or(&[])
    }

    /// Get number of cards in a player's graveyard
    pub fn player_graveyard_size(&self, player_id: PlayerId) -> usize {
        self.game
            .get_player_zones(player_id)
            .map(|zones| zones.graveyard.len())
            .unwrap_or(0)
    }

    /// Get cards in a specific player's library
    pub fn player_library(&self, player_id: PlayerId) -> &[CardId] {
        self.game
            .get_player_zones(player_id)
            .map(|zones| zones.library.cards.as_slice())
            .unwrap_or(&[])
    }

    /// Get the size of a player's library
    ///
    /// This differs from `player_library().len()` because it correctly handles
    /// remote libraries (client shadow state) where the cards vector is empty
    /// but the size is tracked separately.
    pub fn player_library_size(&self, player_id: PlayerId) -> usize {
        self.game
            .get_player_zones(player_id)
            .map(|zones| zones.library.len())
            .unwrap_or(0)
    }

    /// Check if a card is in a specific zone
    pub fn is_card_in_zone(&self, card_id: CardId, zone: Zone) -> bool {
        match zone {
            Zone::Hand => self
                .game
                .get_player_zones(self.player_id)
                .map(|z| z.hand.contains(card_id))
                .unwrap_or(false),
            Zone::Battlefield => self.game.battlefield.contains(card_id),
            Zone::Graveyard => self
                .game
                .get_player_zones(self.player_id)
                .map(|z| z.graveyard.contains(card_id))
                .unwrap_or(false),
            Zone::Library => self
                .game
                .get_player_zones(self.player_id)
                .map(|z| z.library.contains(card_id))
                .unwrap_or(false),
            Zone::Stack => self.game.stack.contains(card_id),
            Zone::Exile => self
                .game
                .get_player_zones(self.player_id)
                .map(|z| z.exile.contains(card_id))
                .unwrap_or(false),
            Zone::Command => false, // Command zone not yet implemented
        }
    }

    /// Get a card's name
    pub fn card_name(&self, card_id: CardId) -> Option<String> {
        self.game.cards.try_get(card_id).map(|c| c.name.to_string())
    }

    /// Check if a card is a land
    pub fn is_land(&self, card_id: CardId) -> bool {
        self.game.cards.try_get(card_id).is_some_and(|c| c.is_land())
    }

    /// Get the current step
    pub fn current_step(&self) -> Step {
        self.game.turn.current_step
    }

    /// Get the current turn number
    pub fn turn_number(&self) -> u32 {
        self.game.turn.turn_number
    }

    /// Get the active player (whose turn it is)
    pub fn active_player(&self) -> PlayerId {
        self.game.turn.active_player
    }

    /// Get a card's name (convenience method)
    pub fn get_card_name(&self, card_id: CardId) -> Option<String> {
        self.card_name(card_id)
    }

    /// Get a reference to a card
    ///
    /// This allows controllers to inspect card properties for decision-making.
    /// Uses try_get() for efficiency in hot paths.
    pub fn get_card(&self, card_id: CardId) -> Option<&crate::core::Card> {
        self.game.cards.try_get(card_id)
    }

    /// Check if a card is tapped
    pub fn is_tapped(&self, card_id: CardId) -> bool {
        self.game.cards.try_get(card_id).is_some_and(|c| c.tapped)
    }

    /// Get access to the game logger
    ///
    /// This allows controllers and other game components to log messages
    /// at appropriate verbosity levels without needing to track verbosity themselves.
    pub fn logger(&self) -> &crate::game::GameLogger {
        &self.game.logger
    }

    /// Get player's current life total
    pub fn life(&self) -> i32 {
        self.game.get_player(self.player_id).ok().map(|p| p.life).unwrap_or(0)
    }

    /// Get player's name
    pub fn player_name(&self) -> String {
        self.get_player_name_by_id(self.player_id)
    }

    /// Get a specific player's name
    pub fn get_player_name_by_id(&self, player_id: PlayerId) -> String {
        self.game
            .get_player(player_id)
            .ok()
            .map(|p| p.name.to_string())
            .unwrap_or_else(|| {
                // Use 1-based indexing for human-readable player numbers
                let player_num = player_id.as_u32() + 1;
                format!("Player {}", player_num)
            })
    }

    /// Get a specific player's life total
    pub fn player_life(&self, player_id: PlayerId) -> i32 {
        self.game.get_player(player_id).ok().map(|p| p.life).unwrap_or(0)
    }

    /// Get all opponent player IDs
    ///
    /// Returns an iterator over all players except the current player.
    /// Useful for multiplayer games.
    pub fn opponents(&self) -> impl Iterator<Item = PlayerId> + '_ {
        self.game
            .players
            .iter()
            .map(|p| p.id)
            .filter(move |&id| id != self.player_id)
    }

    /// Get opponent life total in a 2-player game
    ///
    /// For 2-player games, returns the opponent's life total.
    /// For multiplayer, returns the total life of all opponents combined.
    pub fn opponent_life(&self) -> i32 {
        self.opponents().map(|id| self.player_life(id)).sum()
    }

    /// Get player's mana pool
    pub fn available_mana(&self) -> (u8, u8, u8, u8, u8, u8) {
        // Returns (white, blue, black, red, green, colorless)
        self.game
            .get_player(self.player_id)
            .ok()
            .map(|p| {
                (
                    p.mana_pool.white,
                    p.mana_pool.blue,
                    p.mana_pool.black,
                    p.mana_pool.red,
                    p.mana_pool.green,
                    p.mana_pool.colorless,
                )
            })
            .unwrap_or((0, 0, 0, 0, 0, 0))
    }

    /// Get maximum mana capacity for this player
    ///
    /// Returns the maximum amount of mana of each color that could be produced
    /// if all untapped mana sources were tapped. This accounts for:
    /// - Basic lands (produce one specific color)
    /// - Dual lands (produce choice of X or Y, counted in both colors)
    /// - Any-color lands (counted in all colors)
    /// - Mana creatures like Llanowar Elves (if not summoning sick)
    ///
    /// The return value is (total_sources, W, U, B, R, G, C) where total_sources
    /// is the count of untapped sources, and each color is the max of that color
    /// we could produce.
    ///
    /// Note: For dual lands, they count +1 for both colors but only +1 to total.
    pub fn max_mana_capacity(&self) -> (u8, u8, u8, u8, u8, u8, u8) {
        use crate::game::ManaEngine;

        let mut engine = ManaEngine::new();
        engine.update(self.game, self.player_id);

        let capacity = engine.max_mana_capacity();
        let total = engine.simple_sources().len() + engine.complex_sources().len();

        (
            total as u8,
            capacity.white,
            capacity.blue,
            capacity.black,
            capacity.red,
            capacity.green,
            capacity.colorless,
        )
    }

    /// Check if player can play lands this turn
    pub fn can_play_land(&self) -> bool {
        self.game
            .get_player(self.player_id)
            .ok()
            .map(|p| p.can_play_land())
            .unwrap_or(false)
    }

    /// Get cards on the stack
    ///
    /// Returns cards on the stack in order (bottom to top).
    pub fn stack(&self) -> &[CardId] {
        &self.game.stack.cards
    }

    /// Check if the stack is empty
    ///
    /// Convenience method for checking if there are no spells or abilities on the stack.
    /// Used by AI controllers to determine timing for activated abilities.
    pub fn is_stack_empty(&self) -> bool {
        self.game.stack.is_empty()
    }

    /// Get the current combat state
    ///
    /// Returns information about attackers, blockers, and combat phase status.
    pub fn combat(&self) -> &crate::game::CombatState {
        &self.game.combat
    }

    /// Get the number of actions in the undo log
    ///
    /// Returns the count of reversible actions that have been performed.
    /// Used by the fancy TUI to display action count status.
    pub fn action_count(&self) -> usize {
        self.game.undo_log.len()
    }

    /// Get read-only access to undo log actions
    ///
    /// Used by NetworkController to scan for card reveals since last choice.
    /// Returns a slice of all game actions in chronological order.
    pub fn undo_log_actions(&self) -> &[crate::undo::GameAction] {
        self.game.undo_log.actions()
    }

    /// Format the last N actions as a multi-line string for debugging
    ///
    /// Used for sync debugging in network mode. Returns a string with
    /// one action per line, most recent last, with index prefix.
    pub fn format_last_n_actions(&self, n: usize) -> String {
        self.game.undo_log.format_last_n(n)
    }

    /// Get the number of controller choices made
    ///
    /// Returns the count of times a controller has made a choice.
    /// Used by the fancy TUI to display choice count status alongside action count.
    pub fn choice_count(&self) -> usize {
        self.game.logger.choice_count()
    }

    /// Get a creature's effective power using CR 613 layer system
    ///
    /// Returns the final power after applying all continuous effects (Equipment,
    /// anthems, counters, etc.) in the correct layer order.
    ///
    /// ## Returns
    ///
    /// The effective power, or None if the card is not found or is not a creature.
    pub fn get_effective_power(&self, creature_id: CardId) -> Option<i32> {
        self.game.get_effective_power(creature_id).ok()
    }

    /// Get a creature's effective toughness using CR 613 layer system
    ///
    /// Returns the final toughness after applying all continuous effects (Equipment,
    /// anthems, counters, etc.) in the correct layer order.
    ///
    /// ## Returns
    ///
    /// The effective toughness, or None if the card is not found or is not a creature.
    pub fn get_effective_toughness(&self, creature_id: CardId) -> Option<i32> {
        self.game.get_effective_toughness(creature_id).ok()
    }

    /// Check if a creature has a keyword, including keywords granted by continuous effects
    ///
    /// This method checks both the creature's innate keywords and any keywords
    /// granted by continuous effects (e.g., equipment or auras that grant flying).
    ///
    /// ## Example
    ///
    /// ```ignore
    /// // Check if blocker has indestructible (either innate or granted)
    /// if view.has_keyword_with_effects(blocker_id, Keyword::Indestructible) {
    ///     // Blocker can't be destroyed by damage
    /// }
    /// ```
    pub fn has_keyword_with_effects(&self, card_id: CardId, keyword: crate::core::Keyword) -> bool {
        self.game.has_keyword_with_effects(card_id, keyword)
    }
}

/// Result of a controller choice operation
///
/// This enum allows controllers to return not just a choice, but also
/// special requests like undo, exit, or error conditions.
#[derive(Debug, Clone)]
pub enum ChoiceResult<T> {
    /// Normal choice made successfully
    Ok(T),
    /// Request to undo N actions
    UndoRequest(usize),
    /// Request to cleanly exit the game
    ExitGame,
    /// Error in the controller
    Error(String),
    /// Need human input - pause game loop and return control to caller
    ///
    /// This variant is used by WASM human controllers to signal that the game
    /// should pause and wait for asynchronous user input. The ChoiceContext
    /// contains all information needed to display the choice to the user.
    ///
    /// When the game loop encounters this, it saves its state (turn snapshot +
    /// intra-turn choices) and returns to the caller, which can then display
    /// the choice UI and resume later with the user's selection.
    NeedInput(ChoiceContext),
}

/// Context for a pending choice that requires human input
///
/// This enum captures all the information needed to display a choice to a
/// human player in a WASM/browser context. The game loop packages this up
/// when a human controller returns `ChoiceResult::NeedInput`.
#[derive(Debug, Clone)]
pub enum ChoiceContext {
    /// Choose a spell ability to play (or pass priority)
    SpellAbility {
        /// Available spell abilities (indexed 1..N, with 0 = pass)
        available: Vec<SpellAbility>,
        /// Pre-formatted choice strings for display
        formatted_choices: Vec<String>,
    },
    /// Choose targets for a spell
    Targets {
        /// The spell being targeted
        spell_id: CardId,
        /// Valid target card IDs
        valid_targets: Vec<CardId>,
        /// Pre-formatted target strings for display
        formatted_targets: Vec<String>,
    },
    /// Choose mana sources to tap
    ManaSources {
        /// The cost being paid
        cost: ManaCost,
        /// Available sources to tap
        available_sources: Vec<CardId>,
        /// Pre-formatted source strings for display
        formatted_sources: Vec<String>,
    },
    /// Choose attackers
    Attackers {
        /// Creatures available to attack
        available_creatures: Vec<CardId>,
        /// Pre-formatted creature strings for display
        formatted_creatures: Vec<String>,
    },
    /// Choose blockers
    Blockers {
        /// Creatures available to block
        available_blockers: Vec<CardId>,
        /// Attacking creatures to block
        attackers: Vec<CardId>,
        /// Pre-formatted blocker strings for display
        formatted_blockers: Vec<String>,
        /// Pre-formatted attacker strings for display
        formatted_attackers: Vec<String>,
    },
    /// Choose damage assignment order
    DamageOrder {
        /// The attacking creature
        attacker: CardId,
        /// Blockers to order
        blockers: Vec<CardId>,
        /// Pre-formatted blocker strings for display
        formatted_blockers: Vec<String>,
    },
    /// Choose cards to discard
    Discard {
        /// Cards in hand
        hand: Vec<CardId>,
        /// Number of cards to discard
        count: usize,
        /// Pre-formatted card strings for display
        formatted_hand: Vec<String>,
    },
    /// Choose a card from library
    LibrarySearch {
        /// Valid cards to choose from
        valid_cards: Vec<CardId>,
        /// Pre-formatted card strings for display
        formatted_cards: Vec<String>,
    },
    /// Choose permanents to sacrifice (for Balance, Cataclysm, etc.)
    SacrificePermanents {
        /// Valid permanents that can be sacrificed
        valid_permanents: Vec<CardId>,
        /// Number of permanents to sacrifice
        count: usize,
        /// Description of what type (e.g., "creatures", "lands")
        card_type_description: String,
        /// Pre-formatted permanent strings for display
        formatted_permanents: Vec<String>,
    },
    /// Choose modes for a modal spell (e.g., "Choose one —")
    Modes {
        /// The spell being cast
        spell_id: CardId,
        /// Number of modes to choose
        mode_count: usize,
        /// Minimum modes required
        min_modes: usize,
        /// Whether the same mode can be chosen multiple times
        can_repeat: bool,
        /// Pre-formatted mode description strings for display
        formatted_modes: Vec<String>,
    },
}

impl<T> ChoiceResult<T> {
    /// Helper to check if this is an Ok variant
    pub fn is_ok(&self) -> bool {
        matches!(self, ChoiceResult::Ok(_))
    }

    /// Helper to unwrap the Ok value (panics if not Ok)
    ///
    /// # Panics
    ///
    /// Panics if the result is not `Ok` (i.e., is `UndoRequest`, `ExitGame`, `Error`, or `NeedInput`).
    pub fn unwrap(self) -> T {
        match self {
            ChoiceResult::Ok(value) => value,
            ChoiceResult::UndoRequest(_)
            | ChoiceResult::ExitGame
            | ChoiceResult::Error(_)
            | ChoiceResult::NeedInput(_) => panic!("Called unwrap on non-Ok ChoiceResult"),
        }
    }

    /// Convert to Result for easier handling
    ///
    /// # Errors
    ///
    /// Returns error messages for non-Ok variants (undo request, exit game, need input).
    pub fn into_result(self) -> Result<T, String> {
        match self {
            ChoiceResult::Ok(value) => Ok(value),
            ChoiceResult::Error(msg) => Err(msg),
            ChoiceResult::UndoRequest(n) => Err(format!("Undo request for {} actions", n)),
            ChoiceResult::ExitGame => Err("Exit game requested".to_string()),
            ChoiceResult::NeedInput(_) => Err("Need human input".to_string()),
        }
    }

    /// Check if this is a NeedInput variant
    pub fn is_need_input(&self) -> bool {
        matches!(self, ChoiceResult::NeedInput(_))
    }
}

/// Macro for uniform handling of ChoiceResult at call sites
///
/// This macro reduces verbosity when handling ChoiceResult values in the game loop.
/// It handles all the special cases (UndoRequest, ExitGame, Error) uniformly.
///
/// Usage: Must be called within a loop block:
/// ```ignore
/// let result = loop {
///     let choice = controller.choose_something(...);
///     break handle_choice_result!(choice, game_state, player_id);
/// };
/// ```
///
/// This macro continues to the immediately enclosing loop on undo requests,
/// causing the choice to be re-prompted.
///
/// Special undo values:
/// - `usize::MAX`: Undo to previous choice point for the requesting player
/// - Any other value N: Undo exactly N individual actions
#[macro_export]
macro_rules! handle_choice_result {
    ($result:expr, $game:expr, $player_id:expr) => {
        match $result {
            $crate::game::controller::ChoiceResult::Ok(value) => value,
            $crate::game::controller::ChoiceResult::UndoRequest(n) => {
                if n == usize::MAX {
                    // Special case: undo to previous choice point for the requesting player
                    log::debug!(
                        "[UNDO MACRO] Before undo: undo_log.len()={}, logger.log_count()={}, logger.choice_count()={}",
                        $game.undo_log.len(),
                        $game.logger.log_count(),
                        $game.logger.choice_count()
                    );
                    if let Ok(Some((_actions_undone, choice_log_size))) =
                        $game.undo_to_previous_choice_point($player_id)
                    {
                        log::debug!(
                            "[UNDO MACRO] After undo, before logger truncate: logger.log_count()={}",
                            $game.logger.log_count()
                        );
                        $game.logger.truncate_to(choice_log_size);
                        log::debug!(
                            "[UNDO MACRO] After logger truncate to {}: logger.log_count()={}",
                            choice_log_size,
                            $game.logger.log_count()
                        );
                        // Note: Undo info should be displayed in status bar only, not logged
                    }
                } else {
                    // Normal case: undo N specific actions
                    for _ in 0..n {
                        if let Ok(Some(prior_log_size)) = $game.undo() {
                            $game.logger.truncate_to(prior_log_size);
                        } else {
                            break; // No more actions to undo
                        }
                    }
                }
                // After undo, continue the loop to re-prompt for choice
                continue;
            }
            $crate::game::controller::ChoiceResult::ExitGame => {
                return Err($crate::MtgError::InvalidAction(
                    "Game exit requested by controller".to_string(),
                ));
            }
            $crate::game::controller::ChoiceResult::Error(msg) => {
                return Err($crate::MtgError::InvalidAction(format!("Controller error: {}", msg)));
            }
            $crate::game::controller::ChoiceResult::NeedInput(context) => {
                // Signal that game needs human input
                // This error propagates up to run_until_input() which converts it
                // to GameLoopState::AwaitingInput
                return Err($crate::MtgError::NeedInput(Box::new(context)));
            }
        }
    };
}

/// Macro for handling ChoiceResult in secondary choice contexts
///
/// This variant returns from the enclosing function on undo instead of continuing.
/// Use this for secondary choices (attackers, blockers, targets, etc.) where
/// an undo should exit the current step handler and return control to the main
/// game loop, allowing it to re-evaluate from the rewound game state.
///
/// Usage: Use directly in step handler functions:
/// ```ignore
/// let view = GameStateView::new(self.game, active_player);
/// let choice = controller.choose_attackers(&view, &available_creatures);
/// let attackers = handle_choice_result_break!(choice, self.game, active_player);
/// ```
///
/// On undo, this macro performs the undo and then RETURNS `Ok(None)` from the
/// enclosing function, exiting the step handler. The game loop will then check
/// if the step changed and re-execute from the rewound state.
#[macro_export]
macro_rules! handle_choice_result_break {
    ($result:expr, $game:expr, $player_id:expr) => {
        match $result {
            $crate::game::controller::ChoiceResult::Ok(value) => value,
            $crate::game::controller::ChoiceResult::UndoRequest(n) => {
                if n == usize::MAX {
                    // Special case: undo to previous choice point for the requesting player
                    log::debug!("[UNDO MACRO BREAK] Before undo: undo_log.len()={}, logger.log_count()={}, logger.choice_count()={}",
                              $game.undo_log.len(), $game.logger.log_count(), $game.logger.choice_count());
                    if let Ok(Some((_actions_undone, choice_log_size))) =
                        $game.undo_to_previous_choice_point($player_id)
                    {
                        log::debug!("[UNDO MACRO BREAK] After undo, before logger truncate: logger.log_count()={}", $game.logger.log_count());
                        $game.logger.truncate_to(choice_log_size);
                        log::debug!("[UNDO MACRO BREAK] After logger truncate to {}: logger.log_count()={}", choice_log_size, $game.logger.log_count());
                        // Note: Undo info should be displayed in status bar only, not logged
                    }
                } else {
                    // Normal case: undo N specific actions
                    for _ in 0..n {
                        if let Ok(Some(prior_log_size)) = $game.undo() {
                            $game.logger.truncate_to(prior_log_size);
                        } else {
                            break; // No more actions to undo
                        }
                    }
                }
                // After undo, return from the step handler
                // The game loop will detect that the step changed and re-execute from the rewound state
                return Ok(None);
            }
            $crate::game::controller::ChoiceResult::ExitGame => {
                return Err($crate::MtgError::InvalidAction(
                    "Game exit requested by controller".to_string(),
                ));
            }
            $crate::game::controller::ChoiceResult::Error(msg) => {
                return Err($crate::MtgError::InvalidAction(format!("Controller error: {}", msg)));
            }
            $crate::game::controller::ChoiceResult::NeedInput(context) => {
                // Signal that game needs human input
                // This error propagates up to run_until_input() which converts it
                // to GameLoopState::AwaitingInput
                return Err($crate::MtgError::NeedInput(Box::new(context)));
            }
        }
    };
}

/// Player controller interface
///
/// This trait defines the decision-making interface for players (AI or human).
/// The design matches Java Forge's PlayerController where the controller:
/// 1. Chooses spell abilities to play from a unified list (lands, spells, abilities)
/// 2. Provides callbacks during the spell casting process for targeting and mana payment
/// 3. Makes combat decisions
/// 4. Handles cleanup and notifications
///
/// ## Mana Payment Timing
///
/// Unlike the previous design, mana is NOT tapped during priority rounds.
/// Instead, when a spell is cast, the game follows the 8-step casting process
/// (MTG Rules 601.2), and mana sources are tapped during step 6, which happens
/// AFTER the spell is already on the stack.
pub trait PlayerController {
    /// Get the player ID this controller is responsible for
    fn player_id(&self) -> PlayerId;

    /// Choose a spell ability to play
    ///
    /// This is the main decision point during priority. The controller receives
    /// a list of all available spell abilities:
    /// - Land plays (if can play lands this turn)
    /// - Castable spells (if have mana and in appropriate phase)
    /// - Activated abilities (if can activate)
    ///
    /// Returns ChoiceResult with the chosen ability (or None to pass priority),
    /// or a special request (UndoRequest, ExitGame, Error).
    ///
    /// Controllers that need randomness should maintain their own RNG
    /// (seeded independently from the game engine's RNG).
    ///
    /// ## Java Forge Equivalent
    /// This matches `PlayerController.chooseSpellAbilityToPlay()` which returns
    /// a list of SpellAbilities to play (usually just one, but can be multiple
    /// for special cases like multiple lands in one turn).
    fn choose_spell_ability_to_play(
        &mut self,
        view: &GameStateView,
        available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>>;

    /// Choose targets for a spell or ability
    ///
    /// Called during step 3 of casting a spell (MTG Rules 601.2c).
    /// The controller must choose valid targets from the provided list.
    ///
    /// For spells with no targets, this may not be called, or valid_targets
    /// will be empty.
    ///
    /// Returns ChoiceResult with the chosen targets, or a special request
    /// (UndoRequest, ExitGame, Error).
    ///
    /// ## Java Forge Equivalent
    /// Matches `PlayerController.chooseTargetsFor(SpellAbility)`
    fn choose_targets(
        &mut self,
        view: &GameStateView,
        spell: CardId,
        valid_targets: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>>;

    /// Choose which mana sources to tap to pay a cost
    ///
    /// Called during step 6 of casting a spell (MTG Rules 601.2g).
    /// At this point, the spell is already on the stack.
    ///
    /// The controller must choose which permanents to tap for mana to pay
    /// the given cost. Returns ChoiceResult with the card IDs to tap in order,
    /// or a special request (UndoRequest, ExitGame, Error).
    ///
    /// ## Java Forge Equivalent
    /// This is part of `PlayerController.payManaCost(...)` which the AI
    /// implements with `ComputerUtilMana.payManaCost()`.
    fn choose_mana_sources_to_pay(
        &mut self,
        view: &GameStateView,
        cost: &ManaCost,
        available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>>;

    /// Choose the value of X for a spell with X in its mana cost (MTG CR 601.2b)
    ///
    /// Called during step 2 of casting a spell when the spell's mana cost
    /// contains one or more X symbols. The controller must choose a non-negative
    /// integer for X. The chosen value is multiplied by x_count and added as
    /// generic mana to the total cost.
    ///
    /// ## Parameters
    /// - `view`: Read-only view of the game state
    /// - `spell_id`: The spell being cast (on the stack)
    /// - `max_x`: Maximum X value the player could pay (based on available mana)
    ///
    /// Returns ChoiceResult with the chosen X value (0 to max_x).
    ///
    /// ## Java Forge Equivalent
    /// Matches `PlayerController.announceRequirements()` for X costs
    fn choose_x_value(&mut self, _view: &GameStateView, _spell_id: CardId, max_x: u8) -> ChoiceResult<u8> {
        // Default: choose maximum X value (spend all available mana)
        ChoiceResult::Ok(max_x)
    }

    /// Choose which creatures to declare as attackers
    ///
    /// Called during the declare attackers step.
    /// Returns ChoiceResult with a list of creature card IDs that should attack,
    /// or a special request (UndoRequest, ExitGame, Error).
    fn choose_attackers(
        &mut self,
        view: &GameStateView,
        available_creatures: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>>;

    /// Choose how to block attacking creatures
    ///
    /// Called during the declare blockers step.
    /// Returns ChoiceResult with pairs of (blocker, attacker) indicating which creature
    /// blocks which attacker, or a special request (UndoRequest, ExitGame, Error).
    fn choose_blockers(
        &mut self,
        view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>>;

    /// Choose the damage assignment order for blockers (LEGACY - fallback only)
    ///
    /// Called during combat damage step when an attacker is blocked by multiple creatures.
    /// The attacker's controller chooses the order in which damage will be assigned to blockers.
    /// MTG Rules 509.2: The attacking player announces the damage assignment order.
    ///
    /// NOTE: The engine now uses SMART damage assignment which calls
    /// `choose_blocker_for_lethal_damage` iteratively instead. This method is only
    /// called as a fallback when SMART assignment is disabled.
    ///
    /// Returns ChoiceResult with the blockers in the order damage should be assigned.
    /// All blockers must be included. Can also return special requests (UndoRequest, ExitGame, Error).
    fn choose_damage_assignment_order(
        &mut self,
        view: &GameStateView,
        attacker: CardId,
        blockers: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>>;

    /// SMART damage assignment: Choose which blocker to assign lethal damage to first
    ///
    /// Called iteratively during combat damage step when an attacker has enough power
    /// to kill some (but not all) blockers. The controller chooses which blocker to
    /// prioritize killing.
    ///
    /// # Arguments
    /// * `view` - Current game state view
    /// * `attacker` - The attacking creature
    /// * `killable_blockers` - Blockers that CAN be killed with remaining power
    ///   (each with their lethal damage amount)
    /// * `remaining_power` - How much damage the attacker has left to assign
    ///
    /// # Returns
    /// The CardId of the blocker to assign lethal damage to first.
    /// Default implementation delegates to choose_damage_assignment_order.
    fn choose_blocker_for_lethal_damage(
        &mut self,
        _view: &GameStateView,
        _attacker: CardId,
        killable_blockers: &[(CardId, i32)], // (blocker_id, lethal_damage_needed)
        _remaining_power: i32,
    ) -> ChoiceResult<CardId> {
        // Default: use the first killable blocker (fallback behavior)
        if let Some((blocker_id, _)) = killable_blockers.first() {
            ChoiceResult::Ok(*blocker_id)
        } else {
            ChoiceResult::Error("No killable blockers provided".to_string())
        }
    }

    /// SMART damage assignment: Choose where to assign remaining non-lethal damage
    ///
    /// Called after lethal damage has been assigned to all killable blockers.
    /// The remaining damage cannot kill any blocker, so the controller chooses
    /// where to "waste" this damage. May matter for effects that care about
    /// damage dealt, or for future pump spells.
    ///
    /// # Arguments
    /// * `view` - Current game state view
    /// * `attacker` - The attacking creature
    /// * `remaining_blockers` - Blockers still alive that can receive damage
    /// * `remaining_damage` - How much non-lethal damage to assign
    ///
    /// # Returns
    /// The CardId of the blocker to assign the remaining damage to.
    /// Default implementation picks the first remaining blocker.
    fn choose_blocker_for_remaining_damage(
        &mut self,
        _view: &GameStateView,
        _attacker: CardId,
        remaining_blockers: &[CardId],
        _remaining_damage: i32,
    ) -> ChoiceResult<CardId> {
        // Default: assign to first remaining blocker
        if let Some(&blocker_id) = remaining_blockers.first() {
            ChoiceResult::Ok(blocker_id)
        } else {
            ChoiceResult::Error("No remaining blockers provided".to_string())
        }
    }

    /// Choose cards to discard to maximum hand size
    ///
    /// Called during cleanup step if hand size exceeds maximum.
    /// Returns ChoiceResult with the cards to discard, or a special request
    /// (UndoRequest, ExitGame, Error).
    fn choose_cards_to_discard(
        &mut self,
        view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>>;

    /// Choose a card from library (for tutoring/searching effects)
    ///
    /// Called when a SearchLibrary effect executes (e.g., Vibrant Cityscape,
    /// fetchlands, Demonic Tutor, Evolving Wilds).
    ///
    /// The controller receives a list of card NAMES from the library that match the
    /// search filter. The list may contain duplicates if multiple cards share the same
    /// name (e.g., `["Mountain", "Mountain", "Swamp"]`). The controller chooses an
    /// INDEX into this list, or returns None to decline to find ("fail to find").
    ///
    /// This name-based interface supports both LOCAL and NETWORK modes:
    /// - LOCAL: Game engine builds names from CardIds, maps returned index back to CardId
    /// - NETWORK: Server sends names directly, client returns index, server maps to CardId
    ///
    /// MTG Rules 701.19a: To search a zone, a player looks at all cards in that zone
    /// and may find a card that matches the given description.
    ///
    /// MTG Rules 701.19b: If a player is searching a hidden zone for cards with
    /// a stated quality, they don't have to find a card (they can "fail to find").
    ///
    /// Returns ChoiceResult with the chosen index (or None to fail to find),
    /// or a special request (UndoRequest, ExitGame, Error).
    ///
    /// The `valid_cards` parameter provides full CardDefinition data for each
    /// searchable card, enabling proper evaluation of card properties (types,
    /// mana cost, power/toughness, keywords, etc.) for AI decision-making.
    ///
    /// ## Java Forge Equivalent
    /// Matches `PlayerController.chooseCardsForEffect(..., "Search library")`
    fn choose_from_library(
        &mut self,
        view: &GameStateView,
        valid_cards: &[&CardDefinition],
    ) -> ChoiceResult<Option<usize>>;

    /// Choose a card from library using only card names (network fallback)
    ///
    /// Called when the client can't see library card identities (hidden zone in
    /// network mode), but the server has provided the names of valid cards.
    /// The default implementation picks the first card, matching ZeroController
    /// behavior. Controllers that want smarter name-based evaluation can override.
    ///
    /// Returns the same semantics as `choose_from_library`: `Some(index)` to
    /// select a card, or `None` to decline (fail to find).
    fn choose_from_library_by_names(
        &mut self,
        _view: &GameStateView,
        card_names: &[String],
    ) -> ChoiceResult<Option<usize>> {
        ChoiceResult::Ok(if card_names.is_empty() { None } else { Some(0) })
    }

    /// Take the server-authoritative library search result CardId (network mode only)
    ///
    /// After `choose_from_library` in network mode, the server sends back the actual
    /// CardId that was selected via ChoiceAccepted. The game loop calls this method
    /// to retrieve that CardId when `valid_cards` is empty (hidden library on client).
    /// Returns `None` by default (non-network controllers don't use this).
    fn take_library_search_result(&mut self) -> Option<CardId> {
        None
    }

    /// Set the pending library search CardIds (server-side network mode only)
    ///
    /// Called by the game loop before `choose_from_library` to provide the actual
    /// CardIds corresponding to the valid library cards. The NetworkController
    /// stores these and includes them in the ChoiceRequest so the coordinator
    /// can resolve the client's name index back to an authoritative CardId.
    /// No-op for non-network controllers.
    fn set_pending_library_search_card_ids(&mut self, _card_ids: &[CardId]) {}

    /// Choose permanents to sacrifice
    ///
    /// Called when a player must sacrifice a specific number of permanents,
    /// such as for Balance, Cataclysm, or other sacrifice effects.
    ///
    /// Unlike targeted sacrifice (where the controller of the spell chooses),
    /// this method is called on the controller of the permanents being sacrificed,
    /// allowing each player to choose which of their own permanents to sacrifice.
    ///
    /// MTG Rules 701.17a: To sacrifice a permanent, its controller moves it from
    /// the battlefield directly to its owner's graveyard.
    ///
    /// ## Parameters
    /// - `view`: Read-only view of the game state
    /// - `valid_permanents`: Permanents that can be sacrificed (filtered by type)
    /// - `count`: Exact number of permanents that must be sacrificed
    /// - `card_type_description`: Human-readable description (e.g., "creatures", "lands")
    ///
    /// Returns ChoiceResult with exactly `count` permanents to sacrifice,
    /// or a special request (UndoRequest, ExitGame, Error).
    ///
    /// ## Java Forge Equivalent
    /// Matches `PlayerController.choosePermanentsToSacrifice(sa, min, max, validTargets, message)`
    fn choose_permanents_to_sacrifice(
        &mut self,
        view: &GameStateView,
        valid_permanents: &[CardId],
        count: usize,
        card_type_description: &str,
    ) -> ChoiceResult<SmallVec<[CardId; 8]>>;

    /// Choose which permanents to NOT untap during untap step
    ///
    /// Called during the untap step for permanents with "You may choose not to
    /// untap CARDNAME during your untap step." (MayNotUntap keyword).
    ///
    /// The controller chooses which of the given permanents should stay tapped.
    /// Any permanents not returned will be untapped normally.
    ///
    /// MTG Rules 502.3: "After the active player has determined which permanents
    /// they control will untap, they untap them all simultaneously."
    ///
    /// ## Parameters
    /// - `view`: Read-only view of the game state
    /// - `may_not_untap_permanents`: Tapped permanents with MayNotUntap that could stay tapped
    ///
    /// Returns ChoiceResult with permanents that should STAY TAPPED,
    /// or a special request (UndoRequest, ExitGame, Error).
    ///
    /// ## Java Forge Equivalent
    /// Matches `PlayerController.choosePermanentsToUntap(list)`
    fn choose_permanents_to_not_untap(
        &mut self,
        view: &GameStateView,
        may_not_untap_permanents: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>>;

    /// Choose modes for a modal spell
    ///
    /// Called when casting a modal spell (e.g., "Choose one —", "Choose two —").
    /// The controller must select the required number of modes from the available options.
    ///
    /// MTG Rules 700.2: "A spell or ability is modal if it has two or more options
    /// in a bulleted list preceded by instructions for a player to choose a number
    /// of those options."
    ///
    /// MTG Rules 601.2b: "If the spell is modal, the player announces the mode choice."
    ///
    /// ## Parameters
    /// - `view`: Read-only view of the game state
    /// - `spell_id`: The modal spell being cast
    /// - `mode_descriptions`: Human-readable descriptions of each mode
    /// - `mode_count`: Number of modes to choose
    /// - `min_modes`: Minimum modes required (may be less than mode_count for optional)
    /// - `can_repeat`: Whether the same mode can be chosen multiple times
    ///
    /// Returns ChoiceResult with indices of chosen modes (0-based),
    /// or a special request (UndoRequest, ExitGame, Error).
    ///
    /// ## Java Forge Equivalent
    /// Matches `CharmEffect.makePossibleOptions()` and `CharmAi.chooseOptionsAi()`
    fn choose_modes(
        &mut self,
        view: &GameStateView,
        spell_id: CardId,
        mode_descriptions: &[String],
        mode_count: usize,
        min_modes: usize,
        can_repeat: bool,
    ) -> ChoiceResult<SmallVec<[usize; 4]>>;

    /// Notification that priority was passed
    ///
    /// Called when this controller passes priority, allowing it to track
    /// game flow or update internal state.
    fn on_priority_passed(&mut self, view: &GameStateView);

    /// Notification that the game has ended
    ///
    /// Called when the game is over, with a boolean indicating whether
    /// this player won.
    fn on_game_end(&mut self, view: &GameStateView, won: bool);

    /// Get serializable snapshot state for this controller
    ///
    /// Returns controller-specific state that should be preserved across snapshot/resume.
    /// Most controllers (Random, Heuristic, etc.) return None since they don't have
    /// state to preserve. FixedScriptController returns its current script position.
    ///
    /// This method is called by the snapshot system to capture controller state.
    fn get_snapshot_state(&self) -> Option<serde_json::Value> {
        None // Default implementation returns None
    }

    /// Check if controller has more choices available
    ///
    /// Used for `--stop-when-fixed-exhausted` flag. Returns true if the controller
    /// has more choices to make, false if exhausted (only relevant for FixedScriptController).
    ///
    /// Default implementation returns true (infinite choices for AI/human controllers).
    fn has_more_choices(&self) -> bool {
        true
    }

    /// Choose from a list of string options (for network games)
    ///
    /// This is a simplified choice interface for network games where the server
    /// provides pre-formatted string options. The controller returns an index
    /// into the options array.
    ///
    /// Default implementation reads from stdin for human players.
    /// AI controllers should override to make index-based decisions.
    fn choose_from_options(&mut self, options: &[String]) -> usize {
        use std::io::{self, Write};
        print!("Enter choice (0-{}): ", options.len().saturating_sub(1));
        let _ = io::stdout().flush();

        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_ok() {
            input.trim().parse().unwrap_or(0)
        } else {
            0
        }
    }

    /// Get the controller type for snapshot persistence
    ///
    /// Returns the controller type so snapshots can record which controller
    /// was active, even for stateless controllers like Heuristic and Zero.
    /// This is critical for snapshot/resume functionality - without this,
    /// stateless controllers would be incorrectly restored as Zero controllers.
    fn get_controller_type(&self) -> crate::game::snapshot::ControllerType;

    /// Prepare for a choice by blocking on network if needed
    ///
    /// For network controllers (NetworkLocalController, RemoteController on client side),
    /// this blocks until a ChoiceRequest/OpponentChoice is received from the server.
    /// This ensures that any CardRevealed messages that precede the choice have been
    /// buffered, so sync_to_action() can process them BEFORE abilities are computed.
    ///
    /// Returns true on success (preparation done or not needed).
    /// Returns false if the game should exit (GameEnded/Error received).
    ///
    /// After this returns true for a network controller, the caller should:
    /// 1. Call sync_to_action() to process buffered reveals
    /// 2. Compute available abilities
    /// 3. Call choose_spell_ability_to_play() with the abilities
    ///
    /// The default implementation returns true (no network preparation needed).
    /// NetworkLocalController and RemoteController override this to block on MVar.
    fn prepare_for_priority_choice(&mut self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    include!("controller_tests.rs");
}
