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
    /// Canvas width in pixels (for rendering calculations)
    canvas_width: u32,
    /// Canvas height in pixels (for rendering calculations)
    canvas_height: u32,
    /// Current terminal columns
    term_cols: u16,
    /// Current terminal rows
    term_rows: u16,
}

// Font metrics for 10pt Fira Mono (approximate)
const FONT_SIZE: i32 = 10;
const CHAR_WIDTH: u32 = 6; // ~6px per character at 10pt
const CHAR_HEIGHT: u32 = 14; // ~14px line height at 10pt

impl WasmFancyTuiApp {
    /// Create a new WASM fancy TUI app from a GameState
    ///
    /// canvas_width and canvas_height are in CSS pixels
    pub fn new(
        game: GameState,
        p1_controller_type: WasmControllerType,
        p2_controller_type: WasmControllerType,
        canvas_width: u32,
        canvas_height: u32,
    ) -> Self {
        // Calculate terminal dimensions based on canvas size
        // Reserve some space for the egui control bar (~30px)
        let usable_height = canvas_height.saturating_sub(40);

        // Calculate columns and rows based on font metrics
        let cols = ((canvas_width / CHAR_WIDTH) as u16).max(80).min(200);
        let rows = ((usable_height / CHAR_HEIGHT) as u16).max(20).min(60);

        web_sys::console::log_1(
            &format!(
                "Creating terminal: {}x{} chars for {}x{} canvas",
                cols, rows, canvas_width, canvas_height
            )
            .into(),
        );

        // Create the soft_ratatui backend with CosmicText font rendering
        let soft_backend = SoftBackend::<CosmicText>::new(cols, rows, FONT_SIZE, FONT_DATA);

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
            current_prompt: Some("Game ready. Use buttons above to advance.".to_string()),
            current_choices: Vec::new(),
            game_over: false,
            error_message: None,
            auto_run: false,
            canvas_width,
            canvas_height,
            term_cols: cols,
            term_rows: rows,
        }
    }

    /// Resize the terminal if needed based on new dimensions
    fn resize_terminal_if_needed(&mut self, new_width: u32, new_height: u32) {
        // Calculate new terminal dimensions
        let usable_height = new_height.saturating_sub(60); // More space for control bar
        let new_cols = ((new_width / CHAR_WIDTH) as u16).max(80).min(200);
        let new_rows = ((usable_height / CHAR_HEIGHT) as u16).max(20).min(60);

        // Only resize if dimensions actually changed
        if new_cols != self.term_cols || new_rows != self.term_rows {
            web_sys::console::log_1(
                &format!(
                    "Resizing terminal: {}x{} -> {}x{}",
                    self.term_cols, self.term_rows, new_cols, new_rows
                )
                .into(),
            );

            // Create new backend and terminal
            let soft_backend = SoftBackend::<CosmicText>::new(new_cols, new_rows, FONT_SIZE, FONT_DATA);
            let backend = RataguiBackend::new("mtg-tui", soft_backend);
            self.terminal = Terminal::new(backend).expect("Failed to create terminal");

            self.canvas_width = new_width;
            self.canvas_height = new_height;
            self.term_cols = new_cols;
            self.term_rows = new_rows;
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

        // Set up dark theme with black background
        let mut style = (*ctx.style()).clone();
        style.visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(30, 30, 30);
        style.visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);
        style.visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(50, 50, 50);
        style.visuals.widgets.active.bg_fill = egui::Color32::from_rgb(70, 70, 70);
        style.visuals.override_text_color = Some(egui::Color32::WHITE);
        ctx.set_style(style);

        egui::CentralPanel::default().show(ctx, |ui| {
            // Check for resize - use available size from egui
            let avail = ui.available_size();
            let new_width = avail.x as u32;
            let new_height = avail.y as u32;

            // Resize terminal if window size changed significantly (>10px difference)
            if new_width.abs_diff(self.canvas_width) > 10 || new_height.abs_diff(self.canvas_height) > 10 {
                self.resize_terminal_if_needed(new_width, new_height);
            }

            // Debug: log available size
            unsafe {
                if FRAME_COUNT <= 5 {
                    web_sys::console::log_1(
                        &format!("Frame {}: available size = {}x{}", FRAME_COUNT, avail.x, avail.y).into(),
                    );
                }
            }

            // Control bar at the top with visible buttons
            egui::Frame::none()
                .fill(egui::Color32::from_rgb(20, 20, 20))
                .inner_margin(egui::Margin::symmetric(8, 4))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().button_padding = egui::vec2(12.0, 6.0);

                        if ui.button("Run 1 Turn").clicked() {
                            self.run_one_turn();
                        }

                        let auto_label = if self.auto_run { "Stop Auto" } else { "Auto Run" };
                        if ui.button(auto_label).clicked() {
                            self.auto_run = !self.auto_run;
                        }

                        ui.add_space(20.0);

                        if self.game_over {
                            ui.colored_label(egui::Color32::YELLOW, "Game Over!");
                            let p1_life = self.game.players[0].life;
                            let p2_life = self.game.players[1].life;
                            if p1_life <= 0 {
                                ui.colored_label(egui::Color32::GREEN, "Player 2 wins!");
                            } else if p2_life <= 0 {
                                ui.colored_label(egui::Color32::GREEN, "Player 1 wins!");
                            }
                        }

                        if let Some(ref err) = self.error_message {
                            ui.colored_label(egui::Color32::RED, err);
                        }

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(format!("Turn {}", self.game.turn.turn_number));
                        });
                    });
                });

            ui.add_space(4.0);

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

            // Display the terminal as an egui widget
            // Use available space from ui, leaving room for margins
            let remaining = ui.available_size();
            let desired_size = egui::vec2(
                remaining.x - 8.0, // Small margin
                remaining.y - 8.0, // Small margin
            );

            // Debug: log before adding widget
            unsafe {
                if FRAME_COUNT <= 5 {
                    web_sys::console::log_1(
                        &format!(
                            "Frame {}: adding terminal widget, desired_size={}x{}",
                            FRAME_COUNT, desired_size.x, desired_size.y
                        )
                        .into(),
                    );
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
///
/// canvas_width and canvas_height specify the desired canvas size in CSS pixels.
#[wasm_bindgen]
pub fn launch_fancy_tui(
    card_db: &WasmCardDatabase,
    p1_deck_name: &str,
    p2_deck_name: &str,
    starting_life: i32,
    seed: u64,
    p1_controller: WasmControllerType,
    p2_controller: WasmControllerType,
    canvas_width: u32,
    canvas_height: u32,
) -> Result<(), JsValue> {
    // Create the game using the same logic as WasmGame::from_database
    let game = create_game_from_database(card_db, p1_deck_name, p2_deck_name, starting_life, seed)?;

    let app = WasmFancyTuiApp::new(game, p1_controller, p2_controller, canvas_width, canvas_height);

    // Configure eframe options for WASM
    let mut options = eframe::WebOptions::default();
    // Disable dithering (not related to size, but a reasonable default)
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

    // Set the canvas size to match requested dimensions
    canvas.set_width(canvas_width);
    canvas.set_height(canvas_height);

    // Also set CSS size to prevent scaling
    let style = canvas.style();
    let _ = style.set_property("width", &format!("{}px", canvas_width));
    let _ = style.set_property("height", &format!("{}px", canvas_height));

    web_sys::console::log_1(&format!("Canvas configured: {}x{}", canvas_width, canvas_height).into());

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
