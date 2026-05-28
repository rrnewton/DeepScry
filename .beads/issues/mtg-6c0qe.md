---
title: 'Card Compatibility: Mind Twist'
status: open
priority: 2
issue_type: bug
created_at: 2026-05-28T02:07:23.501379236+00:00
updated_at: 2026-05-28T02:07:23.501379236+00:00
---

# Description

BROKEN: target player discards X cards at random.

Card script: cardsfolder/m/mind_twist.txt
Effect: SP$ Discard | ValidTgts$ Player | NumCards$ X | Mode$ Random

USER BUG REPORT (fix-gameplay-bugs-4pack): "In testing the jeskai aggro old school decks, the opponent cast mind twist on me with X=8 and I did not discard ANY cards."

ROOT CAUSE: effect_converter.rs for ApiType::Discard parses ValidTgts$ Player to placeholder. resolve_target_for_effect then resolves the placeholder PlayerId to card_owner (the caster), not the targeted opponent. So opponent's Mind Twist discards FROM THE OPPONENT, who typically has empty hand.

CR 116.2c: 'Target' is a parameter. CR 601 governs targeting. In a 2-player game, ValidTgts$ Player with no explicit target choice should default to opponent (the practical caster-vs-defender pattern; this matches LoseLife/ForceSacrifice defaults at mod.rs:2337-2349).
