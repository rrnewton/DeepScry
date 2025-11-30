//! Fancy TUI controller with fixed scripted inputs and screenshot capture
//!
//! This controller uses:
//! - Fixed scripted input from RichInputController (uses --fixed-inputs script)
//! - TODO: TUI rendering and screenshot capture (phase 2)
//!
//! Currently this is a thin wrapper around RichInputController. In a future enhancement,
//! we'll add TUI rendering using a ratatui TestBackend to capture screenshots.

use crate::core::{CardId, ManaCost, PlayerId, SpellAbility};
use crate::game::{
    controller::{ChoiceResult, GameStateView, PlayerController},
    snapshot::ControllerType,
    RichInputController,
};
use crate::MtgError;
use smallvec::SmallVec;
use std::path::PathBuf;

/// Fancy TUI with fixed scripted inputs and screenshot capture
///
/// Currently delegates all functionality to RichInputController.
/// TODO: Add TUI rendering and screenshot capture.
pub struct FancyFixedController {
    /// The fixed input controller
    fixed: RichInputController,

    /// Directory to save screenshots (for future use)
    #[allow(dead_code)]
    screenshot_dir: Option<PathBuf>,
}

impl FancyFixedController {
    /// Create a new FancyFixedController
    ///
    /// # Arguments
    /// * `player_id` - The player this controller manages
    /// * `script` - The fixed input script (parsed from --fixed-inputs)
    /// * `screenshot_dir` - Optional directory to save screenshots (not yet implemented)
    pub fn new(player_id: PlayerId, script: Vec<String>, screenshot_dir: Option<PathBuf>) -> Result<Self, MtgError> {
        let fixed = RichInputController::new(player_id, script);

        // Create screenshot directory if specified
        if let Some(ref dir) = screenshot_dir {
            std::fs::create_dir_all(dir)
                .map_err(|e| MtgError::InvalidAction(format!("Failed to create screenshot directory: {}", e)))?;
            log::info!(
                "Screenshot directory created (rendering not yet implemented): {}",
                dir.display()
            );
        }

        Ok(Self { fixed, screenshot_dir })
    }
}

impl PlayerController for FancyFixedController {
    fn player_id(&self) -> PlayerId {
        self.fixed.player_id()
    }

    fn choose_spell_ability_to_play(
        &mut self,
        view: &GameStateView,
        available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>> {
        // TODO: Render TUI and capture screenshot
        self.fixed.choose_spell_ability_to_play(view, available)
    }

    fn choose_targets(
        &mut self,
        view: &GameStateView,
        spell: CardId,
        valid_targets: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        // TODO: Render TUI and capture screenshot
        self.fixed.choose_targets(view, spell, valid_targets)
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        view: &GameStateView,
        cost: &ManaCost,
        available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // TODO: Render TUI and capture screenshot
        self.fixed.choose_mana_sources_to_pay(view, cost, available_sources)
    }

    fn choose_attackers(
        &mut self,
        view: &GameStateView,
        available_creatures: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // TODO: Render TUI and capture screenshot
        self.fixed.choose_attackers(view, available_creatures)
    }

    fn choose_blockers(
        &mut self,
        view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
        // TODO: Render TUI and capture screenshot
        self.fixed.choose_blockers(view, available_blockers, attackers)
    }

    fn choose_damage_assignment_order(
        &mut self,
        view: &GameStateView,
        attacker: CardId,
        blockers: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        // TODO: Render TUI and capture screenshot
        self.fixed.choose_damage_assignment_order(view, attacker, blockers)
    }

    fn choose_cards_to_discard(
        &mut self,
        view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        // TODO: Render TUI and capture screenshot
        self.fixed.choose_cards_to_discard(view, hand, count)
    }

    fn choose_from_library(&mut self, view: &GameStateView, valid_cards: &[CardId]) -> ChoiceResult<Option<CardId>> {
        // TODO: Render TUI and capture screenshot
        self.fixed.choose_from_library(view, valid_cards)
    }

    fn on_priority_passed(&mut self, view: &GameStateView) {
        self.fixed.on_priority_passed(view)
    }

    fn on_game_end(&mut self, view: &GameStateView, won: bool) {
        self.fixed.on_game_end(view, won)
    }

    fn get_controller_type(&self) -> ControllerType {
        // Return Fixed since we're using fixed scripting (even though we might add TUI later)
        ControllerType::Fixed
    }
}
