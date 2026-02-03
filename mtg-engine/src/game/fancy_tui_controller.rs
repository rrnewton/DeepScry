//! Fancy TUI controller with full-screen ratatui interface
//!
//! This controller provides a rich, multi-panel TUI interface similar to MTG Arena,
//! with separate panels for battlefield, hand, card details, prompts, and game state.
//!
//! The UI rendering is delegated to [`FancyTuiRenderer`] which is shared with the
//! WASM browser implementation for exact visual parity.

use crate::core::{CardId, ManaCost, PlayerId, SpellAbility};
use crate::game::controller::{
    format_card_choices, format_spell_ability_choice, format_spell_ability_choices, format_target_choices,
    prompt_mana_source, prompt_spell_ability, prompt_target, sort_spell_abilities, ChoiceResult, GameStateView,
    PlayerController, PROMPT_ATTACKERS,
};
use crate::game::fancy_tui_renderer::{BattlefieldEntity, ChoiceContext, FancyTuiRenderer, FocusedPane};
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers, MouseButton, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use rand::Rng;
use ratatui::{backend::CrosstermBackend, Terminal};
use signal_hook::consts::signal::{SIGCONT, SIGTSTP};
use signal_hook::flag as signal_flag;
use smallvec::SmallVec;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Input action result from user interaction
enum InputAction {
    /// Continue - need to redraw UI (arrow key pressed)
    Continue,
    /// Select a specific choice index
    Select(usize),
    /// Pass/cancel the choice
    Pass,
    /// Exit the game (Ctrl-C pressed)
    Exit,
    /// Undo the most recent action (Z key pressed)
    Undo,
    /// Make a random choice (R key pressed)
    RandomChoice,
    /// Show help/keyboard shortcuts (? or / key pressed)
    ShowHelp,
}

/// Result from prompting for a choice
enum PromptResult {
    /// User made a normal choice
    Choice(Option<usize>),
    /// User requested undo
    Undo,
}

/// A controller that provides a rich TUI interface using ratatui
///
/// This controller handles terminal setup, event handling, and user input.
/// All rendering is delegated to [`FancyTuiRenderer`].
pub struct FancyTuiController {
    player_id: PlayerId,
    /// The shared renderer that handles all UI drawing
    renderer: FancyTuiRenderer,
    /// Whether logger was configured for memory-only mode
    logger_memory_mode_enabled: bool,
}

impl FancyTuiController {
    /// Create a new fancy TUI controller
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the controller cannot be initialized.
    pub fn new(player_id: PlayerId, visual_stacks: bool) -> io::Result<Self> {
        Ok(FancyTuiController {
            player_id,
            renderer: FancyTuiRenderer::new(player_id, visual_stacks),
            logger_memory_mode_enabled: false,
        })
    }

    /// Initialize the terminal for TUI mode
    fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<io::Stdout>>> {
        use crossterm::event::EnableMouseCapture;

        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        Terminal::new(backend)
    }

    /// Restore the terminal to normal mode
    fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
        use crossterm::event::DisableMouseCapture;
        use std::io::Write;

        // Restore terminal state in reverse order of setup
        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
        terminal.show_cursor()?;

        // Flush all pending operations to ensure terminal is fully restored
        terminal.backend_mut().flush()?;

        Ok(())
    }

    /// Configure game logger for memory-only mode (suppress stdout)
    pub fn configure_logger_for_tui(&mut self, _view: &GameStateView) {
        // The logger is in the GameState which we can't mutate through GameStateView
        // We'll need to do this differently - see implementation note
        self.logger_memory_mode_enabled = true;
    }

    /// Save buffered logs to a temp file and print the location
    /// Call this after the game ends and terminal is restored
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the log file cannot be created or written.
    ///
    /// # Panics
    ///
    /// Panics if the system time is before the Unix epoch (should never happen).
    pub fn save_logs_on_exit(&self, view: &GameStateView) -> io::Result<()> {
        if !self.logger_memory_mode_enabled {
            return Ok(());
        }

        let logs = view.logger().logs();
        let log_count = logs.len();

        if log_count == 0 {
            eprintln!("No game logs captured.");
            return Ok(());
        }

        // Create temp file for logs
        let temp_dir = std::env::temp_dir();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let log_path = temp_dir.join(format!("mtg_forge_game_{}.log", timestamp));

        // Write logs to file
        use std::io::Write;
        let mut file = std::fs::File::create(&log_path)?;
        for entry in logs.iter() {
            writeln!(file, "{}", entry.message)?;
        }

        eprintln!("\n>>> Game log saved: {} lines written to:", log_count);
        eprintln!("    {}", log_path.display());

        Ok(())
    }

    /// Wait for user input and update highlighted choice
    ///
    /// Note: Wildcards are intentional - crossterm KeyCode/Event/MouseEventKind have 25+ variants
    /// each, and FocusedPane is internal. We handle the subset of keys/events we use.
    #[allow(clippy::wildcard_enum_match_arm)]
    fn wait_for_choice_input(&mut self, num_choices: usize, view: &GameStateView) -> io::Result<InputAction> {
        // Set up signal handlers for suspend/resume
        let sigtstp_flag = Arc::new(AtomicBool::new(false));
        let sigcont_flag = Arc::new(AtomicBool::new(false));

        // Register SIGTSTP (Ctrl-Z) handler
        let _sigtstp_handle = signal_flag::register(SIGTSTP, Arc::clone(&sigtstp_flag)).map_err(io::Error::other)?;

        // Register SIGCONT (resume) handler
        let _sigcont_handle = signal_flag::register(SIGCONT, Arc::clone(&sigcont_flag)).map_err(io::Error::other)?;

        loop {
            // Check for suspend signal (Ctrl-Z)
            if sigtstp_flag.swap(false, Ordering::Relaxed) {
                // Disable raw mode and leave alternate screen
                disable_raw_mode()?;
                execute!(io::stdout(), LeaveAlternateScreen)?;

                // Send SIGSTOP to ourselves to actually suspend
                #[cfg(unix)]
                unsafe {
                    libc::raise(libc::SIGSTOP);
                }

                // When we resume (SIGCONT received), we'll continue here
            }

            // Check for resume signal
            if sigcont_flag.swap(false, Ordering::Relaxed) {
                // Re-enable raw mode and re-enter alternate screen
                enable_raw_mode()?;
                execute!(io::stdout(), EnterAlternateScreen)?;

                // Return Continue to force a redraw
                return Ok(InputAction::Continue);
            }

            if event::poll(std::time::Duration::from_millis(100))? {
                let event = event::read()?;
                match event {
                    Event::Mouse(mouse_event) => {
                        let (x, y) = (mouse_event.column, mouse_event.row);

                        // Handle scroll wheel for Log pane
                        match mouse_event.kind {
                            MouseEventKind::ScrollUp => {
                                if let Some(info_area) = self.renderer.state.log_pane_area {
                                    if x >= info_area.x
                                        && x < info_area.x + info_area.width
                                        && y >= info_area.y
                                        && y < info_area.y + info_area.height
                                    {
                                        self.renderer.state.log_scroll_up(usize::MAX, 10);
                                        return Ok(InputAction::Continue);
                                    }
                                }
                            }
                            MouseEventKind::ScrollDown => {
                                if let Some(info_area) = self.renderer.state.log_pane_area {
                                    if x >= info_area.x
                                        && x < info_area.x + info_area.width
                                        && y >= info_area.y
                                        && y < info_area.y + info_area.height
                                    {
                                        self.renderer.state.log_scroll_down();
                                        return Ok(InputAction::Continue);
                                    }
                                }
                            }
                            MouseEventKind::ScrollLeft => {
                                // Horizontal scroll left in Log pane (when not wrapping)
                                if let Some(info_area) = self.renderer.state.log_pane_area {
                                    if x >= info_area.x
                                        && x < info_area.x + info_area.width
                                        && y >= info_area.y
                                        && y < info_area.y + info_area.height
                                        && !self.renderer.state.log_wrap_lines
                                    {
                                        self.renderer.state.log_scroll_left();
                                        return Ok(InputAction::Continue);
                                    }
                                }
                            }
                            MouseEventKind::ScrollRight => {
                                // Horizontal scroll right in Log pane (when not wrapping)
                                if let Some(info_area) = self.renderer.state.log_pane_area {
                                    if x >= info_area.x
                                        && x < info_area.x + info_area.width
                                        && y >= info_area.y
                                        && y < info_area.y + info_area.height
                                        && !self.renderer.state.log_wrap_lines
                                    {
                                        self.renderer.state.log_scroll_right();
                                        return Ok(InputAction::Continue);
                                    }
                                }
                            }
                            MouseEventKind::Down(MouseButton::Left) => {
                                // Handle left click - continue below
                            }
                            _ => {}
                        }

                        if let MouseEventKind::Down(MouseButton::Left) = mouse_event.kind {
                            // Check if Actions pane was clicked
                            if let Some(actions_area) = self.renderer.state.actions_pane_area {
                                if x >= actions_area.x
                                    && x < actions_area.x + actions_area.width
                                    && y >= actions_area.y
                                    && y < actions_area.y + actions_area.height
                                {
                                    self.renderer.state.focused_pane = FocusedPane::Actions;
                                    return Ok(InputAction::Continue); // Redraw with new focus
                                }
                            }

                            // Check if Log pane was clicked
                            if let Some(info_area) = self.renderer.state.log_pane_area {
                                if x >= info_area.x
                                    && x < info_area.x + info_area.width
                                    && y >= info_area.y
                                    && y < info_area.y + info_area.height
                                {
                                    self.renderer.state.focused_pane = FocusedPane::Log;
                                    return Ok(InputAction::Continue); // Redraw with new focus
                                }
                            }

                            // Check if Hand pane was clicked
                            if let Some(hand_area) = self.renderer.state.hand_pane_area {
                                if x >= hand_area.x
                                    && x < hand_area.x + hand_area.width
                                    && y >= hand_area.y
                                    && y < hand_area.y + hand_area.height
                                {
                                    self.renderer.state.focused_pane = FocusedPane::Hand;
                                    // Initialize selection to first card if hand not empty
                                    let hand = view.hand();
                                    if !hand.is_empty() && self.renderer.state.selected_card_in_hand.is_none() {
                                        self.renderer.state.selected_card_in_hand = Some(0);
                                        self.renderer.state.selected_card_id = Some(hand[0]);
                                    }
                                    return Ok(InputAction::Continue); // Redraw with new focus
                                }
                            }

                            // Check if any entity was clicked
                            for entity_pos in &self.renderer.state.entity_positions {
                                if x >= entity_pos.area.x
                                    && x < entity_pos.area.x + entity_pos.area.width
                                    && y >= entity_pos.area.y
                                    && y < entity_pos.area.y + entity_pos.area.height
                                {
                                    // Entity clicked! Select its representative card and show details
                                    let representative = entity_pos.entity.representative_card();
                                    self.renderer.state.selected_card_id = Some(representative);

                                    // Update battlefield selection if it's in a battlefield
                                    if let Some(card) = view.get_card(representative) {
                                        if card.controller == view.player_id() {
                                            self.renderer.state.selected_card_in_your_bf = Some(representative);
                                            self.renderer.state.focused_pane = FocusedPane::YourBattlefield;
                                        } else {
                                            self.renderer.state.selected_card_in_opp_bf = Some(representative);
                                            self.renderer.state.focused_pane = FocusedPane::OpponentBattlefield;
                                        }
                                    }

                                    return Ok(InputAction::Continue); // Redraw with new selection
                                }
                            }
                        }
                    }
                    Event::Key(key) => {
                        match key.code {
                            // Pane focus switching (H, I, Y, O, A)
                            KeyCode::Char('h' | 'H') => {
                                self.renderer.state.focused_pane = FocusedPane::Hand;
                                // Initialize selection to first card if hand not empty
                                let hand = view.hand();
                                if !hand.is_empty() && self.renderer.state.selected_card_in_hand.is_none() {
                                    self.renderer.state.selected_card_in_hand = Some(0);
                                    self.renderer.state.selected_card_id = Some(hand[0]);
                                }
                                return Ok(InputAction::Continue); // Redraw needed
                            }
                            KeyCode::Char('l' | 'L') => {
                                self.renderer.state.focused_pane = FocusedPane::Log;
                                return Ok(InputAction::Continue); // Redraw needed
                            }
                            KeyCode::Char('y' | 'Y') => {
                                self.renderer.state.focused_pane = FocusedPane::YourBattlefield;
                                // Initialize selection to first card if battlefield not empty
                                let bf_cards = FancyTuiRenderer::get_battlefield_cards_in_order(view, view.player_id());
                                if !bf_cards.is_empty() && self.renderer.state.selected_card_in_your_bf.is_none() {
                                    self.renderer.state.selected_card_in_your_bf = Some(bf_cards[0]);
                                    self.renderer.state.selected_card_id = Some(bf_cards[0]);
                                }
                                return Ok(InputAction::Continue); // Redraw needed
                            }
                            KeyCode::Char('o' | 'O') => {
                                self.renderer.state.focused_pane = FocusedPane::OpponentBattlefield;
                                // Initialize selection to first card if battlefield not empty
                                if let Some(opp_id) = view.opponents().next() {
                                    let bf_cards = FancyTuiRenderer::get_battlefield_cards_in_order(view, opp_id);
                                    if !bf_cards.is_empty() && self.renderer.state.selected_card_in_opp_bf.is_none() {
                                        self.renderer.state.selected_card_in_opp_bf = Some(bf_cards[0]);
                                        self.renderer.state.selected_card_id = Some(bf_cards[0]);
                                    }
                                }
                                return Ok(InputAction::Continue); // Redraw needed
                            }
                            KeyCode::Char('a' | 'A') => {
                                self.renderer.state.focused_pane = FocusedPane::Actions;
                                return Ok(InputAction::Continue); // Redraw needed
                            }
                            KeyCode::Char('s' | 'S') => {
                                // Stack is now part of Actions pane
                                self.renderer.state.focused_pane = FocusedPane::Actions;
                                return Ok(InputAction::Continue); // Redraw needed
                            }
                            KeyCode::Char('b' | 'B') => {
                                // Log battlefield state
                                let bf_text = crate::game::display::format_battlefield_for_log(view);
                                log::info!("{}", bf_text);
                                return Ok(InputAction::Continue);
                            }
                            // Arrow key navigation - route based on focused pane
                            KeyCode::Up | KeyCode::Char('k') => {
                                match self.renderer.state.focused_pane {
                                    FocusedPane::Actions => {
                                        // Navigate choices in Actions pane
                                        if self.renderer.state.highlighted_choice > 0 {
                                            self.renderer.state.highlighted_choice -= 1;
                                        }
                                        return Ok(InputAction::Continue);
                                    }
                                    FocusedPane::Hand => {
                                        // Navigate cards in Hand pane
                                        let hand = view.hand();
                                        if !hand.is_empty() {
                                            let current = self.renderer.state.selected_card_in_hand.unwrap_or(0);
                                            if current > 0 {
                                                self.renderer.state.selected_card_in_hand = Some(current - 1);
                                                self.renderer.state.selected_card_id = Some(hand[current - 1]);
                                            }
                                        }
                                        return Ok(InputAction::Continue);
                                    }
                                    FocusedPane::YourBattlefield => {
                                        // Navigate cards in Your Battlefield (2D: move up one row)
                                        let bf_cards =
                                            FancyTuiRenderer::get_battlefield_cards_in_order(view, view.player_id());
                                        if !bf_cards.is_empty() {
                                            let current_card = self.renderer.state.selected_card_in_your_bf;
                                            if let Some(current_idx) =
                                                current_card.and_then(|id| bf_cards.iter().position(|&c| c == id))
                                            {
                                                const CARDS_PER_ROW: usize = 4; // Estimate based on typical terminal width
                                                if current_idx >= CARDS_PER_ROW {
                                                    let new_idx = current_idx - CARDS_PER_ROW;
                                                    let new_card = bf_cards[new_idx];
                                                    self.renderer.state.selected_card_in_your_bf = Some(new_card);
                                                    self.renderer.state.selected_card_id = Some(new_card);
                                                }
                                            }
                                        }
                                        return Ok(InputAction::Continue);
                                    }
                                    FocusedPane::OpponentBattlefield => {
                                        // Navigate cards in Opponent Battlefield (2D: move up one row)
                                        if let Some(opp_id) = view.opponents().next() {
                                            let bf_cards =
                                                FancyTuiRenderer::get_battlefield_cards_in_order(view, opp_id);
                                            if !bf_cards.is_empty() {
                                                let current_card = self.renderer.state.selected_card_in_opp_bf;
                                                if let Some(current_idx) =
                                                    current_card.and_then(|id| bf_cards.iter().position(|&c| c == id))
                                                {
                                                    const CARDS_PER_ROW: usize = 4;
                                                    if current_idx >= CARDS_PER_ROW {
                                                        let new_idx = current_idx - CARDS_PER_ROW;
                                                        let new_card = bf_cards[new_idx];
                                                        self.renderer.state.selected_card_in_opp_bf = Some(new_card);
                                                        self.renderer.state.selected_card_id = Some(new_card);
                                                    }
                                                }
                                            }
                                        }
                                        return Ok(InputAction::Continue);
                                    }
                                    FocusedPane::Log => {
                                        // Scroll log up (toward older messages)
                                        self.renderer.state.log_scroll_up(usize::MAX, 10);
                                        return Ok(InputAction::Continue);
                                    }
                                }
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                match self.renderer.state.focused_pane {
                                    FocusedPane::Actions => {
                                        // Navigate choices in Actions pane
                                        if self.renderer.state.highlighted_choice + 1 < num_choices {
                                            self.renderer.state.highlighted_choice += 1;
                                        }
                                        return Ok(InputAction::Continue);
                                    }
                                    FocusedPane::Hand => {
                                        // Navigate cards in Hand pane
                                        let hand = view.hand();
                                        if !hand.is_empty() {
                                            let current = self.renderer.state.selected_card_in_hand.unwrap_or(0);
                                            if current + 1 < hand.len() {
                                                self.renderer.state.selected_card_in_hand = Some(current + 1);
                                                self.renderer.state.selected_card_id = Some(hand[current + 1]);
                                            }
                                        }
                                        return Ok(InputAction::Continue);
                                    }
                                    FocusedPane::YourBattlefield => {
                                        // Navigate cards in Your Battlefield (2D: move down one row)
                                        let bf_cards =
                                            FancyTuiRenderer::get_battlefield_cards_in_order(view, view.player_id());
                                        if !bf_cards.is_empty() {
                                            let current_card = self.renderer.state.selected_card_in_your_bf;
                                            if let Some(current_idx) =
                                                current_card.and_then(|id| bf_cards.iter().position(|&c| c == id))
                                            {
                                                const CARDS_PER_ROW: usize = 4;
                                                let new_idx = current_idx + CARDS_PER_ROW;
                                                if new_idx < bf_cards.len() {
                                                    let new_card = bf_cards[new_idx];
                                                    self.renderer.state.selected_card_in_your_bf = Some(new_card);
                                                    self.renderer.state.selected_card_id = Some(new_card);
                                                }
                                            }
                                        }
                                        return Ok(InputAction::Continue);
                                    }
                                    FocusedPane::OpponentBattlefield => {
                                        // Navigate cards in Opponent Battlefield (2D: move down one row)
                                        if let Some(opp_id) = view.opponents().next() {
                                            let bf_cards =
                                                FancyTuiRenderer::get_battlefield_cards_in_order(view, opp_id);
                                            if !bf_cards.is_empty() {
                                                let current_card = self.renderer.state.selected_card_in_opp_bf;
                                                if let Some(current_idx) =
                                                    current_card.and_then(|id| bf_cards.iter().position(|&c| c == id))
                                                {
                                                    const CARDS_PER_ROW: usize = 4;
                                                    let new_idx = current_idx + CARDS_PER_ROW;
                                                    if new_idx < bf_cards.len() {
                                                        let new_card = bf_cards[new_idx];
                                                        self.renderer.state.selected_card_in_opp_bf = Some(new_card);
                                                        self.renderer.state.selected_card_id = Some(new_card);
                                                    }
                                                }
                                            }
                                        }
                                        return Ok(InputAction::Continue);
                                    }
                                    FocusedPane::Log => {
                                        // Scroll log down (toward newer messages)
                                        self.renderer.state.log_scroll_down();
                                        return Ok(InputAction::Continue);
                                    }
                                }
                            }
                            KeyCode::Left => {
                                match self.renderer.state.focused_pane {
                                    FocusedPane::YourBattlefield => {
                                        // Navigate left in Your Battlefield (2D: move left with wrapping)
                                        let bf_cards =
                                            FancyTuiRenderer::get_battlefield_cards_in_order(view, view.player_id());
                                        if !bf_cards.is_empty() {
                                            let current_card = self.renderer.state.selected_card_in_your_bf;
                                            if let Some(current_idx) =
                                                current_card.and_then(|id| bf_cards.iter().position(|&c| c == id))
                                            {
                                                const CARDS_PER_ROW: usize = 4;
                                                let row = current_idx / CARDS_PER_ROW;
                                                let col = current_idx % CARDS_PER_ROW;

                                                let new_idx = if col > 0 {
                                                    // Move left within the row
                                                    current_idx - 1
                                                } else {
                                                    // Wrap to end of current row
                                                    let row_end = ((row + 1) * CARDS_PER_ROW).min(bf_cards.len());
                                                    row_end - 1
                                                };

                                                let new_card = bf_cards[new_idx];
                                                self.renderer.state.selected_card_in_your_bf = Some(new_card);
                                                self.renderer.state.selected_card_id = Some(new_card);
                                            }
                                        }
                                        return Ok(InputAction::Continue);
                                    }
                                    FocusedPane::OpponentBattlefield => {
                                        // Navigate left in Opponent Battlefield (2D: move left with wrapping)
                                        if let Some(opp_id) = view.opponents().next() {
                                            let bf_cards =
                                                FancyTuiRenderer::get_battlefield_cards_in_order(view, opp_id);
                                            if !bf_cards.is_empty() {
                                                let current_card = self.renderer.state.selected_card_in_opp_bf;
                                                if let Some(current_idx) =
                                                    current_card.and_then(|id| bf_cards.iter().position(|&c| c == id))
                                                {
                                                    const CARDS_PER_ROW: usize = 4;
                                                    let row = current_idx / CARDS_PER_ROW;
                                                    let col = current_idx % CARDS_PER_ROW;

                                                    let new_idx = if col > 0 {
                                                        current_idx - 1
                                                    } else {
                                                        let row_end = ((row + 1) * CARDS_PER_ROW).min(bf_cards.len());
                                                        row_end - 1
                                                    };

                                                    let new_card = bf_cards[new_idx];
                                                    self.renderer.state.selected_card_in_opp_bf = Some(new_card);
                                                    self.renderer.state.selected_card_id = Some(new_card);
                                                }
                                            }
                                        }
                                        return Ok(InputAction::Continue);
                                    }
                                    FocusedPane::Log => {
                                        // Scroll to previous turn header
                                        let logs = view.logger().logs();
                                        let visible_lines = self.renderer.state.log_visible_lines;
                                        self.renderer.state.log_scroll_prev_turn(&logs, visible_lines);
                                        return Ok(InputAction::Continue);
                                    }
                                    _ => {
                                        return Ok(InputAction::Continue);
                                    }
                                }
                            }
                            KeyCode::Right => {
                                match self.renderer.state.focused_pane {
                                    FocusedPane::YourBattlefield => {
                                        // Navigate right in Your Battlefield (2D: move right with wrapping)
                                        let bf_cards =
                                            FancyTuiRenderer::get_battlefield_cards_in_order(view, view.player_id());
                                        if !bf_cards.is_empty() {
                                            let current_card = self.renderer.state.selected_card_in_your_bf;
                                            if let Some(current_idx) =
                                                current_card.and_then(|id| bf_cards.iter().position(|&c| c == id))
                                            {
                                                const CARDS_PER_ROW: usize = 4;
                                                let row = current_idx / CARDS_PER_ROW;
                                                let row_start = row * CARDS_PER_ROW;
                                                let row_end = ((row + 1) * CARDS_PER_ROW).min(bf_cards.len());

                                                let new_idx = if current_idx + 1 < row_end {
                                                    // Move right within the row
                                                    current_idx + 1
                                                } else {
                                                    // Wrap to start of current row
                                                    row_start
                                                };

                                                let new_card = bf_cards[new_idx];
                                                self.renderer.state.selected_card_in_your_bf = Some(new_card);
                                                self.renderer.state.selected_card_id = Some(new_card);
                                            }
                                        }
                                        return Ok(InputAction::Continue);
                                    }
                                    FocusedPane::OpponentBattlefield => {
                                        // Navigate right in Opponent Battlefield (2D: move right with wrapping)
                                        if let Some(opp_id) = view.opponents().next() {
                                            let bf_cards =
                                                FancyTuiRenderer::get_battlefield_cards_in_order(view, opp_id);
                                            if !bf_cards.is_empty() {
                                                let current_card = self.renderer.state.selected_card_in_opp_bf;
                                                if let Some(current_idx) =
                                                    current_card.and_then(|id| bf_cards.iter().position(|&c| c == id))
                                                {
                                                    const CARDS_PER_ROW: usize = 4;
                                                    let row = current_idx / CARDS_PER_ROW;
                                                    let row_start = row * CARDS_PER_ROW;
                                                    let row_end = ((row + 1) * CARDS_PER_ROW).min(bf_cards.len());

                                                    let new_idx = if current_idx + 1 < row_end {
                                                        current_idx + 1
                                                    } else {
                                                        row_start
                                                    };

                                                    let new_card = bf_cards[new_idx];
                                                    self.renderer.state.selected_card_in_opp_bf = Some(new_card);
                                                    self.renderer.state.selected_card_id = Some(new_card);
                                                }
                                            }
                                        }
                                        return Ok(InputAction::Continue);
                                    }
                                    FocusedPane::Log => {
                                        // Scroll to next turn header
                                        let logs = view.logger().logs();
                                        let visible_lines = self.renderer.state.log_visible_lines;
                                        self.renderer.state.log_scroll_next_turn(&logs, visible_lines);
                                        return Ok(InputAction::Continue);
                                    }
                                    _ => {
                                        return Ok(InputAction::Continue);
                                    }
                                }
                            }
                            KeyCode::Enter => {
                                // In Actions pane, select the highlighted choice
                                if self.renderer.state.focused_pane == FocusedPane::Actions {
                                    return Ok(InputAction::Select(self.renderer.state.highlighted_choice));
                                }

                                // In other panes, Enter selects a card to view in Card Details
                                match self.renderer.state.focused_pane {
                                    FocusedPane::Hand => {
                                        if let Some(idx) = self.renderer.state.selected_card_in_hand {
                                            let hand = view.hand();
                                            if idx < hand.len() {
                                                self.renderer.state.selected_card_id = Some(hand[idx]);
                                            }
                                        }
                                    }
                                    FocusedPane::YourBattlefield => {
                                        if let Some(card_id) = self.renderer.state.selected_card_in_your_bf {
                                            self.renderer.state.selected_card_id = Some(card_id);
                                        }
                                    }
                                    FocusedPane::OpponentBattlefield => {
                                        if let Some(card_id) = self.renderer.state.selected_card_in_opp_bf {
                                            self.renderer.state.selected_card_id = Some(card_id);
                                        }
                                    }
                                    FocusedPane::Log | FocusedPane::Actions => {
                                        // Log pane doesn't have cards to select
                                        // Actions pane (with Stack) already handled above
                                    }
                                }

                                return Ok(InputAction::Continue);
                            }
                            KeyCode::Char('p') | KeyCode::Esc => {
                                return Ok(InputAction::Pass);
                            }
                            KeyCode::Char('q') => {
                                return Ok(InputAction::Pass);
                            }
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                return Ok(InputAction::Exit);
                            }
                            KeyCode::Char('z') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                // Ctrl-Z is now handled by SIGTSTP signal handler above
                                // No action needed here - the signal handler will suspend the process
                                return Ok(InputAction::Continue);
                            }
                            KeyCode::Char('Z') => {
                                // Shift+Z: Undo the most recent action
                                return Ok(InputAction::Undo);
                            }
                            KeyCode::Char('r' | 'R') => {
                                // R: Make a random choice
                                return Ok(InputAction::RandomChoice);
                            }
                            KeyCode::Char(c) if c.is_ascii_digit() => {
                                // Digit selection only works when Actions pane is focused
                                if self.renderer.state.focused_pane == FocusedPane::Actions {
                                    let digit = c.to_digit(10).unwrap() as usize;
                                    if digit < num_choices {
                                        return Ok(InputAction::Select(digit));
                                    }
                                }
                            }
                            KeyCode::Char('w' | 'W') => {
                                // W: Toggle line wrapping in log (only when Log pane is focused)
                                if self.renderer.state.focused_pane == FocusedPane::Log {
                                    let logs = view.logger().logs();
                                    self.renderer.state.log_toggle_wrap(logs.len());
                                    return Ok(InputAction::Continue);
                                }
                            }
                            KeyCode::PageUp => {
                                // Page up in log (only when Log pane is focused)
                                if self.renderer.state.focused_pane == FocusedPane::Log {
                                    self.renderer.state.log_page_up(usize::MAX, 10);
                                    return Ok(InputAction::Continue);
                                }
                            }
                            KeyCode::PageDown => {
                                // Page down in log (only when Log pane is focused)
                                if self.renderer.state.focused_pane == FocusedPane::Log {
                                    self.renderer.state.log_page_down(10);
                                    return Ok(InputAction::Continue);
                                }
                            }
                            KeyCode::Home => {
                                // Scroll to beginning (only when Log pane is focused)
                                if self.renderer.state.focused_pane == FocusedPane::Log {
                                    self.renderer.state.log_scroll_home(usize::MAX, 10);
                                    return Ok(InputAction::Continue);
                                }
                            }
                            KeyCode::End => {
                                // Scroll to end (only when Log pane is focused)
                                if self.renderer.state.focused_pane == FocusedPane::Log {
                                    self.renderer.state.log_scroll_end();
                                    return Ok(InputAction::Continue);
                                }
                            }
                            KeyCode::Char('?' | '/') => {
                                // Show help/keyboard shortcuts
                                return Ok(InputAction::ShowHelp);
                            }
                            _ => {}
                        }
                    }
                    Event::Resize(_, _) => {
                        // Terminal was resized - trigger a redraw
                        return Ok(InputAction::Continue);
                    }
                    _ => {}
                }
            }
        }
    }

    /// Show a choice prompt and get user selection
    fn prompt_for_choice(
        &mut self,
        view: &GameStateView,
        prompt: &str,
        choices: &[String],
    ) -> io::Result<PromptResult> {
        self.renderer.state.highlighted_choice = 0;

        let mut terminal = Self::setup_terminal()?;

        loop {
            // Prepare choices with highlighting and numbers using shared function
            let choice_tuples =
                crate::game::display::format_choices_with_numbers(choices, self.renderer.state.highlighted_choice);

            terminal.draw(|f| {
                self.renderer.draw_ui(f, view, Some(prompt), &choice_tuples);
            })?;

            match self.wait_for_choice_input(choices.len(), view)? {
                InputAction::Continue => {
                    // Arrow key pressed, continue loop to redraw
                    continue;
                }
                InputAction::Select(choice) => {
                    Self::restore_terminal(&mut terminal)?;
                    return Ok(PromptResult::Choice(Some(choice)));
                }
                InputAction::Pass => {
                    Self::restore_terminal(&mut terminal)?;
                    return Ok(PromptResult::Choice(None));
                }
                InputAction::Exit => {
                    // Ctrl-C pressed - restore terminal and exit gracefully
                    Self::restore_terminal(&mut terminal)?;
                    eprintln!("Exiting game (Ctrl-C pressed)");
                    std::process::exit(0);
                }
                InputAction::Undo => {
                    // Return undo signal to be handled at controller trait method level
                    Self::restore_terminal(&mut terminal)?;
                    return Ok(PromptResult::Undo);
                }
                InputAction::RandomChoice => {
                    // R key pressed - make a random choice
                    let choice = if choices.is_empty() {
                        None
                    } else {
                        let mut rng = rand::thread_rng();
                        let idx = rng.gen_range(0..choices.len());
                        Some(idx)
                    };
                    Self::restore_terminal(&mut terminal)?;
                    return Ok(PromptResult::Choice(choice));
                }
                InputAction::ShowHelp => {
                    // Show help - for now, log the help info
                    // TODO: Could show a modal help overlay in the future
                    continue;
                }
            }
        }
    }
}

impl PlayerController for FancyTuiController {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn choose_spell_ability_to_play(
        &mut self,
        view: &GameStateView,
        available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>> {
        if available.is_empty() {
            return ChoiceResult::Ok(None);
        }

        // Sort abilities in canonical order: PlayLand, CastSpell, ActivateAbility
        let sorted = sort_spell_abilities(available);

        // Set choice context and valid choices for highlighting
        self.renderer.state.choice_context = ChoiceContext::PlayingSpell;
        self.renderer.state.valid_choices = sorted.iter().map(SpellAbility::card_id).collect();

        let player_name = view.player_name();
        let prompt = prompt_spell_ability(&player_name);

        // Use shared formatting function for consistency with WASM
        let choices = format_spell_ability_choices(view, &sorted);

        match self.prompt_for_choice(view, &prompt, &choices) {
            Ok(PromptResult::Undo) => {
                // Clear choice context
                self.renderer.state.choice_context = ChoiceContext::None;
                self.renderer.state.valid_choices.clear();
                ChoiceResult::UndoRequest(usize::MAX)
            }
            Ok(PromptResult::Choice(choice_opt)) => {
                let result = match choice_opt {
                    Some(0) | None => None, // Pass
                    Some(idx) if idx > 0 && idx <= sorted.len() => Some(sorted[idx - 1].clone()),
                    _ => None,
                };

                // Log the choice
                if let Some(ability) = &result {
                    let choice_description = format_spell_ability_choice(view, ability);
                    view.logger()
                        .controller_choice("TUI", &format!("{} chose {}", player_name, choice_description));
                } else {
                    view.logger()
                        .controller_choice("TUI", &format!("{} chose to pass priority", player_name));
                }

                // Clear choice context after making choice
                self.renderer.state.choice_context = ChoiceContext::None;
                self.renderer.state.valid_choices.clear();

                ChoiceResult::Ok(result)
            }
            Err(e) => {
                // Clear choice context on error
                self.renderer.state.choice_context = ChoiceContext::None;
                self.renderer.state.valid_choices.clear();
                ChoiceResult::Error(format!("Failed to prompt for choice: {}", e))
            }
        }
    }

    fn choose_targets(
        &mut self,
        view: &GameStateView,
        spell: CardId,
        valid_targets: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        if valid_targets.is_empty() {
            return ChoiceResult::Ok(SmallVec::new());
        }

        // Set choice context and valid choices for highlighting
        self.renderer.state.choice_context = ChoiceContext::TargetSelection;
        self.renderer.state.valid_choices = valid_targets.to_vec();

        let spell_name = view.card_name(spell).unwrap_or_default();
        let prompt = prompt_target(&spell_name);

        // Use shared formatting function for consistency with WASM
        let choices = format_target_choices(view, valid_targets, self.player_id);

        let mut targets = SmallVec::new();
        match self.prompt_for_choice(view, &prompt, &choices) {
            Ok(PromptResult::Undo) => {
                self.renderer.state.choice_context = ChoiceContext::None;
                self.renderer.state.valid_choices.clear();
                return ChoiceResult::UndoRequest(usize::MAX);
            }
            Ok(PromptResult::Choice(Some(idx))) if idx > 0 && idx <= valid_targets.len() => {
                targets.push(valid_targets[idx - 1]);
            }
            Ok(PromptResult::Choice(_)) => {}
            Err(e) => {
                self.renderer.state.choice_context = ChoiceContext::None;
                self.renderer.state.valid_choices.clear();
                return ChoiceResult::Error(format!("Failed to prompt for choice: {}", e));
            }
        }

        // Log the choice
        if targets.is_empty() {
            view.logger().controller_choice("TUI", "Chose no target");
        } else {
            let target_names: Vec<String> = targets
                .iter()
                .map(|&card_id| view.card_name(card_id).unwrap_or_else(|| format!("{:?}", card_id)))
                .collect();
            view.logger()
                .controller_choice("TUI", &format!("chose target {}", target_names.join(", ")));
        }

        // Clear choice context after making choice
        self.renderer.state.choice_context = ChoiceContext::None;
        self.renderer.state.valid_choices.clear();

        ChoiceResult::Ok(targets)
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        view: &GameStateView,
        cost: &ManaCost,
        available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        let mut sources = SmallVec::new();
        let needed = cost.cmc() as usize;

        if needed == 0 || available_sources.is_empty() {
            return ChoiceResult::Ok(sources);
        }

        for i in 0..needed {
            let prompt = prompt_mana_source(i + 1, needed);
            // Use shared formatting for consistency with WASM
            let choices = format_card_choices(view, available_sources, self.player_id);

            match self.prompt_for_choice(view, &prompt, &choices) {
                Ok(PromptResult::Undo) => {
                    return ChoiceResult::UndoRequest(usize::MAX);
                }
                Ok(PromptResult::Choice(Some(idx))) if idx < available_sources.len() => {
                    sources.push(available_sources[idx]);
                }
                Ok(PromptResult::Choice(_)) => break,
                Err(e) => {
                    return ChoiceResult::Error(format!("Failed to prompt for mana source: {}", e));
                }
            }
        }

        ChoiceResult::Ok(sources)
    }

    fn choose_attackers(
        &mut self,
        view: &GameStateView,
        available_creatures: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        if available_creatures.is_empty() {
            return ChoiceResult::Ok(SmallVec::new());
        }

        // Set choice context and valid choices for highlighting
        self.renderer.state.choice_context = ChoiceContext::DeclareAttackers;
        self.renderer.state.valid_choices = available_creatures.to_vec();

        let mut attackers = SmallVec::new();

        loop {
            let prompt = PROMPT_ATTACKERS;
            // Use shared formatting and add selection markers
            let base_choices = format_card_choices(view, available_creatures, self.player_id);
            let choices: Vec<String> = std::iter::once("Done".to_string())
                .chain(base_choices.iter().enumerate().map(|(i, choice)| {
                    let card_id = available_creatures[i];
                    let selected = if attackers.contains(&card_id) { " [X]" } else { "" };
                    format!("{}{}", choice, selected)
                }))
                .collect();

            match self.prompt_for_choice(view, prompt, &choices) {
                Ok(PromptResult::Undo) => {
                    self.renderer.state.choice_context = ChoiceContext::None;
                    self.renderer.state.valid_choices.clear();
                    return ChoiceResult::UndoRequest(usize::MAX);
                }
                Ok(PromptResult::Choice(Some(0) | None)) => break,
                Ok(PromptResult::Choice(Some(idx))) if idx > 0 && idx <= available_creatures.len() => {
                    let card_id = available_creatures[idx - 1];
                    if !attackers.contains(&card_id) {
                        attackers.push(card_id);
                    }
                }
                Ok(PromptResult::Choice(_)) => break,
                Err(e) => {
                    self.renderer.state.choice_context = ChoiceContext::None;
                    self.renderer.state.valid_choices.clear();
                    return ChoiceResult::Error(format!("Failed to prompt for attackers: {}", e));
                }
            }
        }

        // Log the choice
        if attackers.is_empty() {
            view.logger().controller_choice(
                "TUI",
                &format!(
                    "chose not to attack with {} available creatures",
                    available_creatures.len()
                ),
            );
        } else {
            view.logger().controller_choice(
                "TUI",
                &format!(
                    "chose {} attackers from {} available creatures",
                    attackers.len(),
                    available_creatures.len()
                ),
            );
        }

        // Clear choice context after making choice
        self.renderer.state.choice_context = ChoiceContext::None;
        self.renderer.state.valid_choices.clear();

        ChoiceResult::Ok(attackers)
    }

    fn choose_blockers(
        &mut self,
        view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
        if attackers.is_empty() || available_blockers.is_empty() {
            return ChoiceResult::Ok(SmallVec::new());
        }

        // Set choice context: both blockers and attackers are valid choices
        self.renderer.state.choice_context = ChoiceContext::DeclareBlockers;
        self.renderer.state.valid_choices = available_blockers.iter().chain(attackers.iter()).copied().collect();

        let mut blocks = SmallVec::new();

        // Pre-format attackers using shared formatting
        let formatted_attackers = format_card_choices(view, attackers, self.player_id);

        // For each blocker, ask which attacker to block
        for &blocker_id in available_blockers {
            let blocker_name = view.card_name(blocker_id).unwrap_or_default();
            let prompt = format!("{}: Block which attacker?", blocker_name);

            // Use shared formatting for attacker choices
            let choices: Vec<String> = std::iter::once("Skip".to_string())
                .chain(formatted_attackers.clone())
                .collect();

            match self.prompt_for_choice(view, &prompt, &choices) {
                Ok(PromptResult::Undo) => {
                    self.renderer.state.choice_context = ChoiceContext::None;
                    self.renderer.state.valid_choices.clear();
                    return ChoiceResult::UndoRequest(usize::MAX);
                }
                Ok(PromptResult::Choice(Some(0) | None)) => continue,
                Ok(PromptResult::Choice(Some(idx))) if idx > 0 && idx <= attackers.len() => {
                    blocks.push((blocker_id, attackers[idx - 1]));
                }
                Ok(PromptResult::Choice(_)) => break,
                Err(e) => {
                    self.renderer.state.choice_context = ChoiceContext::None;
                    self.renderer.state.valid_choices.clear();
                    return ChoiceResult::Error(format!("Failed to prompt for blockers: {}", e));
                }
            }
        }

        // Log the choice
        if blocks.is_empty() {
            view.logger().controller_choice(
                "TUI",
                &format!(
                    "chose not to block (no favorable blocks among {} blockers vs {} attackers)",
                    available_blockers.len(),
                    attackers.len()
                ),
            );
        } else {
            view.logger().controller_choice(
                "TUI",
                &format!("chose {} blockers for {} attackers", blocks.len(), attackers.len()),
            );
        }

        // Clear choice context after making choice
        self.renderer.state.choice_context = ChoiceContext::None;
        self.renderer.state.valid_choices.clear();

        ChoiceResult::Ok(blocks)
    }

    fn choose_damage_assignment_order(
        &mut self,
        _view: &GameStateView,
        _attacker: CardId,
        blockers: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        // For simplicity, just return blockers in order
        // TODO: implement UI for reordering
        ChoiceResult::Ok(blockers.iter().copied().collect())
    }

    fn choose_cards_to_discard(
        &mut self,
        view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        let mut discards = SmallVec::new();

        for i in 0..count {
            let prompt = format!("Discard card {}/{}", i + 1, count);
            let choices: Vec<String> = hand
                .iter()
                .filter(|&card_id| !discards.contains(card_id))
                .map(|&card_id| view.card_name(card_id).unwrap_or_default())
                .collect();

            if choices.is_empty() {
                break;
            }

            match self.prompt_for_choice(view, &prompt, &choices) {
                Ok(PromptResult::Undo) => {
                    return ChoiceResult::UndoRequest(usize::MAX);
                }
                Ok(PromptResult::Choice(Some(idx))) if idx < hand.len() => {
                    let card_id = hand
                        .iter()
                        .filter(|&card_id| !discards.contains(card_id))
                        .nth(idx)
                        .copied();
                    if let Some(card_id) = card_id {
                        discards.push(card_id);
                    }
                }
                Ok(PromptResult::Choice(_)) => break,
                Err(e) => {
                    return ChoiceResult::Error(format!("Failed to prompt for discard: {}", e));
                }
            }
        }

        ChoiceResult::Ok(discards)
    }

    fn choose_from_library(
        &mut self,
        view: &GameStateView,
        valid_cards: &[&crate::loader::CardDefinition],
    ) -> ChoiceResult<Option<usize>> {
        if valid_cards.is_empty() {
            return ChoiceResult::Ok(None);
        }

        let prompt = "Search library: Choose a card";
        let choices: Vec<String> = std::iter::once("Fail to find".to_string())
            .chain(valid_cards.iter().map(|&def| def.name.to_string()))
            .collect();

        match self.prompt_for_choice(view, prompt, &choices) {
            Ok(PromptResult::Undo) => ChoiceResult::UndoRequest(usize::MAX),
            Ok(PromptResult::Choice(Some(0) | None)) => {
                view.logger().controller_choice("TUI", "Chose to fail to find");
                ChoiceResult::Ok(None)
            }
            Ok(PromptResult::Choice(Some(idx))) if idx > 0 && idx <= valid_cards.len() => {
                let def = valid_cards[idx - 1];
                view.logger()
                    .controller_choice("TUI", &format!("Chose {} from library", def.name));
                ChoiceResult::Ok(Some(idx - 1))
            }
            Ok(PromptResult::Choice(_)) => ChoiceResult::Ok(None),
            Err(e) => ChoiceResult::Error(format!("Failed to prompt for library choice: {}", e)),
        }
    }

    fn choose_permanents_to_sacrifice(
        &mut self,
        view: &GameStateView,
        valid_permanents: &[CardId],
        count: usize,
        card_type_description: &str,
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        if valid_permanents.is_empty() || count == 0 {
            return ChoiceResult::Ok(SmallVec::new());
        }

        let mut sacrifices: SmallVec<[CardId; 8]> = SmallVec::new();

        while sacrifices.len() < count {
            let remaining = count - sacrifices.len();
            let prompt = format!(
                "Sacrifice {} {}: Choose {} more",
                card_type_description, remaining, remaining
            );

            // Build choices from remaining (not yet selected) permanents
            let available: Vec<_> = valid_permanents
                .iter()
                .filter(|&card_id| !sacrifices.contains(card_id))
                .collect();

            if available.is_empty() {
                break;
            }

            let choices: Vec<String> = available
                .iter()
                .map(|&&card_id| view.card_name(card_id).unwrap_or_else(|| format!("{:?}", card_id)))
                .collect();

            match self.prompt_for_choice(view, &prompt, &choices) {
                Ok(PromptResult::Undo) => {
                    return ChoiceResult::UndoRequest(usize::MAX);
                }
                Ok(PromptResult::Choice(Some(idx))) if idx < available.len() => {
                    sacrifices.push(*available[idx]);
                }
                Ok(PromptResult::Choice(_)) => {
                    // Invalid choice, try again
                    continue;
                }
                Err(e) => {
                    return ChoiceResult::Error(format!("Failed to prompt for sacrifice: {}", e));
                }
            }
        }

        let names: Vec<String> = sacrifices.iter().filter_map(|&id| view.card_name(id)).collect();
        view.logger().controller_choice(
            "TUI",
            &format!("Chose to sacrifice {}: [{}]", card_type_description, names.join(", ")),
        );

        ChoiceResult::Ok(sacrifices)
    }

    fn choose_permanents_to_not_untap(
        &mut self,
        _view: &GameStateView,
        may_not_untap_permanents: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // For interactive TUI, could prompt user to select which permanents to keep tapped
        // For now, default to untapping everything (return empty list)
        // TODO: Implement interactive selection UI for this choice
        if !may_not_untap_permanents.is_empty() {
            eprintln!(
                "[TUI] Auto-untapping {} permanents with MayNotUntap (interactive selection not yet implemented)",
                may_not_untap_permanents.len()
            );
        }
        ChoiceResult::Ok(SmallVec::new())
    }

    fn choose_modes(
        &mut self,
        view: &GameStateView,
        spell_id: CardId,
        mode_descriptions: &[String],
        mode_count: usize,
        min_modes: usize,
        _can_repeat: bool,
    ) -> ChoiceResult<SmallVec<[usize; 4]>> {
        // Interactive mode selection for fancy TUI
        let spell_name = view
            .get_card_name(spell_id)
            .unwrap_or_else(|| "Unknown Spell".to_string());

        eprintln!(
            "\n=== Choose {} Mode{} for {} ===",
            mode_count,
            if mode_count > 1 { "s" } else { "" },
            spell_name
        );
        eprintln!("Minimum modes required: {}", min_modes);

        for (idx, desc) in mode_descriptions.iter().enumerate() {
            eprintln!("  [{}] {}", idx, desc);
        }

        // For now, default to first N modes since fancy TUI uses key-based input
        // TODO: Add proper interactive mode selection UI
        eprintln!(
            "[TUI] Auto-selecting first {} mode(s) (interactive selection not yet implemented)",
            mode_count
        );
        ChoiceResult::Ok((0..mode_count.min(mode_descriptions.len())).collect())
    }

    fn on_priority_passed(&mut self, _view: &GameStateView) {
        // Logging is handled by the game logger, no local state tracking needed
    }

    fn on_game_end(&mut self, _view: &GameStateView, _won: bool) {
        // Logging is handled by the game logger, no local state tracking needed
    }

    fn get_controller_type(&self) -> crate::game::snapshot::ControllerType {
        // Fancy TUI is treated as a variant of the TUI controller
        crate::game::snapshot::ControllerType::Tui
    }
}
