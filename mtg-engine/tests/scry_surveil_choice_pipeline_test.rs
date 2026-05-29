#![cfg(feature = "network")]
// Wildcard `other =>` arms in these tests intentionally accept "any future
// variant" so they panic with a helpful message instead of failing to compile
// when a new ChoiceType / ChoiceResult variant is added. Enumerating every
// variant inline would defeat that purpose. Similarly, `revealed.clone()` is
// used for assertion readability after the value is moved into a helper —
// the clone is cheap (small Vec<CardId>) and removing it forces awkward
// argument ordering. Pre-existing on `integration` from the scry/surveil
// merge; allow at module scope so `cargo clippy --all-features --tests
// -- -D warnings` (CI) stays green.
#![allow(clippy::wildcard_enum_match_arm, clippy::redundant_clone)]
//! Phase E regression tests for the scry / surveil choice pipeline.
//!
//! These tests pin down behaviours that should NOT regress to the
//! pre-Phase-B engine-baked heuristic pattern. Each test corresponds to
//! one of the Phase E acceptance criteria from the tracking issue.
//!
//! Coverage matrix
//! ===============
//!
//! - `random_controller_*` — RandomController makes a random but
//!   deterministic-given-seed scry/surveil decision.
//! - `surveil_moves_to_graveyard` — surveil's apply step actually puts
//!   the chosen cards in the graveyard zone (post-Phase-B regression
//!   risk: a previous draft routed only through `library.remove`).
//! - `protocol_scry_request_*` — the network protocol's `ChoiceType::Scry`
//!   /`Surveil` carry the revealed CardIds inline so the SCRYING player's
//!   client can render the choice without a separate CardRevealed
//!   round-trip; the response shape is "indices into revealed".
//! - `network_controller_*` — NetworkController's choose_scry_order /
//!   choose_surveil emit a ChoiceRequest with `ChoiceType::Scry` /
//!   `Surveil`, embed the revealed CardIds, and parse a response of
//!   bottom/graveyard indices into a valid ScryDecision/SurveilDecision.
//! - `information_hiding_*` — the protocol embeds revealed CardIds in
//!   the ChoiceRequest addressed to the scrying player; CR 701.18 says
//!   the opponent does not see them, and our protocol upholds that by
//!   construction (only the scrying player's NetworkController calls
//!   `request_choice` with ChoiceType::Scry).

use mtg_engine::core::{Card, CardId, CardType, PlayerId};
use mtg_engine::game::{
    controller::ChoiceResult, GameState, GameStateView, PlayerController, RandomController, ScryDecision,
    SurveilDecision,
};
use mtg_engine::network::{ChoiceRequest, ChoiceResponse, ChoiceType, NetworkController};
use smallvec::SmallVec;
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc;
use std::sync::Arc;

// ============================================================================
// Common test helpers (parallel to scry_surveil_parity_test.rs but kept
// independent so each test file is self-contained / parallelisable).
// ============================================================================

fn build_game_with_library(library_cards: &[(&str, &[CardType])]) -> GameState {
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1 = game.players[0].id;
    for (i, (name, types)) in library_cards.iter().enumerate().rev() {
        let id = CardId::new(2000 + i as u32);
        let mut c = Card::new(id, *name, p1);
        c.set_types(SmallVec::from_vec(types.to_vec()));
        game.cards.insert(id, c);
        if let Some(zones) = game.get_player_zones_mut(p1) {
            zones.library.cards.push(id);
        }
    }
    game
}

fn unwrap_scry(result: ChoiceResult<ScryDecision>) -> ScryDecision {
    match result {
        ChoiceResult::Ok(d) => d,
        ChoiceResult::UndoRequest(_) | ChoiceResult::ExitGame | ChoiceResult::Error(_) | ChoiceResult::NeedInput(_) => {
            panic!("expected Ok ScryDecision, got non-Ok variant in test")
        }
    }
}

fn unwrap_surveil(result: ChoiceResult<SurveilDecision>) -> SurveilDecision {
    match result {
        ChoiceResult::Ok(d) => d,
        ChoiceResult::UndoRequest(_) | ChoiceResult::ExitGame | ChoiceResult::Error(_) | ChoiceResult::NeedInput(_) => {
            panic!("expected Ok SurveilDecision, got non-Ok variant in test")
        }
    }
}

// ============================================================================
// RandomController determinism (Phase E #3)
// ============================================================================

#[test]
fn random_controller_scry_is_deterministic_given_seed() {
    // Same seed → same partition. This is the foundational invariant
    // for snapshot/replay determinism with RandomController.
    let library = vec![
        ("A", &[CardType::Instant][..]),
        ("B", &[CardType::Land][..]),
        ("C", &[CardType::Creature][..]),
        ("D", &[CardType::Sorcery][..]),
    ];

    let game1 = build_game_with_library(&library);
    let game2 = build_game_with_library(&library);
    let p1 = game1.players[0].id;

    let mut rc1 = RandomController::with_seed(p1, 12345);
    let mut rc2 = RandomController::with_seed(p1, 12345);

    let revealed = game1.scry_snapshot_top_n(p1, 4);
    assert_eq!(revealed.len(), 4);

    let view1 = GameStateView::new(&game1, p1);
    let view2 = GameStateView::new(&game2, p1);

    let d1 = unwrap_scry(rc1.choose_scry_order(&view1, &revealed));
    let d2 = unwrap_scry(rc2.choose_scry_order(&view2, &revealed));

    assert_eq!(
        d1.top.as_slice(),
        d2.top.as_slice(),
        "top piles must match for same seed"
    );
    assert_eq!(
        d1.bottom.as_slice(),
        d2.bottom.as_slice(),
        "bottom piles must match for same seed"
    );

    // Sanity: every revealed card lands in exactly one pile.
    let mut all = d1.top.iter().chain(d1.bottom.iter()).copied().collect::<Vec<_>>();
    all.sort_by_key(|c| c.as_u32());
    let mut expected: Vec<CardId> = revealed.iter().copied().collect();
    expected.sort_by_key(|c| c.as_u32());
    assert_eq!(all, expected, "decision must be a partition of revealed");
}

#[test]
fn random_controller_scry_different_seeds_produce_different_decisions() {
    // With a 4-card reveal there are 2^4 = 16 possible partitions; two
    // hand-picked distinct seeds should map to distinct outcomes.
    // (If they ever happen to collide we can swap the seeds.)
    let library = vec![
        ("A", &[CardType::Instant][..]),
        ("B", &[CardType::Land][..]),
        ("C", &[CardType::Creature][..]),
        ("D", &[CardType::Sorcery][..]),
    ];

    let game = build_game_with_library(&library);
    let p1 = game.players[0].id;
    let revealed = game.scry_snapshot_top_n(p1, 4);

    let mut rc_a = RandomController::with_seed(p1, 11);
    let mut rc_b = RandomController::with_seed(p1, 22);

    let view = GameStateView::new(&game, p1);
    let d_a = unwrap_scry(rc_a.choose_scry_order(&view, &revealed));
    let d_b = unwrap_scry(rc_b.choose_scry_order(&view, &revealed));

    assert_ne!(
        (d_a.top.as_slice(), d_a.bottom.as_slice()),
        (d_b.top.as_slice(), d_b.bottom.as_slice()),
        "RandomController seeds 11 and 22 happened to produce identical scry partitions \
         — pick different seeds for this test"
    );
}

#[test]
fn random_controller_surveil_is_deterministic_given_seed() {
    let library = vec![
        ("A", &[CardType::Instant][..]),
        ("B", &[CardType::Creature][..]),
        ("C", &[CardType::Land][..]),
    ];

    let game = build_game_with_library(&library);
    let p1 = game.players[0].id;
    let revealed = game.surveil_snapshot_top_n(p1, 3);

    let mut rc1 = RandomController::with_seed(p1, 7777);
    let mut rc2 = RandomController::with_seed(p1, 7777);

    let view = GameStateView::new(&game, p1);
    let d1 = unwrap_surveil(rc1.choose_surveil(&view, &revealed));
    let d2 = unwrap_surveil(rc2.choose_surveil(&view, &revealed));
    assert_eq!(d1.top.as_slice(), d2.top.as_slice());
    assert_eq!(d1.graveyard.as_slice(), d2.graveyard.as_slice());
}

// ============================================================================
// Surveil graveyard wiring (Phase E #4)
// ============================================================================

#[test]
fn surveil_apply_decision_actually_moves_cards_to_graveyard() {
    // Pin the post-Phase-B/C invariant: cards listed in
    // `decision.graveyard` end up in the graveyard zone (in placement
    // order) and are removed from the library zone.
    //
    // This is the basic ZoneTransfer correctness check called for in
    // the Phase E acceptance criteria. (A separate task tracks
    // routing surveil through `move_card` so enter-the-graveyard
    // triggers fire — out of scope for Phase E, which is a
    // regression-prevention phase, not a correctness-improvement
    // phase.)
    let library = vec![
        ("A", &[CardType::Instant][..]),      // 2000
        ("B", &[CardType::Sorcery][..]),      // 2001
        ("C", &[CardType::Land][..]),         // 2002 — kept on top
        ("filler", &[CardType::Instant][..]), // 2003 — untouched below
    ];
    let mut game = build_game_with_library(&library);
    let p1 = game.players[0].id;

    let revealed = game.surveil_snapshot_top_n(p1, 3);

    let decision = SurveilDecision {
        // Bottom-up: only Land (C) stays on top.
        top: SmallVec::from_slice(&[CardId::new(2002)]),
        // Placement order: A milled first (deepest), then B above it.
        graveyard: SmallVec::from_slice(&[CardId::new(2000), CardId::new(2001)]),
    };
    game.surveil_apply_decision(p1, &revealed, &decision)
        .expect("apply surveil");

    // Library: top-down [Land, filler], no Instant or Sorcery.
    let zones = game.get_player_zones(p1).expect("zones");
    let lib_top_down: Vec<CardId> = zones.library.cards.iter().rev().copied().collect();
    assert_eq!(lib_top_down, vec![CardId::new(2002), CardId::new(2003)]);
    assert!(!zones.library.cards.contains(&CardId::new(2000)));
    assert!(!zones.library.cards.contains(&CardId::new(2001)));

    // Graveyard contains the milled cards in placement order.
    assert_eq!(
        zones.graveyard.cards.as_slice(),
        &[CardId::new(2000), CardId::new(2001)]
    );
}

#[test]
fn surveil_with_empty_decision_is_a_noop() {
    // A controller that decides to mill nothing should leave the
    // library in its original order and the graveyard untouched.
    let library = vec![("A", &[CardType::Instant][..]), ("B", &[CardType::Creature][..])];
    let mut game = build_game_with_library(&library);
    let p1 = game.players[0].id;

    let revealed = game.surveil_snapshot_top_n(p1, 2);
    let decision = SurveilDecision {
        // bottom-up; both stay on top
        top: SmallVec::from_slice(&[CardId::new(2000), CardId::new(2001)]),
        graveyard: SmallVec::new(),
    };
    game.surveil_apply_decision(p1, &revealed, &decision).expect("apply");

    let zones = game.get_player_zones(p1).expect("zones");
    assert!(zones.graveyard.cards.is_empty());
    assert_eq!(zones.library.cards.len(), 2);
}

// ============================================================================
// Protocol shape (Phase E #1, #5)
// ============================================================================

#[test]
fn protocol_scry_choice_type_carries_revealed_cards_inline() {
    // The protocol embeds revealed CardIds directly in
    // ChoiceType::Scry (rather than relying on a separate
    // CardRevealed round-trip), so the scrying player's client can
    // render the choice without an extra message. CR 701.18 — only
    // the scrying player sees these cards; the server only ever
    // sends Scry to the scrying player's controller, so the
    // embedded CardIds never leak to the opponent.
    let revealed_card_ids = vec![CardId::new(101), CardId::new(202), CardId::new(303)];
    let ct = ChoiceType::Scry {
        count: 3,
        revealed_card_ids: revealed_card_ids.clone(),
    };

    // serde round-trip preserves the embedded CardIds.
    let json = serde_json::to_string(&ct).expect("serialize");
    let roundtrip: ChoiceType = serde_json::from_str(&json).expect("deserialize");
    match roundtrip {
        ChoiceType::Scry {
            count,
            revealed_card_ids: rt,
        } => {
            assert_eq!(count, 3);
            assert_eq!(rt, revealed_card_ids);
        }
        other => panic!("expected ChoiceType::Scry, got {:?}", other),
    }
}

#[test]
fn protocol_surveil_choice_type_carries_revealed_cards_inline() {
    let revealed_card_ids = vec![CardId::new(11), CardId::new(22)];
    let ct = ChoiceType::Surveil {
        count: 2,
        revealed_card_ids: revealed_card_ids.clone(),
    };
    let json = serde_json::to_string(&ct).expect("serialize");
    let roundtrip: ChoiceType = serde_json::from_str(&json).expect("deserialize");
    match roundtrip {
        ChoiceType::Surveil {
            count,
            revealed_card_ids: rt,
        } => {
            assert_eq!(count, 2);
            assert_eq!(rt, revealed_card_ids);
        }
        other => panic!("expected ChoiceType::Surveil, got {:?}", other),
    }
}

// ============================================================================
// NetworkController dispatch shape (Phase E #1, end-to-end)
// ============================================================================

/// Helper that runs `NetworkController::choose_scry_order` against a
/// mocked client — the test thread plays the part of the client and
/// hand-crafts a `ChoiceResponse`. Returns the request the server
/// emitted and the decision it constructed from the response.
fn drive_network_scry(revealed: Vec<CardId>, client_response_indices: Vec<usize>) -> (ChoiceRequest, ScryDecision) {
    let game = build_game_with_library(&[
        ("A", &[CardType::Instant][..]),
        ("B", &[CardType::Land][..]),
        ("C", &[CardType::Creature][..]),
    ]);
    let p1 = game.players[0].id;

    let (req_tx, req_rx) = mpsc::channel::<ChoiceRequest>();
    let (resp_tx, resp_rx) = mpsc::channel::<ChoiceResponse>();

    let mut controller = NetworkController::new(p1, req_tx, resp_rx, Arc::new(AtomicUsize::new(0)));

    // Spawn a thread that plays the client: receive the request, then
    // send back the prepared response.
    let handle = std::thread::spawn(move || {
        let req = req_rx.recv().expect("server must send request");
        // Echo the choice_seq the server allocated (ChoiceRequest stores it).
        resp_tx
            .send(ChoiceResponse {
                choice_seq: req.choice_seq,
                choice_indices: client_response_indices,
                spell_ability: None,
                target_card_ids: None,
            })
            .expect("client must be able to reply");
        req
    });

    let view = GameStateView::new(&game, p1);
    let result = controller.choose_scry_order(&view, &revealed);
    let request = handle.join().expect("client thread");
    let decision = unwrap_scry(result);
    (request, decision)
}

#[test]
fn network_controller_emits_scry_choice_request() {
    // The NetworkController must send ChoiceType::Scry with the
    // revealed CardIds embedded — that is the contract the client
    // implementation in WasmRemoteController consumes.
    let revealed = vec![CardId::new(2000), CardId::new(2001), CardId::new(2002)];
    // Client puts position 1 on bottom; positions 0 and 2 stay on top.
    let (request, decision) = drive_network_scry(revealed.clone(), vec![1]);

    match request.choice_type {
        ChoiceType::Scry {
            count,
            revealed_card_ids,
        } => {
            assert_eq!(count, revealed.len());
            assert_eq!(revealed_card_ids, revealed);
        }
        other => panic!("expected ChoiceType::Scry on the wire, got {:?}", other),
    }

    // Decision: bottom = [revealed[1]], top = [revealed[0], revealed[2]]
    // converted to bottom-up — keep_top_down was [2000, 2002] (positions
    // 0 and 2 minus position 1), then `top_top_down.into_iter().rev()`
    // gives [2002, 2000].
    assert_eq!(decision.bottom.as_slice(), &[CardId::new(2001)]);
    assert_eq!(decision.top.as_slice(), &[CardId::new(2002), CardId::new(2000)]);
}

#[test]
fn network_controller_scry_keep_all_on_top_decision() {
    // Client sends an empty bottom list — every revealed card stays
    // on top in revealed (top-down) order, then converted bottom-up.
    let revealed = vec![CardId::new(2000), CardId::new(2001)];
    let (request, decision) = drive_network_scry(revealed.clone(), vec![]);

    assert!(matches!(request.choice_type, ChoiceType::Scry { .. }));
    assert!(decision.bottom.is_empty());
    // top_top_down was [2000, 2001]; reversed bottom-up = [2001, 2000].
    assert_eq!(decision.top.as_slice(), &[CardId::new(2001), CardId::new(2000)]);
}

#[test]
fn network_controller_scry_rejects_out_of_range_index() {
    // Defensive: an out-of-range index in the ChoiceResponse is a
    // protocol violation and must surface as ChoiceResult::Error
    // (NEVER silently truncated).
    let game = build_game_with_library(&[("A", &[CardType::Instant][..])]);
    let p1 = game.players[0].id;
    let revealed = vec![CardId::new(2000)]; // length 1 → only index 0 is valid

    let (req_tx, req_rx) = mpsc::channel::<ChoiceRequest>();
    let (resp_tx, resp_rx) = mpsc::channel::<ChoiceResponse>();

    let mut controller = NetworkController::new(p1, req_tx, resp_rx, Arc::new(AtomicUsize::new(0)));
    let handle = std::thread::spawn(move || {
        let req = req_rx.recv().expect("recv");
        // index 5 is out of range for a 1-card revealed list
        resp_tx
            .send(ChoiceResponse {
                choice_seq: req.choice_seq,
                choice_indices: vec![5],
                spell_ability: None,
                target_card_ids: None,
            })
            .expect("send");
    });

    let view = GameStateView::new(&game, p1);
    let result = controller.choose_scry_order(&view, &revealed);
    handle.join().expect("client thread");

    match result {
        ChoiceResult::Error(msg) => {
            // The upstream `request_choice` validates indices against
            // `options.len()` before our scry-specific check fires, so
            // either error string is acceptable evidence of "we caught
            // an out-of-range index". The KEY invariant is that the
            // engine REJECTS the bad index instead of silently
            // truncating or clamping it (per
            // docs/NETWORK_ARCHITECTURE.md: "desync is ALWAYS fatal").
            let recognized =
                msg.contains("Invalid scry index") || msg.contains("invalid choice index") || msg.contains("DESYNC");
            assert!(
                recognized,
                "expected an out-of-range / desync error message, got: {msg}"
            );
        }
        other => panic!(
            "expected ChoiceResult::Error for out-of-range scry index, got {:?}",
            other
        ),
    }
}

// ============================================================================
// Information hiding (CR 701.18) — Phase E #5
// ============================================================================

#[test]
fn protocol_does_not_expose_scry_to_opponent_player_id() {
    // CR 701.18 says only the scrying player sees the revealed
    // cards. Our protocol enforces this by construction: the
    // engine only ever calls choose_scry_order on the scrying
    // player's controller, so the only ChoiceRequest carrying
    // ChoiceType::Scry is addressed to that player.
    //
    // This test pins the structural invariant that a
    // ChoiceType::Scry IS NOT in ChoiceRequest's `reveals`
    // streaming side-channel — it's embedded inline in the
    // ChoiceType, so it's only sent to the choice's recipient,
    // not broadcast as a CardRevealed to all players.
    let revealed = vec![CardId::new(7), CardId::new(13)];
    let (req_tx, req_rx) = mpsc::channel::<ChoiceRequest>();
    let (resp_tx, resp_rx) = mpsc::channel::<ChoiceResponse>();

    let game = build_game_with_library(&[("A", &[CardType::Instant][..]), ("B", &[CardType::Sorcery][..])]);
    let p1 = game.players[0].id;

    let mut controller = NetworkController::new(p1, req_tx, resp_rx, Arc::new(AtomicUsize::new(0)));
    let handle = std::thread::spawn(move || {
        let req = req_rx.recv().expect("recv");
        resp_tx
            .send(ChoiceResponse {
                choice_seq: req.choice_seq,
                choice_indices: vec![],
                spell_ability: None,
                target_card_ids: None,
            })
            .expect("send");
        req
    });

    let view = GameStateView::new(&game, p1);
    let _ = controller.choose_scry_order(&view, &revealed);
    let request = handle.join().expect("join");

    // The reveals streaming side-channel must NOT contain the scry
    // cards — they belong to the scrying player's hidden choice.
    for r in &request.reveals {
        assert!(
            !revealed.contains(&r.card_id),
            "scry-revealed CardId {:?} must NOT appear in the broadcast `reveals` stream",
            r.card_id
        );
    }

    // The Scry CardIds should be embedded inline in ChoiceType::Scry.
    match request.choice_type {
        ChoiceType::Scry { revealed_card_ids, .. } => assert_eq!(revealed_card_ids, revealed),
        other => panic!("expected ChoiceType::Scry, got {:?}", other),
    }
}

// PlayerId imported for ergonomic access in build_game_with_library; the
// type parameter on the controller helpers below uses it implicitly.
#[allow(dead_code)]
fn _use_player_id(_p: PlayerId) {}
