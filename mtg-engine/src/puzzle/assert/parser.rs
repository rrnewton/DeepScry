//! Assertion DSL parser
//!
//! Converts lines from the `[assertions]` section into a typed `Vec<Assertion>`.
//! All parsing is token-based — no substring matching on structured data.
//!
//! Grammar (simplified EBNF):
//! ```text
//! assertion    ::= negation? player_scope? predicate
//! negation     ::= 'NOT'
//! player_scope ::= 'me' | 'opponent'
//! predicate    ::= life_pred | zone_count_pred | zone_contains_pred
//!                | library_top_pred | game_result_pred | turn_pred
//! ```
//!
//! See `ai_docs/reference/PUZZLE_ASSERTION_DSL.md` for the full grammar.

use crate::{
    puzzle::assert::{AssertZone, Assertion, AssertionKind, Comparator, GameResultPred, PlayerScope},
    MtgError, Result,
};

/// Parse the full `[assertions]` section lines into a list of assertions.
///
/// Blank lines and lines starting with `#` are skipped. Each other line must
/// be a valid assertion or parsing fails.
///
/// # Errors
///
/// Returns an error if any non-blank, non-comment line is not a valid assertion.
pub fn parse_assertions(lines: &[String]) -> Result<Vec<Assertion>> {
    let mut out = Vec::new();
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        out.push(parse_one_assertion(trimmed)?);
    }
    Ok(out)
}

/// Parse one assertion line.
///
/// # Errors
///
/// Returns an error if the line is not a valid assertion.
pub fn parse_one_assertion(line: &str) -> Result<Assertion> {
    let mut tokens: std::collections::VecDeque<&str> = line.split_whitespace().collect();

    if tokens.is_empty() {
        return Err(MtgError::ParseError("Empty assertion line".to_string()));
    }

    // Optional NOT prefix
    let negated = if tokens.front().copied() == Some("NOT") {
        tokens.pop_front();
        true
    } else {
        false
    };

    if tokens.is_empty() {
        return Err(MtgError::ParseError(
            "Assertion has only 'NOT' with no predicate".to_string(),
        ));
    }

    // Optional player scope
    let scope = match tokens.front().copied() {
        Some("me") => {
            tokens.pop_front();
            PlayerScope::Me
        }
        Some("opponent") => {
            tokens.pop_front();
            PlayerScope::Opponent
        }
        _ => PlayerScope::Me,
    };

    if tokens.is_empty() {
        return Err(MtgError::ParseError(
            "Assertion has scope but no predicate keyword".to_string(),
        ));
    }

    let kind = parse_predicate(&mut tokens, scope, line)?;

    if !tokens.is_empty() {
        return Err(MtgError::ParseError(format!(
            "Unexpected extra tokens in assertion '{}': {:?}",
            line,
            tokens.iter().collect::<Vec<_>>()
        )));
    }

    Ok(Assertion {
        negated,
        kind,
        source_line: line.to_string(),
    })
}

/// Parse the predicate part (after optional NOT and scope).
fn parse_predicate(
    tokens: &mut std::collections::VecDeque<&str>,
    scope: PlayerScope,
    source: &str,
) -> Result<AssertionKind> {
    let keyword = tokens.pop_front().unwrap_or("");

    match keyword {
        "life" => {
            let cmp = expect_comparator(tokens, source)?;
            let value = expect_i32(tokens, source)?;
            Ok(AssertionKind::Life { scope, cmp, value })
        }

        "hand" | "graveyard" | "battlefield" | "exile" | "library" => {
            let zone = parse_zone(keyword).expect("keyword already matched zone");
            parse_zone_predicate(tokens, scope, zone, source)
        }

        "game" => {
            let pred_word = tokens.pop_front().ok_or_else(|| {
                MtgError::ParseError(format!(
                    "Expected 'won', 'lost', 'drawn', or 'ended' after 'game' in '{}'",
                    source
                ))
            })?;
            let pred = match pred_word {
                "won" => GameResultPred::Won,
                "lost" => GameResultPred::Lost,
                "drawn" => GameResultPred::Drawn,
                "ended" => GameResultPred::Ended,
                other => {
                    return Err(MtgError::ParseError(format!(
                        "Unknown game result predicate '{}' in '{}'. \
                         Expected: won, lost, drawn, ended",
                        other, source
                    )))
                }
            };
            Ok(AssertionKind::GameResult(pred))
        }

        "turn" => {
            let cmp = expect_comparator(tokens, source)?;
            let value = expect_u32(tokens, source)?;
            Ok(AssertionKind::TurnNumber { cmp, value })
        }

        other => Err(MtgError::ParseError(format!(
            "Unknown assertion keyword '{}' in '{}'. \
             Expected: life, hand, graveyard, battlefield, exile, library, game, turn",
            other, source
        ))),
    }
}

/// Parse zone predicates: `<zone> count <cmp> <n>` or `<zone> contains <name>` or
/// `library top <n> contains <name>`.
fn parse_zone_predicate(
    tokens: &mut std::collections::VecDeque<&str>,
    scope: PlayerScope,
    zone: AssertZone,
    source: &str,
) -> Result<AssertionKind> {
    let next = tokens.pop_front().ok_or_else(|| {
        MtgError::ParseError(format!(
            "Expected 'count', 'contains', or 'top' after zone keyword in '{}'",
            source
        ))
    })?;

    match next {
        "count" => {
            let cmp = expect_comparator(tokens, source)?;
            let value = expect_usize(tokens, source)?;
            Ok(AssertionKind::ZoneCount {
                scope,
                zone,
                cmp,
                value,
            })
        }

        "contains" => {
            let card_name = collect_remaining(tokens);
            if card_name.is_empty() {
                return Err(MtgError::ParseError(format!(
                    "Missing card name after 'contains' in '{}'",
                    source
                )));
            }
            Ok(AssertionKind::ZoneContains { scope, zone, card_name })
        }

        // Special case: `library top N contains <name>`
        "top" if zone == AssertZone::Library => {
            let depth = expect_usize(tokens, source)?;
            let contains_kw = tokens.pop_front().ok_or_else(|| {
                MtgError::ParseError(format!("Expected 'contains' after 'library top N' in '{}'", source))
            })?;
            if contains_kw != "contains" {
                return Err(MtgError::ParseError(format!(
                    "Expected 'contains' after 'library top {}', got '{}' in '{}'",
                    depth, contains_kw, source
                )));
            }
            let card_name = collect_remaining(tokens);
            if card_name.is_empty() {
                return Err(MtgError::ParseError(format!(
                    "Missing card name after 'library top {} contains' in '{}'",
                    depth, source
                )));
            }
            Ok(AssertionKind::LibraryTopContains {
                scope,
                depth,
                card_name,
            })
        }

        other => Err(MtgError::ParseError(format!(
            "Expected 'count', 'contains', or 'top' (for library) after zone keyword in '{}', got '{}'",
            source, other
        ))),
    }
}

// ─── helpers ─────────────────────────────────────────────────────────────────

fn parse_zone(s: &str) -> Option<AssertZone> {
    match s {
        "hand" => Some(AssertZone::Hand),
        "graveyard" => Some(AssertZone::Graveyard),
        "battlefield" => Some(AssertZone::Battlefield),
        "exile" => Some(AssertZone::Exile),
        "library" => Some(AssertZone::Library),
        _ => None,
    }
}

fn expect_comparator(tokens: &mut std::collections::VecDeque<&str>, source: &str) -> Result<Comparator> {
    let tok = tokens
        .pop_front()
        .ok_or_else(|| MtgError::ParseError(format!("Expected comparator (eq/ne/lt/le/gt/ge) in '{}'", source)))?;
    match tok {
        "eq" => Ok(Comparator::Eq),
        "ne" => Ok(Comparator::Ne),
        "lt" => Ok(Comparator::Lt),
        "le" => Ok(Comparator::Le),
        "gt" => Ok(Comparator::Gt),
        "ge" => Ok(Comparator::Ge),
        other => Err(MtgError::ParseError(format!(
            "Unknown comparator '{}' in '{}'. Expected: eq, ne, lt, le, gt, ge",
            other, source
        ))),
    }
}

fn expect_i32(tokens: &mut std::collections::VecDeque<&str>, source: &str) -> Result<i32> {
    let tok = tokens
        .pop_front()
        .ok_or_else(|| MtgError::ParseError(format!("Expected integer value in '{}'", source)))?;
    tok.parse::<i32>()
        .map_err(|_| MtgError::ParseError(format!("Expected integer, got '{}' in '{}'", tok, source)))
}

fn expect_u32(tokens: &mut std::collections::VecDeque<&str>, source: &str) -> Result<u32> {
    let tok = tokens
        .pop_front()
        .ok_or_else(|| MtgError::ParseError(format!("Expected non-negative integer in '{}'", source)))?;
    tok.parse::<u32>()
        .map_err(|_| MtgError::ParseError(format!("Expected non-negative integer, got '{}' in '{}'", tok, source)))
}

fn expect_usize(tokens: &mut std::collections::VecDeque<&str>, source: &str) -> Result<usize> {
    let tok = tokens
        .pop_front()
        .ok_or_else(|| MtgError::ParseError(format!("Expected non-negative integer in '{}'", source)))?;
    tok.parse::<usize>()
        .map_err(|_| MtgError::ParseError(format!("Expected non-negative integer, got '{}' in '{}'", tok, source)))
}

/// Collect all remaining tokens into a space-joined string (for card names with spaces)
fn collect_remaining(tokens: &mut std::collections::VecDeque<&str>) -> String {
    let parts: Vec<&str> = tokens.drain(..).collect();
    parts.join(" ")
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Assertion {
        parse_one_assertion(s).unwrap_or_else(|e| panic!("Failed to parse '{s}': {e}"))
    }

    fn parse_err(s: &str) -> String {
        parse_one_assertion(s).expect_err("Expected parse error").to_string()
    }

    // ── Life assertions ──────────────────────────────────────────────────────

    #[test]
    fn test_life_eq() {
        let a = parse("life eq 20");
        assert!(!a.negated);
        assert!(matches!(
            a.kind,
            AssertionKind::Life {
                scope: PlayerScope::Me,
                cmp: Comparator::Eq,
                value: 20
            }
        ));
    }

    #[test]
    fn test_life_opponent_lt() {
        let a = parse("opponent life lt 5");
        assert!(matches!(
            a.kind,
            AssertionKind::Life {
                scope: PlayerScope::Opponent,
                cmp: Comparator::Lt,
                value: 5
            }
        ));
    }

    #[test]
    fn test_life_negated() {
        let a = parse("NOT life eq 0");
        assert!(a.negated);
        assert!(matches!(a.kind, AssertionKind::Life { value: 0, .. }));
    }

    // ── Zone count assertions ────────────────────────────────────────────────

    #[test]
    fn test_hand_count_ge() {
        let a = parse("hand count ge 2");
        assert!(matches!(
            a.kind,
            AssertionKind::ZoneCount {
                zone: AssertZone::Hand,
                cmp: Comparator::Ge,
                value: 2,
                ..
            }
        ));
    }

    #[test]
    fn test_graveyard_count_eq_zero() {
        let a = parse("opponent graveyard count eq 0");
        assert!(matches!(
            a.kind,
            AssertionKind::ZoneCount {
                scope: PlayerScope::Opponent,
                zone: AssertZone::Graveyard,
                value: 0,
                ..
            }
        ));
    }

    #[test]
    fn test_battlefield_count_gt() {
        let a = parse("battlefield count gt 0");
        assert!(matches!(
            a.kind,
            AssertionKind::ZoneCount {
                zone: AssertZone::Battlefield,
                ..
            }
        ));
    }

    // ── Zone contains assertions ─────────────────────────────────────────────

    #[test]
    fn test_graveyard_contains_single_word() {
        let a = parse("graveyard contains Mountain");
        if let AssertionKind::ZoneContains { card_name, .. } = a.kind {
            assert_eq!(card_name, "Mountain");
        } else {
            panic!("Wrong kind");
        }
    }

    #[test]
    fn test_graveyard_contains_multiword() {
        let a = parse("graveyard contains Grizzly Bears");
        if let AssertionKind::ZoneContains { card_name, .. } = a.kind {
            assert_eq!(card_name, "Grizzly Bears");
        } else {
            panic!("Wrong kind");
        }
    }

    #[test]
    fn test_hand_contains_opponent() {
        let a = parse("opponent hand contains Lightning Bolt");
        if let AssertionKind::ZoneContains { scope, zone, card_name } = a.kind {
            assert_eq!(scope, PlayerScope::Opponent);
            assert_eq!(zone, AssertZone::Hand);
            assert_eq!(card_name, "Lightning Bolt");
        } else {
            panic!("Wrong kind");
        }
    }

    // ── Library top assertions ───────────────────────────────────────────────

    #[test]
    fn test_library_top_contains() {
        let a = parse("library top 3 contains Forest");
        if let AssertionKind::LibraryTopContains { depth, card_name, .. } = a.kind {
            assert_eq!(depth, 3);
            assert_eq!(card_name, "Forest");
        } else {
            panic!("Wrong kind");
        }
    }

    // ── Game result assertions ───────────────────────────────────────────────

    #[test]
    fn test_game_won() {
        let a = parse("game won");
        assert!(matches!(a.kind, AssertionKind::GameResult(GameResultPred::Won)));
    }

    #[test]
    fn test_game_lost() {
        let a = parse("game lost");
        assert!(matches!(a.kind, AssertionKind::GameResult(GameResultPred::Lost)));
    }

    #[test]
    fn test_game_drawn() {
        let a = parse("game drawn");
        assert!(matches!(a.kind, AssertionKind::GameResult(GameResultPred::Drawn)));
    }

    #[test]
    fn test_game_ended() {
        let a = parse("game ended");
        assert!(matches!(a.kind, AssertionKind::GameResult(GameResultPred::Ended)));
    }

    #[test]
    fn test_not_game_lost() {
        let a = parse("NOT game lost");
        assert!(a.negated);
        assert!(matches!(a.kind, AssertionKind::GameResult(GameResultPred::Lost)));
    }

    // ── Turn assertions ──────────────────────────────────────────────────────

    #[test]
    fn test_turn_le() {
        let a = parse("turn le 3");
        assert!(matches!(
            a.kind,
            AssertionKind::TurnNumber {
                cmp: Comparator::Le,
                value: 3
            }
        ));
    }

    // ── Bulk parsing ────────────────────────────────────────────────────────

    #[test]
    fn test_parse_assertions_skips_blanks_and_comments() {
        let lines: Vec<String> = vec![
            "# This is a comment".to_string(),
            "".to_string(),
            "life eq 20".to_string(),
            "# Another comment".to_string(),
            "game won".to_string(),
        ];
        let result = parse_assertions(&lines).unwrap();
        assert_eq!(result.len(), 2);
    }

    // ── Error cases ──────────────────────────────────────────────────────────

    #[test]
    fn test_error_unknown_keyword() {
        let err = parse_err("flying eq 1");
        assert!(err.contains("flying"), "Error should mention the bad keyword");
    }

    #[test]
    fn test_error_missing_comparator() {
        let err = parse_err("life");
        assert!(err.contains("comparator") || err.contains("eq"), "Error: {err}");
    }

    #[test]
    fn test_error_bad_integer() {
        let err = parse_err("life eq notanumber");
        assert!(err.contains("integer") || err.contains("notanumber"), "Error: {err}");
    }

    #[test]
    fn test_error_not_only() {
        let err = parse_err("NOT");
        assert!(err.contains("predicate") || err.contains("NOT"), "Error: {err}");
    }

    #[test]
    fn test_error_zone_contains_no_name() {
        let err = parse_err("graveyard contains");
        assert!(err.contains("card name") || err.contains("contains"), "Error: {err}");
    }

    #[test]
    fn test_error_library_top_no_contains() {
        let err = parse_err("library top 2 forest");
        assert!(err.contains("contains") || err.contains("forest"), "Error: {err}");
    }

    #[test]
    fn test_error_unknown_game_result() {
        let err = parse_err("game survived");
        assert!(err.contains("survived") || err.contains("won"), "Error: {err}");
    }

    #[test]
    fn test_all_comparators_parse() {
        for (s, expected) in &[
            ("eq", Comparator::Eq),
            ("ne", Comparator::Ne),
            ("lt", Comparator::Lt),
            ("le", Comparator::Le),
            ("gt", Comparator::Gt),
            ("ge", Comparator::Ge),
        ] {
            let a = parse(&format!("life {} 10", s));
            if let AssertionKind::Life { cmp, .. } = a.kind {
                assert_eq!(cmp, *expected, "Comparator mismatch for {s}");
            } else {
                panic!("Expected life assertion for {s}");
            }
        }
    }
}
