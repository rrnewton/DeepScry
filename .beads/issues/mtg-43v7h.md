---
title: 'GOAL: fully shared (x,y) layout — single Rust layout engine drives identical TUI + web GUI placement, with explicit virtual-pane overfit'
status: open
priority: 2
issue_type: task
created_at: 2026-05-30T20:48:25.797321506+00:00
updated_at: 2026-05-30T20:48:25.797321506+00:00
---

# Description

## Goal
Make the native TUI and the web card GUI render the SAME layout — not just shared sizing + row structure, but identical (x,y) placement of every card and every card STACK — by routing ALL placement through ONE shared Rust layout engine. Backends become "dumb renderers" that draw at engine-provided coordinates.

## Current state (2026-05-30, as-built — verify before changing)
- `mtg-engine/src/game/layout.rs` — shared PANE layout (3-column grid → pane Rects) via a BackendMetrics trait (cell size / inner-area-after-border per backend). SHARED.
- `mtg-engine/src/game/battlefield_layout.rs` — DOES already compute abstract pixel-coordinate `LayoutRect{x,y,w,h}` for cards (word-wrap flow, section headers open a new row, `pick_card_size_for_battlefield`). So an (x,y) engine EXISTS.
- `mtg-engine/src/game/fancy_tui_renderer.rs` — the shared renderer used by BOTH native TUI and the WASM fancy TUI (RatZilla/egui_ratatui). Consumes battlefield_layout. So `tui_game.html` (fancy TUI) and native `mtg tui` DO share placement.
- ⚠️ THE GAP: `web/native_game.html` (the card-style web GUI — the "nice" one with card images + mouse-wheel scroll) does NOT consume battlefield_layout's (x,y) output. It does its OWN placement with CSS flex/flex-wrap (display:flex; flex-wrap:wrap). So the card GUI's battlefield card/stack positions are independently computed by the browser's flexbox, NOT by the Rust engine → it visibly differs from the TUI layout. (native_game.html references battlefield_layout only via test files, not for live placement.)

## What "done" requires (HIGH BAR)
1. **Single source of truth for (x,y):** the shared Rust layout engine computes, for a given pane bounding box, the bounding box of every card AND every card stack (stacks have explicit x/y/offset, not flexbox). Both backends consume these coordinates verbatim.
2. **Backend contract:** backend determines the drawable pane (x,y,w,h) + cell/px metrics → passes to Rust engine → engine returns placements → backend renders at EXACTLY those coordinates. native_game.html must switch from flexbox auto-placement to absolute positioning driven by engine coords (the fancy TUI already does this via the shared renderer).
3. **Identical layout proof:** an automated test that renders the SAME game state in TUI and web card GUI and asserts the card/stack placements match (within snapping tolerance). Screenshots for both, side by side.
4. **Explicit, intentional overfit (virtual pane):** retain the native GUI's ability to SPILL beyond the window + mouse-wheel scroll, but make it DELIBERATE: if the engine's layout for the real pane yields cards below a configured MINIMUM size, re-run layout against a larger VIRTUAL PANE (taller in the vertical dimension), get a nicer layout, render that, and let the backend scroll the overflow. Must be a named, configurable policy (min-card-size threshold → grow virtual height), not an accident of flexbox. Applies consistently; the engine returns "virtual pane size used" so the backend knows the scrollable extent.
5. DRY: zero placement logic duplicated between native_game.html JS and the Rust engine; the JS becomes a thin renderer of engine output. No HACKY per-backend special-casing of positions.
6. Validate green; new layout-equivalence test wired into make validate; mtg-rules-review N/A (pure presentation) but document the backend contract in a module doc + ai_docs.

## Non-goals / keep
- Keep the mouse-wheel scroll UX (it's good) — just drive its scrollable extent from the engine's virtual-pane size.
- Don't regress the fancy TUI (already shares the renderer).

## Pointers
layout.rs, battlefield_layout.rs, fancy_tui_renderer.rs (shared), web/native_game.html (the one to converge onto engine coords), web/tui_game.html (already shared). Tests: web/test_battlefield_layout.js, test_tapped_rotation.js.
