---
title: 'agentplay: Multiple UI/choice issues'
status: open
priority: 2
issue_type: task
created_at: 2025-11-04T10:35:47.725904785+00:00
updated_at: 2025-11-04T11:02:08.779114245+00:00
---

# Description

Three agentplay UI/choice issues discovered during testing:

## Issues Found

1. ✅ **FIXED (8208665)**: Double-printing of available actions
   - Menu printed twice when stopping at choice boundaries
   - Occurred when saving snapshot then resuming
   - Fixed by moving `check_stop_conditions()` before menu printing

2. ✅ **FIXED (8208665)**: Missing pass priority option
   - No way to pass priority when activated abilities available
   - Violated MTG Rules 117.3a (players must be able to pass)
   - Fixed by redesigning indexing: pass is ALWAYS index [0]

3. ⏸️ **TODO**: Missing library search/tutor UI
   - SearchLibrary effects auto-pick first matching card
   - Should show player a choice menu for library search
   - Requirements:
     * Sorted alphabetically
     * Deduplicated (one "Forest" entry even if 4 in library)
     * Filtered by card type (e.g., only basic lands for Evolving Wilds)
     * Allow "fail to find" option
   - Affects: Vibrant Cityscape, Evolving Wilds, all tutor effects
   - Implementation: Need new `choose_card_from_library()` method in PlayerController

## Test Reproduction

```bash
cargo run --release --bin mtg -- tui decks/ryan_spiderman_draft.dck decks/ryan_spiderman_draft.dck \
    --p1=fixed --p2=fixed --p1-fixed-inputs="0" --p2-fixed-inputs="" \
    --stop-on-choice=2 --seed=42 --json --log-tail=100
```

Expected output now shows:
```
Alice available actions:
  [0] Pass priority
  [1] Play land: Daily Bugle Building
  ...
```
