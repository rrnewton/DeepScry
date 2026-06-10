---
title: 'Card Compatibility: Jade Statue'
status: closed
priority: 3
issue_type: task
depends_on:
  mtg-709: parent-child
created_at: 2026-06-10T20:01:15.609875574+00:00
updated_at: 2026-06-10T20:01:21.007949768+00:00
---

# Description

Card Compatibility (1994 World Championship — Symens B/R/G Zoo, mtg-709). Jade Statue: '{2}: Jade Statue becomes a 3/6 Golem artifact creature until end of combat. Activate only during combat.'

CARD STATUS: WORKING

== Findings (2026-06-10_#3139(3b5e4e6f)) ==
[x] Static/parse: parses as Artifact, ManaCost {4}. WORKING.
[x] Activated ability AB$ Animate Cost$ 2 Power$ 3 Toughness$ 6 Types$ Creature,Artifact,Golem Duration$ UntilEndOfCombat: animates to 3/6 Golem. WORKING (animate path pre-existing).
[x] ActivationPhases$ BeginCombat->EndCombat enforcement (B6): the activated ability is now offered ONLY during the combat phase (BeginCombat..EndCombat inclusive) and rejected in Upkeep/Main1/Main2. CR 602.5 (a timing restriction is part of the ability). Was BROKEN (parsed+enforced nowhere) -> now WORKING.
[N/A] Triggered abilities: none.
[N/A] Targeting: ability targets Self only.

== Fix ==
New core type ActivationPhaseWindow{start,end:Step} (mtg-engine/src/core/effects.rs), parsed from ActivationPhases$ <start>-><end> via Step::from_script_name (mtg-engine/src/game/phase.rs). Stored as ActivatedAbility.activation_phases: Option<ActivationPhaseWindow> (#[serde(default)]). Enforced in push_activatable_abilities (battlefield + graveyard paths, game_loop/actions.rs) alongside the existing sorcery_speed/your_turn_only/activation_condition gates. Reads only turn.current_step (public, deterministically reconstructed on replay) -> rewind-safe, controller-agnostic, no new mutable/per-turn state. Single contiguous-range form only; disjoint multi-range (Upkeep->Main1,Main2->Cleanup 'except combat') deferred to mtg-713 B6 follow-up (parses to None -> ability left unrestricted, not mis-gated). Generalizes to ~150 cardsfolder cards that carry ActivationPhases on an activated ability.

== Regression tests ==
- Unit (parser-shape): effects.rs test_activation_phase_window_parse_jade_statue, test_activation_phase_window_spaced_and_single; loader/card.rs test_parse_jade_statue_activation_phases.
- E2e (puzzle): test_puzzles/jade_statue_combat_only_animate.pzl + puzzle_e2e.rs test_jade_statue_combat_only_animate (asserts ability offered in BeginCombat/DeclareBlockers/EndCombat, rejected in Upkeep/Main1/Main2).

== Reproducer ==
```sh
cargo test -p mtg-engine --features network --test puzzle_e2e test_jade_statue_combat_only_animate
```
Expected: 'test ... ok' + '✓ Jade Statue animate ability is offered only during combat'.
