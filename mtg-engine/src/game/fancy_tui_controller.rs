//! Fancy TUI controller with full-screen ratatui interface
//!
//! This controller provides a rich, multi-panel TUI interface similar to MTG Arena,
//! with separate panels for battlefield, hand, card details, prompts, and game state.
//!
//! The UI rendering is delegated to [`FancyTuiRenderer`] which is shared with the
//! WASM browser implementation for exact visual parity.
//!
//! Event handling is delegated to shared handlers in [`fancy_tui_events`], which are
//! also used by the WASM implementation. This controller converts crossterm events
//! to backend-neutral [`UiEvent`] and dispatches them through the shared handlers.

use crate::core::{CardId, ManaCost, PlayerId, SpellAbility};
use crate::game::controller::{
    format_card_choices, format_spell_ability_choice, format_spell_ability_choices, format_target_choices,
    prompt_mana_source, prompt_spell_ability, prompt_target, sort_spell_abilities, ChoiceResult, GameStateView,
    PlayerController, PROMPT_ATTACKERS,
};
use crate::game::fancy_tui_events::{handle_ui_event, EventResult, KeyInput, ScrollDirection, UiEvent};
use crate::game::fancy_tui_renderer::{ChoiceContext, FancyTuiRenderer};
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
    /// Card image state for terminal-native image rendering
    #[cfg(feature = "ratatui-image")]
    card_image: crate::game::card_image::CardImageState,
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
            #[cfg(feature = "ratatui-image")]
            card_image: crate::game::card_image::CardImageState::new(),
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

        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
        terminal.show_cursor()?;
        terminal.backend_mut().flush()?;

        Ok(())
    }

    /// Configure game logger for memory-only mode (suppress stdout)
    pub fn configure_logger_for_tui(&mut self, _view: &GameStateView) {
        self.logger_memory_mode_enabled = true;
    }

    /// Save buffered logs to a temp file and print the location
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

        let temp_dir = std::env::temp_dir();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let log_path = temp_dir.join(format!("mtg_forge_game_{}.log", timestamp));

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
    /// Reads crossterm events, converts them to backend-neutral `UiEvent`,
    /// dispatches through shared handlers, and maps `EventResult` to `InputAction`.
    #[allow(clippy::wildcard_enum_match_arm)]
    fn wait_for_choice_input(&mut self, num_choices: usize, view: &GameStateView) -> io::Result<InputAction> {
        let sigtstp_flag = Arc::new(AtomicBool::new(false));
        let sigcont_flag = Arc::new(AtomicBool::new(false));
        let _sigtstp_handle = signal_flag::register(SIGTSTP, Arc::clone(&sigtstp_flag)).map_err(io::Error::other)?;
        let _sigcont_handle = signal_flag::register(SIGCONT, Arc::clone(&sigcont_flag)).map_err(io::Error::other)?;

        loop {
            if sigtstp_flag.swap(false, Ordering::Relaxed) {
                disable_raw_mode()?;
                execute!(io::stdout(), LeaveAlternateScreen)?;
                #[cfg(unix)]
                unsafe {
                    libc::raise(libc::SIGSTOP);
                }
            }

            if sigcont_flag.swap(false, Ordering::Relaxed) {
                enable_raw_mode()?;
                execute!(io::stdout(), EnterAlternateScreen)?;
                return Ok(InputAction::Continue);
            }

            if event::poll(std::time::Duration::from_millis(100))? {
                let raw_event = event::read()?;

                let ui_event = match raw_event {
                    Event::Key(key) => {
                        if key.code == KeyCode::Char('z') && key.modifiers.contains(KeyModifiers::CONTROL) {
                            continue;
                        }
                        match crossterm_key_to_input(key.code, key.modifiers) {
                            Some(key_input) => UiEvent::Key(key_input),
                            None => continue,
                        }
                    }
                    Event::Mouse(mouse) => {
                        let (col, row) = (mouse.column, mouse.row);
                        match mouse.kind {
                            MouseEventKind::ScrollUp => UiEvent::MouseWheel {
                                direction: ScrollDirection::Up,
                                col,
                                row,
                            },
                            MouseEventKind::ScrollDown => UiEvent::MouseWheel {
                                direction: ScrollDirection::Down,
                                col,
                                row,
                            },
                            MouseEventKind::ScrollLeft => UiEvent::MouseWheel {
                                direction: ScrollDirection::Left,
                                col,
                                row,
                            },
                            MouseEventKind::ScrollRight => UiEvent::MouseWheel {
                                direction: ScrollDirection::Right,
                                col,
                                row,
                            },
                            MouseEventKind::Down(MouseButton::Left) => UiEvent::MouseClick { col, row },
                            _ => continue,
                        }
                    }
                    Event::Resize(w, h) => UiEvent::Resize { width: w, height: h },
                    _ => continue,
                };

                let result = handle_ui_event(&mut self.renderer.state, ui_event, view, num_choices);

                match result {
                    EventResult::Handled => return Ok(InputAction::Continue),
                    EventResult::NotHandled => continue,
                    EventResult::Pass => return Ok(InputAction::Pass),
                    EventResult::Exit => return Ok(InputAction::Exit),
                    EventResult::Undo => return Ok(InputAction::Undo),
                    EventResult::RandomChoice => return Ok(InputAction::RandomChoice),
                    EventResult::SelectChoice(idx) => return Ok(InputAction::Select(idx)),
                    EventResult::ShowBattlefield => {
                        let bf_text = crate::game::display::format_battlefield_for_log(view);
                        log::info!("{}", bf_text);
                        return Ok(InputAction::Continue);
                    }
                    EventResult::ShowHelp => return Ok(InputAction::ShowHelp),
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
        self.renderer.state.digit_buffer.clear();

        let mut terminal = Self::setup_terminal()?;

        loop {
            let choice_tuples =
                crate::game::display::format_choices_with_numbers(choices, self.renderer.state.highlighted_choice);

            terminal.draw(|f| {
                self.renderer.draw_ui(f, view, Some(prompt), &choice_tuples);
                // Render card image in the card details pane if available
                #[cfg(feature = "ratatui-image")]
                if let Some(area) = self.renderer.state.card_details_pane_area {
                    let card_name = self.renderer.state.selected_card_id.and_then(|id| view.card_name(id));
                    if self
                        .card_image
                        .update_for_card(self.renderer.state.selected_card_id, card_name.as_deref())
                    {
                        self.card_image.render(f, area);
                    }
                }
            })?;

            match self.wait_for_choice_input(choices.len(), view)? {
                InputAction::Continue => continue,
                InputAction::Select(choice) => {
                    Self::restore_terminal(&mut terminal)?;
                    return Ok(PromptResult::Choice(Some(choice)));
                }
                InputAction::Pass => {
                    Self::restore_terminal(&mut terminal)?;
                    return Ok(PromptResult::Choice(None));
                }
                InputAction::Exit => {
                    Self::restore_terminal(&mut terminal)?;
                    eprintln!("Exiting game (Ctrl-C pressed)");
                    std::process::exit(0);
                }
                InputAction::Undo => {
                    Self::restore_terminal(&mut terminal)?;
                    return Ok(PromptResult::Undo);
                }
                InputAction::RandomChoice => {
                    let choice = if choices.is_empty() {
                        None
                    } else {
                        let mut rng = rand::thread_rng();
                        Some(rng.gen_range(0..choices.len()))
                    };
                    Self::restore_terminal(&mut terminal)?;
                    return Ok(PromptResult::Choice(choice));
                }
                InputAction::ShowHelp => continue,
            }
        }
    }
}

/// Convert crossterm key event to backend-neutral `KeyInput`.
#[allow(clippy::wildcard_enum_match_arm)]
fn crossterm_key_to_input(code: KeyCode, modifiers: KeyModifiers) -> Option<KeyInput> {
    match code {
        KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => Some(KeyInput::CtrlC),
        KeyCode::Char('h' | 'H') => Some(KeyInput::FocusHand),
        KeyCode::Char('i' | 'I') => Some(KeyInput::FocusInfo),
        KeyCode::Char('l' | 'L') => Some(KeyInput::FocusInfo), // legacy binding
        KeyCode::Char('y' | 'Y') => Some(KeyInput::FocusYourBf),
        KeyCode::Char('o' | 'O') => Some(KeyInput::FocusOpponentBf),
        KeyCode::Char('a' | 'A') => Some(KeyInput::FocusActions),
        KeyCode::Char('s' | 'S') => Some(KeyInput::FocusStack),
        KeyCode::Char('p' | 'q' | 'Q') => Some(KeyInput::Pass),
        KeyCode::Char('Z') => Some(KeyInput::Undo),
        KeyCode::Char('r' | 'R') => Some(KeyInput::Random),
        KeyCode::Char('b' | 'B') => Some(KeyInput::ShowBattlefield),
        KeyCode::Char('w' | 'W') => Some(KeyInput::ToggleWrap),
        KeyCode::Char('?' | '/') => Some(KeyInput::Help),
        KeyCode::Up | KeyCode::Char('k') => Some(KeyInput::Up),
        KeyCode::Down | KeyCode::Char('j') => Some(KeyInput::Down),
        KeyCode::Left => Some(KeyInput::Left),
        KeyCode::Right => Some(KeyInput::Right),
        KeyCode::Tab => Some(KeyInput::Tab),
        KeyCode::Enter => Some(KeyInput::Enter),
        KeyCode::Esc => Some(KeyInput::Escape),
        KeyCode::Char(' ') => Some(KeyInput::Space),
        KeyCode::PageUp => Some(KeyInput::PageUp),
        KeyCode::PageDown => Some(KeyInput::PageDown),
        KeyCode::Home => Some(KeyInput::Home),
        KeyCode::End => Some(KeyInput::End),
        KeyCode::Backspace => Some(KeyInput::Backspace),
        KeyCode::Char(c) if c.is_ascii_digit() => Some(KeyInput::Digit(c.to_digit(10).unwrap() as u8)),
        _ => None,
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

        let sorted = sort_spell_abilities(available);
        self.renderer.state.choice_context = ChoiceContext::PlayingSpell;
        self.renderer.state.valid_choices = sorted.iter().map(SpellAbility::card_id).collect();

        let player_name = view.player_name();
        let prompt = prompt_spell_ability(&player_name);
        let choices = format_spell_ability_choices(view, &sorted);

        match self.prompt_for_choice(view, &prompt, &choices) {
            Ok(PromptResult::Undo) => {
                self.renderer.state.choice_context = ChoiceContext::None;
                self.renderer.state.valid_choices.clear();
                ChoiceResult::UndoRequest(usize::MAX)
            }
            Ok(PromptResult::Choice(choice_opt)) => {
                let result = match choice_opt {
                    Some(0) | None => None,
                    Some(idx) if idx > 0 && idx <= sorted.len() => Some(sorted[idx - 1].clone()),
                    _ => None,
                };

                if let Some(ability) = &result {
                    let choice_description = format_spell_ability_choice(view, ability);
                    view.logger()
                        .controller_choice("TUI", &format!("{} chose {}", player_name, choice_description));
                } else {
                    view.logger()
                        .controller_choice("TUI", &format!("{} chose to pass priority", player_name));
                }

                self.renderer.state.choice_context = ChoiceContext::None;
                self.renderer.state.valid_choices.clear();
                ChoiceResult::Ok(result)
            }
            Err(e) => {
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

        self.renderer.state.choice_context = ChoiceContext::TargetSelection;
        self.renderer.state.valid_choices = valid_targets.to_vec();

        let spell_name = view.card_name(spell).unwrap_or_default();
        let prompt = prompt_target(&spell_name);
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
            let choices = format_card_choices(view, available_sources, self.player_id);

            match self.prompt_for_choice(view, &prompt, &choices) {
                Ok(PromptResult::Undo) => return ChoiceResult::UndoRequest(usize::MAX),
                Ok(PromptResult::Choice(Some(idx))) if idx < available_sources.len() => {
                    sources.push(available_sources[idx]);
                }
                Ok(PromptResult::Choice(_)) => break,
                Err(e) => return ChoiceResult::Error(format!("Failed to prompt for mana source: {}", e)),
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

        self.renderer.state.choice_context = ChoiceContext::DeclareAttackers;
        self.renderer.state.valid_choices = available_creatures.to_vec();
        let mut attackers = SmallVec::new();

        loop {
            let prompt = PROMPT_ATTACKERS;
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

        self.renderer.state.choice_context = ChoiceContext::DeclareBlockers;
        self.renderer.state.valid_choices = available_blockers.iter().chain(attackers.iter()).copied().collect();
        let mut blocks = SmallVec::new();
        let formatted_attackers = format_card_choices(view, attackers, self.player_id);

        for &blocker_id in available_blockers {
            let blocker_name = view.card_name(blocker_id).unwrap_or_default();
            let prompt = format!("{}: Block which attacker?", blocker_name);
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
                Ok(PromptResult::Undo) => return ChoiceResult::UndoRequest(usize::MAX),
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
                Err(e) => return ChoiceResult::Error(format!("Failed to prompt for discard: {}", e)),
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
                Ok(PromptResult::Undo) => return ChoiceResult::UndoRequest(usize::MAX),
                Ok(PromptResult::Choice(Some(idx))) if idx < available.len() => {
                    sacrifices.push(*available[idx]);
                }
                Ok(PromptResult::Choice(_)) => continue,
                Err(e) => return ChoiceResult::Error(format!("Failed to prompt for sacrifice: {}", e)),
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
        eprintln!(
            "[TUI] Auto-selecting first {} mode(s) (interactive selection not yet implemented)",
            mode_count
        );
        ChoiceResult::Ok((0..mode_count.min(mode_descriptions.len())).collect())
    }

    fn on_priority_passed(&mut self, _view: &GameStateView) {}
    fn on_game_end(&mut self, _view: &GameStateView, _won: bool) {}

    fn get_controller_type(&self) -> crate::game::snapshot::ControllerType {
        crate::game::snapshot::ControllerType::Tui
    }
}
