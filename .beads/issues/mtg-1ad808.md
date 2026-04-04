---
title: 'Swords to Plowshares: exiles but does not grant life'
status: open
priority: 3
issue_type: task
labels:
- single-card
created_at: 2026-04-03T21:40:51.460607385+00:00
updated_at: 2026-04-03T21:40:51.460607385+00:00
---

# Description

## Swords to Plowshares: exiles creature but does not grant life equal to power

Context:
- Date: 2026-04-03
- Decks: old_school/01_rogue_rogerbrand.dck mirror
- Mode: zero-vs-zero, seed 42, controlled hands

Steps to Reproduce:
1. Run: target/release/mtg tui decks/old_school/01_rogue_rogerbrand.dck --p1 zero --p2 zero --seed 42 --p1-draw "Badlands;Sengir Vampire;Swords to Plowshares;Scrubland;Shivan Dragon;Bayou;Disenchant" --p2-draw "Badlands;Sedge Troll;Animate Dead;Scrubland;Triskelion;Swamp;City of Brass" -v verbose --no-color-logs
2. Observe Turn 7: P1 casts Swords to Plowshares targeting Sedge Troll (2/2)

Expected Behavior:
- Sedge Troll is exiled (works correctly)
- P2 gains 2 life (Sedge Troll's power is 2) — life should go from 20 to 22

Actual Behavior:
- Sedge Troll is exiled correctly
- P2 life remains at 20 — no life gain occurs
- No log line for life gain

Rules Notes:
- MTG CR 608.2h: "Exile target creature. Its controller gains life equal to its power."
- The life gain is part of the spell's resolution, not a separate triggered ability.

Likely Cause:
- The card script for Swords to Plowshares likely defines the exile effect but the SubAbility for life gain is either missing or not followed (same root cause as Bazaar of Baghdad: parse_activated_abilities not following SubAbility chains, or parse_effects not following the chain for this specific card).

Card script location: cardsfolder/s/swords_to_plowshares.txt
