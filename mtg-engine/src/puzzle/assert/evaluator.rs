//! Assertion evaluator
//!
//! Evaluates a list of typed `Assertion`s against a final `GameState` and
//! `GameResult`. Returns an `AssertionReport` listing which assertions passed
//! and which failed, with per-assertion failure messages.
//!
//! The evaluator is a **pure library function** — it takes immutable references
//! and has no side effects. The caller (CLI runner or integration test) decides
//! what to do with the report.
//!
//! # Data sources used
//! - Life totals: `GameState::get_player_by_idx(idx)?.life`
//! - Zone contents (hand/graveyard/exile/library/command):
//!   `GameState::get_player_zones(id)?.<zone>.cards`
//! - Battlefield (per-controller filter): `GameState::battlefield.cards`
//!   filtered by `Card::controller == player_id`
//! - Card names: `GameState::cards.get(card_id)?.name`
//! - Game result / turns played: `GameResult::winner`, `::end_reason`,
//!   `::turns_played`

use crate::{
    core::PlayerId,
    game::{GameEndReason, GameResult, GameState},
    puzzle::assert::{AssertZone, Assertion, AssertionKind, GameResultPred, PlayerScope},
};

/// The result of running the assertion evaluator against one puzzle run.
#[derive(Debug)]
pub struct AssertionReport {
    pub passed: Vec<String>,
    pub failed: Vec<AssertionFailure>,
}

impl AssertionReport {
    /// True if every assertion passed.
    pub fn all_passed(&self) -> bool {
        self.failed.is_empty()
    }

    /// Format a human-readable multi-line summary.
    pub fn summary(&self) -> String {
        let total = self.passed.len() + self.failed.len();
        let mut out = format!("Assertions: {}/{} passed", self.passed.len(), total);
        for f in &self.failed {
            out.push('\n');
            out.push_str("  FAIL: ");
            out.push_str(&f.source_line);
            out.push('\n');
            out.push_str("        ");
            out.push_str(&f.reason);
        }
        out
    }
}

/// A single failed assertion with its source line and the reason it failed.
#[derive(Debug)]
pub struct AssertionFailure {
    pub source_line: String,
    pub reason: String,
}

/// Evaluate all assertions in `assertions` against the final game state.
///
/// The "me" player is always `game.players[0]` (P0, the puzzle's human player).
/// The "opponent" is `game.players[1]` (P1).
pub fn evaluate_assertions(assertions: &[Assertion], game: &GameState, result: &GameResult) -> AssertionReport {
    let mut passed = Vec::new();
    let mut failed = Vec::new();

    // Resolve P0/P1 ids once — we don't want per-assertion lookups failing
    // differently if the game somehow has a different player ordering.
    let me_id = game.players.first().map(|p| p.id);
    let opp_id = game.players.get(1).map(|p| p.id);

    for assertion in assertions {
        let outcome = eval_one(assertion, game, result, me_id, opp_id);
        match outcome {
            Ok(true) => passed.push(assertion.source_line.clone()),
            Ok(false) => failed.push(AssertionFailure {
                source_line: assertion.source_line.clone(),
                reason: "predicate evaluated to false".to_string(),
            }),
            Err(reason) => failed.push(AssertionFailure {
                source_line: assertion.source_line.clone(),
                reason,
            }),
        }
    }

    AssertionReport { passed, failed }
}

/// Evaluate one assertion. Returns `Ok(true)` on pass, `Ok(false)` on clean
/// fail, `Err(msg)` on evaluation error (e.g., player not found).
fn eval_one(
    assertion: &Assertion,
    game: &GameState,
    result: &GameResult,
    me_id: Option<PlayerId>,
    opp_id: Option<PlayerId>,
) -> Result<bool, String> {
    let raw = eval_kind(&assertion.kind, game, result, me_id, opp_id)?;
    Ok(if assertion.negated { !raw } else { raw })
}

fn eval_kind(
    kind: &AssertionKind,
    game: &GameState,
    result: &GameResult,
    me_id: Option<PlayerId>,
    opp_id: Option<PlayerId>,
) -> Result<bool, String> {
    match kind {
        AssertionKind::Life { scope, cmp, value } => {
            let player_id = resolve_id(*scope, me_id, opp_id)?;
            let player = game
                .get_player(player_id)
                .map_err(|e| format!("get_player failed: {e}"))?;
            Ok(cmp.eval(player.life, *value))
        }

        AssertionKind::ZoneCount {
            scope,
            zone,
            cmp,
            value,
        } => {
            let player_id = resolve_id(*scope, me_id, opp_id)?;
            let count = zone_count(game, player_id, *zone)?;
            Ok(cmp.eval(count, *value))
        }

        AssertionKind::ZoneContains { scope, zone, card_name } => {
            let player_id = resolve_id(*scope, me_id, opp_id)?;
            zone_contains(game, player_id, *zone, card_name)
        }

        AssertionKind::LibraryTopContains {
            scope,
            depth,
            card_name,
        } => {
            let player_id = resolve_id(*scope, me_id, opp_id)?;
            library_top_contains(game, player_id, *depth, card_name)
        }

        AssertionKind::GameResult(pred) => Ok(eval_game_result(*pred, result, me_id)),

        AssertionKind::TurnNumber { cmp, value } => Ok(cmp.eval(result.turns_played, *value)),
    }
}

// ─── helpers ─────────────────────────────────────────────────────────────────

fn resolve_id(scope: PlayerScope, me_id: Option<PlayerId>, opp_id: Option<PlayerId>) -> Result<PlayerId, String> {
    match scope {
        PlayerScope::Me => me_id.ok_or_else(|| "No P0 player in game".to_string()),
        PlayerScope::Opponent => opp_id.ok_or_else(|| "No P1 player in game".to_string()),
    }
}

fn zone_count(game: &GameState, player_id: PlayerId, zone: AssertZone) -> Result<usize, String> {
    if zone == AssertZone::Battlefield {
        return Ok(game
            .battlefield
            .cards
            .iter()
            .filter(|&&cid| game.cards.get(cid).map(|c| c.controller == player_id).unwrap_or(false))
            .count());
    }
    let zones = game
        .get_player_zones(player_id)
        .ok_or_else(|| format!("No zones for player {:?}", player_id))?;
    Ok(match zone {
        AssertZone::Hand => zones.hand.len(),
        AssertZone::Graveyard => zones.graveyard.len(),
        AssertZone::Exile => zones.exile.len(),
        AssertZone::Library => zones.library.len(),
        AssertZone::Battlefield => unreachable!(),
    })
}

fn zone_contains(game: &GameState, player_id: PlayerId, zone: AssertZone, card_name: &str) -> Result<bool, String> {
    let name_lower = card_name.to_lowercase();

    if zone == AssertZone::Battlefield {
        return Ok(game.battlefield.cards.iter().any(|&cid| {
            game.cards
                .get(cid)
                .map(|c| c.controller == player_id && c.name.as_str().to_lowercase() == name_lower)
                .unwrap_or(false)
        }));
    }

    let zones = game
        .get_player_zones(player_id)
        .ok_or_else(|| format!("No zones for player {:?}", player_id))?;

    let card_zone = match zone {
        AssertZone::Hand => &zones.hand,
        AssertZone::Graveyard => &zones.graveyard,
        AssertZone::Exile => &zones.exile,
        AssertZone::Library => &zones.library,
        AssertZone::Battlefield => unreachable!(),
    };

    Ok(card_zone.cards.iter().any(|&cid| {
        game.cards
            .get(cid)
            .map(|c| c.name.as_str().to_lowercase() == name_lower)
            .unwrap_or(false)
    }))
}

fn library_top_contains(game: &GameState, player_id: PlayerId, depth: usize, card_name: &str) -> Result<bool, String> {
    let name_lower = card_name.to_lowercase();
    let zones = game
        .get_player_zones(player_id)
        .ok_or_else(|| format!("No zones for player {:?}", player_id))?;

    let top_slice = if depth > zones.library.len() {
        &zones.library.cards[..]
    } else {
        &zones.library.cards[..depth]
    };

    Ok(top_slice.iter().any(|&cid| {
        game.cards
            .get(cid)
            .map(|c| c.name.as_str().to_lowercase() == name_lower)
            .unwrap_or(false)
    }))
}

/// Evaluate the game-result predicate. "me" = P0 = `me_id`.
fn eval_game_result(pred: GameResultPred, result: &GameResult, me_id: Option<PlayerId>) -> bool {
    match pred {
        GameResultPred::Won => me_id
            .and_then(|my_id| result.winner.map(|w| w == my_id))
            .unwrap_or(false),
        GameResultPred::Lost => me_id
            .and_then(|my_id| result.winner.map(|w| w != my_id))
            .unwrap_or(false),
        GameResultPred::Drawn => result.end_reason == GameEndReason::Draw,
        GameResultPred::Ended => result.winner.is_some() || result.end_reason == GameEndReason::Draw,
    }
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        core::PlayerId,
        game::{GameEndReason, GameResult},
        puzzle::assert::{AssertionKind, Comparator, GameResultPred, PlayerScope},
    };

    fn pid(n: u32) -> PlayerId {
        PlayerId::new(n)
    }

    fn make_result(winner: Option<PlayerId>, turns: u32, reason: GameEndReason) -> GameResult {
        GameResult {
            winner,
            turns_played: turns,
            end_reason: reason,
            action_count: 0,
        }
    }

    // ── eval_game_result (pure, no GameState needed) ─────────────────────────

    #[test]
    fn test_game_won_p0_wins() {
        let r = make_result(Some(pid(0)), 1, GameEndReason::PlayerDeath(pid(1)));
        assert!(eval_game_result(GameResultPred::Won, &r, Some(pid(0))));
    }

    #[test]
    fn test_game_won_p1_wins() {
        let r = make_result(Some(pid(1)), 1, GameEndReason::PlayerDeath(pid(0)));
        assert!(!eval_game_result(GameResultPred::Won, &r, Some(pid(0))));
    }

    #[test]
    fn test_game_lost_when_p1_wins() {
        let r = make_result(Some(pid(1)), 1, GameEndReason::PlayerDeath(pid(0)));
        assert!(eval_game_result(GameResultPred::Lost, &r, Some(pid(0))));
    }

    #[test]
    fn test_game_lost_when_p0_wins() {
        let r = make_result(Some(pid(0)), 1, GameEndReason::PlayerDeath(pid(1)));
        assert!(!eval_game_result(GameResultPred::Lost, &r, Some(pid(0))));
    }

    #[test]
    fn test_game_drawn() {
        let r = make_result(None, 3, GameEndReason::Draw);
        assert!(eval_game_result(GameResultPred::Drawn, &r, Some(pid(0))));
        assert!(!eval_game_result(GameResultPred::Won, &r, Some(pid(0))));
    }

    #[test]
    fn test_game_ended_draw_is_ended() {
        let r = make_result(None, 3, GameEndReason::Draw);
        assert!(eval_game_result(GameResultPred::Ended, &r, Some(pid(0))));
    }

    #[test]
    fn test_game_ended_turn_limit_is_not_ended() {
        // TurnLimit: no decisive result — "ended" = false (no winner, not a draw)
        let r = make_result(None, 5, GameEndReason::TurnLimit);
        assert!(!eval_game_result(GameResultPred::Ended, &r, Some(pid(0))));
    }

    // ── AssertionReport helpers ───────────────────────────────────────────────

    #[test]
    fn test_report_all_passed() {
        let report = AssertionReport {
            passed: vec!["life eq 20".to_string()],
            failed: vec![],
        };
        assert!(report.all_passed());
        assert!(report.summary().contains("1/1 passed"));
    }

    #[test]
    fn test_report_with_failure() {
        let report = AssertionReport {
            passed: vec![],
            failed: vec![AssertionFailure {
                source_line: "life eq 20".to_string(),
                reason: "predicate evaluated to false".to_string(),
            }],
        };
        assert!(!report.all_passed());
        let s = report.summary();
        assert!(s.contains("0/1 passed"));
        assert!(s.contains("FAIL"));
        assert!(s.contains("life eq 20"));
    }

    #[test]
    fn test_comparator_eval() {
        use Comparator::*;
        assert!(Eq.eval(5i32, 5));
        assert!(!Eq.eval(5i32, 6));
        assert!(Ne.eval(5i32, 6));
        assert!(Lt.eval(4i32, 5));
        assert!(Le.eval(5i32, 5));
        assert!(Gt.eval(6i32, 5));
        assert!(Ge.eval(5i32, 5));
    }
}
