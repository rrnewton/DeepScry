//! Shared combat-rules helpers.
//!
//! This module centralises the "can blocker B legally block attacker A?" check so
//! that the game-loop validation, AI heuristics, and the GUI/TUI choice menus
//! all agree on which blocker assignments are legal.
//!
//! Without this, the GUI would show illegal blocker options (e.g. a non-flying
//! creature blocking a flying attacker), the player would pick one, and
//! `validate_blocking_restrictions` would silently drop the assignment — making
//! it look like the engine ignored the player's input.
//!
//! # Rules implemented
//!
//! Per CR 509 (Declare Blockers Step) and CR 702 evasion abilities:
//!
//! - 509.1a: Blocker must be untapped
//! - 702.9b: Flying — only flying or reach can block
//! - 702.31: Horsemanship — only horsemanship can block
//! - 702.28: Shadow — shadow only blocks shadow; non-shadow only blocks non-shadow
//! - 702.36: Fear — only artifact creatures or black creatures can block
//! - 702.13: Intimidate — only artifact creatures or creatures sharing a color can block
//! - 702.119: Skulk — blocker must have greater power
//! - 702.16: Protection — blockers of a protected-from color cannot block
//! - CantBeBlocked persistent effects (e.g. from Deserter's Disciple)
//!
//! # NOT enforced here (handled at aggregate level)
//!
//! - **Menace (702.111b)** — requires 2+ blockers in total. Whether a *single*
//!   blocker may legally block a menace attacker depends on the rest of the
//!   block declaration, so it can only be enforced on the full assignment.

use crate::core::{CardId, Color, Keyword, KeywordArgs};
use crate::game::controller::GameStateView;
use crate::game::state::GameState;
use smallvec::SmallVec;

/// Returns `true` if `blocker_id` may legally be assigned to block `attacker_id`
/// in the current game state, ignoring only multi-blocker rules (Menace).
///
/// Landwalk (CR 702.14) IS enforced here using the full `GameState`; no
/// separate `can_block_with_view` call is needed for server-side validation.
///
/// Use this from the engine to validate blocker assignments and to build the
/// available-blockers list passed to controllers. Both call sites MUST use the
/// same predicate so the engine never silently drops a legal-looking choice.
pub fn can_block(game: &GameState, attacker_id: CardId, blocker_id: CardId) -> bool {
    can_block_impl(game, attacker_id, blocker_id, None)
}

/// Like [`can_block`], but uses a controller's `GameStateView` for the
/// Landwalk check instead of the full `GameState`.  Use this from controller
/// code (heuristic AI, TUI) where only the shadow/client view is available.
pub fn can_block_with_view(game: &GameState, view: &GameStateView, attacker_id: CardId, blocker_id: CardId) -> bool {
    can_block_impl(game, attacker_id, blocker_id, Some(view))
}

fn can_block_impl(game: &GameState, attacker_id: CardId, blocker_id: CardId, view: Option<&GameStateView>) -> bool {
    // Resolve cards. If either lookup fails, conservatively reject the block.
    let (Ok(attacker), Ok(blocker)) = (game.cards.get(attacker_id), game.cards.get(blocker_id)) else {
        return false;
    };

    // 509.1a: Tapped creatures can't block.
    if blocker.tapped {
        return false;
    }

    // CantBeBlocked persistent effects (e.g. Deserter's Disciple).
    if game.persistent_effects.is_creature_unblockable(attacker_id) {
        return false;
    }

    // 702.9b Flying: only flying or reach may block.
    if game.has_keyword_with_effects(attacker_id, Keyword::Flying)
        && !(game.has_keyword_with_effects(blocker_id, Keyword::Flying)
            || game.has_keyword_with_effects(blocker_id, Keyword::Reach))
    {
        return false;
    }

    // 702.31 Horsemanship: only horsemanship may block.
    if attacker.has_horsemanship() && !blocker.has_horsemanship() {
        return false;
    }

    // 702.28 Shadow: shadow only blocks shadow; non-shadow only blocks non-shadow.
    if attacker.has_shadow() != blocker.has_shadow() {
        return false;
    }

    // 702.36 Fear: only artifact creatures or black creatures may block.
    if attacker.has_fear() && !(blocker.is_artifact() || blocker.is_color(Color::Black)) {
        return false;
    }

    // 702.13 Intimidate: only artifact creatures or creatures sharing a color may block.
    if attacker.has_intimidate() {
        let shares_color = attacker.colors.iter().any(|c| blocker.is_color(*c));
        if !blocker.is_artifact() && !shares_color {
            return false;
        }
    }

    // 702.119 Skulk: blocker must have greater power.
    if attacker.has_skulk() && blocker.current_power() <= attacker.current_power() {
        return false;
    }

    // 702.16 Protection from color: blockers of a protected-from color can't block.
    for color in &blocker.colors {
        if attacker.has_protection_from(*color) {
            return false;
        }
    }

    // CR 509.1b / 509.4: per-creature block restriction
    // (`S:Mode$ CantBlockBy | ValidAttacker$ <filter> | ValidBlocker$ Creature.Self`).
    // Ironclaw Orcs ("can't block creatures with power 2 or greater"): the
    // blocker may not be declared against any attacker matching the filter.
    for static_ability in &blocker.static_abilities {
        if let crate::core::StaticAbility::CantBlockMatching { attacker_filter, .. } = static_ability {
            if attacker_filter.matches(attacker) {
                return false;
            }
        }
    }

    // Global block prohibition: some battlefield permanent (e.g. Light of Day)
    // carries a CantAttackOrBlockMatching static that forbids certain creature
    // types from blocking entirely (CR 509.1b).
    if game.is_block_prohibited(blocker) {
        return false;
    }

    // 702.14 Landwalk: unblockable if defending player controls the named land type.
    // Check using either the controller's view (controller code) or the full game
    // state (server-side engine validation) — both paths must produce the same
    // verdict to avoid engine/controller desync.
    if attacker.has_keyword(Keyword::Landwalk) {
        for keyword_args in attacker.keywords.iter_args() {
            if let KeywordArgs::Landwalk { land_type } = keyword_args {
                let defender_has_land = if let Some(view) = view {
                    view.battlefield().iter().filter_map(|&id| view.get_card(id)).any(|c| {
                        c.controller == blocker.controller
                            && c.is_land()
                            && c.subtypes
                                .iter()
                                .any(|st| st.as_str().eq_ignore_ascii_case(land_type.as_str()))
                    })
                } else {
                    game.battlefield
                        .cards
                        .iter()
                        .filter_map(|&id| game.cards.get(id).ok())
                        .any(|c| {
                            c.controller == blocker.controller
                                && c.is_land()
                                && c.subtypes
                                    .iter()
                                    .any(|st| st.as_str().eq_ignore_ascii_case(land_type.as_str()))
                        })
                };
                if defender_has_land {
                    return false;
                }
            }
        }
    }

    true
}

/// Returns `true` if `blocker_id` can legally block at least one of the
/// supplied `attackers`. Used by the engine to filter the
/// "available blockers" list before passing it to a controller, so the
/// UI never offers a creature that has no legal target at all (e.g. a
/// non-flying, non-reach blocker when every attacker has Flying).
///
/// Note: this is a *necessary* condition for being a useful blocker, not
/// a sufficient one — Menace (which needs 2+ blockers per attacker) is
/// still validated downstream by `validate_blocking_restrictions`.
pub fn is_useful_blocker(game: &GameState, blocker_id: CardId, attackers: &[CardId]) -> bool {
    attackers
        .iter()
        .any(|&attacker_id| can_block(game, attacker_id, blocker_id))
}

/// Returns the subset of `attackers` that `blocker_id` may legally block.
/// Used by interactive controllers to build per-blocker menus that show
/// only the attackers a given creature can actually block, mirroring the
/// engine's `validate_blocking_restrictions` so the engine never silently
/// drops a choice the UI offered.
pub fn legal_attackers_for_blocker(
    game: &GameState,
    blocker_id: CardId,
    attackers: &[CardId],
) -> SmallVec<[CardId; 8]> {
    attackers
        .iter()
        .copied()
        .filter(|&attacker_id| can_block(game, attacker_id, blocker_id))
        .collect()
}
