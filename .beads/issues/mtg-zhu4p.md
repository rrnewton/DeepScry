---
title: 'Web/native GUI: enchantment/aura not rendered on battlefield; no attached-card visual stacking'
status: open
priority: 2
issue_type: bug
created_at: 2026-06-04T00:54:32.321261284+00:00
updated_at: 2026-06-04T00:54:32.321261284+00:00
---

# Description

USER-REPORTED 2026-06-03 (web GUI, mirror julian_spiderman_draft game).

SYMPTOM: Played the enchantment "Friendly Neighborhood". An "Enchantments" section appeared on the player's battlefield but NO card rendered in it (empty section). The enchantment should be attached to a land (enchant-land / aura), but it did not appear on the land either — no visual stacking AND no text note indicating the attachment.

TWO DISTINCT DEFECTS:
1. RENDER DROP: the enchantment card itself is not drawn — the battlefield "Enchantments" section IS created (so the zone/section reaches the layout) but the card render is empty/dropped. Root-cause where the card is lost between the layout data and the GUI draw call (is the card in the section's item list? does it have a name/image the renderer needs? is it filtered out?).
2. ATTACHMENT NOT SURFACED: aura/enchant-land attachment is not shown at all — neither as visual stacking (card overlapping / tucked under the land) nor as a text note. The native GUI appears to have NO visual stacking for attached permanents (auras on lands/creatures, equipment, fortifications). Determine whether the engine even passes the attachment relationship (aura -> attached-to land) into the GUI layout payload; if it does not, that upstream gap is the real fix point.

SCOPE: web GUI (native_game.html) confirmed; verify native GUI too. Part of the broader battlefield-layout-engine review [[design issue]] — this bug is the concrete trigger that motivated it.

ACCEPTANCE: the enchantment renders as a card; the aura->land attachment is visible (stacking and/or an explicit note); reproduced + fixed in a web/UI worktree with validate green. Repro: a game where an enchant-land / aura resolves (Friendly Neighborhood in julian_spiderman_draft).
