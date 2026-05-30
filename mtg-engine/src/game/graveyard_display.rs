//! Backend-neutral "best-effort graveyard listing" helper.
//!
//! The graveyard is displayed at the **bottom of the Hand pane** in every
//! UI (native ratatui TUI, WASM card GUI). The Hand is the critical thing
//! to show in full; the graveyard is best-effort. When there is not enough
//! vertical room to list every graveyard card, we show the **most recent**
//! `K` additions (the tail of the zone, since cards are appended to the
//! graveyard as they die / are discarded) followed by a single
//! `… N more cards` ellision line.
//!
//! This single helper centralizes the top-K + ellision arithmetic so the
//! TUI (Rust) and the WASM GUI (Rust → JSON → JS) agree exactly. The JS
//! renderer in `web/native_game.html` mirrors the same arithmetic in one
//! small function (`graveyardDisplayPlan`) — keep the two in sync; this is
//! the canonical reference.

/// The fixed header line ("Graveyard (N):") always counts against the line
/// budget, so a budget below this leaves no room for any card rows.
pub const GRAVEYARD_HEADER_LINES: usize = 1;

/// A plan for rendering a graveyard listing within a bounded number of
/// lines. Produced by [`plan_graveyard_display`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraveyardDisplayPlan {
    /// Total number of cards in the graveyard.
    pub total: usize,
    /// Number of cards that fit and will be listed (the most recent ones).
    pub shown: usize,
    /// Number of cards elided (`total - shown`). Zero ⇒ no ellision line.
    pub elided: usize,
}

impl GraveyardDisplayPlan {
    /// True if an `… N more cards` ellision line should be rendered.
    pub fn has_ellision(&self) -> bool {
        self.elided > 0
    }

    /// The total number of lines this plan occupies, including the header
    /// and the optional ellision line.
    pub fn line_count(&self) -> usize {
        GRAVEYARD_HEADER_LINES + self.shown + usize::from(self.has_ellision())
    }

    /// The ellision line text (`… N more cards`), or `None` if nothing is
    /// elided. Singular/plural is handled for the 1-card case.
    pub fn ellision_line(&self) -> Option<String> {
        if self.elided == 0 {
            None
        } else if self.elided == 1 {
            Some("… 1 more card".to_string())
        } else {
            Some(format!("… {} more cards", self.elided))
        }
    }
}

/// Given the total graveyard size and the number of lines available for the
/// listing (header + card rows + optional ellision line), compute how many
/// of the **most recent** cards to show and how many to elide.
///
/// `line_budget` is the total vertical room (in lines) the graveyard
/// listing may occupy at the bottom of the Hand pane. The header
/// (`Graveyard (N):`) always consumes one line. If even the header does
/// not fit (`line_budget == 0`), the returned plan shows nothing.
///
/// Behavior:
/// - If every card fits, `shown == total`, `elided == 0` (no ellision).
/// - Otherwise we reserve one line for the `… N more cards` ellision line
///   and fill the rest with the most-recent cards, so `shown + 1` card-area
///   lines (plus the header) fit within the budget.
pub fn plan_graveyard_display(total: usize, line_budget: usize) -> GraveyardDisplayPlan {
    // No room even for the header.
    if line_budget < GRAVEYARD_HEADER_LINES || total == 0 {
        return GraveyardDisplayPlan {
            total,
            shown: 0,
            elided: total,
        };
    }

    // Lines available for card rows after the header.
    let card_lines = line_budget - GRAVEYARD_HEADER_LINES;

    if total <= card_lines {
        // Everything fits — no ellision.
        return GraveyardDisplayPlan {
            total,
            shown: total,
            elided: 0,
        };
    }

    // Need to elide: reserve one card-row line for the ellision line.
    // `card_lines` is >= 1 here because `total > card_lines >= 0` and
    // `total >= 1`; if `card_lines == 0` we show 0 cards and elide all.
    let shown = card_lines.saturating_sub(1);
    GraveyardDisplayPlan {
        total,
        shown,
        elided: total - shown,
    }
}

/// Convenience: split a graveyard slice (ordered oldest → newest, as stored
/// in the zone) into the most-recent `plan.shown` entries to display (in
/// oldest-of-the-visible → newest order) plus the plan. Returns the tail
/// slice so callers avoid cloning.
pub fn visible_recent<'a, T>(cards: &'a [T], plan: &GraveyardDisplayPlan) -> &'a [T] {
    let start = cards.len().saturating_sub(plan.shown);
    &cards[start..]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everything_fits() {
        let p = plan_graveyard_display(3, 10);
        assert_eq!(p.shown, 3);
        assert_eq!(p.elided, 0);
        assert!(!p.has_ellision());
        assert_eq!(p.line_count(), 1 + 3);
        assert_eq!(p.ellision_line(), None);
    }

    #[test]
    fn exact_fit_no_ellision() {
        // budget = header(1) + 4 cards = 5, total = 4 ⇒ all shown.
        let p = plan_graveyard_display(4, 5);
        assert_eq!(p.shown, 4);
        assert_eq!(p.elided, 0);
    }

    #[test]
    fn ellision_reserves_a_line() {
        // budget = header(1) + 4 lines, total = 10.
        // 4 card lines, reserve 1 for ellision ⇒ show 3, elide 7.
        let p = plan_graveyard_display(10, 5);
        assert_eq!(p.shown, 3);
        assert_eq!(p.elided, 7);
        assert!(p.has_ellision());
        assert_eq!(p.line_count(), 1 + 3 + 1);
        assert_eq!(p.ellision_line().as_deref(), Some("… 7 more cards"));
    }

    #[test]
    fn singular_more_card() {
        // total 5, budget header+5? No: budget header(1)+4 card lines, total 5.
        // 4 card lines, total 5 > 4 ⇒ reserve 1 ⇒ show 3, elide 2. Tweak to
        // get exactly 1 elided: total 5, budget = header + 5 lines = 6 ⇒ fits.
        // For 1 elided: card_lines such that total - (card_lines-1) == 1
        // ⇒ card_lines == total ⇒ budget = total + header but total>card_lines.
        // Simplest: total 2, budget header+1 ⇒ card_lines 1, reserve 1, show 0,
        // elide 2 — not 1. Use total 2, but we need show=1 elide=1:
        // card_lines=2 means total<=card_lines (fits). So 1-elided only arises
        // when card_lines-1 == total-1, i.e. exactly one over: total 2,
        // card_lines 1? show=0. Hmm — 1 elided happens at total=K+1 with
        // card_lines=K+0? Let's just assert plural path is correct and that
        // ellision_line handles the literal 1 case directly:
        let p = GraveyardDisplayPlan {
            total: 4,
            shown: 3,
            elided: 1,
        };
        assert_eq!(p.ellision_line().as_deref(), Some("… 1 more card"));
    }

    #[test]
    fn zero_budget_shows_nothing() {
        let p = plan_graveyard_display(5, 0);
        assert_eq!(p.shown, 0);
        assert_eq!(p.elided, 5);
    }

    #[test]
    fn empty_graveyard() {
        let p = plan_graveyard_display(0, 10);
        assert_eq!(p.shown, 0);
        assert_eq!(p.elided, 0);
        assert!(!p.has_ellision());
    }

    #[test]
    fn visible_recent_takes_tail() {
        let cards = [1, 2, 3, 4, 5];
        let p = plan_graveyard_display(5, 4); // header+3 card lines, total 5 > 3 ⇒ show 2, elide 3
        assert_eq!(p.shown, 2);
        assert_eq!(visible_recent(&cards, &p), &[4, 5]);
    }
}
