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
use serde::{Deserialize, Serialize};

/// The structured payload of a single choice — the data the shadow's remote
/// controller needs to replay an opponent's decision without seeing hidden
/// information (mtg-787).
///
/// This is the ONE canonical declaration of the "choice payload" that used to
/// be re-spelled field-by-field in five places: `BufferedFact::Choice` and
/// `ServerMessage::OpponentChoice` (the wire forms), `ChoiceEntry` (the
/// per-controller buffer entry), `OpponentChoiceInfo` (the server-side
/// broadcast record), and the now-deleted `CachedOpponentChoice` (a
/// field-renamed clone in `remote_controller.rs`). The envelope fields that
/// vary by call site — `choice_seq`, `choice_type`, `description`,
/// `action_count`, `player`, transport timestamps — stay on the enclosing
/// type; only the decision payload itself lives here.
///
/// # Wire-format invariance
///
/// The wire forms embed this with `#[serde(flatten)]`, so the four fields
/// appear at the SAME JSON object level they did when spelled inline. The
/// project speaks JSON on the wire (`serde_json`, order-independent), and the
/// rewind/replay buffer comparison is **structural** (`PartialEq` /
/// `choice_indices` field compare in `push_opponent_choice`), never a
/// JSON-string compare — so flattening is byte-compatible and replay-stable.
/// `library_search_result` / `target_card_ids` carry `#[serde(default)]` to
/// stay decode-compatible with legacy senders that omit them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChoicePayload {
    /// The choice as a sequence of integer indices into the controller's
    /// option list. Multi-index choices (mana payment, attackers,
    /// blockers, modes) carry multiple entries.
    pub choice_indices: Vec<usize>,
    /// For `choose_spell_ability_to_play`: the authoritative
    /// `SpellAbility` the server picked, so the shadow controller does
    /// not have to reconstruct it from a `valid` list that may not even
    /// be visible client-side (opponent's hand cards).
    pub spell_ability: Option<SpellAbility>,
    /// For `choose_from_library` / `LibrarySearchByName`: the specific
    /// `CardId` the server's tutor moved to hand. The shadow uses this
    /// instead of an index because the client's view of the library may
    /// not include the matched card (hidden information).
    #[serde(default)]
    pub library_search_result: Option<CardId>,
    /// For SMART damage assignment (`choose_blocker_for_lethal_damage`
    /// / `_for_remaining_damage`): the authoritative `CardId` list the
    /// server chose, used when index-based lookup would point at the
    /// wrong shadow-side blocker (mtg-418).
    #[serde(default)]
    pub target_card_ids: Option<Vec<CardId>>,
}

/// One entry in a per-controller choice buffer (`ActionLog<ChoiceEntry>`).
///
/// Mirrors the structured payload of `ServerMessage::OpponentChoice` — see
/// `docs/NETWORK_ACTION_LOG.md` § 3.1 for ownership and § 5 for the
/// list of legacy paths this replaces (the WASM
/// `WasmNetworkClient::opponent_choices` VecDeque in particular).
#[derive(Debug, Clone)]
pub struct ChoiceEntry {
    /// Server-assigned choice sequence number. This is ALSO the
    /// `ActionLog<ChoiceEntry>` key for the opponent-choice buffer
    /// (`WasmNetworkClient::opponent_choices`): unlike `action_count`,
    /// `choice_seq` is strictly unique and monotonic per choice by
    /// construction (the server bumps it once per `ChoiceRequest`), so it
    /// satisfies `ActionLog::push`'s strict-monotonicity invariant even when
    /// several choices share one `action_count` (mtg-sfihb: multi-step
    /// combat damage assignment — `choose_blocker_for_lethal_damage` then
    /// `choose_blocker_for_remaining_damage` for the same attacker — emits
    /// two choices before any undoable action advances `undo_log.len()`).
    pub choice_seq: u32,
    /// Server-reported `action_count` (= `undo_log.len()` at the moment the
    /// choice was requested). Carried on the payload for diagnostics /
    /// display only. It is NOT the log key, because `action_count` is not
    /// unique per choice (see `choice_seq` above).
    pub action_count: u64,
    /// Pre-formatted server-side description of the choice. Plumbed
    /// through verbatim so the client's `[GAMELOG]` line matches the
    /// server's. Opaque to the controller's decision logic.
    pub description: String,
    /// The structured decision payload (indices + the hidden-info
    /// disambiguators). See [`ChoicePayload`].
    pub payload: ChoicePayload,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::ActionLog;

    fn mk_entry(seq: u32, desc: &str) -> ChoiceEntry {
        mk_entry_ac(seq, 0, desc)
    }

    fn mk_entry_ac(seq: u32, action_count: u64, desc: &str) -> ChoiceEntry {
        ChoiceEntry {
            choice_seq: seq,
            action_count,
            description: desc.into(),
            payload: ChoicePayload {
                choice_indices: vec![seq as usize],
                spell_ability: None,
                library_search_result: None,
                target_card_ids: None,
            },
        }
    }

    #[test]
    fn choice_seq_key_tolerates_duplicate_action_counts() {
        // mtg-sfihb regression (native, target-independent mirror of the
        // wasm-only WasmNetworkClient test): the opponent-choice buffer is
        // keyed by `choice_seq`, NOT `action_count`. During multi-step combat
        // damage assignment the server emits two OpponentChoices
        // (choose_blocker_for_lethal_damage then
        // choose_blocker_for_remaining_damage for the same attacker) with no
        // undoable action between them, so BOTH carry the same action_count.
        // The previously-used `action_count` key made the second push panic
        // with "action_count must be strictly increasing". Keying by
        // `choice_seq` (strictly unique per choice) must NOT panic and must
        // keep the duplicated action_count on each payload.
        //
        // Exact shape observed in the rogerbrand seed-3 mirror at the failing
        // run: action_count=978, choice_seq 181 then 182.
        let mut log: ActionLog<ChoiceEntry> = ActionLog::new();
        log.push(181, mk_entry_ac(181, 978, "lethal damage assignment"));
        log.push(182, mk_entry_ac(182, 978, "remaining damage assignment"));

        assert_eq!(log.len(), 2);
        assert_eq!(log.frontier(), Some(182));
        assert_eq!(log.get(181).unwrap().action_count, 978);
        assert_eq!(log.get(182).unwrap().action_count, 978);
        assert_eq!(log.get(181).unwrap().description, "lethal damage assignment");
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
                assert_eq!(e.payload.choice_indices, vec![10]);
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
