---
title: 'Bug: Peter Porker (Spider-Ham) incorrectly disappears when Food token is sacrificed'
status: open
priority: 2
issue_type: bug
created_at: 2025-11-30T01:37:46.253851447+00:00
updated_at: 2025-11-30T19:45:31.134722926+00:00
---

# Description

## Peter Porker TUI Bug - Actually Dies from State-Based Actions

## Status: INVESTIGATING - Root Cause Found

**UPDATE 2025-11-30_#219(5ae2ec1):** Discovered Peter Porker is DYING from state-based actions, not just disappearing from TUI!

## Problem

After attacking with Spider-Ham, Peter Porker (id=29), the card disappears from the TUI battlefield display. Investigation revealed this is NOT a TUI rendering bug - **Peter Porker is actually being moved to the graveyard by state-based actions**.

## Debug Evidence

### Zone Movement Logs

```
[DEBUG zone] Moving card Spider-Ham, Peter Porker (id=29) from Stack to Battlefield
[DEBUG token] Created token Food Token (id=84) under player 0's control
[DEBUG zone] Moving card Spider-Ham, Peter Porker (id=5) from Battlefield to Graveyard
```

Peter Porker enters battlefield successfully, then later moves to graveyard.

### State-Based Actions Check Needed

Added SBA debug logging (commit 5ae2ec1):
```rust
log::debug!(target: "sba", "SBA check: {} (id={}) damage={} toughness={} has_lethal={} indestructible={}",
    card.name, card_id.as_u32(), card.damage, toughness, has_lethal, card.has_indestructible());
```

**Next step:** Run with `RUST_LOG=sba=debug` to see Peter Porker's exact damage/toughness when it dies.

## Root Cause Hypothesis

Peter Porker is dying because either:
1. **Damage >= Toughness** - Taking combat damage or other damage
2. **Toughness <= 0** - Power/toughness modification reducing toughness to 0 or below

The question is: WHY does Peter Porker have lethal damage or 0 toughness?

## Card Definition

From `spider_ham_peter_porker.txt`:
```
Name:Spider-Ham, Peter Porker
ManaCost:1 G
Types:Legendary Creature Spider Boar Hero
PT:2/2
```

Base stats are 2/2, so something must be modifying them or dealing damage.

## Tools Added for Investigation

### FancyFixed Controller (Phase 1 - Commit 5ae2ec1)

Added `--p1=fancy-fixed` controller type:
- Accepts scripted inputs via `--p1-fixed-inputs`
- Enables automated debugging without interactive terminal
- Currently delegates to RichInputController
- **Phase 2 (mtg-34fc28):** Will add TUI screenshot capture

### Debug Logging

**SBA logging** (state.rs:633-637):
```bash
RUST_LOG=sba=debug cargo run --release --bin mtg -- tui ...
```

Shows damage/toughness for all creatures when checking state-based actions.

### Debug Scripts

**`scripts/debug_peter_porker_tui.sh`:**
- Guarantees Peter Porker in opening hand
- Runs Fancy TUI with debug logging
- Logs saved to `mtg_forge.log`

## Investigation Plan

### Step 1: Capture SBA Logs ✅ READY

Run game with SBA debug logging to see Peter Porker's stats when it dies:

```bash
RUST_LOG=sba=debug,zone=debug ./scripts/debug_peter_porker_tui.sh
```

After bug occurs, check logs:
```bash
grep "Peter Porker.*SBA check" mtg_forge.log
```

Look for damage/toughness values in the log line right before it dies.

### Step 2: Analyze Why Damage/Toughness Changed

Once we know the values, investigate:
- If `damage > 0`: Where did the damage come from?
- If `toughness <= 0`: What reduced the toughness?
- Check combat log, activated abilities, continuous effects

### Step 3: Fix the Bug

Depending on root cause:
- **If incorrectly taking damage:** Fix combat/damage assignment
- **If incorrectly losing toughness:** Fix P/T modification logic
- **If SBA check is wrong:** Fix lethal damage check (unlikely)

## Code Locations

**State-based actions:**
- `mtg-engine/src/game/state.rs:617-653` - `check_lethal_damage()`
- Line 630: `let toughness = card.current_toughness();`
- Line 631: `let has_lethal = card.damage >= toughness as i32;`

**Power/Toughness calculation:**
- `mtg-engine/src/core/card.rs` - `current_toughness()` method
- Need to check if P/T bonuses/reductions are correctly applied

**Combat damage:**
- `mtg-engine/src/game/actions/combat.rs` - Combat damage assignment

## Previous Analysis (Pre-SBA Discovery)

~~Initially thought this was a TUI rendering bug. Investigation found:~~
- ~~Peter Porker IS on battlefield (zone logs confirm)~~
- ~~Peter Porker IS targetable (shows in target UI)~~
- ~~Categorization code looks correct (is_creature() should return true)~~

**CORRECTION:** Peter Porker is NOT on battlefield after the bug - it's in the graveyard!
The "targetable" observation was from an earlier game state before it died.

## Related Work

- **mtg-34fc28:** Phase 2 - Add TUI screenshot capture to FancyFixed controller
  - Will enable visual debugging of TUI state
  - Can capture exact moment Peter Porker disappears
  - Screenshots will show battlefield contents at each choice

## Reproduction

```bash
./scripts/debug_peter_porker_tui.sh
```

1. Turn 1: Play a Forest
2. Turn 2: Play a Forest, cast Spider-Ham Peter Porker (1G)
3. Turn 3: Attack with Peter Porker
4. **BUG:** Peter Porker dies (moves to graveyard)

Peter Porker is guaranteed in opening hand via `--p1-draw` parameter.

---

**Next Action:** Run with SBA debug logging to capture Peter Porker's stats when it dies.
