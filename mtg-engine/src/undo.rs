//! Undo log for efficient game tree search
//!
//! This module provides a transaction log of game actions that can be
//! rewound to efficiently explore the game tree without expensive deep copies.

use crate::core::{CardId, CounterType, Keyword, PlayerId};
use crate::zones::Zone;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use std::fmt;

use crate::game::GameState;

/// Target audience for a card reveal
///
/// Specifies WHO should see a card's identity when it's revealed.
/// Per NETWORK_ARCHITECTURE.md, reveals are first-class game actions logged
/// BEFORE any move that depends on the card's identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RevealTarget {
    /// Reveal to a single player only (e.g., drawing a card - only the owner sees it)
    Player(PlayerId),
    /// Reveal to all players (e.g., card entering battlefield - everyone sees it)
    All,
}

/// Atomic game actions that can be logged and undone
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GameAction {
    /// Move a card between zones
    MoveCard {
        card_id: CardId,
        from_zone: Zone,
        to_zone: Zone,
        owner: PlayerId,
    },

    /// Tap/untap a permanent
    TapCard { card_id: CardId, tapped: bool },

    /// Modify life total (delta is the change, not absolute value)
    ModifyLife { player_id: PlayerId, delta: i32 },

    /// Add mana to pool
    AddMana {
        player_id: PlayerId,
        mana: crate::core::ManaCost,
    },

    /// Empty mana pool (stores previous state for undo)
    EmptyManaPool {
        player_id: PlayerId,
        prev_white: u8,
        prev_blue: u8,
        prev_black: u8,
        prev_red: u8,
        prev_green: u8,
        prev_colorless: u8,
    },

    /// Add counter to card
    AddCounter {
        card_id: CardId,
        counter_type: CounterType,
        amount: u8,
    },

    /// Remove counter from card
    RemoveCounter {
        card_id: CardId,
        counter_type: CounterType,
        amount: u8,
    },

    /// Advance game step
    AdvanceStep {
        from_step: crate::game::Step,
        to_step: crate::game::Step,
    },

    /// Change turn (stores RNG state for proper rewind)
    ChangeTurn {
        from_player: PlayerId,
        to_player: PlayerId,
        turn_number: u32,
        /// RNG state at the START of this turn (for snapshot rewind)
        /// SmallVec<[u8; 64]> fits ChaCha12Rng bincode serialization (56 bytes, no heap allocation)
        /// Size 64 chosen as smallest power-of-2 supported by smallvec that fits 56 bytes
        /// INVARIANT: serialization code asserts exactly 56 bytes to catch future changes
        rng_state: Option<SmallVec<[u8; 64]>>,
    },

    /// Pump creature (temporary stat modification and/or keyword grant)
    PumpCreature {
        card_id: CardId,
        power_delta: i32,
        toughness_delta: i32,
        /// Keywords granted by this pump effect (for undo)
        keywords_granted: smallvec::SmallVec<[Keyword; 2]>,
    },

    /// Set turn_entered_battlefield field (for summoning sickness tracking)
    SetTurnEnteredBattlefield {
        card_id: CardId,
        /// Previous value (None if wasn't on battlefield)
        old_value: Option<u32>,
        /// New value (Some(turn) when entering battlefield, None when leaving)
        new_value: Option<u32>,
    },

    /// Set lands_played_this_turn counter (for land play limit tracking)
    SetLandsPlayedThisTurn {
        player_id: PlayerId,
        /// Previous count
        old_value: u8,
        /// New count
        new_value: u8,
    },

    /// Set attached_to field (for Equipment/Aura attachment tracking)
    SetAttachedTo {
        equipment_id: CardId,
        /// Previous attachment target (None if not attached)
        old_target: Option<CardId>,
        /// New attachment target (None when detaching, Some(card) when attaching)
        new_target: Option<CardId>,
    },

    /// Mark a choice point (for tree search and replay)
    ///
    /// Stores both the fact that a choice occurred and what that choice was,
    /// enabling deterministic replay from snapshots.
    ChoicePoint {
        player_id: PlayerId,
        choice_id: u32,
        /// The actual choice made (for replay). None if choice hasn't been recorded yet.
        choice: Option<crate::game::replay_controller::ReplayChoice>,
    },

    /// Reveal a card's identity (CardID ⟺ CardName binding)
    ///
    /// Part of the late-binding CardID architecture (mtg-qtqcr). This action binds
    /// a pre-allocated CardID to its actual card identity (name).
    ///
    /// ## Target Audience
    ///
    /// The `revealed_to` field specifies WHO should see this reveal:
    /// - `RevealTarget::Player(id)`: Only that player sees it (e.g., drawing a card)
    /// - `RevealTarget::All`: Everyone sees it (e.g., card entering battlefield)
    ///
    /// ## Viewer-Specific Content
    ///
    /// This action is logged by ALL players for EVERY reveal, but with different content:
    /// - Players in the target audience: `name = Some("Lightning Bolt")`
    /// - Players NOT in the audience: `name = None` (keeps action_count in sync)
    ///
    /// This keeps action_count synchronized across all clients while maintaining
    /// information asymmetry.
    ///
    /// ## Write-Once Semantics
    ///
    /// Reveals are monotonic: a CardID can only transition from unrevealed (None)
    /// to revealed (Some). The EntityStore enforces this with a panic if attempting
    /// to insert into an already-occupied slot. This prevents revealing CardID 33
    /// as "Lightning Bolt" then later revealing it as "Mountain".
    ///
    /// For game tree exploration, undo clears the slot back to None, allowing
    /// a subsequent re-reveal (which is fine since each timeline only sees
    /// a single None→Some transition).
    ///
    /// ## Forward Logic
    ///
    /// If `name` is Some, the Card should be instantiated and inserted into
    /// the EntityStore at `card_id` by the caller.
    /// If `name` is None, this is a "dummy" reveal that doesn't modify state
    /// (for opponents who don't learn the card identity).
    ///
    /// ## Undo Logic
    ///
    /// Restores the previous revealed_to_mask value. If old_mask is 0 and
    /// name is Some (card was created by this reveal), clears the card slot.
    RevealCard {
        /// The CardID being revealed
        card_id: CardId,
        /// The revealed card name, or None for late-binding (client doesn't know yet)
        name: Option<String>,
        /// Who should see this reveal
        revealed_to: RevealTarget,
        /// Previous mask value (for undo). If 0, this was the first reveal.
        old_mask: u8,
    },

    /// Set revealed_to_mask field (for tracking which players have seen a card)
    ///
    /// DEPRECATED: Use RevealCard with old_mask instead. This is kept for
    /// backwards compatibility with existing undo logs but should not be
    /// logged in new code.
    SetRevealedToMask {
        card_id: CardId,
        /// Previous mask value (for undo)
        old_value: u8,
        /// New mask value
        new_value: u8,
    },

    /// Shuffle a player's library
    ///
    /// Stores the previous order of CardIds so it can be restored on undo.
    /// This is essential for deterministic replay and game tree search when
    /// tutor effects (search library, then shuffle) are involved.
    ///
    /// ## Network Considerations
    ///
    /// In network mode, after shuffling the server sends a LibraryReordered
    /// message to clients with the new order. The previous_order stored here
    /// is the order BEFORE shuffling, which is only known to the server.
    ShuffleLibrary {
        /// Which player's library was shuffled
        player: PlayerId,
        /// Previous order of CardIds (for undo)
        /// Stored as Vec since library size varies and SmallVec wouldn't help
        previous_order: Vec<CardId>,
    },
}

impl fmt::Display for GameAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GameAction::MoveCard {
                card_id,
                from_zone,
                to_zone,
                owner,
            } => write!(
                f,
                "MoveCard({} {:?} -> {:?} owner=P{})",
                card_id.as_u32(),
                from_zone,
                to_zone,
                owner.as_u32()
            ),
            GameAction::TapCard { card_id, tapped } => {
                if *tapped {
                    write!(f, "Tap({})", card_id.as_u32())
                } else {
                    write!(f, "Untap({})", card_id.as_u32())
                }
            }
            GameAction::ModifyLife { player_id, delta } => {
                write!(f, "Life(P{} {:+})", player_id.as_u32(), delta)
            }
            GameAction::AddMana { player_id, mana } => {
                write!(f, "AddMana(P{} {})", player_id.as_u32(), mana)
            }
            GameAction::EmptyManaPool { player_id, .. } => {
                write!(f, "EmptyMana(P{})", player_id.as_u32())
            }
            GameAction::AddCounter {
                card_id,
                counter_type,
                amount,
            } => write!(f, "AddCounter({} {:?}x{})", card_id.as_u32(), counter_type, amount),
            GameAction::RemoveCounter {
                card_id,
                counter_type,
                amount,
            } => write!(f, "RemoveCounter({} {:?}x{})", card_id.as_u32(), counter_type, amount),
            GameAction::AdvanceStep { from_step, to_step } => {
                write!(f, "Step({:?} -> {:?})", from_step, to_step)
            }
            GameAction::ChangeTurn {
                from_player,
                to_player,
                turn_number,
                ..
            } => write!(
                f,
                "Turn({} P{} -> P{})",
                turn_number,
                from_player.as_u32(),
                to_player.as_u32()
            ),
            GameAction::PumpCreature {
                card_id,
                power_delta,
                toughness_delta,
                keywords_granted,
            } => {
                if keywords_granted.is_empty() {
                    write!(f, "Pump({} {:+}/{:+})", card_id.as_u32(), power_delta, toughness_delta)
                } else {
                    write!(
                        f,
                        "Pump({} {:+}/{:+} +{:?})",
                        card_id.as_u32(),
                        power_delta,
                        toughness_delta,
                        keywords_granted
                    )
                }
            }
            GameAction::SetTurnEnteredBattlefield { card_id, new_value, .. } => {
                write!(f, "SetETB({} turn={:?})", card_id.as_u32(), new_value)
            }
            GameAction::SetLandsPlayedThisTurn {
                player_id, new_value, ..
            } => write!(f, "LandsPlayed(P{} = {})", player_id.as_u32(), new_value),
            GameAction::SetAttachedTo {
                equipment_id,
                new_target,
                ..
            } => write!(f, "Attach({} -> {:?})", equipment_id.as_u32(), new_target),
            GameAction::ChoicePoint {
                player_id,
                choice_id,
                choice,
            } => write!(f, "Choice(P{} #{} = {:?})", player_id.as_u32(), choice_id, choice),
            GameAction::RevealCard {
                card_id,
                name,
                revealed_to,
                old_mask,
            } => {
                let target = match revealed_to {
                    RevealTarget::Player(pid) => format!("P{}", pid.as_u32()),
                    RevealTarget::All => "ALL".to_string(),
                };
                match name {
                    Some(n) => write!(
                        f,
                        "RevealCard({} = \"{}\" to {} mask:0x{:02x})",
                        card_id.as_u32(),
                        n,
                        target,
                        old_mask
                    ),
                    None => write!(
                        f,
                        "RevealCard({} = ??? to {} mask:0x{:02x})",
                        card_id.as_u32(),
                        target,
                        old_mask
                    ),
                }
            }
            GameAction::SetRevealedToMask {
                card_id,
                old_value,
                new_value,
            } => write!(
                f,
                "SetRevealedMask({} 0x{:02x} -> 0x{:02x})",
                card_id.as_u32(),
                old_value,
                new_value
            ),
            GameAction::ShuffleLibrary { player, previous_order } => {
                write!(f, "ShuffleLibrary(P{} {} cards)", player.as_u32(), previous_order.len())
            }
        }
    }
}

impl GameAction {
    /// Apply the inverse of this action to undo it
    ///
    /// Returns Ok(()) if successful, Err if the action cannot be undone
    ///
    /// # Errors
    ///
    /// Returns an error string if the action cannot be undone (e.g., card/player not found).
    pub fn undo(&self, game: &mut GameState) -> Result<(), String> {
        match self {
            GameAction::MoveCard {
                card_id,
                from_zone,
                to_zone,
                owner,
            } => {
                // Reverse the move: move from to_zone back to from_zone
                game.move_card(*card_id, *to_zone, *from_zone, *owner)
                    .map_err(|e| format!("Failed to undo MoveCard: {}", e))?;
            }

            GameAction::TapCard { card_id, tapped } => {
                // Reverse tap state
                if let Ok(card) = game.cards.get_mut(*card_id) {
                    card.tapped = !tapped;
                    // Increment mana version since tap state changed
                    game.increment_mana_version();
                } else {
                    return Err(format!("Card {} not found for TapCard undo", card_id.as_u32()));
                }
            }

            GameAction::ModifyLife { player_id, delta } => {
                // Reverse the life change
                if let Some(player) = game.players.iter_mut().find(|p| p.id == *player_id) {
                    player.life -= delta;
                } else {
                    return Err(format!("Player {} not found for ModifyLife undo", player_id.as_u32()));
                }
            }

            GameAction::AddMana { player_id, mana } => {
                // Remove the mana that was added
                if let Some(player) = game.players.iter_mut().find(|p| p.id == *player_id) {
                    player.mana_pool.white = player.mana_pool.white.saturating_sub(mana.white);
                    player.mana_pool.blue = player.mana_pool.blue.saturating_sub(mana.blue);
                    player.mana_pool.black = player.mana_pool.black.saturating_sub(mana.black);
                    player.mana_pool.red = player.mana_pool.red.saturating_sub(mana.red);
                    player.mana_pool.green = player.mana_pool.green.saturating_sub(mana.green);
                    player.mana_pool.colorless = player.mana_pool.colorless.saturating_sub(mana.colorless);
                } else {
                    return Err(format!("Player {} not found for AddMana undo", player_id.as_u32()));
                }
            }

            GameAction::EmptyManaPool {
                player_id,
                prev_white,
                prev_blue,
                prev_black,
                prev_red,
                prev_green,
                prev_colorless,
            } => {
                // Restore previous mana pool state
                if let Some(player) = game.players.iter_mut().find(|p| p.id == *player_id) {
                    player.mana_pool.white = *prev_white;
                    player.mana_pool.blue = *prev_blue;
                    player.mana_pool.black = *prev_black;
                    player.mana_pool.red = *prev_red;
                    player.mana_pool.green = *prev_green;
                    player.mana_pool.colorless = *prev_colorless;
                } else {
                    return Err(format!(
                        "Player {} not found for EmptyManaPool undo",
                        player_id.as_u32()
                    ));
                }
            }

            GameAction::AddCounter {
                card_id,
                counter_type,
                amount,
            } => {
                // Remove the counters that were added
                game.remove_counters(*card_id, *counter_type, *amount)
                    .map_err(|e| format!("Failed to undo AddCounter: {}", e))?;
            }

            GameAction::RemoveCounter {
                card_id,
                counter_type,
                amount,
            } => {
                // Add back the counters that were removed
                game.add_counters(*card_id, *counter_type, *amount)
                    .map_err(|e| format!("Failed to undo RemoveCounter: {}", e))?;
            }

            GameAction::AdvanceStep { from_step, to_step: _ } => {
                // Restore previous step
                game.turn.current_step = *from_step;
            }

            GameAction::ChangeTurn {
                from_player,
                to_player: _,
                turn_number,
                rng_state,
            } => {
                // Restore previous turn state
                game.turn.active_player = *from_player;
                // Find the player index
                if let Some(idx) = game.players.iter().position(|p| p.id == *from_player) {
                    game.turn.active_player_idx = idx;
                }

                // Restore turn number to the previous turn
                // ChangeTurn logs the NEW turn number, so previous is turn_number - 1
                game.turn.turn_number = turn_number.saturating_sub(1);

                // Restore RNG state if available (using bincode + SmallVec)
                if let Some(rng_bytes) = rng_state {
                    // SmallVec derefs to &[u8], which is what bincode::deserialize expects
                    if let Ok(rng) = bincode::deserialize::<rand_chacha::ChaCha12Rng>(rng_bytes) {
                        *game.rng.borrow_mut() = rng;
                    } else {
                        return Err("Failed to deserialize RNG state".to_string());
                    }
                }
            }

            GameAction::PumpCreature {
                card_id,
                power_delta,
                toughness_delta,
                keywords_granted,
            } => {
                // Reverse the pump by applying negative deltas
                if let Ok(card) = game.cards.get_mut(*card_id) {
                    // Reverse the power/toughness bonus
                    card.power_bonus -= power_delta;
                    card.toughness_bonus -= toughness_delta;
                    // Remove granted keywords
                    for keyword in keywords_granted {
                        card.keywords.remove(*keyword);
                    }
                } else {
                    return Err(format!("Card {} not found for PumpCreature undo", card_id.as_u32()));
                }
            }

            GameAction::SetTurnEnteredBattlefield {
                card_id,
                old_value,
                new_value: _,
            } => {
                // Restore the previous turn_entered_battlefield value
                if let Ok(card) = game.cards.get_mut(*card_id) {
                    card.turn_entered_battlefield = *old_value;
                } else {
                    return Err(format!(
                        "Card {} not found for SetTurnEnteredBattlefield undo",
                        card_id.as_u32()
                    ));
                }
            }

            GameAction::SetLandsPlayedThisTurn {
                player_id,
                old_value,
                new_value: _,
            } => {
                // Restore the previous lands_played_this_turn count
                if let Some(player) = game.players.iter_mut().find(|p| p.id == *player_id) {
                    player.lands_played_this_turn = *old_value;
                } else {
                    return Err(format!(
                        "Player {} not found for SetLandsPlayedThisTurn undo",
                        player_id.as_u32()
                    ));
                }
            }

            GameAction::SetAttachedTo {
                equipment_id,
                old_target,
                new_target: _,
            } => {
                // Restore the previous attached_to value
                if let Ok(equipment) = game.cards.get_mut(*equipment_id) {
                    equipment.attached_to = *old_target;
                } else {
                    return Err(format!(
                        "Equipment {} not found for SetAttachedTo undo",
                        equipment_id.as_u32()
                    ));
                }
            }

            GameAction::ChoicePoint { .. } => {
                // ChoicePoints don't modify game state, nothing to undo
            }

            GameAction::RevealCard {
                card_id,
                name,
                old_mask,
                ..
            } => {
                // Undo reveal: restore the previous mask state
                // Two cases:
                // 1. Card exists (server or client after instantiation):
                //    Restore the old_mask value
                // 2. Card was created by this reveal (late-binding, old_mask=0, name=Some):
                //    Clear the slot entirely
                if let Ok(card) = game.cards.get_mut(*card_id) {
                    // Card exists - restore the mask
                    card.revealed_to_mask = *old_mask;
                } else if *old_mask == 0 && name.is_some() {
                    // Card doesn't exist but was created by this reveal
                    // This shouldn't normally happen since the card should exist
                    // if it was instantiated, but handle it defensively
                    game.cards.clear(*card_id);
                }
                // If card doesn't exist and old_mask != 0, this is a late-binding
                // reveal that never instantiated (opponent's hidden card) - nothing to undo
            }

            GameAction::SetRevealedToMask {
                card_id,
                old_value,
                new_value: _,
            } => {
                // Restore the previous revealed_to_mask value
                if let Ok(card) = game.cards.get_mut(*card_id) {
                    card.revealed_to_mask = *old_value;
                } else {
                    return Err(format!(
                        "Card {} not found for SetRevealedToMask undo",
                        card_id.as_u32()
                    ));
                }
            }

            GameAction::ShuffleLibrary { player, previous_order } => {
                // Restore the library to its previous order
                if let Some(zones) = game
                    .player_zones
                    .iter_mut()
                    .find(|(id, _)| *id == *player)
                    .map(|(_, z)| z)
                {
                    zones.library.cards = previous_order.clone();
                } else {
                    return Err(format!(
                        "Player {} zones not found for ShuffleLibrary undo",
                        player.as_u32()
                    ));
                }
            }
        }

        Ok(())
    }
}

/// Undo log for tracking and rewinding game actions
///
/// This allows efficient tree search by mutating game state forward
/// and then rewinding via the log, instead of expensive deep copies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoLog {
    /// Stack of actions (most recent at end)
    actions: Vec<GameAction>,

    /// Is logging enabled? (can be compiled out for replay benchmarks)
    enabled: bool,

    /// Mark positions for choice points
    choice_points: Vec<usize>,

    /// Log buffer sizes BEFORE each action (for synchronizing log truncation on undo)
    log_sizes: Vec<usize>,
}

impl UndoLog {
    pub fn new() -> Self {
        // Pre-allocate capacity based on typical game length
        // Empirically measured: ~50 actions per turn × 20 turns = ~1000 actions
        // This avoids Vec growth allocations during gameplay
        const ESTIMATED_ACTIONS_PER_TURN: usize = 50;
        const TYPICAL_GAME_LENGTH: usize = 20;
        let estimated_capacity = ESTIMATED_ACTIONS_PER_TURN * TYPICAL_GAME_LENGTH;

        UndoLog {
            actions: Vec::with_capacity(estimated_capacity),
            enabled: true,
            choice_points: Vec::new(), // Small, can grow naturally
            log_sizes: Vec::with_capacity(estimated_capacity),
        }
    }

    /// Create a disabled undo log (for benchmarking)
    pub fn disabled() -> Self {
        UndoLog {
            actions: Vec::new(),
            enabled: false,
            choice_points: Vec::new(),
            log_sizes: Vec::new(),
        }
    }

    /// Log an action along with the log buffer size BEFORE this action
    ///
    /// The prior_log_size allows us to truncate the log buffer to the correct
    /// size when undoing this action, removing all log entries generated by it.
    pub fn log(&mut self, action: GameAction, prior_log_size: usize) {
        if self.enabled {
            self.actions.push(action);
            self.log_sizes.push(prior_log_size);
        }
    }

    /// Mark a choice point in the log
    pub fn mark_choice_point(&mut self) {
        if self.enabled {
            self.choice_points.push(self.actions.len());
        }
    }

    /// Get the most recent action without removing it
    pub fn peek(&self) -> Option<&GameAction> {
        self.actions.last()
    }

    /// Pop and return the most recent action along with its prior log size
    ///
    /// Returns (action, prior_log_size) tuple. The prior_log_size can be used
    /// to truncate the game log to remove entries generated by this action.
    pub fn pop(&mut self) -> Option<(GameAction, usize)> {
        if let Some(action) = self.actions.pop() {
            let log_size = self.log_sizes.pop().unwrap_or(0);
            Some((action, log_size))
        } else {
            None
        }
    }

    /// Get number of actions in log
    pub fn len(&self) -> usize {
        self.actions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    /// Clear all actions up to the most recent choice point
    pub fn rewind_to_choice_point(&mut self) {
        if let Some(checkpoint) = self.choice_points.pop() {
            self.actions.truncate(checkpoint);
            self.log_sizes.truncate(checkpoint);
        }
    }

    /// Rewind to the most recent ChangeTurn action, extracting all ChoicePoint actions
    /// encountered along the way (in forward chronological order).
    ///
    /// This method actually UNDOES the game state by applying the inverse of each action.
    ///
    /// Returns (turn_number, intra_turn_choices, actions_rewound, log_size_at_turn_boundary) where:
    /// - turn_number: The turn number from the most recent ChangeTurn action
    /// - intra_turn_choices: All ChoicePoint actions that occurred after that turn change
    /// - actions_rewound: Total number of actions popped from the log
    /// - log_size_at_turn_boundary: The log buffer size at the turn boundary (for truncation)
    ///
    /// Returns None if undo log is disabled.
    ///
    /// Note: Wildcard is intentional for the inner match - we want to undo ALL GameAction
    /// variants except ChangeTurn (stop point) and ChoicePoint (non-mutating).
    #[allow(clippy::wildcard_enum_match_arm)]
    pub fn rewind_to_turn_start(&mut self, game: &mut GameState) -> Option<(u32, Vec<GameAction>, usize, usize)> {
        if !self.enabled {
            return None;
        }

        let mut choices_reversed = Vec::new();
        let mut turn_number = None;
        let mut actions_rewound = 0;
        let mut log_size_at_turn_boundary = 0;

        // Pop actions in reverse until we find ChangeTurn
        while let Some((action, log_size)) = self.pop() {
            actions_rewound += 1;
            match action {
                GameAction::ChangeTurn { turn_number: tn, .. } => {
                    // DON'T undo the ChangeTurn action - we want the snapshot to represent
                    // the START of this turn, not the END of the previous turn.
                    // Put it back on the log so the game state stays at the turn boundary.
                    self.actions.push(action);
                    self.log_sizes.push(log_size);
                    actions_rewound -= 1; // Don't count this as rewound since we kept it
                    turn_number = Some(tn);
                    log_size_at_turn_boundary = log_size;
                    break;
                }
                GameAction::ChoicePoint { .. } => {
                    // Collect choice points in reverse (don't need to undo, they're non-mutating)
                    choices_reversed.push(action);
                }
                _ => {
                    // Undo all other actions to restore game state
                    if let Err(e) = action.undo(game) {
                        eprintln!("WARNING: Failed to undo action {:?}: {}", action, e);
                    }
                }
            }
        }

        // If we found a ChangeTurn, use that turn number.
        // Otherwise (turn 1), use turn 1 as the turn number.
        // The game state has been rewound either way.
        let effective_turn = turn_number.unwrap_or(1);

        // Reverse the choices to get forward chronological order
        choices_reversed.reverse();
        Some((
            effective_turn,
            choices_reversed,
            actions_rewound,
            log_size_at_turn_boundary,
        ))
    }

    /// Get the most recent turn number from the log, if any ChangeTurn exists
    pub fn current_turn(&self) -> Option<u32> {
        self.actions.iter().rev().find_map(|action| {
            if let GameAction::ChangeTurn { turn_number, .. } = action {
                Some(*turn_number)
            } else {
                None
            }
        })
    }

    /// Clear the entire log
    pub fn clear(&mut self) {
        self.actions.clear();
        self.choice_points.clear();
        self.log_sizes.clear();
    }

    /// Get all actions (for debugging/serialization)
    pub fn actions(&self) -> &[GameAction] {
        &self.actions
    }

    /// Format the last N actions as a multi-line string for debugging
    ///
    /// Returns a string with one action per line, most recent last.
    /// Each line is prefixed with its index in the full action log.
    pub fn format_last_n(&self, n: usize) -> String {
        let len = self.actions.len();
        let start = len.saturating_sub(n);
        let mut result = String::new();
        for (i, action) in self.actions[start..].iter().enumerate() {
            use std::fmt::Write;
            let _ = writeln!(result, "  [{:4}] {}", start + i, action);
        }
        result
    }
}

impl Default for UndoLog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_undo_log() {
        let mut log = UndoLog::new();
        assert_eq!(log.len(), 0);

        let action = GameAction::ModifyLife {
            player_id: PlayerId::new(1),
            delta: -3,
        };

        log.log(action, 0);
        assert_eq!(log.len(), 1);

        let (popped, log_size) = log.pop().unwrap();
        assert!(matches!(popped, GameAction::ModifyLife { .. }));
        assert_eq!(log_size, 0);
        assert_eq!(log.len(), 0);
    }

    #[test]
    fn test_choice_points() {
        let mut log = UndoLog::new();

        log.log(
            GameAction::ModifyLife {
                player_id: PlayerId::new(1),
                delta: -1,
            },
            0,
        );
        log.log(
            GameAction::ModifyLife {
                player_id: PlayerId::new(1),
                delta: -1,
            },
            0,
        );

        log.mark_choice_point();

        log.log(
            GameAction::ModifyLife {
                player_id: PlayerId::new(1),
                delta: -1,
            },
            0,
        );
        log.log(
            GameAction::ModifyLife {
                player_id: PlayerId::new(1),
                delta: -1,
            },
            0,
        );

        assert_eq!(log.len(), 4);

        log.rewind_to_choice_point();
        assert_eq!(log.len(), 2);
    }

    #[test]
    fn test_disabled_log() {
        let mut log = UndoLog::disabled();

        log.log(
            GameAction::ModifyLife {
                player_id: PlayerId::new(1),
                delta: -1,
            },
            0,
        );

        assert_eq!(log.len(), 0); // Nothing logged when disabled
    }

    #[test]
    fn test_rewind_to_turn_start() {
        let mut log = UndoLog::new();
        let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);

        // Simulate turn 1 starting
        log.log(
            GameAction::ChangeTurn {
                from_player: PlayerId::new(0),
                to_player: PlayerId::new(1),
                turn_number: 1,
                rng_state: None,
            },
            0,
        );

        // Some actions during turn 1
        log.log(
            GameAction::ModifyLife {
                player_id: PlayerId::new(1),
                delta: -1,
            },
            0,
        );

        log.log(
            GameAction::ChoicePoint {
                player_id: PlayerId::new(1),
                choice_id: 1,
                choice: None,
            },
            0,
        );

        log.log(
            GameAction::TapCard {
                card_id: CardId::new(1),
                tapped: true,
            },
            0,
        );

        log.log(
            GameAction::ChoicePoint {
                player_id: PlayerId::new(1),
                choice_id: 2,
                choice: None,
            },
            0,
        );

        assert_eq!(log.len(), 5);

        // Rewind to turn start (now requires GameState)
        let result = log.rewind_to_turn_start(&mut game);
        assert!(result.is_some());

        let (turn_number, choices, actions_rewound, _log_size) = result.unwrap();
        assert_eq!(turn_number, 1);
        assert_eq!(choices.len(), 2);
        assert_eq!(actions_rewound, 4); // All 4 actions after ChangeTurn (ChangeTurn is kept)

        // Verify choices are in forward chronological order
        assert!(matches!(
            choices[0],
            GameAction::ChoicePoint {
                player_id: _,
                choice_id: 1,
                choice: None
            }
        ));
        assert!(matches!(
            choices[1],
            GameAction::ChoicePoint {
                player_id: _,
                choice_id: 2,
                choice: None
            }
        ));

        // Log should have the ChangeTurn action still (we stopped AT the turn boundary)
        assert_eq!(log.len(), 1);
        assert!(matches!(log.peek().unwrap(), GameAction::ChangeTurn { .. }));
    }

    #[test]
    fn test_rewind_to_turn_start_no_turn() {
        let mut log = UndoLog::new();
        let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);

        // Add some actions but no ChangeTurn (simulates turn 1)
        log.log(
            GameAction::ModifyLife {
                player_id: PlayerId::new(1),
                delta: -1,
            },
            0,
        );

        log.log(
            GameAction::ChoicePoint {
                player_id: PlayerId::new(1),
                choice_id: 1,
                choice: None,
            },
            0,
        );

        // When no ChangeTurn is found, rewind should still succeed with turn 1
        // This is important for turn 1 where no ChangeTurn has been logged yet
        let result = log.rewind_to_turn_start(&mut game);
        assert!(result.is_some(), "rewind_to_turn_start should return Some for turn 1");

        let (turn_number, choice_actions, actions_rewound, _log_size) = result.unwrap();
        assert_eq!(turn_number, 1, "Turn number should be 1 when no ChangeTurn found");
        assert_eq!(choice_actions.len(), 1, "Should have 1 ChoicePoint action");
        assert_eq!(actions_rewound, 2, "Should have rewound 2 actions");

        // Undo log should be empty after rewinding everything
        assert!(log.is_empty(), "Undo log should be empty after full rewind");
    }

    #[test]
    fn test_current_turn() {
        let mut log = UndoLog::new();

        assert_eq!(log.current_turn(), None);

        log.log(
            GameAction::ChangeTurn {
                from_player: PlayerId::new(0),
                to_player: PlayerId::new(1),
                turn_number: 1,
                rng_state: None,
            },
            0,
        );

        assert_eq!(log.current_turn(), Some(1));

        log.log(
            GameAction::ModifyLife {
                player_id: PlayerId::new(1),
                delta: -1,
            },
            0,
        );

        log.log(
            GameAction::ChangeTurn {
                from_player: PlayerId::new(1),
                to_player: PlayerId::new(0),
                turn_number: 2,
                rng_state: None,
            },
            0,
        );

        // Should return the most recent turn
        assert_eq!(log.current_turn(), Some(2));
    }

    // =========================================================================
    // RevealCard tests (Phase 2, mtg-qtqcr)
    // =========================================================================

    #[test]
    fn test_reveal_card_display_with_name() {
        let action = GameAction::RevealCard {
            card_id: CardId::new(5),
            name: Some("Lightning Bolt".to_string()),
            revealed_to: RevealTarget::All,
            old_mask: 0,
        };

        let display = format!("{}", action);
        assert_eq!(display, "RevealCard(5 = \"Lightning Bolt\" to ALL mask:0x00)");
    }

    #[test]
    fn test_reveal_card_display_to_single_player() {
        let action = GameAction::RevealCard {
            card_id: CardId::new(5),
            name: Some("Lightning Bolt".to_string()),
            revealed_to: RevealTarget::Player(PlayerId::new(1)),
            old_mask: 0,
        };

        let display = format!("{}", action);
        assert_eq!(display, "RevealCard(5 = \"Lightning Bolt\" to P1 mask:0x00)");
    }

    #[test]
    fn test_reveal_card_display_without_name() {
        // Opponent perspective - doesn't know the card name
        let action = GameAction::RevealCard {
            card_id: CardId::new(42),
            name: None,
            revealed_to: RevealTarget::Player(PlayerId::new(0)),
            old_mask: 0,
        };

        let display = format!("{}", action);
        assert_eq!(display, "RevealCard(42 = ??? to P0 mask:0x00)");
    }

    #[test]
    fn test_reveal_card_undo_with_name() {
        use crate::core::Card;

        let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);

        // Reserve a slot for the card (as would be done at game start)
        game.cards.reserve(CardId::new(100));
        assert!(!game.cards.is_revealed(CardId::new(100)));

        // Create a test card and insert (simulating forward execution)
        let mut card = Card::new(CardId::new(100), "Test Card", PlayerId::new(0));
        // Mark as revealed to all (simulating forward execution of reveal)
        card.mark_revealed_to_all();
        game.cards.insert(CardId::new(100), card);
        assert!(game.cards.is_revealed(CardId::new(100)));
        assert!(game.cards.get(CardId::new(100)).unwrap().is_revealed_to_all());

        // Create the RevealCard action with old_mask=0 (was unrevealed before)
        let action = GameAction::RevealCard {
            card_id: CardId::new(100),
            name: Some("Test Card".to_string()),
            revealed_to: RevealTarget::All,
            old_mask: 0,
        };

        // Undo the reveal
        action.undo(&mut game).unwrap();

        // Card should still exist but mask restored to 0
        assert!(game.cards.is_revealed(CardId::new(100))); // card still exists
        assert_eq!(game.cards.get(CardId::new(100)).unwrap().revealed_to_mask, 0);
    }

    #[test]
    fn test_reveal_card_undo_dummy_reveal() {
        // Dummy reveal (opponent perspective) - name is None
        let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);

        // Reserve a slot (slot stays empty for opponent)
        game.cards.reserve(CardId::new(100));
        assert!(!game.cards.is_revealed(CardId::new(100)));

        // Create dummy RevealCard (opponent doesn't learn the card)
        // revealed_to is Player(0), but since we're the opponent (Player 1), name is None
        let action = GameAction::RevealCard {
            card_id: CardId::new(100),
            name: None,
            revealed_to: RevealTarget::Player(PlayerId::new(0)),
            old_mask: 0,
        };

        // Undo should succeed without error (no-op)
        action.undo(&mut game).unwrap();

        // Slot should still be unrevealed
        assert!(!game.cards.is_revealed(CardId::new(100)));
    }

    #[test]
    fn test_reveal_card_round_trip_via_undo_log() {
        use crate::core::Card;

        let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let mut log = UndoLog::new();

        // Reserve slot
        game.cards.reserve(CardId::new(50));

        // Create and insert card (forward execution)
        let mut card = Card::new(CardId::new(50), "Mountain", PlayerId::new(0));
        // Mark as revealed to all (simulating forward execution of reveal)
        card.mark_revealed_to_all();
        game.cards.insert(CardId::new(50), card);

        // Log the reveal action
        log.log(
            GameAction::RevealCard {
                card_id: CardId::new(50),
                name: Some("Mountain".to_string()),
                revealed_to: RevealTarget::All,
                old_mask: 0,
            },
            0,
        );

        // Verify card is revealed
        assert!(game.cards.is_revealed(CardId::new(50)));
        assert!(game.cards.get(CardId::new(50)).unwrap().is_revealed_to_all());
        assert_eq!(log.len(), 1);

        // Pop and undo
        let (action, _) = log.pop().unwrap();
        action.undo(&mut game).unwrap();

        // Card still exists but mask is restored to 0
        assert!(game.cards.is_revealed(CardId::new(50))); // card still exists
        assert_eq!(game.cards.get(CardId::new(50)).unwrap().revealed_to_mask, 0);
        assert!(log.is_empty());
    }
}
