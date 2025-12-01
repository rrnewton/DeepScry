//! WASM Fancy TUI - egui_ratatui-based TUI rendering for browser
//!
//! This module provides the fancy TUI experience in the browser using egui_ratatui.
//! It uses the shared FancyTuiRenderer for consistent rendering between native and WASM.
//!
//! ## Architecture
//!
//! - Uses eframe for the egui application framework
//! - Uses egui_ratatui's RataguiBackend for terminal rendering within egui
//! - Uses FancyTuiRenderer (shared with native) for all TUI drawing
//! - Game state is managed via the WasmGame wrapper

use crate::core::PlayerId;
use crate::game::controller::GameStateView;
use crate::game::logger::OutputMode;
use crate::game::{FancyTuiRenderer, GameLoop, GameState, VerbosityLevel};
use crate::game::{HeuristicController, PlayerController, RandomController, ZeroController};
use crate::loader::CardDefinition;
use eframe::egui;
use egui_ratatui::RataguiBackend;
use ratatui::Terminal;
use soft_ratatui::{CosmicText, SoftBackend};
use std::collections::HashMap;
use std::sync::Arc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::HtmlCanvasElement;

use super::{WasmCardDatabase, WasmControllerType};

// Include a monospace font for TUI rendering
// Using Fira Mono - an open-source monospace font from Mozilla
const FONT_DATA: &[u8] = include_bytes!("../../assets/FiraMono-Regular.ttf");

// Type alias for our backend
type WasmTuiBackend = RataguiBackend<CosmicText>;

/// WASM Fancy TUI Application
///
/// This struct implements the eframe App trait and manages the game state,
/// rendering, and user input for the browser TUI.
pub struct WasmFancyTuiApp {
    /// The game state
    game: GameState,
    /// The TUI renderer (shared logic with native)
    renderer: FancyTuiRenderer,
    /// The ratatui terminal with egui backend
    terminal: Terminal<WasmTuiBackend>,
    /// Player 1 controller type
    p1_controller_type: WasmControllerType,
    /// Player 2 controller type
    p2_controller_type: WasmControllerType,
    /// Current prompt text
    current_prompt: Option<String>,
    /// Current choices (text, is_highlighted)
    current_choices: Vec<(String, bool)>,
    /// Whether the game is over
    game_over: bool,
    /// Error message if any
    error_message: Option<String>,
    /// Auto-run mode (AI vs AI)
    auto_run: bool,
}

impl WasmFancyTuiApp {
    /// Create a new WASM fancy TUI app from a GameState
    pub fn new(
        game: GameState,
        p1_controller_type: WasmControllerType,
        p2_controller_type: WasmControllerType,
    ) -> Self {
        // Create the soft_ratatui backend with CosmicText font rendering
        // Using 100 columns x 40 rows with 10pt font to stay within WebGL texture limits
        // (max texture size is typically 8192px, so we need cols*char_width < 8192)
        // At 10pt, ~6px wide per char, 100*6=600px wide, 40*~12=480px tall - well under limit
        let soft_backend = SoftBackend::<CosmicText>::new(100, 40, 10, FONT_DATA);

        // Wrap in RataguiBackend for egui integration
        let backend = RataguiBackend::new("mtg-tui", soft_backend);
        let terminal = Terminal::new(backend).expect("Failed to create terminal");

        // Create renderer for player 1's perspective
        let player_id = game.players[0].id;
        let renderer = FancyTuiRenderer::new(player_id, true);

        Self {
            game,
            renderer,
            terminal,
            p1_controller_type,
            p2_controller_type,
            current_prompt: Some("Game ready. Click 'Run 1 Turn' to advance.".to_string()),
            current_choices: Vec::new(),
            game_over: false,
            error_message: None,
            auto_run: false,
        }
    }

    /// Run one turn of the game with AI controllers
    fn run_one_turn(&mut self) {
        if self.game_over {
            return;
        }

        let p1_id = self.game.players[0].id;
        let p2_id = self.game.players[1].id;

        // Create controllers based on type
        let mut p1_controller = self.create_controller(self.p1_controller_type, p1_id);
        let mut p2_controller = self.create_controller(self.p2_controller_type, p2_id);

        // Run one turn using the GameLoop
        let mut game_loop = GameLoop::new(&mut self.game).with_verbosity(VerbosityLevel::Normal);

        match game_loop.run_turns(p1_controller.as_mut(), p2_controller.as_mut(), 1) {
            Ok(result) => {
                if result.winner.is_some() {
                    self.game_over = true;
                }
            }
            Err(e) => {
                self.error_message = Some(format!("Game error: {}", e));
                self.game_over = true;
            }
        }
    }

    /// Create a controller based on type
    fn create_controller(&self, controller_type: WasmControllerType, player_id: PlayerId) -> Box<dyn PlayerController> {
        match controller_type {
            WasmControllerType::Zero => Box::new(ZeroController::new(player_id)),
            WasmControllerType::Random => Box::new(RandomController::with_seed(player_id, 42)),
            WasmControllerType::Heuristic => Box::new(HeuristicController::new(player_id)),
        }
    }
}

impl eframe::App for WasmFancyTuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Debug: log frame updates
        static mut FRAME_COUNT: u32 = 0;
        unsafe {
            FRAME_COUNT += 1;
            if FRAME_COUNT <= 5 || FRAME_COUNT % 60 == 0 {
                web_sys::console::log_1(&format!("Frame {}: update called", FRAME_COUNT).into());
            }
        }

        // Auto-run mode: run one turn per frame
        if self.auto_run && !self.game_over {
            self.run_one_turn();
            ctx.request_repaint();
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            // Debug: log available size
            let avail = ui.available_size();
            unsafe {
                if FRAME_COUNT <= 5 {
                    web_sys::console::log_1(&format!("Frame {}: available size = {}x{}", FRAME_COUNT, avail.x, avail.y).into());
                }
            }
            // Control bar at the top
            ui.horizontal(|ui| {
                if ui.button("Run 1 Turn").clicked() {
                    self.run_one_turn();
                }

                let auto_label = if self.auto_run { "Stop Auto" } else { "Auto Run" };
                if ui.button(auto_label).clicked() {
                    self.auto_run = !self.auto_run;
                }

                if self.game_over {
                    ui.label("Game Over!");
                    // Show winner based on life totals
                    let p1_life = self.game.players[0].life;
                    let p2_life = self.game.players[1].life;
                    if p1_life <= 0 {
                        ui.label("Player 2 wins!");
                    } else if p2_life <= 0 {
                        ui.label("Player 1 wins!");
                    }
                }

                if let Some(ref err) = self.error_message {
                    ui.colored_label(egui::Color32::RED, err);
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!("Turn {}", self.game.turn.turn_number));
                });
            });

            ui.separator();

            // Render the TUI to the terminal
            let view = GameStateView::new(&self.game, self.renderer.player_id);
            let prompt = self.current_prompt.as_deref();
            let choices: Vec<(String, bool)> = self.current_choices.clone();

            // Clear and draw to ratatui terminal
            let _ = self.terminal.clear();
            let renderer = &mut self.renderer;
            let _ = self.terminal.draw(|f| {
                renderer.draw_ui(f, &view, prompt, &choices);
            });

            // Display the terminal as an egui widget with fixed size
            // The terminal is 100x40 chars at 10pt (~6px wide, ~12px tall per char)
            // So approximately 600x480 pixels
            let desired_size = egui::vec2(700.0, 450.0);

            // Debug: log before adding widget
            unsafe {
                if FRAME_COUNT <= 5 {
                    web_sys::console::log_1(&format!("Frame {}: adding terminal widget, desired_size={}x{}", FRAME_COUNT, desired_size.x, desired_size.y).into());
                }
            }

            // Use allocate_exact_size to prevent the widget from requesting more space
            let (rect, _response) = ui.allocate_exact_size(desired_size, egui::Sense::hover());

            // Create a child UI constrained to this exact rect
            let mut child_ui = ui.new_child(egui::UiBuilder::new().max_rect(rect));
            child_ui.add(self.terminal.backend_mut());

            // Debug: log after frame completes
            unsafe {
                if FRAME_COUNT <= 5 {
                    web_sys::console::log_1(&format!("Frame {}: frame complete", FRAME_COUNT).into());
                }
            }
        });
    }
}

/// Launch the WASM fancy TUI in the browser
///
/// This function creates and runs the eframe application.
/// The canvas element with id "mtg-fancy-tui-canvas" must exist in the HTML.
#[wasm_bindgen]
pub fn launch_fancy_tui(
    card_db: &WasmCardDatabase,
    p1_deck_name: &str,
    p2_deck_name: &str,
    starting_life: i32,
    seed: u64,
    p1_controller: WasmControllerType,
    p2_controller: WasmControllerType,
) -> Result<(), JsValue> {
    // Create the game using the same logic as WasmGame::from_database
    let game = create_game_from_database(card_db, p1_deck_name, p2_deck_name, starting_life, seed)?;

    let app = WasmFancyTuiApp::new(game, p1_controller, p2_controller);

    // Configure eframe options for WASM
    let mut options = eframe::WebOptions::default();
    // Disable DPI scaling to prevent canvas size feedback loop
    options.dithering = false;

    // Get the canvas element from the DOM
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("No window"))?;
    let document = window.document().ok_or_else(|| JsValue::from_str("No document"))?;
    let canvas = document
        .get_element_by_id("mtg-fancy-tui-canvas")
        .ok_or_else(|| JsValue::from_str("Canvas element 'mtg-fancy-tui-canvas' not found"))?;
    let canvas: HtmlCanvasElement = canvas
        .dyn_into::<HtmlCanvasElement>()
        .map_err(|_| JsValue::from_str("Element is not a canvas"))?;

    // Run the eframe app asynchronously
    wasm_bindgen_futures::spawn_local(async move {
        let runner = eframe::WebRunner::new();
        runner
            .start(canvas, options, Box::new(|_cc| Ok(Box::new(app))))
            .await
            .expect("Failed to start eframe");
    });

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

    // Shuffle libraries
    game.shuffle_library(p1_id);
    game.shuffle_library(p2_id);

    // Draw opening hands (7 cards each)
    for _ in 0..7 {
        let _ = game.draw_card(p1_id);
        let _ = game.draw_card(p2_id);
    }

    Ok(game)
}
