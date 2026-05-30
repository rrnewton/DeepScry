#!/usr/bin/env bash
# seed_salts.sh — THE single bash copy of the per-player seed-derivation
# salts. Source this; do not hand-copy the hex into another script.
#
# These MUST stay byte-identical to the canonical Rust source of truth:
#   mtg-engine/src/game/seed_derivation.rs
#     const P1_SALT: u64 = 0x1234_5678_9ABC_DEF0;
#     const P2_SALT: u64 = 0xFEDC_BA98_7654_3210;
#     derive_player_seed(master, slot) = master.wrapping_add(SALT)
#
# Equivalence between LOCAL (`mtg tui --seed-p1/--seed-p2`, pre-derived) and
# NETWORK (`mtg connect --seed-player`, derives internally) RandomController
# RNG streams depends on these matching exactly. The Rust test
# `seed_derivation::tests::matches_canonical_native_salt_constants` pins the
# Rust values; this file pins the bash mirror. If the Rust salts ever change
# (they are documented "do not change"), update here too — and the
# local-vs-network equivalence legs will catch drift immediately.
#
# bash arithmetic is signed 64-bit; the salts wrap into signed-negative
# values, so we `printf '%u'` to reinterpret the wrapped result as the u64
# bit-pattern Rust produces (an unsigned decimal string).
#
# Usage:
#   source "<repo>/bug_finding/lib/seed_salts.sh"
#   p1=$(derive_p1_seed "$master_seed")
#   p2=$(derive_p2_seed "$master_seed")

# Canonical salts (mirror of seed_derivation.rs — DO NOT diverge).
SEED_P1_SALT=$((0x123456789ABCDEF0))
SEED_P2_SALT=$((0xFEDCBA9876543210))

derive_p1_seed() { printf '%u' $(( $1 + SEED_P1_SALT )); }
derive_p2_seed() { printf '%u' $(( $1 + SEED_P2_SALT )); }
