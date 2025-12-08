---
title: Upstream Java Forge card script issues
status: open
priority: 1
issue_type: task
created_at: 2025-12-08T14:57:03.753505090+00:00
updated_at: 2025-12-08T14:57:03.753505090+00:00
---

# Description

## Upstream Java Forge Card Script Issues

This issue tracks any card scripts in the upstream Java Forge repository (`forge-java/forge-gui/res/cardsfolder/`) that appear to be incorrect or buggy.

## Purpose

When we encounter cards that don't work as expected, we need to determine:
1. Is it a bug in our Rust implementation?
2. Is it a bug in the upstream Java Forge card script?

If it's an upstream bug, we should:
1. Document it here
2. File a PR or issue to the upstream forge repository
3. Optionally create a local workaround

## Upstream Repository

- GitHub: https://github.com/Card-Forge/forge
- Card scripts location: `forge-gui/res/cardsfolder/`

## Cards NOT on this list (verified correct)

### Balance (cardsfolder/b/balance.txt)
- **Status**: Script is CORRECT
- **Issue**: Only land balancing works in our engine
- **Root cause**: Our Rust loader doesn't implement `SubAbility$` chaining
- **Action needed**: Implement SubAbility parsing in our loader (see mtg-3)
- The card script properly chains: Land → Hand → Creature via SVars

## Cards with potential upstream issues

(None identified yet)

## How to verify a card script

1. Read the card script in `forge-java/forge-gui/res/cardsfolder/`
2. Compare against the Oracle text (official MTG rules text)
3. Check the Java effect implementation in `forge-game/src/main/java/forge/game/ability/effects/`
4. Test in Java Forge if possible to see expected behavior
5. If script is wrong, document here and file upstream PR

## Related issues

- mtg-3: MTG feature completeness (tracks our implementation gaps)
- mtg-glaxo: White Weenie deck card implementation
