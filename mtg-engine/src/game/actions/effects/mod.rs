//! Effect-family handler modules for the `execute_effect` dispatcher.
//!
//! `execute_effect` (in the parent `game/actions/mod.rs`) is a large match over
//! every [`crate::core::Effect`] variant. To keep that function and file
//! readable, the per-variant logic is being decomposed into focused submodules
//! grouped by effect family (damage, life, zones, counters, tokens, control,
//! mana, ...). Each submodule adds `impl GameState` methods named
//! `execute_<effect>`; the dispatcher matches the variant and delegates.
//!
//! This is a behavior-preserving structural refactor: handler bodies are moved
//! verbatim from the original inline match arms and proven byte-identical
//! against a fixed determinism baseline (see `debug/refactor_baseline.sh`).
//!
//! Families landed so far:
//! - [`damage`] — DealDamage / DealDamageDivided / EachDamage / DamageAll /
//!   PreventDamage / PreventDamageFromSource / PreventAllCombatDamageThisTurn.

mod damage;
