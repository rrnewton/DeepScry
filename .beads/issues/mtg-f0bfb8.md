---
title: 'Bug: ChangesZone trigger parser ignores ValidCard$ Creature.DamagedBy and other Creature.X patterns'
status: open
priority: 3
issue_type: bug
created_at: 2026-05-13T03:00:17.754791218+00:00
updated_at: 2026-05-13T03:00:17.754791218+00:00
---

# Description

The ChangesZone (Battlefield → Graveyard) trigger parser at
loader/card.rs:1874 only registers triggers when ValidCard\$ matches
exactly 'Card.Self' (line 1877) or 'Card.EquippedBy' (line 1901,
separate branch).

This silently drops triggers from a large class of cards whose 'dies'
trigger uses 'Creature.X' filters, including:

- Sengir Vampire (Creature.DamagedBy) — see mtg-7a1f62
- Cards with 'Creature.Other' (when another creature dies)
- Cards with 'Creature.YouCtrl' (when a creature you control dies)
- Cards with 'Creature.OppCtrl' (when an opponent's creature dies)
- Cards with type-specific filters (Creature.Goblin, Creature.Vampire, ...)

For 'Creature.DamagedBy' specifically, even with parser support the
engine has no infrastructure to track which sources have damaged which
creatures this turn — see TODO at heuristic_controller.rs:8661 :
  'TODO(mtg-147): Implement conditional die triggers with DamagedBy tracking'

Two-part fix needed:

1. Parser-side: Generalise the ChangesZone trigger parser at
   loader/card.rs:1874-1896 to accept any 'Creature.X' filter and
   produce a Trigger with an event that captures both
   (a) Battlefield → Graveyard (creature dies)
   (b) the filter, so check_triggers can evaluate it on dispatch.

2. Engine-side: Add a per-turn 'damaged_by_this_turn' tracking field on
   Card (or on GameState as a sparse map). Populate from
   deal_damage_to_creature calls. Clear at end of turn (CR 514).

Affects multiple compat-suite cards. Filed by compat2 while testing
Sengir Vampire (mtg-7a1f62).
