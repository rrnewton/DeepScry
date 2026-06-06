---
title: 'Deck Compatibility: 04 Henry Temur Otters (2025 World Championship)'
status: open
priority: 3
issue_type: task
created_at: 2026-06-06T04:37:24.203281852+00:00
updated_at: 2026-06-06T04:37:24.203281852+00:00
---

# Description

Track compatibility of all cards in the 04_henry_temur_otters.dck deck from the 2025 MTG World Championship (Magic World Championship 31, Bellevue WA, 3rd-4th Place semifinalist, Shaun Henry, Temur Otters aggro-combo, Standard/Aetherdrift-era card pool).

Deck file: decks/championship/2025/04_henry_temur_otters.dck
Related: mtg-684 (deck collections tracking)

Main deck cards tested (Temur-unique, 2026-06-06):
- Willowrush Verge — PARTIAL (mtg-amj6k)
- Bushwhack — PARTIAL (mtg-1k647)
- Ral, Crackling Wit — PARTIAL (mtg-qet36)
- Analyze the Pollen — BROKEN (mtg-2c2sl, bug mtg-wgwuo)
- Song of Totentanz — WORKING (mtg-s53ct)
- Badgermole Cub — PARTIAL (mtg-ap2y8)
- Roaring Furnace // Steaming Sauna — BROKEN (mtg-xe2n7; Room mechanic not implemented)
- Thundertrap Trainer — PARTIAL (mtg-gavrg)
- Enduring Vitality — PARTIAL (mtg-p2sa9)
- Valley Floodcaller — PARTIAL (mtg-ufmdg)
- Botanical Sanctum — PARTIAL (mtg-rwehp)
- Stomping Ground — PARTIAL (mtg-8qv9l)
- Breeding Pool — PARTIAL (mtg-7k2ss)

Sideboard cards tested (Temur-unique):
- Frostcliff Siege — BROKEN (mtg-c8m5v; ETBReplacement SiegeChoice not working)
- Pawpatch Formation — PARTIAL (mtg-7paah)
- Pyroclasm — WORKING (mtg-nadrp)
- Iroh's Demonstration — PARTIAL (mtg-z1y13; network log bug mtg-381)
- Annul — BROKEN (mtg-7vmno; type restriction not enforced, bug mtg-h0jqf)
- Disdainful Stroke — BROKEN (mtg-ukpsj; CMC restriction not enforced, bug mtg-h0jqf)
- Torpor Orb — PARTIAL (mtg-bp5vm)
- Negate — PARTIAL/likely BROKEN (mtg-hcp7m; nonCreature restriction, bug mtg-h0jqf)
- Essence Scatter — BROKEN (mtg-593zw; creature restriction not enforced, bug mtg-h0jqf)

End-to-end tourney result (2026-06-06_#3008(50175e06)):
- 5 games Temur vs Izzet: no crashes, deck plays (Song of Totentanz, Enduring Vitality, Badgermole Cub, Thundertrap Trainer, Valley Floodcaller observed)
- Temur win rate 60% (3/5)
- PumpCreature fizzled warnings visible (pre-existing issue)

Key bugs found:
1. mtg-h0jqf: CounterSpell ValidTgts type/CMC restrictions not enforced (high priority 2)
2. mtg-wgwuo: SubAbility condition not gating Analyze the Pollen double-search
3. mtg-xe2n7: Roaring Furnace Room mechanic not implemented
4. mtg-c8m5v: Frostcliff Siege ETBReplacement SiegeChoice not handled

DECK STATUS: PARTIAL — game completes, core strategy (creatures + spells) functional; Room/Siege card mechanics and counter restrictions broken
