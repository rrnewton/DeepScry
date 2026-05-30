//! Damage-prevention replacement effects (CR 615 "prevent").
//!
//! This module models *source-filtered* damage prevention shields — a general
//! rules construct (CR 615.1, 615.6) distinct from the simple amount-only
//! `damage_prevention` counter on [`crate::core::Player`] /
//! [`crate::core::Card`]. A [`DamagePreventionShield`] is a continuous,
//! until-end-of-turn replacement effect: when damage that would be dealt to the
//! protected player comes from a *matching source*, that damage is prevented
//! (reduced to 0) and logged as prevented rather than reducing life
//! (CR 615.6).
//!
//! The canonical first instance is **Circle of Protection: Red** — "{1}: The
//! next time a red source of your choice would deal damage to you this turn,
//! prevent that damage." The construct is deliberately color- and
//! source-agnostic so the remaining Circles of Protection (White/Blue/Black/
//! Green) and similar "prevent all/next damage from a source" cards reuse it
//! with only a different [`DamageSourceFilter`].
//!
//! ## Determinism / information-independence
//!
//! A shield is *public* game state stored on the protected player. The source
//! filter resolves against public characteristics (a card's colors, its
//! identity) only — never hidden information — so the same prevention decision
//! is reached identically on the server and on every client's shadow state
//! (see `docs/NETWORK_ARCHITECTURE.md`).

use crate::core::{CardId, Color};
use serde::{Deserialize, Serialize};

/// Which damage *sources* a prevention shield applies to (CR 609.7, the
/// "source of a damage event").
///
/// Resolved against public card characteristics only (colors / identity),
/// keeping prevention network- and WASM-deterministic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DamageSourceFilter {
    /// Any source of the given color (e.g. a hypothetical "prevent all damage
    /// from red sources"). Matches if the source card *is* that color.
    Color(Color),

    /// One specific chosen source, regardless of its colors. The classic
    /// "prevent all damage that target permanent would deal to you".
    SpecificSource(CardId),

    /// A *specific chosen source that is also of the given color* — the
    /// Circle-of-Protection shape (`Card.ChosenCardStrict+RedSource`). The
    /// color is retained for clarity/diagnostics; matching keys off the
    /// chosen source's identity (it was required to be that color when
    /// chosen).
    ColoredSource { color: Color, source: CardId },
}

impl DamageSourceFilter {
    /// Does this filter match a damage source with the given identity/colors?
    ///
    /// `source_is_color(color)` reports whether the source card currently has
    /// that color; passing a closure keeps this type free of any dependency on
    /// the full card store and trivially testable.
    pub fn matches(&self, source: CardId, source_is_color: impl Fn(Color) -> bool) -> bool {
        match self {
            DamageSourceFilter::Color(color) => source_is_color(*color),
            DamageSourceFilter::SpecificSource(chosen) => *chosen == source,
            DamageSourceFilter::ColoredSource { source: chosen, .. } => *chosen == source,
        }
    }
}

/// How much damage a shield prevents before it expires.
///
/// CR 615.1 ("prevent the next N damage") vs the Circle-of-Protection /
/// old-school "prevent *all* damage … this turn" wording. Both expire in the
/// cleanup step (CR 514.2) regardless.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PreventionScope {
    /// Prevent *all* matching damage for the rest of the turn (e.g. a
    /// hypothetical "prevent all damage that red sources would deal to you this
    /// turn"). Never consumed; expires only at cleanup.
    AllThisTurn,

    /// Prevent *all of the next single matching damage event* (of any
    /// magnitude), then expire. This is the Circle-of-Protection wording —
    /// "the next time the chosen source would deal damage to you this turn,
    /// prevent that damage" (CR 615.1). A 4-power red attacker's whole 4-damage
    /// hit is one event and is fully prevented, after which the shield is
    /// spent.
    NextEvent,

    /// Prevent the next `amount` *points* of matching damage, then expire
    /// (CR 615.1 "prevent the next N damage", amount-based — Pentagram of the
    /// Ages-style shields keyed to a source).
    NextPoints(u32),
}

/// A continuous, until-end-of-turn damage-prevention replacement effect
/// attached to the protected player (CR 615.1, 615.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DamagePreventionShield {
    /// Which damage sources this shield prevents.
    pub source: DamageSourceFilter,

    /// How much / how long the shield prevents.
    pub scope: PreventionScope,
}

impl DamagePreventionShield {
    /// Build the Circle-of-Protection shield: prevent the next damage event
    /// from the chosen colored source to the controller this turn, then expire
    /// ("the next time the chosen source would deal damage to you this turn,
    /// prevent that damage").
    pub fn colored_source_next_event(color: Color, source: CardId) -> Self {
        DamagePreventionShield {
            source: DamageSourceFilter::ColoredSource { color, source },
            scope: PreventionScope::NextEvent,
        }
    }

    /// Apply this shield to a would-be damage event from `source` of `amount`.
    ///
    /// Returns the number of points actually prevented (CR 615.6). The
    /// shield's [`PreventionScope::Next`] counter is decremented in place when
    /// it absorbs damage; `AllThisTurn` shields are never consumed (they last
    /// until cleanup). A return of `0` means this shield does not apply.
    pub fn apply(&mut self, source: CardId, amount: u32, source_is_color: impl Fn(Color) -> bool) -> u32 {
        if amount == 0 || !self.source.matches(source, source_is_color) {
            return 0;
        }
        match &mut self.scope {
            PreventionScope::AllThisTurn => amount,
            PreventionScope::NextEvent => {
                // Prevent the entire event, then mark the shield spent so the
                // next matching event is no longer prevented.
                self.scope = PreventionScope::NextPoints(0);
                amount
            }
            PreventionScope::NextPoints(remaining) => {
                let prevented = amount.min(*remaining);
                *remaining -= prevented;
                prevented
            }
        }
    }

    /// Has this shield been fully consumed (a spent `NextPoints(0)` shield)?
    /// Spent shields can be dropped eagerly; all shields are cleared at end of
    /// turn anyway.
    pub fn is_spent(&self) -> bool {
        matches!(self.scope, PreventionScope::NextPoints(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::CardId;

    fn red(_c: Color) -> bool {
        false
    }

    #[test]
    fn colored_source_prevents_next_event_then_expires() {
        let chosen = CardId::new(7);
        let other = CardId::new(8);
        let mut shield = DamagePreventionShield::colored_source_next_event(Color::Red, chosen);
        // Different source: nothing prevented, shield not consumed.
        assert_eq!(shield.apply(other, 5, |_| true), 0);
        assert!(!shield.is_spent());
        // Matching chosen source: the entire event is prevented (any magnitude).
        assert_eq!(shield.apply(chosen, 100, |_| true), 100);
        // Shield is now spent: the next event is no longer prevented.
        assert!(shield.is_spent());
        assert_eq!(shield.apply(chosen, 3, |_| true), 0);
    }

    #[test]
    fn color_filter_uses_source_color_predicate() {
        let src = CardId::new(1);
        let mut shield = DamagePreventionShield {
            source: DamageSourceFilter::Color(Color::Red),
            scope: PreventionScope::AllThisTurn,
        };
        assert_eq!(shield.apply(src, 4, |c| c == Color::Red), 4);
        assert_eq!(shield.apply(src, 4, red), 0);
    }

    #[test]
    fn next_n_scope_decrements_and_spends() {
        let src = CardId::new(2);
        let mut shield = DamagePreventionShield {
            source: DamageSourceFilter::SpecificSource(src),
            scope: PreventionScope::NextPoints(2),
        };
        assert_eq!(shield.apply(src, 5, |_| true), 2);
        assert!(shield.is_spent());
        assert_eq!(shield.apply(src, 5, |_| true), 0);
    }
}
