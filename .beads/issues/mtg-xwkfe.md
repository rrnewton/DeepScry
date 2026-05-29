---
title: 'Bug: ActivatedAbility silently drops IsPresent/PresentCompare activation conditions'
status: open
priority: 2
issue_type: bug
created_at: 2026-05-29T19:55:59.616699495+00:00
updated_at: 2026-05-29T19:55:59.616699495+00:00
---

# Description

Activated abilities (A: lines) that carry an `IsPresent$ ... |
PresentZone$ ... | PresentCompare$ ...` triple meaning "activate only
when condition X holds" have their condition silently dropped by the
parser and the activation-listing path. The action menu offers the
ability unconditionally.

Found while testing Library of Alexandria (mtg-517) — the canonical
example: "{T}: Draw a card. Activate only if you have exactly seven
cards in hand." Script line:

    A:AB\$ Draw | Cost\$ T | PresentZone\$ Hand | IsPresent\$ Card.YouOwn
               | PresentCompare\$ EQ7

Empirically the draw fires regardless of hand size — see mtg-517 for
the gameplay-log reproducer.

== Root cause ==

(1) ActivatedAbility (mtg-engine/src/core/effects.rs:2985) carries no
    `activation_condition` field — it has cost, effects,
    is_mana_ability, sorcery_speed, your_turn_only, exhaust, and a
    description cache. Nothing to hold a presence predicate.

(2) The loader (mtg-engine/src/loader/card.rs) reads IsPresent\$ +
    PresentZone\$ + PresentCompare\$ only inside the S: (static
    abilities) parsing block (around line 3849) and the cost-
    reduction path (around line 3911). The A: parsing path drops
    these parameters.

(3) The effect converter (mtg-engine/src/loader/effect_converter.rs
    ApiType::Draw arm around line 85) reads only NumCards\$ and
    Defined\$; PresentZone\$ / IsPresent\$ / PresentCompare\$ are
    ignored.

(4) The action-listing path (mtg-engine/src/game/game_loop/actions.rs
    around line 940) gates by cost-payability, summoning sickness,
    life cost, sacrifice cost — but has no hook for
    activation-condition predicates.

== Fix sketch ==

(a) Add activation_condition: Option<ActivationCondition> to
    ActivatedAbility, mirroring StaticCondition::ControlsPresent { filter,
    zone, min_count } but with a full CompareCondition (so Library's
    EQ7 is representable — current StaticCondition.min_count is GE-only).
    Reuse the existing CompareCondition enum from core/effects.rs:178.

(b) Loader: in the A: parsing block, parse IsPresent\$ + PresentZone\$ +
    PresentCompare\$ into an ActivationCondition and attach to the
    built ActivatedAbility. Effect converter is unchanged (the
    condition is on the ability, not the effect).

(c) Action-listing: in game_loop/actions.rs, before the
    can_activate-gating block ships the SpellAbility::ActivateAbility,
    evaluate the condition (count_cards_matching_filter() already
    exists for the static-ability path and can be reused) and set
    can_activate = false when it fails.

== Affected cards ==

- Library of Alexandria (mtg-517) — EQ7 hand-size gate.
- Any future card whose A: line uses a presence predicate.

Specifically NOT affected (these already work via other mechanisms):
- Sedge Troll — IsPresent\$ Swamp.YouCtrl is on a STATIC ability (S:
  line), parsed correctly.
- All Hallow's Eve — IsPresent\$ counters_GE1_SCREAM is on a TRIGGER
  (T: line), parsed correctly.

== Verification ==

Once fixed, mtg-517's reproducer (activate Library at hand size 6 or
8) must report "no available action 'activate Library of Alexandria'"
instead of succeeding. Add an e2e shell test
(tests/library_of_alexandria_eq7_gate_e2e.sh) asserting the menu
visibility flips correctly across hand-size transitions.
