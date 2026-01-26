//! FancyFixed controller for scripted TUI debugging
//!
//! This controller uses the shared `FancyTuiRenderer` with ratatui's `TestBackend`
//! to capture screenshots of the game state before each choice. It then delegates
//! actual choice-making to the `RichInputController` for fully automated gameplay.
//!
//! This allows for visual debugging of TUI rendering issues without requiring
//! interactive terminal input.

use crate::core::{CardId, ManaCost, PlayerId, SpellAbility};
use crate::game::controller::{ChoiceResult, GameStateView, PlayerController};
use crate::game::fancy_tui_renderer::{ChoiceContext, FancyTuiRenderer};
use crate::game::snapshot::ControllerType;
use crate::game::RichInputController;
use crate::MtgError;
use ratatui::{backend::TestBackend, Terminal};
use smallvec::SmallVec;
use std::path::PathBuf;

/// A controller that renders TUI screenshots before delegating to RichInputController
pub struct FancyFixedController {
    player_id: PlayerId,
    renderer: FancyTuiRenderer,
    delegate: RichInputController,
    screenshot_counter: usize,
    screenshot_dir: PathBuf,
    /// Terminal width for screenshots (default: 240)
    terminal_width: u16,
    /// Terminal height for screenshots (default: 60)
    terminal_height: u16,
}

impl FancyFixedController {
    /// Default terminal width for screenshots
    pub const DEFAULT_WIDTH: u16 = 240;
    /// Default terminal height for screenshots
    pub const DEFAULT_HEIGHT: u16 = 60;

    /// Create a new FancyFixed controller
    ///
    /// # Arguments
    /// * `player_id` - The player this controller manages
    /// * `script` - The fixed input script (from RichInputController)
    /// * `screenshot_dir` - Optional directory to save screenshots
    ///
    /// # Errors
    ///
    /// Returns an error if the screenshot directory cannot be created.
    pub fn new(player_id: PlayerId, script: Vec<String>, screenshot_dir: Option<PathBuf>) -> Result<Self, MtgError> {
        Self::with_size(
            player_id,
            script,
            screenshot_dir,
            Self::DEFAULT_WIDTH,
            Self::DEFAULT_HEIGHT,
        )
    }

    /// Create a new FancyFixed controller with custom terminal size
    ///
    /// # Arguments
    /// * `player_id` - The player this controller manages
    /// * `script` - The fixed input script (from RichInputController)
    /// * `screenshot_dir` - Optional directory to save screenshots
    /// * `width` - Terminal width for screenshots
    /// * `height` - Terminal height for screenshots
    ///
    /// # Errors
    ///
    /// Returns an error if the screenshot directory cannot be created.
    pub fn with_size(
        player_id: PlayerId,
        script: Vec<String>,
        screenshot_dir: Option<PathBuf>,
        width: u16,
        height: u16,
    ) -> Result<Self, MtgError> {
        let visual_stacks = true; // Use visual stacks by default for better TUI appearance

        // Create screenshot directory
        // Handle empty paths (from snapshot_output.parent() when snapshot_output is just a filename)
        let dir = screenshot_dir
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| PathBuf::from("screenshots"));
        std::fs::create_dir_all(&dir)
            .map_err(|e| MtgError::InvalidAction(format!("Failed to create screenshot directory: {}", e)))?;
        eprintln!("Screenshots will be saved to: {} ({}x{})", dir.display(), width, height);

        Ok(FancyFixedController {
            player_id,
            renderer: FancyTuiRenderer::new(player_id, visual_stacks),
            delegate: RichInputController::new(player_id, script),
            screenshot_counter: 0,
            screenshot_dir: dir,
            terminal_width: width,
            terminal_height: height,
        })
    }

    /// Capture a screenshot of the current game state before making a choice
    fn capture_screenshot(&mut self, view: &GameStateView, prompt: &str, choices: &[String]) -> Result<(), MtgError> {
        // Create a TestBackend with the configured terminal size
        let backend = TestBackend::new(self.terminal_width, self.terminal_height);
        let mut terminal = Terminal::new(backend)
            .map_err(|e| MtgError::InvalidAction(format!("Failed to create test terminal: {}", e)))?;

        // Prepare choices for rendering (highlight the first one as a visual indicator)
        let choice_tuples: Vec<(String, bool)> = choices
            .iter()
            .enumerate()
            .map(|(idx, text)| {
                let numbered_text = format!("[{}] {}", idx, text);
                (numbered_text, idx == 0)
            })
            .collect();

        // Render the UI
        terminal
            .draw(|f| {
                self.renderer.draw_ui(f, view, Some(prompt), &choice_tuples);
            })
            .map_err(|e| MtgError::InvalidAction(format!("Failed to render TUI: {}", e)))?;

        // Get the rendered buffer
        let buffer = terminal.backend().buffer().clone();

        // Save to file
        self.save_buffer_to_file(&buffer, prompt)?;

        Ok(())
    }

    /// Save a ratatui buffer to a text file
    fn save_buffer_to_file(&mut self, buffer: &ratatui::buffer::Buffer, prompt: &str) -> Result<(), MtgError> {
        use std::io::Write;

        // Generate filename
        self.screenshot_counter += 1;
        let safe_prompt = prompt
            .chars()
            .map(|c| if c.is_alphanumeric() || c == ' ' { c } else { '_' })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<&str>>()
            .join("_");
        let truncated_prompt = if safe_prompt.len() > 40 {
            &safe_prompt[..40]
        } else {
            &safe_prompt
        };
        let filename = self
            .screenshot_dir
            .join(format!("{:04}_{}.txt", self.screenshot_counter, truncated_prompt));

        let mut file = std::fs::File::create(&filename)
            .map_err(|e| MtgError::InvalidAction(format!("Failed to create screenshot file: {}", e)))?;

        // Write buffer content line by line
        let area = buffer.area();
        for y in 0..area.height {
            let mut line = String::new();
            for x in 0..area.width {
                let cell = &buffer[(x, y)];
                line.push_str(cell.symbol());
            }
            // Trim trailing whitespace
            let trimmed = line.trim_end();
            writeln!(file, "{}", trimmed)
                .map_err(|e| MtgError::InvalidAction(format!("Failed to write to screenshot: {}", e)))?;
        }

        eprintln!("[SCREENSHOT] Saved: {}", filename.display());

        Ok(())
    }
}

impl PlayerController for FancyFixedController {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn choose_spell_ability_to_play(
        &mut self,
        view: &GameStateView,
        available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>> {
        // Set up renderer state for screenshot
        self.renderer.state.choice_context = ChoiceContext::PlayingSpell;
        self.renderer.state.valid_choices = available.iter().map(SpellAbility::card_id).collect();

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
                SpellAbility::CastFromExile {
                    card_id,
                    alternative_cost,
                    ..
                } => {
                    let name = view.card_name(*card_id).unwrap_or_default();
                    format!("Cast from exile: {} (for {})", name, alternative_cost)
                }
                SpellAbility::Cycle {
                    card_id,
                    cost,
                    search_type,
                } => {
                    let name = view.card_name(*card_id).unwrap_or_default();
                    let type_str = match search_type {
                        Some(st) => format!("{}cycling", st.as_str()),
                        None => "Cycle".to_string(),
                    };
                    format!("{}: {} ({})", type_str, name, cost)
                }
            }))
            .collect();

        // Capture screenshot before delegating (even if available is empty)
        if let Err(e) = self.capture_screenshot(view, &prompt, &choices) {
            eprintln!("Warning: Failed to capture screenshot: {:?}", e);
        }

        // Clean up renderer state
        self.renderer.state.choice_context = ChoiceContext::None;
        self.renderer.state.valid_choices.clear();

        // Early return for empty available (after taking screenshot)
        if available.is_empty() {
            return ChoiceResult::Ok(None);
        }

        // Delegate to RichInputController
        self.delegate.choose_spell_ability_to_play(view, available)
    }

    fn choose_targets(
        &mut self,
        view: &GameStateView,
        spell: CardId,
        valid_targets: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        // Set up renderer state
        self.renderer.state.choice_context = ChoiceContext::TargetSelection;
        self.renderer.state.valid_choices = valid_targets.to_vec();

        let spell_name = view.card_name(spell).unwrap_or_else(|| format!("Card {:?}", spell));
        let prompt = format!("Choose target for {}", spell_name);

        let choices: Vec<String> = valid_targets
            .iter()
            .map(|&card_id| view.card_name(card_id).unwrap_or_else(|| format!("Card {:?}", card_id)))
            .collect();

        // Capture screenshot (even if valid_targets is empty)
        if let Err(e) = self.capture_screenshot(view, &prompt, &choices) {
            eprintln!("Warning: Failed to capture screenshot: {:?}", e);
        }

        // Clean up
        self.renderer.state.choice_context = ChoiceContext::None;
        self.renderer.state.valid_choices.clear();

        // Early return for empty targets (after taking screenshot)
        if valid_targets.is_empty() {
            return ChoiceResult::Ok(SmallVec::new());
        }

        // Delegate
        self.delegate.choose_targets(view, spell, valid_targets)
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        view: &GameStateView,
        cost: &ManaCost,
        available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // For mana choices, we skip screenshots as they can be very frequent
        // and less visually interesting
        self.delegate.choose_mana_sources_to_pay(view, cost, available_sources)
    }

    fn choose_attackers(
        &mut self,
        view: &GameStateView,
        available_creatures: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Set up renderer state
        self.renderer.state.choice_context = ChoiceContext::DeclareAttackers;
        self.renderer.state.valid_choices = available_creatures.to_vec();

        let prompt = "Choose attackers (or Pass to skip)".to_string();

        let choices: Vec<String> = std::iter::once("Done attacking".to_string())
            .chain(
                available_creatures
                    .iter()
                    .map(|&card_id| view.card_name(card_id).unwrap_or_else(|| format!("Card {:?}", card_id))),
            )
            .collect();

        // Capture screenshot (even if available_creatures is empty)
        if let Err(e) = self.capture_screenshot(view, &prompt, &choices) {
            eprintln!("Warning: Failed to capture screenshot: {:?}", e);
        }

        // Clean up
        self.renderer.state.choice_context = ChoiceContext::None;
        self.renderer.state.valid_choices.clear();

        // Early return for empty creatures (after taking screenshot)
        if available_creatures.is_empty() {
            return ChoiceResult::Ok(SmallVec::new());
        }

        // Delegate
        self.delegate.choose_attackers(view, available_creatures)
    }

    fn choose_blockers(
        &mut self,
        view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
        // Set up renderer state
        self.renderer.state.choice_context = ChoiceContext::DeclareBlockers;
        self.renderer.state.valid_choices = available_blockers.to_vec();

        let prompt = "Choose blockers (or Pass to skip)".to_string();

        let choices: Vec<String> = std::iter::once("Done blocking".to_string())
            .chain(
                available_blockers
                    .iter()
                    .map(|&card_id| view.card_name(card_id).unwrap_or_else(|| format!("Card {:?}", card_id))),
            )
            .collect();

        // Capture screenshot (even if available_blockers or attackers is empty)
        if let Err(e) = self.capture_screenshot(view, &prompt, &choices) {
            eprintln!("Warning: Failed to capture screenshot: {:?}", e);
        }

        // Clean up
        self.renderer.state.choice_context = ChoiceContext::None;
        self.renderer.state.valid_choices.clear();

        // Early return for empty blockers or attackers (after taking screenshot)
        if available_blockers.is_empty() || attackers.is_empty() {
            return ChoiceResult::Ok(SmallVec::new());
        }

        // Delegate
        self.delegate.choose_blockers(view, available_blockers, attackers)
    }

    fn choose_damage_assignment_order(
        &mut self,
        view: &GameStateView,
        attacker: CardId,
        blockers: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        // Skip screenshot for damage assignment
        self.delegate.choose_damage_assignment_order(view, attacker, blockers)
    }

    fn choose_cards_to_discard(
        &mut self,
        view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        // Skip screenshot for discard
        self.delegate.choose_cards_to_discard(view, hand, count)
    }

    fn choose_from_library(&mut self, view: &GameStateView, valid_card_names: &[&str]) -> ChoiceResult<Option<usize>> {
        // Skip screenshot for library choice
        self.delegate.choose_from_library(view, valid_card_names)
    }

    fn choose_permanents_to_sacrifice(
        &mut self,
        view: &GameStateView,
        valid_permanents: &[CardId],
        count: usize,
        card_type_description: &str,
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Skip screenshot for sacrifice choice
        self.delegate
            .choose_permanents_to_sacrifice(view, valid_permanents, count, card_type_description)
    }

    fn choose_permanents_to_not_untap(
        &mut self,
        view: &GameStateView,
        may_not_untap_permanents: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Delegate to underlying controller
        self.delegate
            .choose_permanents_to_not_untap(view, may_not_untap_permanents)
    }

    fn choose_modes(
        &mut self,
        view: &GameStateView,
        spell_id: CardId,
        mode_descriptions: &[String],
        mode_count: usize,
        min_modes: usize,
        can_repeat: bool,
    ) -> ChoiceResult<SmallVec<[usize; 4]>> {
        // Delegate to underlying controller
        self.delegate
            .choose_modes(view, spell_id, mode_descriptions, mode_count, min_modes, can_repeat)
    }

    fn on_priority_passed(&mut self, view: &GameStateView) {
        self.delegate.on_priority_passed(view);
    }

    fn on_game_end(&mut self, view: &GameStateView, won: bool) {
        self.delegate.on_game_end(view, won);
    }

    fn get_controller_type(&self) -> ControllerType {
        ControllerType::FancyFixed
    }
}
