//! Fancy TUI controller with full-screen ratatui interface
//!
//! This controller provides a rich, multi-panel TUI interface similar to MTG Arena,
//! with separate panels for battlefield, hand, card details, prompts, and game state.
//!
//! The rendering logic is shared via the `TuiRenderer` module, which can work with
//! any ratatui backend. This controller specifically uses CrosstermBackend for
//! interactive terminal rendering.

use crate::core::{CardId, ManaCost, PlayerId, SpellAbility};
use crate::game::controller::{ChoiceResult, GameStateView, PlayerController};
use crate::game::fancy_tui_renderer::{BattlefieldEntity, ChoiceContext, FocusedPane, TuiRenderer};
use crate::game::snapshot::ControllerType;
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
}

/// Result from prompting for a choice
enum PromptResult {
    /// User made a normal choice
    Choice(Option<usize>),
    /// User requested undo
    Undo,
}

/// A controller that provides a rich TUI interface using ratatui
pub struct FancyTuiController {
    player_id: PlayerId,
    renderer: TuiRenderer,
}

impl FancyTuiController {
    /// Create a new fancy TUI controller
    pub fn new(player_id: PlayerId, visual_stacks: bool) -> io::Result<Self> {
        Ok(FancyTuiController {
            player_id,
            renderer: TuiRenderer::new(visual_stacks),
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
        self.renderer.state_mut().logger_memory_mode_enabled = true;
    }

    /// Save buffered logs to a temp file and print the location
    pub fn save_logs_on_exit(&self, view: &GameStateView) -> io::Result<()> {
        if !self.renderer.state().logger_memory_mode_enabled {
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
                let event = event::read()?;
                match event {
                    Event::Mouse(mouse_event) => {
                        if let MouseEventKind::Down(MouseButton::Left) = mouse_event.kind {
                            let (x, y) = (mouse_event.column, mouse_event.row);

                            if let Some(actions_area) = self.renderer.state().actions_pane_area {
                                if x >= actions_area.x
                                    && x < actions_area.x + actions_area.width
                                    && y >= actions_area.y
                                    && y < actions_area.y + actions_area.height
                                {
                                    self.renderer.state_mut().focused_pane = FocusedPane::Actions;
                                    return Ok(InputAction::Continue);
                                }
                            }

                            if let Some(hand_area) = self.renderer.state().hand_pane_area {
                                if x >= hand_area.x
                                    && x < hand_area.x + hand_area.width
                                    && y >= hand_area.y
                                    && y < hand_area.y + hand_area.height
                                {
                                    self.renderer.state_mut().focused_pane = FocusedPane::Hand;
                                    let hand = view.hand();
                                    if !hand.is_empty() && self.renderer.state().selected_card_in_hand.is_none() {
                                        self.renderer.state_mut().selected_card_in_hand = Some(0);
                                        self.renderer.state_mut().selected_card_id = Some(hand[0]);
                                    }
                                    return Ok(InputAction::Continue);
                                }
                            }

                            for entity_pos in &self.renderer.state().entity_positions {
                                if x >= entity_pos.area.x
                                    && x < entity_pos.area.x + entity_pos.area.width
                                    && y >= entity_pos.area.y
                                    && y < entity_pos.area.y + entity_pos.area.height
                                {
                                    let representative = entity_pos.entity.representative_card();
                                    self.renderer.state_mut().selected_card_id = Some(representative);

                                    if let Some(card) = view.get_card(representative) {
                                        if card.controller == view.player_id() {
                                            self.renderer.state_mut().selected_card_in_your_bf = Some(representative);
                                            self.renderer.state_mut().focused_pane = FocusedPane::YourBattlefield;
                                        } else {
                                            self.renderer.state_mut().selected_card_in_opp_bf = Some(representative);
                                            self.renderer.state_mut().focused_pane = FocusedPane::OpponentBattlefield;
                                        }
                                    }

                                    return Ok(InputAction::Continue);
                                }
                            }
                        }
                    }
                    Event::Key(key) => {
                        match key.code {
                            KeyCode::Char('h') | KeyCode::Char('H') => {
                                self.renderer.state_mut().focused_pane = FocusedPane::Hand;
                                let hand = view.hand();
                                if !hand.is_empty() && self.renderer.state().selected_card_in_hand.is_none() {
                                    self.renderer.state_mut().selected_card_in_hand = Some(0);
                                    self.renderer.state_mut().selected_card_id = Some(hand[0]);
                                }
                                return Ok(InputAction::Continue);
                            }
                            KeyCode::Char('i') | KeyCode::Char('I') => {
                                self.renderer.state_mut().focused_pane = FocusedPane::Info;
                                return Ok(InputAction::Continue);
                            }
                            KeyCode::Char('y') | KeyCode::Char('Y') => {
                                self.renderer.state_mut().focused_pane = FocusedPane::YourBattlefield;
                                // Initialize selection - use get_battlefield_cards_in_order equivalent
                                let battlefield = view.battlefield();
                                let player_cards: Vec<CardId> = battlefield
                                    .iter()
                                    .filter(|&&card_id| {
                                        view.get_card(card_id)
                                            .map(|c| c.controller == view.player_id())
                                            .unwrap_or(false)
                                    })
                                    .copied()
                                    .collect();

                                if !player_cards.is_empty() && self.renderer.state().selected_card_in_your_bf.is_none()
                                {
                                    let first_card = player_cards[0];
                                    self.renderer.state_mut().selected_card_in_your_bf = Some(first_card);
                                    self.renderer.state_mut().selected_card_id = Some(first_card);
                                }
                                return Ok(InputAction::Continue);
                            }
                            KeyCode::Char('o') | KeyCode::Char('O') => {
                                self.renderer.state_mut().focused_pane = FocusedPane::OpponentBattlefield;
                                // Initialize selection for opponent battlefield
                                if let Some(opp_id) = view.opponents().next() {
                                    let battlefield = view.battlefield();
                                    let opp_cards: Vec<CardId> = battlefield
                                        .iter()
                                        .filter(|&&card_id| {
                                            view.get_card(card_id).map(|c| c.controller == opp_id).unwrap_or(false)
                                        })
                                        .copied()
                                        .collect();

                                    if !opp_cards.is_empty() && self.renderer.state().selected_card_in_opp_bf.is_none()
                                    {
                                        let first_card = opp_cards[0];
                                        self.renderer.state_mut().selected_card_in_opp_bf = Some(first_card);
                                        self.renderer.state_mut().selected_card_id = Some(first_card);
                                    }
                                }
                                return Ok(InputAction::Continue);
                            }
                            KeyCode::Char('a') | KeyCode::Char('A') => {
                                self.renderer.state_mut().focused_pane = FocusedPane::Actions;
                                return Ok(InputAction::Continue);
                            }
                            KeyCode::Char('s') | KeyCode::Char('S') => {
                                self.renderer.state_mut().focused_pane = FocusedPane::Stack;
                                return Ok(InputAction::Continue);
                            }
                            KeyCode::Up if self.renderer.state().focused_pane == FocusedPane::Actions => {
                                let state = self.renderer.state_mut();
                                if num_choices > 0 && state.highlighted_choice > 0 {
                                    state.highlighted_choice -= 1;
                                }
                                return Ok(InputAction::Continue);
                            }
                            KeyCode::Down if self.renderer.state().focused_pane == FocusedPane::Actions => {
                                let state = self.renderer.state_mut();
                                if num_choices > 0 && state.highlighted_choice + 1 < num_choices {
                                    state.highlighted_choice += 1;
                                }
                                return Ok(InputAction::Continue);
                            }
                            KeyCode::Up if self.renderer.state().focused_pane == FocusedPane::Hand => {
                                let hand = view.hand();
                                if !hand.is_empty() {
                                    let state = self.renderer.state_mut();
                                    let current = state.selected_card_in_hand.unwrap_or(0);
                                    if current > 0 {
                                        let new_idx = current - 1;
                                        state.selected_card_in_hand = Some(new_idx);
                                        state.selected_card_id = Some(hand[new_idx]);
                                    }
                                }
                                return Ok(InputAction::Continue);
                            }
                            KeyCode::Down if self.renderer.state().focused_pane == FocusedPane::Hand => {
                                let hand = view.hand();
                                if !hand.is_empty() {
                                    let state = self.renderer.state_mut();
                                    let current = state.selected_card_in_hand.unwrap_or(0);
                                    if current + 1 < hand.len() {
                                        let new_idx = current + 1;
                                        state.selected_card_in_hand = Some(new_idx);
                                        state.selected_card_id = Some(hand[new_idx]);
                                    }
                                }
                                return Ok(InputAction::Continue);
                            }
                            KeyCode::Enter if self.renderer.state().focused_pane == FocusedPane::Actions => {
                                return Ok(InputAction::Select(self.renderer.state().highlighted_choice));
                            }
                            KeyCode::Char('p') | KeyCode::Char('P') => {
                                return Ok(InputAction::Pass);
                            }
                            KeyCode::Char('z') | KeyCode::Char('Z') if key.modifiers.is_empty() => {
                                return Ok(InputAction::Undo);
                            }
                            KeyCode::Char('r') | KeyCode::Char('R') => {
                                return Ok(InputAction::RandomChoice);
                            }
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                return Ok(InputAction::Exit);
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn prompt_for_choice(
        &mut self,
        view: &GameStateView,
        prompt: &str,
        choices: &[String],
    ) -> io::Result<PromptResult> {
        self.renderer.state_mut().highlighted_choice = 0;

        let mut terminal = Self::setup_terminal()?;

        loop {
            let choice_tuples: Vec<(String, bool)> = choices
                .iter()
                .enumerate()
                .map(|(idx, text)| {
                    let numbered_text = format!("[{}] {}", idx, text);
                    (numbered_text, idx == self.renderer.state().highlighted_choice)
                })
                .collect();

            terminal.draw(|f| {
                self.renderer.draw_ui(f, view, Some(prompt), &choice_tuples);
            })?;

            match self.wait_for_choice_input(choices.len(), view)? {
                InputAction::Continue => {
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
                        let idx = rng.gen_range(0..choices.len());
                        Some(idx)
                    };
                    Self::restore_terminal(&mut terminal)?;
                    return Ok(PromptResult::Choice(choice));
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

        self.renderer.state_mut().choice_context = ChoiceContext::PlayingSpell;
        self.renderer.state_mut().valid_choices = available
            .iter()
            .map(|ability| match ability {
                SpellAbility::PlayLand { card_id } => *card_id,
                SpellAbility::CastSpell { card_id } => *card_id,
                SpellAbility::ActivateAbility { card_id, .. } => *card_id,
            })
            .collect();

        let player_name = view.player_name();
        let prompt = format!("Priority {}: Choose action", player_name);

        let choices: Vec<String> = std::iter::once("Pass".to_string())
            .chain(available.iter().map(|ability| match ability {
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
                    format!("Activate: {}", name)
                }
            }))
            .collect();

        match self.prompt_for_choice(view, &prompt, &choices) {
            Ok(PromptResult::Undo) => {
                self.renderer.state_mut().choice_context = ChoiceContext::None;
                self.renderer.state_mut().valid_choices.clear();
                ChoiceResult::UndoRequest(usize::MAX)
            }
            Ok(PromptResult::Choice(choice_opt)) => {
                let result = match choice_opt {
                    Some(0) | None => None,
                    Some(idx) if idx > 0 && idx <= available.len() => Some(available[idx - 1].clone()),
                    _ => None,
                };

                if let Some(ability) = &result {
                    let choice_description = match ability {
                        SpellAbility::PlayLand { card_id } => {
                            let name = view.card_name(*card_id).unwrap_or_default();
                            format!("play land: {}", name)
                        }
                        SpellAbility::CastSpell { card_id } => {
                            let name = view.card_name(*card_id).unwrap_or_default();
                            format!("cast spell: {}", name)
                        }
                        SpellAbility::ActivateAbility { card_id, .. } => {
                            let name = view.card_name(*card_id).unwrap_or_default();
                            format!("activate: {}", name)
                        }
                    };
                    view.logger()
                        .controller_choice("TUI", &format!("{} chose {}", player_name, choice_description));
                } else {
                    view.logger()
                        .controller_choice("TUI", &format!("{} chose to pass priority", player_name));
                }

                self.renderer.state_mut().choice_context = ChoiceContext::None;
                self.renderer.state_mut().valid_choices.clear();

                ChoiceResult::Ok(result)
            }
            Err(e) => {
                self.renderer.state_mut().choice_context = ChoiceContext::None;
                self.renderer.state_mut().valid_choices.clear();
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

        self.renderer.state_mut().choice_context = ChoiceContext::TargetSelection;
        self.renderer.state_mut().valid_choices = valid_targets.to_vec();

        let spell_name = view.card_name(spell).unwrap_or_else(|| format!("Card {:?}", spell));
        let prompt = format!("Choose target for {}", spell_name);

        let choices: Vec<String> = valid_targets
            .iter()
            .map(|&card_id| view.card_name(card_id).unwrap_or_else(|| format!("Card {:?}", card_id)))
            .collect();

        match self.prompt_for_choice(view, &prompt, &choices) {
            Ok(PromptResult::Undo) => {
                self.renderer.state_mut().choice_context = ChoiceContext::None;
                self.renderer.state_mut().valid_choices.clear();
                ChoiceResult::UndoRequest(usize::MAX)
            }
            Ok(PromptResult::Choice(choice_opt)) => {
                let result = match choice_opt {
                    Some(idx) if idx < valid_targets.len() => {
                        let mut targets = SmallVec::new();
                        targets.push(valid_targets[idx]);
                        targets
                    }
                    _ => SmallVec::new(),
                };

                self.renderer.state_mut().choice_context = ChoiceContext::None;
                self.renderer.state_mut().valid_choices.clear();

                ChoiceResult::Ok(result)
            }
            Err(e) => {
                self.renderer.state_mut().choice_context = ChoiceContext::None;
                self.renderer.state_mut().valid_choices.clear();
                ChoiceResult::Error(format!("Failed to prompt for target: {}", e))
            }
        }
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        view: &GameStateView,
        cost: &ManaCost,
        available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        if available_sources.is_empty() {
            return ChoiceResult::Ok(SmallVec::new());
        }

        let prompt = format!("Choose mana sources to pay {}", cost);

        let choices: Vec<String> = available_sources
            .iter()
            .map(|&card_id| view.card_name(card_id).unwrap_or_else(|| format!("Card {:?}", card_id)))
            .collect();

        match self.prompt_for_choice(view, &prompt, &choices) {
            Ok(PromptResult::Undo) => ChoiceResult::UndoRequest(usize::MAX),
            Ok(PromptResult::Choice(choice_opt)) => {
                let result = match choice_opt {
                    Some(idx) if idx < available_sources.len() => {
                        let mut sources = SmallVec::new();
                        sources.push(available_sources[idx]);
                        sources
                    }
                    _ => SmallVec::new(),
                };

                ChoiceResult::Ok(result)
            }
            Err(e) => ChoiceResult::Error(format!("Failed to prompt for mana: {}", e)),
        }
    }

    fn choose_attackers(
        &mut self,
        view: &GameStateView,
        available_creatures: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        if available_creatures.is_empty() {
            return ChoiceResult::Ok(SmallVec::new());
        }

        self.renderer.state_mut().choice_context = ChoiceContext::DeclareAttackers;
        self.renderer.state_mut().valid_choices = available_creatures.to_vec();

        let prompt = "Choose attackers (or Pass to skip)".to_string();

        let choices: Vec<String> = std::iter::once("Done attacking".to_string())
            .chain(
                available_creatures
                    .iter()
                    .map(|&card_id| view.card_name(card_id).unwrap_or_else(|| format!("Card {:?}", card_id))),
            )
            .collect();

        match self.prompt_for_choice(view, &prompt, &choices) {
            Ok(PromptResult::Undo) => {
                self.renderer.state_mut().choice_context = ChoiceContext::None;
                self.renderer.state_mut().valid_choices.clear();
                ChoiceResult::UndoRequest(usize::MAX)
            }
            Ok(PromptResult::Choice(choice_opt)) => {
                let result = match choice_opt {
                    Some(0) | None => SmallVec::new(),
                    Some(idx) if idx > 0 && idx <= available_creatures.len() => {
                        let mut attackers = SmallVec::new();
                        attackers.push(available_creatures[idx - 1]);
                        attackers
                    }
                    _ => SmallVec::new(),
                };

                self.renderer.state_mut().choice_context = ChoiceContext::None;
                self.renderer.state_mut().valid_choices.clear();

                ChoiceResult::Ok(result)
            }
            Err(e) => {
                self.renderer.state_mut().choice_context = ChoiceContext::None;
                self.renderer.state_mut().valid_choices.clear();
                ChoiceResult::Error(format!("Failed to prompt for attackers: {}", e))
            }
        }
    }

    fn choose_blockers(
        &mut self,
        view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
        if available_blockers.is_empty() || attackers.is_empty() {
            return ChoiceResult::Ok(SmallVec::new());
        }

        self.renderer.state_mut().choice_context = ChoiceContext::DeclareBlockers;
        self.renderer.state_mut().valid_choices = available_blockers.to_vec();

        let prompt = "Choose blockers (or Pass to skip)".to_string();

        let choices: Vec<String> = std::iter::once("Done blocking".to_string())
            .chain(
                available_blockers
                    .iter()
                    .map(|&card_id| view.card_name(card_id).unwrap_or_else(|| format!("Card {:?}", card_id))),
            )
            .collect();

        match self.prompt_for_choice(view, &prompt, &choices) {
            Ok(PromptResult::Undo) => {
                self.renderer.state_mut().choice_context = ChoiceContext::None;
                self.renderer.state_mut().valid_choices.clear();
                ChoiceResult::UndoRequest(usize::MAX)
            }
            Ok(PromptResult::Choice(_choice_opt)) => {
                self.renderer.state_mut().choice_context = ChoiceContext::None;
                self.renderer.state_mut().valid_choices.clear();

                // For now, simplified blocking - just return empty
                ChoiceResult::Ok(SmallVec::new())
            }
            Err(e) => {
                self.renderer.state_mut().choice_context = ChoiceContext::None;
                self.renderer.state_mut().valid_choices.clear();
                ChoiceResult::Error(format!("Failed to prompt for blockers: {}", e))
            }
        }
    }

    fn choose_damage_assignment_order(
        &mut self,
        view: &GameStateView,
        attacker: CardId,
        blockers: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        if blockers.is_empty() {
            return ChoiceResult::Ok(SmallVec::new());
        }

        let attacker_name = view
            .card_name(attacker)
            .unwrap_or_else(|| format!("Card {:?}", attacker));
        let prompt = format!("Choose damage order for {} (blocking creatures)", attacker_name);

        let choices: Vec<String> = blockers
            .iter()
            .map(|&card_id| view.card_name(card_id).unwrap_or_else(|| format!("Card {:?}", card_id)))
            .collect();

        match self.prompt_for_choice(view, &prompt, &choices) {
            Ok(PromptResult::Undo) => ChoiceResult::UndoRequest(usize::MAX),
            Ok(PromptResult::Choice(_choice_opt)) => {
                // For now, just return blockers in order
                ChoiceResult::Ok(blockers.iter().copied().collect())
            }
            Err(e) => ChoiceResult::Error(format!("Failed to prompt for damage order: {}", e)),
        }
    }

    fn choose_cards_to_discard(
        &mut self,
        view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        if hand.is_empty() || count == 0 {
            return ChoiceResult::Ok(SmallVec::new());
        }

        let prompt = format!("Choose {} card(s) to discard", count);

        let choices: Vec<String> = hand
            .iter()
            .map(|&card_id| view.card_name(card_id).unwrap_or_else(|| format!("Card {:?}", card_id)))
            .collect();

        match self.prompt_for_choice(view, &prompt, &choices) {
            Ok(PromptResult::Undo) => ChoiceResult::UndoRequest(usize::MAX),
            Ok(PromptResult::Choice(choice_opt)) => {
                let result = match choice_opt {
                    Some(idx) if idx < hand.len() => {
                        let mut cards = SmallVec::new();
                        cards.push(hand[idx]);
                        cards
                    }
                    _ => SmallVec::new(),
                };

                ChoiceResult::Ok(result)
            }
            Err(e) => ChoiceResult::Error(format!("Failed to prompt for discard: {}", e)),
        }
    }

    fn choose_from_library(&mut self, view: &GameStateView, valid_cards: &[CardId]) -> ChoiceResult<Option<CardId>> {
        if valid_cards.is_empty() {
            return ChoiceResult::Ok(None);
        }

        let prompt = "Choose a card from library".to_string();

        let choices: Vec<String> = std::iter::once("Cancel".to_string())
            .chain(
                valid_cards
                    .iter()
                    .map(|&card_id| view.card_name(card_id).unwrap_or_else(|| format!("Card {:?}", card_id))),
            )
            .collect();

        match self.prompt_for_choice(view, &prompt, &choices) {
            Ok(PromptResult::Undo) => ChoiceResult::UndoRequest(usize::MAX),
            Ok(PromptResult::Choice(choice_opt)) => {
                let result = match choice_opt {
                    Some(0) | None => None,
                    Some(idx) if idx > 0 && idx <= valid_cards.len() => Some(valid_cards[idx - 1]),
                    _ => None,
                };

                ChoiceResult::Ok(result)
            }
            Err(e) => ChoiceResult::Error(format!("Failed to prompt for library choice: {}", e)),
        }
    }

    fn on_priority_passed(&mut self, _view: &GameStateView) {
        // No-op for TUI controller
    }

    fn on_game_end(&mut self, _view: &GameStateView, won: bool) {
        eprintln!("\n{}", if won { "You won!" } else { "You lost." });
    }

    fn get_controller_type(&self) -> ControllerType {
        ControllerType::Tui
    }
}
