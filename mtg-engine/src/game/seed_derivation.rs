//! Centralized RNG seed derivation for player controllers.
//!
//! All callers that turn a single master seed into per-player controller
//! seeds MUST go through this helper. This guarantees that every execution
//! mode (single-process native, network server/client, stop-and-go
//! snapshot/resume, WASM single-process, WASM network) produces identical
//! controller RNG streams from the same master seed.
//!
//! # Background
//!
//! Historically the codebase had three different seed-derivation schemes:
//!   * native `main.rs` used `master + 0x1234_5678_9ABC_DEF0` for P1 and
//!     `master + 0xFEDC_BA98_7654_3210` for P2;
//!   * `wasm/mod.rs` used `master` for P1 and `master + 1` for P2;
//!   * `wasm/network/ai_harness.rs` used whatever `u32` JS callers passed.
//!
//! These schemes silently disagreed: a `--seed=42` native run and a
//! `--seed=42` WASM run produced totally different RandomController
//! choice streams, which guaranteed cross-mode divergence and was a
//! latent desync bug for network mode (see `docs/NETWORK_ARCHITECTURE.md`:
//! "Desync is ALWAYS Fatal").
//!
//! All those callsites now route through [`derive_player_seed`] so the
//! native salt scheme is the single source of truth.

/// Logical player slot for seed derivation.
///
/// We use a typed enum instead of `u32`/`usize` so callers can never
/// accidentally pass an out-of-range index, and so the slot→salt mapping
/// stays exhaustively covered by the `match` in [`derive_player_seed`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerSlot {
    /// First seat (player index 0). Often called "P1".
    P1,
    /// Second seat (player index 1). Often called "P2".
    P2,
}

impl PlayerSlot {
    /// Construct a [`PlayerSlot`] from a 0-based player index.
    ///
    /// Returns `None` for indices outside the supported range. Two-player
    /// is the only currently supported configuration; if/when multiplayer
    /// is added this enum and [`derive_player_seed`] both need to grow.
    pub fn from_index(idx: usize) -> Option<Self> {
        match idx {
            0 => Some(PlayerSlot::P1),
            1 => Some(PlayerSlot::P2),
            _ => None,
        }
    }
}

/// P1 controller seed salt. The two salts are chosen to have very
/// different bit patterns so neighbouring master seeds produce far-apart
/// per-player seeds.
///
/// **Do not change.** Changing these values invalidates every recorded
/// game seed (replays, regression tests, snapshot fixtures) — they are a
/// stable part of the public seed→game contract.
const P1_SALT: u64 = 0x1234_5678_9ABC_DEF0;

/// P2 controller seed salt. Bit-inverted-ish counterpart to [`P1_SALT`].
/// Same stability guarantee — do not change.
const P2_SALT: u64 = 0xFEDC_BA98_7654_3210;

/// Derive a per-player controller seed from a master seed and player slot.
///
/// This is the single canonical function for going `master → per-player`.
/// Every controller construction site (native CLI, WASM bridge, network
/// AI harness, snapshot restore) MUST use it. Direct `wrapping_add` of a
/// hardcoded constant is a bug.
///
/// # Properties
///
/// * Pure function — no I/O, no global state, no entropy.
/// * Same `(master, slot)` always returns the same seed across binaries
///   and across architectures (it is just a `wrapping_add`).
/// * Independent of how many other slots exist or are seeded — overriding
///   the P1 seed via `--seed-p1` does not perturb the P2 derivation.
pub const fn derive_player_seed(master_seed: u64, slot: PlayerSlot) -> u64 {
    match slot {
        PlayerSlot::P1 => master_seed.wrapping_add(P1_SALT),
        PlayerSlot::P2 => master_seed.wrapping_add(P2_SALT),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p1_and_p2_seeds_differ() {
        let master = 42;
        let p1 = derive_player_seed(master, PlayerSlot::P1);
        let p2 = derive_player_seed(master, PlayerSlot::P2);
        assert_ne!(p1, p2, "P1 and P2 must derive distinct seeds");
    }

    #[test]
    fn deterministic_across_calls() {
        // Re-deriving the same (master, slot) always yields the same value.
        for master in [0u64, 1, 42, 0xDEAD_BEEF, u64::MAX] {
            for slot in [PlayerSlot::P1, PlayerSlot::P2] {
                let a = derive_player_seed(master, slot);
                let b = derive_player_seed(master, slot);
                assert_eq!(a, b, "derive_player_seed must be a pure function");
            }
        }
    }

    #[test]
    fn from_index_round_trip() {
        assert_eq!(PlayerSlot::from_index(0), Some(PlayerSlot::P1));
        assert_eq!(PlayerSlot::from_index(1), Some(PlayerSlot::P2));
        assert_eq!(PlayerSlot::from_index(2), None);
    }

    #[test]
    fn matches_canonical_native_salt_constants() {
        // Lock in the exact native-CLI behaviour as of 2026-05-15. These
        // constants MUST NOT change — they're part of the seed→game
        // reproducibility contract that recorded test seeds depend on.
        assert_eq!(derive_player_seed(0, PlayerSlot::P1), 0x1234_5678_9ABC_DEF0);
        assert_eq!(derive_player_seed(0, PlayerSlot::P2), 0xFEDC_BA98_7654_3210);
        assert_eq!(
            derive_player_seed(42, PlayerSlot::P1),
            42u64.wrapping_add(0x1234_5678_9ABC_DEF0)
        );
        assert_eq!(
            derive_player_seed(42, PlayerSlot::P2),
            42u64.wrapping_add(0xFEDC_BA98_7654_3210)
        );
    }
}
