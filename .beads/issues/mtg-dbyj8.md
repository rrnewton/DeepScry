---
title: 1994 Championship — broken-card root-cause backlog (B1-B18)
status: open
priority: 2
issue_type: task
depends_on:
  mtg-4zlpr: parent-child
created_at: 2026-06-03T03:50:00.970844475+00:00
updated_at: 2026-06-03T03:50:00.970844475+00:00
---

# Description

1994 World Championship compat sweep — full per-card classification + shared
root-cause backlog (mtg-4zlpr umbrella). Compiled by agent compat-1994 from
4 read-only investigation agents driving curated/puzzle reproducers with
verbosity-3 game logs. STAMP below.

== Per-card status (95-card union; the engineering-relevant ones) ==
WORKING (verified this sweep): Clone, Animate Dead, Crumble, Spinal Villain,
  Meekstone (FIXED mtg-tvicv).
WORKING (lock, FIXED): Stasis skip-untap (mtg-f3qdj; PARTIAL overall — upkeep
  self-sac still broken, mtg-xs6pa).
PARTIAL: Vesuvan Doppelganger (ETB copy ok; upkeep re-copy needs AddTriggers$),
  Jade Statue (animate ok; ActivationPhases$ not enforced),
  Berserk (Trample ok; +X/+0 + EOT destroy broken),
  Sylvan Library (trigger fires; draw-2/pay-4 chain unimpl — mtg-548/mtg-651),
  Whirling Dervish (protection ok; EOT +1/+1 counter trigger broken),
  Howling Mine (trigger+IsPresent ok; draws for wrong player).
BROKEN: Old Man of the Sea, Aladdin, Time Elemental, Forcefield, Kismet,
  Ivory Tower, Diamond Valley, Channel, Winter Orb, Magical Hack,
  Sleight of Mind, In the Eye of Chaos, Presence of the Master, Fork,
  Reverse Damage, Siren's Call.
(~44 other union cards already CLOSED=WORKING pre-sweep; ~19 simple cards
 batch-verified WORKING by agent D — basic dorks/lands/vanilla/pump.)

== Shared root causes (engineering backlog; file pointers) ==
B1. get_valid_targets_for_ability has NO GainControl branch (the activated
    sibling of the working spell path). + Effect::GainControl carries no
    ValidTgts restriction; + powerLEX dynamic threshold (X=source power)
    unparsed; + LoseControl$/StaticCommandCheck conditional control-duration
    unimplemented. Cards: Old Man of the Sea (deck01 main x2), Aladdin (SB).
    targeting.rs ~1297; effects.rs GainControl.
B2. AB$ ChangeZone Origin$ Battlefield Destination$ Hand with ValidTgts$
    (targeted bounce) returns None -> ability dropped. Time Elemental.
    effect_converter.rs ChangeZone.
B3. Mode$ Attacks/Blocks trigger parser doesn't handle DB$ DelayedTrigger ->
    empty-effects trigger fires as no-op. Time Elemental (sac+5dmg). card.rs ~2533.
B4. AddTriggers$ unimplemented (add a trigger to a permanent post-copy).
    Vesuvan Doppelganger upkeep re-copy.
B5. AB$ ChooseCard ApiType has no effect_converter arm -> ability dropped.
    Forcefield. effect_converter.rs.
B6. ActivationPhases$ (e.g. BeginCombat->EndCombat) NOT parsed or enforced.
    Jade Statue (combat-only), Siren's Call (Upkeep->BeginCombat OpponentTurn).
B7. ChangeText ApiType unimplemented (resolves as logged no-op). Magical Hack,
    Sleight of Mind. (Hard: text-changing effect.)
B8. SpellCast trigger fires only for permanents where card.controller==caster
    (mod.rs ~7446) -> world/global enchantments never fire on opp casts; AND
    need TriggeredSpellAbility (counter the triggering spell) + TriggeredActivator
    (caster as payer) context. In the Eye of Chaos, Presence of the Master.
B9. Trigger parser Phase$ value "End of Turn"/"End Of Turn" not matched by
    "EndOfTurn"|"End" -> phase trigger silently dropped. card.rs ~2283.
    Whirling Dervish (counter), Berserk (EOT destroy), Siren's Call (destroy
    pacifist). NOTE: these also need IsPresent$ ...dealtDamageToOppThisTurn /
    attackedThisTurn source-condition support (separate gap).
B10. Dynamic LifeAmount$ X in GainLife: triggered path hardcodes amount to 1
     via unwrap_or(1) (Ivory Tower, card.rs ~2358); activated path returns None
     -> ability not offered (Diamond Valley, Sacrificed$CardToughness).
B11. Defined$ TriggeredPlayer for DrawCards resolves to controller, not the
     drawing/active player. Howling Mine. Need triggered_player sentinel +
     drawing_player on the Phase$ Draw trigger context. effect_converter.rs ~124,
     actions/triggers.rs ~211.
B12. Global ETB-tapped replacement R:Event$ Moved | ValidCard$ *.OppCtrl |
     Destination$ Battlefield | ReplaceWith$ ETBTapped unimplemented (only a
     card's OWN enters_tapped honored). Kismet. state.rs ~1150.
B13. AddKeyword$ UntapAdjust:Land:N + per-category untap-count limit in the
     untap step unimplemented (needs a per-player land-untap choice). Winter Orb.
B14. SP$ Effect | Abilities$ <SVar> (grant a temp activated mana ability until
     EOT) unimplemented (handler only reads StaticAbilities$). Channel.
     effect_converter.rs ~963.
B15. SP$ ChooseSource | Choices$ Card,Emblem (non-CoP free-source prevention)
     -> None -> instant not castable. Reverse Damage. effect_converter.rs ~1465.
B16. SP$ Effect | StaticAbilities$ MustAttack unimplemented -> not castable.
     Siren's Call. effect_converter.rs ~974.
B17. CopySpellAbility (SP$ form) doesn't create the copy (existing mtg-152). Fork.
B18. Pump NumAtt$ +X with SVar X=Targeted$CardPower not applied (power-doubling).
     Berserk.

== How to use ==
Pick a Bx item -> it usually unblocks multiple cards -> fix at the layer named,
add parser-shape unit + e2e puzzle regression (see test_puzzles/meekstone_* and
stasis_* as templates), rules-review, update the per-card issue + EFFECT_SUPPORT.
Reproducer commands per card are in this sweep's agent reports (re-derivable via
curated --p1-draw / puzzle + --p1=fixed --verbosity 3).

STAMP: 2026-06-02_#2676(4fb27bff)
