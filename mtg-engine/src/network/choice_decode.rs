//! Shared opponent-choice index decoders (mtg-788 C1).
//!
//! Both the native [`RemoteController`](crate::network::remote_controller) and the
//! WASM `WasmRemoteController` decode the SAME server-sent `choice_indices` into
//! the SAME engine-level selections. Only the *fetch* differs (native blocks on a
//! condvar-backed cursor buffer; WASM polls a non-blocking queue and may yield
//! `NeedInput`); the *decode* — "given these indices and these candidate slices,
//! which CardIds did the opponent pick?" — is byte-identical and MUST stay so, or
//! the two shadows diverge (Desync is ALWAYS Fatal, NETWORK_ARCHITECTURE.md).
//!
//! These are pure functions of `(indices, slices)` with no I/O, no controller
//! state, and no hidden information. Centralising them here removes the
//! duplication the WASM file's old "Code Sharing Note" flagged and guarantees the
//! two controllers can never drift apart on the decode.
//!
//! NOT included (the bodies genuinely differ, so sharing them would be a
//! behaviour change, not a refactor): `choose_damage_assignment_order` (WASM
//! appends the unlisted blockers, native does not), `choose_from_library` (WASM
//! carries an extra server-CardId fallback), and `choose_spell_ability_to_play`
//! (server-ability vs index fallback). Those stay per-controller.

use crate::core::CardId;
use smallvec::SmallVec;

/// Decode a declare-attackers choice.
///
/// Protocol: index `0` = "no attackers / done"; index `N` (1-based) = the
/// creature at `available_creatures[N - 1]`. Out-of-range and the `0` sentinel
/// are skipped.
pub fn decode_attackers(indices: &[usize], available_creatures: &[CardId]) -> SmallVec<[CardId; 8]> {
    indices
        .iter()
        .filter_map(|&idx| {
            if idx > 0 {
                available_creatures.get(idx - 1).copied()
            } else {
                None
            }
        })
        .collect()
}

/// Decode a declare-blockers choice.
///
/// Protocol: index `0` = "no blockers / done"; index `N` (1-based) encodes a
/// `(blocker, attacker)` pair as `pair = N - 1`, `blocker = pair / attackers.len()`,
/// `attacker = pair % attackers.len()`. Out-of-range entries (and a zero
/// `attackers.len()`) are skipped.
pub fn decode_blockers(
    indices: &[usize],
    available_blockers: &[CardId],
    attackers: &[CardId],
) -> SmallVec<[(CardId, CardId); 8]> {
    if attackers.is_empty() {
        return SmallVec::new();
    }
    indices
        .iter()
        .filter_map(|&idx| {
            if idx == 0 {
                return None;
            }
            let pair = idx - 1;
            let blocker = available_blockers.get(pair / attackers.len()).copied()?;
            let attacker = attackers.get(pair % attackers.len()).copied()?;
            Some((blocker, attacker))
        })
        .collect()
}

/// Decode a flat multi-select subset choice (discard / sacrifice / not-untap /
/// mana-source selection): each index is a 0-based position into `slice`.
/// Out-of-range indices are skipped.
///
/// Generic over the caller's `SmallVec` inline size (`A: Array<Item = CardId>`)
/// so each call site keeps its own stack-inline capacity (`[CardId; 7]` for a
/// discard, `[CardId; 8]` for sacrifice/mana, ...) without a heap detour.
pub fn decode_subset<A: smallvec::Array<Item = CardId>>(indices: &[usize], slice: &[CardId]) -> SmallVec<A> {
    indices.iter().filter_map(|&idx| slice.get(idx).copied()).collect()
}

/// Resolve the blocker chosen for (lethal / remaining) combat-damage assignment.
///
/// The server sends BOTH a 1-element `choice_indices` AND the authoritative
/// `target_card_ids` (mtg-418 SMART damage assignment). The CardId is preferred
/// because index-based lookup is unreliable when the shadow's blocker ordering
/// differs from the server's.
///
/// - If a CardId was submitted: it MUST be present in `valid_blockers`. If it is
///   not, the two sides' combat state has diverged — return `Err` (a FATAL
///   desync). The old index fallback MASKED this by silently picking a
///   different, order-dependent blocker, which the rewind vision forbids
///   (mtg-731).
/// - If no CardId was submitted (legacy / no-id peer): fall back to the first
///   index into `valid_blockers`; an out-of-range index is itself a FATAL desync.
///
/// `valid_blockers` is the caller's authoritative candidate list. `context` names
/// the call site ("lethal-damage" / "remaining-damage") for the error message.
/// The returned `Err` string is LOG/DISPLAY only (not hashed), so it is safe for
/// both controllers to share one canonical wording.
///
/// # Errors
///
/// Returns `Err(String)` — a FATAL desync — when a submitted CardId is not in
/// `valid_blockers` (combat-state divergence), or, on the index fallback, when
/// the index is out of range of `valid_blockers`.
pub fn resolve_combat_blocker(
    indices: &[usize],
    target_card_ids: Option<&[CardId]>,
    valid_blockers: &[CardId],
    context: &str,
) -> Result<CardId, String> {
    // Prefer the server-authoritative CardId when present.
    if let Some(&blocker_id) = target_card_ids.and_then(|ids| ids.first()) {
        if valid_blockers.contains(&blocker_id) {
            return Ok(blocker_id);
        }
        return Err(format!(
            "FATAL DESYNC: RemoteController {context} blocker {:?} not in valid_blockers {:?} \
             (combat-state divergence; index fallback removed — mtg-731)",
            blocker_id,
            valid_blockers
                .iter()
                .map(|id| id.as_u32())
                .collect::<SmallVec<[u32; 8]>>(),
        ));
    }

    // Index-based selection ONLY when no CardId was provided.
    let idx = indices.first().copied().unwrap_or(0);
    valid_blockers.get(idx).copied().ok_or_else(|| {
        format!(
            "FATAL DESYNC: RemoteController received invalid {context} blocker index {idx} \
             (only {} valid blockers); client/server state divergence.",
            valid_blockers.len(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u32) -> CardId {
        CardId::new(n)
    }

    #[test]
    fn attackers_one_based_with_zero_pass() {
        let creatures = [id(10), id(11), id(12)];
        assert!(decode_attackers(&[0], &creatures).is_empty());
        let got: Vec<_> = decode_attackers(&[1, 3], &creatures).into_iter().collect();
        assert_eq!(got, vec![id(10), id(12)]);
        // Out-of-range skipped.
        let got: Vec<_> = decode_attackers(&[2, 9], &creatures).into_iter().collect();
        assert_eq!(got, vec![id(11)]);
    }

    #[test]
    fn blockers_pair_encoding() {
        let blockers = [id(20), id(21)];
        let attackers = [id(30), id(31)];
        // pair = idx-1; blocker = pair/2, attacker = pair%2.
        // idx 1 -> pair 0 -> (b0,a0); idx 4 -> pair 3 -> (b1,a1).
        let got: Vec<_> = decode_blockers(&[1, 4], &blockers, &attackers).into_iter().collect();
        assert_eq!(got, vec![(id(20), id(30)), (id(21), id(31))]);
        // 0 = pass.
        assert!(decode_blockers(&[0], &blockers, &attackers).is_empty());
        // No attackers -> empty (no div-by-zero).
        assert!(decode_blockers(&[1], &blockers, &[]).is_empty());
    }

    #[test]
    fn subset_zero_based_skip_out_of_range() {
        let hand = [id(40), id(41), id(42)];
        let got: SmallVec<[CardId; 7]> = decode_subset(&[0, 2, 5], &hand);
        let got: Vec<_> = got.into_iter().collect();
        assert_eq!(got, vec![id(40), id(42)]);
    }

    #[test]
    fn combat_blocker_prefers_cardid() {
        let valid = [id(50), id(51)];
        // CardId present and valid -> used.
        assert_eq!(
            resolve_combat_blocker(&[0], Some(&[id(51)]), &valid, "lethal-damage"),
            Ok(id(51))
        );
        // CardId present but NOT in valid -> fatal.
        assert!(resolve_combat_blocker(&[0], Some(&[id(99)]), &valid, "lethal-damage").is_err());
        // No CardId -> index fallback.
        assert_eq!(
            resolve_combat_blocker(&[1], None, &valid, "remaining-damage"),
            Ok(id(51))
        );
        // No CardId, out-of-range index -> fatal.
        assert!(resolve_combat_blocker(&[9], None, &valid, "remaining-damage").is_err());
    }
}
