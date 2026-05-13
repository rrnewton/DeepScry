---
title: 'Bug: ModifyPT static abilities with Affected$ Card.Self silently fail (self-buffing creatures)'
status: open
priority: 2
issue_type: bug
created_at: 2026-05-13T02:45:58.990022277+00:00
updated_at: 2026-05-13T02:45:58.990022277+00:00
---

# Description

Static abilities of the form

  S:Mode\$ Continuous | Affected\$ Card.Self | AddPower\$ N | AddToughness\$ M | ...

are silently dropped at runtime. Affected creatures are never buffed by their own self-static, so they always show their printed P/T. This affects a large class of cards including:

  - Sedge Troll (LEG, 2/2 + +1/+1 with Swamp) — see mtg-010044
  - Anurid Barkripper (Threshold +2/+2)
  - Aerial Engineer (+2/0 + Flying with artifact)
  - Akki Underling (+2/+1 + First Strike with 7+ cards in hand)
  - Akiri, Line-Slinger (+1/0 per artifact you control)
  - Adelbert Steiner (+1/+1 per Equipment you control)
  - Many more cards in cardsfolder/

Two compounding bugs:

## Bug 1: AffectedSelector::Self_ skipped in calculate_modifypt_effects

mtg-engine/src/game/continuous_effects.rs:788

```
AffectedSelector::Self_ => {
    // Equipment affecting itself (not the equipped creature)
    // Skip - not relevant for this creature's P/T
}
```

The comment is wrong about creatures: creatures with Affected\$ Card.Self ModifyPT statics MUST apply the boost to themselves when calculating their own effective P/T. The arm should be:

```
AffectedSelector::Self_ => {
    if creature_id == source_id && source.is_creature() {
        power_bonus += power;
        toughness_bonus += toughness;
    }
    // Equipment with Self_ still skips (Equipment doesn't get +1/+1 itself)
}
```

## Bug 2: ModifyPT struct has no condition field

mtg-engine/src/loader/card.rs:3640

```
abilities.push(StaticAbility::ModifyPT {
    affected: affected.clone(),
    power,
    toughness,
    description: description.clone(),
});
```

The IsPresent\$ X.YouCtrl / Condition\$ Threshold / CheckSVar\$ X / SVarCompare\$ GE7 qualifiers parsed in the surrounding loop are silently dropped — only StaticAbility::GrantKeyword has a `condition` field today. ModifyPT needs the same plumbing so cards like Sedge Troll (Swamp), Anurid Barkripper (Threshold), Akki Underling (7+ cards in hand) honour their conditional buffs.

## Why it currently fails 'silently'

Because Self_ is skipped (Bug 1), the conditional gap (Bug 2) doesn't even surface — the buff was never going to apply. Fix Bug 1 alone and conditional self-buffers like Sedge Troll become *always-on* (worse than always-off, since they ignore the condition). Fix Bug 2 alone and Self_ is still skipped. Both must be fixed together for cards like Sedge Troll to work correctly.

## Test coverage

mtg-engine/src/game/actions/tests/effects.rs::test_card_compat_sedge_troll pins the parser-level shape (ModifyPT exists with the right power/toughness) but does not assert runtime application — gameplay verification (Sedge Troll showing 3/3 with Swamp on board) remains a TODO until the fix lands.

Reproducer: see mtg-010044.

Tracking: filed by compat2 while testing Sedge Troll.
