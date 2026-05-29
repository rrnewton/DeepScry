//! `ChoiceEntry` — the payload type for the per-controller choice buffer.
//!
//! This is the concrete `T` plugged into `ActionLog<T>` to back the
//! per-controller choice buffer described in `docs/NETWORK_ACTION_LOG.md`
//! § 3.1. Each entry holds the structured payload of an `OpponentChoice`
//! (server -> client) or, in step 3, a buffered local UI click.
//!
//! Each entry is keyed in the log by the **server-reported `action_count`**
//! at which the choice was made. Reads are non-destructive and replayable;
//! a rewind / snapshot resume re-reads the same entry by `action_count` and
//! drives the engine to bit-identical state.
//!
//! # Why a struct, not the wire-protocol enum directly
//!
//! The wire-protocol `ServerMessage::OpponentChoice` variant carries
//! transport-level fields (`choice_seq`, timestamps) that are part of the
//! message envelope, not the choice semantics. The choice buffer needs the
//! choice payload itself — what the opponent decided, plus the structured
//! disambiguators (`spell_ability`, `library_search_result`,
//! `target_card_ids`) the shadow controller needs to apply it. The struct
//! preserves the `choice_seq` for diagnostics (logging, dedup checks) but
//! the **`action_count` lives in the `ActionLog` index, not in the
//! payload**.
//!
//! # CLAUDE.md alignment
//!
//! Strong types: every field is its concrete type
//! (`Vec<usize>` / `SpellAbility` / `CardId` / `Vec<CardId>`), no
//! `String`-tagged discriminants. The description string is the
//! pre-formatted log line from the server (already a `String` on the
//! wire); it is opaque to the controller and is plumbed through verbatim
//! so the `[GAMELOG]` line matches the server's.

use crate::core::{CardId, SpellAbility};

/// One entry in a per-controller choice buffer (`ActionLog<ChoiceEntry>`).
///
/// Mirrors the structured payload of `ServerMessage::OpponentChoice` — see
/// `docs/NETWORK_ACTION_LOG.md` § 3.1 for ownership and § 5 for the
/// list of legacy paths this replaces (the WASM
/// `WasmNetworkClient::opponent_choices` VecDeque in particular).
#[derive(Debug, Clone)]
pub struct ChoiceEntry {
    /// Server-assigned choice sequence number. Kept on the payload (not
    /// the log key) so duplicate-suppression and diagnostic logging still
    /// see the wire-protocol value, even after a rewind / replay re-reads
    /// the entry by `action_count`.
    pub choice_seq: u32,
    /// The choice as a sequence of integer indices into the controller's
    /// option list. Multi-index choices (mana payment, attackers,
    /// blockers, modes) carry multiple entries.
    pub choice_indices: Vec<usize>,
    /// Pre-formatted server-side description of the choice. Plumbed
    /// through verbatim so the client's `[GAMELOG]` line matches the
    /// server's. Opaque to the controller's decision logic.
    pub description: String,
    /// For `choose_spell_ability_to_play`: the authoritative
    /// `SpellAbility` the server picked, so the shadow controller does
    /// not have to reconstruct it from a `valid` list that may not even
    /// be visible client-side (opponent's hand cards).
    pub spell_ability: Option<SpellAbility>,
    /// For `choose_from_library` / `LibrarySearchByName`: the specific
    /// `CardId` the server's tutor moved to hand. The shadow uses this
    /// instead of an index because the client's view of the library may
    /// not include the matched card (hidden information).
    pub library_search_result: Option<CardId>,
    /// For SMART damage assignment (`choose_blocker_for_lethal_damage`
    /// / `_for_remaining_damage`): the authoritative `CardId` list the
    /// server chose, used when index-based lookup would point at the
    /// wrong shadow-side blocker (mtg-418).
    pub target_card_ids: Option<Vec<CardId>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::ActionLog;

    fn mk_entry(seq: u32, desc: &str) -> ChoiceEntry {
        ChoiceEntry {
            choice_seq: seq,
            choice_indices: vec![seq as usize],
            description: desc.into(),
            spell_ability: None,
            library_search_result: None,
            target_card_ids: None,
        }
    }

    #[test]
    fn choice_entries_round_trip_through_action_log() {
        // The strongly-typed payload slots into the generic ActionLog<T>
        // primitive; entries are retrievable by action_count. This is the
        // contract that backs WasmNetworkClient::opponent_choices
        // (docs/NETWORK_ACTION_LOG.md § 3.1).
        let mut log: ActionLog<ChoiceEntry> = ActionLog::new();
        log.push(1, mk_entry(10, "pass"));
        log.push(5, mk_entry(11, "play Forest"));
        log.push(9, mk_entry(12, "attack with 2 creatures"));

        assert_eq!(log.len(), 3);
        assert_eq!(log.frontier(), Some(9));
        match log.get(1) {
            Some(e) => {
                assert_eq!(e.choice_seq, 10);
                assert_eq!(e.choice_indices, vec![10]);
                assert_eq!(e.description, "pass");
            }
            None => panic!("expected entry at ac=1"),
        }
        // Repeated read is non-destructive.
        let first = log.get(5).cloned().unwrap();
        let second = log.get(5).cloned().unwrap();
        assert_eq!(first.choice_seq, second.choice_seq);
        assert_eq!(log.len(), 3);
    }

    #[test]
    fn replay_re_reads_entries_deterministically() {
        // The whole point of the per-controller buffer: rewind to ac=0
        // and replay forward, every read at K returns the same entry
        // forever. This is what makes the buffer/undo_log isomorphism
        // (invariant #11 of docs/NETWORK_ACTION_LOG.md § 9) hold.
        let mut log: ActionLog<ChoiceEntry> = ActionLog::new();
        log.push(2, mk_entry(1, "a"));
        log.push(4, mk_entry(2, "b"));
        log.push(6, mk_entry(3, "c"));

        let forward: Vec<u32> = [2u64, 4, 6]
            .iter()
            .map(|&ac| log.get(ac).map(|e| e.choice_seq).unwrap())
            .collect();
        let replay: Vec<u32> = [2u64, 4, 6]
            .iter()
            .map(|&ac| log.get(ac).map(|e| e.choice_seq).unwrap())
            .collect();
        assert_eq!(forward, replay);
        assert_eq!(log.len(), 3);
    }
}
