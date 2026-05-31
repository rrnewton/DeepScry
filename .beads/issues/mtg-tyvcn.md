---
title: 'Bug: Fireball multi-target DivideEvenly + RaiseCost incomplete (single-target X-damage OK)'
status: in_progress
priority: 3
issue_type: task
created_at: 2026-05-31T00:22:29.827028377+00:00
updated_at: 2026-05-31T10:33:50.834827158+00:00
---

# Description

Bug: Fireball multi-target DivideEvenly + per-target RaiseCost — STILL PARTIAL (single-target X-burn WORKS; multi-target divide mode unimplemented). Investigated in compat-wave17-xburn.

Card: cardsfolder/f/fireball.txt (mtg-505)
Script:
  S:Mode$ RaiseCost | ValidCard$ Card.Self | Type$ Spell | Amount$ IncreaseCost | Relative$ True | EffectZone$ All
  A:SP$ DealDamage | ValidTgts$ Any | NumDmg$ X | TargetMin$ 0 | TargetMax$ MaxTargets | DivideEvenly$ RoundedDown
  SVar:X:Count$xPaid
  SVar:MaxTargets:SVar$MaxPlayers/Plus.MaxPermanents
  SVar:IncreaseCost:TargetedObjects$Amount/Minus.1

WHAT WORKS NOW (verified seed 42): single-target X-burn. Casting Fireball with N mana => X = N-1 damage to ONE chosen target (creature or player). The wave-17 DealDamageXPaid display fix (mtg-ioesm) also removed the phantom "deals N damage to Player" line that previously appeared when Fireball single-targeted a creature — so single-target Fireball logs are now clean. Reproducer:
```sh
./target/release/mtg tui --start-state /tmp/fireball.pzl --p1=fixed --p2=zero \
  --p1-fixed-inputs='cast Fireball;4;Grizzly Bears' --stop-on-choice=3 --seed 42 --verbosity 3
## => "Grizzly Bears takes 4 damage" / "Fireball deals 4 damage to Grizzly Bears" (no phantom player line)
```

WHAT IS STILL BROKEN — multi-target divide mode. Root cause is a chain of unimplemented features, NOT a small bug:
1. DivideEvenly$ RoundedDown is dropped at conversion (effect_converter ApiType::DealDamage reads only NumDmg). The effect carries no "divide among all chosen targets" marker; resolution consumes exactly ONE target and deals full X.
2. TargetMin$ 0 / TargetMax$ MaxTargets (variable target COUNT) is unimplemented. get_valid_targets_for_spell + the casting target-selection loop assume a single target; FixedScriptController::choose_targets and the other controllers only ever RETURN ONE target. There is no "choose 0..N targets" negotiation, so multi-select is unreachable end-to-end (this also brushes the controller layer, which is fenced off for this wave).
3. S:Mode$ RaiseCost | Relative$ True | Amount$ IncreaseCost where IncreaseCost = TargetedObjects$Amount/Minus.1 (cost grows by {1} per target beyond the first). The cost machinery (game_loop/actions.rs::compute_effective_cost) only handles fixed RaisedCost::Mana(N); it has NO access to the chosen-target COUNT, because cost is computed before/independently of target selection. Wiring target-count into cost computation is a cross-cutting change.

REMAINING ENGINE WORK (each is a sizeable, cross-cutting feature):
(a) Capture DivideEvenly on Effect::DealDamage/DealDamageXPaid + divide X = floor(X/N) among the N chosen targets at resolution (CR 601.2d announce division; the source must deal floor(X/N) to EACH, remainder lost).
(b) Variable target-count selection (TargetMin/TargetMax + SVar-derived max) through get_valid_targets_for_spell AND every Controller::choose_targets impl — touches the fenced controller layer; needs its own wave + network-determinism review.
(c) Target-count-dependent RaiseCost (Relative$ True, TargetedObjects$Amount): compute cost AFTER target selection.

These three are interdependent (you cannot divide among N or charge per-target until you can SELECT N). Recommend a dedicated "variable target count" wave that lands (b) first, then (a)+(c).

CARD STATUS: PARTIAL — single-target X-burn works; the "divide X evenly among any number of targets" mode + per-target {1} cost are unimplemented (multi-part, see above). Kept OPEN.
