# `heuristic_controller/` — Heuristic AI controller

The `HeuristicController` is the project's primary AI: a faithful port of Java
Forge's heuristic AI (`forge-ai/src/main/java/forge/ai/`). It makes decisions
with evaluation heuristics rather than simulation or Monte Carlo search.

This directory is the result of splitting the former 10,110-line
`heuristic_controller.rs` into focused submodules. The split is **purely
structural** — no decision logic changed, verified by diffing seeded
heuristic-vs-heuristic game logs before and after (see the tracking issue and
the refactor commit message).

## Why it is split the way it is

Submodules are organized by the **shape of the decision the game asks the
controller to make**, mirroring how the engine presents choices. The single
public entrypoint surface is the `PlayerController` trait impl in `mod.rs`; each
trait method dispatches into the helper methods that live in the topic
submodule. All helper methods are still inherent methods on
`HeuristicController` (added via per-file `impl HeuristicController { … }`
blocks), so there is one controller type with one piece of state — only the
*source layout* is modularized.

Cross-submodule helper calls are common (e.g. almost everything calls
`evaluate_creature`), so the relocated helpers are `pub(crate)` to be reachable
across the sibling modules. `evaluate_creature` is fully `pub` because the
`creature_evaluation_test` integration-test crate and `game_state_evaluator`
call it as public API.

## Submodule map

| File | Responsibility | Key methods |
| --- | --- | --- |
| `mod.rs` | Type definition, constructors, shared helper types (`CombatFactors`, `CombatOutcome`, `ActivatedAbilityType`), and the `PlayerController` **trait impl** — the decision entrypoints the engine calls, which dispatch into the submodules. | `new`, `with_seed`, `set_aggression`, `choose_spell_ability_to_play`, `choose_targets`, `choose_attackers`, `choose_blockers`, `choose_modes`, `choose_from_options`, … |
| `creature_eval.rs` | Creature / card valuation. | `evaluate_creature`, `evaluate_creature_impl`, `evaluate_creature_for_casting`, `evaluate_card_definition_for_library`, `get_best_creature`, `get_worst_creature` |
| `mana_lands.rs` | Mana counting, land selection, mana-source scoring. | `count_available_mana`, `should_play_land`, `is_safe_to_hold_land_for_main2`, `choose_best_land`, `score_mana_source`, `can_mana_creature_attack` |
| `spell_selection.rs` | The master "what to play this priority" dispatcher. | `choose_best_spell` |
| `combat.rs` | Attack evaluation, block assignment, and combat math. | `calculate_combat_factors`, `can_block*`, `should_attack*`, `predict_combat_outcome`, `should_block`, `find_gang_block`, `assign_blocks_phase{1,2,3}`, `reinforce_blockers_*`, `life_in_danger` |
| `spell_eval.rs` | Should-cast evaluation for pumps, removal, and the `should_cast_*` family. | `should_cast_pump`, `should_cast_spell`, `should_cast_board_wipe`, `should_counter_spell`, `choose_best_removal_target`, `use_removal_now`, … |
| `abilities.rs` | Activated-ability classification, scoring, and activation timing. | `should_activate_ability`, `classify_activated_ability`, `should_activate_pump_during_combat`, `has_valuable_ping_target`, `has_valuable_destroy_target` |
| `tests.rs` | The `#[cfg(test)]` unit-test module (uses `super::*`). | — |

## Determinism invariant (read before editing decisions)

Controllers must be **information-independent and deterministic**
(`docs/NETWORK_ARCHITECTURE.md`): the same master seed must yield the same
choice stream on the server (full state) and on a shadow client. Never branch
on hidden information (opponent hand, library order) or on raw RNG state outside
the seeded `rng` field.

Known latent hazard tracked under **mtg-77**: `choose_modes` and
`choose_from_options` in `mod.rs` currently decide by substring-matching
human-readable description strings. If a description ever interpolates runtime
state this becomes a desync source. Both sites carry an in-code
`TODO(mtg-77)`; the structured-evaluation fix is a behavior change and must land
in its own evidence-backed commit, not in a pure refactor.

## Follow-up cleanups

- Break the three longest functions into named helpers:
  `evaluate_creature_impl` (~552 lines), `should_cast_pump` (~395),
  `should_cast_spell` (~307).
- `tests.rs` is ~3,558 lines and could be split by decision topic.
