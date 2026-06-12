---
title: 'Card: Erhnam Djinn — PARTIAL (forestwalk not granted, 1994 WC)'
status: open
priority: 3
issue_type: task
created_at: 2026-06-12T15:24:52.338494129+00:00
updated_at: 2026-06-12T15:24:52.338494129+00:00
---

# Description

## CARD STATUS: PARTIAL

**Card:** Erhnam Djinn
**Decks:** 02_lestree_rg_zoo (4x), 04_defoucaud_zoo (3x)
**Verified:** 2026-06-12_#3139(3b5e4e6ff) — wave-6 sweep

## What works
- 4/5 body, CMC 4 — enters and attacks correctly
- Upkeep trigger fires (T:Mode$ Phase | Phase$ Upkeep)

## What is broken
The upkeep trigger grants Forestwalk to an opponent's creature, but the keyword is NOT
actually granted because the parameterized keyword parsing fails:

1. cardsfolder/e/erhnam_djinn.txt has: `KW$ Landwalk:Forest`
2. In effect_converter.rs ~247, keywords are parsed via `Keyword::from_string("Landwalk:Forest")`
3. `Keyword::from_string()` in keyword_set.rs only handles bare keyword names — it returns `None`
   for parameterized forms like "Landwalk:Forest"
4. Result: `keywords_granted` SmallVec is empty, no keyword is granted, trigger resolves as no-op

Even if the parsing were fixed to yield `Keyword::Landwalk`, the combat rules in combat_rules.rs
use `KeywordArgs::Landwalk { land_type }` to check for forestwalk unblockability. Granting the
bare `Keyword::Landwalk` without a land type would not enforce correct combat rules.

## Root cause
Structural gap: PumpCreature effect uses `Vec<Keyword>` but combat rules need `Vec<KeywordArgs>`.
Two sub-tasks required:
1. Extend `Keyword::from_string()` to handle "Landwalk:Forest" → `Keyword::Landwalk` with land_type
2. Either change PumpCreature to carry `KeywordArgs` or add separate KeywordArgs to `Effect::PumpCreature`
   so the land type is preserved and propagated to `grant_keyword_until_eot` with combat-rules support.

## Gameplay impact
Game still plays correctly without the forestwalk; the opponent creature just doesn't become
unblockable as the rules require. Moderate impact on 1994 WC play fidelity.

## Related
Parent tracker: mtg-709, mtg-713 B23 (log-gap for keyword grants — addressed in wave-6)
