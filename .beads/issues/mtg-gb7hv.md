---
title: 'Clone CR-707 edges: counters/CDA/copy-of-a-copy layering'
status: open
priority: 3
issue_type: bug
created_at: 2026-05-28T06:12:38.271435904+00:00
updated_at: 2026-05-28T06:12:38.271435904+00:00
---

# Description

Follow-up from mtg-uh5gz (Copy Artifact / Clone mechanic).

GameState::apply_clone (mtg-engine/src/game/actions/mod.rs) implements CR 707.2 by
re-instantiating the chosen permanent's printed CardDefinition and transplanting the
copiable values onto the cloning permanent, then layering AddTypes$ on top.

Deferred edges not yet covered (none affect Copy Artifact's old-school use, which copies
vanilla artifacts):

1. Copy-of-a-copy / nested copy effects (CR 707.2): if the chosen permanent is ITSELF a
   copy, apply_clone copies its printed definition rather than its already-modified copiable
   values. Correct CR 707.2 behaviour copies the target's CURRENT copiable values.
2. CDAs that set P/T/color from game state: copied as their printed form, not re-evaluated.
3. Counters / status: intentionally NOT copied (correct per CR 707.2); noted so the
   intentional omission is not mistaken for a bug.

Fix direction: build apply_clone's template from the target's LIVE copiable characteristics
rather than definition.instantiate(), once a copiable-values snapshot helper exists.
