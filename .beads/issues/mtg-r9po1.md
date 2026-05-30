---
title: 'Bug: T:Mode$ DamageDealtOnce + ValidSource$ Card.AttachedBy + TriggerCount$DamageAmount (triggered pseudo-lifelink) unsupported'
status: open
priority: 2
issue_type: bug
created_at: 2026-05-30T23:35:49.101624331+00:00
updated_at: 2026-05-30T23:35:49.101624331+00:00
---

# Description

Engine gap blocking Spirit Link (mtg-544) and any pre-modern 'triggered lifelink' aura/permanent.

Filed: 2026-05-30_#2530(199b91e1), compat-wave13-thedeck.

Card pattern (Spirit Link, cardsfolder/s/spirit_link.txt):
  K:Enchant:Creature
  T:Mode$ DamageDealtOnce | ValidSource$ Card.AttachedBy | Execute$ TrigGain
    | TriggerZones$ Battlefield
    | TriggerDescription$ Whenever enchanted creature deals damage, you gain that much life.
  SVar:TrigGain:DB$ GainLife | Defined$ You | LifeAmount$ X
  SVar:X:TriggerCount$DamageAmount

THREE missing pieces (all in the trigger subsystem):

1. Trigger MODE 'DamageDealtOnce' is not parsed. mtg-engine/src/loader/card.rs
   parse_triggers() handles 'DamageDone' (→ TriggerEvent::DealsCombatDamage) but
   NOT 'DamageDealtOnce'. DamageDealtOnce aggregates all simultaneous damage from
   the source into ONE trigger (lifelink-like, CR 702.15-ish batching). Result:
   the T: line is silently dropped, the aura has no trigger.

2. ValidSource$ Card.AttachedBy is not resolved for trigger firing. The trigger
   lives on the AURA but must fire when the ENCHANTED CREATURE (the card the aura
   is attached to) deals damage. check_triggers() (actions/mod.rs ~6136) only has
   trigger_self_only / requires_other / requires_landfall filters; there is no
   'fires when event_source == the card I'm attached to' filter. Need a new
   trigger flag (e.g. requires_attached_source) + a check that the trigger card's
   attachment target == source_card_id.

3. TriggerCount$DamageAmount is not plumbed. The GainLife amount must equal the
   damage just dealt. TriggerContext (actions/triggers.rs) carries creature_power
   etc. but NOT a damage_amount. The combat firing site
   (actions/combat.rs ~864, where it tracks creature_id→(target,damage_amount) and
   calls check_triggers(DealsCombatDamage, creature_id)) must thread the damage
   amount into a new TriggerContext.damage_amount, and resolve_effect_placeholder
   must fill GainLife { amount } from it (a DynamicAmount::DamageDealt-style hook —
   note DynamicAmount::DamageDealt already exists in core/effects.rs but is only
   reserved for Drain Life, mtg-501). Also non-combat damage (e.g. a pinger
   enchanted) should trigger too; combat is the primary case.

Affected cards (cross-deck lift): Spirit Link (mtg-544; decks 02_thedeck mtg-413,
03_robots mtg-559, 06_troll_disk mtg-562). Other pre-keyword lifelink permanents
share the pattern.

Repro (lifegain does NOT fire today — Player 1 stays at 15):
```sh
## puzzle: p0battlefield=Grizzly Bears|id=10; Spirit Link|id=11|AttachedTo:10  (p0life=15)
./target/release/mtg tui --start-state <puzzle> --p1=fixed --p2=zero \
  --p1-fixed-inputs='attack Grizzly Bears;pass;pass;pass;pass;pass' \
  --stop-on-choice=12 --seed 42 --verbosity 3
```
Observed: 'Grizzly Bears (3) deals 2 damage to Player 2'; NO 'Player 1 gains 2 life' line; P1 life unchanged at 15.

Suggested approach: model as a general triggered-lifelink construct reusing the
existing DynamicAmount::DamageDealt enum + a new TriggerContext.damage_amount and
a requires_attached_source trigger flag, so it lifts all 'gain that much life when
<source> deals damage' cards, not just Spirit Link.
