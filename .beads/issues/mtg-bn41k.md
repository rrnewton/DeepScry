---
title: 'Bug: Drain Life — StoreSVar effect unimplemented, deals damage to None'
status: open
priority: 2
issue_type: bug
created_at: 2026-05-29T16:32:53.422479345+00:00
updated_at: 2026-05-29T16:32:53.422479345+00:00
---

# Description

Drain Life (cardsfolder/d/drain_life.txt) is BROKEN in mtg-forge-rs.

Card: X1B Sorcery. 'Spend only black mana on X. Drain Life deals X damage to any target. You gain life equal to the damage dealt, but not more than the target's life/loyalty/toughness.'

Script relies on a chain of SVars:
  A:SP$ StoreSVar | XColor$ Black | ValidTgts$ Any | SVar$ Limit | ... | SubAbility$ StoreTgtPW
  SVar:DBDamage:DB$ DealDamage | Defined$ Targeted | NumDmg$ X | SubAbility$ DBGainLife
  SVar:DBGainLife:DB$ GainLife | Defined$ You | LifeAmount$ DrainedLifeCard
  SVar:DrainedLifeCard:SVar$Y/LimitMax.Limit  (Y = TotalDamageDoneByThisTurn)

Observed (puzzle: cast Drain Life X=2 at opponent, seed 42):
  Drain Life (3) has unimplemented effect 'StoreSVar'   (x3)
  Drain Life (3) deals X damage to None

Root causes:
1. ApiType StoreSVar is not implemented (silently 'unimplemented effect').
2. The DealDamage SubAbility uses Defined$ Targeted but the target chosen at
   cast time is not threaded into the SubAbility chain, so it resolves to 'None'
   and X is not bound (prints 'X damage' literally).
3. The life-gain cap (min of damage dealt and target life/loyalty/toughness)
   depends on the StoreSVar 'Limit' machinery that is absent.

This is a multi-feature gap (StoreSVar API + SubAbility target threading for
SP$ chains + computed-SVar life cap). Affects Drain Life and likely other
'damage = X, gain that much life capped' cards. Left BROKEN for now; out of
scope for the current Rogerbrand compatibility wave.

Per-card issue: mtg-501 (Card Compatibility: Drain Life).
Found during the 1994 Old School Mono Black Rogerbrand deck pass (mtg-560).
