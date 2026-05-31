---
title: 'Bug: ApiType::ManaReflected (reflected/filter mana) unimplemented — Fellwar Stone, Wild Cantor-style'
status: open
priority: 2
issue_type: task
created_at: 2026-05-31T01:49:00.352052092+00:00
updated_at: 2026-05-31T01:49:00.352052092+00:00
---

# Description

Engine gap: `AB$ ManaReflected | ColorOrType$ Color | Valid$ Land.OppCtrl | ReflectProperty$ Produce` parses to ApiType::ManaReflected but has NO effect-converter arm, so it resolves as a no-op ('Unimplemented effect ManaReflected resolved as no-op') and produces ZERO mana.

Reflected mana = 'add one mana of any color that a land an opponent controls could produce' (Fellwar Stone). Dynamic, board-state dependent: at activation time, enumerate the colors all lands matching Valid$ could produce, let the controller pick one, add it.

Affects: Fellwar Stone (mtg-504), and any ReflectProperty$ Produce card. Discovered in wave-16 robots sweep (mtg-559).

FIX DIRECTION: add an Effect::AddReflectedMana { valid_filter, reflect_property } resolved at execution time — gather candidate colors from matching permanents' mana_production caches, route a color choice to the controller (NOT engine-chosen — CR 605/106), then add. The static mana_production cache can't express this (it's dynamic), so it needs a runtime enumerate-then-choose path. Touches effect enum + effect_converter + a new resolve arm + targeting (color choice). Non-trivial.
