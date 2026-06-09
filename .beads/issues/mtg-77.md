---
title: Heuristic AI completeness tracking
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2026-06-09T15:32:28.178656419+00:00
---

# Description

## Heuristic AI completeness tracking

GARDENING (2026-06-01): this epic has an empty description. It appears to have been a tracking issue for heuristic AI improvements, but the content was lost or never populated.

Current heuristic AI state (from observation):
- The HeuristicController (mtg-engine/src/game/heuristic_controller.rs) is the primary AI
- It handles: spell casting decisions, creature attacks/blocks, mana payment, discard choices, ETB triggers
- Upkeep cost penalties were added (for Juzam Djinn-style upkeep triggers)
- Land play, creature pump, burn targeting all functional

GARDENING: if there are specific missing heuristic AI behaviors, they should be tracked as separate issues rather than under this empty epic.

---

## Determinism hazard: description-substring decisions (flagged 2026-06-09)

Two PlayerController methods decide by substring-matching human-readable
description strings, a latent information-independence/desync hazard under
docs/NETWORK_ARCHITECTURE.md (if any description ever interpolates runtime or
hidden state, the server and shadow client could score identically-presented
options differently and pick different indices -> desync):

- mtg-engine/src/game/heuristic_controller/mod.rs choose_modes(): branches on
  desc.to_lowercase().contains("destroy"/"exile"/"damage"/"counter"/"draw"/"card"/"life"/"+"/"gets").
- mtg-engine/src/game/heuristic_controller/mod.rs choose_from_options(): branches on
  opt.to_lowercase().contains("play"&&"land" / "cast" / "attack"&&!"don't"&&!"no ").

FIX (separate, evidence-backed commit — NOT a pure refactor): decide from the
structured Effect list / option model instead of the rendered string, with a
before/after game-log diff demonstrating the decision change is correct. Both
sites carry an in-code TODO(mtg-77) pointer flagging the hazard.

These sites were relocated as-is (behavior preserved) during the
heuristic_controller.rs -> heuristic_controller/ module split.
