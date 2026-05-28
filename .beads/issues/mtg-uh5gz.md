---
title: 'Card Compatibility: Copy Artifact'
status: open
priority: 2
issue_type: bug
created_at: 2026-05-28T02:34:11.379257633+00:00
updated_at: 2026-05-28T02:34:11.379257633+00:00
---

# Description

BROKEN: ETB Clone replacement not implemented; no prompt for artifact to copy.

Card script: cardsfolder/c/copy_artifact.txt
```
K:ETBReplacement:Copy:DBCopy:Optional
SVar:DBCopy:DB$ Clone | Choices$ Artifact.Other | AddTypes$ Enchantment | SpellDescription$ You may have CARDNAME enter as a copy of any artifact on the battlefield, except it's an enchantment in addition to its other types.
```

USER BUG REPORT (fix-gameplay-bugs-4pack): "Playing rogerbrand mirror match. Copy artifact casts but doesn't ask me to select a target artifact to copy."

ROOT CAUSE: 
1. `K:ETBReplacement:Copy:DBCopy:Optional` parses to KeywordArgs::ETBReplacement {effect_type, details} but loader/card.rs only WIRES this when effect_type==ChooseColor (Thriving lands). The Copy variant is silently ignored, so Copy Artifact enters as a vanilla 1U Enchantment with no choice prompt.
2. `DB$ Clone` ApiType is not in the ApiType enum at all — no Clone variant in mtg-engine/src/loader/ability_parser.rs's ApiType. No Effect::Clone. SVar:DBCopy is parsed but produces no effect.

Implementation requires:
- Add ApiType::Clone and effect_converter handling
- Add Effect::Clone { source, choices_filter, add_types } variant
- Add Controller::choose_clone_target(&[CardId]) callback (or reuse choose_targets with appropriate filter)
- Wire ETBReplacement:Copy in card.rs / state.rs to trigger the choice on ETB
- Handle 'Optional' ("You may...") — a yes/no prompt
- Card needs an AsCopyOf field tracking which artifact it copies (and the AddTypes mod for the enchantment supertype)
- Layer system: Clone copies copiable values per CR 707, then adds Enchantment supertype

Out of scope for fix-gameplay-bugs-4pack (4-pack target was small fixes; Clone is a 200+ LOC feature). Recommend filing as priority-2 standalone for old-school playtest readiness. Affects: Copy Artifact, Clone, Vesuvan Doppelganger, Sakashima the Impostor, mirror entities.
