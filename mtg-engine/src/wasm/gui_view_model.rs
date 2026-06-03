//! View model for the native HTML GUI (`web/native_game.html`).
//!
//! This module exports a structured, semantic snapshot of the game state for
//! the HTML GUI to render. The goal is to keep `native_game.html` a *thin* DOM
//! renderer: every display decision (sort order, section grouping, log color
//! classification, formatted card details, status text, …) is made here in
//! Rust, mirroring the choices the TUI renderer makes.
//!
//! ## Why a dedicated view model?
//!
//! The legacy `tui_get_full_state_json()` exports a fairly raw dump of cards
//! and lets JavaScript decide how to format them. That duplicates the
//! formatting logic that `FancyTuiRenderer` already implements (e.g. hand
//! sort order, battlefield section ordering, log color classification, card
//! detail formatting). When the TUI changes, the HTML GUI silently drifts.
//!
//! With this module:
//! - Hand cards are sorted via the shared `FancyTuiRenderer::get_sorted_hand`
//! - Battlefield sections use the shared `categorize_card` helper
//! - Log color/bold come from the shared `classify_log_message` helper
//! - Selected card details come from a single formatter that mirrors
//!   `draw_card_details`
//!
//! All entities are referenced by stable `CardId` (`u32`) — never by name,
//! and never by debug-printed IDs like `"CardId(35)"`. This lets the GUI
//! key DOM nodes off `card_id` so it can preserve scroll, animations, etc.
//! across redraws.

use serde::Serialize;

use crate::core::{CardId, PlayerId};
use crate::game::controller::GameStateView;
use crate::game::fancy_tui_renderer::{
    categorize_card, classify_log_message, CardCategory, FancyTuiRenderer, LogClass,
};
use crate::game::{GameState, Step};

// ---------------------------------------------------------------------------
// View model types
// ---------------------------------------------------------------------------

/// Top-level snapshot for the HTML GUI.
#[derive(Debug, Clone, Serialize)]
pub struct GuiViewModel {
    /// Schema version. Bump on breaking changes so the JS side can detect a
    /// stale renderer.
    pub schema_version: u32,
    /// Turn number (global, 1-based).
    pub turn_number: u32,
    /// Current step (Debug-formatted, e.g. "Main1").
    pub current_step: String,
    /// Short step abbreviation (e.g. "M1").
    pub current_step_abbrev: &'static str,
    /// Index into `players` of the player whose turn it is.
    pub active_player_idx: usize,
    /// Index into `players` of the player whose perspective the GUI shows.
    pub our_player_idx: usize,
    /// Whether the game is over.
    pub game_over: bool,
    /// Fatal engine/session error to surface to the player, if any. This is the
    /// rewind/replay verifier + monotonicity-invariant failure message set on
    /// the shared `WasmFancyTuiState` (see `fancy_tui.rs`). The terminal renderer
    /// (tui_game.html) draws it directly; native_game.html had no way to see it
    /// because the view model never carried it — so the same fatal assertion that
    /// halts tui_game silently passed in native_game (mtg-436). Exposing it here
    /// gives both pages identical surfacing. `None` when there is no error.
    pub error_message: Option<String>,
    /// Per-player views, in seat order.
    pub players: Vec<PlayerView>,
    /// Cards on the stack (top of vec is top of stack).
    pub stack: Vec<StackEntryView>,
    /// Pre-formatted status bar text (matches the TUI's status line).
    pub status_text: String,
    /// Current prompt the controller is asking about, if any.
    pub current_prompt: Option<String>,
    /// Available choices for the human controller.
    pub choices: Vec<ChoiceView>,
    /// Currently highlighted choice index.
    pub selected_choice_idx: usize,
    /// Semantic context of the current choice (e.g. "DeclareAttackers").
    pub choice_context: &'static str,
    /// Recent log entries, oldest first, with shared semantic color/bold.
    pub logs: Vec<LogEntryView>,
    /// Selected card details (matches the TUI's `draw_card_details` panel).
    /// `None` when no card is selected.
    pub selected_card: Option<CardDetailView>,
}

/// All info the GUI needs to render one player's row + zones.
#[derive(Debug, Clone, Serialize)]
pub struct PlayerView {
    /// Stable PlayerId as u32. Use this as the DOM key.
    pub player_id: u32,
    /// Seat index (0-based).
    pub index: usize,
    /// Player name (e.g. "Player1").
    pub name: String,
    /// Pre-formatted player label (e.g. "Player1 (P1)" or just "P1").
    pub label: String,
    /// Compact "P1" / "P2" badge.
    pub seat_badge: &'static str,
    /// Life total (may be negative).
    pub life: i32,
    /// True if this player is the active player this turn.
    pub is_active: bool,
    /// True if this is the player whose perspective we're rendering.
    pub is_us: bool,
    /// Total cards remaining in library (correct under reveal/peek effects).
    pub library_size: usize,
    /// Hand size (always available, even for opponents).
    pub hand_size: usize,
    /// Graveyard size.
    pub graveyard_size: usize,
    /// Pre-formatted player info bar text — matches `draw_player_info`.
    pub info_bar_text: String,
    /// Current available mana, by color.
    pub mana_pool: ManaPoolView,
    /// Hand contents — populated only for the local (`is_us`) player.
    /// Sorted by `FancyTuiRenderer::get_sorted_hand` (lands first, then by
    /// descending CMC) so the index here matches the TUI's index.
    pub hand: Vec<CardView>,
    /// Battlefield sections owned/controlled by this player, in display order.
    /// For the local player: PWs → Creatures → Enchants → Artifacts → Lands.
    /// For the opponent: lands first (closer to the local player visually).
    pub battlefield_sections: Vec<BattlefieldSection>,
    /// Graveyard contents.
    pub graveyard: Vec<CardView>,
    /// Command zone contents (Commander format).
    pub command_zone: Vec<CardView>,
}

/// Mana pool grouped by color (W/U/B/R/G/C).
#[derive(Debug, Clone, Serialize)]
pub struct ManaPoolView {
    pub white: u8,
    pub blue: u8,
    pub black: u8,
    pub red: u8,
    pub green: u8,
    pub colorless: u8,
}

/// A section of the battlefield grouped by card category.
#[derive(Debug, Clone, Serialize)]
pub struct BattlefieldSection {
    /// Category label (e.g. "Creatures", "Lands"). Stable across builds.
    pub label: &'static str,
    /// Machine-readable category enum.
    pub category: SerializedCardCategory,
    /// Cards in this section, in stable CardId order.
    pub cards: Vec<CardView>,
}

/// Serializable form of `CardCategory` — uses simple strings for JS friendliness.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SerializedCardCategory {
    Planeswalker,
    Creature,
    Enchantment,
    Artifact,
    Land,
    Other,
}

impl From<CardCategory> for SerializedCardCategory {
    fn from(c: CardCategory) -> Self {
        match c {
            CardCategory::Planeswalker => SerializedCardCategory::Planeswalker,
            CardCategory::Creature => SerializedCardCategory::Creature,
            CardCategory::Enchantment => SerializedCardCategory::Enchantment,
            CardCategory::Artifact => SerializedCardCategory::Artifact,
            CardCategory::Land => SerializedCardCategory::Land,
            CardCategory::Other => SerializedCardCategory::Other,
        }
    }
}

/// A card snapshot suitable for rendering as a tile or list entry.
#[derive(Debug, Clone, Serialize)]
pub struct CardView {
    /// Stable card identifier as `u32` — DOM key, image cache key, etc.
    pub card_id: u32,
    /// Display name.
    pub name: String,
    /// Mana cost (e.g. "{1}{R}{R}"); empty string if no cost.
    pub mana_cost: String,
    /// Converted mana cost.
    pub cmc: u32,
    /// Oracle text (may contain newlines).
    pub oracle_text: String,
    /// Pre-built type line (e.g. "Creature - Goblin").
    pub type_line: String,
    /// Card categorization (single, primary).
    pub category: SerializedCardCategory,
    /// Whether the card is currently tapped.
    pub is_tapped: bool,
    /// Whether the card has summoning sickness this turn.
    pub summoning_sick: bool,
    /// Power (effective), `None` for non-creatures.
    pub power: Option<i32>,
    /// Toughness (effective), `None` for non-creatures.
    pub toughness: Option<i32>,
    /// Pre-formatted "P/T" string for the GUI badge, `None` for non-creatures.
    pub formatted_pt: Option<String>,
    /// Marked damage (creatures).
    pub damage: i32,
    /// Pre-computed CSS classes (matches legacy `tui_get_full_state_json`).
    pub css_classes: Vec<&'static str>,
    /// `card_id` of the card this is attached to (equipment/auras).
    pub attached_to: Option<u32>,
    /// Cards attached TO this card (equipment on a creature).
    pub attachments: Vec<AttachmentView>,
    /// True if this card is a valid choice in the current prompt.
    pub is_valid_choice: bool,
    /// True if this card is currently selected in the details pane.
    pub is_selected: bool,
}

/// A minimal reference to an attached card (for badges / stacks).
#[derive(Debug, Clone, Serialize)]
pub struct AttachmentView {
    pub card_id: u32,
    pub name: String,
}

/// A card on the stack.
#[derive(Debug, Clone, Serialize)]
pub struct StackEntryView {
    pub card_id: u32,
    pub name: String,
    /// Index into `players` of the controller of this stack object.
    pub controller_idx: usize,
}

/// One entry in the choices list shown to the human controller.
#[derive(Debug, Clone, Serialize)]
pub struct ChoiceView {
    /// 0-based index in the choice list.
    pub index: usize,
    /// 1-based display number (matches the TUI's "1." / "2." prefix).
    pub display_number: usize,
    /// User-facing choice text.
    pub text: String,
    /// Whether this choice should be visually highlighted (auxiliary hint
    /// from the controller — e.g. recommended action).
    pub highlighted: bool,
}

/// Pre-formatted card details panel — mirrors `draw_card_details`.
#[derive(Debug, Clone, Serialize)]
pub struct CardDetailView {
    /// Stable CardId.
    pub card_id: u32,
    /// "Name (id)" — matches the log format used in `draw_card_details`.
    pub name_with_id: String,
    /// Card name without the id suffix.
    pub name: String,
    /// "Cost: {…}" line, `None` if cmc == 0 and no cost.
    pub cost_line: Option<String>,
    /// "Type: …" line.
    pub type_line: String,
    /// "P/T: 3/3" line, with "(base 2/2)" if buffed. `None` for non-creatures.
    pub pt_line: Option<String>,
    /// "Status: Tapped, Summoning Sick" line. `None` if no statuses apply.
    pub status_line: Option<String>,
    /// Oracle text split into lines (preserves newlines from card text).
    pub oracle_lines: Vec<String>,
}

/// One log entry with shared semantic color/bold.
#[derive(Debug, Clone, Serialize)]
pub struct LogEntryView {
    /// Raw message text.
    pub text: String,
    /// CSS color string (e.g. "#ff5555") matching the TUI palette.
    pub color: &'static str,
    /// Whether the message should be bold (turn headers + damage).
    pub bold: bool,
    /// Whether the message is a "<Choice>" tracker entry; the GUI typically
    /// filters these out but exposes them for debug overlays.
    pub is_choice: bool,
    /// Semantic class — exposed for stylesheets that want to override colors.
    pub semantic_class: &'static str,
}

// ---------------------------------------------------------------------------
// Color mapping (single source of truth)
// ---------------------------------------------------------------------------

/// Map a `LogClass` to a CSS color string and stable className.
///
/// Mirrors the TUI palette in `style_for_log_content`. Centralized here so the
/// GUI and the TUI can never drift on what color a class renders as.
pub fn css_color_for_log_class(class: LogClass) -> (&'static str, &'static str) {
    match class {
        LogClass::TurnHeader => ("#ffd700", "log-turn-header"),
        LogClass::StepHeader => ("#4cc9f0", "log-step-header"),
        LogClass::Combat => ("#ff79c6", "log-combat"),
        LogClass::Damage => ("#ff5555", "log-damage"),
        LogClass::LifeGain | LogClass::Resolves => ("#50fa7b", "log-life-gain"),
        LogClass::ManaTap | LogClass::Targeting => ("#666", "log-aux"),
        LogClass::Choice => ("#4cc9f0", "log-choice"),
        LogClass::Player1 => ("#6272a4", "log-p1"),
        LogClass::Player2 => ("#ff6e6e", "log-p2"),
        LogClass::Default => ("#ccc", "log-default"),
    }
}

// ---------------------------------------------------------------------------
// Card formatting helpers
// ---------------------------------------------------------------------------

/// Build a CSS class list for a card. Centralized so the GUI can never drift
/// from the TUI's understanding of "tapped/land/creature/equipment" classes.
fn build_css_classes(card: &crate::core::Card) -> Vec<&'static str> {
    let mut css: Vec<&'static str> = vec!["card"];
    if card.tapped {
        css.push("tapped");
    }
    if card.is_land() {
        css.push("land");
    }
    if card.is_creature() {
        css.push("creature");
    }
    if card.is_equipment() {
        css.push("equipment");
    }
    if card.is_planeswalker() {
        css.push("planeswalker");
    }
    if card.is_artifact() {
        css.push("artifact");
    }
    if card.is_enchantment() {
        css.push("enchantment");
    }
    css
}

/// Construct the type line (e.g. "Legendary Creature - Human Warrior").
fn format_type_line(card: &crate::core::Card) -> String {
    // CardType is a unit-variant enum with no Display impl; its Debug output
    // is just the variant name ("Land", "Creature"), which is what we want.
    let type_names: Vec<String> = card.types.iter().map(|t| format!("{:?}", t)).collect();
    // Subtype is a String newtype with a Display impl that prints the inner
    // string. Using `{}` here (not `{:?}`) avoids the ugly
    // `Subtype("Basic")` debug rendering that the previous version produced
    // (caught by the playtest agent — see the seed-123 game where the
    // "Plains" detail showed `Subtype("Basic") Subtype("Plains")` instead
    // of `Basic Plains`).
    let subtype_names: Vec<String> = card.subtypes.iter().map(|s| format!("{}", s)).collect();
    if subtype_names.is_empty() {
        type_names.join(" ")
    } else {
        format!("{} - {}", type_names.join(" "), subtype_names.join(" "))
    }
}

/// Build a `CardView`. Hand callers may pass `None` for `effective_pt_override`
/// to compute power/toughness from the card directly; battlefield callers
/// should pass the GameStateView-derived effective values so static buffs
/// (e.g. anthem effects) are reflected.
fn build_card_view(
    game: &GameState,
    card_id: CardId,
    valid_choices: &[CardId],
    selected_card_id: Option<CardId>,
    effective_pt_override: Option<(Option<i32>, Option<i32>)>,
) -> Option<CardView> {
    let card = game.cards.try_get(card_id)?;
    let category = SerializedCardCategory::from(categorize_card(card));

    let (power, toughness) = if let Some((p, t)) = effective_pt_override {
        (p, t)
    } else if card.is_creature() {
        (
            Some(i32::from(card.base_power().unwrap_or(0))),
            Some(i32::from(card.base_toughness().unwrap_or(0))),
        )
    } else {
        (None, None)
    };
    let formatted_pt = if card.is_creature() {
        power.zip(toughness).map(|(p, t)| format!("{}/{}", p, t))
    } else {
        None
    };

    // Find equipment attached TO this card (creatures only).
    let attachments: Vec<AttachmentView> = if card.is_creature() {
        game.battlefield
            .cards
            .iter()
            .filter_map(|&eid| {
                let eq = game.cards.try_get(eid)?;
                if eq.is_equipment() && eq.attached_to == Some(card_id) {
                    Some(AttachmentView {
                        card_id: eid.as_u32(),
                        name: eq.name.to_string(),
                    })
                } else {
                    None
                }
            })
            .collect()
    } else {
        Vec::new()
    };

    let summoning_sick = card.is_creature() && card.turn_entered_battlefield == Some(game.turn.turn_number);

    Some(CardView {
        card_id: card_id.as_u32(),
        name: card.name.to_string(),
        mana_cost: card.mana_cost.to_string(),
        cmc: u32::from(card.mana_cost.cmc()),
        oracle_text: card.text.clone(),
        type_line: format_type_line(card),
        category,
        is_tapped: card.tapped,
        summoning_sick,
        power,
        toughness,
        formatted_pt,
        damage: card.damage,
        css_classes: build_css_classes(card),
        attached_to: card.attached_to.map(|id| id.as_u32()),
        attachments,
        is_valid_choice: valid_choices.contains(&card_id),
        is_selected: selected_card_id == Some(card_id),
    })
}

/// Build the selected card detail panel from a perspective + card id.
///
/// Public wrapper around `build_selected_card_detail` for callers (such as
/// the `tui_select_card` WASM export) that already know the perspective and
/// just need the formatted detail panel JSON. Returns `None` if the card is
/// not present in the game state.
pub fn selected_card_detail(game: &GameState, perspective: PlayerId, card_id: CardId) -> Option<CardDetailView> {
    let view = GameStateView::new(game, perspective);
    build_selected_card_detail(&view, card_id)
}

/// Serialize a single `CardDetailView` (or `null` if `None`) to JSON. This is
/// the response shape used by `tui_select_card` so the GUI can render the
/// details panel without re-fetching the full view model.
pub fn selected_card_detail_json(detail: Option<CardDetailView>) -> String {
    serde_json::to_string(&detail).unwrap_or_else(|_| "null".to_string())
}

/// Build the selected card detail panel. Mirrors `draw_card_details`.
fn build_selected_card_detail(view: &GameStateView, selected: CardId) -> Option<CardDetailView> {
    let card = view.get_card(selected)?;

    let cost_line = if card.mana_cost.cmc() > 0 {
        Some(format!("Cost: {}", card.mana_cost))
    } else {
        None
    };

    let type_line = format_type_line(card);

    let pt_line = if card.is_creature() {
        let power = view
            .get_effective_power(selected)
            .unwrap_or_else(|| i32::from(card.current_power()));
        let toughness = view
            .get_effective_toughness(selected)
            .unwrap_or_else(|| i32::from(card.current_toughness()));
        let base_power = i32::from(card.base_power().unwrap_or(0));
        let base_toughness = i32::from(card.base_toughness().unwrap_or(0));
        Some(if power != base_power || toughness != base_toughness {
            format!("P/T: {}/{} (base {}/{})", power, toughness, base_power, base_toughness)
        } else {
            format!("P/T: {}/{}", power, toughness)
        })
    } else {
        None
    };

    let mut status_parts = Vec::new();
    if card.tapped {
        status_parts.push("Tapped");
    }
    if card.is_creature() && card.turn_entered_battlefield == Some(view.turn_number()) {
        status_parts.push("Summoning Sick");
    }
    let status_line = if status_parts.is_empty() {
        None
    } else {
        Some(format!("Status: {}", status_parts.join(", ")))
    };

    let oracle_lines: Vec<String> = if card.text.is_empty() {
        Vec::new()
    } else {
        card.text.split('\n').map(|s| s.to_string()).collect()
    };

    Some(CardDetailView {
        card_id: selected.as_u32(),
        name_with_id: format!("{} ({})", card.name, selected.as_u32()),
        name: card.name.to_string(),
        cost_line,
        type_line,
        pt_line,
        status_line,
        oracle_lines,
    })
}

// ---------------------------------------------------------------------------
// Player formatting
// ---------------------------------------------------------------------------

/// Build the formatted player label (e.g. "Player1 (P1)", or just "P1" if
/// the name already matches the seat).
fn build_player_label(name: &str, seat_badge: &'static str) -> String {
    if name == seat_badge {
        name.to_string()
    } else {
        format!("{} ({})", name, seat_badge)
    }
}

/// Pre-format the compact info bar text shown above each player's zones.
/// Matches `FancyTuiRenderer::draw_player_info`.
fn build_info_bar_text(label: &str, life: i32, hand: usize, gy: usize, lib: usize) -> String {
    format!("{} | {} life | Hand: {} | GY: {} | Lib: {}", label, life, hand, gy, lib)
}

/// Build battlefield sections for the given owner.
///
/// Card buckets follow `categorize_card`. Section ORDER depends on whose
/// battlefield this is:
/// - Local player ("us"): PWs → Creatures → Enchants → Artifacts → Lands
/// - Opponent: reversed (Lands closest to us, PWs at the back).
fn build_battlefield_sections(
    game: &GameState,
    owner_id: PlayerId,
    perspective_player_id: PlayerId,
    valid_choices: &[CardId],
    selected_card_id: Option<CardId>,
) -> Vec<BattlefieldSection> {
    use std::collections::HashMap;

    // Bucket by category. CardCategory is `Hash`, so HashMap is fine; we
    // re-sort each bucket by CardId below for deterministic output.
    let mut buckets: HashMap<CardCategory, Vec<CardId>> = HashMap::new();
    for &cid in &game.battlefield.cards {
        let Some(card) = game.cards.try_get(cid) else { continue };
        if card.controller != owner_id {
            continue;
        }
        let cat = categorize_card(card);
        buckets.entry(cat).or_default().push(cid);
    }

    let is_player_perspective = owner_id == perspective_player_id;

    let order: &[CardCategory] = if is_player_perspective {
        &[
            CardCategory::Planeswalker,
            CardCategory::Creature,
            CardCategory::Enchantment,
            CardCategory::Artifact,
            CardCategory::Land,
            CardCategory::Other,
        ]
    } else {
        &[
            CardCategory::Land,
            CardCategory::Artifact,
            CardCategory::Enchantment,
            CardCategory::Creature,
            CardCategory::Planeswalker,
            CardCategory::Other,
        ]
    };

    order
        .iter()
        .filter_map(|cat| {
            let mut cards = buckets.remove(cat)?;
            // Sort by stable id for deterministic rendering.
            cards.sort_by_key(|c| c.as_u32());
            let card_views: Vec<CardView> = cards
                .into_iter()
                .filter_map(|cid| {
                    let pt = game.cards.try_get(cid).filter(|c| c.is_creature()).map(|_| {
                        (
                            game.get_effective_power(cid).ok(),
                            game.get_effective_toughness(cid).ok(),
                        )
                    });
                    build_card_view(game, cid, valid_choices, selected_card_id, pt)
                })
                .collect();
            Some(BattlefieldSection {
                label: cat.label(),
                category: SerializedCardCategory::from(*cat),
                cards: card_views,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Top-level builder
// ---------------------------------------------------------------------------

/// Inputs the view-model builder needs that aren't on `GameState` itself.
pub struct ViewModelInputs<'a> {
    /// Whose perspective is this view from?
    pub perspective_player_id: PlayerId,
    /// Currently focused/selected card (for the details pane).
    pub selected_card_id: Option<CardId>,
    /// Cards currently valid as choices (for highlighting).
    pub valid_choices: &'a [CardId],
    /// Current prompt the human is being asked.
    pub current_prompt: Option<&'a str>,
    /// `(text, highlighted)` choice list.
    pub choices: &'a [(String, bool)],
    /// Currently selected choice index.
    pub selected_choice_idx: usize,
    /// Semantic context name for the choice (e.g. "DeclareAttackers").
    pub choice_context: &'static str,
    /// `true` if the game has ended.
    pub game_over: bool,
    /// Fatal engine/session error to surface (rewind/replay verifier +
    /// monotonicity-invariant failure from the shared session). `None` when
    /// there is no error. Threaded through so native_game.html surfaces the
    /// SAME assertion failures the terminal renderer shows in tui_game.html
    /// (mtg-436).
    pub error_message: Option<&'a str>,
    /// How many of the most recent log entries to include (oldest preserved).
    pub log_tail_size: usize,
}

/// Schema version. Bump on breaking changes to the JSON shape.
///
/// v2 (mtg-436): added the top-level `error_message` field so native_game.html
/// can surface the rewind/replay verifier failures the terminal renderer shows.
/// Additive + optional, so older JS that ignores it keeps working.
pub const VIEW_MODEL_SCHEMA_VERSION: u32 = 2;

/// Build the GUI view model from a game state and UI inputs.
///
/// This is the SINGLE source of truth for what `native_game.html` sees.
pub fn build_view_model(game: &GameState, inputs: ViewModelInputs<'_>) -> GuiViewModel {
    let perspective = inputs.perspective_player_id;

    let our_idx = game.players.iter().position(|p| p.id == perspective).unwrap_or(0);
    let active_idx = game
        .players
        .iter()
        .position(|p| p.id == game.turn.active_player)
        .unwrap_or(0);

    // Build per-player views.
    let players: Vec<PlayerView> = game
        .players
        .iter()
        .enumerate()
        .map(|(idx, player)| {
            let pid = player.id;
            let pview = GameStateView::new(game, pid);
            let (w, u, b, r, g, c) = pview.available_mana();

            let is_us = pid == perspective;
            let is_active = pid == game.turn.active_player;
            // Use the player's stable seat (P1 = the perspective player; P2 = others).
            // This matches `draw_player_info`'s convention.
            let seat_badge: &'static str = if pid == perspective { "P1" } else { "P2" };
            let name = player.name.to_string();
            let label = build_player_label(&name, seat_badge);

            let life = player.life;
            let library_size = pview.player_library_size(pid);
            let hand_size = pview.player_hand_size(pid);
            let graveyard_size = pview.player_graveyard_size(pid);
            let info_bar_text = build_info_bar_text(&label, life, hand_size, graveyard_size, library_size);

            // Hand: only populated for the local player. Sorted via shared helper.
            let hand: Vec<CardView> = if is_us {
                let view_for_perspective = GameStateView::new(game, perspective);
                let sorted = FancyTuiRenderer::get_sorted_hand(&view_for_perspective);
                sorted
                    .into_iter()
                    .filter_map(|cid| build_card_view(game, cid, inputs.valid_choices, inputs.selected_card_id, None))
                    .collect()
            } else {
                Vec::new()
            };

            let battlefield_sections =
                build_battlefield_sections(game, pid, perspective, inputs.valid_choices, inputs.selected_card_id);

            let graveyard: Vec<CardView> = pview
                .graveyard()
                .iter()
                .filter_map(|&cid| build_card_view(game, cid, inputs.valid_choices, inputs.selected_card_id, None))
                .collect();

            let command_zone: Vec<CardView> = pview
                .player_command_zone(pid)
                .iter()
                .filter_map(|&cid| build_card_view(game, cid, inputs.valid_choices, inputs.selected_card_id, None))
                .collect();

            PlayerView {
                player_id: pid.as_u32(),
                index: idx,
                name,
                label,
                seat_badge,
                life,
                is_active,
                is_us,
                library_size,
                hand_size,
                graveyard_size,
                info_bar_text,
                mana_pool: ManaPoolView {
                    white: w,
                    blue: u,
                    black: b,
                    red: r,
                    green: g,
                    colorless: c,
                },
                hand,
                battlefield_sections,
                graveyard,
                command_zone,
            }
        })
        .collect();

    // Stack — top-of-stack is the LAST element of `cards`.
    let stack: Vec<StackEntryView> = game
        .stack
        .cards
        .iter()
        .filter_map(|&cid| {
            let card = game.cards.try_get(cid)?;
            let controller_idx = game.players.iter().position(|p| p.id == card.controller).unwrap_or(0);
            Some(StackEntryView {
                card_id: cid.as_u32(),
                name: card.name.to_string(),
                controller_idx,
            })
        })
        .collect();

    // Logs (last `log_tail_size`, oldest first).
    //
    // Apply per-perspective filtering: entries marked `private_to` (e.g.
    // per-card draw lines) reveal hidden info to their owner only. From any
    // other perspective we display the masked `public_message` instead.
    // See `LogEntry::message_for` and bug-draw-reveals-opponent-hand.
    let logs: Vec<LogEntryView> = game
        .logger
        .logs()
        .iter()
        .rev()
        .take(inputs.log_tail_size)
        .rev()
        .map(|entry| {
            let text = entry.message_for(perspective);
            let class = classify_log_message(text);
            let (color, semantic_class) = css_color_for_log_class(class);
            LogEntryView {
                text: text.to_string(),
                color,
                bold: class.is_bold(),
                is_choice: class == LogClass::Choice,
                semantic_class,
            }
        })
        .collect();

    // Choices.
    //
    // Dedup textually-identical actions (mtg-723): when a player holds two
    // copies of the same card, the engine offers one legal action per copy
    // (e.g. two "cast Demonic Tutor (sacrificing Black Lotus)" entries). They
    // are indistinguishable to the player, so we collapse them to a single
    // visible entry. We keep the FIRST occurrence's real engine `index` (so
    // selecting it still resolves to a valid choice — either copy produces the
    // same outcome) and renumber `display_number` 1..N over the deduped list.
    let mut seen_texts: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let choices: Vec<ChoiceView> = inputs
        .choices
        .iter()
        .enumerate()
        .filter(|(_, (text, _))| seen_texts.insert(text.as_str()))
        .enumerate()
        .map(|(display_idx, (engine_idx, (text, highlighted)))| ChoiceView {
            index: engine_idx,
            display_number: display_idx + 1,
            text: text.clone(),
            highlighted: *highlighted,
        })
        .collect();

    // Selected card details.
    let selected_card = inputs.selected_card_id.and_then(|cid| {
        let view = GameStateView::new(game, perspective);
        build_selected_card_detail(&view, cid)
    });

    // Status bar text (matches the legacy renderer for now).
    let current_step = format!("{:?}", game.turn.current_step);
    let current_step_abbrev = FancyTuiRenderer::step_abbrev(game.turn.current_step);
    let active_player_label = if active_idx == our_idx { "P1" } else { "P2" };
    let status_text = if inputs.game_over {
        format!(
            "Turn {} | Phase: {} | Active: {} | GAME OVER",
            game.turn.turn_number, current_step, active_player_label,
        )
    } else {
        format!(
            "Turn {} | Phase: {} | Active: {}",
            game.turn.turn_number, current_step, active_player_label,
        )
    };

    GuiViewModel {
        schema_version: VIEW_MODEL_SCHEMA_VERSION,
        turn_number: game.turn.turn_number,
        current_step,
        current_step_abbrev,
        active_player_idx: active_idx,
        our_player_idx: our_idx,
        game_over: inputs.game_over,
        error_message: inputs.error_message.map(|s| s.to_string()),
        players,
        stack,
        status_text,
        current_prompt: inputs.current_prompt.map(|s| s.to_string()),
        choices,
        selected_choice_idx: inputs.selected_choice_idx,
        choice_context: inputs.choice_context,
        logs,
        selected_card,
    }
}

/// Convenience: render the view model to JSON (defaults to `"{}"` on serialization
/// failure so callers don't need to handle errors explicitly).
pub fn build_view_model_json(game: &GameState, inputs: ViewModelInputs<'_>) -> String {
    let model = build_view_model(game, inputs);
    serde_json::to_string(&model).unwrap_or_else(|_| "{}".to_string())
}

/// Map a `ChoiceContext` to a stable string for JS consumers.
pub fn choice_context_name(ctx: crate::game::fancy_tui_renderer::ChoiceContext) -> &'static str {
    use crate::game::fancy_tui_renderer::ChoiceContext as C;
    match ctx {
        C::PlayingSpell => "PlayingSpell",
        C::DeclareAttackers => "DeclareAttackers",
        C::DeclareBlockers => "DeclareBlockers",
        C::TargetSelection => "TargetSelection",
        C::None => "None",
    }
}

/// Map the controller-layer `ChoiceContext` to a stable string for the GUI.
/// Different from `fancy_tui_renderer::ChoiceContext` even though it serves
/// a similar role.
pub fn pending_choice_context_name(ctx: Option<&crate::game::controller::ChoiceContext>) -> &'static str {
    use crate::game::controller::ChoiceContext as C;
    let Some(ctx) = ctx else { return "None" };
    match ctx {
        C::SpellAbility { .. } => "SpellAbility",
        C::Targets { .. } => "Targets",
        C::ManaSources { .. } => "ManaSources",
        C::Attackers { .. } => "Attackers",
        C::Blockers { .. } => "Blockers",
        C::DamageOrder { .. } => "DamageOrder",
        C::Discard { .. } => "Discard",
        C::LibrarySearch { .. } => "LibrarySearch",
        C::SacrificePermanents { .. } => "SacrificePermanents",
        C::ScryOrder { .. } => "ScryOrder",
        C::Surveil { .. } => "Surveil",
        C::Modes { .. } => "Modes",
    }
}

/// Convenience for callers that don't want to import `Step`.
pub fn step_abbrev(step: Step) -> &'static str {
    FancyTuiRenderer::step_abbrev(step)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_class_colors_match_documented_palette() {
        // Spot-check a few classes we promise to colorize identically to the TUI.
        assert_eq!(css_color_for_log_class(LogClass::TurnHeader).0, "#ffd700");
        assert_eq!(css_color_for_log_class(LogClass::Combat).0, "#ff79c6");
        assert_eq!(css_color_for_log_class(LogClass::Damage).0, "#ff5555");
        assert_eq!(css_color_for_log_class(LogClass::Default).0, "#ccc");
    }

    #[test]
    fn classify_log_message_recognizes_turn_headers() {
        assert_eq!(classify_log_message(">>> Turn 3 — Player1"), LogClass::TurnHeader);
        assert_eq!(classify_log_message("<<<< End of Turn"), LogClass::TurnHeader);
        assert!(LogClass::TurnHeader.is_bold());
    }

    #[test]
    fn classify_log_message_recognizes_combat() {
        assert_eq!(classify_log_message("Goblin attacks Player2"), LogClass::Combat);
        assert_eq!(classify_log_message("Wall blocks Goblin"), LogClass::Combat);
    }

    #[test]
    fn classify_log_message_recognizes_damage() {
        let msg = "Lightning Bolt deals 3 damage to Player2 (life: 17)";
        assert_eq!(classify_log_message(msg), LogClass::Damage);
        assert!(LogClass::Damage.is_bold());
    }

    #[test]
    fn category_label_is_stable() {
        assert_eq!(CardCategory::Creature.label(), "Creatures");
        assert_eq!(CardCategory::Land.label(), "Lands");
        assert_eq!(CardCategory::Planeswalker.label(), "PWs");
    }

    #[test]
    fn schema_version_is_current() {
        // v2 (mtg-436): added the optional `error_message` field.
        assert_eq!(VIEW_MODEL_SCHEMA_VERSION, 2);
    }

    /// End-to-end: build the view model from a minimal real `GameState` and
    /// verify the top-level shape, status text, and player labelling. This
    /// catches regressions where field renames or category enum changes break
    /// the JSON shape that `native_game.html` consumes.
    #[test]
    fn build_view_model_on_minimal_game() {
        use crate::game::GameState;

        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let perspective = game.players[0].id;

        let inputs = ViewModelInputs {
            perspective_player_id: perspective,
            selected_card_id: None,
            valid_choices: &[],
            current_prompt: Some("Press Space to advance"),
            choices: &[],
            selected_choice_idx: 0,
            choice_context: "None",
            game_over: false,
            error_message: None,
            log_tail_size: 100,
        };

        let model = build_view_model(&game, inputs);

        // Schema and turn metadata.
        assert_eq!(model.schema_version, VIEW_MODEL_SCHEMA_VERSION);
        assert_eq!(model.turn_number, game.turn.turn_number);
        assert_eq!(model.our_player_idx, 0);
        assert!(!model.game_over);
        assert_eq!(model.choice_context, "None");
        assert_eq!(model.current_prompt.as_deref(), Some("Press Space to advance"));

        // Two players with stable seat badges.
        assert_eq!(model.players.len(), 2);
        assert_eq!(model.players[0].seat_badge, "P1");
        assert_eq!(model.players[1].seat_badge, "P2");
        assert!(model.players[0].is_us);
        assert!(!model.players[1].is_us);

        // Info bar matches the TUI's compact format.
        let p0 = &model.players[0];
        assert!(
            p0.info_bar_text.starts_with(&format!("{} | 20 life | Hand:", p0.label)),
            "got info bar: {}",
            p0.info_bar_text,
        );

        // Status text contains the canonical "Turn N | Phase: …" prefix.
        assert!(model.status_text.starts_with("Turn "), "{}", model.status_text);
        assert!(model.status_text.contains(" | Phase: "), "{}", model.status_text);

        // JSON serialization must succeed and produce a non-empty object.
        let json = serde_json::to_string(&model).expect("serialize view model");
        assert!(json.contains(&format!("\"schema_version\":{VIEW_MODEL_SCHEMA_VERSION}")));
        assert!(json.contains("\"players\""));
        assert!(json.contains("\"battlefield_sections\""));
        // No error by default; the field is present (null) so the JS can read it.
        assert!(model.error_message.is_none());
        assert!(json.contains("\"error_message\":null"));
    }

    /// mtg-436: the rewind/replay verifier + monotonicity-invariant failure
    /// message set on the shared session MUST be threaded through the view model
    /// so native_game.html surfaces the SAME fatal assertion the terminal
    /// renderer shows in tui_game.html. Before this fix the view model never
    /// carried `error_message`, so a fatal rewind/replay divergence that halts
    /// tui_game passed silently in native_game.
    #[test]
    fn view_model_surfaces_session_error_message() {
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let perspective = game.players[0].id;

        let err = "REWIND/REPLAY DIVERGENCE: action count went backwards (42 -> 40)";
        let inputs = ViewModelInputs {
            perspective_player_id: perspective,
            selected_card_id: None,
            valid_choices: &[],
            current_prompt: None,
            choices: &[],
            selected_choice_idx: 0,
            choice_context: "None",
            game_over: true,
            error_message: Some(err),
            log_tail_size: 100,
        };

        let model = build_view_model(&game, inputs);
        assert_eq!(model.error_message.as_deref(), Some(err));

        // It must round-trip through JSON so the JS view-model reader sees it.
        let json = build_view_model_json(
            &game,
            ViewModelInputs {
                perspective_player_id: perspective,
                selected_card_id: None,
                valid_choices: &[],
                current_prompt: None,
                choices: &[],
                selected_choice_idx: 0,
                choice_context: "None",
                game_over: true,
                error_message: Some(err),
                log_tail_size: 100,
            },
        );
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse view model json");
        assert_eq!(parsed["error_message"], serde_json::Value::String(err.to_string()));
        assert_eq!(parsed["game_over"], serde_json::Value::Bool(true));
    }

    /// Verify that the JSON output uses raw `u32` for card/player IDs (so
    /// `native_game.html` can use them as DOM keys) rather than Rust's `Debug`
    /// format like `"CardId(35)"`.
    #[test]
    fn ids_serialize_as_raw_u32() {
        use crate::game::GameState;

        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let perspective = game.players[0].id;

        let inputs = ViewModelInputs {
            perspective_player_id: perspective,
            selected_card_id: None,
            valid_choices: &[],
            current_prompt: None,
            choices: &[],
            selected_choice_idx: 0,
            choice_context: "None",
            game_over: false,
            error_message: None,
            log_tail_size: 0,
        };
        let json = build_view_model_json(&game, inputs);

        // PlayerId should be a number, not the Debug-printed form.
        assert!(json.contains("\"player_id\":"));
        assert!(
            !json.contains("EntityId"),
            "raw debug-printed ID leaked into JSON: {}",
            json,
        );
    }

    /// Regression for bug-draw-reveals-opponent-hand:
    /// the GUI view model — the JSON shape consumed by
    /// `web/native_game.html` and `web/tui_game.html` — must hide opponent
    /// per-card draw lines, replacing them with the masked
    /// "P draws a card" form.
    ///
    /// Before the fix, every draw line was a plain
    /// `gamelog("P2 draws Disenchant (88)")` and the WASM
    /// exporter served the raw message, leaking P2's hand to P1.
    #[test]
    fn opponent_draws_are_masked_in_view_model_logs() {
        use crate::core::Card;
        use crate::game::GameState;

        let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;
        game.logger.set_output_mode(crate::game::logger::OutputMode::Memory);

        // Seed P2's library with a recognisable card and have P2 draw it.
        let secret_id = game.next_card_id();
        let secret = Card::new(secret_id, "Disenchant".to_string(), p2_id);
        game.cards.insert(secret_id, secret);
        game.get_player_zones_mut(p2_id).unwrap().library.add(secret_id);
        game.draw_card(p2_id).expect("p2 draw");

        // Render the view model from P1's perspective.
        let inputs_p1 = ViewModelInputs {
            perspective_player_id: p1_id,
            selected_card_id: None,
            valid_choices: &[],
            current_prompt: None,
            choices: &[],
            selected_choice_idx: 0,
            choice_context: "None",
            game_over: false,
            error_message: None,
            log_tail_size: 50,
        };
        let json_p1 = build_view_model_json(&game, inputs_p1);

        assert!(
            !json_p1.contains("Disenchant"),
            "P1's view model JSON must NOT contain P2's drawn card name; got {}",
            json_p1,
        );
        assert!(
            json_p1.contains("draws a card"),
            "P1's view model JSON should contain the masked 'draws a card' line; got {}",
            json_p1,
        );

        // From P2's own perspective, the full draw line is preserved.
        let inputs_p2 = ViewModelInputs {
            perspective_player_id: p2_id,
            selected_card_id: None,
            valid_choices: &[],
            current_prompt: None,
            choices: &[],
            selected_choice_idx: 0,
            choice_context: "None",
            game_over: false,
            error_message: None,
            log_tail_size: 50,
        };
        let json_p2 = build_view_model_json(&game, inputs_p2);
        assert!(
            json_p2.contains("Disenchant"),
            "P2's own view model JSON should still show the full draw line; got {}",
            json_p2,
        );
    }

    /// Validate the GUI choice-context name mapping.
    #[test]
    fn pending_choice_context_name_maps_known_variants() {
        use crate::game::controller::ChoiceContext as C;
        // A few representative variants — the function must return a stable string.
        let attackers = C::Attackers {
            available_creatures: Vec::new(),
            formatted_creatures: Vec::new(),
        };
        assert_eq!(pending_choice_context_name(Some(&attackers)), "Attackers");
        assert_eq!(pending_choice_context_name(None), "None");
    }

    /// mtg-723: textually-identical actions (e.g. holding two copies of the
    /// same card → two identical "cast X" entries) must collapse to a single
    /// visible choice. The kept entry must retain a REAL engine index (the
    /// first occurrence) and the deduped list must be renumbered 1..N.
    #[test]
    fn build_view_model_dedups_identical_choices() {
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let perspective = game.players[0].id;

        // Engine offers two identical Demonic Tutor casts (one per copy held)
        // plus a distinct "Pass" action.
        let dup = "cast Demonic Tutor (sacrificing Black Lotus)".to_string();
        let choices = vec![(dup.clone(), false), (dup.clone(), true), ("Pass".to_string(), false)];

        let inputs = ViewModelInputs {
            perspective_player_id: perspective,
            selected_card_id: None,
            valid_choices: &[],
            current_prompt: Some("Choose an action"),
            choices: &choices,
            selected_choice_idx: 0,
            choice_context: "None",
            game_over: false,
            error_message: None,
            log_tail_size: 100,
        };

        let model = build_view_model(&game, inputs);

        // Three raw choices collapse to two distinct entries.
        assert_eq!(model.choices.len(), 2, "duplicate actions must collapse");
        assert_eq!(model.choices[0].text, dup);
        assert_eq!(model.choices[1].text, "Pass");

        // Kept entry retains the FIRST occurrence's real engine index (0); the
        // distinct "Pass" keeps its original engine index (2) so selection
        // still resolves to a valid choice.
        assert_eq!(model.choices[0].index, 0);
        assert_eq!(model.choices[1].index, 2);

        // Display numbers are renumbered 1..N over the visible list.
        assert_eq!(model.choices[0].display_number, 1);
        assert_eq!(model.choices[1].display_number, 2);
    }
}
