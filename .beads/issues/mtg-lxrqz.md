---
title: 'Card Compatibility: Lightning Bolt'
status: open
priority: 2
issue_type: bug
created_at: 2026-05-28T02:30:47.751675650+00:00
updated_at: 2026-05-28T02:30:47.751675650+00:00
---

# Description

BROKEN: cannot target opponent player.

Card script: cardsfolder/l/lightning_bolt.txt
Effect: SP$ DealDamage | ValidTgts$ Any | NumDmg$ 3

USER BUG REPORT (fix-gameplay-bugs-4pack): "I have 2 creatures on the battlefield (opponent none). I cast lightning bolt to damage opponent, but they are not one of the targets I am presented with! Only my own creatures or 'No target'."

ROOT CAUSE: `Controller::choose_targets` accepts only `&[CardId]`. Players have PlayerIds, not CardIds, so they were never in the offered target list. The legacy targeting code path explicitly noted 'Players are also valid targets, but we handle them separately via TargetRef::Player'—but nothing ever offered the player choice.

FIX: encode PlayerId as a sentinel CardId in the valid_targets list using PLAYER_TARGET_BASE = u32::MAX-1000. `Controller::choose_targets` now offers Players for ValidTgts$ Any / ValidTgts$ Player damage. The sentinel is decoded back into a TargetRef::Player at effect-resolution time and in the resolve_spell fizzle-check (CR 608.2b).

CR 115.4: 'each instance of the word "target" is followed by an object or player.'
CR 601.2c: choosing targets is step 6 of casting.
