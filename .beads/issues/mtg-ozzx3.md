---
title: 'Card Compatibility: Fireball'
status: open
priority: 3
issue_type: task
created_at: 2026-05-28T02:03:18.207678778+00:00
updated_at: 2026-05-28T02:03:18.207678778+00:00
---

# Description

Test all behavioral aspects of Fireball in MTG Forge-rs for the 1994 Old School playtest goal.

Card file: cardsfolder/f/fireball.txt

Decks containing this card (1994 Old School playtest):
- 03_robots_jesseisbak (mtg-6ocwz)
- 06_jeskai_aggro_joseantonioprieto (mtg-c86kp)
- 06_troll_disk_daniellebrunazzo (mtg-8muv6)

This is a SKELETON issue filed by the playtest-old-school-1994-skeleton orchestrator on 2026-05-27_#2334(496405da).
Per-aspect verification is the responsibility of the playtester agents.
Follow the per-card workflow in .claude/skills/compatibility_tracking/SKILL.md.

== Aspect checklist ==
For each printed ability/aspect, classify as one of:
  [ ] not-tested
  [WORKING]  — game-log evidence captured
  [PARTIAL]  — primary mode works, secondary mode broken/missing
  [BROKEN]   — cannot cast / crashes / wrong state / silent drop

Static / shape:
- [ ] Parses with correct mana cost
- [ ] Parses with correct P/T (if creature)
- [ ] Parses with correct types/subtypes/supertypes
- [ ] Parses with correct color identity
- [ ] All printed keyword abilities (K:) parse and register (Flying, First Strike, Protection, Regenerate, Banding, ...)
- [ ] Card image and oracle text present in cardsfolder (LEA/ARN/ATQ/LEG/DRK/FEM/HML/ICE printing where appropriate)

Casting / costs:
- [ ] Can be cast for its printed mana cost from hand at legal timing
- [ ] Mana cost (including {X}, hybrid, phyrexian if any) interprets correctly
- [ ] Additional costs (sacrifice, discard, life payment, counter removal) interpret and pay correctly
- [ ] Targeting at cast time (ValidTgts$) accepts/rejects correct objects
- [ ] Stack interactions: spell can be countered, resolves at correct point

Resolution / one-shot effects:
- [ ] Each printed effect resolves with correct game state change
- [ ] Each printed effect produces a correct, non-sentinel game log line
- [ ] Choices on resolution (modes, target choice, X, optional clauses) prompt and resolve

Continuous / static abilities (S:):
- [ ] Each S: line registers and applies in the correct zone
- [ ] Conditional qualifiers (IsPresent$, Threshold$, Affected$) work

Triggered abilities (T:):
- [ ] Each T: line fires on the correct event in the correct zone
- [ ] Trigger zones (TriggerZones$) correct
- [ ] Trigger payload (Execute$) resolves correctly

Activated abilities (A:):
- [ ] Each A: line is offered as a legal activation at legal timing
- [ ] Activation cost (mana / tap / sacrifice / counter / etc.) interprets and pays
- [ ] Effect resolves with correct log line

Replacement effects (R:):
- [ ] Each R: line intercepts the correct event

Mana production (if applicable):
- [ ] Produces the correct colors (Produced$ enum, not collapsed to colorless)
- [ ] "Pay N life" / "deals 1 damage" / "any color" riders work

Zone / phase / timing:
- [ ] Moves between zones (hand→stack→battlefield→graveyard/exile) cleanly
- [ ] Phase-based effects (T:Mode$ Phase, At end of turn, At beginning of upkeep) fire
- [ ] Effects with duration (until end of turn, until your next turn) expire correctly

Interactions:
- [ ] At least one inter-card interaction verified (e.g. counterspell, swords-to-plowshares, animate-dead+target)
- [ ] Mana from this card pays for a relevant spell elsewhere in deck

== Reproducer template ==
./target/release/mtg tui --p1-draw 'Fireball' --p1=heuristic --p2=random --seed 42 --verbosity 3 --json

== Status ==
CARD STATUS: not-tested (skeleton placeholder)

Related: 1994 Old School playtest umbrella (filed in same commit)
