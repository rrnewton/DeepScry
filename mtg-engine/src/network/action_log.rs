//! Generic append-only, `action_count`-indexed, non-destructive log.
//!
//! This is the foundational primitive for the network re-architecture
//! (mtg-o99ow) described in `docs/NETWORK_ACTION_LOG.md`. It is the SHARED
//! substrate that backs
//! THREE distinct owners (per the design doc § 3):
//!
//! 1. **Per-controller choice buffer** (private to each controller).
//!    Each `Controller` impl embeds an `ActionLog<ChoiceEntry>` keyed by the
//!    `action_count` at which that choice was made.
//! 2. **Shadow state-sync log** (`NetworkClient`-owned, shadow mode only).
//!    Buffers server-pushed mutations to the shadow `GameState` —
//!    `CardRevealed` and `LibraryReordered` — at the `action_count` they
//!    apply at.
//! 3. **Server-side MCTS rollout log** (future). Same primitive feeds
//!    hypothetical lines during planning.
//!
//! See `docs/NETWORK_ACTION_LOG.md` for the ownership split rationale.
//!
//! # Invariants (from `docs/NETWORK_ACTION_LOG.md` § 8)
//!
//! 1. **Append-only.** Only the designated appender (WS reader, UI event
//!    handler, MCTS driver) appends. No code ever removes or rewrites
//!    entries.
//! 2. **Strictly monotonically increasing `action_count`.** At most one
//!    entry per `action_count` per log. `push` panics on violation —
//!    per `NETWORK_ARCHITECTURE.md` § *Desync is ALWAYS a Fatal Error*,
//!    we crash rather than silently re-order.
//! 3. **Non-destructive reads.** `get(k)` returns the same `&T` on every
//!    call; rewind / replay is therefore free.
//! 4. **Frontier-bounded.** `frontier()` is the highest `action_count`
//!    appended so far. A read of `action_count > frontier` is the only
//!    legitimate "I need more data" signal.
//!
//! # Why one primitive, not three
//!
//! Three independent owners (each controller + state-sync stream) share
//! the identical access pattern: append in monotonic `action_count`
//! order; look up by `action_count`; report frontier. Centralising
//! that pattern here (DRY per `CLAUDE.md`) means each owner becomes a
//! thin field declaration plus a small adapter, rather than re-implementing
//! the invariants.
//!
//! # Storage choice
//!
//! Single `Vec<(u64, T)>` in `action_count`-ascending order. Lookup is
//! binary search by `action_count` (O(log N)). We deliberately do NOT
//! keep a secondary `HashMap<action_count, usize>`:
//!
//! - The Vec is already in sorted order; the hashmap would be pure
//!   duplication.
//! - Cardinality is small (≲10⁴ entries per game), so log₂N ≤ ~14
//!   comparisons beats a hash.
//! - A second data structure is exactly the anti-pattern we're trying
//!   to eliminate from the existing `pop_*` / `drain_*` paths.
//!
//! If profiling later shows binary search is hot, we can switch to a
//! dense `Vec<Option<T>>` indexed by `action_count - first_ac` (O(1))
//! without changing the public API.

// ═══════════════════════════════════════════════════════════════════════════
// LOG
// ═══════════════════════════════════════════════════════════════════════════

/// Append-only log indexed by `action_count`.
///
/// Generic over the entry type `T`. Each owner picks its own `T`:
/// controllers use a `ChoiceEntry` describing the choice they made;
/// `NetworkClient` uses a `StateSyncEntry` describing a server-pushed
/// state mutation; an MCTS driver could use a `SimulatedEntry`, etc.
#[derive(Debug, Clone)]
pub struct ActionLog<T> {
    /// Entries in strictly `action_count`-ascending order.
    /// `(action_count, payload)` — no parallel index structure.
    entries: Vec<(u64, T)>,
}

impl<T> Default for ActionLog<T> {
    fn default() -> Self {
        Self { entries: Vec::new() }
    }
}

impl<T> ActionLog<T> {
    /// Create an empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append one entry at `action_count`.
    ///
    /// # Panics
    ///
    /// Panics if `action_count <= self.frontier()` — the log is strictly
    /// monotonically increasing, and two entries sharing an `action_count`
    /// would violate invariant #2 of `docs/NETWORK_ACTION_LOG.md`. The
    /// designated appender is the sole writer and is expected to maintain
    /// this invariant; a violation indicates a protocol / wiring bug, not
    /// a recoverable condition. Per `NETWORK_ARCHITECTURE.md` § *Desync
    /// is ALWAYS a Fatal Error*, we crash rather than silently re-order.
    pub fn push(&mut self, action_count: u64, entry: T) {
        if let Some(&(last_ac, _)) = self.entries.last() {
            assert!(
                action_count > last_ac,
                "ActionLog::push: action_count must be strictly increasing \
                 (last={last_ac}, new={action_count}). This is a protocol / \
                 wiring bug; the log is append-only and monotonic by construction.",
            );
        }
        self.entries.push((action_count, entry));
    }

    /// Insert one entry at `action_count`, keeping the backing Vec in
    /// `action_count`-ascending order even when entries ARRIVE out of order.
    ///
    /// This is the arrival-order-INDEPENDENT companion to [`push`](Self::push),
    /// used ONLY by the shadow **state-sync** log (owner #2), never by the
    /// per-controller choice buffer (owner #1). It exists because the server
    /// emits state-sync deltas (reveals + library reorders) for one choice
    /// window via TWO uncoordinated paths — the coordinator's
    /// `LibraryReordered` broadcast (stamped at the reorder's own, often
    /// LARGER, undo-log ac) is sent BEFORE the handler's `choice.reveals`
    /// loop (stamped at each reveal's smaller ac). So a delta at ac 380 can
    /// reach the client ahead of one at ac 376 (mtg-o99ow WASM bug #2). The
    /// log is **keyed and consumed by GAME `action_count`** ([`get`](Self::get)
    /// / [`frontier`](Self::frontier) / [`iter`](Self::iter) all operate on
    /// the sorted Vec), so the wire ARRIVAL order is an efficiency concern,
    /// not a correctness one: re-sorting on insert restores the canonical
    /// game-position order and the apply cursor is unaffected.
    ///
    /// # Panics
    ///
    /// Panics on an EXACT-duplicate `action_count` — two DISTINCT deltas can
    /// never legitimately share an ac (`action_count == undo_log.len()` is
    /// globally unique per logged action; the single atomic-multi case,
    /// library-search candidates, is modeled as ONE `SearchCandidates` entry,
    /// and the pre-game ac-0 initial orders are held outside this log). A
    /// genuine same-ac arrival is therefore a protocol/wiring desync, and per
    /// `NETWORK_ARCHITECTURE.md` § *Desync is ALWAYS a Fatal Error* we crash
    /// rather than silently merge.
    pub fn insert_sorted(&mut self, action_count: u64, entry: T) {
        match self.entries.binary_search_by_key(&action_count, |&(ac, _)| ac) {
            Ok(_) => panic!(
                "ActionLog::insert_sorted: duplicate action_count {action_count}. \
                 Two distinct state-sync deltas must never share an action_count \
                 (it is undo_log.len(), globally unique per logged action). This is \
                 a protocol / wiring desync; the only atomic-multi delta is \
                 SearchCandidates, modeled as a single entry."
            ),
            Err(idx) => self.entries.insert(idx, (action_count, entry)),
        }
    }

    /// Look up an entry by its `action_count`.
    ///
    /// Returns `None` if no entry was pushed at exactly `action_count`
    /// (either it's past the frontier — engine should yield `NeedsInput` —
    /// or this owner had no entry to contribute at that slot; the caller
    /// distinguishes via `frontier()`).
    pub fn get(&self, action_count: u64) -> Option<&T> {
        match self.entries.binary_search_by_key(&action_count, |&(ac, _)| ac) {
            Ok(idx) => Some(&self.entries[idx].1),
            Err(_) => None,
        }
    }

    /// Highest `action_count` appended so far, or `None` if empty.
    ///
    /// Engine compares its requested `action_count K` against this:
    /// `K > frontier()` is the only legitimate "I need more data"
    /// signal (invariant #5 of the design doc).
    pub fn frontier(&self) -> Option<u64> {
        self.entries.last().map(|&(ac, _)| ac)
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True if no entries have been appended.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate `(action_count, &entry)` pairs in append (= `action_count`-
    /// ascending) order. Useful for diagnostics and batch consumers (mtg-o99ow)
    /// that walk the log from the engine's current cursor up to the frontier.
    pub fn iter(&self) -> impl Iterator<Item = (u64, &T)> {
        self.entries.iter().map(|(ac, e)| (*ac, e))
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    // A trivial payload type to exercise the generic. The primitive itself
    // is payload-agnostic; integration tests for the concrete `ChoiceEntry`
    // and `StateSyncEntry` types live with their owners (mtg-o99ow).
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Payload(u32);

    #[test]
    fn new_log_is_empty() {
        let log: ActionLog<Payload> = ActionLog::new();
        assert!(log.is_empty());
        assert_eq!(log.len(), 0);
        assert_eq!(log.frontier(), None);
    }

    #[test]
    fn push_extends_frontier() {
        let mut log = ActionLog::new();
        log.push(3, Payload(30));
        assert_eq!(log.frontier(), Some(3));
        log.push(5, Payload(50));
        assert_eq!(log.frontier(), Some(5));
        log.push(8, Payload(80));
        assert_eq!(log.frontier(), Some(8));
        assert_eq!(log.len(), 3);
    }

    #[test]
    fn get_returns_matching_entry() {
        let mut log = ActionLog::new();
        log.push(2, Payload(20));
        log.push(4, Payload(40));
        log.push(7, Payload(70));

        assert_eq!(log.get(2), Some(&Payload(20)));
        assert_eq!(log.get(4), Some(&Payload(40)));
        assert_eq!(log.get(7), Some(&Payload(70)));

        // Gaps and past-frontier reads return None.
        assert_eq!(log.get(0), None);
        assert_eq!(log.get(3), None); // gap
        assert_eq!(log.get(99), None); // past frontier
    }

    #[test]
    fn reads_are_non_destructive() {
        // Invariant #3: get() is repeatable and idempotent.
        // This is THE property that makes rewind/replay free.
        let mut log = ActionLog::new();
        log.push(10, Payload(100));

        let first = log.get(10).cloned().unwrap();
        let second = log.get(10).cloned().unwrap();
        let third = log.get(10).cloned().unwrap();

        assert_eq!(first, Payload(100));
        assert_eq!(second, Payload(100));
        assert_eq!(third, Payload(100));
        assert_eq!(log.len(), 1, "reads must not remove entries");
        assert_eq!(log.frontier(), Some(10));
    }

    #[test]
    fn frontier_signal_is_the_only_wait_signal() {
        // Invariant #5: K > frontier means "need more data".
        let mut log: ActionLog<Payload> = ActionLog::new();
        assert_eq!(log.frontier(), None);
        assert_eq!(log.get(0), None);

        log.push(5, Payload(50));

        // Engine asks for K=5 → present.
        assert!(log.get(5).is_some());
        assert!(log.frontier().unwrap() >= 5);

        // Engine asks for K=6 → past frontier; caller's protocol is
        // "K > frontier() ⇒ yield NeedsInput and unwind".
        assert!(log.get(6).is_none());
        assert!(log.frontier().unwrap() < 6);
    }

    #[test]
    #[should_panic(expected = "strictly increasing")]
    fn push_equal_action_count_panics() {
        // Invariant #2: at most one entry per action_count.
        let mut log = ActionLog::new();
        log.push(7, Payload(1));
        log.push(7, Payload(2));
    }

    #[test]
    #[should_panic(expected = "strictly increasing")]
    fn push_decreasing_action_count_panics() {
        let mut log = ActionLog::new();
        log.push(10, Payload(1));
        log.push(5, Payload(2));
    }

    #[test]
    fn insert_sorted_tolerates_out_of_order_arrival() {
        // The state-sync log's arrival-order-independent appender: deltas can
        // arrive in any order; the Vec stays game-ac-sorted so get/iter/frontier
        // see canonical game-position order (mtg-o99ow WASM bug #2).
        let mut log = ActionLog::new();
        log.insert_sorted(380, Payload(380));
        log.insert_sorted(376, Payload(376)); // earlier ac arrives later
        log.insert_sorted(8, Payload(8));
        log.insert_sorted(400, Payload(400));
        let acs: Vec<u64> = log.iter().map(|(ac, _)| ac).collect();
        assert_eq!(acs, vec![8, 376, 380, 400]);
        assert_eq!(log.frontier(), Some(400));
        assert_eq!(log.get(376), Some(&Payload(376)));
        assert_eq!(log.get(380), Some(&Payload(380)));
    }

    #[test]
    #[should_panic(expected = "duplicate action_count")]
    fn insert_sorted_panics_on_exact_dup() {
        // Distinct deltas never share an ac; an exact-dup is a genuine desync.
        let mut log = ActionLog::new();
        log.insert_sorted(7, Payload(1));
        log.insert_sorted(7, Payload(2));
    }

    #[test]
    fn rewind_then_replay_returns_identical_entries() {
        // Simulated rewind: driver advances to ac=12, then rewinds (via
        // the engine's undo_log) to before any of these entries, then
        // replays. The log MUST yield bit-identical entries on the replay
        // pass — this is the property that makes the snapshot/resume
        // mechanism (ai_docs/reference/snapshot_architecture.md) work over
        // network inputs without re-fetching from the server.
        let mut log = ActionLog::new();
        log.push(3, Payload(3));
        log.push(6, Payload(6));
        log.push(9, Payload(9));
        log.push(12, Payload(12));

        let forward: Vec<Payload> = [3u64, 6, 9, 12]
            .iter()
            .map(|&ac| log.get(ac).cloned().unwrap())
            .collect();
        let replay: Vec<Payload> = [3u64, 6, 9, 12]
            .iter()
            .map(|&ac| log.get(ac).cloned().unwrap())
            .collect();
        assert_eq!(forward, replay);
        assert_eq!(log.len(), 4);
    }

    #[test]
    fn iter_yields_entries_in_action_count_order() {
        let mut log = ActionLog::new();
        log.push(1, Payload(1));
        log.push(4, Payload(4));
        log.push(10, Payload(10));

        let acs: Vec<u64> = log.iter().map(|(ac, _)| ac).collect();
        assert_eq!(acs, vec![1, 4, 10]);
    }

    #[test]
    fn frontier_unchanged_by_failed_lookups() {
        // Invariant #4: frontier() reflects appender state only, never
        // reader state.
        let mut log = ActionLog::new();
        log.push(2, Payload(20));
        let f_before = log.frontier();
        let _ = log.get(100);
        let _ = log.get(0);
        let _ = log.get(2);
        assert_eq!(log.frontier(), f_before);
    }

    #[test]
    fn sparse_action_counts_resolve_correctly() {
        // Real workloads have sparse action_counts (only Reveal /
        // Reorder / Choice slots are populated; most action_counts have
        // no entry in any given log). Binary search must still find the
        // entries that ARE present and report None for the gaps.
        let mut log = ActionLog::new();
        log.push(1, Payload(1));
        log.push(1000, Payload(2));
        log.push(1_000_000, Payload(3));

        assert_eq!(log.get(1), Some(&Payload(1)));
        assert_eq!(log.get(1000), Some(&Payload(2)));
        assert_eq!(log.get(1_000_000), Some(&Payload(3)));
        assert_eq!(log.get(500), None);
        assert_eq!(log.get(999_999), None);
        assert_eq!(log.frontier(), Some(1_000_000));
    }

    #[test]
    fn generic_over_arbitrary_payload_types() {
        // Smoke test: the primitive really is payload-agnostic.
        // A controller using `String` for its choice description and a
        // state-sync stream using a struct must both compile and behave.
        let mut strings: ActionLog<String> = ActionLog::new();
        strings.push(1, "first".to_string());
        strings.push(2, "second".to_string());
        assert_eq!(strings.get(1).map(String::as_str), Some("first"));

        #[derive(Debug, Clone, PartialEq, Eq)]
        struct StateSync {
            player: u32,
            order: Vec<u32>,
        }
        let mut syncs: ActionLog<StateSync> = ActionLog::new();
        syncs.push(
            10,
            StateSync {
                player: 0,
                order: vec![1, 2, 3],
            },
        );
        assert_eq!(syncs.get(10).unwrap().order, vec![1, 2, 3]);
    }
}
