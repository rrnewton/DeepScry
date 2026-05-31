---
title: 'Bug: RestrictValid$ on mana abilities (spend-only-on-X) is unenforced'
status: open
priority: 2
issue_type: task
created_at: 2026-05-31T01:48:24.981642681+00:00
updated_at: 2026-05-31T01:48:24.981642681+00:00
---

# Description

Engine gap: mana abilities with a `RestrictValid$ <filter>` parameter (e.g. Mishra's Workshop `Produced$ C | Amount$ 3 | RestrictValid$ Spell.Artifact` — 'Spend this mana only to cast artifact spells') produce the mana correctly but the spend RESTRICTION is dropped: the mana can currently be spent on any spell.

This makes such sources strictly MORE permissive than printed (CR 106.8 restricted mana). Affects: Mishra's Workshop (mtg-523), Cavern of Souls, Eldrazi Temple-style cards, ritual/filter mana with usage clauses, etc.

Discovered during the wave-16 robots deck sweep (mtg-559). Workshop's primary mana ability is otherwise WORKING (taps for {C}{C}{C}); the robots deck only spends Workshop mana on artifacts so play is unaffected.

FIX DIRECTION: parse `RestrictValid$` into the AddMana effect / mana pool as a tagged 'restricted mana' bucket that the mana-payment engine only draws from when paying for a spell matching the filter. Touches: effect_converter (parse RestrictValid), ManaCost/mana_pool (restricted-mana tag), mana_payment.rs (filter-aware draw). Non-trivial; defer.
