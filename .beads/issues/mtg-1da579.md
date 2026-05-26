---
title: 'BUG: native_game.html card sizing — feedback loop, wrong resolution, no spread (FIXED)'
status: closed
priority: 2
issue_type: bug
created_at: 2026-05-13T02:41:46.780615141+00:00
updated_at: 2026-05-13T02:41:52.442649729+00:00
---

# Description

**FIXED in commit TBD on layout-engine.**

## Original report

Multiple issues with native_game.html card sizing on layout-engine branch:
1. BOTTOM BATTLEFIELD: Cards small, didn't fill the space
2. TOP BATTLEFIELD: Cards bigger but didn't spread out like tui_game.html
3. STAGES BUG: Cards resized incrementally on each click/keypress
4. WRONG RESOLUTION: Low-res images upscaled, looked blurry

## Root causes

**Bug 3 (stages) → Bugs 1 & 2:** `applyBattlefieldCardSizes` measured
`.card-grid` (whose height is content-driven by flex content) instead
of `.pane-body` (the stable available area driven by the outer CSS
grid). This created a contracting feedback loop:

  tall grid → engine picks big cards → cards rendered → grid shrinks
  to fit smaller cards → next render measures shorter grid → engine
  picks smaller cards → repeat forever.

After 2-3 interactions cards were stuck at min_card (50x80) regardless
of how much pane height was actually available.

**Bug 4 (resolution):** `createCardElement` hard-coded `106` as the
image height passed to `getCardImageFallbackUrls(card.name, 106)`,
which made `tui_get_image_urls` ALWAYS return the 'small' (146x204)
URL — even for battlefield cards 250+ px tall.

**Bug 1/2 (spread):** `.bf-section-cards` had no `justify-content`,
so cards left-aligned within each section instead of spreading
across the available width like tui_game.html.

## Fix (web/native_game.html)

- New `computeBattlefieldCardSizes()` that measures `.pane-body`
  (stable) instead of `.card-grid` (content-driven). Caches result
  in `_battlefieldCardSizes`.
- `applyBattlefieldCardSizes(sizes)` is now a pure CSS-write step.
- `updateUI()` runs `computeBattlefieldCardSizes()` BEFORE
  `renderBattlefield`, then passes per-pane size into
  `renderBattlefield(player, containerId, paneSize)` →
  `createCardElement(card, showMana, imgHeightPx)`. The image
  request gets the actual rendered card height, so
  `tui_get_image_urls` returns 'normal' (488x680) for big cards.
- `.bf-section-cards { justify-content: center; }` — cards spread
  horizontally inside each section.
- `.card-grid` switched to `flex-direction: column; min-height: 100%`
  so it fills the pane-body and breaks the size-feedback loop.

## New e2e test: web/test_card_size_stability.js (in `make validate`)

Asserts:
- Cards bigger than min_card.
- justify-content is centered/spread.
- Card height fills ≥30% of pane height.
- Battlefield <img> uses '/normal/' folder when card height > 204px.
- `--card-w` / `--card-h` STABLE across 6 ArrowDown/Up interactions
  (the feedback-loop guard).

After fix: 10/10 PASS. Cards 173x243px (filling 92% of 263px pane),
stable across 6 interactions.
