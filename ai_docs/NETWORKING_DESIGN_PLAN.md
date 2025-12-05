# MTG Forge-rs Networking Design Plan

**Branch**: `networking`
**Date**: 2025-12-05
**Status**: Draft - Awaiting Approval

## Executive Summary

This document describes a client/server networking architecture for MTG Forge-rs using **deterministic simulation with hidden information enforcement**. Instead of streaming full game state, we synchronize choices and card reveals, allowing each client to run an independent simulation that stays in lockstep with the server.

Key principle: **Clients never receive information they shouldn't have access to** (opponent's hand contents, library order, future draws).

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                         SERVER (Native)                         │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │                    Authoritative GameState               │   │
│  │  - Full library contents and order                       │   │
│  │  - RNG state (ChaCha12Rng)                               │   │
│  │  - Both players' hands                                   │   │
│  └──────────────────────────────────────────────────────────┘   │
│                              │                                   │
│              ┌───────────────┼───────────────┐                   │
│              ▼               ▼               ▼                   │
│     NetworkController  NetworkController   WebSocket            │
│        (Player 1)        (Player 2)        Server               │
└──────────────────────────────────────────────────────────────────┘
                │                       │
        WebSocket                WebSocket
                │                       │
┌───────────────▼───────────────┐ ┌─────▼───────────────────────┐
│      CLIENT 1 (Native/WASM)    │ │    CLIENT 2 (Native/WASM)   │
│  ┌───────────────────────────┐ │ │ ┌───────────────────────────┐│
│  │   Shadow GameState        │ │ │ │   Shadow GameState        ││
│  │  - Own hand (full)        │ │ │ │  - Own hand (full)        ││
│  │  - Opponent hand (count)  │ │ │ │  - Opponent hand (count)  ││
│  │  - RemoteLibrary (buffer) │ │ │ │  - RemoteLibrary (buffer) ││
│  │  - Battlefield (full)     │ │ │ │  - Battlefield (full)     ││
│  └───────────────────────────┘ │ │ └───────────────────────────┘│
│  ┌───────────────────────────┐ │ │ ┌───────────────────────────┐│
│  │   LocalController (TUI)   │ │ │ │   LocalController (TUI)   ││
│  └───────────────────────────┘ │ │ └───────────────────────────┘│
└────────────────────────────────┘ └──────────────────────────────┘
```

---

## Part 1: Protocol Design

### 1.1 Message Types

```rust
// ═══════════════════════════════════════════════════════════════
// CLIENT → SERVER MESSAGES
// ═══════════════════════════════════════════════════════════════

/// Messages sent from client to server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientMessage {
    /// Initial authentication and deck submission
    Authenticate {
        password: String,
        player_name: String,
        deck: DeckList,
    },

    /// Response to a choice request
    SubmitChoice {
        /// Sequence number matching the request
        choice_seq: u32,
        /// The chosen option (index into options array)
        choice_index: usize,
    },

    /// Request to disconnect gracefully
    Disconnect,

    /// Keepalive ping
    Ping { timestamp_ms: u64 },
}

// ═══════════════════════════════════════════════════════════════
// SERVER → CLIENT MESSAGES
// ═══════════════════════════════════════════════════════════════

/// Messages sent from server to client
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMessage {
    /// Authentication result
    AuthResult {
        success: bool,
        error: Option<String>,
        your_player_id: Option<PlayerId>,
    },

    /// Waiting for opponent to connect
    WaitingForOpponent,

    /// Game is starting - includes initial setup info
    GameStarted {
        your_player_id: PlayerId,
        opponent_name: String,
        /// Your opening hand (card IDs and definitions)
        opening_hand: Vec<CardReveal>,
        /// Number of cards in opponent's hand
        opponent_hand_count: usize,
        /// Your library size
        library_size: usize,
        /// Opponent library size (always known per MTG rules)
        opponent_library_size: usize,
        /// Opponent's initial deck list (if deck_visibility is enabled)
        /// This is the INITIAL list before sideboarding - you won't know
        /// which sideboard cards they swapped in for game 2+.
        /// If sideboard is empty, this reveals their exact deck.
        opponent_decklist: Option<DeckListInfo>,
        /// Starting life totals
        starting_life: i32,
        /// Initial game state hash for verification
        initial_state_hash: u64,
    },

    /// Card reveal event (draws, tutors, etc.)
    CardRevealed {
        /// Who the card belongs to
        owner: PlayerId,
        /// The revealed card info
        card: CardReveal,
        /// Reason for reveal
        reason: RevealReason,
    },

    /// Request a choice from this client
    ChoiceRequest {
        /// Sequence number for response correlation
        choice_seq: u32,
        /// Type of choice being requested
        choice_type: ChoiceType,
        /// Human-readable options (for verification)
        options: Vec<String>,
        /// Game state hash at this decision point
        state_hash: u64,
        /// Optional context (e.g., spell being cast)
        context: Option<ChoiceContext>,
    },

    /// Notify client of opponent's choice (for sync)
    OpponentChoice {
        /// Choice sequence
        choice_seq: u32,
        /// What type of choice was made
        choice_type: ChoiceType,
        /// The choice index (for deterministic replay)
        choice_index: usize,
        /// Human-readable description
        description: String,
    },

    /// Debug state dump (sent only when hash verification fails)
    ///
    /// This is NOT part of normal game flow - only sent for diagnostics
    /// when client reports a hash mismatch. Allows diffing client vs server state.
    #[cfg(debug_assertions)]
    DebugStateDump {
        /// Full game state as JSON for debugging
        state_json: String,
        /// What triggered the dump
        reason: String,
        /// Expected hash
        expected_hash: u64,
        /// Client's reported hash (if applicable)
        client_hash: Option<u64>,
    },

    /// Game has ended
    GameEnded {
        winner: Option<PlayerId>,
        reason: GameEndReason,
        final_state_hash: u64,
    },

    /// Error message
    Error {
        message: String,
        fatal: bool,
    },

    /// Keepalive pong
    Pong { timestamp_ms: u64 },
}

// ═══════════════════════════════════════════════════════════════
// SUPPORTING TYPES
// ═══════════════════════════════════════════════════════════════

/// Information about a revealed card
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardReveal {
    /// The card's entity ID
    pub card_id: CardId,
    /// Card name
    pub name: String,
    /// Mana cost string (e.g., "{2}{W}{W}")
    pub mana_cost: String,
    /// Type line (e.g., "Creature - Human Soldier")
    pub type_line: String,
    /// Card text (oracle text)
    pub text: String,
    /// Power/toughness for creatures
    pub pt: Option<(i32, i32)>,
}

/// Reason a card was revealed
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum RevealReason {
    /// Card drawn from library
    Draw,
    /// Card revealed for targeting
    Targeting,
    /// Card played/cast (moved to public zone)
    Played,
    /// Card searched from library (tutor)
    Searched,
    /// Card revealed by an effect
    Effect,
    /// Opening hand
    OpeningHand,
}

/// Type of choice being requested
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChoiceType {
    /// Choose spell/ability to play (or pass)
    Priority { available_count: usize },
    /// Choose targets for spell/ability
    Targets { spell_id: CardId, target_count: usize },
    /// Choose mana sources to tap
    ManaSources { cost: ManaCost },
    /// Choose attackers
    Attackers { available_count: usize },
    /// Choose blockers
    Blockers { attacker_count: usize, blocker_count: usize },
    /// Choose damage assignment order
    DamageOrder { attacker: CardId, blocker_count: usize },
    /// Choose cards to discard
    Discard { count: usize },
    /// Choose card from library (tutor)
    LibrarySearch { valid_count: usize },
}

/// Additional context for choice
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChoiceContext {
    /// Spell being cast (if applicable)
    pub spell: Option<CardReveal>,
    /// Additional description
    pub description: String,
}

/// Opponent's deck list information (tournament-style visibility)
///
/// In tournament play, deck lists are often public knowledge. This struct
/// contains the INITIAL deck list before any sideboarding. After game 1,
/// you know what cards they COULD have, but not which sideboard cards
/// they actually swapped in.
///
/// Note: Deck sizes vary by format (60+ for Standard/Modern, 100 for Commander, etc.)
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
```

### 1.2 State Hash Computation

For verification, we compute a hash of the **public game state** that excludes hidden information.

**Code reuse**: The existing `state_hash.rs` module provides `compute_state_hash()` with configurable field exclusion via `EXCLUDED_FIELDS`. We'll extend this infrastructure rather than duplicating:

```rust
// In src/game/state_hash.rs - extend existing infrastructure

/// Fields to exclude for network hash (hidden information)
const NETWORK_EXCLUDED_FIELDS: &[&str] = &[
    // Existing exclusions for replay/undo
    "choice_id",
    "undo_log",
    "logger",
    "show_choice_menu",
    "output_mode",
    "output_format",
    "numeric_choices",
    "step_header_printed",
    "mana_state_version",
    "token_definitions",

    // Network-specific exclusions (hidden information)
    "rng",                    // Server-only RNG state
    "library",                // Library contents/order (only SIZE is public)
    "hand",                   // Hand contents (only SIZE is public)
];

/// Hash mode determines which fields are excluded
#[derive(Debug, Clone, Copy)]
pub enum HashMode {
    /// Full state hash for replay determinism tests
    Replay,
    /// Hash for undo/redo verification
    UndoTest,
    /// Hash for network sync (excludes hidden information)
    Network,
}

/// Compute state hash with configurable exclusions
///
/// Reuses the JSON-based stripping approach from existing code.
/// For Network mode, we also post-process to include only SIZES
/// for hand and library zones, not their contents.
pub fn compute_state_hash_with_mode(game: &GameState, mode: HashMode) -> u64 {
    let excluded = match mode {
        HashMode::Replay => EXCLUDED_FIELDS,
        HashMode::UndoTest => EXCLUDED_FIELDS_UNDO_TEST,
        HashMode::Network => NETWORK_EXCLUDED_FIELDS,
    };

    let json_value = serde_json::to_value(game).expect("GameState serialization failed");
    let cleaned = strip_fields_recursive(json_value, excluded);

    // For network mode, we need to add zone sizes back
    // (since we stripped the full contents)
    let final_value = if matches!(mode, HashMode::Network) {
        inject_zone_sizes(cleaned, game)
    } else {
        cleaned
    };

    let canonical = serde_json::to_string(&final_value).expect("JSON serialization failed");
    hash_string(&canonical)
}

/// Convenience wrapper for network hash
pub fn compute_network_state_hash(game: &GameState) -> u64 {
    compute_state_hash_with_mode(game, HashMode::Network)
}

/// Inject zone sizes into the hash input (for network mode)
///
/// After stripping hand/library contents, we add back just the sizes
/// since those are public information per MTG rules.
fn inject_zone_sizes(mut value: serde_json::Value, game: &GameState) -> serde_json::Value {
    if let serde_json::Value::Object(ref mut map) = value {
        let mut zone_sizes = serde_json::Map::new();
        for (i, zones) in game.player_zones.iter().enumerate() {
            zone_sizes.insert(
                format!("p{}_hand_size", i),
                serde_json::Value::Number(zones.hand.cards.len().into()),
            );
            zone_sizes.insert(
                format!("p{}_library_size", i),
                serde_json::Value::Number(zones.library.len().into()),
            );
        }
        map.insert("zone_sizes".to_string(), serde_json::Value::Object(zone_sizes));
    }
    value
}
```

**What's hashed (public info)**:
- Battlefield state (all cards, tapped status, counters, attachments)
- Stack contents
- Graveyard contents (public zone)
- Exile contents (usually public)
- Life totals
- Turn/step info
- Hand SIZES (not contents)
- Library SIZES (not contents or order)

**What's excluded (hidden info)**:
- Hand contents
- Library order and contents
- RNG state
- Controller state
- Logging/display state

---

## Part 2: Engine Refactoring

### 2.1 Remote Library Abstraction

The key insight: clients don't know their library contents until cards are revealed. We need a `LibraryMode` enum:

```rust
/// Library operation mode
#[derive(Debug, Clone)]
pub enum LibraryMode {
    /// Local mode: full library contents known
    Local {
        cards: Vec<CardId>,
    },

    /// Remote mode: library contents hidden, receive cards from server
    Remote {
        /// Total cards remaining in library
        size: usize,
        /// Buffer of cards revealed but not yet "drawn" in local sim
        pending_reveals: VecDeque<CardId>,
    },
}

impl LibraryMode {
    /// Draw from top - behavior differs based on mode
    pub fn draw_top(&mut self) -> Option<CardId> {
        match self {
            LibraryMode::Local { cards } => cards.pop(),
            LibraryMode::Remote { size, pending_reveals } => {
                if *size == 0 {
                    return None;
                }
                // Must have a pending reveal from server
                let card = pending_reveals.pop_front()
                    .expect("Remote draw with no pending reveal - sync error!");
                *size -= 1;
                Some(card)
            }
        }
    }

    /// Queue a revealed card (remote mode only)
    pub fn queue_reveal(&mut self, card_id: CardId) {
        match self {
            LibraryMode::Remote { pending_reveals, .. } => {
                pending_reveals.push_back(card_id);
            }
            LibraryMode::Local { .. } => {
                panic!("queue_reveal called in local mode");
            }
        }
    }

    pub fn len(&self) -> usize {
        match self {
            LibraryMode::Local { cards } => cards.len(),
            LibraryMode::Remote { size, .. } => *size,
        }
    }
}
```

### 2.2 Modified CardZone

```rust
/// Card zone with optional remote mode support
pub struct CardZone {
    pub zone_type: Zone,
    pub owner: PlayerId,
    /// Cards in this zone (empty for remote libraries)
    pub cards: Vec<CardId>,
    /// Library-specific remote mode (None for non-library zones)
    pub library_mode: Option<LibraryMode>,
}

impl CardZone {
    /// Create a local library zone
    pub fn new_library(owner: PlayerId, cards: Vec<CardId>) -> Self {
        CardZone {
            zone_type: Zone::Library,
            owner,
            cards, // Used for local iteration
            library_mode: Some(LibraryMode::Local { cards: cards.clone() }),
        }
    }

    /// Create a remote library zone (client mode)
    pub fn new_remote_library(owner: PlayerId, size: usize) -> Self {
        CardZone {
            zone_type: Zone::Library,
            owner,
            cards: Vec::new(), // Empty - we don't know contents
            library_mode: Some(LibraryMode::Remote {
                size,
                pending_reveals: VecDeque::new(),
            }),
        }
    }

    /// Draw from library (handles both modes)
    pub fn draw_top(&mut self) -> Option<CardId> {
        if let Some(ref mut mode) = self.library_mode {
            mode.draw_top()
        } else {
            self.cards.pop()
        }
    }
}
```

### 2.3 Network Controller

A controller that proxies to a remote player:

```rust
/// Controller that communicates with a remote player over network
pub struct NetworkController {
    player_id: PlayerId,
    /// Channel to send choice requests
    request_tx: mpsc::Sender<ChoiceRequest>,
    /// Channel to receive choice responses
    response_rx: mpsc::Receiver<ChoiceResponse>,
    /// Current choice sequence number
    choice_seq: u32,
}

impl NetworkController {
    pub fn new(
        player_id: PlayerId,
        request_tx: mpsc::Sender<ChoiceRequest>,
        response_rx: mpsc::Receiver<ChoiceResponse>,
    ) -> Self {
        NetworkController {
            player_id,
            request_tx,
            response_rx,
            choice_seq: 0,
        }
    }

    /// Send a choice request and wait for response
    fn request_choice(
        &mut self,
        choice_type: ChoiceType,
        options: Vec<String>,
        state_hash: u64,
    ) -> Result<usize, NetworkError> {
        self.choice_seq += 1;

        let request = ChoiceRequest {
            choice_seq: self.choice_seq,
            choice_type,
            options,
            state_hash,
        };

        self.request_tx.blocking_send(request)?;

        let response = self.response_rx.blocking_recv()?;

        if response.choice_seq != self.choice_seq {
            return Err(NetworkError::SequenceMismatch);
        }

        Ok(response.choice_index)
    }
}

impl PlayerController for NetworkController {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn choose_spell_ability_to_play(
        &mut self,
        view: &GameStateView,
        available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>> {
        // Build options list (matching format_choice_menu)
        let mut options = vec!["Pass priority".to_string()];
        for ability in available {
            let desc = match ability {
                SpellAbility::PlayLand { card_id } => {
                    let name = view.card_name(*card_id).unwrap_or_default();
                    format!("Play land: {}", name)
                }
                SpellAbility::CastSpell { card_id } => {
                    let name = view.card_name(*card_id).unwrap_or_default();
                    format!("Cast spell: {}", name)
                }
                SpellAbility::ActivateAbility { card_id, .. } => {
                    let name = view.card_name(*card_id).unwrap_or_default();
                    format!("Activate ability: {}", name)
                }
            };
            options.push(desc);
        }

        let state_hash = compute_network_state_hash(view.game, self.player_id);

        match self.request_choice(
            ChoiceType::Priority { available_count: available.len() },
            options,
            state_hash,
        ) {
            Ok(0) => ChoiceResult::Ok(None), // Pass
            Ok(idx) => ChoiceResult::Ok(Some(available[idx - 1].clone())),
            Err(NetworkError::Disconnected) => ChoiceResult::ExitGame,
            Err(e) => ChoiceResult::Error(e.to_string()),
        }
    }

    // ... similar implementations for other choice methods
}
```

### 2.4 Client Shadow State

The client maintains a "shadow" game state that mirrors the server's state using revealed information:

```rust
/// Client-side game state with hidden information handling
pub struct ClientGameState {
    /// The shadow game state (deterministic simulation)
    pub game: GameState,
    /// Our player ID
    pub our_player: PlayerId,
    /// Cards we know about (revealed to us)
    pub known_cards: HashMap<CardId, CardReveal>,
    /// Expected state hash (from server)
    pub expected_hash: u64,
}

impl ClientGameState {
    /// Process a card reveal from server
    pub fn process_reveal(&mut self, card: CardReveal, reason: RevealReason) {
        // Store the card info
        self.known_cards.insert(card.card_id, card.clone());

        // If it's a draw for us, queue it in our library
        if card.owner == self.our_player && matches!(reason, RevealReason::Draw) {
            if let Some(zones) = self.game.get_player_zones_mut(self.our_player) {
                zones.library.queue_reveal(card.card_id);
            }
        }

        // Instantiate the card in our local registry if needed
        if !self.game.cards.contains(card.card_id) {
            let card_instance = self.instantiate_card(&card);
            self.game.cards.insert(card.card_id, card_instance);
        }
    }

    /// Process an opponent choice notification
    pub fn process_opponent_choice(&mut self, choice_type: ChoiceType, choice_index: usize) {
        // Apply the choice to our shadow state
        // This keeps us in sync without knowing private info
    }

    /// Verify our state matches server's hash
    pub fn verify_hash(&self, expected: u64) -> bool {
        let actual = compute_network_state_hash(&self.game, self.our_player);
        if actual != expected {
            eprintln!("State hash mismatch! Expected {:#x}, got {:#x}", expected, actual);
            false
        } else {
            true
        }
    }
}
```

---

## Part 3: Server Implementation

### 3.1 Server Structure

```rust
/// MTG game server
pub struct GameServer {
    /// Server configuration
    config: ServerConfig,
    /// Currently waiting player (first to connect)
    waiting_player: Option<WaitingPlayer>,
    /// Active games
    games: HashMap<GameId, ActiveGame>,
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub port: u16,
    pub password: String,
    pub max_games: usize,
    pub starting_life: i32,
    /// Whether to share initial deck lists between players (tournament mode)
    ///
    /// When enabled, each player receives their opponent's initial deck list
    /// (main deck + sideboard) at game start. This mirrors tournament play
    /// where deck lists are public. Note:
    /// - This is the INITIAL list, not accounting for sideboard swaps
    /// - If sideboard is empty, opponent knows your exact deck
    /// - Deck SIZE is always visible regardless of this setting (per MTG rules)
    pub deck_visibility: bool,
}

struct WaitingPlayer {
    name: String,
    deck: DeckList,
    connection: WebSocketConnection,
}

struct ActiveGame {
    /// The authoritative game state
    game: GameState,
    /// Game loop (paused, waiting for network input)
    // game_loop: GameLoop,
    /// Player 1 connection
    p1_connection: WebSocketConnection,
    /// Player 2 connection
    p2_connection: WebSocketConnection,
    /// Channel for P1's choices
    p1_choices: (mpsc::Sender<ChoiceResponse>, mpsc::Receiver<ChoiceRequest>),
    /// Channel for P2's choices
    p2_choices: (mpsc::Sender<ChoiceResponse>, mpsc::Receiver<ChoiceRequest>),
}
```

### 3.2 Server Main Loop

```rust
impl GameServer {
    pub async fn run(&mut self) -> Result<()> {
        let listener = TcpListener::bind(("0.0.0.0", self.config.port)).await?;
        println!("MTG Server listening on port {}", self.config.port);

        loop {
            let (stream, addr) = listener.accept().await?;
            let ws = accept_async(stream).await?;

            println!("New connection from {}", addr);

            // Handle authentication
            let auth_msg = self.receive_message(&ws).await?;
            match auth_msg {
                ClientMessage::Authenticate { password, player_name, deck } => {
                    if password != self.config.password {
                        self.send_message(&ws, ServerMessage::AuthResult {
                            success: false,
                            error: Some("Invalid password".to_string()),
                            your_player_id: None,
                        }).await?;
                        continue;
                    }

                    // Check if we have a waiting player
                    if let Some(waiting) = self.waiting_player.take() {
                        // Start game with both players
                        self.start_game(waiting, WaitingPlayer {
                            name: player_name,
                            deck,
                            connection: ws,
                        }).await?;
                    } else {
                        // First player - wait for opponent
                        self.send_message(&ws, ServerMessage::AuthResult {
                            success: true,
                            error: None,
                            your_player_id: Some(PlayerId::new(0)),
                        }).await?;
                        self.send_message(&ws, ServerMessage::WaitingForOpponent).await?;

                        self.waiting_player = Some(WaitingPlayer {
                            name: player_name,
                            deck,
                            connection: ws,
                        });
                    }
                }
                _ => {
                    self.send_message(&ws, ServerMessage::Error {
                        message: "Expected authentication".to_string(),
                        fatal: true,
                    }).await?;
                }
            }
        }
    }

    async fn start_game(&mut self, p1: WaitingPlayer, p2: WaitingPlayer) -> Result<()> {
        // Create game state with both decks
        let mut game = GameState::new_two_player(
            p1.name.clone(),
            p2.name.clone(),
            self.config.starting_life,
        );

        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // Load cards deterministically
        // ... (sorted loading)

        // Shuffle libraries with server RNG
        game.seed_rng(rand::random());
        game.shuffle_library(p1_id);
        game.shuffle_library(p2_id);

        // Draw opening hands
        let p1_hand = self.draw_opening_hand(&mut game, p1_id);
        let p2_hand = self.draw_opening_hand(&mut game, p2_id);

        let initial_hash = compute_network_state_hash(&game, p1_id);

        // Send GameStarted to both players
        self.send_message(&p1.connection, ServerMessage::GameStarted {
            your_player_id: p1_id,
            opponent_name: p2.name.clone(),
            opening_hand: p1_hand.clone(),
            opponent_hand_count: p2_hand.len(),
            library_size: game.get_player_zones(p1_id).unwrap().library.len(),
            opponent_library_size: game.get_player_zones(p2_id).unwrap().library.len(),
            starting_life: self.config.starting_life,
            initial_state_hash: initial_hash,
        }).await?;

        self.send_message(&p2.connection, ServerMessage::GameStarted {
            your_player_id: p2_id,
            opponent_name: p1.name.clone(),
            opening_hand: p2_hand.clone(),
            opponent_hand_count: p1_hand.len(),
            library_size: game.get_player_zones(p2_id).unwrap().library.len(),
            opponent_library_size: game.get_player_zones(p1_id).unwrap().library.len(),
            starting_life: self.config.starting_life,
            initial_state_hash: initial_hash,
        }).await?;

        // Create network controllers and run game loop
        // ... (spawn game task)

        Ok(())
    }
}
```

---

## Part 4: CLI Integration

### 4.1 New Subcommands

Use clap's `#[command(flatten)]` to share argument structures between `tui` and `connect`:

```rust
/// Shared arguments for local player configuration
/// Used by both `tui` (local game) and `connect` (network client)
#[derive(Args, Clone)]
pub struct LocalPlayerArgs {
    /// Deck file (.dck) for this player
    #[arg(value_name = "DECK")]
    pub deck: PathBuf,

    /// Controller type for local play
    #[arg(long, value_enum, default_value = "fancy")]
    pub controller: ControllerType,

    /// Player name
    #[arg(long, default_value = "Player")]
    pub name: String,

    /// Fixed script input (space or comma separated indices)
    #[arg(long, value_name = "CHOICES")]
    pub fixed_inputs: Option<String>,

    /// Terminal width for fancy-fixed controller screenshots
    #[arg(long, default_value = "240")]
    pub screenshot_width: u16,

    /// Terminal height for fancy-fixed controller screenshots
    #[arg(long, default_value = "60")]
    pub screenshot_height: u16,

    /// Random seed for controller (if applicable)
    #[arg(long)]
    pub seed: Option<SeedArg>,
}

#[derive(Subcommand)]
enum Commands {
    /// Text UI Mode - Local game (existing, uses LocalPlayerArgs for P1)
    Tui {
        #[command(flatten)]
        p1: LocalPlayerArgs,

        /// Deck file for player 2 (optional; if omitted, uses P1 deck)
        #[arg(value_name = "PLAYER2_DECK")]
        deck2: Option<PathBuf>,

        /// Player 2 controller type
        #[arg(long, value_enum, default_value = "heuristic")]
        p2: ControllerType,

        /// Player 2 name
        #[arg(long, default_value = "Player2")]
        p2_name: String,

        // ... other existing tui args (seed, start_state, etc.)
    },

    /// Run a game server
    Server {
        /// Port to listen on
        #[arg(long, default_value = "17771")]
        port: u16,

        /// Password required to join
        #[arg(long)]
        password: String,

        /// Starting life total
        #[arg(long, default_value = "20")]
        life: i32,

        /// Share deck lists between players (tournament mode)
        #[arg(long, default_value = "false")]
        deck_visibility: bool,
    },

    /// Connect to a game server as a network client
    Connect {
        #[command(flatten)]
        player: LocalPlayerArgs,

        /// Server address (host:port)
        #[arg(long, default_value = "localhost:17771")]
        server: String,

        /// Password for server
        #[arg(long)]
        password: String,
    },
}
```

### 4.2 Default Port

**Port 17771** ("MTG" on a phone keypad: M=6, T=8, G=4... ok not quite, but easy to remember as "MTG-71").

---

## Part 5: Implementation Tasks

### Phase 1: Protocol Foundation (mtg-XXX)

1. **Create `src/network/mod.rs`** - Module structure
2. **Define protocol types** - `ClientMessage`, `ServerMessage`, supporting types
3. **Implement `compute_network_state_hash()`** - Verification hash
4. **Add dependencies** - `tokio-tungstenite`, `futures`, etc.

### Phase 2: Engine Refactoring (mtg-XXX)

1. **Create `LibraryMode` enum** - Remote library abstraction
2. **Modify `CardZone`** - Add library_mode support
3. **Update `draw_card()`** - Handle remote mode
4. **Update zone iterators** - Consistent behavior

### Phase 3: Network Controller (mtg-XXX)

1. **Create `NetworkController`** - Server-side remote player proxy
2. **Implement `PlayerController` trait** - All choice methods
3. **Create `ClientController`** - Client-side wrapper that sends to server

### Phase 4: Server Implementation (mtg-XXX)

1. **Create `GameServer`** - Connection handling, game lifecycle
2. **Implement WebSocket handling** - tokio-tungstenite
3. **Implement game loop integration** - Run with network controllers
4. **Add CLI `server` subcommand**

### Phase 5: Client Implementation (mtg-XXX)

1. **Create `ClientGameState`** - Shadow state with hidden info
2. **Implement WebSocket client** - Connection, message handling
3. **Integrate with existing TUI** - FancyTuiController as local UI
4. **Add CLI `client` subcommand**

### Phase 6: Testing & Validation (mtg-XXX)

1. **Unit tests** - Protocol serialization, hash computation
2. **Integration tests** - Local server + 2 clients
3. **E2E tests** - Full game with fixed inputs
4. **Determinism verification** - Compare server/client hashes

---

## Part 6: Dependencies to Add

```toml
# In Cargo.toml [dependencies]

# WebSocket (native)
tokio-tungstenite = { version = "0.26", optional = true }
futures-util = { version = "0.3", optional = true }

# WebSocket (WASM) - for future browser client
gloo-net = { version = "0.6", optional = true }

[features]
network = ["tokio-tungstenite", "futures-util"]
network-wasm = ["gloo-net"]
```

---

## Part 7: Entity ID Determinism

The current implementation uses sequential IDs starting from a shared counter. For network sync:

1. **Card loading order must be sorted** - Already done in `game_init.rs`
2. **Player IDs assigned first** - Players get IDs 0 and 1
3. **Cards instantiated in deck order** - P1's deck, then P2's deck
4. **Token creation synchronized** - Server broadcasts token creates

The existing deterministic ID allocation should work, but we need tests to verify both client and server generate identical IDs.

---

## Open Questions

1. **Mulligan support** - How to handle mulligan decisions over network?
2. **Reconnection** - Should we support reconnecting to a game in progress?
3. **Spectators** - Future feature: allow read-only spectators?
4. **Concurrent games** - One server hosting multiple games?

---

## Design Notes

### Deck Visibility (Tournament Mode)

Per MTG tournament rules and your requirements:

- **Deck SIZE is always visible** - Both players always know how many cards are in each other's libraries (this is public information in MTG)
- **Deck CONTENTS are optionally visible** - With `--deck-visibility`, players receive opponent's initial deck list (main + sideboard) at game start
- **Sideboard swaps are hidden** - Even with deck visibility, you only know the initial registered list, not which sideboard cards were swapped in for games 2+
- **If sideboard is empty** - With deck visibility enabled, opponent knows your exact deck

This allows both casual play (hidden lists) and tournament-style play (open lists).

Note: Deck sizes vary by format (60+ main + 15 sideboard for Standard/Modern, 100 singleton for Commander, etc.)

---

## Approval Checklist

- [ ] Protocol message types approved
- [ ] State hash design approved
- [ ] Remote library abstraction approved
- [ ] CLI interface approved
- [ ] Implementation phases approved
- [ ] Testing approach approved

Please review this design and provide feedback or approval to proceed.
