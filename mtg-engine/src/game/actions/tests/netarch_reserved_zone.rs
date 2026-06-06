//! mtg-728 CLASS-A / mtg-725 R1: reserved-id zone-COUNT lockstep.
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

use crate::core::effects::{Effect, TargetRestriction};
use crate::core::{Card, CardId, PlayerId};
use crate::game::GameState;
use crate::zones::Zone;

fn add_to_zone(game: &mut GameState, player: PlayerId, zone: Zone, id: CardId) {
    let z = game.get_player_zones_mut(player).expect("zones");
    match zone {
        Zone::Hand => z.hand.add(id),
        Zone::Library => z.library.add(id),
        Zone::Graveyard => z.graveyard.add(id),
        Zone::Battlefield | Zone::Exile | Zone::Stack | Zone::Command => {
            panic!("add_to_zone unsupported zone {zone:?}")
        }
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
        "mtg-728 R1: shadow must count the {N} reserved (instance-less) opponent \
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

// ===========================================================================
// mtg-728 CLASS-A seed-2 (TIMETWISTER): full mass-shuffle + draw lockstep.
//
// Timetwister (card 55) resolves as
//   `Effect::ChangeZoneAll { origins: [Hand, Graveyard], destination: Library,
//    shuffle: true }`  followed by each player drawing 7.
//
// On a SHADOW game the OPPONENT's hidden Hand + Library cards are reserved
// (instance-less) CardIds. For byte-for-byte server<->shadow lockstep the shadow
// resolution MUST, for EVERY player including the opponent:
//   (a) move the reserved Hand + Graveyard cards into the Library so the library
//       COUNT matches the server (no branch-on-absence dropping reserved ids),
//   (b) consume the SAME shuffle RNG (count-based; depends on (a) being exact),
//   (c) draw the SAME number of cards, leaving the SAME residual library counts.
//
// If any reserved card is silently skipped, the shadow library is short, the
// shuffle advances the ChaCha12 RNG by a different amount, and every later
// shuffle / draw on the shadow desyncs from the server (mtg-725 anti-pattern).
//
// GOLDEN = the SERVER's view: every card in every zone is a real instance.
// SHADOW = a viewer's client: P0 (the viewer) is fully real, P1 (the opponent)
// has its Hand + Graveyard + Library as reserved instance-less ids.
// ===========================================================================

/// Per-player zone population used to build both the golden and shadow games
/// identically (same ids, same counts) so any divergence is purely the
/// shadow's reserved-id handling.
struct Setup {
    hand: u32,
    graveyard: u32,
    library: u32,
}

const VIEWER_BASE: u32 = 4000;
const OPP_BASE: u32 = 5000;

fn populate_player(game: &mut GameState, player: PlayerId, base: u32, s: &Setup, real: bool) {
    let mut next = base;
    let add = |game: &mut GameState, zone: Zone, count: u32, next: &mut u32| {
        for _ in 0..count {
            let id = CardId::new(*next);
            *next += 1;
            if real {
                let card = Card::new(id, "Forest".to_string(), player);
                game.cards.insert(id, card);
            }
            add_to_zone(game, player, zone, id);
        }
    };
    add(game, Zone::Hand, s.hand, &mut next);
    add(game, Zone::Graveyard, s.graveyard, &mut next);
    add(game, Zone::Library, s.library, &mut next);
}

/// Build a two-player game. `opp_reserved` controls whether the opponent (P1)
/// gets real instances (golden / server) or reserved instance-less ids (shadow).
fn build_timetwister_game(viewer: &Setup, opponent: &Setup, opp_reserved: bool) -> (GameState, PlayerId, PlayerId) {
    let mut game = GameState::new_two_player("P0".to_string(), "P1".to_string(), 20);
    if opp_reserved {
        game.set_shadow_game(true);
    }
    let p0 = game.players.first().unwrap().id;
    let p1 = game.players.get(1).unwrap().id;
    // Viewer (P0) is always real — even on the shadow the viewer sees its own
    // cards as concrete instances.
    populate_player(&mut game, p0, VIEWER_BASE, viewer, true);
    // Opponent (P1) is real on the golden, reserved on the shadow.
    populate_player(&mut game, p1, OPP_BASE, opponent, !opp_reserved);
    (game, p0, p1)
}

fn library_len(game: &GameState, player: PlayerId) -> usize {
    game.get_player_zones(player)
        .map(|z| z.library.cards.len())
        .unwrap_or(0)
}

/// Execute the Timetwister effect + draw 7 each, returning the post-resolution
/// observables that MUST match server<->shadow:
///   (per-player library len after move, viewer's drawn ids, rng state after).
fn resolve_timetwister(
    game: &mut GameState,
    p0: PlayerId,
    p1: PlayerId,
) -> (usize, usize, Vec<CardId>, Option<smallvec::SmallVec<[u8; 64]>>) {
    let effect = Effect::ChangeZoneAll {
        restriction: TargetRestriction::any(),
        origins: smallvec::smallvec![Zone::Hand, Zone::Graveyard],
        destination: Zone::Library,
        shuffle: true,
    };
    game.execute_effect(&effect).expect("ChangeZoneAll resolves");

    let lib0_after_move = library_len(game, p0);
    let lib1_after_move = library_len(game, p1);

    // Each player draws 7 (Timetwister's tail). The viewer's draws are real
    // ids whose identity must match the server byte-for-byte.
    let mut viewer_drawn = Vec::new();
    for _ in 0..7 {
        if let (Some(c), _) = game.draw_card(p0).expect("p0 draw") {
            viewer_drawn.push(c);
        }
    }
    for _ in 0..7 {
        let _ = game.draw_card(p1).expect("p1 draw");
    }

    let _ = (lib0_after_move, lib1_after_move);
    let rng_after = game.capture_rng_state();
    (lib0_after_move, lib1_after_move, viewer_drawn, rng_after)
}

/// The lockstep oracle: golden (server) and shadow (client) resolve the SAME
/// Timetwister and MUST agree on every server-observable lockstep quantity.
/// RED before the reserved-id mass-move fix; GREEN after.
#[test]
fn shadow_timetwister_mass_shuffle_draw_matches_golden_mb668_seed2() {
    // Mirror seed-2's pre-Timetwister shape (P0 lib 58 / P1 lib 59 after move,
    // then -7 each from the draws). Exact numbers don't matter for lockstep —
    // only that golden and shadow agree.
    let viewer = Setup {
        hand: 5,
        graveyard: 5,
        library: 48,
    };
    let opponent = Setup {
        hand: 6,
        graveyard: 5,
        library: 48,
    };

    let (mut golden, gp0, gp1) = build_timetwister_game(&viewer, &opponent, false);
    let (g_lib0, g_lib1, g_drawn, g_rng) = resolve_timetwister(&mut golden, gp0, gp1);

    let (mut shadow, sp0, sp1) = build_timetwister_game(&viewer, &opponent, true);
    let (s_lib0, s_lib1, s_drawn, s_rng) = resolve_timetwister(&mut shadow, sp0, sp1);

    assert_eq!(
        s_lib0, g_lib0,
        "mtg-728 seed-2: VIEWER library count after Timetwister mass-move must \
         match the server (viewer is real on both, sanity check)"
    );
    assert_eq!(
        s_lib1, g_lib1,
        "mtg-728 seed-2: OPPONENT library count after Timetwister mass-move must \
         match the server — the reserved opponent Hand+Graveyard cards MUST move \
         into the library; branch-on-absence (try_get=None) must NOT drop them"
    );
    assert_eq!(
        s_drawn, g_drawn,
        "mtg-728 seed-2: the VIEWER's drawn cards must byte-match the server — a \
         short opponent library desyncs the shuffle RNG and changes the draw order"
    );
    assert_eq!(
        s_rng, g_rng,
        "mtg-728 seed-2: the RNG state after the mass shuffle+draw must match the \
         server byte-for-byte (count-based shuffle consumes identical randomness \
         only if the reserved opponent cards all moved)"
    );
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
