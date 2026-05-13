---
title: 'Card Compatibility: Chaos Orb'
status: open
priority: 2
issue_type: task
created_at: 2026-05-13T01:35:05.721026174+00:00
updated_at: 2026-05-13T01:40:44.269365261+00:00
---

# Description

Test all behavioral aspects of Chaos Orb in MTG Forge-rs.

Set: LEA (mtg-3c7c63)
Deck: rogue_rogerbrand (mtg-526f25)
Card script: cardsfolder/b/bazaar_of_baghdad.txt
Oracle: {1}, {T}: If Chaos Orb is on the battlefield, flip Chaos Orb onto the battlefield from a height of at least one foot. If Chaos Orb turns over completely at least once during the flip, destroy all nontoken permanents it touches. Then destroy Chaos Orb.

Test puzzle: test_puzzles/chaos_orb_destroys_target.pzl

Implementation note: FlipOntoBattlefield API is mapped to Effect::DestroyPermanent (mtg-engine/src/loader/effect_converter.rs:1302). The physical flip cannot be simulated digitally so it falls back to 'destroy target nontoken permanent.'

Findings (2026-05-12, compat1):

1. [x] Card loads (parser produces non-empty effect list for activated ability)
2. [x] Card enters battlefield as Artifact
3. [x] Activation cost {1}, {T} pays
4. [partial] FlipOntoBattlefield API resolves — but as 'DestroyPermanent target=CardId(0)' with 'any nontoken' restriction.
5. [BROKEN] DestroyAll subability does NOT destroy opponent's Grizzly Bears. Output shows '-> targeting Chaos Orb (3)' so the targeting picks Chaos Orb ITSELF as the only target, not an opponent permanent.
6. [x] Self-destroy: Chaos Orb DOES go to graveyard ('Chaos Orb (3) goes to graveyard')
7. [partial] Cleanup clears RememberChanged — not yet verified
8. [unverified] Tokens are NOT destroyed (ValidCards Card.IsRemembered+!token)
9. [x] Chaos Orb still destroys itself even when no permanents touched
10. [partial] Game log shows activation + self-destroy, but no flip outcome / touched-permanents listing
11. [unknown] Heuristic AI evaluation

Reproducer: ./target/release/mtg tui --start-state test_puzzles/chaos_orb_destroys_target.pzl --p1=fixed --p2=zero --p1-fixed-inputs='activate Chaos Orb' --stop-on-choice=8 --json --log-tail=120 --seed 42 --verbosity 3

CARD STATUS: PARTIAL — self-destroy works, but no opponent permanent is ever destroyed because the placeholder target (CardId 0) defaults to Chaos Orb itself rather than presenting a target choice. Either:
(a) Effect::DestroyPermanent target should require opponent selection (modify effect_converter.rs FlipOntoBattlefield branch to set restriction.controller=Opponent or to require a TargetChoice prompt), OR
(b) Implement randomized 'touched permanents' simulation that picks 1-N random nontoken permanents on the battlefield from any side. Java's behavior randomly destroys nearby permanents based on where the flip 'lands.'

Recommend filing follow-up bug 'chaos-orb-no-opponent-target' as P3 task referencing this compat issue.
