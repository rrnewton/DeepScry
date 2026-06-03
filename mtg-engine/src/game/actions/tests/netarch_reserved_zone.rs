//! mtg-mb668 CLASS-A / mtg-725 R1: reserved-id zone-COUNT lockstep.
//!
//! `count_cards_matching_filter` filters a zone's cards with
//! `self.cards.try_get(cid)` and returns `false` on `None`. On a SHADOW game the
//! opponent's Hand/Library cards are reserved (instance-less) IDs, so they are
//! EXCLUDED from the count — while the server (which holds the real instances)
//! COUNTS them. Any effect whose magnitude depends on a hidden-zone SIZE
//! (intervening-if "if you have N+ cards in hand", count-based X / cost / damage)
//! therefore diverges server↔shadow: the mtg-725 branch-on-absence anti-pattern.
//!
//! The fix mirrors the sig-2c/2d template (state.rs:1214 `maybe_conceal_in_library`):
//! handle the reserved opponent id SYMMETRICALLY — for a WILDCARD type filter
//! (`""`/`"Card"`/`"Permanent"`) whose ownership/control qualifier is satisfied by
//! the scanned zone's owner (`YouOwn`/`YouCtrl`, since a reserved card in
//! `player_id`'s hidden zone is owned+controlled by `player_id`), COUNT it by id
//! without requiring an instance. Typed/colored filters and opponent-relative
//! (`OppOwn`/`OppCtrl`) qualifiers over a reserved id remain unevaluable without
//! the instance and are out of scope (a hidden zone cannot be conditioned on by
//! type without leaking hidden info — see the audit's SAFE-BY-CONTRACT notes).
//!
//! RED-first: before the fix `shadow_count == 0` while `golden_count == n`.

use crate::core::{Card, CardId, PlayerId};
use crate::game::GameState;
use crate::zones::Zone;

fn add_to_zone(game: &mut GameState, player: PlayerId, zone: Zone, id: CardId) {
    let z = game.get_player_zones_mut(player).expect("zones");
    match zone {
        Zone::Hand => z.hand.add(id),
        Zone::Library => z.library.add(id),
        Zone::Graveyard => z.graveyard.add(id),
        other => panic!("add_to_zone unsupported zone {other:?}"),
    }
}

/// GOLDEN: `n` REAL instances owned by P0 in `zone`. The server's view.
fn build_golden(n: u32, zone: Zone) -> (GameState, PlayerId) {
    let mut game = GameState::new_two_player("P0".to_string(), "P1".to_string(), 20);
    let p0 = game.players.first().unwrap().id;
    for i in 0..n {
        let id = CardId::new(4000 + i);
        let card = Card::new(id, "Forest".to_string(), p0);
        game.cards.insert(id, card);
        add_to_zone(&mut game, p0, zone, id);
    }
    (game, p0)
}

/// SHADOW: the SAME `n` ids in `zone`, but RESERVED (no `Card` instances) and
/// `is_shadow_game` — the late-binding representation of an opponent's hidden
/// zone on a viewer's shadow.
fn build_shadow(n: u32, zone: Zone) -> (GameState, PlayerId) {
    let mut game = GameState::new_two_player("P0".to_string(), "P1".to_string(), 20);
    game.set_shadow_game(true);
    let p0 = game.players.first().unwrap().id;
    for i in 0..n {
        let id = CardId::new(4000 + i);
        add_to_zone(&mut game, p0, zone, id);
    }
    (game, p0)
}

fn assert_shadow_matches_golden(zone: Zone) {
    const N: u32 = 5;
    let (golden, gp) = build_golden(N, zone);
    let golden_count = golden.count_cards_matching_filter(gp, "Card", zone);
    assert_eq!(
        golden_count, N as usize,
        "golden must count its {N} real {zone:?} cards"
    );

    let (shadow, sp) = build_shadow(N, zone);
    let shadow_count = shadow.count_cards_matching_filter(sp, "Card", zone);
    assert_eq!(
        shadow_count, golden_count,
        "mtg-mb668 R1: shadow must count the {N} reserved (instance-less) opponent \
         {zone:?} cards identically to the server — branch-on-absence (try_get=None) \
         must NOT silently drop them"
    );
}

#[test]
fn shadow_counts_reserved_opponent_hand_matches_golden_mb668_r1() {
    assert_shadow_matches_golden(Zone::Hand);
}

#[test]
fn shadow_counts_reserved_opponent_library_matches_golden_mb668_r1() {
    assert_shadow_matches_golden(Zone::Library);
}

/// Guard the fix's SCOPE: a reserved card must count ONLY for a wildcard filter.
/// A TYPED filter over a hidden zone is unevaluable without the instance (the
/// shadow cannot know a reserved card's type), so it must NOT be counted — that
/// would over-count. On the GOLDEN side the typed filter is evaluated normally.
#[test]
fn shadow_does_not_count_reserved_under_typed_or_opp_filter_mb668_r1_scope() {
    const N: u32 = 4;
    let (shadow, sp) = build_shadow(N, Zone::Hand);
    // The wildcard / zone-owner-relative filter DOES count the reserved cards
    // (the fix); a TYPED filter and an OPPONENT-relative ownership qualifier are
    // unevaluable without the instance, so they must stay 0 (no over-count).
    assert_eq!(
        shadow.count_cards_matching_filter(sp, "Card", Zone::Hand),
        N as usize,
        "wildcard 'Card' counts the reserved zone-owner cards (the fix)"
    );
    assert_eq!(
        shadow.count_cards_matching_filter(sp, "Creature", Zone::Hand),
        0,
        "a TYPED filter over reserved (type-unknown) shadow cards must count 0"
    );
    assert_eq!(
        shadow.count_cards_matching_filter(sp, "Card.OppOwn", Zone::Hand),
        0,
        "an OPPONENT-relative qualifier over the zone-owner's reserved cards must \
         count 0 — a reserved card in player_id's hidden zone is player_id-owned"
    );
}
