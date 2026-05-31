---
title: 'Bug: SP$ DealDamage + SubAbility$ Effect chain double-resolves + ReplaceDyingDefined (exile instead) unimplemented'
status: closed
priority: 2
issue_type: task
created_at: 2026-05-31T01:49:00.355729480+00:00
updated_at: 2026-05-31T02:50:45.498875457+00:00
---

# Description

Bug: SP$ DealDamage NumDmg$ X (DealDamageXPaid) display double-resolution + ReplaceDyingDefined (exile instead) — FIXED in compat-wave17-xburn.

Card: cardsfolder/d/disintegrate.txt (mtg-505)
Script:
  A:SP$ DealDamage | ValidTgts$ Any | NumDmg$ X | SubAbility$ DBEffect | ReplaceDyingDefined$ ThisTargetedCard.Creature
  SVar:X:Count$xPaid

ROOT CAUSE (the "double resolution"): NOT a real double-resolution — a DISPLAY-ONLY phantom. NumDmg$ X parses to Effect::DealDamageXPaid (not DealDamage). The actual damage executed correctly ONCE against the chosen creature (deal_damage_to_creature). But the post-resolution display logger in game_loop/priority.rs::resolve_top_spell_from_stack matched ONLY Effect::DealDamage { target: None } to bind the chosen target for logging; the DealDamageXPaid { target: None } variant fell through to a later arm that converted it to DealDamage { target: None } WITHOUT consuming the chosen target. log_effect_execution's TargetRef::None branch then invented a phantom "Disintegrate deals 3 damage to <opponent> (life: N)" line. The opponent's life was NEVER actually changed (verified: P2 stays at 20) — the log line was pure fiction.

SCOPE: this display bug silently affected EVERY X-damage spell (NumDmg$ X / DealDamageXPaid) aimed at a CREATURE — ~700+ cards (Disintegrate, Blaze, Fireball single-target, Mage's Contest, etc.). Cards previously marked WORKING that were only verified at a PLAYER target never showed the bug (player target binds correctly); the phantom line only appears when the chosen target is a permanent. No card's actual game STATE was wrong (display-only), so no WORKING card needs re-classification for correctness — but their gamelogs carried the spurious line.

FIX 1 (priority.rs): added a DealDamageXPaid { target: None } display-mapping arm that consumes targets[target_index] via target_ref_from_chosen_target (mirroring the DealDamage { None } arm), and made the DealDamage display arm set last_resolved_target.

FIX 2 (ReplaceDyingDefined exile-instead-of-dying, CR 614): added Effect::ExileIfWouldDieThisTurn + a per-card Card.exile_if_would_die_this_turn flag honored by GameState::death_destination_for_card (alongside the finality-counter exile-instead rule), cleared at cleanup. parse_effects synthesizes the rider from the tokenized ReplaceDyingDefined$ param (no substring matching) and binds it to the parent DealDamage target via the reuse_previous sentinel. Resolution + execute + display + targeting matches all updated.

Note on CantRegenerate: the DBEffect's NoRegen static is effectively moot for the damage-death path — the engine's check_lethal_damage SBA does not consume regeneration shields (regen is only applied during combat / Destroy), so a lethally-damaged creature dies/exiles regardless. The "can't be regenerated" clause would only matter for an in-combat regen interaction, which a sorcery cannot reach. Minor; not worth a follow-up.

EVIDENCE (mtg tui --verbosity 3, seed 42):
  Grizzly Bears (16) takes 3 damage (total: 3)
  Disintegrate (3) deals 3 damage to Grizzly Bears (16)        <- creature, NOT a phantom Player
  Disintegrate (3): Grizzly Bears (16) will be exiled if it would die this turn
  Grizzly Bears (16) exiled instead of dying                   <- exile, not graveyard
(P2 life stays 20. At a player target Disintegrate still deals real damage: 20 -> 17.)

Reproducer:
```sh
./target/release/mtg tui --start-state test_puzzles/disintegrate_exiles_creature.pzl \
  --p1=fixed --p2=zero --p1-fixed-inputs='cast Disintegrate;3;Grizzly Bears' \
  --stop-on-choice=3 --json --seed 42 --verbosity 3
```
Expected lines:
```
Grizzly Bears (14) takes 3 damage (total: 3)
Disintegrate (3) deals 3 damage to Grizzly Bears (14)
Grizzly Bears (14) exiled instead of dying
```

Unit test: test_card_compat_disintegrate in mtg-engine/src/game/actions/tests/effects.rs
E2E test: tests/disintegrate_exiles_creature_e2e.sh (puzzle: test_puzzles/disintegrate_exiles_creature.pzl)

MTG RULES REVIEW: PASS.
- CR 614.6/616: ReplaceDyingDefined is a self-replacement zone-change effect ("if it would die this turn, exile it instead") set on resolution, duration "this turn" — implemented as a per-card flag cleared at cleanup. Correct.
- CR 120.3 / 704.5g: lethal damage SBA unchanged; only the death destination is redirected. Correct.
- Display fix changes no game semantics (network determinism preserved — controllers and state untouched; only the post-resolution display logger, gated on should_log && !replaying).

CARD STATUS: WORKING — Disintegrate fully functional (X damage to any target, exile-instead-of-dying on lethal).
