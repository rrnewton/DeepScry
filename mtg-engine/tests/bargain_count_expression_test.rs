//! Tests for Count$Bargain.N.M CountExpression parsing and evaluation.
//!
//! Bargain (CR 702.162, The Wilds of Eldraine 2023) is an optional additional cost:
//! "You may sacrifice an artifact, enchantment, or token as you cast this spell."
//! Cards like Torch the Tower use `SVar:X:Count$Bargain.3.2` to deal 3 damage if
//! bargained, 2 if not. Before the fix (mtg-863), the `Count$Bargain.*` body fell
//! through to `CountExpression::Fixed(0)`, causing the spell to always deal 0.

use mtg_engine::core::CountExpression;
use std::collections::HashMap;

/// Count$Bargain.3.2 must parse to CountExpression::Bargain (not Fixed(0)).
#[test]
fn test_count_bargain_parses_correctly() {
    let mut svars = HashMap::new();
    svars.insert("X".to_string(), "Count$Bargain.3.2".to_string());

    let expr = CountExpression::parse("X", &svars);
    assert!(
        matches!(
            expr,
            CountExpression::Bargain {
                bargained_value: 3,
                unbargained_value: 2
            }
        ),
        "Count$Bargain.3.2 should parse to Bargain{{3, 2}}, got {:?}",
        expr
    );
}

/// A Bargain expression must NOT evaluate to 0.
/// Before the fix, it fell through to Fixed(0), making Torch the Tower deal 0 damage.
#[test]
fn test_count_bargain_does_not_evaluate_to_zero() {
    let mut svars = HashMap::new();
    svars.insert("X".to_string(), "Count$Bargain.3.2".to_string());

    let expr = CountExpression::parse("X", &svars);
    // The expression should NOT be Fixed(0) — that was the bug.
    assert!(
        !matches!(expr, CountExpression::Fixed(0)),
        "Count$Bargain.3.2 must not parse to Fixed(0) (pre-fix regression)"
    );
}

/// Count$Bargain.N.M is not Fixed, so effect_converter should route to DealDamageDynamic.
/// This test verifies the expression is treated as variable (non-fixed) so the
/// effect_converter intercept kicks in to emit DealDamageDynamic rather than DealDamageXPaid
/// (the XPaid path would silently resolve X=0 because bargain is not a paid-X cost).
#[test]
fn test_count_bargain_is_not_fixed() {
    let mut svars = HashMap::new();
    svars.insert("X".to_string(), "Count$Bargain.3.2".to_string());

    let expr = CountExpression::parse("X", &svars);
    assert!(!expr.is_fixed(), "Count$Bargain.3.2 must not be a fixed expression");
}

/// Verify symmetric parsing: swapped values are preserved.
#[test]
fn test_count_bargain_preserves_values() {
    let mut svars = HashMap::new();
    svars.insert("X".to_string(), "Count$Bargain.5.1".to_string());

    let expr = CountExpression::parse("X", &svars);
    assert!(
        matches!(
            expr,
            CountExpression::Bargain {
                bargained_value: 5,
                unbargained_value: 1
            }
        ),
        "Count$Bargain.5.1 should preserve bargained=5, unbargained=1, got {:?}",
        expr
    );
}
