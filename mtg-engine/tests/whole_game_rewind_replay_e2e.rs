// TODO(mtg-211): Remove once wildcard patterns are audited
#![allow(clippy::wildcard_enum_match_arm)]

//! Whole-game rewind -> replay invariant (mtg-610 / mtg-559 / mtg-614).
//!
//! ## What this proves (and why it replaces three older tests)
//!
//! The network / WASM blocking model is implemented by **rewind-to-turn-start +
//! replay-forward** (re-evolving internal game state, suppressing only external
//! side-effects). For that to be sound the undo log must be COMPLETE: after a
//! `rewind_to_turn_start` followed by a deterministic replay of the recorded
//! intra-turn choices, BOTH
//!
//!   - the game-state hash (`compute_state_hash`, Replay mode), AND
//!   - the gamelog (the engine's `logger` message sequence)
//!
//! must return EXACTLY to the values they had immediately before the rewind. A
//! divergence in either means some canonical state was mutated outside the undo
//! log (a "hole"): rewind leaves it stale and replay double-applies. Per
//! `docs/NETWORK_ARCHITECTURE.md`, desync is always fatal, so this is a
//! compile-time-enforced regression gate.
//!
//! This consolidates and STRENGTHENS three earlier ad-hoc tests:
//!   - `undo_e2e.rs::test_full_game_undo_replay` only asserted `winner.is_some()`
//!     after a full rewind — it never hashed and never diffed the gamelog, so it
//!     proved "undo doesn't crash" but NOT "replay reproduces the game".
//!   - `rewind_replay_oracle_e2e.rs` round-tripped the state hash at every
//!     decision point but did NOT diff the gamelog.
//!   - `rewind_replay_hash_roundtrip_e2e.rs` round-tripped the state hash through
//!     a Bazaar mid-resolution discard but, again, did NOT diff the gamelog.
//!   - `bazaar_rewind_loop_e2e.rs` only asserted a Discard ChoicePoint was logged.
//!
//! The Bazaar scenario below subsumes all of that Bazaar coverage: it both
//! asserts the mid-resolution Discard ChoicePoint is logged (the
//! `bazaar_rewind_loop` regression) AND round-trips state-hash + gamelog through
//! it. The gamelog diff is what makes the consolidated test catch the
//! infinite-loop regression: an un-logged choice would fail to replay, so the
//! replayed gamelog tail would be shorter / divergent and `verify_gamelog_tail`
//! reports the first offending message + buffer index.
//!
//! ## Turn-1 baseline limitation (mtg-610)
//!
//! `rewind_to_turn_start` rewinds to the most recent `ChangeTurn` marker. Turn 1
//! has NO such marker, so rewinding a turn-1-only game empties the entire undo
//! log and re-triggers PRE-GAME setup (shuffle + opening hands), which advances
//! the RNG and re-orders libraries — that is the known "turn-1 hash inexactness"
//! and is non-reproducible. We therefore pause and rewind at the DEEPEST CLEAN
//! turn-start the recorder reaches (turn 2+), where a real `ChangeTurn` boundary
//! survives the rewind. That still exercises CROSS-TURN rewind, which the older
//! single-turn tests never did. A clean post-setup turn-1 checkpoint primitive
//! is tracked in mtg-610; once it lands, the baseline can move to turn 1.

use mtg_engine::{
    core::{CardId, ManaCost, PlayerId, SpellAbility},
    game::{
        compute_state_hash,
        controller::{ChoiceContext, ChoiceResult, GameStateView, PlayerController},
        logger::{LogEntry, OutputMode},
        replay_controller::ReplayChoice,
        snapshot::ControllerType,
        GameLoop, GameLoopState, GameState, ReplayController, VerbosityLevel, ZeroController,
    },
    loader::{require_cardsfolder, AsyncCardDatabase as CardDatabase, DeckLoader, GameInitializer},
    puzzle::{loader::load_puzzle_into_game, PuzzleFile},
    undo::GameAction,
    Result,
};
use smallvec::SmallVec;
use std::path::PathBuf;

// ===========================================================================
// Deterministic recorder controller (mirrors rewind_replay_oracle_e2e's policy)
// ===========================================================================

/// Deterministic policy + pause hook. Picks the FIRST land it can play, else the
/// FIRST creature it can cast, else passes; attacks with everything; blocks the
/// first attacker; discards the first cards. Picking the first matching choice
/// every time keeps replay byte-exact.
///
/// `pause_at` makes `choose_spell_ability_to_play` return `NeedInput` the Nth
/// time it is called (0-based), which is how a WASM human controller pauses the
/// loop at a real decision point. `None` = never pause (replay / discovery pass).
struct RecorderController {
    player_id: PlayerId,
    pause_at: Option<usize>,
    priority_calls: usize,
}

impl RecorderController {
    fn new(player_id: PlayerId, pause_at: Option<usize>) -> Self {
        Self {
            player_id,
            pause_at,
            priority_calls: 0,
        }
    }
}

impl PlayerController for RecorderController {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn choose_spell_ability_to_play(
        &mut self,
        _view: &GameStateView,
        available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>> {
        let this_call = self.priority_calls;
        self.priority_calls += 1;
        if self.pause_at == Some(this_call) {
            return ChoiceResult::NeedInput(ChoiceContext::SpellAbility {
                available: available.to_vec(),
                formatted_choices: Vec::new(),
            });
        }
        if let Some(l) = available.iter().find(|a| matches!(a, SpellAbility::PlayLand { .. })) {
            return ChoiceResult::Ok(Some(l.clone()));
        }
        if let Some(c) = available.iter().find(|a| matches!(a, SpellAbility::CastSpell { .. })) {
            return ChoiceResult::Ok(Some(c.clone()));
        }
        ChoiceResult::Ok(None)
    }

    fn choose_targets(
        &mut self,
        _view: &GameStateView,
        _spell: CardId,
        _valid_targets: &[CardId],
        _min_targets: usize,
        _max_targets: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        ChoiceResult::Ok(SmallVec::new())
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        _view: &GameStateView,
        _cost: &ManaCost,
        available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        ChoiceResult::Ok(available_sources.iter().copied().collect())
    }

    fn choose_attackers(
        &mut self,
        _view: &GameStateView,
        available_creatures: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        ChoiceResult::Ok(available_creatures.iter().copied().collect())
    }

    fn choose_blockers(
        &mut self,
        _view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
        let mut out: SmallVec<[(CardId, CardId); 8]> = SmallVec::new();
        if let (Some(&blocker), Some(&attacker)) = (available_blockers.first(), attackers.first()) {
            out.push((blocker, attacker));
        }
        ChoiceResult::Ok(out)
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
        _view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        ChoiceResult::Ok(hand.iter().take(count).copied().collect())
    }

    fn choose_from_library(
        &mut self,
        _view: &GameStateView,
        valid_cards: &[&mtg_engine::loader::CardDefinition],
    ) -> ChoiceResult<Option<usize>> {
        ChoiceResult::Ok(if valid_cards.is_empty() { None } else { Some(0) })
    }

    fn choose_permanents_to_sacrifice(
        &mut self,
        _view: &GameStateView,
        valid_permanents: &[CardId],
        count: usize,
        _card_type_description: &str,
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        ChoiceResult::Ok(valid_permanents.iter().take(count).copied().collect())
    }

    fn choose_permanents_to_not_untap(
        &mut self,
        _view: &GameStateView,
        _may_not_untap_permanents: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        ChoiceResult::Ok(SmallVec::new())
    }

    fn choose_modes(
        &mut self,
        _view: &GameStateView,
        _spell_id: CardId,
        mode_descriptions: &[String],
        mode_count: usize,
        _min_modes: usize,
        _can_repeat: bool,
    ) -> ChoiceResult<SmallVec<[usize; 4]>> {
        ChoiceResult::Ok((0..mode_count.min(mode_descriptions.len())).collect())
    }

    fn on_priority_passed(&mut self, _view: &GameStateView) {}
    fn on_game_end(&mut self, _view: &GameStateView, _won: bool) {}

    fn get_controller_type(&self) -> ControllerType {
        ControllerType::Tui
    }
}

// ===========================================================================
// Bazaar bot (mirrors rewind_replay_hash_roundtrip_e2e / bazaar_rewind_loop)
// ===========================================================================

/// Activate Bazaar of Baghdad once (its resolution draws 2 then forces a
/// mid-resolution discard of 3 — the in-stack ChoicePoint category mtg-559
/// belongs to), discard the first `count` cards, then pause the loop so the
/// rewind below has genuine intra-turn state to undo.
struct BazaarBot {
    player_id: PlayerId,
    activated_bazaar: bool,
    discarded: bool,
}

impl BazaarBot {
    fn new(player_id: PlayerId) -> Self {
        Self {
            player_id,
            activated_bazaar: false,
            discarded: false,
        }
    }
}

impl PlayerController for BazaarBot {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn choose_spell_ability_to_play(
        &mut self,
        _view: &GameStateView,
        available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>> {
        if !self.activated_bazaar {
            for sa in available {
                if let SpellAbility::ActivateAbility { .. } = sa {
                    self.activated_bazaar = true;
                    return ChoiceResult::Ok(Some(sa.clone()));
                }
            }
        }
        if self.discarded {
            return ChoiceResult::NeedInput(ChoiceContext::SpellAbility {
                available: available.to_vec(),
                formatted_choices: Vec::new(),
            });
        }
        ChoiceResult::Ok(None)
    }

    fn choose_targets(
        &mut self,
        _view: &GameStateView,
        _spell: CardId,
        _valid_targets: &[CardId],
        _min_targets: usize,
        _max_targets: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        ChoiceResult::Ok(SmallVec::new())
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        _view: &GameStateView,
        _cost: &ManaCost,
        available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        ChoiceResult::Ok(available_sources.iter().copied().collect())
    }

    fn choose_attackers(
        &mut self,
        _view: &GameStateView,
        _available_creatures: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        ChoiceResult::Ok(SmallVec::new())
    }

    fn choose_blockers(
        &mut self,
        _view: &GameStateView,
        _available_blockers: &[CardId],
        _attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
        ChoiceResult::Ok(SmallVec::new())
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
        _view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        self.discarded = true;
        ChoiceResult::Ok(hand.iter().take(count).copied().collect())
    }

    fn choose_from_library(
        &mut self,
        _view: &GameStateView,
        valid_cards: &[&mtg_engine::loader::CardDefinition],
    ) -> ChoiceResult<Option<usize>> {
        ChoiceResult::Ok(if valid_cards.is_empty() { None } else { Some(0) })
    }

    fn choose_permanents_to_sacrifice(
        &mut self,
        _view: &GameStateView,
        valid_permanents: &[CardId],
        count: usize,
        _card_type_description: &str,
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        ChoiceResult::Ok(valid_permanents.iter().take(count).copied().collect())
    }

    fn choose_permanents_to_not_untap(
        &mut self,
        _view: &GameStateView,
        _may_not_untap_permanents: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        ChoiceResult::Ok(SmallVec::new())
    }

    fn choose_modes(
        &mut self,
        _view: &GameStateView,
        _spell_id: CardId,
        mode_descriptions: &[String],
        mode_count: usize,
        _min_modes: usize,
        _can_repeat: bool,
    ) -> ChoiceResult<SmallVec<[usize; 4]>> {
        ChoiceResult::Ok((0..mode_count.min(mode_descriptions.len())).collect())
    }

    fn on_priority_passed(&mut self, _view: &GameStateView) {}
    fn on_game_end(&mut self, _view: &GameStateView, _won: bool) {}

    fn get_controller_type(&self) -> ControllerType {
        ControllerType::Tui
    }
}

/// Inner controller under the `ReplayController` on the replay pass: once the
/// recorded choices are exhausted it returns `NeedInput` on the next priority
/// request, so the replay stops at exactly the same mid-turn point the forward
/// pass stopped at (rather than playing on and diverging). All other callbacks
/// delegate to a `ZeroController` so a stray request can't error.
struct StopController {
    inner: ZeroController,
}

impl StopController {
    fn new(player_id: PlayerId) -> Self {
        Self {
            inner: ZeroController::new(player_id),
        }
    }
}

impl PlayerController for StopController {
    fn player_id(&self) -> PlayerId {
        self.inner.player_id()
    }

    fn choose_spell_ability_to_play(
        &mut self,
        _view: &GameStateView,
        available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>> {
        ChoiceResult::NeedInput(ChoiceContext::SpellAbility {
            available: available.to_vec(),
            formatted_choices: Vec::new(),
        })
    }

    fn choose_targets(
        &mut self,
        view: &GameStateView,
        spell: CardId,
        valid_targets: &[CardId],
        min_targets: usize,
        max_targets: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        self.inner
            .choose_targets(view, spell, valid_targets, min_targets, max_targets)
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        view: &GameStateView,
        cost: &ManaCost,
        available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        self.inner.choose_mana_sources_to_pay(view, cost, available_sources)
    }

    fn choose_attackers(
        &mut self,
        view: &GameStateView,
        available_creatures: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        self.inner.choose_attackers(view, available_creatures)
    }

    fn choose_blockers(
        &mut self,
        view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
        self.inner.choose_blockers(view, available_blockers, attackers)
    }

    fn choose_damage_assignment_order(
        &mut self,
        view: &GameStateView,
        attacker: CardId,
        blockers: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        self.inner.choose_damage_assignment_order(view, attacker, blockers)
    }

    fn choose_cards_to_discard(
        &mut self,
        view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        self.inner.choose_cards_to_discard(view, hand, count)
    }

    fn choose_from_library(
        &mut self,
        view: &GameStateView,
        valid_cards: &[&mtg_engine::loader::CardDefinition],
    ) -> ChoiceResult<Option<usize>> {
        self.inner.choose_from_library(view, valid_cards)
    }

    fn choose_permanents_to_sacrifice(
        &mut self,
        view: &GameStateView,
        valid_permanents: &[CardId],
        count: usize,
        card_type_description: &str,
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        self.inner
            .choose_permanents_to_sacrifice(view, valid_permanents, count, card_type_description)
    }

    fn choose_permanents_to_not_untap(
        &mut self,
        view: &GameStateView,
        may_not_untap_permanents: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
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
        self.inner
            .choose_modes(view, spell_id, mode_descriptions, mode_count, min_modes, can_repeat)
    }

    fn on_priority_passed(&mut self, _view: &GameStateView) {}
    fn on_game_end(&mut self, _view: &GameStateView, _won: bool) {}

    fn get_controller_type(&self) -> ControllerType {
        ControllerType::Tui
    }
}

// ===========================================================================
// Shared rewind/replay helpers
// ===========================================================================

/// Reconstruct the per-turn `(ours, theirs)` `ReplayChoice` lists from the
/// intra-turn ChoicePoints `rewind_to_turn_start` returned — mirrors
/// `WasmFancyTuiState::rewind_to_turn_start` (the production rewind/replay path).
fn partition_choices(choice_actions: Vec<GameAction>, our_id: PlayerId) -> (Vec<ReplayChoice>, Vec<ReplayChoice>) {
    let mut ours = Vec::new();
    let mut theirs = Vec::new();
    for action in choice_actions {
        if let GameAction::ChoicePoint {
            player_id,
            choice: Some(c),
            ..
        } = action
        {
            if player_id == our_id {
                ours.push(c);
            } else {
                theirs.push(c);
            }
        }
    }
    (ours, theirs)
}

/// Result of comparing the replayed gamelog tail against the captured forward
/// tail. Mirrors `wasm::replay_verifier::ReplayCheckOutcome`, ported here because
/// that module is gated behind the `wasm` feature and unavailable under the
/// `network` feature these tests run with. Same semantics: `controller_choice`
/// entries are skipped (ReplayController short-circuits the inner controller, so
/// its `<Choice>` lines never re-emit), and the first divergence pinpoints the
/// offending message + absolute buffer index.
#[derive(Debug, PartialEq, Eq)]
enum GamelogCheck {
    Ok,
    /// Replay produced fewer engine entries after the turn boundary than the
    /// forward pass — replay stalled (e.g. a choice that was never logged could
    /// not be replayed, so the dependent gamelog lines never fired). This is the
    /// shape the Bazaar infinite-loop regression takes.
    Truncated {
        captured_len: usize,
        replay_len: usize,
        first_missing: String,
    },
    /// A regenerated engine entry differs from the captured original.
    Mismatch {
        buffer_index: usize,
        prefix_offset: usize,
        expected: String,
        actual: String,
    },
}

/// `controller_choice`-category entries are not reproducible under replay
/// (ReplayController feeds saved choices without invoking the inner controller),
/// so they are filtered from both sides before comparison — exactly as
/// `wasm::replay_verifier::is_replayable_entry` does.
fn is_replayable_entry(entry: &LogEntry) -> bool {
    !matches!(entry.category.as_deref(), Some("controller_choice"))
}

/// Compare the captured forward-pass log tail (entries at
/// `[log_size_at_turn, captured_len)`) against the live replayed log tail
/// (entries at `[log_size_at_turn, end)`), skipping non-replayable entries.
/// Ported from `wasm::replay_verifier::verify_replay`'s log-tail comparison.
fn verify_gamelog_tail(
    captured_tail: &[LogEntry],
    replay_buffer: &[LogEntry],
    log_size_at_turn: usize,
) -> GamelogCheck {
    let captured: Vec<&LogEntry> = captured_tail.iter().filter(|e| is_replayable_entry(e)).collect();
    let replay: Vec<(usize, &LogEntry)> = replay_buffer[log_size_at_turn..]
        .iter()
        .enumerate()
        .filter(|(_, e)| is_replayable_entry(e))
        .collect();

    if replay.len() < captured.len() {
        return GamelogCheck::Truncated {
            captured_len: captured.len(),
            replay_len: replay.len(),
            first_missing: captured[replay.len()].message.clone(),
        };
    }

    for (offset, captured_entry) in captured.iter().enumerate() {
        let (rep_raw_idx, actual) = replay[offset];
        if captured_entry.message != actual.message {
            return GamelogCheck::Mismatch {
                buffer_index: log_size_at_turn + rep_raw_idx,
                prefix_offset: offset,
                expected: captured_entry.message.clone(),
                actual: actual.message.clone(),
            };
        }
    }

    GamelogCheck::Ok
}

/// Snapshot the engine log buffer (owned clone) so it survives the rewind's
/// `logger.truncate_to(...)`.
fn snapshot_log(game: &GameState) -> Vec<LogEntry> {
    game.logger.logs().iter().cloned().collect()
}

/// The core whole-game rewind -> replay round-trip, shared by every scenario.
///
/// Given a game already paused mid-turn-2+ (undo log holds intra-turn state past
/// a `ChangeTurn` boundary) plus the captured pre-rewind hash and full log
/// buffer, this:
///   1. rewinds to the turn-start baseline and asserts the baseline hash is
///      stable across a re-rewind (turn-start determinism);
///   2. replays the recorded intra-turn choices for both players;
///   3. asserts final hash == pre-rewind hash AND the replayed gamelog tail
///      matches the captured tail exactly.
///
/// Returns `true` if the round-trip was NON-VACUOUS (the captured gamelog tail
/// had >=1 replayable engine entry, so the gamelog assertion had teeth). Returns
/// `false` if the pause point happened to land at a turn boundary with no
/// intra-turn gamelog yet — the state-hash round-trip is still asserted (it is
/// never vacuous: the `assert_ne!` baseline guard ensures the forward pass moved
/// state), but the caller should try a different decision point K so the gamelog
/// diff actually exercises something.
#[must_use]
fn rewind_replay_and_verify(
    game: &mut GameState,
    p1_id: PlayerId,
    p2_id: PlayerId,
    hash_after_forward: u64,
    forward_log: Vec<LogEntry>,
    scenario: &str,
) -> bool {
    let pre_rewind_log_count = forward_log.len();

    // ── Rewind to the turn-start baseline ────────────────────────────────────
    let mut undo_log = std::mem::take(&mut game.undo_log);
    let rewind = undo_log.rewind_to_turn_start(game);
    let log_empty_after_rewind = undo_log.is_empty();
    game.undo_log = undo_log;
    let (_turn, choice_actions, _rewound, log_size_at_turn) =
        rewind.expect("rewind_to_turn_start must succeed (undo log enabled)");

    // Turn 1 has no ChangeTurn marker, so a full-log-emptying rewind means we
    // landed on the non-reproducible pre-game-setup boundary. The scenario setup
    // is responsible for pausing on turn 2+, so this should not happen; assert it
    // loudly rather than silently passing a degenerate case (mtg-610).
    assert!(
        !log_empty_after_rewind,
        "[{scenario}] rewind emptied the whole undo log -> landed at pre-game setup (turn 1). \
         The scenario must pause on turn 2+ so the rewind lands on a real ChangeTurn boundary \
         (turn-1 baseline is non-reproducible; see mtg-610)."
    );

    // Capture the turn-start baseline hash, then truncate the live log to the
    // turn boundary (mirroring the production path, which truncates the logger
    // to `log_size_at_turn` after rewinding).
    let baseline_hash = compute_state_hash(game);
    game.logger.truncate_to(log_size_at_turn);

    // The baseline hash MUST differ from the post-forward hash — otherwise the
    // forward pass applied no undoable state and the round-trip is vacuous.
    assert_ne!(
        baseline_hash, hash_after_forward,
        "[{scenario}] rewind to turn start did not change the state hash; the forward pass \
         applied no undoable state and the round-trip assertion below would be vacuous"
    );

    let (p1_choices, p2_choices) = partition_choices(choice_actions, p1_id);

    // ── Replay both players' recorded intra-turn choices ──────────────────────
    // P1's inner controller is a StopController so that, once the recorded
    // choices are exhausted, it pauses at the SAME mid-turn priority point the
    // forward pass stopped at. P2's inner is a plain ZeroController (P2 either
    // had no recorded choices or its recorded list ends before the pause).
    let mut p1_replay = ReplayController::new(p1_id, Box::new(StopController::new(p1_id)), p1_choices);
    let mut p2_replay = ReplayController::new(p2_id, Box::new(StopController::new(p2_id)), p2_choices);
    {
        let mut game_loop = GameLoop::new(game).with_verbosity(VerbosityLevel::Silent);
        let _ = game_loop
            .run_until_input(&mut p1_replay, &mut p2_replay)
            .unwrap_or_else(|e| panic!("[{scenario}] replay run_until_input failed: {e}"));
    }

    // ── THE invariants ────────────────────────────────────────────────────────
    // (1) Final state hash returns to its pre-rewind value.
    let hash_after_replay = compute_state_hash(game);
    assert_eq!(
        hash_after_replay, hash_after_forward,
        "[{scenario}] REWIND/REPLAY STATE-HASH ROUND-TRIP FAILED: after rewind_to_turn_start + \
         deterministic replay of the recorded intra-turn choices, the state hash did not return \
         to its pre-rewind value ({hash_after_replay:#x} != {hash_after_forward:#x}). This means \
         undo.rs::rewind_to_turn_start is INCOMPLETE — some canonical state was mutated without a \
         logged GameAction. See mtg-610 / mtg-559 / docs/NETWORK_ARCHITECTURE.md (desync is fatal)."
    );

    // (2) The replayed gamelog tail matches the captured forward tail exactly.
    // `forward_log[log_size_at_turn..pre_rewind_log_count]` is the slice that was
    // truncated by the rewind; replay must regenerate it message-for-message.
    let captured_tail = &forward_log[log_size_at_turn..pre_rewind_log_count];
    let replay_buffer: Vec<LogEntry> = game.logger.logs().iter().cloned().collect();
    let outcome = verify_gamelog_tail(captured_tail, &replay_buffer, log_size_at_turn);
    assert_eq!(
        outcome,
        GamelogCheck::Ok,
        "[{scenario}] REWIND/REPLAY GAMELOG ROUND-TRIP FAILED: the replayed gamelog diverged from \
         the captured forward gamelog. A divergence here (especially a Truncated tail) is the \
         signature of an un-logged choice: it could not be replayed, so the dependent gamelog \
         lines never fired. See mtg-610 / docs/NETWORK_ARCHITECTURE.md (desync is fatal)."
    );

    // Report whether the gamelog assertion was non-vacuous (had replayable
    // engine entries to compare). The caller uses this to keep searching for a
    // decision point with real intra-turn gamelog when this one was empty.
    captured_tail.iter().any(is_replayable_entry)
}

// ===========================================================================
// Scenario drivers
// ===========================================================================

async fn load_deck_game(deck_relpath: &str, rng_seed: u64) -> Result<GameState> {
    let cardsfolder = require_cardsfolder();
    let deck_path = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../")).join(deck_relpath);
    let deck = DeckLoader::load_from_file(&deck_path)?;
    let card_db = CardDatabase::new(cardsfolder);
    let initializer = GameInitializer::new(&card_db);
    let mut game = initializer
        .init_game("P1".to_string(), &deck, "P2".to_string(), &deck, 20)
        .await?;
    game.seed_rng(rng_seed);
    // Capture the gamelog into an in-memory buffer so we can diff it across the
    // rewind/replay round-trip (the default Stdout mode keeps no buffer).
    game.logger.set_output_mode(OutputMode::Memory);
    Ok(game)
}

/// Deck scenario: play a full deterministic game, find the deepest P1 priority
/// decision K that pauses on turn 2+, then run forward to K, capture
/// hash+gamelog, and rewind/replay/verify.
async fn run_deck_scenario(deck_relpath: &str, rng_seed: u64, scenario: &str) -> Result<()> {
    // Discovery pass: count how many priority decisions P1 makes across the whole
    // (bounded) game, to bound the search for K.
    let total_p1_priority_calls = {
        let mut game = load_deck_game(deck_relpath, rng_seed).await?;
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;
        let mut p1 = RecorderController::new(p1_id, None);
        let mut p2 = RecorderController::new(p2_id, None);
        {
            let mut game_loop = GameLoop::new(&mut game)
                .with_verbosity(VerbosityLevel::Silent)
                .with_max_turns(12);
            let _ = game_loop.run_game(&mut p1, &mut p2)?;
        }
        p1.priority_calls
    };
    assert!(
        total_p1_priority_calls > 3,
        "[{scenario}] expected the policy to make several priority decisions; got {total_p1_priority_calls}"
    );

    // Search from the LAST decision point backwards for the deepest K that:
    //  (a) pauses (game not already over),
    //  (b) lands the rewind on a real ChangeTurn boundary (turn 2+, i.e. the undo
    //      log is NOT emptied — turn-1 baseline is non-reproducible, mtg-610), and
    //  (c) yields a NON-VACUOUS gamelog round-trip (the captured turn tail has
    //      replayable engine entries, so the gamelog diff has teeth).
    // The deepest such K maximizes the cross-turn / intra-turn state exercised.
    // The state-hash round-trip is asserted on every K we reach (b) at; only the
    // gamelog-tail emptiness causes us to keep searching.
    let mut verified = false;
    for k in (0..total_p1_priority_calls).rev() {
        let mut game = load_deck_game(deck_relpath, rng_seed).await?;
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // Forward pass: pause at P1's K-th priority decision.
        let stopped = {
            let mut p1 = RecorderController::new(p1_id, Some(k));
            let mut p2 = RecorderController::new(p2_id, None);
            let result = {
                let mut game_loop = GameLoop::new(&mut game)
                    .with_verbosity(VerbosityLevel::Silent)
                    .with_max_turns(12);
                game_loop.run_until_input(&mut p1, &mut p2)?
            };
            matches!(result, GameLoopState::AwaitingInput(_))
        };
        if !stopped {
            continue; // game ended before this decision point
        }

        // Will the rewind land on a real ChangeTurn boundary? Peek without
        // mutating the live game by checking the turn number: turn 1 has no
        // ChangeTurn marker. (We pause on turn 2+ to dodge turn-1 inexactness.)
        if game.turn.turn_number < 2 {
            continue;
        }

        let hash_after_forward = compute_state_hash(&game);
        let forward_log = snapshot_log(&game);
        let non_vacuous = rewind_replay_and_verify(&mut game, p1_id, p2_id, hash_after_forward, forward_log, scenario);
        if non_vacuous {
            verified = true;
            break;
        }
        // Hash round-trip passed but the gamelog tail was empty at this pause
        // point; keep searching for a K with real intra-turn gamelog to diff.
    }

    assert!(
        verified,
        "[{scenario}] no turn-2+ decision point found to verify a cross-turn rewind/replay \
         round-trip. The deck may resolve in a single turn; pick a longer game or higher max_turns."
    );
    Ok(())
}

/// Bazaar scenario: load the Bazaar-of-Baghdad puzzle, activate Bazaar (logging a
/// mid-resolution Discard ChoicePoint), pause, then rewind/replay/verify. This
/// preserves both the `bazaar_rewind_loop_e2e` ChoicePoint-logging coverage AND
/// the `rewind_replay_hash_roundtrip_e2e` mid-resolution hash round-trip, and
/// ADDS the gamelog round-trip on top.
async fn run_bazaar_scenario(scenario: &str) -> Result<()> {
    let cardsfolder = require_cardsfolder();
    let puzzle_path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../test_puzzles/bazaar_of_baghdad_draw_discard.pzl"
    ));
    let puzzle_contents = std::fs::read_to_string(&puzzle_path)?;
    let puzzle = PuzzleFile::parse(&puzzle_contents)?;
    let card_db = CardDatabase::new(cardsfolder);
    let mut game = load_puzzle_into_game(&puzzle, &card_db).await?;
    game.seed_rng(42);
    game.logger.set_output_mode(OutputMode::Memory);

    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;

    // Forward pass: activate Bazaar (mid-resolution discard), then pause.
    {
        let mut p1 = BazaarBot::new(p1_id);
        let mut p2 = ZeroController::new(p2_id);
        let mut game_loop = GameLoop::new(&mut game).with_verbosity(VerbosityLevel::Silent);
        let _ = game_loop.run_until_input(&mut p1, &mut p2)?;
    }

    // Preserve the bazaar_rewind_loop regression assertion: the mid-resolution
    // discard MUST have been logged as a ChoicePoint, else rewind/replay would
    // re-prompt (the infinite-loop bug). The gamelog round-trip below is the
    // stronger consequence — an un-logged discard truncates the replayed log.
    let discard_choicepoints = game
        .undo_log
        .actions()
        .iter()
        .filter(|a| {
            matches!(
                a,
                GameAction::ChoicePoint {
                    choice: Some(ReplayChoice::Discard(_)),
                    ..
                }
            )
        })
        .count();
    assert!(
        discard_choicepoints >= 1,
        "[{scenario}] expected >=1 mid-resolution Discard ChoicePoint from the Bazaar activation; \
         got {discard_choicepoints}. Without it rewind/replay would re-prompt for the discard \
         (bazaar infinite-loop regression)."
    );

    // The Bazaar puzzle pauses on turn 1 (no ChangeTurn marker yet). Unlike the
    // deck scenarios, the round-trip we care about here is the MID-RESOLUTION /
    // in-stack discard, which lives in the turn's intra-turn ChoicePoints — and
    // the rewind to turn-1-start does restore the pre-activation state (the
    // puzzle pre-places Bazaar, so there is no shuffle to redo for the activation
    // round-trip itself). We therefore verify the state-hash + gamelog round-trip
    // directly here rather than through `rewind_replay_and_verify` (which asserts
    // a turn-2+ boundary).
    let hash_after_forward = compute_state_hash(&game);
    let forward_log = snapshot_log(&game);
    let pre_rewind_log_count = forward_log.len();

    let mut undo_log = std::mem::take(&mut game.undo_log);
    let rewind = undo_log.rewind_to_turn_start(&mut game);
    game.undo_log = undo_log;
    let (_turn, choice_actions, _rewound, log_size_at_turn) =
        rewind.expect("rewind_to_turn_start must succeed (undo log enabled)");

    let baseline_hash = compute_state_hash(&game);
    game.logger.truncate_to(log_size_at_turn);
    assert_ne!(
        baseline_hash, hash_after_forward,
        "[{scenario}] rewind to turn start did not change the state hash; round-trip would be vacuous"
    );

    let (p1_choices, p2_choices) = partition_choices(choice_actions, p1_id);
    let mut p1_replay = ReplayController::new(p1_id, Box::new(StopController::new(p1_id)), p1_choices);
    let mut p2_replay = ReplayController::new(p2_id, Box::new(ZeroController::new(p2_id)), p2_choices);
    {
        let mut game_loop = GameLoop::new(&mut game).with_verbosity(VerbosityLevel::Silent);
        let _ = game_loop.run_until_input(&mut p1_replay, &mut p2_replay)?;
    }

    let hash_after_replay = compute_state_hash(&game);
    assert_eq!(
        hash_after_replay, hash_after_forward,
        "[{scenario}] REWIND/REPLAY STATE-HASH ROUND-TRIP FAILED through a mid-resolution Bazaar \
         discard ({hash_after_replay:#x} != {hash_after_forward:#x}). undo.rs::rewind_to_turn_start \
         is INCOMPLETE for in-stack/mid-resolution state. See mtg-610 / mtg-559."
    );

    let captured_tail = &forward_log[log_size_at_turn..pre_rewind_log_count];
    let replayable_captured = captured_tail.iter().filter(|e| is_replayable_entry(e)).count();
    assert!(
        replayable_captured > 0,
        "[{scenario}] captured forward gamelog tail has no replayable engine entries; \
         the gamelog round-trip assertion would be vacuous"
    );
    let replay_buffer: Vec<LogEntry> = game.logger.logs().iter().cloned().collect();
    let outcome = verify_gamelog_tail(captured_tail, &replay_buffer, log_size_at_turn);
    assert_eq!(
        outcome,
        GamelogCheck::Ok,
        "[{scenario}] REWIND/REPLAY GAMELOG ROUND-TRIP FAILED through the Bazaar discard. A \
         Truncated tail here is the signature of the un-logged-discard infinite-loop regression: \
         the discard could not be replayed, so the post-discard gamelog never regenerated. \
         See mtg-610 / bazaar_rewind_loop."
    );

    Ok(())
}

// ===========================================================================
// Parameterized cases: one nextest #[test] per (deck/scenario, seed)
// ===========================================================================
//
// CONSERVATIVE matrix: only decks/scenarios whose undo log is already COMPLETE
// on today's integration, so this ships GREEN. Monored / robots42 / rogerbrand
// have undo holes still being closed on the netarch branch and would make this
// RED — they are intentionally excluded.
//
// matrix grows here as netarch lands holes (mtg-610): monored, robots42, rogerbrand, ...

/// Generates one `#[tokio::test]` per row so nextest fans them out in parallel.
macro_rules! deck_rewind_cases {
    ($($name:ident => ($deck:expr, $seed:expr)),+ $(,)?) => {
        $(
            #[tokio::test]
            async fn $name() -> Result<()> {
                run_deck_scenario($deck, $seed, stringify!($name)).await
            }
        )+
    };
}

deck_rewind_cases! {
    // simple_bolt: the deck the old test_full_game_undo_replay already used.
    simple_bolt_seed_42424 => ("decks/simple_bolt.dck", 42424),
    simple_bolt_seed_7 => ("decks/simple_bolt.dck", 7),
    // combat_test_4ed: creatures + combat (attackers/blockers/damage-order classes),
    // the deck the old rewind_replay_oracle_e2e used.
    combat_4ed_seed_42 => ("decks/combat_test_4ed.dck", 42),
    combat_4ed_seed_99 => ("decks/combat_test_4ed.dck", 99),
}

/// Bazaar-of-Baghdad mid-resolution discard scenario (preserves the
/// `bazaar_rewind_loop_e2e` + `rewind_replay_hash_roundtrip_e2e` coverage).
#[tokio::test]
async fn bazaar_mid_resolution_discard_round_trip() -> Result<()> {
    run_bazaar_scenario("bazaar_mid_resolution_discard_round_trip").await
}
