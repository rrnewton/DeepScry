---
title: 'DestroyAll filter coverage: extend beyond ValidCards comma list'
status: open
priority: 3
issue_type: task
created_at: 2026-05-12T14:00:36.237653341+00:00
updated_at: 2026-05-12T14:00:36.237653341+00:00
---

# Description

## Context

Commit 4202c634 added Effect::DestroyAll with comma-separated ValidCards$ filter parsing (e.g. 'Creature,Artifact'). This unblocked Nevinyrral's Disk but does not cover the wider matrix.

## Missing coverage
1. **Restrictions** — DestroyAll with conditions like 'Creature.cmc<=2' (Wrath subtypes), 'Permanent.tapped' (Pestilence-style), 'Creature.attacking'
2. **Indestructible interaction with stacked effects** — equipment-of-equipment interactions when subset destroyed
3. **'Target opponent's' DestroyAll variants** — Plague Wind / Akroma's Vengeance modal
4. **Triggered DestroyAll** — Wrath-on-trigger cards (e.g. Pernicious Deed)
5. **Counters-aware DestroyAll** — 'destroy all creatures with counters'

## Files
- mtg-engine/src/game/effects.rs (DestroyAll implementation)
- mtg-engine/src/loader/card.rs (filter parsing → CardFilterPredicate)

## Verify
Pernicious Deed activation destroys all permanents with cmc<=X. Plague Wind only kills opponent's. Akroma's Vengeance destroys all of one chosen type.
