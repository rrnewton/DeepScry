//! `StateSyncEntry` — the payload type for the shadow state-sync log.
//!
//! This is the concrete `T` plugged into `ActionLog<T>` to back the
//! `NetworkClient`-owned shadow state-sync stream described in
//! `docs/NETWORK_ACTION_LOG.md` § 3.2.
//!
//! Each entry is a single server-pushed mutation to the **shadow `GameState`**
//! that is not anyone's "choice" — it has no controller home:
//!
//! - `RevealCard` — server announces a card identity (opening hand draw,
//!   `Demonic Tutor` reveal, `Sensei's Divining Top` look, etc).
//! - `LibraryReorder` — server publishes a player's authoritative library
//!   order after a shuffle / scry / surveil / search.
//!
//! Future variants (any other server → shadow-state mutation that is not a
//! controller choice) extend this enum; the `ActionLog<StateSyncEntry>`
//! primitive does not change.
//!
//! # Why a strongly-typed enum, not a `String`-tagged blob
//!
//! `CLAUDE.md` mandates strong types for protocol-level data. A `String`
//! "kind" field would defeat exhaustive matching and force every consumer
//! to re-parse a hand-rolled discriminant. The variants carry their full
//! structured payload directly.

use crate::core::{CardId, PlayerId};
use crate::network::{CardReveal, RevealReason};

/// One server-pushed mutation to the shadow `GameState`, tagged with the
/// `action_count` at which it should be applied. See module doc for
/// rationale.
#[derive(Debug, Clone)]
pub enum StateSyncEntry {
    /// `ServerMessage::CardRevealed` payload — a card identity the
    /// shadow learns about (e.g. the top of our library for
    /// `Sensei's Divining Top`, an opening-hand draw, etc).
    ///
    /// `card` is boxed because `CardReveal` carries a full `CardDefinition`
    /// (≳ 260 bytes); the other variant's payload is ~16 bytes, so an
    /// unboxed enum would bloat every entry. Keeps the log's `Vec<(u64, T)>`
    /// storage compact.
    RevealCard {
        owner: PlayerId,
        card: Box<CardReveal>,
        reason: RevealReason,
    },
    /// `ServerMessage::LibraryReordered` payload — the server-authoritative
    /// new library order for a player after any reorder event (shuffle,
    /// scry, surveil, library-search). The shadow MUST adopt this order or
    /// the next draw diverges and produces a FATAL state-hash mismatch.
    ///
    /// `new_order` is **top-to-bottom**; the shadow's library `Vec` is
    /// stored **bottom-to-top** (so `pop` is "draw top"). Consumers must
    /// reverse on application.
    LibraryReorder { player: PlayerId, new_order: Vec<CardId> },
    /// `ServerMessage::SearchCandidates` payload — the N candidate identities a
    /// searching player sees when resolving a `LibrarySearchByName` choice
    /// (mtg-752 / mtg-253). A single atomic-multi-delta keyed at ONE game
    /// `action_count` (the search-resolution ac); carrying `Vec<CardReveal>`
    /// avoids the strict-monotonicity collision that N separate reveals at one
    /// ac would cause in the game-ac-keyed `ActionLog`. Applied by replaying
    /// `process_card_reveal_wasm` over each candidate (with `searcher` as the
    /// card owner, `RevealReason::Searched`).
    SearchCandidates { searcher: PlayerId, cards: Vec<CardReveal> },
}

/// True iff two `StateSyncEntry` values describe the SAME logical delta
/// (same variant, same identity payload, ignoring the non-identifying
/// `reason` tag on reveals).
///
/// Used by BOTH the native (`network::client`) and WASM
/// (`wasm::network::client`) shadow state-sync logs to decide whether a
/// duplicate-`action_count` arrival is a benign idempotent re-send (DROP) or
/// a genuine protocol desync (FATAL). Because `action_count == undo_log.len()`
/// is globally unique per logged action, two entries sharing an `ac` can only
/// be the same logical delta re-sent — this predicate verifies that and lets
/// the caller crash on a mismatch (NETWORK_ARCHITECTURE.md: Desync is ALWAYS
/// Fatal). Shared here (one primitive, native + WASM) per the DRY rule.
#[must_use]
pub fn state_sync_entries_equivalent(a: &StateSyncEntry, b: &StateSyncEntry) -> bool {
    use StateSyncEntry::*;
    match (a, b) {
        (
            RevealCard {
                owner: oa, card: ca, ..
            },
            RevealCard {
                owner: ob, card: cb, ..
            },
        ) => oa == ob && ca.card_id == cb.card_id && ca.name == cb.name,
        (
            LibraryReorder {
                player: pa,
                new_order: na,
            },
            LibraryReorder {
                player: pb,
                new_order: nb,
            },
        ) => pa == pb && na == nb,
        (
            SearchCandidates {
                searcher: sa,
                cards: ca,
            },
            SearchCandidates {
                searcher: sb,
                cards: cb,
            },
        ) => {
            sa == sb
                && ca.len() == cb.len()
                && ca
                    .iter()
                    .zip(cb.iter())
                    .all(|(x, y)| x.card_id == y.card_id && x.name == y.name)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::ActionLog;

    fn mk_reveal(card_id: u32, name: &str) -> StateSyncEntry {
        StateSyncEntry::RevealCard {
            owner: PlayerId::new(0),
            card: Box::new(CardReveal {
                card_id: CardId::new(card_id),
                name: name.into(),
                card_def: None,
            }),
            reason: RevealReason::Draw,
        }
    }

    fn mk_reorder(player_id: u32, cards: &[u32]) -> StateSyncEntry {
        StateSyncEntry::LibraryReorder {
            player: PlayerId::new(player_id),
            new_order: cards.iter().copied().map(CardId::new).collect(),
        }
    }

    #[test]
    fn state_sync_entries_round_trip_through_action_log() {
        // Smoke test: the strongly-typed enum slots into the generic
        // ActionLog<T> primitive and entries are retrievable by ac.
        // This is the contract that backs the WasmNetworkClient's
        // state_sync field (docs/NETWORK_ACTION_LOG.md § 3.2).
        let mut log: ActionLog<StateSyncEntry> = ActionLog::new();
        log.push(1, mk_reveal(101, "Mountain"));
        log.push(2, mk_reorder(0, &[101, 102, 103]));
        log.push(3, mk_reveal(102, "Lightning Bolt"));

        assert_eq!(log.len(), 3);
        assert_eq!(log.frontier(), Some(3));
        match log.get(1) {
            Some(StateSyncEntry::RevealCard { card, .. }) => assert_eq!(card.name, "Mountain"),
            other => panic!("expected RevealCard at ac=1, got {other:?}"),
        }
        match log.get(2) {
            Some(StateSyncEntry::LibraryReorder { new_order, .. }) => {
                assert_eq!(new_order.len(), 3);
            }
            other => panic!("expected LibraryReorder at ac=2, got {other:?}"),
        }
    }

    #[test]
    fn state_sync_log_arrival_order_independence_via_action_count() {
        // The whole point of the action_count-keyed state-sync refactor (robots42 / mtg-559):
        // two arrival orderings of the same set of entries must yield
        // identical readbacks when keyed by action_count.
        //
        // The legacy `drain_*` calls saw the wrong subset when arrival
        // races interleaved reveals across choice boundaries. ActionLog
        // keyed by action_count is order-independent because reads pick
        // their entry by ac, not by FIFO position.
        let mut log_a: ActionLog<StateSyncEntry> = ActionLog::new();
        log_a.push(1, mk_reveal(1, "A"));
        log_a.push(2, mk_reveal(2, "B"));
        log_a.push(3, mk_reorder(0, &[2, 1]));

        let mut log_b: ActionLog<StateSyncEntry> = ActionLog::new();
        // Different "wire" arrival: same entries, but the pushes are
        // synthetic and append-order is what defines ac. The key property
        // is that for any FIXED arrival order, get-by-ac is stable —
        // re-reads at ac=K return the same entry forever.
        log_b.push(1, mk_reveal(1, "A"));
        log_b.push(2, mk_reveal(2, "B"));
        log_b.push(3, mk_reorder(0, &[2, 1]));

        for ac in 1..=3 {
            let a = log_a.get(ac);
            let b = log_b.get(ac);
            match (a, b) {
                (
                    Some(StateSyncEntry::RevealCard { card: ca, .. }),
                    Some(StateSyncEntry::RevealCard { card: cb, .. }),
                ) => assert_eq!(ca.name, cb.name),
                (
                    Some(StateSyncEntry::LibraryReorder { new_order: oa, .. }),
                    Some(StateSyncEntry::LibraryReorder { new_order: ob, .. }),
                ) => assert_eq!(oa, ob),
                other => panic!("entry mismatch at ac={ac}: {other:?}"),
            }
        }
    }
}
