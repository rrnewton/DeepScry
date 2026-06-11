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
//! - [`life`] — GainLife / GainLifeDynamic / LoseLife / SetLife / DrainMana.
//! - [`cards`] — DrawCards / Mill / Scry / Surveil (card-flow).
//! - [`counters`] — PutCounter / PutCounterAll / MultiplyCounter / Proliferate /
//!   RemoveCounter.
//! - [`tapping`] — TapPermanent / UntapPermanent / TapOrUntapPermanent /
//!   TapAll / UntapAll.
//! - [`pump`] — PumpCreature / PumpCreatureVariable / DebuffCreature /
//!   PumpAllCreatures / AnimateAll (stat modification).
//! - [`misc`] — AddMana / ChooseColor / AddTurn / AddPhase / ClearRemembered /
//!   Clone (routing guard) / Unimplemented / NoOp.
//! - [`stack`] — CounterSpell / ConditionalSelfCounter / ModalChoice (routing
//!   guard) / ImmediateTrigger.
//! - [`tokens`] — CreateToken / CopyPermanent / CopySpellAbility.
//! - [`control`] — GainControl / Fight / GrantCantBeBlocked / Regenerate /
//!   AttachEquipment.
//! - [`zones`] — Dig + single-target removal/self-move (DestroyPermanent,
//!   ExilePermanent, ExileIfWouldDieThisTurn, SelfExileFromStack,
//!   MoveSelfBetweenZones); more zone-movement effects to follow.
//! - [`bending`] — Airbend / Earthbend / Firebend (Avatar set mechanics).

mod bending;
mod cards;
mod control;
mod counters;
mod damage;
mod life;
mod misc;
mod pump;
mod stack;
mod tapping;
mod tokens;
mod zones;
