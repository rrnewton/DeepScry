---
title: 'Bug: bare CopySpellAbility self-copied forever (commander OOM)'
status: open
priority: 2
issue_type: bug
created_at: 2026-06-09T19:54:34.415473100+00:00
updated_at: 2026-06-09T19:54:34.415473100+00:00
---

# Description

Bug: bare CopySpellAbility (no Defined$) self-copied forever — commander-format OOM — FIXED

## Status: FIXED (2026-06-09, claude/fix-commander-loop)

The Chain Lightning spell-copy work (mtg-152) made a BARE `CopySpellAbility`
(no `Defined$`) default to `CopySpellSource::Parent`, so every "copy a TARGET
spell" card (Twincast, Reverberate, Fork, Return the Favor, ~230 cards) copied
ITSELF. The copy carried the same self-copy mode → unbounded self-replication.
In a commander game, "Return the Favor" span ~419,000 copies → one mtg process
at ~40 GB → OOM'd the box (72 GB RAM + 18 GB swap, load 53). CI red on
determ.commander (commander_e2e.sh: 124-timeout / OOM).

## Root cause
effect_converter defaulted absent `Defined$` to Parent. But a bare
CopySpellAbility is the "copy a SEPARATELY-TARGETED spell/ability" mechanic
(carries TargetType$/ValidTgts$ naming a DIFFERENT spell), NOT a parent
self-copy. Self-copying is wrong regardless of who decides.

## Fix (two layers)
1. core/effects.rs: new CopySpellSource::TargetedSpell (now the DEFAULT). Bare
   CopySpellAbility lowers to it → SAFE NO-OP (cloning an arbitrary targeted
   stack object is unimplemented). ONLY explicit Defined$ Parent (Chain
   Lightning) is a real parent self-copy. Converter no longer falls back to
   Parent.
2. state.rs: deterministic anti-OOM backstop in copy_spell_onto_stack — refuse
   once MAX_SPELL_COPIES_ON_STACK (100) spell-copies are on the stack. Pure
   function of stack state → desync-safe; fires only on a runaway loop.

## Verification
- commander_e2e.sh PASSES under an 8G/no-swap cgroup cap (was 124-timeout/OOM):
  all 3 tests + 5 seeds OK. ZERO "copies Return the Favor" lines.
- Chain Lightning's legitimate copy chain unaffected (2 copies, terminates on
  {R}{R}, well under the cap).
- Units: test_bare_copyspellability_is_targeted_spell_not_parent,
  test_copy_spell_onto_stack_is_bounded_against_runaway_loop. lib 986 / puzzle_e2e 80.

## Follow-up
mtg-rpmpg: route the optional copy/retarget decision through the PlayerController
(CR 720 — a player may decline an optional loop). This fix removes the wrong
self-copy and adds a deterministic backstop; the controller-authority parity for
the legitimate optional-copy decision remains tracked there.
