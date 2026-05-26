---
title: 'BUG: native_game.html tapped cards show 8° tilt instead of 90° rotation (FIXED)'
status: closed
priority: 2
issue_type: bug
created_at: 2026-05-13T04:06:13.302922776+00:00
updated_at: 2026-05-13T04:06:22.489701324+00:00
---

# Description

**FIXED in commit TBD on layout-engine.**

## Original report

`web/native_game.html` showed tapped cards with `transform: rotate(8deg)` — a
faint visual "tilt" hint. `web/tui_game.html` and the native ratatui TUI
both show full 90° rotation (matching the `battlefield_layout` engine,
which already swaps width/height for tapped cards via
`entity_size`: 1.5× wider, 0.6× shorter).

## Fix (web/native_game.html)

Updated `.card.tapped` from:

```css
.card.tapped {
    transform: rotate(8deg);
    opacity: 0.8;
}
```

To:

```css
.card.tapped {
    /* Swap outer width to var(--card-h) so the flex grid reserves the
       landscape footprint that battlefield_layout::entity_size
       already accounts for. The still-portrait inner content rotates
       90° into that landscape box. */
    width: var(--card-h, 130px);
    min-width: var(--card-h, 80px);
    max-width: var(--card-h, 130px);
    transform: rotate(90deg);
    transform-origin: center center;
    opacity: 0.8;
}
```

CSS specificity: `.card.tapped` (0,2,0) ties with `.card-grid .card`
(0,2,0) but appears later in the stylesheet, so wins. Verified.

## New e2e test: web/test_tapped_rotation.js (in `make validate`)

Wired into `validate-wasm-e2e-step`. Asserts:
- At least one `.card.tapped` is visible after 30 turns of heuristic
  vs heuristic (AI almost always attacks).
- Computed `transform` matrix parses to a 90° rotation (not 8°).
- Computed outer width = grid `--card-h` (landscape footprint, was
  `--card-w` pre-fix).
- No browser errors.

## Verification

Pre-fix: 2/4 FAIL (rotation=8° + width=--card-w).
Post-fix: 4/4 PASS (rotation=90°, width=--card-h=80px from --card-h).
Visual confirmation: `web/screenshots/playtest_game1_seed42_final.png`
(refreshed) shows opponent's 5 attacking creatures rotated 90° with
text reading bottom-to-top, matching tui_game.html's tapped visual.
