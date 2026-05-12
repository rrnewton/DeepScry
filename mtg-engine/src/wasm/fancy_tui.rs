//! WASM Fancy TUI - RatZilla-based TUI rendering for browser
//!
//! This module provides the fancy TUI experience in the browser using RatZilla.
//! It uses the shared FancyTuiRenderer for consistent rendering between native and WASM.
//!
//! ## Architecture
//!
//! - Uses RatZilla's DomBackend for fast DOM-based terminal rendering
//! - Uses FancyTuiRenderer (shared with native) for all TUI drawing
//! - Game state is managed via Rc<RefCell<>> for the render callback
//! - Human input uses the interrupt pattern via run_until_input()

use crate::core::PlayerId;
use crate::game::controller::{
    prompt_discard, prompt_spell_ability, prompt_target, ChoiceContext, GameStateView, PROMPT_ATTACKERS,
    PROMPT_BLOCKERS, PROMPT_DAMAGE_ORDER, PROMPT_LIBRARY_SEARCH,
};
use crate::game::fancy_tui_events::{
    handle_key_event, handle_mouse_click, handle_ui_event, EventResult, KeyInput, ScrollDirection, UiEvent,
};
use crate::game::logger::OutputMode;
use crate::game::PlayerController;
use crate::game::{FancyTuiRenderer, GameLoop, GameLoopState, GameState, VerbosityLevel};
#[cfg(feature = "wasm-network")]
use crate::game::{HeuristicController, RandomController, ZeroController};
use crate::loader::CardDefinition;
use ratzilla::event::{KeyCode, MouseButton, MouseEventKind};
use ratzilla::ratatui::{Frame, Terminal};
use ratzilla::{DomBackend, WebRenderer};

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use wasm_bindgen::prelude::*;

use super::human_controller::{stage_discard_pick, PendingChoice, WasmHumanController};
use super::rich_input_controller::WasmRichInputController;
use super::{WasmCardDatabase, WasmControllerType};
use crate::game::replay_controller::ReplayChoice;
use crate::game::ReplayController;
use crate::undo::GameAction;

// Network controller imports (conditional on wasm-network feature)
#[cfg(feature = "wasm-network")]
use super::network::{ensure_client, SharedNetworkClient, WasmNetworkLocalController, WasmRemoteController};

// Thread-local storage for the global TUI state (for button callbacks)
thread_local! {
    static GLOBAL_TUI_STATE: RefCell<Option<Rc<RefCell<WasmFancyTuiState>>>> = const { RefCell::new(None) };
    // Thread-local storage for the fixed script (set before launching TUI)
    static FIXED_SCRIPT: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
    // Thread-local storage for measured cell dimensions from JavaScript
    // Default to RatZilla's magic numbers (10x20 pixels per cell)
    static CELL_DIMENSIONS: RefCell<(f32, f32)> = const { RefCell::new((10.0, 20.0)) };
}

/// Set the fixed script for player 1's Fixed controller
///
/// Call this before launch_fancy_tui() when using WasmControllerType::Fixed.
/// The script is a list of commands, one per line. Commands include:
/// - `play <land_name>` - Play a land
/// - `cast <spell_name>` - Cast a spell
/// - `activate <card_name>` - Activate an ability
/// - `pass` or `p` or `0` - Pass priority
/// - `*` - Enter wildcard mode (pass until next command matches)
/// - `1`, `2`, etc. - Select by menu index
#[wasm_bindgen]
pub fn set_p1_fixed_script(script: &str) {
    let commands: Vec<String> = script
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    FIXED_SCRIPT.with(|s| {
        *s.borrow_mut() = Some(commands);
    });
}

/// Clear the fixed script for player 1
#[wasm_bindgen]
pub fn clear_p1_fixed_script() {
    FIXED_SCRIPT.with(|s| {
        *s.borrow_mut() = None;
    });
}

/// Clean up global state when exiting the TUI
///
/// Call this before exiting to ensure a clean slate for the next launch.
/// This clears the GLOBAL_TUI_STATE and FIXED_SCRIPT thread-local storage.
#[wasm_bindgen]
pub fn cleanup_tui_state() {
    GLOBAL_TUI_STATE.with(|s| {
        *s.borrow_mut() = None;
    });
    FIXED_SCRIPT.with(|s| {
        *s.borrow_mut() = None;
    });
    log::debug!(target: "wasm_tui", "Cleaned up global TUI state");
}

/// Set cell dimensions measured by JavaScript
///
/// Call this after measuring the browser's font metrics to get accurate
/// pixel dimensions for layout calculations. The values are used by
/// `tui_get_card_positions()` to calculate `layout_width_px` and `layout_height_px`.
///
/// This also updates the renderer's cell dimensions for layout calculations.
///
/// # Arguments
/// * `width_px` - Cell width in pixels (typically 10.0 for RatZilla)
/// * `height_px` - Cell height in pixels (typically 20.0 for RatZilla)
#[wasm_bindgen]
pub fn tui_set_cell_dimensions(width_px: f32, height_px: f32) {
    CELL_DIMENSIONS.with(|dims| {
        *dims.borrow_mut() = (width_px, height_px);
    });

    // Also update the renderer's cell dimensions
    GLOBAL_TUI_STATE.with(|state| {
        if let Some(ref state) = *state.borrow() {
            let mut s = state.borrow_mut();
            s.renderer.set_cell_dimensions(width_px, height_px);
        }
    });

    log::debug!(target: "wasm_tui", "Cell dimensions set: {}x{} px", width_px, height_px);
}

/// Handle a scroll wheel event from the browser.
///
/// Called from JavaScript's `wheel` event listener. Converts pixel coordinates
/// to cell coordinates using measured dimensions and dispatches through the
/// shared scroll handler.
///
/// # Arguments
/// * `x` - Pixel X coordinate of the mouse
/// * `y` - Pixel Y coordinate of the mouse
/// * `delta_y` - Vertical scroll delta (positive = scroll down, negative = scroll up)
/// * `delta_x` - Horizontal scroll delta (positive = scroll right, negative = scroll left)
#[wasm_bindgen]
pub fn tui_scroll_wheel(x: u32, y: u32, delta_y: f64, delta_x: f64) {
    GLOBAL_TUI_STATE.with(|state| {
        if let Some(ref state) = *state.borrow() {
            let mut s = state.borrow_mut();
            let (col, row) = pixels_to_cells(x, y);

            // Determine primary scroll direction
            let direction = if delta_y.abs() >= delta_x.abs() {
                if delta_y < 0.0 {
                    ScrollDirection::Up
                } else {
                    ScrollDirection::Down
                }
            } else if delta_x < 0.0 {
                ScrollDirection::Left
            } else {
                ScrollDirection::Right
            };

            let event = UiEvent::MouseWheel { direction, col, row };

            let WasmFancyTuiState {
                ref game,
                ref mut renderer,
                ref current_choices,
                ..
            } = *s;

            let view = GameStateView::new(game, renderer.player_id);
            let num_choices = current_choices.len();
            let result = handle_ui_event(&mut renderer.state, event, &view, num_choices);

            if result == EventResult::Handled {
                s.needs_redraw = true;
            }
        }
    });
}

/// Get the help text from the shared Rust source.
///
/// Returns the canonical help text with browser-specific shortcuts included.
/// This replaces the JavaScript `getHelpText()` function that previously
/// maintained a separate copy of the help text.
#[wasm_bindgen]
pub fn tui_get_help_text() -> String {
    crate::game::fancy_tui_events::get_help_text(true)
}

/// Get image URLs for a card, in priority order (local, scryfall, gatherer).
///
/// Returns a JSON array of URL strings. JS should try them in order,
/// using `<img onerror>` to fall through to the next source.
///
/// # Arguments
/// * `card_name` - The card name
/// * `height_px` - Desired height in pixels (selects small vs normal size)
#[wasm_bindgen]
pub fn tui_get_image_urls(card_name: &str, height_px: u32) -> String {
    use crate::wasm::image_overlay::{gatherer_url, local_image_url, scryfall_url_by_name, ImageVersion};

    let version = if height_px <= 204 {
        ImageVersion::Small
    } else {
        ImageVersion::Normal
    };

    let urls = vec![
        local_image_url(card_name, version, "/images"),
        scryfall_url_by_name(card_name, version),
        gatherer_url(card_name),
    ];

    serde_json::json!(urls).to_string()
}

/// Get card layout positions for the current game state.
///
/// Computes battlefield card positions using the shared layout engine,
/// returning JSON that game.html can use to position card DOM elements.
///
/// Returns JSON: `{ "your_battlefield": [...], "opp_battlefield": [...] }`
/// where each entry has `{ id, name, x_pct, y_pct, w_pct, h_pct, is_tapped, is_creature, is_land }`.
/// Percentages are relative to the pane's inner area.
#[wasm_bindgen]
pub fn tui_get_card_layout_json() -> String {
    use crate::game::layout::{
        compute_battlefield_card_size, layout_cards_wordwrap, CardCategory, CardItem, CardSizeConfig,
    };

    GLOBAL_TUI_STATE.with(|state| {
        if let Some(ref state) = *state.borrow() {
            let s = state.borrow();
            let game = &s.game;
            let our_pid = s.renderer.player_id;

            let config = CardSizeConfig::default();

            let mut result = serde_json::Map::new();

            // Build list of (label, player_id) pairs for each battlefield
            let mut bf_pairs: Vec<(&str, crate::core::PlayerId)> = vec![("your_battlefield", our_pid)];
            for player in &game.players {
                if player.id != our_pid {
                    bf_pairs.push(("opp_battlefield", player.id));
                }
            }

            for (label, pid) in &bf_pairs {
                let bf_cards: Vec<CardItem> = game
                    .battlefield
                    .cards
                    .iter()
                    .filter_map(|&cid| {
                        let card = game.cards.try_get(cid)?;
                        if card.controller != *pid {
                            return None;
                        }

                        let category = if card.is_planeswalker() {
                            CardCategory::Planeswalker
                        } else if card.is_creature() {
                            CardCategory::Creature
                        } else if card.is_enchantment() {
                            CardCategory::Enchantment
                        } else if card.is_artifact() {
                            CardCategory::Artifact
                        } else {
                            CardCategory::Land
                        };

                        Some(CardItem {
                            id: cid.as_u32(),
                            name: card.name.to_string(),
                            is_tapped: card.tapped,
                            category,
                            stack_size: 1,
                        })
                    })
                    .collect();

                // Use a representative area (100x50 cells) to compute positions as percentages
                let area = ratatui::layout::Rect::new(0, 0, 100, 50);
                let (card_w, card_h) = compute_battlefield_card_size(area, bf_cards.len(), &config);
                let placements = layout_cards_wordwrap(area, &bf_cards, card_w, card_h, &config);

                let cards_json: Vec<serde_json::Value> = placements
                    .iter()
                    .zip(bf_cards.iter())
                    .map(|(p, card)| {
                        serde_json::json!({
                            "id": p.id,
                            "name": card.name,
                            "x_pct": f64::from(p.x),
                            "y_pct": f64::from(p.y),
                            "w_pct": f64::from(p.width),
                            "h_pct": f64::from(p.height),
                            "is_tapped": card.is_tapped,
                            "is_creature": card.category == CardCategory::Creature,
                            "is_land": card.category == CardCategory::Land,
                        })
                    })
                    .collect();

                result.insert(label.to_string(), serde_json::json!(cards_json));
            }

            serde_json::json!(result).to_string()
        } else {
            "{}".to_string()
        }
    })
}

/// Compute the shared pane layout for a given viewport size.
///
/// Returns JSON with pane positions as percentages of the viewport,
/// matching the same layout the TUI renderer uses. This allows
/// game.html to position HTML elements using the shared layout engine
/// instead of hardcoding its own CSS grid proportions.
///
/// Returns JSON: `{ "columns": [left_pct, middle_pct, right_pct], "info_bar_rows": N }`
/// where percentages match the shared `compute_pane_layout()` ratios.
#[wasm_bindgen]
pub fn tui_get_layout_json(viewport_width: u16, viewport_height: u16) -> String {
    use crate::game::fancy_tui_renderer::FancyTuiRenderer;
    use crate::game::layout::{compute_pane_layout, PaneId, PaneLayoutConfig};

    let config = PaneLayoutConfig {
        left_column_pct: FancyTuiRenderer::DEFAULT_LEFT_COLUMN_PCT,
        middle_column_pct: FancyTuiRenderer::DEFAULT_MIDDLE_COLUMN_PCT,
        right_column_pct: FancyTuiRenderer::DEFAULT_RIGHT_COLUMN_PCT,
        boosted_left_column_pct: FancyTuiRenderer::BOOSTED_LEFT_COLUMN_PCT,
        min_width_log: FancyTuiRenderer::MIN_WIDTH_LOG_PANE,
        min_width_actions: FancyTuiRenderer::MIN_WIDTH_ACTIONS_PANE,
        min_width_battlefield: FancyTuiRenderer::MIN_WIDTH_BATTLEFIELD,
        min_width_card_details: FancyTuiRenderer::MIN_WIDTH_CARD_DETAILS,
        min_width_hand: FancyTuiRenderer::MIN_WIDTH_HAND,
    };

    let viewport = ratatui::layout::Rect::new(0, 0, viewport_width, viewport_height);
    let layout = compute_pane_layout(viewport, &config);

    // Convert pane rects to JSON
    let pane_ids = [
        ("log", PaneId::Log),
        ("actions", PaneId::Actions),
        ("opponent_info", PaneId::OpponentInfo),
        ("opponent_battlefield", PaneId::OpponentBattlefield),
        ("your_battlefield", PaneId::YourBattlefield),
        ("your_info", PaneId::YourInfo),
        ("card_details", PaneId::CardDetails),
        ("hand", PaneId::Hand),
    ];

    let mut panes = serde_json::Map::new();
    for (name, id) in &pane_ids {
        if let Some(rect) = layout.get(*id) {
            panes.insert(
                (*name).to_string(),
                serde_json::json!({
                    "x": rect.x,
                    "y": rect.y,
                    "width": rect.width,
                    "height": rect.height,
                    // Percentages for CSS positioning
                    "x_pct": if viewport_width > 0 { f64::from(rect.x) / f64::from(viewport_width) * 100.0 } else { 0.0 },
                    "y_pct": if viewport_height > 0 { f64::from(rect.y) / f64::from(viewport_height) * 100.0 } else { 0.0 },
                    "w_pct": if viewport_width > 0 { f64::from(rect.width) / f64::from(viewport_width) * 100.0 } else { 0.0 },
                    "h_pct": if viewport_height > 0 { f64::from(rect.height) / f64::from(viewport_height) * 100.0 } else { 0.0 },
                }),
            );
        }
    }

    serde_json::json!({
        "viewport_width": viewport_width,
        "viewport_height": viewport_height,
        "panes": panes,
    })
    .to_string()
}

/// Run one turn or continue game - called from JavaScript button
#[wasm_bindgen]
pub fn tui_run_turn() {
    GLOBAL_TUI_STATE.with(|state| {
        if let Some(ref state) = *state.borrow() {
            let mut s = state.borrow_mut();
            s.run_until_choice();
            s.needs_redraw = true; // State changed, need redraw
        }
    });
}

/// Select current choice - called from JavaScript or keyboard Enter
#[wasm_bindgen]
pub fn tui_select_choice() {
    GLOBAL_TUI_STATE.with(|state| {
        if let Some(ref state) = *state.borrow() {
            let mut s = state.borrow_mut();
            s.select_current_choice();
            s.needs_redraw = true; // State changed, need redraw
        }
    });
}

/// Move to previous choice in the list
#[wasm_bindgen]
pub fn tui_prev_choice() {
    GLOBAL_TUI_STATE.with(|state| {
        if let Some(ref state) = *state.borrow() {
            let mut s = state.borrow_mut();
            s.select_previous_choice();
            // needs_redraw already set by select_previous_choice()
        }
    });
}

/// Move to next choice in the list
#[wasm_bindgen]
pub fn tui_next_choice() {
    GLOBAL_TUI_STATE.with(|state| {
        if let Some(ref state) = *state.borrow() {
            let mut s = state.borrow_mut();
            s.select_next_choice();
            // needs_redraw already set by select_next_choice()
        }
    });
}

/// Set choice index directly (for number key quick-select)
///
/// Sets the highlighted choice to the given index without navigating
/// through intermediate choices. Used by native GUI for 1-9 quick select.
#[wasm_bindgen]
pub fn tui_set_choice_idx(idx: usize) {
    GLOBAL_TUI_STATE.with(|state| {
        if let Some(ref state) = *state.borrow() {
            let mut s = state.borrow_mut();
            if idx < s.current_choices.len() {
                s.selected_choice_idx = idx;
                s.update_choice_highlights();
                s.needs_redraw = true;
            }
        }
    });
}

/// Toggle auto-run mode - called from JavaScript button
#[wasm_bindgen]
pub fn tui_toggle_auto() {
    GLOBAL_TUI_STATE.with(|state| {
        if let Some(ref state) = *state.borrow() {
            let mut s = state.borrow_mut();
            s.auto_run = !s.auto_run;
            s.needs_redraw = true; // UI state changed, need redraw
        }
    });
}

/// Select a card by stable id (called from the HTML GUI).
///
/// This routes the GUI's click into the same shared selection state that the
/// ratatui TUI uses for keyboard/mouse selection — `FancyTuiState`'s
/// `selected_card_id` plus the per-pane index/highlight fields. As a result,
/// the TUI's "Card Details" pane and the GUI's details pane always agree on
/// what is selected, and switching between modes mid-game is consistent.
///
/// Returns the freshly-formatted `CardDetailView` JSON for the selected card,
/// or:
/// - the literal string `"null"` when `card_id` is not visible to the
///   perspective player (e.g. opponent's hand, library — protects against
///   hidden-information leaks across the WASM boundary), or
/// - `"null"` when the global TUI state has not been initialized.
///
/// Use `tui_clear_card_selection()` to deselect.
///
/// See `mtg-engine/src/game/fancy_tui_events.rs::select_card_by_id` for the
/// shared selection routine and zone-resolution rules.
#[wasm_bindgen]
pub fn tui_select_card(card_id: u32) -> String {
    use super::gui_view_model::{selected_card_detail, selected_card_detail_json};
    use crate::core::CardId;
    use crate::game::controller::GameStateView;
    use crate::game::fancy_tui_events::{select_card_by_id, CardSelectionResult};

    GLOBAL_TUI_STATE.with(|state| {
        let Some(ref state) = *state.borrow() else {
            return "null".to_string();
        };
        let mut s_ref = state.borrow_mut();
        // Deref once to get a real `&mut WasmFancyTuiState`, which lets the
        // borrow checker split-borrow `&s.game` (immutable) and
        // `&mut s.renderer.state` (mutable) since they are disjoint fields.
        let s: &mut WasmFancyTuiState = &mut s_ref;

        let cid = CardId::new(card_id);
        let perspective = s.renderer.player_id;

        let result = {
            let view = GameStateView::new(&s.game, perspective);
            select_card_by_id(&mut s.renderer.state, cid, &view)
        };

        match result {
            CardSelectionResult::Selected { .. } => {
                s.needs_redraw = true;
                let detail = selected_card_detail(&s.game, perspective, cid);
                selected_card_detail_json(detail)
            }
            CardSelectionResult::NotFound | CardSelectionResult::NotVisible => {
                // Don't mutate state, don't surface an error to JS — return
                // null so the caller can decide what to do. The TUI logger
                // already records illegal selections via the controller path.
                "null".to_string()
            }
        }
    })
}

/// Clear any current card selection (deselect the details pane).
///
/// Updates the same shared state that the TUI uses, so both modes show an
/// empty details pane after this call.
#[wasm_bindgen]
pub fn tui_clear_card_selection() {
    use crate::game::fancy_tui_events::clear_card_selection;

    GLOBAL_TUI_STATE.with(|state| {
        if let Some(ref state) = *state.borrow() {
            let mut s = state.borrow_mut();
            clear_card_selection(&mut s.renderer.state);
            s.needs_redraw = true;
        }
    });
}

/// Get the current selected-card details JSON without changing the selection.
///
/// Returns the formatted `CardDetailView` JSON, or `"null"` if no card is
/// currently selected. Useful for the GUI to refresh the details pane after
/// a state-changing event (e.g. a creature getting +N/+N from a buff) without
/// having to roll the whole view model.
#[wasm_bindgen]
pub fn tui_get_selected_card_details() -> String {
    use super::gui_view_model::{selected_card_detail, selected_card_detail_json};

    GLOBAL_TUI_STATE.with(|state| {
        let Some(ref state) = *state.borrow() else {
            return "null".to_string();
        };
        let s = state.borrow();
        let Some(card_id) = s.renderer.state.selected_card_id else {
            return "null".to_string();
        };
        let detail = selected_card_detail(&s.game, s.renderer.player_id, card_id);
        selected_card_detail_json(detail)
    })
}

/// Get current battlefield cards as JSON for image overlay
/// Returns a JSON array of {name: string, instance_id: number}[]
#[wasm_bindgen]
pub fn tui_get_battlefield_cards() -> String {
    GLOBAL_TUI_STATE.with(|state| {
        if let Some(ref state) = *state.borrow() {
            let s = state.borrow();
            let mut cards = Vec::new();

            // Get all cards on the battlefield (shared zone)
            for &card_id in &s.game.battlefield.cards {
                if let Ok(card) = s.game.cards.get(card_id) {
                    cards.push(serde_json::json!({
                        "name": format!("{}", card.name),
                        "instance_id": format!("{:?}", card_id),
                    }));
                }
            }

            serde_json::to_string(&cards).unwrap_or_else(|_| "[]".to_string())
        } else {
            "[]".to_string()
        }
    })
}

/// Get current game logs as a JSON array of strings
#[wasm_bindgen]
pub fn tui_get_logs_json() -> String {
    GLOBAL_TUI_STATE.with(|state| {
        if let Some(ref state) = *state.borrow() {
            let s = state.borrow();
            let logs: Vec<String> = s.game.logger.logs().iter().map(|entry| entry.message.clone()).collect();
            serde_json::to_string(&logs).unwrap_or_else(|_| "[]".to_string())
        } else {
            "[]".to_string()
        }
    })
}

/// Clear current game logs
#[wasm_bindgen]
pub fn tui_clear_logs() {
    GLOBAL_TUI_STATE.with(|state| {
        if let Some(ref state) = *state.borrow() {
            let mut s = state.borrow_mut();
            s.game.logger.clear_logs();
        }
    });
}

/// Get the structured GUI view model as JSON.
///
/// This is the new entry point for `web/game.html`'s thin DOM renderer:
/// instead of dumping raw card lists and re-deriving display decisions in
/// JavaScript, we build a fully-formatted view model in Rust using the same
/// shared helpers as the ratatui TUI (`FancyTuiRenderer::get_sorted_hand`,
/// `categorize_card`, `classify_log_message`, etc.).
///
/// Schema is defined in `crate::wasm::gui_view_model::GuiViewModel`. Bump
/// `VIEW_MODEL_SCHEMA_VERSION` when changing the JSON shape so the JS side
/// can detect a stale renderer.
///
/// See `tui_get_full_state_json` for the legacy entry point that this is
/// gradually replacing.
#[wasm_bindgen]
pub fn tui_get_gui_view_model_json() -> String {
    use super::gui_view_model::{build_view_model_json, ViewModelInputs};

    GLOBAL_TUI_STATE.with(|state| {
        let Some(ref state) = *state.borrow() else {
            return "{}".to_string();
        };
        let s = state.borrow();

        // The renderer's `valid_choices` (cards selectable by the human) is
        // the right thing to highlight on the GUI as well — same source of
        // truth as the TUI hand/battlefield highlighting.
        let valid_choices: Vec<crate::core::CardId> = s.renderer.state.valid_choices.clone();

        // Map the rendering-side choice context to a stable, JS-friendly name.
        let choice_context = super::gui_view_model::choice_context_name(s.renderer.state.choice_context);

        // The pending controller-side ChoiceContext (e.g. Discard, Targets)
        // gives us a richer semantic label that the GUI can use to e.g. show
        // a "discard mode" hint. We expose the pending context name in the
        // current_prompt fallback below.
        let current_prompt = s.current_prompt.as_deref();

        let inputs = ViewModelInputs {
            perspective_player_id: s.renderer.player_id,
            selected_card_id: s.renderer.state.selected_card_id,
            valid_choices: &valid_choices,
            current_prompt,
            choices: &s.current_choices,
            selected_choice_idx: s.selected_choice_idx,
            choice_context,
            game_over: s.game_over,
            log_tail_size: 200,
        };

        build_view_model_json(&s.game, inputs)
    })
}

/// Get comprehensive game state as JSON for native web GUI
///
/// Returns a rich snapshot of the entire game state including:
/// - Turn number, current step, active player
/// - Per-player: life, mana pool, hand cards, battlefield cards (with P/T, tapped), graveyard, library size
/// - Stack contents
/// - Current prompt and available choices
/// - Recent log entries
///
/// This is the primary data source for the native web GUI (game.html).
/// The TUI version (fancy.html) uses the terminal renderer instead.
///
/// **DEPRECATED**: Prefer `tui_get_gui_view_model_json` which provides a
/// schema-versioned, semantically structured view model that uses the SAME
/// display decisions as the ratatui TUI (sorted hand order, battlefield
/// section grouping, log color classification, etc.). This function is
/// retained for backward compatibility while game.html migrates.
#[wasm_bindgen]
pub fn tui_get_full_state_json() -> String {
    GLOBAL_TUI_STATE.with(|state| {
        if let Some(ref state) = *state.borrow() {
            let s = state.borrow();
            let game = &s.game;

            // Build per-player data
            let mut players_json = Vec::new();
            for (idx, player) in game.players.iter().enumerate() {
                let pid = player.id;
                let pview = GameStateView::new(game, pid);
                let (w, u, b, r, g, c) = pview.available_mana();

                // Hand cards (only show our hand, opponent hand is hidden)
                let hand: Vec<serde_json::Value> = if pid == s.renderer.player_id {
                    pview
                        .hand()
                        .iter()
                        .filter_map(|&cid| {
                            game.cards.try_get(cid).map(|card| {
                                let mut css = vec!["card"];
                                if card.is_land() {
                                    css.push("land");
                                }
                                if card.is_creature() {
                                    css.push("creature");
                                }
                                let power = if card.is_creature() {
                                    Some(i32::from(card.base_power().unwrap_or(0)))
                                } else {
                                    None
                                };
                                let toughness = if card.is_creature() {
                                    Some(i32::from(card.base_toughness().unwrap_or(0)))
                                } else {
                                    None
                                };
                                let formatted_pt = if card.is_creature() {
                                    power.zip(toughness).map(|(p, t)| format!("{}/{}", p, t))
                                } else {
                                    None
                                };
                                serde_json::json!({
                                    "card_id": format!("{:?}", cid),
                                    "name": card.name.to_string(),
                                    "mana_cost": card.mana_cost.to_string(),
                                    "oracle_text": card.text.clone(),
                                    "types": format!("{:?}", card.types.as_slice()),
                                    "is_creature": card.is_creature(),
                                    "is_land": card.is_land(),
                                    "css_classes": css,
                                    "power": power,
                                    "toughness": toughness,
                                    "formatted_pt": formatted_pt,
                                })
                            })
                        })
                        .collect()
                } else {
                    Vec::new()
                };

                // Battlefield cards owned/controlled by this player
                let battlefield: Vec<serde_json::Value> = game
                    .battlefield
                    .cards
                    .iter()
                    .filter_map(|&cid| {
                        let card = game.cards.try_get(cid)?;
                        if card.controller != pid {
                            return None;
                        }
                        let power = game.get_effective_power(cid).ok();
                        let toughness = game.get_effective_toughness(cid).ok();

                        // Pre-compute CSS classes
                        let mut css = vec!["card"];
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

                        // Pre-compute formatted P/T
                        let formatted_pt = if card.is_creature() {
                            power.zip(toughness).map(|(p, t)| format!("{}/{}", p, t))
                        } else {
                            None
                        };

                        // Equipment attachment info
                        let attached_to_id = card.attached_to.map(|id| format!("{:?}", id));
                        let is_equipment = card.is_equipment();

                        // Find equipment attached TO this creature (for rendering stacks)
                        let attachments: Vec<serde_json::Value> = if card.is_creature() {
                            game.battlefield
                                .cards
                                .iter()
                                .filter_map(|&eid| {
                                    let eq = game.cards.try_get(eid)?;
                                    if eq.is_equipment() && eq.attached_to == Some(cid) {
                                        Some(serde_json::json!({
                                            "card_id": format!("{:?}", eid),
                                            "name": eq.name.to_string(),
                                        }))
                                    } else {
                                        None
                                    }
                                })
                                .collect()
                        } else {
                            Vec::new()
                        };

                        Some(serde_json::json!({
                            "card_id": format!("{:?}", cid),
                            "name": card.name.to_string(),
                            "mana_cost": card.mana_cost.to_string(),
                            "oracle_text": card.text.clone(),
                            "is_tapped": card.tapped,
                            "is_creature": card.is_creature(),
                            "is_land": card.is_land(),
                            "is_equipment": is_equipment,
                            "attached_to": attached_to_id,
                            "attachments": attachments,
                            "power": power,
                            "toughness": toughness,
                            "damage": card.damage,
                            "types": format!("{:?}", card.types.as_slice()),
                            "css_classes": css,
                            "formatted_pt": formatted_pt,
                        }))
                    })
                    .collect();

                // Graveyard
                let graveyard: Vec<serde_json::Value> = pview
                    .graveyard()
                    .iter()
                    .filter_map(|&cid| {
                        game.cards.try_get(cid).map(|card| {
                            serde_json::json!({
                                "card_id": format!("{:?}", cid),
                                "name": card.name.to_string(),
                            })
                        })
                    })
                    .collect();

                let lib_size = game.get_player_zones(pid).map(|z| z.library.len()).unwrap_or(0);

                players_json.push(serde_json::json!({
                    "index": idx,
                    "name": player.name.to_string(),
                    "life": player.life,
                    "library_size": lib_size,
                    "hand_size": pview.player_hand_size(pid),
                    "mana_pool": serde_json::json!({"W": w, "U": u, "B": b, "R": r, "G": g, "C": c}),
                    "hand": hand,
                    "battlefield": battlefield,
                    "graveyard": graveyard,
                }));
            }

            // Stack
            let stack: Vec<serde_json::Value> = game
                .stack
                .cards
                .iter()
                .filter_map(|&cid| {
                    let card = game.cards.try_get(cid)?;
                    let controller_idx = game.players.iter().position(|p| p.id == card.controller).unwrap_or(0);
                    Some(serde_json::json!({
                        "card_id": format!("{:?}", cid),
                        "name": card.name.to_string(),
                        "controller_idx": controller_idx,
                    }))
                })
                .collect();

            // Choices
            let choices: Vec<serde_json::Value> = s
                .current_choices
                .iter()
                .enumerate()
                .map(|(i, (text, highlighted))| {
                    serde_json::json!({
                        "index": i,
                        "text": text,
                        "highlighted": *highlighted,
                    })
                })
                .collect();

            // Recent logs (last 100) with CSS color hints
            let logs: Vec<serde_json::Value> = game
                .logger
                .logs()
                .iter()
                .rev()
                .take(100)
                .rev()
                .map(|entry| {
                    let color = css_color_for_log(&entry.message);
                    let bold = entry.message.contains(">>> Turn")
                        || entry.message.contains("<<<< ")
                        || (entry.message.contains("damage") && entry.message.contains("life:"));
                    serde_json::json!({
                        "text": entry.message,
                        "color": color,
                        "bold": bold,
                    })
                })
                .collect();

            // Active player index
            let active_idx = game
                .players
                .iter()
                .position(|p| p.id == game.turn.active_player)
                .unwrap_or(0);

            // Our player index (which player the GUI renders from perspective of)
            let our_idx = game
                .players
                .iter()
                .position(|p| p.id == s.renderer.player_id)
                .unwrap_or(0);
            let opp_idx = if our_idx == 0 { 1 } else { 0 };

            // Pre-compute status bar text
            let status_text = if s.game_over {
                format!(
                    "Turn {} | Phase: {:?} | Active: P{} | GAME OVER",
                    game.turn.turn_number,
                    game.turn.current_step,
                    active_idx + 1
                )
            } else {
                format!(
                    "Turn {} | Phase: {:?} | Active: P{}",
                    game.turn.turn_number,
                    game.turn.current_step,
                    active_idx + 1
                )
            };

            // Build choice context for human player
            let our_view = GameStateView::new(game, s.renderer.player_id);
            let choice_context = our_view.build_choice_context();

            let result = serde_json::json!({
                "turn_number": game.turn.turn_number,
                "current_step": format!("{:?}", game.turn.current_step),
                "active_player_idx": active_idx,
                "our_player_idx": our_idx,
                "game_over": s.game_over,
                "players": players_json,
                // Pre-organized player references (avoid array indexing in JS)
                "our_player": players_json.get(our_idx),
                "opponent": players_json.get(opp_idx),
                "stack": stack,
                "current_prompt": s.current_prompt,
                "choices": choices,
                "selected_choice_idx": s.selected_choice_idx,
                "logs": logs,
                // Pre-computed status bar text
                "status_text": status_text,
                // Choice context for human/agent display
                "choice_context": choice_context,
            });

            serde_json::to_string(&result).unwrap_or_else(|_| "{}".to_string())
        } else {
            "{}".to_string()
        }
    })
}
/// This is called from within the render loop, so it doesn't need to borrow GLOBAL_TUI_STATE
///
/// EntityPosition now stores:
/// - `area`: MAX bounds (cells) - the card's public bounding box
/// - `layout_area_px`: Optional pixel-based layout (if in GUI mode)
///
/// Note: Wildcard is intentional - Entity enum has many variants (hand cards, etc.);
/// we only export battlefield card positions for overlays.
#[allow(clippy::wildcard_enum_match_arm)]
fn export_card_positions_from_renderer(
    entity_positions: &[crate::game::fancy_tui_renderer::EntityPosition],
    game: &GameState,
    player_id: PlayerId,
) -> String {
    use crate::game::fancy_tui_renderer::{CardBounds, Entity};

    let mut positions = Vec::new();
    let view = GameStateView::new(game, player_id);

    // Get cell dimensions from thread-local storage (for fallback calculation)
    let (cell_w_px, cell_h_px) = CELL_DIMENSIONS.with(|dims| *dims.borrow());

    // Helper to create JSON object with layout bounds
    // If layout_area_px is provided, use it; otherwise calculate from cell dimensions
    let make_card_json = |card_id: &crate::core::CardId,
                          name: &str,
                          x: u16,
                          y: u16,
                          width: u16,
                          height: u16,
                          is_tapped: bool,
                          layout_area_px: Option<&crate::game::fancy_tui_renderer::LayoutAreaPx>|
     -> serde_json::Value {
        // Use stored pixel layout if available, otherwise calculate using CardBounds
        let (layout_w_px, layout_h_px) = if let Some(layout) = layout_area_px {
            (layout.width_px, layout.height_px)
        } else {
            // Fallback: calculate MAX bounds using CardBounds for correct MTG aspect ratio
            let bounds = if is_tapped {
                CardBounds::for_gui_tapped(width, height, cell_w_px, cell_h_px)
            } else {
                CardBounds::for_gui(width, height, cell_w_px, cell_h_px)
            };
            (bounds.max_width_px, bounds.max_height_px)
        };

        serde_json::json!({
            "card_id": format!("{:?}", card_id),
            "name": name,
            "x": x,
            "y": y,
            "width": width,
            "height": height,
            "is_tapped": is_tapped,
            "layout_width_px": layout_w_px,
            "layout_height_px": layout_h_px,
        })
    };

    // Extract card positions from renderer state
    // EntityPosition.area is now MAX bounds (the card's public bounding box)
    // We include SingleCard entities and cards from VisualStack/SimpleStack
    for entity_pos in entity_positions {
        match &entity_pos.entity {
            Entity::SingleCard { card_id, .. } => {
                // Check if this card is on the battlefield
                if game.battlefield.cards.contains(card_id) {
                    if let Ok(card) = game.cards.get(*card_id) {
                        let is_tapped = view.is_tapped(*card_id);
                        positions.push(make_card_json(
                            card_id,
                            &card.name.to_string(),
                            entity_pos.area.x,
                            entity_pos.area.y,
                            entity_pos.area.width,
                            entity_pos.area.height,
                            is_tapped,
                            entity_pos.layout_area_px.as_ref(),
                        ));
                    }
                }
            }
            Entity::VisualStack {
                card_ids, tapped_count, ..
            } => {
                // For visual stacks, target the TOP card with diagonal offset
                // The visual stack renderer draws cards diagonally from bottom-left to top-right
                // So we need to offset to the top-right position
                if let Some(&card_id) = card_ids.last() {
                    // LAST card is on top visually
                    if game.battlefield.cards.contains(&card_id) {
                        if let Ok(card) = game.cards.get(card_id) {
                            let is_tapped = *tapped_count > 0;
                            let stack_depth = card_ids.len() as u16;
                            let offset = stack_depth.saturating_sub(1); // DIAGONAL_OFFSET = 1

                            // Adjust layout_area_px for offset if present
                            let adjusted_layout = entity_pos.layout_area_px.as_ref().map(|l| {
                                crate::game::fancy_tui_renderer::LayoutAreaPx {
                                    x_px: l.x_px + f32::from(offset) * cell_w_px,
                                    y_px: l.y_px + f32::from(offset) * cell_h_px,
                                    width_px: l.width_px - f32::from(offset) * cell_w_px,
                                    height_px: l.height_px - f32::from(offset) * cell_h_px,
                                }
                            });

                            positions.push(make_card_json(
                                &card_id,
                                &card.name.to_string(),
                                entity_pos.area.x + offset,
                                entity_pos.area.y + offset,
                                entity_pos.area.width.saturating_sub(offset),
                                entity_pos.area.height.saturating_sub(offset),
                                is_tapped,
                                adjusted_layout.as_ref(),
                            ));
                        }
                    }
                }
            }
            Entity::SimpleStack {
                card_ids, is_tapped, ..
            } => {
                // For simple stacks, use the is_tapped field from the entity
                if let Some(&card_id) = card_ids.first() {
                    if game.battlefield.cards.contains(&card_id) {
                        if let Ok(card) = game.cards.get(card_id) {
                            positions.push(make_card_json(
                                &card_id,
                                &card.name.to_string(),
                                entity_pos.area.x,
                                entity_pos.area.y,
                                entity_pos.area.width,
                                entity_pos.area.height,
                                *is_tapped,
                                entity_pos.layout_area_px.as_ref(),
                            ));
                        }
                    }
                }
            }
            _ => {} // Skip hand cards and other entities
        }
    }

    serde_json::to_string(&positions).unwrap_or_else(|_| "[]".to_string())
}

/// WASM Fancy TUI Application State
///
/// This struct holds all the game state and is shared via Rc<RefCell<>>
/// for access from the render callback.
struct WasmFancyTuiState {
    /// The game state
    game: GameState,
    /// The TUI renderer (shared logic with native)
    renderer: FancyTuiRenderer,
    /// Player 1 controller type
    p1_controller_type: WasmControllerType,
    /// Player 2 controller type
    p2_controller_type: WasmControllerType,
    /// Human controller for player 1 (only if p1 is Human)
    p1_human_controller: Option<WasmHumanController>,
    /// Fixed script controller for player 1 (only if p1 is Fixed)
    p1_fixed_controller: Option<WasmRichInputController>,
    /// Current prompt text
    current_prompt: Option<String>,
    /// Current choices (text, is_highlighted)
    current_choices: Vec<(String, bool)>,
    /// Pending choice context from game loop (waiting for human input)
    pending_context: Option<ChoiceContext>,
    /// Currently selected choice index (for keyboard navigation)
    selected_choice_idx: usize,
    /// Whether the game is over
    game_over: bool,
    /// Error message if any
    error_message: Option<String>,
    /// Auto-run mode (AI vs AI)
    auto_run: bool,
    /// Whether we need to replay choices after user makes a new choice
    /// When true, we rewind to turn start and replay all choices including the new one
    needs_replay: bool,
    /// Whether the UI needs to be redrawn on the next frame
    /// This dirty bit prevents unnecessary redraws at 60Hz when nothing has changed
    needs_redraw: bool,
    /// Whether we're in network mode (separate from controller type)
    /// In network mode, P1 uses WasmNetworkLocalController and P2 uses WasmRemoteController
    #[cfg(feature = "wasm-network")]
    is_network_mode: bool,
    /// Controller seed for deterministic RandomController (network mode)
    /// IMPORTANT: This must match the seed used by native client to ensure identical behavior.
    /// See mtg-d0jg3 for WASM/native behavioral identity requirements.
    #[cfg(feature = "wasm-network")]
    controller_seed: u64,
    /// High-water mark for action count (for monotonicity invariant checking)
    /// This tracks the maximum action count seen during FORWARD progress.
    /// During rewind/replay, action count is allowed to decrease, but after
    /// replay completes, it should exceed this value.
    high_water_action_count: usize,
    /// High-water mark for log count (for monotonicity invariant checking)
    /// Similar to action count - log should only grow during forward progress.
    high_water_log_count: usize,
    /// Whether we're currently in a rewind/replay cycle
    /// Used to suppress monotonicity checks during replay
    in_rewind_replay: bool,
    /// Multi-blocker selection state. Each element is the
    /// `(blocker_idx, attacker_idx)` pair the user has *already* staged in the
    /// current Blockers prompt. The user can keep picking pairs; nothing is
    /// committed to the engine until they select "Done" (idx 0).
    staged_blocker_pairs: Vec<(usize, usize)>,
    /// Mapping from displayed choice index `N+1` (idx 0 is "Done") to the
    /// underlying `(blocker_idx, attacker_idx)` pair in
    /// `ChoiceContext::Blockers::{available_blockers, attackers}`.
    /// Recomputed in `update_choices_from_context` so we only need to
    /// store (and round-trip) legal, not-yet-staged options.
    blocker_choice_pairs: Vec<(usize, usize)>,
    /// Multi-card discard staging state. Each element is an index into the
    /// `ChoiceContext::Discard::hand` list that the user has staged in the
    /// current Discard prompt. Auto-commits to the engine once `count` cards
    /// have been staged. Required because the engine's `choose_cards_to_discard`
    /// is a single call expecting all `count` cards at once (e.g. Bazaar of
    /// Baghdad's "discard 3"), but the UI naturally collects picks one at a time.
    staged_discard_indices: Vec<usize>,
    /// Mapping from the current Discard prompt's displayed choice index to
    /// the underlying hand index in `ChoiceContext::Discard::hand`. Recomputed
    /// in `update_choices_from_context` so we only show not-yet-staged cards.
    discard_choice_indices: Vec<usize>,
}

impl WasmFancyTuiState {
    /// Create a new WASM fancy TUI state from a GameState (local game mode)
    fn new(game: GameState, p1_controller_type: WasmControllerType, p2_controller_type: WasmControllerType) -> Self {
        Self::new_with_network_mode(game, p1_controller_type, p2_controller_type, false, 0)
    }

    /// Create a new WASM fancy TUI state with explicit network mode flag
    ///
    /// # Parameters
    /// - `controller_seed`: Seed for RandomController. MUST match native client's seed
    ///   for behavioral identity (see mtg-d0jg3).
    fn new_with_network_mode(
        game: GameState,
        p1_controller_type: WasmControllerType,
        p2_controller_type: WasmControllerType,
        #[allow(unused_variables)] is_network_mode: bool,
        #[allow(unused_variables)] controller_seed: u64,
    ) -> Self {
        // In network mode, determine which player we control.
        // The non-Remote controller is ours; Remote is the opponent.
        let (our_player_idx, our_controller) = if is_network_mode && p1_controller_type == WasmControllerType::Remote {
            (1, p2_controller_type) // We are P2
        } else {
            (0, p1_controller_type) // We are P1 (or local mode)
        };

        // Create renderer for OUR player's perspective in GUI mode
        let player_id = game.players[our_player_idx].id;
        let (cell_w, cell_h) = CELL_DIMENSIONS.with(|dims| *dims.borrow());
        let renderer = FancyTuiRenderer::new_gui(player_id, true, cell_w, cell_h);

        // Create human controller for our player (could be P1 or P2 in network mode)
        // In network mode with Human controller, this is wrapped by WasmNetworkLocalController
        let p1_human_controller = if our_controller == WasmControllerType::Human {
            Some(WasmHumanController::new(player_id))
        } else {
            None
        };

        // Create fixed script controller if our player is Fixed
        let p1_fixed_controller = if our_controller == WasmControllerType::Fixed {
            // Get the script from thread-local storage (set via set_p1_fixed_script)
            let commands = FIXED_SCRIPT.with(|s| s.borrow().clone()).unwrap_or_default();
            Some(WasmRichInputController::new(player_id, commands))
        } else {
            None
        };

        let prompt = match our_controller {
            WasmControllerType::Human => "Game ready. Your turn to play!".to_string(),
            WasmControllerType::Fixed => "Game ready. Running fixed script...".to_string(),
            WasmControllerType::Random
            | WasmControllerType::Heuristic
            | WasmControllerType::Zero
            | WasmControllerType::Network
            | WasmControllerType::Remote => {
                if is_network_mode {
                    "Network AI game running...".to_string()
                } else {
                    "Game ready. Press Space to advance turn.".to_string()
                }
            }
        };

        // Auto-run for AI controllers in network mode (they don't need user input)
        // Also auto-run for Fixed controller (scripted play)
        #[cfg(feature = "wasm-network")]
        let auto_run = is_network_mode && !matches!(our_controller, WasmControllerType::Human);
        #[cfg(not(feature = "wasm-network"))]
        let auto_run = false;

        Self {
            game,
            renderer,
            p1_controller_type,
            p2_controller_type,
            p1_human_controller,
            p1_fixed_controller,
            current_prompt: Some(prompt),
            current_choices: Vec::new(),
            pending_context: None,
            selected_choice_idx: 0,
            game_over: false,
            error_message: None,
            auto_run,
            needs_replay: false,
            needs_redraw: true, // Initial draw required
            #[cfg(feature = "wasm-network")]
            is_network_mode,
            #[cfg(feature = "wasm-network")]
            controller_seed,
            high_water_action_count: 0,
            high_water_log_count: 0,
            in_rewind_replay: false,
            staged_blocker_pairs: Vec::new(),
            blocker_choice_pairs: Vec::new(),
            staged_discard_indices: Vec::new(),
            discard_choice_indices: Vec::new(),
        }
    }

    /// Check and update monotonicity invariants
    ///
    /// This verifies that action count and log count are monotonically increasing
    /// during forward progress. Violations indicate a bug in the rewind/replay logic.
    ///
    /// Returns an error message if an invariant is violated, None otherwise.
    fn check_monotonicity_invariants(&mut self) -> Option<String> {
        // Skip checks during rewind/replay - values are expected to decrease
        if self.in_rewind_replay {
            return None;
        }

        let current_action_count = self.game.undo_log.len();
        let current_log_count = self.game.logger.log_count();

        // Check action count monotonicity
        if current_action_count < self.high_water_action_count {
            let msg = format!(
                "MONOTONICITY VIOLATION: Action count decreased from {} to {} outside of rewind!",
                self.high_water_action_count, current_action_count
            );
            log::error!(target: "wasm_tui", "{}", msg);
            return Some(msg);
        }

        // Check log count monotonicity
        if current_log_count < self.high_water_log_count {
            let msg = format!(
                "MONOTONICITY VIOLATION: Log count decreased from {} to {} outside of rewind!",
                self.high_water_log_count, current_log_count
            );
            log::error!(target: "wasm_tui", "{}", msg);
            return Some(msg);
        }

        // Update high-water marks
        self.high_water_action_count = current_action_count;
        self.high_water_log_count = current_log_count;

        None
    }

    /// Rewind game state to turn start and return choices made so far, split by player
    ///
    /// This undoes all game state changes since the start of the turn,
    /// returning ReplayChoice entries partitioned into (our_choices, opponent_choices).
    ///
    /// In local mode, only our_choices are used (the AI re-computes its choices).
    /// In network mode, BOTH are needed because the opponent is remote and can't
    /// re-compute choices — they must be replayed from the saved log.
    fn rewind_to_turn_start(&mut self, our_id: PlayerId) -> (Vec<ReplayChoice>, Vec<ReplayChoice>) {
        let log_len_before = self.game.undo_log.len();
        log::debug!(target: "wasm_tui", "REWIND: Undo log has {} actions before rewind", log_len_before);

        let mut undo_log = std::mem::take(&mut self.game.undo_log);
        let result = undo_log.rewind_to_turn_start(&mut self.game);
        self.game.undo_log = undo_log;

        let log_len_after = self.game.undo_log.len();

        // rewind_to_turn_start returns None only if undo log is disabled
        // (which shouldn't happen for WASM TUI, but handle gracefully)
        let (turn_number, choice_actions, actions_rewound, log_size_at_turn) = match result {
            Some(r) => r,
            None => {
                log::warn!(target: "wasm_tui", "REWIND: Undo log disabled!");
                return (Vec::new(), Vec::new());
            }
        };

        // Truncate game logs to match the rewound state
        // This removes log entries generated after the turn started, preventing duplicates
        // when we replay the choices
        self.game.logger.truncate_to(log_size_at_turn);

        // Count total choices for logging
        let total_choices = choice_actions
            .iter()
            .filter(|a| matches!(a, GameAction::ChoicePoint { choice: Some(_), .. }))
            .count();

        log::debug!(
            target: "wasm_tui",
            "REWIND: Rewound to turn {}, {} actions undone, log now {} actions, {} total choice points",
            turn_number, actions_rewound, log_len_after, total_choices
        );

        // Partition choices by player: our choices vs opponent choices
        let mut our_choices = Vec::new();
        let mut opponent_choices = Vec::new();

        for action in choice_actions {
            if let GameAction::ChoicePoint {
                player_id,
                choice: Some(c),
                ..
            } = action
            {
                if player_id == our_id {
                    our_choices.push(c);
                } else {
                    opponent_choices.push(c);
                }
            }
        }

        log::debug!(
            target: "wasm_tui",
            "REWIND: Extracted {} our choices and {} opponent choices for replay",
            our_choices.len(), opponent_choices.len()
        );

        (our_choices, opponent_choices)
    }

    /// Convert a PendingChoice to a ReplayChoice using the current pending_context
    fn pending_choice_to_replay_choice(&self, pending: &PendingChoice) -> ReplayChoice {
        pending.to_replay_choice(self.pending_context.as_ref())
    }

    /// Run the game until input is needed or game ends
    ///
    /// For AI vs AI games, this runs one turn at a time (for step-through mode).
    /// For human player games, this uses the rewind/replay pattern:
    ///
    /// 1. If needs_replay is true (user just made a choice):
    ///    - Rewind game state to the start of the current turn
    ///    - Create a ReplayController with all choices from this turn (including new one)
    ///    - Run the game - it will replay deterministically and continue
    ///
    /// 2. If needs_replay is false (initial run or after turn boundary):
    ///    - Run until we hit NeedInput or game ends
    ///
    /// This pattern is necessary because when NeedInput is thrown, the call stack
    /// unwinds completely. ChoiceContext is NOT a continuation - it doesn't capture
    /// the stack state. To resume mid-turn, we must rewind and replay all choices.
    fn run_until_choice(&mut self) {
        if self.game_over {
            log::warn!(
                target: "wasm_tui",
                "run_until_choice: Skipping - game_over is true (error: {:?})",
                self.error_message
            );
            return;
        }

        let p1_id = self.game.players[0].id;
        let p2_id = self.game.players[1].id;

        // Network mode takes precedence - check early and handle separately
        #[cfg(feature = "wasm-network")]
        if self.is_network_mode {
            self.run_network_mode(p1_id, p2_id);
            return;
        }

        // Create P2 controller (for local games)
        let mut p2_controller = self.create_ai_controller(self.p2_controller_type, p2_id);

        if self.p1_controller_type == WasmControllerType::Human {
            // Human player - use rewind/replay pattern
            if self.needs_replay {
                // User just made a choice - rewind and replay
                self.needs_replay = false;

                // Mark that we're in rewind/replay mode (suppresses monotonicity checks)
                self.in_rewind_replay = true;

                let turn_before = self.game.turn.turn_number;
                log::debug!(target: "wasm_tui", "REPLAY: Starting replay on turn {}", turn_before);

                // Get the new choice from the human controller
                let new_choice = if let Some(ref mut human) = self.p1_human_controller {
                    if let Some(pending) = human.pending_choice.take() {
                        // Convert PendingChoice to ReplayChoice using current context
                        let choice = self.pending_choice_to_replay_choice(&pending);
                        log::debug!(target: "wasm_tui", "REPLAY: New choice = {:?}", choice);
                        Some(choice)
                    } else {
                        log::debug!(target: "wasm_tui", "REPLAY: No pending choice");
                        None
                    }
                } else {
                    None
                };

                // Rewind game state and get previous choices from this turn.
                // BOTH players' choices are saved and replayed to ensure deterministic
                // replay. Previously only human (P1) choices were replayed and AI (P2)
                // recomputed fresh, which could cause non-deterministic divergence
                // (e.g., HeuristicController making different decisions on replay).
                let (mut replay_choices, opponent_choices) = self.rewind_to_turn_start(p1_id);
                let turn_after_rewind = self.game.turn.turn_number;
                log::debug!(
                    target: "wasm_tui",
                    "REPLAY: After rewind - turn {}, {} P1 choices + {} P2 choices to replay",
                    turn_after_rewind, replay_choices.len(), opponent_choices.len()
                );

                // Clear spell_targets and other transient game loop state after rewind.
                // These fields are not tracked in the undo log, so they'd be stale
                // from the previous run and could cause incorrect behavior on replay.
                self.game.spell_targets.clear();
                self.game.pending_cast = None;
                self.game.pending_activation = None;
                self.game.pending_activation_effect_idx = None;
                self.game.pending_cycling_search = None;

                // Add the new choice if we have one
                if let Some(choice) = new_choice {
                    replay_choices.push(choice);
                }
                log::debug!(target: "wasm_tui", "REPLAY: Total P1 choices to replay: {}", replay_choices.len());

                // Create a fresh human controller for the replay.
                // The WasmHumanController doesn't need persistent state - all choices
                // are captured in the replay_choices, and any NEW choice will be
                // handled via handle_game_result() setting pending_context, which
                // then prompts the user for input via the UI.
                let human_controller = WasmHumanController::new(p1_id);

                // Create ReplayController that will replay choices then delegate to human
                let mut replay_controller = ReplayController::new(p1_id, Box::new(human_controller), replay_choices);

                // Wrap P2 controller in ReplayController to replay its saved choices.
                // After replay choices are exhausted, it delegates to the inner AI controller
                // for any NEW choices beyond the replay point. This ensures deterministic
                // replay: both players make exactly the same decisions as before.
                let mut p2_replay_controller = ReplayController::new(p2_id, p2_controller, opponent_choices);

                // Run the game with replay controllers for BOTH players
                // Scope game_loop tightly so self can be accessed afterwards
                let result = {
                    let mut game_loop = GameLoop::new(&mut self.game).with_verbosity(VerbosityLevel::Normal);
                    log::debug!(target: "wasm_tui", "REPLAY: Running game loop with replay controllers for both players...");
                    game_loop.run_until_input(&mut replay_controller, &mut p2_replay_controller)
                };

                let turn_after_run = self.game.turn.turn_number;
                log::debug!(
                    target: "wasm_tui",
                    "REPLAY: Game loop returned on turn {}, result type: {}",
                    turn_after_run,
                    match &result {
                        Ok(GameLoopState::Complete(_)) => "Complete",
                        Ok(GameLoopState::AwaitingInput(_)) => "AwaitingInput",
                        Err(_) => "Error",
                    }
                );

                // Note: We don't need to recover the inner controller because:
                // 1. Any choices already made are captured in replay_choices
                // 2. New choices come through handle_game_result -> pending_context -> UI
                // 3. The self.p1_human_controller is used for getting the pending_choice,
                //    but we've already extracted it above

                // Replay complete - clear the rewind flag
                self.in_rewind_replay = false;

                // Reset high water marks to establish new baseline after replay.
                // With deterministic replay (both players' choices replayed), the
                // action count should match the original. We still reset to handle
                // the case where the new choice leads to a different branch point.
                self.high_water_action_count = self.game.undo_log.len();
                self.high_water_log_count = self.game.logger.log_count();
                log::debug!(
                    target: "wasm_tui",
                    "REPLAY: Reset high water marks after replay - action_count={}, log_count={}",
                    self.high_water_action_count, self.high_water_log_count
                );

                // Handle the game result - this may set pending_context for the next choice
                self.handle_game_result(result);
                self.needs_redraw = true; // State changed, need redraw
            } else {
                // Normal run - no replay needed
                log::debug!(
                    target: "wasm_tui",
                    "NORMAL: Running game loop (no replay), turn {}",
                    self.game.turn.turn_number
                );
                if let Some(ref mut human) = self.p1_human_controller {
                    // Scope game_loop tightly so self can be accessed afterwards
                    let result = {
                        let mut game_loop = GameLoop::new(&mut self.game).with_verbosity(VerbosityLevel::Normal);
                        game_loop.run_until_input(human, p2_controller.as_mut())
                    };
                    let turn_after = self.game.turn.turn_number;
                    log::debug!(
                        target: "wasm_tui",
                        "NORMAL: Game loop returned on turn {}, result type: {}",
                        turn_after,
                        match &result {
                            Ok(GameLoopState::Complete(_)) => "Complete",
                            Ok(GameLoopState::AwaitingInput(_)) => "AwaitingInput",
                            Err(_) => "Error",
                        }
                    );

                    // Check monotonicity invariants after normal run
                    if let Some(violation_msg) = self.check_monotonicity_invariants() {
                        self.error_message = Some(violation_msg);
                        self.game_over = true;
                        self.needs_redraw = true;
                        return;
                    }

                    self.handle_game_result(result);
                    self.needs_redraw = true; // State changed, need redraw
                } else {
                    self.error_message = Some("Human controller not initialized".to_string());
                    self.needs_redraw = true; // Error message changed, need redraw
                }
            }
        } else if self.p1_controller_type == WasmControllerType::Fixed {
            // Fixed script controller - runs the script without user input
            // Uses the same rewind/replay pattern but choices come from the script
            if let Some(ref mut fixed) = self.p1_fixed_controller {
                // Scope game_loop tightly so self can be accessed in match arms
                let result = {
                    let mut game_loop = GameLoop::new(&mut self.game).with_verbosity(VerbosityLevel::Normal);
                    game_loop.run_until_input(fixed, p2_controller.as_mut())
                };

                match result {
                    Ok(GameLoopState::Complete(game_result)) => {
                        // Game ended
                        self.game_over = true;
                        self.pending_context = None;
                        self.current_choices.clear();
                        if let Some(winner) = game_result.winner {
                            let winner_name = self
                                .game
                                .get_player(winner)
                                .map(|p| p.name.to_string())
                                .unwrap_or_else(|_| "Unknown".to_string());
                            self.current_prompt = Some(format!("Game Over! {} wins!", winner_name));
                        } else {
                            self.current_prompt = Some("Game Over! Draw!".to_string());
                        }
                        self.needs_redraw = true; // State changed, need redraw
                    }
                    Ok(GameLoopState::AwaitingInput(context)) => {
                        // Script paused - show the context to user (similar to human)
                        self.pending_context = Some(context.clone());
                        self.selected_choice_idx = 0;
                        self.update_choices_from_context(&context);
                        self.current_prompt = Some("Fixed script waiting - press Space to continue".to_string());
                        self.needs_redraw = true; // State changed, need redraw
                    }
                    Err(e) => {
                        self.error_message = Some(format!("Fixed script error: {}", e));
                        self.game_over = true;
                        self.needs_redraw = true; // State changed, need redraw
                    }
                }
            } else {
                self.error_message = Some("Fixed controller not initialized".to_string());
                self.needs_redraw = true; // Error message changed, need redraw
            }
        } else {
            // AI vs AI - run one turn at a time for step-through mode
            let mut p1_controller = self.create_ai_controller(self.p1_controller_type, p1_id);

            // Scope game_loop tightly so self.game can be accessed in match arms
            let result = {
                let mut game_loop = GameLoop::new(&mut self.game).with_verbosity(VerbosityLevel::Normal);
                game_loop.run_one_turn(p1_controller.as_mut(), p2_controller.as_mut())
            };
            match result {
                Ok(Some(game_result)) => {
                    // Game ended
                    self.game_over = true;
                    self.pending_context = None;
                    self.current_choices.clear();
                    if let Some(winner) = game_result.winner {
                        let winner_name = self
                            .game
                            .get_player(winner)
                            .map(|p| p.name.to_string())
                            .unwrap_or_else(|_| "Unknown".to_string());
                        self.current_prompt = Some(format!("Game Over! {} wins!", winner_name));
                    } else {
                        self.current_prompt = Some("Game Over! Draw!".to_string());
                    }
                    self.needs_redraw = true; // State changed, need redraw
                }
                Ok(None) => {
                    // Turn completed, game continues
                    let turn = self.game.turn.turn_number;
                    self.current_prompt = Some(format!("Turn {} complete. Press Space for next turn.", turn));
                    self.needs_redraw = true; // State changed, need redraw
                }
                Err(e) => {
                    self.error_message = Some(format!("Game error: {}", e));
                    self.game_over = true;
                    self.needs_redraw = true; // State changed, need redraw
                }
            }
        }
    }

    /// Run the game in network mode
    ///
    /// Network mode coordinates with the server:
    /// - Our player uses WasmNetworkLocalController wrapping our controller type
    /// - Opponent uses WasmRemoteController receiving choices from server
    /// - Which player is "ours" depends on p1_controller_type vs p2_controller_type
    /// - For Human controller: uses rewind/replay pattern for resumable game loops
    /// - For AI controllers (Random, Heuristic, Zero): runs directly without replay
    #[cfg(feature = "wasm-network")]
    fn run_network_mode(&mut self, _p1_id: PlayerId, _p2_id: PlayerId) {
        // Get the shared network client
        let network_client = ensure_client();

        // Get our player ID directly from the server assignment (like native client)
        // This is authoritative - the server determines who is P1 and P2
        let our_player_id = match network_client.borrow().our_player_id() {
            Some(id) => id,
            None => {
                log::error!("run_network_mode: Server has not assigned player ID yet");
                self.error_message = Some("Server has not assigned player ID".to_string());
                self.needs_redraw = true;
                return;
            }
        };

        // Determine which player we are based on server assignment (like native client)
        // Native client: `let we_are_p1 = our_player_id.as_u32() == 0;`
        let we_are_p1 = our_player_id.as_u32() == 0;
        let opponent_id = if we_are_p1 { PlayerId::new(1) } else { PlayerId::new(0) };

        // Get our controller type based on server assignment
        let our_controller_type = if we_are_p1 {
            self.p1_controller_type
        } else {
            self.p2_controller_type
        };

        log::debug!(
            "run_network_mode: server assigned us {:?}, we_are_p1={}, opponent_id={:?}, our_controller={:?}",
            our_player_id,
            we_are_p1,
            opponent_id,
            our_controller_type
        );

        // Create remote controller for opponent
        let mut remote_controller = WasmRemoteController::new(opponent_id, network_client.clone());

        // Handle based on our controller type
        match our_controller_type {
            WasmControllerType::Human => {
                // Human controller - use rewind/replay pattern (same as local Human mode)
                self.run_network_mode_human_v2(
                    our_player_id,
                    opponent_id,
                    we_are_p1,
                    network_client,
                    &mut remote_controller,
                );
            }
            WasmControllerType::Random => {
                // Random controller - runs directly without user input
                // IMPORTANT: Use self.controller_seed to match native client behavior (mtg-d0jg3)
                let inner = RandomController::with_seed(our_player_id, self.controller_seed);
                let mut network_local = WasmNetworkLocalController::new(inner, network_client.clone());
                self.run_network_mode_ai_v2(our_player_id, we_are_p1, &mut network_local, &mut remote_controller);
            }
            WasmControllerType::Heuristic => {
                // Heuristic controller
                let inner = HeuristicController::new(our_player_id);
                let mut network_local = WasmNetworkLocalController::new(inner, network_client.clone());
                self.run_network_mode_ai_v2(our_player_id, we_are_p1, &mut network_local, &mut remote_controller);
            }
            WasmControllerType::Zero => {
                // Zero controller (always passes)
                let inner = ZeroController::new(our_player_id);
                let mut network_local = WasmNetworkLocalController::new(inner, network_client.clone());
                self.run_network_mode_ai_v2(our_player_id, we_are_p1, &mut network_local, &mut remote_controller);
            }
            WasmControllerType::Remote => {
                // This shouldn't happen - our_controller_type should never be Remote
                self.error_message = Some("Bug: our_controller_type is Remote".to_string());
                self.needs_redraw = true;
            }
            _ => {
                self.error_message = Some(format!(
                    "Unsupported controller type {:?} for network mode",
                    our_controller_type
                ));
                self.needs_redraw = true;
            }
        }
    }

    /// Run network mode with Human controller (uses rewind/replay pattern)
    ///
    /// # Arguments
    /// * `our_id` - Our player ID
    /// * `opponent_id` - Opponent's player ID (unused, for symmetry)
    /// * `we_are_p1` - Whether we are player 1 (index 0) or player 2 (index 1)
    /// * `network_client` - The shared network client
    /// * `remote_controller` - The remote controller for opponent
    #[cfg(feature = "wasm-network")]
    #[allow(unused_variables)]
    fn run_network_mode_human_v2(
        &mut self,
        our_id: PlayerId,
        opponent_id: PlayerId,
        we_are_p1: bool,
        network_client: SharedNetworkClient,
        remote_controller: &mut WasmRemoteController,
    ) {
        if self.needs_replay {
            // User just made a choice - rewind and replay
            self.needs_replay = false;

            // Mark that we're in rewind/replay mode (suppresses monotonicity checks)
            self.in_rewind_replay = true;

            let turn_before = self.game.turn.turn_number;
            let undo_len_before_rewind = self.game.undo_log.len();
            log::debug!(target: "wasm_tui", "NETWORK REPLAY: Starting on turn {}, undo_log={}", turn_before, undo_len_before_rewind);

            // Take the new pending choice from the stored human controller
            // IMPORTANT: Don't add this to replay_choices! It needs to go through
            // WasmNetworkLocalController so it gets submitted to the server.
            let new_pending_choice = if let Some(ref mut human) = self.p1_human_controller {
                human.pending_choice.take()
            } else {
                None
            };

            if let Some(ref choice) = new_pending_choice {
                log::debug!(target: "wasm_tui", "NETWORK REPLAY: New pending choice = {:?}", choice);
            }

            // Rewind game state and get previous choices from this turn
            // In network mode, we need BOTH players' choices: our choices for our
            // ReplayController, and opponent choices for the opponent's ReplayController.
            // Unlike local mode where the AI can re-compute its choices, the remote
            // opponent's choices must be replayed from the saved log.
            let (our_choices, opponent_choices) = self.rewind_to_turn_start(our_id);

            // Clear spell_targets and other transient game loop state after rewind.
            // These fields are not tracked in the undo log, so they'd be stale
            // from the previous run and could cause incorrect behavior on replay.
            self.game.spell_targets.clear();
            self.game.pending_cast = None;
            self.game.pending_activation = None;
            self.game.pending_activation_effect_idx = None;
            self.game.pending_cycling_search = None;

            log::debug!(
                target: "wasm_tui",
                "NETWORK REPLAY: After rewind - turn {}, undo_log={}, {} our choices + {} opponent choices to replay",
                self.game.turn.turn_number, self.game.undo_log.len(), our_choices.len(), opponent_choices.len()
            );

            // NOTE: We do NOT add new_pending_choice to our_choices!
            // The new choice must go through WasmNetworkLocalController to be
            // submitted to the server. If we add it to replay, it bypasses the server.

            // Create a fresh human controller and set the pending choice on it
            let mut human_controller = WasmHumanController::new(our_id);
            if let Some(pending) = new_pending_choice {
                human_controller.set_pending_choice(pending);
            }
            let network_local = WasmNetworkLocalController::new(human_controller, network_client.clone());

            // Create ReplayController for us: replays our saved choices, then delegates
            // to the WasmNetworkLocalController (which submits new choices to the server)
            let mut our_replay = ReplayController::new(our_id, Box::new(network_local), our_choices);

            // Create ReplayController for opponent: replays their saved choices, then
            // delegates to a fresh WasmRemoteController (which gets new choices from network).
            // This is critical for network mode — unlike local AI, the remote opponent
            // can't re-compute their choices, so we must replay them.
            let fresh_remote = WasmRemoteController::new(opponent_id, network_client.clone());
            let mut opponent_replay = ReplayController::new(opponent_id, Box::new(fresh_remote), opponent_choices);

            // Run the game with both replay controllers
            // Scope game_loop tightly so self can be accessed afterwards
            let result = {
                // Create sync callback that processes pending reveals
                let client_for_sync = network_client.clone();
                let local_player = our_id;
                let sync_callback = move |game: &mut GameState, _target_action: u64| {
                    let reveals = client_for_sync.borrow_mut().drain_reveals();
                    if !reveals.is_empty() {
                        log::debug!(
                            "WASM sync_callback (replay): processing {} reveals at action_count={}",
                            reveals.len(),
                            game.action_count()
                        );
                        for (owner, card, reason) in reveals {
                            process_card_reveal_wasm(game, owner, card, reason, Some(local_player));
                        }
                    }
                };

                let mut game_loop = GameLoop::new(&mut self.game)
                    .with_verbosity(VerbosityLevel::Normal)
                    .with_sync_callback(sync_callback)
                    .skip_opening_hands() // Server handles opening hands via CardRevealed
                    .with_deferred_game_end(); // Server is authoritative about game end

                // Pass controllers in correct order based on which player we are
                if we_are_p1 {
                    game_loop.run_until_input(&mut our_replay, &mut opponent_replay)
                } else {
                    game_loop.run_until_input(&mut opponent_replay, &mut our_replay)
                }
            };

            let turn_after_run = self.game.turn.turn_number;
            log::debug!(
                target: "wasm_tui",
                "NETWORK REPLAY: Game loop returned on turn {}, undo_log={}, result={}",
                turn_after_run,
                self.game.undo_log.len(),
                match &result {
                    Ok(GameLoopState::Complete(_)) => "Complete",
                    Ok(GameLoopState::AwaitingInput(_)) => "AwaitingInput",
                    Err(_) => "Error",
                }
            );

            // Replay complete - clear the rewind flag
            self.in_rewind_replay = false;

            // Reset high-water marks to post-replay values.
            // After replay, the undo_log may be shorter than the pre-rewind high-water mark
            // because normal runs between replays can process opponent choices that advance
            // the game beyond the replay's NeedInput point. Those "extra" opponent choices are
            // in the replay queue but not consumed because the game stops at OUR NeedInput
            // (after the new choice is submitted) before reaching the opponent's choices.
            // This is correct behavior - the hash check validates state correctness.
            self.high_water_action_count = self.game.undo_log.len();
            self.high_water_log_count = self.game.logger.log_count();

            self.handle_game_result(result);
            self.needs_redraw = true;
        } else {
            // Normal run - no replay needed
            log::debug!(
                target: "wasm_tui",
                "NETWORK NORMAL: Running game loop, turn {}, we_are_p1={}, undo_log={}",
                self.game.turn.turn_number,
                we_are_p1,
                self.game.undo_log.len()
            );

            if let Some(ref mut human) = self.p1_human_controller {
                // Create network local controller wrapping the human controller
                // Note: We need to take ownership for the game loop, but we clone the inner state
                let inner_clone = human.clone();
                let mut network_local = WasmNetworkLocalController::new(inner_clone, network_client.clone());

                // Scope game_loop so borrow of self.game ends before accessing self
                let result = {
                    // Create sync callback that processes pending reveals
                    let client_for_sync = network_client.clone();
                    let local_player = our_id;
                    let sync_callback = move |game: &mut GameState, _target_action: u64| {
                        let reveals = client_for_sync.borrow_mut().drain_reveals();
                        if !reveals.is_empty() {
                            log::debug!(
                                "WASM sync_callback (normal): processing {} reveals at action_count={}",
                                reveals.len(),
                                game.action_count()
                            );
                            for (owner, card, reason) in reveals {
                                process_card_reveal_wasm(game, owner, card, reason, Some(local_player));
                            }
                        }
                    };

                    let mut game_loop = GameLoop::new(&mut self.game)
                        .with_verbosity(VerbosityLevel::Normal)
                        .with_sync_callback(sync_callback)
                        .skip_opening_hands() // Server handles opening hands via CardRevealed
                        .with_deferred_game_end(); // Server is authoritative about game end

                    // Pass controllers in correct order based on which player we are
                    if we_are_p1 {
                        game_loop.run_until_input(&mut network_local, remote_controller)
                    } else {
                        game_loop.run_until_input(remote_controller, &mut network_local)
                    }
                };

                let turn_number = self.game.turn.turn_number;
                log::debug!(
                    target: "wasm_tui",
                    "NETWORK NORMAL: Game loop returned on turn {}, undo_log={}, result={}",
                    turn_number,
                    self.game.undo_log.len(),
                    match &result {
                        Ok(GameLoopState::Complete(_)) => "Complete",
                        Ok(GameLoopState::AwaitingInput(_)) => "AwaitingInput",
                        Err(_) => "Error",
                    }
                );

                // Check monotonicity invariants after normal network run
                if let Some(violation_msg) = self.check_monotonicity_invariants() {
                    self.error_message = Some(violation_msg);
                    self.game_over = true;
                    self.needs_redraw = true;
                    return;
                }

                self.handle_game_result(result);
                self.needs_redraw = true;
            } else {
                self.error_message = Some("Human controller not initialized for network mode".to_string());
                self.needs_redraw = true;
            }
        }
    }

    /// Run network mode with an AI controller (no replay needed)
    ///
    /// # Arguments
    /// * `_our_id` - Our player ID (for logging)
    /// * `we_are_p1` - Whether we are player 1 (index 0) or player 2 (index 1)
    /// * `our_controller` - Our local controller
    /// * `remote_controller` - The remote controller for opponent
    #[cfg(feature = "wasm-network")]
    fn run_network_mode_ai_v2<C: PlayerController>(
        &mut self,
        our_id: PlayerId,
        we_are_p1: bool,
        our_controller: &mut WasmNetworkLocalController<C>,
        remote_controller: &mut WasmRemoteController,
    ) {
        let start_turn = self.game.turn.turn_number;
        log::debug!(
            target: "wasm_tui",
            "NETWORK AI: Running game loop, turn {}, we_are_p1={}",
            start_turn,
            we_are_p1
        );

        // Get the network client for the sync callback
        let network_client = ensure_client();

        // Scope game_loop so borrow of self.game ends before accessing self
        let result = {
            // Create sync callback that processes pending reveals
            // This is the WASM equivalent of the native client's sync_callback
            let client_for_sync = network_client.clone();
            let local_player = our_id;
            let sync_callback = move |game: &mut GameState, _target_action: u64| {
                // Drain all pending reveals and process them
                let reveals = client_for_sync.borrow_mut().drain_reveals();
                if !reveals.is_empty() {
                    log::debug!(
                        "WASM sync_callback: processing {} reveals at action_count={}",
                        reveals.len(),
                        game.action_count()
                    );
                    for (owner, card, reason) in reveals {
                        process_card_reveal_wasm(game, owner, card, reason, Some(local_player));
                    }
                }
            };

            let mut game_loop = GameLoop::new(&mut self.game)
                .with_verbosity(VerbosityLevel::Normal)
                .with_sync_callback(sync_callback)
                .skip_opening_hands() // Server handles opening hands via CardRevealed
                .with_deferred_game_end(); // Server is authoritative about game end

            // Pass controllers in the correct order based on which player we are
            // GameLoop expects (p1_controller, p2_controller)
            if we_are_p1 {
                game_loop.run_until_input(our_controller, remote_controller)
            } else {
                game_loop.run_until_input(remote_controller, our_controller)
            }
        };

        let end_turn = self.game.turn.turn_number;
        log::debug!(
            target: "wasm_tui",
            "NETWORK AI: Game loop returned on turn {}",
            end_turn
        );

        self.handle_game_result(result);
        self.needs_redraw = true;
    }

    /// Handle game result after running (for human player games)
    fn handle_game_result(&mut self, result: crate::Result<GameLoopState>) {
        match result {
            Ok(GameLoopState::Complete(game_result)) => {
                // Game ended
                self.game_over = true;
                self.pending_context = None;
                self.current_choices.clear();
                if let Some(winner) = game_result.winner {
                    let winner_name = self
                        .game
                        .get_player(winner)
                        .map(|p| p.name.to_string())
                        .unwrap_or_else(|_| "Unknown".to_string());
                    self.current_prompt = Some(format!("Game Over! {} wins!", winner_name));
                } else {
                    self.current_prompt = Some("Game Over! Draw!".to_string());
                }
            }
            Ok(GameLoopState::AwaitingInput(context)) => {
                // In network mode, detect "waiting for server" contexts:
                // these are SpellAbility with empty available, meaning the controller
                // needs a ChoiceRequest from the server before it can proceed.
                // Don't show these as real choices - just display a status message.
                #[cfg(feature = "wasm-network")]
                if self.is_network_mode {
                    if let ChoiceContext::SpellAbility { available, .. } = &context {
                        if available.is_empty() {
                            log::debug!(
                                target: "wasm_tui",
                                "Waiting for server (not showing as choice), turn {}, undo_log={}",
                                self.game.turn.turn_number,
                                self.game.undo_log.len()
                            );
                            // Set pending_context to block should_auto_run() from re-triggering
                            // run_until_choice() every render frame (which causes duplicate log
                            // messages). tui_run_turn() will bypass this when the ChoiceRequest
                            // arrives via onMessageProcessed.
                            self.pending_context = Some(context);
                            self.current_prompt = Some("Waiting for server...".to_string());
                            self.current_choices.clear();
                            return;
                        }
                    }
                }

                // Need human input - display choices
                self.pending_context = Some(context.clone());
                self.selected_choice_idx = 0;
                self.renderer.state.digit_buffer.clear();
                self.update_choices_from_context(&context);

                // Debug logging: show game state when waiting for input
                let turn = self.game.turn.turn_number;
                let phase = format!("{:?}", self.game.turn.current_step);
                let active_player = self.game.turn.active_player;
                let p1_id = self.game.players[0].id;
                let is_p1_turn = active_player == p1_id;
                let choice_count = self.current_choices.len();
                let context_type = match &context {
                    ChoiceContext::SpellAbility { available, .. } => format!("SpellAbility({})", available.len()),
                    ChoiceContext::Targets { .. } => "Targets".to_string(),
                    ChoiceContext::ManaSources { .. } => "ManaSources".to_string(),
                    ChoiceContext::Attackers { .. } => "Attackers".to_string(),
                    ChoiceContext::Blockers { .. } => "Blockers".to_string(),
                    ChoiceContext::DamageOrder { .. } => "DamageOrder".to_string(),
                    ChoiceContext::Discard { .. } => "Discard".to_string(),
                    ChoiceContext::LibrarySearch { .. } => "LibrarySearch".to_string(),
                    ChoiceContext::SacrificePermanents { .. } => "SacrificePermanents".to_string(),
                    ChoiceContext::Modes { mode_count, .. } => format!("Modes({})", mode_count),
                };
                log::debug!(
                    target: "wasm_tui",
                    "Turn {}, {}, P1's turn: {}, choices: {}, context: {}",
                    turn, phase, is_p1_turn, choice_count, context_type
                );
            }
            Err(e) => {
                log::error!(
                    target: "wasm_tui",
                    "GAME OVER due to error on turn {}: {}",
                    self.game.turn.turn_number,
                    e
                );
                self.error_message = Some(format!("Game error: {}", e));
                self.game_over = true;
            }
        }
    }

    /// Update the current_choices display from a ChoiceContext
    ///
    /// Uses shared formatting functions from controller.rs to ensure consistency
    /// with native TUI.
    fn update_choices_from_context(&mut self, context: &ChoiceContext) {
        self.current_choices.clear();
        // Drop any in-flight multi-blocker selection state when we leave the
        // Blockers prompt, otherwise stale staged pairs would carry over and
        // be applied to a future Blockers prompt on a later turn.
        if !matches!(context, ChoiceContext::Blockers { .. }) {
            self.staged_blocker_pairs.clear();
            self.blocker_choice_pairs.clear();
        }
        // Drop any in-flight multi-card discard staging when we leave the
        // Discard prompt, otherwise stale staged indices could leak into a
        // future Discard prompt (different hand contents, different intent).
        if !matches!(context, ChoiceContext::Discard { .. }) {
            self.staged_discard_indices.clear();
            self.discard_choice_indices.clear();
        }
        let choices: Vec<String> = match context {
            ChoiceContext::SpellAbility { formatted_choices, .. } => formatted_choices.clone(),
            ChoiceContext::Targets { formatted_targets, .. } => {
                // Add "No target" as first option to match TUI
                std::iter::once("No target".to_string())
                    .chain(formatted_targets.clone())
                    .collect()
            }
            ChoiceContext::ManaSources { formatted_sources, .. } => formatted_sources.clone(),
            ChoiceContext::Attackers {
                formatted_creatures, ..
            } => {
                // Use "Done" to match TUI (not "Done (no more attackers)")
                let mut choices = vec!["Done".to_string()];
                choices.extend(formatted_creatures.clone());
                choices
            }
            ChoiceContext::Blockers {
                available_blockers,
                attackers,
                formatted_blockers,
                formatted_attackers,
            } => {
                // Reset and rebuild the index map so it stays in sync with the
                // choices we display below. choice idx 0 is reserved for "Done".
                self.blocker_choice_pairs.clear();
                let mut choices = vec!["Done".to_string()];
                for (b_idx, &blocker_id) in available_blockers.iter().enumerate() {
                    // Skip blockers the user has already staged this prompt -
                    // each creature can only block one attacker.
                    if self.staged_blocker_pairs.iter().any(|(staged_b, _)| *staged_b == b_idx) {
                        continue;
                    }
                    for (a_idx, &attacker_id) in attackers.iter().enumerate() {
                        // Per-pair MTG legality (Flying, Reach, Shadow, Fear, etc).
                        // Mirrors `validate_blocking_restrictions` so the engine
                        // never silently drops a choice we offered.
                        if !crate::game::combat_rules::can_block(&self.game, attacker_id, blocker_id) {
                            continue;
                        }
                        let blocker = formatted_blockers.get(b_idx).cloned().unwrap_or_default();
                        let attacker = formatted_attackers.get(a_idx).cloned().unwrap_or_default();
                        choices.push(format!("{} blocks {}", blocker, attacker));
                        self.blocker_choice_pairs.push((b_idx, a_idx));
                    }
                }
                choices
            }
            ChoiceContext::DamageOrder { formatted_blockers, .. } => formatted_blockers.clone(),
            ChoiceContext::Discard { formatted_hand, .. } => {
                // Rebuild the index map so it stays in sync with the choices we
                // display below: skip cards the user has already staged this prompt.
                self.discard_choice_indices.clear();
                let mut choices = Vec::new();
                for (hand_idx, label) in formatted_hand.iter().enumerate() {
                    if self.staged_discard_indices.contains(&hand_idx) {
                        continue;
                    }
                    choices.push(label.clone());
                    self.discard_choice_indices.push(hand_idx);
                }
                choices
            }
            ChoiceContext::LibrarySearch { formatted_cards, .. } => {
                let mut choices = vec!["Fail to find".to_string()];
                choices.extend(formatted_cards.clone());
                choices
            }
            ChoiceContext::SacrificePermanents {
                formatted_permanents, ..
            } => {
                let mut choices = vec!["Done".to_string()];
                choices.extend(formatted_permanents.clone());
                choices
            }
            ChoiceContext::Modes { formatted_modes, .. } => formatted_modes.clone(),
        };

        // Set prompt based on context type using shared prompt functions
        let prompt = match context {
            ChoiceContext::SpellAbility { .. } => {
                // Get player name from priority player
                if let Some(priority_player) = self.game.turn.priority_player {
                    let player_name = self
                        .game
                        .players
                        .iter()
                        .find(|p| p.id == priority_player)
                        .map(|p| p.name.as_str())
                        .unwrap_or("Player");
                    prompt_spell_ability(player_name)
                } else {
                    prompt_spell_ability("Player")
                }
            }
            ChoiceContext::Targets { spell_id, .. } => {
                // Get spell name from game state
                let spell_name = self
                    .game
                    .cards
                    .get(*spell_id)
                    .map(|c| c.name.as_str())
                    .unwrap_or("spell");
                prompt_target(spell_name)
            }
            ChoiceContext::ManaSources { .. } => "Choose mana source:".to_string(),
            ChoiceContext::Attackers { .. } => PROMPT_ATTACKERS.to_string(),
            ChoiceContext::Blockers {
                formatted_blockers,
                formatted_attackers,
                ..
            } => {
                if self.staged_blocker_pairs.is_empty() {
                    PROMPT_BLOCKERS.to_string()
                } else {
                    // Show running tally so the user can see what they've already
                    // staged before pressing "Done".
                    let staged: Vec<String> = self
                        .staged_blocker_pairs
                        .iter()
                        .map(|(b, a)| {
                            let bn = formatted_blockers.get(*b).cloned().unwrap_or_default();
                            let an = formatted_attackers.get(*a).cloned().unwrap_or_default();
                            format!("{} blocks {}", bn, an)
                        })
                        .collect();
                    format!(
                        "{} (staged: {}; pick more or 'Done')",
                        PROMPT_BLOCKERS,
                        staged.join(", ")
                    )
                }
            }
            ChoiceContext::DamageOrder { .. } => PROMPT_DAMAGE_ORDER.to_string(),
            ChoiceContext::Discard {
                count, formatted_hand, ..
            } => {
                let base = prompt_discard(*count);
                if self.staged_discard_indices.is_empty() {
                    base
                } else {
                    let staged: Vec<String> = self
                        .staged_discard_indices
                        .iter()
                        .map(|i| formatted_hand.get(*i).cloned().unwrap_or_default())
                        .collect();
                    let remaining = count.saturating_sub(self.staged_discard_indices.len());
                    format!("{} (staged: {}; pick {} more)", base, staged.join(", "), remaining)
                }
            }
            ChoiceContext::LibrarySearch { .. } => PROMPT_LIBRARY_SEARCH.to_string(),
            ChoiceContext::SacrificePermanents {
                count,
                card_type_description,
                ..
            } => {
                format!("Choose {} {} to sacrifice:", count, card_type_description)
            }
            ChoiceContext::Modes {
                mode_count, spell_id, ..
            } => {
                let spell_name = self
                    .game
                    .cards
                    .get(*spell_id)
                    .map(|c| c.name.as_str())
                    .unwrap_or("spell");
                format!("Choose {} mode(s) for {}:", mode_count, spell_name)
            }
        };
        self.current_prompt = Some(prompt);

        // Format choices with numeric prefixes using shared function
        self.current_choices = crate::game::display::format_choices_with_numbers(&choices, self.selected_choice_idx);
    }

    /// Handle selection of current choice index
    ///
    /// When the user makes a selection:
    /// 1. Convert the selection index to a PendingChoice
    /// 2. Set it on the human controller
    /// 3. Set needs_replay = true (we'll need to rewind and replay)
    /// 4. Call run_until_choice() which will do the rewind/replay
    fn select_current_choice(&mut self) {
        if self.pending_context.is_none() {
            return;
        }

        // Check if this is a "waiting" context (network mode waiting for server/ack)
        // These contexts have empty available options and should not trigger selection
        if let Some(ChoiceContext::SpellAbility { available, .. }) = &self.pending_context {
            if available.is_empty() {
                // This is a "waiting for server" or "waiting for acknowledgment" context
                // Don't allow selection - just re-run the game loop to check for updates
                self.run_until_choice();
                return;
            }
        }

        // Don't take the context yet - we need it for pending_choice_to_replay_choice
        let idx = self.selected_choice_idx;

        // Convert selection index to PendingChoice based on context type
        let pending = {
            let context = self.pending_context.as_ref().unwrap();
            match context {
                ChoiceContext::SpellAbility { .. } => {
                    // idx 0 = pass, idx 1+ = ability index - 1
                    if idx == 0 {
                        PendingChoice::SpellAbility(None)
                    } else {
                        PendingChoice::SpellAbility(Some(idx))
                    }
                }
                ChoiceContext::Targets { .. } => PendingChoice::Targets(vec![idx]),
                ChoiceContext::ManaSources { .. } => PendingChoice::ManaSources(vec![idx]),
                ChoiceContext::Attackers { .. } => {
                    if idx == 0 {
                        PendingChoice::Attackers(vec![]) // Done
                    } else {
                        PendingChoice::Attackers(vec![idx - 1])
                    }
                }
                ChoiceContext::Blockers { .. } => {
                    if idx == 0 {
                        // "Done": commit everything the user staged this prompt
                        // (which may be empty, meaning "no blocks").
                        let pairs = std::mem::take(&mut self.staged_blocker_pairs);
                        PendingChoice::Blockers(pairs)
                    } else {
                        // Stage the picked pair and re-render: keep the user
                        // here in the Blockers prompt so they can add more
                        // blockers or finalize. We do NOT commit to the engine
                        // yet — that happens when the user picks "Done".
                        let pair_idx = idx - 1;
                        if let Some(&pair) = self.blocker_choice_pairs.get(pair_idx) {
                            self.staged_blocker_pairs.push(pair);
                        }
                        // Recompute choices for the same context; selected idx
                        // back to 0 ("Done") so a quick second Enter submits.
                        let context = self.pending_context.clone().unwrap();
                        self.selected_choice_idx = 0;
                        self.update_choices_from_context(&context);
                        self.needs_redraw = true;
                        return;
                    }
                }
                ChoiceContext::DamageOrder { .. } => PendingChoice::DamageOrder(vec![idx]),
                ChoiceContext::Discard { count, .. } => {
                    // Multi-card discard: stage the picked card and re-render.
                    // Once `count` cards are staged, auto-commit the full Discard
                    // choice. The engine asks for ALL `count` cards in a single
                    // call (e.g. Bazaar of Baghdad's "discard 3"), so we must
                    // accumulate selections client-side before submitting.
                    let count = *count;
                    match stage_discard_pick(
                        &mut self.staged_discard_indices,
                        &self.discard_choice_indices,
                        idx,
                        count,
                    ) {
                        None => {
                            let context = self.pending_context.clone().unwrap();
                            // Reset selection to the first remaining option so a
                            // quick second Enter picks the next card.
                            self.selected_choice_idx = 0;
                            self.update_choices_from_context(&context);
                            self.needs_redraw = true;
                            return;
                        }
                        Some(pending) => {
                            self.discard_choice_indices.clear();
                            pending
                        }
                    }
                }
                ChoiceContext::LibrarySearch { .. } => {
                    if idx == 0 {
                        PendingChoice::LibrarySearch(None)
                    } else {
                        PendingChoice::LibrarySearch(Some(idx - 1))
                    }
                }
                ChoiceContext::SacrificePermanents { .. } => {
                    if idx == 0 {
                        PendingChoice::Sacrifice(vec![]) // Done
                    } else {
                        PendingChoice::Sacrifice(vec![idx - 1])
                    }
                }
                ChoiceContext::Modes { .. } => {
                    // idx directly maps to mode index (0-based)
                    PendingChoice::Modes(vec![idx])
                }
            }
        };

        // Set the pending choice on the human controller
        if let Some(ref mut human) = self.p1_human_controller {
            human.set_pending_choice(pending);
        }

        // Mark that we need to rewind and replay when run_until_choice is called
        // DON'T clear pending_context yet - we need it for pending_choice_to_replay_choice
        self.needs_replay = true;

        // Clear UI state
        self.current_choices.clear();
        self.selected_choice_idx = 0;

        // Continue running the game with rewind/replay
        // Note: run_until_choice() may set a NEW pending_context if we hit another choice point
        // So we don't clear pending_context here - it will be overwritten or left as-is
        self.run_until_choice();
    }

    /// Move selection up in the choice list
    fn select_previous_choice(&mut self) {
        if !self.current_choices.is_empty() && self.selected_choice_idx > 0 {
            self.selected_choice_idx -= 1;
            self.update_choice_highlights();
            self.needs_redraw = true; // UI state changed, need redraw
        }
    }

    /// Move selection down in the choice list
    fn select_next_choice(&mut self) {
        if !self.current_choices.is_empty() && self.selected_choice_idx < self.current_choices.len() - 1 {
            self.selected_choice_idx += 1;
            self.update_choice_highlights();
            self.needs_redraw = true; // UI state changed, need redraw
        }
    }

    /// Update highlight state in current_choices based on selected_choice_idx
    fn update_choice_highlights(&mut self) {
        for (idx, (_, highlighted)) in self.current_choices.iter_mut().enumerate() {
            *highlighted = idx == self.selected_choice_idx;
        }
    }

    /// Create an AI controller based on type
    ///
    /// NOTE: This is used for LOCAL (non-network) games only. In network mode,
    /// controllers are created directly with the proper seed from self.controller_seed
    /// to ensure behavioral identity with native client (see mtg-d0jg3).
    fn create_ai_controller(
        &self,
        controller_type: WasmControllerType,
        player_id: PlayerId,
    ) -> Box<dyn PlayerController> {
        match controller_type {
            #[cfg(feature = "wasm-network")]
            WasmControllerType::Remote => {
                // Remote controller for network opponent - polls network client for choices
                let client = ensure_client();
                Box::new(WasmRemoteController::new(player_id, client))
            }
            _ => super::create_ai_controller(controller_type, player_id, 42),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SHARED TUI SETUP HELPERS
// Used by both launch_fancy_tui() and launch_network_game() to avoid duplication
// ═══════════════════════════════════════════════════════════════════════════

/// Map log message content to CSS color string.
///
/// Mirrors the TUI's `style_for_log_content()` from fancy_tui_renderer.rs.
fn css_color_for_log(message: &str) -> &'static str {
    // Turn headers: yellow
    if message.contains(">>> Turn") || message.contains("<<<< ") {
        return "#ffd700";
    }
    // Step headers: cyan
    if message.starts_with("--- ") && message.ends_with(" ---") {
        return "#4cc9f0";
    }
    // Combat: magenta
    if message.contains("attacks") || message.contains("blocks") {
        return "#ff79c6";
    }
    // Damage/life loss: red
    if (message.contains("damage") && message.contains("life:"))
        || (message.contains("takes") && message.contains("damage"))
    {
        return "#ff5555";
    }
    // Life gain / resolves: green
    if (message.contains("gains") && message.contains("life")) || message.contains("resolves") {
        return "#50fa7b";
    }
    // Mana tapping: dark gray
    if (message.contains("Tap ") && message.contains("for {"))
        || (message.contains("taps") && message.contains("for {"))
    {
        return "#666";
    }
    // Target selection: dark gray
    if message.starts_with("  → targeting") {
        return "#666";
    }
    // Choice markers: cyan dim
    if message.starts_with("<Choice>") {
        return "#4cc9f0";
    }
    // Player-based coloring
    if message.starts_with("Player1") || message.contains(" Player1 ") {
        return "#6272a4";
    }
    if message.starts_with("Player2") || message.contains(" Player2 ") {
        return "#ff6e6e";
    }
    // Default: light gray
    "#ccc"
}

/// Check if auto-run should advance the game this frame.
///
/// Unified logic for both local and network modes:
/// - Must be in auto_run mode and game not over
/// - If a human controller is present, don't run when waiting for human input
///   (pending_context is set). AI-only games run freely.
fn should_auto_run(state: &WasmFancyTuiState) -> bool {
    if !state.auto_run || state.game_over {
        return false;
    }
    let has_human =
        state.p1_controller_type == WasmControllerType::Human || state.p2_controller_type == WasmControllerType::Human;
    // Don't auto-run when waiting for human input
    !(has_human && state.pending_context.is_some())
}

/// Process a keyboard event on the TUI state.
///
/// Converts RatZilla KeyCode to KeyInput and dispatches through the shared
/// event handler. Handles WASM-specific keys (auto-run toggle, card images,
/// controls panel) and human choice navigation.
#[allow(clippy::wildcard_enum_match_arm)]
fn process_key_event(state: &mut WasmFancyTuiState, code: KeyCode) {
    let key_input = match code {
        KeyCode::Char(' ') => Some(KeyInput::Space),
        KeyCode::Char('a' | 'A') => {
            state.auto_run = !state.auto_run;
            state.needs_redraw = true;
            return;
        }
        KeyCode::Char('i') => {
            let _ = js_sys::eval("window.toggleCardImages && window.toggleCardImages()");
            return;
        }
        KeyCode::Char('q' | 'Q') => Some(KeyInput::Pass),
        KeyCode::Esc => Some(KeyInput::Escape),
        KeyCode::Char('h' | 'H') => Some(KeyInput::FocusHand),
        KeyCode::Char('I') => Some(KeyInput::FocusInfo),
        KeyCode::Char('y' | 'Y') => Some(KeyInput::FocusYourBf),
        KeyCode::Char('o' | 'O') => Some(KeyInput::FocusOpponentBf),
        KeyCode::Char('s' | 'S') => Some(KeyInput::FocusStack),
        KeyCode::Char('b' | 'B') => Some(KeyInput::ShowBattlefield),
        KeyCode::Char('c' | 'C') => {
            let _ = js_sys::eval("document.getElementById('btn-toggle-controls')?.click()");
            return;
        }
        KeyCode::Char('?') => Some(KeyInput::Help),
        KeyCode::Char('w' | 'W') => Some(KeyInput::ToggleWrap),
        KeyCode::Tab => Some(KeyInput::Tab),
        KeyCode::Up => Some(KeyInput::Up),
        KeyCode::Down => Some(KeyInput::Down),
        KeyCode::Left => Some(KeyInput::Left),
        KeyCode::Right => Some(KeyInput::Right),
        KeyCode::PageUp => Some(KeyInput::PageUp),
        KeyCode::PageDown => Some(KeyInput::PageDown),
        KeyCode::Home => Some(KeyInput::Home),
        KeyCode::End => Some(KeyInput::End),
        KeyCode::Enter => Some(KeyInput::Enter),
        KeyCode::Backspace => Some(KeyInput::Backspace),
        KeyCode::Char(c) if c.is_ascii_digit() => Some(KeyInput::Digit(c.to_digit(10).unwrap() as u8)),
        _ => None,
    };

    let Some(key) = key_input else { return };

    // Handle human player choice navigation when a choice is pending
    if state.pending_context.is_some() {
        match key {
            KeyInput::Up => {
                state.select_previous_choice();
                return;
            }
            KeyInput::Down => {
                state.select_next_choice();
                return;
            }
            KeyInput::Enter | KeyInput::Space => {
                // If digit buffer is non-empty, parse and select that index
                if !state.renderer.state.digit_buffer.is_empty() {
                    if let Ok(idx) = state.renderer.state.digit_buffer.parse::<usize>() {
                        if idx < state.current_choices.len() {
                            state.selected_choice_idx = idx;
                            state.update_choice_highlights();
                            state.select_current_choice();
                        }
                    }
                    state.renderer.state.digit_buffer.clear();
                    state.needs_redraw = true;
                    return;
                }
                state.select_current_choice();
                state.needs_redraw = true;
                return;
            }
            KeyInput::Digit(n) => {
                let num_choices = state.current_choices.len();
                if num_choices > 10 {
                    // Multi-digit mode: accumulate in buffer, auto-highlight
                    state.renderer.state.digit_buffer.push(char::from(b'0' + n));
                    if let Ok(idx) = state.renderer.state.digit_buffer.parse::<usize>() {
                        if idx < num_choices {
                            state.selected_choice_idx = idx;
                            state.update_choice_highlights();
                        }
                    }
                    state.needs_redraw = true;
                } else {
                    // 1-based selection: press '1' for first choice, '0' for 10th
                    let idx = if n == 0 { 9 } else { (n - 1) as usize };
                    if idx < num_choices {
                        state.selected_choice_idx = idx;
                        state.update_choice_highlights();
                        state.select_current_choice();
                        state.needs_redraw = true;
                    }
                }
                return;
            }
            KeyInput::Backspace => {
                if !state.renderer.state.digit_buffer.is_empty() {
                    state.renderer.state.digit_buffer.pop();
                    if let Ok(idx) = state.renderer.state.digit_buffer.parse::<usize>() {
                        if idx < state.current_choices.len() {
                            state.selected_choice_idx = idx;
                            state.update_choice_highlights();
                        }
                    }
                    state.needs_redraw = true;
                }
                return;
            }
            KeyInput::Escape => {
                if !state.renderer.state.digit_buffer.is_empty() {
                    state.renderer.state.digit_buffer.clear();
                    state.needs_redraw = true;
                    return;
                }
            }
            _ => {}
        }
    }

    // Dispatch through the shared event handler
    let num_choices = state.current_choices.len();
    let WasmFancyTuiState {
        ref game,
        ref mut renderer,
        ..
    } = *state;

    let view = GameStateView::new(game, renderer.player_id);
    let result = handle_key_event(&mut renderer.state, key, &view, num_choices);

    match result {
        EventResult::Handled => {
            state.needs_redraw = true;
        }
        EventResult::NotHandled => {
            if matches!(key, KeyInput::Space) {
                state.run_until_choice();
                state.needs_redraw = true;
            }
        }
        EventResult::Pass | EventResult::Exit => {
            let _ = js_sys::eval("window.showExitConfirmation && window.showExitConfirmation()");
        }
        EventResult::SelectChoice(idx) => {
            if idx < state.current_choices.len() {
                state.selected_choice_idx = idx;
                state.update_choice_highlights();
                state.select_current_choice();
                state.needs_redraw = true;
            }
        }
        EventResult::ShowBattlefield => {
            let WasmFancyTuiState {
                ref game, ref renderer, ..
            } = *state;
            let view = GameStateView::new(game, renderer.player_id);
            let bf_text = crate::game::display::format_battlefield_for_log(&view);
            log::info!("{}", bf_text);
        }
        EventResult::ShowHelp => {
            let _ = js_sys::eval("window.showHelpDialog && window.showHelpDialog()");
        }
        _ => {}
    }
}

/// Convert pixel coordinates to terminal cell coordinates using measured dimensions.
///
/// Uses the `CELL_DIMENSIONS` thread-local which is set by `tui_set_cell_dimensions()`
/// from JavaScript after measuring real font metrics. Falls back to RatZilla defaults
/// (10x20) if not yet measured.
fn pixels_to_cells(x: u32, y: u32) -> (u16, u16) {
    CELL_DIMENSIONS.with(|dims| {
        let (w, h) = *dims.borrow();
        ((x as f32 / w) as u16, (y as f32 / h) as u16)
    })
}

/// Process a mouse click event on the TUI state.
///
/// Converts pixel coordinates to terminal cell coordinates using measured
/// dimensions and dispatches to the shared handle_mouse_click() function.
fn process_mouse_event(state: &mut WasmFancyTuiState, x: u32, y: u32) {
    let (cell_x, cell_y) = pixels_to_cells(x, y);

    let WasmFancyTuiState {
        ref game,
        ref mut renderer,
        ..
    } = *state;

    let view = GameStateView::new(game, renderer.player_id);
    handle_mouse_click(&mut renderer.state, cell_x, cell_y, &view);
    state.needs_redraw = true;
}

/// Draw the TUI frame using the shared renderer.
///
/// Called from within the render callback. Always draws the full frame
/// (RatZilla uses immediate-mode rendering where each frame must be populated).
fn draw_tui_frame(f: &mut Frame, state: &mut WasmFancyTuiState) {
    let WasmFancyTuiState {
        ref game,
        ref mut renderer,
        ref current_prompt,
        ref current_choices,
        ..
    } = *state;

    let player_id = renderer.player_id;
    let view = GameStateView::new(game, player_id);
    let prompt = current_prompt.as_deref();
    let choices: Vec<(String, bool)> = current_choices.clone();
    renderer.draw_ui(f, &view, prompt, &choices);
}

/// Run post-render JavaScript callbacks when state has changed.
///
/// Updates turn info in the header, exports card positions for image overlays,
/// and notifies JavaScript that rendering is complete via onRenderComplete().
fn run_post_render_js_callbacks(state: &mut WasmFancyTuiState) {
    if !state.needs_redraw {
        return;
    }
    state.needs_redraw = false;

    let turn_number = state.game.turn.turn_number;
    let game_over = state.game_over;
    let _ = js_sys::eval(&format!(
        "window.updateTurnInfo && window.updateTurnInfo({}, {})",
        turn_number, game_over
    ));

    let player_id = state.renderer.player_id;
    let positions_json =
        export_card_positions_from_renderer(&state.renderer.state.entity_positions, &state.game, player_id);

    let selected_card_json = if let Some(card_id) = state.renderer.state.selected_card_id {
        if let Ok(card) = state.game.cards.get(card_id) {
            let escaped_name = card.name.as_str().replace('\"', "\\\"");
            if let Some(pane_area) = state.renderer.state.card_details_pane_area {
                format!(
                    r#"{{"card_id": {}, "name": "{}", "pane": {{"x": {}, "y": {}, "width": {}, "height": {}}}}}"#,
                    card_id.as_u32(),
                    escaped_name,
                    pane_area.x,
                    pane_area.y,
                    pane_area.width,
                    pane_area.height
                )
            } else {
                format!(r#"{{"card_id": {}, "name": "{}"}}"#, card_id.as_u32(), escaped_name)
            }
        } else {
            "null".to_string()
        }
    } else {
        "null".to_string()
    };

    let js_code = format!(
        "window.onRenderComplete && window.onRenderComplete({}, {})",
        positions_json, selected_card_json
    );
    let _ = js_sys::eval(&js_code);
}

/// Set up the terminal with event handlers and render callback.
///
/// Shared terminal setup for both local and network modes:
/// 1. Installs keyboard event handler (key navigation, shortcuts)
/// 2. Installs mouse event handler (click-to-select cards)
/// 3. Stores state in thread-local for button callbacks
/// 4. Starts the render loop with auto-run + draw + JS callbacks
fn setup_terminal_and_render(terminal: Terminal<DomBackend>, state: Rc<RefCell<WasmFancyTuiState>>) {
    terminal.on_key_event({
        let state = Rc::clone(&state);
        move |key_event| {
            process_key_event(&mut state.borrow_mut(), key_event.code);
        }
    });

    terminal.on_mouse_event({
        let state = Rc::clone(&state);
        move |mouse_event| {
            if mouse_event.button != MouseButton::Left || mouse_event.event != MouseEventKind::Pressed {
                return;
            }
            process_mouse_event(&mut state.borrow_mut(), mouse_event.x, mouse_event.y);
        }
    });

    GLOBAL_TUI_STATE.with(|s| {
        *s.borrow_mut() = Some(Rc::clone(&state));
    });

    terminal.draw_web({
        move |f| {
            let mut state = state.borrow_mut();
            if should_auto_run(&state) {
                state.run_until_choice();
                state.needs_redraw = true;
            }
            draw_tui_frame(f, &mut state);
            run_post_render_js_callbacks(&mut state);
        }
    });
}

/// Launch the WASM fancy TUI in the browser
///
/// This function creates and runs the RatZilla-based TUI application.
///
/// Note: Wildcards are intentional - ratzilla KeyCode has 25+ variants, KeyInput
/// and FocusedPane have many variants; we handle the subset used in WASM TUI.
///
/// # Errors
///
/// Returns a `JsValue` error if game creation from the database fails.
///
/// # Panics
///
/// Panics if mutex locks are poisoned or internal channel operations fail.
#[wasm_bindgen]
#[allow(clippy::too_many_arguments)]
pub fn launch_fancy_tui(
    card_db: &WasmCardDatabase,
    p1_deck_name: &str,
    p2_deck_name: &str,
    starting_life: i32,
    seed: u64,
    p1_controller: WasmControllerType,
    p2_controller: WasmControllerType,
    _canvas_width: u32,
    _canvas_height: u32,
) -> Result<(), JsValue> {
    // Create the game
    let game = create_game_from_database(card_db, p1_deck_name, p2_deck_name, starting_life, seed)?;

    // Create the shared state
    let state = Rc::new(RefCell::new(WasmFancyTuiState::new(game, p1_controller, p2_controller)));

    // For human controller games, run until the first choice point
    // This populates the initial choice list for the player
    // For fixed controller, also run to start processing the script
    if p1_controller == WasmControllerType::Human || p1_controller == WasmControllerType::Fixed {
        state.borrow_mut().run_until_choice();
    }

    // Create the RatZilla backend, targeting our specific container element
    let backend = DomBackend::new_by_id("ratzilla-terminal")
        .map_err(|e| JsValue::from_str(&format!("Failed to create backend: {}", e)))?;
    let terminal =
        Terminal::new(backend).map_err(|e| JsValue::from_str(&format!("Failed to create terminal: {}", e)))?;

    setup_terminal_and_render(terminal, state);

    Ok(())
}

/// Launch a network game TUI
///
/// Called when the server signals game start (GameStarted message received).
/// The game state will be synchronized with the server.
///
/// Uses the GameStarted data stored in WasmNetworkClient to initialize:
/// - Starting life totals
/// - Player assignment (we may be P0 or P1)
/// - Controller types (Network for us, Remote for opponent)
#[wasm_bindgen]
#[cfg(feature = "wasm-network")]
pub fn launch_network_game(
    card_db: &WasmCardDatabase,
    deck_name: &str,
    controller_type: &str,
    controller_seed: u64,
    _canvas_width: u32,
    _canvas_height: u32,
) -> Result<(), JsValue> {
    log::info!(
        "launch_network_game: Initializing network game TUI with controller={}, seed={}",
        controller_type,
        controller_seed
    );

    // Parse the controller type from JavaScript
    // IMPORTANT: Only human and random are currently supported in WASM network mode.
    // Heuristic and zero are disabled until we achieve proper state synchronization
    // with the native client (mtg-d0jg3).
    let our_controller_type = match controller_type {
        "human" => WasmControllerType::Human,
        "random" => WasmControllerType::Random,
        "heuristic" | "zero" => {
            log::warn!(
                "launch_network_game: Controller '{}' disabled in WASM network mode. \
                 Only 'human' and 'random' are supported until state sync is fixed (mtg-d0jg3). \
                 Defaulting to Human.",
                controller_type
            );
            WasmControllerType::Human
        }
        _ => {
            log::warn!(
                "launch_network_game: Unknown controller type '{}', defaulting to Human",
                controller_type
            );
            WasmControllerType::Human
        }
    };

    // Get network client to read GameStarted data
    let client = ensure_client();
    let client_ref = client.borrow();

    // Get starting life from server (or default to 20)
    let starting_life = client_ref.starting_life();
    let our_player_id = client_ref.our_player_id();
    let opponent_name = client_ref.opponent_name().unwrap_or("Opponent").to_string();
    let our_name = deck_name.to_string(); // Use deck name as our name for now

    // CRITICAL: Get late-binding architecture data (mtg-d0jg3)
    let deck_card_ids = client_ref.deck_card_ids().cloned();
    let rng_state = client_ref.rng_state().to_vec();

    // Clone token definitions from server's GameStarted message
    // These are needed for CreateToken effects (e.g., Clue tokens, Food tokens)
    let token_defs: Vec<(String, crate::loader::CardDefinition)> = client_ref
        .token_definitions()
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    log::info!(
        "launch_network_game: starting_life={}, our_player_id={:?}, opponent={}, deck_card_ids={:?}",
        starting_life,
        our_player_id,
        opponent_name,
        deck_card_ids
            .as_ref()
            .map(|r| format!("P1:[{}..{}), P2:[{}..{})", r.p1_start, r.p1_end, r.p2_start, r.p2_end))
    );

    // Drop the borrow before creating the game
    drop(client_ref);

    // Create the game using late-binding architecture (mtg-d0jg3)
    // CRITICAL: Use init_game_reserve_only_wasm() with server's DeckCardIdRanges
    // This ensures WASM uses the SAME CardIDs as the server for behavioral identity.
    let game = if let Some(ref ranges) = deck_card_ids {
        log::info!("launch_network_game: Using late-binding CardID architecture with ranges");

        // Determine player names based on player assignment
        let (p1_name, p2_name) = match our_player_id {
            Some(pid) if pid.as_u32() == 0 => (our_name.clone(), opponent_name.clone()),
            Some(pid) if pid.as_u32() == 1 => (opponent_name.clone(), our_name.clone()),
            _ => (our_name.clone(), opponent_name.clone()),
        };

        // Create game with reserved CardID slots (same as native client)
        let mut game = init_game_reserve_only_wasm(p1_name, p2_name, starting_life, ranges);

        // Configure logger for WASM TUI: capture to memory for log panel display
        game.logger.set_output_mode(OutputMode::Memory);
        game.logger.set_verbosity(VerbosityLevel::Normal);

        // Initialize RNG from server state for deterministic shuffles
        if !rng_state.is_empty() {
            use rand_chacha::ChaCha12Rng;
            match bincode::deserialize::<ChaCha12Rng>(&rng_state) {
                Ok(rng) => {
                    *game.rng.borrow_mut() = rng;
                    log::info!(
                        "launch_network_game: Initialized RNG from server state ({} bytes)",
                        rng_state.len()
                    );
                }
                Err(e) => {
                    log::error!(
                        "launch_network_game: Failed to deserialize RNG state: {} - shuffles may diverge!",
                        e
                    );
                }
            }
        } else {
            log::warn!("launch_network_game: No RNG state from server - shuffles may diverge!");
        }

        // Enable reveal logging for network games (same as native)
        game.set_skip_reveals(false);

        // Populate token definitions so network game can create tokens
        if !token_defs.is_empty() {
            log::info!("launch_network_game: Populating {} token definitions", token_defs.len());
            for (name, def) in &token_defs {
                game.token_definitions
                    .insert(name.clone(), std::sync::Arc::new(def.clone()));
            }
        }

        game
    } else {
        // Fallback: legacy mode without late-binding (will cause CardID mismatches!)
        log::warn!(
            "launch_network_game: No DeckCardIdRanges from server! Using legacy initialization. \
             This WILL cause CardID mismatches and state desync!"
        );
        let seed = crate::network::now_ms();
        create_game_from_database(card_db, deck_name, deck_name, starting_life, seed)?
    };

    // Determine controller types based on our player assignment
    // Our controller type is what we selected, opponent is always Remote
    // If we're player 0: P1=our controller, P2=Remote
    // If we're player 1: P1=Remote, P2=our controller (but we still view from P1 perspective)
    let (p1_controller_type, p2_controller_type) = match our_player_id {
        Some(pid) if pid.as_u32() == 0 => {
            log::info!("launch_network_game: We are P1 (index 0)");
            (our_controller_type, WasmControllerType::Remote)
        }
        Some(pid) if pid.as_u32() == 1 => {
            log::info!("launch_network_game: We are P2 (index 1)");
            // P1 is remote opponent, P2 is us
            (WasmControllerType::Remote, our_controller_type)
        }
        _ => {
            log::warn!("launch_network_game: Player ID not assigned, defaulting to P1");
            (our_controller_type, WasmControllerType::Remote)
        }
    };

    // Create the shared state with network mode enabled
    let state = Rc::new(RefCell::new(WasmFancyTuiState::new_with_network_mode(
        game,
        p1_controller_type,
        p2_controller_type,
        true, // is_network_mode = true
        controller_seed,
    )));

    // Run until the first choice point
    state.borrow_mut().run_until_choice();

    // Create the RatZilla backend
    let backend = DomBackend::new_by_id("ratzilla-terminal")
        .map_err(|e| JsValue::from_str(&format!("Failed to create backend: {}", e)))?;
    let terminal =
        Terminal::new(backend).map_err(|e| JsValue::from_str(&format!("Failed to create terminal: {}", e)))?;

    setup_terminal_and_render(terminal, state);

    log::info!("launch_network_game: Network TUI ready");
    Ok(())
}

/// Helper function to create a game from database (mirrors WasmGame::from_database logic)
fn create_game_from_database(
    card_db: &WasmCardDatabase,
    p1_deck_name: &str,
    p2_deck_name: &str,
    starting_life: i32,
    seed: u64,
) -> Result<GameState, JsValue> {
    // Look up decks
    let p1_deck = card_db
        .decks
        .get(p1_deck_name)
        .ok_or_else(|| JsValue::from_str(&format!("Deck '{}' not found", p1_deck_name)))?;
    let p2_deck = card_db
        .decks
        .get(p2_deck_name)
        .ok_or_else(|| JsValue::from_str(&format!("Deck '{}' not found", p2_deck_name)))?;

    // Create game state
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), starting_life);
    game.seed_rng(seed);

    // Configure logger for WASM: capture to memory, enable normal verbosity
    game.logger.set_output_mode(OutputMode::Memory);
    game.logger.set_verbosity(VerbosityLevel::Normal);

    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;

    // Helper to add cards from a deck entry
    let add_deck_cards = |game: &mut GameState,
                          owner: PlayerId,
                          entry: &crate::loader::DeckEntry,
                          cards: &HashMap<String, Arc<CardDefinition>>|
     -> Result<(), String> {
        let card_def = cards
            .get(&entry.card_name)
            .ok_or_else(|| format!("Card '{}' not found in database", entry.card_name))?;

        for _ in 0..entry.count {
            let card_id = game.next_entity_id();
            let card = card_def.instantiate(card_id, owner);
            game.cards.insert(card_id, card);
            if let Some(zones) = game.get_player_zones_mut(owner) {
                zones.library.add(card_id);
            }
        }
        Ok(())
    };

    // Add player 1's deck
    for entry in &p1_deck.main_deck {
        add_deck_cards(&mut game, p1_id, entry, &card_db.cards).map_err(|e| JsValue::from_str(&e))?;
    }

    // Add player 2's deck
    for entry in &p2_deck.main_deck {
        add_deck_cards(&mut game, p2_id, entry, &card_db.cards).map_err(|e| JsValue::from_str(&e))?;
    }

    // Copy token definitions from card database into game state
    if !card_db.tokens.is_empty() {
        game.token_definitions = card_db.tokens.clone();
        log::info!("Loaded {} token definitions into game", game.token_definitions.len());
    }

    // Shuffle libraries
    game.shuffle_library(p1_id);
    game.shuffle_library(p2_id);

    // Draw opening hands (7 cards each) BEFORE the turn 1 marker
    for _ in 0..7 {
        let _ = game.draw_card(p1_id);
        let _ = game.draw_card(p2_id);
    }

    // Mark the start of turn 1 AFTER drawing opening hands.
    // This is critical for the rewind/replay pattern: when the user makes their
    // first choice on turn 1, we call rewind_to_turn_start() which looks for a
    // ChangeTurn action. The rewind stops AT this marker, preserving everything
    // before it (including opening hands). Actions AFTER this marker (during turn 1
    // gameplay) get rewound.
    let prior_log_size = game.logger.log_count();
    game.undo_log.log(
        crate::undo::GameAction::ChangeTurn {
            from_player: p1_id, // Doesn't matter for turn 1
            to_player: p1_id,
            turn_number: 1,
            rng_state: None,
        },
        prior_log_size,
    );

    Ok(game)
}

// ═══════════════════════════════════════════════════════════════════════════
// WASM NETWORK GAME INITIALIZATION (Late-Binding Architecture)
// ═══════════════════════════════════════════════════════════════════════════

// Shared with ai_harness.rs - functions live in crate::wasm::network::game_init
#[cfg(feature = "wasm-network")]
use crate::wasm::network::game_init::{init_game_reserve_only_wasm, process_card_reveal_wasm};
