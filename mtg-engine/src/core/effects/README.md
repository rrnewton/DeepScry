# `core/effects` — Effect, Trigger, and Ability Types

This module defines the core data types for card effects, triggered abilities,
static abilities, and activated abilities. It is the central vocabulary of the
engine: every spell resolution, trigger firing, and continuous effect lives here.

## Module layout

| File                    | Contents                                                    |
| ----------------------- | ----------------------------------------------------------- |
| `mod.rs`                | Targeting primitives (`TargetRef`, `ControllerRestriction`, `TargetType`, `DigFilter`), count expressions (`DynamicAmount`, `CountExpression`, `CompareCondition`, `CountModifier`), `TargetRestriction`, and the large `Effect` enum + impl |
| `triggers.rs`           | `TriggerEvent`, `CombatDamageTarget`, `ModalMode`, `Trigger` |
| `static_abilities.rs`   | `StaticAbility`, `AffectedSelector`, and all cost-modification supporting types (`CostReductionTarget`, `CostReductionAmount`, `RaisedCost`, `UnlessCost`, `StaticCondition`, `CompareOp`, `ActivationCondition`, `ActivationPhaseWindow`) |
| `activated_ability.rs`  | `AbilityCache`, `ActivatedAbility`                          |

## Design notes

- All types are `Serialize`/`Deserialize` (for snapshot/resume and network sync).
- The `Effect` enum is the resolved, wire-safe form of a spell or ability's
  effect list. Placeholder `CardId` / `PlayerId` values (zero) are filled in
  at cast/activation time by `resolve_effect_target` in `game/actions/mod.rs`.
- `TargetRestriction` encodes the structured result of parsing a `ValidTgts$`
  parameter; it is **never** compared by substring — always by structured fields.
- `StaticAbility` encodes continuous effects (CR 613 layer system). Each variant
  maps to a specific layer and is evaluated by `game/continuous_effects.rs`.
- `Trigger` encodes triggered abilities (CR 603). The many boolean filter fields
  replace the old string-marker approach and are checked at the single firing
  site in `game/actions/triggers.rs`.
- `ActivatedAbility` includes `AbilityCache`, a pre-computed description-string
  analysis that avoids repeated string scanning in the hot path.
