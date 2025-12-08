//! Network protocol message types
//!
//! Defines all messages exchanged between client and server.

use crate::core::{CardId, ManaCost, PlayerId};
use crate::game::GameEndReason;
use serde::{Deserialize, Serialize};

// ═══════════════════════════════════════════════════════════════════════════
// CLIENT → SERVER MESSAGES
// ═══════════════════════════════════════════════════════════════════════════

/// Messages sent from client to server
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Initial authentication and deck submission
    Authenticate {
        /// Server password
        password: String,
        /// Player's display name
        player_name: String,
        /// Deck to use for the game
        deck: DeckSubmission,
    },

    /// Response to a choice request from server
    SubmitChoice {
        /// Sequence number matching the ChoiceRequest
        choice_seq: u32,
        /// The chosen option index (into the options array)
        choice_index: usize,
        /// Action count at time of choice (undo log position)
        /// Used for synchronization validation - server verifies this matches expected
        #[serde(default)]
        action_count: u64,
    },

    /// Request to disconnect gracefully
    Disconnect,

    /// Keepalive ping
    Ping {
        /// Timestamp in milliseconds
        timestamp_ms: u64,
    },
}

/// Deck submission format for authentication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeckSubmission {
    /// Main deck entries: (card_name, count)
    pub main_deck: Vec<(String, u8)>,
    /// Sideboard entries: (card_name, count)
    pub sideboard: Vec<(String, u8)>,
}

impl DeckSubmission {
    /// Create a new deck submission
    pub fn new(main_deck: Vec<(String, u8)>, sideboard: Vec<(String, u8)>) -> Self {
        Self { main_deck, sideboard }
    }

    /// Total cards in main deck
    pub fn main_deck_size(&self) -> usize {
        self.main_deck.iter().map(|(_, count)| *count as usize).sum()
    }

    /// Total cards in sideboard
    pub fn sideboard_size(&self) -> usize {
        self.sideboard.iter().map(|(_, count)| *count as usize).sum()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SERVER → CLIENT MESSAGES
// ═══════════════════════════════════════════════════════════════════════════

/// Messages sent from server to client
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Authentication result
    AuthResult {
        /// Whether authentication succeeded
        success: bool,
        /// Error message if failed
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        /// Assigned player ID if successful
        #[serde(skip_serializing_if = "Option::is_none")]
        your_player_id: Option<PlayerId>,
    },

    /// Waiting for opponent to connect
    WaitingForOpponent,

    /// Game is starting - includes initial setup info
    GameStarted {
        /// Your assigned player ID
        your_player_id: PlayerId,
        /// Opponent's display name
        opponent_name: String,
        /// Your opening hand (card IDs and definitions)
        opening_hand: Vec<CardReveal>,
        /// Number of cards in opponent's hand
        opponent_hand_count: usize,
        /// Your library size after drawing
        library_size: usize,
        /// Opponent library size after drawing (always visible per MTG rules)
        opponent_library_size: usize,
        /// Opponent's initial deck list (if deck_visibility enabled)
        /// This is the INITIAL list before sideboarding.
        #[serde(skip_serializing_if = "Option::is_none")]
        opponent_decklist: Option<DeckListInfo>,
        /// Starting life total
        starting_life: i32,
        /// Initial game state hash for verification
        initial_state_hash: u64,
    },

    /// Card reveal event (draws, tutors, plays, etc.)
    CardRevealed {
        /// Who the card belongs to
        owner: PlayerId,
        /// The revealed card info
        card: CardReveal,
        /// Why this card is being revealed
        reason: RevealReason,
    },

    /// Request a choice from this client
    ChoiceRequest {
        /// Sequence number for response correlation
        choice_seq: u32,
        /// Type of choice being requested
        choice_type: ChoiceType,
        /// Human-readable options (for verification against client's local computation)
        options: Vec<String>,
        /// Game state hash at this decision point (excludes hidden info)
        state_hash: u64,
        /// Action count at this decision point (undo log position)
        /// Client should verify this matches their local action count
        action_count: u64,
        /// Optional context for the choice
        #[serde(skip_serializing_if = "Option::is_none")]
        context: Option<ChoiceContext>,
    },

    /// Notify client of opponent's choice (for sync)
    OpponentChoice {
        /// Choice sequence number
        choice_seq: u32,
        /// What type of choice was made
        choice_type: ChoiceType,
        /// The choice index selected
        choice_index: usize,
        /// Human-readable description of what was chosen
        description: String,
        /// Action count when this choice was made
        /// Client uses this to verify they're at the same position
        action_count: u64,
    },

    /// Acknowledge receipt of a submitted choice
    ///
    /// Sent by server after receiving a valid SubmitChoice, allowing the client's
    /// NetworkLocalController to unblock and continue processing.
    ChoiceAccepted {
        /// Echo of the choice sequence for correlation
        choice_seq: u32,
        /// Server's action count after processing the choice
        /// Client can verify this matches their expected count
        #[serde(default)]
        action_count: u64,
    },

    /// Game has ended
    GameEnded {
        /// Winner (None for draw)
        #[serde(skip_serializing_if = "Option::is_none")]
        winner: Option<PlayerId>,
        /// Why the game ended
        reason: GameEndReason,
        /// Final state hash for verification
        final_state_hash: u64,
        /// Final action count (undo log length) - used for sync verification
        /// In debug mode, clients should compare this against their own action_count
        action_count: u64,
    },

    /// Error message
    Error {
        /// Error description
        message: String,
        /// Whether this is a fatal error (connection will close)
        fatal: bool,
    },

    /// Keepalive pong response
    Pong {
        /// Echoed timestamp from ping
        timestamp_ms: u64,
    },

    /// Debug state dump (only sent when hash verification fails)
    ///
    /// This is NOT part of normal game flow - only for diagnostics.
    #[cfg(debug_assertions)]
    DebugStateDump {
        /// Full game state as JSON for diffing
        state_json: String,
        /// What triggered the dump
        reason: String,
        /// Expected hash
        expected_hash: u64,
        /// Client's reported hash (if applicable)
        #[serde(skip_serializing_if = "Option::is_none")]
        client_hash: Option<u64>,
    },
}

// ═══════════════════════════════════════════════════════════════════════════
// SUPPORTING TYPES
// ═══════════════════════════════════════════════════════════════════════════

/// Information about a revealed card
///
/// Contains all public information needed to instantiate a card
/// in the client's shadow game state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardReveal {
    /// The card's entity ID (must match server's ID for sync)
    pub card_id: CardId,
    /// Card name
    pub name: String,
    /// Mana cost string (e.g., "{2}{W}{W}")
    pub mana_cost: String,
    /// Type line (e.g., "Creature - Human Soldier")
    pub type_line: String,
    /// Oracle text / rules text
    pub text: String,
    /// Power/toughness for creatures (None for non-creatures)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pt: Option<(i32, i32)>,
}

/// Reason a card was revealed to a player
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevealReason {
    /// Card drawn from library
    Draw,
    /// Card revealed for targeting
    Targeting,
    /// Card played or cast (moved to public zone)
    Played,
    /// Card searched from library (tutor effect)
    Searched,
    /// Card revealed by a game effect
    Effect,
    /// Part of opening hand
    OpeningHand,
    /// Token created
    TokenCreated,
}

/// Type of choice being requested from the player
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChoiceType {
    /// Choose spell/ability to play (or pass priority)
    Priority {
        /// Number of available actions (excluding pass)
        available_count: usize,
    },
    /// Choose targets for spell/ability
    Targets {
        /// The spell/ability being targeted for
        spell_id: CardId,
        /// Number of targets to choose
        target_count: usize,
    },
    /// Choose mana sources to tap for payment
    ManaSources {
        /// The cost being paid
        cost: ManaCost,
    },
    /// Choose which creatures to attack with
    Attackers {
        /// Number of creatures that can attack
        available_count: usize,
    },
    /// Choose blockers and what they block
    Blockers {
        /// Number of attacking creatures
        attacker_count: usize,
        /// Number of potential blockers
        blocker_count: usize,
    },
    /// Choose damage assignment order for multiple blockers
    DamageOrder {
        /// The attacking creature
        attacker: CardId,
        /// Number of blockers to order
        blocker_count: usize,
    },
    /// Choose cards to discard (to hand size or effect)
    Discard {
        /// Number of cards to discard
        count: usize,
    },
    /// Choose a card from library (tutor/search effect)
    LibrarySearch {
        /// Number of valid cards that can be chosen
        valid_count: usize,
    },
    /// Choose permanents to sacrifice (Balance, Cataclysm, etc.)
    Sacrifice {
        /// Number of valid permanents that can be sacrificed
        valid_count: usize,
        /// Number of permanents to sacrifice
        count: usize,
        /// Description of the permanent type (e.g., "creatures", "lands")
        card_type_description: String,
    },
}

/// Additional context for a choice request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChoiceContext {
    /// Spell/ability that triggered this choice (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spell: Option<CardReveal>,
    /// Human-readable description of the choice
    pub description: String,
}

/// Opponent's initial deck list (tournament-style visibility)
///
/// Contains the INITIAL deck list before any sideboarding. After game 1,
/// you know what cards they COULD have, but not which sideboard cards
/// they actually swapped in.
///
/// Note: Deck sizes vary by format (60+ for Standard/Modern, 100 for Commander)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeckListInfo {
    /// Main deck card names and counts
    pub main_deck: Vec<(String, u8)>,
    /// Sideboard card names and counts (empty for Commander)
    pub sideboard: Vec<(String, u8)>,
    /// Total main deck size
    pub main_deck_size: usize,
    /// Total sideboard size
    pub sideboard_size: usize,
}

impl DeckListInfo {
    /// Create from a deck submission
    pub fn from_submission(deck: &DeckSubmission) -> Self {
        Self {
            main_deck: deck.main_deck.clone(),
            sideboard: deck.sideboard.clone(),
            main_deck_size: deck.main_deck_size(),
            sideboard_size: deck.sideboard_size(),
        }
    }

    /// Convert to a DeckList for game initialization
    pub fn to_deck_list(&self) -> crate::loader::DeckList {
        use crate::loader::DeckEntry;

        crate::loader::DeckList {
            main_deck: self
                .main_deck
                .iter()
                .map(|(name, count)| DeckEntry {
                    card_name: name.clone(),
                    count: *count,
                })
                .collect(),
            sideboard: self
                .sideboard
                .iter()
                .map(|(name, count)| DeckEntry {
                    card_name: name.clone(),
                    count: *count,
                })
                .collect(),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_message_serialization() {
        let msg = ClientMessage::Authenticate {
            password: "secret".to_string(),
            player_name: "Alice".to_string(),
            deck: DeckSubmission::new(
                vec![("Lightning Bolt".to_string(), 4), ("Mountain".to_string(), 20)],
                vec![("Pyroclasm".to_string(), 2)],
            ),
        };

        let json = serde_json::to_string(&msg).expect("serialize");
        let roundtrip: ClientMessage = serde_json::from_str(&json).expect("deserialize");

        match roundtrip {
            ClientMessage::Authenticate {
                password,
                player_name,
                deck,
            } => {
                assert_eq!(password, "secret");
                assert_eq!(player_name, "Alice");
                assert_eq!(deck.main_deck_size(), 24);
                assert_eq!(deck.sideboard_size(), 2);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_server_message_serialization() {
        let msg = ServerMessage::ChoiceRequest {
            choice_seq: 42,
            choice_type: ChoiceType::Priority { available_count: 3 },
            options: vec![
                "Pass priority".to_string(),
                "Play land: Mountain".to_string(),
                "Cast spell: Lightning Bolt".to_string(),
            ],
            state_hash: 0xDEADBEEF,
            action_count: 0,
            context: None,
        };

        let json = serde_json::to_string(&msg).expect("serialize");
        let roundtrip: ServerMessage = serde_json::from_str(&json).expect("deserialize");

        match roundtrip {
            ServerMessage::ChoiceRequest {
                choice_seq,
                options,
                state_hash,
                ..
            } => {
                assert_eq!(choice_seq, 42);
                assert_eq!(options.len(), 3);
                assert_eq!(state_hash, 0xDEADBEEF);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_card_reveal_serialization() {
        let reveal = CardReveal {
            card_id: CardId::new(123),
            name: "Serra Angel".to_string(),
            mana_cost: "{3}{W}{W}".to_string(),
            type_line: "Creature - Angel".to_string(),
            text: "Flying, vigilance".to_string(),
            pt: Some((4, 4)),
        };

        let json = serde_json::to_string(&reveal).expect("serialize");
        assert!(json.contains("Serra Angel"));
        assert!(json.contains("4,4") || json.contains("[4, 4]") || json.contains("\"pt\""));

        let roundtrip: CardReveal = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(roundtrip.name, "Serra Angel");
        assert_eq!(roundtrip.pt, Some((4, 4)));
    }

    #[test]
    fn test_choice_type_serialization() {
        let choice = ChoiceType::Targets {
            spell_id: CardId::new(42),
            target_count: 1,
        };

        let json = serde_json::to_string(&choice).expect("serialize");
        assert!(json.contains("targets"));

        let roundtrip: ChoiceType = serde_json::from_str(&json).expect("deserialize");
        match roundtrip {
            ChoiceType::Targets { spell_id, target_count } => {
                assert_eq!(spell_id, CardId::new(42));
                assert_eq!(target_count, 1);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_reveal_reason_serialization() {
        for reason in [
            RevealReason::Draw,
            RevealReason::Targeting,
            RevealReason::Played,
            RevealReason::Searched,
            RevealReason::Effect,
            RevealReason::OpeningHand,
            RevealReason::TokenCreated,
        ] {
            let json = serde_json::to_string(&reason).expect("serialize");
            let roundtrip: RevealReason = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(roundtrip, reason);
        }
    }

    #[test]
    fn test_all_server_message_variants() {
        // Test all ServerMessage variants for round-trip serialization
        let player_id = PlayerId::new(1);
        let card_id = CardId::new(42);

        let messages: Vec<ServerMessage> = vec![
            ServerMessage::AuthResult {
                success: true,
                error: None,
                your_player_id: Some(player_id),
            },
            ServerMessage::AuthResult {
                success: false,
                error: Some("Invalid password".to_string()),
                your_player_id: None,
            },
            ServerMessage::WaitingForOpponent,
            ServerMessage::GameStarted {
                your_player_id: player_id,
                opponent_name: "Bob".to_string(),
                opening_hand: vec![CardReveal {
                    card_id,
                    name: "Mountain".to_string(),
                    mana_cost: String::new(),
                    type_line: "Basic Land - Mountain".to_string(),
                    text: String::new(),
                    pt: None,
                }],
                opponent_hand_count: 7,
                library_size: 53,
                opponent_library_size: 53,
                opponent_decklist: None,
                starting_life: 20,
                initial_state_hash: 0x12345678,
            },
            ServerMessage::ChoiceRequest {
                choice_seq: 1,
                choice_type: ChoiceType::Priority { available_count: 2 },
                options: vec!["Pass".to_string(), "Play Mountain".to_string()],
                state_hash: 0xABCDEF,
                action_count: 0,
                context: None,
            },
            ServerMessage::CardRevealed {
                owner: player_id,
                card: CardReveal {
                    card_id,
                    name: "Lightning Bolt".to_string(),
                    mana_cost: "{R}".to_string(),
                    type_line: "Instant".to_string(),
                    text: "Deal 3 damage to any target.".to_string(),
                    pt: None,
                },
                reason: RevealReason::Draw,
            },
            ServerMessage::OpponentChoice {
                choice_seq: 5,
                choice_type: ChoiceType::Priority { available_count: 0 },
                choice_index: 0,
                description: "Pass priority".to_string(),
                action_count: 0,
            },
            ServerMessage::GameEnded {
                winner: Some(player_id),
                reason: GameEndReason::PlayerDeath(PlayerId::new(0)),
                final_state_hash: 0xFEDCBA98,
                action_count: 123,
            },
            ServerMessage::GameEnded {
                winner: None,
                reason: GameEndReason::Draw,
                final_state_hash: 0,
                action_count: 456,
            },
            ServerMessage::Error {
                message: "Connection timeout".to_string(),
                fatal: true,
            },
            ServerMessage::Pong {
                timestamp_ms: 1234567890,
            },
        ];

        for msg in messages {
            let json = serde_json::to_string(&msg).expect("serialize");
            let roundtrip: ServerMessage = serde_json::from_str(&json).expect("deserialize");
            // Re-serialize to compare (since we can't derive PartialEq for all variants)
            let json2 = serde_json::to_string(&roundtrip).expect("re-serialize");
            assert_eq!(json, json2, "Round-trip failed for message variant");
        }
    }

    #[test]
    fn test_all_client_message_variants() {
        // Test all ClientMessage variants for round-trip serialization
        let messages: Vec<ClientMessage> = vec![
            ClientMessage::Authenticate {
                password: "secret123".to_string(),
                player_name: "Alice".to_string(),
                deck: DeckSubmission::new(
                    vec![("Forest".to_string(), 20), ("Grizzly Bears".to_string(), 4)],
                    vec![],
                ),
            },
            ClientMessage::SubmitChoice {
                choice_seq: 42,
                choice_index: 1,
                action_count: 0,
            },
            ClientMessage::Ping {
                timestamp_ms: 9876543210,
            },
            ClientMessage::Disconnect,
        ];

        for msg in messages {
            let json = serde_json::to_string(&msg).expect("serialize");
            let roundtrip: ClientMessage = serde_json::from_str(&json).expect("deserialize");
            let json2 = serde_json::to_string(&roundtrip).expect("re-serialize");
            assert_eq!(json, json2, "Round-trip failed for message variant");
        }
    }

    #[test]
    fn test_choice_type_all_variants() {
        let card_id = CardId::new(99);

        let choice_types = vec![
            ChoiceType::Priority { available_count: 5 },
            ChoiceType::Targets {
                spell_id: card_id,
                target_count: 2,
            },
            ChoiceType::ManaSources {
                cost: ManaCost::from_string("2R"),
            },
            ChoiceType::Attackers { available_count: 4 },
            ChoiceType::Blockers {
                attacker_count: 3,
                blocker_count: 5,
            },
            ChoiceType::DamageOrder {
                attacker: card_id,
                blocker_count: 2,
            },
            ChoiceType::Discard { count: 2 },
            ChoiceType::LibrarySearch { valid_count: 10 },
        ];

        for ct in choice_types {
            let json = serde_json::to_string(&ct).expect("serialize");
            let roundtrip: ChoiceType = serde_json::from_str(&json).expect("deserialize");
            let json2 = serde_json::to_string(&roundtrip).expect("re-serialize");
            assert_eq!(json, json2, "Round-trip failed for ChoiceType variant");
        }
    }
}
