//! Replay controller that replays logged choices then delegates to another controller
//!
//! This controller is used for snapshot resume: it replays a sequence of predetermined
//! choices (from the snapshot's intra-turn choice log), then hands control to the
//! wrapped controller for subsequent choices.

use crate::core::{CardId, ManaCost, PlayerId, SpellAbility};
use crate::game::controller::{ChoiceResult, GameStateView, PlayerController};
use smallvec::SmallVec;

/// Resolved SMART combat-damage assignment plan: per attacker, the ordered
/// `(blocker_id, damage)` pairs the attacking player chose. Produced by
/// [`crate::game::GameState::assign_combat_damage`] and replayed via
/// [`ReplayChoice::DamageAssignment`] (mtg-610 A2).
pub type DamageAssignmentPlan = Vec<(CardId, SmallVec<[(CardId, i32); 4]>)>;

/// A single recorded choice from a controller
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ReplayChoice {
    /// Choice of spell ability to play (or None to pass priority)
    SpellAbility(Option<SpellAbility>),
    /// Choice of targets for a spell
    Targets(SmallVec<[CardId; 4]>),
    /// Choice of mana sources to tap
    ManaSources(SmallVec<[CardId; 8]>),
    /// Choice of attackers
    Attackers(SmallVec<[CardId; 8]>),
    /// Choice of blockers
    Blockers(SmallVec<[(CardId, CardId); 8]>),
    /// Choice of damage assignment order
    DamageOrder(SmallVec<[CardId; 4]>),
    /// Resolved SMART combat-damage assignment plan for a single combat-damage
    /// step: per attacker, the ordered `(blocker_id, damage)` pairs the attacking
    /// player chose. Recorded so rewind+replay APPLIES the authoritative plan
    /// instead of re-deriving it via `choose_blocker_for_lethal_damage` (which on
    /// a network shadow re-consults the server and double-submits — mtg-610 A2).
    /// Only attackers blocked by >1 creature appear here (single/unblocked need
    /// no choice and are recomputed trivially on replay).
    DamageAssignment(DamageAssignmentPlan),
    /// Choice of cards to discard
    Discard(SmallVec<[CardId; 7]>),
    /// Choice of card from library (or None to fail to find).
    ///
    /// Stores the AUTHORITATIVE fetched [`CardId`], NOT a positional index into
    /// the searcher's `valid_cards` view. The index was shadow-fragile: on an
    /// opponent's shadow the fetched card is hidden/face-down and excluded from
    /// the filtered `valid_cards`, so a `position()` lookup collapsed a real
    /// `Some(card)` fetch to `None` and the fetch was LOST on rewind+replay
    /// (mtg-mb668). Recording the CardId lets replay APPLY the authoritative
    /// move directly instead of re-deriving the selection against the shadow's
    /// incomplete view (mtg-610 hidden-info-replay principle).
    LibrarySearch(Option<CardId>),
    /// Choice of permanents to sacrifice
    Sacrifice(SmallVec<[CardId; 8]>),
    /// Choice of modes for a modal spell
    Modes(SmallVec<[usize; 4]>),
    /// Choice of X value for X-cost spells
    XValue(u8),
    /// Choice for a Scry effect — full partition of the revealed cards
    /// (top + bottom), stored bottom-up exactly like
    /// [`crate::game::ScryDecision`].
    Scry {
        top: SmallVec<[CardId; 4]>,
        bottom: SmallVec<[CardId; 4]>,
    },
    /// Choice for a Surveil effect — full partition of the revealed cards
    /// (top + graveyard), stored bottom-up exactly like
    /// [`crate::game::SurveilDecision`].
    Surveil {
        top: SmallVec<[CardId; 4]>,
        graveyard: SmallVec<[CardId; 4]>,
    },
}

/// Controller that replays a sequence of choices then delegates to another controller
///
/// This is used for snapshot resume. The replay controller:
/// 1. Plays back a predetermined sequence of choices from the snapshot
/// 2. Once all replay choices are exhausted, delegates to the wrapped controller
///
/// ## Usage
///
/// ```rust,ignore
/// // Create a controller with replay choices
/// let replay_choices = vec![
///     ReplayChoice::SpellAbility(Some(SpellAbility::PlayLand { card_id: CardId::new(1) })),
///     ReplayChoice::Targets(SmallVec::new()),
/// ];
///
/// let base_controller = RandomController::with_seed(player_id, 42);
/// let mut replay_controller = ReplayController::new(player_id, base_controller, replay_choices);
///
/// // Use replay_controller normally - it will replay choices then delegate
/// ```
pub struct ReplayController {
    player_id: PlayerId,
    /// The wrapped controller to delegate to after replay is exhausted
    inner: Box<dyn PlayerController>,
    /// Queue of choices to replay (consumed from front)
    replay_choices: Vec<ReplayChoice>,
    /// Current index in the replay queue
    replay_index: usize,
}

impl ReplayController {
    /// Create a new replay controller
    ///
    /// # Arguments
    /// * `player_id` - The player ID this controller manages
    /// * `inner` - The controller to delegate to after replay is exhausted
    /// * `replay_choices` - Sequence of choices to replay before delegating
    pub fn new(player_id: PlayerId, inner: Box<dyn PlayerController>, replay_choices: Vec<ReplayChoice>) -> Self {
        ReplayController {
            player_id,
            inner,
            replay_choices,
            replay_index: 0,
        }
    }

    /// Check if we have more replay choices to consume
    pub fn has_replay_choice(&self) -> bool {
        self.replay_index < self.replay_choices.len()
    }

    /// Recover the wrapped (inner) controller, consuming the [`ReplayController`].
    ///
    /// This is the counterpart of [`ReplayController::new`]: it lets a caller
    /// wrap a **persistent** controller in a fresh `ReplayController` for each
    /// rewind+replay re-entry (passing the accumulated choice history), run the
    /// game loop, then take the persistent controller back out so its internal
    /// state (e.g. a `RandomController`'s Xoshiro RNG) carries forward to the
    /// next re-entry. See `fancy_tui.rs::run_network_mode_replayable` (the WASM
    /// network rewind+replay path) — the persistent inner is delegated to ONLY
    /// for the genuinely-new frontier choice, so its RNG advances exactly once
    /// per real choice, byte-identical to the server's forward-only run.
    pub fn into_inner(self) -> Box<dyn PlayerController> {
        self.inner
    }

    /// Consume the next replay choice of the expected type
    ///
    /// Returns the choice if available and of the correct type, otherwise None.
    fn consume_replay_choice<F, T>(&mut self, extract: F) -> Option<T>
    where
        F: FnOnce(&ReplayChoice) -> Option<T>,
    {
        if !self.has_replay_choice() {
            return None;
        }

        let choice = &self.replay_choices[self.replay_index];
        if let Some(value) = extract(choice) {
            self.replay_index += 1;
            Some(value)
        } else {
            // Type mismatch - this shouldn't happen if replay log is correct
            eprintln!(
                "WARNING: Replay choice type mismatch at index {}. Expected different type, got {:?}",
                self.replay_index, choice
            );
            None
        }
    }
}

impl PlayerController for ReplayController {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn choose_spell_ability_to_play(
        &mut self,
        view: &GameStateView,
        available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>> {
        // Try to consume a replay choice first
        if let Some(choice) = self.consume_replay_choice(|c| {
            if let ReplayChoice::SpellAbility(opt) = c {
                Some(opt.clone())
            } else {
                None
            }
        }) {
            return ChoiceResult::Ok(choice);
        }

        // No replay choice available, delegate to inner controller
        self.inner.choose_spell_ability_to_play(view, available)
    }

    fn choose_targets(
        &mut self,
        view: &GameStateView,
        spell: CardId,
        valid_targets: &[CardId],
        min_targets: usize,
        max_targets: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        // Try to consume a replay choice first. The logged ReplayChoice::Targets
        // already carries the full chosen target vector (variable counts round-
        // trip automatically), so no min/max handling is needed on replay.
        if let Some(targets) = self.consume_replay_choice(|c| {
            if let ReplayChoice::Targets(t) = c {
                Some(t.clone())
            } else {
                None
            }
        }) {
            return ChoiceResult::Ok(targets);
        }

        // No replay choice available, delegate to inner controller
        self.inner
            .choose_targets(view, spell, valid_targets, min_targets, max_targets)
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        view: &GameStateView,
        cost: &ManaCost,
        available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Try to consume a replay choice first
        if let Some(sources) = self.consume_replay_choice(|c| {
            if let ReplayChoice::ManaSources(s) = c {
                Some(s.clone())
            } else {
                None
            }
        }) {
            return ChoiceResult::Ok(sources);
        }

        // No replay choice available, delegate to inner controller
        self.inner.choose_mana_sources_to_pay(view, cost, available_sources)
    }

    fn choose_attackers(
        &mut self,
        view: &GameStateView,
        available_creatures: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Try to consume a replay choice first
        if let Some(attackers) = self.consume_replay_choice(|c| {
            if let ReplayChoice::Attackers(a) = c {
                Some(a.clone())
            } else {
                None
            }
        }) {
            return ChoiceResult::Ok(attackers);
        }

        // No replay choice available, delegate to inner controller
        self.inner.choose_attackers(view, available_creatures)
    }

    fn choose_blockers(
        &mut self,
        view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
        // Try to consume a replay choice first
        if let Some(blockers) = self.consume_replay_choice(|c| {
            if let ReplayChoice::Blockers(b) = c {
                Some(b.clone())
            } else {
                None
            }
        }) {
            return ChoiceResult::Ok(blockers);
        }

        // No replay choice available, delegate to inner controller
        self.inner.choose_blockers(view, available_blockers, attackers)
    }

    fn choose_damage_assignment_order(
        &mut self,
        view: &GameStateView,
        attacker: CardId,
        blockers: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        // Try to consume a replay choice first
        if let Some(order) = self.consume_replay_choice(|c| {
            if let ReplayChoice::DamageOrder(o) = c {
                Some(o.clone())
            } else {
                None
            }
        }) {
            return ChoiceResult::Ok(order);
        }

        // No replay choice available, delegate to inner controller
        self.inner.choose_damage_assignment_order(view, attacker, blockers)
    }

    // Replay the authoritative SMART combat-damage plan (mtg-610 A2). When a
    // `DamageAssignment` was recorded, `assign_combat_damage` consumes it via
    // this method and APPLIES it directly, never reaching the per-blocker
    // sub-choices below — so on a network shadow the already-submitted plan is
    // not re-derived through the inner controller (which would double-submit /
    // stall). Returns `None` for the FIRST resolution of a combat-damage step
    // (nothing recorded yet); that pass falls through to the delegating
    // sub-choice methods below and is itself recorded by `combat_damage_step`.
    fn replay_damage_assignment(&mut self) -> Option<crate::game::DamageAssignmentPlan> {
        // PEEK (non-warning): this is called at the start of EVERY combat-damage
        // step, so a non-`DamageAssignment` next choice (e.g. the upcoming
        // CombatDamage-step priority, or no multi-blocker combat this step) is
        // normal — return None WITHOUT consuming and WITHOUT the type-mismatch
        // warning that `consume_replay_choice` would emit.
        if !self.has_replay_choice() {
            return None;
        }
        if let ReplayChoice::DamageAssignment(plan) = &self.replay_choices[self.replay_index] {
            let plan = plan.clone();
            self.replay_index += 1;
            Some(plan)
        } else {
            None
        }
    }

    // FALLBACK sub-choices for the FIRST (un-recorded) resolution: there is no
    // recorded `DamageAssignment` yet, so the SMART path runs and these MUST
    // delegate to the inner controller. Without these overrides the default
    // trait impls fire instead (auto-pick the first killable blocker), which on
    // a network shadow BYPASSES the inner `WasmNetworkLocalController`'s
    // server-gating + submit (and the `WasmRemoteController`'s opponent-choice
    // pop). Delegating mirrors the forward run, where the inner controller is
    // unwrapped and gates/submits correctly.
    fn choose_blocker_for_lethal_damage(
        &mut self,
        view: &GameStateView,
        attacker: CardId,
        killable_blockers: &[(CardId, i32)],
        remaining_power: i32,
    ) -> ChoiceResult<CardId> {
        self.inner
            .choose_blocker_for_lethal_damage(view, attacker, killable_blockers, remaining_power)
    }

    fn choose_blocker_for_remaining_damage(
        &mut self,
        view: &GameStateView,
        attacker: CardId,
        remaining_blockers: &[CardId],
        remaining_damage: i32,
    ) -> ChoiceResult<CardId> {
        self.inner
            .choose_blocker_for_remaining_damage(view, attacker, remaining_blockers, remaining_damage)
    }

    // The damage-assignment checkpoint/restore (mtg-sfihb) lives on the inner
    // controller's choice-consumption cursor; the default no-op would leave the
    // inner cursor un-checkpointed during replay, breaking the idempotent
    // re-run of the synchronous first pass. Delegate so replay behaves exactly
    // like the forward run.
    fn mark_choice_checkpoint(&mut self) {
        self.inner.mark_choice_checkpoint();
    }

    fn restore_choice_checkpoint(&mut self) {
        self.inner.restore_choice_checkpoint();
    }

    fn choose_cards_to_discard(
        &mut self,
        view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        // Try to consume a replay choice first
        if let Some(discard) = self.consume_replay_choice(|c| {
            if let ReplayChoice::Discard(d) = c {
                Some(d.clone())
            } else {
                None
            }
        }) {
            return ChoiceResult::Ok(discard);
        }

        // No replay choice available, delegate to inner controller
        self.inner.choose_cards_to_discard(view, hand, count)
    }

    fn choose_scry_order(
        &mut self,
        view: &GameStateView,
        revealed: &[CardId],
    ) -> ChoiceResult<crate::game::ScryDecision> {
        if let Some(decision) = self.consume_replay_choice(|c| {
            if let ReplayChoice::Scry { top, bottom } = c {
                Some(crate::game::ScryDecision {
                    top: top.clone(),
                    bottom: bottom.clone(),
                })
            } else {
                None
            }
        }) {
            return ChoiceResult::Ok(decision);
        }
        self.inner.choose_scry_order(view, revealed)
    }

    fn choose_surveil(
        &mut self,
        view: &GameStateView,
        revealed: &[CardId],
    ) -> ChoiceResult<crate::game::SurveilDecision> {
        if let Some(decision) = self.consume_replay_choice(|c| {
            if let ReplayChoice::Surveil { top, graveyard } = c {
                Some(crate::game::SurveilDecision {
                    top: top.clone(),
                    graveyard: graveyard.clone(),
                })
            } else {
                None
            }
        }) {
            return ChoiceResult::Ok(decision);
        }
        self.inner.choose_surveil(view, revealed)
    }

    fn choose_from_library(
        &mut self,
        view: &GameStateView,
        valid_cards: &[&crate::loader::CardDefinition],
    ) -> ChoiceResult<Option<usize>> {
        // NOTE: the recorded LibrarySearch choice is NOT consumed here. The
        // authoritative fetched CardId is replayed via `replay_library_search`
        // (called by `choose_from_library_with_hook` BEFORE this), so the index
        // returned here would be meaningless on a shadow whose `valid_cards`
        // omit the hidden fetched card. Delegate to the inner controller for
        // any non-replay call.
        self.inner.choose_from_library(view, valid_cards)
    }

    fn replay_library_search(&mut self) -> Option<Option<CardId>> {
        self.consume_replay_choice(|c| {
            if let ReplayChoice::LibrarySearch(card_opt) = c {
                Some(*card_opt)
            } else {
                None
            }
        })
    }

    fn choose_permanents_to_sacrifice(
        &mut self,
        view: &GameStateView,
        valid_permanents: &[CardId],
        count: usize,
        card_type_description: &str,
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Try to consume a replay choice first
        if let Some(sacrifices) = self.consume_replay_choice(|c| {
            if let ReplayChoice::Sacrifice(s) = c {
                Some(s.clone())
            } else {
                None
            }
        }) {
            return ChoiceResult::Ok(sacrifices);
        }

        // No replay choice available, delegate to inner controller
        self.inner
            .choose_permanents_to_sacrifice(view, valid_permanents, count, card_type_description)
    }

    fn choose_permanents_to_not_untap(
        &mut self,
        view: &GameStateView,
        may_not_untap_permanents: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // TODO: Could add ReplayChoice::NotUntap variant for replaying untap decisions
        // For now, delegate to inner controller
        self.inner
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
        // Try to consume a replay choice first
        if let Some(modes) = self.consume_replay_choice(|c| {
            if let ReplayChoice::Modes(m) = c {
                Some(m.clone())
            } else {
                None
            }
        }) {
            return ChoiceResult::Ok(modes);
        }

        // No replay choice available, delegate to inner controller
        self.inner
            .choose_modes(view, spell_id, mode_descriptions, mode_count, min_modes, can_repeat)
    }

    fn choose_x_value(&mut self, view: &GameStateView, spell_id: CardId, max_x: u8) -> ChoiceResult<u8> {
        if let Some(x) = self.consume_replay_choice(|c| {
            if let ReplayChoice::XValue(x) = c {
                Some(*x)
            } else {
                None
            }
        }) {
            return ChoiceResult::Ok(x);
        }
        self.inner.choose_x_value(view, spell_id, max_x)
    }

    fn prepare_for_priority_choice(&mut self) -> bool {
        // Delegate to inner controller for network sync behavior
        // This ensures the wrapped controller (e.g., WasmNetworkLocalController)
        // can perform its prepare logic even when wrapped in a ReplayController.
        self.inner.prepare_for_priority_choice()
    }

    fn on_priority_passed(&mut self, view: &GameStateView) {
        // Always delegate notifications to inner controller
        self.inner.on_priority_passed(view);
    }

    fn on_game_end(&mut self, view: &GameStateView, won: bool) {
        // Always delegate notifications to inner controller
        self.inner.on_game_end(view, won);
    }

    fn get_controller_type(&self) -> crate::game::snapshot::ControllerType {
        // Delegate to the inner controller to get its type
        // ReplayController is just a wrapper, so we report the wrapped controller's type
        self.inner.get_controller_type()
    }

    fn wants_context(&self) -> bool {
        self.inner.wants_context()
    }

    fn get_snapshot_state(&self) -> Option<serde_json::Value> {
        // Delegate to inner controller for state serialization
        // This allows the wrapped controller (RandomController, FixedScriptController, etc.)
        // to properly save its state even when wrapped in a ReplayController
        self.inner.get_snapshot_state()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::ZeroController;

    #[test]
    fn test_replay_controller_exhausts_choices() {
        let player_id = PlayerId::new(1);
        let inner = Box::new(ZeroController::new(player_id));

        let replay_choices = vec![
            ReplayChoice::SpellAbility(Some(SpellAbility::PlayLand {
                card_id: CardId::new(10),
            })),
            ReplayChoice::SpellAbility(None), // Pass priority
        ];

        let mut replay = ReplayController::new(player_id, inner, replay_choices);

        // Create a minimal game state for testing
        let game = crate::game::GameState::new_two_player("Player 1".to_string(), "Player 2".to_string(), 20);
        let view = crate::game::GameStateView::new(&game, player_id);

        // First call should return the replayed choice
        assert!(replay.has_replay_choice());
        let choice1 = replay.choose_spell_ability_to_play(&view, &[]);
        assert!(choice1.unwrap().is_some());

        // Second call should return the second replayed choice
        assert!(replay.has_replay_choice());
        let choice2 = replay.choose_spell_ability_to_play(&view, &[]);
        assert!(choice2.unwrap().is_none()); // Second choice was None (pass priority)

        // After exhausting replay choices, should delegate to inner controller
        assert!(!replay.has_replay_choice());
    }

    #[test]
    fn test_replay_controller_delegates_after_exhaustion() {
        let player_id = PlayerId::new(1);
        let inner = Box::new(ZeroController::new(player_id));

        // Empty replay choices - should immediately delegate
        let replay_choices = vec![];

        let replay = ReplayController::new(player_id, inner, replay_choices);
        assert!(!replay.has_replay_choice());
    }
}
